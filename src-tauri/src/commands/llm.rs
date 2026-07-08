//! LLM commands — complete, chat, embed.

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::api::server::NebulaService;
use crate::commands::error::CommandError;
use crate::llm::ChatMessage;
use crate::AppState;

/// Tauri command: raw LLM completion.
#[tauri::command]
#[instrument(skip(state, prompt), fields(otel.kind = "llm_complete"))]
pub async fn llm_complete(
    state: State<'_, AppState>,
    prompt: String,
    model: Option<String>,
) -> Result<String, CommandError> {
    let _ = model; // currently unused; reserved for v0.5 routing
    state
        .llm_complete(prompt)
        .await
        .map_err(|e| CommandError::llm("llm_complete", &e))
}

/// v0.3: multi-message LLM chat.
#[tauri::command]
#[instrument(skip(state, messages), fields(otel.kind = "llm_chat"))]
pub async fn llm_chat(
    state: State<'_, AppState>,
    messages: Vec<(String, String)>,
    model: Option<String>,
) -> Result<LlmChatDto, CommandError> {
    let model_ref = model.as_deref().unwrap_or("");
    let msgs: Vec<ChatMessage> = messages
        .into_iter()
        .map(|(role, content)| ChatMessage {
            role,
            content,
            ..Default::default()
        })
        .collect();
    let resp = if model_ref.is_empty() {
        state.llm.llm.chat(msgs).await
    } else {
        state.llm.llm.chat_with_model(model_ref, msgs).await
    }
    .map_err(|e| CommandError::llm("llm_chat", &e))?;
    Ok(LlmChatDto {
        role: resp.message.role,
        content: resp.message.content,
        model: resp.model,
        eval_count: resp.eval_count.unwrap_or(0) as i64,
        total_duration_ns: resp.total_duration.unwrap_or(0) as i64,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmChatDto {
    pub role: String,
    pub content: String,
    pub model: String,
    pub eval_count: i64,
    pub total_duration_ns: i64,
}

/// v0.3: embed a single text.
#[tauri::command]
#[instrument(skip(state, text), fields(otel.kind = "llm_embed"))]
pub async fn llm_embed(state: State<'_, AppState>, text: String) -> Result<Vec<f32>, CommandError> {
    state
        .memory
        .embedder
        .embed(&text)
        .await
        .map_err(|e| CommandError::llm("llm_embed", &e))
}

// ---------------------------------------------------------------------------
// T-E-C-02: ScreenReader 截图理解 — describe_screenshot 命令。
// ---------------------------------------------------------------------------

/// T-E-C-02: 让 vision_model 描述一张 base64 编码的截图。
///
/// 调用流程:
/// 1. 前端先调 `screenshot` 命令拿到 base64 PNG 字符串。
/// 2. 再调本命令,把 base64 + prompt 发给 vision_model(默认 qwen2.5-vl:3b)。
/// 3. LlmGateway::describe_image 内部复用 OllamaClient::chat — Ollama
///    多模态 API 接受 `{"messages":[{"role","content","images":[b64]}]}`。
///
/// 参数:
/// - `image_b64`: 截图的 base64 字符串(无 data: 前缀)。
/// - `prompt`: 可选,默认 "请详细描述这张截图的内容"。
///
/// 失败场景:
/// - Ollama 离线 → 通过 breaker 短路,返回 "circuit open"。
/// - vision_model 未拉取 → Ollama 返回 404,描述错误透传。
#[tauri::command]
#[instrument(skip(state, image_b64), fields(otel.kind = "describe_screenshot"))]
pub async fn describe_screenshot(
    state: State<'_, AppState>,
    image_b64: String,
    prompt: Option<String>,
) -> Result<String, CommandError> {
    let prompt = prompt.unwrap_or_else(|| "请详细描述这张截图的内容".to_string());
    let msg = ChatMessage {
        role: "user".into(),
        content: prompt,
        images: vec![image_b64],
        ..Default::default()
    };
    state
        .llm
        .llm
        .describe_image(&state.infra.config.vision_model, msg)
        .await
        .map_err(|e| CommandError::llm("describe_screenshot", &e))
}
