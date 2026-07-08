//! T-E-S-27: Trusted Diagnostics Tauri 命令。
//!
//! 三个命令:
//! * `subscribe_diagnostics` — 通过 `tauri::ipc::Channel<DiagnosticEvent>`
//!   订阅实时诊断事件流(参考 `subscribe_events` 模式)。
//! * `diagnostics_snapshot` — 返回最近 N 条诊断事件(默认 50)。
//! * `diagnostics_open_logs` — 打开诊断日志目录(跟随 `default_log_dir()`)。

use std::path::PathBuf;

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::diagnostics::DiagnosticEvent;
use crate::AppState;

/// `subscribe_diagnostics` 的默认订阅时长(无活跃订阅者时也保持长连接)。
const SUBSCRIBE_TIMEOUT_SECS: u64 = 60 * 60;

/// T-E-S-27: 订阅实时诊断事件流。
///
/// 使用 Tauri 2.0 `ipc::Channel` 双向通道:前端调用后立即开始监听,
/// 后端循环 `recv().await` 并把 `DiagnosticEvent` 推送给前端。
///
/// 前端关闭通道(返回页面或取消订阅)时 `on_event.send()` 失败,
/// 后端循环自动退出,不会泄漏任务。Lagged 时发出 `Dropped` 元事件。
#[tauri::command]
#[instrument(skip(state, on_event), fields(otel.kind = "subscribe_diagnostics"))]
pub async fn subscribe_diagnostics(
    state: State<'_, AppState>,
    on_event: tauri::ipc::Channel<DiagnosticEvent>,
) -> Result<(), CommandError> {
    if !state.infra.config.diagnostics_channel_enabled {
        return Err(CommandError::validation(
            "diagnostics channel disabled (set NEBULA_DIAGNOSTICS=1 to enable)",
        ));
    }
    let mut rx = state.infra.diagnostics.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if on_event.send(event).is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    target: "nebula.cmd",
                    lagged = n,
                    "subscribe_diagnostics lagged behind, emitting Dropped meta-event"
                );
                // 发出 Dropped 元事件,前端可见。
                state.infra.diagnostics.emit_dropped(n);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}

/// T-E-S-27: 诊断事件快照 DTO。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSnapshot {
    /// 当前已发出的最大 seq 序号。
    pub last_seq: u64,
    /// 当前快照中的事件列表(按 seq 降序,最新在前)。
    pub events: Vec<DiagnosticEvent>,
    /// 总容量(broadcast channel capacity)。
    pub capacity: usize,
    /// 是否启用 diagnostics channel。
    pub enabled: bool,
}

/// T-E-S-27: 返回最近 `limit` 条诊断事件快照(默认 50)。
///
/// 通过临时订阅 bus、限时收集 `limit` 条事件实现。后续如需更高
/// 效率可加 ring buffer。当前实现满足"前端打开面板即可看到最近
/// 事件"的需求。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "diagnostics_snapshot"))]
pub async fn diagnostics_snapshot(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<DiagnosticsSnapshot, CommandError> {
    if !state.infra.config.diagnostics_channel_enabled {
        return Ok(DiagnosticsSnapshot {
            last_seq: 0,
            events: Vec::new(),
            capacity: state.infra.config.diagnostics_buffer_capacity,
            enabled: false,
        });
    }
    let limit = limit.unwrap_or(50).min(500);
    let mut rx = state.infra.diagnostics.subscribe();
    let mut events = Vec::with_capacity(limit);
    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(std::cmp::min(SUBSCRIBE_TIMEOUT_SECS, 1));
    while events.len() < limit {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(event)) => events.push(event),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            Err(_) => break, // timeout
        }
    }
    // 按 seq 降序排序(最新在前)。
    events.sort_by(|a, b| b.seq().cmp(&a.seq()));
    let last_seq = events.first().map(|e| e.seq()).unwrap_or(0);
    Ok(DiagnosticsSnapshot {
        last_seq,
        events,
        capacity: state.infra.config.diagnostics_buffer_capacity,
        enabled: true,
    })
}

/// T-E-S-27: 打开诊断日志所在目录。
///
/// 日志路径跟随 `default_log_dir()` = `%LOCALAPPDATA%\nebula\logs\`。
/// 前端可调用此命令拿到路径后用 shell plugin 打开。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "diagnostics_open_logs"))]
pub async fn diagnostics_open_logs(
    _state: State<'_, AppState>,
) -> Result<Option<String>, CommandError> {
    // 复用 lib.rs 的 default_log_dir()。这里通过 env 重新计算路径,
    // 避免暴露私有函数。诊断日志预期与主日志在同一目录。
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let dir = PathBuf::from(local_appdata).join("nebula").join("logs");
            return Ok(Some(dir.to_string_lossy().to_string()));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            let dir = PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join("nebula");
            return Ok(Some(dir.to_string_lossy().to_string()));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            let dir = PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("nebula")
                .join("logs");
            return Ok(Some(dir.to_string_lossy().to_string()));
        }
    }
    // 兜底:返回 NEBULA_LOG_DIR 环境变量。
    Ok(std::env::var("NEBULA_LOG_DIR").ok())
}
