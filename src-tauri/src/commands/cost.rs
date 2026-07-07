//! T-E-A-08 / T-E-A-12: 费用报告命令。
//!
//! 提供 `cost_report` Tauri 命令,返回按模型或来源聚合的费用明细,
//! 供前端 CreditsDashboard 复用。CLI 模式(`nebula cost report`)
//! 的入口在 `main.rs`,此处仅负责 Tauri 命令注册。
//!
//! T-E-A-12: 新增 `group_by=source` 参数,返回按 CostSource 分组的聚合,
//! 用于区分 Chat 与 Automation 成本。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::llm::cost_tracker::{ModelCostRow, SourceBucket};
use crate::AppState;

/// T-E-A-12: cost_report 返回的聚合结果。
///
/// `#[serde(untagged)]` 让前端按返回形状直接消费:
///   * `ByModel(Vec<ModelCostRow>)` — 默认,按模型聚合;
///   * `BySource(Vec<SourceBucket>)` — `group_by=source` 时,按来源聚合。
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum CostReport {
    /// 按模型聚合(默认 + `group_by=model`)。
    ByModel(Vec<ModelCostRow>),
    /// 按来源聚合(`group_by=source`)。
    BySource(Vec<SourceBucket>),
}

/// T-E-A-08 / T-E-A-12: 返回按模型或来源聚合的费用明细。
///
/// `month` 格式 "YYYY-MM";`None` 表示当月。
/// `group_by`:
///   * `None` 或 `"model"` — 按模型聚合(默认,向后兼容);
///   * `"source"` — 按 CostSource 分组(Chat/Automation/Cron/Background)。
///
/// 按 `CostRecord.ts` 的 (year, month) 过滤,结果按 `cost_usd` 降序。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "cost_report"))]
pub async fn cost_report(
    state: State<'_, AppState>,
    month: Option<String>,
    group_by: Option<String>,
) -> Result<CostReport, CommandError> {
    match group_by.as_deref() {
        Some("source") => Ok(CostReport::BySource(
            state.cost_tracker.aggregate_by_source(month),
        )),
        // 默认与 "model" 都走按模型聚合(向后兼容)。
        _ => Ok(CostReport::ByModel(
            state.cost_tracker.aggregate_by_model(month),
        )),
    }
}
