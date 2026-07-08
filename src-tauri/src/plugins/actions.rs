//! T-E-S-35: Action 层示例 — `NotifyAction`。
//!
//! 包装 `os::notifications::send`,UI 按钮触发的一次性通知动作。
//! Action 与 Agent 解耦:无状态、不持有 TeamContext、不参与多轮对话。
//!
//! 参数(JSON):
//! - `title`(可选,默认 "nebula")
//! - `body`(可选,默认空串)
//! - `level`(可选,info|success|warning|error,默认 info)

use anyhow::Result;
use async_trait::async_trait;

use crate::os::notifications::{send, Notification, NotificationLevel};

use super::traits::{Action, ActionOutput};

/// 一次性通知动作。包装 `os::notifications::send`。
pub struct NotifyAction;

impl Default for NotifyAction {
    fn default() -> Self {
        Self
    }
}

impl NotifyAction {
    pub fn new() -> Self {
        Self
    }

    fn parse_level(s: Option<&str>) -> NotificationLevel {
        match s {
            Some("success") => NotificationLevel::Success,
            Some("warning") => NotificationLevel::Warning,
            Some("error") => NotificationLevel::Error,
            _ => NotificationLevel::Info,
        }
    }
}

#[async_trait]
impl Action for NotifyAction {
    fn name(&self) -> &str {
        "notify"
    }

    fn description(&self) -> &str {
        "Send a system notification. Params: title (optional), body (optional), \
         level (optional: info|success|warning|error)."
    }

    async fn invoke(&self, params: serde_json::Value) -> Result<ActionOutput> {
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("nebula")
            .to_string();
        let body = params
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let level = Self::parse_level(params.get("level").and_then(|v| v.as_str()));

        let notification = Notification { title, body, level };
        match send(&notification) {
            Ok(()) => Ok(ActionOutput::ok("notification sent")),
            Err(e) => Ok(ActionOutput::err(format!("notify failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn notify_returns_success() {
        let action = NotifyAction::new();
        let out = action
            .invoke(serde_json::json!({"title": "hi", "body": "world", "level": "info"}))
            .await
            .expect("test op should succeed");
        assert!(out.success, "message: {}", out.message);
        assert!(out.message.contains("sent"));
    }

    #[tokio::test]
    async fn notify_uses_defaults_when_params_missing() {
        let action = NotifyAction::new();
        let out = action.invoke(serde_json::json!({})).await.expect("task should complete");
        assert!(out.success);
    }

    #[test]
    fn parse_level_maps_known_strings() {
        assert_eq!(
            NotifyAction::parse_level(Some("info")),
            NotificationLevel::Info
        );
        assert_eq!(
            NotifyAction::parse_level(Some("success")),
            NotificationLevel::Success
        );
        assert_eq!(
            NotifyAction::parse_level(Some("warning")),
            NotificationLevel::Warning
        );
        assert_eq!(
            NotifyAction::parse_level(Some("error")),
            NotificationLevel::Error
        );
        assert_eq!(NotifyAction::parse_level(None), NotificationLevel::Info);
        assert_eq!(
            NotifyAction::parse_level(Some("bogus")),
            NotificationLevel::Info
        );
    }
}
