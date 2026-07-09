//! Skill importer — v1.2 P2 eco compatibility
//!
//! Imports skills from external ecosystems:
//!
//! * **agentskills.io** — The open skill registry. Skills are distributed as
//!   Markdown files following the agentskills.io SKILL.md schema.  This
//!   importer fetches the raw Markdown, parses the YAML front-matter, and
//!   converts it into a nebula [`Skill`].
//!
//! * **ClawHub** — Clawd's community skill hub.  Skills have a `clawhub`
//!   slug that resolves to a GitHub repository; the importer fetches the
//!   `SKILL.md` from the default branch.
//!
//! * **TeamSkillsHub** — Internal team skill registry.  Assets are
//!   downloaded by `asset_id` from the team skills API.
//!
//! ## Safety
//!
//! Imported skills are sandboxed with `trust_level = 0` (user must manually
//! promote them) to prevent supply-chain attacks through third-party skill
//! registries.

use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use super::store::SkillStore;
use super::types::{CreateSkillRequest, Skill};
use crate::security::SsrfGuard;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// External skill source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillSource {
    /// agentskills.io compatible URL (raw SKILL.md).
    AgentskillsIo,
    /// ClawHub slug (e.g. `clawd/text-summarizer`).
    ClawHub,
    /// TeamSkillsHub asset ID.
    TeamSkillsHub,
}

/// Result of an import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Whether the import succeeded.
    pub success: bool,
    /// The imported skill (if successful).
    pub skill: Option<Skill>,
    /// The source URL or identifier.
    pub source: String,
    /// Error message (if failed).
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Importer
// ---------------------------------------------------------------------------

pub struct SkillImporter {
    store: SkillStore,
    client: Client,
    /// T-D-B-10: 可选的 TeamSkillsHub 客户端。
    ///
    /// 注入后,`import_from_teamskillshub` 会通过它拉取 SKILL.md 并解析入库;
    /// 未注入时保持旧行为(返回提示性错误,建议改用 `TeamSkillsHubImporter`)。
    hub_client: Option<super::hub_client::TeamSkillsHubClient>,
}

impl SkillImporter {
    pub fn new(store: SkillStore) -> Self {
        Self {
            store,
            client: Client::builder()
                .user_agent("nebula/1.2 skill-importer")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            hub_client: None,
        }
    }

    /// T-D-B-10: builder 方法,注入可选的 [`TeamSkillsHubClient`]。
    ///
    /// 注入后,`import_from_teamskillshub` 将通过该客户端从 TeamSkillsHub
    /// REST API 拉取技能详情(`GET /api/skills/{asset_id}`),把 `code`
    /// 字段作为 SKILL.md 解析并写入 store —— 不再返回 stub 错误。
    ///
    /// 未注入时(默认),`import_from_teamskillshub` 保持旧行为:返回提示性
    /// 错误,引导调用方使用 [`TeamSkillsHubImporter::import`]。
    pub fn with_hub_client(mut self, hub: Option<super::hub_client::TeamSkillsHubClient>) -> Self {
        self.hub_client = hub;
        self
    }

    /// Import a skill from a raw agentskills.io-compatible URL.
    ///
    /// The URL must point to a raw Markdown file following the
    /// agentskills.io SKILL.md schema (YAML front-matter + body).
    pub async fn import_from_url(&self, url: &str) -> ImportResult {
        let source = url.to_string();
        debug!(target: "nebula.importer", url, "fetching skill");

        let content = match self.fetch_skill_md(url).await {
            Ok(c) => c,
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("fetch failed: {e}")),
                }
            }
        };

        let parsed = match self.parse_skill_md(&content) {
            Ok(s) => s,
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("parse failed: {e}")),
                }
            }
        };

        match self.store_skill(parsed).await {
            Ok(skill) => {
                info!(
                    target: "nebula.importer",
                    id = %skill.id,
                    name = %skill.name,
                    "skill imported successfully"
                );
                ImportResult {
                    success: true,
                    skill: Some(skill),
                    source,
                    error: None,
                }
            }
            Err(e) => ImportResult {
                success: false,
                skill: None,
                source,
                error: Some(format!("store failed: {e}")),
            },
        }
    }

    /// Import a skill from ClawHub by slug (e.g. `clawd/text-summarizer`).
    ///
    /// Resolves the slug to `https://raw.githubusercontent.com/{org}/clawhub-skills/main/{slug}/SKILL.md`.
    pub async fn import_from_clawhub(&self, slug: &str) -> ImportResult {
        // ClawHub slugs are typically `org/skill-name`.
        // The raw URL pattern is: raw.githubusercontent.com/{org}/clawhub-skills/main/{skill}/SKILL.md
        let url = if slug.contains('/') {
            let parts: Vec<&str> = slug.splitn(2, '/').collect();
            format!(
                "https://raw.githubusercontent.com/{}/{}/main/SKILL.md",
                parts[0],
                if parts[0].eq_ignore_ascii_case("clawhub-skills") {
                    parts[1].to_string()
                } else {
                    format!("clawhub-skills/main/{}", parts[1])
                }
            )
        } else {
            format!(
                "https://raw.githubusercontent.com/clawhub-skills/main/{}/SKILL.md",
                slug
            )
        };

        self.import_from_url(&url).await
    }

    /// Import a skill from TeamSkillsHub by asset ID.
    ///
    /// The asset is fetched from the team skills API and parsed into
    /// a nebula skill.
    ///
    /// T-D-B-10: 若通过 [`with_hub_client`] 注入了 [`TeamSkillsHubClient`],
    /// 本方法将直接通过该客户端拉取 `GET /api/skills/{asset_id}`,把 `code`
    /// 字段作为 SKILL.md 解析后写入 store —— 与
    /// [`TeamSkillsHubImporter::import`] 行为一致(但不写 LanceDB,仅 SQLite)。
    /// 未注入客户端时,返回提示性错误,引导调用方使用
    /// [`TeamSkillsHubImporter::import`]。
    pub async fn import_from_teamskillshub(&self, asset_id: &str) -> ImportResult {
        let source = format!("teamskillshub:{asset_id}");

        // T-D-B-10: 未注入 hub client —— 保持旧行为(stub 错误)。
        let hub = match self.hub_client.as_ref() {
            Some(h) => h,
            None => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(
                        "use TeamSkillsHubImporter::import() instead — \
                         SkillImporter has no TeamSkillsHubClient \
                         (call with_hub_client(Some(client)) to enable)"
                            .to_string(),
                    ),
                };
            }
        };

        // 1) 通过 hub client 拉取技能详情。
        let detail = match hub.get_skill(asset_id).await {
            Ok(d) => d,
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("hub fetch failed: {e}")),
                };
            }
        };

        // 2) 把 detail.code 作为 SKILL.md 解析为 CreateSkillRequest。
        let req = match Self::parse_skill_md_inner(&detail.code) {
            Ok(r) => r,
            Err(e) => {
                return ImportResult {
                    success: false,
                    skill: None,
                    source,
                    error: Some(format!("parse failed: {e}")),
                };
            }
        };

        // 3) 构造 Skill 并写入 SQLite(复用 store_skill)。
        match self.store_skill(req).await {
            Ok(skill) => {
                info!(
                    target: "nebula.importer",
                    id = %skill.id,
                    name = %skill.name,
                    source = %source,
                    "skill imported from TeamSkillsHub via with_hub_client"
                );
                ImportResult {
                    success: true,
                    skill: Some(skill),
                    source,
                    error: None,
                }
            }
            Err(e) => ImportResult {
                success: false,
                skill: None,
                source,
                error: Some(format!("store failed: {e}")),
            },
        }
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    async fn fetch_skill_md(&self, url: &str) -> Result<String> {
        // M7b #94: SSRF 校验 — skill URL 是用户可控的,需校验目标地址。
        SsrfGuard::new()
            .validate_url(url)
            .map_err(|e| anyhow!("SSRF validation failed for skill URL: {e}"))?;
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("HTTP request failed")?;

        if !resp.status().is_success() {
            bail!("HTTP {}", resp.status());
        }

        resp.text().await.context("reading response body")
    }

    /// T-S3-A-02: 公共关联函数,解析 SKILL.md 内容为 CreateSkillRequest。
    /// 供 TeamSkillsHubImporter 和其他外部调用者使用。
    pub fn from_skill_md(content: &str) -> Result<CreateSkillRequest> {
        Self::parse_skill_md_inner(content)
    }

    /// Parse an agentskills.io-compatible SKILL.md into a CreateSkillRequest.
    fn parse_skill_md(&self, content: &str) -> Result<CreateSkillRequest> {
        Self::parse_skill_md_inner(content)
    }

    fn parse_skill_md_inner(content: &str) -> Result<CreateSkillRequest> {
        // Parse YAML front-matter.
        let front_matter = Self::extract_yaml_front_matter(content)?;

        let name = front_matter
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .context("'name' field required in front-matter")?;

        let description = front_matter
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let category = front_matter
            .get("category")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "imported".to_string());

        let tags: Vec<String> = front_matter
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // T-S3-A-01: 解析 trust_level（默认 0=未验证）
        let trust_level = front_matter
            .get("trust_level")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;

        // T-S3-A-01: 解析 permissions（如 ["file:read", "network:http"]）
        let permissions: Vec<String> = front_matter
            .get("permissions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // T-S3-A-01: 解析 capabilities 并构造 CapabilitySet
        let capabilities = {
            use super::sandbox::{Capability, CapabilitySet};
            let mut caps = CapabilitySet::new();
            if let Some(arr) = front_matter.get("capabilities").and_then(|v| v.as_array()) {
                for cap in arr {
                    if let Some(s) = cap.as_str() {
                        match s {
                            "file:read" | "FileRead" => caps.grant(Capability::FileRead),
                            "file:write" | "FileWrite" => caps.grant(Capability::FileWrite),
                            "network" | "Network" => caps.grant(Capability::Network),
                            "subprocess" | "Subprocess" => caps.grant(Capability::Subprocess),
                            "env:read" | "EnvRead" => caps.grant(Capability::EnvRead),
                            "clipboard:read" | "ClipboardRead" => {
                                caps.grant(Capability::ClipboardRead)
                            }
                            "llm:call" | "LlmCall" => caps.grant(Capability::LlmCall),
                            "db:access" | "DbAccess" => caps.grant(Capability::DbAccess),
                            _ => {}
                        }
                    }
                }
            }
            caps
        };

        // Extract instructions from the body (everything after front-matter).
        let instructions = Self::extract_body(content);

        Ok(CreateSkillRequest {
            name,
            description,
            language: category,
            code: instructions,
            tags,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level,
            permissions,
            capabilities,
        })
    }

    fn extract_yaml_front_matter(content: &str) -> Result<serde_json::Value> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            bail!("no YAML front-matter found (expected leading '---')");
        }

        let rest = &trimmed[3..];
        let end = rest
            .find("---")
            .context("unclosed front-matter (missing closing '---')")?;

        let yaml_str = &rest[..end];

        let yaml_value: serde_yaml::Value =
            serde_yaml::from_str(yaml_str).with_context(|| "YAML front-matter parse error")?;

        let json_str =
            serde_json::to_string(&yaml_value).with_context(|| "YAML-to-JSON conversion error")?;

        let json_value: serde_json::Value =
            serde_json::from_str(&json_str).with_context(|| "JSON round-trip error")?;

        Ok(json_value)
    }

    fn extract_body(content: &str) -> String {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return content.to_string();
        }

        let rest = &trimmed[3..];
        if let Some(end) = rest.find("---") {
            rest[end + 3..].trim().to_string()
        } else {
            content.to_string()
        }
    }

    async fn store_skill(&self, req: CreateSkillRequest) -> Result<Skill> {
        let now = chrono::Utc::now().timestamp_millis();
        let id = format!("import-{}-{}", req.name, now);
        let skill = Skill {
            id: id.clone(),
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
            source_memory_id: req.source_memory_id.clone(),
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: req.trust_level,
            permissions: req.permissions.clone(),
            capabilities: req.capabilities.clone(),
        };
        self.store.insert(&skill)?;

        // Fetch back the stored skill.
        self.store
            .get(&id)?
            .ok_or_else(|| anyhow::anyhow!("skill not found after insert"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yaml_front_matter() {
        let _importer = SkillImporter {
            store: SkillStore::new(
                crate::memory::sqlite_store::SqliteStore::open(":memory:")
                    .expect("create should succeed"),
            )
            .expect("test op should succeed"),
            client: Client::new(),
            hub_client: None,
        };

        let md = r#"---
name: text-summarizer
description: Summarize long texts into concise bullet points.
category: text
tags: [summarization, nlp, utility]
---

# Text Summarizer

## Instructions
1. Read the input text
2. Identify key points
3. Output a concise summary
"#;

        let result = SkillImporter::from_skill_md(md).expect("test op should succeed");
        assert_eq!(result.name, "text-summarizer");
        assert_eq!(result.language, "text");
        assert_eq!(result.tags, vec!["summarization", "nlp", "utility"]);
        assert!(result.code.contains("Read the input text"));
    }

    #[test]
    fn test_extract_body() {
        let body = SkillImporter::extract_body("---\nname: test\n---\n\n# Title\n\nContent here");
        assert_eq!(body, "# Title\n\nContent here");
    }

    // -----------------------------------------------------------------
    // T-D-B-10: import_from_teamskillshub / with_hub_client 测试
    // -----------------------------------------------------------------

    fn make_importer() -> SkillImporter {
        let store = SkillStore::new(
            crate::memory::sqlite_store::SqliteStore::open(":memory:")
                .expect("create should succeed"),
        )
        .expect("SkillStore::new should succeed");
        SkillImporter::new(store)
    }

    /// T-D-B-10: 未注入 hub client 时,import_from_teamskillshub 应返回
    /// 提示性错误(而非 panic 或网络调用)。
    #[tokio::test]
    async fn test_import_from_teamskillshub_without_client_returns_error() {
        let importer = make_importer();
        let result = importer.import_from_teamskillshub("asset-123").await;
        assert!(!result.success);
        assert!(result.skill.is_none());
        assert_eq!(result.source, "teamskillshub:asset-123");
        let err = result.error.expect("error should be set");
        assert!(
            err.contains("TeamSkillsHubImporter") || err.contains("with_hub_client"),
            "error should guide caller to use TeamSkillsHubImporter or with_hub_client, got: {err}"
        );
    }

    /// T-D-B-10: with_hub_client(Some(client)) 应注入客户端,使后续
    /// import_from_teamskillshub 不再返回 stub 错误(而是尝试网络调用)。
    /// 本测试仅验证注入语义(不实际发网络请求),用 TEST-NET-3 地址避免 DNS。
    #[tokio::test]
    async fn test_with_hub_client_injects_client() {
        let importer = make_importer();
        // 注入一个指向 TEST-NET-3(RFC 5737)地址的 client —— 不会真正
        // 发起成功的请求,但验证注入后不再返回 "no TeamSkillsHubClient" 错误。
        let hub = super::super::hub_client::TeamSkillsHubClient::new("https://203.0.113.1");
        let importer = importer.with_hub_client(Some(hub));

        let result = importer.import_from_teamskillshub("asset-456").await;
        // 应失败(网络不可达),但错误信息应是 "hub fetch failed" 而非
        // "use TeamSkillsHubImporter::import() instead"。
        assert!(!result.success);
        let err = result.error.expect("error should be set");
        assert!(
            !err.contains("TeamSkillsHubImporter::import"),
            "with_hub_client injected; should not return stub error, got: {err}"
        );
        assert!(
            err.contains("hub fetch failed"),
            "expected hub fetch failed error, got: {err}"
        );
    }

    /// T-D-B-10: with_hub_client(None) 应等价于未注入(返回 stub 错误)。
    #[tokio::test]
    async fn test_with_hub_client_none_equals_default() {
        let importer = make_importer().with_hub_client(None);
        let result = importer.import_from_teamskillshub("asset-789").await;
        assert!(!result.success);
        let err = result.error.expect("error should be set");
        assert!(
            err.contains("TeamSkillsHubImporter") || err.contains("with_hub_client"),
            "with_hub_client(None) should return stub error, got: {err}"
        );
    }
}
