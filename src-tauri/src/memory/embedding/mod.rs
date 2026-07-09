//! Embedding & extraction — vectorization and content parsing.
//!
//! * `embedder` — BGE-small-zh-v1.5 embedding client (Ollama HTTP)
//! * `clip_embedder` — CLIP multimodal embedding (image vectorization)
//! * `entity_extractor` — entity/relation extraction from text
//! * `document_extractor` — PDF/DOCX text extraction

pub mod clip_embedder;
pub mod document_extractor;
pub mod embedder;
pub mod entity_extractor;

// T-D-B-04: Re-export parent items so child files can use `super::X`
// to reference memory-level modules after the directory restructure.
use super::types;
