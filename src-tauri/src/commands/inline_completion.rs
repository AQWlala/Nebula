//! T-E-S-51: Level 0 内联补全 Tauri 命令。
//!
//! 前端通过 `invoke('inline_complete', { prefix })` 调用,后端把
//! 请求转发给 `AppState::inline_completion`(`InlineCompletionEngine`)。
//!
//! ## 零成本约定
//!
//! 引擎直连 `OllamaClient::generate_with_options`,**不经过**
//! `LlmGateway::call_remote` / `CostTracker::record` — 本地小模型
//! 补全不计费(spec §设计约束 第 1 条)。
//!
//! ## 失败静默
//!
//! `InlineCompletionEngine::suggest_completion` 内部已把所有错误
//! 映射为 `Ok(None)`(spec §设计约束 第 5 条),因此命令层不会返回
//! `Err`。保留 `Result<Option<String>, String>` 返回类型仅为符合
//! Tauri 命令规范。

use tauri::State;

use crate::AppState;

/// T-E-S-51: 内联补全命令。
///
/// 前端通过 `invoke('inline_complete', { prefix })` 调用,返回
/// `string | null`。返回 `null` 的情况:prefix 太短、防抖命中、
/// Ollama 离线 / 错误、模型返回空 / 回声。
///
/// **失败静默**:任何错误都返回 `Ok(None)` 而非 `Err`,前端不显示
/// 错误(spec §设计约束 第 5 条)。
#[tauri::command]
pub async fn inline_complete(
    state: State<'_, AppState>,
    prefix: String,
) -> Result<Option<String>, String> {
    state
        .inline_completion
        .suggest_completion(&prefix)
        .await
        .map_err(|e| e.to_string())
}
