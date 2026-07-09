//! T-E-C-14: 剪贴板智能监听引擎。
//!
//! [`ClipboardWatcherEngine`] 在后台 500ms 轮询系统剪贴板,对内容做
//! 哈希去重 + 类型检测,把"有结构的"内容(URL/代码/表格/JSON 等)吸收到
//! L2 Episodic 记忆,并通过 `nebula://clipboard-detected` 事件通知
//! 前端。短文本(< 10 字符)与 `Other` 类型被忽略,避免对复制单词等
//! 日常操作产生噪声。
//!
//! 设计参考 [`crate::memory::file_watcher::FileWatcherEngine`]:
//! * `CancellationToken` + `tokio::select!` worker loop
//! * `JoinHandle` 由 engine 持有,`stop()` 时 abort
//! * idempotent 的 `start` / `stop` / `is_running`
//!
//! 与 FileWatcher 的差异:
//! * 不需要 `mpsc` channel(无生产者/消费者分离,轮询 + 处理在同一个 task)
//! * 轮询源是 OS 剪贴板(arboard)而非 notify 文件事件
//! * arboard::Clipboard 在部分平台 !Send,因此每次 poll 用
//!   `tokio::task::spawn_blocking` 包裹,保证 tokio task 仍为 Send

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::memory::sponge::SpongeEngine;
use crate::memory::types::{MemoryLayer, MemoryType, SourceKind};

/// 轮询间隔:500ms(参考 spec §设计约束 第 3 条)。
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// 短文本阈值:字符数 < 10 一律忽略(spec §隐私边界)。
const MIN_CONTENT_CHARS: usize = 10;

/// `content_preview` 截断长度(字符)。
const PREVIEW_CHARS: usize = 200;

/// 剪贴板事件,通过 `nebula://clipboard-detected` 推送给前端。
#[derive(Debug, Serialize, Clone)]
pub struct ClipboardEvent {
    /// 内容预览(前 200 字符),供 toast 显示。
    pub content_preview: String,
    /// 完整内容,前端点击通知后注入到 ChatPanel input。
    pub content_full: String,
    /// 检测到的内容类型。
    pub kind: ClipboardKind,
    /// Unix 毫秒时间戳。
    pub ts: i64,
    /// 内容哈希,前端可据此去重。
    pub hash: u64,
}

/// 剪贴板内容类型(serde tag = "type", lowercase)。
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClipboardKind {
    /// 代码块。`language` 来自 fenced code block 的语言标记,启发式
    /// 检测时为 `None`。
    Code { language: Option<String> },
    /// Markdown 表格(包含 `|---|` 分隔行)。
    MarkdownTable,
    /// 合法 JSON。
    Json,
    /// 单行 URL。
    Url,
    /// TSV / CSV(多行 + 一致分隔符)。
    TsvCsv,
    /// Email 地址。
    Email,
    /// IPv4 / IPv6 地址。
    Ip,
    /// 文件系统路径。
    Path,
    /// 其他无结构纯文本(被 worker 跳过,不写入记忆 / 通知)。
    Other,
}

/// 剪贴板监听引擎。`start` spawn 后台 task,`stop` 取消 + abort。
pub struct ClipboardWatcherEngine {
    cancel: Option<CancellationToken>,
    handle: Option<JoinHandle<()>>,
}

impl ClipboardWatcherEngine {
    /// 构造未启动的引擎。
    pub fn new() -> Self {
        Self {
            cancel: None,
            handle: None,
        }
    }

    /// 启动后台轮询 task。若已在运行,返回错误(不允许多实例)。
    pub fn start(&mut self, sponge: Arc<SpongeEngine>, app: AppHandle) -> Result<(), String> {
        if self.handle.is_some() {
            return Err("clipboard watcher already running".into());
        }
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move {
            info!(target: "nebula.clipboard", "watcher worker started");
            let mut last_hash: u64 = 0;
            let mut interval = tokio::time::interval(POLL_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // 首次 tick 立即触发(检查当前剪贴板内容)。
            interval.tick().await;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_clone.cancelled() => {
                        info!(target: "nebula.clipboard", "watcher worker received cancellation");
                        break;
                    }
                    _ = interval.tick() => {
                        let content_opt = tokio::task::spawn_blocking(|| {
                            arboard::Clipboard::new()
                                .and_then(|mut cb| cb.get_text())
                                .ok()
                        })
                        .await
                        .ok()
                        .flatten();
                        let Some(content) = content_opt else { continue };
                        if content.chars().count() < MIN_CONTENT_CHARS {
                            continue;
                        }
                        let hash = hash_content(&content);
                        if hash == last_hash {
                            continue;
                        }
                        last_hash = hash;
                        let kind = detect_kind(&content);
                        if matches!(kind, ClipboardKind::Other) {
                            // 仅吸收有结构的内容,纯短文本忽略。
                            continue;
                        }
                        let event = ClipboardEvent {
                            content_preview: content.chars().take(PREVIEW_CHARS).collect(),
                            content_full: content.clone(),
                            kind: kind.clone(),
                            ts: chrono::Utc::now().timestamp_millis(),
                            hash,
                        };
                        // 写入记忆:Episodic / L2 / External / tool=clipboard-watcher。
                        if let Err(e) = sponge
                            .absorb_text(
                                MemoryType::Episodic,
                                MemoryLayer::L2,
                                &content,
                                SourceKind::External,
                                Some("clipboard-watcher"),
                            )
                            .await
                        {
                            warn!(
                                target: "nebula.clipboard",
                                error = %e,
                                "sponge.absorb_text failed; continuing"
                            );
                        }
                        // 推送给前端。
                        if let Err(e) = app.emit("nebula://clipboard-detected", &event) {
                            warn!(
                                target: "nebula.clipboard",
                                error = %e,
                                "emit clipboard-detected failed"
                            );
                        }
                        debug!(
                            target: "nebula.clipboard",
                            kind = ?kind,
                            hash,
                            bytes = content.len(),
                            "clipboard event emitted"
                        );
                    }
                }
            }
            info!(target: "nebula.clipboard", "watcher worker exiting");
        });
        self.cancel = Some(cancel);
        self.handle = Some(handle);
        info!(target: "nebula.clipboard", "watcher started");
        Ok(())
    }

    /// 停止后台 task:取消 token + abort handle。Idempotent。
    pub fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        if let Some(handle) = self.handle.take() {
            // cancel 信号会让 select! 退出;abort 兜底防止 spawn_blocking 阻塞。
            handle.abort();
        }
        info!(target: "nebula.clipboard", "watcher stopped");
    }

    /// 是否正在运行(handle 存在且未结束)。
    pub fn is_running(&self) -> bool {
        match &self.handle {
            Some(h) => !h.is_finished(),
            None => false,
        }
    }
}

impl Default for ClipboardWatcherEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 内容检测
// ---------------------------------------------------------------------------

/// 检测剪贴板内容的类型。优先级见 spec §内容检测优先级。
pub fn detect_kind(content: &str) -> ClipboardKind {
    // 1. fenced code: 以 ``` 开头
    if content.starts_with("```") {
        let lang = parse_fenced_lang(content);
        return ClipboardKind::Code { language: lang };
    }
    // 2. markdown table: 包含 |---| 或 | --- | 分隔行
    if content
        .lines()
        .any(|l| l.contains("|---|") || l.contains("| --- |"))
    {
        return ClipboardKind::MarkdownTable;
    }
    // 3. JSON: 能解析为 serde_json::Value
    if serde_json::from_str::<serde_json::Value>(content).is_ok() {
        return ClipboardKind::Json;
    }
    // 4. URL: 单行 URL
    let trimmed = content.trim();
    if trimmed.lines().count() == 1 && is_url(trimmed) {
        return ClipboardKind::Url;
    }
    // 5. heuristic code:常见关键字 / 括号结构
    let code_hints = [
        "function ",
        "def ",
        "class ",
        "import ",
        "pub fn",
        "fn ",
        "{",
        "};",
    ];
    if code_hints.iter().any(|h| content.contains(h)) {
        return ClipboardKind::Code { language: None };
    }
    // 6. TSV / CSV:多行 + 一致分隔符
    if is_tsv_csv(content) {
        return ClipboardKind::TsvCsv;
    }
    // 7. email / ip / path(单行匹配)
    if trimmed.lines().count() == 1 {
        if is_email(trimmed) {
            return ClipboardKind::Email;
        }
        if is_ip(trimmed) {
            return ClipboardKind::Ip;
        }
        if is_path(trimmed) {
            return ClipboardKind::Path;
        }
    }
    ClipboardKind::Other
}

/// 从 fenced code block 第一行解析语言标记。
/// `"```rust\n..."` → `Some("rust")`;`"```\n..."` → `None`。
fn parse_fenced_lang(content: &str) -> Option<String> {
    let first_line = content.lines().next()?;
    let lang = first_line.trim_start_matches("```").trim();
    if lang.is_empty() {
        None
    } else {
        Some(lang.to_string())
    }
}

/// URL 检测:必须以 http:// 或 https:// 开头,且无内嵌空格。
fn is_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && !s.contains(char::is_whitespace)
        && s.len() > "http://".len()
}

/// Email 检测:简单正则 `[^@\s]+@[^@\s]+\.[^@\s]+`。
fn is_email(s: &str) -> bool {
    // T-D-B-07: 字面量保证有效,保留 expect
    let re = regex::Regex::new(r"^[^@\s]+@[^@\s]+\.[^@\s]+$").expect("valid regex");
    re.is_match(s)
}

/// IPv4 / IPv6 检测。
fn is_ip(s: &str) -> bool {
    // IPv4: a.b.c.d,每段 0-255。
    if s.split('.').count() == 4 && s.split('.').all(|p| p.parse::<u8>().is_ok()) {
        return true;
    }
    // IPv6:含至少一个 ':' 且每段为十六进制。
    if s.contains(':') && s.len() <= 39 {
        // T-D-B-07: 字面量保证有效,保留 expect
        let re = regex::Regex::new(r"^[0-9a-fA-F:]+$").expect("valid regex");
        if re.is_match(s) && s.split(':').count() >= 2 {
            return true;
        }
    }
    false
}

/// 文件路径检测:含路径分隔符或盘符前缀,且不含空格(简化启发式)。
fn is_path(s: &str) -> bool {
    if s.contains(char::is_whitespace) {
        return false;
    }
    // Windows 盘符:`C:\foo` 或 `C:/foo`
    if s.len() >= 3 {
        let bytes = s.as_bytes();
        if bytes[1] == b':'
            && (bytes[0].is_ascii_alphabetic())
            && (bytes[2] == b'\\' || bytes[2] == b'/')
        {
            return true;
        }
    }
    // Unix 绝对路径:`/foo/bar`
    if s.starts_with('/') && s.len() > 1 {
        return true;
    }
    // 含反斜杠分隔(Windows 相对路径)
    if s.contains('\\') && s.chars().any(|c| c.is_alphanumeric()) {
        return true;
    }
    false
}

/// TSV/CSV 检测:多行 + 每行包含一致数量的分隔符(`\t` 或 `,`)。
fn is_tsv_csv(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    // 优先尝试 TSV(制表符),其次 CSV(逗号)。
    for sep in ['\t', ','] {
        let counts: Vec<usize> = lines.iter().map(|l| l.matches(sep).count()).collect();
        if counts.iter().all(|&c| c > 0) && counts.iter().all(|&c| c == counts[0]) {
            return true;
        }
    }
    false
}

/// 用 `DefaultHasher` 计算内容哈希(u64)。用于 worker 去重。
pub fn hash_content(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_kind_fenced_code() {
        let content = "```rust\nfn main() {}\n```";
        let kind = detect_kind(content);
        match kind {
            ClipboardKind::Code { language } => {
                assert_eq!(language.as_deref(), Some("rust"));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn test_detect_kind_fenced_code_no_lang() {
        let content = "```\nplain text block\n```";
        let kind = detect_kind(content);
        match kind {
            ClipboardKind::Code { language } => {
                assert!(language.is_none());
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn test_detect_kind_markdown_table() {
        let content = "| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(detect_kind(content), ClipboardKind::MarkdownTable);
    }

    #[test]
    fn test_detect_kind_markdown_table_spaced() {
        let content = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        assert_eq!(detect_kind(content), ClipboardKind::MarkdownTable);
    }

    #[test]
    fn test_detect_kind_json() {
        let content = "{\"a\":1,\"b\":[2,3]}";
        assert_eq!(detect_kind(content), ClipboardKind::Json);
    }

    #[test]
    fn test_detect_kind_json_array() {
        let content = "[1, 2, 3]";
        assert_eq!(detect_kind(content), ClipboardKind::Json);
    }

    #[test]
    fn test_detect_kind_url() {
        let content = "https://example.com";
        assert_eq!(detect_kind(content), ClipboardKind::Url);
    }

    #[test]
    fn test_detect_kind_url_http() {
        let content = "http://localhost:8080/path?q=1";
        assert_eq!(detect_kind(content), ClipboardKind::Url);
    }

    #[test]
    fn test_detect_kind_heuristic_code() {
        let content = "function foo() { return 1; }";
        match detect_kind(content) {
            ClipboardKind::Code { language } => assert!(language.is_none()),
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn test_detect_kind_heuristic_code_def() {
        let content = "def hello_world():\n    print('hi')";
        match detect_kind(content) {
            ClipboardKind::Code { language } => assert!(language.is_none()),
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn test_detect_kind_tsv_csv() {
        let content = "a\tb\tc\n1\t2\t3";
        assert_eq!(detect_kind(content), ClipboardKind::TsvCsv);
    }

    #[test]
    fn test_detect_kind_csv() {
        let content = "a,b,c\n1,2,3\n4,5,6";
        assert_eq!(detect_kind(content), ClipboardKind::TsvCsv);
    }

    #[test]
    fn test_detect_kind_email() {
        let content = "user@example.com";
        assert_eq!(detect_kind(content), ClipboardKind::Email);
    }

    #[test]
    fn test_detect_kind_ipv4() {
        let content = "192.168.1.1";
        assert_eq!(detect_kind(content), ClipboardKind::Ip);
    }

    #[test]
    fn test_detect_kind_ipv6() {
        let content = "::1";
        assert_eq!(detect_kind(content), ClipboardKind::Ip);
    }

    #[test]
    fn test_detect_kind_path_windows() {
        let content = "C:\\Users\\alice\\file.txt";
        assert_eq!(detect_kind(content), ClipboardKind::Path);
    }

    #[test]
    fn test_detect_kind_path_unix() {
        let content = "/home/alice/project";
        assert_eq!(detect_kind(content), ClipboardKind::Path);
    }

    #[test]
    fn test_detect_kind_other() {
        let content = "hello world this is a plain text message";
        assert_eq!(detect_kind(content), ClipboardKind::Other);
    }

    #[test]
    fn test_hash_dedup_same_content_same_hash() {
        let a = "some clipboard content";
        let b = "some clipboard content";
        assert_eq!(hash_content(a), hash_content(b));
    }

    #[test]
    fn test_hash_dedup_different_content_different_hash() {
        let a = "some clipboard content";
        let b = "different clipboard content";
        assert_ne!(hash_content(a), hash_content(b));
    }

    #[test]
    fn test_short_text_ignored_by_worker_filter() {
        // 验证 worker 使用的 < 10 字符阈值:这里直接验证字符数计算逻辑。
        let short = "hello"; // 5 chars
        assert!(short.chars().count() < MIN_CONTENT_CHARS);
        let ok = "hello world"; // 11 chars
        assert!(ok.chars().count() >= MIN_CONTENT_CHARS);
    }

    #[test]
    fn test_is_url_rejects_inner_whitespace() {
        assert!(!is_url("http://example.com with space"));
        assert!(is_url("http://example.com"));
    }

    #[test]
    fn test_is_url_rejects_bare_domain() {
        // 无 scheme 的裸域名不算 URL(避免误报)
        assert!(!is_url("example.com"));
    }

    #[test]
    fn test_is_tsv_csv_rejects_single_line() {
        assert!(!is_tsv_csv("a,b,c"));
    }

    #[test]
    fn test_is_tsv_csv_rejects_inconsistent_columns() {
        assert!(!is_tsv_csv("a,b,c\n1,2"));
    }

    #[test]
    fn test_engine_new_is_not_running() {
        let engine = ClipboardWatcherEngine::new();
        assert!(!engine.is_running());
    }

    #[test]
    fn test_engine_stop_idempotent_on_unstarted() {
        let mut engine = ClipboardWatcherEngine::new();
        engine.stop(); // 不应 panic
        assert!(!engine.is_running());
    }
}
