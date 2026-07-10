//! v1.3 P2-7: 技能市场 — 搜索/安装/更新/发布基础设施。
//!
//! 与 `SkillStore`（CRUD 持久化）和 `SkillImporter`（外部导入）配合，
//! 提供：索引构建、全文搜索、一键安装、更新检查、发布协议。
//!
//! P2-5 扩展：远端版本比对 + 一键更新。`check_remote_updates()` 从远端
//! 拉取 SKILL.md frontmatter，解析 version 字段，与本地 version 做 semver
//! 比对；`update_skill()` 拉取最新 SKILL.md 并替换本地技能文件（保留
//! 用户的 trust_level 设置）。

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::skills::importer::{ImportResult, SkillImporter};
use crate::skills::store::SkillStore;

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 技能市场元数据 — 统一本地和远程技能信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub version: String,
    pub author: String,
    pub rating: f32,
    pub rating_count: u32,
    pub install_count: u32,
    pub icon_url: Option<String>,
    pub source: String,
    pub import_identifier: Option<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
    pub size_bytes: u64,
    pub updated_at: i64,
}

/// 搜索查询参数。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceQuery {
    pub text: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub available_only: bool,
    #[serde(default)]
    pub installed_only: bool,
    #[serde(default)]
    pub updates_only: bool,
    #[serde(default)]
    pub min_rating: f32,
    #[serde(default)]
    pub sort: SortBy,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Relevance,
    Name,
    Rating,
    Installs,
    UpdatedAt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub entry: SkillEntry,
    pub relevance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceResponse {
    pub results: Vec<SearchHit>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceStats {
    pub total_available: usize,
    pub total_installed: usize,
    pub updates_available: usize,
    pub by_source: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub skill_id: String,
    pub name: String,
    pub current_version: String,
    pub latest_version: String,
    pub source: String,
}

/// P2-5: 远端版本检查结果 — 描述单个技能的更新状态。
///
/// 由 [`SkillMarketplace::check_remote_updates`] 返回。前端根据
/// `update_available` 字段决定是否显示"更新"按钮。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUpdateInfo {
    /// 技能 ID。
    pub skill_id: String,
    /// 技能名称。
    pub skill_name: String,
    /// 当前本地版本号（semver）。
    pub current_version: String,
    /// 远端最新版本号（semver）。
    pub latest_version: String,
    /// 是否有更新可用（远端版本 > 本地版本）。
    pub update_available: bool,
    /// 技能的远端 source URL（若有）。
    pub source_url: Option<String>,
    /// 更新日志（可选，从远端 frontmatter 解析）。
    pub changelog: Option<String>,
}

// ---------------------------------------------------------------------------
// P2-5: semver 比对工具函数
// ---------------------------------------------------------------------------

/// P2-5: 解析 semver 字符串为 (major, minor, patch) 元组。
///
/// 接受 `1.2.3` / `0.1.0` / `2.0.0`；拒绝 `1.2` / `latest` / `v1.2.3`。
/// 不引入 semver crate（避免新依赖），仅做格式校验。
fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse::<u32>().ok()?;
    let minor = parts[1].parse::<u32>().ok()?;
    let patch = parts[2].parse::<u32>().ok()?;
    Some((major, minor, patch))
}

/// P2-5: 比较两个 semver 版本字符串。
///
/// 返回 `std::cmp::Ordering`：
/// - `Less` 表示 `a < b`（a 版本更低）
/// - `Equal` 表示 `a == b`（版本相同）
/// - `Greater` 表示 `a > b`（a 版本更高）
///
/// 无法解析的版本视为 `(0, 0, 0)`。
pub fn semver_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let av = parse_semver(a).unwrap_or((0, 0, 0));
    let bv = parse_semver(b).unwrap_or((0, 0, 0));
    av.cmp(&bv)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishManifest {
    pub manifest_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub tags: Vec<String>,
    pub source_url: Option<String>,
    pub dependencies: Vec<String>,
    pub min_nebula_version: Option<String>,
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for PublishManifest {
    fn default() -> Self {
        Self {
            manifest_version: "1.0".into(),
            id: String::new(),
            name: String::new(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            tags: vec![],
            source_url: None,
            dependencies: vec![],
            min_nebula_version: None,
            extra: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// 倒排索引
// ---------------------------------------------------------------------------

struct InvertedIndex {
    inverted: HashMap<String, Vec<(usize, f32)>>,
    entries: Vec<SkillEntry>,
    avg_doc_len: f32,
}

impl InvertedIndex {
    fn new() -> Self {
        Self {
            inverted: HashMap::new(),
            entries: Vec::new(),
            avg_doc_len: 0.0,
        }
    }

    fn build(&mut self, entries: Vec<SkillEntry>) {
        self.entries = entries;
        self.inverted.clear();
        let mut total_len = 0usize;
        for (idx, entry) in self.entries.iter().enumerate() {
            let tokens = tokenize(&entry.name, &entry.description, &entry.tags);
            total_len += tokens.len();
            let tf = term_frequencies(&tokens);
            for (tok, freq) in tf {
                self.inverted.entry(tok).or_default().push((idx, freq));
            }
        }
        self.avg_doc_len = if self.entries.is_empty() {
            0.0
        } else {
            total_len as f32 / self.entries.len() as f32
        };
    }

    fn search(&self, query: &str, top_k: usize) -> Vec<(usize, f32)> {
        let query_tokens = tokenize_simple(query);
        if query_tokens.is_empty() || self.entries.is_empty() {
            return Vec::new();
        }
        let n_docs = self.entries.len() as f32;
        let mut scores: Vec<f32> = vec![0.0; n_docs as usize];
        for qtok in &query_tokens {
            if let Some(postings) = self.inverted.get(qtok) {
                let idf = ((n_docs - postings.len() as f32 + 0.5) / (postings.len() as f32 + 0.5)
                    + 1.0)
                    .ln();
                for &(idx, tf) in postings {
                    scores[idx] += tf * idf;
                }
            }
        }
        let max_score = scores.iter().cloned().fold(0.0f32, f32::max);
        if max_score > 0.0 {
            for s in &mut scores {
                *s /= max_score;
            }
        }
        let mut ranked: Vec<(usize, f32)> = scores
            .into_iter()
            .enumerate()
            .filter(|(_, s)| *s > 0.0)
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_k);
        ranked
    }
}

fn tokenize(name: &str, description: &str, tags: &[String]) -> Vec<String> {
    let text = format!("{name} {description} {}", tags.join(" "));
    let mut tokens: Vec<String> = text
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '.' || c == ':')
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_'))
        .filter(|t| !t.is_empty() && t.len() >= 2)
        .map(|t| t.to_string())
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn tokenize_simple(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = text
        .to_lowercase()
        .split(char::is_whitespace)
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn term_frequencies(tokens: &[String]) -> Vec<(String, f32)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in tokens {
        *counts.entry(t.as_str()).or_default() += 1;
    }
    let total = tokens.len() as f32;
    counts
        .into_iter()
        .map(|(t, c)| (t.to_string(), c as f32 / total))
        .collect()
}

// ---------------------------------------------------------------------------
// SkillMarketplace
// ---------------------------------------------------------------------------

pub struct SkillMarketplace {
    store: Arc<SkillStore>,
    importer: Arc<SkillImporter>,
    index: RwLock<InvertedIndex>,
    entries: RwLock<Vec<SkillEntry>>,
    /// P2-5: HTTP 客户端,用于从远端拉取 SKILL.md frontmatter。
    client: reqwest::Client,
    /// P2-5: 技能 source URL 注册表（skill_id → source URL）。
    ///
    /// 当通过 `install()` 安装技能或通过 `register_source_url()` 注册时,
    /// 记录技能的远端 URL,供 `check_remote_updates()` 和 `update_skill()` 使用。
    source_urls: RwLock<HashMap<String, String>>,
}

impl SkillMarketplace {
    pub fn new(store: Arc<SkillStore>, importer: Arc<SkillImporter>) -> Self {
        Self {
            store,
            importer,
            index: RwLock::new(InvertedIndex::new()),
            entries: RwLock::new(Vec::new()),
            client: reqwest::Client::builder()
                .user_agent("nebula/1.3 skill-updater")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            source_urls: RwLock::new(HashMap::new()),
        }
    }

    /// P2-5: 注册技能的 source URL。
    ///
    /// 在从远端安装技能后调用,记录 URL 供后续更新检查使用。
    /// 同一 skill_id 重复注册会覆盖旧 URL。
    pub fn register_source_url(&self, skill_id: &str, url: &str) {
        self.source_urls
            .write()
            .insert(skill_id.to_string(), url.to_string());
    }

    /// P2-5: 获取技能的已注册 source URL。
    pub fn get_source_url(&self, skill_id: &str) -> Option<String> {
        self.source_urls.read().get(skill_id).cloned()
    }

    /// Build/refresh the index from the local SkillStore.
    pub fn refresh(&self) -> Result<MarketplaceStats, anyhow::Error> {
        let local_skills =
            self.store
                .list(None, None, &[], crate::skills::types::TagMatch::Any, 1000)?;
        let mut entries: Vec<SkillEntry> = Vec::new();
        let installed_ids: HashSet<String> = local_skills.iter().map(|s| s.id.clone()).collect();

        for s in &local_skills {
            entries.push(SkillEntry {
                id: s.id.clone(),
                name: s.name.clone(),
                description: s.description.clone(),
                tags: s.tags.clone(),
                version: "1.0.0".into(),
                author: "local".into(),
                rating: s.avg_rating,
                rating_count: s.rating_count,
                install_count: s.usage_count,
                icon_url: None,
                source: "local".into(),
                import_identifier: None,
                installed: true,
                installed_version: Some("1.0.0".into()),
                update_available: false,
                size_bytes: s.code.len() as u64,
                updated_at: s.updated_at,
            });
        }

        let mut index = InvertedIndex::new();
        index.build(entries.clone());
        *self.index.write() = index;
        *self.entries.write() = entries.clone();

        let installed_count = installed_ids.len();
        let mut by_source = HashMap::new();
        by_source.insert("local".into(), entries.len());

        Ok(MarketplaceStats {
            total_available: entries.len(),
            total_installed: installed_count,
            updates_available: 0,
            by_source,
        })
    }

    /// Full-text search with filters and pagination.
    pub fn search(&self, query: &MarketplaceQuery) -> Result<MarketplaceResponse, anyhow::Error> {
        let index = self.index.read();
        let entries = self.entries.read();

        let mut candidates: Vec<(usize, f32)> = if let Some(ref text) = query.text {
            let hits = index.search(text, 200);
            if hits.is_empty() {
                entries.iter().enumerate().map(|(i, _)| (i, 0.0)).collect()
            } else {
                hits
            }
        } else {
            entries.iter().enumerate().map(|(i, _)| (i, 0.0)).collect()
        };

        candidates.retain(|(idx, _)| {
            let e = &entries[*idx];
            if !query.tags.is_empty()
                && !query
                    .tags
                    .iter()
                    .all(|t| e.tags.iter().any(|et| et.eq_ignore_ascii_case(t)))
            {
                return false;
            }
            if !query.sources.is_empty()
                && !query
                    .sources
                    .iter()
                    .any(|s| e.source.eq_ignore_ascii_case(s))
            {
                return false;
            }
            if query.available_only && e.installed {
                return false;
            }
            if query.installed_only && !e.installed {
                return false;
            }
            if query.updates_only && !e.update_available {
                return false;
            }
            if e.rating < query.min_rating {
                return false;
            }
            true
        });

        match query.sort {
            SortBy::Relevance => candidates
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)),
            SortBy::Name => candidates.sort_by(|a, b| entries[a.0].name.cmp(&entries[b.0].name)),
            SortBy::Rating => candidates.sort_by(|a, b| {
                entries[b.0]
                    .rating
                    .partial_cmp(&entries[a.0].rating)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortBy::Installs => candidates
                .sort_by(|a, b| entries[b.0].install_count.cmp(&entries[a.0].install_count)),
            SortBy::UpdatedAt => {
                candidates.sort_by(|a, b| entries[b.0].updated_at.cmp(&entries[a.0].updated_at))
            }
        }

        let total = candidates.len();
        let page: Vec<SearchHit> = candidates
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|(idx, rel)| SearchHit {
                entry: entries[idx].clone(),
                relevance: rel,
            })
            .collect();

        Ok(MarketplaceResponse {
            results: page,
            total,
            offset: query.offset,
            limit: query.limit,
        })
    }

    /// One-click install from a remote source.
    pub fn install(&self, source: &str, _identifier: &str) -> Result<SkillEntry, anyhow::Error> {
        // Delegate to SkillImporter based on source type.
        //
        // P0 修复:`block_in_place` 包裹 `Handle::current().block_on`,
        // 避免在 async 上下文(Tauri command `marketplace_install` 是 async)
        // 中直接调用 `block_on` 导致 "Cannot block the current thread
        // from within a runtime" panic。`block_in_place` 会把当前
        // runtime worker 的其它任务转移到备份线程,然后安全阻塞。
        let result: ImportResult = match source {
            "agentskills" => {
                // For URL-based import, identifier is the URL
                let url = _identifier.to_string();
                let importer = self.importer.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(async move { importer.import_from_url(&url).await })
                })
            }
            "clawhub" => {
                let slug = _identifier.to_string();
                let importer = self.importer.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(async move { importer.import_from_clawhub(&slug).await })
                })
            }
            "teamskillshub" => {
                let asset_id = _identifier.to_string();
                let importer = self.importer.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        importer.import_from_teamskillshub(&asset_id).await
                    })
                })
            }
            other => anyhow::bail!("unknown source: {other}"),
        };

        // Refresh index after install.
        self.refresh()?;

        let entries = self.entries.read();
        let skill_id = result
            .skill
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let skill_name = result
            .skill
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let skill_tags = result
            .skill
            .as_ref()
            .map(|s| s.tags.clone())
            .unwrap_or_default();

        // P2-5: 记录技能的 source URL,供后续更新检查使用。
        // agentskills source 的 identifier 就是 URL;其他 source 记录 result.source。
        if !skill_id.is_empty() {
            let url_to_register = if source == "agentskills" {
                _identifier.to_string()
            } else {
                result.source.clone()
            };
            if !url_to_register.is_empty() {
                self.register_source_url(&skill_id, &url_to_register);
            }
        }

        let e = entries
            .iter()
            .find(|e| e.id == skill_id)
            .cloned()
            .unwrap_or_else(|| SkillEntry {
                id: skill_id,
                name: skill_name,
                description: String::new(),
                tags: skill_tags,
                version: "1.0.0".into(),
                author: source.into(),
                rating: 0.0,
                rating_count: 0,
                install_count: 1,
                icon_url: None,
                source: source.into(),
                import_identifier: Some(_identifier.into()),
                installed: true,
                installed_version: Some("1.0.0".into()),
                update_available: false,
                size_bytes: 0,
                updated_at: 0,
            });
        Ok(e)
    }

    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let entries = self.entries.read();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for e in entries.iter() {
            for t in &e.tags {
                *counts.entry(t.clone()).or_default() += 1;
            }
        }
        let mut tags: Vec<_> = counts.into_iter().collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1));
        tags
    }

    pub fn stats(&self) -> MarketplaceStats {
        let entries = self.entries.read();
        let installed = entries.iter().filter(|e| e.installed).count();
        let updates = entries.iter().filter(|e| e.update_available).count();
        let mut by_source = HashMap::new();
        for e in entries.iter() {
            *by_source.entry(e.source.clone()).or_default() += 1;
        }
        MarketplaceStats {
            total_available: entries.len(),
            total_installed: installed,
            updates_available: updates,
            by_source,
        }
    }

    pub fn check_updates(&self) -> Vec<UpdateInfo> {
        let entries = self.entries.read();
        entries
            .iter()
            .filter(|e| e.update_available)
            .map(|e| UpdateInfo {
                skill_id: e.id.clone(),
                name: e.name.clone(),
                current_version: e.installed_version.clone().unwrap_or_default(),
                latest_version: e.version.clone(),
                source: e.source.clone(),
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // P2-5: 远端版本检查 + 一键更新
    // ------------------------------------------------------------------

    /// P2-5: 从远端 URL 拉取 SKILL.md 原始内容。
    ///
    /// 复用 SSRF 校验(与 SkillImporter::fetch_skill_md 一致),防止内网地址。
    async fn fetch_remote_skill_md(&self, url: &str) -> Result<String, anyhow::Error> {
        crate::security::SsrfGuard::new()
            .validate_url(url)
            .map_err(|e| anyhow::anyhow!("SSRF validation failed for skill URL: {e}"))?;
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("HTTP request failed: {e}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("HTTP {}", resp.status());
        }
        resp.text()
            .await
            .map_err(|e| anyhow::anyhow!("reading response body failed: {e}"))
    }

    /// P2-5: 从 SKILL.md 内容中解析 version 字段（从 YAML frontmatter）。
    ///
    /// 复用 protocol 层的 `SkillSpecValidator::parse_manifest`,它返回
    /// `SkillManifest`(含 version 字段)。解析失败时返回 None。
    fn extract_version_from_skill_md(content: &str) -> Option<String> {
        let manifest = crate::skills::protocol::SkillSpecValidator::parse_manifest(content).ok()?;
        if manifest.version.is_empty() {
            None
        } else {
            Some(manifest.version)
        }
    }

    /// P2-5: 从 SKILL.md 内容中解析 changelog 字段（可选）。
    ///
    /// 从 YAML frontmatter 的 `changelog` 字段提取。若不存在返回 None。
    fn extract_changelog_from_skill_md(content: &str) -> Option<String> {
        let manifest = crate::skills::protocol::SkillSpecValidator::parse_manifest(content).ok()?;
        manifest
            .source
            .and_then(|sources| {
                // 尝试从 extra 字段或 source 列表中找 changelog
                let _ = sources;
                None
            })
            .or_else(|| {
                // 直接从 YAML frontmatter 提取 changelog 字段
                Self::extract_yaml_field(content, "changelog")
            })
    }

    /// P2-5: 从 SKILL.md 的 YAML frontmatter 中提取指定字段的字符串值。
    ///
    /// 简单的行级解析,避免引入完整的 YAML 解析依赖。
    fn extract_yaml_field(content: &str, field: &str) -> Option<String> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return None;
        }
        let rest = &trimmed[3..];
        let end = rest.find("---")?;
        let yaml_str = &rest[..end];
        let prefix = format!("{field}:");
        for line in yaml_str.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix(&prefix) {
                let value = value.trim();
                // 去掉引号
                let value = value.trim_matches(|c| c == '"' || c == '\'');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
        None
    }

    /// P2-5: 检查所有已安装技能的远端版本更新。
    ///
    /// 遍历本地存储中的所有技能,对于已注册 source URL 的技能:
    /// 1. 从远端拉取最新 SKILL.md frontmatter
    /// 2. 解析远端 version 字段
    /// 3. 与本地 version 做 semver 比对
    /// 4. 如果远端版本 > 本地版本,标记 `update_available = true`
    ///
    /// 无 source URL 的技能（本地创建）返回 `update_available = false`。
    /// 远端拉取失败的技能也返回 `update_available = false`,不中断整体检查。
    pub async fn check_remote_updates(&self) -> Vec<SkillUpdateInfo> {
        // 获取本地所有技能。
        let local_skills = match self.store.list(
            None,
            None,
            &[],
            crate::skills::types::TagMatch::Any,
            1000,
        ) {
            Ok(skills) => skills,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        for skill in &local_skills {
            // 本地版本：从 skill code 字段无法获取,使用默认 "1.0.0"。
            // 注：当前 Skill 表无 version 列,本地版本统一取 "1.0.0"。
            // 若 skill 通过 importer 安装,远端 SKILL.md 的 version 应高于 1.0.0 才触发更新。
            let current_version = "1.0.0".to_string();

            let source_url = self.get_source_url(&skill.id);

            if let Some(ref url) = source_url {
                // 有 source URL 的技能：从远端拉取并比对版本。
                match self.fetch_remote_skill_md(url).await {
                    Ok(content) => {
                        let remote_version =
                            Self::extract_version_from_skill_md(&content).unwrap_or_default();
                        let changelog = Self::extract_changelog_from_skill_md(&content);
                        let update_available = if remote_version.is_empty() {
                            false
                        } else {
                            semver_compare(&remote_version, &current_version)
                                == std::cmp::Ordering::Greater
                        };
                        results.push(SkillUpdateInfo {
                            skill_id: skill.id.clone(),
                            skill_name: skill.name.clone(),
                            current_version: current_version.clone(),
                            latest_version: remote_version,
                            update_available,
                            source_url: Some(url.clone()),
                            changelog,
                        });
                    }
                    Err(_) => {
                        // 远端拉取失败：不标记更新,避免误报。
                        results.push(SkillUpdateInfo {
                            skill_id: skill.id.clone(),
                            skill_name: skill.name.clone(),
                            current_version: current_version.clone(),
                            latest_version: String::new(),
                            update_available: false,
                            source_url: Some(url.clone()),
                            changelog: None,
                        });
                    }
                }
            } else {
                // 无 source URL 的技能：跳过远端检查。
                results.push(SkillUpdateInfo {
                    skill_id: skill.id.clone(),
                    skill_name: skill.name.clone(),
                    current_version: current_version.clone(),
                    latest_version: String::new(),
                    update_available: false,
                    source_url: None,
                    changelog: None,
                });
            }
        }
        results
    }

    /// P2-5: 一键更新单个技能。
    ///
    /// 1. 从远端拉取最新 SKILL.md
    /// 2. 解析为 CreateSkillRequest
    /// 3. 更新 store 中的技能（保留 trust_level 设置）
    /// 4. 刷新 marketplace 索引
    ///
    /// 返回更新后的 SkillUpdateInfo(含新版本号)。
    pub async fn update_skill(&self, skill_id: &str) -> Result<(), String> {
        // 1. 获取 source URL。
        let url = self
            .get_source_url(skill_id)
            .ok_or_else(|| format!("skill {skill_id} has no registered source URL"))?;

        // 2. 获取本地技能（保留 trust_level）。
        let existing = self
            .store
            .get(skill_id)
            .map_err(|e| format!("failed to read skill {skill_id}: {e}"))?
            .ok_or_else(|| format!("skill {skill_id} not found in store"))?;

        // 3. 从远端拉取最新 SKILL.md。
        let content = self
            .fetch_remote_skill_md(&url)
            .await
            .map_err(|e| format!("failed to fetch remote SKILL.md: {e}"))?;

        // 4. 解析为 CreateSkillRequest。
        let req = crate::skills::importer::SkillImporter::from_skill_md(&content)
            .map_err(|e| format!("failed to parse SKILL.md: {e}"))?;

        // 5. 构造更新后的 Skill(保留 id / trust_level / usage_count / ratings)。
        let now = chrono::Utc::now().timestamp();
        let updated_skill = crate::skills::types::Skill {
            id: existing.id.clone(),
            name: req.name,
            description: req.description,
            code: req.code,
            language: req.language,
            tags: req.tags,
            usage_count: existing.usage_count,
            avg_rating: existing.avg_rating,
            rating_count: existing.rating_count,
            created_at: existing.created_at,
            updated_at: now,
            source_memory_id: existing.source_memory_id.clone(),
            activation_condition: existing.activation_condition.clone(),
            platform: existing.platform.clone(),
            min_confidence: existing.min_confidence,
            // P2-5: 保留用户的 trust_level 设置（不因更新重置）。
            trust_level: existing.trust_level,
            permissions: existing.permissions.clone(),
            capabilities: existing.capabilities.clone(),
        };

        // 6. 写入 store（upsert：相同 id 则更新）。
        self.store
            .upsert(&updated_skill)
            .map_err(|e| format!("failed to upsert skill {skill_id}: {e}"))?;

        // 7. 刷新 marketplace 索引。
        let _ = self.refresh();

        Ok(())
    }

    pub fn generate_manifest(&self, skill_id: &str) -> Result<PublishManifest, anyhow::Error> {
        let skill = self
            .store
            .get(skill_id)?
            .ok_or_else(|| anyhow::anyhow!("skill not found: {skill_id}"))?;
        Ok(PublishManifest {
            manifest_version: "1.0".into(),
            id: skill.id,
            name: skill.name,
            version: "0.1.0".into(),
            description: skill.description,
            author: String::new(),
            tags: skill.tags,
            source_url: None,
            dependencies: vec![],
            min_nebula_version: Some("1.3.0".into()),
            extra: HashMap::new(),
        })
    }

    pub fn validate_manifest(manifest: &PublishManifest) -> Result<(), anyhow::Error> {
        if manifest.id.is_empty() {
            anyhow::bail!("id is required");
        }
        if manifest.name.is_empty() {
            anyhow::bail!("name is required");
        }
        if manifest.version.is_empty() {
            anyhow::bail!("version is required");
        }
        if manifest.description.is_empty() {
            anyhow::bail!("description is required");
        }
        let parts: Vec<&str> = manifest.version.split('.').collect();
        if parts.len() != 3 {
            anyhow::bail!("version must be semver (X.Y.Z)");
        }
        for p in parts {
            p.parse::<u32>()
                .map_err(|_| anyhow::anyhow!("version segment '{p}' is not a number"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        let tokens = tokenize(
            "Rust Skill",
            "A rust formatter",
            &["rust".into(), "tools".into()],
        );
        assert!(tokens.iter().any(|t| t == "rust"));
    }

    #[test]
    fn search_finds_entries() {
        let entries = vec![
            SkillEntry {
                id: "a".into(),
                name: "Python Formatter".into(),
                description: "Format Python code".into(),
                tags: vec!["python".into()],
                version: "1.0.0".into(),
                author: "test".into(),
                rating: 4.0,
                rating_count: 10,
                install_count: 100,
                icon_url: None,
                source: "local".into(),
                import_identifier: None,
                installed: true,
                installed_version: Some("1.0.0".into()),
                update_available: false,
                size_bytes: 0,
                updated_at: 0,
            },
            SkillEntry {
                id: "b".into(),
                name: "Rust Formatter".into(),
                description: "Format Rust code".into(),
                tags: vec!["rust".into()],
                version: "1.0.0".into(),
                author: "test".into(),
                rating: 0.0,
                rating_count: 0,
                install_count: 0,
                icon_url: None,
                source: "local".into(),
                import_identifier: None,
                installed: false,
                installed_version: None,
                update_available: false,
                size_bytes: 0,
                updated_at: 0,
            },
        ];
        let mut idx = InvertedIndex::new();
        idx.build(entries);
        let hits = idx.search("Rust", 10);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn validate_manifest_ok() {
        let m = PublishManifest {
            id: "my-skill".into(),
            name: "My Skill".into(),
            version: "1.2.3".into(),
            description: "A test skill".into(),
            ..Default::default()
        };
        assert!(SkillMarketplace::validate_manifest(&m).is_ok());
    }

    #[test]
    fn validate_manifest_bad_version() {
        let m = PublishManifest {
            id: "my-skill".into(),
            name: "My Skill".into(),
            version: "latest".into(),
            description: "A test skill".into(),
            ..Default::default()
        };
        assert!(SkillMarketplace::validate_manifest(&m).is_err());
    }
}
