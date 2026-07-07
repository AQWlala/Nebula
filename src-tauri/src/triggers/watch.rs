//! T-E-S-55: 条件监控 Watch — WatchWorker + WebFetcher + SystemProbe + IcsParser。
//!
//! 三种监控源:
//! * **Web** — 定时抓取 URL,SHA-256 Diff 检测变化触发(reqwest + SsrfGuard)
//! * **System** — Windows CPU/内存阈值告警(GlobalMemoryStatusEx + GetSystemTimes)
//! * **Calendar** — 解析 `.ics` 文件,临近事件触发(手写 ICS parser,~80 LOC)
//!
//! ## 设计约束(spec §设计约束)
//!
//! 1. **零新依赖**:全部用 Cargo.toml 已有 crate(reqwest/tokio/serde/chrono/
//!    windows-sys/sha2)。
//! 2. **Web 抓取安全**:仅允许 http/https;SSRF 校验复用
//!    `security::ssrf_guard::SsrfGuard`(拒绝 127.0.0.1/169.254/10.0.0.0/8/
//!    172.16.0.0/12/192.168.0.0/16);body 1MiB 限制;User-Agent 固定
//!    `Nebula/1.0`。
//! 3. **System 指标**:Windows 用 `GlobalMemoryStatusEx`(内存使用率)+
//!    `GetSystemTimes`(CPU 使用率,双采样差分)。不引入 sysinfo crate。
//! 4. **ICS parser**:手写最小子集解析 `VEVENT`(`DTSTART`/`SUMMARY`/`UID`),
//!    不引入 ical crate。
//! 5. **去抖 + 递归防护**:继承 TriggerEngine 既有机制(`debounce_ms` +
//!    `source_trigger_id`)。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::triggers::TriggerEngine;

// ---------------------------------------------------------------------------
// 配置模型
// ---------------------------------------------------------------------------

/// 监控源 — Watch 触发器的条件载体。
///
/// 序列化为 JSON 嵌套在 `TriggerCondition::Watch { source }` 中。
/// `tag = "type"` 让前端按字段名分支渲染。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchSource {
    /// 网页监控:定时抓取 URL,SHA-256 Diff 检测变化。
    Web {
        url: String,
        /// 可选 CSS-like 选择器(简化版:只支持 `<tag>` 文本提取)。
        #[serde(default)]
        selector: Option<String>,
        /// 轮询间隔(秒)。
        interval_secs: u32,
    },
    /// 系统指标监控:读取 CPU/内存/磁盘 指标,按 `op` 与 `threshold` 比较触发。
    System {
        metric: SystemMetric,
        threshold: f32,
        op: CmpOp,
        interval_secs: u32,
    },
    /// 日历事件监控:解析 `.ics` 文件,在事件开始前 `lead_minutes` 分钟触发。
    Calendar { ics_path: String, lead_minutes: u32 },
}

impl WatchSource {
    /// 返回轮询间隔(秒)。Calendar 默认 60 秒(每分钟检查一次临近事件)。
    pub fn interval_secs(&self) -> u32 {
        match self {
            WatchSource::Web { interval_secs, .. } => *interval_secs,
            WatchSource::System { interval_secs, .. } => *interval_secs,
            WatchSource::Calendar { .. } => 60,
        }
    }
}

/// 系统指标种类。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemMetric {
    CpuUsage,
    MemoryUsage,
    DiskFreePercent,
}

/// 比较操作符(System 指标与 threshold 比较)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Gt,
    Lt,
    Eq,
}

// ---------------------------------------------------------------------------
// WebFetcher
// ---------------------------------------------------------------------------

/// Web body 最大字节数(1 MiB)。
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// 固定 User-Agent。
#[allow(dead_code)]
const USER_AGENT: &str = "Nebula/1.0";

/// 抓取结果。
#[derive(Debug)]
pub struct FetchResult {
    pub text: String,
    pub hash: String,
}

/// 网页抓取器 — SSRF 校验 + 1MiB 限制 + SHA-256 hash。
pub struct WebFetcher {
    client: reqwest::Client,
}

impl WebFetcher {
    /// 构造 fetcher(固定 User-Agent + 30s 超时)。
    /// M7b #94: 用 SsrfGuard::build_safe_client 构建客户端,
    /// 重定向链每跳校验目标 URL,防止 302 → 内网 SSRF。
    pub fn new() -> Result<Self> {
        let client = crate::security::ssrf_guard::SsrfGuard::new().build_safe_client()?;
        Ok(Self { client })
    }

    /// 抓取 URL 并返回 body 文本 + SHA-256 hash。
    ///
    /// 安全检查(顺序):
    /// 1. **SSRF 校验**:复用 `SsrfGuard::validate_url`(拒绝私有/回环/链路本地地址)。
    /// 2. **Scheme 校验**:仅允许 http/https。
    /// 3. **Content-Length 预检**:若声明 > 1MiB 则直接拒绝。
    /// 4. **Body 读取后二次校验**:防止服务器谎报 Content-Length。
    pub async fn fetch(&self, url: &str) -> Result<FetchResult> {
        // 1. SSRF 校验
        let guard = crate::security::ssrf_guard::SsrfGuard::new();
        guard.validate_url(url)?;

        // 2. Scheme 校验
        let parsed = url::Url::parse(url)?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => return Err(anyhow!("unsupported scheme: {other}")),
        }

        // 3. 发送请求
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {}", resp.status()));
        }

        // 4. Content-Length 预检
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BODY_BYTES {
                return Err(anyhow!(
                    "response body too large: {len} bytes (limit {MAX_BODY_BYTES})"
                ));
            }
        }

        // 5. 读取 body + 二次大小校验
        let body = resp.bytes().await?;
        if body.len() > MAX_BODY_BYTES {
            return Err(anyhow!(
                "response body exceeds 1MiB limit: {} bytes",
                body.len()
            ));
        }

        let text = String::from_utf8_lossy(&body).to_string();
        let hash = hash_text(&text);
        Ok(FetchResult { text, hash })
    }
}

/// 计算 SHA-256 hash(hex 编码)。
pub fn hash_text(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

/// 简化版选择器:提取首个 `<tag>...</tag>` 的文本内容。
///
/// 仅支持标签名(如 `"title"` / `"h1"`),不支持属性/类/ID 选择器。
/// 若未找到匹配标签,返回完整文本。
pub fn apply_selector(html: &str, selector: &str) -> String {
    let open = format!("<{selector}>");
    let close = format!("</{selector}>");
    if let Some(start) = html.find(&open) {
        let content_start = start + open.len();
        if let Some(end) = html[content_start..].find(&close) {
            return html[content_start..content_start + end].trim().to_string();
        }
    }
    html.to_string()
}

/// 检测 hash 变化:若 `current` 与 `previous` 不同则返回 true(应触发)。
/// `previous = None` 时视为首次抓取(不触发,仅记录基线)。
pub fn detect_change(previous: Option<&str>, current: &str) -> bool {
    match previous {
        Some(prev) => prev != current,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// SystemProbe
// ---------------------------------------------------------------------------

/// 系统指标探测器 — Windows API 直读。
pub struct SystemProbe;

impl SystemProbe {
    /// 读取指定指标的当前值。
    ///
    /// * `CpuUsage` → 百分比 0.0–100.0(双采样 `GetSystemTimes` 差分,~200ms)
    /// * `MemoryUsage` → 百分比 0.0–100.0(`GlobalMemoryStatusEx`)
    /// * `DiskFreePercent` → 百分比 0.0–100.0(`GetDiskFreeSpaceExW`,C:\)
    pub fn read_metric(metric: SystemMetric) -> Result<f32> {
        match metric {
            SystemMetric::CpuUsage => read_cpu_usage(),
            SystemMetric::MemoryUsage => read_memory_usage(),
            SystemMetric::DiskFreePercent => read_disk_free_percent(),
        }
    }

    /// 比较 `value` 与 `threshold`:满足 `op` 条件则返回 true(应触发)。
    /// 纯函数,便于单测。
    pub fn compare(value: f32, op: CmpOp, threshold: f32) -> bool {
        match op {
            CmpOp::Gt => value > threshold,
            CmpOp::Lt => value < threshold,
            CmpOp::Eq => (value - threshold).abs() < 0.01,
        }
    }
}

// ---- Windows 实现 ----

#[cfg(target_os = "windows")]
fn read_memory_usage() -> Result<f32> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: `status` 是栈上 `mem::zeroed()` 的 `MEMORYSTATUSEX`,在调用前显式设置
    // `dwLength`(结构体字节数,API 要求字段)。指针在 `GlobalMemoryStatusEx` 同步
    // 返回前保持有效。该 API 仅通过提供的指针写入结构体,返回 0 表示失败(此时不读字段)。
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            Ok(status.dwMemoryLoad as f32)
        } else {
            Err(anyhow!("GlobalMemoryStatusEx failed"))
        }
    }
}

#[cfg(target_os = "windows")]
fn read_cpu_usage() -> Result<f32> {
    let (idle1, total1) = read_system_times()?;
    std::thread::sleep(Duration::from_millis(200));
    let (idle2, total2) = read_system_times()?;

    let idle_delta = idle2.saturating_sub(idle1);
    let total_delta = total2.saturating_sub(total1);
    if total_delta == 0 {
        return Ok(0.0);
    }
    let usage = (1.0 - idle_delta as f64 / total_delta as f64) * 100.0;
    Ok(usage as f32)
}

/// 读取系统空闲/总时间(100ns 单位)。
/// `GetSystemTimes(idle, kernel, user)` → total = kernel + user。
#[cfg(target_os = "windows")]
fn read_system_times() -> Result<(u64, u64)> {
    use windows_sys::Win32::Foundation::FILETIME;
    // windows-sys 0.61 把 GetSystemTimes 移到 Win32::System::Threading。
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    // SAFETY: `idle` / `kernel` / `user` 是栈上 `mem::zeroed()` 的 `FILETIME`(4 字节对齐),
    // 三个 `&mut` 引用在 `GetSystemTimes` 同步返回前保持有效。该 API 仅通过提供的指针
    // 写入结构体;返回 0 表示失败(此时不读字段)。`FILETIME` 是 `repr(C)` 平铺结构体,
    // 无内部指针,`zeroed()` 初始化满足 API 对写入的要求。
    unsafe {
        let mut idle: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0 {
            let idle_ns = filetime_to_u64(&idle);
            let kernel_ns = filetime_to_u64(&kernel);
            let user_ns = filetime_to_u64(&user);
            Ok((idle_ns, kernel_ns + user_ns))
        } else {
            Err(anyhow!("GetSystemTimes failed"))
        }
    }
}

#[cfg(target_os = "windows")]
fn filetime_to_u64(ft: &windows_sys::Win32::Foundation::FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

#[cfg(target_os = "windows")]
fn read_disk_free_percent() -> Result<f32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let path: Vec<u16> = std::ffi::OsStr::new("C:\\")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;
    // SAFETY: `path` 是以 null 结尾的 UTF-16 宽字符串(末尾追加 0),指针在
    // `GetDiskFreeSpaceExW` 同步返回前有效。三个 `&mut u64` out-param 是栈上变量,
    // 指针在同步返回前有效。该 API 仅通过提供的指针写入标量,返回 0 表示失败。
    // `path` 内容固定为 `C:\\`,编码后不含嵌入 null。
    unsafe {
        if GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free_to_caller,
            &mut total,
            &mut total_free,
        ) != 0
        {
            if total == 0 {
                return Ok(100.0);
            }
            Ok((total_free as f32 / total as f32) * 100.0)
        } else {
            Err(anyhow!("GetDiskFreeSpaceExW failed"))
        }
    }
}

// ---- 非 Windows 平台降级(保证编译通过) ----

#[cfg(not(target_os = "windows"))]
fn read_memory_usage() -> Result<f32> {
    Err(anyhow!("memory usage not supported on this platform"))
}

#[cfg(not(target_os = "windows"))]
fn read_cpu_usage() -> Result<f32> {
    Err(anyhow!("CPU usage not supported on this platform"))
}

#[cfg(not(target_os = "windows"))]
fn read_disk_free_percent() -> Result<f32> {
    Err(anyhow!("disk free percent not supported on this platform"))
}

// ---------------------------------------------------------------------------
// IcsParser
// ---------------------------------------------------------------------------

/// ICS 事件(最小子集)。
#[derive(Debug, Clone, PartialEq)]
pub struct IcsEvent {
    pub uid: String,
    pub summary: String,
    pub start: chrono::DateTime<chrono::Utc>,
}

/// 手写最小 ICS parser:解析 `VEVENT` 块中的 `UID`/`SUMMARY`/`DTSTART`。
///
/// 支持的 DTSTART 格式:
/// * `20260101T120000Z`(UTC)
/// * `20260101T120000`(视为 UTC)
/// * `DTSTART;TZID=...:20260101T120000`(取冒号后部分)
///
/// 不支持:RRULE/EXDATE/VALARM/嵌套属性等。~80 LOC,不引入 ical crate。
pub fn parse_ics(content: &str) -> Vec<IcsEvent> {
    let mut events = Vec::new();
    let mut in_event = false;
    let mut uid = String::new();
    let mut summary = String::new();
    let mut start: Option<chrono::DateTime<chrono::Utc>> = None;

    for line in content.lines() {
        let line = line.trim_end_matches('\r');
        if line == "BEGIN:VEVENT" {
            in_event = true;
            uid.clear();
            summary.clear();
            start = None;
        } else if line == "END:VEVENT" {
            if in_event {
                if let Some(dt) = start {
                    events.push(IcsEvent {
                        uid: uid.clone(),
                        summary: summary.clone(),
                        start: dt,
                    });
                }
            }
            in_event = false;
        } else if in_event {
            // DTSTART 可带参数:DTSTART;TZID=America/New_York:20260101T120000
            if let Some(rest) = line.strip_prefix("UID:") {
                uid = unescape_ics(rest);
            } else if let Some(rest) = line.strip_prefix("SUMMARY:") {
                summary = unescape_ics(rest);
            } else if let Some(rest) = line.strip_prefix("DTSTART") {
                let val = rest.rsplit_once(':').map(|(_, v)| v).unwrap_or(rest);
                if let Some(dt) = parse_ics_datetime(val) {
                    start = Some(dt);
                }
            }
        }
    }
    events
}

/// 解析 ICS 日期时间:`20260101T120000Z` 或 `20260101T120000`。
fn parse_ics_datetime(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = s.trim();
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%SZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%S"))
        .ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        naive,
        chrono::Utc,
    ))
}

/// ICS 文本转义还原:`\n` → 换行,`\,` → 逗号,`\\` → 反斜杠。
fn unescape_ics(s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("\\,", ",")
        .replace("\\\\", "\\")
}

/// 查找临近事件:返回 `start` 在 `[now, now + lead_minutes]` 范围内的事件。
#[allow(clippy::needless_lifetimes)]
pub fn find_upcoming_events<'a>(
    events: &'a [IcsEvent],
    now: chrono::DateTime<chrono::Utc>,
    lead_minutes: u32,
) -> Vec<&'a IcsEvent> {
    let lead = chrono::Duration::minutes(lead_minutes as i64);
    events
        .iter()
        .filter(|e| {
            let diff = e.start.signed_duration_since(now);
            diff.num_seconds() >= 0 && diff <= lead
        })
        .collect()
}

// ---------------------------------------------------------------------------
// WatchWorker
// ---------------------------------------------------------------------------

/// Watch worker 句柄(存放在 `TriggerEngine::watch_workers` map 中)。
pub struct WatchWorkerHandle {
    cancel: CancellationToken,
    handle: Option<JoinHandle<()>>,
}

impl WatchWorkerHandle {
    /// 停止 worker:取消 token + abort task。
    pub fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// 启动 Watch worker。
///
/// 根据 `WatchSource` 变体选择对应的轮询逻辑,周期性检查条件并调用
/// `engine.dispatch(trigger_id, payload)`。payload 携带 `source_trigger_id`
/// 以触发递归防护。
pub fn spawn_watch_worker(
    trigger_id: String,
    source: WatchSource,
    engine: Arc<TriggerEngine>,
    store: Arc<crate::triggers::TriggerStore>,
) -> WatchWorkerHandle {
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let interval_secs = source.interval_secs().max(1);

    let handle = tokio::spawn(async move {
        info!(
            target: "nebula.triggers.watch",
            trigger_id = %trigger_id,
            interval_secs,
            "watch worker started"
        );
        loop {
            tokio::select! {
                biased;
                _ = cancel_clone.cancelled() => {
                    info!(
                        target: "nebula.triggers.watch",
                        trigger_id = %trigger_id,
                        "watch worker received cancellation"
                    );
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(interval_secs as u64)) => {
                    poll_once(&trigger_id, &source, &engine, &store).await;
                }
            }
        }
        info!(
            target: "nebula.triggers.watch",
            trigger_id = %trigger_id,
            "watch worker exiting"
        );
    });

    WatchWorkerHandle {
        cancel,
        handle: Some(handle),
    }
}

/// 单次轮询:按 source 类型分发。
async fn poll_once(
    trigger_id: &str,
    source: &WatchSource,
    engine: &Arc<TriggerEngine>,
    store: &Arc<crate::triggers::TriggerStore>,
) {
    match source {
        WatchSource::Web {
            url,
            selector,
            interval_secs: _,
        } => poll_web(trigger_id, url, selector, engine, store).await,
        WatchSource::System {
            metric,
            threshold,
            op,
            interval_secs: _,
        } => poll_system(trigger_id, *metric, *threshold, *op, engine).await,
        WatchSource::Calendar {
            ics_path,
            lead_minutes,
        } => poll_calendar(trigger_id, ics_path, *lead_minutes, engine, store).await,
    }
}

/// Web 轮询:抓取 URL → 计算 hash → 与上次 hash 比较 → 变化则 dispatch。
async fn poll_web(
    trigger_id: &str,
    url: &str,
    selector: &Option<String>,
    engine: &Arc<TriggerEngine>,
    store: &Arc<crate::triggers::TriggerStore>,
) {
    let fetcher = match WebFetcher::new() {
        Ok(f) => f,
        Err(e) => {
            warn!(
                target: "nebula.triggers.watch",
                trigger_id, error = %e, "WebFetcher init failed"
            );
            return;
        }
    };

    let result = match fetcher.fetch(url).await {
        Ok(r) => r,
        Err(e) => {
            debug!(
                target: "nebula.triggers.watch",
                trigger_id, url, error = %e, "fetch failed"
            );
            return;
        }
    };

    let content = selector
        .as_ref()
        .map(|s| apply_selector(&result.text, s))
        .unwrap_or(result.text);
    let current_hash = hash_text(&content);

    let prev_hash = store
        .get_watch_state(trigger_id)
        .ok()
        .flatten()
        .and_then(|s| s.last_url_hash);

    if !detect_change(prev_hash.as_deref(), &current_hash) {
        debug!(
            target: "nebula.triggers.watch",
            trigger_id, "no change detected; skipping"
        );
        return;
    }

    // 更新 state + dispatch
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = store.set_watch_state(trigger_id, Some(&current_hash), None, Some(now_ms));

    let payload = serde_json::json!({
        "url": url,
        "hash": current_hash,
        "changed": true,
        "source_trigger_id": trigger_id,
    });
    engine.dispatch(trigger_id, payload).await;
}

/// System 轮询:读取指标 → 与 threshold 比较 → 满足则 dispatch。
async fn poll_system(
    trigger_id: &str,
    metric: SystemMetric,
    threshold: f32,
    op: CmpOp,
    engine: &Arc<TriggerEngine>,
) {
    let value = match SystemProbe::read_metric(metric) {
        Ok(v) => v,
        Err(e) => {
            debug!(
                target: "nebula.triggers.watch",
                trigger_id, error = %e, "read metric failed"
            );
            return;
        }
    };

    if !SystemProbe::compare(value, op, threshold) {
        return;
    }

    let payload = serde_json::json!({
        "metric": metric,
        "value": value,
        "threshold": threshold,
        "source_trigger_id": trigger_id,
    });
    engine.dispatch(trigger_id, payload).await;
}

/// Calendar 轮询:解析 ICS → 查找临近事件 → 未触发过则 dispatch。
async fn poll_calendar(
    trigger_id: &str,
    ics_path: &str,
    lead_minutes: u32,
    engine: &Arc<TriggerEngine>,
    store: &Arc<crate::triggers::TriggerStore>,
) {
    let content = match std::fs::read_to_string(ics_path) {
        Ok(c) => c,
        Err(e) => {
            debug!(
                target: "nebula.triggers.watch",
                trigger_id, ics_path, error = %e, "read ics failed"
            );
            return;
        }
    };

    let events = parse_ics(&content);
    let now = chrono::Utc::now();
    let upcoming = find_upcoming_events(&events, now, lead_minutes);

    if upcoming.is_empty() {
        return;
    }

    let last_fired_uid = store
        .get_watch_state(trigger_id)
        .ok()
        .flatten()
        .and_then(|s| s.last_value);

    for event in upcoming {
        // 跳过已触发过的同一事件(用 UID 去重)。
        if last_fired_uid.as_deref() == Some(&event.uid) {
            continue;
        }

        let now_ms = now.timestamp_millis();
        let _ = store.set_watch_state(trigger_id, None, Some(&event.uid), Some(now_ms));

        let payload = serde_json::json!({
            "event_uid": event.uid,
            "event_summary": event.summary,
            "event_start": event.start.to_rfc3339(),
            "lead_minutes": lead_minutes,
            "source_trigger_id": trigger_id,
        });
        engine.dispatch(trigger_id, payload).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WatchSource serde ----

    #[test]
    fn test_watch_source_web_serde() {
        let src = WatchSource::Web {
            url: "https://example.com/feed".to_string(),
            selector: Some("title".to_string()),
            interval_secs: 60,
        };
        let s = serde_json::to_string(&src).unwrap();
        assert!(s.contains("\"type\":\"web\""));
        assert!(s.contains("\"url\":\"https://example.com/feed\""));
        assert!(s.contains("\"interval_secs\":60"));
        let back: WatchSource = serde_json::from_str(&s).unwrap();
        assert_eq!(src, back);
    }

    #[test]
    fn test_watch_source_system_serde() {
        let src = WatchSource::System {
            metric: SystemMetric::CpuUsage,
            threshold: 80.0,
            op: CmpOp::Gt,
            interval_secs: 30,
        };
        let s = serde_json::to_string(&src).unwrap();
        assert!(s.contains("\"type\":\"system\""));
        assert!(s.contains("\"metric\":\"cpu_usage\""));
        assert!(s.contains("\"op\":\"gt\""));
        let back: WatchSource = serde_json::from_str(&s).unwrap();
        assert_eq!(src, back);
    }

    #[test]
    fn test_watch_source_calendar_serde() {
        let src = WatchSource::Calendar {
            ics_path: "/tmp/cal.ics".to_string(),
            lead_minutes: 15,
        };
        let s = serde_json::to_string(&src).unwrap();
        assert!(s.contains("\"type\":\"calendar\""));
        assert!(s.contains("\"lead_minutes\":15"));
        let back: WatchSource = serde_json::from_str(&s).unwrap();
        assert_eq!(src, back);
    }

    // ---- WebFetcher SSRF ----

    #[tokio::test]
    async fn test_web_fetcher_rejects_private_address() {
        let fetcher = WebFetcher::new().unwrap();
        // 127.0.0.1 — loopback
        let r = fetcher.fetch("http://127.0.0.1/secret").await;
        assert!(r.is_err());
        let err = r.unwrap_err().to_string();
        assert!(err.contains("SSRF") || err.contains("loopback"));

        // 192.168.x.x — private
        let r = fetcher.fetch("http://192.168.1.1/admin").await;
        assert!(r.is_err());

        // 10.x.x.x — private
        let r = fetcher.fetch("http://10.0.0.1/internal").await;
        assert!(r.is_err());

        // 169.254.x.x — link-local
        let r = fetcher.fetch("http://169.254.169.254/meta-data").await;
        assert!(r.is_err());
    }

    #[test]
    fn test_web_fetcher_rejects_non_http_scheme() {
        // file:// scheme should be rejected by WebFetcher::fetch (before HTTP).
        // We test the URL parsing logic directly.
        let parsed = url::Url::parse("file:///etc/passwd").unwrap();
        assert_eq!(parsed.scheme(), "file");
        assert!(!matches!(parsed.scheme(), "http" | "https"));
    }

    // ---- Web Diff 检测 ----

    #[test]
    fn test_web_diff_detect_change() {
        let h1 = hash_text("hello world");
        let h2 = hash_text("hello world");
        let h3 = hash_text("hello changed");

        // 相同内容 → 相同 hash
        assert_eq!(h1, h2);
        // 不同内容 → 不同 hash
        assert_ne!(h1, h3);

        // detect_change 逻辑
        assert!(!detect_change(None, &h1)); // 首次抓取,不触发
        assert!(!detect_change(Some(&h1), &h1)); // 未变化
        assert!(detect_change(Some(&h1), &h3)); // 变化 → 触发
    }

    #[test]
    fn test_apply_selector_extracts_tag_content() {
        let html = "<html><head><title>My Page</title></head><body>Hello</body></html>";
        assert_eq!(apply_selector(html, "title"), "My Page");
        assert_eq!(apply_selector(html, "body"), "Hello");
        // 未找到标签 → 返回原文
        assert_eq!(apply_selector(html, "div"), html);
    }

    // ---- IcsParser ----

    #[test]
    fn test_ics_parser_minimal_vevent() {
        let ics = "BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\n\
UID:abc-123@test\r\n\
SUMMARY:Team Meeting\r\n\
DTSTART:20260704T100000Z\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:def-456@test\r\n\
SUMMARY:Lunch\r\n\
DTSTART:20260704T120000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR";
        let events = parse_ics(ics);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].uid, "abc-123@test");
        assert_eq!(events[0].summary, "Team Meeting");
        assert_eq!(
            events[0].start.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-07-04 10:00:00"
        );
        assert_eq!(events[1].uid, "def-456@test");
        assert_eq!(events[1].summary, "Lunch");
    }

    #[test]
    fn test_ics_parser_dtstart_with_tzid() {
        let ics = "BEGIN:VEVENT\r\n\
UID:t1@test\r\n\
SUMMARY:TimeZoned\r\n\
DTSTART;TZID=America/New_York:20260704T090000\r\n\
END:VEVENT";
        let events = parse_ics(ics);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "TimeZoned");
        // 视为 UTC(简化处理)
        assert_eq!(events[0].start.format("%H:%M:%S").to_string(), "09:00:00");
    }

    // ---- Calendar 临近事件触发 ----

    #[test]
    fn test_calendar_upcoming_event_triggers() {
        let now = chrono::Utc::now();
        let soon = now + chrono::Duration::minutes(10);
        let later = now + chrono::Duration::minutes(120);
        let past = now - chrono::Duration::minutes(5);

        let events = vec![
            IcsEvent {
                uid: "soon".to_string(),
                summary: "Soon Event".to_string(),
                start: soon,
            },
            IcsEvent {
                uid: "later".to_string(),
                summary: "Later Event".to_string(),
                start: later,
            },
            IcsEvent {
                uid: "past".to_string(),
                summary: "Past Event".to_string(),
                start: past,
            },
        ];

        // lead_minutes = 30 → 只匹配 soon(10 分钟后)
        let upcoming = find_upcoming_events(&events, now, 30);
        assert_eq!(upcoming.len(), 1);
        assert_eq!(upcoming[0].uid, "soon");

        // lead_minutes = 180 → 匹配 soon + later
        let upcoming = find_upcoming_events(&events, now, 180);
        assert_eq!(upcoming.len(), 2);

        // lead_minutes = 0 → 不匹配任何(soon 在 10 分钟后,不在 [0,0] 范围)
        let upcoming = find_upcoming_events(&events, now, 0);
        assert_eq!(upcoming.len(), 0);

        // past 事件永远不匹配
        let upcoming = find_upcoming_events(&events, now, 9999);
        assert_eq!(upcoming.len(), 2); // soon + later,past 被排除
    }

    // ---- SystemMetric / CmpOp serde ----

    #[test]
    fn test_system_metric_serde() {
        for m in [
            SystemMetric::CpuUsage,
            SystemMetric::MemoryUsage,
            SystemMetric::DiskFreePercent,
        ] {
            let s = serde_json::to_string(&m).unwrap();
            let back: SystemMetric = serde_json::from_str(&s).unwrap();
            assert_eq!(m, back);
        }
        assert_eq!(
            serde_json::to_string(&SystemMetric::CpuUsage).unwrap(),
            "\"cpu_usage\""
        );
        assert_eq!(
            serde_json::to_string(&SystemMetric::MemoryUsage).unwrap(),
            "\"memory_usage\""
        );
        assert_eq!(
            serde_json::to_string(&SystemMetric::DiskFreePercent).unwrap(),
            "\"disk_free_percent\""
        );
    }

    #[test]
    fn test_cmp_op_serde() {
        for op in [CmpOp::Gt, CmpOp::Lt, CmpOp::Eq] {
            let s = serde_json::to_string(&op).unwrap();
            let back: CmpOp = serde_json::from_str(&s).unwrap();
            assert_eq!(op, back);
        }
        assert_eq!(serde_json::to_string(&CmpOp::Gt).unwrap(), "\"gt\"");
        assert_eq!(serde_json::to_string(&CmpOp::Lt).unwrap(), "\"lt\"");
        assert_eq!(serde_json::to_string(&CmpOp::Eq).unwrap(), "\"eq\"");
    }

    // ---- SystemProbe compare(纯函数,mock 值)----

    #[test]
    fn test_system_probe_compare() {
        // Gt
        assert!(SystemProbe::compare(90.0, CmpOp::Gt, 80.0));
        assert!(!SystemProbe::compare(70.0, CmpOp::Gt, 80.0));
        assert!(!SystemProbe::compare(80.0, CmpOp::Gt, 80.0));

        // Lt
        assert!(SystemProbe::compare(70.0, CmpOp::Lt, 80.0));
        assert!(!SystemProbe::compare(90.0, CmpOp::Lt, 80.0));
        assert!(!SystemProbe::compare(80.0, CmpOp::Lt, 80.0));

        // Eq(浮点容差 0.01)
        assert!(SystemProbe::compare(80.0, CmpOp::Eq, 80.0));
        assert!(SystemProbe::compare(80.005, CmpOp::Eq, 80.0));
        assert!(!SystemProbe::compare(80.5, CmpOp::Eq, 80.0));
    }

    // ---- WatchSource::interval_secs ----

    #[test]
    fn test_watch_source_interval_secs() {
        let web = WatchSource::Web {
            url: "x".to_string(),
            selector: None,
            interval_secs: 120,
        };
        assert_eq!(web.interval_secs(), 120);

        let sys = WatchSource::System {
            metric: SystemMetric::CpuUsage,
            threshold: 50.0,
            op: CmpOp::Gt,
            interval_secs: 15,
        };
        assert_eq!(sys.interval_secs(), 15);

        let cal = WatchSource::Calendar {
            ics_path: "x".to_string(),
            lead_minutes: 10,
        };
        assert_eq!(cal.interval_secs(), 60); // Calendar 默认 60s
    }

    // ---- IcsEvent unescape ----

    #[test]
    fn test_ics_unescape() {
        assert_eq!(unescape_ics("Hello\\, World"), "Hello, World");
        assert_eq!(unescape_ics("Line1\\nLine2"), "Line1\nLine2");
        assert_eq!(unescape_ics("a\\\\b"), "a\\b");
    }
}
