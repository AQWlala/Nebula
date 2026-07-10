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
//!
//! ## v1.0.2
//! * Commands split into logical submodules for maintainability.
//!   All public items are re-exported so `generate_handler!` paths
//!   (`commands::chat`, `commands::memory_store`, etc.) continue to
//!   resolve.

pub mod error;

// Submodules — each groups related commands and DTOs.
pub mod chat;
// T-E-C-14: 剪贴板监听命令(watch_start / stop / status)。
pub mod clipboard;
pub mod core;
pub mod editor;
pub mod llm;
pub mod memory;
pub mod os;
pub mod reflect;
pub mod skill;
pub mod swarm;
pub mod sync;
pub mod work;
pub mod writing;
// T-E-S-28: 对话标注命令(upsert / list / stats / export)。
pub mod annotations;
// v1.2: channel commands — feature-gated behind `channels`.
#[cfg(feature = "channels")]
pub mod channel;
pub mod device;
pub mod export;
pub mod identity;
pub mod security;
// v1.3: WebChat share — feature-gated behind `channels`.
pub mod acl;
pub mod tool;
// v1.3: Plan + 准奏 + L4 价值层命令。
pub mod plan;
// v2.0: Sidecar 管理命令。
pub mod sidecar;
// T-S5-B-01: 浮动窗 / 画中画命令。
pub mod window;
// T-E-S-30: MCP Tauri 命令（工具发现 + 调用）— feature-gated。
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "channels")]
pub mod webchat;
// T-E-S-50: 自主度滑块 L0-L5 命令。
pub mod autonomy;
// T-E-S-51: Level 0 内联补全命令。
pub mod inline_completion;
// T-E-S-59: 统一收件箱命令 — feature-gated behind `channels`。
#[cfg(feature = "channels")]
pub mod inbox;
// T-D-B-13: 系统服务注册命令。
pub mod daemon;
pub use daemon::*;
// T-E-S-10: WorkflowCanvas 命令。
pub mod workflow;
pub use workflow::*;
// T-E-A-07: Credits Dashboard 命令。
pub mod credits;
// T-E-A-08: 费用报告命令。
pub mod cost;
// T-E-S-52: Level 1 定向编辑命令。
pub mod directed_edit;
// T-E-B-09: 文件夹监控索引命令。
pub mod watch;
// T-E-S-41: models.json 动态配置命令。
pub mod models_config;
// P0-1: 模型配置中心命令(provider 状态/连通性测试/模型发现/路由配置)。
pub mod llm_config;
// T-E-S-27: Trusted Diagnostics Channels 命令。
pub mod diagnostics;
// T-E-S-24: 文件快照回滚命令。
pub mod snapshot;
// T-E-S-54: 事件触发器命令(文件/消息/Webhook 三种触发器统一调度)。
pub mod triggers;
// T-E-C-13: 工作场景模板库命令(scenario_list / scenario_get / scenario_instantiate)。
pub mod scenarios;
// T-E-C-17: IM 扫码绑定命令(6 个 im_* 命令)。
pub mod im;
// T-E-A-11: Smart Prefetch 命令(打开文件时预取历史对话预热 SemanticCache)。
pub mod prefetch;
// T-E-B-01: LLM Wiki 编译引擎命令(compile/list/read/search/delete)。
pub mod wiki;
// T-E-B-16: MDRM 5 维关系图谱命令(trace_temporal/find_entities/trace_hierarchy/find_similar/get_graph)。
pub mod mdrm;
// T-E-S-29: Observability 命令(otel_status,内部 cfg 分支)。
pub mod observability;
// T-E-A-14: Arena A/B 测试命令(create_match / vote / leaderboard)。
pub mod arena;
// T-E-S-33: OpenAPI 工具服务器命令 — feature-gated behind `openapi`。
#[cfg(feature = "openapi")]
pub mod openapi;
// Extracted: NebulaService trait impl on AppState (chat / memory_store /
// memory_search / swarm_execute / llm_complete)。
pub mod service;
// T-E-S-39: SOUL.md / AGENTS.md / TOOLS.md persona 命令(reload / get / set_file)。
pub mod persona;
// M6 #82: Master 编排 + L4 审批命令(master_run / master_confirm / master_confirmation_status / master_pending_confirmations)。
pub mod master;
// M6 #78: 进化日志 + 回滚命令(evolution_log_list / evolution_log_get / evolution_rollback /
// evolution_enabled / evolution_set_enabled)。前 3 个 evolution-engine feature 门控,
// 后 2 个运行时开关始终编译。
pub mod evolution;
// M7b #97: Soul 系统运行时开关命令(soul_system_enabled / soul_system_set_enabled)。
// 由 soul-system feature 门控,对齐 evolution_enabled / evolution_set_enabled 模式。
pub mod soul;
// P0-6: Hermes 式自动发明技能命令(auto_invent_get_patterns / accept_pattern /
// reject_pattern / get_config / set_config)。
pub mod skill_auto_invent;

// Re-export the API DTOs so other modules (gRPC, tests) can reach them
// through the `commands` namespace without depending on the internal
// `api::server` module path.
pub mod api {
    pub use crate::api::server::{
        ChatRequestDto, NebulaService, SearchMemoryHit, SearchMemoryRequest, StoreMemoryRequest,
        StoreMemoryResponse,
    };
    pub use crate::skills::types::{
        CreateSkillRequest as CreateSkillDto, ListSkillsRequest as ListSkillsDto,
        RateSkillRequest as RateSkillDto, Skill as SkillDto, SkillResult as SkillResultDto,
        SkillSearchRequest as SkillSearchDto, UseSkillRequest as UseSkillDto,
    };
}

// Re-export all public items from submodules so that
// `commands::chat`, `commands::memory_store`, etc. still resolve
// for `generate_handler!` in `lib.rs`.
pub use acl::*;
// T-E-S-28: 对话标注命令 re-export。
pub use annotations::*;
#[cfg(feature = "channels")]
pub use channel::*;
pub use chat::*;
// T-E-C-14: 剪贴板监听命令 re-export。
pub use clipboard::*;
pub use core::*;
pub use device::*;
pub use editor::*;
pub use export::*;
pub use identity::*;
pub use llm::*;
#[cfg(feature = "mcp")]
pub use mcp::*;
pub use memory::*;
pub use os::*;
pub use plan::*;
pub use reflect::*;
pub use security::*;
pub use sidecar::*;
pub use skill::*;
pub use swarm::*;
pub use sync::*;
pub use tool::*;
#[cfg(feature = "channels")]
pub use webchat::*;
pub use window::*;
pub use work::*;
pub use writing::*;
// T-E-S-50 / T-E-S-51 / T-E-S-59: 新增命令 re-export。
pub use autonomy::*;
#[cfg(feature = "channels")]
pub use inbox::*;
pub use inline_completion::*;
// T-E-A-07 / T-E-S-52: 新增命令 re-export。
pub use credits::*;
// T-E-A-08: 费用报告命令 re-export。
pub use cost::*;
pub use directed_edit::*;
// T-E-B-09: 文件夹监控 re-export。
pub use watch::*;
// T-E-S-41: models.json 动态配置命令 re-export。
pub use models_config::*;
// P0-1: 模型配置中心命令 re-export。
pub use llm_config::*;
// T-E-S-27: Trusted Diagnostics 命令 re-export。
pub use diagnostics::*;
// T-E-S-24: 文件快照回滚命令 re-export。
pub use snapshot::*;
// T-E-S-54: 事件触发器命令 re-export。
pub use triggers::*;
// T-E-C-13: 工作场景模板库命令 re-export。
pub use scenarios::*;
// T-E-C-17: IM 扫码绑定命令 re-export。
pub use im::*;
// T-E-A-11: Smart Prefetch 命令 re-export。
pub use prefetch::*;
// T-E-B-01: Wiki 命令 re-export。
pub use wiki::*;
// T-E-S-29: Observability 命令 re-export。
pub use observability::*;
// T-E-A-14: Arena A/B 测试命令 re-export。
pub use arena::*;
// T-E-S-33: OpenAPI 工具服务器命令 re-export(feature-gated)。
#[cfg(feature = "openapi")]
pub use openapi::*;

// Extracted submodules re-export。
// service 模块仅含 `impl NebulaService for AppState` trait 实现,无私有项可 re-export。
pub use persona::*;
// M6 #82: master 命令 re-export。
pub use master::*;
// M6 #78: evolution 命令 re-export(feature 全 off 时模块内无公开项,
// 门控以避免 unused import 警告)。
#[cfg(any(feature = "evolution-engine", feature = "self-evolution"))]
pub use evolution::*;

#[cfg(feature = "soul-system")]
pub use soul::*;

// P0-6: SkillAutoInventor 命令 re-export。
pub use skill_auto_invent::*;

pub use error::{CommandError, ErrorCode};
