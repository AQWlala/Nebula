//! T-E-S-52: Level 1 定向编辑 Tauri 命令。
//!
//! 选中文字 + 快捷键 → AI 局部改写。复用 LlmGateway::generate(本地 Ollama,
//! 不计费,与 L0 一致)。失败返回 Err(前端 toast 提示,与 L0 失败静默不同)。

use tauri::State;

use crate::AppState;

const DIRECTED_EDIT_PROMPT_TEMPLATE: &str =
    "重写以下文本,保持原意但更清晰简洁,直接输出重写后的文本,不要加任何解释:\n\n{selected}";

/// T-E-S-52: 定向编辑命令。
///
/// 前端通过 `invoke('directed_edit', { selected })` 调用,返回重写后的
/// `string`。**失败返回 `Err`**(与 L0 失败静默不同),前端 toast 提示。
#[tauri::command]
pub async fn directed_edit(state: State<'_, AppState>, selected: String) -> Result<String, String> {
    if selected.trim().is_empty() {
        return Err("选区为空,请先选中文字".to_string());
    }
    let prompt = DIRECTED_EDIT_PROMPT_TEMPLATE.replace("{selected}", &selected);
    state
        .llm
        .llm
        .generate(&prompt)
        .await
        .map_err(|e| e.to_string())
}
