#[cfg(feature = "mcp")]
pub mod client;
#[cfg(feature = "mcp")]
pub mod config;
// T-S2-B-02: JSON-RPC 2.0 帧编解码（tools/list, tools/call 等）。
#[cfg(feature = "mcp")]
pub mod protocol;
// T-E-S-32: MCP stdio 子进程注册表与 supervisor(由 T-E-S-32 subagent 实现)。
// 仅在此处声明 `pub mod registry;` 占位;registry.rs 由另一 subagent 创建。
#[cfg(feature = "mcp")]
pub mod registry;
#[cfg(feature = "mcp")]
pub mod security;
// T-E-S-31: SSE 传输(GET /sse 长连接 + POST /messages)。
#[cfg(feature = "mcp")]
pub mod sse_transport;
// T-E-S-34: Streamable HTTP 传输(单一 endpoint,POST + 可选 SSE)。
#[cfg(feature = "mcp")]
pub mod streamable_http;
#[cfg(feature = "mcp")]
pub mod transport;

// T-E-S-30: MCP → Tool trait 适配器。
//
// `Tool::call` 是同步签名,而 `McpManager::invoke_tool` 是 async。
// 适配器在 `call()` 内部通过 `tokio::task::block_in_place` 把 async 调用
// 桥接到同步上下文(在多线程 runtime 上有效,如 Tauri 默认 runtime);
// 若当前不在 runtime 中(例如某些测试场景),则临时构建一个
// current-thread runtime 完成 `block_on`。
#[cfg(feature = "mcp")]
pub use self::adapter::McpToolAdapter;

#[cfg(feature = "mcp")]
mod adapter {
    use std::sync::Arc;

    use anyhow::Result;

    use crate::mcp::client::{McpManager, McpTool};
    use crate::tools::{Tool, ToolOutput};

    /// T-E-S-30: 把 MCP 工具适配为 `Tool` trait,使其可注册到 ToolRegistry。
    ///
    /// 持有 `Arc<McpManager>`(用于调用 `invoke_tool`)、`server_name`、
    /// 以及 `McpTool` 元数据(name/description/input_schema)。
    /// `call()` 内部转发到 `McpManager::invoke_tool`,把 MCP 返回的
    /// `McpToolResult` 转为 `ToolOutput`。
    pub struct McpToolAdapter {
        manager: Arc<McpManager>,
        server_name: String,
        tool: McpTool,
    }

    impl McpToolAdapter {
        pub fn new(manager: Arc<McpManager>, server_name: String, tool: McpTool) -> Self {
            Self {
                manager,
                server_name,
                tool,
            }
        }
    }

    impl Tool for McpToolAdapter {
        fn name(&self) -> &str {
            // 返回原始 tool name;由 ToolRegistry::register_mcp_tools 加
            // `mcp_<server>_<tool>` 前缀,避免双重前缀。
            &self.tool.name
        }

        fn description(&self) -> &str {
            &self.tool.description
        }

        fn schema(&self) -> serde_json::Value {
            self.tool.input_schema.clone()
        }

        fn call(&self, arguments: serde_json::Value) -> Result<ToolOutput> {
            let manager = self.manager.clone();
            let server_name = self.server_name.clone();
            let tool_name = self.tool.name.clone();
            let invoke = async move {
                manager
                    .invoke_tool(&server_name, &tool_name, arguments)
                    .await
            };

            // 把 async 调用桥接到同步 trait。
            let result = match tokio::runtime::Handle::try_current() {
                Ok(handle) => tokio::task::block_in_place(|| handle.block_on(invoke)),
                Err(_) => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    rt.block_on(invoke)
                }
            }?;

            // 把 McpToolResult.content 数组拼接为文本。
            let text: String = result
                .content
                .iter()
                .filter_map(|c| c.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n");

            Ok(ToolOutput {
                success: !result.is_error,
                result: text,
                error: if result.is_error {
                    Some(format!(
                        "MCP tool '{}' on server '{}' reported isError",
                        result.tool_name, result.server_name
                    ))
                } else {
                    None
                },
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::mcp::client::McpManager;

        /// 编译时 + 元数据验证:adapter 暴露原始 name/description/schema。
        #[test]
        fn adapter_exposes_tool_metadata() {
            let manager = Arc::new(McpManager::new());
            let tool = McpTool {
                name: "fs_read".to_string(),
                description: "Read a file".to_string(),
                server_name: "test".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            };
            let adapter = McpToolAdapter::new(manager, "test".to_string(), tool);
            assert_eq!(adapter.name(), "fs_read");
            assert_eq!(adapter.description(), "Read a file");
            assert_eq!(adapter.schema(), serde_json::json!({"type": "object"}));
        }

        /// call() 在 server 不存在时返回 Err(走 fallback runtime 路径,
        /// 因为普通 #[test] 没有 tokio runtime 上下文)。
        #[test]
        fn adapter_call_errors_when_server_absent() {
            let manager = Arc::new(McpManager::new());
            let tool = McpTool {
                name: "fs_read".to_string(),
                description: "Read a file".to_string(),
                server_name: "missing".to_string(),
                input_schema: serde_json::json!({}),
            };
            let adapter = McpToolAdapter::new(manager, "missing".to_string(), tool);
            let result = adapter.call(serde_json::json!({}));
            assert!(result.is_err());
            let msg = format!("{}", result.unwrap_err());
            assert!(msg.contains("not found"));
        }
    }
}
