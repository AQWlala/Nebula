//! Plan 模式：高风险任务先出方案，用户审批后再执行。

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::memory::values::ActionKind;

/// Plan 请求状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// 等待用户审批。
    Pending,
    /// 已批准，可执行。
    Approved,
    /// 已拒绝。
    Rejected,
    /// 执行中。
    Executing,
    /// 已完成。
    Done,
    /// 执行失败。
    Failed,
}

/// Plan 中的单步。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// 步骤描述。
    pub description: String,
    /// 该步骤的动作分类。
    pub action_kind: ActionKind,
}

/// 一个 Plan 请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    /// 请求 ID（UUID）。
    pub id: String,
    /// 原始任务描述。
    pub task: String,
    /// L4 给出的理由（为什么需要 Plan）。
    pub rationale: String,
    /// 拟执行步骤（v1.3 由调用方填充；v1.4 可由 LLM 自动拆解）。
    pub steps: Vec<PlanStep>,
    /// 预期结果。
    pub expected_outcome: String,
    /// 回滚策略。
    pub rollback_strategy: String,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
    /// 当前状态。
    pub status: PlanStatus,
}

/// Plan 请求注册表（内存态）。
#[derive(Debug, Default)]
pub struct PlanTracker {
    inner: Arc<Mutex<HashMap<String, PlanRequest>>>,
}

impl PlanTracker {
    /// 创建一个新的 Plan 请求并登记。
    ///
    /// v1.3：`steps` / `expected_outcome` / `rollback_strategy` 留空，
    /// 由前端或后续 LLM 拆解填充（通过 [`PlanTracker::update_plan`]）。
    pub fn create(
        &self,
        task: &str,
        rationale: &str,
        action_kind: ActionKind,
    ) -> PlanRequest {
        let req = PlanRequest {
            id: new_id(),
            task: task.to_string(),
            rationale: rationale.to_string(),
            steps: vec![PlanStep {
                description: task.to_string(),
                action_kind,
            }],
            expected_outcome: String::new(),
            rollback_strategy: String::new(),
            created_at: chrono::Utc::now().timestamp_millis(),
            status: PlanStatus::Pending,
        };
        self.inner.lock().insert(req.id.clone(), req.clone());
        req
    }

    /// 更新 Plan 的步骤/预期/回滚（用户或 LLM 编辑方案后调用）。
    pub fn update_plan(
        &self,
        id: &str,
        steps: Vec<PlanStep>,
        expected_outcome: &str,
        rollback_strategy: &str,
    ) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            req.steps = steps;
            req.expected_outcome = expected_outcome.to_string();
            req.rollback_strategy = rollback_strategy.to_string();
            true
        } else {
            false
        }
    }

    /// 批准 Plan。
    pub fn approve(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            if req.status == PlanStatus::Pending {
                req.status = PlanStatus::Approved;
                return true;
            }
        }
        false
    }

    /// 拒绝 Plan。
    pub fn reject(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            if req.status == PlanStatus::Pending {
                req.status = PlanStatus::Rejected;
                return true;
            }
        }
        false
    }

    /// 标记为执行中。
    pub fn mark_executing(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            req.status = PlanStatus::Executing;
            return true;
        }
        false
    }

    /// 标记完成。
    pub fn mark_done(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            req.status = PlanStatus::Done;
            return true;
        }
        false
    }

    /// 是否已批准。
    pub fn is_approved(&self, id: &str) -> bool {
        self.inner
            .lock()
            .get(id)
            .map(|r| r.status == PlanStatus::Approved)
            .unwrap_or(false)
    }

    /// 获取请求。
    pub fn get(&self, id: &str) -> Option<PlanRequest> {
        self.inner.lock().get(id).cloned()
    }
}

/// 生成简短唯一 ID。
fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("plan_{ts:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_approve() {
        let t = PlanTracker::default();
        let req = t.create("批量删除日志", "高风险", ActionKind::BulkDelete);
        assert_eq!(req.status, PlanStatus::Pending);
        assert!(t.approve(&req.id));
        assert!(t.is_approved(&req.id));
    }

    #[test]
    fn reject_pending() {
        let t = PlanTracker::default();
        let req = t.create("x", "r", ActionKind::Transfer);
        assert!(t.reject(&req.id));
        assert!(!t.is_approved(&req.id));
    }

    #[test]
    fn cannot_approve_twice() {
        let t = PlanTracker::default();
        let req = t.create("x", "r", ActionKind::Generic);
        assert!(t.approve(&req.id));
        assert!(!t.approve(&req.id)); // 已批准，不能再批
    }
}
