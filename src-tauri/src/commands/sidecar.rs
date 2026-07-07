//! v2.0 Sidecar 管理命令 — 前端查询 sidecar 状态。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::sidecar::{SidecarKind, SidecarStatus};
use crate::AppState;

/// Sidecar 状态信息（前端可直接消费）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarStatusInfo {
    pub kind: String,
    pub status: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub listen_addr: Option<String>,
}

impl From<(SidecarKind, SidecarStatus, Option<u32>, Option<String>)> for SidecarStatusInfo {
    fn from(
        (kind, status, pid, listen_addr): (SidecarKind, SidecarStatus, Option<u32>, Option<String>),
    ) -> Self {
        let status_str = match &status {
            SidecarStatus::Stopped => "stopped",
            SidecarStatus::Starting => "starting",
            SidecarStatus::Running => "running",
            SidecarStatus::Crashed { .. } => "crashed",
            SidecarStatus::Restarting => "restarting",
        }
        .to_string();

        Self {
            kind: kind.as_str().to_string(),
            status: status_str,
            running: matches!(status, SidecarStatus::Running),
            pid,
            listen_addr,
        }
    }
}

/// 获取所有 sidecar 的状态。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "sidecar_list_status"))]
pub async fn sidecar_list_status(
    state: State<'_, AppState>,
) -> Result<Vec<SidecarStatusInfo>, CommandError> {
    let mut result = Vec::new();
    for kind in SidecarKind::all() {
        let status = state.sidecar_manager.status(kind);
        let pid = state
            .sidecar_manager
            .listen_addr(kind)
            .map(|_| None)
            .unwrap_or(None); // 简化：进程内模式不暴露 pid
        let addr = state.sidecar_manager.listen_addr(kind);
        result.push(SidecarStatusInfo::from((kind, status, pid, addr)));
    }
    Ok(result)
}

/// 手动启动指定 sidecar。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "sidecar_start"))]
pub async fn sidecar_start(state: State<'_, AppState>, kind: String) -> Result<bool, CommandError> {
    let sidecar_kind = parse_kind(&kind)?;
    state
        .sidecar_manager
        .start(sidecar_kind)
        .await
        .map_err(|e| CommandError::internal("sidecar_start", &anyhow::anyhow!("{}", e)))?;
    Ok(true)
}

/// 手动停止指定 sidecar。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "sidecar_stop"))]
pub async fn sidecar_stop(state: State<'_, AppState>, kind: String) -> Result<bool, CommandError> {
    let sidecar_kind = parse_kind(&kind)?;
    state
        .sidecar_manager
        .stop(sidecar_kind)
        .await
        .map_err(|e| CommandError::internal("sidecar_stop", &anyhow::anyhow!("{}", e)))?;
    Ok(true)
}

/// 重启指定 sidecar。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "sidecar_restart"))]
pub async fn sidecar_restart(
    state: State<'_, AppState>,
    kind: String,
) -> Result<bool, CommandError> {
    let sidecar_kind = parse_kind(&kind)?;
    let _ = state.sidecar_manager.stop(sidecar_kind).await;
    state
        .sidecar_manager
        .start(sidecar_kind)
        .await
        .map_err(|e| CommandError::internal("sidecar_restart", &anyhow::anyhow!("{}", e)))?;
    Ok(true)
}

fn parse_kind(kind: &str) -> Result<SidecarKind, CommandError> {
    match kind {
        "memory" => Ok(SidecarKind::Memory),
        "llm" => Ok(SidecarKind::Llm),
        "swarm" => Ok(SidecarKind::Swarm),
        _ => Err(CommandError::validation(format!(
            "unknown sidecar kind: {}",
            kind
        ))),
    }
}
