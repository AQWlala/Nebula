//! Tauri command handlers — the entry points invoked from the
//! front-end. Each command is a thin shim that translates a JSON DTO
//! into a call on the shared [`AppState`].
//!
//! ## v0.1 / v0.2
//! * Blocking SQLite I/O is funnelled through
//!   [`tokio::task::spawn_blocking`] so the Tauri runtime is never
//!   starved.
//! * Errors that escape the layer are mapped to a stable
//!   [`CommandError`] envelope (error code + safe message). The full
//!   chain is logged via `tracing::error!` for debugging.
//! * `chat` writes both the user prompt and the assistant reply to
//!   memory (L1 by default) so the sponge can absorb them.
//!
//! ## v0.3
//! * Skill CRUD commands (`skill_create`, `skill_use`, `skill_rate`,
//!   `skill_list`, `skill_search`).
//! * Memory read-by-id, update-importance, delete, get-many, stats.
//! * Swarm read commands (list-agents, get-agent).
//! * LLM `chat` and `embed` commands (previously only `complete`
//!   existed).
//! * Reflection read-by-id command.

pub mod error;

// v1.0.1 revert: P0#13 (split commands by topic into 9
// submodules) was reverted.  The submodule shims referenced
// items that never moved out of this file, and the in-place
// `pub use super::…` aliases collided with the macro
// `tauri::generate_handler!` name resolution (it walks the
// `super` path looking for `__cmd__name` and `__tauri_command_name_name`
// markers, but the re-export shadows the real function with
// the same name as the module).  Keep the file as the
// single, authoritative command surface for v1.0.1; a proper
// split will be attempted again in v1.0.2 once the API is
// fully stabilised.

// Re-export the API DTOs so other modules (gRPC, tests) can reach them
// through the `commands` namespace without depending on the internal
// `api::server` module path.
pub mod api {
    pub use crate::api::server::{
        ChatRequestDto, NineSnakeService, SearchMemoryHit, SearchMemoryRequest, StoreMemoryRequest,
        StoreMemoryResponse,
    };
    pub use crate::skills::types::{
        CreateSkillRequest as CreateSkillDto, ListSkillsRequest as ListSkillsDto,
        RateSkillRequest as RateSkillDto, Skill as SkillDto, SkillResult as SkillResultDto,
        SkillSearchRequest as SkillSearchDto, UseSkillRequest as UseSkillDto,
    };
}

use serde::{Deserialize, Serialize};
use base64::Engine as _;
use std::path::PathBuf;
use tauri::State;
use tracing::{info, instrument, warn};

use crate::api::server::{
    ChatRequestDto, NineSnakeService, SearchMemoryHit, SearchMemoryRequest, StoreMemoryRequest,
    StoreMemoryResponse,
};
use crate::editor::{self as editor_ops};
use crate::llm::ChatMessage;
use crate::memory::reflect::Reflection;
use crate::memory::sponge::SpongeResult;
use crate::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use crate::memory::MigrationStatus;
use crate::metrics::MetricsSnapshot;
use crate::os::{self, Notification, NotificationLevel};
use crate::skills::types as skill_types;
use crate::swarm::{OrchestrationReport, SwarmTask};
use crate::sync::{self as sync_ops, E2eeIdentity, EncryptedEnvelope, Pair};
use crate::tools::{ToolInput, ToolOutput};
use crate::work::{self as work_ops, TaskStatus, WorkTask};
use crate::writing::{Document, DocumentExport, ExportFormat, WritingTemplate};
use crate::AppState;

pub use error::{CommandError, ErrorCode};

// ---------------------------------------------------------------------------
// Tauri commands.
// ---------------------------------------------------------------------------

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
                target: "nine_snake.cmd",
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in chat"
            );
            return Err(CommandError::validation("chat").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string()
            ));
        }
        if !scan.safe {
            tracing::warn!(
                target: "nine_snake.cmd",
                severity = %severity,
                "non-critical injection warning in chat"
            );
        }
    }

    let resp = state
        .chat(request.clone())
        .await
        .map_err(|e| CommandError::llm("chat", &e))?;
    crate::metrics::global().record_chat();
    info!(target: "nine_snake.cmd", model = %resp.model, "chat ok");

    let state_for_memory = state.inner().clone();
    let user_msg = request.user_message.clone();
    let asst_msg = resp.message.content.clone();
    tokio::spawn(async move {
        if let Err(e) = absorb_chat_turn(&state_for_memory, &user_msg, &asst_msg).await {
            warn!(target: "nine_snake.cmd", error = ?e, "failed to absorb chat turn into memory");
        }
    });

    Ok(ChatResponseDto {
        model: resp.model,
        content: resp.message.content,
        role: resp.message.role,
    })
}

/// Tauri command: store a memory.
#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "memory_store"))]
pub async fn memory_store(
    state: State<'_, AppState>,
    request: StoreMemoryRequest,
) -> Result<StoreMemoryResponse, CommandError> {
    let resp = state
        .memory_store(request)
        .await
        .map_err(|e| CommandError::memory("memory_store", &e))?;
    crate::metrics::global().record_store();
    Ok(resp)
}

/// Tauri command: vector search over the memory store.
#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "memory_search"))]
pub async fn memory_search(
    state: State<'_, AppState>,
    request: SearchMemoryRequest,
) -> Result<Vec<SearchMemoryHit>, CommandError> {
    let resp = state
        .memory_search(request)
        .await
        .map_err(|e| CommandError::lance("memory_search", &e))?;
    crate::metrics::global().record_search();
    Ok(resp)
}

// -----------------------------------------------------------------------
// v0.3: read-side memory commands (used by the gRPC MemoryService and
// by the memory inspector UI).
// -----------------------------------------------------------------------

/// Tauri command: fetch a memory by id.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_get"))]
pub async fn memory_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Memory>, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.get(&id).await })
        .await
        .map_err(|e| CommandError::internal("memory_get", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_get", &e))
}

/// Tauri command: list the N most recent memories.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_list_recent"))]
pub async fn memory_list_recent(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<Memory>, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.list_recent(limit.max(1)).await })
        .await
        .map_err(|e| CommandError::internal("memory_list_recent", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_list_recent", &e))
}

/// Tauri command: update a memory's `importance` (clamped to `[0, 1]`).
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_update_importance"))]
pub async fn memory_update_importance(
    state: State<'_, AppState>,
    id: String,
    importance: f32,
) -> Result<Memory, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.update_importance(&id, importance.clamp(0.0, 1.0)).await })
        .await
        .map_err(|e| CommandError::internal("memory_update_importance", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_update_importance", &e))
}

/// Tauri command: hard-delete a memory.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_delete"))]
pub async fn memory_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let sqlite = state.sqlite.clone();
    let id_for_thread = id.clone();
    let res = tokio::task::spawn(async move { sqlite.delete(&id_for_thread).await })
        .await
        .map_err(|e| CommandError::internal("memory_delete", &anyhow::anyhow!("{e}")))?;
    match res {
        Ok(deleted) => {
            if deleted {
                if let Err(e) = state.lance.delete(&id).await {
                    warn!(target: "nine_snake.cmd", error = ?e, "lance delete failed");
                }
            }
            Ok(deleted)
        }
        Err(e) => Err(CommandError::db("memory_delete", &e)),
    }
}

/// Tauri command: batch-fetch memories by id (preserves the
/// caller's order).
#[tauri::command]
#[instrument(skip(state, ids), fields(otel.kind = "memory_get_many"))]
pub async fn memory_get_many(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<Memory>, CommandError> {
    let sqlite = state.sqlite.clone();
    tokio::task::spawn(async move { sqlite.get_many(&ids).await })
        .await
        .map_err(|e| CommandError::internal("memory_get_many", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_get_many", &e))
}

/// Snapshot of layer distribution for the stats RPC.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryStats {
    pub total: u64,
    pub by_layer: std::collections::HashMap<MemoryLayer, u64>,
}

/// Tauri command: per-layer memory counts.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "memory_stats"))]
pub async fn memory_stats(state: State<'_, AppState>) -> Result<MemoryStats, CommandError> {
    let sqlite = state.sqlite.clone();
    let rows = tokio::task::spawn(async move { sqlite.counts_per_layer().await })
        .await
        .map_err(|e| CommandError::internal("memory_stats", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::db("memory_stats", &e))?;
    let total = rows.values().sum();
    Ok(MemoryStats { total, by_layer: rows })
}

/// Tauri command: dispatch a swarm task.
#[tauri::command]
#[instrument(skip(state, task), fields(otel.kind = "swarm_execute"))]
pub async fn swarm_execute(
    state: State<'_, AppState>,
    task: SwarmTask,
) -> Result<OrchestrationReport, CommandError> {
    // v1.1: Prompt injection scan before processing.
    let scan = crate::security::injection_guard::full_injection_scan(&task.description);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nine_snake.cmd",
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in swarm_execute"
            );
            return Err(CommandError::validation("swarm_execute").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string()
            ));
        }
        if !scan.safe {
            tracing::warn!(
                target: "nine_snake.cmd",
                severity = %severity,
                "non-critical injection warning in swarm_execute"
            );
        }
    }

    let report = state
        .swarm_execute(task)
        .await
        .map_err(|e| CommandError::swarm("swarm_execute", &e))?;
    crate::metrics::global().record_swarm();
    Ok(report)
}

/// v0.3: list the available swarm agents as `(kind, name, system, description)`.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "swarm_list_agents"))]
pub async fn swarm_list_agents(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String, String, String)>, CommandError> {
    Ok(state.swarm.list_agents())
}

/// v0.3: fetch a single swarm agent by kind.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "swarm_get_agent"))]
pub async fn swarm_get_agent(
    state: State<'_, AppState>,
    kind: String,
) -> Result<Option<SwarmAgentInfo>, CommandError> {
    Ok(state.swarm.get_agent(&kind).map(|a| SwarmAgentInfo {
        name: a.name,
        system_prompt: a.system_prompt,
        description: a.description,
    }))
}

/// v0.3: agent descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmAgentInfo {
    pub name: String,
    pub system_prompt: String,
    pub description: String,
}

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
        .map(|(role, content)| ChatMessage { role, content })
        .collect();
    let resp = if model_ref.is_empty() {
        state.llm.chat(msgs).await
    } else {
        state.llm.chat_with_model(model_ref, msgs).await
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
pub async fn llm_embed(
    state: State<'_, AppState>,
    text: String,
) -> Result<Vec<f32>, CommandError> {
    state
        .embedder
        .embed(&text)
        .await
        .map_err(|e| CommandError::llm("llm_embed", &e))
}

// -----------------------------------------------------------------------
// Reflect commands
// -----------------------------------------------------------------------

/// v0.2: Tauri command — trigger a single reflection pass manually.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "reflect_now"))]
pub async fn reflect_now(
    state: State<'_, AppState>,
) -> Result<Vec<Reflection>, CommandError> {
    let engine = state.reflection.clone();
    engine
        .reflect_now()
        .await
        .map_err(|e| CommandError::memory("reflect_now", &e))
}

/// v0.2: Tauri command — list the most recent reflections.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "list_reflections"))]
pub async fn list_reflections(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<Reflection>, CommandError> {
    let engine = state.reflection.clone();
    let lim = limit.unwrap_or(20);
    tokio::task::spawn_blocking(move || engine.list_recent(lim))
        .await
        .map_err(|e| CommandError::internal("list_reflections", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::memory("list_reflections", &e))
}

/// v0.3: fetch a reflection by id.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "get_reflection"))]
pub async fn get_reflection(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Reflection>, CommandError> {
    let engine = state.reflection.clone();
    tokio::task::spawn_blocking(move || engine.get(&id))
        .await
        .map_err(|e| CommandError::internal("get_reflection", &anyhow::anyhow!("{e}")))?
        .map_err(|e| CommandError::memory("get_reflection", &e))
}

// -----------------------------------------------------------------------
// v0.3: Skill CRUD commands
// -----------------------------------------------------------------------

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_create"))]
pub async fn skill_create(
    state: State<'_, AppState>,
    request: skill_types::CreateSkillRequest,
) -> Result<skill_types::Skill, CommandError> {
    state
        .skills
        .create_skill(request)
        .map_err(|e| CommandError::db("skill_create", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_use"))]
pub async fn skill_use(
    state: State<'_, AppState>,
    request: skill_types::UseSkillRequest,
) -> Result<skill_types::SkillResult, CommandError> {
    // v1.1: Prompt injection scan on skill input.
    let input_text = format!("{:?}", request);
    let scan = crate::security::injection_guard::full_injection_scan(&input_text);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nine_snake.cmd",
                "blocked critical injection / credential leak in skill_use"
            );
            return Err(CommandError::validation("skill_use").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string()
            ));
        }
    }

    state
        .skills
        .use_skill(request)
        .await
        .map_err(|e| CommandError::internal("skill_use", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_rate"))]
pub async fn skill_rate(
    state: State<'_, AppState>,
    request: skill_types::RateSkillRequest,
) -> Result<skill_types::Skill, CommandError> {
    state
        .skills
        .rate_skill(request)
        .map_err(|e| CommandError::db("skill_rate", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_list"))]
pub async fn skill_list(
    state: State<'_, AppState>,
    request: skill_types::ListSkillsRequest,
) -> Result<Vec<skill_types::Skill>, CommandError> {
    state
        .skills
        .list_skills(request)
        .map_err(|e| CommandError::db("skill_list", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_search"))]
pub async fn skill_search(
    state: State<'_, AppState>,
    request: skill_types::SkillSearchRequest,
) -> Result<Vec<skill_types::Skill>, CommandError> {
    state
        .skills
        .search_skills(request)
        .map_err(|e| CommandError::db("skill_search", &e))
}

// -----------------------------------------------------------------------
// v0.2 commands (unchanged signatures).
// -----------------------------------------------------------------------

/// v0.2: Tauri command — snapshot the process-wide metrics.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "metrics"))]
pub async fn metrics(state: State<'_, AppState>) -> Result<MetricsSnapshot, CommandError> {
    let _ = state;
    Ok(crate::metrics::global().snapshot())
}

/// v0.2: Tauri command — read the current migration status.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "migration_status"))]
pub async fn migration_status(
    state: State<'_, AppState>,
) -> Result<MigrationStatus, CommandError> {
    let sqlite = state.sqlite.clone();
    let dir = crate::memory::migration::bundled_migrations_dir().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let conn = sqlite.raw_connection();
        let conn = conn.lock();
        crate::memory::migration::migration_status(&conn, &dir)
    })
    .await
    .map_err(|e| CommandError::internal("migration_status", &anyhow::anyhow!("{e}")))?
    .map_err(|e| CommandError::db("migration_status", &e))
}

// ---------------------------------------------------------------------------
// DTOs that flow through the Tauri command boundary.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponseDto {
    pub model: String,
    pub role: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// v1.0: bootstrap, health, perf, and app-settings commands.
// ---------------------------------------------------------------------------

/// v1.0: front-end handshake.  The store calls this on mount to
/// confirm the Tauri runtime is responsive and to record a
/// `bootstrap.start` milestone.  No-op on the back-end side; the
/// actual work is the `AppState::bootstrap` call inside
/// `lib::run`.
#[tauri::command]
#[instrument(fields(otel.kind = "bootstrap"))]
pub async fn bootstrap() -> Result<(), CommandError> {
    Ok(())
}

/// v1.0: reports the running version.  The front-end shows it in
/// the sidebar footer.
#[tauri::command]
pub async fn health(state: State<'_, AppState>) -> Result<HealthDto, CommandError> {
    let ollama_status = {
        let client = state.llm.ollama_client();
        match tokio::time::timeout(std::time::Duration::from_secs(2), client.ping()).await {
            Ok(true) => "ok".to_string(),
            Ok(false) => "down".to_string(),
            Err(_) => "timeout".to_string(),
        }
    };
    Ok(HealthDto {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ollama: ollama_status,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthDto {
    pub status: String,
    pub version: String,
    pub ollama: String,
}

/// v1.0: returns the cold-start report.  Cheap; just a clone of
/// the in-memory `BTreeMap`.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "startup_report"))]
pub async fn startup_report(
    state: State<'_, AppState>,
) -> Result<crate::perf::StartupReport, CommandError> {
    Ok(state.startup_timer.report())
}

/// v1.0: live process sample.  Returns an empty struct when the
/// `perf-telemetry` feature is off; the front-end handles the
/// "no data" case in `StatusBar`.
#[tauri::command]
#[instrument(fields(otel.kind = "perf_sample"))]
pub async fn perf_sample() -> Result<crate::perf::monitor::PerfSample, CommandError> {
    Ok(crate::perf::monitor::PerfSample::empty())
}

// ---------------------------------------------------------------------------
// v1.0: persisted app settings (front-end mirror of the user's
// preferences that need to live on disk rather than localStorage).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AppSettingsDto {
    /// Last-selected mode (writing/work/code).
    pub last_mode: Option<String>,
    /// Last-selected view (chat/swarm/memory/code/skills).
    pub last_view: Option<String>,
    /// Ollama URL.
    pub ollama_url: Option<String>,
    /// Default chat model.
    pub chat_model: Option<String>,
    /// Editor workspace root (relative to the project root or
    /// absolute).
    pub workspace: Option<String>,
    /// UI locale: "zh-CN" or "en-US".
    pub locale: Option<String>,
    /// UI theme: "dark" | "light" | "system".
    pub theme: Option<String>,
    /// Accent color (CSS hex).
    pub accent: Option<String>,
    /// Font size in px.
    pub font_size: Option<u32>,
    /// Auto-save interval in seconds.
    pub autosave_sec: Option<u32>,
    /// Custom shell whitelist additions.
    pub extra_shell_bins: Option<Vec<String>>,
    /// Onboarding completed.
    pub onboarding_done: Option<bool>,
}

fn settings_path() -> std::path::PathBuf {
    let base = std::env::var("NINE_SNAKE_DATA_DIR")
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join("settings.json")
}

fn read_settings() -> AppSettingsDto {
    let p = settings_path();
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => AppSettingsDto::default(),
    }
}

fn write_settings(s: &AppSettingsDto) -> anyhow::Result<()> {
    let p = settings_path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let bytes = serde_json::to_vec_pretty(s)?;
    std::fs::write(&p, bytes)?;
    Ok(())
}

#[tauri::command]
#[instrument(fields(otel.kind = "load_app_settings"))]
pub async fn load_app_settings() -> Result<AppSettingsDto, CommandError> {
    Ok(read_settings())
}

#[tauri::command]
#[instrument(skip(state, settings), fields(otel.kind = "save_app_settings"))]
pub async fn save_app_settings(
    state: State<'_, AppState>,
    settings: AppSettingsDto,
) -> Result<(), CommandError> {
    // v1.0 audit: shell whitelist additions are validated against
    // the in-memory executor.  We currently do not mutate the
    // live whitelist (the executor is `Arc<ShellExecutor>` and
    // `allow` consumes `self`); the user is shown a warning when
    // they add a bin that isn't in the default whitelist.  v1.1
    // will switch to a `parking_lot::RwLock<Vec<String>>` so this
    // can be a hot update.
    if let Some(ref extras) = settings.extra_shell_bins {
        for b in extras {
            if !state.shell.is_allowed(b) {
                tracing::warn!(
                    target: "nine_snake.cmd",
                    bin = %b,
                    "user requested shell bin not in default whitelist; v1.0 cannot hot-add (see docs)"
                );
            }
        }
    }
    write_settings(&settings).map_err(|e| CommandError::internal("save_app_settings", &e))
}

// ---------------------------------------------------------------------------
// v1.0.1 P0#12: API key storage backed by the OS keychain.
//
// In v1.0 the user-supplied API key was written to
// `localStorage` by `Settings.tsx`.  That made the key readable
// by any JavaScript that ran in the WebView (including any
// malicious skill that got XSS via a poisoned memory).  The
// v1.0.1 fix moves the secret into the OS keychain via
// `crate::security::keychain`.  The front-end still passes
// the value through `Settings.tsx`, but it only ever lives in
// memory long enough to be shipped over Tauri IPC; it is
// never written to disk by the front-end.
// ---------------------------------------------------------------------------

/// Tauri command: write the API key into the OS keychain.
/// Returns `Ok(())` — the key is never echoed back.
#[tauri::command]
#[instrument(fields(otel.kind = "set_api_key"))]
pub async fn set_api_key(value: String) -> Result<(), CommandError> {
    if value.trim().is_empty() {
        crate::security::keychain::delete(crate::security::keychain::KEY_API_KEY)
            .map_err(|e| CommandError::internal("set_api_key", &e))?;
        return Ok(());
    }
    crate::security::keychain::set(crate::security::keychain::KEY_API_KEY, &value)
        .map_err(|e| CommandError::internal("set_api_key", &e))
}

/// Masked API key returned to the front-end — never the full secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskedApiKey {
    pub masked: String,
    pub length: usize,
    pub prefix: String,
}

/// Tauri command: read the API key from the OS keychain.
/// Returns a masked version (e.g. `sk-****678`) — the full key
/// is never sent across the IPC boundary.
#[tauri::command]
#[instrument(fields(otel.kind = "get_api_key"))]
pub async fn get_api_key() -> Result<Option<MaskedApiKey>, CommandError> {
    let raw = crate::security::keychain::get(crate::security::keychain::KEY_API_KEY)
        .map_err(|e| CommandError::internal("get_api_key", &e))?;
    Ok(raw.map(|key| {
        let len = key.len();
        let prefix_len = key.len().min(3);
        let suffix_len = key.len().min(3);
        let prefix = key[..prefix_len].to_string();
        let suffix = if len > 6 { &key[len - suffix_len..] } else { "" };
        let masked = if len > 6 {
            format!("{}****{}", &key[..prefix_len], suffix)
        } else if len > 0 {
            format!("{}****", &key[..prefix_len])
        } else {
            String::new()
        };
        MaskedApiKey { masked, length: len, prefix }
    }))
        .map_err(|e| CommandError::internal("get_api_key", &e))
}

/// Tauri command: delete the API key from the OS keychain.
/// Idempotent — deleting a missing entry is a successful no-op.
#[tauri::command]
#[instrument(fields(otel.kind = "delete_api_key"))]
pub async fn delete_api_key() -> Result<(), CommandError> {
    crate::security::keychain::delete(crate::security::keychain::KEY_API_KEY)
        .map_err(|e| CommandError::internal("delete_api_key", &e))
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// Persist a chat turn (user prompt + assistant reply) as a pair of
/// L1 Episodic memories. Best-effort; errors are surfaced to the
/// caller so the spawn-and-forget site can log them.
async fn absorb_chat_turn(
    state: &AppState,
    user_msg: &str,
    asst_msg: &str,
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
        let req = StoreMemoryRequest {
            content: asst_msg.to_string(),
            memory_type: MemoryType::Episodic,
            layer: MemoryLayer::L1,
            source: SourceKind::AgentOutput,
            metadata: Some(serde_json::json!({ "channel": "chat.assistant" })),
        };
        state.memory_store(req).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Implementation of the service trait on `AppState`.
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl NineSnakeService for AppState {
    async fn chat(&self, req: ChatRequestDto) -> anyhow::Result<crate::llm::ChatResponse> {
        let mut msgs: Vec<ChatMessage> = Vec::new();
        if let Some(sys) = req.system.as_deref() {
            msgs.push(ChatMessage::system(sys));
        }
        msgs.push(ChatMessage::user(req.user_message));
        let resp = self.llm.chat(msgs).await?;
        Ok(resp)
    }

    async fn memory_store(&self, req: StoreMemoryRequest) -> anyhow::Result<StoreMemoryResponse> {
        let mut mem = Memory::new(req.memory_type, req.layer, req.content, req.source);
        if let Some(meta) = req.metadata {
            mem.metadata = meta;
        }
        match self.sponge.absorb(mem).await? {
            SpongeResult::Inserted { id } => Ok(StoreMemoryResponse { id, merged: false, similarity: None }),
            SpongeResult::Merged { id, similarity } => Ok(StoreMemoryResponse { id, merged: true, similarity: Some(similarity) }),
            SpongeResult::Duplicate { id } => Ok(StoreMemoryResponse { id, merged: true, similarity: Some(1.0) }),
        }
    }

    async fn memory_search(&self, req: SearchMemoryRequest) -> anyhow::Result<Vec<SearchMemoryHit>> {
        let k = req.k.max(1);
        let query_emb = self.embedder.embed(&req.query).await?;
        let hits = self.lance.search(&query_emb, k).await?;
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
        let memories = self.sqlite.get_many(&ids).await
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
                Some(SearchMemoryHit { memory: m, score: s })
            })
            .collect();
        Ok(out)
    }

    async fn swarm_execute(&self, task: SwarmTask) -> anyhow::Result<OrchestrationReport> {
        self.swarm.execute(task).await
    }

    async fn llm_complete(&self, prompt: String) -> anyhow::Result<String> {
        self.llm.generate(&prompt).await
    }
}

// =====================================================================
// v0.5: writing-mode commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_list_templates"))]
pub async fn writing_list_templates(
    state: State<'_, AppState>,
) -> Result<Vec<WritingTemplate>, CommandError> {
    Ok(state.writing.list_templates())
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_get_template"))]
pub async fn writing_get_template(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<WritingTemplate>, CommandError> {
    Ok(state.writing.get_template(&id))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocumentRequest {
    pub title: String,
    pub template_id: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "writing_create_document"))]
pub async fn writing_create_document(
    state: State<'_, AppState>,
    request: CreateDocumentRequest,
) -> Result<Document, CommandError> {
    let engine = state.writing.clone();
    let req = request;
    tokio::task::spawn_blocking(move || {
        engine
            .create_document(req.title, req.template_id, req.content, req.metadata)
            .map_err(|e| CommandError::internal("writing_create_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_create_document", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state, content), fields(otel.kind = "writing_update_document"))]
pub async fn writing_update_document(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> Result<Document, CommandError> {
    let engine = state.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .update_document(&id, content)
            .map_err(|e| CommandError::internal("writing_update_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_update_document", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_get_document"))]
pub async fn writing_get_document(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Document>, CommandError> {
    let engine = state.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .get_document(&id)
            .map_err(|e| CommandError::internal("writing_get_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_get_document", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_list_documents"))]
pub async fn writing_list_documents(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<Document>, CommandError> {
    let engine = state.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .list_documents(limit.unwrap_or(50))
            .map_err(|e| CommandError::internal("writing_list_documents", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_list_documents", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_delete_document"))]
pub async fn writing_delete_document(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let engine = state.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .delete_document(&id)
            .map_err(|e| CommandError::internal("writing_delete_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_delete_document", &anyhow::anyhow!("{e}")))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    pub id: String,
    /// "markdown" | "html" | "md" | "htm"
    pub format: String,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "writing_export"))]
pub async fn writing_export(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<DocumentExport, CommandError> {
    let format = ExportFormat::from_str(&request.format)
        .map_err(|e| CommandError::validation("writing_export").with_details(e.to_string()))?;
    let engine = state.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .export(&request.id, format)
            .map_err(|e| CommandError::internal("writing_export", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_export", &anyhow::anyhow!("{e}")))?
}

// =====================================================================
// v0.5: work-mode commands
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub title: String,
    pub description: String,
    pub priority: Option<i32>,
    pub due_at: Option<i64>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "work_create_task"))]
pub async fn work_create_task(
    state: State<'_, AppState>,
    request: CreateTaskRequest,
) -> Result<WorkTask, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .create_task(request.title, request.description, request.priority, request.due_at)
            .map_err(|e| CommandError::validation("work_create_task").with_details(e.to_string()))
    })
    .await
    .map_err(|e| CommandError::internal("work_create_task", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_get_task"))]
pub async fn work_get_task(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<WorkTask>, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .get_task(&id)
            .map_err(|e| CommandError::internal("work_get_task", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_get_task", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_list_tasks"))]
pub async fn work_list_tasks(
    state: State<'_, AppState>,
    status: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<WorkTask>, CommandError> {
    let engine = state.work.clone();
    let parsed = status
        .map(|s| TaskStatus::from_str(&s))
        .transpose()
        .map_err(|e| CommandError::validation("work_list_tasks").with_details(e.to_string()))?;
    tokio::task::spawn_blocking(move || {
        engine
            .list_tasks(parsed, limit)
            .map_err(|e| CommandError::internal("work_list_tasks", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_list_tasks", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_set_status"))]
pub async fn work_set_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<WorkTask, CommandError> {
    let parsed = TaskStatus::from_str(&status)
        .map_err(|e| CommandError::validation("work_set_status").with_details(e.to_string()))?;
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .set_status(&id, parsed)
            .map_err(|e| CommandError::internal("work_set_status", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_set_status", &anyhow::anyhow!("{e}")))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTaskRequest {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i32>,
    /// `null` clears the due date, `Some(v)` sets it, `None` leaves it.
    pub due_at: Option<Option<i64>>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "work_update_task"))]
pub async fn work_update_task(
    state: State<'_, AppState>,
    request: UpdateTaskRequest,
) -> Result<WorkTask, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .update_task(&request.id, request.title, request.description, request.priority, request.due_at)
            .map_err(|e| CommandError::internal("work_update_task", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_update_task", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_delete_task"))]
pub async fn work_delete_task(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .delete_task(&id)
            .map_err(|e| CommandError::internal("work_delete_task", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_delete_task", &anyhow::anyhow!("{e}")))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityRequest {
    pub title: String,
    pub due_at: Option<i64>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "work_recommend_priority"))]
pub async fn work_recommend_priority(
    state: State<'_, AppState>,
    request: PriorityRequest,
) -> Result<i32, CommandError> {
    let _ = state;
    Ok(work_ops::recommend_priority(&request.title, request.due_at))
}

#[tauri::command]
#[instrument(skip(state, transcript), fields(otel.kind = "work_summarise_meeting"))]
pub async fn work_summarise_meeting(
    state: State<'_, AppState>,
    transcript: String,
) -> Result<work_ops::MeetingMinutes, CommandError> {
    let _ = state;
    Ok(work_ops::summarise_meeting(&transcript))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_start_timer"))]
pub async fn work_start_timer(
    state: State<'_, AppState>,
    id: String,
) -> Result<WorkTask, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .start_timer(&id)
            .map_err(|e| CommandError::internal("work_start_timer", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_start_timer", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_stop_timer"))]
pub async fn work_stop_timer(
    state: State<'_, AppState>,
) -> Result<Option<WorkTask>, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .stop_timer()
            .map_err(|e| CommandError::internal("work_stop_timer", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_stop_timer", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_add_time"))]
pub async fn work_add_time(
    state: State<'_, AppState>,
    id: String,
    elapsed_ms: i64,
) -> Result<WorkTask, CommandError> {
    let engine = state.work.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .add_time(&id, elapsed_ms)
            .map_err(|e| CommandError::internal("work_add_time", &e))
    })
    .await
    .map_err(|e| CommandError::internal("work_add_time", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "work_active_timer"))]
pub async fn work_active_timer(
    state: State<'_, AppState>,
) -> Result<Option<String>, CommandError> {
    Ok(state.work.active_timer())
}

// =====================================================================
// v0.5: editor commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "editor_workspace_root"))]
pub async fn editor_workspace_root(
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    Ok(state.editor.workspace_root().to_string_lossy().into_owned())
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "editor_read"))]
pub async fn editor_read(
    state: State<'_, AppState>,
    path: String,
) -> Result<editor_ops::FileContent, CommandError> {
    state
        .editor
        .read_file(&path)
        .map_err(|e| CommandError::validation("editor_read").with_details(e.to_string()))
}

#[tauri::command]
#[instrument(skip(state, content), fields(otel.kind = "editor_write"))]
pub async fn editor_write(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<editor_ops::FileContent, CommandError> {
    state
        .editor
        .write_file(&path, &content)
        .map_err(|e| CommandError::validation("editor_write").with_details(e.to_string()))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "editor_list"))]
pub async fn editor_list(
    state: State<'_, AppState>,
    max_depth: Option<usize>,
) -> Result<Vec<editor_ops::FileEntry>, CommandError> {
    state
        .editor
        .list_tree(max_depth)
        .map_err(|e| CommandError::internal("editor_list", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "git_status"))]
pub async fn git_status(
    state: State<'_, AppState>,
) -> Result<editor_ops::GitStatus, CommandError> {
    editor_ops::git_status(state.editor.workspace_root())
        .map_err(|e| CommandError::internal("git_status", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "git_log"))]
pub async fn git_log(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<editor_ops::GitLogEntry>, CommandError> {
    editor_ops::git_log(state.editor.workspace_root(), limit.unwrap_or(20))
        .map_err(|e| CommandError::internal("git_log", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "git_diff"))]
pub async fn git_diff(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<editor_ops::GitDiff, CommandError> {
    let p = path.unwrap_or_default();
    editor_ops::git_diff(state.editor.workspace_root(), &p)
        .map_err(|e| CommandError::internal("git_diff", &e))
}

#[tauri::command]
#[instrument(skip(state, message), fields(otel.kind = "git_commit"))]
pub async fn git_commit(
    state: State<'_, AppState>,
    message: String,
) -> Result<String, CommandError> {
    editor_ops::git_commit(state.editor.workspace_root(), &message)
        .map_err(|e| CommandError::validation("git_commit").with_details(e.to_string()))
}

// =====================================================================
// v0.5: OS commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "os_clipboard_read"))]
pub async fn os_clipboard_read(
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    state
        .clipboard
        .read_text()
        .map_err(|e| CommandError::internal("os_clipboard_read", &e))
}

#[tauri::command]
#[instrument(skip(state, text), fields(otel.kind = "os_clipboard_write"))]
pub async fn os_clipboard_write(
    state: State<'_, AppState>,
    text: String,
) -> Result<(), CommandError> {
    state
        .clipboard
        .write_text(&text)
        .map_err(|e| CommandError::internal("os_clipboard_write", &e))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellExecRequest {
    /// Either a parsed argv array or a single string to be split
    /// via `shell-words`.  Callers SHOULD prefer the array form.
    pub argv: Option<Vec<String>>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "os_shell_exec"))]
pub async fn os_shell_exec(
    state: State<'_, AppState>,
    request: ShellExecRequest,
) -> Result<os::ShellOutput, CommandError> {
    let argv: Vec<String> = if let Some(arr) = request.argv {
        arr
    } else if let Some(cmd) = request.command {
        os::parse_argv(&cmd)
            .map_err(|e| CommandError::validation("os_shell_exec").with_details(e.to_string()))?
    } else {
        return Err(CommandError::validation("os_shell_exec").with_details("argv or command is required".to_string()));
    };
    let cwd: Option<PathBuf> = request.cwd.map(PathBuf::from);
    let shell = state.shell.clone();
    let timeout = request.timeout_ms.map(std::time::Duration::from_millis);
    // v1.0.1 P0#3: `ShellExecutor::exec` is now `async` so the
    // timeout branch can `start_kill()` the child.  No more
    // `spawn_blocking`.
    let exec = if let Some(t) = timeout {
        (*shell).clone().with_timeout(t)
    } else {
        (*shell).clone()
    };
    exec.exec(argv, cwd.as_deref())
        .await
        .map_err(|e| CommandError::validation("os_shell_exec").with_details(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    pub level: Option<String>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "os_notify"))]
pub async fn os_notify(
    state: State<'_, AppState>,
    request: NotifyRequest,
) -> Result<(), CommandError> {
    let _ = state;
    let level = match request.level.as_deref() {
        Some("success") => NotificationLevel::Success,
        Some("warning") => NotificationLevel::Warning,
        Some("error") => NotificationLevel::Error,
        _ => NotificationLevel::Info,
    };
    let n = Notification {
        title: request.title,
        body: request.body,
        level,
    };
    os::send_notification(&n).map_err(|e| CommandError::internal("os_notify", &e))
}

// =====================================================================
// v0.5: sync (E2EE) commands
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakeIdentityResponse {
    pub public_key: String,
    pub secret_key: String,
}

#[tauri::command]
#[instrument(skip(), fields(otel.kind = "sync_make_identity"))]
pub async fn sync_make_identity() -> Result<MakeIdentityResponse, CommandError> {
    let id = E2eeIdentity::generate();
    Ok(MakeIdentityResponse {
        public_key: id.public_key_b64(),
        secret_key: base64::engine::general_purpose::STANDARD.encode(id.secret_bytes()),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptRequest {
    pub plaintext_b64: String,
    pub local_secret_b64: String,
    pub peer_public_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptResponse {
    pub envelope: EncryptedEnvelope,
    pub envelope_b64: String,
    pub fingerprint: String,
}

#[tauri::command]
#[instrument(skip(request), fields(otel.kind = "sync_encrypt"))]
pub async fn sync_encrypt(request: EncryptRequest) -> Result<EncryptResponse, CommandError> {
    let local = identity_from_secret_b64(&request.local_secret_b64)
        .map_err(|e| CommandError::validation("sync_encrypt").with_details(e.to_string()))?;
    let plaintext = base64::engine::general_purpose::STANDARD
        .decode(request.plaintext_b64.as_bytes())
        .map_err(|e| CommandError::validation("sync_encrypt").with_details(format!("plaintext: {e}")))?;
    let (env, fingerprint) = sync_ops::encrypt_for_peer(&local, &request.peer_public_b64, &plaintext)
        .map_err(|e| CommandError::validation("sync_encrypt").with_details(e.to_string()))?;
    let b64 = env
        .to_b64_json()
        .map_err(|e| CommandError::internal("sync_encrypt", &e))?;
    Ok(EncryptResponse { envelope: env, envelope_b64: b64, fingerprint })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptRequest {
    pub envelope: EncryptedEnvelope,
    pub local_secret_b64: String,
    pub peer_public_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptResponse {
    pub plaintext_b64: String,
}

#[tauri::command]
#[instrument(skip(request), fields(otel.kind = "sync_decrypt"))]
pub async fn sync_decrypt(request: DecryptRequest) -> Result<DecryptResponse, CommandError> {
    let local = identity_from_secret_b64(&request.local_secret_b64)
        .map_err(|e| CommandError::validation("sync_decrypt").with_details(e.to_string()))?;
    let pair = Pair::new(local, &request.peer_public_b64)
        .map_err(|e| CommandError::validation("sync_decrypt").with_details(e.to_string()))?;
    let pt = pair
        .decrypt(&request.envelope)
        .map_err(|e| CommandError::validation("sync_decrypt").with_details(e.to_string()))?;
    Ok(DecryptResponse {
        plaintext_b64: base64::engine::general_purpose::STANDARD.encode(&pt),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendSealedRequest {
    pub plaintext_b64: String,
    pub local_secret_b64: String,
    pub peer_public_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendSealedResponse {
    pub envelope_id: String,
    pub fingerprint: String,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "sync_send"))]
pub async fn sync_send(
    state: State<'_, AppState>,
    request: SendSealedRequest,
) -> Result<SendSealedResponse, CommandError> {
    let local = identity_from_secret_b64(&request.local_secret_b64)
        .map_err(|e| CommandError::validation("sync_send").with_details(e.to_string()))?;
    let pair = Pair::new(local, &request.peer_public_b64)
        .map_err(|e| CommandError::validation("sync_send").with_details(e.to_string()))?;
    let pt = base64::engine::general_purpose::STANDARD
        .decode(request.plaintext_b64.as_bytes())
        .map_err(|e| CommandError::validation("sync_send").with_details(format!("plaintext: {e}")))?;
    let transport = state.sync_transport.clone();
    let fingerprint = pair.fingerprint.clone();
    let id = tokio::task::spawn_blocking(move || {
        sync_ops::send_sealed(&transport, &pair, &pt)
            .map_err(|e| CommandError::internal("sync_send", &e))
    })
    .await
    .map_err(|e| CommandError::internal("sync_send", &anyhow::anyhow!("{e}")))??;
    Ok(SendSealedResponse { envelope_id: id, fingerprint })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecvRequest {
    pub local_secret_b64: String,
    pub peer_public_b64: String,
    pub ack: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecvResponse {
    pub messages: Vec<sync_ops::InboxMessage>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "sync_recv"))]
pub async fn sync_recv(
    state: State<'_, AppState>,
    request: RecvRequest,
) -> Result<RecvResponse, CommandError> {
    let _ = state;
    let _ = request;
    // Implementation note: in v0.5 the actual decryption happens
    // in the front-end (we hand back the encrypted envelopes
    // because the local_secret_b64 is a session-only secret and
    // we don't want to round-trip it through the Tauri command
    // boundary on every poll).  The server-side helper exists for
    // future server-side decryption when a service-worker style
    // background process takes over.
    Err(CommandError::internal(
        "sync_recv",
        &anyhow::anyhow!("sync_recv is implemented in the front-end; use sync_decrypt on each envelope"),
    ))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "sync_ack"))]
pub async fn sync_ack(
    state: State<'_, AppState>,
    envelope_id: String,
) -> Result<bool, CommandError> {
    let transport = state.sync_transport.clone();
    tokio::task::spawn_blocking(move || {
        transport
            .ack(&envelope_id)
            .map_err(|e| CommandError::internal("sync_ack", &e))
    })
    .await
    .map_err(|e| CommandError::internal("sync_ack", &anyhow::anyhow!("{e}")))?
}

// ---------------------------------------------------------------------------
// v1.1 P0-2: Tool registry commands
// ---------------------------------------------------------------------------

/// Tool descriptor for the front-end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

/// Tauri command: list all registered tools.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "tool_list"))]
pub async fn tool_list(
    state: State<'_, AppState>,
) -> Result<Vec<ToolDescriptor>, CommandError> {
    let tools = state.tool_registry.list_all();
    Ok(tools
        .into_iter()
        .map(|(name, description, schema)| ToolDescriptor {
            name,
            description,
            schema,
        })
        .collect())
}

/// Tauri command: invoke a registered tool by name with arguments.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "tool_invoke"))]
pub async fn tool_invoke(
    state: State<'_, AppState>,
    tool_name: String,
    arguments: serde_json::Value,
) -> Result<ToolOutput, CommandError> {
    let input = ToolInput { tool_name, arguments };
    state
        .tool_registry
        .invoke(input)
        .map_err(|e| CommandError::internal("tool_invoke", &e))
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn identity_from_secret_b64(b64: &str) -> anyhow::Result<E2eeIdentity> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;
    let bytes = B64.decode(b64.as_bytes())?;
    if bytes.len() != 32 {
        anyhow::bail!("secret must be 32 bytes, got {}", bytes.len());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(E2eeIdentity::from_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_error_validation_includes_code_and_safe_message() {
        let e = CommandError::validation("empty user_message");
        assert_eq!(e.code, ErrorCode::Validation);
        assert!(e.message.contains("empty user_message"));
        assert!(format!("{e}").contains("validation"));
    }

    #[test]
    fn command_error_internal_hides_internal_details() {
        let internal = anyhow::anyhow!("DB at /home/alice/.nine_snake/secret.db blew up");
        let e = CommandError::internal("memory_store", &internal);
        assert!(!e.message.contains("/home/alice"));
        assert!(!e.message.contains("secret"));
        assert_eq!(e.code, ErrorCode::Internal);
    }

    #[test]
    fn command_error_from_anyhow_is_internal() {
        let e: CommandError = anyhow::anyhow!("boom").into();
        assert_eq!(e.code, ErrorCode::Internal);
    }

    #[test]
    fn command_error_not_found() {
        let e = CommandError::not_found("memory");
        assert_eq!(e.code, ErrorCode::NotFound);
        assert!(e.message.contains("memory"));
    }

    #[test]
    fn command_error_memory_uses_memory_code() {
        let e = CommandError::memory("sponge", &anyhow::anyhow!("x"));
        assert_eq!(e.code, ErrorCode::Memory);
    }

    #[test]
    fn command_error_llm_uses_llm_code() {
        let e = CommandError::llm("chat", &anyhow::anyhow!("x"));
        assert_eq!(e.code, ErrorCode::Llm);
    }

    #[test]
    fn command_error_lance_uses_lance_code() {
        let e = CommandError::lance("search", &anyhow::anyhow!("x"));
        assert_eq!(e.code, ErrorCode::Lance);
    }

    #[test]
    fn command_error_db_uses_db_code() {
        let e = CommandError::db("open", &anyhow::anyhow!("x"));
        assert_eq!(e.code, ErrorCode::Db);
    }

    #[test]
    fn command_error_swarm_uses_swarm_code() {
        let e = CommandError::swarm("orchestrate", &anyhow::anyhow!("x"));
        assert_eq!(e.code, ErrorCode::Swarm);
    }

    #[test]
    fn swarm_agent_kind_parses_known_values() {
        use crate::swarm::agents::AgentKind;
        assert_eq!("coder".parse::<AgentKind>().unwrap(), AgentKind::Coder);
        assert_eq!("writer".parse::<AgentKind>().unwrap(), AgentKind::Writer);
        assert_eq!("reviewer".parse::<AgentKind>().unwrap(), AgentKind::Reviewer);
        assert!("unknown".parse::<AgentKind>().is_err());
    }
}

// ---------------------------------------------------------------------------
// v1.2/v1.3: Commands that were registered in generate_handler! but not yet
// implemented. Minimal stubs — replace with real implementations as needed.
// ---------------------------------------------------------------------------

/// Stub: import skill from external registry (v1.2 eco compatibility).
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_import"))]
pub async fn skill_import(
    state: State<'_, AppState>,
    source: String,
    identifier: String,
) -> Result<crate::skills::importer::ImportResult, CommandError> {
    let source = match source.as_str() {
        "agentskills" => crate::skills::importer::SkillSource::AgentskillsIo,
        "clawhub" => crate::skills::importer::SkillSource::ClawHub,
        "teamskillshub" => crate::skills::importer::SkillSource::TeamSkillsHub,
        other => return Err(CommandError::validation("skill_import").with_details(format!("unknown source: {other}"))),
    };
    let importer = crate::skills::importer::SkillImporter::new(
        state.skills.store().clone(),
    );
    let result = match source {
        crate::skills::importer::SkillSource::AgentskillsIo => importer.import_from_url(&identifier).await,
        crate::skills::importer::SkillSource::ClawHub => importer.import_from_clawhub(&identifier).await,
        crate::skills::importer::SkillSource::TeamSkillsHub => importer.import_from_teamskillshub(&identifier).await,
    };
    if result.success {
        Ok(result)
    } else {
        Err(CommandError::internal("skill_import", &anyhow::anyhow!("import failed")))
    }
}

/// v1.2: Get current status of the message bridge.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_status"))]
pub async fn channel_status(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, CommandError> {
    match &state.message_bridge {
        Some(bridge) => Ok(serde_json::json!({
            "connected": bridge.status().connected,
            "endpoint_url": bridge.status().endpoint_url,
            "messages_received": bridge.status().messages_received,
            "messages_sent": bridge.status().messages_sent,
        })),
        None => Ok(serde_json::json!({"connected": false, "channels": []})),
    }
}

/// v1.2: Send a message through the message bridge.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_send"))]
pub async fn channel_send(
    state: State<'_, AppState>,
    target: String,
    text: String,
) -> Result<bool, CommandError> {
    match &state.message_bridge {
        Some(bridge) => {
            let req = crate::channel::types::ChannelSendRequest {
                session_id: target.clone(),
                channel: crate::channel::types::Channel::Web,
                body: text,
                conversation_id: None,
            };
            bridge.send(&req).await
                .map(|_| true)
                .map_err(|e| CommandError::internal("channel_send", &anyhow::anyhow!("{e}")))
        }
        None => Err(CommandError::internal("channel_send", &anyhow::anyhow!("message bridge not configured"))),
    }
}

/// v1.2: Poll the message bridge for new messages.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_poll"))]
pub async fn channel_poll(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, CommandError> {
    match &state.message_bridge {
        Some(bridge) => Ok(bridge.poll().await.into_iter().map(|m| serde_json::to_value(&m).unwrap_or_default()).collect()),
        None => Ok(Vec::new()),
    }
}

/// v1.2: Ping the message bridge.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "channel_ping"))]
pub async fn channel_ping(
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    match &state.message_bridge {
        Some(bridge) => Ok(bridge.ping().await),
        None => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// v1.3 P1-3/P1-4: security + sandbox.
// ---------------------------------------------------------------------------

/// Full injection scan of arbitrary input.
#[tauri::command]
#[instrument(fields(otel.kind = "injection_scan"))]
pub async fn injection_scan(
    input: String,
) -> Result<crate::security::InjectionScanResult, CommandError> {
    Ok(crate::security::full_injection_scan(&input))
}

/// Retrieve sandbox configuration for a skill.
#[tauri::command]
#[instrument(fields(otel.kind = "sandbox_config"))]
pub async fn sandbox_config(
    skill_id: String,
) -> Result<crate::skills::sandbox::SandboxConfig, CommandError> {
    let mut config = crate::skills::sandbox::SandboxConfig::default();
    config.capabilities = crate::skills::sandbox::CapabilitySet::llm_only();
    Ok(config)
}

// ---------------------------------------------------------------------------
// v1.3 P2-7: skill marketplace.
// ---------------------------------------------------------------------------

/// Search the skill marketplace.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_search"))]
pub async fn marketplace_search(
    state: State<'_, AppState>,
    query: crate::skills::marketplace::MarketplaceQuery,
) -> Result<crate::skills::marketplace::MarketplaceResponse, CommandError> {
    state.marketplace.search(&query).map_err(|e| CommandError::internal("marketplace_search", &e))
}

/// Quick search — top 10 results for autocomplete.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_quick_search"))]
pub async fn marketplace_quick_search(
    state: State<'_, AppState>,
    text: String,
) -> Result<crate::skills::marketplace::MarketplaceResponse, CommandError> {
    let q = crate::skills::marketplace::MarketplaceQuery {
        text: Some(text),
        limit: 10,
        ..Default::default()
    };
    state.marketplace.search(&q).map_err(|e| CommandError::internal("marketplace_quick_search", &e))
}

/// One-click install from remote registry.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_install"))]
pub async fn marketplace_install(
    state: State<'_, AppState>,
    source: String,
    identifier: String,
) -> Result<crate::skills::marketplace::SkillEntry, CommandError> {
    state.marketplace.install(&source, &identifier)
        .map_err(|e| CommandError::internal("marketplace_install", &e))
}

/// Check for skill updates.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_check_updates"))]
pub async fn marketplace_check_updates(
    state: State<'_, AppState>,
) -> Result<Vec<crate::skills::marketplace::UpdateInfo>, CommandError> {
    Ok(state.marketplace.check_updates())
}

/// Refresh marketplace index.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_refresh"))]
pub async fn marketplace_refresh(
    state: State<'_, AppState>,
) -> Result<crate::skills::marketplace::MarketplaceStats, CommandError> {
    state.marketplace.refresh()
        .map_err(|e| CommandError::internal("marketplace_refresh", &e))
}

/// Get marketplace stats.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_stats"))]
pub async fn marketplace_stats(
    state: State<'_, AppState>,
) -> Result<crate::skills::marketplace::MarketplaceStats, CommandError> {
    Ok(state.marketplace.stats())
}

/// Get all tags with frequencies.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_tags"))]
pub async fn marketplace_tags(
    state: State<'_, AppState>,
) -> Result<Vec<(String, usize)>, CommandError> {
    Ok(state.marketplace.all_tags())
}

/// Generate publish manifest for a skill.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_generate_manifest"))]
pub async fn marketplace_generate_manifest(
    state: State<'_, AppState>,
    skill_id: String,
) -> Result<crate::skills::marketplace::PublishManifest, CommandError> {
    state.marketplace.generate_manifest(&skill_id)
        .map_err(|e| CommandError::internal("marketplace_generate_manifest", &e))
}

// =====================================================================
// v1.3: DID identity commands
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateDidResponse {
    pub did: String,
    pub public_key_b64: String,
    pub document: crate::identity::DidDocument,
}

#[tauri::command]
#[instrument(fields(otel.kind = "generate_did"))]
pub async fn generate_did(
    public_key_b64: Option<String>,
) -> Result<GenerateDidResponse, CommandError> {
    let did_key = match public_key_b64 {
        Some(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .map_err(|e| CommandError::validation("generate_did").with_details(format!("invalid base64: {e}")))?;
            if bytes.len() != 32 {
                return Err(CommandError::validation("generate_did").with_details(
                    format!("public key must be 32 bytes, got {}", bytes.len())
                ));
            }
            let mut pk = [0u8; 32];
            pk.copy_from_slice(&bytes);
            crate::identity::DidKey::from_public_key(&pk)
        }
        None => {
            let mut pk = [0u8; 32];

            getrandom::getrandom(&mut pk)
                .map_err(|e| CommandError::internal("generate_did", &anyhow::anyhow!("{e}")))?;
            crate::identity::DidKey::from_public_key(&pk)
        }
    };
    let document = crate::identity::DidDocument::from_did_key(&did_key);
    Ok(GenerateDidResponse {
        did: did_key.did.clone(),
        public_key_b64: did_key.public_key_b64(),
        document,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveDidResponse {
    pub did: String,
    pub document: crate::identity::DidDocument,
}

#[tauri::command]
#[instrument(fields(otel.kind = "resolve_did"))]
pub async fn resolve_did(
    did: String,
) -> Result<ResolveDidResponse, CommandError> {
    let did_key = crate::identity::DidKey::parse(&did)
        .map_err(|e| CommandError::validation("resolve_did").with_details(e.to_string()))?;
    let document = crate::identity::DidDocument::from_did_key(&did_key);
    Ok(ResolveDidResponse {
        did: did_key.did,
        document,
    })
}

// =====================================================================
// v1.3: Skill audit log commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_audit_list"))]
pub async fn skill_audit_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<crate::skills::audit::SkillAuditEntry>, CommandError> {
    let logger = state.skill_audit_logger.clone();
    tokio::task::spawn_blocking(move || {
        logger.list(limit.unwrap_or(50))
            .map_err(|e| CommandError::db("skill_audit_list", &e))
    })
    .await
    .map_err(|e| CommandError::internal("skill_audit_list", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_audit_list_for_skill"))]
pub async fn skill_audit_list_for_skill(
    state: State<'_, AppState>,
    skill_id: String,
    limit: Option<usize>,
) -> Result<Vec<crate::skills::audit::SkillAuditEntry>, CommandError> {
    let logger = state.skill_audit_logger.clone();
    tokio::task::spawn_blocking(move || {
        logger.list_for_skill(&skill_id, limit.unwrap_or(50))
            .map_err(|e| CommandError::db("skill_audit_list_for_skill", &e))
    })
    .await
    .map_err(|e| CommandError::internal("skill_audit_list_for_skill", &anyhow::anyhow!("{e}")))?
}

// =====================================================================
// v1.3: chat_stream command
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "chat_stream"))]
pub async fn chat_stream(
    state: State<'_, AppState>,
    request: ChatRequestDto,
) -> Result<Vec<crate::llm::StreamToken>, CommandError> {
    let scan = crate::security::injection_guard::full_injection_scan(&request.user_message);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            return Err(CommandError::validation("chat_stream").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string()
            ));
        }
    }

    let mut msgs: Vec<ChatMessage> = Vec::new();
    if let Some(sys) = request.system.as_deref() {
        msgs.push(ChatMessage::system(sys));
    }
    msgs.push(ChatMessage::user(request.user_message));

    let stream = state.llm.chat_stream(msgs);
    use futures::StreamExt;
    let tokens: Vec<crate::llm::StreamToken> = stream
        .filter_map(|r| async move { r.ok() })
        .collect()
        .await;
    Ok(tokens)
}

// =====================================================================
// v1.3: Data export/import commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "export_memories"))]
pub async fn export_memories(
    state: State<'_, AppState>,
    format: String,
    path: String,
) -> Result<crate::memory::export::ExportManifest, CommandError> {
    let exporter = crate::memory::export::DataExporter::new((*state.sqlite).clone());
    let p = std::path::PathBuf::from(&path);
    match format.as_str() {
        "jsonld" | "json-ld" => {
            tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(exporter.export_jsonld(&p))
            })
            .await
            .map_err(|e| CommandError::internal("export_memories", &anyhow::anyhow!("{e}")))?
            .map_err(|e| CommandError::internal("export_memories", &e))
        }
        _ => Err(CommandError::validation("export_memories").with_details(
            format!("unsupported format: {format}"),
        )),
    }
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "import_memories"))]
pub async fn import_memories(
    state: State<'_, AppState>,
    path: String,
) -> Result<crate::memory::export::ImportResult, CommandError> {
    let exporter = crate::memory::export::DataExporter::new((*state.sqlite).clone());
    let p = std::path::PathBuf::from(&path);
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(exporter.import_jsonld(&p))
    })
    .await
    .map_err(|e| CommandError::internal("import_memories", &anyhow::anyhow!("{e}")))?
    .map_err(|e| CommandError::internal("import_memories", &e))
}

// =====================================================================
// v1.3: Device management commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "list_devices"))]
pub async fn list_devices(
    state: State<'_, AppState>,
) -> Result<Vec<crate::sync::device_manager::DeviceInfo>, CommandError> {
    Ok(state.device_manager.lock().list_devices()
        .map_err(|e| CommandError::internal("list_devices", &e))?)
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "revoke_device"))]
pub async fn revoke_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<bool, CommandError> {
    let result = state.device_manager.lock().revoke_device(&device_id);
    Ok(result.success)
}

// =====================================================================
// v1.3: WebChat share link commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "share_chat"))]
pub async fn share_chat(
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    Ok(state.webchat_service.create_session())
}

// =====================================================================
// v1.3: ACL commands
// =====================================================================

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "acl_set"))]
pub async fn acl_set(
    state: State<'_, AppState>,
    principal: String,
    resource: String,
    permission: String,
    effect: String,
) -> Result<bool, CommandError> {
    let id = uuid::Uuid::new_v4().to_string();
    state.sqlite.insert_acl(&id, &principal, &resource, &permission, &effect)
        .map(|_| true)
        .map_err(|e| CommandError::db("acl_set", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "acl_list"))]
pub async fn acl_list(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String, String, String, String)>, CommandError> {
    state.sqlite.list_acl()
        .map_err(|e| CommandError::db("acl_list", &e))
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "acl_remove"))]
pub async fn acl_remove(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    state.sqlite.remove_acl(&id)
        .map(|_| true)
        .map_err(|e| CommandError::db("acl_remove", &e))
}

// =====================================================================
// v1.3: MCP commands (feature-gated)
// =====================================================================

#[cfg(feature = "mcp")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_list_servers"))]
pub async fn mcp_list_servers(
    state: State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    Ok(state.mcp_manager.list_servers())
}

#[cfg(feature = "mcp")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_add_server"))]
pub async fn mcp_add_server(
    state: State<'_, AppState>,
    config: crate::mcp::config::McpServerConfig,
) -> Result<bool, CommandError> {
    state.mcp_manager.add_server(config);
    Ok(true)
}

#[cfg(feature = "mcp")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_remove_server"))]
pub async fn mcp_remove_server(
    state: State<'_, AppState>,
    name: String,
) -> Result<bool, CommandError> {
    state.mcp_manager.remove_server(&name);
    Ok(true)
}

#[cfg(feature = "mcp")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "mcp_list_tools"))]
pub async fn mcp_list_tools(
    state: State<'_, AppState>,
) -> Result<Vec<crate::mcp::client::McpTool>, CommandError> {
    Ok(state.mcp_manager.list_all_tools().await)
}
