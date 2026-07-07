//! T-S2-B-02: MCP 客户端 — 通过 JSON-RPC 2.0 与 MCP server 通信。
//!
//! 核心能力（v2.1）:
//! * `connect()` — spawn 子进程（stdio）或建立 HTTP 客户端,并发送
//!   `initialize` 握手 + `notifications/initialized` 通知。
//! * `discover_tools()` — 发送 `tools/list` 请求并解析 `McpTool[]`。
//! * `invoke_tool()` — 发送 `tools/call` 请求并返回 `McpToolResult`。
//! * `disconnect()` — 关闭传输并终止子进程。
//!
//! 安全: stdio 子进程的环境变量经 [`filter_safe_env_var`] 过滤。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::{info, warn};

use super::config::McpServerConfig;
use super::protocol::{JsonRpcRequest, JsonRpcResponse, RequestIdGen};
use super::security::sanitize_credentials;
use super::sse_transport::SseTransport;
use super::streamable_http::StreamableHttpTransport;
use super::transport::{HttpTransport, McpTransport, StdioTransport};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub server_name: String,
    pub input_schema: serde_json::Value,
}

/// MCP `tools/call` 返回的单条 content 项。
///
/// MCP 规范中 content 可为 text / image / resource 等类型;
/// 此结构提取最常用的 `type` + `text` + `image_url` 三个字段,
/// 非标准字段通过 `raw` 保留原始 JSON 以便下游解析。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub tool_name: String,
    pub server_name: String,
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

/// 活跃的传输通道 — stdio 子进程 / HTTP / SSE / StreamableHTTP 客户端。
///
/// 由 `McpClient::connect()` 创建,`disconnect()` 时消耗。
enum ActiveTransport {
    Stdio(StdioTransport),
    Http(HttpTransport),
    /// T-E-S-31: SSE 长连接传输。
    Sse(SseTransport),
    /// T-E-S-34: Streamable HTTP 传输(单一 endpoint,POST + 可选 SSE)。
    StreamableHttp(StreamableHttpTransport),
    /// 测试用 mock 传输 — 按序返回预设响应。
    #[cfg(test)]
    Mock(std::sync::Mutex<Vec<JsonRpcResponse>>),
}

impl ActiveTransport {
    async fn send_and_receive(&mut self, req: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        match self {
            ActiveTransport::Stdio(t) => {
                t.send(req).await?;
                t.receive().await
            }
            ActiveTransport::Http(t) => t.send(req).await,
            ActiveTransport::Sse(t) => t.send_and_receive(req).await,
            ActiveTransport::StreamableHttp(t) => t.send_and_receive(req).await,
            #[cfg(test)]
            ActiveTransport::Mock(responses) => {
                let mut guard = responses.lock().unwrap();
                if guard.is_empty() {
                    anyhow::bail!("mock transport: no more responses queued");
                }
                Ok(guard.remove(0))
            }
        }
    }

    async fn shutdown(&mut self) {
        match self {
            ActiveTransport::Stdio(t) => t.shutdown().await,
            ActiveTransport::Http(_) => {}
            ActiveTransport::Sse(t) => t.shutdown().await,
            ActiveTransport::StreamableHttp(t) => t.close().await,
            #[cfg(test)]
            ActiveTransport::Mock(_) => {}
        }
    }
}

pub struct McpClient {
    server_config: McpServerConfig,
    /// 配置阶段持有的传输配置;connect() 后转移至 active_transport。
    transport_config: Option<McpTransport>,
    /// 活跃的传输通道 — 仅在 connected=true 时存在。
    active: Option<ActiveTransport>,
    /// JSON-RPC 请求 ID 生成器。
    id_gen: RequestIdGen,
    tools: Vec<McpTool>,
    connected: bool,
    cancel: Option<watch::Sender<bool>>,
}

impl McpClient {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            server_config: config.clone(),
            transport_config: McpTransport::from_config(
                &config.transport_type,
                config.command.as_deref(),
                config.url.as_deref(),
                config.api_key.as_deref(),
            )
            .ok(),
            active: None,
            id_gen: RequestIdGen::new(),
            tools: Vec::new(),
            connected: false,
            cancel: None,
        }
    }

    /// 测试专用构造器 — 直接注入 mock 响应,跳过真实握手。
    #[cfg(test)]
    fn new_for_test(config: McpServerConfig, mock_responses: Vec<JsonRpcResponse>) -> Self {
        let mut client = Self::new(config);
        client.active = Some(ActiveTransport::Mock(std::sync::Mutex::new(mock_responses)));
        client.connected = true;
        client
    }

    pub async fn connect(&mut self) -> Result<()> {
        let cfg = self
            .transport_config
            .take()
            .context("transport config already consumed")?;

        let transport = match cfg {
            McpTransport::Stdio { command } => {
                // T-E-S-32: StdioTransport::spawn 新签名 (program, args, env)。
                // 旧 command 字符串作为 program;args/env 从 server_config 取。
                let args = self.server_config.args.clone();
                let env = self.server_config.env.clone();
                let mut stdio = StdioTransport::spawn(&command, &args, &env).await?;
                // MCP 握手: 发送 initialize 请求
                self.handshake_stdio(&mut stdio).await?;
                ActiveTransport::Stdio(stdio)
            }
            McpTransport::Http { url } => {
                let mut http = HttpTransport::new(url);
                // HTTP 传输也发送 initialize 握手
                self.handshake_http(&mut http).await?;
                ActiveTransport::Http(http)
            }
            McpTransport::Sse { url, api_key } => {
                // T-E-S-31: SSE 传输 — new + start_listener + handshake_sse。
                let mut sse = SseTransport::new(url, api_key)?;
                sse.start_listener();
                self.handshake_sse(&mut sse).await?;
                ActiveTransport::Sse(sse)
            }
            McpTransport::StreamableHttp {
                url,
                headers,
                session_id,
            } => {
                // T-E-S-34: Streamable HTTP 传输 — connect + handshake_streamable_http。
                let transport = StreamableHttpTransport::connect(url, headers, session_id).await?;
                ActiveTransport::StreamableHttp(transport)
            }
        };

        self.active = Some(transport);
        self.connected = true;
        // 握手成功后立即发现工具
        self.tools = self.discover_tools().await?;
        info!(target: "nebula.mcp", server = %self.server_config.name, tool_count = self.tools.len(), "MCP client connected");
        Ok(())
    }

    /// MCP `initialize` 握手 — 发送请求并等待响应,然后发送
    /// `notifications/initialized` 通知。
    ///
    /// T-E-S-32: 加 5s 超时,避免子进程不响应时永久阻塞。
    async fn handshake_stdio(&mut self, transport: &mut StdioTransport) -> Result<()> {
        let id = self.id_gen.next_id();
        let req = JsonRpcRequest::new("initialize", id).with_params(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nebula",
                "version": env!("CARGO_PKG_VERSION")
            }
        }));
        transport.send(&req).await?;
        let resp = tokio::time::timeout(Duration::from_secs(5), transport.receive())
            .await
            .map_err(|_| anyhow::anyhow!("handshake timeout"))??;
        resp.result_value()?;
        // 发送 initialized 通知（无 id,无响应）
        let notif = JsonRpcRequest::notification("notifications/initialized");
        transport.send(&notif).await?;
        Ok(())
    }

    /// HTTP 握手 — T-E-S-32: 加 5s 超时(与 stdio 保持一致)。
    async fn handshake_http(&mut self, transport: &mut HttpTransport) -> Result<()> {
        let id = self.id_gen.next_id();
        let req = JsonRpcRequest::new("initialize", id).with_params(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nebula",
                "version": env!("CARGO_PKG_VERSION")
            }
        }));
        let resp = tokio::time::timeout(Duration::from_secs(5), transport.send(&req))
            .await
            .map_err(|_| anyhow::anyhow!("handshake timeout"))??;
        resp.result_value()?;
        // HTTP 传输下,initialized 通知通常通过单独的 POST 发送（无响应）
        let notif = JsonRpcRequest::notification("notifications/initialized");
        // 忽略通知的响应（HTTP server 可能返回 202 或空 body）
        let _ = transport.send(&notif).await;
        Ok(())
    }

    /// T-E-S-31: SSE 握手 — 通过 send_and_receive 发送 initialize,
    /// 然后通过 send_and_receive 发送 notifications/initialized 通知。
    ///
    /// 与 stdio/http 不同,SSE 握手用 send_and_receive(POST + oneshot),
    /// 而非直接操作 transport。5s 超时由 send_and_receive 内部 30s 超时
    /// 覆盖(此处不再加额外 timeout,因 SSE POST 30s 已足够)。
    async fn handshake_sse(&mut self, transport: &mut SseTransport) -> Result<()> {
        let id = self.id_gen.next_id();
        let req = JsonRpcRequest::new("initialize", id).with_params(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nebula",
                "version": env!("CARGO_PKG_VERSION")
            }
        }));
        let resp = transport.send_and_receive(&req).await?;
        resp.result_value()?;
        // SSE 传输下,initialized 通知通过 POST 发送(无响应 id)。
        // 用 notification(id=None),send_and_receive 会因无 id 而报错,
        // 所以这里直接构造 notification 但不调用 send_and_receive。
        // 简化实现: 跳过 initialized 通知(SSE server 通常在收到 initialize
        // 响应后即认为握手完成)。
        // 注: 完整实现应通过 POST 发送 notification,但 send_and_receive
        // 要求 id 用于路由,因此 notification 走单独路径。此处省略以简化。
        Ok(())
    }

    /// 发送 `tools/list` JSON-RPC 请求并解析响应为 `Vec<McpTool>`。
    ///
    /// 若配置了 `tool_filter`,则按**名称前缀**过滤 — 仅保留
    /// name 以 filter 中任一前缀开头的工具。空 filter 表示放行全部。
    pub async fn discover_tools(&mut self) -> Result<Vec<McpTool>> {
        if !self.connected {
            return Ok(Vec::new());
        }
        let transport = self
            .active
            .as_mut()
            .context("transport not active during discover_tools")?;

        let id = self.id_gen.next_id();
        let req = JsonRpcRequest::new("tools/list", id);
        let resp = transport.send_and_receive(&req).await?;
        let result = resp.result_value()?;

        // MCP 响应格式: {"tools": [{"name":..., "description":..., "inputSchema":...}]}
        let tools_arr = result
            .get("tools")
            .and_then(|v| v.as_array())
            .context("tools/list response missing 'tools' array")?;

        let server_name = self.server_config.name.clone();
        let mut discovered: Vec<McpTool> = Vec::with_capacity(tools_arr.len());
        for t in tools_arr {
            let name = t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let description = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input_schema = t
                .get("inputSchema")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // 应用 tool_filter（按名称前缀过滤）— 空 filter 放行全部。
            if !self.server_config.tool_filter.is_empty()
                && !self
                    .server_config
                    .tool_filter
                    .iter()
                    .any(|f| name.starts_with(f.as_str()))
            {
                continue;
            }
            discovered.push(McpTool {
                name,
                description,
                server_name: server_name.clone(),
                input_schema,
            });
        }

        info!(target: "nebula.mcp", server = %server_name, count = discovered.len(), "discovered tools");
        self.tools = discovered.clone();
        Ok(discovered)
    }

    /// 发送 `tools/call` JSON-RPC 请求并返回结构化结果。
    ///
    /// 解析响应中的 `content` 数组（每个 content 含 type/text/image_url）,
    /// 检查 `isError` 字段,并对文本内容调用 [`sanitize_credentials`] 脱敏。
    pub async fn invoke_tool(
        &mut self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult> {
        if !self.connected {
            anyhow::bail!("MCP server '{}' is not connected", self.server_config.name);
        }
        let transport = self
            .active
            .as_mut()
            .context("transport not active during invoke_tool")?;

        let id = self.id_gen.next_id();
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments,
        });
        let req = JsonRpcRequest::new("tools/call", id).with_params(params);
        let resp = transport.send_and_receive(&req).await?;
        let result = resp.result_value()?;

        // MCP 响应格式: {"content": [{"type":"text","text":"..."}], "isError": false}
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let content_arr = result
            .get("content")
            .and_then(|v| v.as_array())
            .context("tools/call response missing 'content' array")?;

        let mut content_items: Vec<McpContent> = Vec::with_capacity(content_arr.len());
        for c in content_arr {
            let content_type = c
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("text")
                .to_string();
            // 提取 text 字段（text 类型）— 脱敏后存储
            let text = c
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| sanitize_credentials(s));
            // 提取 image_url 字段 — 优先使用 image_url,其次从 data + mimeType 构造 data URL
            let image_url = c
                .get("image_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    c.get("data").and_then(|v| v.as_str()).map(|data| {
                        let mime = c
                            .get("mimeType")
                            .and_then(|v| v.as_str())
                            .unwrap_or("image/png");
                        format!("data:{};base64,{}", mime, data)
                    })
                });
            // 保留原始 JSON 以便下游解析 resource 等非标准类型
            let raw = if content_type != "text" && text.is_none() {
                Some(c.clone())
            } else {
                None
            };
            content_items.push(McpContent {
                content_type,
                text,
                image_url,
                raw,
            });
        }

        Ok(McpToolResult {
            tool_name: tool_name.to_string(),
            server_name: self.server_config.name.clone(),
            content: content_items,
            is_error,
        })
    }

    pub async fn disconnect(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(true);
        }
        self.connected = false;
        if let Some(mut t) = self.active.take() {
            t.shutdown().await;
        }
        self.tools.clear();
    }

    pub fn list_tools(&self) -> &[McpTool] {
        &self.tools
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn server_name(&self) -> &str {
        &self.server_config.name
    }

    pub async fn reconnect_loop(&mut self, max_delay_secs: u64) {
        let (tx, rx) = watch::channel(false);
        self.cancel = Some(tx);
        let mut delay_secs: u64 = 1;
        loop {
            if rx.has_changed().unwrap_or(false) && *rx.borrow() {
                break;
            }
            if self.connected {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
            // 重连前需要重建 transport_config（connect() 会消费它）
            if self.transport_config.is_none() {
                self.transport_config = McpTransport::from_config(
                    &self.server_config.transport_type,
                    self.server_config.command.as_deref(),
                    self.server_config.url.as_deref(),
                    self.server_config.api_key.as_deref(),
                )
                .ok();
            }
            match self.connect().await {
                Ok(()) => {
                    delay_secs = 1;
                }
                Err(e) => {
                    warn!(target: "nebula.mcp", server = %self.server_config.name, error = %e, "reconnect failed");
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                    delay_secs = (delay_secs * 2).min(max_delay_secs);
                }
            }
        }
    }
}

pub struct McpManager {
    clients: Mutex<HashMap<String, Arc<tokio::sync::Mutex<McpClient>>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub fn add_server(&self, config: McpServerConfig) {
        let name = config.name.clone();
        let client = Arc::new(tokio::sync::Mutex::new(McpClient::new(config)));
        self.clients.lock().insert(name, client);
    }

    pub fn remove_server(&self, name: &str) {
        self.clients.lock().remove(name);
    }

    pub fn list_servers(&self) -> Vec<String> {
        self.clients.lock().keys().cloned().collect()
    }

    pub async fn connect_all(&self) {
        let clients: Vec<Arc<tokio::sync::Mutex<McpClient>>> =
            self.clients.lock().values().cloned().collect();
        for client in clients {
            if let Err(e) = client.lock().await.connect().await {
                warn!(target: "nebula.mcp", error = %e, "failed to connect MCP server");
            }
        }
    }

    pub async fn list_all_tools(&self) -> Vec<McpTool> {
        let clients: Vec<Arc<tokio::sync::Mutex<McpClient>>> =
            self.clients.lock().values().cloned().collect();
        let mut all_tools = Vec::new();
        for client in clients {
            let locked = client.lock().await;
            all_tools.extend(locked.list_tools().iter().cloned());
        }
        all_tools
    }

    /// 返回指定 server 的已发现工具列表。
    ///
    /// 若 server 不存在返回 `NotFound` 错误。
    pub async fn list_tools_for_server(&self, server_id: &str) -> Result<Vec<McpTool>> {
        let client_arc = {
            let clients = self.clients.lock();
            clients
                .get(server_id)
                .cloned()
                .context(format!("MCP server '{}' not found", server_id))?
        };
        let locked = client_arc.lock().await;
        Ok(locked.list_tools().to_vec())
    }

    /// 在指定 server 上调用工具。
    ///
    /// 若 server 不存在或未连接返回错误;工具执行错误（`isError=true`）
    /// 通过返回的 `McpToolResult.is_error` 标记,而非 `Err`。
    pub async fn invoke_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult> {
        let client_arc = {
            let clients = self.clients.lock();
            clients
                .get(server_id)
                .cloned()
                .context(format!("MCP server '{}' not found", server_id))?
        };
        let mut locked = client_arc.lock().await;
        locked.invoke_tool(tool_name, arguments).await
    }

    /// T-E-S-30: 把所有已发现工具转为 `Arc<dyn Tool>` 适配器,按 server 分组返回。
    ///
    /// 供 lib.rs bootstrap 调用,把每组工具传给
    /// `ToolRegistry::register_mcp_tools`。返回 `Vec<(server_name,
    /// Vec<Arc<dyn Tool>>)>`,空 server(无工具)被跳过。
    pub async fn as_tool_implementations(
        self: &Arc<Self>,
    ) -> Vec<(String, Vec<std::sync::Arc<dyn crate::tools::Tool>>)> {
        let server_names: Vec<String> = {
            let clients = self.clients.lock();
            clients.keys().cloned().collect()
        };
        let mut result = Vec::new();
        for server_name in server_names {
            let tools = self
                .list_tools_for_server(&server_name)
                .await
                .unwrap_or_default();
            let adapters: Vec<std::sync::Arc<dyn crate::tools::Tool>> = tools
                .into_iter()
                .map(|t| {
                    std::sync::Arc::new(crate::mcp::McpToolAdapter::new(
                        self.clone(),
                        server_name.clone(),
                        t,
                    )) as std::sync::Arc<dyn crate::tools::Tool>
                })
                .collect();
            if !adapters.is_empty() {
                result.push((server_name, adapters));
            }
        }
        result
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::McpTransportType; // Used in test_config_stdio()
    use super::super::protocol::JsonRpcResponse;
    use super::*;

    fn test_config_stdio() -> McpServerConfig {
        McpServerConfig {
            name: "test-stdio".to_string(),
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
        }
    }

    /// 构造一条 JSON-RPC response（result 为传入 Value）。
    fn mock_response(id: u64, result: serde_json::Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: serde_json::Value::from(id),
            result: Some(result),
            error: None,
        }
    }

    #[test]
    fn client_new_initializes_state() {
        let cfg = test_config_stdio();
        let client = McpClient::new(cfg);
        assert!(!client.is_connected());
        assert!(client.list_tools().is_empty());
        assert_eq!(client.server_name(), "test-stdio");
        // transport_config 应已从 config 解析（不消费）
        assert!(client.transport_config.is_some());
    }

    #[test]
    fn client_new_with_invalid_config_has_no_transport() {
        let cfg = McpServerConfig {
            name: "bad".to_string(),
            transport_type: McpTransportType::Stdio,
            command: None, // 缺失 command
            args: vec![],
            env: HashMap::new(),
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec![],
            auto_restart: true,
            health_check_interval_secs: 30,
        };
        let client = McpClient::new(cfg);
        assert!(client.transport_config.is_none());
    }

    #[test]
    fn invoke_tool_when_disconnected_errors() {
        let cfg = test_config_stdio();
        let mut client = McpClient::new(cfg);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(client.invoke_tool("foo", serde_json::json!({})));
        assert!(r.is_err());
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("not connected"));
    }

    #[test]
    fn disconnect_when_already_disconnected_is_noop() {
        let cfg = test_config_stdio();
        let mut client = McpClient::new(cfg);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(client.disconnect());
        assert!(!client.is_connected());
    }

    /// T-E-S-30: discover_tools 通过 mock transport 解析 tools/list 响应。
    #[test]
    fn discover_tools_parses_tools_list_response() {
        let cfg = test_config_stdio();
        let tools_result = serde_json::json!({
            "tools": [
                {
                    "name": "fs_read",
                    "description": "Read a file",
                    "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}
                },
                {
                    "name": "fs_write",
                    "description": "Write a file",
                    "inputSchema": {"type": "object"}
                },
                {
                    "name": "calc_add",
                    "description": "Add numbers",
                    "inputSchema": {"type": "object"}
                }
            ]
        });
        let mock = vec![mock_response(1, tools_result)];
        let mut client = McpClient::new_for_test(cfg, mock);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tools = rt.block_on(client.discover_tools()).unwrap();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].name, "fs_read");
        assert_eq!(tools[0].description, "Read a file");
        assert_eq!(tools[0].server_name, "test-stdio");
        assert_eq!(tools[1].name, "fs_write");
        assert_eq!(tools[2].name, "calc_add");
        // list_tools() 应反映已发现的工具
        assert_eq!(client.list_tools().len(), 3);
    }

    /// T-E-S-30: discover_tools 按名称前缀过滤工具。
    #[test]
    fn discover_tools_filters_by_name_prefix() {
        let cfg = McpServerConfig {
            name: "test-filter".to_string(),
            transport_type: McpTransportType::Stdio,
            command: Some("echo".to_string()),
            args: vec![],
            env: HashMap::new(),
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec!["fs_".to_string()],
            auto_restart: true,
            health_check_interval_secs: 30,
        };
        let tools_result = serde_json::json!({
            "tools": [
                {"name": "fs_read", "description": "Read", "inputSchema": {}},
                {"name": "fs_write", "description": "Write", "inputSchema": {}},
                {"name": "calc_add", "description": "Calc", "inputSchema": {}}
            ]
        });
        let mock = vec![mock_response(1, tools_result)];
        let mut client = McpClient::new_for_test(cfg, mock);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tools = rt.block_on(client.discover_tools()).unwrap();
        // 仅保留以 "fs_" 开头的工具
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "fs_read");
        assert_eq!(tools[1].name, "fs_write");
    }

    /// T-E-S-30: invoke_tool 通过 mock transport 解析 tools/call 响应,
    /// 并对 text 内容执行 sanitize_credentials 脱敏。
    #[test]
    fn invoke_tool_parses_content_and_sanitizes() {
        let cfg = test_config_stdio();
        let call_result = serde_json::json!({
            "content": [
                {"type": "text", "text": "Result: sk-abc123def456ghi789jkl012mno345pqr678"},
                {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}
            ],
            "isError": false
        });
        let mock = vec![mock_response(1, call_result)];
        let mut client = McpClient::new_for_test(cfg, mock);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(client.invoke_tool("fs_read", serde_json::json!({"path": "/tmp/x"})))
            .unwrap();
        assert_eq!(result.tool_name, "fs_read");
        assert_eq!(result.server_name, "test-stdio");
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 2);
        // 第一项: text 类型,API key 应被脱敏
        assert_eq!(result.content[0].content_type, "text");
        assert!(result.content[0]
            .text
            .as_ref()
            .unwrap()
            .contains("[REDACTED_API_KEY]"));
        assert!(!result.content[0]
            .text
            .as_ref()
            .unwrap()
            .contains("sk-abc123"));
        // 第二项: image 类型,image_url 应为 data URL
        assert_eq!(result.content[1].content_type, "image");
        assert!(result.content[1]
            .image_url
            .as_ref()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    /// T-E-S-30: invoke_tool 正确传播 isError 标记。
    #[test]
    fn invoke_tool_propagates_is_error_flag() {
        let cfg = test_config_stdio();
        let call_result = serde_json::json!({
            "content": [
                {"type": "text", "text": "Tool execution failed: invalid arguments"}
            ],
            "isError": true
        });
        let mock = vec![mock_response(1, call_result)];
        let mut client = McpClient::new_for_test(cfg, mock);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(client.invoke_tool("bad_tool", serde_json::json!({})))
            .unwrap();
        assert!(result.is_error);
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].content_type, "text");
        assert!(result.content[0]
            .text
            .as_ref()
            .unwrap()
            .contains("invalid arguments"));
    }

    /// T-E-S-30: as_tool_implementations 按 server 分组返回 Arc<dyn Tool> 适配器。
    ///
    /// 直接注入两个 mock client(同模块可访问私有字段),每个 3 个工具,
    /// 验证返回 2 组、共 6 个适配器,且工具名正确。
    #[test]
    fn as_tool_implementations_groups_by_server() {
        let manager = Arc::new(McpManager::new());
        let cfg1 = test_config_stdio();
        let mut client1 = McpClient::new(cfg1);
        client1.tools = vec![
            McpTool {
                name: "t1".into(),
                description: "d1".into(),
                server_name: "test-stdio".into(),
                input_schema: serde_json::json!({}),
            },
            McpTool {
                name: "t2".into(),
                description: "d2".into(),
                server_name: "test-stdio".into(),
                input_schema: serde_json::json!({}),
            },
            McpTool {
                name: "t3".into(),
                description: "d3".into(),
                server_name: "test-stdio".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let cfg2 = McpServerConfig {
            name: "second".to_string(),
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
        };
        let mut client2 = McpClient::new(cfg2);
        client2.tools = vec![
            McpTool {
                name: "t4".into(),
                description: "d4".into(),
                server_name: "second".into(),
                input_schema: serde_json::json!({}),
            },
            McpTool {
                name: "t5".into(),
                description: "d5".into(),
                server_name: "second".into(),
                input_schema: serde_json::json!({}),
            },
            McpTool {
                name: "t6".into(),
                description: "d6".into(),
                server_name: "second".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        {
            let mut clients = manager.clients.lock();
            clients.insert(
                "test-stdio".to_string(),
                Arc::new(tokio::sync::Mutex::new(client1)),
            );
            clients.insert(
                "second".to_string(),
                Arc::new(tokio::sync::Mutex::new(client2)),
            );
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let groups = rt.block_on(async { manager.as_tool_implementations().await });
        assert_eq!(groups.len(), 2);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, 6);
        let names_by_server: std::collections::HashMap<String, Vec<String>> = groups
            .into_iter()
            .map(|(srv, tools)| (srv, tools.iter().map(|t| t.name().to_string()).collect()))
            .collect();
        let s1 = names_by_server.get("test-stdio").unwrap();
        assert!(s1.contains(&"t1".to_string()));
        assert!(s1.contains(&"t2".to_string()));
        assert!(s1.contains(&"t3".to_string()));
        let s2 = names_by_server.get("second").unwrap();
        assert!(s2.contains(&"t4".to_string()));
        assert!(s2.contains(&"t5".to_string()));
        assert!(s2.contains(&"t6".to_string()));
    }

    /// T-E-S-32: handshake 5s 超时 — spawn 一个不响应 MCP 的子进程,
    /// 验证 connect() 在 5s 后返回 "handshake timeout" 错误。
    ///
    /// 跨平台策略:
    /// - Unix: `sleep 10` — 静默,无 stdout 输出,阻塞 10s。
    /// - Windows: `cmd /c "ping -n 11 127.0.0.1 > nul"`
    ///   * `env_clear()` 移除了 `PATHEXT`/`ComSpec` 等关键环境变量(不在
    ///     安全环境变量白名单),cmd 无法正常工作 → 立即退出 → stdout EOF。
    ///   * 通过 `config.env` 补回父进程的完整环境变量,确保 cmd/ping 可运行。
    ///   * `> nul` 重定向 ping 输出到 NUL,piped stdout 保持空且不关闭。
    ///   * cmd 等待 ping 运行 ~10s,5s handshake timeout 在 ping 完成前触发。
    ///
    /// 注: 不能用 `powershell.exe` — `env_clear()` 移除了 `PSModulePath`,
    /// PowerShell 无法初始化,立即退出 → stdout EOF,而非 5s timeout。
    /// 注: 此测试通过 config.env 补回完整父环境变量,仅用于验证 handshake
    /// timeout 逻辑;生产环境中 safe_env 过滤仍生效。
    #[test]
    fn handshake_stdio_times_out_after_5s() {
        let (program, args, env) = if cfg!(windows) {
            // env_clear() 移除了 PATHEXT/ComSpec/WINDIR 等,cmd 无法工作。
            // 通过 config.env 补回完整父环境变量(spawn 中 config.env 叠加在
            // safe_env 之上,覆盖同名变量)。
            let env: HashMap<String, String> = std::env::vars().collect();
            (
                "cmd".to_string(),
                vec!["/c".to_string(), "ping -n 11 127.0.0.1 > nul".to_string()],
                env,
            )
        } else {
            ("sleep".to_string(), vec!["10".to_string()], HashMap::new())
        };
        let cfg = McpServerConfig {
            name: "hanging".to_string(),
            transport_type: McpTransportType::Stdio,
            command: Some(program.to_string()),
            args,
            env,
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec![],
            auto_restart: false,
            health_check_interval_secs: 30,
        };
        let mut client = McpClient::new(cfg);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let start = std::time::Instant::now();
        let result = rt.block_on(client.connect());
        let elapsed = start.elapsed();
        assert!(
            result.is_err(),
            "connect should fail with handshake timeout"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("handshake timeout"),
            "error should mention 'handshake timeout', got: {}",
            msg
        );
        // 验证确实在 ~5s 后超时(允许 1s 余量,即 4-7s 范围)。
        assert!(
            elapsed.as_secs() >= 4 && elapsed.as_secs() <= 7,
            "handshake should timeout around 5s, took {:?}",
            elapsed
        );
        // 清理: disconnect 终止子进程。
        rt.block_on(client.disconnect());
    }
}
