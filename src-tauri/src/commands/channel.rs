//! Channel commands — message bridge status, send, poll, ping.

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

/// v1.2: Get current status of the message bridge.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_status"))]
pub async fn channel_status(state: State<'_, AppState>) -> Result<serde_json::Value, CommandError> {
    match &state.channels.message_bridge {
        Some(bridge) => Ok(serde_json::json!({
            "connected": bridge.status().connected,
            "endpoint_url": bridge.status().endpoint_url,
            "messages_received": bridge.status().messages_received,
            "messages_sent": bridge.status().messages_sent,
        })),
        None => Ok(serde_json::json!({"connected": false, "channels": []})),
    }
}

/// v1.2: Send a message through the message bridge.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_send"))]
pub async fn channel_send(
    state: State<'_, AppState>,
    target: String,
    text: String,
) -> Result<bool, CommandError> {
    match &state.channels.message_bridge {
        Some(bridge) => {
            let req = crate::channel::types::ChannelSendRequest {
                session_id: target.clone(),
                channel: crate::channel::types::Channel::Web,
                body: text,
                conversation_id: None,
            };
            bridge
                .send(&req)
                .await
                .map(|_| true)
                .map_err(|e| CommandError::internal("channel_send", &anyhow::anyhow!("{e}")))
        }
        None => Err(CommandError::internal(
            "channel_send",
            &anyhow::anyhow!("message bridge not configured"),
        )),
    }
}

/// v1.2: Poll the message bridge for new messages.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_poll"))]
pub async fn channel_poll(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, CommandError> {
    match &state.channels.message_bridge {
        Some(bridge) => Ok(bridge
            .poll()
            .await
            .into_iter()
            .map(|m| serde_json::to_value(&m).unwrap_or_default())
            .collect()),
        None => Ok(Vec::new()),
    }
}

/// v1.2: Ping the message bridge.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_ping"))]
pub async fn channel_ping(state: State<'_, AppState>) -> Result<bool, CommandError> {
    match &state.channels.message_bridge {
        Some(bridge) => Ok(bridge.ping().await),
        None => Ok(false),
    }
}

/// T-S3-B-01: 列出所有已注册的原生渠道适配器及其状态。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_list_adapters"))]
pub async fn channel_list_adapters(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, CommandError> {
    #[cfg(feature = "channels")]
    {
        let channels = state.channels.channel_router.list_channels();
        Ok(channels
            .iter()
            .map(|(kind, status)| {
                serde_json::json!({
                    "kind": kind.as_str(),
                    "status": format!("{:?}", status),
                })
            })
            .collect())
    }
    #[cfg(not(feature = "channels"))]
    {
        let _ = state;
        Ok(Vec::new())
    }
}

/// T-S3-B-01: 通过原生渠道路由器发送消息。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_send_native"))]
pub async fn channel_send_native(
    state: State<'_, AppState>,
    channel: String,
    message: String,
    reply_to: Option<String>,
) -> Result<bool, CommandError> {
    #[cfg(feature = "channels")]
    {
        use crate::channel::types::ChannelKind;
        let kind = match channel.as_str() {
            "telegram" => ChannelKind::Telegram,
            "discord" => ChannelKind::Discord,
            "webchat" => ChannelKind::WebChat,
            _ => {
                return Err(CommandError::internal(
                    "channel_send_native",
                    &anyhow::anyhow!("unknown channel kind: {channel}"),
                ));
            }
        };
        state
            .channels
            .channel_router
            .send(&kind, &message, reply_to.as_deref())
            .await
            .map(|_| true)
            .map_err(|e| CommandError::internal("channel_send_native", &anyhow::anyhow!("{e}")))
    }
    #[cfg(not(feature = "channels"))]
    {
        let _ = (state, channel, message, reply_to);
        Err(CommandError::internal(
            "channel_send_native",
            &anyhow::anyhow!("channels feature not enabled"),
        ))
    }
}

/// T-S3-B-01: 启动所有已注册的原生渠道适配器。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_start_all"))]
pub async fn channel_start_all(state: State<'_, AppState>) -> Result<bool, CommandError> {
    #[cfg(feature = "channels")]
    {
        state
            .channels
            .channel_router
            .start_all()
            .await
            .map(|_| true)
            .map_err(|e| CommandError::internal("channel_start_all", &anyhow::anyhow!("{e}")))
    }
    #[cfg(not(feature = "channels"))]
    {
        let _ = state;
        Ok(false)
    }
}
