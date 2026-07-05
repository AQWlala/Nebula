//! T-E-S-36 协议层 — SkillManifest / SkillRequest / SkillResponse。
//!
//! 跨传输无关的 Skill 协议定义。本模块只描述"什么是 skill / 如何请求 /
//! 如何响应",不关心具体执行方式(本地 / 远程 / MCP)。执行由
//! [`super::executor`] 负责,能力发现由 [`super::capability`] 负责。
//!
//! ## 设计
//!
//! * [`SkillManifest`] — 声明式描述一个 skill 的元数据与传输方式。
//!   未来静态内置 skill 可在编译时 `include_str!` SKILL.md 后反序列化
//!   为 `SkillManifest`。
//! * [`SkillTransport`] — 三种传输变体(`Local` / `Remote{url}` / `Mcp{server}`),
//!   `#[serde(rename_all = "snake_case")]` 保证向后兼容。
//! * [`SkillRequest`] / [`SkillResponse`] — 跨传输的请求 / 响应信封。

use serde::{Deserialize, Serialize};

/// Skill 清单:声明式描述一个 skill 的元数据与传输方式。
///
/// 与 [`super::types::Skill`] 的区别:`Skill` 是持久化到 SQLite 的完整
/// 记录(含 `code` / `tags` / `usage_count` 等),`SkillManifest` 是跨
/// 传输的轻量描述(仅 name/version/description/capabilities/transport)。
/// 未来 exporter / publisher 可将 `Skill` 转换为 `SkillManifest` 后再
/// 序列化,本期仅在协议层定义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// 声明式能力标签(如 `["file:read", "network"]`)。与
    /// [`super::capability::Capability`] 的 `id` 对应,用于能力反向映射。
    pub capabilities: Vec<String>,
    pub transport: SkillTransport,
}

/// Skill 传输方式。
///
/// `#[serde(rename_all = "snake_case")]` 保证枚举变体名在序列化时为
/// 小写蛇形,与 agentskills.io / MCP 社区惯例一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillTransport {
    /// 本地 in-process 执行(委派给 [`super::executor::LocalExecutor`] /
    /// 既有 sandbox)。
    Local,
    /// 远程 HTTP 执行(委派给 [`super::executor::RemoteExecutor`] +
    /// SSRF 校验)。
    Remote { url: String },
    /// MCP 协议执行(委派给 [`super::executor::McpExecutor`])。
    Mcp { server: String },
}

/// Skill 执行请求(跨传输信封)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRequest {
    /// 目标 skill 名称(与 [`SkillManifest::name`] 对应)。
    pub skill: String,
    /// 输入载荷(任意 JSON 值)。RemoteExecutor 期望包含 `url` 字段。
    pub input: serde_json::Value,
    /// 超时(毫秒)。0 表示用执行器默认超时。
    pub timeout_ms: u32,
}

/// Skill 执行响应(跨传输信封)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    /// 输出载荷(任意 JSON 值)。
    pub output: serde_json::Value,
    /// 错误信息。`None` 表示成功。
    pub error: Option<String>,
    /// 端到端延迟(毫秒)。
    pub latency_ms: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_manifest_local_transport_round_trip() {
        // Local 变体:序列化为 "local" 字符串,反序列化还原。
        let m = SkillManifest {
            name: "echo".to_string(),
            version: "1.0.0".to_string(),
            description: "echo input back".to_string(),
            capabilities: vec!["io".to_string()],
            transport: SkillTransport::Local,
        };
        let j = serde_json::to_string(&m).unwrap();
        // transport 应序列化为 "local"(snake_case)。
        assert!(
            j.contains("\"transport\":\"local\""),
            "expected snake_case 'local', got: {j}"
        );
        let m2: SkillManifest = serde_json::from_str(&j).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_skill_manifest_remote_transport_round_trip() {
        // Remote 变体:序列化为 {"remote":{"url":...}},反序列化还原。
        let m = SkillManifest {
            name: "fetch".to_string(),
            version: "2.0.0".to_string(),
            description: "fetch a remote url".to_string(),
            capabilities: vec!["network".to_string()],
            transport: SkillTransport::Remote {
                url: "https://api.example.com/skill".to_string(),
            },
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"remote\""), "expected 'remote' tag, got: {j}");
        assert!(j.contains("https://api.example.com/skill"));
        let m2: SkillManifest = serde_json::from_str(&j).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_skill_manifest_mcp_transport_round_trip() {
        // Mcp 变体:序列化为 {"mcp":{"server":...}},反序列化还原。
        let m = SkillManifest {
            name: "mcp-tool".to_string(),
            version: "0.1.0".to_string(),
            description: "mcp protocol tool".to_string(),
            capabilities: vec!["mcp".to_string()],
            transport: SkillTransport::Mcp {
                server: "mcp-server-1".to_string(),
            },
        };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"mcp\""), "expected 'mcp' tag, got: {j}");
        assert!(j.contains("mcp-server-1"));
        let m2: SkillManifest = serde_json::from_str(&j).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_skill_request_response_serde() {
        // 请求 / 响应信封应可无损往返。
        let req = SkillRequest {
            skill: "echo".to_string(),
            input: serde_json::json!({"text": "hello"}),
            timeout_ms: 5000,
        };
        let req_j = serde_json::to_string(&req).unwrap();
        let req2: SkillRequest = serde_json::from_str(&req_j).unwrap();
        assert_eq!(req.skill, req2.skill);
        assert_eq!(req.input, req2.input);
        assert_eq!(req.timeout_ms, req2.timeout_ms);

        let resp = SkillResponse {
            output: serde_json::json!({"echo": "hello"}),
            error: None,
            latency_ms: 42,
        };
        let resp_j = serde_json::to_string(&resp).unwrap();
        let resp2: SkillResponse = serde_json::from_str(&resp_j).unwrap();
        assert_eq!(resp.output, resp2.output);
        assert_eq!(resp.error, resp2.error);
        assert_eq!(resp.latency_ms, resp2.latency_ms);
    }
}
