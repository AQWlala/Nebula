//! T-E-S-33: OpenAPI 工具服务器 Tauri 命令。
//!
//! 运行时加载 OpenAPI spec(JSON/YAML),解析为工具并注册到 `ToolRegistry`。
//! 整个模块由 `#[cfg(feature = "openapi")]` 门控,默认不编译。
//!
//! 用法(前端):
//! ```ignore
//! await invoke('openapi_register_tools', {
//!   spec: yamlOrJsonString,
//!   bearerToken: 'xxx' // 可选
//! });
//! ```

#![cfg(feature = "openapi")]

use tauri::State;
use tracing::instrument;

use crate::tools::openapi_server::{OpenApiAuth, OpenApiToolServer};
use crate::AppState;

use super::error::{CommandError, ErrorCode};

/// 解析 OpenAPI spec 并注册生成的工具。
///
/// - `spec`:OpenAPI 3.0/3.1 spec 字符串(JSON 或 YAML)。
/// - `bearer_token`:可选 Bearer token,注入到所有请求的 `Authorization` header。
///
/// 返回注册的工具数量。
#[tauri::command]
#[instrument(skip(state, spec), fields(otel.kind = "openapi_register_tools"))]
pub async fn openapi_register_tools(
    state: State<'_, AppState>,
    spec: String,
    bearer_token: Option<String>,
) -> Result<usize, CommandError> {
    let mut server = OpenApiToolServer::from_spec(&spec).map_err(|e| CommandError {
        code: ErrorCode::Internal,
        message: "OpenAPI spec parse failed".to_string(),
        details: Some(e.to_string()),
    })?;
    if let Some(token) = bearer_token {
        server = server.with_auth(OpenApiAuth::Bearer(token));
    }
    let count = state.infra.tool_registry.register_openapi_tools(server);
    Ok(count)
}
