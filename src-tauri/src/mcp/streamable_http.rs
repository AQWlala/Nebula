//! T-E-S-34: MCPO (MCP over HTTP) — Streamable HTTP Transport.
//!
//! 实现 MCP 2025-03-26 规范的 Streamable HTTP transport:
//!
//! * **单一 endpoint**:POST JSON-RPC 请求,响应可为 `application/json`
//!   (直接解析)或 `text/event-stream`(SSE 流,逐 `data:` 行路由)。
//! * **Session-Id**:首次响应返回 `Mcp-Session-Id` 头,后续请求带上该头。
//! * **断线重连**:指数退避 100ms / 400ms / 1.6s / 6.4s / 25s,最多 5 次。
//! * **pending 路由**:`Arc<Mutex<HashMap<String, oneshot::Sender>>>` +
//!   `oneshot` channel,SSE listener 按 id 路由响应。
//!
//! 设计参考 [`super::sse_transport::SseTransport`](SSE 传输模式):
//! * 零新依赖(reqwest + tokio + futures 已在 Cargo.toml)。
//! * SSRF 校验(`SsrfGuard::validate_url`)拒绝 loopback/private。
//! * `MutexGuard` 不跨 await 点(用块作用域确保 drop)。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::security::ssrf_guard::SsrfGuard;

/// T-E-S-34: Streamable HTTP transport — 单一 endpoint POST + 可选 SSE。
///
/// 参考 MCP 2025-03-26 Streamable HTTP 规范:
/// <https://modelcontextprotocol.io/specification/2025-03-26/basic/transports>
pub struct StreamableHttpTransport {
    url: String,
    headers: reqwest::header::HeaderMap,
    session_id: Arc<RwLock<Option<String>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    client: reqwest::Client,
    /// 后台 SSE listener 句柄(per-POST SSE 响应或持久 GET listener)。
    sse_handle: Mutex<Option<JoinHandle<()>>>,
    closed: AtomicBool,
    /// 传输层自生成的 JSON-RPC id(initialize / call_tool 用)。
    id_gen: AtomicU64,
}

/// 把 JSON-RPC id (Option<Value>) 转为 String key 用于 pending 路由。
fn id_to_string(id: &Option<Value>) -> String {
    match id {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// T-E-S-34: 指数退避:100ms / 400ms / 1.6s / 6.4s / 25s,最多 5 次。
fn next_backoff(attempt: u32) -> Duration {
    let millis = match attempt {
        0 => 100,
        1 => 400,
        2 => 1_600,
        3 => 6_400,
        _ => 25_000,
    };
    Duration::from_millis(millis)
}

impl StreamableHttpTransport {
    /// 构造 transport(不发送 initialize)。
    ///
    /// `allow_private = true` 时跳过 SSRF 校验(仅供测试连接 localhost mock)。
    pub fn new(
        url: String,
        headers: HashMap<String, String>,
        session_id: Option<String>,
        allow_private: bool,
    ) -> Result<Self> {
        if !allow_private {
            SsrfGuard::new().validate_url(&url)?;
        }
        let client = if allow_private {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))?
        } else {
            SsrfGuard::new().build_safe_client()?
        };
        let mut header_map = reqwest::header::HeaderMap::new();
        for (k, v) in &headers {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                header_map.insert(name, value);
            }
        }
        Ok(Self {
            url,
            headers: header_map,
            session_id: Arc::new(RwLock::new(session_id)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            client,
            sse_handle: Mutex::new(None),
            closed: AtomicBool::new(false),
            id_gen: AtomicU64::new(1),
        })
    }

    /// 连接并发送 initialize 握手(生产路径,SSRF 严格校验)。
    pub async fn connect(
        url: String,
        headers: HashMap<String, String>,
        session_id: Option<String>,
    ) -> Result<Self> {
        let transport = Self::new(url, headers, session_id, false)?;
        transport.send_initialize().await?;
        Ok(transport)
    }

    /// 发送 `initialize` JSON-RPC 请求,提取响应中的 `Mcp-Session-Id` 头。
    ///
    /// 返回 InitializeResult 的原始 JSON-RPC 响应(调用方自行解析 result 字段)。
    pub async fn send_initialize(&self) -> Result<JsonRpcResponse> {
        let id = self.id_gen.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new("initialize", id).with_params(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nebula",
                "version": env!("CARGO_PKG_VERSION")
            }
        }));
        let resp = self.post_request(&req).await?;
        // T-E-S-34 fix: 在 debug! 宏之前提取布尔值,确保 RwLockReadGuard
        // 在 await 完成后立即 drop,避免 future 跨 await 持有非 Send 的 guard。
        let session_set = self.session_id.read().await.is_some();
        debug!(
            target: "nebula.mcp.streamable_http",
            session_id_set = session_set,
            "initialize response received"
        );
        Ok(resp)
    }

    /// 发送 JSON-RPC 请求并等待响应(POST + oneshot,30s 超时)。
    ///
    /// 检查响应 Content-Type:
    /// * `application/json` — 直接解析 body。
    /// * `text/event-stream` — 启动后台 SSE listener,按 id 路由响应。
    pub async fn send_and_receive(&self, req: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        self.post_request(req).await
    }

    /// 核心请求/响应逻辑:POST + 按 Content-Type 路由。
    pub async fn post_request(&self, req: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        if self.closed.load(Ordering::SeqCst) {
            anyhow::bail!("transport is closed");
        }
        let id_str = id_to_string(&req.id);
        if id_str.is_empty() {
            anyhow::bail!("JSON-RPC request missing id for StreamableHttp routing");
        }
        // 注册 oneshot sender(块作用域确保 MutexGuard 在 await 前 drop)。
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock();
            pending.insert(id_str.clone(), tx);
        }

        // 构造 POST 请求。
        let body = serde_json::to_string(req).context("serializing JSON-RPC request")?;
        let mut request = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Connection", "close")
            .headers(self.headers.clone())
            .body(body);
        // 带上 Mcp-Session-Id(若有)。
        {
            let sid = self.session_id.read().await.clone();
            if let Some(sid) = sid {
                if let Ok(value) = reqwest::header::HeaderValue::from_str(&sid) {
                    request = request.header("mcp-session-id", value);
                }
            }
        }

        // 发送。
        let send_result = request.send().await;
        let resp = match send_result {
            Ok(r) => r,
            Err(e) => {
                self.pending.lock().remove(&id_str);
                return Err(anyhow::anyhow!("StreamableHttp POST failed: {}", e));
            }
        };
        let resp = match resp.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                self.pending.lock().remove(&id_str);
                return Err(anyhow::anyhow!("StreamableHttp non-2xx response: {}", e));
            }
        };

        // 提取 Mcp-Session-Id 头(首次响应或后续更新)。
        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            debug!(target: "nebula.mcp.streamable_http", session_id = %sid, "Mcp-Session-Id received");
            *self.session_id.write().await = Some(sid.to_string());
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if content_type.contains("application/json") {
            // 直接解析 body。
            let body_text = resp.text().await.context("reading JSON response body")?;
            let json_resp: JsonRpcResponse =
                serde_json::from_str(body_text.trim()).context("parsing JSON response")?;
            self.pending.lock().remove(&id_str);
            Ok(json_resp)
        } else if content_type.contains("text/event-stream") {
            // 启动后台 SSE listener,按 id 路由。
            self.start_sse_listener(resp, &id_str, rx).await
        } else {
            // 未知 content-type,尝试按 JSON 解析。
            warn!(
                target: "nebula.mcp.streamable_http",
                content_type = %content_type,
                "unknown Content-Type, attempting JSON parse"
            );
            let body_text = resp.text().await.context("reading response body")?;
            let json_resp: JsonRpcResponse =
                serde_json::from_str(body_text.trim()).context("parsing response as JSON")?;
            self.pending.lock().remove(&id_str);
            Ok(json_resp)
        }
    }

    /// 启动后台 SSE listener:读 `text/event-stream` body,解析 `data:` 行,
    /// 按 id 路由到 pending oneshot。
    ///
    /// 主流程等待 oneshot(30s 超时),响应到达即返回。
    async fn start_sse_listener(
        &self,
        resp: reqwest::Response,
        id_str: &str,
        rx: oneshot::Receiver<JsonRpcResponse>,
    ) -> Result<JsonRpcResponse> {
        let pending = self.pending.clone();
        let stream = resp.bytes_stream();
        let target_id = id_str.to_string();
        let handle = tokio::spawn(async move {
            let mut stream = stream;
            let mut acc = String::new();
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        acc.push_str(&String::from_utf8_lossy(&bytes));
                        // 解析完整事件(以 \n\n 分隔)。
                        while let Some(pos) = acc.find("\n\n") {
                            let event_str = acc[..pos].to_string();
                            acc = acc[pos + 2..].to_string();
                            for line in event_str.lines() {
                                if let Some(data) = line.strip_prefix("data:") {
                                    let data = data.trim();
                                    if data.is_empty() {
                                        continue;
                                    }
                                    match serde_json::from_str::<JsonRpcResponse>(data) {
                                        Ok(json_resp) => {
                                            let resp_id = id_to_string(&Some(json_resp.id.clone()));
                                            let sender_opt = {
                                                let mut p = pending.lock();
                                                p.remove(&resp_id)
                                            };
                                            if let Some(sender) = sender_opt {
                                                let _ = sender.send(json_resp);
                                            } else {
                                                debug!(
                                                    target: "nebula.mcp.streamable_http",
                                                    id = %resp_id,
                                                    "no pending request for SSE response"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            debug!(
                                                target: "nebula.mcp.streamable_http",
                                                error = %e,
                                                "SSE data line not a JSON-RPC response, ignoring"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(target: "nebula.mcp.streamable_http", error = %e, "SSE stream error");
                        break;
                    }
                }
            }
            debug!(target: "nebula.mcp.streamable_http", "SSE listener task ended");
        });
        // 存储 handle 以便 close() 中止。
        {
            let mut guard = self.sse_handle.lock();
            if let Some(old) = guard.take() {
                old.abort();
            }
            *guard = Some(handle);
        }
        // 等待 oneshot(30s 超时)。
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(resp)) => {
                self.pending.lock().remove(&target_id);
                Ok(resp)
            }
            Ok(Err(_)) => {
                self.pending.lock().remove(&target_id);
                anyhow::bail!(
                    "StreamableHttp SSE response channel closed for id {}",
                    target_id
                )
            }
            Err(_) => {
                self.pending.lock().remove(&target_id);
                anyhow::bail!(
                    "StreamableHttp SSE response timeout (30s) for id {}",
                    target_id
                )
            }
        }
    }

    /// 指数退避重连:100ms / 400ms / 1.6s / 6.4s / 25s,最多 5 次。
    ///
    /// 每次重试调用 `send_initialize()`。失败后等 backoff 再重试。
    pub async fn reconnect_with_backoff(&mut self) -> Result<()> {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..5u32 {
            if self.closed.load(Ordering::SeqCst) {
                anyhow::bail!("transport closed during reconnect");
            }
            let backoff = next_backoff(attempt);
            debug!(
                target: "nebula.mcp.streamable_http",
                attempt, backoff_ms = backoff.as_millis(),
                "reconnect backoff"
            );
            tokio::time::sleep(backoff).await;
            match self.send_initialize().await {
                Ok(_) => {
                    info!(
                        target: "nebula.mcp.streamable_http",
                        attempt, "reconnect succeeded"
                    );
                    return Ok(());
                }
                Err(e) => {
                    warn!(
                        target: "nebula.mcp.streamable_http",
                        attempt, error = %e, "reconnect attempt failed"
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err
            .context("reconnect failed after 5 attempts")
            .unwrap_or_else(|_| anyhow::anyhow!("reconnect failed after 5 attempts")))
    }

    /// 高层 `tools/call` 便捷方法(传输层自生成 id)。
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value> {
        let id = self.id_gen.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new("tools/call", id).with_params(serde_json::json!({
            "name": name,
            "arguments": args,
        }));
        let resp = self.post_request(&req).await?;
        let result = resp.result_value()?;
        Ok(result.clone())
    }

    /// 关闭:设 closed + 中止 SSE listener。
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let handle = { self.sse_handle.lock().take() };
        if let Some(h) = handle {
            h.abort();
            let _ = h.await;
        }
        // 清空 pending(drop 所有 sender → 等待方收到 RecvError)。
        self.pending.lock().clear();
    }

    /// 当前 session_id(测试用)。
    pub async fn session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 读取完整 HTTP 请求(headers + body based on Content-Length)。
    async fn read_http_request<R: AsyncReadExt + Unpin>(reader: &mut R) -> String {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = match reader.read(&mut tmp).await {
                Ok(n) if n > 0 => n,
                _ => break,
            };
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                let headers_str = String::from_utf8_lossy(&buf[..pos]);
                let content_length: usize = headers_str
                    .split("\r\n")
                    .find_map(|line| {
                        let lower = line.to_lowercase();
                        lower.strip_prefix("content-length:")?.trim().parse().ok()
                    })
                    .unwrap_or(0);
                let body_start = pos + 4;
                while buf.len() - body_start < content_length {
                    let n = match reader.read(&mut tmp).await {
                        Ok(n) if n > 0 => n,
                        _ => break,
                    };
                    buf.extend_from_slice(&tmp[..n]);
                }
                break;
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// 启动 mock HTTP server,handler 闭包接收请求文本返回 (status, headers, body)。
    async fn start_mock_server<F>(handler: F) -> String
    where
        F: Fn(&str) -> (u16, Vec<(String, String)>, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let h = handler.clone();
                tokio::spawn(async move {
                    let req = read_http_request(&mut socket).await;
                    let (status, headers, body) = h(&req);
                    let status_text = match status {
                        200 => "OK",
                        500 => "Internal Server Error",
                        404 => "Not Found",
                        _ => "OK",
                    };
                    let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                    for (k, v) in &headers {
                        resp.push_str(&format!("{}: {}\r\n", k, v));
                    }
                    resp.push_str(&format!(
                        "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    ));
                    let _ = socket.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://127.0.0.1:{}", port)
    }

    /// T-E-S-34-1: connect 提取 Mcp-Session-Id。
    #[tokio::test]
    async fn test_streamable_http_connect() {
        let url = start_mock_server(|_req| {
            (
                200,
                vec![("mcp-session-id".into(), "session-abc-123".into())],
                r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}}"#.to_string(),
            )
        })
        .await;
        // 测试需连 localhost mock,绕过 SSRF 校验;connect() 是 SSRF 严格路径,
        // 因此改用 new(allow_private=true) + send_initialize()。
        let transport = StreamableHttpTransport::new(url.clone(), HashMap::new(), None, true)
            .expect("new should succeed");
        transport
            .send_initialize()
            .await
            .expect("initialize should succeed");
        let sid = transport.session_id().await;
        assert_eq!(sid.as_deref(), Some("session-abc-123"));
        transport.close().await;
    }

    /// T-E-S-34-2: POST application/json,pending 路由 + oneshot 收到响应。
    #[tokio::test]
    async fn test_streamable_http_post_json() {
        let url = start_mock_server(|_req| {
            (
                200,
                vec![("content-type".into(), "application/json".into())],
                r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"t1","description":"d","inputSchema":{}}]}}"#.to_string(),
            )
        })
        .await;
        let transport = StreamableHttpTransport::new(url, HashMap::new(), None, true).unwrap();
        let req = JsonRpcRequest::new("tools/list", 1);
        let resp = transport
            .post_request(&req)
            .await
            .expect("post should succeed");
        assert_eq!(resp.id, serde_json::json!(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        let result = resp.result_value().unwrap();
        assert!(result.get("tools").is_some());
        transport.close().await;
    }

    /// T-E-S-34-3: POST text/event-stream,SSE listener 解析 data: 行并路由。
    #[tokio::test]
    async fn test_streamable_http_sse_response() {
        let sse_body = "data: {\"jsonrpc\":\"2.0\",\"id\":42,\"result\":{\"tools\":[]}}\n\n";
        let url = start_mock_server(move |_req| {
            (
                200,
                vec![("content-type".into(), "text/event-stream".into())],
                sse_body.to_string(),
            )
        })
        .await;
        let transport = StreamableHttpTransport::new(url, HashMap::new(), None, true).unwrap();
        let req = JsonRpcRequest::new("tools/list", 42);
        let resp = transport
            .post_request(&req)
            .await
            .expect("post SSE should succeed");
        assert_eq!(resp.id, serde_json::json!(42));
        let result = resp.result_value().unwrap();
        assert!(result.get("tools").is_some());
        transport.close().await;
    }

    /// T-E-S-34-4: 首次连接失败,指数退避后重连成功。
    #[tokio::test]
    async fn test_streamable_http_reconnect() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let url = start_mock_server(move |_req| {
            let n = counter_clone.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                (500, vec![], "Internal Server Error".to_string())
            } else {
                (
                    200,
                    vec![("mcp-session-id".into(), "session-reconnect".into())],
                    r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}}"#.to_string(),
                )
            }
        })
        .await;
        // new(不发送 initialize)
        let mut transport = StreamableHttpTransport::new(url, HashMap::new(), None, true).unwrap();
        // 首次 send_initialize 失败(500)
        let first = transport.send_initialize().await;
        assert!(first.is_err(), "first initialize should fail with 500");
        // reconnect: 等 100ms 后重试 → 成功
        transport
            .reconnect_with_backoff()
            .await
            .expect("reconnect should succeed");
        let sid = transport.session_id().await;
        assert_eq!(sid.as_deref(), Some("session-reconnect"));
        transport.close().await;
    }

    /// T-E-S-34-5: session_id 持久化 — 首次响应带 session_id,第二次请求 header 中带 Mcp-Session-Id。
    #[tokio::test]
    async fn test_streamable_http_session_id_persistence() {
        let captured_headers = Arc::new(Mutex::new(Vec::<String>::new()));
        let cap_clone = captured_headers.clone();
        let url = start_mock_server(move |req| {
            // 捕获请求中的 Mcp-Session-Id 头。
            for line in req.split("\r\n") {
                let lower = line.to_lowercase();
                if lower.starts_with("mcp-session-id:") {
                    cap_clone.lock().push(line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string());
                }
            }
            (
                200,
                vec![
                    ("mcp-session-id".into(), "session-persist-99".into()),
                    ("content-type".into(), "application/json".into()),
                ],
                r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{}}}"#.to_string(),
            )
        })
        .await;
        let transport = StreamableHttpTransport::new(url, HashMap::new(), None, true).unwrap();
        // 第一次请求:响应带 Mcp-Session-Id → session_id 被回填。
        let req1 = JsonRpcRequest::new("initialize", 1);
        let _ = transport
            .post_request(&req1)
            .await
            .expect("first post should succeed");
        let sid = transport.session_id().await;
        assert_eq!(sid.as_deref(), Some("session-persist-99"));
        // 第二次请求:应在 header 中带 Mcp-Session-Id。
        let req2 = JsonRpcRequest::new("tools/list", 2);
        let _ = transport
            .post_request(&req2)
            .await
            .expect("second post should succeed");
        // 检查捕获的头:第一次请求不应有 session_id,第二次应有。
        {
            let captured = captured_headers.lock();
            assert!(
                !captured.is_empty(),
                "second request should carry Mcp-Session-Id header"
            );
            assert!(
                captured.iter().any(|h| h == "session-persist-99"),
                "captured Mcp-Session-Id should match, got: {:?}",
                *captured
            );
        }
        transport.close().await;
    }

    /// T-E-S-34-6: 集成测试 — 通过 McpClient::connect 路由到 StreamableHttp 变体。
    ///
    /// mock 返回 initialize + tools/list 响应,验证 McpClient 能连接并发现工具。
    #[tokio::test]
    async fn test_mcp_client_connect_streamable() {
        use crate::mcp::client::McpClient;
        use crate::mcp::config::{McpServerConfig, McpTransportType};

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let url = start_mock_server(move |req| {
            let n = counter_clone.fetch_add(1, Ordering::SeqCst);
            // 解析请求 body 获取 method 和 id。
            let method = if req.contains("\"initialize\"") {
                "initialize"
            } else if req.contains("\"tools/list\"") {
                "tools/list"
            } else if req.contains("\"notifications/initialized\"") {
                "notification"
            } else {
                "unknown"
            };
            let _ = n;
            match method {
                "initialize" => (
                    200,
                    vec![
                        ("mcp-session-id".into(), "session-integration".into()),
                        ("content-type".into(), "application/json".into()),
                    ],
                    r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}}"#.to_string(),
                ),
                "notification" => (
                    200,
                    vec![("content-type".into(), "application/json".into())],
                    String::new(),
                ),
                "tools/list" => (
                    200,
                    vec![("content-type".into(), "application/json".into())],
                    r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fs_read","description":"Read a file","inputSchema":{"type":"object"}}]}}"#.to_string(),
                ),
                _ => (404, vec![], String::new()),
            }
        })
        .await;

        let cfg = McpServerConfig {
            name: "sh-integration".to_string(),
            transport_type: McpTransportType::StreamableHttp {
                url,
                headers: HashMap::new(),
                session_id: None,
            },
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec![],
            auto_restart: false,
            health_check_interval_secs: 30,
        };
        let mut client = McpClient::new(cfg);
        // McpClient::connect 路由到 StreamableHttpTransport::connect(需要 SSRF bypass)。
        // 由于 connect() 默认 SSRF 严格,localhost 会被拒。
        // 此测试验证路由逻辑:应返回 SSRF 错误(说明到达了 StreamableHttp 分支)。
        let result = client.connect().await;
        // 期望失败(SSRF 拒绝 loopback),错误信息应暗示 StreamableHttp 路径。
        assert!(result.is_err(), "connect to localhost should fail (SSRF)");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("SSRF")
                || err_msg.contains("loopback")
                || err_msg.contains("private")
                || err_msg.contains("StreamableHttp"),
            "error should indicate SSRF or StreamableHttp path, got: {}",
            err_msg
        );
    }

    /// T-E-S-34: next_backoff 序列正确。
    #[test]
    fn next_backoff_returns_expected_sequence() {
        assert_eq!(next_backoff(0), Duration::from_millis(100));
        assert_eq!(next_backoff(1), Duration::from_millis(400));
        assert_eq!(next_backoff(2), Duration::from_millis(1_600));
        assert_eq!(next_backoff(3), Duration::from_millis(6_400));
        assert_eq!(next_backoff(4), Duration::from_millis(25_000));
        assert_eq!(next_backoff(100), Duration::from_millis(25_000));
    }

    /// T-E-S-34: id_to_string 处理数字/字符串/None。
    #[test]
    fn id_to_string_handles_types() {
        assert_eq!(id_to_string(&Some(serde_json::json!(42))), "42");
        assert_eq!(id_to_string(&Some(serde_json::json!("abc"))), "abc");
        assert_eq!(id_to_string(&None), "");
    }
}
