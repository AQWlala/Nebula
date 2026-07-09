//! T-D-B-13: 系统服务注册 Tauri 命令。
//!
//! 三个命令(均为 `DaemonInstaller` 的薄封装,不依赖 `AppState`):
//! * `daemon_status` — 查询服务安装/运行状态(只读,前端可安全调用)。
//! * `daemon_install` — 安装系统服务(写配置 + enable/load)。系统级需提权。
//! * `daemon_uninstall` — 卸载系统服务(停止 + disable + 删配置)。

use tauri::State;

use crate::api::daemon::{DaemonConfig, DaemonInstaller, DaemonStatus};
use crate::commands::error::CommandError;
use crate::AppState;

/// `daemon_status` 请求参数。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DaemonStatusRequest {
    /// 服务名,默认 `nebula`。
    #[serde(default)]
    pub name: Option<String>,
}

/// `daemon_install` 请求参数。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DaemonInstallRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub restart: Option<String>,
}

impl Default for DaemonStatusRequest {
    fn default() -> Self {
        Self { name: None }
    }
}

impl Default for DaemonInstallRequest {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            restart: None,
        }
    }
}

/// T-D-B-13: 查询系统服务状态(只读)。
#[tauri::command]
pub async fn daemon_status(
    _state: State<'_, AppState>,
    request: Option<DaemonStatusRequest>,
) -> Result<DaemonStatus, CommandError> {
    let req = request.unwrap_or_default();
    let cfg = build_config(req.name.as_deref(), None)?;
    let installer = DaemonInstaller::new(cfg);
    installer
        .status()
        .await
        .map_err(|e| CommandError::internal("daemon_status", &e))
}

/// T-D-B-13: 安装系统服务。
#[tauri::command]
pub async fn daemon_install(
    _state: State<'_, AppState>,
    request: Option<DaemonInstallRequest>,
) -> Result<DaemonStatus, CommandError> {
    let req = request.unwrap_or_default();
    let cfg = build_config(req.name.as_deref(), req.restart.as_deref())?;
    let installer = DaemonInstaller::new(cfg);
    installer
        .install()
        .await
        .map_err(|e| CommandError::internal("daemon_install", &e))
}

/// T-D-B-13: 卸载系统服务。
#[tauri::command]
pub async fn daemon_uninstall(
    _state: State<'_, AppState>,
    request: Option<DaemonStatusRequest>,
) -> Result<DaemonStatus, CommandError> {
    let req = request.unwrap_or_default();
    let cfg = build_config(req.name.as_deref(), None)?;
    let installer = DaemonInstaller::new(cfg);
    installer
        .uninstall()
        .await
        .map_err(|e| CommandError::internal("daemon_uninstall", &e))
}

/// 构造 DaemonConfig(基于当前可执行文件路径)。
fn build_config(name: Option<&str>, restart: Option<&str>) -> Result<DaemonConfig, CommandError> {
    let mut cfg = DaemonConfig::default();
    if let Some(n) = name {
        cfg.name = n.to_string();
    }
    if let Some(r) = restart {
        cfg.restart = r.to_string();
    }
    Ok(cfg)
}
