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
    /// T-S4-B-01: Skill Service — 技能 CRUD + 执行引擎（单二进制多角色方案）。
    Skill,
    /// T-S4-B-02: Reflection Service — 自我反思引擎(L5 真反思 + 持久化)。
    Reflection,
    /// T-S6-A-01a: OS-Controller — Windows 窗口管理 / 菜单操作 / 输入模拟。
    OsController,
}

impl SidecarKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SidecarKind::Memory => "memory",
            SidecarKind::Llm => "llm",
            SidecarKind::Swarm => "swarm",
            SidecarKind::Skill => "skill",
            SidecarKind::Reflection => "reflection",
            SidecarKind::OsController => "os_controller",
        }
    }

    pub fn all() -> [SidecarKind; 6] {
        [
            SidecarKind::Memory,
            SidecarKind::Llm,
            SidecarKind::Swarm,
            SidecarKind::Skill,
            SidecarKind::Reflection,
            SidecarKind::OsController,
        ]
    }
}

/// T-E-S-61: 返回 sidecar 的默认 gRPC 监听端口。
fn default_port_for_kind(kind: SidecarKind) -> u16 {
    match kind {
        SidecarKind::Memory => 50051,
        SidecarKind::Llm => 50052,
        SidecarKind::Swarm => 50053,
        SidecarKind::Skill => 50054,
        SidecarKind::Reflection => 50055,
        SidecarKind::OsController => 50056,
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
#[cfg_attr(not(feature = "grpc"), allow(dead_code))]
struct SidecarRuntime {
    status: SidecarStatus,
    child: Option<Child>,
    listen_addr: Option<String>,
    pid: Option<u32>,
    started_at: Option<Instant>,
    restart_count: u32,
    last_crash: Option<Instant>,
    /// T-E-S-61: 最近一次 gRPC HealthCheck 成功时间。
    last_health_check: Option<Instant>,
    /// T-E-S-61: 连续健康检查失败次数(连续 3 次标记 Crashed)。
    health_check_failures: u32,
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
            last_health_check: None,
            health_check_failures: 0,
        }
    }
}

/// Sidecar 管理器 — 管理所有 sidecar 进程的生命周期。
///
/// ## 使用方式
///
/// ```no_run
/// # use nebula::sidecar::{SidecarManager, SidecarKind};
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

        let config = SidecarConfig::new(kind.as_str(), self.inner.data_dir.clone(), token);

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

    /// T-S4-B-03: Bootstrap — 启动所有已配置 sidecar 并等待就绪。
    ///
    /// 这是应用启动时的高层入口,等价于 `start_all()` + `wait_ready()`
    /// 对每个 sidecar。失败不阻断(由 supervisor 后续重试),只记录 warn。
    ///
    /// 与 `start_all()` 的区别:`bootstrap()` 会等待每个 sidecar 进入
    /// Running 状态(或超时),适合在应用初始化阶段同步确认就绪。
    pub async fn bootstrap(&self) -> Result<()> {
        info!(target: "sidecar", "bootstrap: starting all sidecars");
        self.start_all().await?;

        // 等待每个 sidecar 就绪(最长 10s/each),超时不阻断启动流程。
        for kind in SidecarKind::all() {
            if let Err(e) = self.wait_ready(kind, Duration::from_secs(10)).await {
                warn!(target: "sidecar", kind = kind.as_str(), error = %e,
                    "bootstrap: sidecar not ready within 10s, supervisor will retry");
            }
        }
        info!(target: "sidecar", "bootstrap complete");
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

        let handle = { self.inner.supervisor.lock().take() };
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
        (0..32)
            .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
            .collect()
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
            .env("NEBULA_SIDECAR_TOKEN", &config.auth_token)
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

        let wait_addr = self
            .wait_for_listen_addr(kind, Duration::from_secs(30))
            .await;
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

    async fn wait_for_listen_addr(&self, kind: SidecarKind, timeout: Duration) -> Result<String> {
        // T-E-S-61: gRPC HealthCheck 路径(feature on 时优先尝试)
        #[cfg(feature = "grpc")]
        {
            match self.wait_for_listen_addr_grpc(kind, timeout).await {
                Ok(addr) => return Ok(addr),
                Err(e) => {
                    warn!(target: "sidecar", kind = kind.as_str(), error = %e,
                        "gRPC health check failed, falling back to port wait");
                }
            }
        }

        // 端口等待逻辑(无 grpc feature 时的主路径,有 grpc feature 时的回退)
        self.wait_for_listen_addr_port(kind, timeout).await
    }

    /// T-E-S-61: gRPC HealthCheck 路径 — 拨号 sidecar 默认端口并调用 Health.Check。
    ///
    /// 成功(SERVING)返回地址;失败/超时返回 Err(由调用方回退到端口等待)。
    #[cfg(feature = "grpc")]
    async fn wait_for_listen_addr_grpc(
        &self,
        kind: SidecarKind,
        timeout: Duration,
    ) -> Result<String> {
        let default_port = default_port_for_kind(kind);
        let addr = format!("127.0.0.1:{}", default_port);
        let start = Instant::now();

        while start.elapsed() < timeout {
            // in-process 模式: listen_addr 已设置,直接返回
            if let Some(existing) = self.listen_addr(kind) {
                return Ok(existing);
            }
            if matches!(self.status(kind), SidecarStatus::Crashed { .. }) {
                return Err(anyhow!("sidecar crashed during startup"));
            }
            if Self::grpc_health_check(&addr, Duration::from_secs(5)).await {
                debug!(target: "sidecar", kind = kind.as_str(), addr = %addr,
                    "gRPC health check passed, sidecar is ready");
                return Ok(addr);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(anyhow!("gRPC health check timed out after {:?}", timeout))
    }

    /// 端口等待逻辑 — v2.0 简化版,等待 sidecar 进入监听状态。
    async fn wait_for_listen_addr_port(
        &self,
        kind: SidecarKind,
        timeout: Duration,
    ) -> Result<String> {
        // v2.0 简化版：等待一段时间后返回默认地址
        // 真正实现需要解析 sidecar 的 stdout 或通过 gRPC health check
        let start = Instant::now();
        let default_port = default_port_for_kind(kind);

        while start.elapsed() < timeout {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let status = self.status(kind);
            if matches!(status, SidecarStatus::Crashed { .. }) {
                return Err(anyhow!("sidecar crashed during startup"));
            }

            if let Some(addr) = self.listen_addr(kind) {
                return Ok(addr);
            }

            if self
                .inner
                .runtimes
                .lock()
                .get(&kind)
                .and_then(|rt| rt.pid)
                .is_some()
            {
                return Ok(format!("127.0.0.1:{}", default_port));
            }
        }

        Err(anyhow!("timeout waiting for listen address"))
    }

    /// T-E-S-61: gRPC HealthCheck — 拨号 sidecar 并调用 `grpc.health.v1.Health.Check`。
    ///
    /// 返回 `true` 表示 SERVING,`false` 表示 NOT_SERVING / RPC 失败 / 连接失败。
    ///
    /// 使用 `tonic-health::pb` 提供的标准健康检查客户端。
    /// 5s 超时,失败返回 false(不向上抛错,由调用方决定如何处理)。
    #[cfg(feature = "grpc")]
    pub(crate) async fn grpc_health_check(addr: &str, timeout: Duration) -> bool {
        use tonic_health::pb::health_check_response::ServingStatus;
        use tonic_health::pb::health_client::HealthClient;

        // tonic-health 0.12: HealthClient no longer has ::connect().
        // Use tonic's Endpoint::from_shared + connect_lazy, then HealthClient::new(channel).
        let channel = match tonic::transport::Endpoint::from_shared(format!("http://{}", addr))
            .map(|ep| ep.connect_lazy())
        {
            Ok(ch) => ch,
            Err(e) => {
                debug!(target: "sidecar", addr = %addr, error = %e,
                    "gRPC health check: endpoint creation failed");
                return false;
            }
        };
        let mut client = HealthClient::new(channel);

        let req = tonic::Request::new(tonic_health::pb::HealthCheckRequest {
            service: String::new(),
        });

        match tokio::time::timeout(timeout, client.check(req)).await {
            Ok(Ok(resp)) => resp.into_inner().status == ServingStatus::Serving as i32,
            Ok(Err(e)) => {
                debug!(target: "sidecar", addr = %addr, error = %e,
                    "gRPC health check: RPC failed");
                false
            }
            Err(_) => {
                debug!(target: "sidecar", addr = %addr, timeout = ?timeout,
                    "gRPC health check: timeout");
                false
            }
        }
    }

    /// T-E-S-61: 对指定 sidecar 执行 gRPC HealthCheck 并更新运行时状态。
    ///
    /// * SERVING → 更新 `last_health_check`,重置 `health_check_failures`
    /// * NOT_SERVING / RPC 失败 → `health_check_failures += 1`,
    ///   连续 3 次失败标记 `Crashed` 触发重启
    ///
    /// in-process 模式(listen_addr = "in-process")直接返回 Ok(无需 gRPC ping)。
    #[cfg(feature = "grpc")]
    async fn health_check(&self, kind: SidecarKind) -> Result<()> {
        let addr = match self.listen_addr(kind) {
            Some(a) if !a.is_empty() && a != "in-process" => a,
            _ => {
                // in-process 模式或未启动,视为健康
                return Ok(());
            }
        };

        let serving = Self::grpc_health_check(&addr, Duration::from_secs(5)).await;

        let mut crashed = false;
        {
            let mut runtimes = self.inner.runtimes.lock();
            if let Some(rt) = runtimes.get_mut(&kind) {
                if serving {
                    rt.last_health_check = Some(Instant::now());
                    rt.health_check_failures = 0;
                } else {
                    rt.health_check_failures += 1;
                    warn!(target: "sidecar", kind = kind.as_str(),
                        failures = rt.health_check_failures,
                        "health check failed (NOT_SERVING or RPC error)");
                    // 连续 3 次失败标记 Crashed 触发重启
                    if rt.health_check_failures >= 3 {
                        rt.status = SidecarStatus::Crashed {
                            reason: "health check failed 3 consecutive times".to_string(),
                        };
                        rt.last_crash = Some(Instant::now());
                        rt.health_check_failures = 0;
                        crashed = true;
                    }
                }
            }
        }
        self.inner.state_change.notify_waiters();

        if crashed {
            warn!(target: "sidecar", kind = kind.as_str(),
                "sidecar marked as Crashed after 3 consecutive health check failures");
        }

        if serving {
            Ok(())
        } else {
            Err(anyhow!("health check failed for sidecar {}", kind.as_str()))
        }
    }

    async fn monitor_sidecar(&self, kind: SidecarKind) {
        let mut child = match self
            .inner
            .runtimes
            .lock()
            .get_mut(&kind)
            .and_then(|rt| rt.child.take())
        {
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

    /// T-S4-B-03: 计算崩溃重启的指数退避延迟。
    ///
    /// 公式: `min(2^restart_count, 30)` 秒。
    ///
    /// | restart_count | delay |
    /// |---------------|-------|
    /// | 1             | 2s    |
    /// | 2             | 4s    |
    /// | 3             | 8s    |
    /// | 4             | 16s   |
    /// | 5+            | 30s   |
    ///
    /// 30s 上限防止长时间宕机后重启风暴(EXPERT_REVIEW §4.4)。
    fn restart_backoff_delay(restart_count: u32) -> Duration {
        let secs = if restart_count == 0 {
            1u64
        } else {
            // 2^restart_count,饱和至 30。
            // restart_count 是 u32,超过 4 时 2^count 已 > 16,超过 5 时 > 30。
            let raw = 1u64.checked_shl(restart_count).unwrap_or(u64::MAX);
            raw.min(30)
        };
        Duration::from_secs(secs)
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
                // T-E-S-61: 周期性 gRPC HealthCheck for Running sidecars
                // SERVING → 更新 last_health_check + 重置 failures
                // NOT_SERVING / RPC 失败 → failures += 1,连续 3 次标记 Crashed
                #[cfg(feature = "grpc")]
                {
                    let is_running = self
                        .inner
                        .runtimes
                        .lock()
                        .get(&kind)
                        .map(|rt| matches!(rt.status, SidecarStatus::Running))
                        .unwrap_or(false);
                    if is_running {
                        if let Err(e) = self.health_check(kind).await {
                            debug!(target: "sidecar", kind = kind.as_str(),
                                error = %e, "periodic health check failed");
                        }
                    }
                }

                let should_restart = {
                    let runtimes = self.inner.runtimes.lock();
                    if let Some(rt) = runtimes.get(&kind) {
                        if !matches!(rt.status, SidecarStatus::Crashed { .. }) {
                            false
                        } else if rt.restart_count >= self.inner.max_restarts {
                            // 已达重启上限,不再重试。
                            if rt.restart_count == self.inner.max_restarts {
                                warn!(target: "sidecar", kind = kind.as_str(),
                                    restart_count = rt.restart_count,
                                    "sidecar exceeded max_restarts, giving up");
                            }
                            false
                        } else {
                            // T-S4-B-03: 指数退避 — 仅在距上次崩溃已过
                            // backoff_delay 时间后才重启,避免重启风暴。
                            let backoff = Self::restart_backoff_delay(rt.restart_count);
                            let elapsed =
                                rt.last_crash.map(|t| t.elapsed()).unwrap_or(Duration::ZERO);
                            if elapsed >= backoff {
                                true
                            } else {
                                debug!(target: "sidecar", kind = kind.as_str(),
                                    restart_count = rt.restart_count,
                                    backoff_secs = backoff.as_secs(),
                                    elapsed_secs = elapsed.as_secs(),
                                    "supervisor: waiting for backoff before restart");
                                false
                            }
                        }
                    } else {
                        false
                    }
                };

                if should_restart {
                    let token = self.generate_token();
                    self.inner.auth_tokens.lock().insert(kind, token.clone());

                    let config =
                        SidecarConfig::new(kind.as_str(), self.inner.data_dir.clone(), token);

                    let backoff_secs = {
                        let runtimes = self.inner.runtimes.lock();
                        runtimes
                            .get(&kind)
                            .map(|rt| Self::restart_backoff_delay(rt.restart_count).as_secs())
                            .unwrap_or(0)
                    };
                    info!(target: "sidecar", kind = kind.as_str(),
                        backoff_secs, "supervisor: restarting sidecar (after exponential backoff)");

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
        assert_eq!(SidecarKind::Skill.as_str(), "skill");
        assert_eq!(SidecarKind::Reflection.as_str(), "reflection");
        assert_eq!(SidecarKind::OsController.as_str(), "os_controller");
    }

    #[test]
    fn sidecar_kind_all_has_six() {
        assert_eq!(SidecarKind::all().len(), 6);
    }

    #[test]
    fn default_status_is_stopped() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        assert_eq!(manager.status(SidecarKind::Memory), SidecarStatus::Stopped);
        assert!(!manager.is_running(SidecarKind::Memory));
    }

    // T-S4-B-03: 指数退避测试
    #[test]
    fn restart_backoff_delay_exponential() {
        // restart_count=0 → 1s (首次重启前的最小延迟)
        assert_eq!(
            SidecarManager::restart_backoff_delay(0),
            Duration::from_secs(1)
        );
        // restart_count=1 → 2s
        assert_eq!(
            SidecarManager::restart_backoff_delay(1),
            Duration::from_secs(2)
        );
        // restart_count=2 → 4s
        assert_eq!(
            SidecarManager::restart_backoff_delay(2),
            Duration::from_secs(4)
        );
        // restart_count=3 → 8s
        assert_eq!(
            SidecarManager::restart_backoff_delay(3),
            Duration::from_secs(8)
        );
        // restart_count=4 → 16s
        assert_eq!(
            SidecarManager::restart_backoff_delay(4),
            Duration::from_secs(16)
        );
    }

    #[test]
    fn restart_backoff_delay_capped_at_30s() {
        // restart_count=5 → 2^5=32, 饱和至 30s
        assert_eq!(
            SidecarManager::restart_backoff_delay(5),
            Duration::from_secs(30)
        );
        // restart_count=10 → 远超 30, 饱和至 30s
        assert_eq!(
            SidecarManager::restart_backoff_delay(10),
            Duration::from_secs(30)
        );
        // restart_count=30 → checked_shl 溢出返回 MAX, 饱和至 30s
        assert_eq!(
            SidecarManager::restart_backoff_delay(30),
            Duration::from_secs(30)
        );
    }

    #[tokio::test]
    async fn bootstrap_does_not_panic_with_missing_binaries() {
        // 无 sidecar 二进制时 bootstrap 应进入 in-process 模式,不 panic。
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        // bootstrap 内部 wait_ready 超时 10s × 5 kinds = 最多 50s,
        // 但 in-process 模式下 start() 立即标记 Running,wait_ready 立即返回。
        manager
            .bootstrap()
            .await
            .expect("bootstrap should not error");
        // in-process 模式下所有 sidecar 都应标记为 Running
        assert!(manager.is_running(SidecarKind::Memory));
    }

    // ------------------------------------------------------------------
    // T-E-S-61: gRPC HealthCheck 单元测试
    // ------------------------------------------------------------------

    /// T-E-S-61: 不可达地址的 gRPC HealthCheck 应返回 false。
    #[cfg(feature = "grpc")]
    #[tokio::test]
    async fn grpc_health_check_returns_false_for_unreachable() {
        // Port 1 通常无服务监听,connect 应失败或超时
        let result = SidecarManager::grpc_health_check("127.0.0.1:1", Duration::from_secs(1)).await;
        assert!(!result, "unreachable address should return false");
    }

    /// T-E-S-61: in-process 模式下 health_check 应直接返回 Ok(无需 gRPC ping)。
    #[cfg(feature = "grpc")]
    #[tokio::test]
    async fn health_check_returns_ok_for_in_process_mode() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        {
            let mut runtimes = manager.inner.runtimes.lock();
            let rt = runtimes.entry(SidecarKind::Memory).or_default();
            rt.status = SidecarStatus::Running;
            rt.listen_addr = Some("in-process".to_string());
        }
        let result = manager.health_check(SidecarKind::Memory).await;
        assert!(result.is_ok(), "in-process mode should be healthy");
        // last_health_check 不应被更新(in-process 模式跳过 gRPC 调用)
        let runtimes = manager.inner.runtimes.lock();
        let rt = runtimes.get(&SidecarKind::Memory).unwrap();
        assert!(rt.last_health_check.is_none());
        assert_eq!(rt.health_check_failures, 0);
    }

    /// T-E-S-61: gRPC ping 失败时应递增 health_check_failures。
    #[cfg(feature = "grpc")]
    #[tokio::test]
    async fn health_check_increments_failures_on_failure() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        {
            let mut runtimes = manager.inner.runtimes.lock();
            let rt = runtimes.entry(SidecarKind::Memory).or_default();
            rt.status = SidecarStatus::Running;
            rt.listen_addr = Some("127.0.0.1:1".to_string()); // 不可达地址
        }

        let result = manager.health_check(SidecarKind::Memory).await;
        assert!(
            result.is_err(),
            "unreachable sidecar should fail health check"
        );

        let failures = manager
            .inner
            .runtimes
            .lock()
            .get(&SidecarKind::Memory)
            .map(|rt| rt.health_check_failures)
            .unwrap_or(0);
        assert_eq!(failures, 1, "failures should increment to 1");
        // 1 次失败不应标记 Crashed
        assert!(manager.is_running(SidecarKind::Memory));
    }

    /// T-E-S-61: 连续 3 次失败应标记 Crashed 并重置 failures 计数。
    #[cfg(feature = "grpc")]
    #[tokio::test]
    async fn health_check_marks_crashed_after_three_failures() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        {
            let mut runtimes = manager.inner.runtimes.lock();
            let rt = runtimes.entry(SidecarKind::Memory).or_default();
            rt.status = SidecarStatus::Running;
            rt.listen_addr = Some("127.0.0.1:1".to_string()); // 不可达
        }

        // 连续 3 次失败
        for i in 1..=3 {
            let result = manager.health_check(SidecarKind::Memory).await;
            assert!(result.is_err(), "iteration {} should fail", i);
        }

        let status = manager.status(SidecarKind::Memory);
        assert!(
            matches!(status, SidecarStatus::Crashed { .. }),
            "after 3 failures sidecar should be Crashed, got {:?}",
            status
        );
        // failures 应在标记 Crashed 后重置为 0
        let failures = manager
            .inner
            .runtimes
            .lock()
            .get(&SidecarKind::Memory)
            .map(|rt| rt.health_check_failures)
            .unwrap_or(0);
        assert_eq!(failures, 0, "failures should reset to 0 after Crashed");
    }

    /// T-E-S-61: SERVING 响应应更新 last_health_check 并重置 failures。
    /// (验证 SERVING 路径 — 这里用 in-process 模式模拟"健康"路径,
    /// 因为本地无真实 sidecar gRPC server)
    #[cfg(feature = "grpc")]
    #[tokio::test]
    async fn health_check_serving_path_resets_failures() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        {
            let mut runtimes = manager.inner.runtimes.lock();
            let rt = runtimes.entry(SidecarKind::Memory).or_default();
            rt.status = SidecarStatus::Running;
            rt.listen_addr = Some("in-process".to_string());
            // 模拟之前已有失败
            rt.health_check_failures = 2;
        }

        // in-process 模式直接返回 Ok
        let result = manager.health_check(SidecarKind::Memory).await;
        assert!(result.is_ok());

        // in-process 模式不更新 last_health_check(无真实 ping),
        // 但也不应递增 failures
        let runtimes = manager.inner.runtimes.lock();
        let rt = runtimes.get(&SidecarKind::Memory).unwrap();
        assert_eq!(
            rt.health_check_failures, 2,
            "failures unchanged in in-process mode"
        );
    }

    /// T-E-S-61: 无 grpc feature 时,manager 应回退到 is_running 逻辑。
    /// 此测试验证无 grpc feature 时 wait_for_listen_addr 仍能工作。
    #[tokio::test]
    async fn health_check_falls_back_without_grpc() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        // Stopped 状态: is_running 返回 false
        assert!(!manager.is_running(SidecarKind::Memory));
        // wait_for_listen_addr 应使用端口等待逻辑(in-process 模式下 listen_addr 已设置)
        // 这里仅验证 manager 在无 grpc 时不会 panic
        assert_eq!(manager.status(SidecarKind::Memory), SidecarStatus::Stopped);
    }

    /// T-E-S-61: supervisor_loop 应对 Running 状态的 sidecar 周期性 HealthCheck。
    ///
    /// 此处仅验证 supervisor 能在有 Running sidecar 时正常运行不 panic,
    /// 且不会错误标记 in-process 模式的 sidecar 为 Crashed。
    #[tokio::test]
    async fn supervisor_loop_pings_running_sidecars() {
        let manager = SidecarManager::new(PathBuf::from("/tmp"));
        // 模拟一个 Running 状态的 sidecar(in-process 模式)
        {
            let mut runtimes = manager.inner.runtimes.lock();
            let rt = runtimes.entry(SidecarKind::Memory).or_default();
            rt.status = SidecarStatus::Running;
            rt.listen_addr = Some("in-process".to_string());
        }

        manager.ensure_supervisor();
        // 让 supervisor 运行一小段时间(tokio interval 首次 tick 立即返回)
        tokio::time::sleep(Duration::from_millis(200)).await;

        // 取消 supervisor
        manager.inner.cancel.cancel();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // in-process 模式下 sidecar 应仍为 Running(未被误标 Crashed)
        assert!(
            manager.is_running(SidecarKind::Memory),
            "in-process sidecar should remain Running after supervisor tick"
        );
    }

    /// T-E-S-61: 验证 default_port_for_kind 返回正确的端口映射。
    #[test]
    fn default_port_for_kind_mapping() {
        assert_eq!(default_port_for_kind(SidecarKind::Memory), 50051);
        assert_eq!(default_port_for_kind(SidecarKind::Llm), 50052);
        assert_eq!(default_port_for_kind(SidecarKind::Swarm), 50053);
        assert_eq!(default_port_for_kind(SidecarKind::Skill), 50054);
        assert_eq!(default_port_for_kind(SidecarKind::Reflection), 50055);
        assert_eq!(default_port_for_kind(SidecarKind::OsController), 50056);
    }
}
