//! Service trait that any transport (Tauri, HTTP, MCP) can implement.
//!
//! v1.1+ exposes this trait through Tauri commands and gRPC JSON framing.
//! - Skill CRUD API: implemented (v0.3+)
//! - Reflection API: implemented (v0.2+)
//! - Memory export/import: implemented (v1.3+)
//! - L5–L7 write APIs: implemented (v1.1+, with L7 guard)

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::llm::ChatResponse;
use crate::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use crate::swarm::{OrchestrationReport, SwarmTask};

/// Input for [`NebulaService::memory_store`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMemoryRequest {
    pub content: String,
    pub memory_type: MemoryType,
    pub layer: MemoryLayer,
    pub source: SourceKind,
    pub metadata: Option<serde_json::Value>,
}

/// Output of [`NebulaService::memory_store`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMemoryResponse {
    pub id: String,
    pub merged: bool,
    pub similarity: Option<f32>,
}

/// Input for [`NebulaService::memory_search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMemoryRequest {
    pub query: String,
    pub k: usize,
    pub layer: Option<MemoryLayer>,
}

/// Output of [`NebulaService::memory_search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMemoryHit {
    pub memory: Memory,
    pub score: f32,
}

/// Input for [`NebulaService::chat`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequestDto {
    pub user_message: String,
    pub system: Option<String>,
    pub temperature: Option<f32>,
}

/// Service trait implemented by [`crate::AppState`].
#[async_trait]
pub trait NebulaService: Send + Sync {
    /// One-shot chat completion (no memory lookup).
    async fn chat(&self, req: ChatRequestDto) -> Result<ChatResponse>;

    /// Store a memory via the sponge.
    async fn memory_store(&self, req: StoreMemoryRequest) -> Result<StoreMemoryResponse>;

    /// Vector search over the memory store.
    async fn memory_search(&self, req: SearchMemoryRequest) -> Result<Vec<SearchMemoryHit>>;

    /// Run a swarm task.
    async fn swarm_execute(&self, task: SwarmTask) -> Result<OrchestrationReport>;

    /// Raw LLM completion.
    async fn llm_complete(&self, prompt: String) -> Result<String>;
}
