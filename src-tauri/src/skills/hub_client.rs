use std::sync::Arc;

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::memory::embedder::Embedder;
// T-E-S-42: TeamSkillsHubImporter 面向 VectorStore trait 编程,可接受任意后端。
use crate::memory::vector_store::VectorStore;
use crate::security::SsrfGuard;

use super::importer::{ImportResult, SkillImporter};
use super::store::SkillStore;
use super::types::Skill;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub rating: f32,
    pub downloads: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSkillDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub code: String,
    pub language: String,
    pub author: String,
    pub tags: Vec<String>,
}

pub struct TeamSkillsHubClient {
    client: Client,
    base_url: String,
}

impl TeamSkillsHubClient {
    pub fn new(base_url: &str) -> Self {
        let guard = SsrfGuard::new();
        guard
            .validate_url(base_url)
            .expect("TeamSkillsHub URL failed SSRF validation");
        // T-S2-A-02: 使用 SSRF 安全客户端，重定向链每跳验证
        let client = guard
            .build_safe_client()
            .expect("failed to build SSRF-safe client for TeamSkillsHub");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<HubSkillSummary>> {
        let url = format!(
            "{}/api/skills/search?q={}&limit={}",
            self.base_url, query, limit
        );
        let guard = SsrfGuard::new();
        guard.validate_url(&url)?;
        info!(target: "nebula.hub", query = %query, "searching TeamSkillsHub");
        let resp = self.client.get(&url).send().await?;
        let skills: Vec<HubSkillSummary> = resp.json().await?;
        Ok(skills)
    }

    pub async fn get_skill(&self, skill_id: &str) -> Result<HubSkillDetail> {
        let url = format!("{}/api/skills/{}", self.base_url, skill_id);
        let guard = SsrfGuard::new();
        guard.validate_url(&url)?;
        let resp = self.client.get(&url).send().await?;
        let detail: HubSkillDetail = resp.json().await?;
        Ok(detail)
    }

    /// T-S3-A-02: 列出 hub 上所有技能（供批量导入使用）。
    pub async fn list_skills(&self, limit: u32) -> Result<Vec<HubSkillSummary>> {
        let url = format!("{}/api/skills?limit={}", self.base_url, limit);
        let guard = SsrfGuard::new();
        guard.validate_url(&url)?;
        let resp = self.client.get(&url).send().await?;
        let skills: Vec<HubSkillSummary> = resp.json().await?;
        Ok(skills)
    }
}

// ---------------------------------------------------------------------------
// T-S3-A-02: TeamSkillsHubImporter — 从团队 hub 批量导入 SKILL.md
// ---------------------------------------------------------------------------

/// 团队技能 hub 导入器。
///
/// 从 TeamSkillsHub REST API 拉取 SKILL.md 内容,调用
/// [`SkillImporter::from_skill_md()`] 解析,写入 SQLite ([`SkillStore`])
/// 和 LanceDB ([`LanceStore`],可选)。
///
/// ## 安全
///
/// 所有导入的技能默认 `trust_level = 0`(除非 SKILL.md front-matter
/// 显式声明更高等级),需要用户手动提升信任等级后才能执行。
pub struct TeamSkillsHubImporter {
    client: TeamSkillsHubClient,
    store: SkillStore,
    /// 可选的向量存储:存在时将技能描述嵌入 LanceDB 供语义检索。
    lance: Option<Arc<dyn VectorStore>>,
    /// 可选的嵌入器:与 `lance` 配合使用。
    embedder: Option<Arc<Embedder>>,
}

impl TeamSkillsHubImporter {
    /// 创建一个新的 hub 导入器。
    ///
    /// `base_url` 是 TeamSkillsHub API 的根地址(如 `https://skills.example.com`)。
    pub fn new(base_url: &str, store: SkillStore) -> Self {
        let client = TeamSkillsHubClient::new(base_url);
        Self {
            client,
            store,
            lance: None,
            embedder: None,
        }
    }

    /// 附加 LanceDB + Embedder,使导入的技能可被语义检索。
    pub fn with_vector_store(
        mut self,
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
    ) -> Self {
        self.lance = Some(lance);
        self.embedder = Some(embedder);
        self
    }

    /// 导入单个技能。
    ///
    /// 1. 通过 `GET /api/skills/{asset_id}` 获取 `HubSkillDetail`
    /// 2. 将 `code` 字段作为 SKILL.md 解析
    /// 3. 写入 SQLite
    /// 4. (可选) 嵌入描述并写入 LanceDB
    pub async fn import(&self, asset_id: &str) -> ImportResult {
        let source = format!("teamskillshub:{asset_id}");

        // 1) 从 hub API 获取技能详情
        let detail = match self.client.get_skill(asset_id).await {
            Ok(d) => d,
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("hub fetch failed: {e}")),
                }
            }
        };

        // 2) 解析 SKILL.md (code 字段即 SKILL.md 原文)
        let req = match SkillImporter::from_skill_md(&detail.code) {
            Ok(r) => r,
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("parse failed: {e}")),
                }
            }
        };

        // 3) 构造 Skill 并写入 SQLite
        let now = chrono::Utc::now().timestamp_millis();
        let skill = Skill {
            id: format!("hub-{}-{}", detail.id, now),
            name: req.name.clone(),
            description: req.description.clone(),
            code: req.code.clone(),
            language: req.language.clone(),
            tags: req.tags.clone(),
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: now,
            updated_at: now,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: req.trust_level,
            permissions: req.permissions.clone(),
            capabilities: req.capabilities.clone(),
        };

        if let Err(e) = self.store.insert(&skill) {
            return ImportResult {
                success: false,
                skill: None,
                source,
                error: Some(format!("sqlite insert failed: {e}")),
            };
        }

        // 回读确认
        let stored = match self.store.get(&skill.id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some("skill not found after insert".to_string()),
                }
            }
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("sqlite readback failed: {e}")),
                }
            }
        };

        // 4) (可选) 嵌入并写入 LanceDB
        if let (Some(lance), Some(embedder)) = (&self.lance, &self.embedder) {
            let embed_text = format!("{}\n{}", stored.name, stored.description);
            match embedder.embed(&embed_text).await {
                Ok(vector) => {
                    if let Err(e) = lance.upsert(&stored.id, &vector).await {
                        warn!(
                            target: "nebula.hub",
                            error = %e,
                            skill_id = %stored.id,
                            "LanceDB upsert failed (non-fatal)"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        target: "nebula.hub",
                        error = %e,
                        skill_id = %stored.id,
                        "embedding failed (non-fatal)"
                    );
                }
            }
        }

        info!(
            target: "nebula.hub",
            id = %stored.id,
            name = %stored.name,
            "skill imported from TeamSkillsHub"
        );

        ImportResult {
            success: true,
            skill: Some(stored),
            source,
            error: None,
        }
    }

    /// 批量导入:搜索 hub 并导入所有匹配的技能。
    ///
    /// 返回每个技能的导入结果。
    pub async fn import_batch(&self, query: &str, limit: u32) -> Vec<ImportResult> {
        let summaries = match self.client.search(query, limit).await {
            Ok(s) => s,
            Err(e) => {
                return vec![ImportResult {
                    success: false,
                    skill: None,
                    source: format!("teamskillshub:search:{query}"),
                    error: Some(format!("hub search failed: {e}")),
                }];
            }
        };

        let mut results = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let result = self.import(&summary.id).await;
            results.push(result);
        }
        results
    }

    /// 导入 hub 上所有技能(最多 `limit` 个)。
    pub async fn import_all(&self, limit: u32) -> Vec<ImportResult> {
        let summaries = match self.client.list_skills(limit).await {
            Ok(s) => s,
            Err(e) => {
                return vec![ImportResult {
                    success: false,
                    skill: None,
                    source: "teamskillshub:all".to_string(),
                    error: Some(format!("hub list failed: {e}")),
                }];
            }
        };

        let mut results = Vec::with_capacity(summaries.len());
        for summary in summaries {
            let result = self.import(&summary.id).await;
            results.push(result);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constructs_with_valid_url() {
        // Use a TEST-NET-3 address (RFC 5737) to avoid DNS
        // resolution in CI environments.  The address is public
        // (not loopback / private / link-local) so it passes
        // SSRF validation.
        let _client = TeamSkillsHubClient::new("https://203.0.113.1");
    }

    #[test]
    fn importer_constructs_with_store() {
        let sqlite = crate::memory::sqlite_store::SqliteStore::open(":memory:").unwrap();
        let store = SkillStore::new(sqlite).unwrap();
        let _importer = TeamSkillsHubImporter::new("https://203.0.113.1", store);
    }

    #[test]
    fn from_skill_md_parses_hub_format() {
        // 验证 from_skill_md 能正确解析 hub 返回的 SKILL.md 格式
        let md = r#"---
name: code-reviewer
description: Automated code review skill.
category: code
tags: [review, quality]
trust_level: 1
permissions: ["file:read"]
capabilities: ["file:read", "llm:call"]
---

# Code Reviewer

## Instructions
1. Read the code
2. Identify issues
3. Suggest improvements
"#;
        let result = SkillImporter::from_skill_md(md).unwrap();
        assert_eq!(result.name, "code-reviewer");
        assert_eq!(result.language, "code");
        assert_eq!(result.trust_level, 1);
        assert_eq!(result.permissions, vec!["file:read"]);
        assert!(!result.capabilities.is_empty());
        assert!(result.code.contains("Read the code"));
    }
}
