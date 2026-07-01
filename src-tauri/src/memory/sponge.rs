//! Sponge absorption engine.
//!
//! The sponge is the *entry point* for new memories. When a new
//! [`Memory`] is absorbed it is:
//!
//! 1. Embedded (via the shared [`Embedder`]).
//! 2. Compared against existing memories in the vector store; if a
//!    cosine similarity is above `SPONGE_MERGE_THRESHOLD` the two
//!    records are merged instead of being inserted as a duplicate.
//! 3. Tagged with multi-granularity summaries derived from the content.
//! 4. Persisted to SQLite and to LanceDB.
//!
//! The engine is stateless: callers pass in the `Memory` and receive a
//! [`SpongeResult`] describing what happened.

use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, warn};

use super::constants::SPONGE_MERGE_THRESHOLD;
use super::constants::SUMMARY_BUCKETS;
use super::embedder::Embedder;
use super::entity_extractor::EntityExtractor;
use super::graph_search::{GraphSearchConfig, GraphSearchEngine};
use super::lance_store::LanceStore;
use super::sqlite_store::SqliteStore;
use super::types::{Memory, MemoryRelation, MemoryType, MultiGranularity, RelationKind, SourceKind};
use crate::llm::LlmGateway;
use crate::security::SensitiveScanner;

/// What happened when a memory was absorbed.
#[derive(Debug, Clone)]
pub enum SpongeResult {
    /// The new memory was inserted as a brand new record.
    Inserted { id: String },
    /// The new memory was merged into an existing one. The existing
    /// record's `id` is returned.
    Merged { id: String, similarity: f32 },
    /// The new memory was a perfect duplicate; the existing record was
    /// just touched.
    Duplicate { id: String },
    /// v1.5: 记忆因关键词未激活而被降级吸收（importance 衰减）。
    /// 仍然入库，但 importance 被乘以衰减因子，使黑洞引擎更可能压缩它。
    Deactivated { id: String },
}

impl SpongeResult {
    pub fn id(&self) -> &str {
        match self {
            SpongeResult::Inserted { id }
            | SpongeResult::Merged { id, .. }
            | SpongeResult::Duplicate { id }
            | SpongeResult::Deactivated { id } => id,
        }
    }
}

/// v1.5: 关键词激活器。
///
/// 设计文档 v7.0 §3.1 海绵多腔体 — 关键词激活：只有内容命中
/// 激活关键词的记忆才会被全量吸收；未命中的记忆 importance 衰减
/// （但不丢弃，避免信息丢失）。
///
/// 默认激活关键词集覆盖中英文常见的重要信号词。
#[derive(Debug, Clone)]
pub struct KeywordActivator {
    /// 激活关键词集合（小写匹配）。
    keywords: Vec<String>,
    /// 未命中时 importance 的衰减因子。
    decay_factor: f32,
}

impl Default for KeywordActivator {
    fn default() -> Self {
        Self::with_keywords(default_activation_keywords(), 0.3)
    }
}

impl KeywordActivator {
    /// 用指定的关键词集创建激活器。
    pub fn with_keywords(keywords: Vec<String>, decay_factor: f32) -> Self {
        Self {
            keywords,
            decay_factor,
        }
    }

    /// 检查内容是否命中任一激活关键词。
    pub fn activate(&self, content: &str) -> bool {
        if self.keywords.is_empty() {
            return true; // 无关键词 → 总是激活
        }
        let lower = content.to_lowercase();
        self.keywords.iter().any(|kw| lower.contains(kw))
    }

    /// 返回衰减因子。
    pub fn decay_factor(&self) -> f32 {
        self.decay_factor
    }

    /// 添加自定义激活关键词。
    pub fn add_keyword(&mut self, keyword: impl Into<String>) {
        let k = keyword.into().to_lowercase();
        if !k.is_empty() && !self.keywords.contains(&k) {
            self.keywords.push(k);
        }
    }
}

/// 默认激活关键词集（中英文）。
fn default_activation_keywords() -> Vec<String> {
    [
        // 中文重要信号词
        "重要", "记住", "注意", "关键", "必须", "不要忘记", "切记", "重点",
        "紧急", "优先", "核心", "结论", "决定", "发现", "问题", "错误",
        "教训", "经验", "规则", "约定", "偏好", "目标", "计划",
        // 英文重要信号词
        "important", "remember", "note", "key", "must", "critical",
        "warning", "error", "bug", "fix", "todo", "decision", "conclusion",
        "lesson", "rule", "preference", "goal", "plan",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// v1.5: 多腔体配置。
///
/// 设计文档 v7.0 §3.1 海绵多腔体 — 按记忆类型分腔，去重时只在
/// 同类型记忆之间进行。每种记忆类型是一个独立的"腔体"。
#[derive(Debug, Clone)]
pub struct ChamberConfig {
    /// 启用多腔体（按 memory_type 隔离去重）。
    pub enabled: bool,
    /// 搜索候选时额外获取的倍数（如 3x 表示多取 3 倍结果用于类型过滤）。
    pub search_multiplier: usize,
}

impl Default for ChamberConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            search_multiplier: 3,
        }
    }
}

/// Sponge absorption engine.
pub struct SpongeEngine {
    sqlite: Arc<SqliteStore>,
    lance: Arc<LanceStore>,
    embedder: Arc<Embedder>,
    sensitive_scanner: SensitiveScanner,
    entity_extractor: Option<EntityExtractor>,
    /// v1.5: 关键词激活器。
    keyword_activator: KeywordActivator,
    /// v1.5: 多腔体配置。
    chamber_config: ChamberConfig,
}

impl SpongeEngine {
    pub fn new(sqlite: Arc<SqliteStore>, lance: Arc<LanceStore>, embedder: Arc<Embedder>) -> Self {
        Self {
            sqlite,
            lance,
            embedder,
            sensitive_scanner: SensitiveScanner::new(),
            entity_extractor: None,
            keyword_activator: KeywordActivator::default(),
            chamber_config: ChamberConfig::default(),
        }
    }

    pub fn with_llm(mut self, llm: LlmGateway) -> Self {
        self.entity_extractor = Some(EntityExtractor::new(llm));
        self
    }

    /// v1.5: 设置自定义关键词激活器。
    pub fn with_keyword_activator(mut self, activator: KeywordActivator) -> Self {
        self.keyword_activator = activator;
        self
    }

    /// v1.5: 设置多腔体配置。
    pub fn with_chamber_config(mut self, config: ChamberConfig) -> Self {
        self.chamber_config = config;
        self
    }

    /// v1.5: 访问关键词激活器（用于运行时添加关键词）。
    pub fn keyword_activator(&self) -> &KeywordActivator {
        &self.keyword_activator
    }

    /// v1.5: 访问关键词激活器（可变引用）。
    pub fn keyword_activator_mut(&mut self) -> &mut KeywordActivator {
        &mut self.keyword_activator
    }

    /// Absorbs a freshly created memory into the system.
    ///
    /// The supplied [`Memory`] may have an empty `embedding` field; this
    /// function will populate it.
    ///
    /// v1.0.1 P0#10: every write into the `memories` table
    /// (duplicate-touch, merge, or fresh-insert) is now wrapped
    /// in the process-wide `compression_lock` so a
    /// `BlackholeEngine::run_pass` can't be in the middle of
    /// rewriting the same row.  The cost is that absorb briefly
    /// serialises with compression; in exchange the reader in
    /// `sponge::absorb` can no longer observe a half-rewritten
    /// `memories.content` cell.  The lock is held only across
    /// the SQLite write 鈥?not across the (slow) embedding call
    /// 鈥?so latency is bounded.
    pub async fn absorb(&self, mut mem: Memory) -> Result<SpongeResult> {
        // 1. Normalise / strip.
        mem.content = normalise(&mem.content);
        mem.summary = derive_summaries(&mem.content);

        // v1.1 P1-4: 鍦ㄥ惛鏀跺墠鎵弿鏁忔劅鏁版嵁
        let (redacted_content, sensitive_categories) = self.sensitive_scanner.scan(&mem.content);
        if !sensitive_categories.is_empty() {
            tracing::warn!(
                target: "nine_snake.memory",
                ?sensitive_categories,
                "sensitive data detected in memory; redacted before storage"
            );
            mem.content = redacted_content;
        }

        // v1.5: 关键词激活检查。
        // 未命中激活关键词的记忆 importance 衰减，但仍入库（不丢弃信息）。
        let activated = self.keyword_activator.activate(&mem.content);
        if !activated {
            let decay = self.keyword_activator.decay_factor();
            debug!(target: "nine_sponge", decay, "keyword not activated; importance decayed");
            mem.importance *= decay;
        }

        // 2. Embed.
        if mem.embedding.is_empty() {
            mem.embedding = self.embedder.embed(&mem.content).await?;
        }
        if mem.embedding.len() != self.lance.dim() {
            // Embedder enforces dim; this is defensive.
            anyhow::bail!("embedding dim mismatch with vector store");
        }

        // 3. De-duplicate via the vector store.
        // v1.5: 多腔体 — 扩展搜索范围，按 memory_type 过滤候选。
        let top = if self.chamber_config.enabled {
            let expanded_k = 3 * self.chamber_config.search_multiplier.max(1);
            let raw = self.lance.search(&mem.embedding, expanded_k).await?;
            // 按 memory_type 过滤候选（多腔体隔离）。
            self.filter_by_chamber(raw, mem.memory_type).await?
        } else {
            self.lance.search(&mem.embedding, 3).await?
        };
        if let Some((existing_id, sim)) = top.first().cloned() {
            if sim >= SPONGE_MERGE_THRESHOLD {
                if sim > 0.99 {
                    // Effectively identical. Touch and bail.
                    // v1.0.1 P0#10: hold the compression lock only
                    // around the SQLite write.
                    if let Some(mut existing) = self.sqlite.get(&existing_id).await? {
                        let now = chrono::Utc::now().timestamp();
                        existing.touch(now);
                        self.sqlite.update_guarded_spawn(&existing).await?;
                        debug!(target: "nine_sponge", id = %existing_id, sim, "duplicate absorbed");
                        return Ok(SpongeResult::Duplicate { id: existing_id });
                    }
                }
                // Merge: append content, keep the higher-importance
                // slot. The original record is preserved (pinned or
                // not) so the sponge never destroys data 鈥?the black
                // hole is the only engine that compresses.
                if let Some(mut existing) = self.sqlite.get(&existing_id).await? {
                    existing.content = merge_content(&existing.content, &mem.content);
                    existing.summary = derive_summaries(&existing.content);
                    if mem.importance > existing.importance {
                        existing.importance = mem.importance;
                    }
                    existing.access_count = existing.access_count.saturating_add(1);
                    let now = chrono::Utc::now().timestamp();
                    existing.last_access = now;
                    // v1.0.1 P0#10: hold the compression lock for
                    // the duration of the merge write.  We do NOT
                    // re-embed while holding the lock because the
                    // embed call is async and would block
                    // compress for its full duration.
                    let (new_emb, updated_id) = {
                        self.sqlite.update_guarded_spawn(&existing).await?;
                        (existing.content.clone(), existing.id.clone())
                    };
                    // Re-embed merged content after the lock is
                    // released.  The vector store upsert is
                    // independent of the SQLite row state.
                    let new_emb_vec = self.embedder.embed(&new_emb).await?;
                    self.lance.upsert(&updated_id, &new_emb_vec).await?;
                    debug!(target: "nine_sponge", id = %updated_id, sim, "merged into existing");
                    return Ok(SpongeResult::Merged {
                        id: updated_id,
                        similarity: sim,
                    });
                }
            }
        }

        // 4. Insert fresh.
        let now = chrono::Utc::now().timestamp();
        if mem.last_access == 0 {
            mem.last_access = now;
        }
        if mem.created_at == 0 {
            mem.created_at = now;
        }
        // Tag with source metadata so the front-end can show provenance.
        if mem.metadata.get("absorbed_at").is_none() {
            if let serde_json::Value::Object(ref mut map) = mem.metadata {
                map.insert("absorbed_at".to_string(), serde_json::Value::from(now));
                map.insert(
                    "absorbed_via".to_string(),
                    serde_json::Value::from("sponge"),
                );
            }
        }

        // v1.0.1 P0#10: hold the compression lock around the
        // SQLite insert + commit log.
        //
        // v1.0.1 fix B: split the locked section into two
        // halves so the `parking_lot::MutexGuard` (which is
        // `!Send`) is **never** held across an `.await`.
        // `self.lance.upsert(...).await` is the only async
        // call between the SQLite write and the commit log,
        // so we drop the lock around it.  The brief
        // window where neither lock nor the await is held
        // is acceptable: the blackhole pass would still see
        // either the pre-insert state (row missing) or the
        // post-insert state (row present), never a
        // half-written one, because the insert is a single
        // SQL statement that is atomic at the SQLite level.
        {
            if mem.is_sensitive() {
                mem.summary.s2000.clear();
                mem.summary.s500 = redact_marker(&mem.summary.s500);
                mem.summary.s150 = redact_marker(&mem.summary.s150);
                mem.summary.s50 = redact_marker(&mem.summary.s50);
                if let serde_json::Value::Object(ref mut map) = mem.metadata {
                    map.insert("masked".to_string(), serde_json::Value::from(true));
                    map.insert(
                        "mask_reason".to_string(),
                        serde_json::Value::from("sensitive-content-predicate"),
                    );
                }
            }
            self.sqlite.insert_guarded_spawn(&mem).await?;
        }
        // Async write to the vector index 鈥?outside the
        // parking_lot lock so the future stays `Send`.
        self.lance.upsert(&mem.id, &mem.embedding).await?;
        self.sqlite
            .log_commit(
                &uuid::Uuid::new_v4().to_string(),
                None,
                "store",
                &mem.id,
                &serde_json::json!({
                    "source": mem.source.as_str(),
                    "layer": mem.layer.as_str(),
                    "masked": mem.is_sensitive(),
                }),
                "sponge",
                "absorbed new memory",
            )
            .await?;

        // If there are near neighbours below the merge threshold, link
        // them with a "references" relation so the knowledge graph grows.
        for (nid, nsim) in top.iter() {
            if *nsim >= 0.6 && *nsim < SPONGE_MERGE_THRESHOLD {
                let mut rel =
                    MemoryRelation::new(mem.id.clone(), nid.clone(), RelationKind::References);
                rel.weight = *nsim;
                let _ = self.sqlite.add_relation(&rel).await;
            }
        }

        // V2-T-22: LLM-driven entity extraction for richer relations.
        if let Some(ref extractor) = self.entity_extractor {
            let existing_ids: Vec<String> = top.iter().map(|(id, _)| id.clone()).collect();
            match extractor
                .extract(&mem.id, &mem.content, &existing_ids)
                .await
            {
                Ok(extracted) => {
                    for er in extracted {
                        let mut rel = MemoryRelation::new(er.from_id, er.to_id, er.relation);
                        if let Some(evidence) = er.evidence {
                            rel = rel.with_evidence(evidence);
                        }
                        if let Err(e) = self.sqlite.add_relation(&rel).await {
                            warn!(target: "nine_sponge", error = %e, "failed to insert extracted relation");
                        }
                    }
                }
                Err(e) => {
                    warn!(target: "nine_sponge", error = %e, "entity extraction failed; continuing with cosine-based relations");
                }
            }
        }

        debug!(target: "nine_sponge", id = %mem.id, "absorbed new memory");
        Ok(SpongeResult::Inserted { id: mem.id })
    }

    /// v1.5: 多腔体过滤 — 按记忆类型过滤向量搜索候选。
    ///
    /// 批量获取候选记忆的类型，只保留与目标类型相同的候选。
    /// 如果批量获取失败或所有候选类型不匹配，返回空列表
    /// （此时 sponge 会走插入路径，不会误合并到不同类型的记忆）。
    async fn filter_by_chamber(
        &self,
        raw: Vec<(String, f32)>,
        target_type: MemoryType,
    ) -> Result<Vec<(String, f32)>> {
        if raw.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = raw.iter().map(|(id, _)| id.clone()).collect();
        let memories = self.sqlite.get_many(&ids).await.unwrap_or_default();

        // 构建 id → memory_type 映射
        let type_map: std::collections::HashMap<&str, MemoryType> = memories
            .iter()
            .map(|m| (m.id.as_str(), m.memory_type))
            .collect();

        // 过滤：只保留同类型候选
        let filtered: Vec<(String, f32)> = raw
            .into_iter()
            .filter(|(id, _)| {
                type_map.get(id.as_str()).map_or(false, |t| *t == target_type)
            })
            .collect();

        Ok(filtered)
    }

    /// Convenience: build a fresh [`Memory`] from raw inputs and absorb
    /// it. Useful for the Tauri command layer.
    pub async fn absorb_text(
        &self,
        memory_type: super::types::MemoryType,
        layer: super::types::MemoryLayer,
        content: impl Into<String>,
        source: SourceKind,
    ) -> Result<SpongeResult> {
        let m = Memory::new(memory_type, layer, content, source);
        self.absorb(m).await
    }

    /// Hybrid search: vector similarity + optional graph traversal expansion.
    ///
    /// Returns memory IDs from both vector search and graph expansion.
    /// The graph traversal uses BFS from the seed IDs found by vector
    /// search, following `MemoryRelation` edges.
    pub async fn search_with_graph(
        &self,
        query: &str,
        k: usize,
        graph_config: Option<GraphSearchConfig>,
    ) -> Result<Vec<(String, f32)>> {
        let query_emb = self.embedder.embed(query).await?;
        let mut hits = self.lance.search(&query_emb, k).await?;

        if let Some(ref cfg) = graph_config {
            let seed_ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
            if !seed_ids.is_empty() {
                let graph_engine = GraphSearchEngine::new((*self.sqlite).clone());
                let graph_results = graph_engine.traverse(&seed_ids, cfg);
                for gr in graph_results {
                    if !hits.iter().any(|(id, _)| id == &gr.memory_id) {
                        let score = 1.0 / (1.0 + gr.hops as f32);
                        hits.push((gr.memory_id, score));
                    }
                }
            }
        }

        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(hits)
    }
}

/// Normalises a piece of text: trims, collapses internal whitespace.
fn normalise(s: &str) -> String {
    let trimmed = s.trim();
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out
}

/// Produces four summaries at the canonical bucket sizes by truncating
/// the content. The longest bucket (`2000`) is the content itself when
/// shorter than 2000 chars.
fn derive_summaries(content: &str) -> MultiGranularity {
    let mut buckets = vec![String::new(); 4];
    for (i, target) in SUMMARY_BUCKETS.iter().enumerate() {
        buckets[i] = truncate_chars(content, *target);
    }
    MultiGranularity {
        s50: buckets[0].clone(),
        s150: buckets[1].clone(),
        s500: buckets[2].clone(),
        s2000: buckets[3].clone(),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Merges two content blobs by appending a separator.
fn merge_content(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    if b.is_empty() {
        return a.to_string();
    }
    format!("{a}\n---\n{b}")
}

/// v1.0.1 P0#12: short, neutral replacement shown to the user
/// in place of a redacted summary.  We intentionally do NOT
/// include the trigger token (e.g. "secret") in the marker so a
/// future log search doesn't accidentally confirm the
/// redaction was triggered.
const REDACT_MARKER: &str = "[redacted: sensitive content]";

/// Returns a redacted replacement for `s`.  If `s` is already
/// empty we keep it empty; otherwise we collapse the whole
/// summary to the canonical marker.
fn redact_marker(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        REDACT_MARKER.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // v1.0.1 P0#12: the masking test needs the type constructors
    // that the production `use super::types::*` line is the only
    // thing that brings into scope.  Re-import them locally so the
    // test doesn't depend on the parent module's use list.
    use crate::memory::types::{MemoryLayer, MemoryType};

    #[test]
    fn normalise_collapses_whitespace() {
        let n = normalise("  hello   world\n\nfoo  ");
        assert_eq!(n, "hello world foo");
    }

    #[test]
    fn derive_summaries_respects_bucket_sizes() {
        let long: String = "a".repeat(3000);
        let s = derive_summaries(&long);
        assert!(s.s50.chars().count() <= 50);
        assert!(s.s150.chars().count() <= 150);
        assert!(s.s500.chars().count() <= 500);
        assert!(s.s2000.chars().count() <= 2000);
    }

    #[test]
    fn merge_content_dedupes_empty() {
        assert_eq!(merge_content("", "x"), "x");
        assert_eq!(merge_content("x", ""), "x");
        assert!(merge_content("a", "b").contains("---"));
    }

    /// v1.0.1 P0#12: a sensitive `Memory` (one whose `content`
    /// matches the predicate) must have its `s2000` summary
    /// blanked out before the row is written.  We test the
    /// `redact_marker` helper and the predicate plumbing
    /// without standing up the full SpongeEngine (which
    /// requires a LanceDB instance).
    #[test]
    fn summary_masks_api_key_pattern() {
        // Build a sensitive memory.
        let mut m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "MY_API_KEY=sk-abc123def456ghi789jkl012mno345pqr678stu901vwx",
            SourceKind::UserInput,
        );
        m.summary = MultiGranularity {
            s50: "MY_API_KEY=sk-abc123def456ghi789jkl012mno345pqr678stu901vwx".into(),
            s150: "MY_API_KEY=sk-abc123def456ghi789jkl012mno345pqr678stu901vwx".into(),
            s500: "MY_API_KEY=sk-abc123def456ghi789jkl012mno345pqr678stu901vwx".into(),
            s2000: "MY_API_KEY=sk-abc123def456ghi789jkl012mno345pqr678stu901vwx".into(),
        };
        // The predicate must flag the content.
        assert!(m.is_sensitive(), "predicate missed a clear api_key match");
        // Apply the masking pipeline (mirrors what sponge::absorb
        // does at write time).
        m.summary.s2000.clear();
        m.summary.s500 = redact_marker(&m.summary.s500);
        m.summary.s150 = redact_marker(&m.summary.s150);
        m.summary.s50 = redact_marker(&m.summary.s50);
        // s2000 must be empty (the secret is gone).
        assert!(m.summary.s2000.is_empty(), "s2000 must be cleared");
        // The shorter summaries are replaced with the marker,
        // not the secret.
        for s in [&m.summary.s50, &m.summary.s150, &m.summary.s500] {
            assert!(!s.contains("sk-abc"), "summary leaked secret: {s}");
            assert!(
                s.contains("redacted") || s.is_empty(),
                "summary should be marker or empty, got: {s}"
            );
        }
        // The raw `content` is left untouched 鈥?the masking
        // affects the persisted summaries, not the in-memory
        // record (the latter is for the engine's own use).
        assert!(m.content.contains("sk-abc"));
    }

    // ---- v1.5: 关键词激活 + 多腔体测试 ----

    #[test]
    fn keyword_activator_default_activates_on_known_words() {
        let ka = KeywordActivator::default();
        assert!(ka.activate("这是一个重要的决定"));
        assert!(ka.activate("remember this bug"));
        assert!(ka.activate("Please note this critical rule"));
    }

    #[test]
    fn keyword_activator_default_does_not_activate_on_noise() {
        let ka = KeywordActivator::default();
        assert!(!ka.activate("今天天气不错"));
        assert!(!ka.activate("the quick brown fox jumps"));
    }

    #[test]
    fn keyword_activator_empty_keywords_always_activates() {
        let ka = KeywordActivator::with_keywords(Vec::new(), 0.5);
        assert!(ka.activate("任意内容"));
        assert!(ka.activate("anything at all"));
    }

    #[test]
    fn keyword_activator_add_keyword_works() {
        let mut ka = KeywordActivator::with_keywords(Vec::new(), 0.5);
        assert!(!ka.activate("特殊内容"));
        ka.add_keyword("特殊");
        assert!(ka.activate("这是一段特殊内容"));
    }

    #[test]
    fn keyword_activator_case_insensitive() {
        let ka = KeywordActivator::with_keywords(vec!["important".to_string()], 0.5);
        assert!(ka.activate("This is IMPORTANT"));
        assert!(ka.activate("this is important"));
    }

    #[test]
    fn keyword_activator_decay_factor() {
        let ka = KeywordActivator::with_keywords(vec!["key".to_string()], 0.25);
        assert_eq!(ka.decay_factor(), 0.25);
        let default = KeywordActivator::default();
        assert_eq!(default.decay_factor(), 0.3);
    }

    #[test]
    fn chamber_config_default_values() {
        let cfg = ChamberConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.search_multiplier, 3);
    }

    #[test]
    fn default_activation_keywords_not_empty() {
        let kws = default_activation_keywords();
        assert!(!kws.is_empty());
        assert!(kws.contains(&"重要".to_string()));
        assert!(kws.contains(&"important".to_string()));
    }
}
