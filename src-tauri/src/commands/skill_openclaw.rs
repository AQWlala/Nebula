//! P1-6: OpenClaw 兼容命令 — 命令行式安装 + 来源查询。
//!
//! 三个命令对标 OpenClaw 的 `/plugin marketplace add` 体验:
//!
//! * [`install_skill_from_openclaw`] — 从 OpenClaw 社区市场安装(slug 解析到 GitHub 仓库)
//! * [`install_skill_from_url`] — 从任意 URL 安装 SKILL.md(通用安装命令)
//! * [`list_skill_sources`] — 列出所有支持的技能来源(供前端显示兼容性矩阵)

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

// ---------------------------------------------------------------------------
// P1-6: SkillSourceInfo — 技能来源描述(供前端显示兼容性矩阵)
// ---------------------------------------------------------------------------

/// 技能来源信息(供前端显示来源列表 + 兼容性 badge)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceInfo {
    /// 来源标识符(与 SkillSource 枚举的 serde 小写名对齐)。
    pub id: String,
    /// 来源显示名称。
    pub name: String,
    /// 来源描述。
    pub description: String,
    /// 是否与 agentskills.io SKILL.md 格式兼容。
    pub is_compatible: bool,
}

// ---------------------------------------------------------------------------
// P1-6: install_skill_from_openclaw — 从 OpenClaw 社区安装技能
// ---------------------------------------------------------------------------

/// P1-6: 从 OpenClaw 社区市场安装技能。
///
/// slug 解析规则(与 `SkillImporter::import_from_openclaw` 对齐):
/// - `user/repo` → `https://raw.githubusercontent.com/<user>/<repo>/main/SKILL.md`
/// - `user/repo/path` → `https://raw.githubusercontent.com/<user>/<repo>/main/<path>/SKILL.md`
/// - 无 `/` 的 slug → OpenClaw 官方仓库 `openclaw/skills` 下的技能名
///
/// 返回安装成功的技能 ID。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "install_skill_from_openclaw"))]
pub async fn install_skill_from_openclaw(
    state: State<'_, AppState>,
    slug: String,
) -> Result<String, CommandError> {
    let importer = crate::skills::importer::SkillImporter::new(state.swarm.skills.store().clone());
    let result = importer.import_from_openclaw(&slug).await;
    if result.success {
        let skill_id = result
            .skill
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        Ok(skill_id)
    } else {
        Err(CommandError::internal(
            "install_skill_from_openclaw",
            &anyhow::anyhow!(
                "OpenClaw install failed: {}",
                result.error.as_deref().unwrap_or("unknown error")
            ),
        ))
    }
}

// ---------------------------------------------------------------------------
// P1-6: install_skill_from_url — 从任意 URL 安装 SKILL.md
// ---------------------------------------------------------------------------

/// P1-6: 从任意 URL 安装 SKILL.md(通用安装命令,对标 OpenClaw `/plugin marketplace add`)。
///
/// URL 必须指向 raw SKILL.md 文件(agentskills.io 兼容格式:YAML front-matter + body)。
/// 返回安装成功的技能 ID。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "install_skill_from_url"))]
pub async fn install_skill_from_url(
    state: State<'_, AppState>,
    url: String,
) -> Result<String, CommandError> {
    let importer = crate::skills::importer::SkillImporter::new(state.swarm.skills.store().clone());
    let result = importer.import_from_url(&url).await;
    if result.success {
        let skill_id = result
            .skill
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        Ok(skill_id)
    } else {
        Err(CommandError::internal(
            "install_skill_from_url",
            &anyhow::anyhow!(
                "URL install failed: {}",
                result.error.as_deref().unwrap_or("unknown error")
            ),
        ))
    }
}

// ---------------------------------------------------------------------------
// P1-6: list_skill_sources — 列出所有支持的技能来源
// ---------------------------------------------------------------------------

/// P1-6: 列出所有支持的技能来源(供前端显示兼容性矩阵 + 来源筛选器)。
///
/// 返回 `Vec<SkillSourceInfo>`,每项含 id / name / description / is_compatible。
#[tauri::command]
#[instrument(fields(otel.kind = "list_skill_sources"))]
pub async fn list_skill_sources() -> Result<Vec<SkillSourceInfo>, CommandError> {
    Ok(vec![
        SkillSourceInfo {
            id: "agentskills".to_string(),
            name: "agentskills.io".to_string(),
            description: "开放技能注册表 — SKILL.md 标准格式".to_string(),
            is_compatible: true,
        },
        SkillSourceInfo {
            id: "clawhub".to_string(),
            name: "ClawHub".to_string(),
            description: "Clawd 社区技能中心 — slug 解析到 GitHub 仓库".to_string(),
            is_compatible: true,
        },
        SkillSourceInfo {
            id: "openclaw".to_string(),
            name: "OpenClaw".to_string(),
            description: "OpenClaw 社区市场 — 与 agentskills.io 格式完全兼容".to_string(),
            is_compatible: true,
        },
        SkillSourceInfo {
            id: "teamskillshub".to_string(),
            name: "TeamSkillsHub".to_string(),
            description: "团队内部技能注册表 — 通过 REST API 拉取".to_string(),
            is_compatible: true,
        },
        SkillSourceInfo {
            id: "local".to_string(),
            name: "本地".to_string(),
            description: "本地创建或发现的技能".to_string(),
            is_compatible: false,
        },
    ])
}
