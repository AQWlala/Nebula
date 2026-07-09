//! Data I/O — import, export, and external ingestion.
//!
//! * `annotations` — conversation annotations (good/bad) + Dify dataset export
//! * `export` — data exporter/importer (JSON/CSV)
//! * `file_watcher` — folder monitoring + auto-absorb to L3

pub mod annotations;
pub mod export;
pub mod file_watcher;

// T-D-B-04: Re-export parent items so child files can use `super::X`
// to reference memory-level modules after the directory restructure.
use super::sponge;
use super::sqlite_store;
use super::types;
