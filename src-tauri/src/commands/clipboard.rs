//! T-E-C-14: 剪贴板监听 Tauri 命令。
//!
//! 3 个命令,与 [`ClipboardWatcherEngine`] 配套:
//! - `clipboard_watch_start()`  — 启动后台轮询 task
//! - `clipboard_watch_stop()`   — 停止后台 task
//! - `clipboard_watch_status()` — 查询是否运行中
//!
//! 设计与 `commands/watch.rs` 一致:命令只做 state 取锁 + 调用 engine 方法。

use tauri::{AppHandle, State};
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

/// 启动剪贴板监听。若已在运行,返回错误。
///
/// 后台 task 每 500ms 轮询剪贴板,对内容做 hash 去重 + 类型检测,
/// 把"有结构的"内容写入 L2 Episodic 记忆,并通过
/// `nebula://clipboard-detected` 事件通知前端。
#[tauri::command]
#[instrument(skip(state, app), fields(otel.kind = "clipboard_watch_start"))]
pub async fn clipboard_watch_start(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), CommandError> {
    let sponge = state.memory.sponge.clone();
    let mut watcher = state.platform.clipboard_watcher.lock().await;
    watcher
        .start(sponge, app)
        .map_err(|e| CommandError::internal("clipboard_watch_start", &anyhow::anyhow!(e)))
}

/// 停止剪贴板监听。Idempotent:未运行时也返回 Ok。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "clipboard_watch_stop"))]
pub async fn clipboard_watch_stop(state: State<'_, AppState>) -> Result<(), CommandError> {
    let mut watcher = state.platform.clipboard_watcher.lock().await;
    watcher.stop();
    Ok(())
}

/// 查询剪贴板监听是否正在运行。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "clipboard_watch_status"))]
pub async fn clipboard_watch_status(state: State<'_, AppState>) -> Result<bool, CommandError> {
    let watcher = state.platform.clipboard_watcher.lock().await;
    Ok(watcher.is_running())
}
