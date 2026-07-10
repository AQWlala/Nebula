//! T-E-A-03 ModelRouter 智能路由 — 本地小模型分类,决定首选 LLM provider.
//!
//! ## M3 #47 迁移说明
//!
//! 分类器**优先**走 [`UnifiedModelDispatcher::dispatch`](`crate::llm::dispatcher::UnifiedModelDispatcher::dispatch`)
//! (`WorkType::Classifier`),享受 ModelPolicy + 断路器 + 限流 + 缓存 +
//! 成本统计 + `models.json` `work_type_overrides` 用户自定义。
//!
//! 由于 `WorkType::Classifier` 在 `is_local_only()` 中返回 `true`,
//! dispatcher 会强制走本地路径(忽略远端 override),仍然保证零远端成本。
//!
//! 当 dispatcher 未注入时(旧路径,向后兼容),回退直连
//! [`OllamaClient::chat`](crate::llm::ollama::OllamaClient::chat)
//! (默认 `qwen2.5:3b`),不经过 LlmGateway/CostTracker。
//!
//! 失败静默降级为 [`Route::DeepSeek`],不阻断主流程.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use parking_lot::Mutex;
#[allow(unused_imports)]
use tracing::warn;

use crate::llm::ollama::{ChatMessage, OllamaClient};

use crate::llm::dispatcher::{UnifiedModelDispatcher, WorkType};

/// T-E-A-03: 任务复杂度路由结果,决定首选 LLM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// 简单任务 → 本地免费模型.
    Ollama,
    /// 中等任务 → DeepSeek-chat(默认 provider).
    DeepSeek,
    /// 复杂任务 → Anthropic Claude(若配置).
    Anthropic,
    /// 复杂任务 fallback → OpenAI 兼容远程端点.
    Remote,
}

/// T-E-A-03: 本地小模型分类器,根据消息复杂度路由到不同 provider.
///
/// M3 #47: 优先使用注入的 `UnifiedModelDispatcher`(走 `dispatch(Classifier)`),
/// 否则回退直连 `OllamaClient::chat`(向后兼容).
pub struct ModelRouter {
    /// 旧路径回退用 — dispatcher 未注入时使用.
    ollama: Arc<OllamaClient>,
    /// 旧路径使用的分类器模型名(通常 `qwen2.5:3b`).
    classifier_model: String,
    /// M3 #47: 注入的统一调度器(优先路径)。
    dispatcher: Option<Arc<UnifiedModelDispatcher>>,
    cache: Mutex<HashMap<u64, Route>>,
}

impl ModelRouter {
    /// 创建路由器(旧路径)。`classifier_model` 通常为 `qwen2.5:3b`。
    ///
    /// M3 #47: 推荐改用 [`ModelRouter::with_dispatcher`] 注入调度器以
    /// 走 `dispatch(Classifier)` 路径。
    pub fn new(ollama: Arc<OllamaClient>, classifier_model: String) -> Self {
        Self {
            ollama,
            classifier_model,
            dispatcher: None,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// M3 #47: 注入 `UnifiedModelDispatcher`,启用 dispatcher 优先路径。
    ///
    /// 启用后,`classify()` 会先调 `dispatch(WorkType::Classifier, messages)`。
    /// dispatcher 失败/未注入时回退到直连 Ollama 路径。
    pub fn with_dispatcher(mut self, dispatcher: Arc<UnifiedModelDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// M3 #47: 是否已注入 dispatcher(主要用于测试与诊断)。
    pub fn has_dispatcher(&self) -> bool {
        self.dispatcher.is_some()
    }

    /// 主入口:对消息序列分类,返回首选 provider 路由.
    ///
    /// 流程:
    /// 1. 取最后一条 user 消息;若无非 user,返回 [`Route::DeepSeek`].
    /// 2. 哈希消息序列作为缓存 key.
    /// 3. 缓存命中直接返回.
    /// 4. 构造分类 prompt(system + 最后一条 user).
    /// 5. M3 #47: 若 dispatcher 注入,调 `dispatch(Classifier, prompt)`;
    ///    否则回退 `ollama.chat` — 失败静默降级 [`Route::DeepSeek`](不缓存).
    /// 6. 解析 `response.message.content.to_lowercase().trim()`.
    /// 7. `parse_route` → `Some` 缓存并返回;`None` 降级 [`Route::DeepSeek`](不缓存).
    pub async fn classify(&self, messages: &[ChatMessage]) -> Route {
        // 1. 取最后一条 user 消息;若无非 user,返回默认.
        let has_user = messages.iter().rev().any(|m| m.role == "user");
        if !has_user {
            return Route::DeepSeek;
        }

        // 2. 哈希消息序列作为缓存 key.
        let key = Self::hash_messages(messages);

        // 3. 缓存命中直接返回.
        if let Some(route) = self.cache.lock().get(&key) {
            return *route;
        }

        // 4. 构造分类 prompt(system + 最后一条 user).
        let prompt = Self::build_classifier_prompt(messages);

        // 5. M3 #47: 优先走 dispatcher;失败/未注入回退直连 ollama.
        let content_opt = match self.classify_via_dispatcher(&prompt).await {
            Some(c) => Some(c),
            None => match self.classify_via_ollama(&prompt).await {
                Some(c) => Some(c),
                None => return Route::DeepSeek, // 失败降级,不缓存
            },
        };

        // 6/7. 解析 → Some 缓存并返回;None 降级 DeepSeek(不缓存).
        match content_opt.and_then(|c| Self::parse_route(&c)) {
            Some(route) => {
                self.cache.lock().insert(key, route);
                route
            }
            None => Route::DeepSeek,
        }
    }

    /// M3 #47: dispatcher 路径 — 调 `dispatch(WorkType::Classifier, prompt)`。
    ///
    /// 返回 `Some(lowercased_content)` 表示成功;`None` 表示 dispatcher
    /// 未注入或调用失败(此时上层回退到直连 ollama 路径)。
    async fn classify_via_dispatcher(&self, prompt: &[ChatMessage]) -> Option<String> {
        let dispatcher = self.dispatcher.as_ref()?;
        match dispatcher
            .dispatch(WorkType::Classifier, prompt.to_vec())
            .await
        {
            Ok(resp) => Some(resp.message.content.to_lowercase().trim().to_string()),
            Err(e) => {
                warn!(
                    target: "nebula.llm.model_router",
                    error = %e,
                    "dispatcher classify failed, falling back to direct ollama"
                );
                None
            }
        }
    }

    /// 直连 Ollama 路径(向后兼容)。
    ///
    /// 返回 `Some(lowercased_content)` 表示成功;`None` 表示失败。
    async fn classify_via_ollama(&self, prompt: &[ChatMessage]) -> Option<String> {
        match self.ollama.chat(&self.classifier_model, prompt).await {
            Ok(resp) => Some(resp.message.content.to_lowercase().trim().to_string()),
            Err(_) => None,
        }
    }

    /// 解析分类器输出为路由. 匹配前缀 `simpl`/`medi`/`compl`
    /// (大小写不敏感). 不匹配返回 `None`.
    ///
    /// - "simple" / "simpler" / "SIMPLE" → [`Route::Ollama`]
    /// - "medium" / "mediumly" / "Medium" → [`Route::DeepSeek`]
    /// - "complex" / "Complexity" → [`Route::Anthropic`]
    pub fn parse_route(s: &str) -> Option<Route> {
        let s = s.trim().to_lowercase();
        if s.starts_with("simpl") {
            Some(Route::Ollama)
        } else if s.starts_with("medi") {
            Some(Route::DeepSeek)
        } else if s.starts_with("compl") {
            Some(Route::Anthropic)
        } else {
            None
        }
    }

    /// 哈希消息序列(用 `DefaultHasher`). 相同 messages 产生相同 key.
    pub fn hash_messages(messages: &[ChatMessage]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for m in messages {
            m.role.hash(&mut hasher);
            m.content.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// 构造分类器 prompt:system 描述 + 最后一条 user 消息.
    pub fn build_classifier_prompt(messages: &[ChatMessage]) -> Vec<ChatMessage> {
        let system = ChatMessage::system(
            "You are a task complexity classifier. Output ONLY one word: simple, medium, or complex.\n\
             - simple: greeting, factual recall, translation, formatting, short Q&A\n\
             - medium: summarization, rewriting, multi-turn Q&A, code review\n\
             - complex: reasoning, creative writing, complex coding, multi-step planning",
        );
        let last_user = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        vec![system, ChatMessage::user(last_user)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_route_simple() {
        assert_eq!(ModelRouter::parse_route("simple"), Some(Route::Ollama));
    }

    #[test]
    fn test_parse_route_medium() {
        assert_eq!(ModelRouter::parse_route("medium"), Some(Route::DeepSeek));
    }

    #[test]
    fn test_parse_route_complex() {
        assert_eq!(ModelRouter::parse_route("complex"), Some(Route::Anthropic));
    }

    #[test]
    fn test_parse_route_case_insensitive() {
        assert_eq!(ModelRouter::parse_route("SIMPLE"), Some(Route::Ollama));
        assert_eq!(ModelRouter::parse_route("Simple"), Some(Route::Ollama));
        assert_eq!(ModelRouter::parse_route("MEDIUM"), Some(Route::DeepSeek));
        assert_eq!(ModelRouter::parse_route("Complex"), Some(Route::Anthropic));
    }

    #[test]
    fn test_parse_route_prefix_match() {
        // 前缀匹配(starts_with) — 模型可能输出 "simpler" / "mediumly" 等变体.
        assert_eq!(ModelRouter::parse_route("simpler"), Some(Route::Ollama));
        assert_eq!(ModelRouter::parse_route("mediumly"), Some(Route::DeepSeek));
        assert_eq!(
            ModelRouter::parse_route("complexity"),
            Some(Route::Anthropic)
        );
    }

    #[test]
    fn test_parse_route_unknown() {
        assert_eq!(ModelRouter::parse_route("unknown"), None);
        assert_eq!(ModelRouter::parse_route(""), None);
        assert_eq!(ModelRouter::parse_route("hello world"), None);
    }

    #[test]
    fn test_hash_messages_consistent() {
        let msgs = vec![
            ChatMessage::system("you are helpful"),
            ChatMessage::user("hello there"),
        ];
        let h1 = ModelRouter::hash_messages(&msgs);
        let h2 = ModelRouter::hash_messages(&msgs);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_messages_differs_for_different_content() {
        let a = vec![ChatMessage::user("hello")];
        let b = vec![ChatMessage::user("world")];
        assert_ne!(
            ModelRouter::hash_messages(&a),
            ModelRouter::hash_messages(&b)
        );
    }

    #[test]
    fn test_build_classifier_prompt_has_system_and_user() {
        let messages = vec![
            ChatMessage::system("ctx"),
            ChatMessage::user("what is 2+2?"),
            ChatMessage::assistant("4"),
            ChatMessage::user("thanks!"),
        ];
        let prompt = ModelRouter::build_classifier_prompt(&messages);
        assert_eq!(prompt.len(), 2);
        assert_eq!(prompt[0].role, "system");
        assert!(prompt[0].content.contains("complexity classifier"));
        assert_eq!(prompt[1].role, "user");
        // 取最后一条 user 消息.
        assert_eq!(prompt[1].content, "thanks!");
    }

    // 缓存命中路径不调 ollama — 用不可达端点构造 router,手动注入 cache.
    #[tokio::test]
    async fn test_classify_cache_hit() {
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1"));
        let router = ModelRouter::new(ollama, "qwen2.5:3b".to_string());
        let messages = vec![ChatMessage::user("hello")];
        let key = ModelRouter::hash_messages(&messages);
        // 手动注入 cache → classify 应直接返回,不发起 HTTP.
        router.cache.lock().insert(key, Route::Ollama);
        let route = router.classify(&messages).await;
        assert_eq!(route, Route::Ollama);
    }

    // 无 user 消息 → 快速返回默认 Route::DeepSeek,不调 ollama.
    #[tokio::test]
    async fn test_classify_no_user_message_returns_default() {
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1"));
        let router = ModelRouter::new(ollama, "qwen2.5:3b".to_string());
        let messages = vec![ChatMessage::system("only system message")];
        let route = router.classify(&messages).await;
        assert_eq!(route, Route::DeepSeek);
    }

    // M3 #47: 旧路径(dispatcher 未注入)失败时降级 Route::DeepSeek,不缓存。
    #[tokio::test]
    async fn test_classify_ollama_failure_falls_back_to_deepseek() {
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1"));
        let router = ModelRouter::new(ollama, "qwen2.5:3b".to_string());
        let messages = vec![ChatMessage::user("hello")];
        let route = router.classify(&messages).await;
        assert_eq!(route, Route::DeepSeek);
        // 失败结果不应缓存 — 第二次 classify 仍可被 dispatcher 路径接管。
        assert!(router.cache.lock().is_empty());
    }

    // M3 #47: dispatcher 未注入时 has_dispatcher 返回 false。
    #[test]
    fn test_has_dispatcher_returns_false_without_dispatcher() {
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:1"));
        let router = ModelRouter::new(ollama, "qwen2.5:3b".to_string());
        assert!(!router.has_dispatcher());
    }
}
