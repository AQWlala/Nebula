//! EvolutionEngine — 4 Phase 进化管线（M4 任务 #55-58）。
//!
//! 实现 ADR-003 §6.3 的 EvolutionEngine 编译流程：
//!
//! ```text
//! L1 (Rolling message history)
//!   │
//!   ▼ Phase 1: 经验提取（dispatch(Evolution) → L2 Experience）
//!   │
//!   ▼ Phase 2: 知识编译（dispatch(Evolution) → L3 Facts）
//!   │
//!   ▼ Phase 3: 元认知反思（dispatch(Evolution) → L5 Lessons）
//!   │
//!   ▼ Phase 4: Soul 反哺（L5 → SOUL.md evolution-append）
//! ```
//!
//! ## 设计要点
//!
//! - **强制本地路由**：所有 4 Phase LLM 调用经 `dispatch(WorkType::Evolution)`
//!   走本地 Ollama（qwen2.5:7b），不计费、不外发（P0-2 / P1-4 EA-5）。
//! - **domain 隔离**：进化写入经 `absorb_with_principal("evolution:<master_id>", mem)`，
//!   写入 `mem.domain = "<master_id>"`，与 `shared` / `worker:*` 域隔离（P0-9）。
//! - **三层共存**（P0-3 EA-5）：
//!   * `PromptSelfMutator`：Worker 级（agent system_prompt 重写）
//!   * `SkillAutoEvolver`：Skill 级（技能归档/恢复）
//!   * `EvolutionEngine`：Master 级（跨会话 L2/L3/L5 提炼 + SOUL 反哺）
//!   三者通过 `domain` 字段隔离，互不干扰。
//! - **注入防护**：Phase 4 写入 SOUL.md 前调用 `scan_prompt_injection()`，
//!   Critical/High 丢弃并记入 warnings（P1-13）。
//! - **原子写入**：Phase 4 写 SOUL.md 经 `soul::atomic_write`，
//!   write-temp-then-rename + 备份 + 文件锁（P1-14）。
//! - **进化日志**：每次进化写入 `evolution_log.md`，含 provenance + 可回滚（#64）。
//! - **回滚**：`rollback(N)` 从 `evolution_log.md` 查找条目 + 从 SOUL.md 删除对应行（#62）。
//!
//! ## Feature Gate（参考 soul/ 模块的双层 gate 模式）
//!
//! - 编译期：`#[cfg(feature = "evolution-engine")]`（Cargo.toml 已声明，默认 off）
//! - 运行时：复用 `evolution::EVOLUTION_ENABLED`（与 PromptSelfMutator / SkillAutoEvolver 共享）
//!
//! 参见 ADR-004 Feature Flag 策略。

#![cfg(feature = "evolution-engine")]

pub mod log;
pub mod pipeline;
pub mod rollback;

use serde::{Deserialize, Serialize};

/// 重导出核心类型，便于外部使用。
pub use log::{EvolutionLog, EvolutionLogEntry, EvolutionLogError};
pub use pipeline::{EvolutionEngine, EvolutionError, EvolutionPhase, EvolutionResult, PhaseOutput};
pub use rollback::{RollbackError, RollbackResult, Roller};

/// EvolutionEngine 配置 DTO（通过 Tauri 命令边界交换）。
///
/// 复用 `evolution::EvolutionConfig` 的部分字段（如 `prompt_mutator_window`），
/// 新增 4 Phase 专用配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionEngineConfig {
    /// 是否启用（运行时双层 gate：feature flag + 此字段）。
    pub enabled: bool,
    /// 单次 Phase LLM 调用超时（秒）。默认 30（Evolution 走本地 7b/14b，比 SoulCompile 5s 更长）。
    pub phase_timeout_secs: u64,
    /// Phase 1 读取 L1 的窗口大小（最近 N 条 L1 记忆）。默认 50。
    pub phase1_l1_window: usize,
    /// Phase 2 读取 L2 的窗口大小。默认 30。
    pub phase2_l2_window: usize,
    /// Phase 3 读取 L2 的窗口大小。默认 30。
    pub phase3_l2_window: usize,
    /// Phase 3 读取 L3 的窗口大小。默认 30。
    pub phase3_l3_window: usize,
    /// Phase 4 写入 SOUL.md evolution-append 的最大行数。默认 100。
    pub phase4_max_lines: usize,
    /// 进化日志文件路径（通常为 workspace_root / "evolution_log.md"）。
    pub log_path: String,
    /// SOUL.md 文件路径（Phase 4 写入目标）。
    pub soul_md_path: String,
}

impl Default for EvolutionEngineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            phase_timeout_secs: 30,
            phase1_l1_window: 50,
            phase2_l2_window: 30,
            phase3_l2_window: 30,
            phase3_l3_window: 30,
            phase4_max_lines: 100,
            log_path: "evolution_log.md".to_string(),
            soul_md_path: "SOUL.md".to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
