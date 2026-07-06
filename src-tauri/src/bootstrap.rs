//! Bootstrap phase — constructs all subsystems and assembles [`AppState`].
//!
//! This module contains the `bootstrap` method and its phase helpers
//! (`bootstrap_storage`, `bootstrap_ai_core`, `bootstrap_swarm_and_reflection`,
//! `bootstrap_skills`, etc.) plus `shutdown`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use parking_lot::Mutex;
use tauri::Emitter;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::app_config::AppConfig;
use crate::app_state::AppState;
#[cfg(feature = "channels")]
use crate::channel::bridge::MessageBridge;
#[cfg(feature = "channels")]
use crate::channel::webchat::WebChatService;
use crate::editor::EditorState;
use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::OllamaClient;
use crate::llm::prefetch::PrefetchEngine;
use crate::llm::semantic_cache::SemanticCache;
use crate::llm::openai_compat::OpenAICompatClient;
use crate::memory::acl::{AclEffect, AclPermission, AclRule, MemoryAcl};
use crate::memory::blackhole::BlackholeEngine;
use crate::memory::causal_graph::CausalGraphEngine;
use crate::memory::embedder::Embedder;
use crate::memory::l0_cache::L0Cache;
use crate::memory::orchestrator::MemoryOrchestrator;
use crate::memory::reflect::{ReflectConfig, ReflectionEngine};
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::summarizer::SummaryEngine;
use crate::memory::vector_store::{create_vector_store, VectorStore, VectorStoreBackend};
use crate::memory::version_control::MemoryVersionControl;
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
use crate::sync::device_manager::DeviceManager;
use crate::sync::LocalTransport;
use crate::tools::{shell_tool::ShellTool, ToolRegistry};
use crate::work::WorkEngine;
use crate::writing::WritingEngine;

impl AppState {
    /// Bootstraps a fully-wired [`AppState`] from the given config.
    ///
    /// On failure all already-initialised subsystems are dropped; the
    /// returned `anyhow::Error` carries the full context chain.
    pub async fn bootstrap(mut config: AppConfig, app_handle: tauri::AppHandle) -> anyhow::Result<Self> {
        info!(target: "nebula", "bootstrapping app state");
        let startup = StartupTimer::start();
        startup.mark("bootstrap.start");

        // Phase 1: storage (SQLite + migrations + LanceDB)
        let (sqlite, lance) = Self::bootstrap_storage(&config, &startup).await?;

        // Phase 2: AI core (embedder, LLM, sponge, blackhole)
        let (
            embedder,
            llm,
            sponge,
            blackhole,
            exec_approval,
            cost_tracker,
            inline_completion,
            models_config,
            semantic_cache,
        ) = Self::bootstrap_ai_core(&config, &sqlite, &lance, &startup, &app_handle).await?;

        // T-E-A-11: Smart Prefetch 引擎 — 与 LlmGateway 共享同一 Arc<SemanticCache>。
        let prefetch = Arc::new(PrefetchEngine::with_default_config(
            sqlite.clone(),
            lance.clone(),
            embedder.clone(),
            semantic_cache,
        ));
        startup.mark("bootstrap.prefetch");
        info!(target: "nebula", "prefetch engine ready (T-E-A-11)");

        // Phase 3: swarm + reflection
        let tool_registry = Arc::new(ToolRegistry::new());
        let (swarm, reflection, deadlock_detector) = Self::bootstrap_swarm_and_reflection(
            &config, &sqlite, &lance, &embedder, &llm, &sponge, &tool_registry,
        );

        // T-E-S-39: 预加载 SOUL.md/AGENTS.md/TOOLS.md persona。
        let persona_cache = {
            let ws_root = std::path::Path::new(&config.editor_workspace);
            match crate::llm::persona::PersonaConfig::load(ws_root).await {
                Ok(p) => {
                    if !p.is_empty() {
                        info!(
                            target: "nebula",
                            soul = p.soul_md.is_some(),
                            agents = p.agents_md.is_some(),
                            tools = p.tools_md.is_some(),
                            "persona loaded from workspace root"
                        );
                    }
                    Some(Arc::new(parking_lot::RwLock::new(p)))
                }
                Err(e) => {
                    warn!(
                        target: "nebula",
                        error = %e,
                        "failed to preload persona; chat will proceed without it"
                    );
                    None
                }
            }
        };
        if let Some(ref pc) = persona_cache {
            swarm.set_persona(pc.clone());
        }
        config.persona = persona_cache;
        startup.mark("bootstrap.persona");

        // v2.0: 真正的 Self-Reflection 引擎（L5 元认知层升级）。
        let self_reflection = Arc::new(
            crate::memory::self_reflection::SelfReflectionEngine::new(
                sqlite.clone(),
                swarm.values_layer().clone(),
                reflection.config().clone(),
            ),
        );
        startup.mark("bootstrap.self_reflection");

        // Phase 4: skills ecosystem
        let (skills, skill_extractor, skill_composer, marketplace, skill_audit_logger) =
            Self::bootstrap_skills(&config, &sqlite, &llm, &exec_approval);
        swarm.set_composer(skill_composer.clone());

        // v1.4: L0 缓存层 + Memory Orchestrator。
        let l0 = Arc::new(L0Cache::new());
        let mut orchestrator_builder = MemoryOrchestrator::new(
            sqlite.clone(),
            lance.clone(),
            embedder.clone(),
            l0.clone(),
        )
        .with_sponge(sponge.clone());
        if let Some(acl) = sponge.acl() {
            orchestrator_builder = orchestrator_builder.with_acl(acl.clone());
            info!(target: "nebula", "ACL loaded into MemoryOrchestrator");
        }
        let orchestrator = Arc::new(orchestrator_builder);
        startup.mark("bootstrap.memory_orchestrator");

        // v1.5: 多粒度摘要引擎 + 因果图谱引擎。
        let summary_engine = Arc::new(SummaryEngine::new(llm.clone()));
        let causal_graph = Arc::new(CausalGraphEngine::new((*sqlite).clone()));
        startup.mark("bootstrap.causal_graph");

        // v1.6: Git 风格记忆版本控制引擎。
        let version_control = Arc::new(MemoryVersionControl::new(sqlite.clone()));
        startup.mark("bootstrap.version_control");

        // v2.0: Sidecar 进程管理器（默认进程内模式，sidecar 二进制存在时自动切换）。
        let data_dir = std::path::Path::new(&config.db_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let sidecar_manager = crate::sidecar::SidecarManager::new(data_dir);
        startup.mark("bootstrap.sidecar_manager");

        // Phase 5: workspace tooling + final assembly
        #[cfg(feature = "channels")]
        let message_bridge = Self::bootstrap_message_bridge();
        let writing = Arc::new(WritingEngine::new(sqlite.clone(), Some(sponge.clone())));
        let work = Arc::new(WorkEngine::new(sqlite.clone()));
        let editor = Self::bootstrap_editor(&config);
        startup.mark("bootstrap.editor");
        let clipboard = Self::bootstrap_clipboard();
        let shell = Arc::new(ShellExecutor::new());
        tool_registry.register(Arc::new(ShellTool::new((*shell).clone())));
        let sync_transport = Self::bootstrap_sync(&config);
        startup.mark("bootstrap.end");

        // v1.8: 启动性能监控器（后台 1Hz 采样 RSS/CPU）。
        let (_perf_handle, perf_monitor) =
            crate::perf::monitor::PerfMonitor::start(std::time::Duration::from_secs(1));
        std::mem::forget(_perf_handle);
        info!(target: "nebula", "perf monitor started");

        let device_manager = Arc::new(parking_lot::Mutex::new(DeviceManager::new(
            sqlite.raw_connection(),
        )));

        // T-E-S-27: 拿到全局 DiagnosticsBus 单例,装入 AppState。
        let diagnostics = Arc::clone(crate::diagnostics::bus::global());
        info!(
            target: "nebula",
            enabled = config.diagnostics_channel_enabled,
            capacity = config.diagnostics_buffer_capacity,
            "diagnostics bus ready (T-E-S-27)"
        );

        // T-E-S-26: 拿到全局 EventBus 单例,装入 AppState。
        let event_bus = Arc::clone(crate::swarm::event_bus::global());
        info!(target: "nebula", "event bus ready (T-E-S-26)");

        // T-E-B-09: 构造 FileWatcherEngine。
        let file_watcher = Arc::new(
            crate::memory::file_watcher::FileWatcherEngine::new(sponge.clone()),
        );
        let file_watcher_worker: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>> =
            Arc::new(parking_lot::Mutex::new(None));
        if !config.watch_paths.is_empty() {
            let watch_paths: Vec<std::path::PathBuf> = config
                .watch_paths
                .iter()
                .map(std::path::PathBuf::from)
                .collect();
            file_watcher.start(watch_paths);
            if let Some(handle) = file_watcher.clone().spawn_worker() {
                *file_watcher_worker.lock() = Some(handle);
            }
            info!(
                target: "nebula",
                count = config.watch_paths.len(),
                "file watcher started (T-E-B-09)"
            );
        }

        // T-E-C-14: 构造 ClipboardWatcherEngine。此处只构造不启动。
        let clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>> =
            Arc::new(tokio::sync::Mutex::new(crate::os::ClipboardWatcherEngine::new()));
        info!(
            target: "nebula",
            enabled = config.clipboard_watch_enabled,
            "clipboard watcher engine ready (T-E-C-14)"
        );

        // T-E-S-44: 统一存储后端(Local/S3/WebDAV)。
        let storage = crate::storage::StorageBackendFactory::from_config(&config.storage_backend)
            .context("initializing storage backend")?;
        info!(
            target: "nebula",
            kind = config.storage_backend.kind,
            "storage backend ready (T-E-S-44)"
        );

        // T-E-S-24: 文件快照回滚引擎。
        let snapshot_engine = Arc::new(
            crate::snapshot::SnapshotEngine::new(storage.clone())
                .context("initializing snapshot engine")?,
        );
        info!(target: "nebula", "snapshot engine ready (T-E-S-24)");

        // T-E-S-57: 后台通知服务。
        let notification_service = Arc::new(crate::notify::NotificationService::new(app_handle));

        // T-E-S-54: 事件触发器引擎 — 构造 + start()。
        let trigger_engine = Arc::new(crate::triggers::TriggerEngine::new(
            sqlite.clone(),
            swarm.bus().clone(),
            skills.clone(),
            swarm.clone(),
            notification_service.clone(),
        ));
        trigger_engine.clone().start();
        info!(target: "nebula", "trigger engine started (T-E-S-54)");

        // T-E-C-13: 工作场景模板引擎。
        let scenario_templates = Arc::new(
            crate::scenarios::TemplateEngine::load()
                .context("loading scenario templates from scenarios.json")?,
        );
        info!(
            target: "nebula",
            count = scenario_templates.list().len(),
            "scenario template engine loaded (T-E-C-13)"
        );

        // T-E-C-17: IM 绑定引擎。
        let im_engine = Arc::new(crate::im::ImEngine::new(sqlite.clone()));
        info!(target: "nebula", "IM engine ready (T-E-C-17)");

        // T-E-B-01: LLM Wiki 编译引擎。
        let wiki_config = crate::wiki::WikiConfig {
            enabled: config.wiki_enabled,
            subdir: config.wiki_subdir.clone(),
        };
        let wiki = Arc::new(crate::wiki::WikiCompiler::new(
            llm.clone(),
            storage.clone(),
            sqlite.clone(),
            wiki_config,
        )
        .with_memory_sync(
            sponge.clone() as std::sync::Arc<dyn crate::wiki::MemoryRevectorizer>,
            version_control.clone(),
        ));
        info!(
            target: "nebula",
            enabled = config.wiki_enabled,
            subdir = %config.wiki_subdir,
            "wiki compiler ready (T-E-B-01) + memory sync wired (T-E-B-03)"
        );

        // T-E-S-32: MCP — 提前构造 mcp_manager 作为变量,供 mcp_registry 复用。
        #[cfg(feature = "mcp")]
        let mcp_manager = Arc::new(crate::mcp::client::McpManager::new());

        #[cfg(feature = "mcp")]
        let mcp_registry = {
            let log_dir = crate::tracing_setup::default_log_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            Arc::new(crate::mcp::registry::McpServerRegistry::new(
                mcp_manager.clone(),
                log_dir,
            ))
        };
        #[cfg(feature = "mcp")]
        {
            info!(target: "nebula", "MCP server registry ready (T-E-S-32)");
        }

        // T-E-A-14: Arena A/B 测试。
        let arena = Arc::new(crate::llm::arena::ArenaLeaderboard::new()
            .with_store(sqlite.as_ref().clone()));
        if let Err(e) = arena.load_from_store().await {
            warn!(
                target: "nebula",
                error = %e,
                "arena leaderboard load_from_store failed; starting with empty leaderboard (T-E-A-14)"
            );
        }
        info!(target: "nebula", "arena leaderboard ready (T-E-A-14)");

        // M7a #86: UnifiedModelDispatcher — 顶层共享实例。
        #[cfg(feature = "unified-dispatcher")]
        let dispatcher: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>> = {
            use crate::llm::dispatcher::{ModelPolicy, UnifiedModelDispatcher};
            let mc = models_config.read().clone();
            let policy = ModelPolicy::from_models_config(&mc);
            Some(Arc::new(UnifiedModelDispatcher::new(
                llm.clone(),
                policy,
                Some(cost_tracker.clone()),
                None,
                2,
            )))
        };

        // M5 #68 / M6 #82: ApprovalGate + ConfirmationRegistry + MasterOrchestrator
        let confirmation_registry = Arc::new(crate::autonomy::ConfirmationRegistry::new());
        let approval_gate = Arc::new(crate::autonomy::ApprovalGate::new(
            crate::autonomy::WorkerRiskMap::new(),
            confirmation_registry.clone(),
        ));
        info!(target: "nebula", "approval gate + confirmation registry ready (M5 #68)");
        #[cfg(feature = "master-orchestrator")]
        let master_orchestrator = {
            #[cfg(feature = "unified-dispatcher")]
            let dispatcher = dispatcher.clone();
            #[cfg(not(feature = "unified-dispatcher"))]
            let dispatcher: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>> = None;
            let mo = Arc::new(crate::swarm::MasterOrchestrator::new(
                swarm.clone(),
                dispatcher,
            ));
            info!(target: "nebula", "MasterOrchestrator ready (M6 #82, master-orchestrator feature on)");
            mo
        };

        // M6 #78: EvolutionEngine + EvolutionLog + Roller 构造。
        #[cfg(feature = "evolution-engine")]
        let (evolution_engine, evolution_log, roller) = {
            use crate::evolution::engine::{EvolutionEngine, EvolutionLog, Roller, EvolutionEngineConfig};
            use std::path::PathBuf;
            let log_path = PathBuf::from("evolution_log.md");
            let soul_md_path = PathBuf::from("SOUL.md");
            let log = Arc::new(EvolutionLog::new(log_path));
            let roller = Arc::new(Roller::new(log.clone(), soul_md_path));
            let engine = {
                #[cfg(feature = "unified-dispatcher")]
                let dispatcher_opt = dispatcher.clone();
                #[cfg(not(feature = "unified-dispatcher"))]
                let dispatcher_opt: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>> = None;
                if let Some(dispatcher) = dispatcher_opt {
                    let config = EvolutionEngineConfig::default();
                    Some(Arc::new(EvolutionEngine::new(
                        dispatcher,
                        sqlite.clone(),
                        sponge.clone(),
                        log.clone(),
                        config,
                    )))
                } else {
                    None
                }
            };
            info!(target: "nebula", "EvolutionLog + Roller ready (M6 #78, evolution-engine feature on); engine={}", engine.is_some());
            (engine, log, roller)
        };

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
            reflect_worker: Arc::new(Mutex::new(None)),
            #[cfg(feature = "grpc")]
            grpc_server: Arc::new(Mutex::new(None)),
            startup_timer: startup,
            perf_monitor,
            skill_extractor,
            skill_composer,
            marketplace,
            skill_audit_logger,
            #[cfg(feature = "channels")]
            message_bridge,
            #[cfg(feature = "channels")]
            channel_router: Self::bootstrap_channel_router(),
            tool_registry,
            #[cfg(feature = "channels")]
            webchat_service: WebChatService::new(),
            device_manager,
            #[cfg(feature = "mcp")]
            mcp_manager,
            l0,
            orchestrator,
            summary_engine,
            causal_graph,
            version_control,
            sidecar_manager,
            self_reflection,
            exec_approval,
            cost_tracker,
            inline_completion,
            diagnostics,
            file_watcher,
            file_watcher_worker,
            clipboard_watcher,
            models_config,
            notification_service,
            snapshot_engine,
            deadlock_detector,
            event_bus,
            trigger_engine,
            scenario_templates,
            im_engine,
            storage,
            prefetch,
            wiki,
            #[cfg(feature = "mcp")]
            mcp_registry,
            arena,
            approval_gate,
            confirmation_registry,
            #[cfg(feature = "master-orchestrator")]
            master_orchestrator,
            #[cfg(feature = "evolution-engine")]
            evolution_engine,
            #[cfg(feature = "evolution-engine")]
            evolution_log,
            #[cfg(feature = "evolution-engine")]
            roller,
            #[cfg(feature = "unified-dispatcher")]
            dispatcher,
        })
    }

    // -- bootstrap phase helpers --

    pub(crate) async fn bootstrap_storage(
        config: &AppConfig,
        startup: &StartupTimer,
    ) -> anyhow::Result<(Arc<SqliteStore>, Arc<dyn VectorStore>)> {
        let db_path = config.db_path.clone();
        let db_encryption_enabled = config.db_encryption_enabled;
        let sqlite = tokio::task::spawn_blocking(move || -> anyhow::Result<SqliteStore> {
            if db_encryption_enabled {
                #[cfg(feature = "sqlcipher")]
                {
                    let key = crate::security::keychain::resolve_db_encryption_key()
                        .context("DB encryption enabled but no key in keychain")?;
                    SqliteStore::open_encrypted(&db_path, &key)
                        .context("opening encrypted sqlite store")
                }
                #[cfg(not(feature = "sqlcipher"))]
                {
                    anyhow::bail!(
                        "db_encryption_enabled=true but sqlcipher feature not compiled; \
                         rebuild with --features sqlcipher"
                    );
                }
            } else {
                SqliteStore::open(&db_path).context("opening sqlite store")
            }
        })
        .await
        .context("spawn_blocking for sqlite open failed")??;
        let sqlite = Arc::new(sqlite);
        startup.mark("bootstrap.sqlite");

        info!(target: "nebula", "migrations applied during SqliteStore::open");
        startup.mark("bootstrap.migrations");

        let remote_url = match config.vector_store_backend {
            VectorStoreBackend::Qdrant => config.qdrant_url.as_deref(),
            VectorStoreBackend::Chroma => config.chroma_url.as_deref(),
            VectorStoreBackend::Lance => None,
        };
        let lance = create_vector_store(
            config.vector_store_backend,
            &config.lance_path,
            config.embedding_dim,
            remote_url,
        )
        .await
        .context("opening vector store")?;
        startup.mark("bootstrap.lance");
        Ok((sqlite, lance))
    }

    pub(crate) async fn bootstrap_ai_core(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        lance: &Arc<dyn VectorStore>,
        startup: &StartupTimer,
        app_handle: &tauri::AppHandle,
    ) -> anyhow::Result<(
        Arc<Embedder>,
        Arc<LlmGateway>,
        Arc<SpongeEngine>,
        Arc<BlackholeEngine>,
        Arc<crate::skills::exec_approval::ExecApprovalTracker>,
        Arc<crate::llm::cost_tracker::CostTracker>,
        Arc<crate::editor::InlineCompletionEngine>,
        Arc<parking_lot::RwLock<crate::llm::models_config::ModelsConfig>>,
        Arc<SemanticCache>,
    )> {
        let models_config_value =
            crate::llm::models_config::ModelsConfig::load(&config.models_config_path);
        info!(
            target: "nebula",
            path = %config.models_config_path.display(),
            providers = models_config_value.providers.len(),
            "models.json loaded"
        );
        let embedder = Arc::new(Embedder::new(
            OllamaClient::new(config.ollama_url.clone()),
            config.embed_model.clone(),
            config.embedding_dim,
        ));
        let ollama = Arc::new(OllamaClient::new(config.ollama_url.clone()));
        let inline_model = std::env::var("NEBULA_INLINE_MODEL")
            .unwrap_or_else(|_| "qwen2.5-coder:0.5b".to_string());
        let inline_completion = Arc::new(
            crate::editor::InlineCompletionEngine::new(ollama.clone(), inline_model),
        );
        let ollama_for_compress = ollama.clone();
        let ak = crate::security::keychain::resolve_anthropic_key();
        let am = std::env::var("NEBULA_ANTHROPIC_MODEL").ok();
        let exec_approval = Arc::new(
            crate::skills::exec_approval::ExecApprovalTracker::new(
                config.exec_approval_timeout_secs,
            ),
        );
        let cost_tracker = {
            use crate::llm::cost_tracker::CostTracker;
            let base = CostTracker::new();
            let base = match config.automation_daily_budget_usd {
                Some(budget) if budget > 0.0 => {
                    let app_for_emit = app_handle.clone();
                    let callback: Arc<dyn Fn(crate::llm::cost_tracker::BudgetAlert) + Send + Sync> =
                        Arc::new(move |alert| {
                            if let Err(e) = app_for_emit.emit("budget_exceeded", &alert) {
                                tracing::warn!(
                                    target: "nebula.cost_tracker",
                                    error = %e,
                                    "failed to emit budget_exceeded event"
                                );
                            } else {
                                info!(
                                    target: "nebula.cost_tracker",
                                    daily_cost = alert.daily_cost_usd,
                                    budget = alert.budget_usd,
                                    "automation daily budget exceeded; emitted budget_exceeded"
                                );
                            }
                        });
                    base.with_budget_alert(Some(budget), callback)
                }
                _ => base,
            };
            let base = base.attach_store(sqlite.as_ref().clone());
            info!(
                target: "nebula.cost_tracker",
                "cost tracker attached to sqlite store; historical records backfilled"
            );
            Arc::new(base)
        };
        let mut llm_builder = LlmGateway::new(
            ollama,
            config.chat_model.clone(),
            config.llm_provider.clone(),
            Some(config.deepseek_api_url.clone()),
            crate::security::keychain::resolve_deepseek_key(),
            config.remote_fallback_url.clone(),
            ak,
            am,
        );
        let semantic_cache: Arc<SemanticCache> = Arc::new(
            crate::llm::semantic_cache::SemanticCache::default_config(
                lance.clone(),
                embedder.clone(),
            )
            .with_sqlite(sqlite.clone()),
        );
        if config.semantic_cache_enabled {
            llm_builder = llm_builder.with_semantic_cache(semantic_cache.clone());
            info!(target: "nebula", "semantic cache wired into LlmGateway");
        }
        if config.cost_tracker_enabled {
            llm_builder = llm_builder.with_cost_tracker(cost_tracker.clone());
            info!(target: "nebula", "cost tracker wired into LlmGateway");
        }
        if config.token_juice_enabled {
            let compressor = Arc::new(
                crate::llm::token_juice::TokenJuiceCompressor::new(
                    ollama_for_compress.clone(),
                    config.chat_model.clone(),
                    crate::llm::token_juice::TokenJuiceConfig::default(),
                ),
            );
            llm_builder = llm_builder.with_token_juice(compressor);
            info!(target: "nebula", "token juice compressor wired into LlmGateway");
        }
        if config.router_enabled {
            let router = Arc::new(
                crate::llm::model_router::ModelRouter::new(
                    ollama_for_compress.clone(),
                    config.router_classifier_model.clone(),
                ),
            );
            llm_builder = llm_builder.with_model_router(router);
            info!(target: "nebula", "model router wired into LlmGateway");
        }
        if config.daily_budget_usd > 0.0 {
            llm_builder = llm_builder.with_daily_budget(config.daily_budget_usd);
            info!(
                target: "nebula",
                budget = config.daily_budget_usd,
                "daily budget wired into LlmGateway"
            );
        }
        if let Some(base_url) = config.openai_compat_base_url.as_ref() {
            let model = config
                .openai_compat_model
                .clone()
                .unwrap_or_else(|| "local-model".to_string());
            let client = OpenAICompatClient::new(
                base_url.clone(),
                crate::security::keychain::resolve_openai_compat_key(),
                model,
            );
            llm_builder = llm_builder.with_openai_compat(client);
            info!(
                target: "nebula",
                base_url = %base_url,
                "openai-compat client wired into LlmGateway"
            );
        }
        let llm = Arc::new(llm_builder);
        startup.mark("bootstrap.llm");
        let sponge = {
            let mut sponge_builder = SpongeEngine::new(
                sqlite.clone(),
                lance.clone(),
                embedder.clone(),
            );
            sponge_builder = sponge_builder.with_cost_tracker(cost_tracker.clone());
            match Self::load_acl_from_store(&sqlite) {
                Ok(acl) => {
                    sponge_builder = sponge_builder.with_acl(Arc::new(acl));
                    info!(target: "nebula", "ACL loaded into SpongeEngine");
                }
                Err(e) => {
                    warn!(
                        target: "nebula",
                        error = %e,
                        "failed to load ACL rules from SQLite; sponge search will not enforce ACL"
                    );
                }
            }
            Arc::new(sponge_builder)
        };
        let blackhole = Arc::new(BlackholeEngine::new(
            sqlite.clone(),
            lance.clone(),
            config.blackhole_threshold_days,
        ));
        crate::llm::cost_tracker::update_models_config_override(models_config_value.clone());
        let models_config = Arc::new(parking_lot::RwLock::new(models_config_value));
        match crate::security::keychain::migrate_env_to_keychain() {
            Ok(n) if n > 0 => {
                info!(target: "nebula", count = n, "migrated env-var credentials to keychain");
            }
            Ok(_) => {}
            Err(e) => {
                warn!(target: "nebula", error = %e, "credential migration skipped");
            }
        }
        Ok((
            embedder,
            llm,
            sponge,
            blackhole,
            exec_approval,
            cost_tracker,
            inline_completion,
            models_config,
            semantic_cache,
        ))
    }

    /// T-S1-A-04: 从 SQLite `memory_acl` 表加载规则，构建 `MemoryAcl`。
    pub(crate) fn load_acl_from_store(sqlite: &Arc<SqliteStore>) -> anyhow::Result<MemoryAcl> {
        let rows = sqlite.list_acl()?;
        let mut acl = MemoryAcl::new();
        for (id, principal, resource, permission_s, effect_s) in rows {
            let permission = match permission_s.as_str() {
                "read" => AclPermission::Read,
                "write" => AclPermission::Write,
                "delete" => AclPermission::Delete,
                other => {
                    warn!(
                        target: "nebula",
                        acl_id = %id,
                        bad_value = other,
                        "skipping ACL rule with unknown permission"
                    );
                    continue;
                }
            };
            let effect = match effect_s.as_str() {
                "allow" => AclEffect::Allow,
                "deny" => AclEffect::Deny,
                other => {
                    warn!(
                        target: "nebula",
                        acl_id = %id,
                        bad_value = other,
                        "skipping ACL rule with unknown effect"
                    );
                    continue;
                }
            };
            acl.add_rule(AclRule {
                principal,
                resource,
                permission,
                effect,
            });
        }
        Ok(acl)
    }

    pub(crate) fn bootstrap_swarm_and_reflection(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        lance: &Arc<dyn VectorStore>,
        embedder: &Arc<Embedder>,
        llm: &Arc<LlmGateway>,
        sponge: &Arc<SpongeEngine>,
        tool_registry: &Arc<ToolRegistry>,
    ) -> (Arc<SwarmOrchestrator>, Arc<ReflectionEngine>, Arc<crate::swarm::DeadlockDetector>) {
        let swarm = Arc::new(SwarmOrchestrator::new(
            llm.clone(),
            sponge.clone(),
            lance.clone(),
            embedder.clone(),
            sqlite.clone(),
            tool_registry.clone(),
        ));
        let cfg = ReflectConfig {
            window_days: config.reflect_window_days,
            min_importance: config.reflect_min_importance,
            worker_interval_secs: config.reflect_interval_secs,
            ..ReflectConfig::default()
        };
        let reflection = Arc::new(ReflectionEngine::new(
            sqlite.clone(),
            Some(llm.clone()),
            cfg,
        ));
        let mut deadlock_detector = crate::swarm::DeadlockDetector::with_bus(swarm.bus());
        deadlock_detector.start();
        let deadlock_detector = Arc::new(deadlock_detector);
        (swarm, reflection, deadlock_detector)
    }

    pub(crate) fn bootstrap_skills(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        llm: &Arc<LlmGateway>,
        exec_approval: &Arc<crate::skills::exec_approval::ExecApprovalTracker>,
    ) -> (
        Arc<SkillEngine>,
        Arc<SkillExtractor>,
        Arc<SkillComposer>,
        Arc<crate::skills::SkillMarketplace>,
        Arc<SkillAuditLogger>,
    ) {
        let ss = Arc::new(
            SkillStore::new(sqlite.as_ref().clone()).expect("SkillStore::new must succeed"),
        );
        let audit = Arc::new(SkillAuditLogger::new(sqlite.raw_connection()));
        let skills = Arc::new(
            SkillEngine::from_store((*ss).clone(), llm.clone())
                .with_audit(audit.clone())
                .with_exec_approval(exec_approval.clone()),
        );
        info!(target: "nebula", "exec approval tracker wired into SkillEngine");
        let adir = config
            .db_path
            .rsplit_once(std::path::MAIN_SEPARATOR)
            .map(|(d, _)| d)
            .unwrap_or(".")
            .to_string()
            + "/skills_archive";
        let extr = Arc::new(SkillExtractor::new(llm.clone(), ss.clone(), adir));
        let comp = Arc::new(SkillComposer::new(ss.clone(), Some(llm.clone())));
        let imp = Arc::new(SkillImporter::new((*ss).clone()));
        let mp = Arc::new(crate::skills::SkillMarketplace::new(ss, imp));
        let _ = mp.refresh();
        crate::skills::seed_demo_skills(&skills).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e, "failed to seed demo skills");
            Vec::new()
        });
        skills.register_capability(crate::skills::capability::Capability {
            id: "viz:canvas".to_string(),
            name: "Canvas Visualization".to_string(),
            description: "Generate HTML5 canvas visualizations from natural language".to_string(),
            skills: vec!["canvas-creator".to_string()],
        });
        skills.register_capability(crate::skills::capability::Capability {
            id: "viz:mermaid".to_string(),
            name: "Mermaid Diagram".to_string(),
            description: "Generate Mermaid flowchart / sequence / gantt / state / class diagrams"
                .to_string(),
            skills: vec!["mermaid-creator".to_string()],
        });
        skills.register_capability(crate::skills::capability::Capability {
            id: "viz:mindmap".to_string(),
            name: "Mermaid Mindmap".to_string(),
            description: "Generate Mermaid mindmap diagrams from a topic".to_string(),
            skills: vec!["mindmap-creator".to_string()],
        });
        info!(
            target: "nebula",
            count = skills.list_capabilities().len(),
            "viz creator capabilities registered (T-E-S-38)"
        );
        (skills, extr, comp, mp, audit)
    }

    #[cfg(feature = "channels")]
    fn bootstrap_message_bridge() -> Option<Arc<MessageBridge>> {
        let url = std::env::var("NEBULA_BRIDGE_URL").unwrap_or_default();
        let b = MessageBridge::new(&url).map(Arc::new);
        if b.is_some() {
            info!(target: "nebula", bridge_url = %url, "message bridge initialised");
        }
        b
    }

    /// T-S3-B-01: 初始化原生渠道路由器。
    #[cfg(feature = "channels")]
    pub(crate) fn bootstrap_channel_router() -> Arc<crate::channel::ChannelRouter> {
        use crate::channel::{ChannelRouter, DiscordBotAdapter, TelegramBotAdapter};

        let router = Arc::new(ChannelRouter::new());

        let tg_token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        if !tg_token.is_empty() {
            let adapter = TelegramBotAdapter::new(&tg_token);
            router.register(Arc::new(adapter) as Arc<dyn crate::channel::ChannelAdapter>);
            info!(target: "nebula.channel", "Telegram adapter registered");
        }

        let dc_webhook = std::env::var("DISCORD_WEBHOOK_URL").unwrap_or_default();
        if !dc_webhook.is_empty() {
            let adapter = DiscordBotAdapter::new(&dc_webhook);
            router.register(Arc::new(adapter) as Arc<dyn crate::channel::ChannelAdapter>);
            info!(target: "nebula.channel", "Discord adapter registered");
        }

        router
    }

    pub(crate) fn bootstrap_editor(config: &AppConfig) -> EditorState {
        EditorState::new(&config.editor_workspace).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                workspace = %config.editor_workspace,
                "editor workspace unavailable; falling back to current dir");
            EditorState::new(".").expect("current dir is always a directory")
        })
    }

    pub(crate) fn bootstrap_clipboard() -> ClipboardService {
        ClipboardService::new().unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                "clipboard unavailable; using noop fallback");
            ClipboardService::noop()
        })
    }

    pub(crate) fn bootstrap_sync(config: &AppConfig) -> Arc<LocalTransport> {
        Arc::new(LocalTransport::new(&config.sync_inbox).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                inbox = %config.sync_inbox,
                "sync inbox unavailable; using temp dir");
            let tmp = std::env::temp_dir().join("nebula-sync-inbox");
            LocalTransport::new(&tmp).expect("temp dir always works")
        }))
    }

    /// Wakes the background reflection worker, signals the gRPC
    /// server to stop, and awaits both joins with a brief grace
    /// period. Idempotent and safe to call from Tauri shutdown.
    pub async fn shutdown(&self) {
        let notify = self.reflection.shutdown_handle();
        notify.notify_waiters();

        let worker = { self.reflect_worker.lock().take() };
        if let Some(h) = worker {
            match tokio::time::timeout(Duration::from_millis(250), h).await {
                Ok(_) => info!(target: "nebula", "reflection worker stopped"),
                Err(_) => warn!(target: "nebula", "reflection worker did not stop in time"),
            }
        }

        #[cfg(feature = "grpc")]
        {
            let grpc = { self.grpc_server.lock().take() };
            if let Some(h) = grpc {
                h.shutdown().await;
            }
        }

        {
            self.file_watcher_worker.lock().take();
        }
        self.file_watcher.stop().await;

        {
            let mut watcher = self.clipboard_watcher.lock().await;
            watcher.stop();
        }

        self.trigger_engine.stop();
    }

    /// M1 任务 #23: 尝试 Soul 编译（cfg-gated）。
    #[cfg(feature = "soul-system")]
    pub(crate) async fn try_compile_soul(&self) -> Option<String> {
        use crate::soul::soul_system_enabled;

        if !soul_system_enabled() {
            return None;
        }

        let compiler = self.config.soul_compiler.as_ref()?;

        let soul_md_text = self.config.persona.as_ref().and_then(|pc| {
            let guard = pc.read();
            guard.soul_md.clone()
        })?;

        if soul_md_text.trim().is_empty() {
            return None;
        }

        match compiler.compile(&soul_md_text).await {
            Ok(compiled) => {
                if compiled.degraded {
                    tracing::warn!(
                        target: "nebula.soul",
                        warnings = compiled.warnings.len(),
                        "Soul compiled in degraded mode (text-only, no LLM)"
                    );
                } else {
                    tracing::info!(
                        target: "nebula.soul",
                        warnings = compiled.warnings.len(),
                        "Soul compiled successfully"
                    );
                }
                if compiled.system_prompt.is_empty() {
                    None
                } else {
                    Some(compiled.system_prompt)
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "nebula.soul",
                    error = %e,
                    "Soul compile failed; falling back to PersonaConfig"
                );
                None
            }
        }
    }
}
