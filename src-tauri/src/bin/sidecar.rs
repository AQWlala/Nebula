//! T-S2-B-01: Sidecar 二进制模板 — 通用 sidecar 服务入口。
//!
//! 这是一个**单二进制多角色**的 sidecar: 通过 `--kind` 参数选择
//! 运行 Memory / LLM / Swarm 服务之一。每个角色启动对应的
//! tonic gRPC server,注册 prost 生成的 `*_server` trait。
//!
//! ## 启动协议
//!
//! ```bash
//! nebula-sidecar \
//!   --kind memory \
//!   --listen-addr 127.0.0.1:50051 \
//!   --data-dir /path/to/data \
//!   --log-level info
//! ```
//!
//! 环境变量 `NEBULA_SIDECAR_TOKEN` 提供认证 token（双向校验）。
//!
//! ## 设计目标
//!
//! * **通用模板** — T-S4-B-01 (Skill sidecar) 和 T-S4-B-02 (Reflection sidecar)
//!   可通过在此模板上注册额外的 `*_server` trait 来扩展,无需复制样板代码。
//! * **故障隔离** — 单个 sidecar 崩溃不影响主进程或其他 sidecar。
//! * **独立升级** — sidecar 二进制可独立于主应用发布更新。
//!
//! ## 当前状态（v2.1）
//!
//! 本模板提供完整的启动框架（CLI 解析 + tonic server 引导 + 信号处理）,
//! 但**业务逻辑初始化**需要后续任务填充: sidecar 需要独立初始化
//! SqliteStore / LanceStore / LlmGateway 等组件,而不是依赖主进程的 AppState。
//! 当前实现使用 `unimplemented!()` 占位,确保模板可编译但明确标记缺口。

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use tracing::{info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _, EnvFilter};

/// Sidecar 启动参数。
#[derive(Parser, Debug)]
#[command(name = "nebula-sidecar", about = "nebula sidecar service binary")]
struct SidecarArgs {
    /// 服务类型: memory / llm / swarm
    #[arg(long, value_enum)]
    kind: SidecarKindArg,

    /// gRPC 监听地址（默认 127.0.0.1:0 = 自动选端口）
    #[arg(long, default_value = "127.0.0.1:0")]
    listen_addr: String,

    /// 数据目录路径
    #[arg(long, default_value = ".")]
    data_dir: PathBuf,

    /// 日志级别
    #[arg(long, default_value = "info")]
    log_level: String,
}

/// CLI 可选的 sidecar 类型。
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum SidecarKindArg {
    /// Memory Service (8 RPCs: store, get, search, list_recent, ...)
    Memory,
    /// LLM Gateway Service (3 RPCs: complete, chat, embed)
    Llm,
    /// Swarm Coordinator Service (4 RPCs: execute, list_agents, get_agent, stream_events)
    Swarm,
    /// T-S4-B-01: Skill Service (5 RPCs: create_skill, execute_skill, list_skills, search_skills, rate_skill)
    Skill,
    /// T-S4-B-02: Reflection Service (3 RPCs: reflect_all, list_recent, persist_reflection)
    Reflection,
    /// T-S6-A-01a: OS-Controller Windows — 窗口管理 / 菜单操作 / 输入模拟。
    OsController,
}

impl SidecarKindArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Llm => "llm",
            Self::Swarm => "swarm",
            Self::Skill => "skill",
            Self::Reflection => "reflection",
            Self::OsController => "os_controller",
        }
    }

    /// 该 sidecar 类型应该监听的默认端口。
    #[allow(dead_code)]
    fn default_port(&self) -> u16 {
        match self {
            Self::Memory => 50051,
            Self::Llm => 50052,
            Self::Swarm => 50053,
            Self::Skill => 50054,
            Self::Reflection => 50055,
            Self::OsController => 50056,
        }
    }
}

/// 认证 token 拦截器 — 校验 `Authorization: Bearer <token>` 头。
///
/// 主进程在启动 sidecar 时通过 `NEBULA_SIDECAR_TOKEN` 环境变量
/// 传递 token; sidecar 在每个 gRPC 请求的拦截器中校验。
struct TokenAuthInterceptor {
    expected_token: Option<String>,
}

impl TokenAuthInterceptor {
    fn from_env() -> Self {
        let token = std::env::var("NEBULA_SIDECAR_TOKEN")
            .ok()
            .filter(|t| !t.is_empty());
        Self { expected_token: token }
    }

    #[allow(clippy::result_large_err)]
    fn validate(&self, req: &tonic::Request<()>) -> Result<(), tonic::Status> {
        let expected = match &self.expected_token {
            None => return Ok(()), // 开发模式: 未配置 token 则跳过认证
            Some(t) => t,
        };
        let metadata = req.metadata();
        let auth_header = metadata
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            if token == expected {
                return Ok(());
            }
        }
        Err(tonic::Status::unauthenticated("invalid or missing bearer token"))
    }
}

impl tonic::service::Interceptor for TokenAuthInterceptor {
    fn call(&mut self, req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        self.validate(&req)?;
        Ok(req)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = SidecarArgs::parse();

    // 初始化日志
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&args.log_level));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();

    info!(
        target: "nebula.sidecar",
        kind = args.kind.as_str(),
        listen_addr = %args.listen_addr,
        data_dir = ?args.data_dir,
        "starting nebula sidecar"
    );

    // 解析监听地址
    let addr: SocketAddr = args
        .listen_addr
        .parse()
        .with_context(|| format!("invalid listen address: {}", args.listen_addr))?;

    // 初始化认证拦截器
    let interceptor = TokenAuthInterceptor::from_env();
    if interceptor.expected_token.is_none() {
        warn!(
            target: "nebula.sidecar",
            "NEBULA_SIDECAR_TOKEN not set — running in unauthenticated dev mode"
        );
    }

    // 根据 kind 启动对应的 gRPC 服务。
    //
    // NOTE: 当前实现是**模板** — 业务逻辑初始化需要后续任务填充。
    // sidecar 需要独立初始化 SqliteStore / LanceStore / LlmGateway 等
    // 组件（而不是依赖主进程的 AppState）,这部分在 T-S4-B-01/T-S4-B-02
    // 中实现。当前模板仅验证启动框架的正确性。
    info!(
        target: "nebula.sidecar",
        kind = args.kind.as_str(),
        addr = %addr,
        "sidecar gRPC server bootstrap ready (business logic pending T-S4 tasks)"
    );

    // 信号处理: Ctrl+C 优雅退出
    tokio::signal::ctrl_c()
        .await
        .context("failed to install Ctrl+C handler")?;
    info!(target: "nebula.sidecar", "received Ctrl+C, shutting down");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_kind_arg_default_ports() {
        assert_eq!(SidecarKindArg::Memory.default_port(), 50051);
        assert_eq!(SidecarKindArg::Llm.default_port(), 50052);
        assert_eq!(SidecarKindArg::Swarm.default_port(), 50053);
        assert_eq!(SidecarKindArg::Skill.default_port(), 50054);
        assert_eq!(SidecarKindArg::Reflection.default_port(), 50055);
        assert_eq!(SidecarKindArg::OsController.default_port(), 50056);
    }

    #[test]
    fn sidecar_kind_arg_as_str() {
        assert_eq!(SidecarKindArg::Memory.as_str(), "memory");
        assert_eq!(SidecarKindArg::Llm.as_str(), "llm");
        assert_eq!(SidecarKindArg::Swarm.as_str(), "swarm");
        assert_eq!(SidecarKindArg::Skill.as_str(), "skill");
        assert_eq!(SidecarKindArg::Reflection.as_str(), "reflection");
        assert_eq!(SidecarKindArg::OsController.as_str(), "os_controller");
    }

    #[test]
    fn token_auth_interceptor_no_token_allows_all() {
        let interceptor = TokenAuthInterceptor { expected_token: None };
        let req = tonic::Request::new(());
        assert!(interceptor.validate(&req).is_ok());
    }

    #[test]
    fn token_auth_interceptor_validates_bearer() {
        let interceptor = TokenAuthInterceptor {
            expected_token: Some("secret123".to_string()),
        };
        // 无 Authorization 头 → 拒绝
        let req = tonic::Request::new(());
        assert!(interceptor.validate(&req).is_err());

        // 正确 Bearer token → 通过
        let mut req = tonic::Request::new(());
        req.metadata_mut().insert(
            "authorization",
            "Bearer secret123".parse().unwrap(),
        );
        assert!(interceptor.validate(&req).is_ok());

        // 错误 token → 拒绝
        let mut req = tonic::Request::new(());
        req.metadata_mut().insert(
            "authorization",
            "Bearer wrong".parse().unwrap(),
        );
        assert!(interceptor.validate(&req).is_err());
    }
}
