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
//!
//! ## Module layout (T-D-B-04)
//!
//! Files are grouped by function into subdirectories:
//!
//! | Group | Purpose |
//! |-------|---------|
//! | [`storage`] | Persistent backends (SQLite, LanceDB, migration, cache, VCS) |
//! | [`engines`] | Core processing pipelines (blackhole, sponge, orchestrator, reflect…) |
//! | [`search`] | Query interfaces (BM25, hybrid, graph, DSL) |
//! | [`embedding`] | Vectorization & content extraction |
//! | [`graph`] | Multi-dimensional relationship modeling (causal, MDRM, consistency) |
//! | [`io`] | Import, export, external ingestion |
//! | [`values`] | L4 value layer (Constitutional AI + Risk + Privacy) |
//! | [`vector_store`] | VectorStore trait abstraction (Lance/Qdrant/Chroma) |
//!
//! Root-level modules: [`types`], [`acl`], [`layers`].
//!
//! Backward compatibility: each moved module is re-exported via `pub use`
//! so `crate::memory::sqlite_store` still resolves alongside the new
//! `crate::memory::storage::sqlite_store` path.

// ---- Root-level modules (too fundamental to move) ----
pub mod acl;
pub mod layers;
pub mod types;

// ---- Grouped subdirectories (T-D-B-04) ----
pub mod embedding;
pub mod engines;
pub mod graph;
#[allow(clippy::module_inception)]
pub mod io;
pub mod search;
pub mod storage;

// Existing subdirectories
pub mod values;
pub mod vector_store;

// ---- Backward-compat module re-exports ----
// These allow existing `crate::memory::MODULE` paths to continue resolving
// after the physical file moves. New code should prefer the grouped path
// (e.g. `crate::memory::storage::sqlite_store`).

// storage/
pub use storage::l0_cache;
pub use storage::lance_store;
pub use storage::migration;
#[cfg(feature = "sqlcipher")]
pub use storage::sqlite_cipher;
pub use storage::sqlite_store;
pub use storage::version_control;

// engines/
pub use engines::blackhole;
pub use engines::forgetting;
pub use engines::importance;
pub use engines::orchestrator;
pub use engines::reflect;
pub use engines::self_reflection;
pub use engines::sponge;
pub use engines::summarizer;

// search/
pub use search::bm25;
pub use search::graph_search;
pub use search::hybrid_search;
pub use search::query_dsl;

// embedding/
pub use embedding::clip_embedder;
pub use embedding::document_extractor;
pub use embedding::embedder;
pub use embedding::entity_extractor;

// graph/
pub use graph::causal_graph;
pub use graph::consistency;
pub use graph::mdrm_graph;

// io/
pub use io::annotations;
pub use io::export;
pub use io::file_watcher;

// ---- Type re-exports (unchanged, resolved via module re-exports above) ----
pub use acl::{AclEffect, AclPermission, AclRule, MemoryAcl, PrincipalDomainMap};
pub use annotations::{Annotation, AnnotationStats, AnnotationStore};
pub use blackhole::BlackholeEngine;
pub use bm25::Bm25Searcher;
pub use causal_graph::{CausalChain, CausalGraphConfig, CausalGraphEngine, CausalNode};
pub use clip_embedder::ClipEmbedder;
pub use consistency::{analyze, CitedMemory, ConsistencyReport, ConsistencyWarning};
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
pub use mdrm_graph::{
    dimension_of, GraphEdge, GraphNode, GraphNodeRole, GraphSnapshot, MdrmConfig, MdrmEngine,
    RelationDimension,
};
pub use migration::{Migration, MigrationState, MigrationStatus};
pub use orchestrator::{ContextBundle, MemoryOrchestrator, TaskHint};
pub use query_dsl::{
    translate as translate_dsl, Expr as DslExpr, Field as DslField, LayerSpec, QueryAst,
};
pub use reflect::{ReflectConfig, Reflection, ReflectionEngine, RoundGuard};
pub use sponge::SpongeEngine;
pub use sqlite_store::SqliteStore;
pub use summarizer::SummaryEngine;
pub use types::{Memory, MemoryLayer, MemoryType, MultiGranularity, RelationKind, SourceKind};
pub use values::{ValuesLayer, Verdict};
pub use vector_store::{create_vector_store, VectorStore, VectorStoreBackend};
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
