//! v1.0: performance monitoring & telemetry.
//!
//! Two layers:
//!
//! 1. **In-process counters** (cheap, always-on) — see
//!    [`crate::metrics`]. These add zero overhead and survive
//!    across commands; the perf module only reads from them.
//! 2. **System telemetry** (opt-in via the `perf-telemetry` feature)
//!    — RSS / VSZ / CPU via `sysinfo`.  Used by the
//!    [`monitor::PerfMonitor`] background task to keep a rolling
//!    sample of memory usage and surface it in the status bar.
//!
//! The module also exposes a tiny startup profiler ([`StartupTimer`])
//! that marks named milestones during `AppState::bootstrap` so the
//! cold-start budget is auditable.  The data ends up in
//! [`report::StartupReport`].

pub mod monitor;
pub mod report;

pub use monitor::PerfMonitor;
pub use report::{StartupReport, StartupTimer};

/// Target RSS budget (bytes) for an idle v1.0 build on desktop.
/// The status bar turns red if the live sample exceeds this.
pub const RSS_BUDGET_BYTES: u64 = 500 * 1024 * 1024; // 500 MiB

/// Cold-start budget (milliseconds).  The startup report is
/// considered "green" if every milestone finishes inside this
/// envelope; otherwise it is reported with a `> budget` flag.
///
/// T-E-D-01: 调整为 3000ms(原 Windows 8000ms / macOS Linux 5000ms)。
/// 目标 < 3s,作为冷启动优化的回归报警基线:
/// - bootstrap 并行化(Phase 2/3/4 tokio::join!)
/// - 网络/IO 阻塞操作后台化(SkillMarketplace::refresh / seed_demo_skills)
/// - L0Cache 预热(后台 spawn)
/// - SemanticCache 持久化 + 重启后预热(migration 031)
/// - 前端顶层视图 lazy 化(main.tsx)
///
/// 若启动时间超过此值,perf report 会标记 `> budget`,触发回归排查。
/// 平台差异不再区分(并行化 + 后台化后 Windows 与 macOS/Linux 都应达标)。
pub const COLD_START_BUDGET_MS: u128 = 3_000;

/// Soft latency budget for user-facing commands (milliseconds).
/// The perf monitor stamps every Tauri command invocation; the
/// histogram bucket counts in the report are computed relative
/// to this value.
pub const COMMAND_LATENCY_BUDGET_MS: u128 = 200;
