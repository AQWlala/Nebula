use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport_type: McpTransportType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// T-E-S-32: 子进程参数列表(stdio 模式)。
    #[serde(default)]
    pub args: Vec<String>,
    /// T-E-S-32: 子进程额外环境变量(stdio 模式,叠加在 filter_safe_env_vars 白名单之上)。
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// T-E-S-31: SSE/HTTP 共用 API key(可选,Authorization: Bearer)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub tool_filter: Vec<String>,
    /// T-E-S-32: 是否由 supervisor 自动重启(默认 true)。
    #[serde(default = "default_true")]
    pub auto_restart: bool,
    /// T-E-S-32: 健康检查间隔(秒,supervisor 用 tools/list 心跳)。
    #[serde(default = "default_30")]
    pub health_check_interval_secs: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_30() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportType {
    Stdio,
    Http,
    /// T-E-S-31: SSE 长连接传输(POST + GET /sse)。
    Sse,
    /// T-E-S-34: 2025-03-26 Streamable HTTP transport(单一 endpoint)。
    /// POST 请求 → 响应可为 application/json 或 text/event-stream;
    /// 首次响应返回 Mcp-Session-Id,后续请求带该头。
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        /// 可选 session 持久化(首次响应后回填)。
        #[serde(default)]
        session_id: Option<String>,
    },
}

/// T-E-S-32: mcp_servers.json 顶层结构(仿 ModelsConfig 模式)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServersConfig {
    pub version: u32,
    pub servers: Vec<McpServerConfig>,
}

impl McpServersConfig {
    /// 内置默认配置(空 server 列表,version=1)。
    pub fn default_builtin() -> Self {
        Self {
            version: 1,
            servers: Vec::new(),
        }
    }

    /// 解析 mcp_servers.json 路径(`<app_data_dir>/mcp_servers.json`)。
    /// 失败时回退到当前目录(`./mcp_servers.json`)以保证测试可运行。
    pub fn resolve_path() -> PathBuf {
        match crate::backup::commands::resolve_app_data_dir() {
            Ok(dir) => dir.join("mcp_servers.json"),
            Err(e) => {
                warn!(
                    target: "nebula.mcp.config",
                    error = %e,
                    "resolve_app_data_dir failed; falling back to ./mcp_servers.json"
                );
                PathBuf::from("mcp_servers.json")
            }
        }
    }

    /// 从指定路径加载;文件不存在或解析失败时回退 `default_builtin()`。
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<McpServersConfig>(&bytes) {
                Ok(cfg) => {
                    debug!(
                        target: "nebula.mcp.config",
                        path = %path.display(),
                        servers = cfg.servers.len(),
                        "mcp_servers.json loaded"
                    );
                    cfg
                }
                Err(e) => {
                    warn!(
                        target: "nebula.mcp.config",
                        path = %path.display(),
                        error = %e,
                        "mcp_servers.json parse failed; falling back to default_builtin"
                    );
                    Self::default_builtin()
                }
            },
            Err(_) => {
                debug!(
                    target: "nebula.mcp.config",
                    path = %path.display(),
                    "mcp_servers.json not found; using default_builtin"
                );
                Self::default_builtin()
            }
        }
    }

    /// 写入到指定路径(创建父目录,pretty JSON)。
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir for {}", path.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(self).context("serializing mcp_servers.json")?;
        std::fs::write(path, bytes)
            .with_context(|| format!("writing mcp_servers.json to {}", path.display()))?;
        Ok(())
    }

    /// 校验:server name 唯一;transport_type 与 command/url 字段匹配。
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.servers {
            if !seen.insert(s.name.as_str()) {
                anyhow::bail!("duplicate MCP server name: {}", s.name);
            }
            match &s.transport_type {
                McpTransportType::Stdio => {
                    if s.command.is_none() {
                        anyhow::bail!("MCP server '{}' has stdio transport but no command", s.name);
                    }
                }
                McpTransportType::Http | McpTransportType::Sse => {
                    if s.url.is_none() {
                        anyhow::bail!(
                            "MCP server '{}' has {:?} transport but no url",
                            s.name,
                            s.transport_type
                        );
                    }
                }
                McpTransportType::StreamableHttp { url, .. } => {
                    if url.is_empty() {
                        anyhow::bail!(
                            "MCP server '{}' has StreamableHttp transport but empty url",
                            s.name
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serializes_roundtrip() {
        let cfg = McpServerConfig {
            name: "test-server".to_string(),
            transport_type: McpTransportType::Stdio,
            command: Some("npx".to_string()),
            args: vec![],
            env: HashMap::new(),
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec![],
            auto_restart: true,
            health_check_interval_secs: 30,
        };
        let json = serde_json::to_string(&cfg).expect("serialize should succeed");
        let parsed: McpServerConfig = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(parsed.name, "test-server");
        assert_eq!(parsed.transport_type, McpTransportType::Stdio);
    }

    /// T-E-S-31: Sse 变体序列化为 "sse"(rename_all = "lowercase")。
    #[test]
    fn sse_variant_serializes_as_lowercase() {
        let cfg = McpServerConfig {
            name: "sse-server".to_string(),
            transport_type: McpTransportType::Sse,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: Some("https://example.com/sse".to_string()),
            api_key: Some("secret".to_string()),
            enabled: true,
            tool_filter: vec![],
            auto_restart: true,
            health_check_interval_secs: 30,
        };
        let json = serde_json::to_string(&cfg).expect("serialize should succeed");
        assert!(json.contains("\"transport_type\":\"sse\""));
        let parsed: McpServerConfig = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(parsed.transport_type, McpTransportType::Sse);
        assert_eq!(parsed.url.as_deref(), Some("https://example.com/sse"));
        assert_eq!(parsed.api_key.as_deref(), Some("secret"));
    }

    /// T-E-S-31 + T-E-S-32: 完整 McpServerConfig 含 args/env 序列化往返。
    #[test]
    fn config_with_args_env_roundtrip() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let cfg = McpServerConfig {
            name: "stdio-full".to_string(),
            transport_type: McpTransportType::Stdio,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "@mcp/server".to_string()],
            env,
            url: None,
            api_key: None,
            enabled: false,
            tool_filter: vec!["fs_".to_string()],
            auto_restart: false,
            health_check_interval_secs: 60,
        };
        let json = serde_json::to_string(&cfg).expect("serialize should succeed");
        let parsed: McpServerConfig = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(parsed.args, vec!["-y", "@mcp/server"]);
        assert_eq!(parsed.env.get("FOO").map(|s| s.as_str()), Some("bar"));
        assert!(!parsed.enabled);
        assert!(!parsed.auto_restart);
        assert_eq!(parsed.health_check_interval_secs, 60);
    }

    /// T-E-S-32: 向后兼容旧格式(无 args/env/api_key/auto_restart/health_check 字段)。
    #[test]
    fn config_backward_compat_old_format() {
        let old_json = r#"{
            "name": "legacy",
            "transport_type": "stdio",
            "command": "npx",
            "url": null,
            "enabled": true,
            "tool_filter": []
        }"#;
        let parsed: McpServerConfig = serde_json::from_str(old_json).expect("parse should succeed");
        assert_eq!(parsed.name, "legacy");
        assert!(parsed.args.is_empty());
        assert!(parsed.env.is_empty());
        assert!(parsed.api_key.is_none());
        assert!(parsed.auto_restart);
        assert_eq!(parsed.health_check_interval_secs, 30);
    }

    /// T-E-S-32: McpServersConfig load/save 往返(tempfile)。
    #[test]
    fn mcp_servers_config_load_save_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().expect("create should succeed");
        let cfg = McpServersConfig {
            version: 1,
            servers: vec![McpServerConfig {
                name: "a".to_string(),
                transport_type: McpTransportType::Sse,
                command: None,
                args: vec![],
                env: HashMap::new(),
                url: Some("https://example.com/sse".to_string()),
                api_key: None,
                enabled: true,
                tool_filter: vec![],
                auto_restart: true,
                health_check_interval_secs: 30,
            }],
        };
        cfg.save(tmp.path()).expect("update should succeed");
        let loaded = McpServersConfig::load(tmp.path());
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers[0].name, "a");
        assert_eq!(loaded.servers[0].transport_type, McpTransportType::Sse);
    }

    /// T-E-S-32: McpServersConfig::validate 检测重复 name。
    #[test]
    fn mcp_servers_config_validate_rejects_duplicate_names() {
        let cfg = McpServersConfig {
            version: 1,
            servers: vec![
                McpServerConfig {
                    name: "dup".to_string(),
                    transport_type: McpTransportType::Stdio,
                    command: Some("echo".to_string()),
                    args: vec![],
                    env: HashMap::new(),
                    url: None,
                    api_key: None,
                    enabled: true,
                    tool_filter: vec![],
                    auto_restart: true,
                    health_check_interval_secs: 30,
                },
                McpServerConfig {
                    name: "dup".to_string(),
                    transport_type: McpTransportType::Stdio,
                    command: Some("echo".to_string()),
                    args: vec![],
                    env: HashMap::new(),
                    url: None,
                    api_key: None,
                    enabled: true,
                    tool_filter: vec![],
                    auto_restart: true,
                    health_check_interval_secs: 30,
                },
            ],
        };
        assert!(cfg.validate().is_err());
    }

    /// T-E-S-32: McpServersConfig::load 文件不存在时回退 default_builtin。
    #[test]
    fn mcp_servers_config_load_missing_file_falls_back() {
        let path = PathBuf::from("/nonexistent/path/that/does/not/exist/mcp_servers.json");
        let loaded = McpServersConfig::load(&path);
        assert_eq!(loaded.version, 1);
        assert!(loaded.servers.is_empty());
    }

    /// T-E-S-34: StreamableHttp 变体序列化为 "streamable_http"(snake_case)。
    #[test]
    fn streamable_http_variant_serializes_as_snake_case() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer tok".to_string());
        let cfg = McpServerConfig {
            name: "sh-server".to_string(),
            transport_type: McpTransportType::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                headers,
                session_id: Some("sid-123".to_string()),
            },
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec![],
            auto_restart: true,
            health_check_interval_secs: 30,
        };
        let json = serde_json::to_string(&cfg).expect("serialize should succeed");
        assert!(
            json.contains("\"streamable_http\""),
            "should contain streamable_http tag: {}",
            json
        );
        let parsed: McpServerConfig = serde_json::from_str(&json).expect("parse should succeed");
        match parsed.transport_type {
            McpTransportType::StreamableHttp {
                url,
                headers,
                session_id,
            } => {
                assert_eq!(url, "https://example.com/mcp");
                assert_eq!(
                    headers.get("Authorization").map(|s| s.as_str()),
                    Some("Bearer tok")
                );
                assert_eq!(session_id.as_deref(), Some("sid-123"));
            }
            other => panic!("expected StreamableHttp, got {:?}", other),
        }
    }

    /// T-E-S-34: StreamableHttp 缺失 headers/session_id 时回退默认值。
    #[test]
    fn streamable_http_variant_defaults() {
        let json = r#"{
            "name": "sh-defaults",
            "transport_type": {"streamable_http": {"url": "https://example.com/mcp"}},
            "enabled": true,
            "tool_filter": []
        }"#;
        let parsed: McpServerConfig = serde_json::from_str(json).expect("parse should succeed");
        match parsed.transport_type {
            McpTransportType::StreamableHttp {
                url,
                headers,
                session_id,
            } => {
                assert_eq!(url, "https://example.com/mcp");
                assert!(headers.is_empty());
                assert!(session_id.is_none());
            }
            other => panic!("expected StreamableHttp, got {:?}", other),
        }
    }

    /// T-E-S-34: validate 拒绝 StreamableHttp 空 url。
    #[test]
    fn validate_rejects_empty_streamable_http_url() {
        let cfg = McpServersConfig {
            version: 1,
            servers: vec![McpServerConfig {
                name: "bad-sh".to_string(),
                transport_type: McpTransportType::StreamableHttp {
                    url: String::new(),
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
                auto_restart: true,
                health_check_interval_secs: 30,
            }],
        };
        assert!(cfg.validate().is_err());
    }
}
