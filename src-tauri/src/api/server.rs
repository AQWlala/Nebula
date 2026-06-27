//! Service trait that any transport (Tauri, HTTP, MCP) can implement.
//!
//! v0.1 only exposes this trait through Tauri commands. The following
//! surfaces are intentionally deferred:
//!
//! - TODO(v0.5): gRPC / HTTP transport — see design doc §13
//! - TODO(v0.5): Skill CRUD API (`skills` table is in the schema but
//!   unused at the command layer)
//! - TODO(v0.5): Reflection write API (`reflections` table exists but
//!   has no command)
//! - TODO(v0.5): Memory branch / import-export / backup API
//! - TODO(v0.5): Direct L5/L6/L7 write APIs (v0.1 demonstrates L0–L4)

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::llm::ChatResponse;
use crate::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use crate::swarm::{OrchestrationReport, SwarmTask};

/// Input for [`NineSnakeService::memory_store`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMemoryRequest {
    pub content: String,
    pub memory_type: MemoryType,
    pub layer: MemoryLayer,
    pub source: SourceKind,
    pub metadata: Option<serde_json::Value>,
}

/// Output of [`NineSnakeService::memory_store`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMemoryResponse {
    pub id: String,
    pub merged: bool,
    pub similarity: Option<f32>,
}

/// Input for [`NineSnakeService::memory_search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMemoryRequest {
    pub query: String,
    pub k: usize,
    pub layer: Option<MemoryLayer>,
}

/// Output of [`NineSnakeService::memory_search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMemoryHit {
    pub memory: Memory,
    pub score: f32,
}

/// Input for [`NineSnakeService::chat`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequestDto {
    pub user_message: String,
    pub system: Option<String>,
    pub temperature: Option<f32>,
}

/// Service trait implemented by [`crate::AppState`].
#[async_trait]
pub trait NineSnakeService: Send + Sync {
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
