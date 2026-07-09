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
/// T-D-B-14: LLM Gateway sidecar 服务处理器。
pub mod llm_service;
pub mod manager;
/// T-D-B-14: Memory sidecar 服务处理器。
pub mod memory_service;
/// T-S6-A-01a: OS-Controller sidecar 服务处理器。
pub mod os_controller_service;
// T-E-C-05: OS-Controller Sidecar 守护进程 — 独立进程运行 OS 控制能力。
pub mod os_controller_daemon;
pub mod protocol;
/// T-S4-B-02: Reflection sidecar 服务处理器。
pub mod reflection_service;
/// T-S4-B-01: Skill sidecar 服务处理器。
pub mod skill_service;
/// T-D-B-14: Swarm Coordinator sidecar 服务处理器。
pub mod swarm_service;

pub use llm_service::LlmServiceHandler;
pub use manager::{SidecarKind, SidecarManager, SidecarStatus};
pub use memory_service::MemoryServiceHandler;
pub use os_controller_service::OsControllerServiceHandler;
pub use protocol::SidecarConfig;
pub use reflection_service::ReflectionServiceHandler;
pub use skill_service::SkillServiceHandler;
pub use swarm_service::SwarmServiceHandler;

use std::path::PathBuf;

/// Sidecar 二进制文件的默认查找路径。
///
/// 优先顺序：
/// 1. 环境变量 `NEBULA_SIDECAR_DIR` 指定的目录
/// 2. 当前可执行文件同目录下的 `sidecars/` 子目录
/// 3. 开发模式下的 `target/debug/` 或 `target/release/`
pub fn default_sidecar_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NEBULA_SIDECAR_DIR") {
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
    format!("nebula-{}.exe", kind.as_str())
}

#[cfg(not(windows))]
pub fn sidecar_exe_name(kind: SidecarKind) -> String {
    format!("nebula-{}", kind.as_str())
}
