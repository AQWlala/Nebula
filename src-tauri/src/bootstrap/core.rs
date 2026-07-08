use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::app_config::AppConfig;
use crate::app_state::AppState;
#[cfg(feature = "channels")]
use crate::channel::webchat::WebChatService;
use crate::llm::prefetch::PrefetchEngine;
use crate::memory::causal_graph::CausalGraphEngine;
use crate::memory::l0_cache::L0Cache;
use crate::memory::orchestrator::MemoryOrchestrator;
use crate::memory::summarizer::SummaryEngine;
use crate::memory::version_control::MemoryVersionControl;
use crate::os::ShellExecutor;
use crate::perf::StartupTimer;
use crate::sync::device_manager::DeviceManager;
use crate::tools::{shell_tool::ShellTool, ToolRegistry};
use crate::work::WorkEngine;
use crate::writing::WritingEngine;

impl AppState {
    /// Bootstraps a fully-wired [`AppState`] from the given config.
    ///
    /// On failure all already-initialised subsystems are dropped; the
    /// returned `anyhow::Error` carries the full context chain.
    pub async fn bootstrap(
        mut config: AppConfig,
        app_handle: tauri::AppHandle,
    ) -> anyhow::Result<Self> {
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
            &config,
            &sqlite,
            &lance,
            &embedder,
            &llm,
            &sponge,
            &tool_registry,
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
        let self_reflection = Arc::new(crate::memory::self_reflection::SelfReflectionEngine::new(
            sqlite.clone(),
            swarm.values_layer().clone(),
            reflection.config().clone(),
        ));
        startup.mark("bootstrap.self_reflection");

        // Phase 4: skills ecosystem
        let (skills, skill_extractor, skill_composer, marketplace, skill_audit_logger) =
            Self::bootstrap_skills(&config, &sqlite, &llm, &exec_approval);
        swarm.set_composer(skill_composer.clone());

        // v1.4: L0 缓存层 + Memory Orchestrator。
        let l0 = Arc::new(L0Cache::new());
        let mut orchestrator_builder =
            MemoryOrchestrator::new(sqlite.clone(), lance.clone(), embedder.clone(), l0.clone())
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
        // T-E-C-08: Shadow Workspace 引擎 — 注入 repo root 以启用 git worktree 隔离。
        let shadow_engine =
            Arc::new(crate::shadow_workspace::ShadowWorkspaceEngine::with_default());
        shadow_engine.set_repo_root(editor.workspace_root().to_path_buf());
        // T-E-C-10: 长任务引擎 — 复用 sqlite + shadow_engine,bootstrap 时恢复 Running→Paused。
        let long_task_engine = Arc::new(crate::long_task::LongTaskEngine::new(
            sqlite.clone(),
            shadow_engine.clone(),
        ));
        if let Err(e) = long_task_engine.bootstrap() {
            tracing::warn!(target: "nebula", error = %e, "long_task bootstrap failed");
        } else {
            tracing::info!(target: "nebula", "long_task engine bootstrapped");
        }
        let clipboard = Self::bootstrap_clipboard();
        let shell = Arc::new(ShellExecutor::new());
        tool_registry.register(Arc::new(ShellTool::new((*shell).clone())));
        let sync_transport = Self::bootstrap_sync(&config);
        startup.mark("bootstrap.end");

        // v1.8: 启动性能监控器（后台 1Hz 采样 RSS/CPU）。
        // T-D-B-03: handle 存入 InfraSubsystem 替代 std::mem::forget。
        let (perf_handle, perf_monitor) =
            crate::perf::monitor::PerfMonitor::start(std::time::Duration::from_secs(1));
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
            info!(
                target: "nebula",
                count = config.watch_paths.len(),
                "file watcher started (T-E-B-09)"
            );
        }

        // T-E-C-14: 构造 ClipboardWatcherEngine。此处只构造不启动。
        let clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>> =
            Arc::new(tokio::sync::Mutex::new(
                crate::os::ClipboardWatcherEngine::new(),
            ));
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
            let log_dir = crate::tracing_setup::default_log_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."));
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
        let arena = Arc::new(
            crate::llm::arena::ArenaLeaderboard::new().with_store(sqlite.as_ref().clone()),
        );
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

        // T-D-C-08: 从环境变量初始化 master-orchestrator 运行时开关
        #[cfg(feature = "master-orchestrator")]
        crate::swarm::init_master_orchestrator_from_env();

        // M5 #68 / M6 #82: ApprovalGate + ConfirmationRegistry + MasterOrchestrator
        let confirmation_registry = Arc::new(crate::autonomy::ConfirmationRegistry::new());
        let approval_gate = Arc::new(crate::autonomy::ApprovalGate::new(
            crate::autonomy::WorkerRiskMap::new(),
            confirmation_registry.clone(),
        ));
        info!(target: "nebula", "approval gate + confirmation registry ready (M5 #68)");
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
                info!(target: "nebula", "MasterOrchestrator ready (M6 #82, runtime ON, T-D-C-08)");
                mo
            } else {
                // 运行时开关关闭时仍构造占位 Arc，避免 Option 化整个字段
                let mo = Arc::new(crate::swarm::MasterOrchestrator::new(
                    swarm.clone(),
                    None,
                ));
                warn!(target: "nebula", "MasterOrchestrator disabled at runtime (T-D-C-08); commands will reject");
                mo
            }
        };

        // M6 #78: EvolutionEngine + EvolutionLog + Roller 构造。
        #[cfg(feature = "evolution-engine")]
        let (evolution_engine, evolution_log, roller) = {
            use crate::evolution::engine::{
                EvolutionEngine, EvolutionEngineConfig, EvolutionLog, Roller,
            };
            use std::path::PathBuf;
            let log_path = PathBuf::from("evolution_log.md");
            let soul_md_path = PathBuf::from("SOUL.md");
            let log = Arc::new(EvolutionLog::new(log_path));
            let roller = Arc::new(Roller::new(log.clone(), soul_md_path));
            let engine = {
                #[cfg(feature = "unified-dispatcher")]
                let dispatcher_opt = dispatcher.clone();
                #[cfg(not(feature = "unified-dispatcher"))]
                let dispatcher_opt: Option<
                    Arc<crate::llm::dispatcher::UnifiedModelDispatcher>,
                > = None;
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
                message_bridge,
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
                perf_handle,
                tool_registry,
                diagnostics,
                approval_gate,
                confirmation_registry,
            },
        })
    }
}
