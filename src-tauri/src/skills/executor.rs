//! T-E-S-36 执行层 — SkillExecutor trait + Local / Remote / Mcp 实现。
//!
//! 每种传输方式独立实现 [`SkillExecutor`],错误处理隔离。`SkillEngine`
//! 门面根据 [`super::protocol::SkillTransport`] 选择对应执行器。
//!
//! * [`LocalExecutor`] — in-process 执行。本期内置 `echo` skill(原样
//!   返回 input),未来委派给既有 sandbox([`super::sandbox`])。
//! * [`RemoteExecutor`] — HTTP 执行,用 [`crate::security::SsrfGuard`]
//!   校验目标 URL,拒绝私有地址(127.0.0.1 / 169.254 / 10.0.0.0/8 /
//!   172.16.0.0/12 / 192.168.0.0/16)。
//! * [`McpExecutor`] — MCP protocol stub,本期返回 `NotImplemented`。

use std::time::Instant;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::protocol::{SkillRequest, SkillResponse};
use crate::security::SsrfGuard;

/// Skill 执行器 trait。
///
/// 三种实现:`LocalExecutor` / `RemoteExecutor` / `McpExecutor`。
/// `async fn` 通过 `async_trait` 派遣,支持 `dyn SkillExecutor`。
#[async_trait]
pub trait SkillExecutor: Send + Sync {
    /// 执行一个 [`SkillRequest`],返回 [`SkillResponse`]。
    async fn execute(&self, req: SkillRequest) -> Result<SkillResponse>;
}

// ---------------------------------------------------------------------------
// LocalExecutor
// ---------------------------------------------------------------------------

/// 本地 in-process 执行器。
///
/// 本期内置 `echo` skill(原样返回 input)。未来可委派给既有 sandbox
/// ([`super::sandbox`])或 `SkillEngine::use_skill` 路径。
pub struct LocalExecutor;

impl LocalExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LocalExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SkillExecutor for LocalExecutor {
    async fn execute(&self, req: SkillRequest) -> Result<SkillResponse> {
        let start = Instant::now();
        // 内置 echo skill:原样返回 input。
        if req.skill == "echo" {
            return Ok(SkillResponse {
                output: req.input,
                error: None,
                latency_ms: start.elapsed().as_millis() as u64,
            });
        }
        // 未知 skill:返回错误响应(不返回 Err,以便调用方区分"执行失败"
        // 与"skill 不存在")。
        Ok(SkillResponse {
            output: serde_json::Value::Null,
            error: Some(format!("unknown local skill: {}", req.skill)),
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// RemoteExecutor
// ---------------------------------------------------------------------------

/// 远程 HTTP 执行器。
///
/// 用 [`SsrfGuard`] 校验目标 URL,拒绝私有地址。本期仅做校验 + 返回
/// 验证摘要,不实际发起 HTTP 请求(避免测试依赖网络)。未来接入真实
/// HTTP 调用时,`reqwest::Client` 已由 [`SsrfGuard::build_safe_client`]
/// 构造,重定向链每跳都会 SSRF 校验。
pub struct RemoteExecutor {
    #[allow(dead_code)] // 本期仅用于校验,未来发起真实请求时使用。
    client: reqwest::Client,
    ssrf_guard: SsrfGuard,
}

impl RemoteExecutor {
    /// 构造远程执行器(SSRF 安全客户端)。
    pub fn new() -> Result<Self> {
        let guard = SsrfGuard::new();
        let client = guard.build_safe_client()?;
        Ok(Self {
            client,
            ssrf_guard: guard,
        })
    }

    /// 校验 URL 是否安全(拒绝私有地址)。
    pub fn validate_url(&self, url: &str) -> Result<()> {
        self.ssrf_guard.validate_url(url)
    }
}

impl Default for RemoteExecutor {
    fn default() -> Self {
        Self::new().expect("RemoteExecutor::new must succeed with default SsrfGuard")
    }
}

#[async_trait]
impl SkillExecutor for RemoteExecutor {
    async fn execute(&self, req: SkillRequest) -> Result<SkillResponse> {
        let start = Instant::now();
        // 从 input 中读取 url 字段(简化协议)。
        let url = req
            .input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("RemoteExecutor: input.url is required"))?;

        // SSRF 校验:拒绝私有地址。
        self.ssrf_guard.validate_url(url)?;

        // 本期不实际发起 HTTP 请求(避免测试依赖网络)。
        Ok(SkillResponse {
            output: serde_json::json!({
                "status": "validated",
                "url": url,
                "skill": req.skill,
            }),
            error: None,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// McpExecutor
// ---------------------------------------------------------------------------

/// MCP 协议执行器(stub)。
///
/// 本期返回 `NotImplemented`。未来接入真实 MCP client(见
/// [`crate::mcp`])后,`server` 字段标识目标 MCP server。
pub struct McpExecutor {
    server: String,
}

impl McpExecutor {
    pub fn new(server: impl Into<String>) -> Self {
        Self {
            server: server.into(),
        }
    }

    pub fn server(&self) -> &str {
        &self.server
    }
}

#[async_trait]
impl SkillExecutor for McpExecutor {
    async fn execute(&self, req: SkillRequest) -> Result<SkillResponse> {
        // T-E-S-36: MCP protocol stub,本期返回 NotImplemented。
        Ok(SkillResponse {
            output: serde_json::Value::Null,
            error: Some(format!(
                "McpExecutor: MCP protocol not implemented (server={}, skill={})",
                self.server, req.skill
            )),
            latency_ms: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_executor_echo() {
        // echo skill:原样返回 input。
        let executor = LocalExecutor::new();
        let req = SkillRequest {
            skill: "echo".to_string(),
            input: serde_json::json!({"text": "hello", "n": 42}),
            timeout_ms: 1000,
        };
        let resp = executor.execute(req).await.expect("task should complete");
        assert!(resp.error.is_none(), "echo should succeed");
        assert_eq!(resp.output, serde_json::json!({"text": "hello", "n": 42}));
        assert!(resp.latency_ms < 1000, "echo should be fast");
    }

    #[tokio::test]
    async fn test_local_executor_unknown_skill() {
        // 未知 skill:返回 error 字段(不返回 Err)。
        let executor = LocalExecutor::new();
        let req = SkillRequest {
            skill: "nonexistent".to_string(),
            input: serde_json::Value::Null,
            timeout_ms: 1000,
        };
        let resp = executor.execute(req).await.expect("task should complete");
        assert!(resp.error.is_some(), "unknown skill should set error");
        assert!(resp
            .error
            .expect("assertion value")
            .contains("unknown local skill"));
    }

    #[tokio::test]
    async fn test_remote_executor_rejects_private_address() {
        // SSRF 校验:拒绝 127.0.0.1 / 192.168 / 10 / 169.254 / 172.16-31。
        let executor = RemoteExecutor::new().expect("create should succeed");

        let private_addrs = [
            "http://127.0.0.1/api",
            "http://192.168.1.1/api",
            "http://10.0.0.1/api",
            "http://169.254.169.254/latest/meta-data/",
            "http://172.16.0.1/api",
            "http://172.31.255.255/api",
        ];
        for url in private_addrs {
            let req = SkillRequest {
                skill: "fetch".to_string(),
                input: serde_json::json!({"url": url}),
                timeout_ms: 1000,
            };
            let result = executor.execute(req).await;
            assert!(
                result.is_err(),
                "RemoteExecutor must reject private address: {url}"
            );
            let msg = format!("{}", result.unwrap_err());
            assert!(
                msg.contains("SSRF") || msg.contains("not allowed"),
                "expected SSRF rejection for {url}, got: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_remote_executor_requires_url_field() {
        // input 缺少 url 字段:返回 Err。
        let executor = RemoteExecutor::new().expect("create should succeed");
        let req = SkillRequest {
            skill: "fetch".to_string(),
            input: serde_json::json!({"text": "no url"}),
            timeout_ms: 1000,
        };
        let result = executor.execute(req).await;
        assert!(result.is_err(), "missing url should return Err");
    }

    #[tokio::test]
    async fn test_mcp_executor_returns_not_implemented() {
        // MCP stub:返回 NotImplemented 错误字段。
        let executor = McpExecutor::new("test-server");
        let req = SkillRequest {
            skill: "mcp-tool".to_string(),
            input: serde_json::Value::Null,
            timeout_ms: 1000,
        };
        let resp = executor.execute(req).await.expect("task should complete");
        assert!(resp.error.is_some(), "McpExecutor should set error");
        let err = resp.error.expect("test op should succeed");
        assert!(
            err.contains("not implemented"),
            "expected NotImplemented: {err}"
        );
        assert!(err.contains("test-server"));
    }
}
