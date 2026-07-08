//! M6 #78: 进化日志 + 回滚 Tauri 命令。
//!
//! 提供 evolution_log_list / evolution_log_get / evolution_rollback /
//! evolution_enabled / evolution_set_enabled / evolution_run 六个命令,供前端调用。
//!
//! ## Feature Gate
//!
//! - 前 3 个命令(log_list / log_get / rollback)由 `evolution-engine` feature 门控,
//!   因为它们依赖 EvolutionLog / Roller 等仅在该 feature 下编译的类型。
//! - 后 3 个命令(enabled / set_enabled / run)由 `self-evolution` feature 门控。
//! - feature 全 off 时这些命令不编译,前端 invoke 会返回 "command not found"。
//!
//! ## evolution_run
//!
//! 触发 4 Phase 进化管线(Extract → Compile → Reflect → Soul)。
//! 长耗时操作,前端应显示进度指示器。
//! 结果通过 Tauri event "evolution-completed" 推送。

#[cfg(any(feature = "evolution-engine", feature = "self-evolution"))]
use tauri::State;
#[cfg(any(feature = "evolution-engine", feature = "self-evolution"))]
use tracing::instrument;

#[cfg(any(feature = "evolution-engine", feature = "self-evolution"))]
use crate::AppState;

// M7b #91: CommandError 仅在 evolution-engine / self-evolution feature
// 开启时被使用,门控导入避免默认构建下 unused import 警告。
#[cfg(any(feature = "evolution-engine", feature = "self-evolution"))]
use super::error::CommandError;

// P2-A: Emitter trait needed for app.emit() in evolution_run
#[cfg(feature = "evolution-engine")]
use tauri::Emitter;

/// 列出全部进化日志条目(按写入顺序)。
///
/// 返回 `Vec<EvolutionLogEntry>`,空列表表示无进化记录或日志文件不存在。
#[cfg(feature = "evolution-engine")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "evolution_log_list"))]
pub async fn evolution_log_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::evolution::engine::EvolutionLogEntry>, CommandError> {
    let log = state.swarm.evolution_log.clone();
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
    let log = state.swarm.evolution_log.clone();
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
    let roller = state.swarm.roller.clone();
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

/// 触发 4 Phase 进化管线(Extract → Compile → Reflect → Soul)。
///
/// 这是长耗时操作(LLM 调用 × 4 阶段),前端应显示进度指示器。
/// 完成后通过 Tauri event "evolution-completed" 推送结果。
///
/// `master_id` 用于记忆 domain 隔离,默认 "default"。
///
/// ## 错误处理
///
/// - 进化引擎未启用 → 返回错误提示
/// - 运行时开关关闭 → 自动启用后执行
/// - 单个 Phase 失败 → 记 warning 并继续下一 Phase(不中断)
/// - 所有 Phase 完成 → 返回 EvolutionResult
#[cfg(feature = "evolution-engine")]
#[tauri::command]
#[instrument(skip(state, app), fields(otel.kind = "evolution_run"))]
pub async fn evolution_run(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    master_id: Option<String>,
) -> Result<crate::evolution::engine::EvolutionResult, CommandError> {
    let engine = state.swarm.evolution_engine.as_ref().ok_or_else(|| {
        CommandError::internal(
            "evolution_run",
            &anyhow::anyhow!(
                "evolution engine not initialized — enable 'evolution-engine' feature"
            ),
        )
    })?;

    let mid = master_id.unwrap_or_else(|| "default".to_string());

    // 确保运行时开关已启用
    if !crate::evolution::evolution_enabled() {
        crate::evolution::set_evolution_enabled(true);
    }

    tracing::info!(target: "nebula.evolution", master_id = %mid, "starting 4-phase evolution run");

    let result = engine
        .run(&mid)
        .await
        .map_err(|e| CommandError::internal("evolution_run", &anyhow::anyhow!("{e}")))?;

    tracing::info!(
        target: "nebula.evolution",
        master_id = %mid,
        degraded = result.degraded,
        warnings = result.warnings.len(),
        phases = result.phases.len(),
        "evolution run completed"
    );

    // 推送完成事件到前端
    let _ = app.emit("evolution-completed", &result);

    Ok(result)
}

/// 查询进化引擎是否已初始化(feature flag + AppState)。
///
/// 与 `evolution_enabled` 的区别:
/// - `evolution_enabled` 检查运行时开关
/// - `evolution_engine_ready` 检查引擎是否已构造(feature on + bootstrap 完成)
#[cfg(feature = "evolution-engine")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "evolution_engine_ready"))]
pub async fn evolution_engine_ready(state: State<'_, AppState>) -> Result<bool, CommandError> {
    Ok(state.swarm.evolution_engine.is_some())
}
