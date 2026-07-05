//! IPC 客户端封装 — 统一的 sidecar gRPC 客户端接口。
//!
//! v2.0 架构中，主进程通过 gRPC 与各 sidecar 通信。
//! 本模块提供高层封装，隐藏 gRPC 细节。
//!
//! 为了保证开发体验，当 sidecar 二进制不存在时，
//! 自动回退到"进程内模式"（直接调用本地引擎）。
//!
//! T-S2-B-01: 新增业务 RPC 方法 — 通过 tonic 生成的 `*_service_client`
//! 拨号 sidecar 并调用对应的 gRPC 方法。

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
#[cfg(feature = "grpc")]
use anyhow::Context;

use super::manager::{SidecarKind, SidecarManager};

/// IPC 客户端模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcMode {
    /// 进程内模式（sidecar 二进制不存在时的回退）。
    InProcess,
    /// gRPC 远程模式。
    Grpc { addr: String },
}

/// 创建到 sidecar 的 tonic Channel。
///
/// 在 `InProcess` 模式下返回 `None`。
#[cfg(feature = "grpc")]
async fn dial_sidecar(addr: &str) -> Result<tonic::transport::Channel> {
    tonic::transport::Endpoint::from_shared(format!("http://{}", addr))
        .context("failed to parse sidecar gRPC endpoint")?
        .timeout(Duration::from_secs(30))
        .connect()
        .await
        .context("failed to connect to sidecar gRPC endpoint")
}

// ---------------------------------------------------------------------------
// Memory Service IPC
// ---------------------------------------------------------------------------

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
            IpcMode::Grpc { addr } => {
                let _ = addr; // T-E-S-61: grpc feature off 时静默 unused 警告
                #[cfg(feature = "grpc")]
                {
                    // T-E-S-61: 真正的 gRPC HealthCheck
                    if SidecarManager::grpc_health_check(addr, Duration::from_secs(5)).await {
                        return Ok(true);
                    }
                    // 失败回退到 is_running
                }
                Ok(self.manager.is_running(SidecarKind::Memory))
            }
        }
    }

    /// T-S2-B-01: 存储记忆 — 通过 gRPC 调用 MemoryService.Store。
    #[cfg(feature = "grpc")]
    pub async fn store_memory(
        &self,
        content: String,
        memory_type: i32,
        layer: i32,
    ) -> Result<String> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("store_memory requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                let channel = dial_sidecar(addr).await?;
                let mut client =
                    crate::grpc::tonic_server::generated::memory_service_client::MemoryServiceClient::new(channel);
                let req = crate::grpc::tonic_server::generated::StoreMemoryRequest {
                    content,
                    memory_type,
                    layer,
                    source: String::new(),
                    metadata_json: String::new(),
                };
                let resp = client
                    .store(tonic::Request::new(req))
                    .await
                    .context("MemoryService.Store RPC failed")?
                    .into_inner();
                Ok(resp.id)
            }
        }
    }

    /// T-S2-B-01: 搜索记忆 — 通过 gRPC 调用 MemoryService.Search。
    #[cfg(feature = "grpc")]
    pub async fn search_memory(&self, query: String, k: u32) -> Result<Vec<(String, f32)>> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("search_memory requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                let channel = dial_sidecar(addr).await?;
                let mut client =
                    crate::grpc::tonic_server::generated::memory_service_client::MemoryServiceClient::new(channel);
                let req = crate::grpc::tonic_server::generated::SearchRequest {
                    query,
                    k,
                    layer: 0, // Unspecified
                };
                let resp = client
                    .search(tonic::Request::new(req))
                    .await
                    .context("MemoryService.Search RPC failed")?
                    .into_inner();
                Ok(resp
                    .hits
                    .into_iter()
                    .filter_map(|h| h.memory.map(|m| (m.id, h.score)))
                    .collect())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LLM Service IPC
// ---------------------------------------------------------------------------

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
            IpcMode::Grpc { addr } => {
                let _ = addr; // T-E-S-61: grpc feature off 时静默 unused 警告
                #[cfg(feature = "grpc")]
                {
                    // T-E-S-61: 真正的 gRPC HealthCheck
                    if SidecarManager::grpc_health_check(addr, Duration::from_secs(5)).await {
                        return Ok(true);
                    }
                    // 失败回退到 is_running
                }
                Ok(self.manager.is_running(SidecarKind::Llm))
            }
        }
    }

    /// T-S2-B-01: 聊天 — 通过 gRPC 调用 LlmService.Chat。
    #[cfg(feature = "grpc")]
    pub async fn chat(&self, messages: Vec<(String, String)>, model: String) -> Result<String> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("chat requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                let channel = dial_sidecar(addr).await?;
                let mut client =
                    crate::grpc::tonic_server::generated::llm_service_client::LlmServiceClient::new(channel);
                let req = crate::grpc::tonic_server::generated::ChatRequest {
                    messages: messages
                        .into_iter()
                        .map(|(role, content)| {
                            crate::grpc::tonic_server::generated::ChatMessage { role, content }
                        })
                        .collect(),
                    model,
                    temperature: 0.7,
                };
                let resp = client
                    .chat(tonic::Request::new(req))
                    .await
                    .context("LlmService.Chat RPC failed")?
                    .into_inner();
                Ok(resp.message.map(|m| m.content).unwrap_or_default())
            }
        }
    }

    /// T-S2-B-01: 嵌入 — 通过 gRPC 调用 LlmService.Embed。
    #[cfg(feature = "grpc")]
    pub async fn embed(&self, text: String) -> Result<Vec<f32>> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("embed requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                let channel = dial_sidecar(addr).await?;
                let mut client =
                    crate::grpc::tonic_server::generated::llm_service_client::LlmServiceClient::new(channel);
                let req = crate::grpc::tonic_server::generated::EmbedRequest { text };
                let resp = client
                    .embed(tonic::Request::new(req))
                    .await
                    .context("LlmService.Embed RPC failed")?
                    .into_inner();
                Ok(resp.vector)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Swarm Service IPC
// ---------------------------------------------------------------------------

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
            IpcMode::Grpc { addr } => {
                let _ = addr; // T-E-S-61: grpc feature off 时静默 unused 警告
                #[cfg(feature = "grpc")]
                {
                    // T-E-S-61: 真正的 gRPC HealthCheck
                    if SidecarManager::grpc_health_check(addr, Duration::from_secs(5)).await {
                        return Ok(true);
                    }
                    // 失败回退到 is_running
                }
                Ok(self.manager.is_running(SidecarKind::Swarm))
            }
        }
    }

    /// T-S2-B-01: 执行 swarm 任务 — 通过 gRPC 调用 SwarmService.Execute。
    #[cfg(feature = "grpc")]
    pub async fn execute(&self, task_description: String) -> Result<String> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("execute requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                let channel = dial_sidecar(addr).await?;
                let mut client =
                    crate::grpc::tonic_server::generated::swarm_service_client::SwarmServiceClient::new(channel);
                let req = crate::grpc::tonic_server::generated::SwarmRequest {
                    description: task_description,
                    pipeline: Vec::new(),
                    max_retries: 3,
                };
                let resp = client
                    .execute(tonic::Request::new(req))
                    .await
                    .context("SwarmService.Execute RPC failed")?
                    .into_inner();
                Ok(format!("approved={}, verdict={}", resp.approved, resp.verdict))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unified IPC Layer
// ---------------------------------------------------------------------------

/// T-S4-B-01: Skill Service IPC 客户端。
///
/// 与 `MemoryIpcClient` 镜像,在 sidecar 二进制存在时走 gRPC,
/// 否则回退到进程内模式(由调用方直接使用 `SkillEngine`)。
pub struct SkillIpcClient {
    mode: IpcMode,
    manager: SidecarManager,
    #[allow(dead_code)]
    timeout: Duration,
}

impl SkillIpcClient {
    pub fn new(manager: SidecarManager) -> Self {
        let mode = if manager.is_running(SidecarKind::Skill) {
            manager
                .listen_addr(SidecarKind::Skill)
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
            IpcMode::Grpc { addr } => {
                let _ = addr; // T-E-S-61: grpc feature off 时静默 unused 警告
                #[cfg(feature = "grpc")]
                {
                    // T-E-S-61: 真正的 gRPC HealthCheck
                    if SidecarManager::grpc_health_check(addr, Duration::from_secs(5)).await {
                        return Ok(true);
                    }
                    // 失败回退到 is_running
                }
                Ok(self.manager.is_running(SidecarKind::Skill))
            }
        }
    }

    /// T-S4-B-01: 执行技能 — gRPC 模式下调用 SkillService.ExecuteSkill。
    ///
    /// 进程内模式下返回错误(应由调用方直接使用 SkillEngine)。
    #[cfg(feature = "grpc")]
    pub async fn execute_skill(
        &self,
        skill_id: String,
        params: std::collections::HashMap<String, String>,
    ) -> Result<String> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("execute_skill requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                // T-S4-B-01: 当 proto 定义 SkillService 后,此处替换为
                // SkillServiceClient::new(channel).execute_skill(req)。
                // 目前仅返回地址信息,表示 gRPC 模式已就绪。
                let _ = (addr, skill_id, params);
                anyhow::bail!("SkillService gRPC client not yet wired (proto pending)")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// T-S4-B-02: Reflection Service IPC
// ---------------------------------------------------------------------------

/// T-S4-B-02: Reflection Service IPC 客户端。
///
/// 与 `SkillIpcClient` 镜像,在 sidecar 二进制存在时走 gRPC,
/// 否则回退到进程内模式(由调用方直接使用 `SelfReflectionEngine`)。
pub struct ReflectionIpcClient {
    mode: IpcMode,
    manager: SidecarManager,
    #[allow(dead_code)]
    timeout: Duration,
}

impl ReflectionIpcClient {
    pub fn new(manager: SidecarManager) -> Self {
        let mode = if manager.is_running(SidecarKind::Reflection) {
            manager
                .listen_addr(SidecarKind::Reflection)
                .map(|addr| IpcMode::Grpc { addr })
                .unwrap_or(IpcMode::InProcess)
        } else {
            IpcMode::InProcess
        };

        Self {
            mode,
            manager,
            timeout: Duration::from_secs(120),
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
            IpcMode::Grpc { addr } => {
                let _ = addr; // T-E-S-61: grpc feature off 时静默 unused 警告
                #[cfg(feature = "grpc")]
                {
                    // T-E-S-61: 真正的 gRPC HealthCheck
                    if SidecarManager::grpc_health_check(addr, Duration::from_secs(5)).await {
                        return Ok(true);
                    }
                    // 失败回退到 is_running
                }
                Ok(self.manager.is_running(SidecarKind::Reflection))
            }
        }
    }

    /// T-S4-B-02: 执行一次完整自我反思 — gRPC 模式下调用 ReflectionService.ReflectAll。
    ///
    /// 进程内模式下返回错误(应由调用方直接使用 SelfReflectionEngine)。
    #[cfg(feature = "grpc")]
    pub async fn reflect_all(&self) -> Result<()> {
        match &self.mode {
            IpcMode::InProcess => {
                anyhow::bail!("reflect_all requires sidecar mode (currently InProcess)")
            }
            IpcMode::Grpc { addr } => {
                // T-S4-B-02: 当 proto 定义 ReflectionService 后,此处替换为
                // ReflectionServiceClient::new(channel).reflect_all(req)。
                let _ = addr;
                anyhow::bail!("ReflectionService gRPC client not yet wired (proto pending)")
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
    /// T-S4-B-01: Skill 服务 IPC 客户端。
    pub skill: Arc<SkillIpcClient>,
    /// T-S4-B-02: Reflection 服务 IPC 客户端。
    pub reflection: Arc<ReflectionIpcClient>,
    manager: SidecarManager,
}

impl IpcLayer {
    pub fn new(manager: SidecarManager) -> Self {
        Self {
            memory: Arc::new(MemoryIpcClient::new(manager.clone())),
            llm: Arc::new(LlmIpcClient::new(manager.clone())),
            swarm: Arc::new(SwarmIpcClient::new(manager.clone())),
            skill: Arc::new(SkillIpcClient::new(manager.clone())),
            reflection: Arc::new(ReflectionIpcClient::new(manager.clone())),
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
        let kh = self.skill.health_check().await.unwrap_or(false);
        let rh = self.reflection.health_check().await.unwrap_or(false);
        mh && lh && sh && kh && rh
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
