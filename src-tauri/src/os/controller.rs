//! T-S6-A-01a: OS-Controller Windows — 窗口管理 / 菜单操作 / 输入模拟。
//! T-S6-A-01b: macOS — skeleton only(骨架占位,尚未接入 AppKit/CoreGraphics)。
//! T-S6-A-01c: Linux — skeleton only(骨架占位,尚未接入 X11/Wayland)。
//!
//! 当前实现: 基本窗口管理(读取前台窗口标题、列出所有可见窗口)。
//! macOS: skeleton only (T-S6-A-01b) — 仅占位,返回 `Err`,后续任务填充真实 API。
//! Linux: skeleton only (T-S6-A-01c) — 仅占位,返回 `Err`,需 X11/Wayland 集成。
//!
//! ## 架构
//!
//! 本模块是 OS-Controller 的纯后端实现,封装 `windows` crate 的 Win32 API
//! 调用。它被 [`OsControllerServiceHandler`](crate::sidecar::OsControllerServiceHandler)
//! 包装后,既可在 sidecar 进程中通过 gRPC 暴露,也可在进程内模式直接调用。
//!
//! ## 平台支持
//!
//! * Windows — 通过 `windows` crate 调用 Win32 API(完整实现)。
//! * macOS — skeleton only (T-S6-A-01b),返回 `Err` 占位,不调用真实 API。
//! * Linux — skeleton only (T-S6-A-01c),返回 `Err` 占位,需 X11/Wayland 集成。
//! * 其他平台 — 提供空实现(返回 `None` / 空 Vec),保证跨平台编译通过。
//!
//! 平台分发通过 `OsControllerService` 方法上的 `#[cfg(target_os = ...)]` 守卫完成,
//! Tauri 命令(`os_get_foreground_window` / `os_list_windows`)与 sidecar handler
//! 均调用这些方法,因此天然平台无关,无需在 lib.rs 中区分平台。
//!
//! ## 后续 TODO
//!
//! * 菜单操作 — 通过 UIAutomation 调用菜单项
//! * 输入模拟 — 模拟键盘/鼠标输入(SendInput)
//! * macOS 真实实现 — 接入 AppKit/CoreGraphics(NSWorkspace / CGWindowList)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// 窗口信息。
///
/// 平台无关结构体,Windows 与 macOS 共用。`hwnd` 字段在 macOS 下语义为
/// 窗口标识(将来映射到 `CGWindowID`),当前骨架阶段不使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    /// 窗口句柄(u64 以便 JSON 序列化)。
    pub hwnd: u64,
    /// 窗口标题。
    pub title: String,
    /// 进程 ID。
    pub process_id: u32,
    /// 是否为前台窗口。
    pub is_foreground: bool,
}

/// OS-Controller 服务 — 封装 Windows UIAutomation / Win32 API 调用。
///
/// 无状态服务,所有方法都是独立的平台 API 调用。
/// macOS 下为骨架占位,返回 `Err`。
pub struct OsControllerService;

impl OsControllerService {
    /// 创建新的 OS-Controller 服务实例。
    pub fn new() -> Self {
        Self
    }

    /// 健康检查 — 始终返回 Ok(若服务实例存在则可用)。
    pub async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// 获取前台窗口信息。
    ///
    /// 调用 `GetForegroundWindow` 获取当前前台窗口句柄,再读取标题与进程 ID。
    /// 若无前台窗口(如桌面处于焦点),返回 `Ok(None)`。
    #[cfg(target_os = "windows")]
    #[instrument(skip(self))]
    pub fn get_foreground_window(&self) -> Result<Option<WindowInfo>> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };

        // SAFETY: `GetForegroundWindow` 无参数,返回 HWND(可能为 null,空指针表示无前台窗口)。
        // `GetWindowThreadProcessId` 的第二个参数是 `&mut process_id`(栈上 u32 out-param),
        // 指针在同步返回前有效。两个 API 都不接受需要调用方保证生命周期的字符串/缓冲区指针。
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return Ok(None);
            }
            let title = get_window_title(hwnd);
            let mut process_id: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut process_id);
            Ok(Some(WindowInfo {
                hwnd: hwnd as usize as u64,
                title,
                process_id,
                is_foreground: true,
            }))
        }
    }

    /// macOS 平台 — T-S6-A-01b 骨架占位,尚未接入 AppKit/CoreGraphics。
    ///
    /// 返回 `Err` 表示 macOS 实现尚未完成;后续任务将填充真实 API 调用
    /// (如 `NSWorkspace.frontmostApplication` / `CGWindowListCopyWindowInfo`)。
    #[cfg(target_os = "macos")]
    #[instrument(skip(self))]
    pub fn get_foreground_window(&self) -> Result<Option<WindowInfo>> {
        Err(anyhow::anyhow!(
            "macOS OS-Controller not yet implemented; skeleton only"
        ))
    }

    /// Linux 平台 — T-S6-A-01c 骨架占位,尚未接入 X11/Wayland。
    ///
    /// 返回 `Err` 表示 Linux 实现尚未完成;后续任务将填充真实 API 调用
    /// (如 X11 `XGetInputFocus` / Wayland `xdg-desktop-portal`)。
    #[cfg(target_os = "linux")]
    #[instrument(skip(self))]
    pub fn get_foreground_window(&self) -> Result<Option<WindowInfo>> {
        Err(anyhow::anyhow!(
            "Linux OS-Controller not yet implemented; skeleton only (requires X11/Wayland integration)"
        ))
    }

    /// 其他平台 — 无前台窗口可读取,返回 `None`。
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    #[instrument(skip(self))]
    pub fn get_foreground_window(&self) -> Result<Option<WindowInfo>> {
        Ok(None)
    }

    /// 列出所有可见且有标题的窗口。
    ///
    /// 通过 `EnumWindows` 枚举顶层窗口,过滤掉不可见窗口与无标题窗口。
    #[cfg(target_os = "windows")]
    #[instrument(skip(self))]
    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;

        let mut windows: Vec<WindowInfo> = Vec::new();
        let lparam = &mut windows as *mut Vec<WindowInfo> as isize;
        // SAFETY: `lparam` 是 `&mut windows` 的原始指针,指向调用方栈上的 Vec。
        // Vec 在 `EnumWindows` 同步返回前保持存活;回调 `enum_windows_proc` 通过
        // 该指针把窗口信息 push 进 Vec。`EnumWindows` 是同步 API,返回后不再持有指针。
        unsafe {
            if EnumWindows(Some(enum_windows_proc), lparam) == 0 {
                return Err(anyhow::anyhow!("EnumWindows failed"));
            }
        }

        // 标记前台窗口
        let fg_hwnd = {
            use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            // SAFETY: `GetForegroundWindow` 无参数,仅返回一个 HWND 标量,不通过指针写入调用方内存。
            unsafe { GetForegroundWindow() as usize as u64 }
        };
        for w in &mut windows {
            if w.hwnd == fg_hwnd {
                w.is_foreground = true;
            }
        }

        Ok(windows)
    }

    /// macOS 平台 — T-S6-A-01b 骨架占位,尚未接入 AppKit/CoreGraphics。
    ///
    /// 返回 `Err` 表示 macOS 实现尚未完成;后续任务将填充真实 API 调用
    /// (如 `CGWindowListCopyWindowInfo` 枚举顶层窗口)。
    #[cfg(target_os = "macos")]
    #[instrument(skip(self))]
    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        Err(anyhow::anyhow!(
            "macOS OS-Controller not yet implemented; skeleton only"
        ))
    }

    /// Linux 平台 — T-S6-A-01c 骨架占位,尚未接入 X11/Wayland。
    ///
    /// 返回 `Err` 表示 Linux 实现尚未完成;后续任务将填充真实 API 调用
    /// (如 X11 `XQueryTree` 枚举顶层窗口)。
    #[cfg(target_os = "linux")]
    #[instrument(skip(self))]
    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        Err(anyhow::anyhow!(
            "Linux OS-Controller not yet implemented; skeleton only (requires X11/Wayland integration)"
        ))
    }

    /// 其他平台 — 无窗口可列出,返回空 Vec。
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    #[instrument(skip(self))]
    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(Vec::new())
    }

    /// TODO(T-S6-A-01a): 菜单操作 — 通过 UIAutomation 调用菜单项。
    ///
    /// 当前为占位实现,后续任务填充。
    #[instrument(skip(self))]
    pub fn invoke_menu_item(&self, _menu_path: &str) -> Result<()> {
        anyhow::bail!("menu invocation not yet implemented (T-S6-A-01a TODO)")
    }

    /// TODO(T-S6-A-01a): 输入模拟 — 模拟键盘/鼠标输入。
    ///
    /// 当前为占位实现,后续任务填充。
    #[instrument(skip(self))]
    pub fn simulate_input(&self, _input: &str) -> Result<()> {
        anyhow::bail!("input simulation not yet implemented (T-S6-A-01a TODO)")
    }
}

impl Default for OsControllerService {
    fn default() -> Self {
        Self::new()
    }
}

// ----------------------------------------------------------------------
// Windows 平台: Win32 API 辅助函数
// ----------------------------------------------------------------------

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{HWND, LPARAM};
#[cfg(target_os = "windows")]
type WinBool = i32;

/// 读取窗口标题(GetWindowTextW)。
///
/// 缓冲区上限 512 个 UTF-16 字符(足够覆盖绝大多数窗口标题)。
/// 返回空字符串表示窗口无标题。
#[cfg(target_os = "windows")]
fn get_window_title(hwnd: HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    // SAFETY: `buf` 是栈上 512 元素的 `[u16; 512]` 数组,指针与容量(`buf.len() as i32`)
    // 在 `GetWindowTextW` 同步返回前保持有效。该 API 将窗口标题以 UTF-16 写入缓冲区
    // 并返回字符数(不含 null);返回值 <=0 表示无标题。`hwnd` 由调用方传入,
    // 来自 `GetForegroundWindow` / `EnumWindows` 回调,均为有效的窗口句柄。
    unsafe {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// `EnumWindows` 回调 — 收集可见且有标题的顶层窗口。
///
/// 通过 `LPARAM` 传递 `&mut Vec<WindowInfo>` 的裸指针。
/// 返回 `1`(WinBool true)表示继续枚举。
#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> WinBool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};

    let state = &mut *(lparam as *mut Vec<WindowInfo>);

    // 仅收集可见窗口
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }

    let title = get_window_title(hwnd);
    // 跳过无标题窗口(避免列出大量系统辅助窗口)
    if title.is_empty() {
        return 1;
    }

    let mut process_id: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut process_id);

    state.push(WindowInfo {
        hwnd: hwnd as usize as u64,
        title,
        process_id,
        is_foreground: false,
    });

    1
}

// ----------------------------------------------------------------------
// macOS 平台: T-S6-A-01b 骨架占位
// ----------------------------------------------------------------------
//
// 当前仅为骨架,不调用任何 AppKit/CoreGraphics API。
// 真实实现将在后续任务中补充:
//   * `get_foreground_window` → `NSWorkspace.frontmostApplication` +
//     `CGWindowListCopyWindowInfo`(kCGWindowWindowID 匹配)
//   * `list_windows` → `CGWindowListCopyWindowInfo`(kCGWindowListOptionOnScreenOnly)
//
// 由于 `OsControllerService` 的方法已通过 `#[cfg(target_os = "macos")]`
// 直接返回 `Err`,此处无需额外辅助函数。保留该分节以便后续填充。

// ----------------------------------------------------------------------
// Tauri 命令(平台无关 — 分发由 `OsControllerService` 方法上的 cfg 完成)
// ----------------------------------------------------------------------

/// 获取前台窗口信息。
#[tauri::command]
#[allow(dead_code)]
pub async fn os_get_foreground_window() -> Result<Option<WindowInfo>, String> {
    let svc = OsControllerService::new();
    svc.get_foreground_window().map_err(|e| format!("{e:#}"))
}

/// 列出所有可见窗口。
#[tauri::command]
#[allow(dead_code)]
pub async fn os_list_windows() -> Result<Vec<WindowInfo>, String> {
    let svc = OsControllerService::new();
    svc.list_windows().map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_check_returns_ok() {
        let svc = OsControllerService::new();
        assert!(svc.health_check().await.unwrap());
    }

    #[test]
    fn default_impl_works() {
        let _svc = OsControllerService::default();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn get_foreground_window_does_not_panic() {
        // 在测试环境下前台窗口可能为终端/IDE,调用应不 panic。
        let svc = OsControllerService::new();
        let _ = svc.get_foreground_window();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn list_windows_returns_visible() {
        // 测试环境下至少有进程自身窗口或终端窗口可见。
        let svc = OsControllerService::new();
        let result = svc.list_windows();
        // 不强制断言非空(无头环境可能无窗口),只验证不报错。
        assert!(result.is_ok(), "list_windows should not error");
    }

    #[test]
    fn invoke_menu_item_is_todo() {
        let svc = OsControllerService::new();
        let result = svc.invoke_menu_item("File/Open");
        assert!(result.is_err());
    }

    #[test]
    fn simulate_input_is_todo() {
        let svc = OsControllerService::new();
        let result = svc.simulate_input("hello");
        assert!(result.is_err());
    }
}
