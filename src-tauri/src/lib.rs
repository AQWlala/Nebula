//! `nine_snake_lib` — the library crate backing the `nine-snake` binary.
//!
//! The crate is organised as a small collection of mostly independent
//! subsystems that communicate through a shared [`AppState`] living inside
//! the Tauri managed-state container:
//!
//! * [`memory`]   — the 8-layer v7.0 memory system
//! * [`llm`]      — model gateway (Ollama + optional remote fallback)
//! * [`swarm`]    — multi-agent orchestration
//! * [`api`]      — internal Rust-side service trait surface
//! * [`commands`] — the Tauri command handlers exposed to the front-end
//! * [`metrics`]  — process-wide atomic counters
//! * [`grpc`]     — v0.3: optional gRPC server (22 RPCs, tonic 0.12)
//! * [`skills`]   — v0.3: skill CRUD + execution engine
//!
//! The public surface intentionally re-exports a few well-known types so
//! downstream crates (and the binary) don't have to memorise module paths.

pub mod api;
pub mod channel;
pub mod commands;
pub mod editor;
pub mod error_ui;
pub mod llm;
pub mod memory;
pub mod metrics;
pub mod os;
pub mod perf;
// v0.5: OS keychain + sensitive-data redaction.
pub mod security;
pub mod skills;
pub mod swarm;
pub mod sync;
pub mod identity;
// v1.3: MCP protocol client (feature-gated).
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod work;
pub mod writing;
// v1.1 P0-2: Tool abstraction layer.
pub mod tools;

// v1.3: closed-loop self-evolution (task outcomes + skill archive +
// prompt mutator).  Off by default; enable with
// `--features self-evolution`.
#[cfg(feature = "self-evolution")]
pub mod evolution;

#[cfg(feature = "grpc")]
pub mod grpc;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use parking_lot::Mutex;
use tauri::Manager;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

use crate::editor::EditorState;
use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::OllamaClient;
use crate::memory::blackhole::BlackholeEngine;
use crate::memory::embedder::Embedder;
use crate::memory::lance_store::LanceStore;
use crate::memory::reflect::{ReflectConfig, ReflectionEngine};
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::sponge::SpongeEngine;
use crate::os::ClipboardService;
use crate::os::ShellExecutor;
use crate::perf::StartupTimer;
use crate::skills::audit::SkillAuditLogger;
use crate::skills::engine::SkillEngine;
use crate::skills::extractor::SkillExtractor;
use crate::skills::importer::SkillImporter;
use crate::skills::store::SkillStore;
use crate::swarm::composer::SkillComposer;
use crate::swarm::orchestrator::SwarmOrchestrator;
use crate::channel::bridge::MessageBridge;
use crate::channel::webchat::WebChatService;
use crate::sync::device_manager::DeviceManager;
use crate::sync::LocalTransport;
use crate::tools::{shell_tool::ShellTool, ToolRegistry};
use crate::work::WorkEngine;
use crate::writing::WritingEngine;

/// Configuration sourced from environment variables (with sensible defaults).
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Path to the SQLite database file.
    pub db_path: String,
    /// Path to the LanceDB vector store directory.
    pub lance_path: String,
    /// Base URL of the Ollama HTTP server.
    pub ollama_url: String,
    /// Default chat model name served by Ollama.
    pub chat_model: String,
    /// Default embedding model name served by Ollama.
    pub embed_model: String,
    /// Optional remote fallback URL (e.g. OpenAI-compatible endpoint).
    pub remote_fallback_url: Option<String>,
    /// Number of days of inactivity before the black-hole engine may compress.
    pub blackhole_threshold_days: u32,
    /// Embedding vector dimensionality.
    pub embedding_dim: usize,
    /// v0.2: background reflection worker period in seconds. `0`
    /// disables the worker.
    pub reflect_interval_secs: u64,
    /// v0.2: reflection window size in days.
    pub reflect_window_days: i64,
    /// v0.2: minimum source-memory importance for reflection.
    pub reflect_min_importance: f32,
    /// v0.3: enable the in-process gRPC server. Default `true` (set
    /// `NINE_SNAKE_GRPC=0` to disable). The gRPC port is configured
    /// via `grpc_bind_addr`.
    pub grpc_enabled: bool,
    /// v0.3: bind address for the gRPC server. Default
    /// `127.0.0.1:50051`.
    pub grpc_bind_addr: String,
    /// v0.5: workspace root for the editor.  All file operations
    /// are sandboxed to this directory.
    pub editor_workspace: String,
    /// v0.5: directory used by the local sync transport.  Defaults
    /// to `<config_dir>/sync_inbox`.
    pub sync_inbox: String,
}

impl AppConfig {
    /// Loads configuration from environment variables, falling back to
    /// defaults appropriate for a first-run local development setup.
    pub fn from_env() -> Self {
        Self {
            db_path: std::env::var("NINE_SNAKE_DB")
                .unwrap_or_else(|_| "nine_snake.db".to_string()),
            lance_path: std::env::var("NINE_SNAKE_LANCE")
                .unwrap_or_else(|_| "nine_snake_lance".to_string()),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string()),
            chat_model: std::env::var("NINE_SNAKE_CHAT_MODEL")
                .unwrap_or_else(|_| "qwen2.5:3b".to_string()),
            embed_model: std::env::var("NINE_SNAKE_EMBED_MODEL")
                .unwrap_or_else(|_| "BAAI/bge-small-zh-v1.5".to_string()),
            remote_fallback_url: std::env::var("NINE_SNAKE_REMOTE_URL").ok(),
            blackhole_threshold_days: std::env::var("NINE_SNAKE_BH_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            embedding_dim: std::env::var("NINE_SNAKE_EMBED_DIM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(512),
            reflect_interval_secs: std::env::var("NINE_SNAKE_REFLECT_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_INTERVAL_SECS),
            reflect_window_days: std::env::var("NINE_SNAKE_REFLECT_WINDOW_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_WINDOW_DAYS),
            reflect_min_importance: std::env::var("NINE_SNAKE_REFLECT_MIN_IMPORTANCE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_MIN_IMPORTANCE),
            grpc_enabled: std::env::var("NINE_SNAKE_GRPC")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            grpc_bind_addr: std::env::var("NINE_SNAKE_GRPC_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:50051".to_string()),
            editor_workspace: std::env::var("NINE_SNAKE_WORKSPACE")
                .unwrap_or_else(|_| ".".to_string()),
            sync_inbox: std::env::var("NINE_SNAKE_SYNC_INBOX")
                .unwrap_or_else(|_| "sync_inbox".to_string()),
        }
    }
}

/// The single managed-state struct shared across Tauri commands.
///
/// Cloning the handle is cheap: every field is an `Arc` or itself clonable.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub sqlite: Arc<SqliteStore>,
    pub lance: Arc<LanceStore>,
    pub embedder: Arc<Embedder>,
    pub llm: Arc<LlmGateway>,
    pub sponge: Arc<SpongeEngine>,
    pub blackhole: Arc<BlackholeEngine>,
    pub swarm: Arc<SwarmOrchestrator>,
    /// v0.2: L5 reflection engine. Always present (even if LLM is
    /// unavailable, in which case the engine falls back to template
    /// synthesis).
    pub reflection: Arc<ReflectionEngine>,
    /// v0.3: skill CRUD + execution engine.
    pub skills: Arc<SkillEngine>,
    /// v1.2: skill closed-loop learning 鈥?auto-extracts reusable skills from swarm tasks.
    pub skill_extractor: Arc<SkillExtractor>,
    /// v1.2: skill auto-composer for orchestration upgrade.
    pub skill_composer: Arc<SkillComposer>,
    /// v1.3 P2-7: skill marketplace — search, install, update, publish.
    pub marketplace: Arc<skills::SkillMarketplace>,
    /// v1.3: skill audit logger.
    pub skill_audit_logger: Arc<SkillAuditLogger>,
    /// v1.2: multi-channel message bridge (JiWenSwarm delivery fabric).
    pub message_bridge: Option<Arc<MessageBridge>>,
    /// v0.3: handle to the reflection background worker, so the
    /// `AppState::shutdown` call can `await` the join (rather than
    /// dropping a `JoinHandle` into a static `OnceCell` as v0.2 did).
    pub reflect_worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// v0.3: handle to the in-process gRPC server task.
    #[cfg(feature = "grpc")]
    pub grpc_server: Arc<Mutex<Option<grpc::GrpcHandle>>>,
    /// v0.5: writing engine (long-form documents + template library).
    pub writing: Arc<WritingEngine>,
    /// v0.5: work engine (kanban + time tracking).
    pub work: Arc<WorkEngine>,
    /// v0.5: editor state (file ops, watcher, git).
    pub editor: EditorState,
    /// v0.5: clipboard service.
    pub clipboard: ClipboardService,
    /// v0.5: shell executor with whitelist.
    pub shell: Arc<ShellExecutor>,
    /// v0.5: local sync transport.
    pub sync_transport: Arc<LocalTransport>,
    /// v1.0: startup profiler (milestones + final report).
    pub startup_timer: StartupTimer,
    /// v1.1 P0-2: tool registry with registered tools (shell, etc.).
    pub tool_registry: Arc<ToolRegistry>,
    /// v1.3: WebChat share link service.
    pub webchat_service: WebChatService,
    /// v1.3: device manager for sync pairing.
    pub device_manager: Arc<parking_lot::Mutex<DeviceManager>>,
    /// v1.3: MCP manager (feature-gated).
    #[cfg(feature = "mcp")]
    pub mcp_manager: crate::mcp::client::McpManager,
}

impl AppState {
    /// Bootstraps a fully-wired [`AppState`] from the given config.
    ///
    /// On failure all already-initialised subsystems are dropped; the
    /// returned `anyhow::Error` carries the full context chain.
    pub async fn bootstrap(config: AppConfig) -> anyhow::Result<Self> {
        info!(target: "nine_snake", "bootstrapping app state");

        // v1.0: startup profiler marks each milestone so the
        // front-end (and CI) can audit the cold-start budget.
        let startup = StartupTimer::start();
        startup.mark("bootstrap.start");

        // 1. SQLite — apply migrations synchronously on a blocking thread.
        let db_path = config.db_path.clone();
        let sqlite = tokio::task::spawn_blocking(move || SqliteStore::open(&db_path))
            .await
            .context("spawn_blocking for sqlite open failed")?
            .context("opening sqlite store")?;
        let sqlite = Arc::new(sqlite);
        startup.mark("bootstrap.sqlite");

        // 1b. v0.2 — run the migration runner (idempotent; 002+).
        let sqlite_for_migrations = sqlite.clone();
        let migrations_dir = crate::memory::migration::bundled_migrations_dir().to_path_buf();
        let applied = tokio::task::spawn_blocking(move || {
            let conn = sqlite_for_migrations.raw_connection();
            let conn = conn.lock();
            crate::memory::migration::run_migrations(&conn, &migrations_dir)
        })
        .await
        .context("spawn_blocking for migrations failed")?
        .context("running migrations")?;
        if !applied.is_empty() {
            info!(
                target: "nine_snake",
                count = applied.len(),
                last = applied.last().map(|m| m.version).unwrap_or(0),
                "applied v0.2 migrations"
            );
        }
        startup.mark("bootstrap.migrations");

        // 2. LanceDB vector store.
        let lance = Arc::new(
            LanceStore::open(&config.lance_path, config.embedding_dim)
                .await
                .context("opening lance store")?,
        );
        startup.mark("bootstrap.lance");

        // 3. Embedder (Ollama HTTP client).
        let embedder = Arc::new(Embedder::new(
            OllamaClient::new(config.ollama_url.clone()),
            config.embed_model.clone(),
            config.embedding_dim,
        ));

        // 4. LLM gateway (chat/generate).
        let ollama = Arc::new(OllamaClient::new(config.ollama_url.clone()));
        // v1.1 P0-1: Anthropic fallback is enabled when NINE_SNAKE_ANTHROPIC_KEY is set.
        let anthropic_key = std::env::var("NINE_SNAKE_ANTHROPIC_KEY").ok();
        let anthropic_model = std::env::var("NINE_SNAKE_ANTHROPIC_MODEL").ok();
        let llm = Arc::new(LlmGateway::new(
            ollama,
            config.chat_model.clone(),
            config.remote_fallback_url.clone(),
            anthropic_key,
            anthropic_model,
        ));
        startup.mark("bootstrap.llm");

        // 5. Sponge (absorption) and Black-hole (compression) engines.
        let lance_for_sponge = lance.clone();
        let sqlite_for_sponge = sqlite.clone();
        let embedder_for_sponge = embedder.clone();
        let sponge = Arc::new(SpongeEngine::new(
            sqlite_for_sponge,
            lance_for_sponge,
            embedder_for_sponge,
        ));

        let blackhole = Arc::new(BlackholeEngine::new(
            sqlite.clone(),
            lance.clone(),
            config.blackhole_threshold_days,
        ));

        // 6. Swarm orchestrator (with RAG support via lance + embedder).
        let swarm = Arc::new(SwarmOrchestrator::new(
            llm.clone(),
            sponge.clone(),
            lance.clone(),
            embedder.clone(),
            sqlite.clone(),
        ));

        // 7. v0.2 — reflection engine (L5). `llm = Some(...)` keeps the
        //    LLM path enabled; the engine falls back to a template when
        //    the gateway returns an error.
        let reflection_cfg = ReflectConfig {
            window_days: config.reflect_window_days,
            min_importance: config.reflect_min_importance,
            worker_interval_secs: config.reflect_interval_secs,
            ..ReflectConfig::default()
        };
        let reflection = Arc::new(ReflectionEngine::new(
            sqlite.clone(),
            Some(llm.clone()),
            reflection_cfg,
        ));

        // 8. v0.3 — skill store (shared between engine and extractor).
        let skill_store = Arc::new(
            SkillStore::new((*sqlite).clone())
                .expect("SkillStore::new must succeed when migrations have been run"),
        );

        // 8a-audit. v1.3 — skill audit logger (created before engine so it can be wired in).
        let skill_audit_logger = Arc::new(SkillAuditLogger::new(sqlite.raw_connection()));

        // 8a. v0.3 — skill engine (uses shared store + audit logger).
        let skill_engine = SkillEngine::from_store(
            (*skill_store).clone(),
            llm.clone(),
        ).with_audit(skill_audit_logger.clone());
        let skills = Arc::new(skill_engine);

        // 8b. v1.2 — skill auto-extractor for closed-loop learning.
        let skill_extractor = Arc::new(SkillExtractor::new(
            llm.clone(),
            skill_store.clone(),
            config.db_path
                .rsplit_once(std::path::MAIN_SEPARATOR)
                .map(|(d, _)| d)
                .unwrap_or(".")
                .to_string()
                + "/skills_archive",
        ));

        // 8c. v1.2 — skill composer for orchestration upgrade.
        let skill_composer = Arc::new(SkillComposer::new(
            skill_store.clone(),
            Some(llm.clone()),
        ));

        // 8d. v1.3 P2-7 — skill marketplace.
        let skill_importer = Arc::new(SkillImporter::new((*skill_store).clone()));
        let marketplace = Arc::new(skills::SkillMarketplace::new(
            skill_store.clone(),
            skill_importer,
        ));
        let _ = marketplace.refresh(); // build initial index


        // 9. v1.2 — message bridge (multi-channel through JiuwenSwarm).
        let bridge_url = std::env::var("NINE_SNAKE_BRIDGE_URL").unwrap_or_default();
        let message_bridge = MessageBridge::new(&bridge_url).map(Arc::new);
        if message_bridge.is_some() {
            info!(target: "nine_snake", bridge_url = %bridge_url, "message bridge initialised");

        // Wire composer to swarm orchestrator.
        swarm.set_composer(skill_composer.clone());
        }


        // 9. v0.5 — writing engine.
        let writing = Arc::new(WritingEngine::new(
            sqlite.clone(),
            Some(sponge.clone()),
        ));

        // 10. v0.5 — work engine.
        let work = Arc::new(WorkEngine::new(sqlite.clone()));

        // 11. v0.5 — editor state (file ops / watcher).
        let editor = EditorState::new(&config.editor_workspace)
            .unwrap_or_else(|e| {
                tracing::warn!(target: "nine_snake", error = ?e, workspace = %config.editor_workspace, "editor workspace unavailable; falling back to current dir");
                EditorState::new(".").expect("current dir is always a directory")
            });
        startup.mark("bootstrap.editor");

        // 12. v0.5 — clipboard.
        let clipboard = ClipboardService::new().unwrap_or_else(|e| {
            tracing::warn!(target: "nine_snake", error = ?e, "clipboard unavailable; using noop fallback");
            ClipboardService::noop()
        });

        // 13. v0.5 — shell executor with the default whitelist.
        let shell = Arc::new(ShellExecutor::new());

        // 14. v1.1 P0-2 — tool registry with registered tools.
        let tool_registry = Arc::new(ToolRegistry::new());
        tool_registry.register(Arc::new(ShellTool::new((*shell).clone())));

        // 15. v0.5 — local sync transport.
        let sync_transport = Arc::new(
            LocalTransport::new(&config.sync_inbox)
                .unwrap_or_else(|e| {
                    tracing::warn!(target: "nine_snake", error = ?e, inbox = %config.sync_inbox, "sync inbox unavailable; using temp dir");
                    let tmp = std::env::temp_dir().join("nine-snake-sync-inbox");
                    LocalTransport::new(&tmp).expect("temp dir always works")
                }),
        );
        startup.mark("bootstrap.end");

        let device_manager = Arc::new(parking_lot::Mutex::new(DeviceManager::new(sqlite.raw_connection())));

        Ok(Self {
            config: Arc::new(config),
            sqlite,
            lance,
            embedder,
            llm,
            sponge,
            blackhole,
            swarm,
            reflection,
            skills,
            writing,
            work,
            editor,
            clipboard,
            shell,
            sync_transport,
            // Worker + gRPC handles are installed at startup time
            // (in `run`) so that the same `AppState` instance can be
            // shared between the Tauri runtime and standalone
            // integration tests without spinning up network servers.
            reflect_worker: Arc::new(Mutex::new(None)),
            #[cfg(feature = "grpc")]
            grpc_server: Arc::new(Mutex::new(None)),
            startup_timer: startup,
            skill_extractor,
            skill_composer,
            marketplace,
            skill_audit_logger,
            message_bridge,
            tool_registry,
            webchat_service: WebChatService::new(),
            device_manager,
            #[cfg(feature = "mcp")]
            mcp_manager: crate::mcp::client::McpManager::new(),
        })
    }

    /// Wakes the background reflection worker, signals the gRPC
    /// server to stop, and awaits both joins with a brief grace
    /// period. Idempotent and safe to call from Tauri shutdown.
    pub async fn shutdown(&self) {
        let notify = self.reflection.shutdown_handle();
        notify.notify_waiters();

        // Take the worker handle out of the mutex and await it.
        let worker = { self.reflect_worker.lock().take() };
        if let Some(h) = worker {
            // Give the worker up to 250 ms to exit cleanly. We use a
            // timeout so a misbehaving worker can't deadlock the
            // shutdown path.
            match tokio::time::timeout(Duration::from_millis(250), h).await {
                Ok(_) => info!(target: "nine_snake", "reflection worker stopped"),
                Err(_) => warn!(target: "nine_snake", "reflection worker did not stop in time"),
            }
        }

        // Tear down the gRPC server (if any) gracefully.
        #[cfg(feature = "grpc")]
        {
            let grpc = { self.grpc_server.lock().take() };
            if let Some(h) = grpc {
                h.shutdown().await;
            }
        }
    }
}

/// Installs the `tracing` subscriber. Safe to call multiple times.
///
/// v0.2: writes structured JSON to stdout when the
/// `NINE_SNAKE_LOG_FORMAT=json` environment variable is set; the
/// default remains human-readable pretty output.
///
/// v1.0: when `NINE_SNAKE_LOG_DIR` is set we also write a
/// daily-rotated JSON log file via `tracing_appender`.  This is
/// what the user-facing "Open logs folder" menu points at.
pub fn init_tracing() {
    static INIT: once_cell::sync::OnceCell<()> = once_cell::sync::OnceCell::new();
    INIT.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,nine_snake=debug"));
        let use_json = std::env::var("NINE_SNAKE_LOG_FORMAT")
            .map(|v| v.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if let Ok(dir) = std::env::var("NINE_SNAKE_LOG_DIR") {
            // v1.0: also append to a daily-rotated file.  We use
            // a non-blocking guard so a slow disk cannot stall the
            // Tauri command loop.
            let appender = tracing_appender::rolling::daily(&dir, "nine-snake.log");
            let (nb, _guard) = tracing_appender::non_blocking(appender);
            let _ = Box::leak(Box::new(_guard));
            if use_json {
                let _ = fmt()
                    .with_env_filter(filter.clone())
                    .json()
                    .with_writer(nb)
                    .try_init();
            } else {
                let _ = fmt()
                    .with_env_filter(filter.clone())
                    .with_writer(nb)
                    .try_init();
            }
        } else if use_json {
            let _ = fmt().with_env_filter(filter).json().try_init();
        } else {
            let _ = fmt().with_env_filter(filter).try_init();
        }
    });
}

/// Tauri application entry — builds the runtime, wires commands, runs the app.
pub fn run() {
    init_tracing();
    info!(target: "nine_snake", version = env!("CARGO_PKG_VERSION"), "starting nine-snake");

    let config = AppConfig::from_env();
    // v1.0.1 fix: `?expr` is the `Debug`-format shorthand that
    // `tracing` recognises only when the *argument* is a literal
    // expression.  `?config.db_path` is parsed as a single
    // identifier that starts with `?`, which the macro cannot
    // disambiguate.  Use the explicit form `db_path = ?db_path`.
    info!(target: "nine_snake", db_path = ?config.db_path, "loaded configuration");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        // v0.5: OS integration plugins.
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        // v0.5: autostart is opt-in at runtime; we only initialise
        // the plugin here so the user can toggle it from settings.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let handle = app.handle().clone();
            // Bootstrap state asynchronously so we don't block Tauri's main thread.
            //
            // v0.3 fix: when bootstrap fails, surface a user-facing
            // dialog and exit the application. The v0.2 behaviour of
            // "warn and continue" left the user with an uninitialised
            // app and no clue what to do.
            tauri::async_runtime::spawn(async move {
                match AppState::bootstrap(config.clone()).await {
                    Ok(state) => {
                        // Start the reflection background worker; the
                        // JoinHandle is parked on the AppState so
                        // shutdown can await it (see
                        // `AppState::shutdown`).
                        if let Some(h) = state.reflection.clone().spawn_worker() {
                            *state.reflect_worker.lock() = Some(h);
                            info!(target: "nine_snake", "reflection worker started");
                        }

                        // v0.3: optionally start the in-process gRPC
                        // server.
                        #[cfg(feature = "grpc")]
                        if state.config.grpc_enabled {
                            match grpc::start_server(
                                state.config.grpc_bind_addr.clone(),
                                state.clone(),
                            )
                            .await
                            {
                                Ok(handle) => {
                                    info!(
                                        target: "nine_snake",
                                        addr = %state.config.grpc_bind_addr,
                                        "gRPC server started"
                                    );
                                    *state.grpc_server.lock() = Some(handle);
                                }
                                Err(e) => {
                                    error!(
                                        target: "nine_snake",
                                        error = ?e,
                                        "gRPC server failed to start; continuing without it"
                                    );
                                }
                            }
                        } else {
                            info!(target: "nine_snake", "gRPC server disabled by config");
                        }

                        handle.manage(state);
                        info!(target: "nine_snake", "app state ready");
                    }
                    Err(e) => {
                        error!(target: "nine_snake", error = ?e, "failed to bootstrap app state");
                        // v0.3: tell the user, then exit. The dialog
                        // plugin is registered above so the call
                        // resolves. The process exit ensures the user
                        // doesn't end up with a half-broken UI.
                        use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                        let _ = handle
                            .dialog()
                            .message(format!("九头蛇启动失败：{e:#}\n\n将退出应用。"))
                            .title("Nine-snake bootstrap error")
                            .kind(MessageDialogKind::Error)
                            .blocking_show();
                        std::process::exit(1);
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::health,
            // v1.0.1 revert: P0#13 commands-by-topic split is
            // gone, so every entry uses the flat `commands::name`
            // path.  `chat` is the only name that historically
            // collided with a `chat` module elsewhere (e.g. the
            // gRPC-generated client) — it now resolves to the
            // function in `commands::mod`.
            commands::chat,
            commands::memory_store,
            commands::memory_search,
            commands::memory_get,
            commands::memory_list_recent,
            commands::memory_update_importance,
            commands::memory_delete,
            commands::memory_get_many,
            commands::memory_stats,
            commands::swarm_execute,
            commands::swarm_list_agents,
            commands::swarm_get_agent,
            commands::llm_complete,
            commands::llm_chat,
            commands::llm_embed,
            commands::reflect_now,
            commands::list_reflections,
            commands::get_reflection,
            commands::metrics,
            commands::migration_status,
            // v1.0: perf + settings commands.
            commands::startup_report,
            commands::perf_sample,
            commands::load_app_settings,
            commands::save_app_settings,
            // v0.3: Skill CRUD.
            commands::skill_create,
            commands::skill_use,
            commands::skill_rate,
            commands::skill_list,
            commands::skill_search,
            commands::skill_import,
            // v0.5: writing.
            commands::writing_list_templates,
            commands::writing_get_template,
            commands::writing_create_document,
            commands::writing_update_document,
            commands::writing_get_document,
            commands::writing_list_documents,
            commands::writing_delete_document,
            commands::writing_export,
            // v0.5: work.
            commands::work_create_task,
            commands::work_get_task,
            commands::work_list_tasks,
            commands::work_set_status,
            commands::work_update_task,
            commands::work_delete_task,
            commands::work_recommend_priority,
            commands::work_summarise_meeting,
            commands::work_start_timer,
            commands::work_stop_timer,
            commands::work_add_time,
            commands::work_active_timer,
            // v0.5: editor.
            commands::editor_read,
            commands::editor_write,
            commands::editor_list,
            commands::editor_workspace_root,
            commands::git_status,
            commands::git_log,
            commands::git_diff,
            commands::git_commit,
            // v0.5: OS.
            commands::os_clipboard_read,
            commands::os_clipboard_write,
            commands::os_shell_exec,
            commands::os_notify,
            // v0.5: sync.
            commands::sync_encrypt,
            commands::sync_decrypt,
            commands::sync_send,
            commands::sync_recv,
            commands::sync_ack,
            commands::sync_make_identity,
            // v1.3: DID identity.
            commands::generate_did,
            commands::resolve_did,
            // v1.3: skill audit.
            commands::skill_audit_list,
            commands::skill_audit_list_for_skill,
            // v1.3: chat_stream.
            commands::chat_stream,
            // v1.3: data export/import.
            commands::export_memories,
            commands::import_memories,
            // v1.3: device management.
            commands::list_devices,
            commands::revoke_device,
            // v1.3: WebChat share.
            commands::share_chat,
            // v1.3: ACL.
            commands::acl_set,
            commands::acl_list,
            commands::acl_remove,
            // v1.3: MCP (feature-gated).
            #[cfg(feature = "mcp")]
            commands::mcp_list_servers,
            #[cfg(feature = "mcp")]
            commands::mcp_add_server,
            #[cfg(feature = "mcp")]
            commands::mcp_remove_server,
            #[cfg(feature = "mcp")]
            commands::mcp_list_tools,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Convenience wrapper for non-Tauri contexts (tests, CLI) that want the
/// same [`AppState`] wiring without spawning a window.
pub async fn build_state_for_tests(config: AppConfig) -> anyhow::Result<AppState> {
    AppState::bootstrap(config).await
}

/// Re-exported for convenience.
pub use memory::reflect::Reflection;
pub use memory::types::{Memory, MemoryLayer, MemoryType, MultiGranularity};
pub use memory::MigrationStatus;
pub use metrics::MetricsSnapshot;