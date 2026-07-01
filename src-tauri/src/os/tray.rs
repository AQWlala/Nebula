//! v1.7: 系统托盘（最简版）。
//!
//! 设计文档 v7.0 §6 OS 集成 — 托盘常驻九头蛇 logo。
//!
//! v1.7 实现范围（最简版）：
//! * 静态图标（不动态旋转/闪烁，动态效果放 v1.5+）
//! * 右键菜单：显示主窗口 / 隐藏到托盘 / 退出
//! * 左键单击：切换主窗口显隐
//!
//! 会议结论（第四轮桌面端架构设计 line 594）：v1.0 托盘最简版，
//! 动态效果放 v1.5。Phase 6 沿用此结论。
//!
//! ## 关闭窗口 = 最小化到托盘
//!
//! 用户点关闭按钮时，不退出应用，而是隐藏到托盘。这是"常驻"
//! 体验的核心。退出只能通过托盘菜单的"退出"或快捷键 Cmd+Q。

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};

/// 托盘菜单 item id。
const MENU_SHOW: &str = "tray_show";
const MENU_HIDE: &str = "tray_hide";
const MENU_QUIT: &str = "tray_quit";

/// 初始化系统托盘。在 `setup` 闭包中调用。
///
/// 失败时仅记录日志，不阻断启动——托盘是锦上添花，不是核心。
pub fn setup(app: &AppHandle) {
    if let Err(e) = try_setup(app) {
        tracing::warn!(
            target: "nine_snake.os.tray",
            error = %e,
            "system tray setup failed; continuing without tray"
        );
    }
}

fn try_setup(app: &AppHandle) -> anyhow::Result<()> {
    let show_item = MenuItem::with_id(app, MENU_SHOW, "显示主窗口", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, MENU_HIDE, "隐藏到托盘", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "退出九头蛇", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &hide_item, &quit_item])?;

    TrayIconBuilder::with_id("nine-snake-tray")
        .tooltip("九头蛇 · nine-snake")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => {
                if let Some(w) = main_window(app) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            MENU_HIDE => {
                if let Some(w) = main_window(app) {
                    let _ = w.hide();
                }
            }
            MENU_QUIT => {
                tracing::info!(target: "nine_snake.os.tray", "user clicked quit; exiting");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击切换显隐
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = main_window(app) {
                    if w.is_visible().unwrap_or(false) {
                        let _ = w.hide();
                    } else {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    tracing::info!(target: "nine_snake.os.tray", "system tray initialized");
    Ok(())
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}
