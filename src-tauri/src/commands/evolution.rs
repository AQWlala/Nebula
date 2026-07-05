//! M6 #78: 进化日志 + 回滚 Tauri 命令。
//!
//! 提供 evolution_log_list / evolution_log_get / evolution_rollback /
//! evolution_enabled / evolution_set_enabled 五个命令,供前端调用。
//!
//! ## Feature Gate
//!
//! - 前 3 个命令(log_list / log_get / rollback)由 `evolution-engine` feature 门控,
//!   因为它们依赖 EvolutionLog / Roller 等仅在该 feature 下编译的类型。
//! - 后 2 个命令(enabled / set_enabled)由 `self-evolution` feature 门控,
//!   因为 `evolution_enabled()` / `set_evolution_enabled()` 在 `evolution` 模块中,
//!   该模块整体由 `self-evolution` 门控。`evolution-engine` 隐含 `self-evolution`,
//!   因此启用进化引擎时这两个命令必然可用。
//! - feature 全 off 时这些命令不编译,前端 invoke 会返回 "command not found"。
//!
//! ## 注意
//!
//! - `evolution_run` 命令(触发 4 Phase 进化)未包含在本次实现中,
//!   因为 EvolutionEngine 需要 dispatcher 注入,且 4 Phase 是长耗时操作,
//!   需要流式事件推送(类似 master_run),留待后续迭代。
//! - `evolution_config_get` / `evolution_config_set` 也未包含,
//!   因为 EvolutionEngineConfig 当前通过代码硬编码,无运行时修改需求。

use tauri::State;
use tracing::instrument;

use crate::AppState;

// M7b #91: CommandError 仅在 evolution-engine / self-evolution feature
// 开启时被使用,门控导入避免默认构建下 unused import 警告。
#[cfg(any(feature = "evolution-engine", feature = "self-evolution"))]
use super::error::CommandError;

/// 列出全部进化日志条目(按写入顺序)。
///
/// 返回 `Vec<EvolutionLogEntry>`,空列表表示无进化记录或日志文件不存在。
#[cfg(feature = "evolution-engine")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "evolution_log_list"))]
pub async fn evolution_log_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::evolution::engine::EvolutionLogEntry>, CommandError> {
    let log = state.evolution_log.clone();
    let entries = log
        .list_all()
        .map_err(|e| CommandError::internal("evolution_log_list", &anyhow::anyhow!("{e}")))?;
    Ok(entries)
}

/// 查询单条进化日志条目(通过 entry_id)。
///
/// 找不到返回 None。
#[cfg(feature = "evolution-engine")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "evolution_log_get"))]
pub async fn evolution_log_get(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<Option<crate::evolution::engine::EvolutionLogEntry>, CommandError> {
    let log = state.evolution_log.clone();
    let entry = log
        .find_entry(&entry_id)
        .map_err(|e| CommandError::internal("evolution_log_get", &anyhow::anyhow!("{e}")))?;
    Ok(entry)
}

/// 回滚最近 N 条 Phase 4 (Soul) 进化写入。
///
/// 仅回滚 SOUL.md evolution-append section 内的段落,不回滚 L2/L3/L5 记忆
/// (历史事实不可破坏审计链)。回滚后从 evolution_log.md 删除对应条目。
///
/// `n = 0` 时无操作。`n` 超过实际条目数时按实际数量回滚,不报错。
#[cfg(feature = "evolution-engine")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "evolution_rollback"))]
pub async fn evolution_rollback(
    state: State<'_, AppState>,
    n: usize,
) -> Result<crate::evolution::engine::RollbackResult, CommandError> {
    let roller = state.roller.clone();
    let result = roller
        .rollback(n)
        .await
        .map_err(|e| CommandError::internal("evolution_rollback", &anyhow::anyhow!("{e}")))?;
    Ok(result)
}

/// 查询进化引擎运行时开关状态。
///
/// 返回 `true` 表示进化引擎已启用(可执行 4 Phase),`false` 表示已禁用。
/// 注意:即使返回 `true`,若 `evolution-engine` feature 未编译,命令仍不可用。
#[cfg(feature = "self-evolution")]
#[tauri::command]
#[instrument(fields(otel.kind = "evolution_enabled"))]
pub async fn evolution_enabled() -> Result<bool, CommandError> {
    Ok(crate::evolution::evolution_enabled())
}

/// 设置进化引擎运行时开关。
///
/// `enabled = true` 启用,`false` 禁用。禁用后 EvolutionEngine 不会自动执行 4 Phase。
/// 此开关仅影响运行时,不影响 feature flag 编译期决策。
#[cfg(feature = "self-evolution")]
#[tauri::command]
#[instrument(fields(otel.kind = "evolution_set_enabled"))]
pub async fn evolution_set_enabled(enabled: bool) -> Result<(), CommandError> {
    crate::evolution::set_evolution_enabled(enabled);
    Ok(())
}
