//! v2.0 Sidecar 进程拆分 — 多进程架构。
//!
//! 设计文档 v7.0 §14 Sidecar Architecture：
//! Memory Service / Swarm Coordinator / LLM Gateway 作为独立进程运行，
//! 主进程（Tauri UI）通过 gRPC 与 sidecar 通信。
//!
//! ## 优势
//!
//! * **故障隔离** — 单个 sidecar 崩溃不影响整体 UI
//! * **资源隔离** — 重计算服务独立分配资源
//! * **独立升级** — sidecar 可独立发布更新
//! * **多实例部署** — sidecar 可部署在远程机器上
//!
//! ## 模块结构
//!
//! * [`manager`] — SidecarManager：进程生命周期管理 + 健康检查 + 自动重启
//! * [`ipc`] — IPC 客户端封装：统一的 gRPC client 接口
//! * [`protocol`] — sidecar 启动协议 / 握手 / 配置传递

pub mod ipc;
pub mod manager;
pub mod protocol;

pub use manager::{SidecarKind, SidecarManager, SidecarStatus};
pub use protocol::SidecarConfig;

use std::path::PathBuf;

/// Sidecar 二进制文件的默认查找路径。
///
/// 优先顺序：
/// 1. 环境变量 `NINE_SNAKE_SIDECAR_DIR` 指定的目录
/// 2. 当前可执行文件同目录下的 `sidecars/` 子目录
/// 3. 开发模式下的 `target/debug/` 或 `target/release/`
pub fn default_sidecar_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NINE_SNAKE_SIDECAR_DIR") {
        return PathBuf::from(dir);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sidecars = parent.join("sidecars");
            if sidecars.exists() {
                return sidecars;
            }
            return parent.to_path_buf();
        }
    }

    PathBuf::from(".")
}

/// Sidecar 二进制文件名（根据平台）。
#[cfg(windows)]
pub fn sidecar_exe_name(kind: SidecarKind) -> String {
    format!("nine-snake-{}.exe", kind.as_str())
}

#[cfg(not(windows))]
pub fn sidecar_exe_name(kind: SidecarKind) -> String {
    format!("nine-snake-{}", kind.as_str())
}
