//! T-E-B-16: MDRM 5 维关系图谱 Tauri 命令(5 个)。
//!
//! - `mdrm_trace_temporal`   — 时序维度:沿 `Before` 边追溯时间链
//! - `mdrm_find_entities`    — 实体维度:查找同实体记忆簇
//! - `mdrm_trace_hierarchy`  — 层级维度:追溯 Contains/DerivedFrom 层级
//! - `mdrm_find_similar`     — 相似度维度:查找相似记忆
//! - `mdrm_get_graph`        — 多维度组合查询(供前端图谱视图)
//!
//! 所有命令返回 `GraphSnapshot`(nodes + edges + truncated flag),前端
//! 可直接用于 PixiJS/D3 力导向图渲染。

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::memory::mdrm_graph::{GraphSnapshot, MdrmConfig, MdrmEngine, RelationDimension};
use crate::AppState;

/// 查询参数(可选字段,缺省时用 MdrmConfig::default())。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MdrmQueryParams {
    /// 最大遍历深度(默认 4)。
    pub max_depth: Option<u32>,
    /// 最多返回节点数(默认 200)。
    pub max_nodes: Option<usize>,
    /// 最多返回边数(默认 500)。
    pub max_edges: Option<usize>,
    /// 关系权重下限(默认 0.1)。
    pub min_weight: Option<f32>,
}

impl MdrmQueryParams {
    /// 合并为 MdrmConfig,缺省字段用 default。
    fn to_config(&self) -> MdrmConfig {
        let mut cfg = MdrmConfig::default();
        if let Some(d) = self.max_depth {
            cfg.max_depth = d;
        }
        if let Some(n) = self.max_nodes {
            cfg.max_nodes = n;
        }
        if let Some(e) = self.max_edges {
            cfg.max_edges = e;
        }
        if let Some(w) = self.min_weight {
            cfg.min_weight = w;
        }
        cfg
    }
}

/// 从 AppState 的 sqlite store 构造 MdrmEngine。
fn build_engine(state: &State<'_, AppState>) -> MdrmEngine {
    MdrmEngine::new((*state.memory.sqlite).clone())
}

/// 时序维度:沿 `Before` 边追溯时间链。
///
/// 语义:`A Before B` 表示 A 先于 B。本命令返回以 `memory_id` 为起点的
/// 双向时序链(向前找先决事件,向后找后续事件)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mdrm_trace_temporal"))]
pub async fn mdrm_trace_temporal(
    state: State<'_, AppState>,
    memory_id: String,
    params: Option<MdrmQueryParams>,
) -> Result<GraphSnapshot, CommandError> {
    let engine = build_engine(&state);
    let cfg = params.unwrap_or_default().to_config();
    Ok(engine.trace_temporal(&memory_id, &cfg).await)
}

/// 实体维度:查找与 `memory_id` 指向同一实体的所有记忆。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mdrm_find_entities"))]
pub async fn mdrm_find_entities(
    state: State<'_, AppState>,
    memory_id: String,
    params: Option<MdrmQueryParams>,
) -> Result<GraphSnapshot, CommandError> {
    let engine = build_engine(&state);
    let cfg = params.unwrap_or_default().to_config();
    Ok(engine.find_entities(&memory_id, &cfg).await)
}

/// 层级维度:追溯 `memory_id` 的包含层级(Contains/DerivedFrom)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mdrm_trace_hierarchy"))]
pub async fn mdrm_trace_hierarchy(
    state: State<'_, AppState>,
    memory_id: String,
    params: Option<MdrmQueryParams>,
) -> Result<GraphSnapshot, CommandError> {
    let engine = build_engine(&state);
    let cfg = params.unwrap_or_default().to_config();
    Ok(engine.trace_hierarchy(&memory_id, &cfg).await)
}

/// 相似度维度:查找与 `memory_id` 相似的所有记忆。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mdrm_find_similar"))]
pub async fn mdrm_find_similar(
    state: State<'_, AppState>,
    memory_id: String,
    params: Option<MdrmQueryParams>,
) -> Result<GraphSnapshot, CommandError> {
    let engine = build_engine(&state);
    let cfg = params.unwrap_or_default().to_config();
    Ok(engine.find_similar(&memory_id, &cfg).await)
}

/// 多维度组合查询:在指定维度集合内做 BFS 遍历。
///
/// `dims` 为维度字符串列表,可选值:`causal`/`temporal`/`entity`/`hierarchical`/`similarity`。
/// 缺省或空列表表示全部 5 维(等价于 `get_full_graph`)。
///
/// 返回 `GraphSnapshot`,前端可直接渲染节点 + 边。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mdrm_get_graph"))]
pub async fn mdrm_get_graph(
    state: State<'_, AppState>,
    memory_id: String,
    dims: Option<Vec<String>>,
    params: Option<MdrmQueryParams>,
) -> Result<GraphSnapshot, CommandError> {
    let engine = build_engine(&state);
    let cfg = params.unwrap_or_default().to_config();

    // 解析维度字符串;空或无效维度退化为全 5 维
    let parsed_dims: Vec<RelationDimension> = dims
        .as_ref()
        .map(|v| {
            v.iter()
                .filter_map(|s| RelationDimension::from_str_lossy(s))
                .collect()
        })
        .unwrap_or_default();

    if parsed_dims.is_empty() {
        Ok(engine.get_full_graph(&memory_id, &cfg).await)
    } else {
        Ok(engine.query_multi_dim(&memory_id, &parsed_dims, &cfg).await)
    }
}
