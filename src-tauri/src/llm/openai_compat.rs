//! T-E-S-40 OpenAI 兼容层 — 一个可复用的 OpenAI 兼容客户端。
//!
//! 支持 vLLM / LMStudio / OpenRouter / DeepSeek 等任何实现了
//! `POST /v1/chat/completions`(或 `/chat/completions`)协议的端点。
//! 通过预设工厂快速接入常见服务商,也可通过 [`OpenAICompatClient::new`]
//! 自定义。
//!
//! 设计要点:
//! - **复用** `crate::llm::ollama::ChatMessage` 作为通用消息类型(避免转换)。
//! - **SSRF 校验内置**(对齐 `gateway::call_remote` 的 `SsrfGuard` 用法)。
//! - **解析 reasoning_content**(DeepSeek-R1 等推理模型返回字段)。
//! - 只实现非流式 `chat()`;流式 `chat_stream` 留待后续(T-E-S-40 标记 TODO)。

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::llm::ollama::ChatMessage;
use crate::security::SsrfGuard;

/// OpenAI 兼容 API 的 token 用量字段。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenAIUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

/// OpenAI 兼容 API 的非流式响应(只解析我们关心的字段)。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenAIChatResponse {
    #[serde(default)]
    pub choices: Vec<OpenAIChoice>,
    #[serde(default)]
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIChoice {
    pub message: OpenAIRespMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIRespMessage {
    pub role: String,
    pub content: String,
    /// DeepSeek-R1 等推理模型返回的推理内容(可选)。
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

/// T-E-S-40: OpenAI 兼容客户端。
///
/// 通过预设工厂([`OpenAICompatClient::deepseek`] / [`OpenAICompatClient::vllm`] /
/// [`OpenAICompatClient::lmstudio`] / [`OpenAICompatClient::openrouter`])
/// 快速接入常见服务商,也可通过 [`OpenAICompatClient::new`] 自定义。
#[derive(Clone)]
pub struct OpenAICompatClient {
    pub base_url: String,
    pub api_key: Option<String>,
    pub http: Client,
    pub default_model: String,
}

impl OpenAICompatClient {
    /// 自定义构造。`base_url` 应包含协议与主机(可含路径前缀,如
    /// `https://api.deepseek.com/v1`);末尾的 `/` 会被自动 trim。
    /// `api_key` 为 `None` 时不发送 `Authorization` 头(适配 LMStudio 本地)。
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        default_model: impl Into<String>,
    ) -> Self {
        // M7b #94: 用 SsrfGuard::build_safe_client 构建 http client,
        // 重定向链每跳校验目标 URL,阻止 SSRF。
        // allow_loopback=true 允许本地 LLM 端点(127.0.0.1,如 vLLM/LMStudio)。
        let http = SsrfGuard::new()
            .with_allow_loopback(true)
            .build_safe_client()
            .unwrap_or_else(|e| {
                tracing::warn!(
                    target: "nebula.ssrf",
                    error = %e,
                    "failed to build SSRF-safe client for OpenAI compat; falling back to default"
                );
                Client::builder()
                    .timeout(Duration::from_secs(120))
                    .build()
                    .expect("reqwest client should build")
            });
        Self {
            base_url: base_url.into(),
            api_key,
            http,
            default_model: default_model.into(),
        }
    }

    /// DeepSeek 官方 API 预设。`url` 通常为 `https://api.deepseek.com/v1`。
    pub fn deepseek(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self::new(url, Some(key.into()), "deepseek-chat")
    }

    /// vLLM 预设。`url` 通常为 `http://localhost:8000/v1`(本地点,
    /// SSRF 校验需 `allow_private`,此处由调用方自行负责 —— vLLM 部署
    /// 通常在可信内网)。
    pub fn vllm(url: impl Into<String>, key: impl Into<String>) -> Self {
        let k = key.into();
        Self::new(
            url,
            if k.is_empty() { None } else { Some(k) },
            "default-model",
        )
    }

    /// LMStudio 本地预设。LMStudio 通常监听 `http://localhost:1234/v1`,
    /// 不需要 API key。
    pub fn lmstudio(url: impl Into<String>) -> Self {
        Self::new(url, None, "local-model")
    }

    /// OpenRouter 预设。`key` 为 OpenRouter API key(sk-or-...)。
    /// base_url 固定为 `https://openrouter.ai/api/v1`。
    pub fn openrouter(key: impl Into<String>) -> Self {
        Self::new(
            "https://openrouter.ai/api/v1",
            Some(key.into()),
            "openai/gpt-4o-mini",
        )
    }

    /// 返回默认模型名。
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// 非流式 chat 调用。返回 `(text, prompt_tokens, completion_tokens)`。
    ///
    /// - 复用 `ollama::ChatMessage` 作为输入(项目通用类型)。
    /// - 内置 SSRF 校验(对齐 `gateway::call_remote` 的 SsrfGuard 用法)。
    /// - 解析 `reasoning_content`(DeepSeek-R1 等推理模型返回字段),
    ///   仅把正文 content 返回给调用方,reasoning 不在本次范围内透传。
    pub async fn chat(&self, model: &str, msgs: &[ChatMessage]) -> Result<(String, u64, u64)> {
        // M7b #94: SSRF 校验 — 用 with_allow_loopback 替代 with_allow_private。
        // 允许 loopback(127.0.0.1,本地 vLLM/LMStudio)但拒绝其他私网(10.x/172.16.x/192.168.x)。
        // self.http 已在构造时用 build_safe_client 构建(重定向链每跳校验)。
        let ssrf_guard = SsrfGuard::new().with_allow_loopback(true);
        let url = self.completions_url();
        ssrf_guard
            .validate_url(&url)
            .map_err(|e| anyhow!("SSRF validation failed: {e}"))?;

        let req_body = serde_json::json!({
            "model": model,
            "messages": msgs.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
            "stream": false,
        });

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&req_body);
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("openai-compat request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("openai-compat API error: {status} - {body}"));
        }
        let resp_json: OpenAIChatResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("openai-compat response parse failed: {e}"))
            .context("parse openai-compat chat response")?;

        let content = resp_json
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        let (prompt_tokens, completion_tokens) = match resp_json.usage {
            Some(u) => (u.prompt_tokens, u.completion_tokens),
            None => (0, 0),
        };

        // 透传 token 用量到全局 metrics(对齐 gateway::call_deepseek)。
        crate::metrics::global().record_token_usage(prompt_tokens, completion_tokens);

        Ok((content, prompt_tokens, completion_tokens))
    }

    /// 拼接 `/chat/completions` 端点。
    /// 若 base_url 已含 `/v1` 前缀则直接追加 `/chat/completions`,
    /// 否则追加 `/v1/chat/completions`(对齐 OpenAI 官方路径)。
    pub fn completions_url(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.ends_with("/v1") {
            format!("{trimmed}/chat/completions")
        } else {
            format!("{trimmed}/v1/chat/completions")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_deepseek_uses_key() {
        let c = OpenAICompatClient::deepseek("https://api.deepseek.com/v1", "sk-test-xxx");
        assert_eq!(c.base_url, "https://api.deepseek.com/v1");
        assert_eq!(c.api_key.as_deref(), Some("sk-test-xxx"));
        assert_eq!(c.default_model, "deepseek-chat");
    }

    #[test]
    fn preset_vllm_empty_key_becomes_none() {
        let c = OpenAICompatClient::vllm("http://localhost:8000/v1", "");
        assert_eq!(c.base_url, "http://localhost:8000/v1");
        assert!(c.api_key.is_none());
    }

    #[test]
    fn preset_vllm_with_key_keeps_key() {
        let c = OpenAICompatClient::vllm("http://localhost:8000/v1", "token-abc");
        assert_eq!(c.api_key.as_deref(), Some("token-abc"));
    }

    #[test]
    fn preset_lmstudio_no_key() {
        let c = OpenAICompatClient::lmstudio("http://localhost:1234/v1");
        assert_eq!(c.base_url, "http://localhost:1234/v1");
        assert!(c.api_key.is_none());
        assert_eq!(c.default_model, "local-model");
    }

    #[test]
    fn preset_openrouter_uses_official_url() {
        let c = OpenAICompatClient::openrouter("sk-or-xxx");
        assert_eq!(c.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(c.api_key.as_deref(), Some("sk-or-xxx"));
        assert_eq!(c.default_model, "openai/gpt-4o-mini");
    }

    #[test]
    fn completions_url_with_v1_prefix() {
        let c = OpenAICompatClient::deepseek("https://api.deepseek.com/v1", "k");
        assert_eq!(
            c.completions_url(),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn completions_url_without_v1_prefix_appends_v1() {
        let c = OpenAICompatClient::new("https://api.example.com", None, "m");
        assert_eq!(
            c.completions_url(),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn completions_url_trims_trailing_slash() {
        let c = OpenAICompatClient::new("https://api.example.com/v1/", None, "m");
        assert_eq!(
            c.completions_url(),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn usage_parsing_default_zero() {
        // 缺失 usage 字段时回退到 (0, 0)。
        let json = r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#;
        let resp: OpenAIChatResponse = serde_json::from_str(json).unwrap();
        assert!(resp.usage.is_none());
    }

    #[test]
    fn usage_parsing_with_tokens() {
        let json = r#"{
            "choices":[{"message":{"role":"assistant","content":"hi"}}],
            "usage":{"prompt_tokens":12,"completion_tokens":3}
        }"#;
        let resp: OpenAIChatResponse = serde_json::from_str(json).unwrap();
        let u = resp.usage.unwrap();
        assert_eq!(u.prompt_tokens, 12);
        assert_eq!(u.completion_tokens, 3);
    }

    #[test]
    fn reasoning_content_parsing() {
        // DeepSeek-R1 风格响应:含 reasoning_content。
        let json = r#"{
            "choices":[{
                "message":{
                    "role":"assistant",
                    "content":"final answer",
                    "reasoning_content":"let me think step by step..."
                }
            }],
            "usage":{"prompt_tokens":5,"completion_tokens":2}
        }"#;
        let resp: OpenAIChatResponse = serde_json::from_str(json).unwrap();
        let msg = &resp.choices[0].message;
        assert_eq!(msg.content, "final answer");
        assert_eq!(
            msg.reasoning_content.as_deref(),
            Some("let me think step by step...")
        );
    }

    #[test]
    fn reasoning_content_optional() {
        // 普通模型不返回 reasoning_content。
        let json = r#"{
            "choices":[{"message":{"role":"assistant","content":"hello"}}],
            "usage":{"prompt_tokens":1,"completion_tokens":1}
        }"#;
        let resp: OpenAIChatResponse = serde_json::from_str(json).unwrap();
        assert!(resp.choices[0].message.reasoning_content.is_none());
    }
}
