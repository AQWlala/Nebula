//! T-E-S-24 文件快照回滚 Tauri 命令。
//!
//! 提供 snapshot_create / snapshot_rollback / snapshot_discard / snapshot_list
//! 四个命令，供前端调用。
//!
//! T-E-S-44: SnapshotEngine 已改为 async,命令层不再需要 spawn_blocking,
//! 直接 await 引擎方法即可。

use std::path::PathBuf;

use tauri::State;
use tracing::instrument;

use crate::snapshot::SnapshotInfoDto;
use crate::AppState;

use super::error::CommandError;

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "snapshot_create"))]
pub async fn snapshot_create(
    state: State<'_, AppState>,
    working_dir: String,
    files: Vec<String>,
) -> Result<String, CommandError> {
    let wd = PathBuf::from(&working_dir);
    let file_paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();

    let engine = state.snapshot_engine.clone();
    let id = engine
        .create_snapshot(&wd, &file_paths)
        .await
        .map_err(|e| CommandError::internal("snapshot_create", &e))?;

    Ok(id)
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "snapshot_rollback"))]
pub async fn snapshot_rollback(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    let engine = state.snapshot_engine.clone();
    engine
        .rollback(&id)
        .await
        .map_err(|e| CommandError::internal("snapshot_rollback", &e))?;

    Ok(())
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "snapshot_discard"))]
pub async fn snapshot_discard(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    let engine = state.snapshot_engine.clone();
    engine
        .discard(&id)
        .await
        .map_err(|e| CommandError::internal("snapshot_discard", &e))?;

    Ok(())
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "snapshot_list"))]
pub async fn snapshot_list(state: State<'_, AppState>) -> Result<Vec<SnapshotInfoDto>, CommandError> {
    let engine = state.snapshot_engine.clone();
    let list = engine.list_snapshots();
    let dtos: Vec<SnapshotInfoDto> = list.into_iter().map(SnapshotInfoDto::from).collect();
    Ok(dtos)
}
