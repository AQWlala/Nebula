//! v0.5: OS integration — clipboard, shell, notifications.
//!
//! The actual Tauri plugin wiring (autostart, global shortcut) lives
//! in `lib.rs::run` because those need an `AppHandle`.  This module
//! is the pure backend facade, plus v1.7 `tray` which is AppHandle-
//! aware.

pub mod clipboard;
// T-E-C-14: 剪贴板智能监听 — 后台轮询 + 内容检测 + sponge 吸收。
pub mod clipboard_watcher;
// T-S6-A-01a: OS-Controller — Windows 窗口管理 / 菜单操作 / 输入模拟。
pub mod controller;
// T-E-D-06: Windows 右键菜单 "问Nebula" 注册表读写(HKCU)。
pub mod context_menu;
// v1.7: 文件关联 + OS 文件拖入处理。
pub mod file_handler;
pub mod notifications;
// T-S6-A-02: 电源管理 — 监听系统睡眠/唤醒,暂停/恢复 LLM 与蜂群任务。
pub mod power;
pub mod shell;
// v1.7: 全局快捷键接线（需要 AppHandle）。
pub mod shortcut;
// v1.7: 系统托盘（需要 AppHandle，所以放在 os 模块下而非 lib.rs）。
pub mod tray;
// T-E-C-03: UiAutomator 抽象层 — trait-based UI 自动化抽象。
pub mod uiautomator;
// T-E-C-01: OS-Controller VLM 模式 — 截图→VLM 分析→操作执行闭环。
pub mod controller_vlm;
// T-E-C-04: ActionExecutor — 统一 UI 动作执行器。
pub mod action_executor;
// T-E-C-11: 操作录制回放 — UI 操作录制 + 回放。
pub mod action_recorder;
// T-E-C-12: Design Mode — 可视化设计界面创建自动化流程。
pub mod design_mode;
// T-E-C-07: Remote Operator — 远程操作器,通过远程连接控制 OS-Controller Sidecar。
pub mod remote_operator;

pub use clipboard::ClipboardService;
// T-E-C-14: 剪贴板监听引擎 re-export。
pub use clipboard_watcher::{ClipboardEvent, ClipboardKind, ClipboardWatcherEngine};
pub use controller::{OsControllerService, WindowInfo};
pub use notifications::{send as send_notification, Notification, NotificationLevel};
pub use power::{PowerManager, PowerState};
pub use shell::{parse_argv, ShellExecutor, ShellOutput, DEFAULT_TIMEOUT};
