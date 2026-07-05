//! M7b #97: Soul 系统运行时开关 Tauri 命令。
//!
//! 提供 soul_system_enabled / soul_system_set_enabled 两个命令,供前端调用。
//! 对齐 evolution_enabled / evolution_set_enabled 模式。
//!
//! ## Feature Gate
//!
//! 这两个命令由 `soul-system` feature 门控,因为 `soul_system_enabled()` /
//! `set_soul_system_enabled()` 在 `soul` 模块中,该模块整体由 `soul-system` 门控。
//! feature off 时这些命令不编译,前端 invoke 会返回 "command not found"。
//!
//! ## 设计
//!
//! Soul 系统的运行时开关遵循 ADR-004 feature flag 策略:
//! - 编译期:`#[cfg(feature = "soul-system")]` gate
//! - 运行时:`SOUL_SYSTEM_ENABLED: AtomicBool`(默认 false)
//! - 初始化:lib.rs setup 阶段调用 `soul::init_from_env()` 读取环境变量
//! - 运行时切换:Settings UI 通过本命令调用 `set_soul_system_enabled()`

use tracing::instrument;

#[cfg(feature = "soul-system")]
use super::CommandError;

/// 查询 Soul 系统运行时开关状态。
///
/// 返回 `true` 表示 Soul 系统已启用(SoulCompiler 可执行),`false` 表示已禁用。
/// 注意:即使返回 `true`,若 `soul-system` feature 未编译,命令仍不可用。
#[cfg(feature = "soul-system")]
#[tauri::command]
#[instrument(fields(otel.kind = "soul_system_enabled"))]
pub async fn soul_system_enabled() -> Result<bool, CommandError> {
    Ok(crate::soul::soul_system_enabled())
}

/// 设置 Soul 系统运行时开关。
///
/// `enabled = true` 启用,`false` 禁用。禁用后 SoulCompiler 不会自动执行。
/// 此开关仅影响运行时,不影响 feature flag 编译期决策。
#[cfg(feature = "soul-system")]
#[tauri::command]
#[instrument(fields(otel.kind = "soul_system_set_enabled"))]
pub async fn soul_system_set_enabled(enabled: bool) -> Result<(), CommandError> {
    crate::soul::set_soul_system_enabled(enabled);
    Ok(())
}
