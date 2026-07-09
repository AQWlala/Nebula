//! T-E-C-05: OS-Controller Sidecar 守护进程。
//!
//! 作为独立进程运行 OS 控制能力,通过 IPC(Unix socket / Windows named pipe)
//! 与主进程通信。与 [`crate::sidecar::os_controller_service`] 的 RPC handler 不同,
//! 本模块实现完整的守护进程生命周期:IPC 服务器、命令分发、权限管控、运行统计、
//! 优雅关闭。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  IPC (Unix socket / Windows named pipe, JSON 行协议)
//!    ▼
//! OsControllerDaemon (守护进程)
//!    │  权限检查 → 命令分发 → 统计更新
//!    ▼
//! OsControllerService (Win32 API / 平台抽象)
//! ```
//!
//! ## 命令协议
//!
//! 每行一个 JSON 序列化的 [`OsControllerCommand`],守护进程返回一行
//! JSON 序列化的 [`OsControllerResponse`]。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

// ======================================================================
// BoundingBox — 屏幕区域矩形
// ======================================================================

/// 屏幕区域边界框(像素坐标)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    /// 左上角 X 坐标。
    pub x: f32,
    /// 左上角 Y 坐标。
    pub y: f32,
    /// 宽度。
    pub width: f32,
    /// 高度。
    pub height: f32,
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}

impl BoundingBox {
    /// 创建新的边界框。
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

// ======================================================================
// PermissionPolicy — 权限策略
// ======================================================================

/// 权限策略 — 控制 OS 操作的授权方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicy {
    /// 允许所有操作。
    AllowAll,
    /// 每次操作需询问用户确认。
    AskUser,
    /// 拒绝所有操作。
    DenyAll,
    /// 白名单模式 — 仅允许列表中的操作。
    Whitelist(Vec<String>),
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        PermissionPolicy::AskUser
    }
}

// ======================================================================
// PermissionResult — 权限检查结果
// ======================================================================

/// 权限检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionResult {
    /// 已授权。
    Granted,
    /// 已拒绝(附带原因)。
    Denied(String),
    /// 需用户确认(附带提示)。
    AskUser(String),
}

// ======================================================================
// OsControllerDaemonConfig — 守护进程配置
// ======================================================================

/// OS-Controller 守护进程配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsControllerDaemonConfig {
    /// IPC 套接字路径(Unix socket 路径 / Windows named pipe 名称来源)。
    pub ipc_socket_path: PathBuf,
    /// 是否启用视觉语言模型(VLM)屏幕分析。
    pub enable_vlm: bool,
    /// 截图轮询间隔(秒)。
    pub screenshot_interval_secs: u64,
    /// 最大并发任务数。
    pub max_concurrent_tasks: usize,
    /// 工作目录。
    pub working_dir: PathBuf,
    /// 日志级别。
    pub log_level: String,
    /// 是否自动授予权限(AskUser 策略下自动放行)。
    pub auto_grant_permissions: bool,
    /// 权限策略。
    pub permission_policy: PermissionPolicy,
}

/// 默认 IPC 套接字路径。
fn default_ipc_socket_path() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push("nebula-os-controller.sock");
    p
}

impl Default for OsControllerDaemonConfig {
    fn default() -> Self {
        Self {
            ipc_socket_path: default_ipc_socket_path(),
            enable_vlm: true,
            screenshot_interval_secs: 5,
            max_concurrent_tasks: 4,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            log_level: "info".to_string(),
            auto_grant_permissions: false,
            permission_policy: PermissionPolicy::AskUser,
        }
    }
}

impl OsControllerDaemonConfig {
    /// 创建配置构造器。
    pub fn builder() -> OsControllerDaemonConfigBuilder {
        OsControllerDaemonConfigBuilder::new()
    }
}

/// [`OsControllerDaemonConfig`] 构造器。
#[derive(Debug, Clone)]
pub struct OsControllerDaemonConfigBuilder {
    config: OsControllerDaemonConfig,
}

impl OsControllerDaemonConfigBuilder {
    /// 创建新的构造器(以默认配置为起点)。
    pub fn new() -> Self {
        Self {
            config: OsControllerDaemonConfig::default(),
        }
    }

    /// 设置 IPC 套接字路径。
    pub fn ipc_socket_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.ipc_socket_path = path.into();
        self
    }

    /// 设置是否启用 VLM。
    pub fn enable_vlm(mut self, enable: bool) -> Self {
        self.config.enable_vlm = enable;
        self
    }

    /// 设置截图轮询间隔(秒)。
    pub fn screenshot_interval_secs(mut self, secs: u64) -> Self {
        self.config.screenshot_interval_secs = secs;
        self
    }

    /// 设置最大并发任务数。
    pub fn max_concurrent_tasks(mut self, max: usize) -> Self {
        self.config.max_concurrent_tasks = max;
        self
    }

    /// 设置工作目录。
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.config.working_dir = dir.into();
        self
    }

    /// 设置日志级别。
    pub fn log_level(mut self, level: impl Into<String>) -> Self {
        self.config.log_level = level.into();
        self
    }

    /// 设置是否自动授予权限。
    pub fn auto_grant_permissions(mut self, auto: bool) -> Self {
        self.config.auto_grant_permissions = auto;
        self
    }

    /// 设置权限策略。
    pub fn permission_policy(mut self, policy: PermissionPolicy) -> Self {
        self.config.permission_policy = policy;
        self
    }

    /// 构建最终配置。
    pub fn build(self) -> OsControllerDaemonConfig {
        self.config
    }
}

impl Default for OsControllerDaemonConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ======================================================================
// OsControllerCommand — IPC 命令
// ======================================================================

/// OS-Controller IPC 命令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsControllerCommand {
    /// 截图(可选指定区域)。
    Screenshot { region: Option<BoundingBox> },
    /// 鼠标点击。
    Click { x: f32, y: f32 },
    /// 输入文本。
    Type { text: String },
    /// 按键。
    KeyPress { key: String },
    /// 滚动。
    Scroll { direction: String, amount: f32 },
    /// 启动应用。
    LaunchApp { path: String },
    /// 获取前台窗口信息。
    GetForegroundWindow,
    /// 分析屏幕内容。
    AnalyzeScreen { goal: String },
    /// 执行目标(多步操作循环)。
    ExecuteGoal { goal: String, max_steps: usize },
    /// 健康检查。
    HealthCheck,
    /// 关闭守护进程。
    Shutdown,
}

impl OsControllerCommand {
    /// 返回命令对应的权限操作名(用于权限检查)。
    pub fn action_name(&self) -> &'static str {
        match self {
            OsControllerCommand::Screenshot { .. } => "screenshot",
            OsControllerCommand::Click { .. } => "click",
            OsControllerCommand::Type { .. } => "type",
            OsControllerCommand::KeyPress { .. } => "key_press",
            OsControllerCommand::Scroll { .. } => "scroll",
            OsControllerCommand::LaunchApp { .. } => "launch_app",
            OsControllerCommand::GetForegroundWindow => "get_foreground_window",
            OsControllerCommand::AnalyzeScreen { .. } => "analyze_screen",
            OsControllerCommand::ExecuteGoal { .. } => "execute_goal",
            OsControllerCommand::HealthCheck => "health_check",
            OsControllerCommand::Shutdown => "shutdown",
        }
    }
}

// ======================================================================
// OsControllerResponse — IPC 响应
// ======================================================================

/// OS-Controller IPC 响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsControllerResponse {
    /// 截图结果。
    Screenshot {
        path: String,
        width: u32,
        height: u32,
    },
    /// 操作完成。
    ActionCompleted { success: bool, message: String },
    /// 窗口信息。
    WindowInfo {
        title: String,
        process_name: String,
        bbox: BoundingBox,
    },
    /// 屏幕分析结果。
    Analysis {
        description: String,
        elements: Vec<String>,
    },
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
        tasks_completed: u64,
    },
    /// 错误。
    Error { code: i32, message: String },
}

// ======================================================================
// DaemonStats — 守护进程运行统计
// ======================================================================

/// 守护进程运行统计。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaemonStats {
    /// 运行时长(秒)。
    pub uptime_secs: u64,
    /// 接收的命令总数。
    pub commands_received: u64,
    /// 成功的命令数。
    pub commands_succeeded: u64,
    /// 失败的命令数。
    pub commands_failed: u64,
    /// 最近一次命令的操作名。
    pub last_command: Option<String>,
    /// 最近一次错误信息。
    pub last_error: Option<String>,
}

// ======================================================================
// OsControllerDaemon — 守护进程
// ======================================================================

/// OS-Controller Sidecar 守护进程。
///
/// 作为独立进程运行,通过 IPC 接收命令、执行 OS 控制操作、返回结果。
/// 支持权限管控、运行统计和优雅关闭。
///
/// ## 使用方式
///
/// ```no_run
/// # use nebula::sidecar::os_controller_daemon::*;
/// # async fn example() -> anyhow::Result<()> {
/// let config = OsControllerDaemonConfig::builder()
///     .permission_policy(PermissionPolicy::AllowAll)
///     .build();
/// let daemon = OsControllerDaemon::new(config);
/// daemon.run().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct OsControllerDaemon {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// 守护进程配置。
    config: OsControllerDaemonConfig,
    /// 运行统计(受 Mutex 保护)。
    stats: Mutex<DaemonStats>,
    /// 关闭标志(原子布尔)。
    shutdown_flag: AtomicBool,
    /// 取消令牌(用于通知 accept loop 退出)。
    cancel: CancellationToken,
    /// 启动时间。
    started_at: Instant,
}

impl OsControllerDaemon {
    /// 创建新的守护进程实例。
    pub fn new(config: OsControllerDaemonConfig) -> Self {
        info!(
            target: "nebula.sidecar.os_controller_daemon",
            socket = %config.ipc_socket_path.display(),
            policy = ?config.permission_policy,
            "OsControllerDaemon created"
        );
        Self {
            inner: Arc::new(Inner {
                config,
                stats: Mutex::new(DaemonStats::default()),
                shutdown_flag: AtomicBool::new(false),
                cancel: CancellationToken::new(),
                started_at: Instant::now(),
            }),
        }
    }

    /// 守护进程主循环 — 启动 IPC 服务器并等待关闭信号。
    ///
    /// 调用后会阻塞直到收到 `Shutdown` 命令或外部取消。
    pub async fn run(self) -> Result<()> {
        info!("OS-Controller daemon starting");
        self.start_ipc_server().await?;
        // 等待关闭信号(由 Shutdown 命令或 shutdown() 方法触发)
        self.inner.cancel.cancelled().await;
        info!("OS-Controller daemon stopped");
        Ok(())
    }

    /// 处理 IPC 命令 — 权限检查 → 分发 → 统计更新。
    #[instrument(skip(self))]
    pub async fn handle_command(&self, cmd: OsControllerCommand) -> Result<OsControllerResponse> {
        let action = cmd.action_name();
        debug!(action = action, "handling command");

        // 更新接收统计
        {
            let mut stats = self.inner.stats.lock();
            stats.commands_received += 1;
            stats.last_command = Some(action.to_string());
            stats.uptime_secs = self.inner.started_at.elapsed().as_secs();
        }

        // 权限检查 → 分发
        let response = match self.check_permission(action) {
            PermissionResult::Granted => self.dispatch(cmd).await,
            PermissionResult::Denied(msg) => OsControllerResponse::Error {
                code: 403,
                message: msg,
            },
            PermissionResult::AskUser(msg) => OsControllerResponse::Error {
                code: 401,
                message: msg,
            },
        };

        // 更新成功/失败统计
        {
            let mut stats = self.inner.stats.lock();
            if matches!(response, OsControllerResponse::Error { .. }) {
                stats.commands_failed += 1;
                if let OsControllerResponse::Error { message, .. } = &response {
                    stats.last_error = Some(message.clone());
                }
            } else {
                stats.commands_succeeded += 1;
            }
        }

        Ok(response)
    }

    /// 启动 IPC 服务器 — 绑定套接字并进入 accept 循环(后台任务)。
    pub async fn start_ipc_server(&self) -> Result<()> {
        let daemon = self.clone();
        tokio::spawn(async move {
            if let Err(e) = daemon.accept_loop().await {
                error!(error = %e, "IPC accept loop terminated with error");
            }
        });
        Ok(())
    }

    /// 优雅关闭守护进程。
    pub async fn shutdown(&self) -> Result<()> {
        info!("OS-Controller daemon shutdown requested");
        self.inner.shutdown_flag.store(true, Ordering::SeqCst);
        self.inner.cancel.cancel();
        Ok(())
    }

    /// 权限检查 — 根据策略判断操作是否允许。
    pub fn check_permission(&self, action: &str) -> PermissionResult {
        match &self.inner.config.permission_policy {
            PermissionPolicy::AllowAll => PermissionResult::Granted,
            PermissionPolicy::DenyAll => {
                PermissionResult::Denied(format!("操作 '{}' 被拒绝(DenyAll 策略)", action))
            }
            PermissionPolicy::AskUser => {
                if self.inner.config.auto_grant_permissions {
                    PermissionResult::Granted
                } else {
                    PermissionResult::AskUser(format!(
                        "操作 '{}' 需要用户确认(AskUser 策略)",
                        action
                    ))
                }
            }
            PermissionPolicy::Whitelist(allowed) => {
                if allowed.iter().any(|a| a == action) {
                    PermissionResult::Granted
                } else {
                    PermissionResult::Denied(format!(
                        "操作 '{}' 不在白名单中(Whitelist 策略)",
                        action
                    ))
                }
            }
        }
    }

    /// 获取当前运行统计快照(含实时 uptime)。
    pub fn stats(&self) -> DaemonStats {
        let mut s = self.inner.stats.lock().clone();
        s.uptime_secs = self.inner.started_at.elapsed().as_secs();
        s
    }

    /// 是否已收到关闭信号。
    pub fn is_shutdown(&self) -> bool {
        self.inner.shutdown_flag.load(Ordering::SeqCst)
    }

    // ------------------------------------------------------------------
    // 内部方法
    // ------------------------------------------------------------------

    /// 命令分发(权限检查通过后调用)。
    ///
    /// 各 OS 操作的 stub — 真实实现将委托 [`OsControllerService`](crate::os::controller::OsControllerService)
    /// 或对应平台 API。
    async fn dispatch(&self, cmd: OsControllerCommand) -> OsControllerResponse {
        match cmd {
            OsControllerCommand::Screenshot { region: _ } => {
                // TODO: 接入 screenshots crate 执行真实截图
                OsControllerResponse::Screenshot {
                    path: String::new(),
                    width: 0,
                    height: 0,
                }
            }
            OsControllerCommand::Click { x, y } => {
                // TODO: 接入 OsControllerService 模拟鼠标点击
                OsControllerResponse::ActionCompleted {
                    success: true,
                    message: format!("点击 ({}, {}) 已模拟", x, y),
                }
            }
            OsControllerCommand::Type { text } => {
                // TODO: 接入 OsControllerService 模拟键盘输入
                OsControllerResponse::ActionCompleted {
                    success: true,
                    message: format!("已输入 {} 个字符", text.chars().count()),
                }
            }
            OsControllerCommand::KeyPress { key } => {
                // TODO: 接入 OsControllerService 模拟按键
                OsControllerResponse::ActionCompleted {
                    success: true,
                    message: format!("按键 '{}' 已模拟", key),
                }
            }
            OsControllerCommand::Scroll { direction, amount } => {
                // TODO: 接入 OsControllerService 模拟滚动
                OsControllerResponse::ActionCompleted {
                    success: true,
                    message: format!("滚动 {} {}", amount, direction),
                }
            }
            OsControllerCommand::LaunchApp { path } => {
                // TODO: 接入 tokio::process::Command 启动应用
                OsControllerResponse::ActionCompleted {
                    success: true,
                    message: format!("应用启动请求已发送: {}", path),
                }
            }
            OsControllerCommand::GetForegroundWindow => {
                // TODO: 委托 OsControllerService::get_foreground_window() 获取真实窗口信息
                OsControllerResponse::WindowInfo {
                    title: String::new(),
                    process_name: String::new(),
                    bbox: BoundingBox::default(),
                }
            }
            OsControllerCommand::AnalyzeScreen { goal } => {
                if !self.inner.config.enable_vlm {
                    return OsControllerResponse::Error {
                        code: 501,
                        message: "VLM 未启用,屏幕分析不可用".to_string(),
                    };
                }
                // TODO: 接入 VLM 进行屏幕分析
                OsControllerResponse::Analysis {
                    description: format!("已分析屏幕(目标: {})", goal),
                    elements: Vec::new(),
                }
            }
            OsControllerCommand::ExecuteGoal { goal, max_steps } => {
                // TODO: 接入目标执行引擎(截图 → 分析 → 操作 循环)
                OsControllerResponse::ExecutionResult {
                    success: true,
                    steps: 0,
                    summary: format!("目标 '{}' 执行完成(max_steps={})", goal, max_steps),
                }
            }
            OsControllerCommand::HealthCheck => {
                let s = self.stats();
                OsControllerResponse::Health {
                    status: "healthy".to_string(),
                    uptime_secs: s.uptime_secs,
                    tasks_completed: s.commands_succeeded,
                }
            }
            OsControllerCommand::Shutdown => {
                self.shutdown().await.ok();
                OsControllerResponse::ActionCompleted {
                    success: true,
                    message: "关闭指令已接收".to_string(),
                }
            }
        }
    }

    /// IPC accept 循环(Unix socket)。
    #[cfg(unix)]
    async fn accept_loop(&self) -> Result<()> {
        use tokio::net::UnixListener;

        // 清理可能残留的旧套接字文件
        let _ = std::fs::remove_file(&self.inner.config.ipc_socket_path);
        let listener = UnixListener::bind(&self.inner.config.ipc_socket_path)?;
        info!(
            path = %self.inner.config.ipc_socket_path.display(),
            "IPC Unix socket listening"
        );

        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
                            let daemon = self.clone();
                            tokio::spawn(async move {
                                daemon.handle_connection(stream).await;
                            });
                        }
                        Err(e) => {
                            warn!(error = %e, "accept failed");
                        }
                    }
                }
                _ = self.inner.cancel.cancelled() => {
                    info!("IPC accept loop cancelled");
                    break;
                }
            }
        }
        Ok(())
    }

    /// IPC accept 循环(Windows named pipe)。
    #[cfg(windows)]
    async fn accept_loop(&self) -> Result<()> {
        use tokio::net::windows::named_pipe::ServerOptions;

        let pipe_name = self.pipe_name();
        info!(pipe = %pipe_name, "IPC named pipe listening");

        let mut first = true;
        loop {
            let server = ServerOptions::new()
                .first_pipe_instance(first)
                .create(&pipe_name)?;
            first = false;

            tokio::select! {
                accept = server.connect() => {
                    match accept {
                        Ok(()) => {
                            let daemon = self.clone();
                            tokio::spawn(async move {
                                daemon.handle_connection(server).await;
                            });
                        }
                        Err(e) => {
                            warn!(error = %e, "pipe connect failed");
                        }
                    }
                }
                _ = self.inner.cancel.cancelled() => {
                    info!("IPC accept loop cancelled");
                    break;
                }
            }
        }
        Ok(())
    }

    /// 从配置的 ipc_socket_path 派生 Windows named pipe 名称。
    #[cfg(windows)]
    fn pipe_name(&self) -> String {
        let name = self
            .inner
            .config
            .ipc_socket_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("nebula-os-controller");
        // 去除可能的 .sock 后缀
        let name = name.trim_end_matches(".sock");
        format!(r"\\.\pipe\{}", name)
    }

    /// 处理单个 IPC 连接 — 逐行读取 JSON 命令、返回 JSON 响应。
    async fn handle_connection<RW>(&self, stream: RW)
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF,客户端关闭连接
                Ok(_) => {}
                Err(e) => {
                    warn!(error = %e, "read failed");
                    break;
                }
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // 反序列化命令(仅一次),同时判断是否为 Shutdown
            let parsed = serde_json::from_str::<OsControllerCommand>(trimmed);
            let is_shutdown = matches!(&parsed, Ok(OsControllerCommand::Shutdown));

            let response =
                match parsed {
                    Ok(cmd) => self.handle_command(cmd).await.unwrap_or_else(|e| {
                        OsControllerResponse::Error {
                            code: 500,
                            message: e.to_string(),
                        }
                    }),
                    Err(e) => OsControllerResponse::Error {
                        code: 400,
                        message: format!("无效命令: {}", e),
                    },
                };

            // 序列化并写回响应(单行 JSON)
            let resp_json = serde_json::to_string(&response).unwrap_or_else(|_| {
                r#"{"error":{"code":500,"message":"response serialization failed"}}"#.to_string()
            });
            if write_half.write_all(resp_json.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }

            // Shutdown 命令处理完毕后关闭当前连接
            if is_shutdown {
                break;
            }
        }
    }
}

// ======================================================================
// 单元测试
// ======================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // 辅助函数
    // ------------------------------------------------------------------

    /// 构造指定权限策略的守护进程实例。
    fn make_daemon(policy: PermissionPolicy) -> OsControllerDaemon {
        let config = OsControllerDaemonConfig::builder()
            .permission_policy(policy)
            .build();
        OsControllerDaemon::new(config)
    }

    // ------------------------------------------------------------------
    // 配置:默认值 / builder
    // ------------------------------------------------------------------

    #[test]
    fn test_config_defaults() {
        let cfg = OsControllerDaemonConfig::default();
        assert!(cfg.enable_vlm);
        assert_eq!(cfg.screenshot_interval_secs, 5);
        assert_eq!(cfg.max_concurrent_tasks, 4);
        assert_eq!(cfg.log_level, "info");
        assert!(!cfg.auto_grant_permissions);
        assert_eq!(cfg.permission_policy, PermissionPolicy::AskUser);
    }

    #[test]
    fn test_config_builder() {
        let cfg = OsControllerDaemonConfig::builder()
            .ipc_socket_path("/tmp/test.sock")
            .enable_vlm(false)
            .screenshot_interval_secs(10)
            .max_concurrent_tasks(8)
            .working_dir("/tmp/work")
            .log_level("debug")
            .auto_grant_permissions(true)
            .permission_policy(PermissionPolicy::AllowAll)
            .build();
        assert_eq!(cfg.ipc_socket_path, PathBuf::from("/tmp/test.sock"));
        assert!(!cfg.enable_vlm);
        assert_eq!(cfg.screenshot_interval_secs, 10);
        assert_eq!(cfg.max_concurrent_tasks, 8);
        assert_eq!(cfg.working_dir, PathBuf::from("/tmp/work"));
        assert_eq!(cfg.log_level, "debug");
        assert!(cfg.auto_grant_permissions);
        assert_eq!(cfg.permission_policy, PermissionPolicy::AllowAll);
    }

    // ------------------------------------------------------------------
    // PermissionPolicy 序列化
    // ------------------------------------------------------------------

    #[test]
    fn test_permission_policy_allow_all_serialize() {
        let p = PermissionPolicy::AllowAll;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"allow_all\"");
        let de: PermissionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(de, p);
    }

    #[test]
    fn test_permission_policy_deny_all_serialize() {
        let p = PermissionPolicy::DenyAll;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"deny_all\"");
        let de: PermissionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(de, p);
    }

    #[test]
    fn test_permission_policy_whitelist_serialize() {
        let p = PermissionPolicy::Whitelist(vec!["screenshot".into(), "click".into()]);
        let json = serde_json::to_string(&p).unwrap();
        let de: PermissionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(de, p);
    }

    // ------------------------------------------------------------------
    // OsControllerCommand 序列化往返
    // ------------------------------------------------------------------

    #[test]
    fn test_command_click_roundtrip() {
        let cmd = OsControllerCommand::Click { x: 100.5, y: 200.3 };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: OsControllerCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_screenshot_roundtrip() {
        let cmd = OsControllerCommand::Screenshot {
            region: Some(BoundingBox::new(10.0, 20.0, 800.0, 600.0)),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: OsControllerCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_health_check_roundtrip() {
        let cmd = OsControllerCommand::HealthCheck;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"health_check\"");
        let de: OsControllerCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    #[test]
    fn test_command_execute_goal_roundtrip() {
        let cmd = OsControllerCommand::ExecuteGoal {
            goal: "打开记事本".into(),
            max_steps: 10,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let de: OsControllerCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(de, cmd);
    }

    // ------------------------------------------------------------------
    // OsControllerResponse 序列化
    // ------------------------------------------------------------------

    #[test]
    fn test_response_health_serialize() {
        let resp = OsControllerResponse::Health {
            status: "healthy".into(),
            uptime_secs: 42,
            tasks_completed: 10,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: OsControllerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_error_serialize() {
        let resp = OsControllerResponse::Error {
            code: 403,
            message: "denied".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: OsControllerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    #[test]
    fn test_response_execution_result_serialize() {
        let resp = OsControllerResponse::ExecutionResult {
            success: true,
            steps: 3,
            summary: "done".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: OsControllerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(de, resp);
    }

    // ------------------------------------------------------------------
    // 权限检查:各种策略
    // ------------------------------------------------------------------

    #[test]
    fn test_check_permission_allow_all() {
        let d = make_daemon(PermissionPolicy::AllowAll);
        assert_eq!(d.check_permission("screenshot"), PermissionResult::Granted);
        assert_eq!(d.check_permission("click"), PermissionResult::Granted);
    }

    #[test]
    fn test_check_permission_deny_all() {
        let d = make_daemon(PermissionPolicy::DenyAll);
        match d.check_permission("screenshot") {
            PermissionResult::Denied(_) => {}
            other => panic!("期望 Denied, 得到 {:?}", other),
        }
    }

    #[test]
    fn test_check_permission_ask_user() {
        let d = make_daemon(PermissionPolicy::AskUser);
        match d.check_permission("screenshot") {
            PermissionResult::AskUser(_) => {}
            other => panic!("期望 AskUser, 得到 {:?}", other),
        }
    }

    #[test]
    fn test_check_permission_ask_user_auto_grant() {
        let config = OsControllerDaemonConfig::builder()
            .permission_policy(PermissionPolicy::AskUser)
            .auto_grant_permissions(true)
            .build();
        let d = OsControllerDaemon::new(config);
        assert_eq!(d.check_permission("screenshot"), PermissionResult::Granted);
    }

    #[test]
    fn test_check_permission_whitelist_allowed() {
        let d = make_daemon(PermissionPolicy::Whitelist(vec![
            "screenshot".into(),
            "click".into(),
        ]));
        assert_eq!(d.check_permission("screenshot"), PermissionResult::Granted);
        assert_eq!(d.check_permission("click"), PermissionResult::Granted);
    }

    #[test]
    fn test_check_permission_whitelist_denied() {
        let d = make_daemon(PermissionPolicy::Whitelist(vec!["screenshot".into()]));
        match d.check_permission("click") {
            PermissionResult::Denied(_) => {}
            other => panic!("期望 Denied, 得到 {:?}", other),
        }
        // 白名单内的仍放行
        assert_eq!(d.check_permission("screenshot"), PermissionResult::Granted);
    }

    // ------------------------------------------------------------------
    // 命令分发逻辑
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_handle_command_health_check() {
        let d = make_daemon(PermissionPolicy::AllowAll);
        let resp = d
            .handle_command(OsControllerCommand::HealthCheck)
            .await
            .unwrap();
        match resp {
            OsControllerResponse::Health { status, .. } => {
                assert_eq!(status, "healthy");
            }
            other => panic!("期望 Health, 得到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_command_shutdown_sets_flag() {
        let d = make_daemon(PermissionPolicy::AllowAll);
        assert!(!d.is_shutdown());
        let resp = d
            .handle_command(OsControllerCommand::Shutdown)
            .await
            .unwrap();
        match resp {
            OsControllerResponse::ActionCompleted { success, .. } => {
                assert!(success);
            }
            other => panic!("期望 ActionCompleted, 得到 {:?}", other),
        }
        assert!(d.is_shutdown());
    }

    #[tokio::test]
    async fn test_handle_command_permission_denied() {
        let d = make_daemon(PermissionPolicy::DenyAll);
        let resp = d
            .handle_command(OsControllerCommand::Click { x: 1.0, y: 2.0 })
            .await
            .unwrap();
        match resp {
            OsControllerResponse::Error { code, .. } => {
                assert_eq!(code, 403);
            }
            other => panic!("期望 Error 403, 得到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_command_click_dispatch() {
        let d = make_daemon(PermissionPolicy::AllowAll);
        let resp = d
            .handle_command(OsControllerCommand::Click { x: 10.0, y: 20.0 })
            .await
            .unwrap();
        match resp {
            OsControllerResponse::ActionCompleted { success, .. } => {
                assert!(success);
            }
            other => panic!("期望 ActionCompleted, 得到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_command_analyze_screen_vlm_disabled() {
        let config = OsControllerDaemonConfig::builder()
            .permission_policy(PermissionPolicy::AllowAll)
            .enable_vlm(false)
            .build();
        let d = OsControllerDaemon::new(config);
        let resp = d
            .handle_command(OsControllerCommand::AnalyzeScreen {
                goal: "test".into(),
            })
            .await
            .unwrap();
        match resp {
            OsControllerResponse::Error { code, .. } => {
                assert_eq!(code, 501);
            }
            other => panic!("期望 Error 501, 得到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_command_analyze_screen_vlm_enabled() {
        let config = OsControllerDaemonConfig::builder()
            .permission_policy(PermissionPolicy::AllowAll)
            .enable_vlm(true)
            .build();
        let d = OsControllerDaemon::new(config);
        let resp = d
            .handle_command(OsControllerCommand::AnalyzeScreen {
                goal: "test".into(),
            })
            .await
            .unwrap();
        match resp {
            OsControllerResponse::Analysis { .. } => {}
            other => panic!("期望 Analysis, 得到 {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // DaemonStats 统计
    // ------------------------------------------------------------------

    #[test]
    fn test_daemon_stats_default() {
        let stats = DaemonStats::default();
        assert_eq!(stats.uptime_secs, 0);
        assert_eq!(stats.commands_received, 0);
        assert_eq!(stats.commands_succeeded, 0);
        assert_eq!(stats.commands_failed, 0);
        assert_eq!(stats.last_command, None);
        assert_eq!(stats.last_error, None);
    }

    #[tokio::test]
    async fn test_daemon_stats_updated_after_command() {
        let d = make_daemon(PermissionPolicy::AllowAll);
        d.handle_command(OsControllerCommand::HealthCheck)
            .await
            .unwrap();
        let stats = d.stats();
        assert_eq!(stats.commands_received, 1);
        assert_eq!(stats.commands_succeeded, 1);
        assert_eq!(stats.commands_failed, 0);
        assert_eq!(stats.last_command, Some("health_check".to_string()));
        assert!(stats.last_error.is_none());
    }

    #[tokio::test]
    async fn test_daemon_stats_tracks_failures() {
        let d = make_daemon(PermissionPolicy::DenyAll);
        d.handle_command(OsControllerCommand::Click { x: 1.0, y: 2.0 })
            .await
            .unwrap();
        let stats = d.stats();
        assert_eq!(stats.commands_received, 1);
        assert_eq!(stats.commands_failed, 1);
        assert_eq!(stats.commands_succeeded, 0);
        assert!(stats.last_error.is_some());
    }

    // ------------------------------------------------------------------
    // action_name 映射
    // ------------------------------------------------------------------

    #[test]
    fn test_command_action_name() {
        assert_eq!(
            OsControllerCommand::HealthCheck.action_name(),
            "health_check"
        );
        assert_eq!(OsControllerCommand::Shutdown.action_name(), "shutdown");
        assert_eq!(
            OsControllerCommand::GetForegroundWindow.action_name(),
            "get_foreground_window"
        );
        assert_eq!(
            OsControllerCommand::Click { x: 0.0, y: 0.0 }.action_name(),
            "click"
        );
        assert_eq!(
            OsControllerCommand::Screenshot { region: None }.action_name(),
            "screenshot"
        );
    }

    // ------------------------------------------------------------------
    // shutdown 方法 / 标志
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_shutdown_method_sets_flag() {
        let d = make_daemon(PermissionPolicy::AllowAll);
        assert!(!d.is_shutdown());
        d.shutdown().await.unwrap();
        assert!(d.is_shutdown());
    }

    // ------------------------------------------------------------------
    // BoundingBox
    // ------------------------------------------------------------------

    #[test]
    fn test_bounding_box_new_and_default() {
        let b = BoundingBox::new(10.0, 20.0, 800.0, 600.0);
        assert_eq!(b.x, 10.0);
        assert_eq!(b.y, 20.0);
        assert_eq!(b.width, 800.0);
        assert_eq!(b.height, 600.0);
        let d = BoundingBox::default();
        assert_eq!(d, BoundingBox::new(0.0, 0.0, 0.0, 0.0));
    }
}
