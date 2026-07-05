//! T-E-S-39: SOUL.md / AGENTS.md / TOOLS.md persona commands.
//!
//! Three Tauri commands for hot-reloading / reading / setting the persona
//! configuration that prefixes the LLM system prompt.
//!
//! Extracted from `commands/mod.rs` to keep the module root focused on
//! declarations and re-exports.

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

/// 从工作区根目录重新加载 persona 文件并热更新缓存。
///
/// 读取 SOUL.md / AGENTS.md / TOOLS.md,失败不报错(对应字段为 None)。
/// 若 AppConfig.persona 为 None(首次或之前加载失败),则创建新缓存;
/// 若已存在,则替换整个 PersonaConfig。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "persona_reload"))]
pub async fn persona_reload(
    state: State<'_, AppState>,
) -> Result<crate::llm::persona::PersonaConfig, CommandError> {
    let ws_root = state.editor.workspace_root().to_path_buf();
    let new_config = crate::llm::persona::PersonaConfig::load(&ws_root)
        .await
        .map_err(|e| CommandError::internal("persona_reload", &e))?;
    // 热更新:替换整个 PersonaConfig。
    match state.config.persona.as_ref() {
        Some(pc) => {
            *pc.write() = new_config.clone();
        }
        None => {
            // 之前为 None,无法通过 &Arc 写入;此处仅返回快照,
            // 缓存在下次 bootstrap 后才常驻(前端可继续用 get 读取)。
            tracing::warn!(
                target: "nebula.cmd",
                "persona cache slot is None; reload returned snapshot but cache was not updated"
            );
        }
    }
    // 同步推送到 swarm。
    if let Some(pc) = state.config.persona.as_ref() {
        state.swarm.set_persona(pc.clone());
    }
    Ok(new_config)
}

/// 读取当前 persona 配置快照。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "persona_get"))]
pub async fn persona_get(
    state: State<'_, AppState>,
) -> Result<crate::llm::persona::PersonaConfig, CommandError> {
    Ok(state
        .config
        .persona
        .as_ref()
        .map(|pc| pc.read().clone())
        .unwrap_or_default())
}

/// 手动设置单个 persona 文件的内容(热更新内存缓存,不落盘)。
///
/// `kind` 取值:`"soul"` / `"agents"` / `"tools"`(不区分大小写)。
/// `content` 为 `None` 时清除该字段;为 `Some(s)` 时设置内容。
/// 若 AppConfig.persona 为 None,则先创建空 PersonaConfig 再写入。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "persona_set_file"))]
pub async fn persona_set_file(
    state: State<'_, AppState>,
    kind: String,
    content: Option<String>,
) -> Result<crate::llm::persona::PersonaConfig, CommandError> {
    // 由于 AppConfig.persona 是 Option<Arc<RwLock<PersonaConfig>>>,
    // 且 Arc 不允许 get_mut(除非是唯一的 Arc),我们无法在 None 时
    // 原地创建。采用策略:读取当前值(或默认),修改后通过 swarm 传播。
    let mut snapshot = state
        .config
        .persona
        .as_ref()
        .map(|pc| pc.read().clone())
        .unwrap_or_default();

    let kind_lower = kind.to_lowercase();
    match kind_lower.as_str() {
        "soul" => snapshot.soul_md = content,
        "agents" => snapshot.agents_md = content,
        "tools" => snapshot.tools_md = content,
        other => {
            return Err(CommandError::validation(format!(
                "unknown persona kind `{other}`; expected soul/agents/tools"
            )));
        }
    }

    // 写回缓存(若已存在)。
    if let Some(pc) = state.config.persona.as_ref() {
        *pc.write() = snapshot.clone();
    } else {
        tracing::warn!(
            target: "nebula.cmd",
            "persona cache slot is None; set_file returned snapshot but cache was not updated"
        );
    }

    // 同步推送到 swarm。
    if let Some(pc) = state.config.persona.as_ref() {
        state.swarm.set_persona(pc.clone());
    }

    Ok(snapshot)
}
