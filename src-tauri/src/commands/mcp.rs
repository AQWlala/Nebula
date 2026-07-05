//! T-E-S-30 / T-E-S-32: MCP Tauri 命令 — 工具发现 + 子进程管理。
//!
//! 本模块将 MCP 能力暴露给前端:
//! * `mcp_list_servers` / `mcp_add_server` / `mcp_remove_server` — 配置管理。
//! * `mcp_list_tools` / `mcp_invoke_tool` — 工具发现与调用。
//! * `mcp_server_list` / `mcp_server_start` / `mcp_server_stop` /
//!   `mcp_server_status` / `mcp_server_logs` — T-E-S-32 子进程生命周期。

use tauri::State;
use tracing::instrument;

use crate::AppState;
use crate::mcp::client::{McpTool, McpToolResult};
use crate::mcp::config::McpServerConfig;
use crate::mcp::registry::{McpServerInfo, McpServerStatus};

use super::error::{CommandError, ErrorCode};

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_list_servers"))]
pub async fn mcp_list_servers(state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    Ok(state.mcp_manager.list_servers())
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_add_server"))]
pub async fn mcp_add_server(
    state: State<'_, AppState>,
    config: McpServerConfig,
) -> Result<bool, CommandError> {
    state.mcp_manager.add_server(config);
    Ok(true)
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_remove_server"))]
pub async fn mcp_remove_server(
    state: State<'_, AppState>,
    name: String,
) -> Result<bool, CommandError> {
    state.mcp_manager.remove_server(&name);
    // T-E-S-30: 同步清理 ToolRegistry 中对应前缀(mcp_<server>_)的工具。
    state.tool_registry.unregister_server(&name);
    Ok(true)
}

/// 列出指定 server 已发现的工具。
///
/// `server_id` 为空字符串时返回**所有** server 的工具（聚合）。
/// server 不存在时返回 `not_found` 错误。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_list_tools"))]
pub async fn mcp_list_tools(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<Vec<McpTool>, CommandError> {
    if server_id.is_empty() {
        return Ok(state.mcp_manager.list_all_tools().await);
    }
    state
        .mcp_manager
        .list_tools_for_server(&server_id)
        .await
        .map_err(|e| CommandError {
            code: ErrorCode::NotFound,
            message: format!("MCP server '{}' not found", server_id),
            details: Some(e.to_string()),
        })
}

/// 在指定 server 上调用工具。
///
/// `params` 作为 `arguments` 传入 `tools/call` JSON-RPC 请求。
/// 工具执行错误（`isError=true`）通过返回的 `McpToolResult.is_error`
/// 标记,而非 `Err` — 仅在 server 不存在或传输失败时返回 `Err`。
#[tauri::command]
#[instrument(skip(state, params), fields(otel.kind = "mcp_invoke_tool"))]
pub async fn mcp_invoke_tool(
    state: State<'_, AppState>,
    server_id: String,
    tool_name: String,
    params: serde_json::Value,
) -> Result<McpToolResult, CommandError> {
    state
        .mcp_manager
        .invoke_tool(&server_id, &tool_name, params)
        .await
        .map_err(|e| CommandError {
            code: ErrorCode::Internal,
            message: format!("MCP invoke_tool failed: {}", e),
            details: Some(e.to_string()),
        })
}

// ----------------------------------------------------------------------
// T-E-S-32: MCP Server 子进程生命周期命令
//
// 集成完成:lib.rs AppState 已添加 `mcp_registry: Arc<McpServerRegistry>` 字段,
// bootstrap 中构造并注入,invoke_handler 注册 5 个命令。
// ----------------------------------------------------------------------

/// 列出所有 MCP server 的运行时信息(状态/pid/重启次数/日志路径)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_server_list"))]
pub async fn mcp_server_list(
    state: State<'_, AppState>,
) -> Result<Vec<McpServerInfo>, CommandError> {
    Ok(state.mcp_registry.list())
}

/// 启动指定的 MCP server(stdio 子进程)。
///
/// 若 server 已在运行,等价于 no-op(返回 Ok)。若 server 不在配置中,
/// 返回 `not_found` 错误。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_server_start"))]
pub async fn mcp_server_start(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), CommandError> {
    state
        .mcp_registry
        .start(&name)
        .await
        .map_err(|e| CommandError {
            code: ErrorCode::Internal,
            message: format!("MCP server '{}' start failed", name),
            details: Some(e.to_string()),
        })
}

/// 停止指定的 MCP server(kill 子进程 + disconnect client)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_server_stop"))]
pub async fn mcp_server_stop(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), CommandError> {
    state
        .mcp_registry
        .stop(&name)
        .await
        .map_err(|e| CommandError {
            code: ErrorCode::Internal,
            message: format!("MCP server '{}' stop failed", name),
            details: Some(e.to_string()),
        })
}

/// 查询指定 MCP server 的当前状态。
///
/// server 不存在时返回 `not_found` 错误。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_server_status"))]
pub async fn mcp_server_status(
    state: State<'_, AppState>,
    name: String,
) -> Result<McpServerStatus, CommandError> {
    state
        .mcp_registry
        .status(&name)
        .ok_or_else(|| CommandError {
            code: ErrorCode::NotFound,
            message: format!("MCP server '{}' not found", name),
            details: None,
        })
}

/// 读取指定 MCP server 的日志(最后 N 行)。
///
/// `tail` 为 None 时默认返回最后 100 行。日志路径为
/// `<log_dir>/mcp-<name>.log`。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_server_logs"))]
pub async fn mcp_server_logs(
    state: State<'_, AppState>,
    name: String,
    tail: Option<usize>,
) -> Result<Vec<String>, CommandError> {
    let tail = tail.unwrap_or(100);
    state
        .mcp_registry
        .logs(&name, tail)
        .await
        .map_err(|e| CommandError {
            code: ErrorCode::Internal,
            message: format!("MCP server '{}' logs failed", name),
            details: Some(e.to_string()),
        })
}
