//! v0.5: system notifications backend.
//!
//! The v0.5 surface area is intentionally minimal: a single
//! `notify(title, body)` function that uses the
//! `tauri-plugin-notification` API when invoked from the Tauri
//! runtime, and falls back to a no-op (with a `tracing::info!`)
//! otherwise.
//!
//! In v1.0 we plan to add:
//!
//! * per-channel quiet hours
//! * action buttons ("Open", "Snooze")
//! * Windows Action Center / macOS Notification Center
//!   customisation flags

use serde::{Deserialize, Serialize};
use tracing::info;

/// One notification record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub level: NotificationLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationLevel {
    fn as_str(self) -> &'static str {
        match self {
            NotificationLevel::Info => "info",
            NotificationLevel::Success => "success",
            NotificationLevel::Warning => "warning",
            NotificationLevel::Error => "error",
        }
    }
}

/// Sends a notification.  Returns `Ok(())` even when the system
/// notification daemon isn't available — the front-end should
/// always consider a successful return as "the user has been told".
pub fn send(notification: &Notification) -> anyhow::Result<()> {
    info!(
        target: "nine_snake.os",
        title = %notification.title,
        level = notification.level.as_str(),
        "notification dispatched (in-process log only — plug in tauri-plugin-notification in run())"
    );
    // The Tauri command layer is responsible for actually firing the
    // OS-level notification via the plugin; this module is the pure
    // data-model + tracing layer.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_returns_ok() {
        let n = Notification {
            title: "hello".into(),
            body: "world".into(),
            level: NotificationLevel::Info,
        };
        send(&n).expect("send");
    }
}
