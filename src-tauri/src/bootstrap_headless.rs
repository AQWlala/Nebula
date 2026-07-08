//! Headless bootstrap — constructs [`AppState`] without an `AppHandle`.
//!
//! Used by non-Tauri contexts (tests, CLI, daemon) that want the same
//! [`AppState`] wiring without spawning a window.

use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::app_config::AppConfig;
use crate::app_state::AppState;
#[cfg(feature = "channels")]
use crate::channel::webchat::WebChatService;
use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::OllamaClient;
use crate::llm::openai_compat::OpenAICompatClient;
use crate::llm::prefetch::PrefetchEngine;
use crate::llm::semantic_cache::SemanticCache;
use crate::memory::blackhole::BlackholeEngine;
use crate::memory::causal_graph::CausalGraphEngine;
use crate::memory::embedder::Embedder;
use crate::memory::l0_cache::L0Cache;
use crate::memory::orchestrator::MemoryOrchestrator;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::summarizer::SummaryEngine;
use crate::memory::vector_store::VectorStore;
use crate::memory::version_control::MemoryVersionControl;
use crate::os::ShellExecutor;
use crate::perf::StartupTimer;
use crate::sync::device_manager::DeviceManager;
use crate::tools::{shell_tool::ShellTool, ToolRegistry};
use crate::work::WorkEngine;
use crate::writing::WritingEngine;

impl AppState {
    /// T-E-C-20: headless 模式 bootstrap — 无 `AppHandle`,跳过桌面特有功能。
    pub async fn bootstrap_headless(mut config: AppConfig) -> anyhow::Result<Self> {
        info!(target: "nebula", "bootstrapping app state (headless, T-E-C-20)");
        let startup = StartupTimer::start();
        startup.mark("bootstrap.start");

        // Phase 1: storage
        let (sqlite, lance) = Self::bootstrap_storage(&config, &startup).await?;

        // Phase 2: AI core — headless 变体,无需 AppHandle
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
        ) = Self::bootstrap_ai_core_headless(&config, &sqlite, &lance, &startup).await?;

        // Smart Prefetch
        let prefetch = Arc::new(PrefetchEngine::with_default_config(
            sqlite.clone(),
            lance.clone(),
            embedder.clone(),
            semantic_cache.clone(),
        ));
        startup.mark("bootstrap.prefetch");

        // Phase 3: swarm + reflection
        let tool_registry = Arc::new(ToolRegistry::new());
        let (swarm, reflection, deadlock_detector) = Self::bootstrap_swarm_and_reflection(
            &config,
            &sqlite,
            &lance,
            &embedder,
            &llm,
            &sponge,
            &tool_registry,
        );

        // persona
        let persona_cache = {
            let ws_root = std::path::Path::new(&config.editor_workspace);
            match crate::llm::persona::PersonaConfig::load(ws_root).await {
                Ok(p) => {
                    if !p.is_empty() {
                        info!(target: "nebula", "persona loaded (headless)");
                    }
                    Some(Arc::new(parking_lot::RwLock::new(p)))
                }
                Err(e) => {
                    warn!(target: "nebula", error = %e, "persona load failed (headless)");
                    None
                }
            }
        };
        if let Some(ref pc) = persona_cache {
            swarm.set_persona(pc.clone());
        }
        config.persona = persona_cache;

        // Self-Reflection
        let self_reflection = Arc::new(crate::memory::self_reflection::SelfReflectionEngine::new(
            sqlite.clone(),
            swarm.values_layer().clone(),
            reflection.config().clone(),
        ));

        // Phase 4: skills
        let (skills, skill_extractor, skill_composer, marketplace, skill_audit_logger) =
            Self::bootstrap_skills(&config, &sqlite, &llm, &exec_approval);
        swarm.set_composer(skill_composer.clone());

        // L0 + Orchestrator
        let l0 = Arc::new(L0Cache::new());
        let mut orchestrator_builder =
            MemoryOrchestrator::new(sqlite.clone(), lance.clone(), embedder.clone(), l0.clone())
                .with_sponge(sponge.clone());
        if let Some(acl) = sponge.acl() {
            orchestrator_builder = orchestrator_builder.with_acl(acl.clone());
        }
        let orchestrator = Arc::new(orchestrator_builder);

        // Summary + CausalGraph
        let summary_engine = Arc::new(SummaryEngine::new(llm.clone()));
        let causal_graph = Arc::new(CausalGraphEngine::new((*sqlite).clone()));

        // Version Control
        let version_control = Arc::new(MemoryVersionControl::new(sqlite.clone()));

        // Sidecar (headless: 仅进程内模式)
        let data_dir = std::path::Path::new(&config.db_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let sidecar_manager = crate::sidecar::SidecarManager::new(data_dir);

        // Workspace tooling
        let writing = Arc::new(WritingEngine::new(sqlite.clone(), Some(sponge.clone())));
        let work = Arc::new(WorkEngine::new(sqlite.clone()));
        let editor = Self::bootstrap_editor(&config);
        // T-E-C-08: Shadow Workspace 引擎 — 注入 repo root。
        let shadow_engine =
            Arc::new(crate::shadow_workspace::ShadowWorkspaceEngine::with_default());
        shadow_engine.set_repo_root(editor.workspace_root().to_path_buf());
        // T-E-C-10: 长任务引擎(复用 sqlite + shadow_engine)。
        let long_task_engine = Arc::new(crate::long_task::LongTaskEngine::new(
            sqlite.clone(),
            shadow_engine.clone(),
        ));
        let _ = long_task_engine.bootstrap();
        let clipboard = Self::bootstrap_clipboard();
        let shell = Arc::new(ShellExecutor::new());
        tool_registry.register(Arc::new(ShellTool::new((*shell).clone())));
        let sync_transport = Self::bootstrap_sync(&config);

        // Perf monitor (headless: 仍采样,可由 /metrics 端点暴露)
        let (_perf_handle, perf_monitor) =
            crate::perf::monitor::PerfMonitor::start(std::time::Duration::from_secs(1));
        std::mem::forget(_perf_handle);

        let device_manager = Arc::new(parking_lot::Mutex::new(DeviceManager::new(
            sqlite.raw_connection(),
        )));

        // Diagnostics
        let diagnostics = Arc::clone(crate::diagnostics::bus::global());

        // T-E-S-26: EventBus (全局单例)
        let event_bus = Arc::clone(crate::swarm::event_bus::global());

        // File watcher (headless: 仍支持,目录监控不依赖 GUI)
        let file_watcher = Arc::new(crate::memory::file_watcher::FileWatcherEngine::new(
            sponge.clone(),
        ));
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
        }

        // Clipboard watcher (headless: 构造但不启动)
        let clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>> =
            Arc::new(tokio::sync::Mutex::new(
                crate::os::ClipboardWatcherEngine::new(),
            ));

        // Storage backend
        let storage = crate::storage::StorageBackendFactory::from_config(&config.storage_backend)
            .context("initializing storage backend")?;

        // Snapshot engine
        let snapshot_engine = Arc::new(
            crate::snapshot::SnapshotEngine::new(storage.clone())
                .context("initializing snapshot engine")?,
        );

        // T-E-C-20: headless 模式 NotificationService。
        let notification_service = Arc::new(crate::notify::NotificationService::new_headless());

        // Trigger engine
        let trigger_engine = Arc::new(crate::triggers::TriggerEngine::new(
            sqlite.clone(),
            swarm.bus().clone(),
            skills.clone(),
            swarm.clone(),
            notification_service.clone(),
        ));
        trigger_engine.clone().start();

        // Scenario templates
        let scenario_templates = Arc::new(
            crate::scenarios::TemplateEngine::load()
                .context("loading scenario templates from scenarios.json")?,
        );

        // IM engine
        let im_engine = Arc::new(crate::im::ImEngine::new(sqlite.clone()));

        // Wiki
        let wiki_config = crate::wiki::WikiConfig {
            enabled: config.wiki_enabled,
            subdir: config.wiki_subdir.clone(),
        };
        let wiki = Arc::new(
            crate::wiki::WikiCompiler::new(
                llm.clone(),
                storage.clone(),
                sqlite.clone(),
                wiki_config,
            )
            .with_memory_sync(
                sponge.clone() as std::sync::Arc<dyn crate::wiki::MemoryRevectorizer>,
                version_control.clone(),
            ),
        );

        #[cfg(feature = "mcp")]
        let mcp_manager = Arc::new(crate::mcp::client::McpManager::new());
        #[cfg(feature = "mcp")]
        let mcp_registry = {
            let log_dir = crate::tracing_setup::default_log_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            Arc::new(crate::mcp::registry::McpServerRegistry::new(
                mcp_manager.clone(),
                log_dir,
            ))
        };

        // T-E-A-14: Arena A/B 测试 — headless 模式同样构造 leaderboard。
        let arena = Arc::new(
            crate::llm::arena::ArenaLeaderboard::new().with_store(sqlite.as_ref().clone()),
        );
        if let Err(e) = arena.load_from_store().await {
            warn!(
                target: "nebula",
                error = %e,
                "arena leaderboard load_from_store failed (headless); starting empty (T-E-A-14)"
            );
        }
        info!(target: "nebula", "arena leaderboard ready (headless, T-E-A-14)");

        // M7a #86 (headless): UnifiedModelDispatcher — 顶层共享实例。
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

        // T-D-C-08: 从环境变量初始化 master-orchestrator 运行时开关 (headless)
        #[cfg(feature = "master-orchestrator")]
        crate::swarm::init_master_orchestrator_from_env();

        // M5 #68 / M6 #82 (headless): ApprovalGate + ConfirmationRegistry + MasterOrchestrator
        let confirmation_registry = Arc::new(crate::autonomy::ConfirmationRegistry::new());
        let approval_gate = Arc::new(crate::autonomy::ApprovalGate::new(
            crate::autonomy::WorkerRiskMap::new(),
            confirmation_registry.clone(),
        ));
        info!(target: "nebula", "approval gate ready (headless, M5 #68)");
        #[cfg(feature = "master-orchestrator")]
        let master_orchestrator = {
            if crate::swarm::master_orchestrator_enabled() {
                #[cfg(feature = "unified-dispatcher")]
                let dispatcher = dispatcher.clone();
                #[cfg(not(feature = "unified-dispatcher"))]
                let dispatcher: Option<
                    Arc<crate::llm::dispatcher::UnifiedModelDispatcher>,
                > = None;
                let mo = Arc::new(crate::swarm::MasterOrchestrator::new(
                    swarm.clone(),
                    dispatcher,
                ));
                info!(target: "nebula", "MasterOrchestrator ready (headless, M6 #82, runtime ON, T-D-C-08)");
                mo
            } else {
                let mo = Arc::new(crate::swarm::MasterOrchestrator::new(
                    swarm.clone(),
                    None,
                ));
                warn!(target: "nebula", "MasterOrchestrator disabled at runtime (headless, T-D-C-08)");
                mo
            }
        };

        // M6 #78: EvolutionEngine + EvolutionLog + Roller 构造(headless)。
        #[cfg(feature = "evolution-engine")]
        let (evolution_engine, evolution_log, roller) = {
            use crate::evolution::engine::{
                EvolutionEngine, EvolutionEngineConfig, EvolutionLog, Roller,
            };
            use std::path::PathBuf;
            let log = Arc::new(EvolutionLog::new(PathBuf::from("evolution_log.md")));
            let roller = Arc::new(Roller::new(log.clone(), PathBuf::from("SOUL.md")));
            let engine = {
                #[cfg(feature = "unified-dispatcher")]
                let dispatcher_opt = dispatcher.clone();
                #[cfg(not(feature = "unified-dispatcher"))]
                let dispatcher_opt: Option<
                    Arc<crate::llm::dispatcher::UnifiedModelDispatcher>,
                > = None;
                if let Some(dispatcher) = dispatcher_opt {
                    Some(Arc::new(EvolutionEngine::new(
                        dispatcher,
                        sqlite.clone(),
                        sponge.clone(),
                        log.clone(),
                        EvolutionEngineConfig::default(),
                    )))
                } else {
                    None
                }
            };
            info!(target: "nebula", "EvolutionLog + Roller ready (headless, M6 #78); engine={}", engine.is_some());
            (engine, log, roller)
        };

        Ok(Self {
            memory: crate::app_state::MemorySubsystem {
                sqlite,
                lance,
                embedder,
                sponge,
                blackhole,
                reflection,
                reflect_worker: Arc::new(Mutex::new(None)),
                l0,
                orchestrator,
                summary_engine,
                causal_graph,
                version_control,
                self_reflection,
                file_watcher,
                file_watcher_worker,
            },
            llm: crate::app_state::LlmSubsystem {
                llm,
                cost_tracker,
                prefetch,
                models_config,
                arena,
                inline_completion,
                #[cfg(feature = "unified-dispatcher")]
                dispatcher,
            },
            swarm: crate::app_state::SwarmSubsystem {
                swarm,
                skills,
                skill_extractor,
                skill_composer,
                marketplace,
                skill_audit_logger,
                exec_approval,
                event_bus,
                deadlock_detector,
                trigger_engine,
                scenario_templates,
                shadow_engine,
                long_task_engine,
                #[cfg(feature = "master-orchestrator")]
                master_orchestrator,
                #[cfg(feature = "evolution-engine")]
                evolution_engine,
                #[cfg(feature = "evolution-engine")]
                evolution_log,
                #[cfg(feature = "evolution-engine")]
                roller,
            },
            channels: crate::app_state::ChannelSubsystem {
                #[cfg(feature = "channels")]
                message_bridge: None,
                #[cfg(feature = "channels")]
                channel_router: Self::bootstrap_channel_router(),
                #[cfg(feature = "channels")]
                webchat_service: WebChatService::new(),
                im_engine,
            },
            platform: crate::app_state::PlatformSubsystem {
                writing,
                work,
                editor,
                clipboard,
                shell,
                sync_transport,
                device_manager,
                notification_service,
                snapshot_engine,
                storage,
                oauth_manager: Arc::new(crate::identity::OAuthManager::new()),
                #[cfg(feature = "grpc")]
                grpc_server: Arc::new(Mutex::new(None)),
                #[cfg(feature = "mcp")]
                mcp_manager,
                #[cfg(feature = "mcp")]
                mcp_registry,
                sidecar_manager,
                wiki,
                clipboard_watcher,
            },
            infra: crate::app_state::InfraSubsystem {
                config: Arc::new(config),
                startup_timer: startup,
                perf_monitor,
                tool_registry,
                diagnostics,
                approval_gate,
                confirmation_registry,
            },
        })
    }

    /// T-E-C-20: headless 变体的 AI 核心初始化 — 无预算告警回调。
    async fn bootstrap_ai_core_headless(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        lance: &Arc<dyn VectorStore>,
        startup: &StartupTimer,
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

        let embedder = Arc::new(Embedder::new(
            OllamaClient::new(config.ollama_url.clone()),
            config.embed_model.clone(),
            config.embedding_dim,
        ));
        let ollama = Arc::new(OllamaClient::new(config.ollama_url.clone()));

        let inline_model = std::env::var("NEBULA_INLINE_MODEL")
            .unwrap_or_else(|_| "qwen2.5-coder:0.5b".to_string());
        let inline_completion = Arc::new(crate::editor::InlineCompletionEngine::new(
            ollama.clone(),
            inline_model,
        ));

        let ollama_for_compress = ollama.clone();
        let ak = crate::security::keychain::resolve_anthropic_key();
        let am = std::env::var("NEBULA_ANTHROPIC_MODEL").ok();

        let exec_approval = Arc::new(crate::skills::exec_approval::ExecApprovalTracker::new(
            config.exec_approval_timeout_secs,
        ));

        let cost_tracker = {
            use crate::llm::cost_tracker::CostTracker;
            let base = CostTracker::new();
            let base = base.attach_store(sqlite.as_ref().clone());
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
        }
        if config.cost_tracker_enabled {
            llm_builder = llm_builder.with_cost_tracker(cost_tracker.clone());
        }
        if config.token_juice_enabled {
            let compressor = Arc::new(crate::llm::token_juice::TokenJuiceCompressor::new(
                ollama_for_compress.clone(),
                config.chat_model.clone(),
                crate::llm::token_juice::TokenJuiceConfig::default(),
            ));
            llm_builder = llm_builder.with_token_juice(compressor);
        }
        if config.router_enabled {
            let router = Arc::new(crate::llm::model_router::ModelRouter::new(
                ollama_for_compress.clone(),
                config.router_classifier_model.clone(),
            ));
            llm_builder = llm_builder.with_model_router(router);
        }
        if config.daily_budget_usd > 0.0 {
            llm_builder = llm_builder.with_daily_budget(config.daily_budget_usd);
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
        }
        let llm = Arc::new(llm_builder);
        startup.mark("bootstrap.llm");

        let sponge = {
            let mut sponge_builder =
                SpongeEngine::new(sqlite.clone(), lance.clone(), embedder.clone());
            sponge_builder = sponge_builder.with_cost_tracker(cost_tracker.clone());
            match Self::load_acl_from_store(&sqlite) {
                Ok(acl) => {
                    sponge_builder = sponge_builder.with_acl(Arc::new(acl));
                }
                Err(e) => {
                    warn!(target: "nebula", error = %e, "ACL load failed (headless)");
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
            Ok(n) if n > 0 => info!(target: "nebula", count = n, "migrated env keys (headless)"),
            _ => {}
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
}
