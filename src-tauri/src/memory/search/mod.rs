//! Search & retrieval — query interfaces for the memory system.
//!
//! * `bm25` — BM25 keyword search (FTS5)
//! * `hybrid_search` — BM25 + vector hybrid search
//! * `graph_search` — graph traversal search engine
//! * `query_dsl` — Dataview-style query DSL (recursive descent parser + AST → SQL)

pub mod bm25;
pub mod graph_search;
pub mod hybrid_search;
pub mod query_dsl;

// T-D-B-04: Re-export parent items so child files can use `super::X`
// to reference memory-level modules after the directory restructure.
use super::embedder;
use super::sqlite_store;
use super::types;
use super::vector_store;
