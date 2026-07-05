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

use super::acl::{AclPermission, MemoryAcl};
use super::constants::SPONGE_MERGE_THRESHOLD;
use super::constants::SUMMARY_BUCKETS;
use super::embedder::Embedder;
use super::entity_extractor::EntityExtractor;
use super::graph_search::{GraphSearchConfig, GraphSearchEngine};
use super::hybrid_search::{HybridSearchConfig, HybridSearcher};
use super::sqlite_store::SqliteStore;
use super::types::{Memory, MemoryRelation, MemoryType, MultiGranularity, RelationKind, SourceKind};
// T-E-S-42: SpongeEngine 面向 VectorStore trait 编程,可接受任意后端。
use super::vector_store::VectorStore;
use crate::llm::cost_tracker::CostTracker;
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
    lance: Arc<dyn VectorStore>,
    embedder: Arc<Embedder>,
    sensitive_scanner: SensitiveScanner,
    entity_extractor: Option<EntityExtractor>,
    /// v1.5: 关键词激活器。
    keyword_activator: KeywordActivator,
    /// v1.5: 多腔体配置。
    chamber_config: ChamberConfig,
    /// T-S1-A-04: 记忆访问控制列表。
    ///
    /// `None` 表示该 SpongeEngine 实例不启用 ACL 过滤（供内部可信路径
    /// 使用，如 reflection worker、blackhole 压缩）。`Some` 表示外部
    /// 主体（Skill / MCP / REST）调用 `search_with_acl()` 时按规则
    /// 过滤。默认策略由 `MemoryAcl::check()` 决定（T-S1-PRE-02 已改
    /// 为可信主体 allow + 其他 deny-all）。
    acl: Option<Arc<MemoryAcl>>,
    /// T-E-A-09: 成本追踪器（可选）。
    ///
    /// `None` 表示未注入 cost_tracker（旧调用路径 / 测试环境），
    /// `absorb()` 不会写入 `mem.ingest_cost`（保持 `None`）。
    /// `Some` 表示已注入，`absorb()` 会在 LLM 抽取前后采样
    /// `total_cost_usd()` 差值，写入 `mem.ingest_cost`。
    cost_tracker: Option<Arc<CostTracker>>,
}

impl SpongeEngine {
    pub fn new(sqlite: Arc<SqliteStore>, lance: Arc<dyn VectorStore>, embedder: Arc<Embedder>) -> Self {
        Self {
            sqlite,
            lance,
            embedder,
            sensitive_scanner: SensitiveScanner::new(),
            entity_extractor: None,
            keyword_activator: KeywordActivator::default(),
            chamber_config: ChamberConfig::default(),
            acl: None,
            cost_tracker: None,
        }
    }

    /// T-E-B-11: Hybrid search combining BM25 keyword search and
    /// vector similarity search.
    ///
    /// Delegates to [`HybridSearcher`] which:
    /// 1. Runs BM25 (FTS5) and vector search (LanceDB) in parallel.
    /// 2. Normalises both score lists to `[0, 1]`.
    /// 3. Fuses: `alpha * vector + (1 - alpha) * bm25` (default
    ///    `alpha = 0.6`, vector-leaning).
    /// 4. Deduplicates by `memory_id` and truncates to `limit`.
    ///
    /// Returns `(Memory, fused_score)` pairs sorted by descending
    /// score. Unlike [`search_with_graph`], this returns full
    /// [`Memory`] entries (hydrated from SQLite) rather than just
    /// `(id, score)` tuples, and it combines keyword-exact recall
    /// (BM25) with semantic recall (vector) for better coverage.
    pub async fn search_hybrid(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(Memory, f64)>> {
        let searcher = HybridSearcher::new(
            self.sqlite.clone(),
            self.lance.clone(),
            self.embedder.clone(),
        );
        searcher.search(query, limit).await
    }

    /// T-E-B-11: Hybrid search with a custom fusion config.
    ///
    /// Allows callers to tune `alpha` (vector vs BM25 weight) and
    /// `over_fetch` (candidate pool size). See [`HybridSearchConfig`].
    pub async fn search_hybrid_with_config(
        &self,
        query: &str,
        limit: usize,
        config: HybridSearchConfig,
    ) -> Result<Vec<(Memory, f64)>> {
        let searcher = HybridSearcher::new(
            self.sqlite.clone(),
            self.lance.clone(),
            self.embedder.clone(),
        )
        .with_config(config);
        searcher.search(query, limit).await
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

    /// T-S1-A-04: 注入 MemoryAcl，启用 `search_with_acl()` 的结果过滤。
    ///
    /// 不调用此方法时 `acl` 为 `None`，`search_with_acl()` 会直接返回
    /// 未过滤的结果（等同于 `search_with_graph()`），保证内部可信路径
    /// （reflection / blackhole / orchestrator 以 system 主体运行）不
    /// 受影响。外部主体（Skill / MCP / REST）的搜索请求应由调用方改走
    /// `search_with_acl()` 并传入 requester_id。
    pub fn with_acl(mut self, acl: Arc<MemoryAcl>) -> Self {
        self.acl = Some(acl);
        self
    }

    /// T-S1-A-04: 访问当前注入的 ACL（供测试与运行时检查）。
    pub fn acl(&self) -> Option<&Arc<MemoryAcl>> {
        self.acl.as_ref()
    }

    /// T-E-A-09: 注入 CostTracker,启用 `absorb()` 中的成本采样。
    ///
    /// 注入后,`absorb()` 会在入口记录 `total_cost_usd()` 起点,在
    /// LLM 抽取(`entity_extractor.extract()`)完成后记录终点,将
    /// 差值写入 `mem.ingest_cost`。Duplicate / Merged 分支不调用
    /// LLM 抽取,因此差值通常为 `Some(0.0)`(已追踪但为零)。
    ///
    /// 未注入时 `absorb()` 保持 `mem.ingest_cost = None`(向后兼容)。
    pub fn with_cost_tracker(mut self, tracker: Arc<CostTracker>) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// T-E-A-09: 访问当前注入的 CostTracker（供测试与运行时检查）。
    pub fn cost_tracker(&self) -> Option<&Arc<CostTracker>> {
        self.cost_tracker.as_ref()
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
        // T-E-A-09: 入口记录成本起点(若 cost_tracker 已注入)。
        // 用于在 LLM 抽取后计算差值,写入 mem.ingest_cost。
        let cost_before = self.cost_tracker.as_ref().map(|t| t.total_cost_usd());

        // M7b #94: injection_guard 纵深防御。
        // 在 sponge 层再次扫描(即使上层 service.rs 已扫描),防止任何绕过命令层的路径
        // (如 gRPC 直接调用、内部代码直接 absorb)写入恶意注入内容。
        // Critical/High 命中时 sanitize(用占位符替换),不拒绝存储(避免破坏正常流程)。
        let scan = crate::security::injection_guard::full_injection_scan(&mem.content);
        if let Some(severity) = scan.max_severity {
            if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
                tracing::warn!(
                    target: "nebula.security",
                    hits = scan.injection_hits.len(),
                    leaks = scan.credential_leaks.len(),
                    "critical injection detected in sponge.absorb; sanitizing content"
                );
                // 用安全占位符替换恶意内容,保留记忆条目但消除注入向量。
                mem.content = format!(
                    "[BLOCKED BY INJECTION GUARD: {} injection hits, {} credential leaks]",
                    scan.injection_hits.len(),
                    scan.credential_leaks.len()
                );
            } else if !scan.safe {
                tracing::warn!(
                    target: "nebula.security",
                    severity = %severity,
                    "non-critical injection warning in sponge.absorb"
                );
            }
        }

        // 1. Normalise / strip.
        mem.content = normalise(&mem.content);
        mem.summary = derive_summaries(&mem.content);

        // v1.1 P1-4: 鍦ㄥ惛鏀跺墠鎵弿鏁忔劅鏁版嵁
        let (redacted_content, sensitive_categories) = self.sensitive_scanner.scan(&mem.content);
        if !sensitive_categories.is_empty() {
            tracing::warn!(
                target: "nebula.memory",
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
                        // T-E-A-09: Duplicate 分支未走 LLM 抽取,
                        // 差值为 .0(tracker 存在时)。mem 即将被丢弃,
                        // 此处仅保持字段一致性(不持久化)。
                        let cost_after = self.cost_tracker.as_ref().map(|t| t.total_cost_usd());
                        mem.ingest_cost = match (cost_before, cost_after) {
                            (Some(b), Some(a)) => Some(a - b),
                            _ => None,
                        };
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
                    // T-E-A-09: Merged 分支未走 LLM 抽取,差值为 0.0
                    // (tracker 存在时)。mem 即将被丢弃,此处仅保持
                    // 字段一致性(不持久化)。
                    let cost_after = self.cost_tracker.as_ref().map(|t| t.total_cost_usd());
                    mem.ingest_cost = match (cost_before, cost_after) {
                        (Some(b), Some(a)) => Some(a - b),
                        _ => None,
                    };
                    debug!(target: "nine_sponge", id = %updated_id, sim, "merged into existing");
                    return Ok(SpongeResult::Merged {
                        id: updated_id,
                        similarity: sim,
                    });
                }
            }
        }

        // V2-T-22 + T-E-A-09: LLM-driven entity extraction for richer relations.
        //
        // 原本此块在 insert 之后,现移到 insert 之前,以便:
        // 1. 在 LLM 抽取后采样 cost_after,计算差值;
        // 2. 将 ingest_cost 写入 mem,随 insert_guarded_spawn 持久化
        //    (避免二次 UPDATE)。
        // 仅 collect 关系,add_relation 调用保留在 insert 之后
        // (与 near-neighbour 关系添加保持一致,避免潜在 FK 约束)。
        let extracted_relations: Vec<_> = if let Some(ref extractor) = self.entity_extractor {
            let existing_ids: Vec<String> = top.iter().map(|(id, _)| id.clone()).collect();
            match extractor
                .extract(&mem.id, &mem.content, &existing_ids)
                .await
            {
                Ok(extracted) => extracted,
                Err(e) => {
                    warn!(target: "nine_sponge", error = %e, "entity extraction failed; continuing with cosine-based relations");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // T-E-A-09: LLM 抽取后记录 cost_after,计算差值写入 mem.ingest_cost。
        // - tracker 存在:Some(after - before)(通常 > 0,因 LLM 调用产生费用)
        // - tracker 不存在:None(向后兼容,旧调用路径)
        let cost_after = self.cost_tracker.as_ref().map(|t| t.total_cost_usd());
        mem.ingest_cost = match (cost_before, cost_after) {
            (Some(b), Some(a)) => Some(a - b),
            _ => None,
        };

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
        // T-E-B-04: 写入结构化 provenance 到 metadata。
        // tool 优先取 absorb_text 写入的 `provenance_tool`(若存在),
        // 否则 None。保留上方 absorbed_at/absorbed_via 以向后兼容。
        if mem.metadata.get("provenance").is_none() {
            let tool = mem
                .metadata
                .get("provenance_tool")
                .and_then(|v| v.as_str());
            let provenance = crate::memory::types::Provenance::new(
                mem.source.as_str(),
                tool,
                &mem.content,
            );
            if let serde_json::Value::Object(ref mut map) = mem.metadata {
                map.insert(
                    "provenance".to_string(),
                    serde_json::to_value(&provenance).unwrap_or(serde_json::Value::Null),
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

        // V2-T-22: 持久化 LLM 抽取的关系(extract 在 insert 之前完成)。
        // T-E-A-09: relations 在 insert 之前 collect,在此处 add_relation
        // 以确保 memory row 已存在(避免潜在 FK 约束)。
        for er in extracted_relations {
            let mut rel = MemoryRelation::new(er.from_id, er.to_id, er.relation);
            if let Some(evidence) = er.evidence {
                rel = rel.with_evidence(evidence);
            }
            if let Err(e) = self.sqlite.add_relation(&rel).await {
                warn!(target: "nine_sponge", error = %e, "failed to insert extracted relation");
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

    /// M2b #36: 带 principal 的记忆吸收。
    ///
    /// 与 [`absorb`](Self::absorb) 相同,但在吸收前根据 `principal` 解析
    /// domain 并写入 `mem.domain`。这是 M2b domain 隔离的写入路径:
    ///
    /// * `evolution:agent_a` → `mem.domain = "agent_a"`
    /// * `worker:task_42` → 查 ACL 的 PrincipalDomainMap(若已注入)→ master_domain
    /// * `system` / `owner` / `local` → `mem.domain = "shared"`(默认)
    /// * 未知 principal → `mem.domain` 保持原值(默认 "shared")
    ///
    /// **向后兼容**: 旧 [`absorb`](Self::absorb) 保留,不修改 `mem.domain`
    /// (Memory::new() 默认 "shared"),等价于 `absorb_with_principal("system", mem)`。
    pub async fn absorb_with_principal(
        &self,
        principal: &str,
        mut mem: Memory,
    ) -> Result<SpongeResult> {
        // 解析 principal → domain
        let domain = self.resolve_principal_domain(principal);
        if let Some(d) = domain {
            mem.domain = d;
        }
        // 委托给 absorb() 完成实际写入
        self.absorb(mem).await
    }

    /// M2b #36: 解析 principal → domain。优先使用 ACL 的 PrincipalDomainMap
    /// (若已注入),否则使用内联规则(与 PrincipalDomainMap::resolve 相同)。
    fn resolve_principal_domain(&self, principal: &str) -> Option<String> {
        if let Some(acl) = &self.acl {
            if let Some(map) = acl.principal_domains() {
                return map.resolve(principal);
            }
        }
        // 内联回退(与 PrincipalDomainMap::resolve 规则一致)
        if let Some(rest) = principal.strip_prefix("evolution:") {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
        const TRUSTED_PRINCIPALS: &[&str] = &["system", "owner", "local"];
        if TRUSTED_PRINCIPALS.contains(&principal) {
            return Some("shared".to_string());
        }
        None
    }

    /// Convenience: build a fresh [`Memory`] from raw inputs and absorb
    /// it. Useful for the Tauri command layer.
    ///
    /// T-E-B-04: `tool` 参数记录触发吸收的 agent / 工具名(如
    /// `"writer"` / `"sponge"` / `"user"`),写入 `metadata.provenance_tool`,
    /// 由 `absorb()` 在构造 `Provenance` 时读取。传 `None` 保持向后兼容
    /// (来源信息仍由 `source` 字段保留)。
    pub async fn absorb_text(
        &self,
        memory_type: super::types::MemoryType,
        layer: super::types::MemoryLayer,
        content: impl Into<String>,
        source: SourceKind,
        tool: Option<&str>,
    ) -> Result<SpongeResult> {
        let mut m = Memory::new(memory_type, layer, content, source);
        // T-E-B-04: 记录 provenance tool,供 absorb() 写入结构化 provenance。
        if let Some(t) = tool {
            if let serde_json::Value::Object(ref mut map) = m.metadata {
                map.insert("provenance_tool".to_string(), serde_json::Value::from(t));
            }
        }
        self.absorb(m).await
    }

    /// T-E-B-09: 读取文件内容并吸收到记忆系统。供 FileWatcherEngine
    /// 在文件变更事件触发时调用。内部委托给 `absorb_text`,带
    /// `tool = Some("file-watcher")` provenance 标记,以便审计追踪。
    pub async fn absorb_file(
        &self,
        path: &std::path::Path,
        memory_type: super::types::MemoryType,
        layer: super::types::MemoryLayer,
        source: SourceKind,
    ) -> Result<SpongeResult> {
        // T-E-B-12: PDF/DOCX 二进制文档走 document_extractor;其余走原文本路径。
        let content = if super::document_extractor::detect_kind(path).is_some() {
            let (_kind, text) = super::document_extractor::extract_document_text(path)?;
            text
        } else {
            tokio::fs::read_to_string(path).await?
        };
        self.absorb_text(memory_type, layer, content, source, Some("file-watcher"))
            .await
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

    /// T-S1-A-04: 带主体权限过滤的搜索入口。
    ///
    /// 行为：
    /// 1. 调用 `search_with_graph()` 获得候选 (id, score) 列表。
    /// 2. 若 `self.acl` 为 `None`，直接返回未过滤结果（向后兼容内部可信路径）。
    /// 3. 若 `self.acl` 为 `Some`，对每条结果调用
    ///    `MemoryAcl::check(requester_id, memory_id, Read)`，保留 allow 项。
    /// 4. 通过 `tracing::warn!` 记录被过滤条目（不含内容，仅 id + requester），
    ///    便于安全审计；未来 T-S1-B-03 仪表盘接入时改写为 Prometheus 计数器。
    ///
    /// 返回值中的 `filtered_count` 字段供调用方观测被拒条目数。
    pub async fn search_with_acl(
        &self,
        query: &str,
        k: usize,
        graph_config: Option<GraphSearchConfig>,
        requester_id: &str,
    ) -> Result<AclFilteredSearch> {
        let hits = self.search_with_graph(query, k, graph_config).await?;

        let Some(acl) = &self.acl else {
            // 未注入 ACL：完全放行（内部可信路径）
            return Ok(AclFilteredSearch {
                results: hits,
                filtered_count: 0,
                acl_enforced: false,
            });
        };

        let mut kept = Vec::with_capacity(hits.len());
        let mut filtered_count = 0usize;
        for (id, score) in hits {
            if acl.check(requester_id, &id, AclPermission::Read) {
                kept.push((id, score));
            } else {
                filtered_count += 1;
                warn!(
                    target: "nebula.memory.acl",
                    requester = requester_id,
                    memory_id = %id,
                    "sponge search result denied by ACL"
                );
            }
        }
        Ok(AclFilteredSearch {
            results: kept,
            filtered_count,
            acl_enforced: true,
        })
    }
}

/// T-S1-A-04: `search_with_acl()` 的返回包装。
#[derive(Debug, Clone)]
pub struct AclFilteredSearch {
    /// 通过 ACL 过滤后的 (memory_id, score) 列表，按分数降序。
    pub results: Vec<(String, f32)>,
    /// 被 ACL 拒绝的条目数（供可观测性使用）。
    pub filtered_count: usize,
    /// 是否实际执行了 ACL 检查（false 表示未注入 ACL，直接放行）。
    pub acl_enforced: bool,
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
        // M7b #90 分类 A: KeywordActivator::activate 在 keywords 为空时返回 true
        // (无关键词 → 总是激活,见行 95-96)。原测试用 Vec::new() 初始集,
        // 导致 `assert!(!ka.activate(...))` 失败。改用非空初始集 ["other"],
        // 这样 activate("特殊内容") 不匹配 "other" → false,验证 add_keyword
        // 后能匹配新关键词 "特殊"。
        let mut ka = KeywordActivator::with_keywords(vec!["other".to_string()], 0.5);
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

    // ---- T-S1-A-04: ACL 注入与过滤测试 ----

    /// `with_acl()` builder 应将 ACL 注入 SpongeEngine，且 `acl()` 访问器返回同一引用。
    /// 不调用 `with_acl()` 时 `acl()` 应返回 `None`。
    #[test]
    fn acl_builder_injects_and_accessor_returns_reference() {
        use crate::memory::acl::{AclEffect, AclPermission, AclRule};
        // 构造一个带规则的 ACL
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "skill-1".into(),
            resource: "mem-secret".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        let acl_arc = Arc::new(acl);

        // 我们无法在单测中构造完整 SpongeEngine（需要 SQLite + LanceDB +
        // Ollama embedder），因此直接验证 ACL 模块行为，确保
        // `search_with_acl()` 依赖的 `MemoryAcl::check()` 语义正确。
        assert!(!acl_arc.check("skill-1", "mem-secret", AclPermission::Read));
        // M7b #90 分类 D: skill-1 非可信主体,对 mem-other 无匹配规则 → 默认 deny。
        // 原断言 `assert!(...)` 残留 allow-by-default 假设,与本测试行 1049
        // (`assert!(!acl_arc.check("external", ...))` 非可信默认 deny)自相矛盾。
        assert!(!acl_arc.check("skill-1", "mem-other", AclPermission::Read));
        // 可信主体默认 allow（T-S1-PRE-02）
        assert!(acl_arc.check("system", "mem-secret", AclPermission::Read));
        // 非可信主体对未匹配资源默认 deny（T-S1-PRE-02）
        assert!(!acl_arc.check("external", "mem-anything", AclPermission::Read));
    }

    /// `AclFilteredSearch` 结构体的字段语义：`acl_enforced=false` 表示
    /// 未注入 ACL（内部可信路径），`filtered_count` 应为 0。
    #[test]
    fn acl_filtered_search_passthrough_when_no_acl() {
        let passthrough = AclFilteredSearch {
            results: vec![("m1".into(), 0.9), ("m2".into(), 0.5)],
            filtered_count: 0,
            acl_enforced: false,
        };
        assert_eq!(passthrough.results.len(), 2);
        assert_eq!(passthrough.filtered_count, 0);
        assert!(!passthrough.acl_enforced);
    }

    /// `AclFilteredSearch` 在 ACL 生效时正确记录被拒条目数。
    #[test]
    fn acl_filtered_search_records_denied_count() {
        let enforced = AclFilteredSearch {
            results: vec![("m1".into(), 0.9)],
            filtered_count: 2,
            acl_enforced: true,
        };
        assert_eq!(enforced.results.len(), 1);
        assert_eq!(enforced.filtered_count, 2);
        assert!(enforced.acl_enforced);
    }

    // ---- T-E-A-09: cost_tracker 注入与差值计算测试 ----

    /// `CostTracker::total_cost_usd()` 在 record() 后应正确累加。
    /// 这是 `absorb()` 中差值计算的基础(`cost_after - cost_before`)。
    #[test]
    fn cost_tracker_total_cost_usd_accumulates_after_record() {
        let tracker = CostTracker::new();
        // 初始为 0
        let before = tracker.total_cost_usd();
        assert!(before >= 0.0);
        // record 后应 > before(取决于模型单价)
        tracker.record("gpt-4o", 100, 50);
        let after = tracker.total_cost_usd();
        assert!(
            after > before,
            "total_cost_usd should increase after record: before={before}, after={after}"
        );
    }

    /// 注入 mock CostTracker 后,差值计算逻辑应正确反映 LLM 调用费用。
    ///
    /// 此测试验证 `absorb()` 中使用的差值公式:
    /// `ingest_cost = Some(cost_after - cost_before)`(tracker 存在时)。
    /// 我们无法在单测中构造完整 SpongeEngine(需要 SQLite + LanceDB +
    /// Ollama embedder),因此直接验证差值计算的语义。
    #[test]
    fn ingest_cost_diff_matches_tracker_delta() {
        let tracker = CostTracker::new();
        let cost_before = tracker.total_cost_usd();
        // 模拟 LLM 抽取调用
        tracker.record("gpt-4o", 200, 100);
        let cost_after = tracker.total_cost_usd();
        let ingest_cost = Some(cost_after - cost_before);
        assert!(
            ingest_cost.unwrap() > 0.0,
            "ingest_cost should be positive after LLM call"
        );
    }

    /// 未注入 tracker 时,差值计算应返回 None(向后兼容)。
    /// 这对应 `absorb()` 中 `cost_tracker = None` 的分支。
    #[test]
    fn ingest_cost_none_when_tracker_absent() {
        // 模拟 cost_tracker = None 的情况
        let cost_tracker: Option<Arc<CostTracker>> = None;
        let cost_before = cost_tracker.as_ref().map(|t| t.total_cost_usd());
        let cost_after = cost_tracker.as_ref().map(|t| t.total_cost_usd());
        let ingest_cost = match (cost_before, cost_after) {
            (Some(b), Some(a)) => Some(a - b),
            _ => None,
        };
        assert!(ingest_cost.is_none(), "ingest_cost should be None when tracker absent");
    }

    /// tracker 存在但无 LLM 调用时(Duplicate/Merged 分支),差值应为 0.0。
    #[test]
    fn ingest_cost_zero_when_no_llm_call() {
        let tracker = Arc::new(CostTracker::new());
        let cost_tracker: Option<Arc<CostTracker>> = Some(tracker);
        let cost_before = cost_tracker.as_ref().map(|t| t.total_cost_usd());
        // 无 LLM 调用,cost_after == cost_before
        let cost_after = cost_tracker.as_ref().map(|t| t.total_cost_usd());
        let ingest_cost = match (cost_before, cost_after) {
            (Some(b), Some(a)) => Some(a - b),
            _ => None,
        };
        assert_eq!(ingest_cost, Some(0.0));
    }
}
