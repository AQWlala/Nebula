//! T-E-C-07: Remote Operator — 远程操作器模块。
//!
//! 允许通过远程连接(HTTP/HTTPS)控制远端的 OS-Controller Sidecar。
//! 提供与本地 OS-Controller 对等的操作能力(截图、点击、输入、按键、
//! 目标执行等),但通过远程协议转发到远端 Sidecar 执行。
//!
//! ## 架构
//!
//! ```text
//! 本地 RemoteOperator
//!    │  HTTP/HTTPS + JSON(RemoteCommand → RemoteResponse)
//!    ▼
//! 远端 OS-Controller Sidecar (Daemon)
//!    │  Win32 API / 平台抽象
//!    ▼
//! 远端操作系统
//! ```
//!
//! ## 连接管理
//!
//! * [`RemoteOperator::connect`] — 建立 HTTP 客户端并执行健康检查。
//! * [`RemoteOperator::disconnect`] — 断开连接,释放客户端资源。
//! * [`RemoteOperator::is_connected`] — 检查当前连接状态。
//!
//! ## 重试策略
//!
//! `send_command` 在网络错误时自动重试,最多 `max_retries` 次。
//!
//! ## 注册
//!
//! 本模块当前未在 `os/mod.rs` 中注册(主控统一处理)。注册时添加:
//! ```ignore
//! // in src-tauri/src/os/mod.rs
//! pub mod remote_operator;
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, instrument, warn};

// ======================================================================
// RemoteRegion — 远程截图区域
// ======================================================================

/// 远程截图区域(像素坐标)。
///
/// 用于 [`RemoteCommand::Screenshot`] 指定截取的屏幕区域。
/// `None` 表示截取整个屏幕。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRegion {
    /// 左上角 X 坐标。
    pub x: i32,
    /// 左上角 Y 坐标。
    pub y: i32,
    /// 宽度。
    pub width: u32,
    /// 高度。
    pub height: u32,
}

impl RemoteRegion {
    /// 创建新的截图区域。
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

// ======================================================================
// RemoteScreenshot — 远程截图结果
// ======================================================================

/// 远程截图结果 — base64 编码的图像数据 + 尺寸 + 格式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteScreenshot {
    /// base64 编码的图像数据。
    pub data_base64: String,
    /// 图像宽度(像素)。
    pub width: u32,
    /// 图像高度(像素)。
    pub height: u32,
    /// 图像格式(如 "png"、"jpeg")。
    pub format: String,
}

// ======================================================================
// RemoteWindowInfo — 远程窗口信息
// ======================================================================

/// 远程窗口信息 — 描述远端桌面上一个可见窗口。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteWindowInfo {
    /// 窗口标题。
    pub title: String,
    /// 进程名。
    pub process_name: String,
    /// 左上角 X 坐标。
    pub x: i32,
    /// 左上角 Y 坐标。
    pub y: i32,
    /// 窗口宽度。
    pub width: u32,
    /// 窗口高度。
    pub height: u32,
}

// ======================================================================
// RemoteHealthStatus — 远程健康状态
// ======================================================================

/// 远程 Sidecar 的健康状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteHealthStatus {
    /// 是否已连接且健康。
    pub connected: bool,
    /// 远端运行时长(秒)。
    pub uptime_secs: u64,
    /// 远端版本号。
    pub version: String,
    /// 已完成的任务数。
    pub tasks_completed: u64,
}

// ======================================================================
// RemoteExecutionResult — 远程目标执行结果
// ======================================================================

/// 远程目标执行结果 — 包含执行状态、步数、摘要与过程中截取的截图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteExecutionResult {
    /// 是否成功完成目标。
    pub success: bool,
    /// 执行步数。
    pub steps: u32,
    /// 执行摘要。
    pub summary: String,
    /// 执行过程中的截图(base64 编码列表)。
    pub screenshots: Vec<String>,
}

// ======================================================================
// RemoteCommand — 远程命令枚举
// ======================================================================

/// 远程命令 — 本地 → 远端 Sidecar 的操作指令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommand {
    /// 截图(可选指定区域)。
    Screenshot { region: Option<RemoteRegion> },
    /// 鼠标点击。
    Click { x: i32, y: i32 },
    /// 输入文本。
    Type { text: String },
    /// 按键。
    KeyPress { key: String },
    /// 滚动。
    Scroll { direction: String, amount: f32 },
    /// 执行高层目标(多步操作循环)。
    ExecuteGoal { goal: String, max_steps: usize },
    /// 健康检查。
    HealthCheck,
    /// 列出可见窗口。
    ListWindows,
}

// ======================================================================
// RemoteResponse — 远程响应枚举
// ======================================================================

/// 远程响应 — 远端 Sidecar → 本地的操作结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteResponse {
    /// 截图结果。
    Screenshot {
        data_base64: String,
        width: u32,
        height: u32,
    },
    /// 操作成功。
    Success { message: String },
    /// 窗口列表。
    WindowList { windows: Vec<RemoteWindowInfo> },
    /// 目标执行结果。
    ExecutionResult {
        success: bool,
        steps: u32,
        summary: String,
    },
    /// 健康状态。
    Health {
        status: String,
        uptime_secs: u64,
        version: String,
    },
    /// 错误。
    Error { code: i32, message: String },
}

// ======================================================================
// ConnectionState — 连接状态枚举
// ======================================================================

/// 远程操作器连接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// 已断开。
    Disconnected,
    /// 连接中。
    Connecting,
    /// 已连接。
    Connected,
    /// 出错。
    Error,
    /// 重连中。
    Reconnecting,
}

impl Default for ConnectionState {
    fn default() -> Self {
        ConnectionState::Disconnected
    }
}

// ======================================================================
// RemoteOperatorStats — 操作器运行统计
// ======================================================================

/// 远程操作器运行统计。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteOperatorStats {
    /// 发送的命令总数。
    pub total_commands: u64,
    /// 成功的命令数。
    pub successful: u64,
    /// 失败的命令数。
    pub failed: u64,
    /// 平均延迟(毫秒)。
    pub avg_latency_ms: f64,
    /// 最近一次命令时间。
    pub last_command_at: Option<DateTime<Utc>>,
}

impl Default for RemoteOperatorStats {
    fn default() -> Self {
        Self {
            total_commands: 0,
            successful: 0,
            failed: 0,
            avg_latency_ms: 0.0,
            last_command_at: None,
        }
    }
}

// ======================================================================
// RemoteOperatorConfig — 配置 + Builder
// ======================================================================

/// 远程操作器配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteOperatorConfig {
    /// 远程地址(如 "192.168.1.100:7878")。
    pub endpoint: String,
    /// 认证令牌(Bearer Token)。
    pub auth_token: Option<String>,
    /// 连接超时(秒)。
    pub connect_timeout_secs: u64,
    /// 请求超时(秒)。
    pub request_timeout_secs: u64,
    /// 最大重试次数。
    pub max_retries: u32,
    /// 是否启用 TLS。
    pub enable_tls: bool,
    /// 是否跳过 TLS 证书验证。
    pub tls_skip_verify: bool,
}

impl Default for RemoteOperatorConfig {
    fn default() -> Self {
        Self {
            endpoint: "127.0.0.1:7878".to_string(),
            auth_token: None,
            connect_timeout_secs: 10,
            request_timeout_secs: 30,
            max_retries: 3,
            enable_tls: false,
            tls_skip_verify: false,
        }
    }
}

impl RemoteOperatorConfig {
    /// 创建配置构造器。
    pub fn builder() -> RemoteOperatorConfigBuilder {
        RemoteOperatorConfigBuilder::new()
    }
}

/// [`RemoteOperatorConfig`] 构造器。
#[derive(Debug, Clone)]
pub struct RemoteOperatorConfigBuilder {
    config: RemoteOperatorConfig,
}

impl RemoteOperatorConfigBuilder {
    /// 创建新的构造器(以默认配置为起点)。
    pub fn new() -> Self {
        Self {
            config: RemoteOperatorConfig::default(),
        }
    }

    /// 设置远程地址。
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.endpoint = endpoint.into();
        self
    }

    /// 设置认证令牌。
    pub fn auth_token(mut self, token: impl Into<String>) -> Self {
        self.config.auth_token = Some(token.into());
        self
    }

    /// 设置连接超时(秒)。
    pub fn connect_timeout_secs(mut self, secs: u64) -> Self {
        self.config.connect_timeout_secs = secs;
        self
    }

    /// 设置请求超时(秒)。
    pub fn request_timeout_secs(mut self, secs: u64) -> Self {
        self.config.request_timeout_secs = secs;
        self
    }

    /// 设置最大重试次数。
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.config.max_retries = retries;
        self
    }

    /// 设置是否启用 TLS。
    pub fn enable_tls(mut self, enable: bool) -> Self {
        self.config.enable_tls = enable;
        self
    }

    /// 设置是否跳过 TLS 证书验证。
    pub fn tls_skip_verify(mut self, skip: bool) -> Self {
        self.config.tls_skip_verify = skip;
        self
    }

    /// 构建最终配置。
    pub fn build(self) -> RemoteOperatorConfig {
        self.config
    }
}

impl Default for RemoteOperatorConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ======================================================================
// RemoteOperator — 远程操作器
// ======================================================================

/// 远程操作器 — 通过 HTTP/HTTPS 控制远端 OS-Controller Sidecar。
///
/// ## 使用方式
///
/// ```no_run
/// # use nebula_lib::os::remote_operator::*;
/// # async fn example() -> anyhow::Result<()> {
/// let config = RemoteOperatorConfig::builder()
///     .endpoint("192.168.1.100:7878")
///     .auth_token("secret")
///     .build();
/// let operator = RemoteOperator::new(config);
/// operator.connect().await?;
/// operator.click(100, 200).await?;
/// operator.disconnect().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RemoteOperator {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// 操作器配置。
    config: RemoteOperatorConfig,
    /// 当前连接状态。
    state: Mutex<ConnectionState>,
    /// HTTP 客户端(连接后存在,断开后为 None)。
    client: Mutex<Option<reqwest::Client>>,
    /// 运行统计。
    stats: Mutex<RemoteOperatorStats>,
}

impl RemoteOperator {
    /// 创建新的远程操作器实例。
    pub fn new(config: RemoteOperatorConfig) -> Self {
        info!(
            target: "nebula.os.remote_operator",
            endpoint = %config.endpoint,
            enable_tls = config.enable_tls,
            "RemoteOperator created"
        );
        Self {
            inner: Arc::new(Inner {
                config,
                state: Mutex::new(ConnectionState::default()),
                client: Mutex::new(None),
                stats: Mutex::new(RemoteOperatorStats::default()),
            }),
        }
    }

    /// 建立连接 — 构建 HTTP 客户端并执行健康检查。
    ///
    /// 若已连接则直接返回 Ok。连接过程中状态依次经过
    /// `Connecting → Connected`(成功)或 `Connecting → Error`(失败)。
    #[instrument(skip(self))]
    pub async fn connect(&self) -> Result<()> {
        // 已连接则直接返回
        if self.is_connected().await {
            return Ok(());
        }

        // 设置为连接中
        *self.inner.state.lock() = ConnectionState::Connecting;

        // 构建 HTTP 客户端
        let client = match self.build_client() {
            Ok(c) => c,
            Err(e) => {
                *self.inner.state.lock() = ConnectionState::Error;
                error!(error = %e, "构建 HTTP 客户端失败");
                bail!("构建 HTTP 客户端失败: {}", e);
            }
        };

        // 存储客户端并临时设为已连接(以便执行健康检查)
        *self.inner.client.lock() = Some(client);
        *self.inner.state.lock() = ConnectionState::Connected;

        // 执行健康检查(不重试,快速失败)
        match self.send_command_once(&RemoteCommand::HealthCheck).await {
            Ok(RemoteResponse::Health { .. }) => {
                info!(
                    endpoint = %self.inner.config.endpoint,
                    "RemoteOperator connected"
                );
                Ok(())
            }
            Ok(other) => {
                *self.inner.state.lock() = ConnectionState::Error;
                *self.inner.client.lock() = None;
                bail!("健康检查返回意外响应: {:?}", other);
            }
            Err(e) => {
                *self.inner.state.lock() = ConnectionState::Error;
                *self.inner.client.lock() = None;
                error!(error = %e, "RemoteOperator connect failed");
                bail!("连接健康检查失败: {}", e);
            }
        }
    }

    /// 断开连接 — 释放 HTTP 客户端资源。
    #[instrument(skip(self))]
    pub async fn disconnect(&self) -> Result<()> {
        *self.inner.state.lock() = ConnectionState::Disconnected;
        *self.inner.client.lock() = None;
        info!("RemoteOperator disconnected");
        Ok(())
    }

    /// 检查是否已连接。
    pub async fn is_connected(&self) -> bool {
        let state = *self.inner.state.lock();
        let has_client = self.inner.client.lock().is_some();
        state == ConnectionState::Connected && has_client
    }

    /// 发送命令 — 带重试机制。
    ///
    /// 在网络错误时自动重试,最多 `max_retries` 次。
    /// 每次调用都会更新运行统计。
    #[instrument(skip(self, cmd))]
    pub async fn send_command(&self, cmd: RemoteCommand) -> Result<RemoteResponse> {
        if !self.is_connected().await {
            bail!("未连接到远程操作器");
        }

        let start = Instant::now();
        let mut last_error: Option<anyhow::Error> = None;

        // attempt 0 为首次尝试,1..=max_retries 为重试
        for attempt in 0..=self.inner.config.max_retries {
            if attempt > 0 {
                debug!(attempt, "重试命令");
            }
            match self.send_command_once(&cmd).await {
                Ok(resp) => {
                    let latency_ms = start.elapsed().as_millis() as f64;
                    let is_success = !matches!(&resp, RemoteResponse::Error { .. });
                    self.update_stats(latency_ms, is_success);
                    return Ok(resp);
                }
                Err(e) => {
                    warn!(attempt, error = %e, "命令发送失败");
                    last_error = Some(e);
                }
            }
        }

        // 所有重试均失败
        let latency_ms = start.elapsed().as_millis() as f64;
        self.update_stats(latency_ms, false);

        bail!(
            "命令发送失败(已重试 {} 次): {}",
            self.inner.config.max_retries,
            last_error.map(|e| e.to_string()).unwrap_or_default()
        )
    }

    /// 远程截图。
    ///
    /// `region` 为 `None` 时截取整个屏幕。
    pub async fn screenshot(&self, region: Option<RemoteRegion>) -> Result<RemoteScreenshot> {
        let resp = self
            .send_command(RemoteCommand::Screenshot { region })
            .await?;
        match resp {
            RemoteResponse::Screenshot {
                data_base64,
                width,
                height,
            } => Ok(RemoteScreenshot {
                data_base64,
                width,
                height,
                format: "png".to_string(),
            }),
            RemoteResponse::Error { code, message } => {
                bail!("远程截图失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 远程点击。
    pub async fn click(&self, x: i32, y: i32) -> Result<()> {
        let resp = self.send_command(RemoteCommand::Click { x, y }).await?;
        match resp {
            RemoteResponse::Success { .. } => Ok(()),
            RemoteResponse::Error { code, message } => {
                bail!("远程点击失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 远程输入文本。
    pub async fn type_text(&self, text: &str) -> Result<()> {
        let resp = self
            .send_command(RemoteCommand::Type {
                text: text.to_string(),
            })
            .await?;
        match resp {
            RemoteResponse::Success { .. } => Ok(()),
            RemoteResponse::Error { code, message } => {
                bail!("远程输入失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 远程按键。
    pub async fn key_press(&self, key: &str) -> Result<()> {
        let resp = self
            .send_command(RemoteCommand::KeyPress {
                key: key.to_string(),
            })
            .await?;
        match resp {
            RemoteResponse::Success { .. } => Ok(()),
            RemoteResponse::Error { code, message } => {
                bail!("远程按键失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 远程执行高层目标 — 多步操作循环(截图 → 分析 → 操作)。
    pub async fn execute_goal(
        &self,
        goal: &str,
        max_steps: usize,
    ) -> Result<RemoteExecutionResult> {
        let resp = self
            .send_command(RemoteCommand::ExecuteGoal {
                goal: goal.to_string(),
                max_steps,
            })
            .await?;
        match resp {
            RemoteResponse::ExecutionResult {
                success,
                steps,
                summary,
            } => Ok(RemoteExecutionResult {
                success,
                steps,
                summary,
                screenshots: Vec::new(),
            }),
            RemoteResponse::Error { code, message } => {
                bail!("目标执行失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 健康检查 — 查询远端 Sidecar 的运行状态。
    pub async fn health_check(&self) -> Result<RemoteHealthStatus> {
        let resp = self.send_command(RemoteCommand::HealthCheck).await?;
        match resp {
            RemoteResponse::Health {
                status,
                uptime_secs,
                version,
            } => Ok(RemoteHealthStatus {
                connected: status == "healthy",
                uptime_secs,
                version,
                tasks_completed: 0,
            }),
            RemoteResponse::Error { code, message } => {
                bail!("健康检查失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 列出远端可见窗口。
    pub async fn list_windows(&self) -> Result<Vec<RemoteWindowInfo>> {
        let resp = self.send_command(RemoteCommand::ListWindows).await?;
        match resp {
            RemoteResponse::WindowList { windows } => Ok(windows),
            RemoteResponse::Error { code, message } => {
                bail!("列出窗口失败 [{}]: {}", code, message)
            }
            other => bail!("意外的响应类型: {:?}", other),
        }
    }

    /// 获取当前连接状态。
    pub fn state(&self) -> ConnectionState {
        *self.inner.state.lock()
    }

    /// 获取当前运行统计快照。
    pub fn stats(&self) -> RemoteOperatorStats {
        self.inner.stats.lock().clone()
    }

    // ------------------------------------------------------------------
    // 内部方法
    // ------------------------------------------------------------------

    /// 构建 reqwest 客户端(根据配置设置超时与 TLS)。
    fn build_client(&self) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(self.inner.config.connect_timeout_secs))
            .timeout(Duration::from_secs(self.inner.config.request_timeout_secs));

        if self.inner.config.tls_skip_verify {
            builder = builder.danger_accept_invalid_certs(true);
        }

        Ok(builder.build()?)
    }

    /// 构造命令 URL。
    fn command_url(&self) -> String {
        let scheme = if self.inner.config.enable_tls {
            "https"
        } else {
            "http"
        };
        format!("{}://{}/command", scheme, self.inner.config.endpoint)
    }

    /// 单次发送命令(不重试)。
    ///
    /// 序列化 `RemoteCommand` 为 JSON,POST 到远端,解析 `RemoteResponse`。
    async fn send_command_once(&self, cmd: &RemoteCommand) -> Result<RemoteResponse> {
        let client = self
            .inner
            .client
            .lock()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("HTTP 客户端未初始化"))?;

        let mut request = client.post(self.command_url()).json(cmd);

        // 添加认证头
        if let Some(token) = &self.inner.config.auth_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await?;
        let resp: RemoteResponse = response.json().await?;
        Ok(resp)
    }

    /// 更新运行统计(内部方法)。
    fn update_stats(&self, latency_ms: f64, is_success: bool) {
        let mut stats = self.inner.stats.lock();
        stats.total_commands += 1;
        if is_success {
            stats.successful += 1;
        } else {
            stats.failed += 1;
        }
        // 增量计算平均延迟
        let total = stats.total_commands as f64;
        stats.avg_latency_ms = (stats.avg_latency_ms * (total - 1.0) + latency_ms) / total;
        stats.last_command_at = Some(Utc::now());
    }
}

impl Default for RemoteOperator {
    fn default() -> Self {
        Self::new(RemoteOperatorConfig::default())
    }
}

// ======================================================================
// 单元测试
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // RemoteOperatorConfig:默认值 / builder
    // ------------------------------------------------------------------

    #[test]
    fn test_config_defaults() {
        let cfg = RemoteOperatorConfig::default();
        assert_eq!(cfg.endpoint, "127.0.0.1:7878");
        assert!(cfg.auth_token.is_none());
        assert_eq!(cfg.connect_timeout_secs, 10);
        assert_eq!(cfg.request_timeout_secs, 30);
        assert_eq!(cfg.max_retries, 3);
        assert!(!cfg.enable_tls);
        assert!(!cfg.tls_skip_verify);
    }

    #[test]
    fn test_config_builder() {
        let cfg = RemoteOperatorConfig::builder()
            .endpoint("192.168.1.100:7878")
            .auth_token("secret-token")
            .connect_timeout_secs(5)
            .request_timeout_secs(60)
            .max_retries(5)
            .enable_tls(true)
            .tls_skip_verify(true)
            .build();
        assert_eq!(cfg.endpoint, "192.168.1.100:7878");
        assert_eq!(cfg.auth_token, Some("secret-token".to_string()));
        assert_eq!(cfg.connect_timeout_secs, 5);
        assert_eq!(cfg.request_timeout_secs, 60);
        assert_eq!(cfg.max_retries, 5);
        assert!(cfg.enable_tls);
        assert!(cfg.tls_skip_verify);
    }

    // ------------------------------------------------------------------
    // RemoteCommand 序列化往返(所有变体)
    // ------------------------------------------------------------------

    #[test]
    fn test_command_screenshot_roundtrip() {
        let cmd = RemoteCommand::Screenshot {
            region: Some(RemoteRegion::new(10, 20, 800, 600)),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_click_roundtrip() {
        let cmd = RemoteCommand::Click { x: 100, y: 200 };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_type_roundtrip() {
        let cmd = RemoteCommand::Type {
            text: "你好,世界".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_key_press_roundtrip() {
        let cmd = RemoteCommand::KeyPress {
            key: "Enter".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_scroll_roundtrip() {
        let cmd = RemoteCommand::Scroll {
            direction: "down".into(),
            amount: 3.5,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_execute_goal_roundtrip() {
        let cmd = RemoteCommand::ExecuteGoal {
            goal: "打开记事本并输入文本".into(),
            max_steps: 15,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_health_check_roundtrip() {
        let cmd = RemoteCommand::HealthCheck;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"health_check\"");
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_list_windows_roundtrip() {
        let cmd = RemoteCommand::ListWindows;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"list_windows\"");
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    // ------------------------------------------------------------------
    // RemoteResponse 序列化往返(所有变体)
    // ------------------------------------------------------------------

    #[test]
    fn test_response_screenshot_roundtrip() {
        let resp = RemoteResponse::Screenshot {
            data_base64: "iVBORw0KGgo=".into(),
            width: 1920,
            height: 1080,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: RemoteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_success_roundtrip() {
        let resp = RemoteResponse::Success {
            message: "操作完成".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: RemoteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_window_list_roundtrip() {
        let resp = RemoteResponse::WindowList {
            windows: vec![
                RemoteWindowInfo {
                    title: "记事本".into(),
                    process_name: "notepad.exe".into(),
                    x: 100,
                    y: 50,
                    width: 800,
                    height: 600,
                },
                RemoteWindowInfo {
                    title: "Chrome".into(),
                    process_name: "chrome.exe".into(),
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: RemoteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_execution_result_roundtrip() {
        let resp = RemoteResponse::ExecutionResult {
            success: true,
            steps: 5,
            summary: "目标已完成".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: RemoteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_health_roundtrip() {
        let resp = RemoteResponse::Health {
            status: "healthy".into(),
            uptime_secs: 3600,
            version: "1.0.0".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: RemoteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_error_roundtrip() {
        let resp = RemoteResponse::Error {
            code: 500,
            message: "内部错误".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: RemoteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    // ------------------------------------------------------------------
    // ConnectionState:默认值 + 状态转换 + 序列化
    // ------------------------------------------------------------------

    #[test]
    fn test_connection_state_default() {
        assert_eq!(ConnectionState::default(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_connection_state_serialization() {
        let states = vec![
            ConnectionState::Disconnected,
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Error,
            ConnectionState::Reconnecting,
        ];
        for state in &states {
            let json = serde_json::to_string(state).unwrap();
            let de: ConnectionState = serde_json::from_str(&json).unwrap();
            assert_eq!(de, *state);
        }
        // 验证 snake_case 序列化
        assert_eq!(
            serde_json::to_string(&ConnectionState::Disconnected).unwrap(),
            "\"disconnected\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectionState::Connected).unwrap(),
            "\"connected\""
        );
    }

    #[test]
    fn test_connection_state_transitions() {
        // 模拟状态转换:Disconnected → Connecting → Connected → Disconnected
        let mut state = ConnectionState::default();
        assert_eq!(state, ConnectionState::Disconnected);

        state = ConnectionState::Connecting;
        assert_eq!(state, ConnectionState::Connecting);

        state = ConnectionState::Connected;
        assert_eq!(state, ConnectionState::Connected);

        state = ConnectionState::Disconnected;
        assert_eq!(state, ConnectionState::Disconnected);

        // 模拟出错路径:Connecting → Error → Reconnecting → Connected
        state = ConnectionState::Connecting;
        state = ConnectionState::Error;
        assert_eq!(state, ConnectionState::Error);

        state = ConnectionState::Reconnecting;
        assert_eq!(state, ConnectionState::Reconnecting);

        state = ConnectionState::Connected;
        assert_eq!(state, ConnectionState::Connected);
    }

    // ------------------------------------------------------------------
    // RemoteRegion / RemoteScreenshot / RemoteWindowInfo 结构
    // ------------------------------------------------------------------

    #[test]
    fn test_remote_region_new() {
        let r = RemoteRegion::new(10, 20, 800, 600);
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        assert_eq!(r.width, 800);
        assert_eq!(r.height, 600);
    }

    #[test]
    fn test_remote_region_serialization() {
        let r = RemoteRegion::new(0, 0, 1920, 1080);
        let json = serde_json::to_string(&r).unwrap();
        let de: RemoteRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(de, r);
    }

    #[test]
    fn test_remote_screenshot_structure() {
        let s = RemoteScreenshot {
            data_base64: "iVBORw0KGgo=".into(),
            width: 800,
            height: 600,
            format: "png".into(),
        };
        assert_eq!(s.width, 800);
        assert_eq!(s.height, 600);
        assert_eq!(s.format, "png");
        // 序列化往返
        let json = serde_json::to_string(&s).unwrap();
        let de: RemoteScreenshot = serde_json::from_str(&json).unwrap();
        assert_eq!(de, s);
    }

    #[test]
    fn test_remote_window_info_structure() {
        let w = RemoteWindowInfo {
            title: "Visual Studio Code".into(),
            process_name: "Code.exe".into(),
            x: -100,
            y: 50,
            width: 1920,
            height: 1080,
        };
        assert_eq!(w.title, "Visual Studio Code");
        assert_eq!(w.process_name, "Code.exe");
        assert_eq!(w.x, -100);
        assert_eq!(w.y, 50);
        assert_eq!(w.width, 1920);
        assert_eq!(w.height, 1080);
        // 序列化往返
        let json = serde_json::to_string(&w).unwrap();
        let de: RemoteWindowInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(de, w);
    }

    // ------------------------------------------------------------------
    // RemoteHealthStatus 构建
    // ------------------------------------------------------------------

    #[test]
    fn test_remote_health_status_construction() {
        let h = RemoteHealthStatus {
            connected: true,
            uptime_secs: 7200,
            version: "2.0.0".into(),
            tasks_completed: 42,
        };
        assert!(h.connected);
        assert_eq!(h.uptime_secs, 7200);
        assert_eq!(h.version, "2.0.0");
        assert_eq!(h.tasks_completed, 42);
        // 序列化往返
        let json = serde_json::to_string(&h).unwrap();
        let de: RemoteHealthStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(de, h);
    }

    // ------------------------------------------------------------------
    // RemoteOperatorStats 统计
    // ------------------------------------------------------------------

    #[test]
    fn test_remote_operator_stats_default() {
        let stats = RemoteOperatorStats::default();
        assert_eq!(stats.total_commands, 0);
        assert_eq!(stats.successful, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.avg_latency_ms, 0.0);
        assert!(stats.last_command_at.is_none());
    }

    #[test]
    fn test_remote_operator_stats_serialization() {
        let stats = RemoteHealthStatus {
            connected: true,
            uptime_secs: 100,
            version: "1.0".into(),
            tasks_completed: 5,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let de: RemoteHealthStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(de, stats);
    }

    // ------------------------------------------------------------------
    // RemoteExecutionResult 结构
    // ------------------------------------------------------------------

    #[test]
    fn test_remote_execution_result_structure() {
        let r = RemoteExecutionResult {
            success: true,
            steps: 8,
            summary: "已打开记事本并输入文本".into(),
            screenshots: vec!["screenshot1_base64".into(), "screenshot2_base64".into()],
        };
        assert!(r.success);
        assert_eq!(r.steps, 8);
        assert_eq!(r.summary, "已打开记事本并输入文本");
        assert_eq!(r.screenshots.len(), 2);
        // 序列化往返
        let json = serde_json::to_string(&r).unwrap();
        let de: RemoteExecutionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(de, r);
    }

    // ------------------------------------------------------------------
    // 空 region 的 screenshot 命令
    // ------------------------------------------------------------------

    #[test]
    fn test_screenshot_command_none_region() {
        let cmd = RemoteCommand::Screenshot { region: None };
        let json = serde_json::to_string(&cmd).unwrap();
        // 验证 JSON 中 region 为 null
        assert!(json.contains("\"region\":null"));
        let de: RemoteCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
        // 确认反序列化后 region 确实为 None
        match de {
            RemoteCommand::Screenshot { region } => assert!(region.is_none()),
            other => panic!("期望 Screenshot, 得到 {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // list_windows 响应解析
    // ------------------------------------------------------------------

    #[test]
    fn test_list_windows_response_parsing() {
        let json = r#"{"window_list":{"windows":[{"title":"记事本","process_name":"notepad.exe","x":100,"y":50,"width":800,"height":600},{"title":"计算器","process_name":"calc.exe","x":200,"y":100,"width":400,"height":500}]}}"#;
        let resp: RemoteResponse = serde_json::from_str(json).unwrap();
        match resp {
            RemoteResponse::WindowList { windows } => {
                assert_eq!(windows.len(), 2);
                assert_eq!(windows[0].title, "记事本");
                assert_eq!(windows[0].process_name, "notepad.exe");
                assert_eq!(windows[0].x, 100);
                assert_eq!(windows[0].y, 50);
                assert_eq!(windows[0].width, 800);
                assert_eq!(windows[0].height, 600);
                assert_eq!(windows[1].title, "计算器");
                assert_eq!(windows[1].process_name, "calc.exe");
            }
            other => panic!("期望 WindowList, 得到 {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // RemoteOperator:创建与初始状态
    // ------------------------------------------------------------------

    #[test]
    fn test_new_operator_starts_disconnected() {
        let op = RemoteOperator::new(RemoteOperatorConfig::default());
        assert_eq!(op.state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_is_connected_false_when_not_connected() {
        let op = RemoteOperator::new(RemoteOperatorConfig::default());
        assert!(!op.is_connected().await);
    }

    #[tokio::test]
    async fn test_disconnect_on_new_operator_succeeds() {
        let op = RemoteOperator::new(RemoteOperatorConfig::default());
        op.disconnect().await.unwrap();
        assert_eq!(op.state(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_default_operator_uses_default_config() {
        let op = RemoteOperator::default();
        assert_eq!(op.state(), ConnectionState::Disconnected);
        let stats = op.stats();
        assert_eq!(stats.total_commands, 0);
    }

    // ------------------------------------------------------------------
    // command_url 构造(TLS 切换)
    // ------------------------------------------------------------------

    #[test]
    fn test_command_url_http() {
        let op = RemoteOperator::new(
            RemoteOperatorConfig::builder()
                .endpoint("192.168.1.100:7878")
                .enable_tls(false)
                .build(),
        );
        assert_eq!(op.command_url(), "http://192.168.1.100:7878/command");
    }

    #[test]
    fn test_command_url_https() {
        let op = RemoteOperator::new(
            RemoteOperatorConfig::builder()
                .endpoint("remote.example.com:443")
                .enable_tls(true)
                .build(),
        );
        assert_eq!(op.command_url(), "https://remote.example.com:443/command");
    }
}
