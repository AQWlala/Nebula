//! T-E-S-31: MCP SSE 传输层 — GET /sse 长连接 + POST /messages 双通道。
//!
//! 本模块实现 MCP 规范的 SSE(Stream of Server-Encoded Events)传输:
//!
//! * 客户端通过 `GET /sse` 建立长连接,服务器推送事件流
//!   (首个 `endpoint` 事件告知 POST URL,后续推送 JSON-RPC 响应)。
//! * 客户端通过 `POST /messages` 发送 JSON-RPC 请求,body 为单条 JSON。
//!
//! ## 设计要点
//!
//! 1. **零新依赖**: reqwest 已启用 `stream` feature;手写 SSE 行解析器
//!    (跨 chunk 缓冲,见 [`SseEventAccumulator`])。
//! 2. **双 client**: `post_client`(30s 超时) + `sse_client`(无超时,长连接)。
//! 3. **后台监听 task**: 持续读 `bytes_stream`,解析事件,通过 oneshot
//!    channel 路由响应给等待中的 `send_and_receive`。
//! 4. **重连**: 指数退避 1s→2→4→8→16→30,最多 10 次(可由 [`ReconnectConfig`]
//!    调整;成功连接后重置退避计数);失败向所有 pending sender 发 Err。
//! 5. **SSRF 校验**: `new()` 第一行 `SsrfGuard::validate_url`,拒绝
//!    loopback/private/link-local/CGNAT/broadcast。
//! 6. **拆分 new/start**: `new()` 只校验 + 构造状态(不启动 task),
//!    `start_listener()` 启动后台 task。测试用 `new()` 不 start,避免 hang。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::protocol::{parse_frame, JsonRpcRequest, JsonRpcResponse};
use crate::security::ssrf_guard::SsrfGuard;

/// SSE 事件(解析后的单条事件)。
///
/// 见 <https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream>
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` 字段(默认为 "message")。
    pub event: Option<String>,
    /// `data:` 字段行(多行 data 用 `\n` 拼接)。
    pub data: Vec<String>,
    /// `id:` 字段(用于 Last-Event-ID 续传)。
    pub id: Option<String>,
}

impl SseEvent {
    fn data_joined(&self) -> String {
        self.data.join("\n")
    }
}

/// SSE 行解析器(跨 chunk 缓冲)。
///
/// 状态机: 累积 `line_buf`,遇 `\n` 触发行处理;遇空行触发事件分派。
#[derive(Debug, Default)]
struct SseEventAccumulator {
    /// 跨 chunk 未结束的字节缓冲(UTF-8,可能含未完成的行)。
    line_buf: String,
    /// 当前正在累积的事件(空行时分派并重置)。
    current: SseEvent,
}

impl SseEventAccumulator {
    fn new() -> Self {
        Self::default()
    }

    /// 喂入一个 chunk,返回该 chunk 内分派出的完整事件列表。
    ///
    /// 规则:
    /// - `\n` 或 `\r\n` 结束一行
    /// - 空行 → 触发事件分派(若 current 非空)+ 重置
    /// - `:` 开头 → 注释,忽略
    /// - `data:xxx` → push(去前导空格)
    /// - `event:xxx` → 设 event
    /// - `id:xxx` → 设 id
    /// - `retry:xxx` → 解析但忽略
    fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        // 把 chunk 追加到缓冲(可能引入多行)。
        let chunk_str = match std::str::from_utf8(chunk) {
            Ok(s) => s.to_string(),
            Err(_) => {
                // UTF-8 边界切断 — 丢弃无效字节(简化实现,生产场景应缓冲不完整字节)。
                warn!(target: "nebula.mcp.sse", "SSE chunk has invalid UTF-8, skipping");
                return Vec::new();
            }
        };
        self.line_buf.push_str(&chunk_str);

        let mut events = Vec::new();
        while let Some(nl_pos) = self.line_buf.find('\n') {
            // 提取一行(去除 \n;若前一个字符是 \r 也去除)。
            let mut line = self.line_buf[..nl_pos].to_string();
            if line.ends_with('\r') {
                line.pop();
            }
            // 从缓冲移除已处理的行 + \n。
            self.line_buf = self.line_buf[nl_pos + 1..].to_string();

            if line.is_empty() {
                // 空行 → 触发事件分派(若 current 有内容)。
                if !self.current.data.is_empty() || self.current.event.is_some() {
                    events.push(std::mem::take(&mut self.current));
                }
                continue;
            }
            if line.starts_with(':') {
                // 注释,忽略。
                continue;
            }
            // 解析 `field: value` 或 `field`(value 为空)。
            let (field, value) = match line.find(':') {
                Some(colon_pos) => {
                    let f = &line[..colon_pos];
                    let mut v = &line[colon_pos + 1..];
                    // 去前导空格(单个)。
                    if v.starts_with(' ') {
                        v = &v[1..];
                    }
                    (f, v)
                }
                None => (line.as_str(), ""),
            };
            match field {
                "data" => self.current.data.push(value.to_string()),
                "event" => self.current.event = Some(value.to_string()),
                "id" => self.current.id = Some(value.to_string()),
                "retry" => {
                    // 解析但忽略(规范要求重连时遵守 retry,本实现简化)。
                    if let Ok(ms) = value.parse::<u64>() {
                        debug!(target: "nebula.mcp.sse", retry_ms = ms, "SSE retry hint received");
                    }
                }
                _ => {
                    // 未知字段,忽略(规范要求忽略未知字段名)。
                }
            }
        }
        events
    }
}

/// SSE 重连策略配置(可调参数)。
///
/// 默认值: 最多 10 次重连,初始 1s,最大 30s,产生 1→2→4→8→16→30→30→... 退避序列。
/// 成功连接后,退避计数会被重置(见 [`SseTransport::start_listener`] 中 `attempt = 0`)。
#[derive(Debug, Clone, Copy)]
pub struct ReconnectConfig {
    /// 最大重连尝试次数(首次失败后的重试次数上限,默认 10)。
    pub max_reconnect_attempts: u32,
    /// 初始重连延迟(默认 1s,作为指数退避的基数)。
    pub initial_reconnect_delay: Duration,
    /// 最大重连延迟(默认 30s,退避封顶值)。
    pub max_reconnect_delay: Duration,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_reconnect_attempts: 10,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(30),
        }
    }
}

impl ReconnectConfig {
    /// 计算第 `attempt` 次重连的退避延迟(0-indexed)。
    ///
    /// 公式: `initial * 2^attempt`,封顶 `max`。
    /// 默认配置下序列为 1→2→4→8→16→30→30→...(见 [`ReconnectConfig::default`])。
    ///
    /// `attempt` 从 0 开始(首次失败后调用 `backoff(0)` 返回 `initial`)。
    pub fn backoff(&self, attempt: u32) -> Duration {
        // 用秒粒度计算,与原 next_backoff 行为一致。
        let initial_secs = self.initial_reconnect_delay.as_secs().max(1);
        let max_secs = self.max_reconnect_delay.as_secs().max(initial_secs);
        // 限位 shift 位数避免溢出(2^20 * 1s ≈ 12 天,远超任何合理的 max)。
        let exp = attempt.min(20);
        let scaled = initial_secs.checked_shl(exp).unwrap_or(u64::MAX);
        Duration::from_secs(scaled.min(max_secs))
    }
}

/// 重连退避计算(使用默认 [`ReconnectConfig`]): 1→2→4→8→16→30(封顶)。
///
/// 兼容性保留: 内部测试调用以验证默认退避序列。
#[cfg(test)]
fn next_backoff(attempt: u32) -> Duration {
    ReconnectConfig::default().backoff(attempt)
}

/// T-E-S-31: SSE 传输 — GET /sse 长连接 + POST /messages 双通道。
pub struct SseTransport {
    base_url: String,
    api_key: Option<String>,
    /// POST 通道 client(30s 超时)。
    post_client: reqwest::Client,
    /// GET /sse 长连接 client(无超时)。
    sse_client: reqwest::Client,
    /// 等待响应的 pending 请求映射: request_id → oneshot sender。
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// 后台监听 task 句柄(None 表示未启动)。
    listener_handle: Option<JoinHandle<()>>,
    /// 取消信号(watch)。
    cancel: watch::Sender<bool>,
    /// 最后收到的事件 ID(用于 Last-Event-ID 续传)。
    last_event_id: Arc<Mutex<Option<String>>>,
    /// 动态更新的 POST URL(首个 endpoint 事件告知)。
    post_url: Arc<tokio::sync::RwLock<String>>,
    /// 重连策略配置(最大重连次数 + 退避参数)。
    reconnect_config: ReconnectConfig,
}

impl SseTransport {
    /// 构造 SseTransport — 仅校验 URL + 构造 client,不启动后台 task。
    ///
    /// 调用方需在握手前显式调用 [`start_listener`](Self::start_listener)。
    /// 这样拆分是为了测试: 测试可只调用 `new()` 验证 SSRF 拒绝/接受,
    /// 而不启动真实网络监听(避免 hang)。
    ///
    /// 使用默认重连策略(见 [`ReconnectConfig::default`]: 最多 10 次重连,
    /// 1→30s 指数退避)。如需自定义,用 [`Self::new_with_config`]。
    pub fn new(url: String, api_key: Option<String>) -> Result<Self> {
        Self::new_with_config(url, api_key, ReconnectConfig::default())
    }

    /// 构造 SseTransport — 使用自定义 [`ReconnectConfig`]。
    ///
    /// 与 [`Self::new`] 相同,但允许调用方覆盖最大重连次数与退避参数,
    /// 适用于长生命周期服务(更高的 `max_reconnect_attempts`)或测试场景
    /// (更短的 `initial_reconnect_delay` 加速测试)。
    pub fn new_with_config(
        url: String,
        api_key: Option<String>,
        reconnect_config: ReconnectConfig,
    ) -> Result<Self> {
        // SSRF 校验(第一行)。
        SsrfGuard::new().validate_url(&url)?;
        // 构造 SSRF 安全 client(重定向链每跳校验)。
        let post_client = SsrfGuard::new().build_safe_client()?;
        let sse_client = {
            // SSE 长连接无超时;复用 SSRF 安全重定向策略。
            let guard = SsrfGuard::new();
            let policy_guard = guard.clone();
            let policy = reqwest::redirect::Policy::custom(move |attempt| {
                if let Err(e) = policy_guard.validate_url(&attempt.url().to_string()) {
                    attempt.error(e.to_string())
                } else {
                    attempt.follow()
                }
            });
            reqwest::Client::builder()
                .redirect(policy)
                .build()
                .map_err(|e| anyhow::anyhow!("failed to build SSE client: {e}"))?
        };

        let (cancel, _) = watch::channel(false);

        Ok(Self {
            base_url: url,
            api_key,
            post_client,
            sse_client,
            pending: Arc::new(Mutex::new(HashMap::new())),
            listener_handle: None,
            cancel,
            last_event_id: Arc::new(Mutex::new(None)),
            post_url: Arc::new(tokio::sync::RwLock::new(String::new())),
            reconnect_config,
        })
    }

    /// 启动后台 SSE 监听 task。
    ///
    /// task 持续 GET /sse + bytes_stream + SseEventAccumulator,
    /// 解析事件并路由响应。流断开时按 [`ReconnectConfig`] 指数退避重连
    /// (默认最多 10 次,1→2→4→8→16→30→30→...;成功连接后重置退避计数)。
    pub fn start_listener(&mut self) {
        if self.listener_handle.is_some() {
            return;
        }
        let sse_client = self.sse_client.clone();
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let pending = self.pending.clone();
        let mut cancel_rx = self.cancel.subscribe();
        let last_event_id = self.last_event_id.clone();
        let post_url = self.post_url.clone();
        let reconnect_cfg = self.reconnect_config;

        let handle = tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                if cancel_rx.has_changed().unwrap_or(false) && *cancel_rx.borrow() {
                    debug!(target: "nebula.mcp.sse", "SSE listener cancelled");
                    return;
                }

                let mut req = sse_client
                    .get(&base_url)
                    .header("Accept", "text/event-stream")
                    .header("Cache-Control", "no-cache");
                if let Some(ref key) = api_key {
                    req = req.header("Authorization", format!("Bearer {}", key));
                }
                if let Some(id) = last_event_id.lock().clone() {
                    req = req.header("Last-Event-ID", id);
                }

                match req.send().await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!(
                                target: "nebula.mcp.sse",
                                status = %resp.status(),
                                attempt,
                                "SSE GET failed"
                            );
                        } else {
                            if attempt > 0 {
                                info!(
                                    target: "nebula.mcp.sse",
                                    attempt,
                                    "SSE stream reconnected, resetting backoff"
                                );
                            } else {
                                info!(target: "nebula.mcp.sse", "SSE stream connected");
                            }
                            attempt = 0; // 成功连接,重置退避计数
                            let mut stream = resp.bytes_stream();
                            let mut acc = SseEventAccumulator::new();
                            let mut stream_err = false;
                            while let Some(chunk_result) = stream.next().await {
                                if cancel_rx.has_changed().unwrap_or(false) && *cancel_rx.borrow() {
                                    debug!(target: "nebula.mcp.sse", "SSE listener cancelled mid-stream");
                                    return;
                                }
                                match chunk_result {
                                    Ok(chunk) => {
                                        let events = acc.feed(&chunk);
                                        for ev in events {
                                            // 更新 last_event_id(若有)。
                                            if let Some(id) = &ev.id {
                                                *last_event_id.lock() = Some(id.clone());
                                            }
                                            // 首个 endpoint 事件 → 更新 post_url。
                                            if ev.event.as_deref() == Some("endpoint") {
                                                let url = ev.data_joined();
                                                debug!(target: "nebula.mcp.sse", endpoint = %url, "SSE endpoint received");
                                                *post_url.write().await = url;
                                                continue;
                                            }
                                            // 尝试作为 JSON-RPC 响应路由。
                                            let data = ev.data_joined();
                                            if data.trim().is_empty() {
                                                continue;
                                            }
                                            match parse_frame(data.trim()) {
                                                Ok(resp) => {
                                                    if let Some(id_num) =
                                                        resp.id.as_u64().or_else(|| {
                                                            resp.id.as_i64().map(|i| i as u64)
                                                        })
                                                    {
                                                        let sender_opt = {
                                                            let mut p = pending.lock();
                                                            p.remove(&id_num)
                                                        };
                                                        if let Some(sender) = sender_opt {
                                                            let _ = sender.send(resp);
                                                        } else {
                                                            debug!(target: "nebula.mcp.sse", id = id_num, "no pending request for SSE response");
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    debug!(target: "nebula.mcp.sse", error = %e, "SSE event not a JSON-RPC response, ignoring");
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!(target: "nebula.mcp.sse", error = %e, attempt, "SSE stream error");
                                        stream_err = true;
                                        break;
                                    }
                                }
                            }
                            if !stream_err {
                                debug!(target: "nebula.mcp.sse", "SSE stream ended gracefully");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            target: "nebula.mcp.sse",
                            error = %e,
                            attempt,
                            "SSE GET request failed"
                        );
                    }
                }

                // 流断开 → 重连(指数退避,最多 max_reconnect_attempts 次)。
                if attempt >= reconnect_cfg.max_reconnect_attempts {
                    warn!(
                        target: "nebula.mcp.sse",
                        attempt,
                        max = reconnect_cfg.max_reconnect_attempts,
                        "SSE reconnect attempts exhausted, failing all pending"
                    );
                    fail_all_pending(&pending, "SSE reconnect failed after max attempts");
                    return;
                }
                let backoff = reconnect_cfg.backoff(attempt);
                warn!(
                    target: "nebula.mcp.sse",
                    attempt,
                    next_attempt = attempt + 1,
                    max = reconnect_cfg.max_reconnect_attempts,
                    backoff_secs = backoff.as_secs(),
                    "SSE reconnect attempt failed, backing off"
                );
                // 用 select 同时等待 backoff 和 cancel 信号。
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = cancel_rx.changed() => {
                        if *cancel_rx.borrow() {
                            debug!(target: "nebula.mcp.sse", "SSE listener cancelled during backoff");
                            return;
                        }
                    }
                }
                attempt += 1;
            }
        });
        self.listener_handle = Some(handle);
    }

    /// 发送 JSON-RPC 请求并等待响应(POST + oneshot,30s 超时)。
    ///
    /// 流程: 注册 pending sender → POST 请求 → 等待 oneshot(30s) → 返回响应。
    pub async fn send_and_receive(&mut self, req: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        // 取 request id(u64 形式;非 u64 id 用 fallback)。
        let id_num = req
            .id
            .as_ref()
            .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)))
            .context("JSON-RPC request missing u64 id for SSE routing")?;

        // 注册 oneshot sender。
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock();
            if pending.contains_key(&id_num) {
                anyhow::bail!("duplicate request id {} in pending SSE map", id_num);
            }
            pending.insert(id_num, tx);
        }

        // 等 post_url 就绪(首个 endpoint 事件可能尚未到达)。
        let post_url = {
            let max_wait = Duration::from_secs(10);
            let mut elapsed = Duration::from_millis(0);
            loop {
                let url = self.post_url.read().await.clone();
                if !url.is_empty() {
                    break url;
                }
                if elapsed >= max_wait {
                    // 移除 pending sender 避免 leak。
                    self.pending.lock().remove(&id_num);
                    anyhow::bail!("SSE post_url not received within 10s (no endpoint event)");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                elapsed += Duration::from_millis(100);
            }
        };

        // POST 请求。
        let mut post_req = self.post_client.post(&post_url).json(req);
        if let Some(ref key) = self.api_key {
            post_req = post_req.header("Authorization", format!("Bearer {}", key));
        }
        if let Err(e) = post_req.send().await {
            self.pending.lock().remove(&id_num);
            anyhow::bail!("SSE POST request failed: {}", e);
        }

        // 等待 oneshot(30s 超时)。
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => {
                // sender 被 drop(listener task 失败时 fail_all_pending 已发送 Err,
                // 此分支理论上不应触发)。
                self.pending.lock().remove(&id_num);
                anyhow::bail!("SSE response channel closed unexpectedly")
            }
            Err(_) => {
                self.pending.lock().remove(&id_num);
                anyhow::bail!("SSE response timeout (30s) for request id {}", id_num)
            }
        }
    }

    /// 关闭: 发 cancel 信号 + 等 listener_handle 结束。
    pub async fn shutdown(&mut self) {
        let _ = self.cancel.send(true);
        if let Some(handle) = self.listener_handle.take() {
            // 等最多 2 秒,超时则 abandon。
            match tokio::time::timeout(Duration::from_secs(2), handle).await {
                Ok(_) => {}
                Err(_) => {
                    warn!(target: "nebula.mcp.sse", "SSE listener did not shutdown within 2s, abandoning");
                }
            }
        }
        // 清空 pending(向所有 sender 发 Err)。
        fail_all_pending(&self.pending, "SSE transport shutdown");
    }
}

/// 向所有 pending sender 发送错误响应(通过 drop sender 实现,
/// 等待方会收到 RecvError)。
fn fail_all_pending(
    pending: &Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    _reason: &str,
) {
    let mut p = pending.lock();
    p.clear(); // drop 所有 sender → 等待方收到 RecvError
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- SseEventAccumulator 测试 ----------

    /// T-E-S-31: SSE 行解析器解析 data/event/id。
    #[test]
    fn sse_parser_parses_data_event_id() {
        let mut acc = SseEventAccumulator::new();
        let chunk = b"event: result\ndata: {\"id\":1}\nid: 42\n\n";
        let events = acc.feed(chunk);
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.event.as_deref(), Some("result"));
        assert_eq!(ev.data, vec!["{\"id\":1}".to_string()]);
        assert_eq!(ev.id.as_deref(), Some("42"));
    }

    /// T-E-S-31: SSE 行解析器处理跨 chunk(单个事件被切到两个 chunk)。
    #[test]
    fn sse_parser_handles_cross_chunk() {
        let mut acc = SseEventAccumulator::new();
        // 第一个 chunk 切断在 data 字段中间。
        let chunk1 = b"event: result\ndata: {\"id\":";
        let chunk2 = b"1}\n\n";
        let events1 = acc.feed(chunk1);
        assert!(events1.is_empty(), "no complete event yet");
        let events2 = acc.feed(chunk2);
        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].event.as_deref(), Some("result"));
        assert_eq!(events2[0].data, vec!["{\"id\":1}".to_string()]);
    }

    /// T-E-S-31: SSE 行解析器忽略注释(`:` 开头)。
    #[test]
    fn sse_parser_ignores_comments() {
        let mut acc = SseEventAccumulator::new();
        let chunk = b": this is a comment\ndata: hello\n\n";
        let events = acc.feed(chunk);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, vec!["hello".to_string()]);
    }

    /// T-E-S-31: SSE 行解析器空行触发事件分派。
    #[test]
    fn sse_parser_empty_line_dispatches_event() {
        let mut acc = SseEventAccumulator::new();
        // 两个事件,各由空行分派。
        let chunk = b"data: first\n\ndata: second\n\n";
        let events = acc.feed(chunk);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, vec!["first".to_string()]);
        assert_eq!(events[1].data, vec!["second".to_string()]);
    }

    /// T-E-S-31: SSE 行解析器多行 data 用 \n 拼接。
    #[test]
    fn sse_parser_multiline_data_joined() {
        let mut acc = SseEventAccumulator::new();
        let chunk = b"data: line1\ndata: line2\ndata: line3\n\n";
        let events = acc.feed(chunk);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, vec!["line1", "line2", "line3"]);
        assert_eq!(events[0].data_joined(), "line1\nline2\nline3");
    }

    /// T-E-S-31: SSE 行解析器处理 \r\n 行尾。
    #[test]
    fn sse_parser_handles_crlf_line_endings() {
        let mut acc = SseEventAccumulator::new();
        let chunk = b"data: hello\r\n\r\n";
        let events = acc.feed(chunk);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, vec!["hello".to_string()]);
    }

    // ---------- next_backoff 测试 ----------

    /// T-E-S-31: 重连退避计算 next_backoff(1→2→4→8→16→30)。
    #[test]
    fn next_backoff_returns_expected_sequence() {
        assert_eq!(next_backoff(0), Duration::from_secs(1));
        assert_eq!(next_backoff(1), Duration::from_secs(2));
        assert_eq!(next_backoff(2), Duration::from_secs(4));
        assert_eq!(next_backoff(3), Duration::from_secs(8));
        assert_eq!(next_backoff(4), Duration::from_secs(16));
        assert_eq!(next_backoff(5), Duration::from_secs(30));
        // 封顶 30s。
        assert_eq!(next_backoff(100), Duration::from_secs(30));
    }

    // ---------- ReconnectConfig 默认值 + backoff 测试 ----------

    /// T-E-S-31: ReconnectConfig::default 提供期望的默认值
    /// (max=10, initial=1s, max=30s)。
    #[test]
    fn reconnect_config_default_values() {
        let cfg = ReconnectConfig::default();
        assert_eq!(cfg.max_reconnect_attempts, 10);
        assert_eq!(cfg.initial_reconnect_delay, Duration::from_secs(1));
        assert_eq!(cfg.max_reconnect_delay, Duration::from_secs(30));
    }

    /// T-E-S-31: 默认 ReconnectConfig 退避序列与 next_backoff 一致
    /// (1→2→4→8→16→30→30→...)。
    #[test]
    fn reconnect_config_backoff_default_sequence() {
        let cfg = ReconnectConfig::default();
        assert_eq!(cfg.backoff(0), Duration::from_secs(1));
        assert_eq!(cfg.backoff(1), Duration::from_secs(2));
        assert_eq!(cfg.backoff(2), Duration::from_secs(4));
        assert_eq!(cfg.backoff(3), Duration::from_secs(8));
        assert_eq!(cfg.backoff(4), Duration::from_secs(16));
        // 1<<5 = 32,封顶至 30。
        assert_eq!(cfg.backoff(5), Duration::from_secs(30));
        assert_eq!(cfg.backoff(10), Duration::from_secs(30));
    }

    /// T-E-S-31: 自定义 initial_reconnect_delay 改变退避基数。
    #[test]
    fn reconnect_config_custom_initial_delay() {
        let cfg = ReconnectConfig {
            max_reconnect_attempts: 5,
            initial_reconnect_delay: Duration::from_secs(2),
            max_reconnect_delay: Duration::from_secs(30),
        };
        // 2 → 4 → 8 → 16 → 30 → 30 ...
        assert_eq!(cfg.backoff(0), Duration::from_secs(2));
        assert_eq!(cfg.backoff(1), Duration::from_secs(4));
        assert_eq!(cfg.backoff(2), Duration::from_secs(8));
        assert_eq!(cfg.backoff(3), Duration::from_secs(16));
        // 2<<4 = 32,封顶至 30。
        assert_eq!(cfg.backoff(4), Duration::from_secs(30));
        assert_eq!(cfg.backoff(99), Duration::from_secs(30));
    }

    /// T-E-S-31: 自定义 max_reconnect_delay 改变封顶值。
    #[test]
    fn reconnect_config_custom_max_delay() {
        let cfg = ReconnectConfig {
            max_reconnect_attempts: 3,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(10),
        };
        // 1 → 2 → 4 → 8 → 10(封顶)
        assert_eq!(cfg.backoff(0), Duration::from_secs(1));
        assert_eq!(cfg.backoff(1), Duration::from_secs(2));
        assert_eq!(cfg.backoff(2), Duration::from_secs(4));
        assert_eq!(cfg.backoff(3), Duration::from_secs(8));
        // 1<<4 = 16,封顶至 10。
        assert_eq!(cfg.backoff(4), Duration::from_secs(10));
        assert_eq!(cfg.backoff(50), Duration::from_secs(10));
    }

    /// T-E-S-31: max < initial 时退避仍 >= initial(防卫性,避免配置反转
    /// 导致退避塌缩为 0)。
    #[test]
    fn reconnect_config_backoff_when_max_below_initial() {
        let cfg = ReconnectConfig {
            max_reconnect_attempts: 3,
            initial_reconnect_delay: Duration::from_secs(5),
            max_reconnect_delay: Duration::from_secs(2),
        };
        // max_secs 被 clamp 到 initial_secs,因此所有 attempt 都返回 5s。
        assert_eq!(cfg.backoff(0), Duration::from_secs(5));
        assert_eq!(cfg.backoff(10), Duration::from_secs(5));
    }

    /// T-E-S-31: new_with_config 接受自定义 ReconnectConfig 并存储。
    #[test]
    fn sse_transport_new_with_config_stores_config() {
        let cfg = ReconnectConfig {
            max_reconnect_attempts: 7,
            initial_reconnect_delay: Duration::from_secs(2),
            max_reconnect_delay: Duration::from_secs(45),
        };
        let r = SseTransport::new_with_config("https://1.1.1.1/sse".to_string(), None, cfg);
        assert!(r.is_ok());
        let transport = r.unwrap();
        assert_eq!(transport.reconnect_config.max_reconnect_attempts, 7);
        assert_eq!(
            transport.reconnect_config.initial_reconnect_delay,
            Duration::from_secs(2)
        );
        assert_eq!(
            transport.reconnect_config.max_reconnect_delay,
            Duration::from_secs(45)
        );
    }

    /// T-E-S-31: new() 使用默认 ReconnectConfig(10 次,1s,30s)。
    #[test]
    fn sse_transport_new_uses_default_reconnect_config() {
        let r = SseTransport::new("https://1.1.1.1/sse".to_string(), None);
        assert!(r.is_ok());
        let transport = r.unwrap();
        assert_eq!(transport.reconnect_config.max_reconnect_attempts, 10);
        assert_eq!(
            transport.reconnect_config.initial_reconnect_delay,
            Duration::from_secs(1)
        );
        assert_eq!(
            transport.reconnect_config.max_reconnect_delay,
            Duration::from_secs(30)
        );
    }

    // ---------- SseTransport SSRF 测试 ----------

    /// T-E-S-31: SseTransport::new 拒绝 192.168.x。
    #[test]
    fn sse_transport_rejects_192_168() {
        let r = SseTransport::new("http://192.168.1.1/sse".to_string(), None);
        assert!(r.is_err());
    }

    /// T-E-S-31: SseTransport::new 拒绝 127.x(loopback)。
    #[test]
    fn sse_transport_rejects_loopback() {
        let r = SseTransport::new("http://127.0.0.1/sse".to_string(), None);
        assert!(r.is_err());
    }

    /// T-E-S-31: SseTransport::new 拒绝 10.x。
    #[test]
    fn sse_transport_rejects_10() {
        let r = SseTransport::new("http://10.0.0.1/sse".to_string(), None);
        assert!(r.is_err());
    }

    /// T-E-S-31: SseTransport::new 拒绝 169.254.x(link-local)。
    #[test]
    fn sse_transport_rejects_link_local() {
        let r = SseTransport::new("http://169.254.169.254/sse".to_string(), None);
        assert!(r.is_err());
    }

    /// T-E-S-31: SseTransport::new 接受公网 URL。
    ///
    /// 注意: `new()` 不启动后台 task(拆分 new/start_listener),
    /// 因此此测试不会 hang。仅验证 SSRF 校验通过 + client 构造成功。
    ///
    /// 使用公网 IP `1.1.1.1`(Cloudflare DNS)而非域名,避免测试环境
    /// DNS 不可用导致误失败。
    #[test]
    fn sse_transport_accepts_public_url() {
        let r = SseTransport::new("https://1.1.1.1/sse".to_string(), Some("key".to_string()));
        assert!(r.is_ok(), "public URL should pass SSRF check");
        let transport = r.unwrap();
        assert_eq!(transport.base_url, "https://1.1.1.1/sse");
        assert_eq!(transport.api_key.as_deref(), Some("key"));
        // listener_handle 应为 None(未 start)。
        assert!(transport.listener_handle.is_none());
        // post_url 初始为空。
        let rt = tokio::runtime::Runtime::new().unwrap();
        let post_url = rt.block_on(transport.post_url.read()).clone();
        assert!(post_url.is_empty());
    }

    /// T-E-S-31: SseTransport::new 接受公网 URL + None api_key。
    #[test]
    fn sse_transport_accepts_public_url_no_api_key() {
        let r = SseTransport::new("https://1.1.1.1/sse".to_string(), None);
        assert!(r.is_ok());
    }
}
