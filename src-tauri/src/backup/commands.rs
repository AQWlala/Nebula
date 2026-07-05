//! T-S6-A-03: 自动备份 Tauri 命令 — 前端触发的备份/列出/恢复。
//!
//! 命令清单(需在 `lib.rs` 的 `invoke_handler!` 宏中注册):
//! - [`backup_now`] — 立即执行一次备份,返回备份目录路径
//! - [`backup_list`] — 列出所有备份(按日期降序)
//! - [`backup_restore`] — 从指定日期的备份恢复(当前为 stub)

use std::path::PathBuf;

use tauri::Manager;

use crate::backup::scheduler::{default_backup_root, BackupInfo, BackupScheduler};

/// 立即执行一次备份,返回备份目录路径。
///
/// 数据库路径解析顺序:
/// 1. 从 `AppState.config` 获取(最准确,反映实际运行时路径)
/// 2. 回退到方案 B:独立解析 `%LOCALAPPDATA%\com.nebula.desktop\`
#[allow(dead_code)]
#[tauri::command]
pub async fn backup_now(app: tauri::AppHandle) -> Result<String, String> {
    let (db_path, lance_dir) = resolve_db_paths(&app)?;
    let scheduler = BackupScheduler::new(app, db_path, lance_dir);
    let path = scheduler.backup_once().map_err(|e| format!("{e:#}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// 列出所有备份(按日期降序)。
#[allow(dead_code)]
#[tauri::command]
pub async fn backup_list(app: tauri::AppHandle) -> Result<Vec<BackupInfo>, String> {
    let (db_path, lance_dir) = resolve_db_paths(&app)?;
    let scheduler = BackupScheduler::new(app, db_path, lance_dir);
    let list = scheduler.list_backups().map_err(|e| format!("{e:#}"))?;
    Ok(list)
}

/// 从指定日期的备份恢复。
///
/// TODO(见 ROADMAP): 完整恢复逻辑尚未实现。当前仅验证备份目录存在。
///
/// 完整实现需要:
/// 1. 关闭当前 SQLite / LanceDB 连接(需要 AppState.shutdown 或等价机制)
/// 2. 复制备份文件回原数据库位置
/// 3. 重启应用或重新初始化存储层
///
/// 当前返回 `Ok(())` 表示备份存在,前端可据此提示用户手动重启。
#[allow(dead_code)]
#[tauri::command]
pub async fn backup_restore(app: tauri::AppHandle, date: String) -> Result<(), String> {
    // 验证 date 格式(YYYYMMDD)
    if date.len() != 8 || !date.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid date format, expected YYYYMMDD: {date}"));
    }

    let _ = resolve_db_paths(&app)?;
    let backup_root = default_backup_root();
    let backup_dir = backup_root.join(&date);

    if !backup_dir.exists() {
        return Err(format!("backup not found: {date}"));
    }

    tracing::info!(
        target: "nebula.backup",
        date = %date,
        path = %backup_dir.display(),
        "backup_restore: stub (full restore not yet implemented)"
    );
    Ok(())
}

/// 解析数据库路径。
///
/// 优先从 `AppState.config` 获取(最准确,反映实际运行时路径);
/// 若 AppState 尚未初始化,回退到方案 B:独立解析
/// `%LOCALAPPDATA%\com.nebula.desktop\`(Windows)。
fn resolve_db_paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    // 方案 A:从 AppState 获取实际路径
    if let Some(state) = app.try_state::<crate::AppState>() {
        let db_path = PathBuf::from(&state.config.db_path);
        let lance_dir = PathBuf::from(&state.config.lance_path);
        return Ok((db_path, lance_dir));
    }

    // 方案 B:独立解析 %LOCALAPPDATA%\com.nebula.desktop\
    let data_dir = resolve_app_data_dir()?;
    Ok((
        data_dir.join("nebula.db"),
        data_dir.join("nebula_lance"),
    ))
}

/// 解析应用数据目录(方案 B 回退,不依赖 app state)。
///
/// Windows: `%LOCALAPPDATA%\com.nebula.desktop\`
/// macOS:   `~/Library/Application Support/com.nebula.desktop/`
/// Linux:   `~/.local/share/com.nebula.desktop/`
pub(crate) fn resolve_app_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("com.nebula.desktop"))
            .map_err(|_| "LOCALAPPDATA environment variable not set".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|d| {
                PathBuf::from(d)
                    .join("Library/Application Support/com.nebula.desktop")
            })
            .map_err(|_| "HOME environment variable not set".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOME")
            .map(|d| PathBuf::from(d).join(".local/share/com.nebula.desktop"))
            .map_err(|_| "HOME environment variable not set".to_string())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err("unsupported platform".to_string())
    }
}
