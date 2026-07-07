//! `nebula::memory` — v7.0 layered memory system.
//!
//! The memory subsystem is the heart of nebula. It provides:
//!
//! * a strongly typed [`Memory`] value object with five cognitive
//!   [`MemoryType`]s and eight [`MemoryLayer`]s (L0..L7),
//! * a SQLite-backed structured store ([`sqlite_store`]),
//! * a LanceDB-backed dense vector store ([`lance_store`]),
//! * an embedding client ([`embedder`]) targeting BGE-small-zh-v1.5 via
//!   the local Ollama HTTP endpoint,
//! * an [`importance`] scorer that combines access frequency, recency and
//!   explicit user feedback,
//! * a [`blackhole`] compression engine that *never* deletes memories
//!   (it only densifies them) and respects the L7 singularity layer,
//! * a [`sponge`] absorption engine that de-duplicates, normalises and
//!   links incoming memories before they reach the hot path, and
//! * a [`reflect`] meta-cognitive engine (L5) that periodically
//!   summarises recent high-importance memories into reflections.
//!
//! The [`types`] module is the canonical source of truth for the data
//! model; every other module re-exports the relevant types from there.

pub mod acl;
// T-E-S-28: 对话消息标注(good/bad)+ 反馈回流 + Dify 风格数据集导出。
pub mod annotations;
pub mod blackhole;
// T-E-B-11: BM25 关键词搜索（基于 FTS5）。
pub mod bm25;
// T-E-B-14: Dataview 式查询 DSL（手写递归下降 parser + AST → SQL 翻译）。
pub mod query_dsl;
// T-E-S-64: 反幻觉一致性检查器（badge 数据来源）。
pub mod consistency;
// v1.5: 因果图谱推理引擎。
pub mod causal_graph;
// T-E-B-16: MDRM 5 维关系图谱(因果/时序/实体/层级/相似度)。
pub mod mdrm_graph;
// T-S6-B-01: CLIP 多模态嵌入(图片向量化)。
pub mod clip_embedder;
// T-E-B-12: PDF/DOCX 文档文本提取。
pub mod document_extractor;
pub mod embedder;
// T-E-B-09: 文件夹监控索引(自动吸收文件到记忆系统)。
pub mod entity_extractor;
pub mod export;
pub mod file_watcher;
pub mod forgetting;
pub mod graph_search;
pub mod importance;
// v1.4: L0 缓存层（LRU + Session Context + Pre-fetch）。
pub mod l0_cache;
pub mod lance_store;
// v1.4: Memory Orchestrator（5 类记忆协调 + 上下文组装）。
pub mod layers;
pub mod migration;
pub mod orchestrator;
pub mod reflect;
pub mod sponge;
// v2.0: 真正的 Self-Reflection — L5 元认知层升级。
pub mod self_reflection;
pub mod sqlite_store;
// T-E-S-43: SQLite 明文↔密文迁移(CipherMigrator)— feature-gated 整文件。
#[cfg(feature = "sqlcipher")]
pub mod sqlite_cipher;
// v1.5: LLM 驱动的多粒度摘要生成。
pub mod summarizer;
pub mod types;
// v1.6: Git 风格记忆版本控制（branch/commit/log/diff/revert/merge）。
pub mod version_control;
// v1.3: L4 价值层（Constitutional AI + Risk Assessor + Privacy Guard + Value Predictor）。
pub mod values;
// T-E-B-11: BM25 + 向量混合搜索。
pub mod hybrid_search;
// T-E-S-42: VectorStore trait 抽象（LanceDB / Qdrant / ChromaDB）。
pub mod vector_store;

pub use acl::{AclEffect, AclPermission, AclRule, MemoryAcl, PrincipalDomainMap};
// T-E-S-28: 对话标注 + Dify 数据集导出。
pub use annotations::{Annotation, AnnotationStats, AnnotationStore};
pub use blackhole::BlackholeEngine;
pub use bm25::Bm25Searcher;
// T-E-B-14: Dataview 式查询 DSL 顶层类型透传。
pub use query_dsl::{
    translate as translate_dsl, Expr as DslExpr, Field as DslField, LayerSpec, QueryAst,
};
// T-E-S-64: 反幻觉一致性检查器类型透传。
pub use causal_graph::{CausalChain, CausalGraphConfig, CausalGraphEngine, CausalNode};
pub use consistency::{analyze, CitedMemory, ConsistencyReport, ConsistencyWarning};
// T-E-B-16: MDRM 5 维关系图谱类型透传。
pub use mdrm_graph::{
    dimension_of, GraphEdge, GraphNode, GraphNodeRole, GraphSnapshot, MdrmConfig, MdrmEngine,
    RelationDimension,
};
// T-S6-B-01: 多模态嵌入抽象与 CLIP 实现。
pub use clip_embedder::ClipEmbedder;
pub use embedder::{create_embedder, Embedder, EmbedderKind, EmbedderTrait};
pub use entity_extractor::{EntityExtractor, ExtractedRelation};
pub use export::{DataExporter, ExportManifest, ImportResult, RelationEntity};
pub use forgetting::{ForgettingCandidate, ForgettingConfig, ForgettingEngine, TickResult};
pub use graph_search::{GraphSearchConfig, GraphSearchEngine, GraphSearchResult};
pub use hybrid_search::{HybridSearchConfig, HybridSearcher};
pub use importance::ImportanceScorer;
pub use l0_cache::{L0Cache, L0Stats};
pub use lance_store::LanceStore;
pub use layers::LayerPolicy;
pub use migration::{Migration, MigrationState, MigrationStatus};
pub use orchestrator::{ContextBundle, MemoryOrchestrator, TaskHint};
pub use reflect::{ReflectConfig, Reflection, ReflectionEngine, RoundGuard};
pub use sponge::SpongeEngine;
pub use sqlite_store::SqliteStore;
pub use summarizer::SummaryEngine;
pub use types::{Memory, MemoryLayer, MemoryType, MultiGranularity, RelationKind, SourceKind};
pub use values::{ValuesLayer, Verdict};
pub use version_control::{CommitDiff, CommitRecord, MemoryBranch, MemoryVersionControl};
// T-E-S-42: VectorStore trait 抽象(LanceDB/Qdrant/ChromaDB)。
pub use vector_store::{create_vector_store, VectorStore, VectorStoreBackend};

/// Constants shared across the memory subsystem.
pub mod constants {
    /// Default cosine-similarity threshold above which the sponge considers
    /// two memories "the same" and merges them.
    pub const SPONGE_MERGE_THRESHOLD: f32 = 0.85;

    /// Hard lower bound on memory importance. Below this value (and after
    /// the configured inactivity threshold) the black-hole may compress.
    pub const BLACKHOLE_IMPORTANCE_FLOOR: f32 = 0.10;

    /// Multi-granularity summary length buckets (characters).
    pub const SUMMARY_BUCKETS: [usize; 4] = [50, 150, 500, 2000];
}
