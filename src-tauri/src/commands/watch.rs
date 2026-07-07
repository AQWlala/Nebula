//! T-E-B-09: 文件夹监控 Tauri 命令。
//!
//! 4 个命令,与 `FileWatcherEngine` 配套:
//! - `watch_start(paths)` — 启动监控(若已有 worker 则 reload)
//! - `watch_stop()`       — 停止监控
//! - `watch_status()`     — 查询 `WatchStatus` { active, paths }
//! - `watch_list_paths()` — 仅返回当前路径列表

use std::path::PathBuf;
use std::time::Duration;

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::memory::file_watcher::WatchStatus;
use crate::AppState;

/// 启动(或热更新)文件夹监控。
///
/// 若 engine 尚未启动,会同时 `start` + `spawn_worker`。
/// 若已启动,会调 `reload_paths` 替换 watcher 集合。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "watch_start"))]
pub async fn watch_start(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();

    // 若 worker 尚未启动,先 start + spawn_worker;
    // 否则用 reload_paths 热更新。
    let needs_start = state.file_watcher_worker.lock().is_none();
    if needs_start {
        state.file_watcher.start(path_bufs);
        if let Some(handle) = state.file_watcher.clone().spawn_worker() {
            *state.file_watcher_worker.lock() = Some(handle);
        }
    } else {
        state.file_watcher.reload_paths(path_bufs);
    }
    Ok(())
}

/// 停止文件夹监控 + 取消消费者 task。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "watch_stop"))]
pub async fn watch_stop(state: State<'_, AppState>) -> Result<(), CommandError> {
    // 1. cancel + 清空 watchers + 清空 sender/receiver
    state.file_watcher.stop().await;
    // 2. 取出 worker handle 并 timeout-await(250ms)
    let handle = state.file_watcher_worker.lock().take();
    if let Some(h) = handle {
        let _ = tokio::time::timeout(Duration::from_millis(250), h).await;
    }
    Ok(())
}

/// 查询当前监控状态。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "watch_status"))]
pub async fn watch_status(state: State<'_, AppState>) -> Result<WatchStatus, CommandError> {
    Ok(state.file_watcher.status())
}

/// 仅返回当前监控路径列表(字符串形式)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "watch_list_paths"))]
pub async fn watch_list_paths(state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    Ok(state.file_watcher.list_paths())
}
