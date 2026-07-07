//! T-E-S-36 能力层 — Capability + CapabilityRegistry。
//!
//! 声明式能力发现:每个 [`Capability`] 描述一类能力(id/name/description)
//! 及其反向映射到 skills 列表(`skills: Vec<String>`)。调用方通过
//! [`CapabilityRegistry::match_by_intent`] 或
//! [`CapabilityRegistry::match_by_input`] 找到合适的能力,再从能力的
//! `skills` 列表选择具体 skill 执行。
//!
//! ## 反向映射
//!
//! `Capability.skills` 是 **Capability → Skills** 的反向索引,预留供
//! T-E-S-37(tags 子系统)使用。例如 `Capability { id: "file:read",
//! skills: ["read-file", "grep-file"] }` 表示 `read-file` 与
//! `grep-file` 两个 skill 都提供 `file:read` 能力。
//!
//! ## 匹配算法
//!
//! * `match_by_intent` — 关键词匹配:在 capability 的 name / description
//!   / skills 中查找关键词(大小写不敏感)。
//! * `match_by_input` — schema 兼容性检查:input 是 JSON object 时,
//!   优先按 `skill` / `capability` 字段精确匹配;否则松散兼容返回全部。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 能力描述:声明一个能力及其关联的 skills 列表(反向映射)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Capability {
    /// 能力 id(如 `"file:read"` / `"network"` / `"llm:call"`)。
    pub id: String,
    /// 人类可读名称(如 `"File Read"`)。
    pub name: String,
    /// 能力描述。
    pub description: String,
    /// 反向映射:支持此能力的 skill 名称列表。
    pub skills: Vec<String>,
}

/// 能力注册中心。
///
/// 线程安全由调用方保证(`SkillEngine` 用 `RwLock` 包装)。本结构自身
/// 不是 `Sync`,但通过 `&self` 的只读方法可安全共享。
#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    caps: HashMap<String, Capability>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个能力。若 `id` 已存在则覆盖。
    pub fn register(&mut self, cap: Capability) {
        self.caps.insert(cap.id.clone(), cap);
    }

    /// 按 id 查找能力。
    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.caps.get(id)
    }

    /// 列出所有已注册能力。
    pub fn list_all(&self) -> Vec<&Capability> {
        self.caps.values().collect()
    }

    /// 关键词匹配:将 intent 按空白分割为多个关键词,在 name /
    /// description / skills 中查找任意关键词(大小写不敏感)。
    /// 任一关键词命中即返回该能力。空 intent 返回全部能力(向后兼容)。
    pub fn match_by_intent(&self, intent: &str) -> Vec<&Capability> {
        let trimmed = intent.trim();
        // 空 intent:向后兼容,返回全部能力(原 impl 的行为)。
        if trimmed.is_empty() {
            return self.caps.values().collect();
        }
        let needles: Vec<String> = trimmed
            .split_whitespace()
            .map(|w| w.to_ascii_lowercase())
            .collect();
        self.caps
            .values()
            .filter(|c| {
                let name = c.name.to_ascii_lowercase();
                let desc = c.description.to_ascii_lowercase();
                let skill_lowers: Vec<String> =
                    c.skills.iter().map(|s| s.to_ascii_lowercase()).collect();
                needles.iter().any(|needle| {
                    name.contains(needle)
                        || desc.contains(needle)
                        || skill_lowers.iter().any(|s| s.contains(needle))
                })
            })
            .collect()
    }

    /// input schema 兼容性检查(简化版)。
    ///
    /// * input 是 object 且有 `"skill"` 字段(string):返回 `skills`
    ///   列表包含该 skill 的能力。
    /// * input 是 object 且有 `"capability"` 字段(string):返回该 id
    ///   对应的能力。
    /// * input 是 object 但无上述字段:松散兼容,返回全部能力。
    /// * input 不是 object:不兼容,返回空 vec。
    pub fn match_by_input(&self, input: &serde_json::Value) -> Vec<&Capability> {
        let Some(obj) = input.as_object() else {
            return Vec::new();
        };
        if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
            return self
                .caps
                .values()
                .filter(|c| c.skills.iter().any(|s| s == skill))
                .collect();
        }
        if let Some(cap_id) = obj.get("capability").and_then(|v| v.as_str()) {
            return self.caps.values().filter(|c| c.id == cap_id).collect();
        }
        // 松散兼容:input 是 object 但无明确字段,返回全部。
        self.caps.values().collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_registry() -> CapabilityRegistry {
        let mut reg = CapabilityRegistry::new();
        reg.register(Capability {
            id: "file:read".to_string(),
            name: "File Read".to_string(),
            description: "Read file contents from disk".to_string(),
            skills: vec!["read-file".to_string(), "grep-file".to_string()],
        });
        reg.register(Capability {
            id: "network".to_string(),
            name: "Network Access".to_string(),
            description: "Make HTTP requests to remote URLs".to_string(),
            skills: vec!["fetch-url".to_string()],
        });
        reg.register(Capability {
            id: "llm:call".to_string(),
            name: "LLM Call".to_string(),
            description: "Invoke a language model".to_string(),
            skills: vec!["summarize".to_string(), "translate".to_string()],
        });
        reg
    }

    #[test]
    fn test_register_and_match_by_intent() {
        // 注册后,match_by_intent 应按关键词命中 name / description / skills。
        let reg = sample_registry();

        // 命中 name("File Read" 含 "file")。
        let hits = reg.match_by_intent("file");
        assert_eq!(hits.len(), 1, "intent 'file' should match File Read");
        assert_eq!(hits[0].id, "file:read");

        // 命中 description("HTTP requests" 含 "http")。
        let hits = reg.match_by_intent("http");
        assert_eq!(hits.len(), 1, "intent 'http' should match Network Access");
        assert_eq!(hits[0].id, "network");

        // 命中 skills 列表("summarize")。
        let hits = reg.match_by_intent("summarize");
        assert_eq!(hits.len(), 1, "intent 'summarize' should match LLM Call");
        assert_eq!(hits[0].id, "llm:call");

        // 大小写不敏感。
        let hits = reg.match_by_intent("FILE");
        assert_eq!(
            hits.len(),
            1,
            "intent 'FILE' should match case-insensitively"
        );

        // 无命中。
        assert!(reg.match_by_intent("nonexistent").is_empty());
    }

    #[test]
    fn test_match_by_input_schema_compatibility() {
        let reg = sample_registry();

        // input 有 "skill" 字段:返回 skills 列表包含该 skill 的能力。
        let input = serde_json::json!({"skill": "read-file", "content": "x"});
        let hits = reg.match_by_input(&input);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "file:read");

        // input 有 "capability" 字段:返回该 id 对应能力。
        let input = serde_json::json!({"capability": "network", "url": "x"});
        let hits = reg.match_by_input(&input);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "network");

        // input 是 object 但无明确字段:松散兼容返回全部。
        let input = serde_json::json!({"text": "hello"});
        let hits = reg.match_by_input(&input);
        assert_eq!(hits.len(), 3, "loose compat should return all capabilities");

        // input 不是 object:不兼容返回空。
        let hits = reg.match_by_input(&serde_json::json!("plain string"));
        assert!(hits.is_empty(), "non-object input should not match");
    }

    #[test]
    fn test_list_all() {
        let reg = sample_registry();
        let all = reg.list_all();
        assert_eq!(all.len(), 3, "should have 3 registered capabilities");
        // list_all 返回引用,顺序由 HashMap 决定,故只检查数量与 id 集合。
        let ids: std::collections::HashSet<&str> = all.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains("file:read"));
        assert!(ids.contains("network"));
        assert!(ids.contains("llm:call"));
    }

    #[test]
    fn test_capability_reverse_mapping_to_skills() {
        // 能力反向映射:Capability → Skills 索引正确。
        // 注册后,通过 get(id) 应能取到完整的 skills 列表。
        let reg = sample_registry();

        let file_read = reg.get("file:read").expect("file:read must be registered");
        assert_eq!(
            file_read.skills,
            vec!["read-file".to_string(), "grep-file".to_string()],
            "reverse mapping Capability -> Skills must be intact"
        );

        let llm = reg.get("llm:call").expect("llm:call must be registered");
        assert_eq!(
            llm.skills,
            vec!["summarize".to_string(), "translate".to_string()],
            "reverse mapping for llm:call must be intact"
        );

        // 反向查询:给定一个 skill 名,应能找到它支持的全部能力。
        let skill_name = "summarize";
        let supporting_caps: Vec<&str> = reg
            .list_all()
            .into_iter()
            .filter(|c| c.skills.iter().any(|s| s == skill_name))
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            supporting_caps,
            vec!["llm:call"],
            "skill 'summarize' should map back to capability 'llm:call'"
        );
    }
}
