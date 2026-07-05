//! T-E-A-02 TokenJuice: 三级压缩编排器。
//!
//! 在 `LlmGateway::chat()` 入口处、L0 cache_key 计算之前对消息列表
//! 做三级压缩,目标 -85% token 消耗:
//!
//! * **L1 脱敏**:复用 `PrivacyGuard::redact`,对 user/tool 消息
//!   抹除 API key / 手机号 / 身份证等 PII。
//! * **L2 压缩**:对 user 消息做 HTML->MD(正则简化版)、长 URL
//!   缩短、全角->半角规范化。
//! * **L3 摘要**:超过阈值时,把旧消息交给 `LlmGateway::generate`
//!   生成摘要,用一条 system 消息替代;失败静默回退原文。
//!
//! ## 设计要点
//!
//! * 三级独立可开关(`TokenJuiceConfig`)。
//! * `tool_calls` / `tool_call_id` / `name` 字段原样保留,只压缩
//!   `content`。
//! * L3 摘要结果用 `parking_lot::Mutex<HashMap<u64, String>>` 缓存
//!   (key = 旧消息段哈希),避免非确定性破坏 L0 cache。
//! * 所有压缩步骤失败静默,降级为原消息。

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use tracing::warn;

use crate::llm::ollama::{ChatMessage, OllamaClient};
use crate::memory::values::privacy_guard::PrivacyGuard;

// ---- L2 HTML->MD 预编译正则 ----
// `regex` crate 不支持反向引用,故 header / b / strong / i / em
// 使用分开的模式或宽松闭合标签匹配。
static RE_SCRIPT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<script.*?</script>").expect("valid regex"));
static RE_STYLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<style.*?</style>").expect("valid regex"));
static RE_BR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<br\s*/?>").expect("valid regex"));
static RE_P_CLOSE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)</p>").expect("valid regex"));
static RE_P_OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<p[^>]*>").expect("valid regex"));
static RE_H: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<h([1-6])[^>]*>(.*?)</h[1-6]>").expect("valid regex"));
static RE_LI: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<li[^>]*>").expect("valid regex"));
static RE_A: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?is)<a\s+href="([^"]+)"[^>]*>(.*?)</a>"#).expect("valid regex")
});
static RE_B: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<b[^>]*>(.*?)</b>").expect("valid regex"));
static RE_STRONG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<strong[^>]*>(.*?)</strong>").expect("valid regex"));
static RE_I: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<i[^>]*>(.*?)</i>").expect("valid regex"));
static RE_EM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<em[^>]*>(.*?)</em>").expect("valid regex"));
static RE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").expect("valid regex"));

// L2 URL 缩短:裸 http(s) URL,排除引号/尖括号/圆括号以免吞掉 markdown 链接外壳。
static RE_URL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"https?://[^\s<>"')]+"#).expect("valid regex"));

/// T-E-A-02: TokenJuice 三级压缩配置。每级独立可开关。
#[derive(Debug, Clone)]
pub struct TokenJuiceConfig {
    /// L1 脱敏开关(默认 true)。
    pub l1_redact_enabled: bool,
    /// L2 压缩开关(默认 true)。
    pub l2_compress_enabled: bool,
    /// L3 摘要开关(默认 true)。
    pub l3_summary_enabled: bool,
    /// L3 触发阈值:消息数超过此值才摘要(默认 6)。
    pub l3_summary_threshold_msgs: usize,
    /// L3 保留最近 N 条原文不参与摘要(默认 4)。
    pub l3_summary_keep_recent: usize,
}

impl Default for TokenJuiceConfig {
    fn default() -> Self {
        Self {
            l1_redact_enabled: true,
            l2_compress_enabled: true,
            l3_summary_enabled: true,
            l3_summary_threshold_msgs: 6,
            l3_summary_keep_recent: 4,
        }
    }
}

/// T-E-A-02: TokenJuice 三级压缩编排器。
///
/// 持有 `Arc<OllamaClient>`(L3 摘要用,直连本地 Ollama 零成本)与
/// 一个进程内摘要缓存。`compress` 是无副作用流水线:输入消息被消费,
/// 返回压缩后的新消息列表。
pub struct TokenJuiceCompressor {
    config: TokenJuiceConfig,
    ollama: Arc<OllamaClient>,
    model: String,
    summary_cache: Mutex<HashMap<u64, String>>,
}

impl TokenJuiceCompressor {
    /// 构造压缩器。`ollama` + `model` 用于 L3 摘要;若 L3 关闭,不会被调用。
    pub fn new(ollama: Arc<OllamaClient>, model: String, config: TokenJuiceConfig) -> Self {
        Self {
            config,
            ollama,
            model,
            summary_cache: Mutex::new(HashMap::new()),
        }
    }

    /// 主入口:对消息列表依次执行 L1->L2->L3,返回压缩后的消息。
    ///
    /// 任何一步失败都静默降级(保留原消息),不阻断主流程。压缩收益
    /// 通过 `metrics::global().record_token_usage(saved, 0)` 累加观测
    /// (`record_tokens_saved` 尚不存在,复用最接近的 token 用量计数器)。
    pub async fn compress(&self, messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let tokens_before = Self::estimate_tokens(&messages);

        let mut compressed = messages;

        // L1: 对 user/tool 消息脱敏。
        if self.config.l1_redact_enabled {
            for m in compressed.iter_mut() {
                if m.role == "user" || m.role == "tool" {
                    let original = m.content.clone();
                    m.content = self.l1_redact(&original);
                }
            }
        }

        // L2: 对 user 消息做 HTML->MD + URL 缩短 + 全角->半角。
        if self.config.l2_compress_enabled {
            for m in compressed.iter_mut() {
                if m.role == "user" {
                    let original = m.content.clone();
                    let mut s = self.l2_compress_html(&original);
                    s = self.l2_shorten_urls(&s);
                    s = self.l2_normalize_ascii(&s);
                    m.content = s;
                }
            }
        }

        // L3: 超阈值时摘要旧消息,保留最近 N 条原文。
        if self.config.l3_summary_enabled
            && compressed.len() > self.config.l3_summary_threshold_msgs
        {
            let keep_recent = self.config.l3_summary_keep_recent.min(compressed.len());
            let old_count = compressed.len() - keep_recent;
            if old_count > 0 {
                // 克隆旧消息段以避免跨 await 借用 compressed。
                let old_messages: Vec<ChatMessage> = compressed[..old_count].to_vec();
                if let Some(summary) = self.l3_summarize(&old_messages).await {
                    let mut new_msgs = Vec::with_capacity(keep_recent + 1);
                    new_msgs.push(ChatMessage::system(format!("【对话摘要】{}", summary)));
                    new_msgs.extend(compressed[old_count..].iter().cloned());
                    compressed = new_msgs;
                }
                // summary == None: 保留原文(失败静默)。
            }
        }

        let tokens_after = Self::estimate_tokens(&compressed);
        let saved = tokens_before.saturating_sub(tokens_after);
        if saved > 0 {
            // metrics 无 record_tokens_saved;复用 record_token_usage
            // 累加到 prompt token 计数器,主 agent 集成时可换专用计数器。
            crate::metrics::global().record_token_usage(saved as u64, 0);
        }

        compressed
    }

    /// 估算消息列表的 token 数(chars/2 启发式,不引入 tiktoken)。
    pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
        let chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        chars / 2
    }

    /// L1: 调 `PrivacyGuard::redact` 抹除 PII。
    fn l1_redact(&self, content: &str) -> String {
        PrivacyGuard::new().redact(content)
    }

    /// L2: HTML->MD 正则简化版。去除 script/style/标签外壳,转换
    /// 常见块级与行内标签为 markdown 等价物。
    fn l2_compress_html(&self, content: &str) -> String {
        let s = RE_SCRIPT.replace_all(content, "").to_string();
        let s = RE_STYLE.replace_all(&s, "").to_string();
        let s = RE_BR.replace_all(&s, "\n").to_string();
        let s = RE_P_CLOSE.replace_all(&s, "\n\n").to_string();
        let s = RE_P_OPEN.replace_all(&s, "").to_string();
        let s = RE_H
            .replace_all(&s, |caps: &regex::Captures| {
                let level: usize = caps
                    .get(1)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(1);
                let text = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                format!("{} {}", "#".repeat(level), text)
            })
            .to_string();
        let s = RE_LI.replace_all(&s, "\n- ").to_string();
        let s = RE_A.replace_all(&s, "[$2]($1)").to_string();
        let s = RE_B.replace_all(&s, "**$1**").to_string();
        let s = RE_STRONG.replace_all(&s, "**$1**").to_string();
        let s = RE_I.replace_all(&s, "*$1*").to_string();
        let s = RE_EM.replace_all(&s, "*$1*").to_string();
        let s = RE_TAG.replace_all(&s, "").to_string();
        s
    }

    /// L2: 裸 URL 超过 50 字符时转为 `[link](url)`,缩短可见文本。
    /// 短 URL 原样保留。
    fn l2_shorten_urls(&self, content: &str) -> String {
        RE_URL
            .replace_all(content, |caps: &regex::Captures| {
                let url = caps.get(0).map(|m| m.as_str()).unwrap_or("");
                if url.chars().count() > 50 {
                    format!("[link]({})", url)
                } else {
                    url.to_string()
                }
            })
            .to_string()
    }

    /// L2: 全角->半角规范化。覆盖 U+FF01..U+FF5E(ASCII 区)与
    /// U+3000(全角空格)。
    fn l2_normalize_ascii(&self, content: &str) -> String {
        content
            .chars()
            .map(|c| {
                let code = c as u32;
                if code == 0x3000 {
                    ' '
                } else if (0xFF01..=0xFF5E).contains(&code) {
                    char::from_u32(code - 0xFEE0).unwrap_or(c)
                } else {
                    c
                }
            })
            .collect()
    }

    /// L3: 把旧消息段交给 LLM 摘要。带缓存命中检查(相同消息段只摘要
    /// 一次)。LLM 失败返回 `None`(调用方保留原文)。
    async fn l3_summarize(&self, old_messages: &[ChatMessage]) -> Option<String> {
        let key = messages_hash(old_messages);
        // 缓存命中:直接返回,跳过 LLM 调用。
        if let Some(cached) = self.summary_cache.lock().get(&key) {
            return Some(cached.clone());
        }
        let prompt = build_summary_prompt(old_messages);
        match self.ollama.generate(&self.model, &prompt).await {
            Ok(resp) => {
                let trimmed = resp.response.trim().to_string();
                self.summary_cache.lock().insert(key, trimmed.clone());
                Some(trimmed)
            }
            Err(e) => {
                warn!(
                    target: "nebula.llm.token_juice",
                    error = %e,
                    "L3 summarize failed; keeping original messages"
                );
                None
            }
        }
    }
}

/// 计算消息段的稳定哈希,用作 L3 摘要缓存 key。
/// 只哈希 role + content(tool_calls 等字段不参与,保持确定性)。
fn messages_hash(messages: &[ChatMessage]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for m in messages {
        m.role.hash(&mut hasher);
        m.content.hash(&mut hasher);
    }
    hasher.finish()
}

/// 构造 L3 摘要 prompt:把旧消息段渲染为 `[role]: content` 列表。
fn build_summary_prompt(messages: &[ChatMessage]) -> String {
    let mut buf = String::with_capacity(512);
    buf.push_str("请将以下对话历史压缩为简洁摘要(不超过200字),保留关键信息、决策和未完成事项:\n\n");
    for m in messages {
        buf.push_str(&format!("[{}]: {}\n", m.role, m.content));
    }
    buf.push_str("\n只输出摘要正文,不要附加说明。");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ollama::{FunctionCall, OllamaClient, ToolCall};

    fn make_compressor() -> TokenJuiceCompressor {
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1".to_string()));
        TokenJuiceCompressor::new(ollama, "test-model".to_string(), TokenJuiceConfig::default())
    }

    #[test]
    fn test_l1_redact_replaces_api_key() {
        let compressor = make_compressor();
        let redacted =
            compressor.l1_redact("api_key=sk-abcdefghijklmnopqrstuvwxyz1234567890");
        assert!(
            redacted.contains("[REDACTED]"),
            "expected [REDACTED] in: {}",
            redacted
        );
        assert!(
            !redacted.contains("sk-abcdefghijklmnopqrstuvwxyz1234567890"),
            "raw api key should be removed: {}",
            redacted
        );
    }

    #[test]
    fn test_l2_html_to_md_basic() {
        let compressor = make_compressor();
        let out = compressor.l2_compress_html("<p>hello</p>");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn test_l2_html_to_md_links() {
        let compressor = make_compressor();
        let out = compressor.l2_compress_html(r#"<a href="http://x.com">text</a>"#);
        assert_eq!(out, "[text](http://x.com)");
    }

    #[test]
    fn test_l2_html_to_md_headings() {
        let compressor = make_compressor();
        let out = compressor.l2_compress_html("<h2>Title</h2>");
        assert_eq!(out.trim(), "## Title");
    }

    #[test]
    fn test_l2_html_to_md_strips_script_style() {
        let compressor = make_compressor();
        let out = compressor
            .l2_compress_html("<script>alert(1)</script><style>x{}</style><p>ok</p>");
        assert!(!out.contains("alert"));
        assert!(!out.contains("style"));
        assert_eq!(out.trim(), "ok");
    }

    #[test]
    fn test_l2_shorten_urls_long() {
        let compressor = make_compressor();
        let long_url =
            "http://example.com/very/long/path/that/is/definitely/more/than/fifty/characters";
        assert!(long_url.chars().count() > 50);
        let out = compressor.l2_shorten_urls(long_url);
        assert_eq!(out, format!("[link]({})", long_url));
    }

    #[test]
    fn test_l2_shorten_urls_short_preserved() {
        let compressor = make_compressor();
        let short_url = "http://x.com/a";
        let out = compressor.l2_shorten_urls(short_url);
        assert_eq!(out, short_url);
    }

    #[test]
    fn test_l2_normalize_ascii_fullwidth() {
        let compressor = make_compressor();
        let out = compressor.l2_normalize_ascii("ABC123 ：！");
        assert_eq!(out, "ABC123 :!");
    }

    #[test]
    fn test_estimate_tokens_basic() {
        let msg = ChatMessage::user("hello");
        assert_eq!(TokenJuiceCompressor::estimate_tokens(&[msg]), 2);
    }

    #[test]
    fn test_estimate_tokens_multiple() {
        let msgs = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
        ];
        assert_eq!(TokenJuiceCompressor::estimate_tokens(&msgs), 3);
    }

    #[tokio::test]
    async fn test_l3_summary_cache_hit() {
        let compressor = make_compressor();
        let messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi there"),
        ];
        let key = messages_hash(&messages);
        compressor
            .summary_cache
            .lock()
            .insert(key, "cached summary".to_string());
        let result = compressor.l3_summarize(&messages).await;
        assert_eq!(result, Some("cached summary".to_string()));
    }

    #[tokio::test]
    async fn test_l3_summary_miss_returns_none_on_llm_failure() {
        let compressor = make_compressor();
        let messages = vec![ChatMessage::user("hello")];
        let result = compressor.l3_summarize(&messages).await;
        assert!(result.is_none(), "expected None when LLM unavailable");
    }

    #[tokio::test]
    async fn test_compress_preserves_tool_calls() {
        let compressor = make_compressor();
        let msg = ChatMessage {
            role: "assistant".into(),
            content: "calling tool".into(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                ty: "function".into(),
                function: FunctionCall {
                    name: "search".into(),
                    arguments: "{}".into(),
                },
            }]),
            ..Default::default()
        };
        let result = compressor.compress(vec![msg]).await;
        assert_eq!(result.len(), 1);
        let tool_calls = result[0].tool_calls.as_ref().expect("tool_calls preserved");
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].function.name, "search");
        assert_eq!(result[0].content, "calling tool");
    }

    #[tokio::test]
    async fn test_compress_l1_redacts_user_message() {
        let mut config = TokenJuiceConfig::default();
        config.l3_summary_enabled = false;
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1".to_string()));
        let compressor = TokenJuiceCompressor::new(ollama, "test-model".to_string(), config);
        let msg = ChatMessage::user("api_key=sk-abcdefghijklmnopqrstuvwxyz1234567890");
        let result = compressor.compress(vec![msg]).await;
        assert_eq!(result.len(), 1);
        assert!(result[0].content.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn test_compress_disabled_passes_through() {
        let config = TokenJuiceConfig {
            l1_redact_enabled: false,
            l2_compress_enabled: false,
            l3_summary_enabled: false,
            l3_summary_threshold_msgs: 0,
            l3_summary_keep_recent: 0,
        };
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1".to_string()));
        let compressor = TokenJuiceCompressor::new(ollama, "test-model".to_string(), config);
        let msg = ChatMessage::user("<p>api_key=sk-abcdefghijklmnopqrstuvwxyz1234567890</p>");
        let result = compressor.compress(vec![msg]).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].content,
            "<p>api_key=sk-abcdefghijklmnopqrstuvwxyz1234567890</p>"
        );
    }

    #[test]
    fn test_config_defaults() {
        let c = TokenJuiceConfig::default();
        assert!(c.l1_redact_enabled);
        assert!(c.l2_compress_enabled);
        assert!(c.l3_summary_enabled);
        assert_eq!(c.l3_summary_threshold_msgs, 6);
        assert_eq!(c.l3_summary_keep_recent, 4);
    }
}
