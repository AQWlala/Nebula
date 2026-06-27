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
pub const COLD_START_BUDGET_MS: u128 = if cfg!(target_os = "windows") {
    8_000 // Windows: antivirus + Defender overhead
} else {
    5_000 // macOS / Linux
};

/// Soft latency budget for user-facing commands (milliseconds).
/// The perf monitor stamps every Tauri command invocation; the
/// histogram bucket counts in the report are computed relative
/// to this value.
pub const COMMAND_LATENCY_BUDGET_MS: u128 = 200;
