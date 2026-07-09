//! Implementation of the [`NebulaService`] trait on [`AppState`].
//!
//! This bridges the Tauri command layer with the gRPC API layer
//! ([`crate::api::server`]).  The trait methods are the non-streaming
//! counterparts of the Tauri commands in [`crate::commands::chat`] /
//! [`crate::commands::memory`] and reuse the same [`AppState`] fields
//! (orchestrator, sponge, lance, embedder, llm, swarm).
//!
//! Extracted from `commands/mod.rs` to keep the module root focused on
//! declarations and re-exports.

use crate::api::server::{
    ChatRequestDto, NebulaService, SearchMemoryHit, SearchMemoryRequest, StoreMemoryRequest,
    StoreMemoryResponse,
};
use crate::llm::ChatMessage;
use crate::memory::sponge::SpongeResult;
use crate::memory::types::Memory;
use crate::swarm::{OrchestrationReport, SwarmTask};
use crate::AppState;

/// M7b #94: 在 gRPC service 层执行 injection_scan,统一覆盖所有非 Tauri 命令入口。
///
/// Critical/High 级别命中时返回 Err,阻止请求进入 LLM / memory / swarm 路径。
/// Low/Medium 级别仅记日志不拦截(与 Tauri 命令层行为一致)。
fn injection_guard_check(caller: &str, text: &str) -> anyhow::Result<()> {
    let scan = crate::security::injection_guard::full_injection_scan(text);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nebula.security",
                caller = caller,
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in service layer"
            );
            anyhow::bail!("输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截");
        }
        if !scan.safe {
            tracing::warn!(
                target: "nebula.security",
                caller = caller,
                severity = %severity,
                "non-critical injection warning in service layer"
            );
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl NebulaService for AppState {
    async fn chat(&self, req: ChatRequestDto) -> anyhow::Result<crate::llm::ChatResponse> {
        // M7b #94: service 层 injection_scan(gRPC 入口覆盖)。
        injection_guard_check("service.chat", &req.user_message)?;

        // T-S1-A-02: MemoryOrchestrator 接入 chat 路径。
        // 根据用户消息组装相关记忆上下文，拼接到 system prompt 前。
        let context_bundle = self
            .memory
            .orchestrator
            .assemble_context(&req.user_message, "system")
            .await?;

        // T-E-S-39: 注入 SOUL.md/AGENTS.md/TOOLS.md persona 前缀。
        // persona 缓存位于 AppConfig(Option<Arc<RwLock<PersonaConfig>>>),
        // 读取失败/为空时跳过(不阻塞 chat)。
        //
        // M1 任务 #23: Soul vs PersonaConfig 共存逻辑。
        // 优先级：Soul（CompiledSoul.system_prompt）> PersonaConfig。
        // - Soul 启用且编译成功：用 CompiledSoul.system_prompt 替代 <soul> 部分，
        //   保留 AGENTS.md / TOOLS.md（PersonaConfig 仍提供）。
        // - Soul 未启用或编译失败：回退到 PersonaConfig 全量注入。
        let persona_prefix = self.infra.config.persona.as_ref().and_then(|pc| {
            let guard = pc.read();
            if guard.is_empty() {
                None
            } else {
                Some(guard.to_system_prefix())
            }
        });

        // M1 任务 #23: 尝试 Soul 编译（cfg-gated）。
        // Soul 启用时，CompiledSoul.system_prompt 替代 persona_prefix。
        #[cfg(feature = "soul-system")]
        let soul_prompt = self.try_compile_soul().await;
        #[cfg(not(feature = "soul-system"))]
        let soul_prompt: Option<String> = None;

        // Soul 优先于 PersonaConfig。
        let final_prefix = soul_prompt.or(persona_prefix);

        let mut msgs: Vec<ChatMessage> = Vec::new();
        if !context_bundle.text.is_empty() {
            // 有记忆上下文时，把记忆作为 system prompt 的一部分注入。
            let sys_with_context = if let Some(sys) = req.system.as_deref() {
                format!("{sys}\n\n【相关记忆上下文】\n{}", context_bundle.text)
            } else {
                format!("【相关记忆上下文】\n{}", context_bundle.text)
            };
            // persona/Soul 前缀拼接到 system prompt 最前。
            let final_sys = match &final_prefix {
                Some(pp) => format!("{pp}\n{sys_with_context}"),
                None => sys_with_context,
            };
            msgs.push(ChatMessage::system(&final_sys));
        } else if let Some(sys) = req.system.as_deref() {
            let final_sys = match &final_prefix {
                Some(pp) => format!("{pp}\n{sys}"),
                None => sys.to_string(),
            };
            msgs.push(ChatMessage::system(&final_sys));
        } else if final_prefix.is_some() {
            // 无原 system prompt 但有 persona/Soul:单独注入。
            msgs.push(ChatMessage::system(
                final_prefix.as_deref().expect("must succeed"),
            ));
        }
        msgs.push(ChatMessage::user(req.user_message));
        // M7a #86: ADR-003 Phase 4 — chat 走 UnifiedModelDispatcher。
        // 双路径回滚(P1-19):unified-dispatcher feature on 且 dispatcher 已注入
        // 时走 `dispatch(WorkType::Chat)`;否则回退到 `LlmGateway::chat`。
        #[cfg(feature = "unified-dispatcher")]
        {
            let resp = if let Some(dispatcher) = &self.llm.dispatcher {
                use crate::llm::dispatcher::WorkType;
                dispatcher.dispatch(WorkType::Chat, msgs).await?
            } else {
                self.llm.llm.chat(msgs).await?
            };
            // T-E-S-64: 调用 consistency::analyze 生成反幻觉报告。
            // trait 返回类型 `crate::llm::ChatResponse` 无法携带报告,
            // 此处仅做可观测性记录(risk_score / warning 数);前端 badge
            // 数据由流式路径 `chat_stream`(commands/chat.rs)直接注入
            // `ChatComplete.consistency` 字段。
            let report = crate::memory::consistency::analyze(
                &context_bundle.cited_memories,
                &resp.message.content,
            );
            tracing::debug!(
                target: "nebula.memory.consistency",
                risk_score = report.risk_score,
                warnings = report.warnings.len(),
                cited = report.cited.len(),
                "consistency report generated (non-streaming path)"
            );
            return Ok(resp);
        }
        // 非 unified-dispatcher 编译路径:走旧 LlmGateway::chat(行为等价)。
        #[cfg(not(feature = "unified-dispatcher"))]
        {
            let resp = { self.llm.llm.chat(msgs).await? };
            // T-E-S-64: 调用 consistency::analyze 生成反幻觉报告。
            // trait 返回类型 `crate::llm::ChatResponse` 无法携带报告,
            // 此处仅做可观测性记录(risk_score / warning 数);前端 badge
            // 数据由流式路径 `chat_stream`(commands/chat.rs)直接注入
            // `ChatComplete.consistency` 字段。
            let report = crate::memory::consistency::analyze(
                &context_bundle.cited_memories,
                &resp.message.content,
            );
            tracing::debug!(
                target: "nebula.memory.consistency",
                risk_score = report.risk_score,
                warnings = report.warnings.len(),
                cited = report.cited.len(),
                "consistency report generated (non-streaming path)"
            );
            Ok(resp)
        }
    }

    async fn memory_store(&self, req: StoreMemoryRequest) -> anyhow::Result<StoreMemoryResponse> {
        // M7b #94: memory_store 不在此处做 injection_scan,
        // 依赖 sponge.absorb 的纵深防御(sanitize 而非拒绝)。
        // 这样 LLM 输出(SourceKind::AgentOutput)被 sponge sanitize 为占位符,
        // 而用户输入(SourceKind::UserInput)已在 chat 命令入口扫描过。
        let mut mem = Memory::new(req.memory_type, req.layer, req.content, req.source);
        if let Some(meta) = req.metadata {
            mem.metadata = meta;
        }
        match self.memory.sponge.absorb(mem).await? {
            SpongeResult::Inserted { id } => Ok(StoreMemoryResponse {
                id,
                merged: false,
                similarity: None,
            }),
            SpongeResult::Merged { id, similarity } => Ok(StoreMemoryResponse {
                id,
                merged: true,
                similarity: Some(similarity),
            }),
            SpongeResult::Duplicate { id } => Ok(StoreMemoryResponse {
                id,
                merged: true,
                similarity: Some(1.0),
            }),
            // v1.5: 关键词未激活的降级吸收 — 仍算插入，但标记为未合并。
            SpongeResult::Deactivated { id } => Ok(StoreMemoryResponse {
                id,
                merged: false,
                similarity: None,
            }),
        }
    }

    async fn memory_search(
        &self,
        req: SearchMemoryRequest,
    ) -> anyhow::Result<Vec<SearchMemoryHit>> {
        let k = req.k.max(1);
        let query_emb = self.memory.embedder.embed(&req.query).await?;
        let hits = self.memory.lance.search(&query_emb, k).await?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
        let memories = self
            .memory
            .sqlite
            .get_many(&ids)
            .await
            .map_err(|e| anyhow::anyhow!("get_many error: {e}"))?;

        let score_by_id: std::collections::HashMap<&str, f32> =
            hits.iter().map(|(id, s)| (id.as_str(), *s)).collect();
        let mut ordered: Vec<(Memory, f32)> = memories
            .into_iter()
            .filter_map(|m| score_by_id.get(m.id.as_str()).map(|s| (m, *s)))
            .collect();
        ordered.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let out = ordered
            .into_iter()
            .filter_map(|(m, s)| {
                if let Some(layer) = req.layer {
                    if m.layer != layer {
                        return None;
                    }
                }
                Some(SearchMemoryHit {
                    memory: m,
                    score: s,
                })
            })
            .collect();
        Ok(out)
    }

    async fn swarm_execute(&self, task: SwarmTask) -> anyhow::Result<OrchestrationReport> {
        // M7b #94: service 层 injection_scan(swarm 任务描述扫描)。
        injection_guard_check("service.swarm_execute", &task.description)?;
        self.swarm.swarm.execute(task).await
    }

    async fn llm_complete(&self, prompt: String) -> anyhow::Result<String> {
        // M7b #94: service 层 injection_scan(裸 prompt 调用扫描)。
        injection_guard_check("service.llm_complete", &prompt)?;
        self.llm.llm.generate(&prompt).await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn swarm_agent_kind_parses_known_values() {
        use crate::swarm::agents::AgentKind;
        assert_eq!(
            "coder".parse::<AgentKind>().expect("parse should succeed"),
            AgentKind::Coder
        );
        assert_eq!(
            "writer"
                .parse::<AgentKind>()
                .expect("update should succeed"),
            AgentKind::Writer
        );
        assert_eq!(
            "reviewer"
                .parse::<AgentKind>()
                .expect("parse should succeed"),
            AgentKind::Reviewer
        );
        assert!("unknown".parse::<AgentKind>().is_err());
    }
}
