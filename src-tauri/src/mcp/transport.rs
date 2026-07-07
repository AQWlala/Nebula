//! T-S2-B-02: MCP 传输层 — stdio 子进程 + HTTP 两种模式。
//!
//! 本模块在原 `McpTransport` 配置枚举之上新增真正的 I/O 能力:
//!
//! * [`StdioTransport`] — spawn 一个 MCP server 子进程,通过 stdin/stdout
//!   以换行分隔的 JSON-RPC 2.0 帧通信。子进程环境变量经过
//!   [`filter_safe_env_vars`] 过滤,仅传递白名单变量。
//! * [`HttpTransport`] — 通过 HTTP POST 携带 JSON-RPC body 与远程
//!   MCP server 通信（一次请求一次响应）。
//!
//! T-E-S-31: 新增 [`McpTransport::Sse`] 配置变体;实际 I/O 由
//! [`super::sse_transport::SseTransport`] 完成。
//! T-E-S-32: [`StdioTransport::spawn`] 改签名,接收 `(program, args, env)`;
//! stderr 改 piped,提供 `take_stderr()` 供 registry 日志 pump。
//!
//! [`filter_safe_env_vars`]: super::security::filter_safe_env_vars

use std::collections::HashMap;
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
// M7b #94: SSRF 校验
use crate::security::SsrfGuard;
use tracing::{debug, warn};

use super::config::McpTransportType;
use super::protocol::{parse_frame, write_frame, JsonRpcRequest, JsonRpcResponse};
use super::security::filter_safe_env_vars;

/// 配置阶段的传输类型（不含 I/O 句柄）。
///
/// 保留以兼容现有 `McpClient::connect()` 调用链;实际 I/O 通过
/// [`StdioTransport`] / [`HttpTransport`] / [`super::sse_transport::SseTransport`] 完成。
#[derive(Debug, Clone)]
pub enum McpTransport {
    Stdio {
        command: String,
    },
    Http {
        url: String,
    },
    /// T-E-S-31: SSE 长连接传输(GET /sse + POST /messages)。
    Sse {
        url: String,
        api_key: Option<String>,
    },
    /// T-E-S-34: Streamable HTTP 传输(单一 endpoint,POST + 可选 SSE)。
    StreamableHttp {
        url: String,
        headers: HashMap<String, String>,
        session_id: Option<String>,
    },
}

impl McpTransport {
    pub fn from_config(
        transport_type: &McpTransportType,
        command: Option<&str>,
        url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Self> {
        match transport_type {
            McpTransportType::Stdio => {
                let cmd =
                    command.ok_or_else(|| anyhow::anyhow!("stdio transport requires a command"))?;
                Ok(McpTransport::Stdio {
                    command: cmd.to_string(),
                })
            }
            McpTransportType::Http => {
                let u = url.ok_or_else(|| anyhow::anyhow!("http transport requires a url"))?;
                Ok(McpTransport::Http { url: u.to_string() })
            }
            McpTransportType::Sse => {
                let u = url.ok_or_else(|| anyhow::anyhow!("sse transport requires a url"))?;
                Ok(McpTransport::Sse {
                    url: u.to_string(),
                    api_key: api_key.map(|s| s.to_string()),
                })
            }
            McpTransportType::StreamableHttp {
                url: sh_url,
                headers,
                session_id,
            } => Ok(McpTransport::StreamableHttp {
                url: sh_url.clone(),
                headers: headers.clone(),
                session_id: session_id.clone(),
            }),
        }
    }
}

/// 活跃的 stdio 传输 — 持有子进程及其 stdin/stdout/stderr 句柄。
///
/// 生命周期由 `McpClient` 管理: `connect()` 时 `spawn()`,`disconnect()`
/// 时 drop（子进程随 stdin 关闭而退出）。
pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// T-E-S-32: stderr 改 piped,供 registry 日志 pump。
    /// 用 Option 以支持 `take_stderr()` 转移所有权。
    stderr: Option<ChildStderr>,
}

impl StdioTransport {
    /// T-E-S-32: spawn 一个 MCP server 子进程并接管其 stdin/stdout/stderr。
    ///
    /// 改签名: 接收 `(program, args, env)` 而非单字符串。
    /// `program` 为可执行文件名,`args` 为参数列表,`env` 为额外环境变量
    /// (叠加在 `filter_safe_env_vars` 白名单之上)。
    ///
    /// 环境变量仅保留 [`SAFE_ENV_VARS`] 白名单成员 + config.env,避免向第三方
    /// 子进程泄漏 API key、token 等敏感信息。
    ///
    /// [`SAFE_ENV_VARS`]: super::security::SAFE_ENV_VARS
    pub async fn spawn(
        program: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        // T-S2-B-02: 安全环境变量过滤 — 仅传递白名单成员给子进程。
        let raw_env: HashMap<String, String> = std::env::vars().collect();
        let mut safe_env = filter_safe_env_vars(&raw_env);
        // T-E-S-32: 叠加 config.env(覆盖同名白名单变量)。
        for (k, v) in env {
            safe_env.insert(k.clone(), v.clone());
        }

        debug!(target: "nebula.mcp", program, args = ?args, env_count = safe_env.len(), "spawning MCP stdio child");

        let mut cmd = Command::new(program);
        cmd.args(args)
            .env_clear()
            .envs(safe_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // T-E-S-32: stderr 改 piped,供 registry 日志 pump。
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // tokio::process::Command 在 Windows 上提供 inherent `creation_flags`
            // 方法（CREATE_NO_WINDOW = 0x08000000 隐藏控制台窗口）。
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn MCP server: {}", program))?;
        let stdin = child.stdin.take().context("child stdin not captured")?;
        let stdout_raw = child.stdout.take().context("child stdout not captured")?;
        let stdout = BufReader::new(stdout_raw);
        let stderr = child.stderr.take();

        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
        })
    }

    /// T-E-S-32: 转移 stderr 句柄供 registry 日志 pump。
    /// 第二次调用返回 None。
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }

    /// 发送一条 JSON-RPC 请求帧并 flush stdin。
    pub async fn send(&mut self, req: &JsonRpcRequest) -> Result<()> {
        let frame = write_frame(req)?;
        self.stdin
            .write_all(frame.as_bytes())
            .await
            .context("failed to write JSON-RPC frame to child stdin")?;
        self.stdin
            .flush()
            .await
            .context("failed to flush child stdin")?;
        debug!(target: "nebula.mcp", method = %req.method, "sent JSON-RPC frame");
        Ok(())
    }

    /// 读取一行（一条 JSON-RPC 响应帧）。
    ///
    /// 跳过空行;遇到 EOF 返回 `Err` 表示子进程已退出。
    pub async fn receive(&mut self) -> Result<JsonRpcResponse> {
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .await
                .context("failed to read JSON-RPC frame from child stdout")?;
            if n == 0 {
                anyhow::bail!("MCP child stdout closed (EOF)");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                // 跳过空行（某些 server 在帧间插入空行）
                continue;
            }
            // 跳过非 JSON 的日志行（stderr 已分离,但某些 server 仍把日志写到 stdout）
            if !trimmed.starts_with('{') {
                debug!(target: "nebula.mcp", line = trimmed, "skipping non-JSON stdout line");
                continue;
            }
            return parse_frame(trimmed).context("failed to parse JSON-RPC response");
        }
    }

    /// 终止子进程。优先尝试 `kill()`;若失败仅记录警告。
    pub async fn shutdown(&mut self) {
        // 关闭 stdin 促使子进程自行退出
        let _ = self.stdin.shutdown().await;
        // 等待最多 1 秒,超时则强制 kill
        match tokio::time::timeout(std::time::Duration::from_secs(1), self.child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                warn!(target: "nebula.mcp", "MCP child did not exit gracefully, killing");
                let _ = self.child.start_kill();
            }
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // 尽力 kill;若已退出则忽略错误
        let _ = self.child.start_kill();
    }
}

/// HTTP 传输 — 通过 reqwest POST 与远程 MCP server 通信。
///
/// MCP 规范允许 HTTP 传输（每条请求一个 POST,响应在 body 中返回）。
/// 本实现保持极简: 不支持 SSE 长连接,适合一次性 `tools/list` /
/// `tools/call` 调用。
pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(url: String) -> Self {
        // M7b #94: SSRF 校验 + build_safe_client(重定向链每跳校验)。
        // MCP server URL 用户可控,必须防止 SSRF 到内网。
        let client = SsrfGuard::new().build_safe_client().unwrap_or_else(|e| {
            tracing::warn!(
                target: "nebula.ssrf",
                error = %e,
                url = %url,
                "failed to build SSRF-safe client for MCP HttpTransport; falling back"
            );
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        });
        Self { url, client }
    }

    pub async fn send(&mut self, req: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        let resp = self
            .client
            .post(&self.url)
            .json(req)
            .send()
            .await
            .context("HTTP transport: POST request failed")?
            .error_for_status()
            .context("HTTP transport: non-2xx response")?;
        let body = resp
            .text()
            .await
            .context("HTTP transport: failed to read response body")?;
        parse_frame(body.trim()).context("HTTP transport: failed to parse response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_transport_from_config() {
        let t =
            McpTransport::from_config(&McpTransportType::Stdio, Some("npx"), None, None).unwrap();
        match t {
            McpTransport::Stdio { command } => assert_eq!(command, "npx"),
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn http_transport_from_config() {
        let t = McpTransport::from_config(
            &McpTransportType::Http,
            None,
            Some("https://example.com/mcp"),
            None,
        )
        .unwrap();
        match t {
            McpTransport::Http { url } => assert_eq!(url, "https://example.com/mcp"),
            _ => panic!("expected Http"),
        }
    }

    /// T-E-S-31: McpTransport::from_config Sse 分支。
    #[test]
    fn sse_transport_from_config() {
        let t = McpTransport::from_config(
            &McpTransportType::Sse,
            None,
            Some("https://example.com/sse"),
            Some("secret-key"),
        )
        .unwrap();
        match t {
            McpTransport::Sse { url, api_key } => {
                assert_eq!(url, "https://example.com/sse");
                assert_eq!(api_key.as_deref(), Some("secret-key"));
            }
            _ => panic!("expected Sse"),
        }
    }

    #[test]
    fn stdio_config_requires_command() {
        let r = McpTransport::from_config(&McpTransportType::Stdio, None, None, None);
        assert!(r.is_err());
    }

    #[test]
    fn http_config_requires_url() {
        let r = McpTransport::from_config(&McpTransportType::Http, None, None, None);
        assert!(r.is_err());
    }

    /// T-E-S-31: SSE 配置缺失 url 时报错。
    #[test]
    fn sse_config_requires_url() {
        let r = McpTransport::from_config(&McpTransportType::Sse, None, None, Some("key"));
        assert!(r.is_err());
    }

    /// T-E-S-34: McpTransport::from_config StreamableHttp 分支。
    #[test]
    fn streamable_http_transport_from_config() {
        let mut headers = HashMap::new();
        headers.insert("X-Custom".to_string(), "v".to_string());
        let tt = McpTransportType::StreamableHttp {
            url: "https://example.com/mcp".to_string(),
            headers,
            session_id: Some("sid".to_string()),
        };
        let t = McpTransport::from_config(&tt, None, None, None).unwrap();
        match t {
            McpTransport::StreamableHttp {
                url,
                headers,
                session_id,
            } => {
                assert_eq!(url, "https://example.com/mcp");
                assert_eq!(headers.get("X-Custom").map(|s| s.as_str()), Some("v"));
                assert_eq!(session_id.as_deref(), Some("sid"));
            }
            other => panic!("expected StreamableHttp, got {:?}", other),
        }
    }

    /// 验证 `filter_safe_env_vars` 在 transport 模块可用（生产调用点前置检查）。
    #[test]
    fn filter_safe_env_vars_is_callable() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        env.insert("SECRET".to_string(), "leak".to_string());
        let filtered = filter_safe_env_vars(&env);
        assert!(filtered.contains_key("PATH"));
        assert!(!filtered.contains_key("SECRET"));
    }

    /// T-E-S-32: StdioTransport::spawn with args/env(跨平台 mock)。
    /// Windows 用 `cmd /c echo`,Unix 用 `echo`。
    /// 仅验证 spawn 成功 + take_stderr 返回 Some,不验证输出。
    #[tokio::test]
    async fn stdio_spawn_with_args_env_succeeds() {
        let (program, args) = if cfg!(windows) {
            (
                "cmd",
                vec!["/c".to_string(), "echo".to_string(), "hello".to_string()],
            )
        } else {
            ("echo", vec!["hello".to_string()])
        };
        let mut env = HashMap::new();
        env.insert("MCP_TEST_VAR".to_string(), "value".to_string());
        let mut transport = StdioTransport::spawn(program, &args, &env)
            .await
            .expect("spawn should succeed");
        // take_stderr 第一次返回 Some
        let stderr = transport.take_stderr();
        assert!(stderr.is_some(), "stderr should be captured");
        // 第二次返回 None
        assert!(transport.take_stderr().is_none());
        transport.shutdown().await;
    }
}
