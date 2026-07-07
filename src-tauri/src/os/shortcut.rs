//! v1.7: 全局快捷键接线。
//!
//! 设计文档 v7.0 §6 OS 集成 — 全局快捷键表（第四轮 line 366-376）。
//!
//! Phase 6 实现的快捷键（P0 优先级）：
//! * `Cmd/Ctrl+Shift+H` — 唤起/隐藏主窗口（toggle）
//! * `Cmd/Ctrl+Shift+M` — 切换到记忆画布
//! * `Cmd/Ctrl+Shift+S` — 切换到蜂群视图
//! * `Cmd/Ctrl+Q` — 真正退出（不是最小化）
//!
//! 未实现（放后续版本）：
//! * `Cmd/Ctrl+Shift+Space` — 快速输入（Raycast 模式，需独立小窗）
//! * `Cmd/Ctrl+1/2/3` — 切换写作/工作/Code 三视角（需前端配合）
//!
//! ## 冲突处理
//!
//! 快捷键可能被其他应用占用（如 macOS 搜狗输入法占用 Shift+Space）。
//! 注册失败时记录 warn 日志但不阻断启动——用户仍可通过托盘/界面操作。

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 注册所有全局快捷键。在 `setup` 闭包中调用。
pub fn setup(app: &AppHandle) {
    register(app, "CmdOrCtrl+Shift+H", on_toggle_main_window);
    register(app, "CmdOrCtrl+Shift+M", on_switch_to_memory);
    register(app, "CmdOrCtrl+Shift+S", on_switch_to_swarm);
    register(app, "CmdOrCtrl+Q", on_quit);
}

fn register<F>(app: &AppHandle, accel: &str, handler: F)
where
    F: Fn(&AppHandle) + Send + Sync + 'static,
{
    let shortcut: Shortcut = match accel.parse() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "nebula.os.shortcut",
                accel,
                error = %e,
                "failed to parse shortcut; skipping"
            );
            return;
        }
    };

    let app_clone = app.clone();
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_app, _sc, event| {
            // 只在按下（KeyDown）时触发，避免 Up/Down 双触发
            if event.state == ShortcutState::Pressed {
                handler(&app_clone);
            }
        })
    {
        tracing::warn!(
            target: "nebula.os.shortcut",
            accel,
            error = %e,
            "failed to register shortcut; it may be occupied by another app"
        );
    } else {
        tracing::info!(target: "nebula.os.shortcut", accel, "shortcut registered");
    }
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

/// Cmd/Ctrl+Shift+H：切换主窗口显隐。
fn on_toggle_main_window(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        if w.is_visible().unwrap_or(false) {
            // 已可见 → 聚焦；如果已聚焦则隐藏（toggle 语义）
            // 简化版：始终聚焦，不隐藏（隐藏用托盘）
            let _ = w.set_focus();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

/// Cmd/Ctrl+Shift+M：切换到记忆画布。
/// 通过 emit 事件让前端切换 view，避免直接操作前端状态。
fn on_switch_to_memory(app: &AppHandle) {
    // 先确保窗口可见
    if let Some(w) = main_window(app) {
        if !w.is_visible().unwrap_or(false) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
    let _ = app.emit("nebula://switch-view", "memory");
}

/// Cmd/Ctrl+Shift+S：切换到蜂群视图。
fn on_switch_to_swarm(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        if !w.is_visible().unwrap_or(false) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
    let _ = app.emit("nebula://switch-view", "swarm");
}

/// Cmd/Ctrl+Q：真正退出（绕过"关闭=最小化到托盘"）。
fn on_quit(app: &AppHandle) {
    tracing::info!(target: "nebula.os.shortcut", "user pressed Cmd+Q; exiting");
    app.exit(0);
}
