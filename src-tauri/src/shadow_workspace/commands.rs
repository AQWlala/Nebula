//! T-E-C-08 / T-E-C-09: Shadow Workspace Tauri 命令。
//!
//! 生命周期(T-E-C-08):
//! - `shadow_create`        — 创建隔离 worktree + 分支
//! - `shadow_list`          — 列出所有 workspace
//! - `shadow_status`        — 查询单个 workspace 状态
//! - `shadow_diff`          — 获取 workspace 与 base 的 diff
//! - `shadow_run_command`   — 在 worktree 内执行命令(供 Agent,自动录屏)
//! - `shadow_complete`      — 标记任务完成
//! - `shadow_fail`          — 标记任务失败
//! - `shadow_merge`         — 合并回 base + 清理
//! - `shadow_abort`         — 丢弃 + 清理
//! - `shadow_cleanup`       — 清理已完结的 worktree 目录
//!
//! 录屏回放(T-E-C-09):
//! - `shadow_record`            — 手动记录一条操作(文件修改/备注)
//! - `shadow_recording_list`    — 获取 workspace 操作时间线
//! - `shadow_recording_clear`   — 清除录屏
//!
//! 所有命令返回 `CommandError`,引擎操作通过 `AppState.shadow_engine` 访问。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::shadow_workspace::{OperationKind, OperationRecord, ShadowWorkspace};
use crate::AppState;

/// 创建 Shadow Workspace。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_create"))]
pub async fn shadow_create(
    state: State<'_, AppState>,
    task_description: String,
    base_branch: Option<String>,
) -> Result<ShadowWorkspace, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .create(task_description, base_branch)
            .map_err(|e| CommandError::internal("shadow_create", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_create_blocking", &anyhow::anyhow!(e)))?
}

/// 列出所有 workspace(按创建时间降序)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_list"))]
pub async fn shadow_list(
    state: State<'_, AppState>,
) -> Result<Vec<ShadowWorkspace>, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || engine.list())
        .await
        .map_err(|e| CommandError::internal("shadow_list_blocking", &anyhow::anyhow!(e)))
}

/// 查询单个 workspace 状态。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_status"))]
pub async fn shadow_status(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Option<ShadowWorkspace>, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || engine.get(&workspace_id))
        .await
        .map_err(|e| CommandError::internal("shadow_status_blocking", &anyhow::anyhow!(e)))
}

/// 获取 workspace 与 base_branch 的 diff。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_diff"))]
pub async fn shadow_diff(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<String, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .diff(&workspace_id)
            .map_err(|e| CommandError::internal("shadow_diff", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_diff_blocking", &anyhow::anyhow!(e)))?
}

/// 在 worktree 内执行命令(供 Agent 使用)。
#[tauri::command]
#[instrument(skip(state, args), fields(otel.kind = "shadow_run_command"))]
pub async fn shadow_run_command(
    state: State<'_, AppState>,
    workspace_id: String,
    program: String,
    args: Vec<String>,
) -> Result<String, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .run_command(&workspace_id, &program, &args)
            .map_err(|e| CommandError::internal("shadow_run_command", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_run_command_blocking", &anyhow::anyhow!(e)))?
}

/// 标记 workspace 任务完成。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_complete"))]
pub async fn shadow_complete(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<ShadowWorkspace, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .complete(&workspace_id)
            .map_err(|e| CommandError::internal("shadow_complete", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_complete_blocking", &anyhow::anyhow!(e)))?
}

/// 标记 workspace 任务失败。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_fail"))]
pub async fn shadow_fail(
    state: State<'_, AppState>,
    workspace_id: String,
    error: String,
) -> Result<ShadowWorkspace, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .fail(&workspace_id, error)
            .map_err(|e| CommandError::internal("shadow_fail", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_fail_blocking", &anyhow::anyhow!(e)))?
}

/// 合并 workspace 分支回 base_branch,然后清理 worktree + 分支。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_merge"))]
pub async fn shadow_merge(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<ShadowWorkspace, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .merge(&workspace_id)
            .map_err(|e| CommandError::internal("shadow_merge", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_merge_blocking", &anyhow::anyhow!(e)))?
}

/// 丢弃 workspace:强制清理 worktree + 删除分支,不可逆。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_abort"))]
pub async fn shadow_abort(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<ShadowWorkspace, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .abort(&workspace_id)
            .map_err(|e| CommandError::internal("shadow_abort", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_abort_blocking", &anyhow::anyhow!(e)))?
}

/// 清理已完结(Merged/Aborted)的 worktree 目录(保留元数据记录)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_cleanup"))]
pub async fn shadow_cleanup(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .cleanup(&workspace_id)
            .map_err(|e| CommandError::internal("shadow_cleanup", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_cleanup_blocking", &anyhow::anyhow!(e)))?
}

// ---- T-E-C-09: 任务录屏回放 ----

/// 手动记录一条操作(文件修改/备注)。
///
/// 命令执行由 `shadow_run_command` 自动记录,无需调用此方法;
/// 此命令供 Agent 显式记录文件操作和备注。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_record"))]
pub async fn shadow_record(
    state: State<'_, AppState>,
    workspace_id: String,
    kind: OperationKind,
    target: String,
    detail: String,
    success: bool,
    message: String,
) -> Result<OperationRecord, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .record_operation(&workspace_id, kind, target, detail, success, message)
            .map_err(|e| CommandError::internal("shadow_record", &e))
    })
    .await
    .map_err(|e| CommandError::internal("shadow_record_blocking", &anyhow::anyhow!(e)))?
}

/// 获取 workspace 的完整操作时间线(按 seq 升序,供前端回放)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_recording_list"))]
pub async fn shadow_recording_list(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<OperationRecord>, CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || engine.get_recording(&workspace_id))
        .await
        .map_err(|e| CommandError::internal("shadow_recording_list_blocking", &anyhow::anyhow!(e)))
}

/// 清除 workspace 的录屏(合并/丢弃后可选清理)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "shadow_recording_clear"))]
pub async fn shadow_recording_clear(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<(), CommandError> {
    let engine = state.shadow_engine.clone();
    tokio::task::spawn_blocking(move || engine.clear_recording(&workspace_id))
        .await
        .map_err(|e| CommandError::internal("shadow_recording_clear_blocking", &anyhow::anyhow!(e)))?;
    Ok(())
}
