//! T-E-C-10: 异步长任务 Tauri 命令。
//!
//! - `long_task_create`        — 创建任务(目标 + 步骤 + 可选 workspace/plan)
//! - `long_task_get`           — 获取单个任务
//! - `long_task_list`          — 列出任务(可按状态过滤)
//! - `long_task_steps`         — 获取任务的所有步骤
//! - `long_task_start`         — 启动任务(Pending/Paused → Running)
//! - `long_task_pause`         — 暂停任务(Running → Paused)
//! - `long_task_resume`        — 恢复任务(Paused → Running)
//! - `long_task_cancel`        — 取消任务(→ Cancelled)
//! - `long_task_delete`        — 删除任务(硬删除)
//!
//! 所有命令返回 `CommandError`,引擎操作通过 `AppState.long_task_engine` 访问。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::long_task::{LongTask, LongTaskStatus, LongTaskStep, StepInput};
use crate::AppState;

/// 创建新长任务。
#[tauri::command]
#[instrument(skip(state, steps), fields(otel.kind = "long_task_create"))]
pub async fn long_task_create(
    state: State<'_, AppState>,
    goal: String,
    steps: Vec<StepInput>,
    workspace_id: Option<String>,
    plan_id: Option<String>,
) -> Result<LongTask, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .create_task(goal, steps, workspace_id, plan_id)
            .map_err(|e| CommandError::internal("long_task_create", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_create_blocking", &anyhow::anyhow!(e)))?
}

/// 获取单个任务。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_get"))]
pub async fn long_task_get(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<LongTask>, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .get_task(&task_id)
            .map_err(|e| CommandError::internal("long_task_get", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_get_blocking", &anyhow::anyhow!(e)))?
}

/// 列出任务(可按状态过滤)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_list"))]
pub async fn long_task_list(
    state: State<'_, AppState>,
    status: Option<LongTaskStatus>,
) -> Result<Vec<LongTask>, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .list_tasks(status)
            .map_err(|e| CommandError::internal("long_task_list", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_list_blocking", &anyhow::anyhow!(e)))?
}

/// 获取任务的所有步骤(按 seq 升序)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_steps"))]
pub async fn long_task_steps(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<LongTaskStep>, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .get_steps(&task_id)
            .map_err(|e| CommandError::internal("long_task_steps", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_steps_blocking", &anyhow::anyhow!(e)))?
}

/// 启动任务(Pending/Paused → Running)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_start"))]
pub async fn long_task_start(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<LongTask, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .start(&task_id)
            .map_err(|e| CommandError::internal("long_task_start", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_start_blocking", &anyhow::anyhow!(e)))?
}

/// 暂停任务(Running → Paused)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_pause"))]
pub async fn long_task_pause(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<LongTask, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .pause(&task_id)
            .map_err(|e| CommandError::internal("long_task_pause", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_pause_blocking", &anyhow::anyhow!(e)))?
}

/// 恢复任务(Paused → Running)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_resume"))]
pub async fn long_task_resume(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<LongTask, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .resume(&task_id)
            .map_err(|e| CommandError::internal("long_task_resume", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_resume_blocking", &anyhow::anyhow!(e)))?
}

/// 取消任务(→ Cancelled)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_cancel"))]
pub async fn long_task_cancel(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<LongTask, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .cancel(&task_id)
            .map_err(|e| CommandError::internal("long_task_cancel", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_cancel_blocking", &anyhow::anyhow!(e)))?
}

/// 删除任务(硬删除,级联删除步骤)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "long_task_delete"))]
pub async fn long_task_delete(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, CommandError> {
    let engine = state.swarm.long_task_engine.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .delete_task(&task_id)
            .map_err(|e| CommandError::internal("long_task_delete", &e))
    })
    .await
    .map_err(|e| CommandError::internal("long_task_delete_blocking", &anyhow::anyhow!(e)))?
}
