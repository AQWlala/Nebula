//! Core memory engines — the active processing pipelines.
//!
//! * `blackhole` — compression engine (never deletes, only densifies)
//! * `sponge` — absorption engine (de-duplicate, normalise, link)
//! * `orchestrator` — 5-type memory coordination + context assembly
//! * `reflect` — L5 meta-cognitive reflection
//! * `self_reflection` — v2.0 self-reflection upgrade
//! * `forgetting` — forgetting engine (tick-based candidate selection)
//! * `importance` — importance scorer (frequency + recency + feedback)
//! * `summarizer` — LLM-driven multi-granularity summarization

pub mod blackhole;
pub mod forgetting;
pub mod importance;
pub mod orchestrator;
pub mod reflect;
pub mod self_reflection;
pub mod sponge;
pub mod summarizer;
// T-E-B-15: AI 自动整理 MOC（Map of Content）层次结构。
pub mod moc;

// T-D-B-04: Re-export parent items so child files can use `super::X`
// to reference memory-level modules after the directory restructure.
use super::acl;
use super::constants;
use super::document_extractor;
use super::embedder;
use super::entity_extractor;
use super::graph_search;
use super::hybrid_search;
use super::layers;
use super::sqlite_store;
use super::types;
use super::vector_store;
use super::ReflectConfig;
