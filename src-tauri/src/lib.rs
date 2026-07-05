//! `nebula_lib` — the library crate backing the `nebula` binary.
//!
//! The crate is organised as a small collection of mostly independent
//! subsystems that communicate through a shared [`AppState`] living inside
//! the Tauri managed-state container:
//!
//! * [`memory`]   — the 5-layer v7.0 memory system (L0-L5)
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

// Clippy: allow style lints that are noisy across this codebase but do
// not indicate correctness issues.  Individual modules may still fix
// them opportunistically.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::derivable_impls,
    clippy::should_implement_trait,
    clippy::manual_strip,
    clippy::len_without_is_empty,
    clippy::unnecessary_sort_by,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::needless_borrow,
    clippy::manual_clamp,
    clippy::empty_line_after_doc_comments,
    // clippy::await_holding_lock — P0 修复:重新启用此 lint,所有 MutexGuard 跨 await
    // 必须用块作用域 drop,避免静默引入死锁。cost_tracker.rs / arena.rs 已正确处理。
    clippy::field_reassign_with_default,
    clippy::new_without_default
)]

pub mod api;
#[cfg(feature = "channels")]
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
pub mod identity;
pub mod security;
pub mod skills;
pub mod swarm;
pub mod sync;
// v1.3: MCP protocol client (feature-gated).
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod work;
pub mod writing;
// v1.1 P0-2: Tool abstraction layer.
pub mod tools;
// T-E-S-35: 5 层插件模型 — Filter/Action/Pipe/Tool/Skill 分层(Open WebUI)。
pub mod plugins;
// T-E-S-50: 自主度滑块 L0-L5 框架。
pub mod autonomy;
// v1.3: Plan 模式 + 准奏环节（L4 价值层配套）。
pub mod plan;
// v1.8: observability layer (OpenTelemetry tracing export, gated).
pub mod observability;
// T-E-S-27: Trusted Diagnostics Channels — 独立可信诊断通道。
pub mod diagnostics;
// v2.0: Sidecar 进程拆分 — 多进程架构（Memory/LLM/Swarm 独立进程）。
pub mod sidecar;
// T-S6-A-03: 自动备份 — 每日 02:00 备份 SQLite + LanceDB。
pub mod backup;
// T-E-S-24: 文件快照回滚引擎。
pub mod snapshot;
// T-E-S-57: 后台执行通知服务。
pub mod notify;
// T-E-S-54: 事件触发器(文件/消息/Webhook 三种触发器统一调度)。
pub mod triggers;
// T-E-C-13: 工作场景模板库(Writer/Coder/Manager 三套场景模板 + 工作流)。
pub mod scenarios;
// T-E-S-44: StorageBackend trait — 统一 Local/S3/WebDAV 存储后端抽象。
pub mod storage;
// T-E-C-17: IM 扫码绑定(Feishu/WeCom/DingTalk webhook 推送)。
pub mod im;
// T-E-B-01: LLM Wiki 编译引擎(每次对话后异步编译结构化 Markdown 笔记)。
pub mod wiki;

// v1.3: closed-loop self-evolution (task outcomes + skill archive +
// prompt mutator).  Off by default; enable with
// `--features self-evolution`.
#[cfg(feature = "self-evolution")]
pub mod evolution;

// M1 里程碑：Soul 系统（SOUL.md + SoulCompiler + 注入防护 + 原子写入）。
// 默认 off；启用需 `--features soul-system`。
// 运行时还需环境变量 `SOUL_SYSTEM_ENABLED=1` 或 Settings UI 显式开启。
// 参见 ADR-004 Feature Flag 策略。
#[cfg(feature = "soul-system")]
pub mod soul;

#[cfg(feature = "grpc")]
pub mod grpc;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use parking_lot::Mutex;
use tauri::{Emitter, Manager};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, registry, util::SubscriberInitExt as _, EnvFilter};

#[cfg(feature = "channels")]
use crate::channel::bridge::MessageBridge;
#[cfg(feature = "channels")]
use crate::channel::webchat::WebChatService;
use crate::editor::EditorState;
use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::OllamaClient;
// T-E-A-11: Smart Prefetch 引擎 + SemanticCache 共享注入。
use crate::llm::prefetch::PrefetchEngine;
use crate::llm::semantic_cache::SemanticCache;
// T-E-S-40: OpenAI 兼容 provider 客户端。
use crate::llm::openai_compat::OpenAICompatClient;
use crate::memory::acl::{AclEffect, AclPermission, AclRule, MemoryAcl};
use crate::memory::blackhole::BlackholeEngine;
use crate::memory::causal_graph::CausalGraphEngine;
use crate::memory::embedder::Embedder;
use crate::memory::l0_cache::L0Cache;
// T-E-S-42: AppState.lance 改为 Arc<dyn VectorStore>,支持运行时切换后端。
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

/// Configuration sourced from environment variables (with sensible defaults).
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Path to the SQLite database file.
    pub db_path: String,
    /// Path to the LanceDB vector store directory.
    pub lance_path: String,
    /// Base URL of the Ollama HTTP server. 仍用于 embedding (bge-small-zh)。
    /// 若要禁用本地 Ollama,设为空字符串。
    pub ollama_url: String,
    /// 默认 chat 模型名。v1.2 起默认 `deepseek-chat`。
    /// 可选值:deepseek-chat / deepseek-reasoner / qwen2.5:3b / claude-3-5-haiku 等。
    pub chat_model: String,
    /// 默认 embedding 模型名 (仍走 Ollama)。
    pub embed_model: String,
    /// 主 LLM provider (deepseek / ollama / openai-compat / anthropic)。
    /// v1.2 起默认 `deepseek`。决定 chat 请求优先走哪条路径。
    pub llm_provider: String,
    /// DeepSeek API base URL (主路径)。
    pub deepseek_api_url: String,
    /// DeepSeek API key (从 DEEPSEEK_API_KEY 读取)。
    pub deepseek_api_key: Option<String>,
    /// 可选的远程 fallback URL (OpenAI 兼容,如 Azure OpenAI / 自建端点)。
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
    /// `NEBULA_GRPC=0` to disable). The gRPC port is configured
    /// via `grpc_bind_addr`.
    pub grpc_enabled: bool,
    /// v0.3: bind address for the gRPC server. Default
    /// `127.0.0.1:50051`.
    pub grpc_bind_addr: String,
    /// T-S2-B-03a: enable the REST API server. Default `false`
    /// (set `NEBULA_REST=1` to enable). Requires `rest-api` feature.
    pub rest_enabled: bool,
    /// T-S2-B-03a: bind address for the REST API server. Default
    /// `127.0.0.1:8080`.
    pub rest_bind_addr: String,
    /// T-S2-B-03a: Bearer token / API key for REST API auth.
    /// If empty, auth is bypassed (development mode only).
    /// Set `NEBULA_REST_TOKEN` to require auth.
    pub rest_api_token: Option<String>,
    /// v0.5: workspace root for the editor.  All file operations
    /// are sandboxed to this directory.
    pub editor_workspace: String,
    /// v0.5: directory used by the local sync transport.  Defaults
    /// to `<config_dir>/sync_inbox`.
    pub sync_inbox: String,
    /// T-E-S-20: exec 类操作审批超时(秒)。超时后 fail-closed 自动拒绝。
    /// 默认 60,可设 `NEBULA_EXEC_APPROVAL_TIMEOUT_SECS` 覆盖。
    pub exec_approval_timeout_secs: u64,
    /// T-E-A-01: 是否启用 L0.5 语义缓存。默认 true。
    /// 设 `NEBULA_SEMANTIC_CACHE=0` 关闭。
    pub semantic_cache_enabled: bool,
    /// T-E-A-06: 是否启用 Token 费用追踪。默认 true。
    /// 设 `NEBULA_COST_TRACKER=0` 关闭。
    pub cost_tracker_enabled: bool,
    /// T-E-A-02: 是否启用 TokenJuice 三级压缩。默认 true。
    /// 设 `NEBULA_TOKEN_JUICE=0` 关闭。
    pub token_juice_enabled: bool,
    /// T-E-A-03: 是否启用 ModelRouter 智能路由。默认 true。
    /// 设 `NEBULA_ROUTER=0` 关闭。
    pub router_enabled: bool,
    /// T-E-A-03: ModelRouter 分类器模型名。默认 `qwen2.5:3b`。
    pub router_classifier_model: String,
    /// T-E-A-04: 是否启用 Prefix-Cache 适配层。默认 true。
    /// 设 `NEBULA_PREFIX_CACHE=0` 关闭。
    pub prefix_cache_enabled: bool,
    /// T-E-A-05: 日预算(USD),0=不限制。env: NEBULA_DAILY_BUDGET_USD
    pub daily_budget_usd: f64,
    /// T-E-A-12: 自动化(触发器/Cron/后台)每日预算(USD),None=不限制。
    /// 超阈值时 emit `budget_exceeded` 事件。env: NEBULA_AUTOMATION_DAILY_BUDGET_USD
    pub automation_daily_budget_usd: Option<f64>,
    /// T-E-B-09: 启动时监控的文件夹列表(逗号分隔)。
    /// env: NEBULA_WATCH_PATHS
    pub watch_paths: Vec<String>,
    /// T-E-C-14: 是否启用剪贴板智能监听。默认 false(需用户显式开启)。
    /// env: NEBULA_CLIPBOARD_WATCH=1
    pub clipboard_watch_enabled: bool,
    /// T-E-S-40: OpenAI 兼容 provider base URL(vLLM/LMStudio/OpenRouter/自建)。
    /// env: NEBULA_OPENAI_COMPAT_URL。配置后 llm_provider 应设为 "openai-compat"。
    pub openai_compat_base_url: Option<String>,
    /// T-E-S-40: OpenAI 兼容 provider API key(可空,适配 LMStudio 本地)。
    /// env: NEBULA_OPENAI_COMPAT_KEY。
    pub openai_compat_api_key: Option<String>,
    /// T-E-S-40: OpenAI 兼容 provider 默认模型名(如 "openai/gpt-4o-mini" / "llama-3.1-8b-instruct")。
    /// env: NEBULA_OPENAI_COMPAT_MODEL。
    pub openai_compat_model: Option<String>,
    /// T-E-S-27: 是否启用 Trusted Diagnostics 通道。默认 true。
    /// 设 `NEBULA_DIAGNOSTICS=0` 关闭。
    pub diagnostics_channel_enabled: bool,
    /// T-E-S-27: DiagnosticsBus broadcast 容量。默认 512。
    /// env: NEBULA_DIAGNOSTICS_BUFFER_CAPACITY
    pub diagnostics_buffer_capacity: usize,
    /// T-E-S-41: models.json 路径(`<app_data_dir>/models.json`)。
    /// 由 `ModelsConfig::resolve_path()` 解析;失败回退 `./models.json`。
    pub models_config_path: std::path::PathBuf,
    /// T-E-S-39: SOUL.md/AGENTS.md/TOOLS.md persona 缓存(运行时共享)。
    /// None 直到首次加载;bootstrap 预加载或 persona_reload 命令后填充。
    pub persona: Option<Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>>,
    /// M1 任务 #23: SoulCompiler 实例（cfg-gated）。
    ///
    /// 仅在 `soul-system` feature 编译且 `SOUL_SYSTEM_ENABLED` 运行时开启时可用。
    /// 优先级：Soul > PersonaConfig。当 Soul 编译成功时，CompiledSoul.system_prompt
    /// 替代 PersonaConfig 的 `<soul>` 部分；AGENTS.md / TOOLS.md 仍由 PersonaConfig 提供。
    #[cfg(feature = "soul-system")]
    pub soul_compiler: Option<Arc<crate::soul::SoulCompiler>>,
    /// T-E-S-57: 是否启用系统通知。默认 true。
    /// 设 `NEBULA_NOTIFICATIONS=0` 关闭。
    pub notifications_enabled: bool,
    /// T-E-S-57: 是否在悬浮球上显示任务计数角标。默认 true。
    /// 设 `NEBULA_BALL_BADGE=0` 关闭。
    pub floating_ball_task_badge: bool,
    /// T-E-S-44: 存储后端配置(local/s3/webdav)。
    pub storage_backend: crate::storage::StorageConfig,
    /// T-E-S-42: 向量存储后端类型(lance / qdrant / chroma)。默认 lance。
    /// env: NEBULA_VECTOR_STORE_BACKEND
    pub vector_store_backend: VectorStoreBackend,
    /// T-E-S-42: Qdrant REST API 根 URL(如 `http://127.0.0.1:6333`)。
    /// 仅当 `vector_store_backend = qdrant` 时使用。
    /// env: NEBULA_QDRANT_URL
    pub qdrant_url: Option<String>,
    /// T-E-S-42: ChromaDB REST API 根 URL(如 `http://127.0.0.1:8000`)。
    /// 仅当 `vector_store_backend = chroma` 时使用。
    /// env: NEBULA_CHROMA_URL
    pub chroma_url: Option<String>,
    /// T-E-B-01: 是否启用 LLM Wiki 编译引擎。默认 true。
    /// 设 `NEBULA_WIKI=0` 关闭(chat_stream 后不编译笔记)。
    pub wiki_enabled: bool,
    /// T-E-B-01: Wiki 笔记子目录名(默认 "wiki")。
    /// env: NEBULA_WIKI_SUBDIR
    pub wiki_subdir: String,
    /// T-E-S-43: 是否启用 DB 加密(SQLCipher)。默认 false。
    /// env: NEBULA_DB_ENCRYPTION=1
    /// 注:实际加密由 `db_encryption_enable` 命令触发迁移,
    /// 此字段仅指示当前是否应使用加密模式打开 DB。
    pub db_encryption_enabled: bool,
    /// T-E-S-32: mcp_servers.json 配置路径(可选)。
    /// None 时使用默认路径 `<app_data_dir>/mcp_servers.json`。
    /// env: NEBULA_MCP_SERVERS_PATH
    pub mcp_servers_path: Option<std::path::PathBuf>,
    /// T-E-C-02: 视觉模型名称(默认 qwen2.5-vl:3b),供 ScreenReader
    /// 通过 Ollama 多模态 API 理解截图。env: NEBULA_VISION_MODEL
    pub vision_model: String,
}

impl AppConfig {
    /// Loads configuration from environment variables, falling back to
    /// defaults appropriate for a first-run local development setup.
    pub fn from_env() -> Self {
        Self {
            db_path: std::env::var("NEBULA_DB").unwrap_or_else(|_| "nebula.db".to_string()),
            lance_path: std::env::var("NEBULA_LANCE")
                .unwrap_or_else(|_| "nebula_lance".to_string()),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string()),
            chat_model: std::env::var("NEBULA_CHAT_MODEL")
                .unwrap_or_else(|_| "deepseek-chat".to_string()),
            embed_model: std::env::var("NEBULA_EMBED_MODEL")
                .unwrap_or_else(|_| "BAAI/bge-small-zh-v1.5".to_string()),
            // v1.2: 默认走 DeepSeek;设 NEBULA_LLM_PROVIDER=ollama 可回退本地。
            llm_provider: std::env::var("NEBULA_LLM_PROVIDER")
                .unwrap_or_else(|_| "deepseek".to_string()),
            deepseek_api_url: std::env::var("DEEPSEEK_API_URL")
                .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string()),
            deepseek_api_key: std::env::var("DEEPSEEK_API_KEY").ok(),
            remote_fallback_url: std::env::var("NEBULA_REMOTE_URL").ok(),
            blackhole_threshold_days: std::env::var("NEBULA_BH_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            embedding_dim: std::env::var("NEBULA_EMBED_DIM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(512),
            reflect_interval_secs: std::env::var("NEBULA_REFLECT_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_INTERVAL_SECS),
            reflect_window_days: std::env::var("NEBULA_REFLECT_WINDOW_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_WINDOW_DAYS),
            reflect_min_importance: std::env::var("NEBULA_REFLECT_MIN_IMPORTANCE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_MIN_IMPORTANCE),
            grpc_enabled: std::env::var("NEBULA_GRPC")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            grpc_bind_addr: std::env::var("NEBULA_GRPC_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:50051".to_string()),
            rest_enabled: std::env::var("NEBULA_REST")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            rest_bind_addr: std::env::var("NEBULA_REST_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            rest_api_token: std::env::var("NEBULA_REST_TOKEN").ok().filter(|t| !t.is_empty()),
            editor_workspace: std::env::var("NEBULA_WORKSPACE")
                .unwrap_or_else(|_| ".".to_string()),
            sync_inbox: std::env::var("NEBULA_SYNC_INBOX")
                .unwrap_or_else(|_| "sync_inbox".to_string()),
            exec_approval_timeout_secs: std::env::var("NEBULA_EXEC_APPROVAL_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            semantic_cache_enabled: std::env::var("NEBULA_SEMANTIC_CACHE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            cost_tracker_enabled: std::env::var("NEBULA_COST_TRACKER")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            token_juice_enabled: std::env::var("NEBULA_TOKEN_JUICE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            router_enabled: std::env::var("NEBULA_ROUTER")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            router_classifier_model: std::env::var("NEBULA_ROUTER_MODEL")
                .unwrap_or_else(|_| "qwen2.5:3b".to_string()),
            prefix_cache_enabled: std::env::var("NEBULA_PREFIX_CACHE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            daily_budget_usd: std::env::var("NEBULA_DAILY_BUDGET_USD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            // T-E-A-12: 自动化每日预算(None=不限制),超阈值 emit budget_exceeded。
            automation_daily_budget_usd: std::env::var("NEBULA_AUTOMATION_DAILY_BUDGET_USD")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| *v > 0.0),
            // T-E-S-40: OpenAI 兼容 provider 配置(可选)。
            openai_compat_base_url: std::env::var("NEBULA_OPENAI_COMPAT_URL").ok(),
            openai_compat_api_key: std::env::var("NEBULA_OPENAI_COMPAT_KEY").ok(),
            openai_compat_model: std::env::var("NEBULA_OPENAI_COMPAT_MODEL").ok(),
            // T-E-S-27: Trusted Diagnostics 配置(默认开启,容量 512)。
            diagnostics_channel_enabled: std::env::var("NEBULA_DIAGNOSTICS")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            diagnostics_buffer_capacity: std::env::var("NEBULA_DIAGNOSTICS_BUFFER_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(512),
            // T-E-B-09: 文件夹监控路径(逗号分隔),默认空 Vec。
            watch_paths: std::env::var("NEBULA_WATCH_PATHS")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            // T-E-C-14: 剪贴板监听开关,默认 false(用户需显式开启)。
            clipboard_watch_enabled: std::env::var("NEBULA_CLIPBOARD_WATCH")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            // T-E-S-41: models.json 路径(<app_data_dir>/models.json)。
            models_config_path: crate::llm::models_config::ModelsConfig::resolve_path(),
            // T-E-S-39: persona 缓存,由 bootstrap 预加载填充。
            persona: None,
            // M1 任务 #23: SoulCompiler 实例（cfg-gated）。
            // 由 bootstrap 在 dispatcher 构造后填充（若 soul_system_enabled）。
            #[cfg(feature = "soul-system")]
            soul_compiler: None,
            // T-E-S-57: 通知开关,默认开启。
            notifications_enabled: std::env::var("NEBULA_NOTIFICATIONS")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            // T-E-S-57: 悬浮球任务角标开关,默认开启。
            floating_ball_task_badge: std::env::var("NEBULA_BALL_BADGE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            // T-E-S-44: 存储后端配置。env: NEBULA_STORAGE_BACKEND
            // (local/s3/webdav,默认 local)。
            storage_backend: {
                let kind = std::env::var("NEBULA_STORAGE_BACKEND")
                    .unwrap_or_else(|_| "local".to_string());
                let root = std::env::var("NEBULA_STORAGE_ROOT")
                    .unwrap_or_else(|_| {
                        std::env::temp_dir()
                            .join("nebula-storage")
                            .to_string_lossy()
                            .to_string()
                    });
                crate::storage::StorageConfig {
                    kind,
                    root,
                    webdav_url: std::env::var("NEBULA_WEBDAV_URL").ok(),
                    webdav_username: std::env::var("NEBULA_WEBDAV_USERNAME").ok(),
                    webdav_password: std::env::var("NEBULA_WEBDAV_PASSWORD").ok(),
                    s3_bucket: std::env::var("NEBULA_S3_BUCKET").ok(),
                    s3_region: std::env::var("NEBULA_S3_REGION").ok(),
                    s3_endpoint: std::env::var("NEBULA_S3_ENDPOINT").ok(),
                }
            },
            // T-E-S-42: 向量存储后端类型(lance / qdrant / chroma)。默认 lance。
            vector_store_backend: {
                let raw = std::env::var("NEBULA_VECTOR_STORE_BACKEND")
                    .unwrap_or_else(|_| "lance".to_string());
                match raw.to_lowercase().as_str() {
                    "qdrant" => VectorStoreBackend::Qdrant,
                    "chroma" => VectorStoreBackend::Chroma,
                    _ => VectorStoreBackend::Lance,
                }
            },
            // T-E-S-42: Qdrant REST API 根 URL(可选)。
            qdrant_url: std::env::var("NEBULA_QDRANT_URL").ok(),
            // T-E-S-42: ChromaDB REST API 根 URL(可选)。
            chroma_url: std::env::var("NEBULA_CHROMA_URL").ok(),
            // T-E-B-01: Wiki 编译引擎开关,默认开启。
            wiki_enabled: std::env::var("NEBULA_WIKI")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            // T-E-B-01: Wiki 笔记子目录,默认 "wiki"。
            wiki_subdir: std::env::var("NEBULA_WIKI_SUBDIR")
                .unwrap_or_else(|_| "wiki".to_string()),
            // T-E-S-43: DB 加密开关,默认 false(用户需显式开启)。
            db_encryption_enabled: std::env::var("NEBULA_DB_ENCRYPTION")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            // T-E-S-32: mcp_servers.json 路径(可选)。
            mcp_servers_path: std::env::var("NEBULA_MCP_SERVERS_PATH")
                .ok()
                .map(std::path::PathBuf::from),
            // T-E-C-02: 视觉模型名(默认 qwen2.5-vl:3b),env: NEBULA_VISION_MODEL。
            vision_model: std::env::var("NEBULA_VISION_MODEL")
                .unwrap_or_else(|_| "qwen2.5-vl:3b".to_string()),
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
    /// v1.2: skill closed-loop learning 鈥?auto-extracts reusable skills from swarm tasks.
    pub skill_extractor: Arc<SkillExtractor>,
    /// v1.2: skill auto-composer for orchestration upgrade.
    pub skill_composer: Arc<SkillComposer>,
    /// v1.3 P2-7: skill marketplace — search, install, update, publish.
    pub marketplace: Arc<skills::SkillMarketplace>,
    /// v1.3: skill audit logger.
    pub skill_audit_logger: Arc<SkillAuditLogger>,
    #[cfg(feature = "channels")]
    /// v1.2: multi-channel message bridge (JiWenSwarm delivery fabric).
    pub message_bridge: Option<Arc<MessageBridge>>,
    /// T-S3-B-01: 原生通信渠道路由器（Telegram/Discord 直连，不经过 JiuWenSwarm）。
    #[cfg(feature = "channels")]
    pub channel_router: Arc<crate::channel::ChannelRouter>,
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
    /// v1.8: 性能监控器（后台 1Hz 采样 RSS/CPU，前端通过 perf_sample 命令轮询）。
    pub perf_monitor: crate::perf::monitor::PerfMonitor,
    /// v1.1 P0-2: tool registry with registered tools (shell, etc.).
    pub tool_registry: Arc<ToolRegistry>,
    #[cfg(feature = "channels")]
    /// v1.3: WebChat share link service.
    pub webchat_service: WebChatService,
    /// v1.3: device manager for sync pairing.
    pub device_manager: Arc<parking_lot::Mutex<DeviceManager>>,
    /// v1.3: MCP manager (feature-gated).
    #[cfg(feature = "mcp")]
    pub mcp_manager: Arc<crate::mcp::client::McpManager>,
    /// v1.4: L0 缓存层（LRU 热记忆 + 会话上下文窗口 + 预取队列）。
    /// 设计文档 v7.0 §2.1 L0 Cache Layer 的实现。
    pub l0: Arc<L0Cache>,
    /// v1.4: Memory Orchestrator（L2 认知层协调器）。
    /// 设计文档 v7.0 §4.3 上下文组装策略：5 类记忆协调 + ≤3 类组合 + ≤3000 token。
    pub orchestrator: Arc<MemoryOrchestrator>,
    /// v1.5: LLM 驱动的多粒度摘要引擎（50/150/500/2000 字符四级）。
    pub summary_engine: Arc<SummaryEngine>,
    /// v1.5: 因果图谱推理引擎（根因追溯 + 效果链 + 解释路径）。
    pub causal_graph: Arc<CausalGraphEngine>,
    /// v1.6: Git 风格记忆版本控制（branch/commit/log/diff/revert/merge）。
    /// 设计文档 v7.0 §3.4 L3 应用层 — 记忆版本控制。
    pub version_control: Arc<MemoryVersionControl>,
    /// v2.0: Sidecar 进程管理器（Memory/LLM/Swarm 独立进程管理）。
    /// 设计文档 v7.0 §14 Sidecar Architecture。
    pub sidecar_manager: crate::sidecar::SidecarManager,
    /// v2.0: 真正的 Self-Reflection 引擎（L5 元认知层升级）。
    /// 设计文档 v7.0 §2.1 L5 Metacognitive Layer。
    pub self_reflection: Arc<crate::memory::self_reflection::SelfReflectionEngine>,
    /// T-E-S-20: exec 类操作审批注册表(供 Tauri 命令查询当前 Pending 请求)。
    pub exec_approval: Arc<crate::skills::exec_approval::ExecApprovalTracker>,
    /// T-E-A-06: Token 费用追踪器(供 Tauri 命令查询按日/月聚合费用)。
    pub cost_tracker: Arc<crate::llm::cost_tracker::CostTracker>,
    /// T-E-S-51: Level 0 内联补全引擎(本地小模型,零成本)。
    pub inline_completion: Arc<crate::editor::InlineCompletionEngine>,
    /// T-E-S-27: Trusted Diagnostics 通道(独立可信诊断事件流)。
    /// 通过 `OnceLock` 全局单例 + AppState 双重暴露:bootstrap 后由
    /// AppState 持有引用,init_tracing 中的 DiagnosticsLayer 仍使用
    /// `diagnostics::bus::global()` 拿到同一实例。
    ///
    /// 注:`global()` 返回 `&'static Arc<DiagnosticsBus>`(OnceLock 单例),
    /// AppState 通过 `Arc::clone(global())` 持有 `Arc<DiagnosticsBus>`。
    pub diagnostics: Arc<crate::diagnostics::bus::DiagnosticsBus>,
    /// T-E-B-09: 文件夹监控引擎(自动吸收文件到记忆系统)。
    pub file_watcher: Arc<crate::memory::file_watcher::FileWatcherEngine>,
    /// T-E-B-09: file watcher 消费者 task 的 JoinHandle(供 shutdown 等待)。
    pub file_watcher_worker: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>>,
    /// T-E-C-14: 剪贴板监听引擎(后台轮询 + 内容检测 + sponge 吸收)。
    /// 用 `tokio::sync::Mutex` 因为命令路径在锁内 `.await` engine.start()。
    pub clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>>,
    /// T-E-S-41: models.json 动态配置(provider/model 注册表)。
    /// 支持 default_provider/default_model 热更新;provider 列表修改
    /// 重启生效(由 cost_tracker 通过 override 间接感知)。
    pub models_config: Arc<parking_lot::RwLock<crate::llm::models_config::ModelsConfig>>,
    /// T-E-S-57: 后台通知服务(系统通知 + 悬浮球状态)。
    pub notification_service: Arc<crate::notify::NotificationService>,
    /// T-E-S-24: 文件快照回滚引擎。
    pub snapshot_engine: Arc<crate::snapshot::SnapshotEngine>,
    /// T-E-S-05: 死锁检测器(WFG + DFS 环检测 + 后台检测循环)。
    pub deadlock_detector: Arc<crate::swarm::DeadlockDetector>,
    /// T-E-S-26: EventBus — 协议化事件总线(自动包装 EventEnvelope 后广播)。
    /// 与 DiagnosticsBus 模式一致:全局单例 + AppState 双重暴露。
    pub event_bus: Arc<crate::swarm::event_bus::EventBus>,
    /// T-E-S-54: 事件触发器引擎(文件/消息/Webhook 三种触发器统一调度)。
    /// 在 bootstrap 中构造 + start(),shutdown 时停止 worker。
    pub trigger_engine: Arc<crate::triggers::TriggerEngine>,
    /// T-E-C-13: 工作场景模板引擎(编译时嵌入 scenarios.json,启动时解析)。
    /// 纯内存只读,所有命令路径直接查询。
    pub scenario_templates: Arc<crate::scenarios::TemplateEngine>,
    /// T-E-C-17: IM 绑定引擎(Feishu/WeCom/DingTalk webhook 广播)。
    /// 持有 ImBindingStore + SSRF 安全 reqwest client,notify_im 与 im_* 命令共用。
    pub im_engine: Arc<crate::im::ImEngine>,
    /// T-E-S-44: 统一存储后端(Local/S3/WebDAV)。供 snapshot 等子系统使用。
    pub storage: crate::storage::DynStorageBackend,
    /// T-E-A-11: Smart Prefetch 引擎(打开文件时预取历史对话预热 SemanticCache)。
    /// 与 LlmGateway 共享同一 Arc<SemanticCache> 实例,预取写入的条目
    /// 后续 chat 调用可直接命中跳过推理。
    pub prefetch: Arc<PrefetchEngine>,
    /// T-E-B-01: LLM Wiki 编译引擎(每次对话后异步编译结构化 Markdown 笔记)。
    /// chat_stream 完成后 spawn-and-forget 异步调用 compile_turn,
    /// 失败仅记日志不阻塞主路径。config.wiki_enabled=false 时仍构造,
    /// 但 compile_turn 内部短路返回。
    pub wiki: Arc<crate::wiki::WikiCompiler>,
    /// T-E-S-32: MCP Server Registry — stdio 子进程生命周期管理。
    /// 持有 Arc<McpManager>,supervisor_loop 每 5s 健康检查 + 崩溃重启限流。
    /// 仅在 `mcp` feature 启用时编译。
    #[cfg(feature = "mcp")]
    pub mcp_registry: Arc<crate::mcp::registry::McpServerRegistry>,
    /// T-E-A-14: Arena A/B 测试 — 模型对战 + ELO 排行榜。
    /// 持有 `Mutex<HashMap<model, elo>>` 内存 + `Option<SqliteStore>` 持久化。
    /// bootstrap 阶段 `with_store(sqlite.clone())` + `load_from_store()` 回填。
    pub arena: Arc<crate::llm::arena::ArenaLeaderboard>,
    /// M5 #68: L4 审批门禁 + nonce 防重放 + 5 分钟超时。
    /// AppState 持有 `Arc<ApprovalGate>`,chat / evolution / master_run 多个
    /// 调用方共享同一 registry(pending confirmations 进程内状态)。
    pub approval_gate: Arc<crate::autonomy::ApprovalGate>,
    /// M5 #68: Pending confirmations 注册表(AppState 持有 Arc,审批命令直接访问)。
    pub confirmation_registry: Arc<crate::autonomy::ConfirmationRegistry>,
    /// M6 #82: MasterOrchestrator — 顶层任务编排器(DAG 拆解 + 按层执行 + 结果综合)。
    /// 通过 `master_run` Tauri 命令暴露给前端,实时推送 MasterEvent 流。
    /// 仅在 `master-orchestrator` feature 启用时构造。
    #[cfg(feature = "master-orchestrator")]
    pub master_orchestrator: Arc<crate::swarm::MasterOrchestrator>,
    /// M6 #78: EvolutionEngine — 4 Phase 自我进化引擎。
    /// 仅在 `evolution-engine` feature 启用时构造,否则为 None。
    #[cfg(feature = "evolution-engine")]
    pub evolution_engine: Option<Arc<crate::evolution::engine::EvolutionEngine>>,
    /// M6 #78: EvolutionLog — 进化日志读写(`evolution_log.md`)。
    #[cfg(feature = "evolution-engine")]
    pub evolution_log: Arc<crate::evolution::engine::EvolutionLog>,
    /// M6 #78: Roller — SOUL.md evolution-append 段落级回滚器。
    #[cfg(feature = "evolution-engine")]
    pub roller: Arc<crate::evolution::engine::Roller>,
    /// M7a #86: UnifiedModelDispatcher — 顶层 LLM 调度入口(chat / chat_stream
    /// / GenericAgent / MasterOrchestrator / EvolutionEngine / SoulCompiler 共享)。
    /// 仅在 `unified-dispatcher` feature 启用时构造;否则为 None,chat 路径
    /// 回退到 `LlmGateway::chat` / `LlmGateway::chat_stream`(P1-19 双路径回滚)。
    #[cfg(feature = "unified-dispatcher")]
    pub dispatcher: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>>,
}

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
        // prefetch 内部用 tokio::join! 并行三路检索(路径 LIKE + BM25 + 向量),
        // 配对 chat turn 后写入 SemanticCache,后续 LLM 调用命中缓存跳过推理。
        let prefetch = Arc::new(PrefetchEngine::with_default_config(
            sqlite.clone(),
            lance.clone(),
            embedder.clone(),
            semantic_cache,
        ));
        startup.mark("bootstrap.prefetch");
        info!(target: "nebula", "prefetch engine ready (T-E-A-11)");

        // Phase 3: swarm + reflection
        // T-E-S-02: 提前构造 tool_registry,供 SwarmOrchestrator 与 AppState 共享同一 Arc。
        // ShellTool 等具体工具在 Phase 5 注册(依赖 shell executor);ToolRegistry 内部
        // 使用 RwLock,后续 register 对已持有 Arc 的 swarm agent 可见。
        let tool_registry = Arc::new(ToolRegistry::new());
        let (swarm, reflection, deadlock_detector) = Self::bootstrap_swarm_and_reflection(
            &config, &sqlite, &lance, &embedder, &llm, &sponge, &tool_registry,
        );

        // T-E-S-39: 预加载 SOUL.md/AGENTS.md/TOOLS.md persona。
        // 从 workspace root 读取;读取失败不阻塞启动(warn + None)。
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
        // 注入 persona 到 swarm(供 GenericAgent::run 使用)。
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
        // L0Cache 是纯内存结构,不依赖外部资源;MemoryOrchestrator 依赖
        // sqlite/lance/embedder/l0,所以在这里组装。
        let l0 = Arc::new(L0Cache::new());
        let mut orchestrator_builder = MemoryOrchestrator::new(
            sqlite.clone(),
            lance.clone(),
            embedder.clone(),
            l0.clone(),
        )
        .with_sponge(sponge.clone());
        // T-E-S-21: 共享 SpongeEngine 的 ACL 实例,确保 assemble_context
        // 过滤未授权记忆。sponge 的 ACL 在 bootstrap_ai_core 中已从
        // SQLite 加载（T-S1-A-04）;此处复用同一 Arc 实例,避免重复加载。
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
        // MonitorHandle 被 _perf_handle 持有，drop 时停止采样。
        let (_perf_handle, perf_monitor) =
            crate::perf::monitor::PerfMonitor::start(std::time::Duration::from_secs(1));
        std::mem::forget(_perf_handle); // 保持运行直到进程退出
        info!(target: "nebula", "perf monitor started");

        let device_manager = Arc::new(parking_lot::Mutex::new(DeviceManager::new(
            sqlite.raw_connection(),
        )));

        // T-E-S-27: 拿到全局 DiagnosticsBus 单例,装入 AppState。
        // init_tracing 中已经(或将会)通过同一个 global() 实例 install
        // DiagnosticsLayer。global() 返回 &'static Arc<DiagnosticsBus>,
        // 用 Arc::clone 拿到 Arc<DiagnosticsBus> 装入 AppState。
        let diagnostics = Arc::clone(crate::diagnostics::bus::global());
        info!(
            target: "nebula",
            enabled = config.diagnostics_channel_enabled,
            capacity = config.diagnostics_buffer_capacity,
            "diagnostics bus ready (T-E-S-27)"
        );

        // T-E-S-26: 拿到全局 EventBus 单例,装入 AppState。
        // 与 DiagnosticsBus 模式一致:global() 返回 &'static Arc<EventBus>,
        // AppState 通过 Arc::clone 持有引用。
        let event_bus = Arc::clone(crate::swarm::event_bus::global());
        info!(target: "nebula", "event bus ready (T-E-S-26)");

        // T-E-B-09: 构造 FileWatcherEngine。若 config.watch_paths 非空,
        // 启动 watchers + 消费者 task。
        let file_watcher = Arc::new(
            crate::memory::file_watcher::FileWatcherEngine::new(sponge.clone()),
        );
        let file_watcher_worker: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>> =
            Arc::new(parking_lot::Mutex::new(None));
        if !config.watch_paths.is_empty() {
            let watch_paths: Vec<std::path::PathBuf> = config
                .watch_paths
                .iter()
                .map(|s| std::path::PathBuf::from(s))
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

        // T-E-C-14: 构造 ClipboardWatcherEngine。此处只构造不启动 —
        // 启动需要 AppHandle(用于 emit 事件),由前端通过
        // `clipboard_watch_start` 命令触发,或在 setup 回调中根据
        // `config.clipboard_watch_enabled` 自动启动。
        let clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>> =
            Arc::new(tokio::sync::Mutex::new(crate::os::ClipboardWatcherEngine::new()));
        info!(
            target: "nebula",
            enabled = config.clipboard_watch_enabled,
            "clipboard watcher engine ready (T-E-C-14)"
        );

        // T-E-S-44: 统一存储后端(Local/S3/WebDAV)。snapshot 等子系统通过
        // 此 Arc<dyn StorageBackend> 间接调用,运行时由 config 决定具体后端。
        let storage = crate::storage::StorageBackendFactory::from_config(&config.storage_backend)
            .context("initializing storage backend")?;
        info!(
            target: "nebula",
            kind = config.storage_backend.kind,
            "storage backend ready (T-E-S-44)"
        );

        // T-E-S-24: 文件快照回滚引擎。持有 Arc<dyn StorageBackend>,
        // 内部用 backend 异步调用(替代旧版 std::fs 同步操作)。
        let snapshot_engine = Arc::new(
            crate::snapshot::SnapshotEngine::new(storage.clone())
                .context("initializing snapshot engine")?,
        );
        info!(target: "nebula", "snapshot engine ready (T-E-S-24)");

        // T-E-S-57: 后台通知服务(系统通知 + 悬浮球状态)。提前构造,
        // 供 T-E-S-54 trigger_engine 复用同一 Arc<NotificationService>。
        let notification_service = Arc::new(crate::notify::NotificationService::new(app_handle));

        // T-E-S-54: 事件触发器引擎 — 构造 + start()。
        // 依赖 sqlite(bus 用 swarm.bus() 的 Arc<AgentBus>)、skills、swarm、notify。
        // start() 内部从 DB 加载已持久化的触发器,启动消息订阅 + webhook
        // server(8088 端口)+ 文件触发器 worker。webhook 启动失败降级,
        // 不阻断 bootstrap。
        let trigger_engine = Arc::new(crate::triggers::TriggerEngine::new(
            sqlite.clone(),
            swarm.bus().clone(),
            skills.clone(),
            swarm.clone(),
            notification_service.clone(),
        ));
        trigger_engine.clone().start();
        info!(target: "nebula", "trigger engine started (T-E-S-54)");

        // T-E-C-13: 工作场景模板引擎 — 编译时嵌入 scenarios.json,
        // 启动时 serde_json::from_str 解析一次。解析失败(数据损坏)
        // 阻断启动,因为前端依赖模板库可用。
        let scenario_templates = Arc::new(
            crate::scenarios::TemplateEngine::load()
                .context("loading scenario templates from scenarios.json")?,
        );
        info!(
            target: "nebula",
            count = scenario_templates.list().len(),
            "scenario template engine loaded (T-E-C-13)"
        );

        // T-E-C-17: IM 绑定引擎 — 持有 ImBindingStore(依赖 sqlite)。
        // webhook 发送时通过 SsrfGuard::build_safe_client() 构建每次的
        // reqwest::client(避免长期持有可能被 SSRF 重定向绕过的 client)。
        let im_engine = Arc::new(crate::im::ImEngine::new(sqlite.clone()));
        info!(target: "nebula", "IM engine ready (T-E-C-17)");

        // T-E-B-01: LLM Wiki 编译引擎 — 持有 LlmGateway + storage + sqlite。
        // chat_stream 完成后 spawn-and-forget 调用 compile_turn(幂等:同 turn_id 短路)。
        // 失败仅记日志,不阻塞主路径;config.wiki_enabled=false 时 compile_turn 内部短路。
        let wiki_config = crate::wiki::WikiConfig {
            enabled: config.wiki_enabled,
            subdir: config.wiki_subdir.clone(),
        };
        // T-E-B-03: 注入 sponge + version_control,启用 update_note_from_user
        // 双向同步(SQLite UPDATE → sponge.absorb_text → 文件重写 → vc.commit → LogEvent::Updated)。
        // sponge 通过 MemoryRevectorizer trait 注入,便于测试 mock;
        // version_control 已在 bootstrap 早期(line ~774)构造完毕。
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
        // 之前 mcp_manager 在 Ok(Self {...}) 内联构造,无法被 registry 引用。
        #[cfg(feature = "mcp")]
        let mcp_manager = Arc::new(crate::mcp::client::McpManager::new());

        // T-E-S-32: MCP Server Registry — 持有 Arc<McpManager>,
        // supervisor_loop 每 5s 健康检查 + 崩溃重启限流(3 次/小时)。
        // 仅在 `mcp` feature 启用时构造;log_dir 复用 default_log_dir()。
        #[cfg(feature = "mcp")]
        let mcp_registry = {
            let log_dir = default_log_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            Arc::new(crate::mcp::registry::McpServerRegistry::new(
                mcp_manager.clone(),
                log_dir,
            ))
        };
        #[cfg(feature = "mcp")]
        {
            info!(target: "nebula", "MCP server registry ready (T-E-S-32)");
        }

        // T-E-A-14: Arena A/B 测试 — 构造 ArenaLeaderboard + with_store +
        // load_from_store。失败仅 warn,不阻塞启动(leaderboard 仍可用内存模式)。
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
        // 提升到 bootstrap 顶层,供 chat / chat_stream / master_orchestrator /
        // evolution_engine 共用同一 Arc(避免重复构造 + 保证一致 ModelPolicy)。
        // feature off 时为 None,所有调用方走 LlmGateway 旧路径(P1-19 回滚策略)。
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
        // 共享 Arc<ConfirmationRegistry>,chat / evolution / master_run 多调用方共用。
        // MasterOrchestrator 仅在 master-orchestrator feature 启用时构造,
        // 复用同一 Arc<SwarmOrchestrator> + dispatcher(可选,unified-dispatcher feature
        // 关时为 None,内部回退到 swarm.llm 兜底)。
        // M7a #86: 复用顶层 dispatcher(不再独立构造)。
        let confirmation_registry = Arc::new(crate::autonomy::ConfirmationRegistry::new());
        let approval_gate = Arc::new(crate::autonomy::ApprovalGate::new(
            crate::autonomy::WorkerRiskMap::new(),
            confirmation_registry.clone(),
        ));
        info!(target: "nebula", "approval gate + confirmation registry ready (M5 #68)");
        #[cfg(feature = "master-orchestrator")]
        let master_orchestrator = {
            // M7a #86: 复用顶层 dispatcher(feature off 时为 None)。
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
        // 仅在 evolution-engine feature 启用时构造;EvolutionEngine 需要 dispatcher,
        // dispatcher 未注入时为 None,回退到 swarm.llm 兜底(与 MasterOrchestrator 同模式)。
        // M7a #86: 复用顶层 dispatcher(不再独立构造)。
        #[cfg(feature = "evolution-engine")]
        let (evolution_engine, evolution_log, roller) = {
            use crate::evolution::engine::{EvolutionEngine, EvolutionLog, Roller, EvolutionEngineConfig};
            use std::path::PathBuf;
            let log_path = PathBuf::from("evolution_log.md");
            let soul_md_path = PathBuf::from("SOUL.md");
            let log = Arc::new(EvolutionLog::new(log_path));
            let roller = Arc::new(Roller::new(log.clone(), soul_md_path));
            let engine = {
                // M7a #86: 复用顶层 dispatcher(feature off 时为 None)。
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
            // T-S3-B-01: 初始化原生渠道路由器，从环境变量读取 Telegram/Discord 配置。
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
            // M7a #86: 顶层 dispatcher(unified-dispatcher feature on 时非 None)。
            #[cfg(feature = "unified-dispatcher")]
            dispatcher,
        })
    }

    // -- bootstrap phase helpers --

    async fn bootstrap_storage(
        config: &AppConfig,
        startup: &StartupTimer,
    ) -> anyhow::Result<(Arc<SqliteStore>, Arc<dyn VectorStore>)> {
        let db_path = config.db_path.clone();
        // T-E-A-13: db_encryption_enabled=true 时走 SQLCipher 加密路径
        // (SqliteStore::open_encrypted),否则走明文路径 (SqliteStore::open)。
        // key 从 keychain 读取(slot: KEY_DB_ENCRYPTION_KEY);失败则 fallback
        // 到 env var NEBULA_DB_ENCRYPTION_KEY(resolve_db_encryption_key 内部
        // 处理)。key 完全缺失时返回 Err,进程退出并提示用户(参考 spec R3)。
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

        // T-E-D-01: migrations 已在 `SqliteStore::open` 内部应用(sqlite_store.rs:94)。
        // 此处不再重复调用 `run_bundled_migrations`(冗余的 spawn_blocking +
        // Mutex 锁 + PRAGMA user_version 查询),仅打点。
        info!(target: "nebula", "migrations applied during SqliteStore::open");
        startup.mark("bootstrap.migrations");

        // T-E-S-42: 通过工厂函数创建向量存储,按 AppConfig.vector_store_backend
        // 选择 LanceDB / Qdrant / ChromaDB 后端。Lance 后端使用 lance_path,
        // Qdrant/Chroma 后端使用对应的 remote_url。
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

    async fn bootstrap_ai_core(
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
        // T-E-A-11: 返回 SemanticCache 供 PrefetchEngine 共享同一实例。
        Arc<SemanticCache>,
    )> {
        // T-E-S-41: 加载 models.json(不存在回退 default_builtin),
        // 推送 override 到 cost_tracker 让 model_price() 立即看到新 pricing。
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
        // T-E-S-51: 在 ollama Arc 被 LlmGateway 消费前 clone 一份,构造
        // 内联补全引擎。模型名可通过 NEBULA_INLINE_MODEL 覆盖,默认
        // qwen2.5-coder:0.5b(spec §设计约束 第 2 条推荐)。
        let inline_model = std::env::var("NEBULA_INLINE_MODEL")
            .unwrap_or_else(|_| "qwen2.5-coder:0.5b".to_string());
        let inline_completion = Arc::new(
            crate::editor::InlineCompletionEngine::new(ollama.clone(), inline_model),
        );
        // T-E-A-02/03: 在 ollama Arc 被 LlmGateway::new 消费前 clone,
        // 供 TokenJuice(L3 摘要)和 ModelRouter(分类器)直连本地 Ollama。
        let ollama_for_compress = ollama.clone();
        // T-E-S-23: keychain 优先 → env var 兜底(凭证与 settings.json 解耦)。
        let ak = crate::security::keychain::resolve_anthropic_key();
        let am = std::env::var("NEBULA_ANTHROPIC_MODEL").ok();
        // T-E-S-20: ExecApprovalTracker 在此处构造,供 SkillEngine
        // (在 bootstrap_skills 中)与 AppState 共享。
        let exec_approval = Arc::new(
            crate::skills::exec_approval::ExecApprovalTracker::new(
                config.exec_approval_timeout_secs,
            ),
        );
        // T-E-A-06: Token 费用追踪器(默认启用)。
        // T-E-A-12: 若配置了 automation_daily_budget_usd,注入预算告警回调,
        // 当日 Automation 累计费用超阈值时 emit `budget_exceeded` 事件。
        // T-E-A-13: 注入 SqliteStore(主 DB)启用费用持久化 — `attach_store`
        // 内部调用 `load_from_store_blocking()` 把 cost_records 表回填到内存,
        // 后续 record_async 异步写入。注入位置参考 SemanticCache with_sqlite
        // (lib.rs line ~1170),在 Arc::new 之前以 builder 风格调用。
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
            // T-E-A-13: 注入主 DB SqliteStore(SqliteStore: Clone,内部 Arc
            // 廉价克隆)。attach_store 内部 load_from_store_blocking 回填内存。
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
        // T-E-A-01: L0.5 语义缓存(默认启用)。
        // T-E-A-11: 始终创建 Arc<SemanticCache>,供 LlmGateway(若启用)
        // 与 PrefetchEngine 共享同一实例。禁用时 LlmGateway 不接入,
        // 但 PrefetchEngine 仍可独立写入(后续启用时可命中)。
        let semantic_cache: Arc<SemanticCache> = Arc::new(
            crate::llm::semantic_cache::SemanticCache::default_config(
                lance.clone(),
                embedder.clone(),
            )
            // T-E-D-01: 注入 sqlite 引用,支持 SemanticCache entries 持久化
            // 到 semantic_cache_entries 表(migration 031),重启后可 prewarm 恢复。
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
        // T-E-A-02: TokenJuice 三级压缩(默认启用)。复用 ollama clone
        // 做 L3 摘要(直连本地 Ollama,零成本,不经 LlmGateway/CostTracker)。
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
        // T-E-A-03: ModelRouter 智能路由(默认启用)。分类器直连 Ollama
        // (零成本,参考 InlineCompletionEngine 模式)。
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
        // T-E-A-05: 日预算限制(超限自动降级到 Ollama)。
        if config.daily_budget_usd > 0.0 {
            llm_builder = llm_builder.with_daily_budget(config.daily_budget_usd);
            info!(
                target: "nebula",
                budget = config.daily_budget_usd,
                "daily budget wired into LlmGateway"
            );
        }
        // T-E-S-40: OpenAI 兼容 provider(vLLM/LMStudio/OpenRouter/自建)。
        // 仅在 base_url 配置时注入;key 可空(LMStudio 本地),model 缺省回退 "local-model"。
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
        // T-S1-A-04: 从 SQLite 加载 ACL 规则并注入 SpongeEngine。
        // 加载失败不阻断启动（记录 warn，sponge.acl 保持 None，
        // `search_with_acl()` 对所有主体放行 —— 等同于 v2.0 行为）。
        let sponge = {
            let mut sponge_builder = SpongeEngine::new(
                sqlite.clone(),
                lance.clone(),
                embedder.clone(),
            );
            // T-E-A-09: 注入 CostTracker,absorb() 时采样 total_cost_usd() 差值
            // 记录到 mem.ingest_cost(向量化零成本,仅 LLM 抽取产生费用)。
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
        // T-E-S-41: 推送 override 到 cost_tracker,让 model_price() 立即
        // 看到 models.json 里的 pricing(无需等到首次 LLM 调用触发
        // OnceLock 懒加载——懒加载路径读盘可能与 save 命令的 override
        // 竞争;这里预先 init 缓存 + 推送 override 保证一致性)。
        crate::llm::cost_tracker::update_models_config_override(models_config_value.clone());
        let models_config = Arc::new(parking_lot::RwLock::new(models_config_value));
        // T-E-S-23: 尝试将 env var 中的 key 迁移到 keychain(幂等,非阻塞)。
        // keychain 不可用时静默跳过;迁移 >0 时记录 info 日志。
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
    ///
    /// `list_acl()` 返回 `Vec<(id, principal, resource, permission, effect)>`，
    /// 其中 `permission`/`effect` 是字符串（"read"/"write"/"delete" 与
    /// "allow"/"deny"）。本方法负责字符串 → 枚举的解析，解析失败的行
    /// 被跳过并 `warn!` 记录（不阻断启动）。
    fn load_acl_from_store(sqlite: &Arc<SqliteStore>) -> anyhow::Result<MemoryAcl> {
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

    fn bootstrap_swarm_and_reflection(
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

    fn bootstrap_skills(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        llm: &Arc<LlmGateway>,
        exec_approval: &Arc<crate::skills::exec_approval::ExecApprovalTracker>,
    ) -> (
        Arc<SkillEngine>,
        Arc<SkillExtractor>,
        Arc<SkillComposer>,
        Arc<skills::SkillMarketplace>,
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
        let mp = Arc::new(skills::SkillMarketplace::new(ss, imp));
        let _ = mp.refresh();
        // v2.0: seed built-in demo skills on first run (idempotent).
        crate::skills::seed_demo_skills(&skills).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e, "failed to seed demo skills");
            Vec::new()
        });
        // T-E-S-38: 注册三个可视化 creator 能力(viz:canvas / viz:mermaid / viz:mindmap)。
        // 能力层反向映射:Capability → Skills,供 match_by_intent / match_by_input 查询。
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
    ///
    /// 从环境变量读取配置:
    /// * `TELEGRAM_BOT_TOKEN` — Telegram Bot API token
    /// * `DISCORD_WEBHOOK_URL` — Discord webhook URL
    ///
    /// 未配置的渠道不会被注册,路由器以空状态启动。
    #[cfg(feature = "channels")]
    fn bootstrap_channel_router() -> Arc<crate::channel::ChannelRouter> {
        use crate::channel::{ChannelRouter, DiscordBotAdapter, TelegramBotAdapter};

        let router = Arc::new(ChannelRouter::new());

        // Telegram
        let tg_token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        if !tg_token.is_empty() {
            let adapter = TelegramBotAdapter::new(&tg_token);
            router.register(Arc::new(adapter) as Arc<dyn crate::channel::ChannelAdapter>);
            info!(target: "nebula.channel", "Telegram adapter registered");
        }

        // Discord
        let dc_webhook = std::env::var("DISCORD_WEBHOOK_URL").unwrap_or_default();
        if !dc_webhook.is_empty() {
            let adapter = DiscordBotAdapter::new(&dc_webhook);
            router.register(Arc::new(adapter) as Arc<dyn crate::channel::ChannelAdapter>);
            info!(target: "nebula.channel", "Discord adapter registered");
        }

        router
    }

    fn bootstrap_editor(config: &AppConfig) -> EditorState {
        EditorState::new(&config.editor_workspace).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                workspace = %config.editor_workspace,
                "editor workspace unavailable; falling back to current dir");
            EditorState::new(".").expect("current dir is always a directory")
        })
    }

    fn bootstrap_clipboard() -> ClipboardService {
        ClipboardService::new().unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                "clipboard unavailable; using noop fallback");
            ClipboardService::noop()
        })
    }

    fn bootstrap_sync(config: &AppConfig) -> Arc<LocalTransport> {
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

        // Take the worker handle out of the mutex and await it.
        let worker = { self.reflect_worker.lock().take() };
        if let Some(h) = worker {
            // Give the worker up to 250 ms to exit cleanly. We use a
            // timeout so a misbehaving worker can't deadlock the
            // shutdown path.
            match tokio::time::timeout(Duration::from_millis(250), h).await {
                Ok(_) => info!(target: "nebula", "reflection worker stopped"),
                Err(_) => warn!(target: "nebula", "reflection worker did not stop in time"),
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

        // T-E-B-09: 停止 file watcher(取消消费者 task + 清空 watchers)。
        // stop() 内部会 take worker_handle 并 timeout-await,因此这里
        // 无需重复等待。file_watcher_worker Mutex 中的 handle 也清空。
        {
            self.file_watcher_worker.lock().take();
        }
        self.file_watcher.stop().await;

        // T-E-C-14: 停止 clipboard watcher(取消 token + abort task)。
        {
            let mut watcher = self.clipboard_watcher.lock().await;
            watcher.stop();
        }

        // T-E-S-54: 停止 trigger engine(消息订阅 + webhook + 文件触发器)。
        // cancel token + abort 各 worker task + 清空 file_workers map。
        self.trigger_engine.stop();
    }

    /// M1 任务 #23: 尝试 Soul 编译（cfg-gated）。
    ///
    /// 当 `soul-system` feature 编译且 `SOUL_SYSTEM_ENABLED` 运行时开启时，
    /// 从 AppConfig.soul_compiler 获取 SoulCompiler，从 PersonaConfig.soul_md
    /// 读取 SOUL.md 文本，调用 compile() 编译。
    ///
    /// 返回 `Some(system_prompt)` 表示编译成功（用 CompiledSoul.system_prompt
    /// 替代 persona_prefix）；返回 `None` 表示未启用或编译失败（回退 PersonaConfig）。
    ///
    /// 注意：此方法不会 panic，所有错误转为 warn 日志并返回 None。
    #[cfg(feature = "soul-system")]
    async fn try_compile_soul(&self) -> Option<String> {
        use crate::soul::soul_system_enabled;

        // 1. 运行时开关检查
        if !soul_system_enabled() {
            return None;
        }

        // 2. SoulCompiler 实例检查
        let compiler = self.config.soul_compiler.as_ref()?;

        // 3. 从 PersonaConfig 读取 SOUL.md 文本（复用现有加载路径）
        //    PersonaConfig.soul_md 字段存储 SOUL.md 原文。
        let soul_md_text = self.config.persona.as_ref().and_then(|pc| {
            let guard = pc.read();
            guard.soul_md.clone()
        })?;

        if soul_md_text.trim().is_empty() {
            return None;
        }

        // 4. 编译
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

/// Installs the `tracing` subscriber. Safe to call multiple times.
///
/// v0.2: writes structured JSON to stdout when the
/// `NEBULA_LOG_FORMAT=json` environment variable is set; the
/// default remains human-readable pretty output.
///
/// v1.0: when `NEBULA_LOG_DIR` is set we also write a
/// daily-rotated JSON log file via `tracing_appender`.  This is
/// what the user-facing "Open logs folder" menu points at.
///
/// v1.1.9: 默认日志目录。即使未设置 `NEBULA_LOG_DIR`,也写入
/// 平台默认的 app data 目录,以便用户在遇到启动崩溃时能找到日志。
pub fn init_tracing() {
    static INIT: once_cell::sync::OnceCell<()> = once_cell::sync::OnceCell::new();
    INIT.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,nebula=debug"));
        let use_json = std::env::var("NEBULA_LOG_FORMAT")
            .map(|v| v.eq_ignore_ascii_case("json"))
            .unwrap_or(false);

        // v1.8: 尝试构建 OpenTelemetry OTLP 层。
        // 由 NEBULA_OTLP_ENDPOINT 环境变量控制；未设置则返回 None。
        // T-E-S-29: 整个 OTel 路径门控 `otel` feature — feature off 时
        // 不编译 opentelemetry 依赖,otel_layer 始终为 None。
        // 用 `tracing_subscriber::layer::Identity`(实现 Layer<Registry>)
        // 作为 feature off 时的占位类型,保证 match 两个分支类型一致。
        #[cfg(feature = "otel")]
        let otel_layer = {
            let otel_endpoint = crate::observability::otel::otlp_endpoint_from_env();
            let otel_service = crate::observability::otel::otlp_service_name_from_env();
            otel_endpoint
                .as_ref()
                .and_then(|ep| crate::observability::otel::try_build_layer(ep, &otel_service))
        };
        #[cfg(not(feature = "otel"))]
        let otel_layer: Option<tracing_subscriber::layer::Identity> = None;

        // T-E-S-27: Trusted Diagnostics Layer。
        // 用全局单例 bus 避免与 AppState::bootstrap 的循环依赖:
        // init_tracing 在 bootstrap 之前调用,layer 通过
        // `diagnostics::bus::global()` 拿到 bus,bootstrap 中
        // `AppState.diagnostics = bus::global()` 拿到同一实例。
        // 当 NEBULA_DIAGNOSTICS=0 时不安装 layer(转发无效)。
        let diagnostics_enabled = std::env::var("NEBULA_DIAGNOSTICS")
            .ok()
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let diagnostics_layer: Option<crate::diagnostics::DiagnosticsLayer> = if diagnostics_enabled {
            Some(crate::diagnostics::DiagnosticsLayer::new())
        } else {
            None
        };

        // 日志目录:优先用 NEBULA_LOG_DIR,否则用平台默认目录。
        let log_dir = std::env::var("NEBULA_LOG_DIR").ok().map(PathBuf::from);
        let log_dir = log_dir.or_else(default_log_dir);

        let nb_writer: Option<tracing_appender::non_blocking::NonBlocking> = if let Some(dir) = &log_dir {
            let _ = std::fs::create_dir_all(dir);
            // 安装 panic hook:将 panic 信息写入日志文件,避免
            // `windows_subsystem = "windows"` 下 panic 被静默吞掉。
            let panic_dir = dir.clone();
            std::panic::set_hook(Box::new(move |info| {
                let panic_file = panic_dir.join("nebula-panic.log");
                let msg = format!(
                    "[{}] PANIC: {}\n",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                    info
                );
                let _ = std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&panic_file)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
                eprintln!("{msg}");
            }));
            let appender = tracing_appender::rolling::daily(dir, "nebula.log");
            let (nb, _guard) = tracing_appender::non_blocking(appender);
            let _ = Box::leak(Box::new(_guard));
            Some(nb)
        } else {
            None
        };

        // 用 match 方式分别构建 subscriber 再 try_init。
        // OTel 层必须先加到 bare Registry 上
        // (它实现 Layer<Registry> 而非 Layer<Layered<...>>)。
        // T-E-S-27: DiagnosticsLayer 用 Option<L> 形式加入 — 当
        // NEBULA_DIAGNOSTICS=0 时为 None,tracing_subscriber 对
        // Option<L> 有通用的 Layer 实现空操作。
        match (otel_layer, nb_writer, use_json) {
            (Some(otel), Some(nb), true) => {
                let _ = registry()
                    .with(otel)
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer().with_writer(nb).json())
                    .try_init();
            }
            (Some(otel), Some(nb), false) => {
                let _ = registry()
                    .with(otel)
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer().with_writer(nb))
                    .try_init();
            }
            (Some(otel), None, true) => {
                let _ = registry()
                    .with(otel)
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer().json())
                    .try_init();
            }
            (Some(otel), None, false) => {
                let _ = registry()
                    .with(otel)
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer())
                    .try_init();
            }
            (None, Some(nb), true) => {
                let _ = registry()
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer().with_writer(nb).json())
                    .try_init();
            }
            (None, Some(nb), false) => {
                let _ = registry()
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer().with_writer(nb))
                    .try_init();
            }
            (None, None, true) => {
                let _ = registry()
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer().json())
                    .try_init();
            }
            (None, None, false) => {
                let _ = registry()
                    .with(diagnostics_layer)
                    .with(filter)
                    .with(fmt::layer())
                    .try_init();
            }
        }
    });
}

/// 返回平台默认的日志目录。
fn default_log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|d| PathBuf::from(d).join("nebula").join("logs"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| PathBuf::from(d).join("Library/Logs/nebula"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| PathBuf::from(d).join(".local/share/nebula/logs"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    None
}

/// Tauri application entry — builds the runtime, wires commands, runs the app.
pub fn run() {
    init_tracing();
    info!(target: "nebula", version = env!("CARGO_PKG_VERSION"), "starting nebula");

    let config = AppConfig::from_env();
    // v1.0.1 fix: `?expr` is the `Debug`-format shorthand that
    // `tracing` recognises only when the *argument* is a literal
    // expression.  `?config.db_path` is parsed as a single
    // identifier that starts with `?`, which the macro cannot
    // disambiguate.  Use the explicit form `db_path = ?db_path`.
    info!(target: "nebula", db_path = ?config.db_path, "loaded configuration");

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
        // v1.7: "关闭窗口 = 最小化到托盘"。
        // 用户点关闭按钮时隐藏窗口而非退出，退出只能通过托盘菜单或 Cmd+Q。
        // 若托盘未初始化成功，则保持原有"关闭=退出"行为。
        .on_window_event(move |window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let app = window.app_handle();
                    // T-S5-B-01: 浮动窗 (floating-chat) 直接关闭,不最小化到托盘。
                    // T-E-D-03: 悬浮球 (floating-ball) 同样直接关闭。
                    // T-E-D-07: 浮动进度窗 (floating-progress) 同样直接关闭。
                    // 只有主窗口 + 托盘存在时才阻止关闭并隐藏。
                    if window.label() != "floating-chat"
                        && window.label() != "floating-ball"
                        && window.label() != "floating-progress"
                        && app.tray_by_id("nebula-tray").is_some()
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                // v1.7: OS 文件拖入窗口。T-E-D-06: 按 window label 分流 —
                // floating-ball 窗口的拖拽语义是"吸收到记忆"(emit_ball_drag_drop),
                // 其余窗口的拖拽语义是"打开为代码文件"(emit_drag_drop)。
                tauri::WindowEvent::DragDrop(drag_drop) => {
                    if let tauri::DragDropEvent::Drop { paths, .. } = drag_drop {
                        let app = window.app_handle();
                        if window.label() == crate::commands::window::FLOATING_BALL_LABEL {
                            crate::os::file_handler::emit_ball_drag_drop(app, paths);
                        } else {
                            crate::os::file_handler::emit_drag_drop(app, paths);
                        }
                    }
                }
                _ => {}
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            // 将相对路径的 db_path / lance_path 解析到 app data dir,
            // 避免从快捷方式启动时工作目录为 System32 导致 DB 文件
            // 创建失败或落到错误位置。
            let mut config = config.clone();
            if let Ok(data_dir) = app.path().app_data_dir() {
                std::fs::create_dir_all(&data_dir).ok();
                if !std::path::Path::new(&config.db_path).is_absolute() {
                    config.db_path = data_dir.join(&config.db_path)
                        .to_string_lossy().to_string();
                }
                if !std::path::Path::new(&config.lance_path).is_absolute() {
                    config.lance_path = data_dir.join(&config.lance_path)
                        .to_string_lossy().to_string();
                }
                info!(target: "nebula", data_dir = ?data_dir, db_path = ?config.db_path, "resolved data paths");
            }

            // v1.7: 系统托盘 + 全局快捷键接线。
            // 失败时仅记录日志，不阻断启动（托盘/快捷键是锦上添花）。
            crate::os::tray::setup(app.handle());
            crate::os::shortcut::setup(app.handle());

            // T-S6-A-02: 电源管理 — 启动后台睡眠/唤醒监测线程。
            let power_mgr = std::sync::Arc::new(crate::os::PowerManager::new(app.handle().clone()));
            power_mgr.clone().start();
            app.manage(power_mgr);

            // v1.7: 处理通过 argv 传入的文件路径（双击文件打开）。
            crate::os::file_handler::handle_argv_files(app.handle());
            // T-E-D-06: 检测 `--ask <path>` argv(右键"问Nebula"菜单触发)。
            crate::os::file_handler::handle_ask_argv(app.handle());
            // Bootstrap state asynchronously so we don't block Tauri's main thread.
            //
            // v0.3 fix: when bootstrap fails, surface a user-facing
            // dialog and exit the application. The v0.2 behaviour of
            // "warn and continue" left the user with an uninitialised
            // app and no clue what to do.
            tauri::async_runtime::spawn(async move {
                match AppState::bootstrap(config.clone(), handle.clone()).await {
                    Ok(state) => {
                        // Start the reflection background worker; the
                        // JoinHandle is parked on the AppState so
                        // shutdown can await it (see
                        // `AppState::shutdown`).
                        if let Some(h) = state.reflection.clone().spawn_worker() {
                            *state.reflect_worker.lock() = Some(h);
                            info!(target: "nebula", "reflection worker started");
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
                                        target: "nebula",
                                        addr = %state.config.grpc_bind_addr,
                                        "gRPC server started"
                                    );
                                    *state.grpc_server.lock() = Some(handle);
                                }
                                Err(e) => {
                                    error!(
                                        target: "nebula",
                                        error = ?e,
                                        "gRPC server failed to start; continuing without it"
                                    );
                                }
                            }
                        } else {
                            info!(target: "nebula", "gRPC server disabled by config");
                        }

                        // v1.8: 可选 Prometheus /metrics 端点（env
                        // `NEBULA_METRICS_ADDR` 控制，默认关闭）。
                        // JoinHandle 用 mem::forget 保持运行直到进程退出。
                        if let Some(addr) = crate::metrics::exporter::bind_addr_from_env() {
                            let h = crate::metrics::exporter::start(
                                addr.clone(),
                                state.perf_monitor.clone(),
                            );
                            std::mem::forget(h);
                            info!(
                                target: "nebula.metrics",
                                addr = %addr,
                                "prometheus exporter started"
                            );
                        }

                        // v1.3: MCP — connect all configured servers and
                        // register their tools into the ToolRegistry.
                        #[cfg(feature = "mcp")]
                        {
                            state.mcp_manager.connect_all().await;
                            let mcp_tools = state.mcp_manager.list_all_tools().await;
                            if !mcp_tools.is_empty() {
                                info!(target: "nebula", count = mcp_tools.len(), "MCP tools discovered");
                            }
                            // T-E-S-30: 把 MCP 工具注册到 ToolRegistry,
                            // 让 Agent 可通过 tool_invoke 调用。
                            let tool_groups = state.mcp_manager.as_tool_implementations().await;
                            for (server_name, tools) in tool_groups {
                                let count = tools.len();
                                state.tool_registry.register_mcp_tools(&server_name, tools);
                                info!(
                                    target: "nebula",
                                    server = %server_name,
                                    count,
                                    "MCP tools registered into ToolRegistry"
                                );
                            }
                        }

                        // T-S6-A-03: 启动自动备份调度器(每日 02:00)。
                        // 在 manage(state) 之前读取 config,避免所有权移动。
                        {
                            let app_handle = handle.clone();
                            let db_path = std::path::PathBuf::from(&state.config.db_path);
                            let lance_dir = std::path::PathBuf::from(&state.config.lance_path);
                            let scheduler = crate::backup::BackupScheduler::new(app_handle, db_path, lance_dir);
                            if let Err(e) = scheduler.start() {
                                info!(target: "nebula.backup", error = ?e, "failed to start backup scheduler");
                            }
                        }

                        // T-E-C-14: 若 config.clipboard_watch_enabled=true,
                        // 在 manage 之前自动启动剪贴板监听(需要 AppHandle)。
                        let clipboard_auto_started = state.config.clipboard_watch_enabled;
                        if clipboard_auto_started {
                            let app_handle = handle.clone();
                            let sponge = state.sponge.clone();
                            let mut watcher = state.clipboard_watcher.lock().await;
                            match watcher.start(sponge, app_handle) {
                                Ok(_) => info!(target: "nebula", "clipboard watcher auto-started (T-E-C-14)"),
                                Err(e) => warn!(target: "nebula", error = %e, "clipboard watcher auto-start failed"),
                            }
                        }

                        let notification_svc = state.notification_service.clone();
                        let swarm_bus = state.swarm.bus().clone();

                        handle.manage(state);
                        info!(target: "nebula", "app state ready");

                        // T-E-S-61: 异步启动 sidecar 进程,不阻塞 UI。
                        // bootstrap() 内部 start_all + wait_ready(10s × kinds),
                        // 失败仅 warn 不阻断应用。supervisor_loop 已实现指数退避重启。
                        {
                            let sidecar_manager = handle.state::<crate::AppState>().sidecar_manager.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = sidecar_manager.bootstrap().await {
                                    warn!(target: "nebula.sidecar", error = %e,
                                        "sidecar bootstrap failed (non-blocking, T-E-S-61)");
                                } else {
                                    info!(target: "nebula.sidecar",
                                        "sidecar bootstrap completed (T-E-S-61)");
                                }
                            });
                        }

                        // T-E-D-01: L0Cache 后台预热,降低首响延迟。
                        // 预填最近 64 条 memory 到 L0 LRU。SemanticCache 已通过
                        // with_sqlite() 注入 sqlite 引用,check() 在 entries map
                        // miss 时自动 fallback 查 SQLite semantic_cache_entries 表。
                        {
                            let state = handle.state::<crate::AppState>();
                            let l0_cache = state.l0.clone();
                            let sqlite_for_l0 = state.sqlite.clone();
                            tauri::async_runtime::spawn(async move {
                                l0_cache.prewarm_from_store(&sqlite_for_l0, 64).await;
                                info!(target: "nebula.l0_cache",
                                    "L0Cache prewarmed with recent memories (T-E-D-01)");
                            });
                        }

                        // T-E-D-02: Ollama 模型预热(后台 spawn,非阻塞)。
                        // 仅当 provider == "ollama" 且 chat_model 非空时触发;
                        // warmup_model 内部先 GET /api/ps 检查是否已加载,
                        // 未加载才 POST /api/generate(num_predict=1)触发加载到显存。
                        // 失败仅 warn! 不影响应用启动。
                        {
                            let state = handle.state::<crate::AppState>();
                            let ollama = state.llm.ollama_client().clone();
                            let chat_model = state.config.chat_model.clone();
                            let cfg_provider = state.config.llm_provider.clone();
                            tauri::async_runtime::spawn(async move {
                                if cfg_provider == "ollama" && !chat_model.is_empty() {
                                    if let Err(e) = ollama.warmup_model(&chat_model).await {
                                        warn!(
                                            target: "nebula.ollama",
                                            error = %e,
                                            "warmup_model failed (non-blocking, T-E-D-02)"
                                        );
                                    } else {
                                        info!(
                                            target: "nebula.ollama",
                                            model = %chat_model,
                                            "warmup_model completed (T-E-D-02)"
                                        );
                                    }
                                }
                            });
                        }

                        // T-E-C-02: 视觉模型(vision_model)预热,降低首张截图
                        // describe_image 的首响延迟。后台 spawn,非阻塞;
                        // 模型未拉取或 Ollama 离线时仅 warn! 不影响应用启动。
                        // 与 chat_model warmup 独立 — 即使 chat_model 走 DeepSeek,
                        // vision_model 仍走 Ollama 多模态 API。
                        {
                            let state = handle.state::<crate::AppState>();
                            let ollama = state.llm.ollama_client().clone();
                            let vision_model = state.config.vision_model.clone();
                            tauri::async_runtime::spawn(async move {
                                if !vision_model.is_empty() {
                                    if let Err(e) = ollama.warmup_model(&vision_model).await {
                                        warn!(
                                            target: "nebula.ollama",
                                            error = %e,
                                            "vision_model warmup failed (non-blocking, T-E-C-02)"
                                        );
                                    } else {
                                        info!(
                                            target: "nebula.ollama",
                                            model = %vision_model,
                                            "vision_model warmup completed (T-E-C-02)"
                                        );
                                    }
                                }
                            });
                        }

                        // T-E-S-57: 启动后台任务订阅 SwarmEvent，
                        // 触发系统通知 + 悬浮球状态更新。
                        tauri::async_runtime::spawn(async move {
                            let mut rx = swarm_bus.subscribe_events();
                            info!(target: "nebula.notify", "notification event subscriber started");
                            loop {
                                match rx.recv().await {
                                    Ok(event) => {
                                        notification_svc.handle_swarm_event(&event);
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                        warn!(
                                            target: "nebula.notify",
                                            lagged = n,
                                            "notification subscriber lagged, skipping stale events"
                                        );
                                        continue;
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                        info!(target: "nebula.notify", "notification subscriber channel closed, exiting");
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!(target: "nebula", error = ?e, "failed to bootstrap app state");
                        // v0.3: tell the user, then exit. The dialog
                        // plugin is registered above so the call
                        // resolves. The process exit ensures the user
                        // doesn't end up with a half-broken UI.
                        use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                        let _ = handle
                            .dialog()
                            .message(format!("Nebula启动失败：{e:#}\n\n将退出应用。"))
                            .title("nebula bootstrap error")
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
            commands::health_full,
            // T-E-S-62: doctor 全子系统健康检查。
            commands::doctor_run,
            // v1.0.1 revert: P0#13 commands-by-topic split is

            // path.  `chat` is the only name that historically
            // collided with a `chat` module elsewhere (e.g. the
            // gRPC-generated client) — it now resolves to the
            commands::chat,
            commands::memory_store,
            commands::memory_search,
            commands::memory_get,
            commands::memory_list_recent,
            commands::memory_update_importance,
            commands::memory_delete,
            commands::memory_get_many,
            // T-E-B-14: Dataview 式查询 DSL 命令。
            commands::memory_query_dsl,
            commands::memory_stats,
            // T-S1-A-02: MemoryOrchestrator 上下文组装 IPC。
            commands::memory_orchestrator_run,
            // v1.5: 因果图谱 + 多粒度摘要命令。
            commands::causal_trace_root_causes,
            commands::causal_find_effects,
            commands::causal_explain,
            commands::summary_generate,
            // v1.6: Git 风格记忆版本控制命令。
            commands::memory_branch_list,
            commands::memory_branch_create,
            commands::memory_branch_checkout,
            commands::memory_branch_delete,
            commands::memory_commit,
            commands::memory_log,
            commands::memory_diff,
            commands::memory_revert,
            commands::memory_merge,
            commands::swarm_execute,
            commands::swarm_list_agents,
            commands::swarm_get_agent,
            // T-S1-B-02: Swarm 可视化事件流订阅。
            commands::subscribe_events,
            // T-E-S-05: 死锁检测状态查询。
            commands::deadlock_status,
            commands::llm_complete,
            commands::llm_chat,
            commands::llm_embed,
            // T-E-C-02: ScreenReader 截图理解 — describe_screenshot 命令。
            commands::describe_screenshot,
            commands::reflect_now,
            commands::list_reflections,
            commands::get_reflection,
            // v2.0: 真正的 Self-Reflection。
            commands::self_reflect_now,
            commands::metrics,
            // T-E-D-02: TTFT(首响时间)统计快照命令。
            commands::metrics_ttft,
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
            // T-E-S-37: skill-pool tags — 返回 Vec<TagCount> 供前端热门标签云。
            commands::skill_tags,
            // T-E-S-45: ClawHub bidirectional compatibility — skill export.
            commands::skill_export_clawhub,
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
            // T-E-C-02: ScreenReader 截图理解 — screenshot 命令(vision feature cfg-gate)。
            commands::screenshot,
            // v1.7: 自启动控制。
            commands::os_autostart_enable,
            commands::os_autostart_disable,
            commands::os_autostart_is_enabled,
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
            // T-E-C-16: chat export (DOCX).
            commands::export_chat_docx,
            // v1.3: device management.
            commands::list_devices,
            commands::revoke_device,
            // v1.3: WebChat share — feature-gated behind `channels`.
            #[cfg(feature = "channels")]
            commands::share_chat,
            // v1.3: ACL.
            commands::acl_set,
            commands::acl_list,
            commands::acl_remove,
            // v1.0.1 P0#12: API key (OS keychain).
            commands::set_api_key,
            commands::get_api_key,
            commands::delete_api_key,
            // T-E-S-40: 多 provider keychain 命令(deepseek/openai-compat/anthropic)。
            commands::set_provider_api_key,
            commands::get_provider_api_key,
            // v1.2: channel (message bridge) — feature-gated.
            #[cfg(feature = "channels")]
            commands::channel_status,
            #[cfg(feature = "channels")]
            commands::channel_send,
            #[cfg(feature = "channels")]
            commands::channel_poll,
            #[cfg(feature = "channels")]
            commands::channel_ping,
            // T-S3-B-01: 原生渠道适配器命令 — feature-gated.
            #[cfg(feature = "channels")]
            commands::channel_list_adapters,
            #[cfg(feature = "channels")]
            commands::channel_send_native,
            #[cfg(feature = "channels")]
            commands::channel_start_all,
            // v1.1 P1-4: security scan.
            commands::injection_scan,
            commands::sandbox_config,
            // v1.1 P0-2: tool registry.
            commands::tool_list,
            commands::tool_invoke,
            // v1.3 P2-7: skill marketplace.
            commands::marketplace_search,
            commands::marketplace_quick_search,
            commands::marketplace_install,
            commands::marketplace_check_updates,
            commands::marketplace_refresh,
            commands::marketplace_stats,
            commands::marketplace_tags,
            commands::marketplace_generate_manifest,
            // T-E-S-46: 技能发布(GitHub Gist / 本地文件)。
            commands::skill_publish,
            // v1.3: MCP (feature-gated).
            #[cfg(feature = "mcp")]
            commands::mcp_list_servers,
            #[cfg(feature = "mcp")]
            commands::mcp_add_server,
            #[cfg(feature = "mcp")]
            commands::mcp_remove_server,
            #[cfg(feature = "mcp")]
            commands::mcp_list_tools,
            // v1.3: Plan + 准奏 + L4 价值层。
            commands::plan_pre_check,
            commands::plan_approve_confirmation,
            commands::plan_deny_confirmation,
            commands::plan_approve_plan,
            commands::plan_reject_plan,
            commands::plan_get_plan,
            commands::plan_get_confirmation,
            commands::values_redact,
            // v2.0: Sidecar 管理命令。
            commands::sidecar_list_status,
            commands::sidecar_start,
            commands::sidecar_stop,
            commands::sidecar_restart,
            // T-S5-B-01: 浮动窗 / 画中画。
            commands::open_floating_chat,
            // T-E-D-03: 桌面悬浮球。
            commands::open_floating_ball,
            // T-E-D-07: 浮动进度窗 + swarm 任务取消。
            commands::open_floating_progress,
            commands::swarm_cancel,
            // M6 #82: Master 编排 + L4 审批命令。
            // master_run 仅在 master-orchestrator feature 启用时编译。
            #[cfg(feature = "master-orchestrator")]
            commands::master_run,
            commands::master_confirm,
            commands::master_confirmation_status,
            commands::master_pending_confirmations,
            // M6 #78: 进化日志 + 回滚命令。
            // 前 3 个仅在 evolution-engine feature 启用时编译;
            // 后 2 个运行时开关在 self-evolution feature 启用时编译
            // (evolution-engine implies self-evolution,因此前 3 个启用时后 2 个必然可用)。
            #[cfg(feature = "evolution-engine")]
            commands::evolution_log_list,
            #[cfg(feature = "evolution-engine")]
            commands::evolution_log_get,
            #[cfg(feature = "evolution-engine")]
            commands::evolution_rollback,
            #[cfg(feature = "self-evolution")]
            commands::evolution_enabled,
            #[cfg(feature = "self-evolution")]
            commands::evolution_set_enabled,
            // M7b #97: Soul 系统运行时开关命令。
            #[cfg(feature = "soul-system")]
            commands::soul_system_enabled,
            #[cfg(feature = "soul-system")]
            commands::soul_system_set_enabled,
            // T-E-S-04: MoA(Mixture of Agents)执行命令。
            commands::moa_execute,
            // T-S6-A-02: 电源管理。
            crate::os::power::power_state,
            crate::os::power::power_pause,
            crate::os::power::power_resume,
            // T-S6-A-03: 自动备份。
            crate::backup::commands::backup_now,
            crate::backup::commands::backup_list,
            crate::backup::commands::backup_restore,
            // T-S6-B-03: CRDT op 日志。
            crate::sync::crdt_op_log::crdt_op_stats,
            crate::sync::crdt_op_log::crdt_op_pending,
            // T-S6-A-01a: OS-Controller Windows。
            crate::os::controller::os_get_foreground_window,
            crate::os::controller::os_list_windows,
            // T-S6-B-01: CLIP 多模态嵌入。
            commands::memory::embed_image,
            // T-E-D-06: 文件拖拽自动吸收到记忆系统。
            commands::memory::sponge_absorb_file,
            // T-E-D-06: Windows 右键菜单 "问Nebula" 安装/卸载/状态查询。
            crate::os::context_menu::context_menu_install,
            crate::os::context_menu::context_menu_uninstall,
            crate::os::context_menu::context_menu_status,
            // T-E-A-06 / T-E-S-20: 费用聚合 + exec 审批列表。
            commands::cost_summary,
            commands::exec_approval_list,
            // T-E-S-50: 自主度滑块 L0-L5 命令。
            commands::autonomy_get_level,
            commands::autonomy_set_level,
            commands::autonomy_list_levels,
            commands::autonomy_route,
            // T-E-S-51: Level 0 内联补全命令。
            commands::inline_complete,
            // T-E-S-59: 统一收件箱命令(feature-gated behind `channels`)。
            #[cfg(feature = "channels")]
            commands::inbox_list,
            #[cfg(feature = "channels")]
            commands::inbox_send,
            #[cfg(feature = "channels")]
            commands::inbox_reply,
            #[cfg(feature = "channels")]
            commands::inbox_mark_read,
            #[cfg(feature = "channels")]
            commands::inbox_unread_count,
            // T-E-A-07: Credits Dashboard 命令。
            commands::credits_overview,
            // T-E-A-08: 费用报告命令(按模型聚合)。
            commands::cost_report,
            // T-E-S-52: Level 1 定向编辑命令。
            commands::directed_edit,
            // T-E-B-09: 文件夹监控索引命令。
            commands::watch_start,
            commands::watch_stop,
            commands::watch_status,
            commands::watch_list_paths,
            // T-E-C-14: 剪贴板智能监听命令。
            commands::clipboard_watch_start,
            commands::clipboard_watch_stop,
            commands::clipboard_watch_status,
            // T-E-S-27: Trusted Diagnostics Channels 命令。
            commands::subscribe_diagnostics,
            commands::diagnostics_snapshot,
            commands::diagnostics_open_logs,
            // T-E-S-41: models.json 动态配置命令(5 个 models_config_* +
            // 2 个 provider key 读写包装)。
            commands::models_config_load,
            commands::models_config_save,
            commands::models_config_set_default,
            // M7a #86 / P1-22: 配置热重载 — 从磁盘重新加载 models.json。
            commands::models_config_reload,
            commands::models_config_add_provider,
            commands::models_config_remove_provider,
            commands::set_provider_key,
            commands::get_provider_key,
            // M6 #83: Provider 连通性测试。
            commands::models_config_test_provider,
            // T-E-S-39: SOUL.md persona 注入命令。
            commands::persona_reload,
            commands::persona_get,
            commands::persona_set_file,
            // T-E-S-28: 对话标注 + Dify 数据集导出。
            commands::annotation_upsert,
            commands::annotation_list,
            commands::annotation_stats,
            commands::annotation_export,
            // T-E-S-24: 文件快照回滚。
            commands::snapshot_create,
            commands::snapshot_rollback,
            commands::snapshot_discard,
            commands::snapshot_list,
            // T-E-S-54: 事件触发器命令(create/list/delete/enable/fire_log)。
            commands::trigger_create,
            commands::trigger_list,
            commands::trigger_delete,
            commands::trigger_enable,
            commands::trigger_fire_log,
            // T-E-S-55: 条件监控 Watch 测试命令(单次轮询预览)。
            commands::watch_test,
            // T-E-C-13: 工作场景模板库命令(scenario_list / scenario_get / scenario_instantiate)。
            commands::scenario_list,
            commands::scenario_get,
            commands::scenario_instantiate,
            // T-E-C-17: IM 扫码绑定命令(6 个 im_* 命令)。
            commands::im_create_webhook_binding,
            commands::im_list_bindings,
            commands::im_delete_binding,
            commands::im_set_enabled,
            commands::im_test_send,
            commands::im_broadcast,
            // T-E-A-11: Smart Prefetch — 打开文件时预取历史对话预热 SemanticCache。
            commands::prefetch_for_file,
            // T-E-B-01: LLM Wiki 编译引擎命令(compile/list/read/search/delete)。
            commands::wiki_compile,
            commands::wiki_list,
            commands::wiki_read,
            commands::wiki_search,
            commands::wiki_delete,
            // T-E-B-06: Wiki _index.md 全量重生命令(供前端"刷新目录"按钮)。
            commands::wiki_regen_index,
            // T-E-B-05: Wiki 双向链接反向查询命令(获取指向指定笔记的所有笔记)。
            commands::wiki_backlinks,
            // T-E-B-13: Wiki 知识卡片聚合命令(note + body + definition + backlinks)。
            commands::wiki_get_card,
            // T-E-B-03: 记忆双向同步 — 用户编辑 wiki 笔记后回写 Memory(SQLite + sponge + vc + log)。
            commands::wiki_update_from_user,
            // T-E-S-29: OpenTelemetry 状态查询命令(始终注册,内部 cfg 分支)。
            commands::otel_status,
            // T-E-S-43: SQLite 加密命令(status 始终编译;enable/disable cfg-gated)。
            commands::db_encryption_status,
            commands::db_encryption_enable,
            commands::db_encryption_disable,
            // T-E-S-32: MCP Server 子进程生命周期命令(feature-gated)。
            #[cfg(feature = "mcp")]
            commands::mcp_server_list,
            #[cfg(feature = "mcp")]
            commands::mcp_server_start,
            #[cfg(feature = "mcp")]
            commands::mcp_server_stop,
            #[cfg(feature = "mcp")]
            commands::mcp_server_status,
            #[cfg(feature = "mcp")]
            commands::mcp_server_logs,
            // T-E-A-14: Arena A/B 测试命令(create_match / vote / leaderboard)。
            commands::arena_create_match,
            commands::arena_vote,
            commands::arena_leaderboard,
            // T-E-S-33: OpenAPI 工具服务器命令(feature-gated)。
            #[cfg(feature = "openapi")]
            commands::openapi_register_tools,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Convenience wrapper for non-Tauri contexts (tests, CLI) that want the
/// same [`AppState`] wiring without spawning a window.
pub async fn build_state_for_tests(
    config: AppConfig,
    app_handle: tauri::AppHandle,
) -> anyhow::Result<AppState> {
    AppState::bootstrap(config, app_handle).await
}

// ---------------------------------------------------------------------------
// T-E-C-20: headless Docker 模式 — 无 Tauri 窗口的 bootstrap 路径。
// ---------------------------------------------------------------------------

impl AppState {
    /// T-E-C-20: headless 模式 bootstrap — 无 `AppHandle`,跳过桌面特有功能
    /// (通知弹窗、预算事件 emit、悬浮球等)。
    ///
    /// 流程与 `bootstrap()` 一致,但:
    /// - `NotificationService` 使用 no-op 实现(headless cfg)
    /// - `CostTracker` 无预算告警回调(无法 emit Tauri event)
    /// - `ClipboardWatcherEngine` 不启动(无桌面)
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
            &config, &sqlite, &lance, &embedder, &llm, &sponge, &tool_registry,
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
        let self_reflection = Arc::new(
            crate::memory::self_reflection::SelfReflectionEngine::new(
                sqlite.clone(),
                swarm.values_layer().clone(),
                reflection.config().clone(),
            ),
        );

        // Phase 4: skills
        let (skills, skill_extractor, skill_composer, marketplace, skill_audit_logger) =
            Self::bootstrap_skills(&config, &sqlite, &llm, &exec_approval);
        swarm.set_composer(skill_composer.clone());

        // L0 + Orchestrator
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
        let file_watcher = Arc::new(
            crate::memory::file_watcher::FileWatcherEngine::new(sponge.clone()),
        );
        let file_watcher_worker: Arc<parking_lot::Mutex<Option<JoinHandle<()>>>> =
            Arc::new(parking_lot::Mutex::new(None));
        if !config.watch_paths.is_empty() {
            let watch_paths: Vec<std::path::PathBuf> = config
                .watch_paths
                .iter()
                .map(|s| std::path::PathBuf::from(s))
                .collect();
            file_watcher.start(watch_paths);
            if let Some(handle) = file_watcher.clone().spawn_worker() {
                *file_watcher_worker.lock() = Some(handle);
            }
        }

        // Clipboard watcher (headless: 构造但不启动)
        let clipboard_watcher: Arc<tokio::sync::Mutex<crate::os::ClipboardWatcherEngine>> =
            Arc::new(tokio::sync::Mutex::new(crate::os::ClipboardWatcherEngine::new()));

        // Storage backend
        let storage = crate::storage::StorageBackendFactory::from_config(&config.storage_backend)
            .context("initializing storage backend")?;

        // Snapshot engine
        let snapshot_engine = Arc::new(
            crate::snapshot::SnapshotEngine::new(storage.clone())
                .context("initializing snapshot engine")?,
        );

        // T-E-C-20: headless 模式 NotificationService — 无需 AppHandle,
        // 所有方法在 cfg(headless) 下为 no-op(日志代替弹窗)。
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
        // T-E-B-03: 注入 sponge + version_control(headless 模式同样需要双向同步)。
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

        #[cfg(feature = "mcp")]
        let mcp_manager = Arc::new(crate::mcp::client::McpManager::new());
        #[cfg(feature = "mcp")]
        let mcp_registry = {
            let log_dir = default_log_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            Arc::new(crate::mcp::registry::McpServerRegistry::new(
                mcp_manager.clone(),
                log_dir,
            ))
        };

        // T-E-A-14: Arena A/B 测试 — headless 模式同样构造 leaderboard。
        let arena = Arc::new(crate::llm::arena::ArenaLeaderboard::new()
            .with_store(sqlite.as_ref().clone()));
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

        // M5 #68 / M6 #82 (headless): ApprovalGate + ConfirmationRegistry + MasterOrchestrator
        let confirmation_registry = Arc::new(crate::autonomy::ConfirmationRegistry::new());
        let approval_gate = Arc::new(crate::autonomy::ApprovalGate::new(
            crate::autonomy::WorkerRiskMap::new(),
            confirmation_registry.clone(),
        ));
        info!(target: "nebula", "approval gate ready (headless, M5 #68)");
        #[cfg(feature = "master-orchestrator")]
        let master_orchestrator = {
            // M7a #86: 复用顶层 dispatcher(feature off 时为 None)。
            #[cfg(feature = "unified-dispatcher")]
            let dispatcher = dispatcher.clone();
            #[cfg(not(feature = "unified-dispatcher"))]
            let dispatcher: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>> = None;
            let mo = Arc::new(crate::swarm::MasterOrchestrator::new(
                swarm.clone(),
                dispatcher,
            ));
            info!(target: "nebula", "MasterOrchestrator ready (headless, M6 #82)");
            mo
        };

        // M6 #78: EvolutionEngine + EvolutionLog + Roller 构造(headless)。
        // M7a #86: 复用顶层 dispatcher(不再独立构造)。
        #[cfg(feature = "evolution-engine")]
        let (evolution_engine, evolution_log, roller) = {
            use crate::evolution::engine::{EvolutionEngine, EvolutionLog, Roller, EvolutionEngineConfig};
            use std::path::PathBuf;
            let log = Arc::new(EvolutionLog::new(PathBuf::from("evolution_log.md")));
            let roller = Arc::new(Roller::new(log.clone(), PathBuf::from("SOUL.md")));
            let engine = {
                // M7a #86: 复用顶层 dispatcher(feature off 时为 None)。
                #[cfg(feature = "unified-dispatcher")]
                let dispatcher_opt = dispatcher.clone();
                #[cfg(not(feature = "unified-dispatcher"))]
                let dispatcher_opt: Option<Arc<crate::llm::dispatcher::UnifiedModelDispatcher>> = None;
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
            message_bridge: None,
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
            // M7a #86 (headless): 顶层 dispatcher。
            #[cfg(feature = "unified-dispatcher")]
            dispatcher,
        })
    }

    /// T-E-C-20: headless 变体的 AI 核心初始化 — 无预算告警回调(无法 emit Tauri event)。
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

        // T-E-C-20: headless 模式 — CostTracker 无预算告警回调(无法 emit)。
        let cost_tracker = {
            use crate::llm::cost_tracker::CostTracker;
            let base = CostTracker::new();
            // headless: 不注入 budget_alert callback(无 AppHandle 可 emit)
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
            let compressor = Arc::new(
                crate::llm::token_juice::TokenJuiceCompressor::new(
                    ollama_for_compress.clone(),
                    config.chat_model.clone(),
                    crate::llm::token_juice::TokenJuiceConfig::default(),
                ),
            );
            llm_builder = llm_builder.with_token_juice(compressor);
        }
        if config.router_enabled {
            let router = Arc::new(
                crate::llm::model_router::ModelRouter::new(
                    ollama_for_compress.clone(),
                    config.router_classifier_model.clone(),
                ),
            );
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
            let mut sponge_builder = SpongeEngine::new(
                sqlite.clone(),
                lance.clone(),
                embedder.clone(),
            );
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

        // headless: 迁移 env key → keychain(可能失败,soft skip)
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

/// Re-exported for convenience.
pub use memory::reflect::Reflection;
pub use memory::types::{Memory, MemoryLayer, MemoryType, MultiGranularity};
pub use memory::MigrationStatus;
pub use metrics::MetricsSnapshot;

#[cfg(test)]
mod tests {
    use super::AppConfig;

    /// T-E-C-02: AppConfig 默认 vision_model 应为 "qwen2.5-vl:3b"。
    /// 测试只检查默认值 OR env 覆盖值,不修改 env var(避免并行测试 race)。
    #[test]
    fn test_vision_model_default() {
        let config = AppConfig::from_env();
        match std::env::var("NEBULA_VISION_MODEL") {
            Ok(v) => {
                // env 被设置时,验证 env 覆盖生效(供 CI 调试)。
                assert_eq!(
                    config.vision_model, v,
                    "vision_model should equal NEBULA_VISION_MODEL env var when set"
                );
            }
            Err(_) => {
                // env 未设置时,验证默认值。
                assert_eq!(
                    config.vision_model, "qwen2.5-vl:3b",
                    "default vision_model must be qwen2.5-vl:3b per spec"
                );
            }
        }
    }
}
