//! Graph & relations — multi-dimensional relationship modeling.
//!
//! * `causal_graph` — causal inference graph (v1.5)
//! * `mdrm_graph` — MDRM 5-dim relation graph (causal/temporal/entity/hierarchical/similarity)
//! * `consistency` — anti-hallucination consistency checker

pub mod causal_graph;
pub mod consistency;
pub mod mdrm_graph;

// T-D-B-04: Re-export parent items so child files can use `super::X`
// to reference memory-level modules after the directory restructure.
use super::sqlite_store;
use super::types;
