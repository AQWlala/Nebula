//! v1.3 Plan + 准奏 commands — L4 价值层门禁的前端入口。
//!
//! 前端工作流：
//! 1. 调用 [`plan_pre_check`] 检查任务是否需要门禁；
//! 2. 若返回 `Gate`，展示 [`PendingGate`] 给用户；
//! 3. 用户确认/拒绝后调用对应的 approve/deny 命令；
//! 4. 批准后调用 `swarm_execute` 执行任务。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::plan::{ConfirmationRequest, PlanRequest};
use crate::swarm::{PreCheckResult, SwarmTask};
use crate::AppState;

/// L4 门禁预检：在执行任务前检查是否需要准奏/Plan。
///
/// 返回 `Allow` → 可直接 `swarm_execute`；
/// 返回 `Deny` → 展示理由，拒绝执行；
/// 返回 `Gate` → 展示给用户，等待 approve/deny。
#[tauri::command]
#[instrument(skip(state, task), fields(otel.kind = "plan_pre_check"))]
pub async fn plan_pre_check(
    state: State<'_, AppState>,
    task: SwarmTask,
) -> Result<PreCheckResult, CommandError> {
    Ok(state.swarm.pre_check(&task))
}

/// 批准准奏请求。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "plan_approve_confirmation"))]
pub async fn plan_approve_confirmation(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<bool, CommandError> {
    Ok(state.swarm.plan_engine().approve_confirmation(&request_id))
}

/// 拒绝准奏请求。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "plan_deny_confirmation"))]
pub async fn plan_deny_confirmation(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<bool, CommandError> {
    Ok(state.swarm.plan_engine().deny_confirmation(&request_id))
}

/// 批准 Plan 请求。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "plan_approve_plan"))]
pub async fn plan_approve_plan(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<bool, CommandError> {
    Ok(state.swarm.plan_engine().approve_plan(&request_id))
}

/// 拒绝 Plan 请求。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "plan_reject_plan"))]
pub async fn plan_reject_plan(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<bool, CommandError> {
    Ok(state.swarm.plan_engine().reject_plan(&request_id))
}

/// 获取 Plan 请求详情（供前端展示方案步骤）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "plan_get_plan"))]
pub async fn plan_get_plan(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<PlanRequest>, CommandError> {
    Ok(state.swarm.plan_engine().get_plan(&request_id))
}

/// 获取准奏请求详情。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "plan_get_confirmation"))]
pub async fn plan_get_confirmation(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<ConfirmationRequest>, CommandError> {
    Ok(state.swarm.plan_engine().get_confirmation(&request_id))
}

/// 脱敏入口：在内容发送给 LLM 之前调用，返回脱敏后的内容。
#[tauri::command]
#[instrument(skip(state, content), fields(otel.kind = "values_redact"))]
pub async fn values_redact(
    state: State<'_, AppState>,
    content: String,
) -> Result<String, CommandError> {
    Ok(state.swarm.values_layer().redact(&content))
}
