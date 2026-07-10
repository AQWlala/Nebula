//! P0-6 SkillAutoInventor Tauri 命令层。
//!
//! 暴露 5 个命令给前端,用于查看 / 审核 / 配置自动发明的技能:
//!
//! * `auto_invent_get_patterns` —— 获取已检测到的重复模式列表。
//! * `auto_invent_accept_pattern` —— 接受模式并生成技能(返回技能名)。
//! * `auto_invent_reject_pattern` —— 拒绝模式。
//! * `auto_invent_get_config` —— 获取当前配置。
//! * `auto_invent_set_config` —— 更新配置(启用 / 阈值)。
//!
//! ## 单例设计
//!
//! [`SkillAutoInventor`] 的核心价值在于跨调用累积操作历史与已检测模式。
//! 若每次命令都新建实例,状态会丢失,自动发明机制就失去意义。
//! 因此本模块用 `once_cell::sync::Lazy` 持有一个进程级单例,
//! 所有命令共享同一实例。
//!
//! 单例使用默认配置初始化(阈值 5 / 历史 1000 / 路径 `~/.nebula/skills/auto-invented/`)。
//! 运行时配置变更通过 `auto_invent_set_config` 命令持久化到单例内存中
//! (进程重启后回到默认值,与现有 evolution_enabled / soul_system_enabled
//! 等运行时开关行为一致)。

use once_cell::sync::Lazy;
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::skills::auto_inventor::{AutoInventorConfig, DetectedPattern, SkillAutoInventor};
use crate::AppState;

/// 进程级单例:所有命令共享同一 [`SkillAutoInventor`] 实例。
///
/// 用默认配置初始化。运行时通过 `auto_invent_set_config` 修改启用状态
/// 与阈值。`history_size` / `skills_dir` 不通过命令修改(避免运行时
/// 频繁迁移缓冲区或路径混乱)。
static AUTO_INVENTOR: Lazy<SkillAutoInventor> = Lazy::new(SkillAutoInventor::with_defaults);

/// 获取已检测到的重复模式列表。
///
/// 返回 [`Vec<DetectedPattern>`],按 `first_seen` 升序排列。
/// 仅返回历史检测到的模式;若需要触发新一轮检测,应在调用本命令前
/// 通过 `SkillEngine` / `audit` 系统调用 `record_operation` +
/// `detect_patterns`(本命令不触发检测,只读已有状态)。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "auto_invent_get_patterns"))]
pub async fn auto_invent_get_patterns(
    _state: State<'_, AppState>,
) -> Result<Vec<DetectedPattern>, CommandError> {
    Ok(AUTO_INVENTOR.list_patterns().await)
}

/// 接受模式并生成技能。
///
/// 流程:
/// 1. 调用 [`SkillAutoInventor::review_pattern`] (accepted=true)。
/// 2. 若成功,返回生成的技能名(形如 `auto-invented-<8 位 hash>`)。
///
/// 草稿写入 `<skills_dir>/<skill-name>/SKILL.md`,frontmatter 中
/// `trust_level: 0`(安全红线 —— 用户须手动提升后才能在沙箱外执行)。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "auto_invent_accept_pattern"))]
pub async fn auto_invent_accept_pattern(
    _state: State<'_, AppState>,
    pattern_id: String,
) -> Result<String, CommandError> {
    let path = AUTO_INVENTOR
        .review_pattern(&pattern_id, true)
        .await
        .map_err(|e| CommandError::internal("auto_invent_accept_pattern", &anyhow::anyhow!("{e}")))?
        .ok_or_else(|| {
            CommandError::internal(
                "auto_invent_accept_pattern",
                &anyhow::anyhow!("review_pattern returned no path for accepted pattern"),
            )
        })?;
    // 返回技能名(目录名),便于前端后续引用。
    let skill_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| pattern_id.clone());
    Ok(skill_name)
}

/// 拒绝模式。
///
/// 仅标记 `review_status = "rejected"`,不写文件。已生成的草稿文件
/// (若有)不会被删除(避免误删用户后续编辑)。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "auto_invent_reject_pattern"))]
pub async fn auto_invent_reject_pattern(
    _state: State<'_, AppState>,
    pattern_id: String,
) -> Result<(), CommandError> {
    AUTO_INVENTOR
        .review_pattern(&pattern_id, false)
        .await
        .map_err(|e| {
            CommandError::internal("auto_invent_reject_pattern", &anyhow::anyhow!("{e}"))
        })?;
    Ok(())
}

/// 获取当前配置。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "auto_invent_get_config"))]
pub async fn auto_invent_get_config(
    _state: State<'_, AppState>,
) -> Result<AutoInventorConfig, CommandError> {
    Ok(AUTO_INVENTOR.config())
}

/// 更新配置。
///
/// * `enabled` —— `Some(b)` 设置启用状态;`None` 保持不变。
/// * `threshold` —— `Some(n)` 设置 `pattern_threshold`(必须 >= 2);
///   `None` 保持不变。
///
/// `history_size` / `skills_dir` 不通过本命令修改。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "auto_invent_set_config"))]
pub async fn auto_invent_set_config(
    _state: State<'_, AppState>,
    enabled: Option<bool>,
    threshold: Option<usize>,
) -> Result<(), CommandError> {
    AUTO_INVENTOR
        .set_config(enabled, threshold)
        .map_err(|e| CommandError::validation(format!("auto_invent_set_config: {e}")))
}
