//! v1.4 Memory Orchestrator — L2 认知层协调器。
//!
//! 对应设计文档 v7.0 §2.1 的 L2 Memory Orchestrator + §4.3 上下文组装策略。
//!
//! ## 职责
//!
//! * 协调 5 种记忆类型的检索与注入
//! * 上下文组装策略（设计文档 §4.3）：
//!   - 一次 LLM 调用最多注入 **3 种**记忆类型
//!   - 默认组合：**Semantic + Episodic**
//!   - 加 Procedural（如果任务匹配 Skill）
//!   - 加 Emotional（如果涉及偏好）
//!   - Metacognitive 只在反思时使用
//!   - **总 token 上限**：3000

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};

use crate::memory::acl::{AclPermission, MemoryAcl};
// T-E-S-64: CitedMemory 用于 ContextBundle.cited_memories 字段。
use crate::memory::consistency::CitedMemory;
use crate::memory::embedder::Embedder;
use crate::memory::l0_cache::L0Cache;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::types::{Memory, MemoryType};
// T-E-S-42: MemoryOrchestrator 面向 VectorStore trait 编程,可接受任意后端。
use crate::memory::vector_store::VectorStore;

/// 上下文组装的 token 上限（设计文档 §4.3）。
const CONTEXT_TOKEN_BUDGET: usize = 3000;
/// 每种记忆类型最多取多少条。
const PER_TYPE_LIMIT: usize = 5;
/// 向量检索 top-k。
const VECTOR_TOP_K: usize = 20;

/// 粗略 token 估算（1 token ≈ 2 字符，中英混合启发式）。
fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 2).max(1)
}

/// 组装好的上下文束。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    /// 组装后的文本（可直接注入 LLM 系统提示）。
    pub text: String,
    /// 各记忆类型贡献的条目数。
    pub type_counts: Vec<(MemoryType, usize)>,
    /// 实际使用的 token 数（估算）。
    pub tokens_used: usize,
    /// 命中的记忆 id（供 L0 缓存回填）。
    pub memory_ids: Vec<String>,
    /// T-E-S-64: 被引用的记忆精简视图(从 `mem.metadata.provenance`
    /// 提取 source/tool/content_hash,snippet 取 content 前 80 字符)。
    /// 供 `consistency::analyze` 生成反幻觉 badge 报告。
    /// `#[serde(default)]` 保证旧版反序列化不破坏。
    #[serde(default)]
    pub cited_memories: Vec<CitedMemory>,
}

impl ContextBundle {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            type_counts: Vec::new(),
            tokens_used: 0,
            memory_ids: Vec::new(),
            cited_memories: Vec::new(),
        }
    }
}

/// 任务类型提示（决定选哪些记忆类型）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskHint {
    /// 是否为写作类任务（加 Procedural）。
    pub is_writing: bool,
    /// 是否涉及偏好/情感（加 Emotional）。
    pub involves_preference: bool,
    /// 是否为反思任务（加 Metacognitive）。
    pub is_reflection: bool,
}

impl TaskHint {
    /// 从任务描述自动推断提示。
    pub fn infer(description: &str) -> Self {
        let lower = description.to_lowercase();
        let is_writing = ["写", "总结", "规划", "设计", "write", "summarize", "plan", "design"]
            .iter()
            .any(|k| lower.contains(k));
        let involves_preference = ["喜欢", "偏好", "习惯", "prefer", "like", "habit", "情感"]
            .iter()
            .any(|k| lower.contains(k));
        let is_reflection = ["反思", "回顾", "总结经验", "reflect", "review", "lesson"]
            .iter()
            .any(|k| lower.contains(k));
        Self {
            is_writing,
            involves_preference,
            is_reflection,
        }
    }
}

/// Memory Orchestrator。
pub struct MemoryOrchestrator {
    sqlite: Arc<SqliteStore>,
    lance: Arc<dyn VectorStore>,
    embedder: Arc<Embedder>,
    l0: Arc<L0Cache>,
    /// T-S1-A-02: 可选的 SpongeEngine，用于 graph-enhanced 搜索。
    sponge: Option<Arc<SpongeEngine>>,
    /// T-E-S-21: 记忆访问控制列表，用于过滤注入 prompt 的记忆。
    ///
    /// `None` 表示不启用 ACL 过滤（供内部可信路径使用，如 reflection
    /// worker）。`Some` 表示所有从 L0Cache/LanceDB/SpongeEngine 获取
    /// 的记忆在注入 prompt 前都必须通过 `MemoryAcl::check()`。
    /// 默认策略由 `MemoryAcl::check()` 决定（可信主体 allow + 其他
    /// deny-all）。
    acl: Option<Arc<MemoryAcl>>,
}

impl MemoryOrchestrator {
    pub fn new(
        sqlite: Arc<SqliteStore>,
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
        l0: Arc<L0Cache>,
    ) -> Self {
        Self {
            sqlite,
            lance,
            embedder,
            l0,
            sponge: None,
            acl: None,
        }
    }

    /// T-S1-A-02: 注入 SpongeEngine 以启用 graph-enhanced 搜索。
    pub fn with_sponge(mut self, sponge: Arc<SpongeEngine>) -> Self {
        self.sponge = Some(sponge);
        self
    }

    /// T-E-S-21: 注入 MemoryAcl，启用 assemble_context 的记忆过滤。
    ///
    /// 不调用此方法时 `acl` 为 `None`，`assemble_context()` 不执行
    /// ACL 过滤（向后兼容内部可信路径）。外部主体（Skill / MCP / REST）
    /// 的上下文组装请求应由调用方注入 ACL 并传入 requester_id 作为
    /// `principal`。
    pub fn with_acl(mut self, acl: Arc<MemoryAcl>) -> Self {
        self.acl = Some(acl);
        self
    }

    /// T-E-S-21: 访问当前注入的 ACL（供测试与运行时检查）。
    pub fn acl(&self) -> Option<&Arc<MemoryAcl>> {
        self.acl.as_ref()
    }

    /// 为一个任务组装上下文。
    ///
    /// 流程：
    /// 1. 推断任务提示（决定记忆类型组合，最多 3 种）
    /// 2. 向量检索 top-k 相关记忆
    /// 3. **T-E-S-21: ACL 过滤** — 未通过 `check(principal, id, Read)`
    ///    的记忆不注入 prompt，并记录 metrics（acl_deny 计数器）
    /// 4. 按类型分组 + 截断
    /// 5. 在 3000 token 预算内组装文本
    ///
    /// `principal` 标识请求上下文的主体（如 "system"、"skill-xxx"、
    /// "user-xxx"）。未注入 ACL 时 `principal` 不生效。
    #[instrument(target = "nebula.memory", skip(self), fields(otel.kind = "memory"))]
    pub async fn assemble_context(
        &self,
        task: &str,
        principal: &str,
    ) -> anyhow::Result<ContextBundle> {
        let hint = TaskHint::infer(task);
        let selected_types = select_memory_types(&hint);

        debug!(
            target: "nebula.memory.orchestrator",
            task = %task.chars().take(60).collect::<String>(),
            types = ?selected_types,
            principal = principal,
            "assembling context"
        );

        // 1. 向量检索（T-S1-A-02: 优先用 SpongeEngine 的 graph-enhanced 搜索）。
        let hits = if let Some(ref sponge) = self.sponge {
            sponge.search_with_graph(task, VECTOR_TOP_K, None).await?
        } else {
            let query_emb = self.embedder.embed(task).await?;
            self.lance.search(&query_emb, VECTOR_TOP_K).await?
        };
        if hits.is_empty() {
            return Ok(ContextBundle::empty());
        }

        // 2. 取完整 Memory
        let ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
        let memories = self.sqlite.get_many(&ids).await?;

        // 3. T-E-S-21: ACL 过滤 — 所有记忆来源（LanceDB/SpongeEngine/SQLite
        //    hydration）在注入 prompt 前都必须通过 ACL check。未注入
        //    ACL 时直接放行（内部可信路径向后兼容）。拒绝的记忆记录
        //    warn 日志（不含内容，仅 id + principal）便于安全审计，
        //    `acl.check()` 内部已上报 metrics（acl_deny 计数器）。
        let memories = apply_acl_filter(self.acl.as_deref(), memories, principal);
        if memories.is_empty() {
            return Ok(ContextBundle::empty());
        }

        // 4. 按 id 建立分数映射（用于排序）
        let score_by_id: std::collections::HashMap<&str, f32> =
            hits.iter().map(|(id, s)| (id.as_str(), *s)).collect();

        // 5. 按选中类型分组
        let mut grouped: std::collections::HashMap<MemoryType, Vec<Memory>> =
            std::collections::HashMap::new();
        for mem in memories {
            if selected_types.contains(&mem.memory_type) {
                grouped.entry(mem.memory_type).or_default().push(mem);
            }
        }

        // 6. 每组按向量分数排序，取前 PER_TYPE_LIMIT 条
        for mems in grouped.values_mut() {
            mems.sort_by(|a, b| {
                let sa = score_by_id.get(a.id.as_str()).copied().unwrap_or(0.0);
                let sb = score_by_id.get(b.id.as_str()).copied().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
            mems.truncate(PER_TYPE_LIMIT);
        }

        // 7. 在 token 预算内组装文本
        let mut text = String::new();
        let mut tokens_used = 0;
        let mut type_counts = Vec::new();
        let mut memory_ids = Vec::new();
        // T-E-S-64: 收集被引用记忆的精简视图,供 consistency::analyze。
        let mut cited_memories: Vec<CitedMemory> = Vec::new();

        for mt in &selected_types {
            if let Some(mems) = grouped.get(mt) {
                let mut count = 0;
                for mem in mems {
                    let line = format!("[{}] {}\n", mt, mem.content);
                    let line_tokens = estimate_tokens(&line);
                    if tokens_used + line_tokens > CONTEXT_TOKEN_BUDGET {
                        break;
                    }
                    text.push_str(&line);
                    tokens_used += line_tokens;
                    memory_ids.push(mem.id.clone());
                    // T-E-S-64: 从 mem.metadata.provenance 提取
                    // source/tool/content_hash,snippet 取 content 前 80 字符。
                    cited_memories.push(extract_cited_memory(mem));
                    // 回填 L0 热缓存
                    self.l0.insert(mem.clone());
                    count += 1;
                }
                if count > 0 {
                    type_counts.push((*mt, count));
                }
            }
        }

        Ok(ContextBundle {
            text,
            type_counts,
            tokens_used,
            memory_ids,
            cited_memories,
        })
    }

    /// 快速路径：仅从 L0 会话上下文取记忆（不查向量库）。
    #[instrument(target = "nebula.memory", skip(self), fields(otel.kind = "memory"))]
    pub fn session_context(&self, mt: Option<MemoryType>) -> Vec<Arc<Memory>> {
        match mt {
            Some(t) => self.l0.session_by_type(t),
            None => self.l0.session_snapshot(),
        }
    }
}

/// T-E-S-21: 按 ACL 过滤记忆列表。
///
/// 未注入 ACL 时直接返回原列表（内部可信路径向后兼容）。
/// 注入 ACL 时对每条记忆调用 `check(principal, id, Read)`，
/// 拒绝的记忆不保留并记录 warn 日志（不含内容，仅 id + principal，
/// 便于安全审计）。`acl.check()` 内部会上报 metrics（acl_deny 计数器）。
fn apply_acl_filter(
    acl: Option<&MemoryAcl>,
    memories: Vec<Memory>,
    principal: &str,
) -> Vec<Memory> {
    let Some(acl) = acl else {
        return memories;
    };
    memories
        .into_iter()
        .filter(|m| {
            let allowed = acl.check(principal, &m.id, AclPermission::Read);
            if !allowed {
                warn!(
                    target: "nebula.memory.acl",
                    principal = principal,
                    memory_id = %m.id,
                    "assemble_context denied memory by ACL"
                );
            }
            allowed
        })
        .collect()
}

/// T-E-S-64: 从 `Memory.metadata.provenance` 提取 `CitedMemory`。
///
/// - `source` 优先取 `provenance.source`,缺失时回退到 `Memory.source.as_str()`。
/// - `tool` 取 `provenance.tool`(可为 None)。
/// - `content_hash` 取 `provenance.content_hash`(可为 None)。
/// - `snippet` 取 `Memory.content` 前 80 字符(按 char 截断,不破坏 UTF-8)。
fn extract_cited_memory(mem: &Memory) -> CitedMemory {
    let provenance = mem.metadata.get("provenance");
    let source = provenance
        .and_then(|p| p.get("source"))
        .and_then(|s| s.as_str())
        .unwrap_or_else(|| mem.source.as_str())
        .to_string();
    let tool = provenance
        .and_then(|p| p.get("tool"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let content_hash = provenance
        .and_then(|p| p.get("content_hash"))
        .and_then(|h| h.as_str())
        .map(|s| s.to_string());
    let snippet: String = mem.content.chars().take(80).collect();
    CitedMemory {
        id: mem.id.clone(),
        source,
        tool,
        content_hash,
        snippet,
    }
}

/// 根据任务提示选择记忆类型组合（最多 3 种）。
fn select_memory_types(hint: &TaskHint) -> Vec<MemoryType> {
    use MemoryType::*;
    let mut types = vec![Semantic, Episodic]; // 默认组合
    if hint.is_writing {
        types.push(Procedural);
    }
    if hint.involves_preference {
        types.push(Emotional);
    }
    if hint.is_reflection {
        types.push(Metacognitive);
    }
    // 设计规定最多 3 种
    types.truncate(3);
    types
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::acl::{AclEffect, AclRule};
    use crate::memory::types::{MemoryLayer, SourceKind};

    #[test]
    fn select_types_default() {
        let hint = TaskHint::default();
        let types = select_memory_types(&hint);
        assert_eq!(types, vec![MemoryType::Semantic, MemoryType::Episodic]);
    }

    #[test]
    fn select_types_writing_adds_procedural() {
        let hint = TaskHint {
            is_writing: true,
            ..Default::default()
        };
        let types = select_memory_types(&hint);
        assert_eq!(types.len(), 3);
        assert!(types.contains(&MemoryType::Procedural));
    }

    #[test]
    fn select_types_max_three() {
        let hint = TaskHint {
            is_writing: true,
            involves_preference: true,
            is_reflection: true,
        };
        let types = select_memory_types(&hint);
        assert!(types.len() <= 3, "max 3 types, got {}", types.len());
    }

    #[test]
    fn task_hint_infers_writing() {
        let hint = TaskHint::infer("帮我写一份周报");
        assert!(hint.is_writing);
    }

    #[test]
    fn task_hint_infers_preference() {
        let hint = TaskHint::infer("我喜欢简洁的风格");
        assert!(hint.involves_preference);
    }

    #[test]
    fn task_hint_infers_reflection() {
        let hint = TaskHint::infer("反思一下这次失败的原因");
        assert!(hint.is_reflection);
    }

    #[test]
    fn empty_bundle() {
        let b = ContextBundle::empty();
        assert!(b.text.is_empty());
        assert_eq!(b.tokens_used, 0);
    }

    fn make_mem(id: &str, content: &str, mt: MemoryType) -> Memory {
        let mut m = Memory::new(mt, MemoryLayer::L1, content.to_string(), SourceKind::UserInput);
        m.id = id.to_string();
        m
    }

    // ---- T-E-S-21: ACL 过滤单元测试 ----

    /// T-E-S-21: `apply_acl_filter` 在未注入 ACL 时应放行所有记忆
    /// （内部可信路径向后兼容）。
    #[test]
    fn acl_filter_passthrough_when_no_acl() {
        let mems = vec![
            make_mem("m1", "alpha", MemoryType::Semantic),
            make_mem("m2", "beta", MemoryType::Episodic),
        ];
        let filtered = apply_acl_filter(None, mems, "external-user");
        assert_eq!(filtered.len(), 2, "no ACL → all memories pass through");
    }

    /// T-E-S-21: 注入 ACL 后，被 deny 规则匹配的记忆应被过滤掉，
    /// 其余记忆保留。验证未授权内容不进入 prompt。
    #[test]
    fn acl_filter_removes_denied_memories() {
        let mut acl = MemoryAcl::new();
        // 拒绝 user-1 对 mem-secret 的读访问
        acl.add_rule(AclRule {
            principal: "user-1".into(),
            resource: "mem-secret".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        // 拒绝所有主体对 mem-leak 的读访问（通配符 principal）
        acl.add_rule(AclRule {
            principal: "*".into(),
            resource: "mem-leak".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        // M7b #90 分类 D: deny-all 默认策略下,非可信主体 user-1 对 mem-public
        // 无匹配规则会被默认拒绝。补显式 Allow 规则,使 mem-public 通过过滤,
        // 与测试意图(过滤后剩 mem-public 一条)一致。
        acl.add_rule(AclRule {
            principal: "user-1".into(),
            resource: "mem-public".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Allow,
        });

        let mems = vec![
            make_mem("mem-secret", "secret content", MemoryType::Semantic),
            make_mem("mem-leak", "leaked content", MemoryType::Episodic),
            make_mem("mem-public", "public content", MemoryType::Semantic),
        ];

        let filtered = apply_acl_filter(Some(&acl), mems, "user-1");
        assert_eq!(
            filtered.len(),
            1,
            "denied memories must be filtered out"
        );
        assert_eq!(filtered[0].id, "mem-public");
        assert!(
            !filtered.iter().any(|m| m.id == "mem-secret"),
            "denied mem-secret must not appear in prompt context"
        );
        assert!(
            !filtered.iter().any(|m| m.id == "mem-leak"),
            "denied mem-leak must not appear in prompt context"
        );
    }

    /// T-E-S-21: 可信主体（system/owner/local）在无匹配规则时应默认放行，
    /// 不受 deny-all 默认策略影响。非可信主体在无匹配规则时被拒绝。
    #[test]
    fn acl_filter_trusted_principal_passes_unmatched() {
        let acl = MemoryAcl::new(); // 无规则

        let mems = vec![
            make_mem("m1", "alpha", MemoryType::Semantic),
            make_mem("m2", "beta", MemoryType::Episodic),
        ];

        // 可信主体 "system" → 默认 allow
        let filtered = apply_acl_filter(Some(&acl), mems.clone(), "system");
        assert_eq!(
            filtered.len(),
            2,
            "trusted principal 'system' should pass all memories"
        );

        // 非可信主体 "external" → 默认 deny-all
        let filtered = apply_acl_filter(Some(&acl), mems, "external");
        assert_eq!(
            filtered.len(),
            0,
            "untrusted principal should be denied all memories by default"
        );
    }

    /// T-E-S-21: `with_acl()` builder 应将 ACL 注入 MemoryOrchestrator，
    /// 且 `acl()` 访问器返回同一引用。不调用 `with_acl()` 时 `acl()`
    /// 应返回 `None`。
    #[test]
    fn acl_builder_injects_and_accessor_returns_reference() {
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "skill-1".into(),
            resource: "mem-secret".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        let acl_arc = Arc::new(acl);

        // 验证 check() 语义（与 sponge 测试一致）
        assert!(!acl_arc.check("skill-1", "mem-secret", AclPermission::Read));
        // M7b #90 分类 D: skill-1 非可信主体,对 mem-other 无匹配规则 → 默认 deny。
        // 原断言 `assert!(...)` 残留 allow-by-default 假设,与 M2b deny-all 矛盾。
        assert!(!acl_arc.check("skill-1", "mem-other", AclPermission::Read));
        assert!(acl_arc.check("system", "mem-secret", AclPermission::Read));
    }

    /// T-E-S-21: 空记忆列表 + ACL 过滤 → 仍为空（不 panic）。
    #[test]
    fn acl_filter_empty_input() {
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "*".into(),
            resource: "*".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        let filtered = apply_acl_filter(Some(&acl), Vec::new(), "anyone");
        assert!(filtered.is_empty());
    }
}
