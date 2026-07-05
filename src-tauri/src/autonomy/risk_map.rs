//! M5 任务 #67: WorkerRiskMap — 风险等级映射 + 动态阈值 + autonomy_level 联动。
//!
//! 与 `memory::values::risk_assessor::RiskLevel`（4 级裁定 Safe /
//! NeedsConfirm / NeedsPlan / Forbidden，供价值层使用）正交，
//! 本模块聚焦 **Worker 执行层** 的简化 3 级分类（High / Medium / Low），
//! 供 [`ApprovalGate`](super::approval::ApprovalGate) 决定是否需要
//! 用户审批 / 是否走 Plan 模式。
//!
//! ## 三级风险
//!
//! | Tier   | 审批策略                       | 适用场景                       |
//! |--------|-------------------------------|------------------------------|
//! | High   | 必须 L4 审批（confirmation_id） | AiSelfModify / BulkDelete / Transfer |
//! | Medium | 视 autonomy_level 动态判定      | Execute / Send / Delete / Modify |
//! | Low    | 直接放行                        | Read / Write（非批量） / Generic |
//!
//! ## 动态阈值
//!
//! `autonomy_level` 越高，可放行的风险越大：
//! - L2/L3：Medium 默认进入审批
//! - L4/L5：Medium 默认放行（蜂群自主 / 后台自动化场景）
//! - High **永远** 需审批（除 L5 后台 Evolution 写入：通过 `bypass_for_background_evolution`）
//!
//! ## 设计要点
//!
//! - **不重定义 `RiskLevel`**：与 `memory::values::risk_assessor::RiskLevel`
//!   正交，本模块用自己的 `RiskTier`（High/Medium/Low），不互相替代。
//! - **autonomy_level 联动**：`assess_with_autonomy()` 在判定 Medium 时
//!   读取 `AutonomyLevel`，L4+ 放行，L2/L3 进入审批。
//! - **L5 后台例外**：L5 + ActionKind::AiSelfModify（EvolutionEngine Phase 4
//!   后台写入 SOUL.md）通过 `bypass_background_ai_self_modify` 选项放行，
//!   避免后台无人值守时被审批门阻断。
//! - **可配置阈值**：`WorkerRiskMap::with_thresholds()` 支持自定义
//!   `medium_threshold_score` / `high_threshold_score`，默认 0.4 / 0.7。

use serde::{Deserialize, Serialize};

use super::AutonomyLevel;
use crate::memory::values::risk_assessor::{ActionKind, RiskAssessor, RiskVerdict};

/// 三级风险分级（Worker 执行层）。
///
/// 与 `memory::values::risk_assessor::RiskLevel`（4 级，价值层用）
/// 正交，本枚举仅 3 级，供 [`ApprovalGate`](super::approval::ApprovalGate)
/// 在 Worker 执行前快速判定是否需要审批。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    /// 低风险 — 直接放行。
    Low,
    /// 中风险 — 视 autonomy_level 动态判定。
    Medium,
    /// 高风险 — 必须 L4 审批（除 L5 后台例外）。
    High,
}

impl RiskTier {
    /// 字符串形式（`"low"` / `"medium"` / `"high"`）。
    pub fn as_str(&self) -> &'static str {
        match self {
            RiskTier::Low => "low",
            RiskTier::Medium => "medium",
            RiskTier::High => "high",
        }
    }

    /// 从字符串反序列化（未知值回退到 Low）。
    pub fn from_str(s: &str) -> Self {
        match s {
            "medium" => RiskTier::Medium,
            "high" => RiskTier::High,
            _ => RiskTier::Low,
        }
    }
}

/// WorkerRiskMap 配置（可运行时调整）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskThresholds {
    /// ≥ 该分数升为 Medium（默认 0.4）。
    pub medium_threshold_score: f32,
    /// ≥ 该分数升为 High（默认 0.7）。
    pub high_threshold_score: f32,
    /// L5 后台 + AiSelfModify 时是否放行（默认 true，避免后台阻断）。
    pub bypass_background_ai_self_modify: bool,
}

impl Default for RiskThresholds {
    fn default() -> Self {
        Self {
            medium_threshold_score: 0.4,
            high_threshold_score: 0.7,
            bypass_background_ai_self_modify: true,
        }
    }
}

/// Worker 风险映射器。
///
/// 内部委托 [`RiskAssessor`] 得到 `RiskVerdict`（含 0-1 分数），
/// 按阈值映射为 `RiskTier`，再结合 `autonomy_level` 决定最终是否
/// 需要审批。
#[derive(Debug, Clone)]
pub struct WorkerRiskMap {
    assessor: RiskAssessor,
    thresholds: RiskThresholds,
}

impl Default for WorkerRiskMap {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerRiskMap {
    pub fn new() -> Self {
        Self {
            assessor: RiskAssessor::new(),
            thresholds: RiskThresholds::default(),
        }
    }

    /// 注入自定义阈值（builder 风格）。
    pub fn with_thresholds(mut self, thresholds: RiskThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// 读取当前阈值。
    pub fn thresholds(&self) -> &RiskThresholds {
        &self.thresholds
    }

    /// 评估单动作得到 RiskTier（不含 autonomy 联动）。
    ///
    /// 步骤：
    /// 1. 委托 `RiskAssessor::assess(kind, description)` 得到 `RiskVerdict`
    /// 2. 按 `score` 阈值映射为 `RiskTier`
    /// 3. `ActionKind::AiSelfModify` / `BulkDelete` / `Transfer` /
    ///    `RemoteLlmDispatch` 强制 High（无论分数）
    pub fn assess(&self, kind: ActionKind, description: &str) -> RiskTier {
        // 强制 High 的动作（不可降级）
        // - AiSelfModify: 不可逆、影响系统人格
        // - BulkDelete / Transfer: 数据丢失 / 资金风险
        // - RemoteLlmDispatch (P1-15): 用户输入发送到远端 provider,隐私硬约束
        if matches!(
            kind,
            ActionKind::BulkDelete
                | ActionKind::Transfer
                | ActionKind::AiSelfModify
                | ActionKind::RemoteLlmDispatch
        ) {
            return RiskTier::High;
        }

        let verdict: RiskVerdict = self.assessor.assess(kind, description);
        self.tier_from_score(verdict.score)
    }

    /// 评估 + autonomy_level 联动，返回 `true` 表示需要审批。
    ///
    /// 规则：
    /// - High：永远需要审批（除非 `bypass_background_ai_self_modify`
    ///   且 (kind, autonomy) == (AiSelfModify, L5)）
    /// - Medium：L2/L3 需审批；L4/L5 放行
    /// - Low：永远放行
    pub fn needs_approval(
        &self,
        kind: ActionKind,
        description: &str,
        autonomy: AutonomyLevel,
    ) -> bool {
        let tier = self.assess(kind, description);
        match tier {
            RiskTier::Low => false,
            RiskTier::Medium => matches!(
                autonomy,
                AutonomyLevel::L0InlineCompletion
                    | AutonomyLevel::L1DirectedEdit
                    | AutonomyLevel::L2Chat
                    | AutonomyLevel::L3Plan
            ),
            RiskTier::High => {
                // L5 后台 + AiSelfModify 例外出参
                if self.thresholds.bypass_background_ai_self_modify
                    && kind == ActionKind::AiSelfModify
                    && autonomy == AutonomyLevel::L5Background
                {
                    return false;
                }
                true
            }
        }
    }

    /// 内部：按分数映射 RiskTier。
    fn tier_from_score(&self, score: f32) -> RiskTier {
        if score >= self.thresholds.high_threshold_score {
            RiskTier::High
        } else if score >= self.thresholds.medium_threshold_score {
            RiskTier::Medium
        } else {
            RiskTier::Low
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_is_low_risk() {
        let m = WorkerRiskMap::new();
        assert_eq!(m.assess(ActionKind::Read, "查询记忆"), RiskTier::Low);
    }

    #[test]
    fn delete_is_medium_by_default() {
        let m = WorkerRiskMap::new();
        // Delete 默认 score = 0.6 → Medium（≥ 0.4 < 0.7）
        assert_eq!(m.assess(ActionKind::Delete, "删一条"), RiskTier::Medium);
    }

    #[test]
    fn bulk_delete_forced_high() {
        let m = WorkerRiskMap::new();
        // BulkDelete 强制 High，不论描述
        assert_eq!(m.assess(ActionKind::BulkDelete, "any"), RiskTier::High);
    }

    #[test]
    fn transfer_forced_high() {
        let m = WorkerRiskMap::new();
        assert_eq!(m.assess(ActionKind::Transfer, "转账 1 元"), RiskTier::High);
    }

    #[test]
    fn ai_self_modify_forced_high() {
        let m = WorkerRiskMap::new();
        assert_eq!(
            m.assess(ActionKind::AiSelfModify, "写 SOUL.md"),
            RiskTier::High
        );
    }

    #[test]
    fn remote_llm_dispatch_forced_high() {
        // M5 #71 / P1-15: RemoteLlmDispatch 强制 High(隐私硬约束)
        let m = WorkerRiskMap::new();
        assert_eq!(
            m.assess(ActionKind::RemoteLlmDispatch, "用户任务描述"),
            RiskTier::High
        );
    }

    #[test]
    fn remote_llm_dispatch_needs_approval_at_all_autonomy_levels() {
        // M5 #71 / P1-15: 隐私提示不受 autonomy 影响,L5 也要审批
        let m = WorkerRiskMap::new();
        for level in [
            AutonomyLevel::L0InlineCompletion,
            AutonomyLevel::L1DirectedEdit,
            AutonomyLevel::L2Chat,
            AutonomyLevel::L3Plan,
            AutonomyLevel::L4Swarm,
            AutonomyLevel::L5Background,
        ] {
            assert!(
                m.needs_approval(ActionKind::RemoteLlmDispatch, "test", level),
                "RemoteLlmDispatch must require approval at {:?}",
                level
            );
        }
    }

    #[test]
    fn execute_destructive_is_high() {
        let m = WorkerRiskMap::new();
        // rm -rf → score 0.8 → High
        assert_eq!(
            m.assess(ActionKind::Execute, "rm -rf /tmp/old"),
            RiskTier::High
        );
    }

    #[test]
    fn execute_normal_is_medium() {
        let m = WorkerRiskMap::new();
        // 普通 Shell → score 0.5 → Medium
        assert_eq!(
            m.assess(ActionKind::Execute, "ls -la"),
            RiskTier::Medium
        );
    }

    #[test]
    fn needs_approval_high_always_except_l5_background_ai_self_modify() {
        let m = WorkerRiskMap::new();
        // High (AiSelfModify) — L2 需审批
        assert!(m.needs_approval(
            ActionKind::AiSelfModify,
            "evolve",
            AutonomyLevel::L2Chat
        ));
        // L5 + AiSelfModify + bypass=true → 放行
        assert!(!m.needs_approval(
            ActionKind::AiSelfModify,
            "evolve",
            AutonomyLevel::L5Background
        ));
        // 关闭 bypass → L5 也需审批
        let m2 = WorkerRiskMap::new().with_thresholds(RiskThresholds {
            bypass_background_ai_self_modify: false,
            ..RiskThresholds::default()
        });
        assert!(m2.needs_approval(
            ActionKind::AiSelfModify,
            "evolve",
            AutonomyLevel::L5Background
        ));
    }

    #[test]
    fn needs_approval_medium_depends_on_autonomy() {
        let m = WorkerRiskMap::new();
        // Medium (Delete) — L2/L3 需审批
        assert!(m.needs_approval(
            ActionKind::Delete,
            "x",
            AutonomyLevel::L2Chat
        ));
        assert!(m.needs_approval(
            ActionKind::Delete,
            "x",
            AutonomyLevel::L3Plan
        ));
        // L4/L5 放行
        assert!(!m.needs_approval(
            ActionKind::Delete,
            "x",
            AutonomyLevel::L4Swarm
        ));
        assert!(!m.needs_approval(
            ActionKind::Delete,
            "x",
            AutonomyLevel::L5Background
        ));
    }

    #[test]
    fn needs_approval_low_never() {
        let m = WorkerRiskMap::new();
        // Low (Read) — 任何 autonomy 都放行
        for level in AutonomyLevel::all() {
            assert!(!m.needs_approval(ActionKind::Read, "查询", *level));
        }
    }

    #[test]
    fn custom_thresholds_shift_tier() {
        // 提高阈值：Delete (0.6) 在 medium_threshold=0.7 时降为 Low
        let m = WorkerRiskMap::new().with_thresholds(RiskThresholds {
            medium_threshold_score: 0.7,
            high_threshold_score: 0.85,
            bypass_background_ai_self_modify: true,
        });
        assert_eq!(m.assess(ActionKind::Delete, "x"), RiskTier::Low);
    }

    #[test]
    fn tier_serde_roundtrip() {
        for tier in [RiskTier::Low, RiskTier::Medium, RiskTier::High] {
            let s = serde_json::to_string(&tier).unwrap();
            let back: RiskTier = serde_json::from_str(&s).unwrap();
            assert_eq!(back, tier);
        }
        // from_str 未知值回退 Low
        assert_eq!(RiskTier::from_str("unknown"), RiskTier::Low);
        assert_eq!(RiskTier::from_str("high"), RiskTier::High);
    }

    #[test]
    fn thresholds_default_values() {
        let t = RiskThresholds::default();
        assert!((t.medium_threshold_score - 0.4).abs() < 1e-6);
        assert!((t.high_threshold_score - 0.7).abs() < 1e-6);
        assert!(t.bypass_background_ai_self_modify);
    }
}
