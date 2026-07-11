//! v2.4 T-EVAL-007: Agent 评估体系 — Trace 导出 Tauri 命令
//!
//! ## 命令
//!
//! * `eval_list_traces` — 列出所有 trace（按创建时间倒序）
//! * `eval_export_trace` — 导出指定 trace 为 JSONL 文件
//! * `eval_delete_trace` — 删除指定 trace
//!
//! ## Feature 门控
//!
//! 整个模块由 `eval` feature 门控。未启用 eval feature 时,
//! 这些命令不存在,前端调用会返回 "command not found"。

use std::path::PathBuf;

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::eval::TraceStore;
use crate::AppState;

/// 内部辅助:从 AppState 构造 `TraceStore`。
fn build_store(state: &AppState) -> Result<TraceStore, CommandError> {
    TraceStore::from_sqlite_store(&state.memory.sqlite)
        .map_err(|e| CommandError::db("eval_trace_store_init", &anyhow::anyhow!("{e}")))
}

/// 列出所有 trace（按创建时间倒序）。
///
/// - `limit`: 最多返回的 trace 数量（默认 50）
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "eval_list_traces"))]
#[allow(unused)]
pub async fn eval_list_traces(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<crate::eval::TraceRow>, CommandError> {
    let store = build_store(&state)?;
    let rows = store
        .list_traces(limit.unwrap_or(50))
        .map_err(|e| CommandError::db("eval_list_traces", &anyhow::anyhow!("{e}")))?;
    Ok(rows)
}

/// 导出指定 trace 为 JSONL 文件。
///
/// - `trace_id`: 要导出的 trace ID
/// - `output_path`: 输出文件路径（JSONL 格式）
///
/// 返回导出的 span 数量。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "eval_export_trace"))]
#[allow(unused)]
pub async fn eval_export_trace(
    state: State<'_, AppState>,
    trace_id: String,
    output_path: String,
) -> Result<usize, CommandError> {
    let store = build_store(&state)?;
    let spans = store
        .load_trace(&trace_id)
        .map_err(|e| CommandError::db("eval_load_trace", &anyhow::anyhow!("{e}")))?;

    if spans.is_empty() {
        return Err(CommandError::not_found(format!("trace: {trace_id}")));
    }

    let path = PathBuf::from(&output_path);
    crate::eval::export_spans_jsonl(&spans, &path)
        .map_err(|e| CommandError::db("eval_export_jsonl", &anyhow::anyhow!("{e}")))?;

    Ok(spans.len())
}

/// 删除指定 trace（级联删除其所有 span）。
///
/// - `trace_id`: 要删除的 trace ID
///
/// 返回删除的行数（0 = 不存在, 1 = 已删除）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "eval_delete_trace"))]
#[allow(unused)]
pub async fn eval_delete_trace(
    state: State<'_, AppState>,
    trace_id: String,
) -> Result<usize, CommandError> {
    let store = build_store(&state)?;
    let deleted = store
        .delete_trace(&trace_id)
        .map_err(|e| CommandError::db("eval_delete_trace", &anyhow::anyhow!("{e}")))?;
    Ok(deleted)
}

/// 从内存中的 TraceCollector 导出所有 span 为 JSONL 文件。
///
/// 这个命令不需要 trace_id,直接从内存中的 TraceCollector 导出
/// 所有已收集的 span。适用于运行时实时导出。
///
/// - `output_path`: 输出文件路径（JSONL 格式）
///
/// 返回导出的 span 数量。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "eval_export_all"))]
#[allow(unused)]
pub async fn eval_export_all(
    state: State<'_, AppState>,
    output_path: String,
) -> Result<usize, CommandError> {
    // 从 AppState 获取 TraceCollector（如果存在）
    // 注意:TraceCollector 是通过 with_trace() 注入到 Master/Swarm/Evolution 中的,
    // AppState 本身不持有 TraceCollector。这个命令需要一个全局的 TraceCollector
    // 或者从 Master/Swarm 中获取。
    //
    // Phase 1 实现:返回错误,提示用户使用 eval_export_trace 命令。
    Err(CommandError::validation(
        "use eval_export_trace with a specific trace_id instead. \
         TraceCollector is injected into Master/Swarm/Evolution, not AppState.",
    ))
}
