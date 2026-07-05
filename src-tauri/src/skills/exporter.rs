//! Skill exporter — T-E-S-45 ClawHub bidirectional compatibility.
//!
//! 序列化 nebula [`Skill`] 为 agentskills.io `SKILL.md` 格式
//! (YAML front-matter + Markdown body)。这是 [`importer`] 的逆函数:
//!
//! ```text
//! importer::from_skill_md : SKILL.md -> CreateSkillRequest
//! exporter::to_skill_md   : Skill -> SKILL.md
//! ```
//!
//! ## 字段映射
//!
//! 与 `importer::parse_skill_md_inner` 字段契约对称:
//!
//! | SKILL.md front-matter | Skill 字段          |
//! |-----------------------|---------------------|
//! | `name`                | `skill.name`        |
//! | `description`         | `skill.description` |
//! | `category`            | `skill.language`    |
//! | `tags`                | `skill.tags`        |
//! | `trust_level`         | `skill.trust_level` |
//! | `permissions`         | `skill.permissions` |
//! | `capabilities`        | `skill.capabilities`(反向映射) |
//! | body                  | `skill.code`        |
//!
//! 扩展字段(`platform` / `min_confidence` / `activation_condition`)写入
//! front-matter 自定义键。导入侧目前不解析这些键,但写入以保证未来兼容。
//!
//! ## capabilities 反向映射
//!
//! `CapabilitySet` → `Vec<String>`。对照 `importer.rs` 的 8 个字符串匹配项,
//! 使用 `Capability::Display` 实现(`"file:read"` / `"network"` 等)。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::info;

use super::sandbox::{Capability, CapabilitySet};
use super::types::{ActivationCondition, Skill};

/// SKILL.md 序列化器。
pub struct SkillExporter;

// T-E-S-36: re-export SkillManifest 供未来 to_skill_md 改用协议层类型。
// 当前 to_skill_md 仍接受 &Skill(类型不兼容,保留旧逻辑);未来 exporter
// 可增加 From<&Skill> for SkillManifest 转换后切换。
pub use super::protocol::SkillManifest;

/// YAML front-matter 的序列化结构。
///
/// 字段名与 `importer::parse_skill_md_inner` 读取的键严格对称,
/// 确保 8 个核心字段可无损往返。
#[derive(Serialize)]
struct SkillFrontMatter<'a> {
    name: &'a str,
    description: &'a str,
    category: &'a str,
    #[serde(default)]
    tags: &'a [String],
    #[serde(default)]
    trust_level: u8,
    #[serde(default)]
    permissions: &'a [String],
    #[serde(default)]
    capabilities: Vec<String>,
    /// 扩展字段:平台限制。`None` 时不写入 front-matter。
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<&'a [String]>,
    /// 扩展字段:最小置信度。`None` 时不写入 front-matter。
    #[serde(skip_serializing_if = "Option::is_none")]
    min_confidence: Option<f32>,
    /// 扩展字段:激活条件。`None` 时不写入 front-matter。
    #[serde(skip_serializing_if = "Option::is_none")]
    activation_condition: Option<&'a ActivationCondition>,
}

impl SkillExporter {
    /// 序列化 [`Skill`] 为 `SKILL.md` 字符串。
    ///
    /// 输出格式:
    /// ```text
    /// ---
    /// name: ...
    /// description: ...
    /// category: ...
    /// tags:
    ///   - ...
    /// trust_level: ...
    /// permissions:
    ///   - ...
    /// capabilities:
    ///   - ...
    /// ---
    ///
    /// <skill.code>
    /// ```
    pub fn to_skill_md(skill: &Skill) -> Result<String> {
        let caps_strings = capabilities_to_strings(&skill.capabilities);
        let front_matter = SkillFrontMatter {
            name: &skill.name,
            description: &skill.description,
            category: &skill.language,
            tags: &skill.tags,
            trust_level: skill.trust_level,
            permissions: &skill.permissions,
            capabilities: caps_strings,
            platform: skill.platform.as_deref(),
            min_confidence: skill.min_confidence,
            activation_condition: skill.activation_condition.as_ref(),
        };

        let yaml = serde_yaml::to_string(&front_matter)
            .context("failed to serialize SKILL.md front-matter as YAML")?;

        // serde_yaml::to_string 追加一个尾部换行;去掉后再按 SKILL.md
        // 规范重新拼接分隔符。
        let yaml_trimmed = yaml.trim_end();

        let body = if skill.code.is_empty() {
            String::new()
        } else {
            format!("\n{}", skill.code)
        };

        Ok(format!("---\n{yaml_trimmed}\n---\n{body}"))
    }

    /// 写入 `<dir>/<skill.name>/SKILL.md`。
    ///
    /// 自动创建 `<dir>/<skill.name>/` 目录(含父目录)。返回最终写入的
    /// `SKILL.md` 完整路径。
    pub fn export_to_dir(skill: &Skill, dir: &Path) -> Result<PathBuf> {
        let skill_dir = dir.join(&skill.name);
        std::fs::create_dir_all(&skill_dir)
            .with_context(|| format!("failed to create skill dir: {}", skill_dir.display()))?;

        let md = Self::to_skill_md(skill)?;
        let md_path = skill_dir.join("SKILL.md");
        std::fs::write(&md_path, &md)
            .with_context(|| format!("failed to write SKILL.md: {}", md_path.display()))?;

        info!(
            target: "nebula.skills.exporter",
            path = %md_path.display(),
            skill_id = %skill.id,
            "SKILL.md exported"
        );
        Ok(md_path)
    }
}

/// `CapabilitySet` → `Vec<String>` 反向映射。
///
/// 使用 `Capability::Display` 实现,输出 `"file:read"` / `"network"` /
/// `"subprocess"` / `"env:read"` / `"clipboard:read"` / `"llm:call"` /
/// `"db:access"` / `"file:write"` —— 与 `importer.rs` 的字符串匹配项
/// 完全对称(每个字符串都落在 importer 的 8 个 `match` 分支上)。
fn capabilities_to_strings(caps: &CapabilitySet) -> Vec<String> {
    caps.granted()
        .iter()
        .map(|c: &Capability| c.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::importer::SkillImporter;
    use crate::skills::sandbox::{Capability, CapabilitySet};
    use crate::skills::types::Skill;

    /// 构造一个测试用 `Skill`,包含全部 8 个核心字段 + 部分扩展字段。
    fn sample_skill() -> Skill {
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::FileRead);
        caps.grant(Capability::LlmCall);

        Skill {
            id: "sk-test-001".to_string(),
            name: "text-summarizer".to_string(),
            description: "Summarize long texts into concise bullet points.".to_string(),
            code: "# Text Summarizer\n\n## Instructions\n1. Read the input text\n2. Identify key points\n3. Output a concise summary".to_string(),
            language: "llm".to_string(),
            tags: vec!["summarization".to_string(), "nlp".to_string()],
            usage_count: 5,
            avg_rating: 4.5,
            rating_count: 2,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            source_memory_id: None,
            activation_condition: None,
            platform: Some(vec!["linux".to_string(), "macos".to_string()]),
            min_confidence: Some(0.8),
            trust_level: 2,
            permissions: vec!["file:read".to_string()],
            capabilities: caps,
        }
    }

    #[test]
    fn test_to_skill_md_generates_yaml_front_matter() {
        let skill = sample_skill();
        let md = SkillExporter::to_skill_md(&skill).unwrap();

        // 1. 必须以 YAML front-matter 起始分隔符开头。
        assert!(
            md.starts_with("---\n"),
            "SKILL.md must start with '---\\n', got: {md:?}"
        );

        // 2. 必须包含闭合分隔符。
        let rest = &md[4..];
        let end = rest
            .find("---")
            .expect("SKILL.md must contain closing '---'");

        // 3. front-matter 段必须包含全部 7 个核心字段键。
        let front_matter = &rest[..end];
        assert!(front_matter.contains("name: text-summarizer"));
        assert!(front_matter.contains("description:"));
        assert!(front_matter.contains("category: llm"));
        assert!(front_matter.contains("trust_level: 2"));
        // tags / permissions / capabilities 是 YAML 列表
        assert!(front_matter.contains("tags:"));
        assert!(front_matter.contains("permissions:"));
        assert!(front_matter.contains("capabilities:"));

        // 4. body 段(skill.code)必须出现在闭合分隔符之后。
        let body = rest[end + 3..].trim();
        assert!(
            body.contains("# Text Summarizer"),
            "body must contain skill.code, got: {body:?}"
        );
    }

    #[test]
    fn test_round_trip() {
        // Skill → SKILL.md → from_skill_md → 字段一致(8 个核心字段)。
        let original = sample_skill();
        let md = SkillExporter::to_skill_md(&original).unwrap();
        let parsed = SkillImporter::from_skill_md(&md).unwrap();

        // 8 个核心字段无损往返。
        assert_eq!(parsed.name, original.name, "name round-trip mismatch");
        assert_eq!(
            parsed.description, original.description,
            "description round-trip mismatch"
        );
        assert_eq!(
            parsed.language, original.language,
            "language/category round-trip mismatch"
        );
        assert_eq!(parsed.code, original.code, "code/body round-trip mismatch");
        assert_eq!(parsed.tags, original.tags, "tags round-trip mismatch");
        assert_eq!(
            parsed.trust_level, original.trust_level,
            "trust_level round-trip mismatch"
        );
        assert_eq!(
            parsed.permissions, original.permissions,
            "permissions round-trip mismatch"
        );
        assert_eq!(
            parsed.capabilities, original.capabilities,
            "capabilities round-trip mismatch"
        );

        // 扩展字段:importer 不解析,因此 round-trip 后应为 None/默认。
        // (这是 spec 允许的:核心 8 字段无损,扩展字段仅写入 front-matter。)
        assert_eq!(parsed.platform, None);
        assert_eq!(parsed.min_confidence, None);
        assert_eq!(parsed.activation_condition, None);
    }

    #[test]
    fn test_capabilities_reverse_mapping() {
        // 全部 8 个 Capability 都应反向映射为 importer 可识别的字符串。
        let caps = CapabilitySet::full_trust();
        let strings = capabilities_to_strings(&caps);

        // full_trust 授予全部 8 个能力。
        assert_eq!(strings.len(), 8, "full_trust must grant all 8 capabilities");

        // 每个字符串都应落在 importer 的 match 分支中 —— 验证方式:
        // 重新通过 importer 的 from_skill_md 解析,确认 capabilities 完整还原。
        let skill = Skill {
            id: "sk-caps".to_string(),
            name: "caps-test".to_string(),
            description: "tests capability reverse mapping".to_string(),
            code: "body".to_string(),
            language: "llm".to_string(),
            tags: vec![],
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: caps,
        };

        let md = SkillExporter::to_skill_md(&skill).unwrap();
        let parsed = SkillImporter::from_skill_md(&md).unwrap();

        // 反向 + 正向映射后,能力集应当完全一致(full_trust)。
        assert_eq!(
            parsed.capabilities,
            skill.capabilities,
            "capabilities reverse mapping must be symmetric"
        );
        // 显式验证每个能力都存在。
        for cap in Capability::ALL {
            assert!(
                parsed.capabilities.has(*cap),
                "capability {cap} lost in round-trip"
            );
        }
    }

    #[test]
    fn test_export_to_dir_creates_file() {
        let skill = sample_skill();
        let dir = std::env::temp_dir().join(format!(
            "nebula-exporter-test-{}",
            uuid::Uuid::new_v4()
        ));

        let path = SkillExporter::export_to_dir(&skill, &dir).unwrap();

        // 1. 返回路径必须是 <dir>/<skill.name>/SKILL.md
        assert!(path.ends_with("SKILL.md"));
        assert!(path.starts_with(&dir));
        assert!(path.to_string_lossy().contains("text-summarizer"));

        // 2. 文件必须存在且内容可被 from_skill_md 解析。
        assert!(path.exists(), "SKILL.md file must exist at {path:?}");
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed = SkillImporter::from_skill_md(&content).unwrap();
        assert_eq!(parsed.name, "text-summarizer");

        // 3. 清理。
        let _ = std::fs::remove_dir_all(&dir);
    }
}
