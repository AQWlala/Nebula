//! `nine_snake::memory` — v7.0 layered memory system.
//!
//! The memory subsystem is the heart of nine-snake. It provides:
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
pub mod blackhole;
// v1.5: 因果图谱推理引擎。
pub mod causal_graph;
pub mod embedder;
pub mod entity_extractor;
pub mod export;
pub mod forgetting;
pub mod graph_search;
pub mod importance;
// v1.4: L0 缓存层（LRU + Session Context + Pre-fetch）。
pub mod l0_cache;
pub mod lance_store;
// v1.4: Memory Orchestrator（5 类记忆协调 + 上下文组装）。
pub mod orchestrator;
pub mod layers;
pub mod migration;
pub mod reflect;
pub mod sponge;
// v2.0: 真正的 Self-Reflection — L5 元认知层升级。
pub mod self_reflection;
pub mod sqlite_store;
// v1.5: LLM 驱动的多粒度摘要生成。
pub mod summarizer;
pub mod types;
// v1.6: Git 风格记忆版本控制（branch/commit/log/diff/revert/merge）。
pub mod version_control;
// v1.3: L4 价值层（Constitutional AI + Risk Assessor + Privacy Guard + Value Predictor）。
pub mod values;

pub use acl::{AclEffect, AclPermission, AclRule, MemoryAcl};
pub use blackhole::BlackholeEngine;
pub use causal_graph::{CausalChain, CausalGraphConfig, CausalGraphEngine, CausalNode};
pub use embedder::Embedder;
pub use entity_extractor::{EntityExtractor, ExtractedRelation};
pub use export::{DataExporter, ExportManifest, ImportResult};
pub use forgetting::{ForgettingCandidate, ForgettingConfig, ForgettingEngine};
pub use graph_search::{GraphSearchConfig, GraphSearchEngine, GraphSearchResult};
pub use importance::ImportanceScorer;
pub use l0_cache::{L0Cache, L0Stats};
pub use lance_store::LanceStore;
pub use layers::LayerPolicy;
pub use orchestrator::{ContextBundle, MemoryOrchestrator, TaskHint};
pub use migration::{Migration, MigrationState, MigrationStatus};
pub use reflect::{ReflectConfig, Reflection, ReflectionEngine};
pub use sponge::SpongeEngine;
pub use sqlite_store::SqliteStore;
pub use summarizer::SummaryEngine;
pub use types::{Memory, MemoryLayer, MemoryType, MultiGranularity, RelationKind, SourceKind};
pub use values::{ValuesLayer, Verdict};
pub use version_control::{CommitDiff, CommitRecord, MemoryBranch, MemoryVersionControl};

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
