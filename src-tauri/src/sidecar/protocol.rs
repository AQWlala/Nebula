//! Sidecar 启动协议 — 配置传递 + 握手。
//!
//! 主进程启动 sidecar 时通过命令行参数 + 环境变量传递配置：
//! - `--listen-addr` — gRPC 监听地址（默认 `127.0.0.1:0` 表示自动选端口）
//! - `--data-dir` — 数据目录
//! - `NEBULA_SIDECAR_TOKEN` — 认证 token（双向校验）
//!
//! 启动后主进程通过 HealthCheck 确认 sidecar 就绪。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Sidecar 启动配置（主进程 → sidecar）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarConfig {
    /// 服务类型标识。
    pub kind: String,
    /// gRPC 监听地址。
    pub listen_addr: String,
    /// 数据目录。
    pub data_dir: PathBuf,
    /// 认证 token（双向校验）。
    pub auth_token: String,
    /// 日志级别。
    pub log_level: String,
}

impl SidecarConfig {
    pub fn new(kind: &str, data_dir: PathBuf, auth_token: String) -> Self {
        Self {
            kind: kind.to_string(),
            listen_addr: "127.0.0.1:0".to_string(),
            data_dir,
            auth_token,
            log_level: "info".to_string(),
        }
    }

    pub fn with_listen_addr(mut self, addr: impl Into<String>) -> Self {
        self.listen_addr = addr.into();
        self
    }
}

/// Sidecar 就绪响应（HealthCheck 返回）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarReady {
    pub kind: String,
    pub version: String,
    pub listen_addr: String,
    pub pid: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_config_defaults() {
        let cfg = SidecarConfig::new(
            "memory",
            PathBuf::from("/tmp/data"),
            "token123".to_string(),
        );
        assert_eq!(cfg.kind, "memory");
        assert_eq!(cfg.listen_addr, "127.0.0.1:0");
        assert_eq!(cfg.log_level, "info");
    }
}
