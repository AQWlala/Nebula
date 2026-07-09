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

// ── subsystem modules ──────────────────────────────────────────────
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
// T-E-C-08: Shadow Workspace — Agent 隔离执行环境(git worktree + 临时分支)。
pub mod long_task;
pub mod shadow_workspace;
// T-E-C-15: 语音交互引擎(STT + TTS + 唤醒词 + 音频捕获抽象)。
pub mod voice;
// T-E-C-18: OAuth 集成层（5 服务）— GitHub/Google/Microsoft/Slack/Notion。
// PKCE + 本地回调服务器 + AES-256-GCM 加密 SQLite token 存储。
pub mod oauth;
// T-E-C-06: Hybrid Browser Agent — API + VLM 双模式浏览器自动化。
// API 模式(reqwest + regex)完整实现;VLM 模式(vision feature + Ollama)框架化。
pub mod browser;

// v1.3: closed-loop self-evolution (task outcomes + skill archive +
// prompt mutator).  Off by default; enable with
// `--features self-evolution`.
#[cfg(feature = "self-evolution")]
pub mod evolution;

// T-E-L-02: 5 字段 cron 表达式解析器 — 通用工具，无 feature gate。
// 文件物理位置在 evolution/cron_expr.rs，但用 #[path] 绕过 evolution 模块的
// #![cfg(feature = "self-evolution")] 门控，使 CI 用 --features grpc,channels
// 即可编译测试。cron_scheduler.rs (self-evolution 门控) 通过 crate::cron_expr 引用。
#[path = "evolution/cron_expr.rs"]
pub mod cron_expr;

// M1 里程碑：Soul 系统（SOUL.md + SoulCompiler + 注入防护 + 原子写入）。
// 默认 off；启用需 `--features soul-system`。
// 运行时还需环境变量 `SOUL_SYSTEM_ENABLED=1` 或 Settings UI 显式开启。
// 参见 ADR-004 Feature Flag 策略。
#[cfg(feature = "soul-system")]
pub mod soul;

#[cfg(feature = "grpc")]
pub mod grpc;

// ── app-assembly modules (P0-B split from the former lib.rs monolith) ─
/// Configuration loaded from env vars. See [`AppConfig`].
pub mod app_config;
/// The shared managed-state struct. See [`AppState`].
pub mod app_state;
/// Bootstrap phase — wires all subsystems into [`AppState`].
pub mod bootstrap;
/// Headless bootstrap variant (no `AppHandle`).
pub mod bootstrap_headless;
/// Tauri entry — `run()` and `build_state_for_tests`.
pub mod tauri_setup;
/// Tracing subscriber setup (`init_tracing`, `default_log_dir`).
pub mod tracing_setup;

// ── public re-exports (stable API surface) ─────────────────────────
pub use app_config::AppConfig;
pub use app_state::AppState;
pub use tauri_setup::{build_state_for_tests, run};

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
