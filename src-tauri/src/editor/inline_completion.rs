//! T-E-S-51: Level 0 内联补全引擎。
//!
//! 设计目标(spec §背景):
//! * **零成本** — 不走 `LlmGateway::call_remote` / `CostTracker`,直接
//!   复用本地 `OllamaClient` 的 `/api/generate` 端点。
//! * **小模型** — 由调用方注入 model 名(推荐 `qwen2.5-coder:0.5b`),
//!   引擎不硬编码。
//! * **max_tokens=20** — 通过 `GenerateOptions { num_predict: Some(20),
//!   temperature: Some(0.2) }` 限流,避免本地小模型一泻千里。
//! * **失败静默** — `suggest_completion` 任何错误返回 `Ok(None)`,前端
//!   不显示错误,用户输入体验不被破坏。
//! * **300ms 防抖** — 同一前缀在 300ms 内重复请求直接返回 `None`,
//!   避免每次按键都打 Ollama。
//!
//! 引擎刻意不依赖 `AppState` / `LlmGateway`,以便单独编译与单测。
//! Tauri 命令层(`commands::inline_completion::inline_complete`)负责
//! 把 `AppState` 中的 `OllamaClient` 注入进来。

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::llm::ollama::{GenerateOptions, GenerateResponse, OllamaClient};

/// 防抖窗口 — 同一前缀两次请求之间的最小间隔。
///
/// 选 300ms 是 spec §设计约束 第 4 条的硬性要求:后端用 `Instant` 比较,
/// 前端用 `setTimeout` + `clearTimeout`。前端 300ms 防抖已挡掉大部分
/// 按键,这里的 300ms 是后端兜底(防止前端 bug 或多窗口触发)。
pub const DEBOUNCE_WINDOW: Duration = Duration::from_millis(300);

/// prefix 最小有效长度(trim 后)。短于该长度直接返回 `None`,避免
/// 在用户刚开始输入时就开始打模型。
pub const MIN_PREFIX_LEN: usize = 3;

/// T-E-S-51: Level 0 内联补全引擎。
///
/// 持有一个 `Arc<OllamaClient>`(共享 LlmGateway 的 primary client)、
/// 一个 model 名、以及两个 `Mutex` 包裹的防抖状态(上次请求时刻 +
/// 上次请求的 prefix)。引擎自身是 `Send + Sync` 的,可安全放进
/// `AppState` 的 `Arc` 字段中。
pub struct InlineCompletionEngine {
    ollama: Arc<OllamaClient>,
    model: String,
    last_call: Mutex<Option<Instant>>,
    last_prefix: Mutex<String>,
}

impl InlineCompletionEngine {
    /// 构造引擎。`ollama` 通常来自 `LlmGateway::ollama_client()` 的
    /// `Arc` clone;`model` 是本地小模型名(如 `qwen2.5-coder:0.5b`)。
    pub fn new(ollama: Arc<OllamaClient>, model: String) -> Self {
        Self {
            ollama,
            model,
            last_call: Mutex::new(None),
            last_prefix: Mutex::new(String::new()),
        }
    }

    /// 暴露底层 model 名(供命令层日志 / 测试断言)。
    pub fn model(&self) -> &str {
        &self.model
    }

    /// T-E-S-51: 对给定 `prefix` 给出补全建议。
    ///
    /// 决策树(spec §必须交付 第 2 条):
    /// 1. `prefix.trim().len() < 3` → `Ok(None)`
    /// 2. 距上次请求 < 300ms **且** 当前 prefix 是上次 prefix 的
    ///    前缀(即用户在快速连续输入)→ `Ok(None)`
    /// 3. 调 `ollama.generate_with_options(model, prompt, defaults)`
    /// 4. 截断到第一行(按 `\n` split 取第一段),`trim`
    /// 5. 结果为空 **或** 等于 prefix → `Ok(None)`(避免回声)
    /// 6. 任何错误 → `Ok(None)`(失败静默,spec §设计约束 第 5 条)
    ///
    /// 注意:`last_call` / `last_prefix` 的更新发生在 *请求发起前*,
    /// 这样即使请求失败,下次同前缀的请求仍会被防抖挡掉。
    pub async fn suggest_completion(&self, prefix: &str) -> anyhow::Result<Option<String>> {
        // (1) 短 prefix 直接放弃。
        if prefix.trim().len() < MIN_PREFIX_LEN {
            return Ok(None);
        }

        // (2) 防抖:同前缀 300ms 内不重复请求。
        if self.should_debounce(prefix) {
            return Ok(None);
        }

        // 记录本次请求时刻 + prefix(请求前记录,失败也保留)。
        self.last_call.lock().replace(Instant::now());
        *self.last_prefix.lock() = prefix.to_string();

        // (3) 调 Ollama。任何错误 → 静默返回 None。
        let prompt = build_prompt(prefix);
        let resp = match self
            .ollama
            .generate_with_options(
                &self.model,
                &prompt,
                GenerateOptions::inline_completion_defaults(),
            )
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };

        // (4) + (5) 截断 + 空值/回声过滤。
        let suggestion = truncate_completion(&resp.response);
        if suggestion.is_empty() || suggestion == prefix {
            return Ok(None);
        }
        Ok(Some(suggestion))
    }

    /// 防抖判定:距上次请求 < 300ms 且当前 prefix 是上次 prefix 的
    /// 前缀(即用户在原 prefix 上继续输入)→ 返回 `true`(挡掉)。
    ///
    /// 设计权衡:用 "prefix 是上次 prefix 的前缀" 而非 "完全相等",
    /// 是因为前端已经 300ms 防抖了,后端再挡完全相等的请求意义不大;
    /// 真正要挡的是 "用户连续输入,prefix 在快速变化" 的场景。
    fn should_debounce(&self, prefix: &str) -> bool {
        let last_call = self.last_call.lock();
        let last_prefix = self.last_prefix.lock();
        match *last_call {
            Some(t) => {
                let elapsed = t.elapsed();
                elapsed < DEBOUNCE_WINDOW && prefix.starts_with(&*last_prefix)
            }
            None => false,
        }
    }
}

/// 构造发给 Ollama 的 prompt。
///
/// 模板刻意简单(spec §必须交付 第 2 条):用 `Continuation:` 引导模型
/// 接着 prefix 写,而不是用 chat 模板 — `/api/generate` 是 raw
/// completion 端点,适合补全场景。
fn build_prompt(prefix: &str) -> String {
    format!(
        "Complete the user's message prefix. Prefix: \"{}\"\nContinuation:",
        prefix
    )
}

/// T-E-S-51: 把模型的 raw 输出截断成单行建议。
///
/// 规则(spec §必须交付 第 2 条):
/// 1. 按 `\n` split,取第一段
/// 2. `trim` 掉首尾空白
/// 3. 返回结果(可能为空字符串)
///
/// 抽成独立函数便于单测 — `OllamaClient` 难以 mock(它是具体类型
/// 而非 trait),但截断逻辑是纯字符串处理,单独测即可覆盖。
pub fn truncate_completion(raw: &str) -> String {
    raw.split('\n').next().unwrap_or("").trim().to_string()
}

/// T-E-S-51: 从 `GenerateResponse` 抽取纯文本(供测试 / 未来扩展)。
///
/// 当前实现直接读 `response` 字段;留这个函数是为了将来若 response
/// shape 变化(如多字段拼接)只改一处。
#[allow(dead_code)]
fn extract_text(resp: &GenerateResponse) -> &str {
    &resp.response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ollama::OllamaClient;

    // ---------- truncate_completion 纯逻辑测试 ----------

    #[test]
    fn truncate_completion_takes_first_line_and_trims() {
        // 多行 → 取第一行;首尾空白 trim。
        assert_eq!(
            truncate_completion("hello world\nsecond line"),
            "hello world"
        );
        assert_eq!(truncate_completion("  padded  \nrest"), "padded");
        // 单行也 trim。
        assert_eq!(truncate_completion("  only line  "), "only line");
    }

    #[test]
    fn truncate_completion_empty_input_returns_empty() {
        assert_eq!(truncate_completion(""), "");
        assert_eq!(truncate_completion("\n\n\n"), "");
        assert_eq!(truncate_completion("   "), "");
    }

    #[test]
    fn truncate_completion_handles_crlf_style() {
        // 模型输出有时含 \r\n — split('\n') 后第一段会带 \r,trim 会清掉。
        assert_eq!(truncate_completion("hello\r\nworld"), "hello");
    }

    // ---------- build_prompt 测试 ----------

    #[test]
    fn build_prompt_contains_prefix_and_continuation_marker() {
        let p = build_completion_prompt_for_test("你好");
        assert!(p.contains("你好"));
        assert!(p.contains("Continuation:"));
        assert!(p.contains("Prefix:"));
    }

    #[test]
    fn build_prompt_with_special_chars_is_preserved() {
        // 含引号 / 反斜杠的 prefix 应原样保留(不做转义 — prompt 是给
        // 模型读的文本,不是 JSON)。
        let p = build_completion_prompt_for_test(r#"he said "hi" \n"#);
        assert!(p.contains(r#"he said "hi" \n"#));
    }

    // ---------- 短 prefix 返回 None ----------

    #[tokio::test]
    async fn short_prefix_returns_none() {
        // 用一个绝对不可达的 URL 构造 client,确保即使引擎逻辑漏判
        // 也不会真的发出请求(短 prefix 在请求前就 return)。
        let ollama = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_millis(50),
        ));
        let engine = InlineCompletionEngine::new(ollama, "qwen2.5-coder:0.5b".to_string());

        // trim 后长度 < 3 的各种情况。
        assert_eq!(engine.suggest_completion("").await.unwrap(), None);
        assert_eq!(engine.suggest_completion("a").await.unwrap(), None);
        assert_eq!(engine.suggest_completion("ab").await.unwrap(), None);
        assert_eq!(engine.suggest_completion("   a  ").await.unwrap(), None);
        // 长度 == 3 应该放行(不返回 None on length check),但会因
        // 连接失败而静默返回 None — 这里只测短 prefix 的 fast path。
    }

    // ---------- 防抖逻辑测试 ----------

    #[tokio::test]
    async fn debounce_blocks_same_prefix_within_window() {
        let ollama = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_millis(50),
        ));
        let engine = InlineCompletionEngine::new(ollama, "qwen2.5-coder:0.5b".to_string());

        // 第一次请求:prefix 足够长,通过长度检查;然后会尝试连 Ollama
        // (失败,返回 None)。但 last_call / last_prefix 已被记录。
        let _ = engine.suggest_completion("hello world").await.unwrap();
        // 立即再请求同一前缀 — 应被防抖挡掉(返回 None)。
        // 注意:即使第二次也会因连接失败返回 None,我们用
        // `last_call` 是否在 300ms 内来验证防抖命中。
        let blocked = engine.should_debounce("hello world");
        assert!(blocked, "同前缀 300ms 内应被防抖挡掉");

        // 不同前缀(不是上次前缀的延伸)不应被挡。
        let blocked_other = engine.should_debounce("totally different");
        assert!(
            !blocked_other,
            "完全不同的前缀不应被防抖挡掉(但会因连接失败返回 None)"
        );

        // 上次前缀的延伸("hello world more")应被挡 — 用户在连续输入。
        let blocked_ext = engine.should_debounce("hello world more");
        assert!(blocked_ext, "前缀延伸应被防抖挡掉");
    }

    #[tokio::test]
    async fn debounce_expires_after_window() {
        let ollama = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_millis(50),
        ));
        let engine = InlineCompletionEngine::new(ollama, "qwen2.5-coder:0.5b".to_string());

        // 手动设置 last_call 为窗口外的时间,模拟 "300ms 已过"。
        *engine.last_call.lock() = Some(Instant::now() - Duration::from_millis(400));
        *engine.last_prefix.lock() = "hello world".to_string();

        let blocked = engine.should_debounce("hello world");
        assert!(!blocked, "超过 300ms 后不应再防抖");
    }

    // ---------- 失败静默测试 ----------

    #[tokio::test]
    async fn suggest_completion_silently_returns_none_on_ollama_error() {
        // 用不可达 URL + 短超时,确保 generate_with_options 必失败。
        let ollama = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_millis(50),
        ));
        let engine = InlineCompletionEngine::new(ollama, "qwen2.5-coder:0.5b".to_string());

        let result = engine
            .suggest_completion("a sufficiently long prefix")
            .await;
        // 失败静默:不返回 Err,而是 Ok(None)。
        assert!(result.is_ok(), "失败应静默,不返回 Err");
        assert_eq!(result.unwrap(), None);
    }

    // ---------- 辅助函数 ----------

    fn build_completion_prompt_for_test(prefix: &str) -> String {
        build_prompt(prefix)
    }
}
