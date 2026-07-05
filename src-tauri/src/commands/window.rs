//! T-S5-B-01: 浮动窗 / 画中画 — 打开独立浮动聊天窗口的命令。
//!
//! 浮动窗通过 `WebviewWindowBuilder` 运行时创建,不在 `tauri.conf.json`
//! 的 `windows` 数组中预声明。它复用主窗口的前端 (devUrl / frontendDist),
//! 仅通过 URL 查询参数 `?view=floating` 区分视图 (见 `main.tsx` 路由)。

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tracing::instrument;

/// 浮动聊天窗口的 label,用于去重与 set_focus。
const FLOATING_CHAT_LABEL: &str = "floating-chat";

/// T-E-D-03: 桌面悬浮球的 label,用于去重与 toggle 显隐。
pub const FLOATING_BALL_LABEL: &str = "floating-ball";

/// T-E-D-07: 浮动进度窗的 label,用于去重与刷新。
pub const FLOATING_PROGRESS_LABEL: &str = "floating-progress";

/// 打开浮动聊天窗口 (PIP 风格)。
///
/// - 若窗口已存在,则聚焦已存在的窗口而非重复创建。
/// - 否则用 `WebviewWindowBuilder` 创建新窗口:
///   - 无边框、置顶、半透明、不在任务栏显示
///   - 内部尺寸 380x560
///   - URL 拼接 `?view=floating` 查询参数,加载同一前端的浮动视图
///   - center: false — 避免遮挡主窗口
///
/// 返回 `Result<(), String>` 用于向前端传递错误信息。
#[tauri::command]
#[instrument(skip(app), fields(otel.kind = "open_floating_chat"))]
pub async fn open_floating_chat(app: tauri::AppHandle) -> Result<(), String> {
    // 若窗口已存在,直接聚焦而非重复创建
    if let Some(win) = app.get_webview_window(FLOATING_CHAT_LABEL) {
        win.set_focus()
            .map_err(|e| format!("set_focus failed: {e}"))?;
        return Ok(());
    }

    // 构造浮动窗 URL — 复用主窗口前端,通过 query 参数区分视图。
    // dev 模式下解析为 <devUrl>/?view=floating,
    // prod 模式下解析为 <asset>/index.html?view=floating。
    let url = WebviewUrl::App("/?view=floating".into());

    WebviewWindowBuilder::new(&app, FLOATING_CHAT_LABEL, url)
        .title("Nebula · 浮动")
        .inner_size(380.0, 560.0)
        .decorations(false)
        .always_on_top(true)
        .resizable(true)
        .transparent(true)
        .skip_taskbar(true)
        .build()
        .map_err(|e| format!("failed to build floating-chat window: {e}"))?;

    Ok(())
}

/// T-E-D-03: 打开桌面悬浮球 (80x80 状态指示器,240x240 透明窗口)。
///
/// - 若窗口已存在,则 toggle 显隐 (可见→hide / 隐藏→show),**不抢焦点**。
/// - 否则用 `WebviewWindowBuilder` 创建新窗口:
///   - 无边框、置顶、透明、不在任务栏显示
///   - 内部尺寸 240x240(透明窗口,球本身 80x80 居中,留出菜单展开空间)
///   - URL 拼接 `?view=ball` 查询参数,加载同一前端的悬浮球视图
///
/// 与 `open_floating_chat` 的差异:不可缩放、toggle 而非 set_focus。
#[tauri::command]
#[instrument(skip(app), fields(otel.kind = "open_floating_ball"))]
pub async fn open_floating_ball(app: tauri::AppHandle) -> Result<(), String> {
    // 已存在时 toggle 显隐 (球不应抢焦点)
    if let Some(win) = app.get_webview_window(FLOATING_BALL_LABEL) {
        if win.is_visible().unwrap_or(false) {
            win.hide().map_err(|e| format!("hide ball failed: {e}"))?;
        } else {
            win.show().map_err(|e| format!("show ball failed: {e}"))?;
        }
        return Ok(());
    }

    // 构造悬浮球 URL — 复用主窗口前端,通过 query 参数区分视图。
    let url = WebviewUrl::App("/?view=ball".into());

    WebviewWindowBuilder::new(&app, FLOATING_BALL_LABEL, url)
        .title("Nebula · 悬浮球")
        .inner_size(240.0, 240.0)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .transparent(true)
        .skip_taskbar(true)
        .build()
        .map_err(|e| format!("failed to build floating-ball window: {e}"))?;

    Ok(())
}

/// T-E-D-07: 打开浮动进度窗 (360x180,右下角置顶透明)。
///
/// 长任务执行时显示进度条 + 中断按钮,不阻塞用户操作主窗口。
/// - 若窗口已存在,先 close 再重建(刷新 task_id + title)。
/// - 用 `WebviewWindowBuilder` 创建新窗口:
///   - 无边框、置顶、透明、不在任务栏显示、不可缩放
///   - 内部尺寸 360x180
///   - 定位:主显示器右下角(x = monitor_w - 380, y = monitor_h - 220)
///   - URL 拼接 `?view=progress&taskId=<id>&title=<encoded>` 查询参数
///
/// `task_id` 由前端从 SwarmEvent 流中匹配;`title` 为可选的任务标题,
/// 缺省为 "任务执行中"。
#[tauri::command]
#[instrument(skip(app), fields(otel.kind = "open_floating_progress"))]
pub async fn open_floating_progress(
    app: tauri::AppHandle,
    task_id: String,
    title: Option<String>,
) -> Result<(), String> {
    // 若窗口已存在,先 close 再创建(刷新 task_id + title)。
    if let Some(win) = app.get_webview_window(FLOATING_PROGRESS_LABEL) {
        let _ = win.close();
        // 给 Tauri 一点时间回收窗口 label,避免重建时 label 冲突。
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let display_title = title.clone().unwrap_or_else(|| "任务执行中".to_string());

    // 构造浮动进度窗 URL — 复用主窗口前端,通过 query 参数区分视图 +
    // 传递 task_id / title。用 form_urlencoded 正确编码特殊字符,
    // 前端用 URLSearchParams 解析(form_urlencoded 的 + / %XX 都兼容)。
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("view", "progress")
        .append_pair("taskId", &task_id)
        .append_pair("title", &display_title)
        .finish();
    let url = WebviewUrl::App(format!("/?{query}").into());

    // 定位:主显示器右下角,留出 20px 边距 + 适应 360x180 窗口。
    // fallback (无显示器/取尺寸失败):不指定位置,Tauri 默认居中。
    let mut builder = WebviewWindowBuilder::new(&app, FLOATING_PROGRESS_LABEL, url)
        .title(display_title)
        .inner_size(360.0, 180.0)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .transparent(true)
        .skip_taskbar(true);

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let mon_size = monitor.size();
        let mon_pos = monitor.position();
        // 逻辑像素:物理尺寸 / scale_factor。Tauri 的 position/inner_size
        // 使用逻辑像素,需要除以 scale_factor 换算。
        let scale = monitor.scale_factor();
        let mon_w_logical = mon_size.width as f64 / scale;
        let mon_h_logical = mon_size.height as f64 / scale;
        let mon_x_logical = mon_pos.x as f64 / scale;
        let mon_y_logical = mon_pos.y as f64 / scale;
        // 右下角:monitor 右边 - 380(窗口宽 360 + 20 边距),
        // 底部 - 220(窗口高 180 + 40 边距,避开任务栏)。
        let x = mon_x_logical + mon_w_logical - 380.0;
        let y = mon_y_logical + mon_h_logical - 220.0;
        builder = builder.position(x, y);
    }

    builder
        .build()
        .map_err(|e| format!("failed to build floating-progress window: {e}"))?;

    Ok(())
}
