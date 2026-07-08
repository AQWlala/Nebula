//! T-E-A-07: Credits Dashboard 命令。
//!
//! 聚合 `CostTracker` 的按日/周/月费用统计与 provider/agent 分桶，
//! 附带 L0.5 语义缓存命中/未命中计数，供前端 CreditsDashboard 展示。

use std::sync::atomic::Ordering;

use tauri::State;

use crate::commands::error::CommandError;
use crate::llm::cost_tracker::{
    DailyAggregate, MonthlyAggregate, ProviderBucket, SourceBucket, WeeklyAggregate,
};
use crate::AppState;

/// Credits 仪表盘汇总数据。
#[derive(Debug, serde::Serialize)]
pub struct CreditsOverview {
    /// 按日聚合（升序）。
    pub daily: Vec<DailyAggregate>,
    /// 按周聚合（ISO 周一为起点，升序）。
    pub weekly: Vec<WeeklyAggregate>,
    /// 按月聚合（升序）。
    pub monthly: Vec<MonthlyAggregate>,
    /// 按 provider 分桶（cost_usd 降序，None → "unknown"）。
    pub by_provider: Vec<ProviderBucket>,
    /// 按 agent 分桶（cost_usd 降序，None → "unknown"）。
    pub by_agent: Vec<ProviderBucket>,
    /// T-E-A-12: 按来源(source)分桶(cost_usd 降序)。
    /// 区分 Chat 与 Automation 成本,供前端 Chat/Automation 分栏展示。
    pub by_source: Vec<SourceBucket>,
    /// M6 #81: 按 WorkType 分桶(cost_usd 降序)。
    /// 区分 chat / swarm_worker / swarm_synthesize / master_task / evolution /
    /// soul_compile / classifier 7 类调用,供前端分域展示 + local/remote 分离。
    /// (Evolution / SoulCompile / Classifier 为 local_only,零远端成本。)
    pub by_work_type: Vec<SourceBucket>,
    /// 累计总费用（USD）。
    pub total_cost_usd: f64,
    /// L0.5 语义缓存命中数。
    pub semantic_cache_hits: u64,
    /// L0.5 语义缓存未命中数。
    pub semantic_cache_misses: u64,
    /// T-E-A-10: 估算节省的金额(USD)。
    pub cost_saved_usd: f64,
    /// T-E-A-10: Prefix-Cache 命中累计 token 数。
    pub prefix_cache_cached_tokens: u64,
}

/// T-E-A-07: 返回费用仪表盘聚合数据（按日/周/月 + provider/agent 分桶 +
/// 累计总费用 + 语义缓存命中/未命中）。
///
/// T-E-A-12: 附带 by_source 分桶,供前端 Chat vs Automation 分栏展示。
#[tauri::command]
pub async fn credits_overview(state: State<'_, AppState>) -> Result<CreditsOverview, CommandError> {
    let tracker = &state.llm.cost_tracker;
    let metrics = crate::metrics::global();
    Ok(CreditsOverview {
        daily: tracker.aggregate_by_day(),
        weekly: tracker.aggregate_by_week(),
        monthly: tracker.aggregate_by_month(),
        by_provider: tracker.aggregate_by_provider(),
        by_agent: tracker.aggregate_by_agent(),
        by_source: tracker.aggregate_by_source(None),
        by_work_type: tracker.aggregate_by_work_type(None),
        total_cost_usd: tracker.total_cost_usd(),
        semantic_cache_hits: metrics.semantic_cache_hits.load(Ordering::Relaxed),
        semantic_cache_misses: metrics.semantic_cache_misses.load(Ordering::Relaxed),
        cost_saved_usd: metrics.cost_saved_usd(),
        prefix_cache_cached_tokens: metrics.prefix_cache_cached_tokens.load(Ordering::Relaxed),
    })
}
