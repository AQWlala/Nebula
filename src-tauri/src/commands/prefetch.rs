//! T-E-A-11: Smart Prefetch Tauri 命令。
//!
//! 前端在打开文件 / 拖入文件时调用 `prefetch_for_file`,后台触发
//! [`PrefetchEngine::prefetch`] 三路检索历史对话并预热 SemanticCache。
//! 命令本身是 thin shim:从 AppState 取 `prefetch` 引擎,调用 prefetch,
//! 返回 [`PrefetchStats`]。所有降级在引擎内部完成,命令层不 panic。
//!
//! ## 设计要点
//!
//! * **非阻塞**:prefetch 内部用 `tokio::join!` 并行三路检索,每路
//!   都通过 `spawn_blocking` 或 async I/O,不阻塞 Tauri runtime。
//! * **降级**:embed 失败 / 路径不存在 / 无历史 全部降级为 debug
//!   日志,返回 `pairs_prefetched = 0` 的 stats,不返回错误。
//! * **5 分钟去重**:引擎内部维护 `HashMap<PathBuf, Instant>`,
//!   5 分钟内重复调用直接返回 `skipped_dedup = true`。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::llm::prefetch::PrefetchStats;
use crate::AppState;

/// T-E-A-11: 为指定文件路径预取相关历史对话到 SemanticCache。
///
/// 前端在 `nebula://open-file` / `nebula://drag-drop` 监听器
/// 内调用此命令。命令非阻塞,降级时返回 0 pairs(不报错)。
///
/// ## 参数
///
/// - `path` — 文件绝对路径。引擎内部用 `PathBuf::canonicalize()` 标准化。
///
/// ## 返回
///
/// [`PrefetchStats`] — 包含 path/pairs_prefetched/bm25_hits/vector_hits/
/// path_hits/skipped_dedup/elapsed_ms 七个字段。
///
/// ## 错误
///
/// 仅在 AppState 未初始化(prefetch 字段缺失)时返回 `internal` 错误,
/// 正常降级路径不报错。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "prefetch_for_file"))]
pub async fn prefetch_for_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<PrefetchStats, CommandError> {
    if path.trim().is_empty() {
        return Err(CommandError::validation("prefetch_for_file: path is empty"));
    }
    let engine = state.prefetch.clone();
    let stats = engine.prefetch(&path).await;
    Ok(stats)
}
