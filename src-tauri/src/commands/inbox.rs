//! T-E-S-59 统一收件箱 Tauri 命令 — 全部 `#[cfg(feature = "channels")]` 门控。
//!
//! 五个命令:
//! * `inbox_list` — 列出消息(支持渠道过滤 + 分页)
//! * `inbox_send` — 主动发起新消息到指定渠道
//! * `inbox_reply` — 回复原消息(根据 source_channel 路由)
//! * `inbox_mark_read` — 标记消息已读
//! * `inbox_unread_count` — 未读消息数
//!
//! 注:本文件由本任务创建,但 `lib.rs` 的 `tauri::generate_handler!`
//! 未注册这些命令(由主 agent 统一集成)。因此所有命令都加
//! `#[allow(unused)]` 以避免 dead_code 警告。
//!
//! AppState 暂无 `inbox_manager` 字段,各命令现场用
//! `state.sqlite.raw_connection()` + `state.channel_router` 构造
//! `InboxManager` / `InboxStore`。

#![cfg(feature = "channels")]

use tauri::State;
use tracing::instrument;

use crate::channel::{InboxManager, InboxStore, UnifiedMessage};
use crate::commands::error::CommandError;
use crate::AppState;

/// 内部辅助:从 AppState 构造 `InboxStore`(只读路径用)。
fn build_store(state: &AppState) -> Result<InboxStore, CommandError> {
    let conn = state.sqlite.raw_connection();
    InboxStore::new(conn).map_err(|e| CommandError::db("inbox_store_init", &anyhow::anyhow!("{e}")))
}

/// 内部辅助:从 AppState 构造 `InboxManager`(出站路径用)。
fn build_manager(state: &AppState) -> Result<InboxManager, CommandError> {
    let store = build_store(state)?;
    Ok(InboxManager::new(store, state.channel_router.clone()))
}

/// 列出收件箱消息。按时间戳降序。
///
/// `channel` 为 `None` 时返回所有渠道;否则仅返回该渠道的消息。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "inbox_list"))]
#[allow(unused)]
pub async fn inbox_list(
    state: State<'_, AppState>,
    limit: u32,
    offset: u32,
    channel: Option<String>,
) -> Result<Vec<UnifiedMessage>, CommandError> {
    let store = build_store(&state)?;
    let list = store
        .list(limit, offset, channel.as_deref())
        .map_err(|e| CommandError::db("inbox_list", &anyhow::anyhow!("{e}")))?;
    Ok(list)
}

/// 主动发起新消息到指定渠道。
///
/// `target_channel` 必须是 `ChannelKind::as_str()` 之一
/// (telegram / discord / webchat / jiuwenswarm)。
#[tauri::command]
#[instrument(skip(state, body), fields(otel.kind = "inbox_send"))]
#[allow(unused)]
pub async fn inbox_send(
    state: State<'_, AppState>,
    target_channel: String,
    body: String,
) -> Result<(), CommandError> {
    let manager = build_manager(&state)?;
    manager
        .send_new(&target_channel, &body)
        .await
        .map_err(|e| CommandError::internal("inbox_send", &anyhow::anyhow!("{e}")))?;
    Ok(())
}

/// 回复原消息。根据原消息 `source_channel` 路由出站。
#[tauri::command]
#[instrument(skip(state, body), fields(otel.kind = "inbox_reply"))]
#[allow(unused)]
pub async fn inbox_reply(
    state: State<'_, AppState>,
    message_id: String,
    body: String,
) -> Result<(), CommandError> {
    let manager = build_manager(&state)?;
    manager
        .send_reply(&message_id, &body)
        .await
        .map_err(|e| CommandError::internal("inbox_reply", &anyhow::anyhow!("{e}")))?;
    Ok(())
}

/// 标记一组消息为已读。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "inbox_mark_read"))]
#[allow(unused)]
pub async fn inbox_mark_read(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<(), CommandError> {
    let store = build_store(&state)?;
    store
        .mark_read(&ids)
        .map_err(|e| CommandError::db("inbox_mark_read", &anyhow::anyhow!("{e}")))?;
    Ok(())
}

/// 返回未读消息数。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "inbox_unread_count"))]
#[allow(unused)]
pub async fn inbox_unread_count(state: State<'_, AppState>) -> Result<u32, CommandError> {
    let store = build_store(&state)?;
    let n = store
        .unread_count()
        .map_err(|e| CommandError::db("inbox_unread_count", &anyhow::anyhow!("{e}")))?;
    Ok(n)
}
