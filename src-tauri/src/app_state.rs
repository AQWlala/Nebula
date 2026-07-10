//! [`AppState`] — the single managed-state struct shared across Tauri commands.
//!
//! T-D-B-16: 67 个 Arc 字段按职能分组为 6 个 SubSystem,降低认知负担并明确所有权边界。
//! 访问路径从 `state.xxx` 变为 `state.<subsystem>.xxx`。

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

/// 记忆子系统:存储 / 向量 / 嵌入 / 海绵黑洞 / 反思 / L0 缓存 / 编排 / 摘要 / 因果 / 版本 / 文件监控 / 自反思。
#[derive(Clone)]
pub struct MemorySubsystem {
    pub sqlite: Arc<SqliteStore>,
    /// T-E-S-42: 向量存储 trait 对象,运行时按 AppConfig.vector_store_backend
    /// 选择 LanceDB / Qdrant / ChromaDB 后端。11 个调用方面向 trait 编程。
    pub lance: Arc<dyn VectorStore>,
    pub embedder: Arc<Embedder>,
    pub sponge: Arc<SpongeEngine>,
    pub blackhole: Arc<BlackholeEngine>,
    /// v0.2: L5 reflection engine. Always present (even if LLM is
    /// unavailable, in which case the engine falls back to template
    /// synthesis).
    pub reflection: Arc<ReflectionEngine>,
    /// v0.3: handle to the reflection background worker, so the
    /// `AppState::shutdown` call can `await` the join.
    pub reflect_worker: Arc<Mutex<Option<JoinHandle<()>>>>,
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
    /// v2.0: 真正的 Self-Reflection 引擎（L5 元认知层升级）。
    pub self_reflection: Arc<crate::memory::self_reflection::SelfReflectionEngine>,
    /// T-E-B-09: 文件夹监控引擎。
    pub file_watcher: Arc<crate::memory::file_watcher::FileWatcherEngine>,
    /// T-E-B-09: file watcher 消费者 task 的 JoinHandle。
    pub file_watcher_worker: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
}

/// LLM 子系统:网关 / 费用追踪 / 预取 / 模型配置 / Arena / 内联补全 / 统一分发器 / 模型健康追踪。
#[derive(Clone)]
pub struct LlmSubsystem {
    pub llm: Arc<LlmGateway>,
    /// T-E-A-06: Token 费用追踪器。
    pub cost_tracker: Arc<crate::llm::cost_tracker::CostTracker>,
    /// T-E-A-11: Smart Prefetch 引擎。
    pub prefetch: Arc<PrefetchEngine>,
    /// T-E-S-41: models.json 动态配置。
    pub models_config: Arc<parking_lot::RwLock<crate::llm::models_config::ModelsConfig>>,
    /// T-E-A-14: Arena A/B 测试。
    pub arena: Arc<crate::llm::arena::ArenaLeaderboard>,
    /// T-E-S-51: Level 0 内联补全引擎。
    pub inline_completion: Arc<crate::editor::InlineCompletionEngine>,
    /// M7a #86: UnifiedModelDispatcher。
    /// P0-2: unified-dispatcher 默认启用；运行时关闭或未注入时为 None。
    pub dispatcher: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>>,
    /// P1-1: 模型健康追踪器 — 记录每个 provider 的延迟 / 错误 / 断路器状态。
    pub model_health_tracker: Arc<crate::llm::model_health::ModelHealthTracker>,
}

/// 蜂群子系统:编排器 / 技能生态 / 事件总线 / 死锁检测 / 主控 / 进化 / 影子工作区 / 长任务 / 触发器 / 场景模板。
#[derive(Clone)]
pub struct SwarmSubsystem {
    pub swarm: Arc<SwarmOrchestrator>,
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
    /// T-E-S-20: exec 类操作审批注册表。
    pub exec_approval: Arc<crate::skills::exec_approval::ExecApprovalTracker>,
    /// T-E-S-26: EventBus — 协议化事件总线。
    pub event_bus: Arc<crate::swarm::event_bus::EventBus>,
    /// T-E-S-05: 死锁检测器。
    pub deadlock_detector: Arc<crate::swarm::DeadlockDetector>,
    /// T-E-S-54: 事件触发器引擎。
    pub trigger_engine: Arc<crate::triggers::TriggerEngine>,
    /// T-E-C-13: 工作场景模板引擎。
    pub scenario_templates: Arc<crate::scenarios::TemplateEngine>,
    /// T-E-C-08: Shadow Workspace 引擎 — Agent 隔离执行环境(git worktree)。
    pub shadow_engine: Arc<crate::shadow_workspace::ShadowWorkspaceEngine>,
    /// T-E-C-10: 长任务引擎 — 后台分步执行跨小时/跨天的复杂任务。
    pub long_task_engine: Arc<crate::long_task::LongTaskEngine>,
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
}

/// 通道子系统:消息桥 / 路由器 / WebChat / IM 绑定。
#[derive(Clone)]
pub struct ChannelSubsystem {
    /// v1.2: multi-channel message bridge (JiWenSwarm delivery fabric).
    #[cfg(feature = "channels")]
    pub message_bridge: Option<Arc<crate::channel::bridge::MessageBridge>>,
    /// T-S3-B-01: 原生通信渠道路由器（Telegram/Discord 直连，不经过 JiuWenSwarm）。
    #[cfg(feature = "channels")]
    pub channel_router: Arc<crate::channel::ChannelRouter>,
    /// v1.3: WebChat share link service.
    #[cfg(feature = "channels")]
    pub webchat_service: crate::channel::webchat::WebChatService,
    /// T-E-C-17: IM 绑定引擎。
    pub im_engine: Arc<crate::im::ImEngine>,
}

/// 平台子系统:写作 / 工作 / 编辑器 / 剪贴板 / Shell / 同步 / 设备 / 通知 / 快照 / 存储 / OAuth / gRPC / MCP / Sidecar / Wiki / 剪贴板监听。
#[derive(Clone)]
pub struct PlatformSubsystem {
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
    /// v1.3: device manager for sync pairing.
    pub device_manager: Arc<parking_lot::Mutex<DeviceManager>>,
    /// T-E-S-57: 后台通知服务。
    pub notification_service: Arc<crate::notify::NotificationService>,
    /// T-E-S-24: 文件快照回滚引擎。
    pub snapshot_engine: Arc<crate::snapshot::SnapshotEngine>,
    /// T-E-S-44: 统一存储后端。
    pub storage: crate::storage::DynStorageBackend,
    /// P1-B: OAuth 2.0 manager — aggregates providers, manages token lifecycle.
    pub oauth_manager: Arc<crate::identity::OAuthManager>,
    /// v0.3: handle to the in-process gRPC server task.
    #[cfg(feature = "grpc")]
    pub grpc_server: Arc<Mutex<Option<crate::grpc::GrpcHandle>>>,
    /// v1.3: MCP manager (feature-gated).
    #[cfg(feature = "mcp")]
    pub mcp_manager: Arc<crate::mcp::client::McpManager>,
    /// T-E-S-32: MCP Server Registry。
    #[cfg(feature = "mcp")]
    pub mcp_registry: Arc<crate::mcp::registry::McpServerRegistry>,
    /// v2.0: Sidecar 进程管理器。
    pub sidecar_manager: crate::sidecar::SidecarManager,
    /// T-E-B-01: LLM Wiki 编译引擎。
    pub wiki: Arc<crate::wiki::WikiCompiler>,
    /// T-E-C-14: 剪贴板监听引擎。
    pub clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>>,
}

/// 基础设施子系统:配置 / 启动计时器 / 性能监控 / 工具注册 / 诊断 / 审批门禁 / 确认注册。
#[derive(Clone)]
pub struct InfraSubsystem {
    pub config: Arc<AppConfig>,
    /// v1.0: startup profiler (milestones + final report).
    pub startup_timer: StartupTimer,
    /// v1.8: 性能监控器（后台 1Hz 采样 RSS/CPU）。
    pub perf_monitor: crate::perf::monitor::PerfMonitor,
    /// T-D-B-03: PerfMonitor 后台任务的 abort 句柄。
    /// 存入 AppState 替代 `std::mem::forget`,shutdown 时正常 Drop 停止采样。
    pub perf_handle: crate::perf::monitor::MonitorHandle,
    /// v1.1 P0-2: tool registry with registered tools (shell, etc.).
    pub tool_registry: Arc<ToolRegistry>,
    /// T-E-S-27: Trusted Diagnostics 通道。
    pub diagnostics: Arc<crate::diagnostics::bus::DiagnosticsBus>,
    /// M5 #68: L4 审批门禁 + nonce 防重放 + 5 分钟超时。
    pub approval_gate: Arc<crate::autonomy::ApprovalGate>,
    /// M5 #68: Pending confirmations 注册表。
    pub confirmation_registry: Arc<crate::autonomy::ConfirmationRegistry>,
}

/// The single managed-state struct shared across Tauri commands.
///
/// All subsystems are stored as `Arc` so that Tauri commands can clone
/// individual handles cheaply without locking the entire state.
///
/// T-D-B-16: 67 个字段按职能分组为 6 个 SubSystem。
#[derive(Clone)]
pub struct AppState {
    pub memory: MemorySubsystem,
    pub llm: LlmSubsystem,
    pub swarm: SwarmSubsystem,
    pub channels: ChannelSubsystem,
    pub platform: PlatformSubsystem,
    pub infra: InfraSubsystem,
}
