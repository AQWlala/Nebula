//! T-E-S-32: MCP Server Registry — stdio 子进程生命周期管理。
//!
//! 借鉴 [`SidecarManager`](crate::sidecar::SidecarManager) 模式,管理本地 MCP
//! 服务器(stdio 子进程)的启动/停止/健康检查/崩溃重启。
//!
//! ## 设计要点
//!
//! * 持有 `Arc<McpManager>`,所有 client 操作(tools/list, tools/call)走 mcp_manager。
//! * 进程生命周期(spawn/monitor/restart/kill)由本 registry 管理。
//! * supervisor_loop 每 5s tick:健康检查 + 崩溃重启限流(3 次/小时)。
//! * 日志:每个 server 一个 `mcp-<name>.log` 文件(覆盖模式)。
//!
//! ## 与 T-E-S-31 的协作
//!
//! T-E-S-31 subagent 扩展 `McpServerConfig`(args/env/auto_restart/health_check_interval_secs)
//! 和 `StdioTransport::spawn(program, args, env)` + `take_stderr()`。本模块当前使用
//! 现有 API(command 字符串),待 T-E-S-31 完成后可增强 stderr 捕获与参数化 spawn。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::client::McpManager;
use super::config::{McpServerConfig, McpTransportType};

/// MCP server 运行状态。
///
/// 借鉴 [`SidecarStatus`](crate::sidecar::SidecarStatus),新增 `Disabled` 表示
/// 因重启次数超限被禁用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase", tag = "state")]
pub enum McpServerStatus {
    /// 未启动(手动停止或尚未 start)。
    Stopped,
    /// 启动中(已调用 connect,等待握手完成)。
    Starting,
    /// 运行中(握手成功,可接受 tools/list / tools/call)。
    Running,
    /// 已崩溃,等待 supervisor 重启或手动处理。
    Crashed { reason: String },
    /// 正在重启(supervisor 触发)。
    Restarting,
    /// 已禁用(1 小时内重启 ≥ 3 次,需手动 start 重置)。
    Disabled,
}

/// 单个 MCP server 的运行时状态。
struct McpServerRuntime {
    status: McpServerStatus,
    pid: Option<u32>,
    started_at: Option<Instant>,
    restart_count: u32,
    /// 滑动窗口(1h)内的重启时间戳,用于限流。
    restart_timestamps: Vec<Instant>,
    last_crash: Option<Instant>,
    last_health_check: Option<Instant>,
    log_path: PathBuf,
}

impl McpServerRuntime {
    fn new(log_path: PathBuf) -> Self {
        Self {
            status: McpServerStatus::Stopped,
            pid: None,
            started_at: None,
            restart_count: 0,
            restart_timestamps: Vec::new(),
            last_crash: None,
            last_health_check: None,
            log_path,
        }
    }
}

/// 对外暴露的 server 信息(序列化给前端)。
#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    pub name: String,
    pub status: McpServerStatus,
    pub pid: Option<u32>,
    /// 启动后经过的秒数(None 表示未启动)。
    pub uptime_secs: Option<u64>,
    pub restart_count: u32,
    pub log_path: String,
}

/// mcp_servers.json 文档结构。
///
/// 注:T-E-S-31 subagent 将在 `config.rs` 添加 `McpServersConfig`。本结构
/// 为本地反序列化用,字段兼容,主 agent 集成时可切换到 `config::McpServersConfig`。
#[derive(Debug, Deserialize)]
struct McpServersDocument {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    servers: Vec<McpServerConfig>,
}

fn default_version() -> u32 {
    1
}

struct Inner {
    /// 已加载的 server 配置(按 name 索引)。
    configs: RwLock<HashMap<String, McpServerConfig>>,
    /// 运行时状态(按 name 索引)。
    runtimes: Mutex<HashMap<String, McpServerRuntime>>,
    /// 共享的 MCP manager(所有 client 操作走这里)。
    mcp_manager: Arc<McpManager>,
    /// supervisor 取消令牌。
    cancel: CancellationToken,
    /// supervisor 任务句柄。
    supervisor: Mutex<Option<JoinHandle<()>>>,
    /// 状态变更通知(用于 wait_ready 等)。
    state_change: Notify,
    /// 每小时最大重启次数(滑动窗口)。
    max_restarts_per_hour: u32,
    /// 日志目录(mcp-<name>.log 所在目录)。
    log_dir: PathBuf,
}

/// MCP Server Registry — 管理所有本地 MCP 服务器(stdio)的生命周期。
///
/// ## 使用方式
///
/// ```no_run
/// # use std::sync::Arc;
/// # use std::path::PathBuf;
/// # use nebula_lib::mcp::client::McpManager;
/// # use nebula_lib::mcp::registry::McpServerRegistry;
/// # async fn example() -> anyhow::Result<()> {
/// let manager = Arc::new(McpManager::new());
/// let registry = McpServerRegistry::new(manager, PathBuf::from("/tmp/logs"));
/// registry.load_config(std::path::Path::new("/tmp/mcp_servers.json")).await?;
/// registry.bootstrap().await?;
/// let info = registry.list();
/// registry.stop_all().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct McpServerRegistry {
    inner: Arc<Inner>,
}

impl McpServerRegistry {
    /// 创建新的 registry。
    pub fn new(mcp_manager: Arc<McpManager>, log_dir: PathBuf) -> Self {
        info!(target: "nebula.mcp.registry", dir = %log_dir.display(), "MCP registry initialized");
        Self {
            inner: Arc::new(Inner {
                configs: RwLock::new(HashMap::new()),
                runtimes: Mutex::new(HashMap::new()),
                mcp_manager,
                cancel: CancellationToken::new(),
                supervisor: Mutex::new(None),
                state_change: Notify::new(),
                max_restarts_per_hour: 3,
                log_dir,
            }),
        }
    }

    /// 从 mcp_servers.json 加载配置并注册到 mcp_manager。
    ///
    /// 文件不存在时返回 Ok(空配置),不阻断启动。
    pub async fn load_config(&self, path: &Path) -> Result<()> {
        let content = if path.exists() {
            tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("failed to read mcp_servers.json: {}", path.display()))?
        } else {
            info!(target: "nebula.mcp.registry", path = %path.display(), "mcp_servers.json not found, starting with empty config");
            return Ok(());
        };

        let doc: McpServersDocument = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse mcp_servers.json: {}", path.display()))?;

        let mut configs = self.inner.configs.write();
        for server_cfg in doc.servers {
            let name = server_cfg.name.clone();
            // 注册到 mcp_manager(add_server 创建未连接的 client)
            self.inner.mcp_manager.add_server(server_cfg.clone());
            // 初始化 runtime
            let log_path = self.log_path_for(&name);
            let mut runtimes = self.inner.runtimes.lock();
            runtimes
                .entry(name.clone())
                .or_insert_with(|| McpServerRuntime::new(log_path));
            configs.insert(name, server_cfg);
        }
        info!(target: "nebula.mcp.registry", count = configs.len(), "loaded MCP server configs");
        Ok(())
    }

    /// 启动所有 enabled 服务器 + 启动 supervisor。
    pub async fn bootstrap(&self) -> Result<()> {
        info!(target: "nebula.mcp.registry", "bootstrap: starting all enabled MCP servers");

        let enabled_names: Vec<String> = {
            let configs = self.inner.configs.read();
            configs
                .iter()
                .filter(|(_, c)| c.enabled)
                .map(|(n, _)| n.clone())
                .collect()
        };

        for name in &enabled_names {
            if let Err(e) = self.start(name).await {
                warn!(target: "nebula.mcp.registry", server = %name, error = %e,
                    "bootstrap: failed to start server, supervisor will retry");
            }
        }

        self.ensure_supervisor();
        info!(target: "nebula.mcp.registry", "bootstrap complete");
        Ok(())
    }

    /// 手动启动单个 server。
    ///
    /// 流程:
    /// 1. 读 configs 拿 McpServerConfig
    /// 2. mcp_manager.add_server(config)(若已存在则覆盖)
    /// 3. mcp_manager.connect_all()(连接所有未连接 client)
    /// 4. 通过 invoke_tool ping 检测连接是否成功
    /// 5. 置 Running / Crashed
    pub async fn start(&self, name: &str) -> Result<()> {
        let config = {
            let configs = self.inner.configs.read();
            configs
                .get(name)
                .cloned()
                .with_context(|| format!("MCP server '{}' not found in configs", name))?
        };

        // 标记 Starting
        {
            let mut runtimes = self.inner.runtimes.lock();
            let log_path = self.log_path_for(name);
            let rt = runtimes
                .entry(name.to_string())
                .or_insert_with(|| McpServerRuntime::new(log_path));
            rt.status = McpServerStatus::Starting;
        }
        self.write_log(name, &format!("[{}] starting server", chrono::Local::now()));

        // 注册到 mcp_manager(add_server 覆盖旧 client)
        self.inner.mcp_manager.add_server(config.clone());

        // 连接(带超时,防止 handshake 挂起)
        let connect_result = tokio::time::timeout(
            Duration::from_secs(15),
            self.inner.mcp_manager.connect_all(),
        )
        .await;

        if let Err(_elapsed) = connect_result {
            warn!(target: "nebula.mcp.registry", server = %name,
                "connect_all timed out during start");
        }

        // 检测连接是否成功:用 invoke_tool 发一个 ping
        let connected = self.check_connected(name).await;

        {
            let mut runtimes = self.inner.runtimes.lock();
            if let Some(rt) = runtimes.get_mut(name) {
                if connected {
                    rt.status = McpServerStatus::Running;
                    rt.started_at = Some(Instant::now());
                    info!(target: "nebula.mcp.registry", server = %name, "MCP server is running");
                    self.write_log(
                        name,
                        &format!("[{}] server started (running)", chrono::Local::now()),
                    );
                } else {
                    rt.status = McpServerStatus::Crashed {
                        reason: "handshake failed".to_string(),
                    };
                    rt.last_crash = Some(Instant::now());
                    warn!(target: "nebula.mcp.registry", server = %name,
                        "MCP server failed to start (handshake failed)");
                    self.write_log(
                        name,
                        &format!(
                            "[{}] server crashed: handshake failed",
                            chrono::Local::now()
                        ),
                    );
                }
            }
        }

        self.inner.state_change.notify_waiters();
        self.ensure_supervisor();
        Ok(())
    }

    /// 手动停止单个 server:disconnect client + 标记 Stopped。
    pub async fn stop(&self, name: &str) -> Result<()> {
        info!(target: "nebula.mcp.registry", server = %name, "stopping MCP server");

        // remove_server 会 drop client → StdioTransport Drop → child.start_kill()
        self.inner.mcp_manager.remove_server(name);

        // 重新 add_server(未连接),以便后续 start 重启
        if let Some(config) = self.inner.configs.read().get(name) {
            self.inner.mcp_manager.add_server(config.clone());
        }

        {
            let mut runtimes = self.inner.runtimes.lock();
            if let Some(rt) = runtimes.get_mut(name) {
                rt.status = McpServerStatus::Stopped;
                rt.pid = None;
                rt.started_at = None;
            }
        }
        self.write_log(name, &format!("[{}] server stopped", chrono::Local::now()));
        self.inner.state_change.notify_waiters();
        Ok(())
    }

    /// 查询单个 server 状态。
    pub fn status(&self, name: &str) -> Option<McpServerStatus> {
        self.inner
            .runtimes
            .lock()
            .get(name)
            .map(|rt| rt.status.clone())
    }

    /// 列出所有 server 信息。
    pub fn list(&self) -> Vec<McpServerInfo> {
        let runtimes = self.inner.runtimes.lock();
        let configs = self.inner.configs.read();

        // 合并 configs 和 runtimes(configs 中有但 runtimes 中没有的也列出)
        let mut names: Vec<String> = configs.keys().cloned().collect();
        for name in runtimes.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }

        names
            .into_iter()
            .map(|name| {
                let rt = runtimes.get(&name);
                McpServerInfo {
                    name: name.clone(),
                    status: rt
                        .map(|r| r.status.clone())
                        .unwrap_or(McpServerStatus::Stopped),
                    pid: rt.and_then(|r| r.pid),
                    uptime_secs: rt.and_then(|r| r.started_at.map(|t| t.elapsed().as_secs())),
                    restart_count: rt.map(|r| r.restart_count).unwrap_or(0),
                    log_path: self.log_path_for(&name).to_string_lossy().to_string(),
                }
            })
            .collect()
    }

    /// 读 mcp-<name>.log 最后 N 行。
    pub async fn logs(&self, name: &str, tail: usize) -> Result<Vec<String>> {
        let log_path = self.log_path_for(name);
        if !log_path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&log_path)
            .await
            .with_context(|| format!("failed to read log: {}", log_path.display()))?;
        let lines: Vec<&str> = content.lines().collect();
        let start = if lines.len() > tail {
            lines.len() - tail
        } else {
            0
        };
        Ok(lines[start..].iter().map(|s| s.to_string()).collect())
    }

    /// 停止所有 server + cancel supervisor。
    pub async fn stop_all(&self) -> Result<()> {
        info!(target: "nebula.mcp.registry", "stopping all MCP servers");
        self.inner.cancel.cancel();

        // 等待 supervisor 退出
        let handle = self.inner.supervisor.lock().take();
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }

        // 逐个 stop
        let names: Vec<String> = self.inner.configs.read().keys().cloned().collect();
        for name in names {
            let _ = self.stop(&name).await;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // 内部方法
    // ------------------------------------------------------------------

    fn log_path_for(&self, name: &str) -> PathBuf {
        self.inner.log_dir.join(format!("mcp-{}.log", name))
    }

    /// 同步写日志(追加模式)。日志写入频率低,用 std::fs 足够。
    fn write_log(&self, name: &str, line: &str) {
        let path = self.log_path_for(name);
        use std::io::Write;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{}", line);
        }
    }

    /// 检测 server 是否已连接:发一个 invoke_tool ping。
    ///
    /// - Ok(result) → 已连接(server 响应了,即使 isError=true)
    /// - Err("not connected") → 未连接
    /// - Err(其他) → 传输错误(可能已崩溃)
    /// - 超时 5s → 视为未连接
    async fn check_connected(&self, name: &str) -> bool {
        let ping = self.inner.mcp_manager.invoke_tool(
            name,
            "__mcp_health_check_ping__",
            serde_json::json!({}),
        );
        match tokio::time::timeout(Duration::from_secs(5), ping).await {
            Ok(Ok(_)) => true,
            Ok(Err(e)) => {
                let msg = format!("{}", e);
                !msg.contains("not connected")
            }
            Err(_) => false,
        }
    }

    /// 健康检查:发 tools/call ping,5s 超时。
    ///
    /// 成功(Ok)→ 更新 last_health_check;失败 → 置 Crashed。
    pub async fn health_check(&self, name: &str) -> Result<()> {
        let ping =
            self.inner
                .mcp_manager
                .invoke_tool(name, "__mcp_health_check__", serde_json::json!({}));
        match tokio::time::timeout(Duration::from_secs(5), ping).await {
            Ok(Ok(_)) => {
                let mut runtimes = self.inner.runtimes.lock();
                if let Some(rt) = runtimes.get_mut(name) {
                    rt.last_health_check = Some(Instant::now());
                }
                Ok(())
            }
            Ok(Err(e)) => {
                // 错误可能是 "not connected" 或传输错误
                Err(anyhow::anyhow!("health check failed: {}", e))
            }
            Err(_) => Err(anyhow::anyhow!("health check timeout (5s)")),
        }
    }

    /// 启动 supervisor(若尚未启动)。
    fn ensure_supervisor(&self) {
        let mut supervisor = self.inner.supervisor.lock();
        if supervisor.is_some() {
            return;
        }
        let manager = self.clone();
        let handle = tokio::spawn(async move {
            manager.supervisor_loop().await;
        });
        *supervisor = Some(handle);
    }

    /// supervisor 循环:每 5s tick,健康检查 + 崩溃重启。
    ///
    /// 借鉴 [`SidecarManager::supervisor_loop`](crate::sidecar::SidecarManager::supervisor_loop)。
    async fn supervisor_loop(&self) {
        debug!(target: "nebula.mcp.registry", "supervisor loop started");
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = self.inner.cancel.cancelled() => {
                    debug!(target: "nebula.mcp.registry", "supervisor loop cancelled");
                    break;
                }
            }

            let names: Vec<String> = self.inner.configs.read().keys().cloned().collect();
            for name in names {
                self.supervisor_tick(&name).await;
            }
        }
    }

    /// 单个 server 的 supervisor tick。
    async fn supervisor_tick(&self, name: &str) {
        // 读取当前状态 + 配置
        let (status, health_interval_secs, auto_restart) = {
            let runtimes = self.inner.runtimes.lock();
            let configs = self.inner.configs.read();
            let rt = match runtimes.get(name) {
                Some(rt) => rt,
                None => return,
            };
            let cfg = configs.get(name);
            // T-E-S-31 将添加 auto_restart / health_check_interval_secs 字段;
            // 当前使用默认值(true / 30)。
            let auto_restart = cfg.map(|c| c.enabled).unwrap_or(true);
            let health_interval = 30u64; // 默认 30s;T-E-S-31 后改为 cfg.health_check_interval_secs
            (rt.status.clone(), health_interval, auto_restart)
        };

        match status {
            McpServerStatus::Running => {
                // 检查是否到健康检查时间
                let should_check = {
                    let runtimes = self.inner.runtimes.lock();
                    runtimes
                        .get(name)
                        .and_then(|rt| rt.last_health_check)
                        .map(|t| t.elapsed().as_secs() >= health_interval_secs)
                        .unwrap_or(true) // 从未检查过
                };

                if should_check {
                    if let Err(e) = self.health_check(name).await {
                        warn!(target: "nebula.mcp.registry", server = %name,
                            error = %e, "health check failed, marking crashed");
                        let mut runtimes = self.inner.runtimes.lock();
                        if let Some(rt) = runtimes.get_mut(name) {
                            rt.status = McpServerStatus::Crashed {
                                reason: format!("health check: {}", e),
                            };
                            rt.last_crash = Some(Instant::now());
                        }
                        self.write_log(
                            name,
                            &format!("[{}] health check failed: {}", chrono::Local::now(), e),
                        );
                    }
                }
            }
            McpServerStatus::Crashed { .. } => {
                if !auto_restart {
                    return;
                }

                // 检查重启限流
                let should_restart = {
                    let mut runtimes = self.inner.runtimes.lock();
                    let rt = match runtimes.get_mut(name) {
                        Some(rt) => rt,
                        None => return,
                    };

                    // 滑动窗口:清除 1h 前的时间戳
                    let one_hour_ago = Instant::now() - Duration::from_secs(3600);
                    rt.restart_timestamps.retain(|&t| t > one_hour_ago);

                    if rt.restart_timestamps.len() >= self.inner.max_restarts_per_hour as usize {
                        warn!(target: "nebula.mcp.registry", server = %name,
                            restarts = rt.restart_timestamps.len(),
                            "exceeded max_restarts_per_hour, disabling");
                        rt.status = McpServerStatus::Disabled;
                        self.write_log(
                            name,
                            &format!(
                                "[{}] disabled (restart limit exceeded)",
                                chrono::Local::now()
                            ),
                        );
                        return;
                    }

                    // 指数退避:距上次崩溃需过 min(2^n, 30)s
                    let backoff = Self::restart_backoff_delay(rt.restart_count);
                    let elapsed = rt.last_crash.map(|t| t.elapsed()).unwrap_or(Duration::ZERO);
                    if elapsed >= backoff {
                        rt.status = McpServerStatus::Restarting;
                        rt.restart_count += 1;
                        rt.restart_timestamps.push(Instant::now());
                        true
                    } else {
                        false
                    }
                };

                if should_restart {
                    info!(target: "nebula.mcp.registry", server = %name,
                        restart_count = {
                            let runtimes = self.inner.runtimes.lock();
                            runtimes.get(name).map(|rt| rt.restart_count).unwrap_or(0)
                        },
                        "supervisor: restarting server (after exponential backoff)");
                    self.write_log(
                        name,
                        &format!("[{}] supervisor restarting server", chrono::Local::now()),
                    );

                    // 重新 start
                    if let Err(e) = self.start(name).await {
                        error!(target: "nebula.mcp.registry", server = %name,
                            error = %e, "supervisor: restart failed");
                    }
                }
            }
            McpServerStatus::Disabled | McpServerStatus::Stopped => {
                // 不处理
            }
            McpServerStatus::Starting | McpServerStatus::Restarting => {
                // 启动中,等待
            }
        }
    }

    /// 计算崩溃重启的指数退避延迟。
    ///
    /// 公式: `min(2^restart_count, 30)` 秒。
    /// 借鉴 [`SidecarManager::restart_backoff_delay`](crate::sidecar::SidecarManager::restart_backoff_delay)。
    fn restart_backoff_delay(restart_count: u32) -> Duration {
        let secs = if restart_count == 0 {
            1u64
        } else {
            let raw = 1u64.checked_shl(restart_count).unwrap_or(u64::MAX);
            raw.min(30)
        };
        Duration::from_secs(secs)
    }
}

impl Drop for McpServerRegistry {
    fn drop(&mut self) {
        self.inner.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个 stdio 测试配置。
    fn test_config(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport_type: McpTransportType::Stdio,
            command: Some(command.to_string()),
            args: vec![],
            env: std::collections::HashMap::new(),
            url: None,
            api_key: None,
            enabled: true,
            tool_filter: vec![],
            auto_restart: true,
            health_check_interval_secs: 30,
        }
    }

    /// 跨平台的"短暂运行"命令(2-3s 后退出,不响应 MCP)。
    fn short_lived_command() -> &'static str {
        if cfg!(windows) {
            "cmd /c ping 127.0.0.1 -n 2"
        } else {
            "sleep 1"
        }
    }

    /// 跨平台的"立即退出"命令(exit code 1)。
    fn immediate_exit_command() -> &'static str {
        if cfg!(windows) {
            "cmd /c exit 1"
        } else {
            "sh -c \"exit 1\""
        }
    }

    #[test]
    fn restart_backoff_delay_exponential() {
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(0),
            Duration::from_secs(1)
        );
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(1),
            Duration::from_secs(2)
        );
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(2),
            Duration::from_secs(4)
        );
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(3),
            Duration::from_secs(8)
        );
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(4),
            Duration::from_secs(16)
        );
    }

    #[test]
    fn restart_backoff_delay_capped_at_30s() {
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(5),
            Duration::from_secs(30)
        );
        assert_eq!(
            McpServerRegistry::restart_backoff_delay(10),
            Duration::from_secs(30)
        );
    }

    /// 单测 1:registry start/stop 生命周期。
    ///
    /// 使用短命命令(ping/sleep),handshake 会失败,server 标记为 Crashed。
    /// stop 后标记为 Stopped。
    #[tokio::test]
    async fn registry_start_stop_lifecycle() {
        let manager = Arc::new(McpManager::new());
        let temp_dir = tempfile::tempdir().expect("test op should succeed");
        let registry = McpServerRegistry::new(manager, temp_dir.path().to_path_buf());

        // 添加配置
        {
            let mut configs = registry.inner.configs.write();
            configs.insert(
                "test-mock".to_string(),
                test_config("test-mock", short_lived_command()),
            );
        }
        {
            let mut runtimes = registry.inner.runtimes.lock();
            runtimes.insert(
                "test-mock".to_string(),
                McpServerRuntime::new(registry.log_path_for("test-mock")),
            );
        }
        registry
            .inner
            .mcp_manager
            .add_server(test_config("test-mock", short_lived_command()));

        // start(handshake 会失败 → Crashed)
        let _ = registry.start("test-mock").await;

        // 等待 connect_all 超时或完成
        tokio::time::sleep(Duration::from_millis(500)).await;

        let status = registry.status("test-mock").expect("test op should succeed");
        assert!(
            matches!(status, McpServerStatus::Crashed { .. })
                || matches!(status, McpServerStatus::Running)
                || matches!(status, McpServerStatus::Starting),
            "expected Crashed/Running/Starting after start, got {:?}",
            status
        );

        // stop
        registry.stop("test-mock").await.expect("task should complete");
        let status = registry.status("test-mock").expect("test op should succeed");
        assert_eq!(status, McpServerStatus::Stopped);
    }

    /// 单测 2:supervisor 崩溃重启。
    ///
    /// 使用立即退出命令(cmd /c exit 1 / sh -c "exit 1"),server 启动即崩溃。
    /// 手动模拟 supervisor tick:Crashed → Restarting → start → Crashed。
    #[tokio::test]
    async fn supervisor_crash_restart() {
        let manager = Arc::new(McpManager::new());
        let temp_dir = tempfile::tempdir().expect("test op should succeed");
        let registry = McpServerRegistry::new(manager, temp_dir.path().to_path_buf());

        // 添加配置
        {
            let mut configs = registry.inner.configs.write();
            configs.insert(
                "crash-test".to_string(),
                test_config("crash-test", immediate_exit_command()),
            );
        }
        registry
            .inner
            .mcp_manager
            .add_server(test_config("crash-test", immediate_exit_command()));

        // 初始化 runtime 为 Crashed(模拟已崩溃)
        {
            let mut runtimes = registry.inner.runtimes.lock();
            runtimes.insert(
                "crash-test".to_string(),
                McpServerRuntime {
                    status: McpServerStatus::Crashed {
                        reason: "test crash".to_string(),
                    },
                    pid: None,
                    started_at: None,
                    restart_count: 0,
                    restart_timestamps: Vec::new(),
                    last_crash: Some(Instant::now() - Duration::from_secs(10)),
                    last_health_check: None,
                    log_path: registry.log_path_for("crash-test"),
                },
            );
        }

        // 调用 supervisor_tick(应触发 restart)
        registry.supervisor_tick("crash-test").await;

        // 验证 restart_count 增加
        let runtimes = registry.inner.runtimes.lock();
        let rt = runtimes.get("crash-test").expect("get should succeed");
        assert!(
            rt.restart_count >= 1,
            "expected restart_count >= 1, got {}",
            rt.restart_count
        );
    }

    /// 单测 3:重启限流 3 次/小时。
    ///
    /// 模拟 restart_timestamps 已有 3 条记录,supervisor 应置 Disabled。
    #[tokio::test]
    async fn restart_rate_limit_disables_after_3() {
        let manager = Arc::new(McpManager::new());
        let temp_dir = tempfile::tempdir().expect("test op should succeed");
        let registry = McpServerRegistry::new(manager, temp_dir.path().to_path_buf());

        {
            let mut configs = registry.inner.configs.write();
            configs.insert(
                "rate-limit".to_string(),
                test_config("rate-limit", immediate_exit_command()),
            );
        }
        registry
            .inner
            .mcp_manager
            .add_server(test_config("rate-limit", immediate_exit_command()));

        // 模拟已重启 3 次(在滑动窗口内)
        {
            let mut runtimes = registry.inner.runtimes.lock();
            let now = Instant::now();
            runtimes.insert(
                "rate-limit".to_string(),
                McpServerRuntime {
                    status: McpServerStatus::Crashed {
                        reason: "test crash".to_string(),
                    },
                    pid: None,
                    started_at: None,
                    restart_count: 3,
                    restart_timestamps: vec![now, now, now],
                    last_crash: Some(now - Duration::from_secs(10)),
                    last_health_check: None,
                    log_path: registry.log_path_for("rate-limit"),
                },
            );
        }

        // supervisor tick 应置 Disabled(而非 restart)
        registry.supervisor_tick("rate-limit").await;

        let status = registry.status("rate-limit").expect("test op should succeed");
        assert_eq!(status, McpServerStatus::Disabled);
    }

    /// 单测 4:健康检查超时置 Crashed。
    ///
    /// 手动将 runtime 设为 Running,然后调用 health_check。
    /// 由于 mcp_manager 中没有连接的 client,invoke_tool 返回 "not connected" 错误,
    /// health_check 返回 Err。supervisor_tick 会置 Crashed。
    #[tokio::test]
    async fn health_check_failure_marks_crashed() {
        let manager = Arc::new(McpManager::new());
        let temp_dir = tempfile::tempdir().expect("test op should succeed");
        let registry = McpServerRegistry::new(manager, temp_dir.path().to_path_buf());

        // 添加配置 + 未连接的 client
        {
            let mut configs = registry.inner.configs.write();
            configs.insert(
                "health-test".to_string(),
                test_config("health-test", short_lived_command()),
            );
        }
        registry
            .inner
            .mcp_manager
            .add_server(test_config("health-test", short_lived_command()));

        // 模拟 Running 状态(但 client 未连接)
        {
            let mut runtimes = registry.inner.runtimes.lock();
            runtimes.insert(
                "health-test".to_string(),
                McpServerRuntime {
                    status: McpServerStatus::Running,
                    pid: None,
                    started_at: Some(Instant::now()),
                    restart_count: 0,
                    restart_timestamps: Vec::new(),
                    last_crash: None,
                    last_health_check: None,
                    log_path: registry.log_path_for("health-test"),
                },
            );
        }

        // 调用 health_check(应失败,因为 client 未连接)
        let result = registry.health_check("health-test").await;
        assert!(
            result.is_err(),
            "health_check should fail for unconnected server"
        );

        // supervisor_tick 应将 Running + health_check 失败 → Crashed
        registry.supervisor_tick("health-test").await;

        let status = registry.status("health-test").expect("test op should succeed");
        assert!(
            matches!(status, McpServerStatus::Crashed { .. }),
            "expected Crashed after health check failure, got {:?}",
            status
        );
    }

    /// 单测 5:mcp_server_logs 返回 tail 行。
    ///
    /// 写入若干行到日志文件,验证 logs() 返回最后 N 行。
    #[tokio::test]
    async fn logs_returns_tail_lines() {
        let manager = Arc::new(McpManager::new());
        let temp_dir = tempfile::tempdir().expect("test op should succeed");
        let registry = McpServerRegistry::new(manager, temp_dir.path().to_path_buf());

        let log_path = registry.log_path_for("log-test");
        std::fs::create_dir_all(log_path.parent().expect("create should succeed")).expect("create should succeed");

        // 写入 5 行
        for i in 1..=5 {
            registry.write_log("log-test", &format!("line {}", i));
        }

        // 读最后 3 行
        let tail = registry.logs("log-test", 3).await.expect("task should complete");
        assert_eq!(tail.len(), 3);
        assert!(tail[0].contains("line 3"));
        assert!(tail[1].contains("line 4"));
        assert!(tail[2].contains("line 5"));

        // tail 大于文件行数 → 返回全部
        let all = registry.logs("log-test", 100).await.expect("task should complete");
        assert_eq!(all.len(), 5);
    }

    /// 单测 6:list() 返回所有已注册 server 信息。
    #[tokio::test]
    async fn list_returns_all_servers() {
        let manager = Arc::new(McpManager::new());
        let temp_dir = tempfile::tempdir().expect("test op should succeed");
        let registry = McpServerRegistry::new(manager, temp_dir.path().to_path_buf());

        // 添加 2 个配置
        {
            let mut configs = registry.inner.configs.write();
            configs.insert(
                "server-a".to_string(),
                test_config("server-a", short_lived_command()),
            );
            configs.insert(
                "server-b".to_string(),
                test_config("server-b", short_lived_command()),
            );
        }

        let info = registry.list();
        assert_eq!(info.len(), 2);

        let names: Vec<&str> = info.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"server-a"));
        assert!(names.contains(&"server-b"));

        // 所有 server 默认 Stopped
        for i in &info {
            assert_eq!(i.status, McpServerStatus::Stopped);
            assert_eq!(i.restart_count, 0);
        }
    }
}
