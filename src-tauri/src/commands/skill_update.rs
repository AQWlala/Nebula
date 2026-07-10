//! P2-5: 技能更新检查命令 — 远端版本比对 + 一键更新。
//!
//! 三个命令对标应用商店的"检查更新"体验:
//!
//! * [`check_skill_updates`] — 遍历所有已安装技能,从远端拉取 SKILL.md frontmatter,
//!   比对 version 字段,返回 `Vec<SkillUpdateInfo>`。
//! * [`update_skill`] — 一键更新单个技能(拉取远端 SKILL.md,替换本地文件,保留 trust_level)。
//! * [`update_all_skills`] — 一键更新所有有更新的技能,返回更新数量。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::skills::marketplace::SkillUpdateInfo;
use crate::AppState;

// ---------------------------------------------------------------------------
// check_skill_updates — 检查所有技能的远端版本更新
// ---------------------------------------------------------------------------

/// P2-5: 检查所有已安装技能的远端版本更新。
///
/// 遍历本地存储中的所有技能,对于已注册 source URL 的技能:
/// 1. 从远端拉取最新 SKILL.md frontmatter
/// 2. 解析远端 version 字段
/// 3. 与本地 version 做 semver 比对
/// 4. 返回 `Vec<SkillUpdateInfo>`,前端根据 `update_available` 字段显示更新按钮
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "check_skill_updates"))]
pub async fn check_skill_updates(
    state: State<'_, AppState>,
) -> Result<Vec<SkillUpdateInfo>, CommandError> {
    let marketplace = state.swarm.marketplace.clone();
    let updates = marketplace.check_remote_updates().await;
    Ok(updates)
}

// ---------------------------------------------------------------------------
// update_skill — 一键更新单个技能
// ---------------------------------------------------------------------------

/// P2-5: 一键更新单个技能。
///
/// 从远端拉取最新 SKILL.md,替换本地技能文件,保留用户的 trust_level 设置。
/// 更新完成后刷新 marketplace 索引。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "update_skill"))]
pub async fn update_skill(
    state: State<'_, AppState>,
    skill_id: String,
) -> Result<(), CommandError> {
    let marketplace = state.swarm.marketplace.clone();
    marketplace
        .update_skill(&skill_id)
        .await
        .map_err(|e| CommandError::internal("update_skill", &anyhow::anyhow!(e)))
}

// ---------------------------------------------------------------------------
// update_all_skills — 一键更新所有有更新的技能
// ---------------------------------------------------------------------------

/// P2-5: 一键更新所有有更新的技能,返回成功更新的数量。
///
/// 先调用 `check_remote_updates` 获取更新列表,然后对 `update_available == true`
/// 的技能逐个调用 `update_skill`。单个技能更新失败不中断整体流程,仅跳过该技能。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "update_all_skills"))]
pub async fn update_all_skills(state: State<'_, AppState>) -> Result<usize, CommandError> {
    let marketplace = state.swarm.marketplace.clone();
    let updates = marketplace.check_remote_updates().await;
    let mut updated_count = 0usize;
    for info in &updates {
        if !info.update_available {
            continue;
        }
        // 单个技能更新失败不中断整体流程,仅跳过。
        if marketplace.update_skill(&info.skill_id).await.is_ok() {
            updated_count += 1;
        }
    }
    Ok(updated_count)
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::marketplace::{semver_compare, SkillUpdateInfo};

    // --- semver 比对逻辑测试 ---

    #[test]
    fn semver_compare_patch_diff() {
        // 1.0.0 < 1.0.1
        assert_eq!(semver_compare("1.0.0", "1.0.1"), std::cmp::Ordering::Less);
    }

    #[test]
    fn semver_compare_minor_diff() {
        // 1.0.0 < 1.1.0
        assert_eq!(semver_compare("1.0.0", "1.1.0"), std::cmp::Ordering::Less);
    }

    #[test]
    fn semver_compare_major_diff() {
        // 1.0.0 < 2.0.0
        assert_eq!(semver_compare("1.0.0", "2.0.0"), std::cmp::Ordering::Less);
    }

    #[test]
    fn semver_compare_equal() {
        // 1.0.0 == 1.0.0
        assert_eq!(semver_compare("1.0.0", "1.0.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn semver_compare_greater() {
        // 2.1.3 > 1.5.7
        assert_eq!(
            semver_compare("2.1.3", "1.5.7"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn semver_compare_invalid_version() {
        // 无法解析的版本视为 (0, 0, 0)
        assert_eq!(semver_compare("latest", "0.0.0"), std::cmp::Ordering::Equal);
        assert_eq!(semver_compare("1.2", "0.0.0"), std::cmp::Ordering::Equal);
    }

    // --- SkillUpdateInfo 序列化测试 ---

    #[test]
    fn skill_update_info_serializes_with_all_fields() {
        let info = SkillUpdateInfo {
            skill_id: "test-skill".into(),
            skill_name: "Test Skill".into(),
            current_version: "1.0.0".into(),
            latest_version: "1.2.0".into(),
            update_available: true,
            source_url: Some("https://example.com/SKILL.md".into()),
            changelog: Some("Bug fixes and performance improvements".into()),
        };
        let json = serde_json::to_string(&info).expect("serialization failed");
        assert!(json.contains("\"skill_id\":\"test-skill\""));
        assert!(json.contains("\"latest_version\":\"1.2.0\""));
        assert!(json.contains("\"update_available\":true"));
        assert!(json.contains("\"changelog\":\"Bug fixes and performance improvements\""));
    }

    #[test]
    fn skill_update_info_serializes_with_null_fields() {
        let info = SkillUpdateInfo {
            skill_id: "local-skill".into(),
            skill_name: "Local Skill".into(),
            current_version: "1.0.0".into(),
            latest_version: String::new(),
            update_available: false,
            source_url: None,
            changelog: None,
        };
        let json = serde_json::to_string(&info).expect("serialization failed");
        assert!(json.contains("\"source_url\":null"));
        assert!(json.contains("\"changelog\":null"));
        assert!(json.contains("\"update_available\":false"));
    }

    #[test]
    fn skill_update_info_round_trips_through_json() {
        let info = SkillUpdateInfo {
            skill_id: "round-trip".into(),
            skill_name: "Round Trip".into(),
            current_version: "0.1.0".into(),
            latest_version: "0.2.0".into(),
            update_available: true,
            source_url: Some("https://example.com/skill.md".into()),
            changelog: None,
        };
        let json = serde_json::to_string(&info).expect("serialization failed");
        let deserialized: SkillUpdateInfo =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(deserialized.skill_id, "round-trip");
        assert_eq!(deserialized.current_version, "0.1.0");
        assert_eq!(deserialized.latest_version, "0.2.0");
        assert!(deserialized.update_available);
        assert!(deserialized.changelog.is_none());
    }
}
