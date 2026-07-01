//! IPC 客户端封装 — 统一的 sidecar gRPC 客户端接口。
//!
//! v2.0 架构中，主进程通过 gRPC 与各 sidecar 通信。
//! 本模块提供高层封装，隐藏 gRPC 细节。
//!
//! 为了保证开发体验，当 sidecar 二进制不存在时，
//! 自动回退到"进程内模式"（直接调用本地引擎）。

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::manager::{SidecarKind, SidecarManager};

/// IPC 客户端模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcMode {
    /// 进程内模式（sidecar 二进制不存在时的回退）。
    InProcess,
    /// gRPC 远程模式。
    Grpc { addr: String },
}

/// Memory Service IPC 客户端。
pub struct MemoryIpcClient {
    mode: IpcMode,
    manager: SidecarManager,
    #[allow(dead_code)]
    timeout: Duration,
}

impl MemoryIpcClient {
    pub fn new(manager: SidecarManager) -> Self {
        let mode = if manager.is_running(SidecarKind::Memory) {
            manager
                .listen_addr(SidecarKind::Memory)
                .map(|addr| IpcMode::Grpc { addr })
                .unwrap_or(IpcMode::InProcess)
        } else {
            IpcMode::InProcess
        };

        Self {
            mode,
            manager,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn mode(&self) -> &IpcMode {
        &self.mode
    }

    pub fn is_grpc(&self) -> bool {
        matches!(self.mode, IpcMode::Grpc { .. })
    }

    /// 健康检查。
    pub async fn health_check(&self) -> Result<bool> {
        match &self.mode {
            IpcMode::InProcess => Ok(true),
            IpcMode::Grpc { .. } => {
                Ok(self.manager.is_running(SidecarKind::Memory))
            }
        }
    }
}

/// LLM Service IPC 客户端。
pub struct LlmIpcClient {
    mode: IpcMode,
    manager: SidecarManager,
    #[allow(dead_code)]
    timeout: Duration,
}

impl LlmIpcClient {
    pub fn new(manager: SidecarManager) -> Self {
        let mode = if manager.is_running(SidecarKind::Llm) {
            manager
                .listen_addr(SidecarKind::Llm)
                .map(|addr| IpcMode::Grpc { addr })
                .unwrap_or(IpcMode::InProcess)
        } else {
            IpcMode::InProcess
        };

        Self {
            mode,
            manager,
            timeout: Duration::from_secs(60),
        }
    }

    pub fn mode(&self) -> &IpcMode {
        &self.mode
    }

    pub fn is_grpc(&self) -> bool {
        matches!(self.mode, IpcMode::Grpc { .. })
    }

    /// 健康检查。
    pub async fn health_check(&self) -> Result<bool> {
        match &self.mode {
            IpcMode::InProcess => Ok(true),
            IpcMode::Grpc { .. } => {
                Ok(self.manager.is_running(SidecarKind::Llm))
            }
        }
    }
}

/// Swarm Service IPC 客户端。
pub struct SwarmIpcClient {
    mode: IpcMode,
    manager: SidecarManager,
    #[allow(dead_code)]
    timeout: Duration,
}

impl SwarmIpcClient {
    pub fn new(manager: SidecarManager) -> Self {
        let mode = if manager.is_running(SidecarKind::Swarm) {
            manager
                .listen_addr(SidecarKind::Swarm)
                .map(|addr| IpcMode::Grpc { addr })
                .unwrap_or(IpcMode::InProcess)
        } else {
            IpcMode::InProcess
        };

        Self {
            mode,
            manager,
            timeout: Duration::from_secs(300),
        }
    }

    pub fn mode(&self) -> &IpcMode {
        &self.mode
    }

    pub fn is_grpc(&self) -> bool {
        matches!(self.mode, IpcMode::Grpc { .. })
    }

    /// 健康检查。
    pub async fn health_check(&self) -> Result<bool> {
        match &self.mode {
            IpcMode::InProcess => Ok(true),
            IpcMode::Grpc { .. } => {
                Ok(self.manager.is_running(SidecarKind::Swarm))
            }
        }
    }
}

/// 统一 IPC 层 — 持有所有 sidecar 客户端。
#[derive(Clone)]
pub struct IpcLayer {
    pub memory: Arc<MemoryIpcClient>,
    pub llm: Arc<LlmIpcClient>,
    pub swarm: Arc<SwarmIpcClient>,
    manager: SidecarManager,
}

impl IpcLayer {
    pub fn new(manager: SidecarManager) -> Self {
        Self {
            memory: Arc::new(MemoryIpcClient::new(manager.clone())),
            llm: Arc::new(LlmIpcClient::new(manager.clone())),
            swarm: Arc::new(SwarmIpcClient::new(manager.clone())),
            manager,
        }
    }

    pub fn manager(&self) -> &SidecarManager {
        &self.manager
    }

    /// 检查所有 sidecar 是否都在运行。
    pub async fn all_healthy(&self) -> bool {
        let mh = self.memory.health_check().await.unwrap_or(false);
        let lh = self.llm.health_check().await.unwrap_or(false);
        let sh = self.swarm.health_check().await.unwrap_or(false);
        mh && lh && sh
    }

    /// 获取当前运行模式描述。
    pub fn mode_description(&self) -> String {
        let modes: Vec<String> = SidecarKind::all()
            .iter()
            .map(|k| {
                let status = self.manager.status(*k);
                format!("{}:{:?}", k.as_str(), status)
            })
            .collect();
        modes.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ipc_mode_in_process_default() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        let client = MemoryIpcClient::new(manager);
        assert!(!client.is_grpc());
    }

    #[test]
    fn ipc_layer_creation() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        let layer = IpcLayer::new(manager);
        assert!(!layer.memory.is_grpc());
        assert!(!layer.llm.is_grpc());
        assert!(!layer.swarm.is_grpc());
    }
}
