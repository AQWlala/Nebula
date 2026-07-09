//! T-E-S-20: exec 类操作审批门禁 — fail-closed 超时拒绝。
//!
//! 设计目标：当一个 exec 类操作（shell 命令执行、Python 沙箱技能等）
//! 需要用户审批时，若用户在 `exec_approval_timeout_secs`（默认 60s）
//! 内未响应，则**自动拒绝**执行（fail-closed），而不是默认放行。
//!
//! ## 与 `plan::confirmation` 的区别
//!
//! * [`crate::plan::confirmation::ConfirmationTracker`] 是通用的"准奏"
//!   机制，覆盖删除/发送/转账等不可逆操作，默认 5 分钟超时。
//! * 本模块专用于 **exec 类操作**，超时更短（默认 60s），且超时后
//!   会被 [`SkillAuditLogger`] 记录为 `timeout_fail_closed` 事件，
//!   便于安全审计追溯。
//!
//! ## 同步原语
//!
//! 内部状态用 `parking_lot::Mutex` 保护（与 `ConfirmationTracker`
//! 一致）；异步等待用 `tokio::sync::Notify`，这样 [`SkillEngine`]
//! 可以在 `use_skill` 中以 `tokio::time::timeout` 包裹
//! `notify.notified()` 来实现"等待审批或超时"的竞速。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// 审批默认超时（秒）。可被 [`ExecApprovalTracker::new`] 覆盖，
/// 也可由 `AppConfig::exec_approval_timeout_secs` 全局配置。
pub const DEFAULT_EXEC_APPROVAL_TIMEOUT_SECS: u64 = 60;

/// T-E-S-20 审计日志中记录的拒绝原因字符串。
pub const TIMEOUT_FAIL_CLOSED_REASON: &str = "timeout_fail_closed";

/// exec 审批请求状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecApprovalStatus {
    /// 等待用户响应。
    Pending,
    /// 已批准。
    Approved,
    /// 已拒绝（用户主动拒绝或安全策略拒绝）。
    Denied,
    /// 超时 fail-closed（用户未在规定时间内响应，自动拒绝）。
    TimeoutFailClosed,
}

impl ExecApprovalStatus {
    /// 是否为终态（不再变化）。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ExecApprovalStatus::Approved
                | ExecApprovalStatus::Denied
                | ExecApprovalStatus::TimeoutFailClosed
        )
    }

    /// 是否允许执行（仅 `Approved` 为真）。
    pub fn is_allowed(self) -> bool {
        matches!(self, ExecApprovalStatus::Approved)
    }
}

/// 一个 exec 审批请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecApprovalRequest {
    /// 请求 ID。
    pub id: String,
    /// 关联的 skill / 命令标识（便于审计追溯）。
    pub skill_id: String,
    /// 待审批的动作描述（人类可读，会写入审计摘要）。
    pub action: String,
    /// 创建时间（Unix 毫秒）。
    pub created_at: i64,
    /// 当前状态。
    pub status: ExecApprovalStatus,
}

/// 内部条目：请求 + 唤醒句柄。
///
/// `Notify` 让 [`SkillEngine`] 可以异步等待审批结果，而无需轮询。
#[derive(Debug)]
struct ExecApprovalEntry {
    request: ExecApprovalRequest,
    notify: Arc<Notify>,
}

/// exec 审批注册表（内存态）。
///
/// 线程安全：内部用 `parking_lot::Mutex<HashMap<..>>` 保护。
/// 所有公开方法都是 `&self`，可安全通过 `Arc` 共享。
#[derive(Debug)]
pub struct ExecApprovalTracker {
    inner: Arc<Mutex<HashMap<String, ExecApprovalEntry>>>,
    timeout: Duration,
}

impl ExecApprovalTracker {
    /// 创建一个审批注册表，超时为 `timeout_secs` 秒。
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// 当前配置的超时时长。
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 创建一个 exec 审批请求。
    ///
    /// 返回请求快照与一个 [`Arc<Notify>`] 唤醒句柄。审批方调用
    /// [`approve`](Self::approve) / [`deny`](Self::deny) 会触发
    /// `notify_one()`，调用方可通过 `notify.notified()` 感知。
    pub fn request(&self, skill_id: &str, action: &str) -> (ExecApprovalRequest, Arc<Notify>) {
        let notify = Arc::new(Notify::new());
        let req = ExecApprovalRequest {
            id: new_id(),
            skill_id: skill_id.to_string(),
            action: action.to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            status: ExecApprovalStatus::Pending,
        };
        let entry = ExecApprovalEntry {
            request: req.clone(),
            notify: notify.clone(),
        };
        self.inner.lock().insert(req.id.clone(), entry);
        (req, notify)
    }

    /// 批准审批请求。仅在 `Pending` 态生效；返回是否成功转态。
    pub fn approve(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(entry) = g.get_mut(id) {
            if entry.request.status == ExecApprovalStatus::Pending {
                entry.request.status = ExecApprovalStatus::Approved;
                entry.notify.notify_one();
                return true;
            }
        }
        false
    }

    /// 拒绝审批请求。仅在 `Pending` 态生效；返回是否成功转态。
    pub fn deny(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(entry) = g.get_mut(id) {
            if entry.request.status == ExecApprovalStatus::Pending {
                entry.request.status = ExecApprovalStatus::Denied;
                entry.notify.notify_one();
                return true;
            }
        }
        false
    }

    /// 主动将一个 `Pending` 请求标记为超时 fail-closed。
    ///
    /// 这是 fail-closed 的核心：当调用方（如 [`SkillEngine`]）判定
    /// 已超时时调用此方法，将请求从"无响应"转为"明确拒绝"。返回
    /// 是否成功转态（仅 `Pending` 态可转）。
    pub fn mark_timeout_fail_closed(&self, id: &str) -> bool {
        let mut g = self.inner.lock();
        if let Some(entry) = g.get_mut(id) {
            if entry.request.status == ExecApprovalStatus::Pending {
                entry.request.status = ExecApprovalStatus::TimeoutFailClosed;
                // 唤醒任何正在等待的 future，让它看到终态。
                entry.notify.notify_one();
                return true;
            }
        }
        false
    }

    /// 检查单个请求是否已超时；若是 `Pending` 且超过 `timeout`，
    /// 标记为 `TimeoutFailClosed` 并返回 `true`。
    ///
    /// 这是 fail-closed 的探测入口：调用方在等待循环里周期性
    /// 调用，或在 `tokio::time::timeout` 触发后调用，以把"无响应"
    /// 落实为"拒绝"。
    pub fn check_timeout_fail_closed(&self, id: &str) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        let mut g = self.inner.lock();
        if let Some(entry) = g.get_mut(id) {
            if entry.request.status == ExecApprovalStatus::Pending
                && now - entry.request.created_at > self.timeout.as_millis() as i64
            {
                entry.request.status = ExecApprovalStatus::TimeoutFailClosed;
                entry.notify.notify_one();
                return true;
            }
        }
        false
    }

    /// 清理所有已超时的 `Pending` 请求（标记为 `TimeoutFailClosed`），
    /// 返回这些请求的快照（供调用方批量写审计日志）。
    pub fn sweep_expired(&self) -> Vec<ExecApprovalRequest> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut g = self.inner.lock();
        let mut expired = Vec::new();
        for entry in g.values_mut() {
            if entry.request.status == ExecApprovalStatus::Pending
                && now - entry.request.created_at > self.timeout.as_millis() as i64
            {
                entry.request.status = ExecApprovalStatus::TimeoutFailClosed;
                entry.notify.notify_one();
                expired.push(entry.request.clone());
            }
        }
        expired
    }

    /// 获取请求快照。
    pub fn get(&self, id: &str) -> Option<ExecApprovalRequest> {
        self.inner.lock().get(id).map(|e| e.request.clone())
    }

    /// 当前状态。
    pub fn status(&self, id: &str) -> Option<ExecApprovalStatus> {
        self.inner.lock().get(id).map(|e| e.request.status)
    }

    /// 是否已批准。
    pub fn is_approved(&self, id: &str) -> bool {
        self.status(id).map(|s| s.is_allowed()).unwrap_or(false)
    }

    /// 是否因超时被 fail-closed。
    pub fn is_timeout_fail_closed(&self, id: &str) -> bool {
        self.status(id)
            .map(|s| s == ExecApprovalStatus::TimeoutFailClosed)
            .unwrap_or(false)
    }

    /// 当前待审批请求数量（主要用于诊断/测试）。
    pub fn pending_count(&self) -> usize {
        self.inner
            .lock()
            .values()
            .filter(|e| e.request.status == ExecApprovalStatus::Pending)
            .count()
    }

    /// 返回当前所有审批请求快照（按创建时间升序）。
    ///
    /// 供 Tauri `exec_approval_list` 命令查询当前 Pending / Approved /
    /// Denied / TimeoutFailClosed 全部状态。请求快照是 clone 的，
    /// 调用方修改不影响注册表内部状态。
    pub fn list_all(&self) -> Vec<ExecApprovalRequest> {
        let mut all: Vec<ExecApprovalRequest> = self
            .inner
            .lock()
            .values()
            .map(|e| e.request.clone())
            .collect();
        all.sort_by_key(|r| r.created_at);
        all
    }
}

impl Default for ExecApprovalTracker {
    fn default() -> Self {
        Self::new(DEFAULT_EXEC_APPROVAL_TIMEOUT_SECS)
    }
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("exec_appr_{ts:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// T-E-S-20: 一个 `Pending` 请求在超过超时窗口后，必须被
    /// `check_timeout_fail_closed` 转为 `TimeoutFailClosed`。
    #[test]
    fn pending_request_times_out_fail_closed() {
        // 1 秒超时，便于快速测试。
        let tracker = ExecApprovalTracker::new(1);
        let (req, _notify) = tracker.request("skill-1", "exec python script");
        assert_eq!(req.status, ExecApprovalStatus::Pending);

        // 立即检查：尚未超时。
        assert!(!tracker.check_timeout_fail_closed(&req.id));
        assert_eq!(
            tracker.status(&req.id).expect("test op should succeed"),
            ExecApprovalStatus::Pending
        );

        // 等待超过窗口。
        thread::sleep(Duration::from_millis(1100));
        assert!(tracker.check_timeout_fail_closed(&req.id));
        assert_eq!(
            tracker.status(&req.id).expect("test op should succeed"),
            ExecApprovalStatus::TimeoutFailClosed
        );
        assert!(tracker.is_timeout_fail_closed(&req.id));
        assert!(!tracker.is_approved(&req.id));
    }

    /// T-E-S-20: 已批准的请求不会被超时逻辑误判为 fail-closed。
    #[test]
    fn approved_request_is_not_expired() {
        let tracker = ExecApprovalTracker::new(1);
        let (req, _notify) = tracker.request("skill-2", "exec shell");
        assert!(tracker.approve(&req.id));
        assert!(tracker.is_approved(&req.id));

        thread::sleep(Duration::from_millis(1100));
        // 已是终态，check 不应改写。
        assert!(!tracker.check_timeout_fail_closed(&req.id));
        assert_eq!(
            tracker.status(&req.id).expect("test op should succeed"),
            ExecApprovalStatus::Approved
        );
    }

    /// T-E-S-20: `deny` 后请求进入 `Denied` 终态，超时逻辑不再生效。
    #[test]
    fn denied_request_is_terminal() {
        let tracker = ExecApprovalTracker::new(1);
        let (req, _notify) = tracker.request("skill-3", "exec rm");
        assert!(tracker.deny(&req.id));
        assert!(!tracker.is_approved(&req.id));
        assert_eq!(
            tracker.status(&req.id).expect("assertion value"),
            ExecApprovalStatus::Denied
        );
        thread::sleep(Duration::from_millis(1100));
        assert!(!tracker.check_timeout_fail_closed(&req.id));
        assert_eq!(
            tracker.status(&req.id).expect("assertion value"),
            ExecApprovalStatus::Denied
        );
    }

    /// T-E-S-20: `mark_timeout_fail_closed` 直接将 Pending 标记为
    /// 超时拒绝（用于 `tokio::time::timeout` 触发后的显式落态）。
    #[test]
    fn mark_timeout_fail_closed_works() {
        let tracker = ExecApprovalTracker::new(60);
        let (req, _notify) = tracker.request("skill-4", "exec python");
        assert!(tracker.mark_timeout_fail_closed(&req.id));
        assert!(tracker.is_timeout_fail_closed(&req.id));
        assert!(!tracker.is_approved(&req.id));
        // 重复标记应失败（已终态）。
        assert!(!tracker.mark_timeout_fail_closed(&req.id));
    }

    /// T-E-S-20: `sweep_expired` 批量清理超时请求并返回快照。
    #[test]
    fn sweep_expired_returns_expired_requests() {
        let tracker = ExecApprovalTracker::new(1);
        let (r1, _) = tracker.request("skill-a", "exec a");
        let (r2, _) = tracker.request("skill-b", "exec b");
        // r1 批准，r2 放任超时。
        assert!(tracker.approve(&r1.id));
        thread::sleep(Duration::from_millis(1100));

        let expired = tracker.sweep_expired();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, r2.id);
        assert_eq!(expired[0].status, ExecApprovalStatus::TimeoutFailClosed);
        // r1 仍为 Approved。
        assert_eq!(
            tracker.status(&r1.id).expect("test op should succeed"),
            ExecApprovalStatus::Approved
        );
    }

    /// T-E-S-20: 审批通过后，等待方应被唤醒。
    #[tokio::test]
    async fn notify_fires_on_approve() {
        let tracker = Arc::new(ExecApprovalTracker::new(60));
        let (req, notify) = tracker.request("skill-async", "exec async");

        let t = tracker.clone();
        let id = req.id.clone();
        // 在另一个任务里延迟批准。
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(t.approve(&id));
        });

        // 等待审批，1s 超时兜底。
        let res = tokio::time::timeout(Duration::from_secs(1), notify.notified()).await;
        assert!(res.is_ok(), "notify should fire before timeout");
        assert!(tracker.is_approved(&req.id));
    }

    /// T-E-S-20: 超时后 `tokio::time::timeout` 触发，调用方应能
    /// 通过 `mark_timeout_fail_closed` 把请求落为 fail-closed。
    #[tokio::test]
    async fn timeout_triggers_fail_closed_via_notify() {
        // 50ms 超时，快速验证。
        let tracker = Arc::new(ExecApprovalTracker::new(60));
        let (req, notify) = tracker.request("skill-to", "exec to");

        // 用远短于 tracker.timeout 的时间等待，模拟"用户未响应"。
        let waited = tokio::time::timeout(Duration::from_millis(50), notify.notified()).await;
        assert!(waited.is_err(), "should time out with no approver");

        // 显式落态为 fail-closed。
        assert!(tracker.mark_timeout_fail_closed(&req.id));
        assert!(tracker.is_timeout_fail_closed(&req.id));
        assert!(!tracker.is_approved(&req.id));
    }

    #[test]
    fn default_uses_60s_timeout() {
        let t = ExecApprovalTracker::default();
        assert_eq!(t.timeout(), Duration::from_secs(60));
    }

    #[test]
    fn pending_count_tracks_pending_only() {
        let tracker = ExecApprovalTracker::new(60);
        let (r1, _) = tracker.request("a", "x");
        let (r2, _) = tracker.request("b", "y");
        assert_eq!(tracker.pending_count(), 2);
        tracker.approve(&r1.id);
        assert_eq!(tracker.pending_count(), 1);
        tracker.deny(&r2.id);
        assert_eq!(tracker.pending_count(), 0);
    }
}
