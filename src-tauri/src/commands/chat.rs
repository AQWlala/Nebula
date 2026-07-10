//! Chat commands — `chat` and `chat_stream`.

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{info, instrument, warn};

use crate::api::server::{ChatRequestDto, NebulaService, StoreMemoryRequest};
use crate::commands::error::CommandError;
use crate::llm::ChatMessage;
use crate::memory::types::{MemoryLayer, MemoryType, SourceKind};
use crate::AppState;

/// Tauri command: send a chat message, return the assistant reply, and
/// persist both sides to memory (L1).
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "chat"))]
pub async fn chat(
    state: State<'_, AppState>,
    request: ChatRequestDto,
) -> Result<ChatResponseDto, CommandError> {
    // v1.1: Prompt injection scan before processing.
    let scan = crate::security::injection_guard::full_injection_scan(&request.user_message);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nebula.cmd",
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in chat"
            );
            return Err(CommandError::validation("chat").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string(),
            ));
        }
        if !scan.safe {
            tracing::warn!(
                target: "nebula.cmd",
                severity = %severity,
                "non-critical injection warning in chat"
            );
        }
    }

    let start = std::time::Instant::now();
    let resp = state
        .chat(request.clone())
        .await
        .map_err(|e| CommandError::llm("chat", &e))?;
    // v1.8: 记录 LLM chat 延迟（微秒）。
    crate::metrics::global().record_chat_latency(start.elapsed().as_micros() as u64);
    crate::metrics::global().record_chat();
    info!(target: "nebula.cmd", model = %resp.model, "chat ok");

    let state_for_memory = state.inner().clone();
    let user_msg = request.user_message.clone();
    let asst_msg = resp.message.content.clone();
    tokio::spawn(async move {
        // T-E-S-28: 非流式路径不生成 turn_id(只有流式 ChatComplete 才注入),
        // 此处传 None 保持向后兼容。
        if let Err(e) = absorb_chat_turn(&state_for_memory, &user_msg, &asst_msg, None).await {
            warn!(target: "nebula.cmd", error = ?e, "failed to absorb chat turn into memory");
        }
    });

    Ok(ChatResponseDto {
        model: resp.model,
        content: resp.message.content,
        role: resp.message.role,
        reasoning_chain: resp.reasoning_chain.clone(),
        // T-E-S-64: 非流式路径由 AppState::chat() 内部生成报告(仅日志),
        // 此处 DTO 字段保持 None(trait 返回类型无法携带报告)。
        consistency: None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponseDto {
    pub model: String,
    pub role: String,
    pub content: String,
    /// T-E-B-17: 推理链(可选)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_chain: Option<crate::llm::reasoning::ReasoningChain>,
    /// T-E-S-64: 反幻觉一致性报告(可选)。
    /// 非流式路径目前不填充(trait 返回类型限制);流式路径由
    /// `chat_stream` 直接注入 `ChatComplete.consistency`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consistency: Option<crate::memory::consistency::ConsistencyReport>,
}

/// T-S1-B-01a: 流式 chat 命令，使用 Tauri 2.0 `ipc::Channel` 向后端
/// 推送 token，前端可实时渲染并支持中途取消。
///
/// 返回值 `ChatComplete` 包含完整拼接后的消息（供前端在流结束后
/// 做最终状态同步）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatComplete {
    pub model: String,
    pub content: String,
    pub role: String,
    /// T-E-B-17: 推理链(可选)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_chain: Option<crate::llm::reasoning::ReasoningChain>,
    /// T-E-S-64: 反幻觉一致性报告(可选)。
    /// 由 `chat_stream` 在流结束后调用 `consistency::analyze` 生成。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consistency: Option<crate::memory::consistency::ConsistencyReport>,
    /// T-E-S-28: 本次 assistant 回复的 turn_id(UUID v4)。
    /// 前端用它关联 👍/👎 标注按钮,调用 `annotation_upsert` 时回传。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[tauri::command]
#[instrument(skip(state, on_token), fields(otel.kind = "chat_stream"))]
pub async fn chat_stream(
    state: State<'_, AppState>,
    request: ChatRequestDto,
    on_token: tauri::ipc::Channel<crate::llm::StreamToken>,
) -> Result<ChatComplete, CommandError> {
    let scan = crate::security::injection_guard::full_injection_scan(&request.user_message);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            return Err(CommandError::validation("chat_stream").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string(),
            ));
        }
    }

    // T-S1-A-02: 同样注入记忆上下文到流式路径。
    let context_bundle = state
        .memory
        .orchestrator
        .assemble_context(&request.user_message, "system")
        .await
        .map_err(|e| CommandError::internal("chat_stream", &e))?;

    // T-E-S-28: 为本次 assistant 回复生成 turn_id(UUID v4)。
    // 透传到前端 ChatComplete.turn_id,前端用它关联 👍/👎 标注按钮。
    let turn_id = uuid::Uuid::new_v4().to_string();

    let mut msgs: Vec<ChatMessage> = Vec::new();
    if !context_bundle.text.is_empty() {
        let sys_with_context = if let Some(sys) = request.system.as_deref() {
            format!("{sys}\n\n【相关记忆上下文】\n{}", context_bundle.text)
        } else {
            format!("【相关记忆上下文】\n{}", context_bundle.text)
        };
        msgs.push(ChatMessage::system(&sys_with_context));
    } else if let Some(sys) = request.system.as_deref() {
        msgs.push(ChatMessage::system(sys));
    }
    msgs.push(ChatMessage::user(request.user_message.clone()));

    let model = state.llm.llm.default_model();
    let mut full_content = String::new();

    // T-E-D-02: TTFT(首响时间)埋点 — 记录 stream 启动时刻,
    // 首个非空 token 到达时计算耗时(微秒)写入 metrics。
    let ttft_start = std::time::Instant::now();
    let mut first_token_recorded = false;

    // M7a #86: ADR-003 Phase 4 — chat_stream 走 UnifiedModelDispatcher。
    // P0-2: unified-dispatcher 默认启用；dispatcher 已注入时走
    // `dispatch_stream(WorkType::Chat)`，否则（运行时禁用或未注入）回退到
    // `LlmGateway::chat_stream`。注:dispatch_stream 远端路径内部就是转发
    // gateway.chat_stream,行为等价;本地路径走 OllamaClient.chat_stream,
    // 与 LlmGateway 的 ollama 路径一致。
    let stream = if let Some(dispatcher) = &state.llm.dispatcher {
        use crate::llm::dispatcher::WorkType;
        dispatcher.dispatch_stream(WorkType::Chat, msgs)
    } else {
        state.llm.llm.chat_stream(msgs)
    };
    use futures::StreamExt;
    let mut stream = stream;
    while let Some(result) = stream.next().await {
        match result {
            Ok(token) => {
                if !token.text.is_empty() {
                    // T-E-D-02: 首个非空 token 到达,记录 TTFT(仅一次)。
                    if !first_token_recorded {
                        let ttft_us = ttft_start.elapsed().as_micros() as u64;
                        crate::metrics::global().record_ttft(ttft_us);
                        first_token_recorded = true;
                    }
                    full_content.push_str(&token.text);
                }
                // Channel 发送失败（前端已关闭）时不阻断，只是停止推送。
                if on_token.send(token).is_err() {
                    break;
                }
            }
            Err(e) => {
                tracing::warn!(target: "nebula.cmd", error = %e, "chat_stream token error");
            }
        }
    }

    // T-E-S-28: 后台持久化 chat turn(user + assistant)到 L1 Episodic 记忆。
    // turn_id 写入 metadata,便于后续通过 turn_id 反查关联的记忆条目。
    let state_for_memory = state.inner().clone();
    let user_msg_for_memory = request.user_message.clone();
    let asst_msg_for_memory = full_content.clone();
    let turn_id_for_memory = turn_id.clone();
    // T-E-B-01: 复制一份供 wiki 编译使用(spawn-and-forget,失败仅记日志)。
    let state_for_wiki = state.inner().clone();
    let user_msg_for_wiki = request.user_message.clone();
    let asst_msg_for_wiki = full_content.clone();
    let turn_id_for_wiki = turn_id.clone();
    tokio::spawn(async move {
        if let Err(e) = absorb_chat_turn(
            &state_for_memory,
            &user_msg_for_memory,
            &asst_msg_for_memory,
            Some(&turn_id_for_memory),
        )
        .await
        {
            warn!(
                target: "nebula.cmd",
                error = ?e,
                "failed to absorb chat turn into memory"
            );
        }

        // T-E-B-01: LLM Wiki 编译 — spawn-and-forget,失败仅记日志。
        // compile_turn 幂等(同 turn_id 短路);config.wiki_enabled=false 时
        // 内部仍调 LLM 但 compile_turn 会因 enabled=false 短路。
        // 注:turn_id 为空字符串时不编译(与原 Some(tid) 语义一致)。
        if !turn_id_for_wiki.is_empty() {
            if let Err(e) = state_for_wiki
                .platform
                .wiki
                .compile_turn(&turn_id_for_wiki, &user_msg_for_wiki, &asst_msg_for_wiki)
                .await
            {
                warn!(
                    target: "nebula.cmd",
                    error = ?e,
                    turn_id = %turn_id_for_wiki,
                    "failed to compile wiki note from chat turn (T-E-B-01)"
                );
            }
        }
    });

    Ok(ChatComplete {
        model: model.to_string(),
        content: full_content.clone(),
        role: "assistant".to_string(),
        reasoning_chain: None,
        // T-E-S-64: 流结束后调用 consistency::analyze 生成反幻觉报告。
        // analyze 为同步函数(<1ms),不阻塞主路径。
        consistency: Some(crate::memory::consistency::analyze(
            &context_bundle.cited_memories,
            &full_content,
        )),
        // T-E-S-28: 透传 turn_id 到前端,供标注按钮关联。
        turn_id: Some(turn_id),
    })
}

/// Persist a chat turn (user prompt + assistant reply) as a pair of
/// L1 Episodic memories. Best-effort; errors are surfaced to the
/// caller so the spawn-and-forget site can log them.
///
/// T-E-S-28: `turn_id` 写入 assistant 消息的 metadata,便于后续通过
/// turn_id 反查关联的记忆条目(用于标注反馈回流分析)。
/// `turn_id = None` 时(如非流式路径)保持向后兼容,不写入该字段。
async fn absorb_chat_turn(
    state: &AppState,
    user_msg: &str,
    asst_msg: &str,
    turn_id: Option<&str>,
) -> anyhow::Result<()> {
    if !user_msg.trim().is_empty() {
        let req = StoreMemoryRequest {
            content: user_msg.to_string(),
            memory_type: MemoryType::Episodic,
            layer: MemoryLayer::L1,
            source: SourceKind::UserInput,
            metadata: Some(serde_json::json!({ "channel": "chat.user" })),
        };
        state.memory_store(req).await?;
    }
    if !asst_msg.trim().is_empty() {
        let metadata = match turn_id {
            Some(tid) => serde_json::json!({ "channel": "chat.assistant", "turn_id": tid }),
            None => serde_json::json!({ "channel": "chat.assistant" }),
        };
        let req = StoreMemoryRequest {
            content: asst_msg.to_string(),
            memory_type: MemoryType::Episodic,
            layer: MemoryLayer::L1,
            source: SourceKind::AgentOutput,
            metadata: Some(metadata),
        };
        state.memory_store(req).await?;
    }
    Ok(())
}
