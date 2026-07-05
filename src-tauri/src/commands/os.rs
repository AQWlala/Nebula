//! OS commands — clipboard, shell, notify.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::os::{self, Notification, NotificationLevel};
use crate::AppState;

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "os_clipboard_read"))]
pub async fn os_clipboard_read(state: State<'_, AppState>) -> Result<String, CommandError> {
    state
        .clipboard
        .read_text()
        .map_err(|e| CommandError::internal("os_clipboard_read", &e))
}

#[tauri::command]
#[instrument(skip(state, text), fields(otel.kind = "os_clipboard_write"))]
pub async fn os_clipboard_write(
    state: State<'_, AppState>,
    text: String,
) -> Result<(), CommandError> {
    state
        .clipboard
        .write_text(&text)
        .map_err(|e| CommandError::internal("os_clipboard_write", &e))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellExecRequest {
    /// Either a parsed argv array or a single string to be split
    /// via `shell-words`.  Callers SHOULD prefer the array form.
    pub argv: Option<Vec<String>>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "os_shell_exec"))]
pub async fn os_shell_exec(
    state: State<'_, AppState>,
    request: ShellExecRequest,
) -> Result<os::ShellOutput, CommandError> {
    let argv: Vec<String> = if let Some(arr) = request.argv {
        arr
    } else if let Some(cmd) = request.command {
        os::parse_argv(&cmd)
            .map_err(|e| CommandError::validation("os_shell_exec").with_details(e.to_string()))?
    } else {
        return Err(CommandError::validation("os_shell_exec")
            .with_details("argv or command is required".to_string()));
    };
    let cwd: Option<PathBuf> = request.cwd.map(PathBuf::from);
    let shell = state.shell.clone();
    let timeout = request.timeout_ms.map(std::time::Duration::from_millis);
    // v1.0.1 P0#3: `ShellExecutor::exec` is now `async` so the
    // timeout branch can `start_kill()` the child.  No more
    // `spawn_blocking`.
    let exec = if let Some(t) = timeout {
        (*shell).clone().with_timeout(t)
    } else {
        (*shell).clone()
    };
    exec.exec(argv, cwd.as_deref())
        .await
        .map_err(|e| CommandError::validation("os_shell_exec").with_details(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    pub level: Option<String>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "os_notify"))]
pub async fn os_notify(
    state: State<'_, AppState>,
    app: AppHandle,
    request: NotifyRequest,
) -> Result<(), CommandError> {
    let _ = state;
    let level = match request.level.as_deref() {
        Some("success") => NotificationLevel::Success,
        Some("warning") => NotificationLevel::Warning,
        Some("error") => NotificationLevel::Error,
        _ => NotificationLevel::Info,
    };
    let n = Notification {
        title: request.title,
        body: request.body,
        level,
    };
    // v1.7: 先记录到 in-process 日志（保持向后兼容），再通过
    // tauri-plugin-notification 真正发送 OS 通知。
    os::send_notification(&n)?;
    app.notification()
        .builder()
        .title(&n.title)
        .body(&n.body)
        .show()
        .map_err(|e| CommandError::internal("os_notify", &anyhow::anyhow!("{e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// v1.7: 自启动控制命令（前端 Settings 页面 toggle 用）。
// ---------------------------------------------------------------------------

/// Tauri 命令：启用开机自启动。
#[tauri::command]
#[instrument(skip(app), fields(otel.kind = "os_autostart_enable"))]
pub async fn os_autostart_enable(app: AppHandle) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .enable()
        .map_err(|e| CommandError::internal("os_autostart_enable", &anyhow::anyhow!("{e}")))?;
    tracing::info!(target: "nebula.os", "autostart enabled");
    Ok(())
}

/// Tauri 命令：禁用开机自启动。
#[tauri::command]
#[instrument(skip(app), fields(otel.kind = "os_autostart_disable"))]
pub async fn os_autostart_disable(app: AppHandle) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .disable()
        .map_err(|e| CommandError::internal("os_autostart_disable", &anyhow::anyhow!("{e}")))?;
    tracing::info!(target: "nebula.os", "autostart disabled");
    Ok(())
}

/// Tauri 命令：查询当前自启动状态。
#[tauri::command]
#[instrument(skip(app), fields(otel.kind = "os_autostart_is_enabled"))]
pub async fn os_autostart_is_enabled(app: AppHandle) -> Result<bool, CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    let enabled = app
        .autolaunch()
        .is_enabled()
        .map_err(|e| CommandError::internal("os_autostart_is_enabled", &anyhow::anyhow!("{e}")))?;
    Ok(enabled)
}

// ---------------------------------------------------------------------------
// T-E-C-02: ScreenReader 截图命令 — 捕获主屏并返回 base64 PNG。
// ---------------------------------------------------------------------------

/// T-E-C-02: 截取主屏并返回 base64 编码的 PNG。
///
/// 启用方式: `cargo build --features vision`(或前端 toggle 启用 vision feature)。
/// screenshots crate 用于屏幕捕获, image crate 用于 PNG 编码,
/// base64 已是非 optional 依赖(0.22),直接复用。
///
/// 失败场景:
/// - vision feature 未启用 → 返回错误信息提示用户重新编译。
/// - 无显示器(SSH/headless) → "no display available"。
/// - 屏幕捕获失败(权限被拒) → e.to_string()。
///
/// 注:只截主屏(Screen::all().next()),多屏 / 区域选择见 spec §2 Out of scope。
#[tauri::command]
#[instrument(fields(otel.kind = "screenshot"))]
pub async fn screenshot() -> Result<String, String> {
    #[cfg(feature = "vision")]
    {
        use base64::Engine;
        use screenshots::Screen;
        // 收集所有显示器,取第一个作为主屏。
        let screens = Screen::all().map_err(|e| e.to_string())?;
        let screen = screens
            .into_iter()
            .next()
            .ok_or("no display available".to_string())?;
        // 截图 — 在 Windows 上调用 GDI,在 macOS 上调用 CGDisplay,在 Linux 上调用 X11。
        // screenshots 0.8 的 `Screen::capture` 返回 `screenshots::Image`,它是
        // `image::ImageBuffer<Rgba<u8>, Vec<u8>>` 的类型别名(已是 RgbaImage)。
        // 但 screenshots 可能依赖不同版本的 image crate,所以用 `as_raw()` 取
        // 原始像素字节(&Vec<u8>),clone 后用我们自己的 image 0.25 重建 RgbaImage,
        // 这样跨 image crate 版本也能工作。
        let img = screen.capture().map_err(|e| e.to_string())?;
        let rgba: image::RgbaImage = image::ImageBuffer::from_raw(
            img.width(),
            img.height(),
            img.as_raw().clone(),
        )
        .ok_or("failed to construct RgbaImage from raw RGBA bytes".to_string())?;
        // 编码为 PNG,再 base64 包装。
        let mut buf = std::io::Cursor::new(Vec::<u8>::new());
        rgba.write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let png_bytes = buf.into_inner();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        Ok(b64)
    }
    #[cfg(not(feature = "vision"))]
    {
        // vision feature 未启用时,返回明确错误信息(非 panic,前端可处理)。
        Err("vision feature not enabled, rebuild with --features vision".to_string())
    }
}

#[cfg(test)]
mod screenshot_tests {
    use super::screenshot;

    /// T-E-C-02: vision feature 未启用时 screenshot 命令应返回错误信息,
    /// 而非 panic。vision feature 启用时此测试仍编译(返回 Ok 或真实截图)。
    #[tokio::test]
    async fn test_screenshot_command_feature_gated() {
        let result = screenshot().await;
        #[cfg(not(feature = "vision"))]
        {
            // 非 vision feature:必须返回错误信息。
            let err = result.expect_err("non-vision feature must return error");
            assert!(
                err.contains("vision feature not enabled"),
                "unexpected error message: {err}"
            );
        }
        #[cfg(feature = "vision")]
        {
            // vision feature 启用时:测试环境可能无显示器(SSH/CI),
            // 此时返回 Err 是合理的;若能截图,则返回 base64 字符串。
            match result {
                Ok(b64) => {
                    // base64 字符串长度应 > 0 且不含 data: 前缀(纯 base64)。
                    assert!(!b64.is_empty(), "base64 png should not be empty");
                    assert!(!b64.starts_with("data:"), "should be raw base64, not data URL");
                }
                Err(_) => {
                    // 测试环境无显示器或权限被拒 — 接受。
                }
            }
        }
    }
}
