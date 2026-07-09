//! Skill-related DTOs.
//!
//! These types are the wire shape for both the Tauri command layer and
//! the gRPC SkillService. They map 1:1 onto the gRPC proto messages
//! in `proto/nebula.proto`.

use serde::{Deserialize, Serialize};

use super::sandbox::CapabilitySet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActivationCondition {
    #[serde(rename = "keyword")]
    Keyword { pattern: String },
    #[serde(rename = "intent")]
    Intent { category: String },
    #[serde(rename = "context")]
    Context { key: String, value: String },
    #[serde(rename = "always")]
    Always,
}

impl ActivationCondition {
    pub fn matches(
        &self,
        input: &str,
        context: &std::collections::HashMap<String, String>,
    ) -> bool {
        match self {
            ActivationCondition::Always => true,
            ActivationCondition::Keyword { pattern } => {
                input.to_lowercase().contains(&pattern.to_lowercase())
            }
            ActivationCondition::Intent { category } => {
                input.to_lowercase().contains(&category.to_lowercase())
            }
            ActivationCondition::Context { key, value } => {
                context.get(key).map(|v| v == value).unwrap_or(false)
            }
        }
    }
}

/// A skill record. Persisted in the `skills` table (see
/// `migrations/001_initial.sql`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub code: String,
    pub language: String,
    pub tags: Vec<String>,
    pub usage_count: u32,
    pub avg_rating: f32,
    pub rating_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub source_memory_id: Option<String>,
    #[serde(default)]
    pub activation_condition: Option<ActivationCondition>,
    #[serde(default)]
    pub platform: Option<Vec<String>>,
    #[serde(default)]
    pub min_confidence: Option<f32>,
    /// T-S3-A-01: 信任级别（0=未验证导入，1=用户确认，2=社区信任，3=官方认证）
    #[serde(default)]
    pub trust_level: u8,
    /// T-S3-A-01: 声明式权限（如 "file:read", "network:http" 等）
    #[serde(default)]
    pub permissions: Vec<String>,
    /// T-S3-A-01: 能力集（与沙箱 CapabilitySet 对应）
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

/// The output of running a skill. `execution_time_ms` is wall-clock
/// time on the local machine; `tokens_used` is only populated for
/// LLM-driven skills.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillResult {
    pub skill_id: String,
    pub output: String,
    pub execution_time_ms: u64,
    pub tokens_used: u32,
}

// ---------------------------------------------------------------------------
// Request / response envelopes (DTOs that flow over the Tauri boundary).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateSkillRequest {
    pub name: String,
    pub description: String,
    pub code: String,
    pub language: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source_memory_id: Option<String>,
    #[serde(default)]
    pub activation_condition: Option<ActivationCondition>,
    #[serde(default)]
    pub platform: Option<Vec<String>>,
    #[serde(default)]
    pub min_confidence: Option<f32>,
    /// T-S3-A-01: 信任级别
    #[serde(default)]
    pub trust_level: u8,
    /// T-S3-A-01: 声明式权限
    #[serde(default)]
    pub permissions: Vec<String>,
    /// T-S3-A-01: 能力集
    #[serde(default)]
    pub capabilities: CapabilitySet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseSkillRequest {
    pub id: String,
    #[serde(default)]
    pub params: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateSkillRequest {
    pub id: String,
    pub rating: f32,
}

/// T-E-S-37: 多 tag 匹配模式。
///
/// * `Any` — OR 语义,任一 tag 命中即返回(默认值,向后兼容)。
/// * `All` — AND 语义,所有 tag 都必须命中。
///
/// 序列化为 lowercase 字符串("any" / "all"),与前端 `tag_match?: 'any' | 'all'` 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TagMatch {
    #[default]
    Any,
    All,
}

/// T-E-S-37: tag + 频次聚合行,供 `skill_tags` 命令返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListSkillsRequest {
    #[serde(default)]
    pub language: Option<String>,
    /// 旧字段(单 tag,向后兼容)。当 `tags` 非空时忽略此字段。
    #[serde(default)]
    pub tag: Option<String>,
    /// T-E-S-37: 新字段(多 tag)。与 `tag` 字段互斥语义:若非空,则以多 tag 逻辑
    /// (按 `tag_match` 模式 OR / AND)替换单 tag 过滤。
    #[serde(default)]
    pub tags: Vec<String>,
    /// T-E-S-37: 多 tag 匹配模式(默认 `Any` = OR)。
    #[serde(default)]
    pub tag_match: TagMatch,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_serializes_with_snake_case_keys() {
        let s = Skill {
            id: "x".to_string(),
            name: "n".to_string(),
            description: "d".to_string(),
            code: "c".to_string(),
            language: "rust".to_string(),
            tags: vec!["a".to_string()],
            usage_count: 1,
            avg_rating: 0.5,
            rating_count: 1,
            created_at: 1,
            updated_at: 1,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: CapabilitySet::new(),
        };
        let j = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(j.contains("\"avg_rating\":0.5"));
        assert!(j.contains("\"usage_count\":1"));
    }

    /// T-E-S-37: TagMatch 序列化为 lowercase,默认值为 `Any`。
    #[test]
    fn tag_match_serializes_lowercase_and_defaults_to_any() {
        assert_eq!(
            serde_json::to_string(&TagMatch::Any).expect("serialize should succeed"),
            "\"any\""
        );
        assert_eq!(
            serde_json::to_string(&TagMatch::All).expect("serialize should succeed"),
            "\"all\""
        );
        // 默认 = Any
        assert_eq!(TagMatch::default(), TagMatch::Any);
        // 反序列化大小写敏感:lowercase 输入应能还原。
        let m: TagMatch = serde_json::from_str("\"all\"").expect("parse should succeed");
        assert_eq!(m, TagMatch::All);
    }

    /// T-E-S-37: ListSkillsRequest 新字段默认值与向后兼容。
    ///
    /// 旧前端只发 `{ language, tag, limit }` 时,新字段应反序列化为
    /// `tags = []`,`tag_match = Any`(默认值),不报错。
    #[test]
    fn list_skills_request_backwards_compatible_with_old_payload() {
        let req: ListSkillsRequest =
            serde_json::from_str(r#"{"language":"rust","tag":"math","limit":10}"#)
                .expect("parse should succeed");
        assert_eq!(req.language.as_deref(), Some("rust"));
        assert_eq!(req.tag.as_deref(), Some("math"));
        assert_eq!(req.limit, 10);
        assert!(req.tags.is_empty(), "tags should default to empty vec");
        assert_eq!(
            req.tag_match,
            TagMatch::Any,
            "tag_match should default to Any"
        );
    }

    /// T-E-S-37: 多 tag payload 能正确反序列化。
    #[test]
    fn list_skills_request_parses_multi_tags_and_match_mode() {
        let req: ListSkillsRequest =
            serde_json::from_str(r#"{"tags":["rust","math"],"tag_match":"all","limit":5}"#)
                .expect("test op should succeed");
        assert_eq!(req.tags, vec!["rust".to_string(), "math".to_string()]);
        assert_eq!(req.tag_match, TagMatch::All);
        assert_eq!(req.limit, 5);
        assert!(req.tag.is_none());
        assert!(req.language.is_none());
    }
}
