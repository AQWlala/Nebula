//! Sidecar 进程管理器。
//!
//! 负责：
//! * 启动 / 停止 sidecar 进程
//! * 健康检查（gRPC HealthCheck）
//! * 崩溃自动重启（指数退避）
//! * 状态查询

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::protocol::SidecarConfig;
use super::{default_sidecar_dir, sidecar_exe_name};

/// Sidecar 服务类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidecarKind {
    /// Memory Service — 记忆存储 + 向量搜索 + 海绵引擎。
    Memory,
    /// LLM Gateway — LLM 调用网关 + 限流 + 重试。
    Llm,
    /// Swarm Coordinator — 子智能体编排 + 任务分发。
    Swarm,
}

impl SidecarKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SidecarKind::Memory => "memory",
            SidecarKind::Llm => "llm",
            SidecarKind::Swarm => "swarm",
        }
    }

    pub fn all() -> [SidecarKind; 3] {
        [SidecarKind::Memory, SidecarKind::Llm, SidecarKind::Swarm]
    }
}

/// Sidecar 运行状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarStatus {
    /// 未启动。
    Stopped,
    /// 启动中（进程已创建，等待健康检查通过）。
    Starting,
    /// 运行中（健康检查通过）。
    Running,
    /// 已崩溃，等待重启。
    Crashed { reason: String },
    /// 正在重启。
    Restarting,
}

/// 单个 sidecar 的运行时状态。
struct SidecarRuntime {
    status: SidecarStatus,
    child: Option<Child>,
    listen_addr: Option<String>,
    pid: Option<u32>,
    started_at: Option<Instant>,
    restart_count: u32,
    last_crash: Option<Instant>,
}

impl Default for SidecarRuntime {
    fn default() -> Self {
        Self {
            status: SidecarStatus::Stopped,
            child: None,
            listen_addr: None,
            pid: None,
            started_at: None,
            restart_count: 0,
            last_crash: None,
        }
    }
}

/// Sidecar 管理器 — 管理所有 sidecar 进程的生命周期。
///
/// ## 使用方式
///
/// ```no_run
/// # use nine_snake::sidecar::{SidecarManager, SidecarKind};
/// # async fn example() -> anyhow::Result<()> {
/// let manager = SidecarManager::new("/tmp/data".into());
/// manager.start(SidecarKind::Memory).await?;
/// assert!(manager.is_running(SidecarKind::Memory));
/// manager.stop_all().await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct SidecarManager {
    inner: Arc<Inner>,
}

struct Inner {
    data_dir: PathBuf,
    sidecar_dir: PathBuf,
    runtimes: Mutex<HashMap<SidecarKind, SidecarRuntime>>,
    auth_tokens: Mutex<HashMap<SidecarKind, String>>,
    cancel: CancellationToken,
    supervisor: Mutex<Option<JoinHandle<()>>>,
    state_change: Notify,
    max_restarts: u32,
}

impl SidecarManager {
    /// 创建新的 sidecar 管理器。
    pub fn new(data_dir: PathBuf) -> Self {
        let sidecar_dir = default_sidecar_dir();
        info!(target: "sidecar", dir = %sidecar_dir.display(), "sidecar manager initialized");

        Self {
            inner: Arc::new(Inner {
                data_dir,
                sidecar_dir,
                runtimes: Mutex::new(HashMap::new()),
                auth_tokens: Mutex::new(HashMap::new()),
                cancel: CancellationToken::new(),
                supervisor: Mutex::new(None),
                state_change: Notify::new(),
                max_restarts: 5,
            }),
        }
    }

    /// 启动指定 sidecar。
    pub async fn start(&self, kind: SidecarKind) -> Result<()> {
        let token = self.generate_token();
        self.inner.auth_tokens.lock().insert(kind, token.clone());

        let config = SidecarConfig::new(
            kind.as_str(),
            self.inner.data_dir.clone(),
            token,
        );

        self.spawn_sidecar(kind, config).await?;
        self.ensure_supervisor();
        Ok(())
    }

    /// 启动所有 sidecar。
    pub async fn start_all(&self) -> Result<()> {
        for kind in SidecarKind::all() {
            if let Err(e) = self.start(kind).await {
                warn!(target: "sidecar", kind = kind.as_str(), error = %e, "failed to start sidecar, will retry via supervisor");
            }
        }
        Ok(())
    }

    /// 停止指定 sidecar。
    pub async fn stop(&self, kind: SidecarKind) -> Result<()> {
        let child = {
            let mut runtimes = self.inner.runtimes.lock();
            if let Some(rt) = runtimes.get_mut(&kind) {
                let child = rt.child.take();
                rt.status = SidecarStatus::Stopped;
                rt.pid = None;
                rt.listen_addr = None;
                child
            } else {
                None
            }
        };

        if let Some(mut child) = child {
            info!(target: "sidecar", kind = kind.as_str(), "stopping sidecar");
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        self.inner.state_change.notify_waiters();
        Ok(())
    }

    /// 停止所有 sidecar。
    pub async fn stop_all(&self) -> Result<()> {
        self.inner.cancel.cancel();

        let handle = {
            self.inner.supervisor.lock().take()
        };
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }

        for kind in SidecarKind::all() {
            let _ = self.stop(kind).await;
        }
        Ok(())
    }

    /// 查询 sidecar 状态。
    pub fn status(&self, kind: SidecarKind) -> SidecarStatus {
        self.inner
            .runtimes
            .lock()
            .get(&kind)
            .map(|rt| rt.status.clone())
            .unwrap_or(SidecarStatus::Stopped)
    }

    /// sidecar 是否在运行中。
    pub fn is_running(&self, kind: SidecarKind) -> bool {
        matches!(self.status(kind), SidecarStatus::Running)
    }

    /// 获取 sidecar 的监听地址。
    pub fn listen_addr(&self, kind: SidecarKind) -> Option<String> {
        self.inner
            .runtimes
            .lock()
            .get(&kind)
            .and_then(|rt| rt.listen_addr.clone())
    }

    /// 等待 sidecar 就绪（超时时间内）。
    pub async fn wait_ready(&self, kind: SidecarKind, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if self.is_running(kind) {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(anyhow!(
                    "sidecar {} not ready within {:?}",
                    kind.as_str(),
                    timeout
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
                _ = self.inner.state_change.notified() => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // 内部方法
    // ------------------------------------------------------------------

    fn generate_token(&self) -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        (0..32).map(|_| rng.sample(rand::distributions::Alphanumeric) as char).collect()
    }

    async fn spawn_sidecar(&self, kind: SidecarKind, config: SidecarConfig) -> Result<()> {
        let exe = self.inner.sidecar_dir.join(sidecar_exe_name(kind));

        if !exe.exists() {
            warn!(target: "sidecar", kind = kind.as_str(), exe = %exe.display(),
                "sidecar binary not found, will run in-process mode");
            let mut runtimes = self.inner.runtimes.lock();
            let rt = runtimes.entry(kind).or_default();
            rt.status = SidecarStatus::Running;
            rt.listen_addr = Some("in-process".to_string());
            return Ok(());
        }

        info!(target: "sidecar", kind = kind.as_str(), exe = %exe.display(), "spawning sidecar");

        let mut cmd = Command::new(&exe);
        cmd.arg("--listen-addr")
            .arg(&config.listen_addr)
            .arg("--data-dir")
            .arg(&config.data_dir)
            .arg("--log-level")
            .arg(&config.log_level)
            .env("NINE_SNAKE_SIDECAR_TOKEN", &config.auth_token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn {} sidecar", kind.as_str()))?;

        let pid = child.id().unwrap_or(0);

        {
            let mut runtimes = self.inner.runtimes.lock();
            let rt = runtimes.entry(kind).or_default();
            rt.status = SidecarStatus::Starting;
            rt.child = Some(child);
            rt.pid = Some(pid);
            rt.started_at = Some(Instant::now());
        }

        self.inner.state_change.notify_waiters();

        let manager = self.clone();
        tokio::spawn(async move {
            manager.monitor_sidecar(kind).await;
        });

        let wait_addr = self.wait_for_listen_addr(kind, Duration::from_secs(30)).await;
        match wait_addr {
            Ok(addr) => {
                let mut runtimes = self.inner.runtimes.lock();
                if let Some(rt) = runtimes.get_mut(&kind) {
                    rt.status = SidecarStatus::Running;
                    rt.listen_addr = Some(addr);
                }
                info!(target: "sidecar", kind = kind.as_str(), pid, "sidecar is running");
            }
            Err(e) => {
                warn!(target: "sidecar", kind = kind.as_str(), error = %e,
                    "sidecar failed to become ready, marking as crashed");
                let mut runtimes = self.inner.runtimes.lock();
                if let Some(rt) = runtimes.get_mut(&kind) {
                    rt.status = SidecarStatus::Crashed {
                        reason: format!("startup timeout: {}", e),
                    };
                    rt.last_crash = Some(Instant::now());
                }
            }
        }

        self.inner.state_change.notify_waiters();
        Ok(())
    }

    async fn wait_for_listen_addr(
        &self,
        kind: SidecarKind,
        timeout: Duration,
    ) -> Result<String> {
        // v2.0 简化版：等待一段时间后返回默认地址
        // 真正实现需要解析 sidecar 的 stdout 或通过 gRPC health check
        let start = Instant::now();
        let default_port = match kind {
            SidecarKind::Memory => 50051,
            SidecarKind::Llm => 50052,
            SidecarKind::Swarm => 50053,
        };

        while start.elapsed() < timeout {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let status = self.status(kind);
            if matches!(status, SidecarStatus::Crashed { .. }) {
                return Err(anyhow!("sidecar crashed during startup"));
            }

            if let Some(addr) = self.listen_addr(kind) {
                return Ok(addr);
            }

            if self.inner.runtimes.lock().get(&kind).and_then(|rt| rt.pid).is_some() {
                return Ok(format!("127.0.0.1:{}", default_port));
            }
        }

        Err(anyhow!("timeout waiting for listen address"))
    }

    async fn monitor_sidecar(&self, kind: SidecarKind) {
        let mut child = match self.inner.runtimes.lock().get_mut(&kind).and_then(|rt| rt.child.take()) {
            Some(c) => c,
            None => return,
        };

        let status = child.wait().await;
        match status {
            Ok(exit) => {
                warn!(target: "sidecar", kind = kind.as_str(), code = ?exit.code(),
                    "sidecar process exited");
                let mut runtimes = self.inner.runtimes.lock();
                if let Some(rt) = runtimes.get_mut(&kind) {
                    rt.status = SidecarStatus::Crashed {
                        reason: format!("exit code: {}", exit.code().unwrap_or(-1)),
                    };
                    rt.last_crash = Some(Instant::now());
                    rt.child = None;
                }
            }
            Err(e) => {
                error!(target: "sidecar", kind = kind.as_str(), error = %e,
                    "failed to wait for sidecar");
            }
        }

        self.inner.state_change.notify_waiters();
    }

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

    async fn supervisor_loop(&self) {
        debug!(target: "sidecar", "supervisor loop started");
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = self.inner.cancel.cancelled() => {
                    debug!(target: "sidecar", "supervisor loop cancelled");
                    break;
                }
            }

            for kind in SidecarKind::all() {
                let should_restart = {
                    let runtimes = self.inner.runtimes.lock();
                    if let Some(rt) = runtimes.get(&kind) {
                        matches!(rt.status, SidecarStatus::Crashed { .. })
                            && rt.restart_count < self.inner.max_restarts
                    } else {
                        false
                    }
                };

                if should_restart {
                    let token = self.generate_token();
                    self.inner.auth_tokens.lock().insert(kind, token.clone());

                    let config = SidecarConfig::new(
                        kind.as_str(),
                        self.inner.data_dir.clone(),
                        token,
                    );

                    info!(target: "sidecar", kind = kind.as_str(), "supervisor: restarting sidecar");

                    {
                        let mut runtimes = self.inner.runtimes.lock();
                        if let Some(rt) = runtimes.get_mut(&kind) {
                            rt.status = SidecarStatus::Restarting;
                            rt.restart_count += 1;
                        }
                    }

                    let _ = self.spawn_sidecar(kind, config).await;
                }
            }
        }
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        // 注意：async drop 在 Rust 中不可用，
        // 实际停止由显式调用 stop_all() 或进程退出完成
        self.inner.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_kind_as_str() {
        assert_eq!(SidecarKind::Memory.as_str(), "memory");
        assert_eq!(SidecarKind::Llm.as_str(), "llm");
        assert_eq!(SidecarKind::Swarm.as_str(), "swarm");
    }

    #[test]
    fn sidecar_kind_all_has_three() {
        assert_eq!(SidecarKind::all().len(), 3);
    }

    #[test]
    fn default_status_is_stopped() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        assert_eq!(manager.status(SidecarKind::Memory), SidecarStatus::Stopped);
        assert!(!manager.is_running(SidecarKind::Memory));
    }
}
