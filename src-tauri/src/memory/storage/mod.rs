//! Storage layer — persistent backends for the memory system.
//!
//! * `sqlite_store` — SQLite structured store (L1/L2/L3 records)
//! * `lance_store` — LanceDB dense vector store
//! * `sqlite_cipher` — SQLCipher encryption migration (feature-gated)
//! * `migration` — schema migration framework
//! * `l0_cache` — L0 LRU hot cache (session context + pre-fetch)
//! * `version_control` — Git-style memory versioning (branch/commit/diff/revert)

pub mod l0_cache;
pub mod lance_store;
pub mod migration;
#[cfg(feature = "sqlcipher")]
pub mod sqlite_cipher;
pub mod sqlite_store;
pub mod version_control;

// T-D-B-04: Re-export parent items so child files can use `super::X`
// to reference memory-level modules after the directory restructure.
use super::embedder;
use super::query_dsl;
use super::types;
