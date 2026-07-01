//! Memory commands — store, search, read, update, delete, stats.

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{instrument, warn};

use crate::api::server::{
    NineSnakeService, SearchMemoryHit, SearchMemoryRequest, StoreMemoryRequest, StoreMemoryResponse,
};
use crate::commands::error::CommandError;
use crate::memory::types::{Memory, MemoryLayer, SourceKind};
use crate::AppState;

/// Tauri command: store a memory.
///
/// L7 (Singularity) guard: only `SourceKind::System` may write to L7.
/// Front-end requests with `layer: L7` from non-System sources are
/// silently demoted to L6 (Values) to prevent memory poisoning.
#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "memory_store"))]
pub async fn memory_store(
    state: State<'_, AppState>,
    mut request: StoreMemoryRequest,
) -> Result<StoreMemoryResponse, CommandError> {
    if request.layer == MemoryLayer::L7 && request.source != SourceKind::System {
        warn!(
            target: "nine_snake.cmd",
            source = ?request.source,
            "non-System source attempted L7 write; demoting to L6"
        );
        request.layer = MemoryLayer::L6;
    }
    let resp = state
        .memory_store(request)
        .await
        .map_err(|e| CommandError::memory("memory_store", &e))?;
    crate::metrics::global().record_store();
    Ok(resp)
}

/// Tauri command: vector search over the memory store.
#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "memory_search"))]
pub async fn memory_search(
    state: State<'_, AppState>,
    request: SearchMemoryRequest,
) -> Result<Vec<SearchMemoryHit>, CommandError> {
    let start = std::time::Instant::now();
    let resp = state
        .memory_search(request)
        .await
        .map_err(|e| CommandError::lance("memory_search", &e))?;
    // v1.8: 记录检索延迟（微秒）。
    crate::metrics::global().record_search_latency(start.elapsed().as_micros() as u64);
    crate::metrics::global().record_search();
    Ok(resp)
}

/// Tauri command: fetch a memory by id.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_get"))]
pub async fn memory_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Memory>, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.get(&id).await })
        .await
        .map_err(|e| CommandError::internal("memory_get", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_get", &e))
}

/// Tauri command: list the N most recent memories.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_list_recent"))]
pub async fn memory_list_recent(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<Memory>, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.list_recent(limit.max(1)).await })
        .await
        .map_err(|e| CommandError::internal("memory_list_recent", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_list_recent", &e))
}

/// Tauri command: update a memory's `importance` (clamped to `[0, 1]`).
///
/// L7 guard: memories on L7 cannot have their importance lowered
/// below 0.9 — this prevents accidental demotion of core-value
/// memories that should only be removed via explicit delete.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_update_importance"))]
pub async fn memory_update_importance(
    state: State<'_, AppState>,
    id: String,
    importance: f32,
) -> Result<Memory, CommandError> {
    let sqlite = state.sqlite.clone();
    let sqlite_for_check = sqlite.clone();
    let id_clone = id.clone();
    let mem = tokio::task::spawn(async move { sqlite_for_check.get(&id_clone).await })
        .await
        .map_err(|e| CommandError::internal("memory_update_importance", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_update_importance", &e))?;
    let clamped = importance.clamp(0.0, 1.0);
    let final_importance = if let Some(m) = &mem {
        if m.layer == MemoryLayer::L7 && clamped < 0.9 {
            warn!(
                target: "nine_snake.cmd",
                id = %id,
                requested = clamped,
                "L7 memory importance cannot be lowered below 0.9; clamping"
            );
            0.9
        } else {
            clamped
        }
    } else {
        clamped
    };
    tokio::task::spawn(async move { sqlite.update_importance(&id, final_importance).await })
        .await
        .map_err(|e| CommandError::internal("memory_update_importance", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_update_importance", &e))
}

/// Tauri command: hard-delete a memory.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_delete"))]
pub async fn memory_delete(state: State<'_, AppState>, id: String) -> Result<bool, CommandError> {
    let sqlite = state.sqlite.clone();
    let id_for_thread = id.clone();
    let res = tokio::task::spawn(async move { sqlite.delete(&id_for_thread).await })
        .await
        .map_err(|e| CommandError::internal("memory_delete", &anyhow::anyhow!("{e}")))?;
    match res {
        Ok(deleted) => {
            if deleted {
                if let Err(e) = state.lance.delete(&id).await {
                    warn!(target: "nine_snake.cmd", error = ?e, "lance delete failed");
                }
            }
            Ok(deleted)
        }
        Err(e) => Err(CommandError::db("memory_delete", &e)),
    }
}

/// Tauri command: batch-fetch memories by id (preserves the
/// caller's order).
#[tauri::command]
#[instrument(skip(state, ids), fields(otel.kind = "memory_get_many"))]
pub async fn memory_get_many(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<Memory>, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.get_many(&ids).await })
        .await
        .map_err(|e| CommandError::internal("memory_get_many", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_get_many", &e))
}

/// Snapshot of layer distribution for the stats RPC.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryStats {
    pub total: u64,
    pub by_layer: std::collections::HashMap<MemoryLayer, u64>,
}

/// Tauri command: per-layer memory counts.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_stats"))]
pub async fn memory_stats(state: State<'_, AppState>) -> Result<MemoryStats, CommandError> {
    let sqlite = state.sqlite.clone();
    let rows = tokio::task::spawn(async move { sqlite.counts_per_layer().await })
        .await
        .map_err(|e| CommandError::internal("memory_stats", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_stats", &e))?;
    let total = rows.values().sum();
    Ok(MemoryStats {
        total,
        by_layer: rows,
    })
}

// ---- v1.5: 因果图谱 + 多粒度摘要命令 ----

/// v1.5: 追溯一个记忆的根本原因链。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "causal_trace_root_causes"))]
pub async fn causal_trace_root_causes(
    state: State<'_, AppState>,
    memory_id: String,
    max_depth: Option<u32>,
) -> Result<Vec<crate::memory::causal_graph::CausalChain>, CommandError> {
    let config = crate::memory::causal_graph::CausalGraphConfig {
        max_depth: max_depth.unwrap_or(5),
        ..Default::default()
    };
    let engine = state.causal_graph.clone();
    let result = tokio::task::spawn_blocking(move || engine.trace_root_causes(&memory_id, &config))
        .await
        .map_err(|e| CommandError::internal("causal_trace_root_causes", &anyhow::anyhow!("{e}")))?;
    Ok(result)
}

/// v1.5: 查找一个记忆的所有下游效果。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "causal_find_effects"))]
pub async fn causal_find_effects(
    state: State<'_, AppState>,
    memory_id: String,
    max_depth: Option<u32>,
) -> Result<Vec<crate::memory::causal_graph::CausalChain>, CommandError> {
    let config = crate::memory::causal_graph::CausalGraphConfig {
        max_depth: max_depth.unwrap_or(5),
        ..Default::default()
    };
    let engine = state.causal_graph.clone();
    let result = tokio::task::spawn_blocking(move || engine.find_effects(&memory_id, &config))
        .await
        .map_err(|e| CommandError::internal("causal_find_effects", &anyhow::anyhow!("{e}")))?;
    Ok(result)
}

/// v1.5: 生成一条最可能的因果解释路径。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "causal_explain"))]
pub async fn causal_explain(
    state: State<'_, AppState>,
    memory_id: String,
) -> Result<Option<crate::memory::causal_graph::CausalChain>, CommandError> {
    let engine = state.causal_graph.clone();
    let result = tokio::task::spawn_blocking(move || engine.explain(&memory_id))
        .await
        .map_err(|e| CommandError::internal("causal_explain", &anyhow::anyhow!("{e}")))?;
    Ok(result)
}

/// v1.5: 为一段内容生成多粒度摘要（50/150/500/2000 字符）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "summary_generate"))]
pub async fn summary_generate(
    state: State<'_, AppState>,
    content: String,
) -> Result<crate::memory::types::MultiGranularity, CommandError> {
    let engine = state.summary_engine.clone();
    let mg = engine
        .generate(&content)
        .await;
    Ok(mg)
}

// ---------------------------------------------------------------------------
// v1.6: Git 风格记忆版本控制命令（branch / commit / log / diff / revert / merge）
// ---------------------------------------------------------------------------

/// Tauri 命令：列出所有记忆分支。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_branch_list"))]
pub async fn memory_branch_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::memory::version_control::MemoryBranch>, CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.list_branches())
        .await
        .map_err(|e| CommandError::internal("memory_branch_list", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_branch_list", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：创建新分支。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_branch_create"))]
pub async fn memory_branch_create(
    state: State<'_, AppState>,
    name: String,
) -> Result<crate::memory::version_control::MemoryBranch, CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.create_branch(&name))
        .await
        .map_err(|e| CommandError::internal("memory_branch_create", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_branch_create", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：切换活跃分支。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_branch_checkout"))]
pub async fn memory_branch_checkout(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.checkout(&name))
        .await
        .map_err(|e| CommandError::internal("memory_branch_checkout", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_branch_checkout", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：删除分支。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_branch_delete"))]
pub async fn memory_branch_delete(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.delete_branch(&name))
        .await
        .map_err(|e| CommandError::internal("memory_branch_delete", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_branch_delete", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：在当前分支上创建 commit。
#[tauri::command]
#[instrument(skip(state, payload), fields(otel.kind = "memory_commit"))]
pub async fn memory_commit(
    state: State<'_, AppState>,
    action: String,
    target_id: String,
    payload: serde_json::Value,
    author: String,
    message: String,
) -> Result<String, CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.commit(&action, &target_id, &payload, &author, &message))
        .await
        .map_err(|e| CommandError::internal("memory_commit", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_commit", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：查看当前分支提交历史。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_log"))]
pub async fn memory_log(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<crate::memory::version_control::CommitRecord>, CommandError> {
    let vc = state.version_control.clone();
    let limit = limit.unwrap_or(50);
    tokio::task::spawn_blocking(move || vc.log(limit))
        .await
        .map_err(|e| CommandError::internal("memory_log", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_log", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：比较两个 commit 之间的差异。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_diff"))]
pub async fn memory_diff(
    state: State<'_, AppState>,
    from_commit: String,
    to_commit: String,
) -> Result<crate::memory::version_control::CommitDiff, CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.diff(&from_commit, &to_commit))
        .await
        .map_err(|e| CommandError::internal("memory_diff", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_diff", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：回滚到某个 commit（生成 revert commit，不删除历史）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_revert"))]
pub async fn memory_revert(
    state: State<'_, AppState>,
    target_commit_id: String,
    author: String,
    message: String,
) -> Result<String, CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.revert(&target_commit_id, &author, &message))
        .await
        .map_err(|e| CommandError::internal("memory_revert", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_revert", &anyhow::anyhow!("{e}")))
}

/// Tauri 命令：合并分支（将 source_branch 的 commit 追加到当前活跃分支）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_merge"))]
pub async fn memory_merge(
    state: State<'_, AppState>,
    source_branch: String,
) -> Result<Vec<String>, CommandError> {
    let vc = state.version_control.clone();
    tokio::task::spawn_blocking(move || vc.merge(&source_branch))
        .await
        .map_err(|e| CommandError::internal("memory_merge", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::internal("memory_merge", &anyhow::anyhow!("{e}")))
}
