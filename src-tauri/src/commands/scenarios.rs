//! T-E-C-13: 工作场景模板命令 — scenario_list / scenario_get / scenario_instantiate。
//!
//! 三个命令都是 `TemplateEngine` 的薄封装,从 AppState 拿到引擎实例后
//! 直接查询。`scenario_instantiate` 返回可传给 `swarm_execute` 的
//! `SwarmTask`,前端可串联调用启动蜂群。

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::scenarios::{ScenarioCategory, ScenarioTemplate};
use crate::swarm::SwarmTask;
use crate::AppState;

/// 列出场景模板。
///
/// `category` 为 `None` 时返回全部模板;为 `Some(category)` 时只返回
/// 该分类下的模板。前端按分类分组渲染时传 `Some`。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "scenario_list"))]
pub async fn scenario_list(
    state: State<'_, AppState>,
    category: Option<ScenarioCategory>,
) -> Result<Vec<ScenarioTemplate>, CommandError> {
    let engine = &state.scenario_templates;
    let result: Vec<ScenarioTemplate> = match category {
        Some(cat) => engine.list_by_category(cat).into_iter().cloned().collect(),
        None => engine.list().to_vec(),
    };
    Ok(result)
}

/// 按 id 查询单个场景模板。
///
/// 返回 `Some(template)` 或 `None`(id 不存在)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "scenario_get"))]
pub async fn scenario_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ScenarioTemplate>, CommandError> {
    Ok(state.scenario_templates.get_by_id(&id).cloned())
}

/// scenario_instantiate 的请求 DTO。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantiateScenarioRequest {
    /// 模板 id。
    pub id: String,
    /// 用户输入(填入 `user_prompt_template` 的 `{{user_input}}` 占位符)。
    pub user_input: String,
}

/// 实例化场景模板:把 `user_input` 填入模板,返回可传给 `swarm_execute`
/// 的 `SwarmTask`。
///
/// 前端典型流程:
/// 1. `scenario_instantiate({ id, user_input })` → 拿到 `SwarmTask`
/// 2. `swarm_execute(task)` → 启动蜂群
///
/// 返回 `None` 表示 `id` 不存在(前端应展示"模板不存在"提示)。
#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "scenario_instantiate"))]
pub async fn scenario_instantiate(
    state: State<'_, AppState>,
    request: InstantiateScenarioRequest,
) -> Result<Option<SwarmTask>, CommandError> {
    Ok(state
        .scenario_templates
        .instantiate(&request.id, &request.user_input))
}
