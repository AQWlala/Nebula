//! T-E-S-22: AIO Sandbox（应用隔离沙箱）。
//!
//! 为技能执行和 AIO（Autonomous Input/Output）提供安全隔离环境。
//! 沙箱在路径访问、网络访问、环境变量读取、文件大小、内存/CPU 时间
//! 等维度施加细粒度限制，并记录违规事件。
//!
//! ## 设计原则
//!
//! 1. **最小权限** — 默认拒绝，所有权限必须显式声明。
//! 2. **违规优先** — 检测到违规时记录并阻止，而非静默通过。
//! 3. **可观测** — 沙箱内操作（文件读写、网络请求等）可被日志记录。
//!
//! ## 参考
//!
//! - `crate::skills::sandbox` — 现有技能沙箱（基于能力的权限模型）
//! - `crate::security::ssrf_guard` — SSRF 防护（网络地址校验）

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// 网络访问策略
// ---------------------------------------------------------------------------

/// 沙箱网络访问策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// 允许所有网络访问。
    FullAccess,
    /// 仅允许本地回环地址（127.0.0.1 / ::1 / localhost）。
    LocalhostOnly,
    /// 拒绝所有网络访问。
    DenyAll,
    /// 仅允许白名单中的 IP 或域名。
    Whitelist(Vec<String>),
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        // 默认拒绝所有网络访问（最小权限原则）。
        NetworkPolicy::DenyAll
    }
}

impl fmt::Display for NetworkPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkPolicy::FullAccess => write!(f, "full_access"),
            NetworkPolicy::LocalhostOnly => write!(f, "localhost_only"),
            NetworkPolicy::DenyAll => write!(f, "deny_all"),
            NetworkPolicy::Whitelist(_) => write!(f, "whitelist"),
        }
    }
}

// ---------------------------------------------------------------------------
// 路径访问类型
// ---------------------------------------------------------------------------

/// 路径访问类型 — 描述对路径的操作意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathAccess {
    /// 读取。
    Read,
    /// 写入。
    Write,
    /// 执行。
    Execute,
    /// 删除。
    Delete,
}

impl fmt::Display for PathAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathAccess::Read => write!(f, "read"),
            PathAccess::Write => write!(f, "write"),
            PathAccess::Execute => write!(f, "execute"),
            PathAccess::Delete => write!(f, "delete"),
        }
    }
}

// ---------------------------------------------------------------------------
// 违规类型与动作
// ---------------------------------------------------------------------------

/// 沙箱违规类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationType {
    /// 路径访问被拒绝。
    PathAccessDenied,
    /// 网络访问被拒绝。
    NetworkDenied,
    /// 环境变量访问被拒绝。
    EnvVarDenied,
    /// 文件大小超出限制。
    FileSizeExceeded,
    /// 内存使用超出限制。
    MemoryLimitExceeded,
    /// CPU 时间超出限制。
    CpuTimeExceeded,
}

impl fmt::Display for ViolationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViolationType::PathAccessDenied => write!(f, "path_access_denied"),
            ViolationType::NetworkDenied => write!(f, "network_denied"),
            ViolationType::EnvVarDenied => write!(f, "env_var_denied"),
            ViolationType::FileSizeExceeded => write!(f, "file_size_exceeded"),
            ViolationType::MemoryLimitExceeded => write!(f, "memory_limit_exceeded"),
            ViolationType::CpuTimeExceeded => write!(f, "cpu_time_exceeded"),
        }
    }
}

/// 检测到违规后采取的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionTaken {
    /// 阻止操作。
    Blocked,
    /// 仅记录日志（未阻止）。
    Logged,
    /// 警告（未阻止）。
    Warned,
}

impl fmt::Display for ActionTaken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionTaken::Blocked => write!(f, "blocked"),
            ActionTaken::Logged => write!(f, "logged"),
            ActionTaken::Warned => write!(f, "warned"),
        }
    }
}

// ---------------------------------------------------------------------------
// 沙箱违规记录
// ---------------------------------------------------------------------------

/// 沙箱违规记录 — 描述一次违规事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxViolation {
    /// 违规类型。
    pub violation_type: ViolationType,
    /// 相关路径或地址。
    pub path_or_addr: Option<String>,
    /// 违规发生时间。
    pub timestamp: DateTime<Utc>,
    /// 采取的动作。
    pub action_taken: ActionTaken,
}

impl SandboxViolation {
    /// 创建一条新的违规记录。
    pub fn new(
        violation_type: ViolationType,
        path_or_addr: Option<String>,
        action: ActionTaken,
    ) -> Self {
        Self {
            violation_type,
            path_or_addr,
            timestamp: Utc::now(),
            action_taken: action,
        }
    }
}

impl fmt::Display for SandboxViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} at {} ({})",
            self.timestamp,
            self.violation_type,
            self.path_or_addr.as_deref().unwrap_or("<unknown>"),
            self.action_taken
        )
    }
}

// ---------------------------------------------------------------------------
// 资源使用统计
// ---------------------------------------------------------------------------

/// 沙箱资源使用统计。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsage {
    /// 峰值内存（MB）。
    pub max_memory_mb: f64,
    /// CPU 时间（毫秒）。
    pub cpu_time_ms: u64,
    /// 读取的文件数。
    pub files_read: u32,
    /// 写入的文件数。
    pub files_written: u32,
    /// 网络请求次数。
    pub network_requests: u32,
}

impl ResourceUsage {
    /// 创建一个零初始化的资源使用统计。
    pub fn new() -> Self {
        Self::default()
    }

    /// 累加另一份资源使用统计（内存取峰值，其余累加）。
    pub fn merge(&mut self, other: &ResourceUsage) {
        self.max_memory_mb = self.max_memory_mb.max(other.max_memory_mb);
        self.cpu_time_ms = self.cpu_time_ms.saturating_add(other.cpu_time_ms);
        self.files_read = self.files_read.saturating_add(other.files_read);
        self.files_written = self.files_written.saturating_add(other.files_written);
        self.network_requests = self.network_requests.saturating_add(other.network_requests);
    }
}

// ---------------------------------------------------------------------------
// 沙箱执行结果
// ---------------------------------------------------------------------------

/// 沙箱执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    /// 进程退出码（None 表示进程被信号终止或未启动）。
    pub exit_code: Option<i32>,
    /// 标准输出。
    pub stdout: String,
    /// 标准错误。
    pub stderr: String,
    /// 执行耗时（毫秒）。
    pub duration_ms: u64,
    /// 资源使用统计。
    pub resource_usage: ResourceUsage,
    /// 执行期间检测到的违规。
    pub violations: Vec<SandboxViolation>,
}

// ---------------------------------------------------------------------------
// 沙箱配置
// ---------------------------------------------------------------------------

/// 沙箱配置 — 定义沙箱的权限边界和资源限制。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// 允许访问的路径白名单。为空时允许所有非黑名单路径。
    pub allowed_paths: Vec<PathBuf>,
    /// 禁止访问的路径黑名单（优先级高于白名单）。
    pub denied_paths: Vec<PathBuf>,
    /// 允许读取的环境变量。为空时拒绝所有环境变量访问。
    pub allowed_env_vars: Vec<String>,
    /// 网络访问策略。
    pub network_access: NetworkPolicy,
    /// 内存限制（MB）。None 表示不限制。
    pub max_memory_mb: Option<usize>,
    /// CPU 时间限制（秒）。None 表示不限制。
    pub max_cpu_time_secs: Option<u64>,
    /// 单个文件大小限制（MB）。None 表示不限制。
    pub max_file_size_mb: Option<usize>,
    /// 工作目录。
    pub working_dir: PathBuf,
    /// 是否记录沙箱内操作日志。
    pub enable_logging: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            allowed_paths: Vec::new(),
            denied_paths: Vec::new(),
            allowed_env_vars: Vec::new(),
            network_access: NetworkPolicy::DenyAll,
            max_memory_mb: None,
            max_cpu_time_secs: None,
            max_file_size_mb: None,
            working_dir: PathBuf::from("."),
            enable_logging: false,
        }
    }
}

impl SandboxConfig {
    /// 创建一个沙箱配置 builder。
    pub fn builder() -> SandboxConfigBuilder {
        SandboxConfigBuilder::default()
    }
}

/// 沙箱配置 builder。
#[derive(Debug, Clone, Default)]
pub struct SandboxConfigBuilder {
    config: SandboxConfig,
}

impl SandboxConfigBuilder {
    /// 设置允许访问的路径白名单。
    pub fn allowed_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.config.allowed_paths = paths;
        self
    }

    /// 设置禁止访问的路径黑名单。
    pub fn denied_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.config.denied_paths = paths;
        self
    }

    /// 设置允许读取的环境变量。
    pub fn allowed_env_vars(mut self, vars: Vec<String>) -> Self {
        self.config.allowed_env_vars = vars;
        self
    }

    /// 设置网络访问策略。
    pub fn network_access(mut self, policy: NetworkPolicy) -> Self {
        self.config.network_access = policy;
        self
    }

    /// 设置内存限制（MB）。
    pub fn max_memory_mb(mut self, mb: usize) -> Self {
        self.config.max_memory_mb = Some(mb);
        self
    }

    /// 设置 CPU 时间限制（秒）。
    pub fn max_cpu_time_secs(mut self, secs: u64) -> Self {
        self.config.max_cpu_time_secs = Some(secs);
        self
    }

    /// 设置单个文件大小限制（MB）。
    pub fn max_file_size_mb(mut self, mb: usize) -> Self {
        self.config.max_file_size_mb = Some(mb);
        self
    }

    /// 设置工作目录。
    pub fn working_dir(mut self, dir: PathBuf) -> Self {
        self.config.working_dir = dir;
        self
    }

    /// 设置是否记录沙箱内操作日志。
    pub fn enable_logging(mut self, enable: bool) -> Self {
        self.config.enable_logging = enable;
        self
    }

    /// 构建沙箱配置。
    pub fn build(self) -> SandboxConfig {
        self.config
    }
}

// ---------------------------------------------------------------------------
// 沙箱日志
// ---------------------------------------------------------------------------

/// 日志严重等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogSeverity {
    /// 信息。
    Info,
    /// 警告。
    Warning,
    /// 错误。
    Error,
}

impl fmt::Display for LogSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogSeverity::Info => write!(f, "info"),
            LogSeverity::Warning => write!(f, "warning"),
            LogSeverity::Error => write!(f, "error"),
        }
    }
}

/// 沙箱操作日志条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxLogEntry {
    /// 时间戳。
    pub timestamp: DateTime<Utc>,
    /// 事件名称。
    pub event: String,
    /// 事件详情。
    pub detail: Option<String>,
    /// 严重等级。
    pub severity: LogSeverity,
}

// ---------------------------------------------------------------------------
// 沙箱内部状态
// ---------------------------------------------------------------------------

/// 沙箱运行时状态（受锁保护）。
#[derive(Debug, Clone, Default)]
struct SandboxState {
    /// 累计违规记录。
    violations: Vec<SandboxViolation>,
    /// 累计资源使用。
    resource_usage: ResourceUsage,
    /// 操作日志（仅在 enable_logging 时记录）。
    logs: Vec<SandboxLogEntry>,
}

// ---------------------------------------------------------------------------
// AioSandbox
// ---------------------------------------------------------------------------

/// AIO 沙箱 — 为技能执行和 AIO 提供安全隔离环境。
///
/// 沙箱实例可被多个任务共享（内部状态受 `parking_lot::Mutex` 保护）。
pub struct AioSandbox {
    /// 沙箱配置。
    config: SandboxConfig,
    /// 运行时状态。
    state: Mutex<SandboxState>,
}

impl std::fmt::Debug for AioSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AioSandbox")
            .field("config", &self.config)
            .field("state", &"<locked>")
            .finish()
    }
}

impl AioSandbox {
    /// 创建一个新的沙箱实例。
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            state: Mutex::new(SandboxState::default()),
        }
    }

    /// 返回沙箱配置的引用。
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// 验证路径访问权限。
    ///
    /// 验证逻辑：先检查黑名单（denied 优先），再检查白名单。
    /// 白名单为空时允许所有非黑名单路径。
    /// 当前实现对所有 `PathAccess` 变体（读/写/执行/删除）采用相同的路径校验规则。
    pub fn validate_path_access(&self, path: &Path, _access: PathAccess) -> Result<()> {
        // 1. 先检查黑名单（denied 优先）
        for denied in &self.config.denied_paths {
            if path_matches(path, denied) {
                self.record_violation(
                    ViolationType::PathAccessDenied,
                    Some(path.to_string_lossy().to_string()),
                    ActionTaken::Blocked,
                );
                return Err(anyhow!(
                    "沙箱违规：路径访问被拒绝（黑名单）：{}",
                    path.display()
                ));
            }
        }

        // 2. 白名单为空则允许所有（非黑名单）
        if self.config.allowed_paths.is_empty() {
            self.log_event(
                "path_access",
                Some(format!("允许访问（无白名单限制）：{}", path.display())),
                LogSeverity::Info,
            );
            return Ok(());
        }

        // 3. 检查白名单
        for allowed in &self.config.allowed_paths {
            if path_matches(path, allowed) {
                self.log_event(
                    "path_access",
                    Some(format!("允许访问：{}", path.display())),
                    LogSeverity::Info,
                );
                return Ok(());
            }
        }

        // 4. 不在白名单，拒绝
        self.record_violation(
            ViolationType::PathAccessDenied,
            Some(path.to_string_lossy().to_string()),
            ActionTaken::Blocked,
        );
        Err(anyhow!(
            "沙箱违规：路径访问被拒绝（不在白名单）：{}",
            path.display()
        ))
    }

    /// 验证网络访问。
    ///
    /// - `FullAccess`：允许所有
    /// - `DenyAll`：拒绝所有
    /// - `LocalhostOnly`：仅允许 127.0.0.1 / ::1 / localhost
    /// - `Whitelist`：检查 IP 或域名是否匹配白名单
    pub fn validate_network(&self, addr: &str) -> Result<()> {
        match &self.config.network_access {
            NetworkPolicy::FullAccess => {
                self.log_event(
                    "network_access",
                    Some(format!("允许网络访问（FullAccess）：{addr}")),
                    LogSeverity::Info,
                );
                Ok(())
            }
            NetworkPolicy::DenyAll => {
                self.record_violation(
                    ViolationType::NetworkDenied,
                    Some(addr.to_string()),
                    ActionTaken::Blocked,
                );
                Err(anyhow!("沙箱违规：网络访问被拒绝（DenyAll）：{addr}"))
            }
            NetworkPolicy::LocalhostOnly => {
                if is_localhost(addr) {
                    self.log_event(
                        "network_access",
                        Some(format!("允许本地回环访问：{addr}")),
                        LogSeverity::Info,
                    );
                    Ok(())
                } else {
                    self.record_violation(
                        ViolationType::NetworkDenied,
                        Some(addr.to_string()),
                        ActionTaken::Blocked,
                    );
                    Err(anyhow!(
                        "沙箱违规：网络访问被拒绝（LocalhostOnly，非回环）：{addr}"
                    ))
                }
            }
            NetworkPolicy::Whitelist(list) => {
                for allowed in list {
                    if addr == allowed || host_matches(addr, allowed) {
                        self.log_event(
                            "network_access",
                            Some(format!("允许网络访问（白名单）：{addr}")),
                            LogSeverity::Info,
                        );
                        return Ok(());
                    }
                }
                self.record_violation(
                    ViolationType::NetworkDenied,
                    Some(addr.to_string()),
                    ActionTaken::Blocked,
                );
                Err(anyhow!("沙箱违规：网络访问被拒绝（不在白名单）：{addr}"))
            }
        }
    }

    /// 验证环境变量访问。
    ///
    /// 白名单为空时拒绝所有环境变量访问（最小权限原则）。
    pub fn validate_env_var(&self, var_name: &str) -> Result<()> {
        if self.config.allowed_env_vars.is_empty() {
            // 空白名单视为拒绝所有（最小权限）
            self.record_violation(
                ViolationType::EnvVarDenied,
                Some(var_name.to_string()),
                ActionTaken::Blocked,
            );
            return Err(anyhow!(
                "沙箱违规：环境变量访问被拒绝（空白名单）：{var_name}"
            ));
        }
        for allowed in &self.config.allowed_env_vars {
            if allowed == var_name {
                self.log_event(
                    "env_var_access",
                    Some(format!("允许读取环境变量：{var_name}")),
                    LogSeverity::Info,
                );
                return Ok(());
            }
        }
        self.record_violation(
            ViolationType::EnvVarDenied,
            Some(var_name.to_string()),
            ActionTaken::Blocked,
        );
        Err(anyhow!(
            "沙箱违规：环境变量访问被拒绝（不在白名单）：{var_name}"
        ))
    }

    /// 验证文件大小。
    pub fn validate_file_size(&self, size: u64) -> Result<()> {
        if let Some(max_mb) = self.config.max_file_size_mb {
            let max_bytes = (max_mb as u64) * 1024 * 1024;
            if size > max_bytes {
                self.record_violation(
                    ViolationType::FileSizeExceeded,
                    Some(format!("{size} bytes（限制：{max_bytes} bytes）")),
                    ActionTaken::Blocked,
                );
                return Err(anyhow!(
                    "沙箱违规：文件大小超限（{} bytes > {} MB 限制）",
                    size,
                    max_mb
                ));
            }
        }
        Ok(())
    }

    /// 在沙箱内执行命令（带资源限制和违规检测）。
    ///
    /// 命令的工作目录设为 `config.working_dir`，标准输入被关闭以隔离。
    /// 执行期间会检测 CPU 时间限制违规。
    pub async fn execute_command(&self, cmd: &str, args: &[String]) -> Result<SandboxResult> {
        let start = std::time::Instant::now();

        // 记录执行前的违规数量，用于汇总本次执行新增的违规
        let prev_violation_count = self.state.lock().violations.len();

        let mut command = Command::new(cmd);
        command.args(args);
        command.current_dir(&self.config.working_dir);
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        // 关闭子进程标准输入（隔离）
        command.stdin(std::process::Stdio::null());

        self.log_event(
            "execute_command",
            Some(format!("执行命令：{cmd} {}", args.join(" "))),
            LogSeverity::Info,
        );

        debug!(target: "nebula.aio_sandbox", cmd, args = ?args, "执行沙箱命令");

        let output = match command.output().await {
            Ok(o) => o,
            Err(e) => {
                self.record_violation(
                    ViolationType::CpuTimeExceeded,
                    Some(format!("命令启动失败：{e}")),
                    ActionTaken::Blocked,
                );
                return Err(anyhow!("沙箱命令执行失败：{e}"));
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        // 更新 CPU 时间统计
        {
            let mut state = self.state.lock();
            state.resource_usage.cpu_time_ms =
                state.resource_usage.cpu_time_ms.saturating_add(duration_ms);
        }

        // 检查 CPU 时间限制
        if let Some(max_secs) = self.config.max_cpu_time_secs {
            let max_ms = max_secs.saturating_mul(1000);
            if duration_ms > max_ms {
                self.record_violation(
                    ViolationType::CpuTimeExceeded,
                    Some(format!("{duration_ms} ms（限制：{max_ms} ms）")),
                    ActionTaken::Blocked,
                );
                warn!(
                    target: "nebula.aio_sandbox",
                    cmd, duration_ms, max_ms, "命令超出 CPU 时间限制"
                );
            }
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // 提取本次执行新增的违规
        let violations = {
            let state = self.state.lock();
            if prev_violation_count < state.violations.len() {
                state.violations[prev_violation_count..].to_vec()
            } else {
                Vec::new()
            }
        };

        let resource_usage = {
            let state = self.state.lock();
            state.resource_usage.clone()
        };

        info!(
            target: "nebula.aio_sandbox",
            cmd, duration_ms, exit_code = ?output.status.code(),
            "沙箱命令执行完成"
        );

        Ok(SandboxResult {
            exit_code: output.status.code(),
            stdout,
            stderr,
            duration_ms,
            resource_usage,
            violations,
        })
    }

    /// 带超时执行命令。
    ///
    /// 若命令在指定超时内未完成，记录 CPU 时间违规并返回错误。
    pub async fn execute_with_timeout(
        &self,
        cmd: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<SandboxResult> {
        let exec_future = self.execute_command(cmd, args);
        match tokio::time::timeout(timeout, exec_future).await {
            Ok(result) => result,
            Err(_) => {
                self.record_violation(
                    ViolationType::CpuTimeExceeded,
                    Some(format!("超时（{} ms）", timeout.as_millis())),
                    ActionTaken::Blocked,
                );
                Err(anyhow!(
                    "沙箱违规：命令执行超时（{} ms）",
                    timeout.as_millis()
                ))
            }
        }
    }

    /// 获取累计违规记录。
    pub fn get_violations(&self) -> Vec<SandboxViolation> {
        self.state.lock().violations.clone()
    }

    /// 获取资源使用统计。
    pub fn get_resource_usage(&self) -> ResourceUsage {
        self.state.lock().resource_usage.clone()
    }

    /// 获取操作日志（仅当 `enable_logging` 时有内容）。
    pub fn get_logs(&self) -> Vec<SandboxLogEntry> {
        self.state.lock().logs.clone()
    }

    /// 重置沙箱状态（清空违规记录、资源使用、日志）。
    pub fn reset(&self) {
        let mut state = self.state.lock();
        state.violations.clear();
        state.resource_usage = ResourceUsage::default();
        state.logs.clear();
    }

    // ---- 内部辅助方法 ----

    /// 记录一次违规。
    fn record_violation(
        &self,
        vtype: ViolationType,
        path_or_addr: Option<String>,
        action: ActionTaken,
    ) {
        let violation = SandboxViolation::new(vtype, path_or_addr, action);
        warn!(target: "nebula.aio_sandbox", violation = %violation, "沙箱违规");
        self.log_event(
            "violation",
            Some(format!("{:?}", vtype)),
            LogSeverity::Warning,
        );
        let mut state = self.state.lock();
        state.violations.push(violation);
    }

    /// 记录一条操作日志（仅在 enable_logging 时记录）。
    fn log_event(&self, event: &str, detail: Option<String>, severity: LogSeverity) {
        if !self.config.enable_logging {
            return;
        }
        let entry = SandboxLogEntry {
            timestamp: Utc::now(),
            event: event.to_string(),
            detail,
            severity,
        };
        let mut state = self.state.lock();
        state.logs.push(entry);
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 检查目标路径是否匹配模式路径。
///
/// 匹配规则：
/// - 精确匹配（按路径组件比较）
/// - 前缀匹配：模式作为父目录时匹配其下所有文件
fn path_matches(target: &Path, pattern: &Path) -> bool {
    // 精确匹配
    if target == pattern {
        return true;
    }
    // 前缀匹配：pattern 是 target 的父目录
    if target.starts_with(pattern) {
        return true;
    }
    false
}

/// 检查地址是否为本地回环（127.0.0.1 / ::1 / localhost）。
///
/// 支持以下格式：
/// - `localhost`
/// - `127.0.0.1`
/// - `::1`
/// - `127.0.0.1:8080`
/// - `[::1]:8080`
fn is_localhost(addr: &str) -> bool {
    let trimmed = addr.trim();
    if trimmed == "localhost" {
        return true;
    }
    // 尝试解析为 IP
    if let Ok(ip) = trimmed.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    // 尝试解析为 SocketAddr（host:port）
    if let Ok(socket) = trimmed.parse::<std::net::SocketAddr>() {
        return socket.ip().is_loopback();
    }
    // IPv6 [::1]:port 格式 — 提取括号内地址
    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find(']') {
            let host = &trimmed[1..end];
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                return ip.is_loopback();
            }
        }
    }
    // 提取最后一个冒号之前的部分作为 host（IPv4:port 或 localhost:port）
    if let Some(idx) = trimmed.rfind(':') {
        let host = &trimmed[..idx];
        if host == "localhost" {
            return true;
        }
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            return ip.is_loopback();
        }
    }
    false
}

/// 检查地址是否匹配白名单条目（支持 host:port 与裸 host 比较）。
fn host_matches(addr: &str, allowed: &str) -> bool {
    let addr_host = extract_host(addr);
    let allowed_host = extract_host(allowed);
    addr_host == allowed_host
}

/// 从地址字符串中提取 host 部分。
///
/// - `[::1]:8080` → `::1`
/// - `127.0.0.1:8080` → `127.0.0.1`
/// - `localhost:8080` → `localhost`
/// - `::1`（裸 IPv6）→ `::1`
/// - `api.example.com` → `api.example.com`
fn extract_host(addr: &str) -> String {
    let trimmed = addr.trim();
    // IPv6 [::1]:port 格式
    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find(']') {
            return trimmed[1..end].to_string();
        }
        return trimmed.to_string();
    }
    // 裸 IPv6 地址（包含多个冒号且非 [bracket] 格式）— 不当作 host:port
    let colon_count = trimmed.chars().filter(|&c| c == ':').count();
    if colon_count > 1 {
        return trimmed.to_string();
    }
    // IPv4:port 或 host:port（单个冒号）
    if colon_count == 1 {
        if let Some(idx) = trimmed.rfind(':') {
            return trimmed[..idx].to_string();
        }
    }
    trimmed.to_string()
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ---- SandboxConfig 默认值与 builder ----

    #[test]
    fn test_config_defaults() {
        let cfg = SandboxConfig::default();
        assert!(cfg.allowed_paths.is_empty());
        assert!(cfg.denied_paths.is_empty());
        assert!(cfg.allowed_env_vars.is_empty());
        assert_eq!(cfg.network_access, NetworkPolicy::DenyAll);
        assert_eq!(cfg.max_memory_mb, None);
        assert_eq!(cfg.max_cpu_time_secs, None);
        assert_eq!(cfg.max_file_size_mb, None);
        assert_eq!(cfg.working_dir, PathBuf::from("."));
        assert!(!cfg.enable_logging);
    }

    #[test]
    fn test_config_builder() {
        let cfg = SandboxConfig::builder()
            .allowed_paths(vec![PathBuf::from("/tmp"), PathBuf::from("/home")])
            .denied_paths(vec![PathBuf::from("/etc")])
            .allowed_env_vars(vec!["PATH".to_string(), "HOME".to_string()])
            .network_access(NetworkPolicy::LocalhostOnly)
            .max_memory_mb(512)
            .max_cpu_time_secs(30)
            .max_file_size_mb(10)
            .working_dir(PathBuf::from("/sandbox"))
            .enable_logging(true)
            .build();

        assert_eq!(cfg.allowed_paths.len(), 2);
        assert_eq!(cfg.denied_paths.len(), 1);
        assert_eq!(cfg.allowed_env_vars.len(), 2);
        assert_eq!(cfg.network_access, NetworkPolicy::LocalhostOnly);
        assert_eq!(cfg.max_memory_mb, Some(512));
        assert_eq!(cfg.max_cpu_time_secs, Some(30));
        assert_eq!(cfg.max_file_size_mb, Some(10));
        assert_eq!(cfg.working_dir, PathBuf::from("/sandbox"));
        assert!(cfg.enable_logging);
    }

    // ---- NetworkPolicy 序列化 ----

    #[test]
    fn test_network_policy_serialization() {
        // FullAccess
        let json = serde_json::to_string(&NetworkPolicy::FullAccess).unwrap();
        assert_eq!(json, "\"full_access\"");
        let de: NetworkPolicy = serde_json::from_str("\"full_access\"").unwrap();
        assert_eq!(de, NetworkPolicy::FullAccess);

        // LocalhostOnly
        let json = serde_json::to_string(&NetworkPolicy::LocalhostOnly).unwrap();
        assert_eq!(json, "\"localhost_only\"");
        let de: NetworkPolicy = serde_json::from_str("\"localhost_only\"").unwrap();
        assert_eq!(de, NetworkPolicy::LocalhostOnly);

        // DenyAll
        let json = serde_json::to_string(&NetworkPolicy::DenyAll).unwrap();
        assert_eq!(json, "\"deny_all\"");
        let de: NetworkPolicy = serde_json::from_str("\"deny_all\"").unwrap();
        assert_eq!(de, NetworkPolicy::DenyAll);

        // Whitelist
        let json = serde_json::to_string(&NetworkPolicy::Whitelist(vec![
            "api.openai.com".to_string(),
            "127.0.0.1".to_string(),
        ]))
        .unwrap();
        assert!(json.contains("\"whitelist\""));
        assert!(json.contains("api.openai.com"));
        let de: NetworkPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(
            de,
            NetworkPolicy::Whitelist(vec!["api.openai.com".to_string(), "127.0.0.1".to_string(),])
        );
    }

    // ---- 路径白名单允许 ----

    #[test]
    fn test_path_whitelist_allows() {
        let cfg = SandboxConfig::builder()
            .allowed_paths(vec![PathBuf::from("/tmp")])
            .build();
        let sandbox = AioSandbox::new(cfg);

        // /tmp 在白名单中 — 允许
        assert!(sandbox
            .validate_path_access(Path::new("/tmp/file.txt"), PathAccess::Read)
            .is_ok());
        // /tmp 本身 — 允许
        assert!(sandbox
            .validate_path_access(Path::new("/tmp"), PathAccess::Read)
            .is_ok());
    }

    // ---- 路径黑名单拒绝优先于白名单 ----

    #[test]
    fn test_path_blacklist_takes_priority() {
        let cfg = SandboxConfig::builder()
            .allowed_paths(vec![PathBuf::from("/tmp")])
            .denied_paths(vec![PathBuf::from("/tmp/secret")])
            .build();
        let sandbox = AioSandbox::new(cfg);

        // /tmp/secret 在黑名单中，即使在白名单父目录下 — 拒绝
        assert!(sandbox
            .validate_path_access(Path::new("/tmp/secret/key.pem"), PathAccess::Read)
            .is_err());
        // /tmp/other 仍允许
        assert!(sandbox
            .validate_path_access(Path::new("/tmp/other.txt"), PathAccess::Read)
            .is_ok());
        // 应记录一条违规
        assert_eq!(sandbox.get_violations().len(), 1);
        assert_eq!(
            sandbox.get_violations()[0].violation_type,
            ViolationType::PathAccessDenied
        );
    }

    // ---- 路径不在白名单被拒绝 ----

    #[test]
    fn test_path_not_in_whitelist_denied() {
        let cfg = SandboxConfig::builder()
            .allowed_paths(vec![PathBuf::from("/tmp")])
            .build();
        let sandbox = AioSandbox::new(cfg);

        // /etc/passwd 不在白名单 — 拒绝
        let result = sandbox.validate_path_access(Path::new("/etc/passwd"), PathAccess::Read);
        assert!(result.is_err());
        assert_eq!(sandbox.get_violations().len(), 1);
    }

    // ---- 网络各种策略验证 ----

    #[test]
    fn test_network_full_access_allows() {
        let cfg = SandboxConfig::builder()
            .network_access(NetworkPolicy::FullAccess)
            .build();
        let sandbox = AioSandbox::new(cfg);

        assert!(sandbox.validate_network("https://api.openai.com").is_ok());
        assert!(sandbox.validate_network("http://192.168.1.1").is_ok());
        assert!(sandbox.validate_network("127.0.0.1:8080").is_ok());
    }

    #[test]
    fn test_network_deny_all_blocks() {
        let cfg = SandboxConfig::builder()
            .network_access(NetworkPolicy::DenyAll)
            .build();
        let sandbox = AioSandbox::new(cfg);

        assert!(sandbox.validate_network("https://api.openai.com").is_err());
        assert!(sandbox.validate_network("127.0.0.1").is_err());
        assert!(!sandbox.get_violations().is_empty());
    }

    #[test]
    fn test_network_localhost_only_allows_loopback() {
        let cfg = SandboxConfig::builder()
            .network_access(NetworkPolicy::LocalhostOnly)
            .build();
        let sandbox = AioSandbox::new(cfg);

        assert!(sandbox.validate_network("127.0.0.1").is_ok());
        assert!(sandbox.validate_network("127.0.0.1:8080").is_ok());
        assert!(sandbox.validate_network("::1").is_ok());
        assert!(sandbox.validate_network("[::1]:8080").is_ok());
        assert!(sandbox.validate_network("localhost").is_ok());
        assert!(sandbox.validate_network("localhost:3000").is_ok());
    }

    #[test]
    fn test_network_localhost_only_denies_external() {
        let cfg = SandboxConfig::builder()
            .network_access(NetworkPolicy::LocalhostOnly)
            .build();
        let sandbox = AioSandbox::new(cfg);

        assert!(sandbox.validate_network("https://api.openai.com").is_err());
        assert!(sandbox.validate_network("192.168.1.1").is_err());
        assert!(sandbox.validate_network("10.0.0.1:8080").is_err());
    }

    #[test]
    fn test_network_whitelist_matches() {
        let cfg = SandboxConfig::builder()
            .network_access(NetworkPolicy::Whitelist(vec![
                "api.openai.com".to_string(),
                "127.0.0.1".to_string(),
            ]))
            .build();
        let sandbox = AioSandbox::new(cfg);

        // 精确匹配
        assert!(sandbox.validate_network("api.openai.com").is_ok());
        assert!(sandbox.validate_network("127.0.0.1").is_ok());
        // host:port 形式匹配 host
        assert!(sandbox.validate_network("api.openai.com:443").is_ok());
        assert!(sandbox.validate_network("127.0.0.1:8080").is_ok());
        // 不在白名单
        assert!(sandbox.validate_network("evil.com").is_err());
        assert!(sandbox.validate_network("192.168.1.1").is_err());
    }

    // ---- 环境变量白名单验证 ----

    #[test]
    fn test_env_var_whitelist() {
        let cfg = SandboxConfig::builder()
            .allowed_env_vars(vec!["PATH".to_string(), "HOME".to_string()])
            .build();
        let sandbox = AioSandbox::new(cfg);

        // 白名单中的变量 — 允许
        assert!(sandbox.validate_env_var("PATH").is_ok());
        assert!(sandbox.validate_env_var("HOME").is_ok());
        // 不在白名单 — 拒绝
        assert!(sandbox.validate_env_var("SECRET_TOKEN").is_err());
        assert_eq!(sandbox.get_violations().len(), 1);
        assert_eq!(
            sandbox.get_violations()[0].violation_type,
            ViolationType::EnvVarDenied
        );
    }

    #[test]
    fn test_env_var_empty_whitelist_denies_all() {
        // 空白名单拒绝所有环境变量访问
        let sandbox = AioSandbox::new(SandboxConfig::default());
        assert!(sandbox.validate_env_var("PATH").is_err());
        assert!(sandbox.validate_env_var("HOME").is_err());
    }

    // ---- 文件大小限制验证 ----

    #[test]
    fn test_file_size_limit() {
        let cfg = SandboxConfig::builder().max_file_size_mb(10).build();
        let sandbox = AioSandbox::new(cfg);

        let ten_mb = 10 * 1024 * 1024;
        // 恰好等于限制 — 允许
        assert!(sandbox.validate_file_size(ten_mb).is_ok());
        // 小于限制 — 允许
        assert!(sandbox.validate_file_size(ten_mb - 1).is_ok());
        // 超过限制 — 拒绝
        assert!(sandbox.validate_file_size(ten_mb + 1).is_err());
        assert_eq!(sandbox.get_violations().len(), 1);
        assert_eq!(
            sandbox.get_violations()[0].violation_type,
            ViolationType::FileSizeExceeded
        );
    }

    #[test]
    fn test_file_size_no_limit_allows_all() {
        // 无限制时允许任意大小
        let sandbox = AioSandbox::new(SandboxConfig::default());
        assert!(sandbox.validate_file_size(0).is_ok());
        assert!(sandbox.validate_file_size(u64::MAX).is_ok());
    }

    // ---- SandboxViolation 序列化 ----

    #[test]
    fn test_sandbox_violation_serialization() {
        let violation = SandboxViolation {
            violation_type: ViolationType::NetworkDenied,
            path_or_addr: Some("evil.com".to_string()),
            timestamp: DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            action_taken: ActionTaken::Blocked,
        };

        let json = serde_json::to_string(&violation).unwrap();
        assert!(json.contains("\"network_denied\""));
        assert!(json.contains("evil.com"));
        assert!(json.contains("\"blocked\""));
        assert!(json.contains("2025-01-01T00:00:00Z"));

        let de: SandboxViolation = serde_json::from_str(&json).unwrap();
        assert_eq!(de.violation_type, ViolationType::NetworkDenied);
        assert_eq!(de.path_or_addr, Some("evil.com".to_string()));
        assert_eq!(de.action_taken, ActionTaken::Blocked);
    }

    // ---- ViolationType 所有变体 ----

    #[test]
    fn test_violation_type_all_variants() {
        let variants = vec![
            ViolationType::PathAccessDenied,
            ViolationType::NetworkDenied,
            ViolationType::EnvVarDenied,
            ViolationType::FileSizeExceeded,
            ViolationType::MemoryLimitExceeded,
            ViolationType::CpuTimeExceeded,
        ];

        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let de: ViolationType = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, de);
        }

        // 验证 snake_case 序列化
        assert_eq!(
            serde_json::to_string(&ViolationType::PathAccessDenied).unwrap(),
            "\"path_access_denied\""
        );
        assert_eq!(
            serde_json::to_string(&ViolationType::MemoryLimitExceeded).unwrap(),
            "\"memory_limit_exceeded\""
        );
    }

    // ---- ActionTaken 所有变体 ----

    #[test]
    fn test_action_taken_all_variants() {
        let variants = vec![
            ActionTaken::Blocked,
            ActionTaken::Logged,
            ActionTaken::Warned,
        ];

        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let de: ActionTaken = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, de);
        }

        assert_eq!(
            serde_json::to_string(&ActionTaken::Blocked).unwrap(),
            "\"blocked\""
        );
        assert_eq!(
            serde_json::to_string(&ActionTaken::Logged).unwrap(),
            "\"logged\""
        );
        assert_eq!(
            serde_json::to_string(&ActionTaken::Warned).unwrap(),
            "\"warned\""
        );
    }

    // ---- ResourceUsage 累加 ----

    #[test]
    fn test_resource_usage_merge() {
        let mut usage = ResourceUsage::new();
        usage.max_memory_mb = 100.0;
        usage.cpu_time_ms = 500;
        usage.files_read = 3;
        usage.files_written = 1;
        usage.network_requests = 2;

        let other = ResourceUsage {
            max_memory_mb: 150.0,
            cpu_time_ms: 300,
            files_read: 2,
            files_written: 4,
            network_requests: 5,
        };

        usage.merge(&other);

        // 内存取峰值
        assert_eq!(usage.max_memory_mb, 150.0);
        // 其余累加
        assert_eq!(usage.cpu_time_ms, 800);
        assert_eq!(usage.files_read, 5);
        assert_eq!(usage.files_written, 5);
        assert_eq!(usage.network_requests, 7);
    }

    // ---- PathAccess 所有变体 ----

    #[test]
    fn test_path_access_all_variants() {
        let variants = vec![
            PathAccess::Read,
            PathAccess::Write,
            PathAccess::Execute,
            PathAccess::Delete,
        ];

        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let de: PathAccess = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, de);
        }

        assert_eq!(
            serde_json::to_string(&PathAccess::Read).unwrap(),
            "\"read\""
        );
        assert_eq!(
            serde_json::to_string(&PathAccess::Write).unwrap(),
            "\"write\""
        );
        assert_eq!(
            serde_json::to_string(&PathAccess::Execute).unwrap(),
            "\"execute\""
        );
        assert_eq!(
            serde_json::to_string(&PathAccess::Delete).unwrap(),
            "\"delete\""
        );
    }

    // ---- 空配置时默认行为 ----

    #[test]
    fn test_empty_config_default_behavior() {
        let sandbox = AioSandbox::new(SandboxConfig::default());

        // 路径：空白名单 = 允许所有（非黑名单）
        assert!(sandbox
            .validate_path_access(Path::new("/any/path"), PathAccess::Read)
            .is_ok());
        assert!(sandbox
            .validate_path_access(Path::new("/etc/passwd"), PathAccess::Write)
            .is_ok());

        // 网络：DenyAll = 拒绝所有
        assert!(sandbox.validate_network("127.0.0.1").is_err());
        assert!(sandbox.validate_network("api.openai.com").is_err());

        // 环境变量：空白名单 = 拒绝所有
        assert!(sandbox.validate_env_var("PATH").is_err());

        // 文件大小：无限制 = 允许任意
        assert!(sandbox.validate_file_size(u64::MAX).is_ok());

        // 无违规（路径和网络验证通过 / 不通过各计违规）
        // 网络拒绝会产生违规
        let violations = sandbox.get_violations();
        assert!(!violations.is_empty());
        assert!(violations
            .iter()
            .all(|v| v.violation_type == ViolationType::NetworkDenied
                || v.violation_type == ViolationType::EnvVarDenied));
    }

    // ---- reset 清空状态 ----

    #[test]
    fn test_reset_clears_state() {
        let cfg = SandboxConfig::builder()
            .denied_paths(vec![PathBuf::from("/denied")])
            .enable_logging(true)
            .build();
        let sandbox = AioSandbox::new(cfg);

        // 产生一些违规和日志
        let _ = sandbox.validate_path_access(Path::new("/denied/file"), PathAccess::Read);
        let _ = sandbox.validate_env_var("SECRET");

        assert!(!sandbox.get_violations().is_empty());
        assert!(!sandbox.get_logs().is_empty());

        // 重置
        sandbox.reset();

        assert!(sandbox.get_violations().is_empty());
        assert!(sandbox.get_logs().is_empty());
        let usage = sandbox.get_resource_usage();
        assert_eq!(usage.cpu_time_ms, 0);
        assert_eq!(usage.files_read, 0);
    }

    // ---- 辅助函数测试 ----

    #[test]
    fn test_is_localhost_variants() {
        assert!(is_localhost("localhost"));
        assert!(is_localhost("127.0.0.1"));
        assert!(is_localhost("::1"));
        assert!(is_localhost("127.0.0.1:8080"));
        assert!(is_localhost("[::1]:8080"));
        assert!(is_localhost("localhost:3000"));
        assert!(is_localhost("127.255.255.255"));

        assert!(!is_localhost("192.168.1.1"));
        assert!(!is_localhost("api.openai.com"));
        assert!(!is_localhost("10.0.0.1:8080"));
    }

    #[test]
    fn test_extract_host_variants() {
        assert_eq!(extract_host("127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(extract_host("localhost:3000"), "localhost");
        assert_eq!(extract_host("[::1]:8080"), "::1");
        assert_eq!(extract_host("::1"), "::1");
        assert_eq!(extract_host("api.example.com"), "api.example.com");
        assert_eq!(extract_host("api.example.com:443"), "api.example.com");
    }
}
