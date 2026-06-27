//! v0.5: OS integration — clipboard, shell, notifications.
//!
//! The actual Tauri plugin wiring (system tray, autostart, global
//! shortcut) lives in `lib.rs::run` because those need an
//! `AppHandle`.  This module is the pure backend facade.

pub mod clipboard;
pub mod notifications;
pub mod shell;

pub use clipboard::ClipboardService;
pub use notifications::{send as send_notification, Notification, NotificationLevel};
pub use shell::{parse_argv, ShellExecutor, ShellOutput, DEFAULT_TIMEOUT};
