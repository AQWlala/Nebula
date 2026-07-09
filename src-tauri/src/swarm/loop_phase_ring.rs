//! T-E-L-08a: Loop 运行时阶段环 — Loop 执行过程中的阶段可视化与状态追踪。
//!
//! 本模块提供 Loop 执行过程中的阶段状态机管理（[`PhaseRing`]），支持
//! 阶段推进、暂停/恢复、失败/取消/完成等状态转换，并生成阶段环可视化
//! 数据（[`PhaseRingVisualizer`]）供前端画布组件渲染。
//!
//! ## 阶段推进顺序
//!
//! ```text
//! Initialize → Plan → Execute → Monitor → Reflect → Iterate
//!                                                        │
//!                                  ┌─────────────────────┘
//!                                  ▼
//!                       iteration < max?
//!                         │       │
//!                        Yes      No
//!                         │       │
//!                         ▼       ▼
//!                       Plan   Complete
//! ```
//!
//! - `enable_monitoring = false` 时跳过 Monitor（Execute → Reflect / Iterate）
//! - `enable_reflection = false` 时跳过 Reflect（Monitor / Execute → Iterate）
//! - Paused 可通过 resume 恢复到之前的阶段
//! - Failed / Complete / Cancelled 为终态
//!
//! ## 线程安全
//!
//! [`PhaseRing`] 内部通过 `parking_lot::RwLock` 保护状态，所有 mutating
//! 方法均接受 `&self`，可安全跨线程共享（`Send` + `Sync`）。
//!
//! ## Feature Gate
//!
//! 与 `loop_def.rs` 一致，由 `master-orchestrator` feature 门控。

#![cfg(feature = "master-orchestrator")]

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// 默认值函数（供 #[serde(default = "...")] 引用）
// ---------------------------------------------------------------------------

/// `max_history` 默认值 — 保留最近 100 条阶段历史。
fn default_max_history() -> usize {
    100
}
/// `auto_advance` 默认值 — 自动推进阶段。
fn default_auto_advance() -> bool {
    true
}
/// `enable_monitoring` 默认值 — 启用监控阶段。
fn default_enable_monitoring() -> bool {
    true
}
/// `enable_reflection` 默认值 — 启用反思阶段。
fn default_enable_reflection() -> bool {
    true
}
/// `max_iterations` 默认值 — 最多迭代 10 次。
fn default_max_iterations() -> u32 {
    10
}
/// `pause_on_failure` 默认值 — 失败时不暂停（直接进入 Failed 终态）。
fn default_pause_on_failure() -> bool {
    false
}

// ---------------------------------------------------------------------------
// LoopPhase — Loop 运行时阶段枚举
// ---------------------------------------------------------------------------

/// Loop 运行时阶段枚举 — 描述 Loop 执行生命周期中的当前阶段。
///
/// 与 SQL/JSON 中的字面量严格对齐（`#[serde(rename_all = "snake_case")]`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopPhase {
    /// 初始化 — Loop 启动后的第一个阶段。
    Initialize,
    /// 规划 — 制定执行计划。
    Plan,
    /// 执行 — 执行计划中的动作。
    Execute,
    /// 监控 — 监控执行结果（可通过 config 跳过）。
    Monitor,
    /// 反思 — 反思执行过程并调整策略（可通过 config 跳过）。
    Reflect,
    /// 迭代 — 判断是否需要下一轮循环。
    Iterate,
    /// 完成 — Loop 成功结束（终态）。
    Complete,
    /// 失败 — Loop 执行失败（终态）。
    Failed,
    /// 暂停 — Loop 被暂停（可通过 resume 恢复）。
    Paused,
    /// 取消 — Loop 被取消（终态）。
    Cancelled,
}

impl LoopPhase {
    /// 返回阶段的 snake_case 字符串表示（与 serde 序列化一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            LoopPhase::Initialize => "initialize",
            LoopPhase::Plan => "plan",
            LoopPhase::Execute => "execute",
            LoopPhase::Monitor => "monitor",
            LoopPhase::Reflect => "reflect",
            LoopPhase::Iterate => "iterate",
            LoopPhase::Complete => "complete",
            LoopPhase::Failed => "failed",
            LoopPhase::Paused => "paused",
            LoopPhase::Cancelled => "cancelled",
        }
    }

    /// 返回人类可读标签（首字母大写，供可视化显示）。
    pub fn label(self) -> &'static str {
        match self {
            LoopPhase::Initialize => "Initialize",
            LoopPhase::Plan => "Plan",
            LoopPhase::Execute => "Execute",
            LoopPhase::Monitor => "Monitor",
            LoopPhase::Reflect => "Reflect",
            LoopPhase::Iterate => "Iterate",
            LoopPhase::Complete => "Complete",
            LoopPhase::Failed => "Failed",
            LoopPhase::Paused => "Paused",
            LoopPhase::Cancelled => "Cancelled",
        }
    }

    /// 返回阶段对应的可视化颜色（十六进制 RGB）。
    pub fn color(self) -> &'static str {
        match self {
            LoopPhase::Initialize => "#4A90D9", // 蓝色 — 初始化
            LoopPhase::Plan => "#9B59B6",       // 紫色 — 规划
            LoopPhase::Execute => "#E67E22",    // 橙色 — 执行
            LoopPhase::Monitor => "#1ABC9C",    // 青色 — 监控
            LoopPhase::Reflect => "#F1C40F",    // 黄色 — 反思
            LoopPhase::Iterate => "#3498DB",    // 靛蓝 — 迭代
            LoopPhase::Complete => "#2ECC71",   // 绿色 — 完成
            LoopPhase::Failed => "#E74C3C",     // 红色 — 失败
            LoopPhase::Paused => "#95A5A6",     // 灰色 — 暂停
            LoopPhase::Cancelled => "#7F8C8D",  // 深灰 — 取消
        }
    }

    /// 是否为终态（Complete / Failed / Cancelled）。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            LoopPhase::Complete | LoopPhase::Failed | LoopPhase::Cancelled
        )
    }
}

impl std::fmt::Display for LoopPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// PhaseTransition — 阶段转换记录
// ---------------------------------------------------------------------------

/// 单次阶段转换记录 — 描述从一个阶段到另一个阶段的完整信息。
///
/// 由 [`PhaseRing::advance`] / [`PhaseRing::pause`] 等方法生成，追加到
/// 阶段历史中，用于审计追溯和可视化回放。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseTransition {
    /// 转换前的阶段。
    pub from_phase: LoopPhase,
    /// 转换后的阶段。
    pub to_phase: LoopPhase,
    /// 转换发生的时间戳（UTC）。
    pub timestamp: DateTime<Utc>,
    /// 触发转换的操作名称（如 "advance" / "pause" / "fail"）。
    pub trigger: String,
    /// 额外元数据（如失败原因、暂停标记等）。
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// 在前一阶段停留的毫秒数。
    #[serde(default)]
    pub duration_in_previous_ms: u64,
}

// ---------------------------------------------------------------------------
// PhaseRingConfig — 阶段环配置
// ---------------------------------------------------------------------------

/// 阶段环配置 — 控制 [`PhaseRing`] 的行为参数。
///
/// 通过 builder 方法链式构造：
/// ```rust,ignore
/// let config = PhaseRingConfig::default()
///     .with_max_iterations(5)
///     .with_enable_monitoring(false);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRingConfig {
    /// 保留的最大阶段历史条数（超出后丢弃最早的）。
    #[serde(default = "default_max_history")]
    pub max_history: usize,
    /// 是否自动推进阶段（外部自动化循环可检查此标志）。
    #[serde(default = "default_auto_advance")]
    pub auto_advance: bool,
    /// 是否启用 Monitor 阶段（false 时 Execute → Reflect / Iterate）。
    #[serde(default = "default_enable_monitoring")]
    pub enable_monitoring: bool,
    /// 是否启用 Reflect 阶段（false 时 Monitor / Execute → Iterate）。
    #[serde(default = "default_enable_reflection")]
    pub enable_reflection: bool,
    /// 最大迭代次数（达到后从 Iterate → Complete）。
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// 失败时暂停（true 时 fail() 转入 Paused 而非 Failed）。
    #[serde(default = "default_pause_on_failure")]
    pub pause_on_failure: bool,
}

impl Default for PhaseRingConfig {
    fn default() -> Self {
        Self {
            max_history: default_max_history(),
            auto_advance: default_auto_advance(),
            enable_monitoring: default_enable_monitoring(),
            enable_reflection: default_enable_reflection(),
            max_iterations: default_max_iterations(),
            pause_on_failure: default_pause_on_failure(),
        }
    }
}

impl PhaseRingConfig {
    /// Builder: 设置最大历史条数。
    pub fn with_max_history(mut self, max_history: usize) -> Self {
        self.max_history = max_history;
        self
    }

    /// Builder: 设置是否自动推进。
    pub fn with_auto_advance(mut self, auto_advance: bool) -> Self {
        self.auto_advance = auto_advance;
        self
    }

    /// Builder: 设置是否启用监控阶段。
    pub fn with_enable_monitoring(mut self, enable_monitoring: bool) -> Self {
        self.enable_monitoring = enable_monitoring;
        self
    }

    /// Builder: 设置是否启用反思阶段。
    pub fn with_enable_reflection(mut self, enable_reflection: bool) -> Self {
        self.enable_reflection = enable_reflection;
        self
    }

    /// Builder: 设置最大迭代次数。
    pub fn with_max_iterations(mut self, max_iterations: u32) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    /// Builder: 设置失败时是否暂停。
    pub fn with_pause_on_failure(mut self, pause_on_failure: bool) -> Self {
        self.pause_on_failure = pause_on_failure;
        self
    }
}

// ---------------------------------------------------------------------------
// PhaseRingSnapshot — 阶段环快照
// ---------------------------------------------------------------------------

/// 阶段环快照 — 某一时刻 [`PhaseRing`] 的完整状态投影。
///
/// 由 [`PhaseRing::to_snapshot`] 生成，可序列化后发送给前端渲染。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRingSnapshot {
    /// 当前阶段。
    pub current_phase: LoopPhase,
    /// 当前迭代次数。
    #[serde(default)]
    pub iteration: u32,
    /// 阶段转换历史（按时间顺序，最旧的在前）。
    #[serde(default)]
    pub transitions: Vec<PhaseTransition>,
    /// Loop 开始时间（UTC）。
    pub started_at: DateTime<Utc>,
    /// Loop 结束时间（UTC，终态时为 Some）。
    #[serde(default)]
    pub ended_at: Option<DateTime<Utc>>,
    /// 是否处于终态。
    #[serde(default)]
    pub is_terminal: bool,
    /// 阶段环配置。
    pub config: PhaseRingConfig,
}

// ---------------------------------------------------------------------------
// PhaseRing — 阶段环核心
// ---------------------------------------------------------------------------

/// 阶段环内部可变状态（由 `RwLock` 保护）。
#[derive(Debug, Clone)]
struct PhaseRingInner {
    /// 当前阶段。
    current_phase: LoopPhase,
    /// 暂停前的阶段（用于 resume 恢复）。
    previous_phase: Option<LoopPhase>,
    /// 当前迭代次数（每次 Iterate → Plan 递增）。
    iteration: u32,
    /// 阶段转换历史。
    transitions: Vec<PhaseTransition>,
    /// Loop 开始时间。
    started_at: DateTime<Utc>,
    /// Loop 结束时间（终态时为 Some）。
    ended_at: Option<DateTime<Utc>>,
    /// 当前阶段进入时间（用于计算阶段持续时长）。
    current_phase_entered_at: DateTime<Utc>,
    /// 各阶段累积持续时长。
    phase_durations: HashMap<LoopPhase, Duration>,
}

/// Loop 运行时阶段环 — 管理阶段推进与状态追踪的核心结构。
///
/// 内部通过 `parking_lot::RwLock` 保护状态，所有方法接受 `&self`，
/// 可安全跨线程共享。
///
/// ## 使用方式
///
/// ```rust,ignore
/// let ring = PhaseRing::new(PhaseRingConfig::default());
/// assert_eq!(ring.current_phase(), LoopPhase::Initialize);
/// ring.advance()?; // → Plan
/// ring.advance()?; // → Execute
/// ring.pause()?;   // → Paused
/// ring.resume()?;  // → Execute (恢复)
/// ```
pub struct PhaseRing {
    /// 内部可变状态。
    inner: RwLock<PhaseRingInner>,
    /// 不可变配置。
    config: PhaseRingConfig,
}

impl PhaseRing {
    /// 构造一个新的阶段环，初始阶段为 `Initialize`，迭代次数为 0。
    pub fn new(config: PhaseRingConfig) -> Self {
        let now = Utc::now();
        let mut phase_durations = HashMap::new();
        phase_durations.insert(LoopPhase::Initialize, Duration::ZERO);

        Self {
            inner: RwLock::new(PhaseRingInner {
                current_phase: LoopPhase::Initialize,
                previous_phase: None,
                iteration: 0,
                transitions: Vec::new(),
                started_at: now,
                ended_at: None,
                current_phase_entered_at: now,
                phase_durations,
            }),
            config,
        }
    }

    /// 返回当前阶段。
    pub fn current_phase(&self) -> LoopPhase {
        self.inner.read().current_phase
    }

    /// 是否处于终态（Complete / Failed / Cancelled）。
    pub fn is_terminal(&self) -> bool {
        self.inner.read().current_phase.is_terminal()
    }

    /// 是否可以推进（非终态且非 Paused）。
    pub fn can_advance(&self) -> bool {
        let inner = self.inner.read();
        !inner.current_phase.is_terminal() && inner.current_phase != LoopPhase::Paused
    }

    /// 返回当前迭代次数。
    pub fn iteration_count(&self) -> u32 {
        self.inner.read().iteration
    }

    /// 返回下一预期阶段（终态和 Paused 返回 None）。
    pub fn next_expected_phase(&self) -> Option<LoopPhase> {
        let inner = self.inner.read();
        next_expected_phase_inner(&inner, &self.config)
    }

    /// 推进到下一阶段（按阶段推进顺序自动计算）。
    ///
    /// 从 Iterate 推进到 Plan 时，迭代次数递增。
    pub fn advance(&self) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if !can_advance_inner(&inner) {
            bail!(
                "cannot advance from current phase: {:?}",
                inner.current_phase
            );
        }
        let next = next_expected_phase_inner(&inner, &self.config)
            .ok_or_else(|| anyhow!("no next expected phase from {:?}", inner.current_phase))?;

        // 从 Iterate 推进到 Plan 时递增迭代计数
        if inner.current_phase == LoopPhase::Iterate && next == LoopPhase::Plan {
            inner.iteration += 1;
        }

        let transition =
            Self::record_transition(&mut inner, next, "advance", HashMap::new(), &self.config);
        debug!(
            target: "nebula.loop.phase_ring",
            from = ?transition.from_phase,
            to = ?transition.to_phase,
            iteration = inner.iteration,
            "phase advanced"
        );
        Ok(transition)
    }

    /// 跳转到指定阶段（不按顺序，显式跳转）。
    ///
    /// 从终态调用会返回错误。从 Paused 跳转时清除 previous_phase。
    /// 跳转到 Paused 时保存当前阶段为 previous_phase（与 pause 行为一致）。
    pub fn advance_to(&self, phase: LoopPhase) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if inner.current_phase.is_terminal() {
            bail!(
                "cannot advance_to from terminal phase: {:?}",
                inner.current_phase
            );
        }

        // 跳转到 Paused 时保存当前阶段
        if phase == LoopPhase::Paused && inner.current_phase != LoopPhase::Paused {
            inner.previous_phase = Some(inner.current_phase);
        }
        // 从 Paused 跳转到其他阶段时清除 previous_phase
        if inner.current_phase == LoopPhase::Paused && phase != LoopPhase::Paused {
            inner.previous_phase = None;
        }

        let transition = Self::record_transition(
            &mut inner,
            phase,
            "advance_to",
            HashMap::new(),
            &self.config,
        );
        debug!(
            target: "nebula.loop.phase_ring",
            from = ?transition.from_phase,
            to = ?transition.to_phase,
            "phase advanced_to"
        );
        Ok(transition)
    }

    /// 暂停 Loop（当前阶段 → Paused，保存 previous_phase 供 resume 恢复）。
    pub fn pause(&self) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if inner.current_phase.is_terminal() {
            bail!(
                "cannot pause from terminal phase: {:?}",
                inner.current_phase
            );
        }
        if inner.current_phase == LoopPhase::Paused {
            bail!("already paused");
        }
        inner.previous_phase = Some(inner.current_phase);
        let transition = Self::record_transition(
            &mut inner,
            LoopPhase::Paused,
            "pause",
            HashMap::new(),
            &self.config,
        );
        debug!(
            target: "nebula.loop.phase_ring",
            from = ?transition.from_phase,
            "phase paused"
        );
        Ok(transition)
    }

    /// 恢复 Loop（从 Paused → 之前的阶段）。
    pub fn resume(&self) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if inner.current_phase != LoopPhase::Paused {
            bail!(
                "not paused, cannot resume (current: {:?})",
                inner.current_phase
            );
        }
        let target = inner
            .previous_phase
            .ok_or_else(|| anyhow!("no previous phase to resume to"))?;
        inner.previous_phase = None;
        let transition =
            Self::record_transition(&mut inner, target, "resume", HashMap::new(), &self.config);
        debug!(
            target: "nebula.loop.phase_ring",
            to = ?transition.to_phase,
            "phase resumed"
        );
        Ok(transition)
    }

    /// 标记 Loop 失败。
    ///
    /// 当 `config.pause_on_failure = true` 时转入 Paused（非终态，可 resume 恢复）；
    /// 否则转入 Failed 终态。
    pub fn fail(&self, reason: &str) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if inner.current_phase.is_terminal() {
            bail!("already in terminal phase: {:?}", inner.current_phase);
        }

        let mut metadata = HashMap::new();
        metadata.insert("reason".to_string(), reason.to_string());

        if self.config.pause_on_failure {
            // 失败时暂停：保存当前阶段以便 resume 恢复
            metadata.insert("paused_on_failure".to_string(), "true".to_string());
            if inner.current_phase != LoopPhase::Paused {
                inner.previous_phase = Some(inner.current_phase);
            }
            let transition = Self::record_transition(
                &mut inner,
                LoopPhase::Paused,
                "fail",
                metadata,
                &self.config,
            );
            debug!(
                target: "nebula.loop.phase_ring",
                from = ?transition.from_phase,
                reason = reason,
                "phase failed (paused on failure)"
            );
            Ok(transition)
        } else {
            let transition = Self::record_transition(
                &mut inner,
                LoopPhase::Failed,
                "fail",
                metadata,
                &self.config,
            );
            debug!(
                target: "nebula.loop.phase_ring",
                from = ?transition.from_phase,
                reason = reason,
                "phase failed"
            );
            Ok(transition)
        }
    }

    /// 取消 Loop（当前阶段 → Cancelled 终态）。
    pub fn cancel(&self) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if inner.current_phase.is_terminal() {
            bail!("already in terminal phase: {:?}", inner.current_phase);
        }
        let transition = Self::record_transition(
            &mut inner,
            LoopPhase::Cancelled,
            "cancel",
            HashMap::new(),
            &self.config,
        );
        debug!(
            target: "nebula.loop.phase_ring",
            from = ?transition.from_phase,
            "phase cancelled"
        );
        Ok(transition)
    }

    /// 完成 Loop（当前阶段 → Complete 终态）。
    pub fn complete(&self) -> Result<PhaseTransition> {
        let mut inner = self.inner.write();
        if inner.current_phase.is_terminal() {
            bail!("already in terminal phase: {:?}", inner.current_phase);
        }
        let transition = Self::record_transition(
            &mut inner,
            LoopPhase::Complete,
            "complete",
            HashMap::new(),
            &self.config,
        );
        debug!(
            target: "nebula.loop.phase_ring",
            from = ?transition.from_phase,
            "phase completed"
        );
        Ok(transition)
    }

    /// 返回阶段转换历史的克隆（按时间顺序，最旧的在前）。
    pub fn history(&self) -> Vec<PhaseTransition> {
        self.inner.read().transitions.clone()
    }

    /// 返回指定阶段的累积持续时长（含当前进行中的时长）。
    ///
    /// 从未进入过的阶段返回 `None`。
    pub fn phase_duration(&self, phase: &LoopPhase) -> Option<Duration> {
        let inner = self.inner.read();
        let accumulated = inner
            .phase_durations
            .get(phase)
            .copied()
            .unwrap_or(Duration::ZERO);

        // 如果查询的是当前阶段，加上正在进行的时长
        if inner.current_phase == *phase {
            let now = Utc::now();
            let ongoing_ms = (now - inner.current_phase_entered_at).num_millis().max(0) as u64;
            let total = accumulated + Duration::from_millis(ongoing_ms);
            return Some(total);
        }

        // 非当前阶段：仅当曾在 phase_durations 中记录过时返回 Some
        if inner.phase_durations.contains_key(phase) {
            Some(accumulated)
        } else {
            None
        }
    }

    /// 返回 Loop 的总持续时长（从 started_at 到 now 或 ended_at）。
    pub fn total_duration(&self) -> Duration {
        let inner = self.inner.read();
        let end = inner.ended_at.unwrap_or_else(Utc::now);
        let ms = (end - inner.started_at).num_millis().max(0) as u64;
        Duration::from_millis(ms)
    }

    /// 生成当前状态的快照。
    pub fn to_snapshot(&self) -> PhaseRingSnapshot {
        let inner = self.inner.read();
        PhaseRingSnapshot {
            current_phase: inner.current_phase,
            iteration: inner.iteration,
            transitions: inner.transitions.clone(),
            started_at: inner.started_at,
            ended_at: inner.ended_at,
            is_terminal: inner.current_phase.is_terminal(),
            config: self.config.clone(),
        }
    }

    /// 重置到初始状态（Initialize, iteration = 0, 清空历史）。
    pub fn reset(&self) {
        let mut inner = self.inner.write();
        let now = Utc::now();
        let mut phase_durations = HashMap::new();
        phase_durations.insert(LoopPhase::Initialize, Duration::ZERO);
        *inner = PhaseRingInner {
            current_phase: LoopPhase::Initialize,
            previous_phase: None,
            iteration: 0,
            transitions: Vec::new(),
            started_at: now,
            ended_at: None,
            current_phase_entered_at: now,
            phase_durations,
        };
        debug!(
            target: "nebula.loop.phase_ring",
            "phase ring reset to Initialize"
        );
    }

    // ---- 内部辅助 ----

    /// 记录一次阶段转换（计算时长、更新历史、更新当前阶段）。
    fn record_transition(
        inner: &mut PhaseRingInner,
        to: LoopPhase,
        trigger: &str,
        metadata: HashMap<String, String>,
        config: &PhaseRingConfig,
    ) -> PhaseTransition {
        let now = Utc::now();
        let from = inner.current_phase;

        // 计算前一阶段的持续时长
        let duration_in_previous_ms =
            (now - inner.current_phase_entered_at).num_millis().max(0) as u64;

        // 累加到 phase_durations
        *inner.phase_durations.entry(from).or_insert(Duration::ZERO) +=
            Duration::from_millis(duration_in_previous_ms);

        // 创建转换记录
        let transition = PhaseTransition {
            from_phase: from,
            to_phase: to,
            timestamp: now,
            trigger: trigger.to_string(),
            metadata,
            duration_in_previous_ms,
        };

        // 推入历史（遵守 max_history 上限）
        inner.transitions.push(transition.clone());
        if inner.transitions.len() > config.max_history {
            let excess = inner.transitions.len() - config.max_history;
            inner.transitions.drain(0..excess);
        }

        // 更新当前阶段
        inner.current_phase = to;
        inner.current_phase_entered_at = now;

        // 终态设置 ended_at
        if to.is_terminal() {
            inner.ended_at = Some(now);
        }

        transition
    }
}

// ---------------------------------------------------------------------------
// 内部辅助函数
// ---------------------------------------------------------------------------

/// 判断当前阶段是否可以推进（非终态且非 Paused）。
fn can_advance_inner(inner: &PhaseRingInner) -> bool {
    !inner.current_phase.is_terminal() && inner.current_phase != LoopPhase::Paused
}

/// 计算下一预期阶段。
fn next_expected_phase_inner(
    inner: &PhaseRingInner,
    config: &PhaseRingConfig,
) -> Option<LoopPhase> {
    match inner.current_phase {
        LoopPhase::Initialize => Some(LoopPhase::Plan),
        LoopPhase::Plan => Some(LoopPhase::Execute),
        LoopPhase::Execute => {
            if config.enable_monitoring {
                Some(LoopPhase::Monitor)
            } else if config.enable_reflection {
                Some(LoopPhase::Reflect)
            } else {
                Some(LoopPhase::Iterate)
            }
        }
        LoopPhase::Monitor => {
            if config.enable_reflection {
                Some(LoopPhase::Reflect)
            } else {
                Some(LoopPhase::Iterate)
            }
        }
        LoopPhase::Reflect => Some(LoopPhase::Iterate),
        LoopPhase::Iterate => {
            if inner.iteration < config.max_iterations {
                Some(LoopPhase::Plan)
            } else {
                Some(LoopPhase::Complete)
            }
        }
        // 终态和 Paused 无下一预期阶段
        LoopPhase::Complete | LoopPhase::Failed | LoopPhase::Paused | LoopPhase::Cancelled => None,
    }
}

// ---------------------------------------------------------------------------
// PhaseNodeVisual — 阶段节点可视化数据
// ---------------------------------------------------------------------------

/// 阶段节点可视化数据 — 描述画布上单个阶段节点的位置和显示属性。
///
/// 由 [`PhaseRingVisualizer`] 生成，供前端画布组件渲染阶段环。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseNodeVisual {
    /// 阶段枚举值。
    pub phase: LoopPhase,
    /// 人类可读标签。
    pub label: String,
    /// X 坐标。
    pub x: f64,
    /// Y 坐标。
    pub y: f64,
    /// 是否为当前阶段。
    #[serde(default)]
    pub is_current: bool,
    /// 是否已完成（已过渡离开）。
    #[serde(default)]
    pub is_completed: bool,
    /// 是否为终态阶段。
    #[serde(default)]
    pub is_terminal: bool,
    /// 可视化颜色（十六进制 RGB）。
    pub color: String,
}

// ---------------------------------------------------------------------------
// PhaseRingVisualizer — 阶段环可视化数据生成
// ---------------------------------------------------------------------------

/// 阶段环可视化数据生成器 — 将 [`PhaseRing`] 状态转换为前端可渲染的节点布局。
///
/// 支持两种布局：
/// - **环形布局**（[`to_circular_layout`](Self::to_circular_layout)）：节点均匀分布在圆周上
/// - **线性布局**（[`to_linear_layout`](Self::to_linear_layout)）：节点水平排列
///
/// 无内部状态，可安全共享。
pub struct PhaseRingVisualizer;

impl PhaseRingVisualizer {
    /// 构造一个新的可视化数据生成器。
    pub fn new() -> Self {
        Self
    }

    /// 根据配置返回要显示的阶段列表（主循环 + Complete）。
    fn phases_for_layout(&self, config: &PhaseRingConfig) -> Vec<LoopPhase> {
        let mut phases = vec![LoopPhase::Initialize, LoopPhase::Plan, LoopPhase::Execute];
        if config.enable_monitoring {
            phases.push(LoopPhase::Monitor);
        }
        if config.enable_reflection {
            phases.push(LoopPhase::Reflect);
        }
        phases.push(LoopPhase::Iterate);
        phases.push(LoopPhase::Complete);
        phases
    }

    /// 生成环形布局 — 节点均匀分布在指定圆心和半径的圆周上。
    pub fn to_circular_layout(
        &self,
        ring: &PhaseRing,
        radius: f64,
        center: (f64, f64),
    ) -> Vec<PhaseNodeVisual> {
        let snapshot = ring.to_snapshot();
        let phases = self.phases_for_layout(&snapshot.config);
        let n = phases.len();
        let current = snapshot.current_phase;

        // 已完成的阶段（在历史中作为 from_phase 出现过的阶段）
        let is_completed = |phase: LoopPhase| -> bool {
            snapshot.transitions.iter().any(|t| t.from_phase == phase) && phase != current
        };

        let mut nodes: Vec<PhaseNodeVisual> = phases
            .iter()
            .enumerate()
            .map(|(i, &phase)| {
                let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
                PhaseNodeVisual {
                    phase,
                    label: phase.label().to_string(),
                    x: center.0 + radius * angle.cos(),
                    y: center.1 + radius * angle.sin(),
                    is_current: false, // 由 highlight_current 设置
                    is_completed: is_completed(phase),
                    is_terminal: phase.is_terminal(),
                    color: phase.color().to_string(),
                }
            })
            .collect();

        self.highlight_current(&mut nodes, &current);
        nodes
    }

    /// 生成线性布局 — 节点水平排列在指定宽度内。
    pub fn to_linear_layout(&self, ring: &PhaseRing, width: f64) -> Vec<PhaseNodeVisual> {
        let snapshot = ring.to_snapshot();
        let phases = self.phases_for_layout(&snapshot.config);
        let n = phases.len();
        let current = snapshot.current_phase;

        let is_completed = |phase: LoopPhase| -> bool {
            snapshot.transitions.iter().any(|t| t.from_phase == phase) && phase != current
        };

        let mut nodes: Vec<PhaseNodeVisual> = phases
            .iter()
            .enumerate()
            .map(|(i, &phase)| {
                let x = if n > 1 {
                    width * (i as f64) / ((n - 1) as f64)
                } else {
                    width / 2.0
                };
                PhaseNodeVisual {
                    phase,
                    label: phase.label().to_string(),
                    x,
                    y: 0.0,
                    is_current: false,
                    is_completed: is_completed(phase),
                    is_terminal: phase.is_terminal(),
                    color: phase.color().to_string(),
                }
            })
            .collect();

        self.highlight_current(&mut nodes, &current);
        nodes
    }

    /// 高亮当前阶段 — 将匹配 `current` 的节点 `is_current` 设为 true，其余设为 false。
    pub fn highlight_current(&self, visual: &mut Vec<PhaseNodeVisual>, current: &LoopPhase) {
        for node in visual.iter_mut() {
            node.is_current = node.phase == *current;
        }
    }
}

impl Default for PhaseRingVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // LoopPhase 序列化测试
    // ===================================================================

    /// 测试 `LoopPhase` 所有变体的序列化/反序列化（snake_case）。
    #[test]
    fn test_loop_phase_serde_variants() {
        let cases = [
            (LoopPhase::Initialize, "initialize"),
            (LoopPhase::Plan, "plan"),
            (LoopPhase::Execute, "execute"),
            (LoopPhase::Monitor, "monitor"),
            (LoopPhase::Reflect, "reflect"),
            (LoopPhase::Iterate, "iterate"),
            (LoopPhase::Complete, "complete"),
            (LoopPhase::Failed, "failed"),
            (LoopPhase::Paused, "paused"),
            (LoopPhase::Cancelled, "cancelled"),
        ];
        for (phase, expected) in cases {
            let json = serde_json::to_string(&phase).expect("serialize should succeed");
            assert_eq!(json, format!("\"{expected}\""));
            let back: LoopPhase = serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(back, phase);
        }
    }

    /// 测试 `LoopPhase::as_str` / `Display` / `is_terminal`。
    #[test]
    fn test_loop_phase_methods() {
        assert_eq!(LoopPhase::Initialize.as_str(), "initialize");
        assert_eq!(format!("{}", LoopPhase::Plan), "plan");
        assert!(LoopPhase::Complete.is_terminal());
        assert!(LoopPhase::Failed.is_terminal());
        assert!(LoopPhase::Cancelled.is_terminal());
        assert!(!LoopPhase::Initialize.is_terminal());
        assert!(!LoopPhase::Paused.is_terminal());
        assert!(!LoopPhase::Iterate.is_terminal());
    }

    // ===================================================================
    // PhaseTransition 序列化测试
    // ===================================================================

    /// 测试 `PhaseTransition` 的序列化/反序列化 round-trip。
    #[test]
    fn test_phase_transition_serde() {
        let now = Utc::now();
        let mut metadata = HashMap::new();
        metadata.insert("reason".to_string(), "test".to_string());

        let transition = PhaseTransition {
            from_phase: LoopPhase::Execute,
            to_phase: LoopPhase::Paused,
            timestamp: now,
            trigger: "pause".to_string(),
            metadata,
            duration_in_previous_ms: 1500,
        };

        let json = serde_json::to_string(&transition).expect("serialize should succeed");
        let back: PhaseTransition =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(back.from_phase, LoopPhase::Execute);
        assert_eq!(back.to_phase, LoopPhase::Paused);
        assert_eq!(back.trigger, "pause");
        assert_eq!(back.duration_in_previous_ms, 1500);
        assert_eq!(back.metadata.get("reason"), Some(&"test".to_string()));
    }

    // ===================================================================
    // PhaseRingConfig 默认值 / builder 测试
    // ===================================================================

    /// 测试 `PhaseRingConfig` 默认值。
    #[test]
    fn test_phase_ring_config_default() {
        let config = PhaseRingConfig::default();
        assert_eq!(config.max_history, 100);
        assert!(config.auto_advance);
        assert!(config.enable_monitoring);
        assert!(config.enable_reflection);
        assert_eq!(config.max_iterations, 10);
        assert!(!config.pause_on_failure);
    }

    /// 测试 `PhaseRingConfig` builder 链式调用。
    #[test]
    fn test_phase_ring_config_builder() {
        let config = PhaseRingConfig::default()
            .with_max_history(50)
            .with_auto_advance(false)
            .with_enable_monitoring(false)
            .with_enable_reflection(false)
            .with_max_iterations(3)
            .with_pause_on_failure(true);
        assert_eq!(config.max_history, 50);
        assert!(!config.auto_advance);
        assert!(!config.enable_monitoring);
        assert!(!config.enable_reflection);
        assert_eq!(config.max_iterations, 3);
        assert!(config.pause_on_failure);
    }

    // ===================================================================
    // PhaseRing new / current_phase 测试
    // ===================================================================

    /// 测试 `PhaseRing::new` 和 `current_phase`。
    #[test]
    fn test_phase_ring_new_and_current_phase() {
        let ring = PhaseRing::new(PhaseRingConfig::default());
        assert_eq!(ring.current_phase(), LoopPhase::Initialize);
        assert_eq!(ring.iteration_count(), 0);
        assert!(!ring.is_terminal());
        assert!(ring.can_advance());
        assert!(ring.history().is_empty());
    }

    // ===================================================================
    // advance 推进顺序测试
    // ===================================================================

    /// 测试 advance 推进顺序：Initialize → Plan → Execute → Monitor → Reflect → Iterate → Plan。
    #[test]
    fn test_advance_order() {
        let ring = PhaseRing::new(PhaseRingConfig::default());

        // Initialize → Plan
        let t1 = ring.advance().unwrap();
        assert_eq!(t1.from_phase, LoopPhase::Initialize);
        assert_eq!(t1.to_phase, LoopPhase::Plan);
        assert_eq!(ring.current_phase(), LoopPhase::Plan);

        // Plan → Execute
        let t2 = ring.advance().unwrap();
        assert_eq!(t2.from_phase, LoopPhase::Plan);
        assert_eq!(t2.to_phase, LoopPhase::Execute);

        // Execute → Monitor
        let t3 = ring.advance().unwrap();
        assert_eq!(t3.from_phase, LoopPhase::Execute);
        assert_eq!(t3.to_phase, LoopPhase::Monitor);

        // Monitor → Reflect
        let t4 = ring.advance().unwrap();
        assert_eq!(t4.from_phase, LoopPhase::Monitor);
        assert_eq!(t4.to_phase, LoopPhase::Reflect);

        // Reflect → Iterate
        let t5 = ring.advance().unwrap();
        assert_eq!(t5.from_phase, LoopPhase::Reflect);
        assert_eq!(t5.to_phase, LoopPhase::Iterate);

        // Iterate → Plan（iteration 递增）
        let t6 = ring.advance().unwrap();
        assert_eq!(t6.from_phase, LoopPhase::Iterate);
        assert_eq!(t6.to_phase, LoopPhase::Plan);
        assert_eq!(ring.iteration_count(), 1);
    }

    // ===================================================================
    // advance_to 跳转测试
    // ===================================================================

    /// 测试 `advance_to` 显式跳转。
    #[test]
    fn test_advance_to() {
        let ring = PhaseRing::new(PhaseRingConfig::default());

        // 从 Initialize 跳转到 Execute
        let t = ring.advance_to(LoopPhase::Execute).unwrap();
        assert_eq!(t.from_phase, LoopPhase::Initialize);
        assert_eq!(t.to_phase, LoopPhase::Execute);
        assert_eq!(ring.current_phase(), LoopPhase::Execute);
        assert_eq!(t.trigger, "advance_to");

        // 从终态跳转应失败
        ring.advance_to(LoopPhase::Complete).unwrap();
        assert!(ring.is_terminal());
        assert!(ring.advance_to(LoopPhase::Plan).is_err());
    }

    // ===================================================================
    // pause / resume 测试
    // ===================================================================

    /// 测试 pause / resume 恢复到之前的阶段。
    #[test]
    fn test_pause_and_resume() {
        let ring = PhaseRing::new(PhaseRingConfig::default());
        ring.advance().unwrap(); // → Plan
        ring.advance().unwrap(); // → Execute

        // 暂停
        let pause_t = ring.pause().unwrap();
        assert_eq!(pause_t.from_phase, LoopPhase::Execute);
        assert_eq!(pause_t.to_phase, LoopPhase::Paused);
        assert_eq!(ring.current_phase(), LoopPhase::Paused);
        assert!(!ring.is_terminal());
        assert!(!ring.can_advance(), "Paused should not be advanceable");

        // 恢复
        let resume_t = ring.resume().unwrap();
        assert_eq!(resume_t.from_phase, LoopPhase::Paused);
        assert_eq!(resume_t.to_phase, LoopPhase::Execute);
        assert_eq!(ring.current_phase(), LoopPhase::Execute);
        assert!(ring.can_advance());

        // 非 Paused 状态 resume 应失败
        assert!(ring.resume().is_err());
    }

    // ===================================================================
    // fail / cancel / complete 终态测试
    // ===================================================================

    /// 测试 fail / cancel / complete 都进入终态。
    #[test]
    fn test_fail_cancel_complete_terminal() {
        // fail
        let ring1 = PhaseRing::new(PhaseRingConfig::default());
        let fail_t = ring1.fail("something went wrong").unwrap();
        assert_eq!(fail_t.to_phase, LoopPhase::Failed);
        assert_eq!(ring1.current_phase(), LoopPhase::Failed);
        assert!(ring1.is_terminal());
        assert!(!ring1.can_advance());
        assert!(ring1.to_snapshot().ended_at.is_some());

        // cancel
        let ring2 = PhaseRing::new(PhaseRingConfig::default());
        let cancel_t = ring2.cancel().unwrap();
        assert_eq!(cancel_t.to_phase, LoopPhase::Cancelled);
        assert!(ring2.is_terminal());

        // complete
        let ring3 = PhaseRing::new(PhaseRingConfig::default());
        let complete_t = ring3.complete().unwrap();
        assert_eq!(complete_t.to_phase, LoopPhase::Complete);
        assert!(ring3.is_terminal());

        // 终态后再次 fail/cancel/complete 应失败
        assert!(ring1.fail("again").is_err());
        assert!(ring2.cancel().is_err());
        assert!(ring3.complete().is_err());
    }

    // ===================================================================
    // is_terminal / can_advance 测试
    // ===================================================================

    /// 测试 `is_terminal` 和 `can_advance` 在不同阶段的行为。
    #[test]
    fn test_is_terminal_and_can_advance() {
        let ring = PhaseRing::new(PhaseRingConfig::default());

        // Initialize — 可推进，非终态
        assert!(!ring.is_terminal());
        assert!(ring.can_advance());

        // Paused — 不可推进，非终态
        ring.pause().unwrap();
        assert!(!ring.is_terminal());
        assert!(!ring.can_advance());

        // resume → Initialize
        ring.resume().unwrap();
        assert!(ring.can_advance());

        // Failed — 终态，不可推进
        ring.fail("err").unwrap();
        assert!(ring.is_terminal());
        assert!(!ring.can_advance());
    }

    // ===================================================================
    // next_expected_phase 各阶段测试
    // ===================================================================

    /// 测试 `next_expected_phase` 在各阶段的返回值。
    #[test]
    fn test_next_expected_phase() {
        let ring = PhaseRing::new(PhaseRingConfig::default());

        // Initialize → Plan
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Plan));
        ring.advance().unwrap(); // → Plan

        // Plan → Execute
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Execute));
        ring.advance().unwrap(); // → Execute

        // Execute → Monitor
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Monitor));
        ring.advance().unwrap(); // → Monitor

        // Monitor → Reflect
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Reflect));
        ring.advance().unwrap(); // → Reflect

        // Reflect → Iterate
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Iterate));
        ring.advance().unwrap(); // → Iterate

        // Iterate → Plan (iteration=0 < 10)
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Plan));

        // Paused → None
        ring.advance_to(LoopPhase::Paused).unwrap();
        assert_eq!(ring.next_expected_phase(), None);

        // 终态 → None
        ring.reset();
        ring.complete().unwrap();
        assert_eq!(ring.next_expected_phase(), None);
    }

    /// 测试 `enable_monitoring = false` 时 Execute → Reflect（跳过 Monitor）。
    #[test]
    fn test_next_expected_phase_skip_monitor() {
        let config = PhaseRingConfig::default().with_enable_monitoring(false);
        let ring = PhaseRing::new(config);
        ring.advance_to(LoopPhase::Execute).unwrap();
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Reflect));
    }

    /// 测试 `enable_reflection = false` 时 Monitor → Iterate（跳过 Reflect）。
    #[test]
    fn test_next_expected_phase_skip_reflect() {
        let config = PhaseRingConfig::default().with_enable_reflection(false);
        let ring = PhaseRing::new(config);
        ring.advance_to(LoopPhase::Monitor).unwrap();
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Iterate));
    }

    /// 测试 `enable_monitoring = false && enable_reflection = false` 时 Execute → Iterate。
    #[test]
    fn test_next_expected_phase_skip_both() {
        let config = PhaseRingConfig::default()
            .with_enable_monitoring(false)
            .with_enable_reflection(false);
        let ring = PhaseRing::new(config);
        ring.advance_to(LoopPhase::Execute).unwrap();
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Iterate));
    }

    /// 测试 `max_iterations = 0` 时 Iterate → Complete。
    #[test]
    fn test_next_expected_phase_iterate_to_complete() {
        let config = PhaseRingConfig::default().with_max_iterations(0);
        let ring = PhaseRing::new(config);
        ring.advance_to(LoopPhase::Iterate).unwrap();
        assert_eq!(ring.next_expected_phase(), Some(LoopPhase::Complete));
    }

    // ===================================================================
    // iteration_count 递增测试
    // ===================================================================

    /// 测试迭代次数在 Iterate → Plan 时递增。
    #[test]
    fn test_iteration_count_increment() {
        let ring = PhaseRing::new(PhaseRingConfig::default().with_max_iterations(3));

        // 走完第一轮到 Iterate
        ring.advance().unwrap(); // → Plan
        ring.advance().unwrap(); // → Execute
        ring.advance().unwrap(); // → Monitor
        ring.advance().unwrap(); // → Reflect
        ring.advance().unwrap(); // → Iterate
        assert_eq!(ring.iteration_count(), 0);

        // Iterate → Plan, iteration 递增到 1
        ring.advance().unwrap();
        assert_eq!(ring.iteration_count(), 1);

        // 走完第二轮到 Iterate
        ring.advance().unwrap(); // → Execute
        ring.advance().unwrap(); // → Monitor
        ring.advance().unwrap(); // → Reflect
        ring.advance().unwrap(); // → Iterate
        assert_eq!(ring.iteration_count(), 1);

        // Iterate → Plan, iteration 递增到 2
        ring.advance().unwrap();
        assert_eq!(ring.iteration_count(), 2);
    }

    /// 测试达到 max_iterations 后从 Iterate → Complete。
    #[test]
    fn test_max_iterations_completes() {
        let ring = PhaseRing::new(PhaseRingConfig::default().with_max_iterations(1));

        // 第一轮: Initialize → ... → Iterate
        ring.advance().unwrap(); // → Plan
        ring.advance().unwrap(); // → Execute
        ring.advance().unwrap(); // → Monitor
        ring.advance().unwrap(); // → Reflect
        ring.advance().unwrap(); // → Iterate
        assert_eq!(ring.iteration_count(), 0);

        // Iterate → Plan (iteration = 1)
        ring.advance().unwrap();
        assert_eq!(ring.iteration_count(), 1);

        // 第二轮: Plan → ... → Iterate
        ring.advance().unwrap(); // → Execute
        ring.advance().unwrap(); // → Monitor
        ring.advance().unwrap(); // → Reflect
        ring.advance().unwrap(); // → Iterate
        assert_eq!(ring.iteration_count(), 1);

        // Iterate → Complete (1 < 1 is false)
        let t = ring.advance().unwrap();
        assert_eq!(t.to_phase, LoopPhase::Complete);
        assert!(ring.is_terminal());
    }

    // ===================================================================
    // history 记录测试
    // ===================================================================

    /// 测试阶段历史正确记录每次转换。
    #[test]
    fn test_history_records() {
        let ring = PhaseRing::new(PhaseRingConfig::default());

        // 初始历史为空
        assert!(ring.history().is_empty());

        // 推进两个阶段
        ring.advance().unwrap(); // Initialize → Plan
        ring.advance().unwrap(); // Plan → Execute

        let history = ring.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].from_phase, LoopPhase::Initialize);
        assert_eq!(history[0].to_phase, LoopPhase::Plan);
        assert_eq!(history[0].trigger, "advance");
        assert_eq!(history[1].from_phase, LoopPhase::Plan);
        assert_eq!(history[1].to_phase, LoopPhase::Execute);

        // 验证 timestamp 合理
        assert!(history[0].timestamp <= Utc::now());
        assert!(history[0].timestamp <= history[1].timestamp);
    }

    /// 测试 `max_history` 限制历史条数。
    #[test]
    fn test_history_max_limit() {
        let ring = PhaseRing::new(PhaseRingConfig::default().with_max_history(3));

        // 用 advance_to 产生 5 条历史
        ring.advance_to(LoopPhase::Plan).unwrap();
        ring.advance_to(LoopPhase::Execute).unwrap();
        ring.advance_to(LoopPhase::Monitor).unwrap();
        ring.advance_to(LoopPhase::Reflect).unwrap();
        ring.advance_to(LoopPhase::Iterate).unwrap();

        let history = ring.history();
        assert_eq!(
            history.len(),
            3,
            "history should be capped at max_history=3"
        );
        // 保留最新的 3 条
        assert_eq!(history[0].from_phase, LoopPhase::Execute);
        assert_eq!(history[2].to_phase, LoopPhase::Iterate);
    }

    // ===================================================================
    // reset 重置测试
    // ===================================================================

    /// 测试 reset 恢复到初始状态。
    #[test]
    fn test_reset() {
        let ring = PhaseRing::new(PhaseRingConfig::default().with_max_iterations(3));

        // 推进几个阶段
        ring.advance().unwrap(); // → Plan
        ring.advance().unwrap(); // → Execute
        ring.advance().unwrap(); // → Monitor
        assert_eq!(ring.history().len(), 3);

        // 重置
        ring.reset();
        assert_eq!(ring.current_phase(), LoopPhase::Initialize);
        assert_eq!(ring.iteration_count(), 0);
        assert!(ring.history().is_empty());
        assert!(!ring.is_terminal());
        assert!(ring.can_advance());

        // 重置后可正常推进
        ring.advance().unwrap();
        assert_eq!(ring.current_phase(), LoopPhase::Plan);
    }

    // ===================================================================
    // to_snapshot 序列化测试
    // ===================================================================

    /// 测试 `to_snapshot` 生成正确的快照且可序列化。
    #[test]
    fn test_snapshot_serde() {
        let ring = PhaseRing::new(PhaseRingConfig::default().with_max_iterations(3));
        ring.advance().unwrap(); // → Plan
        ring.advance().unwrap(); // → Execute

        let snapshot = ring.to_snapshot();
        assert_eq!(snapshot.current_phase, LoopPhase::Execute);
        assert_eq!(snapshot.iteration, 0);
        assert_eq!(snapshot.transitions.len(), 2);
        assert!(snapshot.started_at <= Utc::now());
        assert!(snapshot.ended_at.is_none());
        assert!(!snapshot.is_terminal);
        assert_eq!(snapshot.config.max_iterations, 3);

        // 序列化 round-trip
        let json = serde_json::to_string(&snapshot).expect("serialize should succeed");
        let de: PhaseRingSnapshot =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(de.current_phase, snapshot.current_phase);
        assert_eq!(de.iteration, snapshot.iteration);
        assert_eq!(de.transitions.len(), snapshot.transitions.len());
        assert_eq!(de.config.max_iterations, 3);
    }

    /// 测试终态快照包含 `ended_at` 和 `is_terminal`。
    #[test]
    fn test_snapshot_terminal() {
        let ring = PhaseRing::new(PhaseRingConfig::default());
        ring.fail("err").unwrap();
        let snapshot = ring.to_snapshot();
        assert!(snapshot.is_terminal);
        assert!(snapshot.ended_at.is_some());
    }

    // ===================================================================
    // phase_duration / total_duration 测试
    // ===================================================================

    /// 测试 `phase_duration` 和 `total_duration`。
    #[test]
    fn test_phase_duration_and_total_duration() {
        let ring = PhaseRing::new(PhaseRingConfig::default());

        // 初始: Initialize 阶段正在进行
        let init_dur = ring.phase_duration(&LoopPhase::Initialize);
        assert!(init_dur.is_some(), "Initialize duration should be Some");

        // Plan 阶段尚未进入
        let plan_dur = ring.phase_duration(&LoopPhase::Plan);
        assert!(
            plan_dur.is_none(),
            "Plan duration should be None before entering"
        );

        // 推进到 Plan
        ring.advance().unwrap();

        // Plan 阶段正在进行
        let plan_dur = ring.phase_duration(&LoopPhase::Plan);
        assert!(
            plan_dur.is_some(),
            "Plan duration should be Some after entering"
        );

        // Initialize 阶段已结束，仍有累积时长
        let init_dur = ring.phase_duration(&LoopPhase::Initialize);
        assert!(
            init_dur.is_some(),
            "Initialize duration should still be Some"
        );

        // Failed 阶段从未进入
        let failed_dur = ring.phase_duration(&LoopPhase::Failed);
        assert!(failed_dur.is_none(), "Failed duration should be None");

        // 总持续时间非负
        let total = ring.total_duration();
        assert!(total >= Duration::from_millis(0));
    }

    // ===================================================================
    // pause_on_failure 测试
    // ===================================================================

    /// 测试 `pause_on_failure = true` 时 fail 进入 Paused 而非 Failed。
    #[test]
    fn test_pause_on_failure() {
        let config = PhaseRingConfig::default().with_pause_on_failure(true);
        let ring = PhaseRing::new(config);
        ring.advance_to(LoopPhase::Execute).unwrap();

        // fail 应进入 Paused（非终态）
        let t = ring.fail("test error").unwrap();
        assert_eq!(t.to_phase, LoopPhase::Paused);
        assert_eq!(ring.current_phase(), LoopPhase::Paused);
        assert!(!ring.is_terminal(), "Paused should not be terminal");

        // metadata 应包含 reason 和 paused_on_failure
        assert_eq!(t.metadata.get("reason"), Some(&"test error".to_string()));
        assert_eq!(
            t.metadata.get("paused_on_failure"),
            Some(&"true".to_string())
        );

        // resume 恢复到 Execute
        let resume_t = ring.resume().unwrap();
        assert_eq!(resume_t.to_phase, LoopPhase::Execute);
        assert_eq!(ring.current_phase(), LoopPhase::Execute);
    }

    // ===================================================================
    // PhaseRingVisualizer 环形布局测试
    // ===================================================================

    /// 测试 `to_circular_layout` 生成正确的环形布局。
    #[test]
    fn test_visualizer_circular_layout() {
        let ring = PhaseRing::new(PhaseRingConfig::default());
        let viz = PhaseRingVisualizer::new();
        let nodes = viz.to_circular_layout(&ring, 100.0, (200.0, 200.0));

        // 默认配置: Initialize, Plan, Execute, Monitor, Reflect, Iterate, Complete = 7 个节点
        assert_eq!(nodes.len(), 7);

        // 当前阶段 Initialize 应被高亮
        let init = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Initialize)
            .expect("Initialize node should exist");
        assert!(init.is_current);

        // 其他阶段不应高亮
        let plan = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Plan)
            .expect("Plan node should exist");
        assert!(!plan.is_current);

        // 验证节点在圆上（距离圆心约等于半径）
        for node in &nodes {
            let dx = node.x - 200.0;
            let dy = node.y - 200.0;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                (dist - 100.0).abs() < 1e-6,
                "node {:?} should be on circle, got dist={}",
                node.phase,
                dist
            );
        }

        // 验证标签和颜色非空
        for node in &nodes {
            assert!(!node.label.is_empty());
            assert!(!node.color.is_empty());
        }

        // Complete 是终态
        let complete = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Complete)
            .expect("Complete node should exist");
        assert!(complete.is_terminal);
    }

    /// 测试 `enable_monitoring = false` 时环形布局跳过 Monitor。
    #[test]
    fn test_visualizer_circular_layout_skip_monitor() {
        let config = PhaseRingConfig::default()
            .with_enable_monitoring(false)
            .with_enable_reflection(false);
        let ring = PhaseRing::new(config);
        let viz = PhaseRingVisualizer::new();
        let nodes = viz.to_circular_layout(&ring, 100.0, (0.0, 0.0));

        // Initialize, Plan, Execute, Iterate, Complete = 5 个节点
        assert_eq!(nodes.len(), 5);
        assert!(!nodes.iter().any(|n| n.phase == LoopPhase::Monitor));
        assert!(!nodes.iter().any(|n| n.phase == LoopPhase::Reflect));
    }

    // ===================================================================
    // PhaseRingVisualizer 线性布局测试
    // ===================================================================

    /// 测试 `to_linear_layout` 生成正确的线性布局。
    #[test]
    fn test_visualizer_linear_layout() {
        let ring = PhaseRing::new(PhaseRingConfig::default());
        let viz = PhaseRingVisualizer::new();
        let nodes = viz.to_linear_layout(&ring, 700.0);

        assert_eq!(nodes.len(), 7);

        // 第一个节点 x=0，最后一个节点 x=700
        assert!((nodes[0].x - 0.0).abs() < 1e-6);
        assert!((nodes.last().unwrap().x - 700.0).abs() < 1e-6);

        // y 都为 0
        for node in &nodes {
            assert!((node.y - 0.0).abs() < 1e-6);
        }

        // 当前阶段 Initialize 高亮
        let init = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Initialize)
            .expect("Initialize should exist");
        assert!(init.is_current);

        // 节点 x 坐标单调递增
        for i in 1..nodes.len() {
            assert!(
                nodes[i].x >= nodes[i - 1].x,
                "x should be monotonically increasing"
            );
        }
    }

    // ===================================================================
    // highlight_current 测试
    // ===================================================================

    /// 测试 `highlight_current` 正确设置 `is_current` 标志。
    #[test]
    fn test_highlight_current() {
        let viz = PhaseRingVisualizer::new();
        let mut nodes = vec![
            PhaseNodeVisual {
                phase: LoopPhase::Initialize,
                label: "Initialize".to_string(),
                x: 0.0,
                y: 0.0,
                is_current: true,
                is_completed: false,
                is_terminal: false,
                color: "#4A90D9".to_string(),
            },
            PhaseNodeVisual {
                phase: LoopPhase::Plan,
                label: "Plan".to_string(),
                x: 100.0,
                y: 0.0,
                is_current: false,
                is_completed: false,
                is_terminal: false,
                color: "#9B59B6".to_string(),
            },
        ];

        // 高亮 Plan
        viz.highlight_current(&mut nodes, &LoopPhase::Plan);
        assert!(!nodes[0].is_current, "Initialize should not be current");
        assert!(nodes[1].is_current, "Plan should be current");

        // 高亮 Initialize
        viz.highlight_current(&mut nodes, &LoopPhase::Initialize);
        assert!(nodes[0].is_current, "Initialize should be current");
        assert!(!nodes[1].is_current, "Plan should not be current");
    }

    // ===================================================================
    // PhaseNodeVisual 结构测试
    // ===================================================================

    /// 测试 `PhaseNodeVisual` 结构体字段和序列化。
    #[test]
    fn test_phase_node_visual_structure() {
        let node = PhaseNodeVisual {
            phase: LoopPhase::Execute,
            label: "Execute".to_string(),
            x: 100.0,
            y: 200.0,
            is_current: true,
            is_completed: false,
            is_terminal: false,
            color: "#E67E22".to_string(),
        };

        assert_eq!(node.phase, LoopPhase::Execute);
        assert_eq!(node.label, "Execute");
        assert!((node.x - 100.0).abs() < f64::EPSILON);
        assert!((node.y - 200.0).abs() < f64::EPSILON);
        assert!(node.is_current);
        assert!(!node.is_completed);
        assert!(!node.is_terminal);
        assert_eq!(node.color, "#E67E22");

        // 序列化 round-trip
        let json = serde_json::to_string(&node).expect("serialize should succeed");
        let de: PhaseNodeVisual = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(de.phase, node.phase);
        assert_eq!(de.label, node.label);
        assert!((de.x - node.x).abs() < f64::EPSILON);
        assert!((de.y - node.y).abs() < f64::EPSILON);
        assert_eq!(de.is_current, node.is_current);
        assert_eq!(de.color, node.color);
    }

    // ===================================================================
    // PhaseRingVisualizer Default 测试
    // ===================================================================

    /// 测试 `PhaseRingVisualizer` 实现 `Default` trait。
    #[test]
    fn test_visualizer_default() {
        let viz = PhaseRingVisualizer::default();
        let ring = PhaseRing::new(PhaseRingConfig::default());
        let nodes = viz.to_circular_layout(&ring, 50.0, (0.0, 0.0));
        assert!(!nodes.is_empty());
    }

    // ===================================================================
    // 推进完整循环后 is_completed 标记测试
    // ===================================================================

    /// 测试推进几个阶段后，已完成阶段在可视化中标记为 is_completed。
    #[test]
    fn test_visualizer_completed_phases() {
        let ring = PhaseRing::new(PhaseRingConfig::default());
        ring.advance().unwrap(); // Initialize → Plan
        ring.advance().unwrap(); // Plan → Execute

        let viz = PhaseRingVisualizer::new();
        let nodes = viz.to_circular_layout(&ring, 100.0, (0.0, 0.0));

        // Initialize 已完成
        let init = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Initialize)
            .unwrap();
        assert!(init.is_completed, "Initialize should be completed");

        // Execute 是当前阶段，不是 completed
        let exec = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Execute)
            .unwrap();
        assert!(!exec.is_completed, "Execute is current, not completed");
        assert!(exec.is_current);

        // Plan 已完成
        let plan = nodes.iter().find(|n| n.phase == LoopPhase::Plan).unwrap();
        assert!(plan.is_completed, "Plan should be completed");

        // Monitor 未进入，不是 completed
        let monitor = nodes
            .iter()
            .find(|n| n.phase == LoopPhase::Monitor)
            .unwrap();
        assert!(!monitor.is_completed, "Monitor should not be completed");
    }
}
