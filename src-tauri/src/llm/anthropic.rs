//! v1.1 P0-1: Anthropic Claude HTTP client.
//!
//! Supports `claude-3-5-haiku`、`claude-3-5-sonnet`、`claude-3-opus`
//! 等模型。使用 Anthropic Messages API (`POST /v1/messages`)，
//! 并通过 `x-api-key` header 进行认证。

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

// T-E-A-04: chat() 返回值升级为 (String, AnthropicUsage),gateway.rs call_anthropic 需主 agent 适配。

/// Anthropic response usage statistics (prompt-caching aware).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// Minimum token count required to enable prefix caching for a system
/// prompt. Sonnet/Opus 门槛为 1024；Haiku 为 2048，这里取 1024 保守。
const MIN_CACHE_TOKENS: usize = 1024;

/// Anthropic message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// A single message in an Anthropic conversation.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// Response from the Claude API.
#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// Request payload for the Anthropic Messages API.
#[derive(Debug, Serialize)]
struct Request {
    model: String,
    messages: Vec<RequestMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<serde_json::Value>,
}

/// A message inside an Anthropic API request.
#[derive(Debug, Serialize)]
struct RequestMessage {
    role: String,
    content: String,
}

/// Anthropic Claude HTTP client.
#[derive(Clone)]
pub struct AnthropicClient {
    base_url: String,
    api_key: String,
    pub model: String,
    http: Client,
    prefix_cache_enabled: bool,
    min_cache_tokens: usize,
}

impl AnthropicClient {
    /// Creates a new client.
    ///
    /// # Arguments
    /// * `api_key` — Anthropic API key (`sk-ant-...`).
    /// * `model` — Model name, e.g. `claude-3-5-haiku-20241022`.
    /// * `base_url` — Override for proxy/self-hosted deployments.
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        let base_url = base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string());
        Self {
            base_url,
            api_key,
            model,
            http: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                // T-D-B-07: 字面量保证有效 — timeout 配置不会导致 build 失败,保留 expect
                .expect("reqwest client should build"),
            prefix_cache_enabled: true,
            min_cache_tokens: MIN_CACHE_TOKENS,
        }
    }

    /// T-E-A-04: Creates a new client with explicit prefix-cache
    /// configuration.
    ///
    /// # Arguments
    /// * `api_key` — Anthropic API key (`sk-ant-...`).
    /// * `model` — Model name, e.g. `claude-3-5-sonnet-20241022`.
    /// * `base_url` — Override for proxy/self-hosted deployments.
    /// * `prefix_cache_enabled` — Whether to inject `cache_control`
    ///   on long system prompts.
    /// * `min_cache_tokens` — Estimated-token threshold below which
    ///   prefix caching is skipped (use [`MIN_CACHE_TOKENS`] for the
    ///   default).
    pub fn new_with_cache_config(
        api_key: String,
        model: String,
        base_url: Option<String>,
        prefix_cache_enabled: bool,
        min_cache_tokens: usize,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string());
        Self {
            base_url,
            api_key,
            model,
            http: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                // T-D-B-07: 字面量保证有效 — timeout 配置不会导致 build 失败,保留 expect
                .expect("reqwest client should build"),
            prefix_cache_enabled,
            min_cache_tokens,
        }
    }
    /// Sends a chat request and returns the assistant's text response
    /// together with the prompt-caching-aware usage statistics.
    pub async fn chat(&self, messages: &[Message]) -> Result<(String, AnthropicUsage)> {
        let payload = self.build_request(messages);
        let url = format!("{}/v1/messages", self.base_url);
        let ssrf_guard = crate::security::SsrfGuard::new();
        ssrf_guard
            .validate_url(&url)
            .map_err(|e| anyhow::anyhow!("SSRF validation failed: {e}"))?;

        debug!(target: "nebula.llm", model = %self.model, "calling Anthropic API");

        let resp = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Anthropic HTTP request to {url} failed"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!(
                target: "nebula.llm",
                status = %status,
                body = %body,
                "Anthropic API error"
            );
            return Err(anyhow!("Anthropic API error {status}: {body}"));
        }

        let parsed: Response = resp
            .json()
            .await
            .with_context(|| "failed to parse Anthropic response")?;

        let text = parsed
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .filter_map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() {
            return Err(anyhow!("Anthropic returned no text content"));
        }

        let usage = parsed.usage.unwrap_or_default();

        debug!(target: "nebula.llm", chars = text.len(), "Anthropic response received");
        Ok((text, usage))
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the configured API key.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the configured model name.
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// Returns a reference to the underlying HTTP client.
    pub fn http(&self) -> &Client {
        &self.http
    }

    fn build_request(&self, messages: &[Message]) -> Request {
        let mut system_str: Option<String> = None;
        let msgs: Vec<RequestMessage> = messages
            .iter()
            .filter_map(|m| {
                let role_str = match m.role {
                    Role::System => {
                        // Anthropic uses a dedicated system field.
                        system_str = Some(m.content.clone());
                        return None;
                    }
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                Some(RequestMessage {
                    role: role_str.to_string(),
                    content: m.content.clone(),
                })
            })
            .collect();

        // T-E-A-04: 若启用 prefix cache 且 system 估算 token 数
        // (chars/2) 达到门槛,则用 content-block 数组形式注入
        // cache_control: ephemeral;否则保持纯字符串向后兼容。
        let system: Option<serde_json::Value> = system_str.map(|s| {
            if self.prefix_cache_enabled && s.chars().count() / 2 >= self.min_cache_tokens {
                serde_json::json!([
                    {
                        "type": "text",
                        "text": s,
                        "cache_control": { "type": "ephemeral" }
                    }
                ])
            } else {
                serde_json::Value::String(s)
            }
        });

        Request {
            model: self.model.clone(),
            messages: msgs,
            max_tokens: 4096,
            system,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_message_mapping() {
        let client = AnthropicClient::new(
            "sk-ant-test".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            None,
        );

        let messages = vec![
            Message {
                role: Role::System,
                content: "You are a helpful assistant.".to_string(),
            },
            Message {
                role: Role::User,
                content: "Hello, Claude.".to_string(),
            },
        ];

        let req = client.build_request(&messages);
        assert_eq!(req.model, "claude-3-5-haiku-20241022");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, "user");
        // 短 system 以纯字符串形式发送。
        assert_eq!(
            req.system,
            Some(serde_json::Value::String(
                "You are a helpful assistant.".to_string()
            ))
        );
    }

    #[test]
    fn test_build_request_short_system_no_cache_control() {
        let client = AnthropicClient::new(
            "sk-ant-test".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            None,
        );
        let short_system = "You are a helpful assistant.".to_string();
        let messages = vec![
            Message {
                role: Role::System,
                content: short_system.clone(),
            },
            Message {
                role: Role::User,
                content: "Hi".to_string(),
            },
        ];
        let req = client.build_request(&messages);
        let system = req.system.expect("system should be present");
        // 短 system (< 1024 tokens) 不注入 cache_control,以纯字符串发送。
        assert_eq!(system, serde_json::Value::String(short_system));
    }

    #[test]
    fn test_build_request_long_system_injects_cache_control() {
        let client = AnthropicClient::new(
            "sk-ant-test".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            None,
        );
        // 2100 chars / 2 = 1050 tokens >= 1024,触发 prefix cache。
        let long_system = "a".repeat(2100);
        let messages = vec![
            Message {
                role: Role::System,
                content: long_system.clone(),
            },
            Message {
                role: Role::User,
                content: "Hi".to_string(),
            },
        ];
        let req = client.build_request(&messages);
        let system = req.system.expect("system should be present");
        // 应为 content-block 数组形式,且包含 cache_control。
        assert!(system.is_array(), "system should be a content-block array");
        let arr = system.as_array().expect("test op should succeed");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], serde_json::json!("text"));
        assert_eq!(arr[0]["text"], serde_json::json!(long_system));
        assert_eq!(
            arr[0]["cache_control"]["type"],
            serde_json::json!("ephemeral")
        );
    }

    #[test]
    fn test_anthropic_usage_deserialize() {
        let json = r#"{
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": 200,
            "cache_read_input_tokens": 300
        }"#;
        let usage: AnthropicUsage = serde_json::from_str(json).expect("parse should succeed");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.cache_creation_input_tokens, 200);
        assert_eq!(usage.cache_read_input_tokens, 300);
    }

    #[test]
    fn test_anthropic_usage_default() {
        let json = r#"{}"#;
        let usage: AnthropicUsage = serde_json::from_str(json).expect("parse should succeed");
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.cache_creation_input_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
        // Default 派生也应全 0。
        let default = AnthropicUsage::default();
        assert_eq!(default.input_tokens, 0);
        assert_eq!(default.output_tokens, 0);
        assert_eq!(default.cache_creation_input_tokens, 0);
        assert_eq!(default.cache_read_input_tokens, 0);
    }
}
