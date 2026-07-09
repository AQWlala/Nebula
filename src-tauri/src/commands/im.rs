//! T-E-C-17: IM 绑定 Tauri 命令(6 个)。
//!
//! - `im_create_webhook_binding` — 创建 webhook 绑定(SSRF 校验 + 落盘)
//! - `im_list_bindings`         — 列出所有绑定
//! - `im_delete_binding`        — 删除绑定(幂等)
//! - `im_set_enabled`           — 启用/禁用绑定
//! - `im_test_send`             — 单条绑定的测试发送
//! - `im_broadcast`             — 广播到所有已启用绑定

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::im::{ImMessage, ImMessageLevel, ImPlatform};
use crate::AppState;

/// 创建 webhook 绑定的请求 DTO。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWebhookBindingRequest {
    /// 平台(feishu / wecom / dingtalk)。
    pub platform: String,
    /// Webhook URL(公网,SSRF 校验)。
    pub url: String,
    /// 用户可读名称(如 "团队群"),可空。
    #[serde(default)]
    pub display_name: String,
}

/// 广播消息的请求 DTO。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastRequest {
    pub title: String,
    pub body: String,
    /// 可选 markdown 正文。
    #[serde(default)]
    pub markdown: Option<String>,
    /// 等级(默认 info)。
    #[serde(default)]
    pub level: Option<ImMessageLevel>,
}

/// 广播结果(成功数 + 失败数)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastResult {
    pub success: usize,
    pub failure: usize,
}

/// 创建 webhook 绑定。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "im_create_webhook_binding"))]
pub async fn im_create_webhook_binding(
    state: State<'_, AppState>,
    request: CreateWebhookBindingRequest,
) -> Result<crate::im::ImBinding, CommandError> {
    let platform_str = request.platform.clone();
    let url = request.url.clone();
    let display_name = request.display_name.clone();

    let engine = state.channels.im_engine.clone();
    tokio::task::spawn_blocking(move || {
        let platform = ImPlatform::from_str_lossy(&platform_str)
            .map_err(|e| CommandError::validation(format!("invalid platform: {e}")))?;
        engine
            .create_webhook_binding(platform, url, display_name)
            .map_err(|e| CommandError::internal("im_create_webhook_binding", &e))
    })
    .await
    .map_err(|e| CommandError::internal("im_create_webhook_binding", &anyhow::anyhow!(e)))?
}

/// 列出所有 IM 绑定。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "im_list_bindings"))]
pub async fn im_list_bindings(
    state: State<'_, AppState>,
) -> Result<Vec<crate::im::ImBinding>, CommandError> {
    let engine = state.channels.im_engine.clone();
    let bindings = tokio::task::spawn_blocking(move || {
        engine
            .list_bindings()
            .map_err(|e| CommandError::internal("im_list_bindings", &e))
    })
    .await
    .map_err(|e| CommandError::internal("im_list_bindings", &anyhow::anyhow!(e)))??;
    Ok(bindings)
}

/// 删除 IM 绑定(幂等)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "im_delete_binding"))]
pub async fn im_delete_binding(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let engine = state.channels.im_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .delete_binding(&id)
            .map_err(|e| CommandError::internal("im_delete_binding", &e))
    })
    .await
    .map_err(|e| CommandError::internal("im_delete_binding", &anyhow::anyhow!(e)))?
}

/// 启用/禁用 IM 绑定。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "im_set_enabled"))]
pub async fn im_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), CommandError> {
    let engine = state.channels.im_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .set_enabled(&id, enabled)
            .map_err(|e| CommandError::internal("im_set_enabled", &e))
    })
    .await
    .map_err(|e| CommandError::internal("im_set_enabled", &anyhow::anyhow!(e)))?
}

/// 单条绑定的测试发送。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "im_test_send"))]
pub async fn im_test_send(
    state: State<'_, AppState>,
    id: String,
    title: String,
    body: String,
) -> Result<(), CommandError> {
    let message = ImMessage::new(title, body);
    state
        .channels
        .im_engine
        .test_send(&id, &message)
        .await
        .map_err(|e| CommandError::internal("im_test_send", &e))
}

/// 广播到所有已启用绑定(并发发送,部分失败不影响其他)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "im_broadcast"))]
pub async fn im_broadcast(
    state: State<'_, AppState>,
    request: BroadcastRequest,
) -> Result<BroadcastResult, CommandError> {
    let message = ImMessage {
        title: request.title,
        body: request.body,
        markdown: request.markdown,
        level: request.level.unwrap_or_default(),
    };
    let (success, failure) = state.channels.im_engine.broadcast(message).await;
    Ok(BroadcastResult { success, failure })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_request_deserializes_minimal() {
        let json = r#"{"title":"t","body":"b"}"#;
        let req: BroadcastRequest = serde_json::from_str(json).expect("parse should succeed");
        assert_eq!(req.title, "t");
        assert_eq!(req.body, "b");
        assert_eq!(req.markdown, None);
        assert_eq!(req.level, None);
    }

    #[test]
    fn broadcast_request_deserializes_full() {
        let json = r##"{"title":"t","body":"b","markdown":"# md","level":"warning"}"##;
        let req: BroadcastRequest = serde_json::from_str(json).expect("parse should succeed");
        assert_eq!(req.markdown.as_deref(), Some("# md"));
        assert_eq!(req.level.expect("assertion value"), ImMessageLevel::Warning);
    }

    #[test]
    fn create_webhook_binding_request_deserializes() {
        let json = r#"{"platform":"feishu","url":"https://x.example.com","display_name":"g"}"#;
        let req: CreateWebhookBindingRequest =
            serde_json::from_str(json).expect("create should succeed");
        assert_eq!(req.platform, "feishu");
        assert_eq!(req.url, "https://x.example.com");
        assert_eq!(req.display_name, "g");
    }

    #[test]
    fn create_webhook_binding_request_default_display_name() {
        let json = r#"{"platform":"wecom","url":"https://y.example.com"}"#;
        let req: CreateWebhookBindingRequest =
            serde_json::from_str(json).expect("create should succeed");
        assert_eq!(req.display_name, "");
    }
}
