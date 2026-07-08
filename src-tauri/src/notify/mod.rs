//! T-E-S-57: 后台执行通知服务。
//!
//! 封装系统通知 + 悬浮球状态广播 + 去重逻辑。
//! - 系统通知: 使用 tauri-plugin-notification
//! - 悬浮球状态: 通过 `nebula://floating-ball-state` 事件广播
//! - 去重: 同类任务 5s 内不重复弹窗
//!
//! T-E-C-20: headless 模式下 `app` 字段被 cfg 移除,
//! 所有通知方法在 headless cfg 下为 no-op(日志代替弹窗)。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "headless"))]
use tauri::{AppHandle, Emitter, Manager};
#[cfg(not(feature = "headless"))]
use tauri_plugin_notification::NotificationExt;
#[cfg(not(feature = "headless"))]
use tracing::warn;

use tracing::debug;

use crate::swarm::SwarmEvent;

const DEDUPLICATION_WINDOW_MS: i64 = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FloatingBallState {
    Idle,
    Thinking,
    Listening,
    Speaking,
    Working { task_count: u32 },
}

pub struct NotificationService {
    #[cfg(not(feature = "headless"))]
    app: Option<AppHandle>,
    last_notified: Mutex<HashMap<String, i64>>,
    active_tasks: AtomicU32,
}

impl NotificationService {
    /// 创建 NotificationService。
    /// - 非 headless: 存储 AppHandle,用于系统通知和悬浮球状态。
    /// - headless: AppHandle 被忽略(字段不存在),所有通知方法为 no-op。
    pub fn new(
        #[cfg(not(feature = "headless"))] app: AppHandle,
        #[cfg(feature = "headless")] _app: tauri::AppHandle,
    ) -> Self {
        Self {
            #[cfg(not(feature = "headless"))]
            app: Some(app),
            last_notified: Mutex::new(HashMap::new()),
            active_tasks: AtomicU32::new(0),
        }
    }

    /// T-E-C-20: headless 模式构造器 — 无需 AppHandle。
    #[cfg(feature = "headless")]
    pub fn new_headless() -> Self {
        Self {
            last_notified: Mutex::new(HashMap::new()),
            active_tasks: AtomicU32::new(0),
        }
    }

    /// 非 headless 模式下也可创建无 AppHandle 的实例(测试用)。
    #[cfg(not(feature = "headless"))]
    pub fn new_headless() -> Self {
        Self {
            app: None,
            last_notified: Mutex::new(HashMap::new()),
            active_tasks: AtomicU32::new(0),
        }
    }
}

// ---------------------------------------------------------------------------
// T-E-C-20: headless 模式 — 所有通知方法为 no-op(日志代替弹窗)。
// ---------------------------------------------------------------------------
#[cfg(feature = "headless")]
impl NotificationService {
    pub fn notify_task_completed(&self, title: &str, task_id: &str) {
        debug!(target: "nebula.notify", title, task_id, "task completed (headless, no-op)");
    }

    pub fn notify_task_failed(&self, title: &str, task_id: &str, error: &str) {
        debug!(target: "nebula.notify", title, task_id, error, "task failed (headless, no-op)");
    }

    pub fn notify_approval_required(&self, title: &str, kind: &str) {
        debug!(target: "nebula.notify", title, kind, "approval required (headless, no-op)");
    }

    pub fn notify_im(&self, title: &str, body: &str) {
        debug!(target: "nebula.notify", title, body, "IM notification (headless, no-op)");
    }

    pub fn increment_active_tasks(&self) {}
    pub fn decrement_active_tasks(&self) {}
    pub fn active_task_count(&self) -> u32 {
        0
    }

    pub fn handle_swarm_event(&self, _event: &SwarmEvent) {}
}

// ---------------------------------------------------------------------------
// GUI 模式 — 完整通知实现。
// ---------------------------------------------------------------------------
#[cfg(not(feature = "headless"))]
impl NotificationService {
    pub fn notify_task_completed(&self, title: &str, task_id: &str) {
        let key = format!("completed:{}", task_id);
        if !self.should_notify(&key) {
            return;
        }

        let config_enabled = self
            .app
            .as_ref()
            .and_then(|app| app.try_state::<crate::AppState>())
            .map(|s| s.infra.config.notifications_enabled)
            .unwrap_or(true);

        if config_enabled {
            let body = format!("任务已完成: {}", title);
            self.send_system_notification("任务完成", &body);
        }

        self.update_floating_ball_state();
    }

    pub fn notify_task_failed(&self, title: &str, task_id: &str, error: &str) {
        let key = format!("failed:{}", task_id);
        if !self.should_notify(&key) {
            return;
        }

        let config_enabled = self
            .app
            .as_ref()
            .and_then(|app| app.try_state::<crate::AppState>())
            .map(|s| s.infra.config.notifications_enabled)
            .unwrap_or(true);

        if config_enabled {
            let body = format!("任务失败: {}\n错误: {}", title, error);
            self.send_system_notification("任务失败", &body);
        }

        self.update_floating_ball_state();
    }

    pub fn notify_approval_required(&self, title: &str, kind: &str) {
        let key = format!("approval:{}", kind);
        if !self.should_notify(&key) {
            return;
        }

        let config_enabled = self
            .app
            .as_ref()
            .and_then(|app| app.try_state::<crate::AppState>())
            .map(|s| s.infra.config.notifications_enabled)
            .unwrap_or(true);

        if config_enabled {
            let body = format!("需要您的审批: {} ({})", title, kind);
            self.send_system_notification("需要审批", &body);
        }
    }

    /// T-E-C-17: 通过 IM webhook 广播通知(Feishu/WeCom/DingTalk)。
    pub fn notify_im(&self, title: &str, body: &str) {
        let key = format!("im:{}", title);
        if !self.should_notify(&key) {
            return;
        }

        let Some(app) = self.app.clone() else {
            return;
        };
        let title_owned = title.to_string();
        let body_owned = body.to_string();
        tauri::async_runtime::spawn(async move {
            if let Some(state) = app.try_state::<crate::AppState>() {
                let message = crate::im::ImMessage::new(title_owned, body_owned);
                let (success, failure) = state.channels.im_engine.broadcast(message).await;
                tracing::debug!(
                    target: "nebula.notify",
                    success, failure,
                    "notify_im broadcast completed"
                );
            }
        });
    }

    pub fn increment_active_tasks(&self) {
        self.active_tasks.fetch_add(1, Ordering::SeqCst);
        self.update_floating_ball_state();
    }

    pub fn decrement_active_tasks(&self) {
        let prev = self.active_tasks.fetch_sub(1, Ordering::SeqCst);
        if prev == 0 {
            self.active_tasks.store(0, Ordering::SeqCst);
        }
        self.update_floating_ball_state();
    }

    pub fn active_task_count(&self) -> u32 {
        self.active_tasks.load(Ordering::SeqCst)
    }

    fn update_floating_ball_state(&self) {
        let badge_enabled = self
            .app
            .as_ref()
            .and_then(|app| app.try_state::<crate::AppState>())
            .map(|s| s.infra.config.floating_ball_task_badge)
            .unwrap_or(true);

        let count = self.active_tasks.load(Ordering::SeqCst);
        let state = if count > 0 && badge_enabled {
            FloatingBallState::Working { task_count: count }
        } else {
            FloatingBallState::Idle
        };

        if let Some(ref app) = self.app {
            if let Err(e) = app.emit("nebula://floating-ball-state", &state) {
                debug!(target: "nebula.notify", error = %e, "failed to emit floating-ball-state");
            }
        }
    }

    fn should_notify(&self, key: &str) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        let mut map = self.last_notified.lock();

        if let Some(&last) = map.get(key) {
            if now - last < DEDUPLICATION_WINDOW_MS {
                return false;
            }
        }

        map.insert(key.to_string(), now);
        true
    }

    fn send_system_notification(&self, title: &str, body: &str) {
        #[cfg(feature = "custom-protocol")]
        {
            let _ = title;
            let _ = body;
            return;
        }

        let Some(ref app) = self.app else {
            return;
        };

        let result = app.notification().builder().title(title).body(body).show();

        if let Err(e) = result {
            warn!(target: "nebula.notify", error = %e, "failed to show system notification");
            if let Err(e2) = app.emit(
                "nebula://toast-notification",
                serde_json::json!({
                    "title": title,
                    "body": body,
                }),
            ) {
                debug!(target: "nebula.notify", error = %e2, "failed to emit toast fallback");
            }
        }
    }

    pub fn handle_swarm_event(&self, event: &SwarmEvent) {
        match event {
            SwarmEvent::AgentStarted { task_id, .. } => {
                self.increment_active_tasks();
                let _ = task_id;
            }
            SwarmEvent::AgentCompleted {
                task_id,
                success,
                error,
                agent_kind,
                ..
            } => {
                self.decrement_active_tasks();
                if !success {
                    let err_msg = error.clone().unwrap_or_else(|| "未知错误".to_string());
                    let title = format!("{:?} 代理", agent_kind);
                    self.notify_task_failed(&title, task_id, &err_msg);
                }
            }
            SwarmEvent::SwarmCompleted {
                task_id,
                success_count,
                failure_count,
                ..
            } => {
                if *failure_count == 0 && *success_count > 0 {
                    let title = format!("任务完成 ({} 个成功)", success_count);
                    self.notify_task_completed(&title, task_id);
                } else if *failure_count > 0 {
                    let title = format!(
                        "任务部分失败 ({} 成功 / {} 失败)",
                        success_count, failure_count
                    );
                    self.notify_task_failed(&title, task_id, "部分代理执行失败");
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    struct TestDeduplicator {
        last_notified: Mutex<HashMap<String, i64>>,
    }

    impl TestDeduplicator {
        fn new() -> Self {
            Self {
                last_notified: Mutex::new(HashMap::new()),
            }
        }

        fn should_notify(&self, key: &str) -> bool {
            let now = chrono::Utc::now().timestamp_millis();
            let mut map = self.last_notified.lock();

            if let Some(&last) = map.get(key) {
                if now - last < DEDUPLICATION_WINDOW_MS {
                    return false;
                }
            }

            map.insert(key.to_string(), now);
            true
        }
    }

    #[test]
    fn test_deduplication_same_key_within_window() {
        let dedup = TestDeduplicator::new();
        let key = "test:dedup";

        assert!(dedup.should_notify(key));
        assert!(!dedup.should_notify(key));
    }

    #[test]
    fn test_deduplication_different_keys() {
        let dedup = TestDeduplicator::new();

        assert!(dedup.should_notify("key1"));
        assert!(dedup.should_notify("key2"));
    }

    #[test]
    fn test_active_tasks_increment_decrement() {
        let counter = AtomicU32::new(0);

        assert_eq!(counter.load(Ordering::SeqCst), 0);

        counter.fetch_add(1, Ordering::SeqCst);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        counter.fetch_add(1, Ordering::SeqCst);
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        counter.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        counter.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_active_tasks_no_negative() {
        let counter = AtomicU32::new(0);

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        let prev = counter.fetch_sub(1, Ordering::SeqCst);
        if prev == 0 {
            counter.store(0, Ordering::SeqCst);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_floating_ball_state_serialization() {
        let idle = FloatingBallState::Idle;
        let json = serde_json::to_string(&idle).unwrap();
        assert!(json.contains("\"state\":\"idle\""));

        let working = FloatingBallState::Working { task_count: 3 };
        let json = serde_json::to_string(&working).unwrap();
        assert!(json.contains("\"state\":\"working\""));
        assert!(json.contains("\"task_count\":3"));
    }
}
