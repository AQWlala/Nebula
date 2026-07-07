//! M5 任务 #68 + #69 + #70: L4 审批门禁 + 进化写入审批 + 超时/nonce 防重放。
//!
//! ## 设计
//!
//! - **ApprovalGate**：单点入口 `assess(ActionKind, description, autonomy)`
//!   返回 [`ApprovalVerdict`]（Allow / ConfirmRequired(payload)）。
//! - **ConfirmationRegistry**：进程内 pending confirmations 注册表，
//!   `confirmation_id`（UUID nonce）+ `created_at`（用于 5 分钟超时判定）。
//! - **5 分钟超时**：`check_confirmation(id)` 在 5 分钟外返回
//!   `Expired`，调用方据此丢弃。
//! - **nonce 防重放**：`mark_confirmed(id)` 后该 id 失效，
//!   二次提交返回 `AlreadyUsed`（防同一 confirmation 被多次消费）。
//!
//! ## 与 MasterEvent 的关系
//!
//! `MasterEvent::UserConfirmationRequired` 已含 `confirmation_id` + `created_at`
//! 字段（M3 #52）。本模块的 `PendingConfirmation` 与之字段一致，
//! 由调用方（chat.rs / EvolutionEngine Phase 4）负责 emit 事件。
//!
//! ## 不做的事
//!
//! - **不直接调用 tauri AppHandle**：保持模块解耦，由 bootstrap 注入
//!   emit 回调（与 `CostTracker::with_budget_alert` 同模式）。
//! - **不持久化**：pending confirmations 是进程内状态，重启即清空
//!   （与历史 emit 的事件保持一致，重启后未确认的请求视为已废弃）。

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use super::risk_map::{RiskTier, WorkerRiskMap};
use super::AutonomyLevel;
use crate::memory::values::risk_assessor::ActionKind;

/// 5 分钟超时（毫秒）。M5 #70 硬约束。
pub const CONFIRMATION_TIMEOUT_MS: i64 = 5 * 60 * 1000;

/// 审批结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalVerdict {
    /// 放行（Low 或 Medium-L4+ 自动放行）。
    Allow {
        /// 风险分级（供日志/前端展示）。
        risk_tier: RiskTier,
        reason: String,
    },
    /// 需要用户审批（High 或 Medium-L2/L3）。
    ConfirmRequired {
        /// 风险分级（High 或 Medium）。
        risk_tier: RiskTier,
        /// 显示给用户的提示语。
        prompt: String,
        /// 防重放 nonce（UUID v4）。
        confirmation_id: String,
        /// 创建时间戳（毫秒），用于 5 分钟超时判定。
        created_at: i64,
        /// diff 展示（仅 ActionKind::AiSelfModify 时非空）。
        diff: Option<String>,
    },
}

/// 单条 pending confirmation 的进程内记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingConfirmation {
    pub confirmation_id: String,
    pub action_kind: ActionKind,
    pub risk_tier: RiskTier,
    pub prompt: String,
    pub diff: Option<String>,
    pub created_at: i64,
    pub confirmed_at: Option<i64>,
}

/// `check_confirmation` / `mark_confirmed` 的返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    /// 已确认（首次提交）。
    Confirmed,
    /// 已过期（> 5 分钟）。
    Expired,
    /// confirmation_id 不存在或已被消费。
    NotFound,
    /// 已被消费过（防重放）。
    AlreadyUsed,
}

/// 审批门禁。
///
/// 单点入口 `assess()` 综合风险映射 + autonomy_level 联动，
/// 返回 `Allow` 或 `ConfirmRequired`。`ConfirmRequired` 携带的
/// `confirmation_id` 注册到 [`ConfirmationRegistry`]，调用方据此
/// emit `MasterEvent::UserConfirmationRequired` 事件。
#[derive(Debug, Clone)]
pub struct ApprovalGate {
    risk_map: WorkerRiskMap,
    registry: Arc<ConfirmationRegistry>,
}

impl ApprovalGate {
    pub fn new(risk_map: WorkerRiskMap, registry: Arc<ConfirmationRegistry>) -> Self {
        Self { risk_map, registry }
    }

    /// 评估单动作。
    ///
    /// - `kind`：动作分类（AiSelfModify 强制 High）
    /// - `description`：动作描述（含 bulk/destructive 信号检测）
    /// - `autonomy`：当前自主度等级（L2-L5）
    /// - `diff`：可选 diff 文本（AiSelfModify 时填，展示给用户）
    pub fn assess(
        &self,
        kind: ActionKind,
        description: &str,
        autonomy: AutonomyLevel,
        diff: Option<String>,
    ) -> ApprovalVerdict {
        let tier = self.risk_map.assess(kind, description);
        if !self.risk_map.needs_approval(kind, description, autonomy) {
            return ApprovalVerdict::Allow {
                risk_tier: tier,
                reason: format!("autonomy {autonomy:?} permits {tier:?} risk"),
            };
        }
        // 需审批
        let confirmation_id = Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp_millis();
        let prompt = format!(
            "Action: {:?} (risk={})\nDescription: {description}",
            kind,
            tier.as_str()
        );
        let pending = PendingConfirmation {
            confirmation_id: confirmation_id.clone(),
            action_kind: kind,
            risk_tier: tier,
            prompt: prompt.clone(),
            diff: diff.clone(),
            created_at,
            confirmed_at: None,
        };
        self.registry.register(pending);
        info!(
            target: "nebula.autonomy.approval",
            action_kind = ?kind,
            risk_tier = ?tier,
            autonomy = ?autonomy,
            confirmation_id = %confirmation_id,
            "approval required"
        );
        ApprovalVerdict::ConfirmRequired {
            risk_tier: tier,
            prompt,
            confirmation_id,
            created_at,
            diff,
        }
    }

    /// 便捷方法：检查 confirmation 是否在 5 分钟内且未被消费。
    pub fn check_confirmation(&self, id: &str) -> ConfirmationStatus {
        self.registry.check(id)
    }

    /// 便捷方法：标记 confirmation 已确认（防重放）。
    pub fn mark_confirmed(&self, id: &str) -> ConfirmationStatus {
        self.registry.mark_confirmed(id)
    }

    /// 当前 pending 数量（供测试/诊断）。
    pub fn pending_count(&self) -> usize {
        self.registry.pending_count()
    }
}

/// Pending confirmations 注册表。
///
/// `Arc<ConfirmationRegistry>` 可挂在 `AppState` / `ApprovalGate` 上，
/// 供 chat / evolution 多个调用方共享。
#[derive(Debug, Default)]
pub struct ConfirmationRegistry {
    inner: Mutex<HashMap<String, PendingConfirmation>>,
}

impl ConfirmationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一条 pending confirmation（覆盖同 id，理论不会发生因 UUID）。
    pub fn register(&self, pending: PendingConfirmation) {
        let mut g = self.inner.lock();
        g.insert(pending.confirmation_id.clone(), pending);
    }

    /// 检查 confirmation 状态：
    /// - 不存在 → `NotFound`
    /// - 已 confirmed → `AlreadyUsed`
    /// - 已过期 → `Expired`
    /// - 未过期 → `Confirmed`（不消费，仍可再次确认；用 `mark_confirmed` 消费）
    pub fn check(&self, id: &str) -> ConfirmationStatus {
        let g = self.inner.lock();
        let Some(p) = g.get(id) else {
            return ConfirmationStatus::NotFound;
        };
        if p.confirmed_at.is_some() {
            return ConfirmationStatus::AlreadyUsed;
        }
        let now = chrono::Utc::now().timestamp_millis();
        if now - p.created_at > CONFIRMATION_TIMEOUT_MS {
            return ConfirmationStatus::Expired;
        }
        ConfirmationStatus::Confirmed
    }

    /// 标记 confirmed（防重放）。
    /// 二次提交返回 `AlreadyUsed`，首次返回 `Confirmed`。
    pub fn mark_confirmed(&self, id: &str) -> ConfirmationStatus {
        let mut g = self.inner.lock();
        let Some(p) = g.get_mut(id) else {
            return ConfirmationStatus::NotFound;
        };
        if p.confirmed_at.is_some() {
            return ConfirmationStatus::AlreadyUsed;
        }
        let now = chrono::Utc::now().timestamp_millis();
        if now - p.created_at > CONFIRMATION_TIMEOUT_MS {
            return ConfirmationStatus::Expired;
        }
        p.confirmed_at = Some(now);
        ConfirmationStatus::Confirmed
    }

    /// 取出 pending（供调用方 emit 事件 / 展示 diff）。
    pub fn get(&self, id: &str) -> Option<PendingConfirmation> {
        self.inner.lock().get(id).cloned()
    }

    /// 当前 pending 数量。
    pub fn pending_count(&self) -> usize {
        self.inner.lock().len()
    }

    /// M6 #82: 返回所有 pending(包含已确认 / 已过期),供前端 Tauri 命令展示。
    /// 前端按 `confirmed_at` + `created_at` + `CONFIRMATION_TIMEOUT_MS` 自行过滤。
    pub fn all_pending(&self) -> Vec<PendingConfirmation> {
        self.inner.lock().values().cloned().collect()
    }

    /// 清理已确认 / 已过期的条目（GC 入口，由后台 worker 周期调用）。
    pub fn gc(&self) -> usize {
        let now = chrono::Utc::now().timestamp_millis();
        let mut g = self.inner.lock();
        let before = g.len();
        g.retain(|_, p| p.confirmed_at.is_none() && now - p.created_at <= CONFIRMATION_TIMEOUT_MS);
        before - g.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gate() -> ApprovalGate {
        let registry = Arc::new(ConfirmationRegistry::new());
        ApprovalGate::new(WorkerRiskMap::new(), registry)
    }

    #[test]
    fn allow_low_risk_read() {
        let gate = make_gate();
        let v = gate.assess(ActionKind::Read, "查询记忆", AutonomyLevel::L2Chat, None);
        assert!(matches!(v, ApprovalVerdict::Allow { .. }));
    }

    #[test]
    fn confirm_required_for_high_ai_self_modify_l2() {
        let gate = make_gate();
        let v = gate.assess(
            ActionKind::AiSelfModify,
            "Phase 4 写 SOUL.md",
            AutonomyLevel::L2Chat,
            Some("@@ diff @@".to_string()),
        );
        match v {
            ApprovalVerdict::ConfirmRequired {
                risk_tier,
                confirmation_id,
                diff,
                ..
            } => {
                assert_eq!(risk_tier, RiskTier::High);
                assert!(!confirmation_id.is_empty());
                assert_eq!(diff.as_deref(), Some("@@ diff @@"));
                assert_eq!(gate.pending_count(), 1);
            }
            other => panic!("expected ConfirmRequired, got {other:?}"),
        }
    }

    #[test]
    fn l5_ai_self_modify_bypassed_when_enabled() {
        let gate = make_gate();
        let v = gate.assess(
            ActionKind::AiSelfModify,
            "Phase 4 后台进化",
            AutonomyLevel::L5Background,
            None,
        );
        // 默认 bypass=true → Allow
        assert!(matches!(v, ApprovalVerdict::Allow { .. }));
    }

    #[test]
    fn medium_l4_swarm_no_approval() {
        let gate = make_gate();
        let v = gate.assess(
            ActionKind::Delete,
            "删除一条记忆",
            AutonomyLevel::L4Swarm,
            None,
        );
        assert!(matches!(v, ApprovalVerdict::Allow { .. }));
    }

    #[test]
    fn medium_l2_needs_approval() {
        let gate = make_gate();
        let v = gate.assess(
            ActionKind::Delete,
            "删除一条记忆",
            AutonomyLevel::L2Chat,
            None,
        );
        match v {
            ApprovalVerdict::ConfirmRequired { risk_tier, .. } => {
                assert_eq!(risk_tier, RiskTier::Medium);
            }
            other => panic!("expected ConfirmRequired, got {other:?}"),
        }
    }

    #[test]
    fn confirmation_check_then_mark_confirmed() {
        let gate = make_gate();
        let v = gate.assess(ActionKind::AiSelfModify, "x", AutonomyLevel::L2Chat, None);
        let id = match v {
            ApprovalVerdict::ConfirmRequired {
                confirmation_id, ..
            } => confirmation_id,
            _ => panic!(),
        };
        // 首次 check → Confirmed（未消费）
        assert_eq!(gate.check_confirmation(&id), ConfirmationStatus::Confirmed);
        // mark_confirmed → Confirmed（首次消费）
        assert_eq!(gate.mark_confirmed(&id), ConfirmationStatus::Confirmed);
        // 二次 mark → AlreadyUsed
        assert_eq!(gate.mark_confirmed(&id), ConfirmationStatus::AlreadyUsed);
        // check 后 → AlreadyUsed
        assert_eq!(
            gate.check_confirmation(&id),
            ConfirmationStatus::AlreadyUsed
        );
    }

    #[test]
    fn confirmation_expired_after_5min() {
        let registry = Arc::new(ConfirmationRegistry::new());
        // 手动塞一条 6 分钟前创建的 pending
        let old_id = "old-id".to_string();
        let six_min_ago = chrono::Utc::now().timestamp_millis() - (6 * 60 * 1000);
        registry.register(PendingConfirmation {
            confirmation_id: old_id.clone(),
            action_kind: ActionKind::AiSelfModify,
            risk_tier: RiskTier::High,
            prompt: "old".to_string(),
            diff: None,
            created_at: six_min_ago,
            confirmed_at: None,
        });
        // check → Expired
        assert_eq!(registry.check(&old_id), ConfirmationStatus::Expired);
        // mark → Expired
        assert_eq!(
            registry.mark_confirmed(&old_id),
            ConfirmationStatus::Expired
        );
    }

    #[test]
    fn confirmation_not_found_for_unknown_id() {
        let registry = ConfirmationRegistry::new();
        assert_eq!(registry.check("nope"), ConfirmationStatus::NotFound);
        assert_eq!(
            registry.mark_confirmed("nope"),
            ConfirmationStatus::NotFound
        );
    }

    #[test]
    fn gc_removes_confirmed_and_expired() {
        let registry = Arc::new(ConfirmationRegistry::new());
        // 一条已 confirmed
        let confirmed_id = "c1".to_string();
        registry.register(PendingConfirmation {
            confirmation_id: confirmed_id.clone(),
            action_kind: ActionKind::AiSelfModify,
            risk_tier: RiskTier::High,
            prompt: "x".to_string(),
            diff: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            confirmed_at: Some(chrono::Utc::now().timestamp_millis()),
        });
        // 一条 expired
        let expired_id = "e1".to_string();
        registry.register(PendingConfirmation {
            confirmation_id: expired_id.clone(),
            action_kind: ActionKind::AiSelfModify,
            risk_tier: RiskTier::High,
            prompt: "x".to_string(),
            diff: None,
            created_at: chrono::Utc::now().timestamp_millis() - (10 * 60 * 1000),
            confirmed_at: None,
        });
        // 一条 pending（未过期未确认）
        let pending_id = "p1".to_string();
        registry.register(PendingConfirmation {
            confirmation_id: pending_id.clone(),
            action_kind: ActionKind::AiSelfModify,
            risk_tier: RiskTier::High,
            prompt: "x".to_string(),
            diff: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            confirmed_at: None,
        });
        assert_eq!(registry.pending_count(), 3);
        let removed = registry.gc();
        assert_eq!(removed, 2);
        assert_eq!(registry.pending_count(), 1);
        // pending 仍在
        assert_eq!(registry.check(&pending_id), ConfirmationStatus::Confirmed);
    }

    #[test]
    fn verdict_serialization_roundtrip() {
        let v1 = ApprovalVerdict::Allow {
            risk_tier: RiskTier::Low,
            reason: "ok".to_string(),
        };
        let s1 = serde_json::to_string(&v1).unwrap();
        let back: ApprovalVerdict = serde_json::from_str(&s1).unwrap();
        match back {
            ApprovalVerdict::Allow { risk_tier, reason } => {
                assert_eq!(risk_tier, RiskTier::Low);
                assert_eq!(reason, "ok");
            }
            _ => panic!("wrong variant"),
        }

        let v2 = ApprovalVerdict::ConfirmRequired {
            risk_tier: RiskTier::High,
            prompt: "p".to_string(),
            confirmation_id: "cid".to_string(),
            created_at: 12345,
            diff: Some("@@".to_string()),
        };
        let s2 = serde_json::to_string(&v2).unwrap();
        assert!(s2.contains("\"kind\":\"confirm_required\""));
        let back2: ApprovalVerdict = serde_json::from_str(&s2).unwrap();
        match back2 {
            ApprovalVerdict::ConfirmRequired {
                risk_tier,
                confirmation_id,
                diff,
                ..
            } => {
                assert_eq!(risk_tier, RiskTier::High);
                assert_eq!(confirmation_id, "cid");
                assert_eq!(diff.as_deref(), Some("@@"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn confirmation_timeout_ms_is_5_minutes() {
        assert_eq!(CONFIRMATION_TIMEOUT_MS, 5 * 60 * 1000);
    }
}
