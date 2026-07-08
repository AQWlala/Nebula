//! v1.3 Plan 模式 + 准奏环节。
//!
//! 对应设计文档"风险表"与白皮书 §"关键操作必须用户准奏""高风险操作必须 Plan 模式"。
//!
//! ## 两个职责
//!
//! * [`plan_mode`] — 高风险任务先出方案（[`PlanRequest`]），用户审批后再执行。
//! * [`confirmation`] — 不可逆操作（删除/发送/转账）强制准奏（[`ConfirmationRequest`]）。
//!
//! ## 流程
//!
//! ```text
//! SwarmTask → ValuesLayer::evaluate()
//!   ├─ Verdict::Allow    → 直接执行
//!   ├─ Verdict::Confirm  → ConfirmationRequest → 用户准奏 → 执行/取消
//!   ├─ Verdict::Plan     → PlanRequest → 用户审批 → 执行/拒绝
//!   └─ Verdict::Deny     → 拒绝
//! ```

pub mod confirmation;
pub mod plan_mode;

pub use confirmation::{ConfirmationRequest, ConfirmationStatus, ConfirmationTracker};
pub use plan_mode::{PlanRequest, PlanStatus, PlanStep, PlanTracker};

use crate::memory::values::{ActionKind, Verdict};

/// L4 评估后产生的"待办事项"：可能是准奏请求或 Plan 请求。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PendingGate {
    /// 需要准奏（来自 `Verdict::Confirm`）。
    Confirm(ConfirmationRequest),
    /// 需要 Plan 审批（来自 `Verdict::Plan`）。
    Plan(PlanRequest),
}

/// Plan + 准奏 统一引擎。
///
/// 持有两个独立的 tracker（plan / confirmation），分别管理待审批的请求。
/// 请求在内存中保存（v1.3 不持久化；v1.4 可落库以防重启丢失）。
#[derive(Debug, Default)]
pub struct PlanEngine {
    plans: PlanTracker,
    confirmations: ConfirmationTracker,
}

impl PlanEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 根据 L4 裁定创建一个待办门禁。
    ///
    /// `Allow` / `Deny` 返回 `None`（无需用户介入）。
    pub fn create_gate(
        &self,
        verdict: &Verdict,
        task: &str,
        action_kind: ActionKind,
    ) -> Option<PendingGate> {
        match verdict {
            Verdict::Allow | Verdict::Deny { .. } => None,
            Verdict::Confirm { prompt } => {
                let req = self.confirmations.create(task, prompt, action_kind);
                Some(PendingGate::Confirm(req))
            }
            Verdict::Plan { prompt } => {
                let req = self.plans.create(task, prompt, action_kind);
                Some(PendingGate::Plan(req))
            }
        }
    }

    /// 准奏请求是否已批准。
    pub fn is_confirmed(&self, request_id: &str) -> bool {
        self.confirmations.is_approved(request_id)
    }

    /// Plan 请求是否已批准。
    pub fn is_plan_approved(&self, request_id: &str) -> bool {
        self.plans.is_approved(request_id)
    }

    /// 批准准奏请求。
    pub fn approve_confirmation(&self, request_id: &str) -> bool {
        self.confirmations.approve(request_id)
    }

    /// 拒绝准奏请求。
    pub fn deny_confirmation(&self, request_id: &str) -> bool {
        self.confirmations.deny(request_id)
    }

    /// 批准 Plan 请求。
    pub fn approve_plan(&self, request_id: &str) -> bool {
        self.plans.approve(request_id)
    }

    /// 拒绝 Plan 请求。
    pub fn reject_plan(&self, request_id: &str) -> bool {
        self.plans.reject(request_id)
    }

    /// 获取 Plan 请求（供前端展示方案详情）。
    pub fn get_plan(&self, request_id: &str) -> Option<PlanRequest> {
        self.plans.get(request_id)
    }

    /// 获取准奏请求。
    pub fn get_confirmation(&self, request_id: &str) -> Option<ConfirmationRequest> {
        self.confirmations.get(request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_gate_for_confirm() {
        let engine = PlanEngine::new();
        let gate = engine.create_gate(
            &Verdict::Confirm {
                prompt: "确认删除?".into(),
            },
            "删除 config.json",
            ActionKind::Delete,
        );
        assert!(matches!(gate, Some(PendingGate::Confirm(_))));
    }

    #[test]
    fn create_gate_for_plan() {
        let engine = PlanEngine::new();
        let gate = engine.create_gate(
            &Verdict::Plan {
                prompt: "需 Plan".into(),
            },
            "批量删除",
            ActionKind::BulkDelete,
        );
        assert!(matches!(gate, Some(PendingGate::Plan(_))));
    }

    #[test]
    fn no_gate_for_allow() {
        let engine = PlanEngine::new();
        assert!(engine
            .create_gate(&Verdict::Allow, "x", ActionKind::Read)
            .is_none());
    }

    #[test]
    fn confirmation_flow() {
        let engine = PlanEngine::new();
        let gate = engine.create_gate(
            &Verdict::Confirm { prompt: "p".into() },
            "t",
            ActionKind::Delete,
        );
        let req_id = match gate.expect("test op should succeed") {
            PendingGate::Confirm(c) => c.id,
            _ => unreachable!(),
        };
        assert!(!engine.is_confirmed(&req_id));
        assert!(engine.approve_confirmation(&req_id));
        assert!(engine.is_confirmed(&req_id));
    }
}
