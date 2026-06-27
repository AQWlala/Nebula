//! v1.0: startup-time profiler + final report.
//!
//! [`StartupTimer`] is a no-`std`-lock stopwatch that captures
//! named milestones during `AppState::bootstrap` and the
//! Tauri `setup` hook.  When the front-end (or a CLI tool) calls
//! the `startup_report` command we hand back a [`StartupReport`]
//! that lists every milestone with the delta from `t0` and a
//! green/amber/red traffic light per [`crate::perf::COLD_START_BUDGET_MS`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::perf::COLD_START_BUDGET_MS;

/// A single milestone, recorded relative to `t0` (the moment the
/// process started the bootstrap sequence).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupMilestone {
    pub name: String,
    pub elapsed_ms: u128,
    pub over_budget: bool,
}

/// End-of-startup report.  Serialised as JSON for the front-end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupReport {
    pub total_ms: u128,
    pub over_budget: bool,
    pub milestones: Vec<StartupMilestone>,
    /// Convenience: which milestone took the most wall-clock time.
    pub slowest: Option<String>,
    /// Whether the budget was met.
    pub status: StartupStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StartupStatus {
    /// Everything finished within the cold-start budget.
    Green,
    /// One or more milestones exceeded the budget but the app
    /// still came up.  Surface a warning in the status bar.
    Amber,
    /// The bootstrap itself failed; the report will be partial.
    Red,
}

/// A handle to a startup timer.  Clone freely.
#[derive(Clone)]
pub struct StartupTimer {
    inner: Arc<Inner>,
}

struct Inner {
    t0: Instant,
    milestones: Mutex<BTreeMap<String, StartupMilestone>>,
}

impl StartupTimer {
    /// Start a new timer.  Capture the moment of creation as `t0`.
    pub fn start() -> Self {
        Self {
            inner: Arc::new(Inner {
                t0: Instant::now(),
                milestones: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Record a milestone.  Calling `mark` twice with the same
    /// name is idempotent (the second call is a no-op so we don't
    /// double-count).  Returns the elapsed ms for the milestone.
    pub fn mark(&self, name: &str) -> u128 {
        let mut g = self.inner.milestones.lock();
        if g.contains_key(name) {
            return g[name].elapsed_ms;
        }
        let elapsed = self.inner.t0.elapsed().as_millis();
        let m = StartupMilestone {
            name: name.to_string(),
            elapsed_ms: elapsed,
            over_budget: elapsed > COLD_START_BUDGET_MS,
        };
        g.insert(name.to_string(), m);
        elapsed
    }

    /// Build a final report.  Sorted alphabetically by milestone
    /// name (BTreeMap) so the JSON is stable across runs.
    pub fn report(&self) -> StartupReport {
        let g = self.inner.milestones.lock();
        let total = self.inner.t0.elapsed().as_millis();
        let milestones: Vec<StartupMilestone> = g.values().cloned().collect();
        let over_budget = milestones.iter().any(|m| m.over_budget);
        let slowest = milestones
            .iter()
            .max_by_key(|m| m.elapsed_ms)
            .map(|m| m.name.clone());
        let status = if total > COLD_START_BUDGET_MS * 2 {
            StartupStatus::Red
        } else if over_budget {
            StartupStatus::Amber
        } else {
            StartupStatus::Green
        };
        StartupReport {
            total_ms: total,
            over_budget,
            milestones,
            slowest,
            status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn report_is_green_when_fast() {
        let t = StartupTimer::start();
        t.mark("boot");
        t.mark("sqlite_open");
        let r = t.report();
        assert_eq!(r.status, StartupStatus::Green);
        assert_eq!(r.milestones.len(), 2);
        assert!(r.slowest.is_some());
    }

    #[test]
    fn duplicate_mark_is_idempotent() {
        let t = StartupTimer::start();
        let first = t.mark("x");
        thread::sleep(Duration::from_millis(5));
        let second = t.mark("x");
        assert_eq!(first, second);
        assert_eq!(t.report().milestones.len(), 1);
    }

    #[test]
    fn amber_when_one_milestone_is_slow() {
        let t = StartupTimer::start();
        // Manually push a milestone over the budget; this is the
        // only way to fake slowness in a unit test.
        t.inner.as_ref().milestones.lock().insert(
            "slow".into(),
            StartupMilestone {
                name: "slow".into(),
                elapsed_ms: COLD_START_BUDGET_MS + 1,
                over_budget: true,
            },
        );
        let r = t.report();
        assert_eq!(r.status, StartupStatus::Amber);
    }
}
