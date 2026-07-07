//! 准奏环节：不可逆操作（删除/发送/转账）强制用户确认。

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::memory::values::ActionKind;

/// 准奏请求状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    /// 等待用户确认。
    Pending,
    /// 已批准。
    Approved,
    /// 已拒绝。
    Denied,
    /// 超时（默认 5 分钟未响应）。
    Expired,
}

/// 一个准奏请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationRequest {
    /// 请求 ID。
    pub id: String,
    /// 待确认的动作描述。
    pub action: String,
    /// L4 风险评估给出的理由。
    pub risk: String,
    /// 动作分类。
    pub action_kind: ActionKind,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
    /// 当前状态。
    pub status: ConfirmationStatus,
}

/// 准奏请求注册表（内存态）。
#[derive(Debug, Default)]
pub struct ConfirmationTracker {
    inner: Arc<Mutex<HashMap<String, ConfirmationRequest>>>,
}

/// 准奏默认超时（5 分钟）。
const CONFIRMATION_TIMEOUT_MS: i64 = 5 * 60 * 1000;

impl ConfirmationTracker {
    /// 创建一个准奏请求。
    pub fn create(&self, action: &str, risk: &str, action_kind: ActionKind) -> ConfirmationRequest {
        let req = ConfirmationRequest {
            id: new_id(),
            action: action.to_string(),
            risk: risk.to_string(),
            action_kind,
            created_at: chrono::Utc::now().timestamp_millis(),
            status: ConfirmationStatus::Pending,
        };
        self.inner.lock().insert(req.id.clone(), req.clone());
        req
    }

    /// 批准准奏。
    pub fn approve(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            if req.status == ConfirmationStatus::Pending && !self.is_expired(req) {
                req.status = ConfirmationStatus::Approved;
                return true;
            }
        }
        false
    }

    /// 拒绝准奏。
    pub fn deny(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(req) = g.get_mut(id) {
            if req.status == ConfirmationStatus::Pending {
                req.status = ConfirmationStatus::Denied;
                return true;
            }
        }
        false
    }

    /// 是否已批准。
    pub fn is_approved(&self, id: &str) -> bool {
        self.inner
            .lock()
            .get(id)
            .map(|r| r.status == ConfirmationStatus::Approved)
            .unwrap_or(false)
    }

    /// 获取请求。
    pub fn get(&self, id: &str) -> Option<ConfirmationRequest> {
        self.inner.lock().get(id).cloned()
    }

    /// 清理已超时的 Pending 请求（标记为 Expired）。
    pub fn sweep_expired(&self) -> usize {
        let now = chrono::Utc::now().timestamp_millis();
        let mut g = self.inner.lock();
        let mut count = 0;
        for req in g.values_mut() {
            if req.status == ConfirmationStatus::Pending
                && now - req.created_at > CONFIRMATION_TIMEOUT_MS
            {
                req.status = ConfirmationStatus::Expired;
                count += 1;
            }
        }
        count
    }

    fn is_expired(&self, req: &ConfirmationRequest) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        now - req.created_at > CONFIRMATION_TIMEOUT_MS
    }
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("conf_{ts:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_approve() {
        let t = ConfirmationTracker::default();
        let req = t.create("删除 config.json", "不可逆", ActionKind::Delete);
        assert_eq!(req.status, ConfirmationStatus::Pending);
        assert!(t.approve(&req.id));
        assert!(t.is_approved(&req.id));
    }

    #[test]
    fn deny_works() {
        let t = ConfirmationTracker::default();
        let req = t.create("发送邮件", "不可撤回", ActionKind::Send);
        assert!(t.deny(&req.id));
        assert!(!t.is_approved(&req.id));
    }

    #[test]
    fn unknown_id_not_approved() {
        let t = ConfirmationTracker::default();
        assert!(!t.is_approved("nonexistent"));
    }
}
