//! v0.5: OS integration — clipboard, shell, notifications.
//!
//! The actual Tauri plugin wiring (autostart, global shortcut) lives
//! in `lib.rs::run` because those need an `AppHandle`.  This module
//! is the pure backend facade, plus v1.7 `tray` which is AppHandle-
//! aware.

pub mod clipboard;
// v1.7: 文件关联 + OS 文件拖入处理。
pub mod file_handler;
pub mod notifications;
pub mod shell;
// v1.7: 全局快捷键接线（需要 AppHandle）。
pub mod shortcut;
// v1.7: 系统托盘（需要 AppHandle，所以放在 os 模块下而非 lib.rs）。
pub mod tray;

pub use clipboard::ClipboardService;
pub use notifications::{send as send_notification, Notification, NotificationLevel};
pub use shell::{parse_argv, ShellExecutor, ShellOutput, DEFAULT_TIMEOUT};
