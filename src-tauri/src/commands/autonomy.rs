//! T-E-S-50: Autonomy Tauri 命令。
//!
//! 前端通过这些命令读写全局自主度等级、枚举可用等级、调试路由。
//! 等级存储在 `crate::autonomy::AutonomyState`(进程内全局单例)。
//!
//! ## P1 持久化
//! P0 阶段等级仅存内存;P1 会落库到 SQLite `app_settings` 表
//! (key="autonomy_level"),届时命令会改为读写 `AppState.sqlite`。

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::autonomy::{
    default_config, get_level, set_level, AutonomyConfig, AutonomyDispatch, AutonomyLevel,
    AutonomyRouter,
};
use crate::commands::error::CommandError;
use crate::AppState;

/// `AutonomyConfig` 的 DTO(前端可读,与 `AutonomyConfig` 字段一致)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyConfigDto {
    pub requires_approval: bool,
    pub runs_in_background: bool,
    pub auto_execute: bool,
    pub allows_inline_ui: bool,
    pub routes_to_swarm: bool,
    pub routes_to_plan: bool,
}

impl From<AutonomyConfig> for AutonomyConfigDto {
    fn from(c: AutonomyConfig) -> Self {
        Self {
            requires_approval: c.requires_approval,
            runs_in_background: c.runs_in_background,
            auto_execute: c.auto_execute,
            allows_inline_ui: c.allows_inline_ui,
            routes_to_swarm: c.routes_to_swarm,
            routes_to_plan: c.routes_to_plan,
        }
    }
}

/// 单个等级的元信息(供前端渲染滑块)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyLevelInfo {
    /// Wire 字符串("L0".."L5")。
    pub level: String,
    /// 数值索引(0..=5)。
    pub index: u8,
    /// 英文标签。
    pub label: String,
    /// 中文标签。
    pub label_zh: String,
    /// 英文描述。
    pub description: String,
    /// 中文描述。
    pub description_zh: String,
    /// 该等级的行为参数。
    pub config: AutonomyConfigDto,
}

/// 读取当前自主度等级。
///
/// 返回 wire 字符串("L0".."L5")。默认 "L2"。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "autonomy_get_level"))]
pub async fn autonomy_get_level(_state: State<'_, AppState>) -> Result<String, CommandError> {
    Ok(get_level().as_str().to_string())
}

/// 设置自主度等级。
///
/// `level` 接受 "L0".."L5"(大小写不敏感)。无效值返回 validation 错误。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "autonomy_set_level"))]
pub async fn autonomy_set_level(
    _state: State<'_, AppState>,
    level: String,
) -> Result<(), CommandError> {
    let parsed = AutonomyLevel::parse(&level).ok_or_else(|| {
        CommandError::validation("autonomy_set_level").with_details(format!(
            "invalid autonomy level '{level}', expected one of L0/L1/L2/L3/L4/L5"
        ))
    })?;
    set_level(parsed);
    Ok(())
}

/// 枚举全部 6 档等级(含 label/description/config),供前端渲染滑块。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "autonomy_list_levels"))]
pub async fn autonomy_list_levels(
    _state: State<'_, AppState>,
) -> Result<Vec<AutonomyLevelInfo>, CommandError> {
    Ok(AutonomyLevel::all()
        .iter()
        .map(|&lvl| AutonomyLevelInfo {
            level: lvl.as_str().to_string(),
            index: lvl.as_u8(),
            label: lvl.label().to_string(),
            label_zh: lvl.label_zh().to_string(),
            description: lvl.description().to_string(),
            description_zh: lvl.description_zh().to_string(),
            config: default_config(lvl).into(),
        })
        .collect())
}

/// 调试用:返回指定等级 + 任务的路由决策(Debug 字符串)。
///
/// 不实际执行任务,只展示 `AutonomyRouter::route` 的决策结果,
/// 便于前端/测试验证路由逻辑。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "autonomy_route"))]
pub async fn autonomy_route(
    _state: State<'_, AppState>,
    level: String,
    task: String,
) -> Result<String, CommandError> {
    let parsed = AutonomyLevel::parse(&level).ok_or_else(|| {
        CommandError::validation("autonomy_route")
            .with_details(format!("invalid autonomy level '{level}'"))
    })?;
    let dispatch: AutonomyDispatch = AutonomyRouter.route(parsed, &task);
    Ok(format!("{dispatch:?}"))
}
