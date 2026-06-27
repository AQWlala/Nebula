//! `nine_snake::evolution` — self-evolution loop (v1.3).
//!
//! Building blocks for the closed-loop, agent-level self-evolution
//! promised by the README and `docs/ARCHITECTURE.md` §3.1 (planned).
//!
//! The module is intentionally *additive*: every public entry point
//! is either behind an explicit `feature = "self-evolution"` gate or
//! requires an `evolution_enabled()` flag to be flipped on by the
//! Settings UI.  The default code path of v1.1 stays unchanged.
//!
//! Sub-modules:
//!   * `outcome`        - TaskOutcome DTO + OutcomeLedger + Sqlite backend
//!   * `outcome_collectors`  - hooks that auto-emit outcomes from skills/swarm/chat
//!   * `skill_evolver`  - SkillAutoEvolver + EvolutionPolicy + SkillArchive
//!   * `prompt_mutator` - PromptSelfMutator + Snapshot store + rollback
//!   * `goal_signal`    - goal function (win rate) derivation
//!
//! The wiring into `SkillEngine`, `SwarmOrchestrator`, `ChatPanel` (Rust
//! side) and `Memory::Reflect` happens via the small adapter structs
//! in `outcome_collectors.rs` and `skill_evolver.rs`, *never* by
//! mutating existing call sites outside of feature-gated blocks.

#![cfg(feature = "self-evolution")]

pub mod goal_signal;
pub mod outcome;
pub mod outcome_collectors;
pub mod prompt_mutator;
pub mod skill_evolver;

use serde::{Deserialize, Serialize};

/// Master switch for the whole self-evolution loop.  Read by every
/// mutator / evolver before doing work.  A static default of `false`
/// means that simply upgrading to v1.3 with the new code in place
/// does NOT change runtime behaviour — the user has to flip this in
/// Settings before any prompts are rewritten or skills archived.
pub static EVOLUTION_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn evolution_enabled() -> bool {
    EVOLUTION_ENABLED.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn set_evolution_enabled(on: bool) {
    EVOLUTION_ENABLED.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// Versioned config DTO exchanged over the Tauri command boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionConfig {
    pub enabled: bool,
    /// `success_count >= usage_count * (1.0 - rate_floor)` means "well
    /// used & well rated"; below that the skill is a candidate for
    /// archive.  Default 0.5 = at least half the ratings must be >= 0.5.
    pub archive_rate_floor: f32,
    /// Minimum `usage_count` before a skill is a candidate for
    /// archive / mutation.  Default 20.
    pub archive_min_usage: u32,
    /// How often (in seconds) the background worker runs the goal
    /// signal derivation.  Default 3600 (1 hour).  `0` disables the
    /// worker entirely.
    pub background_period_secs: u64,
    /// Number of recent outcomes to feed into a PromptSelfMutator
    /// pass.  Default 30.
    pub prompt_mutator_window: u32,
    /// Confidence threshold (`>=`) to consider an outcome "winning".
    /// Default 0.7.
    pub goal_confidence_threshold: f32,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            archive_rate_floor: 0.5,
            archive_min_usage: 20,
            background_period_secs: 3600,
            prompt_mutator_window: 30,
            goal_confidence_threshold: 0.7,
        }
    }
}

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub struct EvolutionWorker {
    config: EvolutionConfig,
    cancel: CancellationToken,
}

impl EvolutionWorker {
    pub fn new(config: EvolutionConfig) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
        }
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub async fn run(self, mutator: prompt_mutator::SqlitePromptSelfMutator) {
        if self.config.background_period_secs == 0 {
            info!(target: "nine_snake.evolution", "background worker disabled (period=0)");
            return;
        }

        info!(
            target: "nine_snake.evolution",
            period_secs = self.config.background_period_secs,
            "evolution worker started"
        );

        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(self.config.background_period_secs)) => {
                    if !evolution_enabled() {
                        continue;
                    }

                    let agents = ["coder", "writer", "reviewer", "researcher", "planner"];
                    for agent in agents {
                        match mutator.run_once(agent, "") {
                            Ok(Some(result)) => {
                                info!(
                                    target: "nine_snake.evolution",
                                    agent = result.target,
                                    snapshot = %result.snapshot_id,
                                    "prompt mutation proposed"
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                warn!(target: "nine_snake.evolution", agent, error = %e, "mutation pass failed");
                            }
                        }
                    }
                }
                _ = self.cancel.cancelled() => {
                    info!(target: "nine_snake.evolution", "worker cancelled");
                    return;
                }
            }
        }
    }
}
