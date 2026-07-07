//! T-E-D-06: Windows 右键菜单 "问Nebula" 注册表读写。
//!
//! 注册表路径(用户级 HKCU,免管理员权限):
//!   HKCU\Software\Classes\*\shell\AskNebula
//!     MUIVerb = "问Nebula"
//!     Icon    = "<exe-path>"
//!     \command
//!       (默认) = "\"<exe-path>\" --ask \"%1\""
//!
//! 非 Windows 平台所有操作返回 `Err("not supported on this platform")`。

use serde::Serialize;
use tracing::instrument;

/// T-E-D-06: 右键菜单安装/卸载/状态查询返回值。
///
/// 镜像 `src/lib/tauri.ts` 中的 `ContextMenuStatus` 接口。
#[derive(Debug, Clone, Serialize)]
pub struct ContextMenuStatus {
    pub installed: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Windows 实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod win {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegOpenKeyExW, RegSetValueExW, HKEY, KEY_READ,
        KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    /// 主键路径(相对于 HKCU)。`*` 表示对所有文件类型生效。
    pub(super) const REG_KEY: &str = "Software\\Classes\\*\\shell\\AskNebula";
    pub(super) const REG_SUBKEY_COMMAND: &str = "command";
    pub(super) const REG_VAL_MUI_VERB: &str = "MUIVerb";
    pub(super) const REG_VAL_ICON: &str = "Icon";
    pub(super) const DISPLAY_NAME: &str = "问Nebula";

    /// 将 &str 转为以 null 结尾的 UTF-16 宽字符串。
    pub(super) fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 创建(或打开)注册表键,返回 HKEY。调用方负责 `close_key`。
    pub(super) fn create_key(parent: HKEY, subkey: &str) -> Result<HKEY, String> {
        let subkey_w = wide(subkey);
        let mut hkey: HKEY = std::ptr::null_mut();
        let ret = unsafe {
            RegCreateKeyExW(
                parent,
                subkey_w.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                std::ptr::null(),
                &mut hkey,
                std::ptr::null_mut(),
            )
        };
        if ret != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW failed: error code {ret}"));
        }
        Ok(hkey)
    }

    /// 打开注册表键(只读)。键不存在时返回 Err。
    pub(super) fn open_key_read(parent: HKEY, subkey: &str) -> Result<HKEY, String> {
        let subkey_w = wide(subkey);
        let mut hkey: HKEY = std::ptr::null_mut();
        let ret = unsafe { RegOpenKeyExW(parent, subkey_w.as_ptr(), 0, KEY_READ, &mut hkey) };
        if ret != ERROR_SUCCESS {
            return Err(format!("RegOpenKeyExW failed: error code {ret}"));
        }
        Ok(hkey)
    }

    /// 设置字符串值(REG_SZ)。`value_name` 为空字符串表示设置默认值。
    pub(super) fn set_string_value(hkey: HKEY, value_name: &str, data: &str) -> Result<(), String> {
        let name_w = wide(value_name);
        let data_w = wide(data);
        let byte_len = (data_w.len() * 2) as u32;
        // SAFETY: `name_w` 与 `data_w` 都是以 null 结尾的 UTF-16 宽字符串,指针在
        // 同步返回前保持有效。`byte_len` 包含末尾 null(两个字节),符合 REG_SZ 规范。
        // `data_w.as_ptr() as *const u8` 是对相同内存的字节级重解释,无对齐问题(u16 对齐 >= u8)。
        let ret = unsafe {
            RegSetValueExW(
                hkey,
                name_w.as_ptr(),
                0,
                REG_SZ,
                data_w.as_ptr() as *const u8,
                byte_len,
            )
        };
        if ret != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW failed: error code {ret}"));
        }
        Ok(())
    }

    /// 删除注册表键(叶子键,不能有子键)。
    pub(super) fn delete_key(parent: HKEY, subkey: &str) -> Result<(), String> {
        let subkey_w = wide(subkey);
        // SAFETY: `subkey_w` 是以 null 结尾的 UTF-16 宽字符串,指针在同步返回前有效。
        // `RegDeleteKeyW` 只接受两个标量参数,不通过指针写入调用方内存。
        let ret = unsafe { RegDeleteKeyW(parent, subkey_w.as_ptr()) };
        if ret != ERROR_SUCCESS {
            return Err(format!("RegDeleteKeyW failed: error code {ret}"));
        }
        Ok(())
    }

    /// 关闭注册表键句柄。
    pub(super) fn close_key(hkey: HKEY) {
        // SAFETY: `hkey` 是先前 `RegCreateKeyExW` / `RegOpenKeyExW` 成功返回的有效句柄,
        // 且尚未被 close。`RegCloseKey` 只接受一个标量句柄,不通过指针写入调用方内存。
        unsafe { RegCloseKey(hkey) };
    }

    /// 获取当前 exe 路径。
    pub(super) fn current_exe_path() -> Result<String, String> {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| format!("failed to get current exe: {e}"))
    }
}

#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;

#[cfg(target_os = "windows")]
pub fn install() -> Result<(), String> {
    let exe_path = win::current_exe_path()?;
    let command_value = format!("\"{exe_path}\" --ask \"%1\"");

    // 1. 创建主键 AskNebula。
    let hkey = win::create_key(HKEY_CURRENT_USER, win::REG_KEY)?;
    // 2. 设置 MUIVerb(显示名称)。
    if let Err(e) = win::set_string_value(hkey, win::REG_VAL_MUI_VERB, win::DISPLAY_NAME) {
        win::close_key(hkey);
        return Err(e);
    }
    // 3. 设置 Icon(用 exe 作为图标)。
    if let Err(e) = win::set_string_value(hkey, win::REG_VAL_ICON, &exe_path) {
        win::close_key(hkey);
        return Err(e);
    }
    win::close_key(hkey);

    // 4. 创建 command 子键并设置默认值。
    let cmd_subkey = format!("{}\\{}", win::REG_KEY, win::REG_SUBKEY_COMMAND);
    let cmd_hkey = win::create_key(HKEY_CURRENT_USER, &cmd_subkey)?;
    if let Err(e) = win::set_string_value(cmd_hkey, "", &command_value) {
        win::close_key(cmd_hkey);
        return Err(e);
    }
    win::close_key(cmd_hkey);

    tracing::info!(
        target: "nebula.os.context_menu",
        "right-click menu 'AskNebula' installed"
    );
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn uninstall() -> Result<(), String> {
    // 先删除 command 子键,再删除主键(RegDeleteKeyW 只删叶子键)。
    let cmd_subkey = format!("{}\\{}", win::REG_KEY, win::REG_SUBKEY_COMMAND);
    // command 子键可能不存在(已部分卸载),忽略错误继续删主键。
    let _ = win::delete_key(HKEY_CURRENT_USER, &cmd_subkey);
    let result = win::delete_key(HKEY_CURRENT_USER, win::REG_KEY);
    if result.is_ok() {
        tracing::info!(
            target: "nebula.os.context_menu",
            "right-click menu 'AskNebula' uninstalled"
        );
    }
    result
}

#[cfg(target_os = "windows")]
pub fn is_installed() -> bool {
    // 尝试打开 command 子键;成功即视为已安装。
    let cmd_subkey = format!("{}\\{}", win::REG_KEY, win::REG_SUBKEY_COMMAND);
    match win::open_key_read(HKEY_CURRENT_USER, &cmd_subkey) {
        Ok(hkey) => {
            win::close_key(hkey);
            true
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// 非 Windows 桩实现
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
pub fn install() -> Result<(), String> {
    Err("not supported on this platform".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn uninstall() -> Result<(), String> {
    Err("not supported on this platform".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn is_installed() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

/// T-E-D-06: 安装 Windows 右键菜单 "问Nebula"(写 HKCU 注册表,免管理员)。
#[tauri::command]
#[instrument(fields(otel.kind = "context_menu_install"))]
pub fn context_menu_install() -> ContextMenuStatus {
    match install() {
        Ok(()) => ContextMenuStatus {
            installed: true,
            error: None,
        },
        Err(e) => ContextMenuStatus {
            installed: false,
            error: Some(e),
        },
    }
}

/// T-E-D-06: 卸载 Windows 右键菜单 "问Nebula"(删除 HKCU 注册表项)。
#[tauri::command]
#[instrument(fields(otel.kind = "context_menu_uninstall"))]
pub fn context_menu_uninstall() -> ContextMenuStatus {
    match uninstall() {
        Ok(()) => ContextMenuStatus {
            installed: false,
            error: None,
        },
        Err(e) => ContextMenuStatus {
            installed: is_installed(),
            error: Some(e),
        },
    }
}

/// T-E-D-06: 查询 Windows 右键菜单 "问Nebula" 当前安装状态。
#[tauri::command]
#[instrument(fields(otel.kind = "context_menu_status"))]
pub fn context_menu_status() -> ContextMenuStatus {
    ContextMenuStatus {
        installed: is_installed(),
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_stubs_return_not_supported() {
        assert!(install().is_err());
        assert!(uninstall().is_err());
        assert!(!is_installed());
    }

    #[test]
    fn context_menu_status_struct_constructs() {
        let s = ContextMenuStatus {
            installed: false,
            error: None,
        };
        assert!(!s.installed);
        assert!(s.error.is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn wide_string_includes_null_terminator() {
        assert_eq!(super::win::wide("AB"), vec![0x41, 0x42, 0]);
        assert_eq!(super::win::wide(""), vec![0]);
    }
}
