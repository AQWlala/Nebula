//! T-S6-A-01a: Sidecar OS-Controller 服务 — 单二进制多角色方案。
//!
//! 延续 T-S4-B-01 / T-S4-B-02 的单二进制多角色方案
//! (`nebula-sidecar --kind=os_controller`),为 Windows 窗口管理 /
//! 菜单操作 / 输入模拟提供独立进程隔离。
//!
//! 本模块定义 OS-Controller sidecar 的服务处理器 [`OsControllerServiceHandler`],
//! 它包装 [`OsControllerService`] 并暴露与 gRPC 服务方法对应的 RPC 接口。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  gRPC (tonic)
//!    ▼
//! nebula-sidecar --kind=os_controller  (监听 127.0.0.1:50056)
//!    │  OsControllerServiceHandler
//!    ▼
//! OsControllerService (Win32 API: 窗口管理 / 菜单 / 输入)
//!    │
//!    ▼
//! windows crate (UIAutomation / Win32)
//! ```
//!
//! ## RPC 映射
//!
//! | gRPC RPC                | OsControllerService 方法     |
//! |-------------------------|------------------------------|
//! | `GetForegroundWindow`   | `get_foreground_window`      |
//! | `ListWindows`           | `list_windows`               |
//! | `InvokeMenuItem`        | `invoke_menu_item` (TODO T-S6-A-01a)    |
//! | `SimulateInput`         | `simulate_input` (TODO T-S6-A-01a)      |
//! | `HealthCheck`           | (always ok)                  |

use std::sync::Arc;

use anyhow::Result;
use tracing::{info, instrument};

use crate::os::controller::{OsControllerService, WindowInfo};

/// OS-Controller sidecar 服务处理器。
///
/// 包装 [`OsControllerService`],为 gRPC 服务端提供业务逻辑入口。
/// 在进程内模式下也可直接使用(无需 gRPC)。
pub struct OsControllerServiceHandler {
    service: Arc<OsControllerService>,
}

impl OsControllerServiceHandler {
    /// 创建新的 OS-Controller 服务处理器。
    ///
    /// 通常在 sidecar 进程启动时构造。`OsControllerService` 是无状态的,
    /// 可安全地用 `Arc` 共享。
    pub fn new(service: Arc<OsControllerService>) -> Self {
        info!(
            target: "nebula.sidecar.os_controller",
            "OsControllerServiceHandler initialized"
        );
        Self { service }
    }

    /// 访问底层 OsControllerService(供 IPC 客户端在进程内模式下直接调用)。
    pub fn service(&self) -> &Arc<OsControllerService> {
        &self.service
    }

    /// RPC: HealthCheck — 始终返回 Ok(若 handler 存在则服务可用)。
    pub async fn health_check(&self) -> Result<bool> {
        self.service.health_check().await
    }

    /// RPC: GetForegroundWindow — 获取前台窗口信息。
    #[instrument(skip(self))]
    pub fn get_foreground_window(&self) -> Result<Option<WindowInfo>> {
        self.service.get_foreground_window()
    }

    /// RPC: ListWindows — 列出所有可见窗口。
    #[instrument(skip(self))]
    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        self.service.list_windows()
    }
}

impl std::fmt::Debug for OsControllerServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsControllerServiceHandler")
            .field("service", &"Arc<OsControllerService>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handler() -> OsControllerServiceHandler {
        OsControllerServiceHandler::new(Arc::new(OsControllerService::new()))
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let h = make_handler();
        assert!(h.health_check().await.expect("task should complete"));
    }

    #[test]
    fn service_accessor_works() {
        let h = make_handler();
        let _ = h.service();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn list_windows_via_handler_does_not_error() {
        let h = make_handler();
        assert!(h.list_windows().is_ok(), "list_windows should not error");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn get_foreground_window_via_handler_does_not_panic() {
        let h = make_handler();
        let _ = h.get_foreground_window();
    }
}
