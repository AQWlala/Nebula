//! [`AppState`] — the single managed-state struct shared across Tauri commands.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::task::JoinHandle;

use crate::app_config::AppConfig;
use crate::editor::EditorState;
use crate::llm::gateway::LlmGateway;
use crate::llm::prefetch::PrefetchEngine;
use crate::memory::blackhole::BlackholeEngine;
use crate::memory::causal_graph::CausalGraphEngine;
use crate::memory::embedder::Embedder;
use crate::memory::l0_cache::L0Cache;
use crate::memory::orchestrator::MemoryOrchestrator;
use crate::memory::reflect::ReflectionEngine;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::summarizer::SummaryEngine;
use crate::memory::vector_store::VectorStore;
use crate::memory::version_control::MemoryVersionControl;
use crate::os::ClipboardService;
use crate::os::ShellExecutor;
use crate::perf::StartupTimer;
use crate::skills::audit::SkillAuditLogger;
use crate::skills::engine::SkillEngine;
use crate::skills::extractor::SkillExtractor;
use crate::swarm::composer::SkillComposer;
use crate::swarm::orchestrator::SwarmOrchestrator;
use crate::sync::device_manager::DeviceManager;
use crate::sync::LocalTransport;
use crate::tools::ToolRegistry;
use crate::work::WorkEngine;
use crate::writing::WritingEngine;

/// The single managed-state struct shared across Tauri commands.
///
/// All subsystems are stored as `Arc` so that Tauri commands can clone
/// individual handles cheaply without locking the entire state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub sqlite: Arc<SqliteStore>,
    /// T-E-S-42: 向量存储 trait 对象,运行时按 AppConfig.vector_store_backend
    /// 选择 LanceDB / Qdrant / ChromaDB 后端。11 个调用方面向 trait 编程。
    pub lance: Arc<dyn VectorStore>,
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
    /// v1.2: skill closed-loop learning — auto-extracts reusable skills from swarm tasks.
    pub skill_extractor: Arc<SkillExtractor>,
    /// v1.2: skill auto-composer for orchestration upgrade.
    pub skill_composer: Arc<SkillComposer>,
    /// v1.3 P2-7: skill marketplace — search, install, update, publish.
    pub marketplace: Arc<crate::skills::SkillMarketplace>,
    /// v1.3: skill audit logger.
    pub skill_audit_logger: Arc<SkillAuditLogger>,
    #[cfg(feature = "channels")]
    /// v1.2: multi-channel message bridge (JiWenSwarm delivery fabric).
    pub message_bridge: Option<Arc<crate::channel::bridge::MessageBridge>>,
    /// T-S3-B-01: 原生通信渠道路由器（Telegram/Discord 直连，不经过 JiuWenSwarm）。
    #[cfg(feature = "channels")]
    pub channel_router: Arc<crate::channel::ChannelRouter>,
    /// v0.3: handle to the reflection background worker, so the
    /// `AppState::shutdown` call can `await` the join.
    pub reflect_worker: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// v0.3: handle to the in-process gRPC server task.
    #[cfg(feature = "grpc")]
    pub grpc_server: Arc<Mutex<Option<crate::grpc::GrpcHandle>>>,
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
    /// v1.8: 性能监控器（后台 1Hz 采样 RSS/CPU）。
    pub perf_monitor: crate::perf::monitor::PerfMonitor,
    /// v1.1 P0-2: tool registry with registered tools (shell, etc.).
    pub tool_registry: Arc<ToolRegistry>,
    #[cfg(feature = "channels")]
    /// v1.3: WebChat share link service.
    pub webchat_service: crate::channel::webchat::WebChatService,
    /// v1.3: device manager for sync pairing.
    pub device_manager: Arc<parking_lot::Mutex<DeviceManager>>,
    /// v1.3: MCP manager (feature-gated).
    #[cfg(feature = "mcp")]
    pub mcp_manager: Arc<crate::mcp::client::McpManager>,
    /// v1.4: L0 缓存层（LRU 热记忆 + 会话上下文窗口 + 预取队列）。
    pub l0: Arc<L0Cache>,
    /// v1.4: Memory Orchestrator（L2 认知层协调器）。
    pub orchestrator: Arc<MemoryOrchestrator>,
    /// v1.5: LLM 驱动的多粒度摘要引擎。
    pub summary_engine: Arc<SummaryEngine>,
    /// v1.5: 因果图谱推理引擎。
    pub causal_graph: Arc<CausalGraphEngine>,
    /// v1.6: Git 风格记忆版本控制。
    pub version_control: Arc<MemoryVersionControl>,
    /// v2.0: Sidecar 进程管理器。
    pub sidecar_manager: crate::sidecar::SidecarManager,
    /// v2.0: 真正的 Self-Reflection 引擎（L5 元认知层升级）。
    pub self_reflection: Arc<crate::memory::self_reflection::SelfReflectionEngine>,
    /// T-E-S-20: exec 类操作审批注册表。
    pub exec_approval: Arc<crate::skills::exec_approval::ExecApprovalTracker>,
    /// T-E-A-06: Token 费用追踪器。
    pub cost_tracker: Arc<crate::llm::cost_tracker::CostTracker>,
    /// T-E-S-51: Level 0 内联补全引擎。
    pub inline_completion: Arc<crate::editor::InlineCompletionEngine>,
    /// T-E-S-27: Trusted Diagnostics 通道。
    pub diagnostics: Arc<crate::diagnostics::bus::DiagnosticsBus>,
    /// T-E-B-09: 文件夹监控引擎。
    pub file_watcher: Arc<crate::memory::file_watcher::FileWatcherEngine>,
    /// T-E-B-09: file watcher 消费者 task 的 JoinHandle。
    pub file_watcher_worker: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
    /// T-E-C-14: 剪贴板监听引擎。
    pub clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>>,
    /// T-E-S-41: models.json 动态配置。
    pub models_config: Arc<parking_lot::RwLock<crate::llm::models_config::ModelsConfig>>,
    /// T-E-S-57: 后台通知服务。
    pub notification_service: Arc<crate::notify::NotificationService>,
    /// T-E-S-24: 文件快照回滚引擎。
    pub snapshot_engine: Arc<crate::snapshot::SnapshotEngine>,
    /// T-E-S-05: 死锁检测器。
    pub deadlock_detector: Arc<crate::swarm::DeadlockDetector>,
    /// T-E-S-26: EventBus — 协议化事件总线。
    pub event_bus: Arc<crate::swarm::event_bus::EventBus>,
    /// T-E-S-54: 事件触发器引擎。
    pub trigger_engine: Arc<crate::triggers::TriggerEngine>,
    /// T-E-C-13: 工作场景模板引擎。
    pub scenario_templates: Arc<crate::scenarios::TemplateEngine>,
    /// T-E-C-17: IM 绑定引擎。
    pub im_engine: Arc<crate::im::ImEngine>,
    /// T-E-S-44: 统一存储后端。
    pub storage: crate::storage::DynStorageBackend,
    /// T-E-A-11: Smart Prefetch 引擎。
    pub prefetch: Arc<PrefetchEngine>,
    /// T-E-B-01: LLM Wiki 编译引擎。
    pub wiki: Arc<crate::wiki::WikiCompiler>,
    /// T-E-S-32: MCP Server Registry。
    #[cfg(feature = "mcp")]
    pub mcp_registry: Arc<crate::mcp::registry::McpServerRegistry>,
    /// T-E-A-14: Arena A/B 测试。
    pub arena: Arc<crate::llm::arena::ArenaLeaderboard>,
    /// M5 #68: L4 审批门禁 + nonce 防重放 + 5 分钟超时。
    pub approval_gate: Arc<crate::autonomy::ApprovalGate>,
    /// M5 #68: Pending confirmations 注册表。
    pub confirmation_registry: Arc<crate::autonomy::ConfirmationRegistry>,
    /// M6 #82: MasterOrchestrator。
    #[cfg(feature = "master-orchestrator")]
    pub master_orchestrator: Arc<crate::swarm::MasterOrchestrator>,
    /// M6 #78: EvolutionEngine。
    #[cfg(feature = "evolution-engine")]
    pub evolution_engine: Option<Arc<crate::evolution::engine::EvolutionEngine>>,
    /// M6 #78: EvolutionLog。
    #[cfg(feature = "evolution-engine")]
    pub evolution_log: Arc<crate::evolution::engine::EvolutionLog>,
    /// M6 #78: Roller — SOUL.md evolution-append 段落级回滚器。
    #[cfg(feature = "evolution-engine")]
    pub roller: Arc<crate::evolution::engine::Roller>,
    /// M7a #86: UnifiedModelDispatcher。
    #[cfg(feature = "unified-dispatcher")]
    pub dispatcher: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>>,
}
