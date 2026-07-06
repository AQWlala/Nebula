//! Tonic-based gRPC server implementation for the nebula T-S2-B-01 task.
//!
//! This file implements the 5 tonic-generated server traits
//! (`MemoryService`, `SwarmService`, `ReflectService`, `LlmService`,
//! `SkillService`) on [`TonicServiceImpl`], delegating each RPC to the
//! underlying [`crate::AppState`].
//!
//! Unlike the hand-rolled JSON shim in `server.rs`, this module uses the
//! real prost-generated types from `proto/nebula.v1.rs` and the
//! tonic `Request`/`Response`/`Status` error model.

pub mod generated {
    include!("proto/nebula.v1.rs");
}

use async_trait::async_trait;
use tracing::{info, warn};

use crate::api::server::NebulaService as ApiNebulaService;
use crate::skills::types as skill_types;
use crate::AppState;

/// Tonic service implementation that wraps [`AppState`] and exposes
/// the 5 prost-generated server traits.
pub struct TonicServiceImpl {
    state: AppState,
}

impl TonicServiceImpl {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

// ===========================================================================
// MemoryService (8 RPCs)
// ===========================================================================

#[async_trait]
impl generated::memory_service_server::MemoryService for TonicServiceImpl {
    async fn store(
        &self,
        request: tonic::Request<generated::StoreMemoryRequest>,
    ) -> std::result::Result<tonic::Response<generated::StoreMemoryResponse>, tonic::Status>
    {
        let req = request.into_inner();
        let layer = layer_from_prost(req.layer);
        let memory_type = memory_type_from_prost(req.memory_type);
        let source = req
            .source
            .parse::<crate::memory::SourceKind>()
            .unwrap_or(crate::memory::SourceKind::UserInput);
        let command_req = crate::api::server::StoreMemoryRequest {
            content: req.content,
            memory_type,
            layer,
            source,
            metadata: if req.metadata_json.is_empty() {
                None
            } else {
                Some(serde_json::from_str(&req.metadata_json).map_err(|e| {
                    tonic::Status::internal(format!("metadata decode error: {e}"))
                })?)
            },
        };
        let resp = self
            .state
            .memory_store(command_req)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::StoreMemoryResponse {
            id: resp.id,
            merged: resp.merged,
            similarity: resp.similarity.unwrap_or(0.0),
        }))
    }

    async fn get(
        &self,
        request: tonic::Request<generated::GetMemoryRequest>,
    ) -> std::result::Result<tonic::Response<generated::Memory>, tonic::Status> {
        let req = request.into_inner();
        let m = self
            .state
            .sqlite
            .get(&req.id)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?
            .ok_or_else(|| tonic::Status::not_found("memory not found"))?;
        Ok(tonic::Response::new(memory_to_prost(m)))
    }

    async fn search(
        &self,
        request: tonic::Request<generated::SearchRequest>,
    ) -> std::result::Result<tonic::Response<generated::SearchResponse>, tonic::Status> {
        let req = request.into_inner();
        let command_req = crate::api::server::SearchMemoryRequest {
            query: req.query,
            k: if req.k == 0 { 10 } else { req.k as usize },
            layer: if req.layer == generated::MemoryLayer::Unspecified as i32 {
                None
            } else {
                Some(layer_from_prost(req.layer))
            },
        };
        let hits = self
            .state
            .memory_search(command_req)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::SearchResponse {
            hits: hits
                .into_iter()
                .map(|h| generated::SearchHit {
                    memory: Some(memory_to_prost(h.memory)),
                    score: h.score,
                })
                .collect(),
        }))
    }

    async fn list_recent(
        &self,
        request: tonic::Request<generated::ListRecentRequest>,
    ) -> std::result::Result<tonic::Response<generated::ListRecentResponse>, tonic::Status>
    {
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            20
        } else {
            req.limit as usize
        };
        let mems = self
            .state
            .sqlite
            .list_recent(limit)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::ListRecentResponse {
            memories: mems.into_iter().map(memory_to_prost).collect(),
        }))
    }

    async fn update_importance(
        &self,
        request: tonic::Request<generated::UpdateImportanceRequest>,
    ) -> std::result::Result<tonic::Response<generated::Memory>, tonic::Status> {
        let req = request.into_inner();
        let importance = req.importance.clamp(0.0, 1.0);
        let m = self
            .state
            .sqlite
            .update_importance(&req.id, importance)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(memory_to_prost(m)))
    }

    async fn delete(
        &self,
        request: tonic::Request<generated::DeleteRequest>,
    ) -> std::result::Result<tonic::Response<generated::DeleteResponse>, tonic::Status> {
        let req = request.into_inner();
        let deleted = self
            .state
            .sqlite
            .delete(&req.id)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        if deleted {
            if let Err(e) = self.state.lance.delete(&req.id).await {
                warn!(
                    target: "nebula.grpc",
                    error = ?e,
                    "lance delete failed"
                );
            }
        }
        Ok(tonic::Response::new(generated::DeleteResponse { deleted }))
    }

    async fn get_many(
        &self,
        request: tonic::Request<generated::GetManyRequest>,
    ) -> std::result::Result<tonic::Response<generated::GetManyResponse>, tonic::Status> {
        let req = request.into_inner();
        let mems = self
            .state
            .sqlite
            .get_many(&req.ids)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::GetManyResponse {
            memories: mems.into_iter().map(memory_to_prost).collect(),
        }))
    }

    async fn get_stats(
        &self,
        request: tonic::Request<generated::StatsRequest>,
    ) -> std::result::Result<tonic::Response<generated::StatsResponse>, tonic::Status> {
        let _req = request.into_inner();
        let rows = self
            .state
            .sqlite
            .counts_per_layer()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let total = rows.values().sum();
        Ok(tonic::Response::new(generated::StatsResponse {
            total_memories: total,
            by_layer_l0: rows
                .get(&crate::memory::MemoryLayer::L0)
                .copied()
                .unwrap_or(0),
            by_layer_l1: rows
                .get(&crate::memory::MemoryLayer::L1)
                .copied()
                .unwrap_or(0),
            by_layer_l2: rows
                .get(&crate::memory::MemoryLayer::L2)
                .copied()
                .unwrap_or(0),
            by_layer_l3: rows
                .get(&crate::memory::MemoryLayer::L3)
                .copied()
                .unwrap_or(0),
            by_layer_l4: rows
                .get(&crate::memory::MemoryLayer::L4)
                .copied()
                .unwrap_or(0),
            by_layer_l5: rows
                .get(&crate::memory::MemoryLayer::L5)
                .copied()
                .unwrap_or(0),
            by_layer_l6: rows
                .get(&crate::memory::MemoryLayer::L6)
                .copied()
                .unwrap_or(0),
            by_layer_l7: rows
                .get(&crate::memory::MemoryLayer::L7)
                .copied()
                .unwrap_or(0),
        }))
    }
}

// ===========================================================================
// SwarmService (4 RPCs)
// ===========================================================================

/// Server-streaming response type for `StreamEvents`.
pub type StreamEventsStream = std::pin::Pin<
    Box<
        dyn tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<generated::SwarmEvent, tonic::Status>,
            > + std::marker::Send
            + 'static,
    >,
>;

#[async_trait]
impl generated::swarm_service_server::SwarmService for TonicServiceImpl {
    type StreamEventsStream = StreamEventsStream;

    async fn execute(
        &self,
        request: tonic::Request<generated::SwarmRequest>,
    ) -> std::result::Result<tonic::Response<generated::SwarmResponse>, tonic::Status> {
        let req = request.into_inner();
        let agents: Vec<String> = req
            .pipeline
            .iter()
            .map(|k| agent_kind_from_prost(*k))
            .collect();
        let task = crate::swarm::SwarmTask {
            description: req.description,
            agent_count: if agents.is_empty() {
                3
            } else {
                agents.len().clamp(2, 6) as u32
            },
            max_retries: req.max_retries,
            agents,
        };
        let result = self
            .state
            .swarm_execute(task)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::SwarmResponse {
            approved: result.approved,
            verdict: String::new(),
            outputs: result
                .outputs
                .into_iter()
                .map(|o| generated::AgentOutput {
                    kind: swarm_agent_kind_to_prost(o.kind),
                    author: o.author,
                    body: o.body,
                    confidence: o.confidence,
                })
                .collect(),
        }))
    }

    async fn list_agents(
        &self,
        request: tonic::Request<generated::ListAgentsRequest>,
    ) -> std::result::Result<tonic::Response<generated::ListAgentsResponse>, tonic::Status>
    {
        let _req = request.into_inner();
        let agents = self.state.swarm.list_agents();
        Ok(tonic::Response::new(generated::ListAgentsResponse {
            agents: agents
                .into_iter()
                .map(|(kind, name, sys, desc)| generated::Agent {
                    kind: agent_kind_to_prost_from_str(&kind),
                    name,
                    system_prompt: sys,
                    description: desc,
                })
                .collect(),
        }))
    }

    async fn get_agent(
        &self,
        request: tonic::Request<generated::GetAgentRequest>,
    ) -> std::result::Result<tonic::Response<generated::Agent>, tonic::Status> {
        let req = request.into_inner();
        let kind_str = agent_kind_from_prost(req.kind);
        let agent = self
            .state
            .swarm
            .get_agent(&kind_str)
            .ok_or_else(|| tonic::Status::not_found(format!("agent {kind_str}")))?;
        Ok(tonic::Response::new(generated::Agent {
            kind: req.kind,
            name: agent.name,
            system_prompt: agent.system_prompt,
            description: agent.description,
        }))
    }

    async fn stream_events(
        &self,
        request: tonic::Request<generated::StreamEventsRequest>,
    ) -> std::result::Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        let req = request.into_inner();
        info!(
            target: "nebula.grpc",
            task_id = %req.task_id,
            "stream_events subscription opened"
        );
        let mut rx = self.state.swarm.bus().subscribe_events();

        let stream = async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        yield Ok(swarm_event_to_prost(evt));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        yield Ok(generated::SwarmEvent {
                            event_type: "lagged".to_string(),
                            agent: generated::AgentKind::Unspecified as i32,
                            body: format!("skipped {n} events"),
                            ts: chrono::Utc::now().timestamp(),
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        };
        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

// ===========================================================================
// ReflectService (3 RPCs)
// ===========================================================================

#[async_trait]
impl generated::reflect_service_server::ReflectService for TonicServiceImpl {
    async fn reflect_now(
        &self,
        request: tonic::Request<generated::ReflectRequest>,
    ) -> std::result::Result<tonic::Response<generated::ReflectResponse>, tonic::Status> {
        let _req = request.into_inner();
        let engine = self.state.reflection.clone();
        let rows = engine
            .reflect_now()
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::ReflectResponse {
            reflections: rows.into_iter().map(reflection_to_prost).collect(),
        }))
    }

    async fn list_reflections(
        &self,
        request: tonic::Request<generated::ListReflectionsRequest>,
    ) -> std::result::Result<tonic::Response<generated::ListReflectionsResponse>, tonic::Status>
    {
        let req = request.into_inner();
        let limit = if req.limit == 0 {
            20
        } else {
            req.limit as usize
        };
        let engine = self.state.reflection.clone();
        let rows = tokio::task::spawn_blocking(move || engine.list_recent(limit))
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::ListReflectionsResponse {
            reflections: rows.into_iter().map(reflection_to_prost).collect(),
        }))
    }

    async fn get_reflection(
        &self,
        request: tonic::Request<generated::GetReflectionRequest>,
    ) -> std::result::Result<tonic::Response<generated::Reflection>, tonic::Status> {
        let req = request.into_inner();
        let engine = self.state.reflection.clone();
        let id = req.id;
        let r = tokio::task::spawn_blocking(move || engine.get(&id))
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?
            .map_err(|e| tonic::Status::internal(e.to_string()))?
            .ok_or_else(|| tonic::Status::not_found("reflection not found"))?;
        Ok(tonic::Response::new(reflection_to_prost(r)))
    }
}

// ===========================================================================
// LlmService (3 RPCs)
// ===========================================================================

#[async_trait]
impl generated::llm_service_server::LlmService for TonicServiceImpl {
    async fn complete(
        &self,
        request: tonic::Request<generated::CompleteRequest>,
    ) -> std::result::Result<tonic::Response<generated::CompleteResponse>, tonic::Status> {
        let req = request.into_inner();
        let text = self
            .state
            .llm_complete(req.prompt)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::CompleteResponse {
            text,
            model: self.state.config.chat_model.clone(),
            eval_count: 0,
            total_duration_ns: 0,
        }))
    }

    async fn chat(
        &self,
        request: tonic::Request<generated::ChatRequest>,
    ) -> std::result::Result<tonic::Response<generated::ChatResponse>, tonic::Status> {
        let req = request.into_inner();
        let msgs: Vec<crate::llm::ChatMessage> = req
            .messages
            .into_iter()
            .map(|m| crate::llm::ChatMessage {
                role: m.role,
                content: m.content,
                ..Default::default()
            })
            .collect();
        let model = if req.model.is_empty() {
            None
        } else {
            Some(req.model)
        };
        let resp = if let Some(ref m) = model {
            self.state.llm.chat_with_model(m, msgs).await
        } else {
            self.state.llm.chat(msgs).await
        }
        .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::ChatResponse {
            message: Some(generated::ChatMessage {
                role: resp.message.role,
                content: resp.message.content,
            }),
            model: resp.model,
            eval_count: resp.eval_count.unwrap_or(0) as i64,
            total_duration_ns: resp.total_duration.unwrap_or(0) as i64,
        }))
    }

    async fn embed(
        &self,
        request: tonic::Request<generated::EmbedRequest>,
    ) -> std::result::Result<tonic::Response<generated::EmbedResponse>, tonic::Status> {
        let req = request.into_inner();
        let v = self
            .state
            .embedder
            .embed(&req.text)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let dim = v.len() as u32;
        Ok(tonic::Response::new(generated::EmbedResponse { vector: v, dim }))
    }
}

// ===========================================================================
// SkillService (5 RPCs)
// ===========================================================================

#[async_trait]
impl generated::skill_service_server::SkillService for TonicServiceImpl {
    async fn create(
        &self,
        request: tonic::Request<generated::CreateSkillRequest>,
    ) -> std::result::Result<tonic::Response<generated::Skill>, tonic::Status> {
        let req = request.into_inner();
        let r = self
            .state
            .skills
            .create_skill(skill_types::CreateSkillRequest {
                name: req.name,
                description: req.description,
                code: req.code,
                language: req.language,
                tags: req.tags,
                source_memory_id: if req.source_memory_id.is_empty() {
                    None
                } else {
                    Some(req.source_memory_id)
                },
                ..Default::default()
            })
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(skill_to_prost(r)))
    }

    async fn r#use(
        &self,
        request: tonic::Request<generated::UseSkillRequest>,
    ) -> std::result::Result<tonic::Response<generated::UseSkillResponse>, tonic::Status> {
        let req = request.into_inner();
        let r = self
            .state
            .skills
            .use_skill(skill_types::UseSkillRequest {
                id: req.id,
                params: req.params,
            })
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::UseSkillResponse {
            result: Some(generated::SkillResult {
                skill_id: r.skill_id,
                output: r.output,
                execution_time_ms: r.execution_time_ms,
                tokens_used: r.tokens_used,
            }),
        }))
    }

    async fn rate(
        &self,
        request: tonic::Request<generated::RateSkillRequest>,
    ) -> std::result::Result<tonic::Response<generated::Skill>, tonic::Status> {
        let req = request.into_inner();
        let r = self
            .state
            .skills
            .rate_skill(skill_types::RateSkillRequest {
                id: req.id,
                rating: req.rating,
            })
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(skill_to_prost(r)))
    }

    async fn list(
        &self,
        request: tonic::Request<generated::ListSkillsRequest>,
    ) -> std::result::Result<tonic::Response<generated::ListSkillsResponse>, tonic::Status> {
        let req = request.into_inner();
        let r = self
            .state
            .skills
            .list_skills(skill_types::ListSkillsRequest {
                language: if req.language.is_empty() {
                    None
                } else {
                    Some(req.language)
                },
                tag: if req.tag.is_empty() {
                    None
                } else {
                    Some(req.tag)
                },
                limit: if req.limit == 0 { 50 } else { req.limit },
                ..Default::default()
            })
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::ListSkillsResponse {
            skills: r.into_iter().map(skill_to_prost).collect(),
        }))
    }

    async fn search(
        &self,
        request: tonic::Request<generated::SearchSkillsRequest>,
    ) -> std::result::Result<tonic::Response<generated::SearchSkillsResponse>, tonic::Status>
    {
        let req = request.into_inner();
        let r = self
            .state
            .skills
            .search_skills(skill_types::SkillSearchRequest {
                query: req.query,
                limit: if req.limit == 0 { 50 } else { req.limit },
            })
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        Ok(tonic::Response::new(generated::SearchSkillsResponse {
            skills: r.into_iter().map(skill_to_prost).collect(),
        }))
    }
}

// ===========================================================================
// Conversion helpers
// ===========================================================================

/// Converts a Rust [`crate::memory::Memory`] into the prost [`generated::Memory`].
fn memory_to_prost(m: crate::memory::Memory) -> generated::Memory {
    generated::Memory {
        id: m.id,
        memory_type: memory_type_to_prost(m.memory_type),
        layer: layer_to_prost(m.layer),
        content: m.content,
        summary_50: m.summary.s50,
        summary_150: m.summary.s150,
        summary_500: m.summary.s500,
        summary_2000: m.summary.s2000,
        importance: m.importance,
        access_count: m.access_count,
        last_access: m.last_access,
        created_at: m.created_at,
        source: m.source.as_str().to_string(),
        metadata_json: m.metadata.to_string(),
        compressed_from: m.compressed_from.unwrap_or_default(),
        compression_gen: m.compression_gen,
        pinned: m.pinned,
    }
}

/// Converts a Rust [`crate::memory::MemoryLayer`] to the prost `i32` enum value.
fn layer_to_prost(l: crate::memory::MemoryLayer) -> i32 {
    use crate::memory::MemoryLayer as L;
    match l {
        L::L0 => generated::MemoryLayer::L0 as i32,
        L::L1 => generated::MemoryLayer::L1 as i32,
        L::L2 => generated::MemoryLayer::L2 as i32,
        L::L3 => generated::MemoryLayer::L3 as i32,
        L::L4 => generated::MemoryLayer::L4 as i32,
        L::L5 => generated::MemoryLayer::L5 as i32,
        L::L6 => generated::MemoryLayer::L6 as i32,
        L::L7 => generated::MemoryLayer::L7 as i32,
    }
}

/// Converts a Rust [`crate::memory::MemoryType`] to the prost `i32` enum value.
fn memory_type_to_prost(t: crate::memory::MemoryType) -> i32 {
    use crate::memory::MemoryType as T;
    match t {
        T::Semantic => generated::MemoryType::Semantic as i32,
        T::Episodic => generated::MemoryType::Episodic as i32,
        T::Procedural => generated::MemoryType::Procedural as i32,
        T::Emotional => generated::MemoryType::Emotional as i32,
        T::Metacognitive => generated::MemoryType::Metacognitive as i32,
    }
}

/// Converts a prost `MemoryLayer` `i32` back to the Rust enum.
/// `Unspecified` (0) and any unknown value fall back to `L1`.
fn layer_from_prost(v: i32) -> crate::memory::MemoryLayer {
    use crate::memory::MemoryLayer as L;
    match v {
        1 => L::L0,
        2 => L::L1,
        3 => L::L2,
        4 => L::L3,
        5 => L::L4,
        6 => L::L5,
        7 => L::L6,
        8 => L::L7,
        // 0 (Unspecified) and any out-of-range value default to L1,
        // matching the existing hand-rolled server behavior.
        _ => L::L1,
    }
}

/// Converts a prost `MemoryType` `i32` back to the Rust enum.
/// `Unspecified` (0) and any unknown value fall back to `Semantic`.
fn memory_type_from_prost(v: i32) -> crate::memory::MemoryType {
    use crate::memory::MemoryType as T;
    match v {
        1 => T::Semantic,
        2 => T::Episodic,
        3 => T::Procedural,
        4 => T::Emotional,
        5 => T::Metacognitive,
        _ => T::Semantic,
    }
}

/// Converts a Rust [`crate::memory::Reflection`] into the prost [`generated::Reflection`].
fn reflection_to_prost(r: crate::memory::Reflection) -> generated::Reflection {
    generated::Reflection {
        id: r.id,
        source_memories: r.source_memories,
        content: r.content,
        layer: layer_to_prost(r.layer),
        memory_type: memory_type_to_prost(r.memory_type),
        importance: r.importance,
        trigger_kind: r.trigger_kind,
        lessons: r.lessons,
        confidence: r.confidence,
        created_at: r.created_at,
    }
}

/// Converts a Rust [`crate::skills::types::Skill`] into the prost [`generated::Skill`].
fn skill_to_prost(s: crate::skills::types::Skill) -> generated::Skill {
    generated::Skill {
        id: s.id,
        name: s.name,
        description: s.description,
        code: s.code,
        language: s.language,
        tags: s.tags,
        usage_count: s.usage_count,
        avg_rating: s.avg_rating,
        rating_count: s.rating_count,
        created_at: s.created_at,
        updated_at: s.updated_at,
        source_memory_id: s.source_memory_id.unwrap_or_default(),
    }
}

/// Converts a prost `AgentKind` `i32` into the swarm's string-based
/// agent kind identifier (e.g. `"coder"`, `"writer"`, `"reviewer"`).
/// `Unspecified` (0) and any unknown value map to `"unspecified"`.
fn agent_kind_from_prost(v: i32) -> String {
    match v {
        1 => "coder".to_string(),
        2 => "writer".to_string(),
        3 => "reviewer".to_string(),
        _ => "unspecified".to_string(),
    }
}

/// Converts a swarm string-based agent kind into the prost `AgentKind` `i32`.
/// Unknown strings map to `Unspecified` (0).
fn agent_kind_to_prost_from_str(s: &str) -> i32 {
    match s {
        "coder" => generated::AgentKind::Coder as i32,
        "writer" => generated::AgentKind::Writer as i32,
        "reviewer" => generated::AgentKind::Reviewer as i32,
        _ => generated::AgentKind::Unspecified as i32,
    }
}

/// Converts a Rust [`crate::swarm::AgentKind`] into the prost `AgentKind` `i32`.
/// `Generic`, `Researcher`, and `Planner` have no prost equivalent and
/// map to `Unspecified` (0).
fn swarm_agent_kind_to_prost(kind: crate::swarm::AgentKind) -> i32 {
    use crate::swarm::AgentKind as K;
    match kind {
        K::Coder => generated::AgentKind::Coder as i32,
        K::Writer => generated::AgentKind::Writer as i32,
        K::Reviewer => generated::AgentKind::Reviewer as i32,
        // Generic, Researcher, Planner — no prost equivalent.
        _ => generated::AgentKind::Unspecified as i32,
    }
}

/// Converts a structured [`crate::swarm::events::SwarmEvent`] enum into
/// the flat prost [`generated::SwarmEvent`] struct.
///
/// The `event_type` field is the snake_case variant name; the `body`
/// field is a JSON object carrying the variant's payload; the `agent`
/// field is the prost `AgentKind` for variants that carry an agent
/// (or `Unspecified` otherwise); the `ts` field is the variant's
/// Unix-millisecond timestamp.
fn swarm_event_to_prost(evt: crate::swarm::events::SwarmEvent) -> generated::SwarmEvent {
    use crate::swarm::events::SwarmEvent as E;

    match evt {
        E::AgentStarted {
            agent_kind,
            task_id,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "agent_started".to_string(),
            agent: swarm_agent_kind_to_prost(agent_kind),
            body: serde_json::json!({ "task_id": task_id }).to_string(),
            ts: timestamp,
        },
        E::AgentCompleted {
            agent_kind,
            task_id,
            success,
            error,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "agent_completed".to_string(),
            agent: swarm_agent_kind_to_prost(agent_kind),
            body: serde_json::json!({
                "task_id": task_id,
                "success": success,
                "error": error,
            })
            .to_string(),
            ts: timestamp,
        },
        E::NegotiationStarted {
            task_id,
            candidate_count,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "negotiation_started".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "task_id": task_id,
                "candidate_count": candidate_count,
            })
            .to_string(),
            ts: timestamp,
        },
        E::ArbitrationResolved {
            task_id,
            chosen_kind,
            method,
            conflict_detected,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "arbitration_resolved".to_string(),
            agent: swarm_agent_kind_to_prost(chosen_kind),
            body: serde_json::json!({
                "task_id": task_id,
                "chosen_kind": chosen_kind,
                "method": method,
                "conflict_detected": conflict_detected,
            })
            .to_string(),
            ts: timestamp,
        },
        E::SwarmCompleted {
            task_id,
            success_count,
            failure_count,
            approved,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "swarm_completed".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "task_id": task_id,
                "success_count": success_count,
                "failure_count": failure_count,
                "approved": approved,
            })
            .to_string(),
            ts: timestamp,
        },
        E::AgentToolCall {
            agent_id,
            agent_role,
            tool_name,
            start_ts,
            end_ts,
            duration_ms,
            success,
            output_preview,
            error,
            task_id,
        } => generated::SwarmEvent {
            event_type: "agent_tool_call".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "agent_id": agent_id,
                "agent_role": agent_role,
                "tool_name": tool_name,
                "start_ts": start_ts,
                "end_ts": end_ts,
                "duration_ms": duration_ms,
                "success": success,
                "output_preview": output_preview,
                "error": error,
                "task_id": task_id,
            })
            .to_string(),
            ts: start_ts,
        },
        E::AgentOutputChunk {
            agent_id,
            delta,
            ts,
            task_id,
        } => generated::SwarmEvent {
            event_type: "agent_output_chunk".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "agent_id": agent_id,
                "delta": delta,
                "task_id": task_id,
            })
            .to_string(),
            ts,
        },
        E::DeadlockDetected {
            cycle,
            detected_at,
        } => generated::SwarmEvent {
            event_type: "deadlock_detected".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "cycle": cycle,
            })
            .to_string(),
            ts: detected_at,
        },
        E::TreeOfThoughtsStarted {
            task_id,
            branches,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "tree_of_thoughts_started".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "task_id": task_id,
                "branches": branches,
            })
            .to_string(),
            ts: timestamp,
        },
        E::PathCompleted {
            task_id,
            path_id,
            strategy,
            timestamp,
        } => generated::SwarmEvent {
            event_type: "path_completed".to_string(),
            agent: generated::AgentKind::Unspecified as i32,
            body: serde_json::json!({
                "task_id": task_id,
                "path_id": path_id,
                "strategy": strategy,
            })
            .to_string(),
            ts: timestamp,
        },
    }
}

// ===========================================================================
// Server startup — uses tonic::transport::Server (replaces hand-rolled hyper shim)
// ===========================================================================

use std::net::SocketAddr;
use tokio::sync::oneshot;
use tracing::error;

/// Handle to a running tonic gRPC server. Dropping it sends a shutdown
/// signal; `.shutdown().await` waits for the server task to exit.
pub struct TonicGrpcHandle {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl TonicGrpcHandle {
    /// Returns the local address the server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Signals shutdown and waits for the server task to exit.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            match join.await {
                Ok(_) => info!(target: "nebula.grpc", addr = %self.addr, "tonic gRPC server stopped"),
                Err(e) => warn!(target: "nebula.grpc", error = ?e, "tonic gRPC server join error"),
            }
        }
    }
}

impl Drop for TonicGrpcHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Starts a real tonic gRPC server bound to `bind_addr`.
///
/// This replaces the hand-rolled hyper HTTP/2 shim (`server.rs::start_server`)
/// with `tonic::transport::Server`, using the prost-generated server traits
/// from `proto/nebula.v1.rs`.
///
/// All 5 services (Memory, Swarm, Reflect, LLM, Skill) are registered on a
/// single tonic server, each accessible at its generated path prefix
/// (e.g. `/nebula.v1.MemoryService/Store`).
///
/// 关键修复：先 bind TcpListener 获取实际端口（bind_addr 为 :0 时
/// 由 OS 分配随机端口），再用 serve_with_incoming 让 tonic 使用已绑定
/// 的 listener。否则 TonicGrpcHandle.addr 会保存 :0 而非实际端口，
/// 导致客户端连接失败。
pub async fn start_tonic_server(bind_addr: String, state: AppState) -> anyhow::Result<TonicGrpcHandle> {
    let addr: SocketAddr = bind_addr
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid gRPC bind address '{}': {}", bind_addr, e))?;

    // 先 bind TcpListener 获取实际端口（bind_addr 为 :0 时由 OS 分配）
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind gRPC listener on {addr}: {e}"))?;
    let bound = listener.local_addr()?;
    info!(target: "nebula.grpc", requested = %addr, bound = %bound, "gRPC listener bound");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let impl_arc = std::sync::Arc::new(TonicServiceImpl::new(state));

    // Clone Arcs for each service registration — tonic's add_service takes ownership.
    let memory_impl = impl_arc.clone();
    let swarm_impl = impl_arc.clone();
    let reflect_impl = impl_arc.clone();
    let llm_impl = impl_arc.clone();
    let skill_impl = impl_arc.clone();

    let join = tokio::spawn(async move {
        // 用 serve_with_incoming_shutdown 代替 serve_with_shutdown，
        // 这样 tonic 使用我们已经 bind 好的 listener（端口已知），而不是
        // 内部再 bind。tonic 0.12 用 TcpIncoming::from_listener 转换。
        let incoming = match tonic::transport::server::TcpIncoming::from_listener(listener, true, None) {
            Ok(i) => i,
            Err(e) => {
                error!(target: "nebula.grpc", error = %e, "failed to create TcpIncoming");
                return;
            }
        };
        let result = tonic::transport::Server::builder()
            .add_service(generated::memory_service_server::MemoryServiceServer::from_arc(memory_impl))
            .add_service(generated::swarm_service_server::SwarmServiceServer::from_arc(swarm_impl))
            .add_service(generated::reflect_service_server::ReflectServiceServer::from_arc(reflect_impl))
            .add_service(generated::llm_service_server::LlmServiceServer::from_arc(llm_impl))
            .add_service(generated::skill_service_server::SkillServiceServer::from_arc(skill_impl))
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_rx.await;
                info!(target: "nebula.grpc", "tonic gRPC server received shutdown signal");
            })
            .await;

        if let Err(e) = result {
            error!(target: "nebula.grpc", error = %e, "tonic gRPC server error");
        }
    });

    info!(target: "nebula.grpc", addr = %bound, "tonic gRPC server task spawned (5 services, 22 RPCs)");

    Ok(TonicGrpcHandle {
        addr: bound,
        shutdown_tx: Some(shutdown_tx),
        join: Some(join),
    })
}

