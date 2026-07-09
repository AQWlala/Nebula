//! T-E-S-53: Cron 定时任务引擎。
//!
//! 独立于 `cron_scheduler.rs` 的通用 cron 任务引擎，不与 evolution 逻辑耦合。
//!
//! ## 能力
//!
//! - **cron 表达式调度** — 复用 `crate::cron_expr::CronExpr`（5 字段标准 cron），
//!   通过 `compute_next_run()` 计算下次运行时间。
//! - **任务注册/注销/启用/禁用** — `register_task` / `unregister_task` /
//!   `enable_task` / `disable_task`，重复 `task_id` 注册会被拒绝。
//! - **执行追踪** — 每次执行产生 `CronExecutionRecord`，写入环形历史
//!   （`VecDeque`，每个任务最多保留 100 条），可通过 `list_executions()` 查询。
//! - **失败重试** — `CronTaskDef` 内置 `max_retries` + `retry_delay_secs`；
//!   `RetryPolicy` + `BackoffStrategy`（Exponential/Linear/Fixed）提供退避计算。
//! - **引擎循环** — `start()` 启动定时 tick 循环（到期任务自动执行），
//!   `stop()` 通过 `AtomicBool` 标志停止。
//!
//! ## 设计约束
//!
//! 1. **不与 evolution 耦合** — 不引用 `crate::evolution::*`，仅复用通用工具
//!    `crate::cron_expr::CronExpr`（本身无 feature 门控）。
//! 2. **线程安全** — 任务表与历史均用 `parking_lot::RwLock` 保护。
//! 3. **尽力而为** — 单任务执行失败记 warning，不中断引擎循环。
//! 4. **可停止** — `start()` 返回 `JoinHandle`，`stop()` 置标志位使循环退出。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// 任务类型 / 状态
// ---------------------------------------------------------------------------

/// Cron 任务类型。
///
/// - `OneShot` — 一次性任务（执行一次后不再调度）。
/// - `Recurring` — 基于 cron 表达式的周期任务。
/// - `Interval` — 固定间隔任务（语义上与 `Recurring` 区分，供调用方分支处理）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CronTaskType {
    OneShot,
    Recurring,
    Interval,
}

/// Cron 任务执行状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CronTaskStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
    Disabled,
}

// ---------------------------------------------------------------------------
// 任务定义 / 执行记录
// ---------------------------------------------------------------------------

/// Cron 任务定义。
///
/// 注册时传入，`CronEngine` 内部以 `task_id` 为键存储。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTaskDef {
    /// 任务唯一标识。
    pub task_id: String,
    /// 5 字段 cron 表达式（如 `"0 9 * * 1-5"`）。
    pub cron_expr: String,
    /// 人类可读的任务名称。
    pub task_name: String,
    /// 任务类型。
    pub task_type: CronTaskType,
    /// 要执行的命令（如 `"echo"` / `"ls"`）。
    pub command: String,
    /// 命令参数。
    #[serde(default)]
    pub args: Vec<String>,
    /// 是否启用（默认 `true`）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 最大重试次数（默认 3）。
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// 重试基础延迟-秒（默认 60）。
    #[serde(default = "default_retry_delay_secs")]
    pub retry_delay_secs: u64,
    /// 单次执行超时-秒（`None` 表示不超时）。
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// 上次执行时间（UTC）。
    #[serde(default)]
    pub last_run: Option<DateTime<Utc>>,
    /// 下次预计执行时间（UTC）。
    #[serde(default)]
    pub next_run: Option<DateTime<Utc>>,
    /// 标签（供调用方分组/过滤）。
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Default for CronTaskDef {
    fn default() -> Self {
        Self {
            task_id: String::new(),
            cron_expr: String::new(),
            task_name: String::new(),
            task_type: CronTaskType::OneShot,
            command: String::new(),
            args: Vec::new(),
            enabled: true,
            max_retries: 3,
            retry_delay_secs: 60,
            timeout_secs: None,
            last_run: None,
            next_run: None,
            tags: Vec::new(),
        }
    }
}

/// 单次执行记录。
///
/// 由 `execute_task()` 产生，写入引擎历史（`VecDeque`，每任务最多 100 条）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronExecutionRecord {
    /// 对应的 task_id。
    pub task_id: String,
    /// 执行开始时间（UTC）。
    pub started_at: DateTime<Utc>,
    /// 执行结束时间（UTC，`None` 表示尚未结束）。
    pub finished_at: Option<DateTime<Utc>>,
    /// 执行状态。
    pub status: CronTaskStatus,
    /// 进程退出码（`None` 表示未获得，如超时/启动失败）。
    pub exit_code: Option<i32>,
    /// stdout 输出。
    pub output: String,
    /// 错误信息（失败时设置）。
    pub error: Option<String>,
    /// 本次执行累计的重试次数（0 表示首次即成功）。
    pub retry_count: u32,
}

// ---------------------------------------------------------------------------
// 重试策略
// ---------------------------------------------------------------------------

/// 退避策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    /// 指数退避：delay = base * 2^retry_count。
    Exponential,
    /// 线性退避：delay = base * (retry_count + 1)。
    Linear,
    /// 固定退避：delay = base。
    Fixed,
}

/// 重试策略。
///
/// `calculate_delay(retry_count)` 返回第 `retry_count` 次重试前应等待的秒数。
/// `retry_count = 0` 表示第一次重试（即首次执行失败后的首次重试）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// 最大重试次数。
    pub max_retries: u32,
    /// 退避策略。
    pub backoff: BackoffStrategy,
    /// 基础延迟-秒。
    pub delay_base_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff: BackoffStrategy::Fixed,
            delay_base_secs: 60,
        }
    }
}

impl RetryPolicy {
    /// 计算第 `retry_count` 次重试前的延迟-秒。
    ///
    /// - `Exponential`：`base * 2^retry_count`（如 base=60：60→120→240）。
    /// - `Linear`：`base * (retry_count + 1)`（如 base=60：60→120→180）。
    /// - `Fixed`：`base`（如 base=60：60→60→60）。
    pub fn calculate_delay(&self, retry_count: u32) -> u64 {
        match self.backoff {
            BackoffStrategy::Fixed => self.delay_base_secs,
            BackoffStrategy::Linear => self.delay_base_secs.saturating_mul(retry_count as u64 + 1),
            BackoffStrategy::Exponential => {
                // 限制移位以防溢出（u64 最多移 63 位）。
                let shift = retry_count.min(62);
                self.delay_base_secs.saturating_mul(1u64 << shift)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// 每个任务保留的最大历史记录条数。
const MAX_HISTORY_PER_TASK: usize = 100;

/// Cron 定时任务引擎。
///
/// 通用 cron 任务调度器，独立于 evolution 逻辑。内部用 `parking_lot::RwLock`
/// 保护任务表与执行历史，支持多线程并发读（`list_tasks` / `list_executions`）。
pub struct CronEngine {
    /// 任务表：task_id → CronTaskDef。
    tasks: RwLock<HashMap<String, CronTaskDef>>,
    /// 执行历史（全局 VecDeque，每任务最多保留 100 条，最新的在尾部）。
    history: RwLock<VecDeque<CronExecutionRecord>>,
    /// 引擎运行标志（`start()` 置 true，`stop()` 置 false）。
    running: Arc<AtomicBool>,
}

impl CronEngine {
    /// 构造空引擎。
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            history: RwLock::new(VecDeque::new()),
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 注册任务。
    ///
    /// 重复 `task_id` 返回错误。注册时若 `next_run` 为 `None`，自动计算
    /// 首次运行时间（基于 `cron_expr` 与当前 UTC 时间）。
    pub fn register_task(&self, mut def: CronTaskDef) -> Result<()> {
        let mut tasks = self.tasks.write();
        if tasks.contains_key(&def.task_id) {
            bail!("task already registered: {}", def.task_id);
        }
        // 若未显式设置 next_run，计算首次运行时间。
        if def.next_run.is_none() {
            if let Ok(next) = self.compute_next_run(&def.cron_expr, Utc::now()) {
                def.next_run = Some(next);
            }
            // 解析失败时 next_run 保持 None，任务不会被 tick 触发（直到修复 cron_expr）。
        }
        tasks.insert(def.task_id.clone(), def);
        Ok(())
    }

    /// 注销任务。
    ///
    /// 任务不存在返回错误。历史记录保留（可供审计）。
    pub fn unregister_task(&self, task_id: &str) -> Result<()> {
        let mut tasks = self.tasks.write();
        if tasks.remove(task_id).is_none() {
            bail!("task not found: {task_id}");
        }
        Ok(())
    }

    /// 启用任务。
    pub fn enable_task(&self, task_id: &str) -> Result<()> {
        let mut tasks = self.tasks.write();
        let t = tasks
            .get_mut(task_id)
            .ok_or_else(|| anyhow!("task not found: {task_id}"))?;
        t.enabled = true;
        Ok(())
    }

    /// 禁用任务。
    pub fn disable_task(&self, task_id: &str) -> Result<()> {
        let mut tasks = self.tasks.write();
        let t = tasks
            .get_mut(task_id)
            .ok_or_else(|| anyhow!("task not found: {task_id}"))?;
        t.enabled = false;
        Ok(())
    }

    /// 列出所有任务（快照）。
    pub fn list_tasks(&self) -> Vec<CronTaskDef> {
        self.tasks.read().values().cloned().collect()
    }

    /// 查询指定任务的执行历史（最新在前，最多 `limit` 条）。
    pub fn list_executions(&self, task_id: &str, limit: usize) -> Vec<CronExecutionRecord> {
        let hist = self.history.read();
        hist.iter()
            .rev()
            .filter(|r| r.task_id == task_id)
            .take(limit)
            .cloned()
            .collect()
    }

    /// 计算 cron 表达式在 `from` 之后的下一次匹配时间。
    ///
    /// 返回严格大于 `from` 的第一个匹配分钟（秒/纳秒归零）。
    /// 复用 `crate::cron_expr::CronExpr` 进行解析与匹配。
    /// 上限为一年（约 52.7 万分钟），超出则返回错误（防止无效表达式死循环）。
    pub fn compute_next_run(&self, cron_expr: &str, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let expr = crate::cron_expr::CronExpr::parse(cron_expr)?;
        // 截断到分钟（秒/纳秒归零）。
        let mut t = from
            .with_second(0)
            .ok_or_else(|| anyhow!("truncate second failed"))?
            .with_nanosecond(0)
            .ok_or_else(|| anyhow!("truncate nanosecond failed"))?;
        // 确保严格大于 from（避免立即重复触发）。
        if t <= from {
            t = t + chrono::Duration::minutes(1);
        }
        // 上限：一年（防止无效表达式导致死循环）。
        let max_iters = 366 * 24 * 60;
        for _ in 0..max_iters {
            if expr.matches(t) {
                return Ok(t);
            }
            t = t + chrono::Duration::minutes(1);
        }
        bail!("no matching time found within one year for cron expr: '{cron_expr}'")
    }

    /// 返回当前时刻需要执行的任务 ID 列表。
    ///
    /// 条件：`enabled == true` 且 `next_run <= now`。
    /// 不修改任务状态（执行与 next_run 更新由 `execute_task` 完成）。
    pub fn tick(&self, now: DateTime<Utc>) -> Vec<String> {
        let tasks = self.tasks.read();
        tasks
            .iter()
            .filter(|(_, t)| t.enabled && t.next_run.map_or(false, |nr| nr <= now))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// 执行单个任务（含重试）。
    ///
    /// 用 `tokio::process::Command` 执行 `def.command` + `def.args`，
    /// 若 `timeout_secs` 设置则带超时。失败时按 `max_retries` + `retry_delay_secs`
    /// 重试，最终返回一条 `CronExecutionRecord`（`retry_count` 记录重试次数）。
    ///
    /// 返回值：
    /// - `Ok(record)` — 任务已执行（成功或失败，看 `record.status`）。
    /// - `Err(_)` — 操作性错误（任务不存在等）。
    pub async fn execute_task(&self, task_id: &str) -> Result<CronExecutionRecord> {
        let def = {
            let tasks = self.tasks.read();
            tasks
                .get(task_id)
                .cloned()
                .ok_or_else(|| anyhow!("task not found: {task_id}"))?
        };

        let started_at = Utc::now();
        let mut retry_count: u32 = 0;
        let mut last_error: Option<String> = None;

        loop {
            match self.run_command_once(&def).await {
                Ok(output) => {
                    // 执行成功。
                    let finished_at = Utc::now();
                    let exit_code = output.status.code();
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    // 退出码非 0 视为失败。
                    let success = output.status.success();
                    let record = CronExecutionRecord {
                        task_id: def.task_id.clone(),
                        started_at,
                        finished_at: Some(finished_at),
                        status: if success {
                            CronTaskStatus::Success
                        } else {
                            CronTaskStatus::Failed
                        },
                        exit_code,
                        output: if stdout.is_empty() { stderr } else { stdout },
                        error: if success {
                            None
                        } else {
                            Some(format!("exit code {:?}", exit_code))
                        },
                        retry_count,
                    };
                    self.finalize_execution(&def, &record);
                    return Ok(record);
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    warn!(
                        target: "nebula.cron_engine",
                        task_id = %def.task_id,
                        retry_count,
                        error = %err_msg,
                        "command execution failed"
                    );
                    last_error = Some(err_msg);
                    if retry_count < def.max_retries {
                        // 等待重试延迟后重试。
                        let delay = def.retry_delay_secs;
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        retry_count += 1;
                    } else {
                        break;
                    }
                }
            }
        }

        // 所有重试均失败。
        let finished_at = Utc::now();
        let record = CronExecutionRecord {
            task_id: def.task_id.clone(),
            started_at,
            finished_at: Some(finished_at),
            status: CronTaskStatus::Failed,
            exit_code: None,
            output: String::new(),
            error: last_error,
            retry_count,
        };
        self.finalize_execution(&def, &record);
        Ok(record)
    }

    /// 执行单次命令（不含重试）。
    async fn run_command_once(&self, def: &CronTaskDef) -> Result<std::process::Output> {
        let mut cmd = Command::new(&def.command);
        cmd.args(&def.args);
        if let Some(secs) = def.timeout_secs {
            let output = tokio::time::timeout(Duration::from_secs(secs), cmd.output())
                .await
                .map_err(|_| anyhow!("task timeout after {secs}s"))??;
            Ok(output)
        } else {
            let output = cmd.output().await?;
            Ok(output)
        }
    }

    /// 执行收尾：推入历史 + 更新任务 last_run/next_run。
    fn finalize_execution(&self, def: &CronTaskDef, record: &CronExecutionRecord) {
        // 推入历史（带每任务 100 条上限修剪）。
        self.push_history(record.clone());

        // 更新任务的 last_run + next_run。
        let now = record.finished_at.unwrap_or_else(Utc::now);
        let mut tasks = self.tasks.write();
        if let Some(t) = tasks.get_mut(&def.task_id) {
            t.last_run = Some(now);
            // OneShot 任务执行后不再调度。
            if t.task_type == CronTaskType::OneShot {
                t.next_run = None;
                t.enabled = false;
            } else {
                // 重新计算下次运行时间。
                match self.compute_next_run(&t.cron_expr, now) {
                    Ok(next) => t.next_run = Some(next),
                    Err(e) => {
                        warn!(
                            target: "nebula.cron_engine",
                            task_id = %t.task_id,
                            error = %e,
                            "compute_next_run failed; next_run cleared"
                        );
                        t.next_run = None;
                    }
                }
            }
        }
    }

    /// 推入执行历史，并确保该任务不超过 `MAX_HISTORY_PER_TASK` 条。
    fn push_history(&self, record: CronExecutionRecord) {
        let mut hist = self.history.write();
        hist.push_back(record.clone());
        // 修剪：若该 task_id 超过上限，移除最旧的一条。
        let count = hist.iter().filter(|r| r.task_id == record.task_id).count();
        if count > MAX_HISTORY_PER_TASK {
            // 找到最旧的该 task_id 记录索引并移除。
            if let Some(idx) = hist.iter().position(|r| r.task_id == record.task_id) {
                hist.remove(idx);
            }
        }
    }

    /// 启动引擎循环。
    ///
    /// 每 `interval` 触发一次 `tick()`，对到期任务调用 `execute_task()`。
    /// 返回 `JoinHandle`，调用方可 await 或忽略。`stop()` 使循环退出。
    pub fn start(self: Arc<Self>, interval: Duration) -> JoinHandle<()> {
        self.running.store(true, Ordering::SeqCst);
        let engine = Arc::clone(&self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            info!(target: "nebula.cron_engine", "cron engine loop started");
            loop {
                ticker.tick().await;
                if !engine.running.load(Ordering::SeqCst) {
                    info!(target: "nebula.cron_engine", "cron engine loop stopped");
                    break;
                }
                let now = Utc::now();
                let due = engine.tick(now);
                for task_id in due {
                    match engine.execute_task(&task_id).await {
                        Ok(record) => {
                            if record.status == CronTaskStatus::Failed {
                                warn!(
                                    target: "nebula.cron_engine",
                                    task_id = %task_id,
                                    retry_count = record.retry_count,
                                    "task execution failed after retries"
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                target: "nebula.cron_engine",
                                task_id = %task_id,
                                error = %e,
                                "execute_task error"
                            );
                        }
                    }
                }
            }
        })
    }

    /// 停止引擎循环（置 `AtomicBool` 标志，下一次 tick 时退出）。
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        info!(target: "nebula.cron_engine", "cron engine stop requested");
    }
}

impl Default for CronEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// serde 默认值辅助函数
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}

fn default_max_retries() -> u32 {
    3
}

fn default_retry_delay_secs() -> u64 {
    60
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 构造一个基础任务定义（enabled=true，cron_expr=`* * * * *`）。
    fn make_def(task_id: &str) -> CronTaskDef {
        CronTaskDef {
            task_id: task_id.to_string(),
            cron_expr: "* * * * *".to_string(),
            task_name: format!("test-{task_id}"),
            task_type: CronTaskType::Recurring,
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            enabled: true,
            max_retries: 3,
            retry_delay_secs: 60,
            timeout_secs: None,
            last_run: None,
            next_run: None,
            tags: vec![],
        }
    }

    // ---- 任务注册 / 注销 ----

    #[test]
    fn register_task_adds_to_list() {
        // 任务注册后应出现在 list_tasks 中。
        let engine = CronEngine::new();
        assert!(engine.list_tasks().is_empty());
        engine.register_task(make_def("t1")).expect("register ok");
        let tasks = engine.list_tasks();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, "t1");
    }

    #[test]
    fn register_duplicate_task_fails() {
        // 重复 task_id 注册应返回错误。
        let engine = CronEngine::new();
        engine
            .register_task(make_def("dup"))
            .expect("first register ok");
        let err = engine
            .register_task(make_def("dup"))
            .expect_err("duplicate should fail");
        assert!(format!("{err}").contains("already registered"));
    }

    #[test]
    fn unregister_task_removes_from_list() {
        // 注销后任务不再出现在列表中。
        let engine = CronEngine::new();
        engine.register_task(make_def("t1")).expect("register ok");
        assert_eq!(engine.list_tasks().len(), 1);
        engine.unregister_task("t1").expect("unregister ok");
        assert!(engine.list_tasks().is_empty());
    }

    #[test]
    fn unregister_unknown_task_fails() {
        // 注销不存在的任务返回错误。
        let engine = CronEngine::new();
        let err = engine
            .unregister_task("nonexistent")
            .expect_err("should fail");
        assert!(format!("{err}").contains("not found"));
    }

    // ---- enable / disable ----

    #[test]
    fn enable_task_sets_enabled_true() {
        let engine = CronEngine::new();
        let mut def = make_def("t1");
        def.enabled = false;
        engine.register_task(def).expect("register ok");
        // 初始为 disabled。
        assert!(!engine.list_tasks()[0].enabled);
        engine.enable_task("t1").expect("enable ok");
        assert!(engine.list_tasks()[0].enabled);
    }

    #[test]
    fn disable_task_sets_enabled_false() {
        let engine = CronEngine::new();
        engine.register_task(make_def("t1")).expect("register ok");
        assert!(engine.list_tasks()[0].enabled);
        engine.disable_task("t1").expect("disable ok");
        assert!(!engine.list_tasks()[0].enabled);
    }

    #[test]
    fn disable_unknown_task_fails() {
        let engine = CronEngine::new();
        assert!(engine.disable_task("nope").is_err());
    }

    // ---- compute_next_run ----

    #[test]
    fn compute_next_run_every_minute() {
        // "* * * * *" — 每分钟。from=12:00:00 → next=12:01:00。
        let engine = CronEngine::new();
        let from = Utc
            .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
            .single()
            .unwrap();
        let next = engine
            .compute_next_run("* * * * *", from)
            .expect("compute ok");
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 1, 0)
                .single()
                .unwrap()
        );
    }

    #[test]
    fn compute_next_run_every_hour() {
        // "0 * * * *" — 每小时整点。from=12:30:00 → next=13:00:00。
        let engine = CronEngine::new();
        let from = Utc
            .with_ymd_and_hms(2026, 7, 10, 12, 30, 0)
            .single()
            .unwrap();
        let next = engine
            .compute_next_run("0 * * * *", from)
            .expect("compute ok");
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 7, 10, 13, 0, 0)
                .single()
                .unwrap()
        );
    }

    #[test]
    fn compute_next_run_daily_330() {
        // "30 3 * * *" — 每天 03:30。from=2026-07-10 12:00 → next=2026-07-11 03:30。
        let engine = CronEngine::new();
        let from = Utc
            .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
            .single()
            .unwrap();
        let next = engine
            .compute_next_run("30 3 * * *", from)
            .expect("compute ok");
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 7, 11, 3, 30, 0)
                .single()
                .unwrap()
        );
    }

    #[test]
    fn compute_next_run_invalid_expr_fails() {
        let engine = CronEngine::new();
        let from = Utc::now();
        assert!(engine.compute_next_run("not a cron", from).is_err());
    }

    // ---- tick ----

    #[test]
    fn tick_returns_due_tasks() {
        // next_run <= now 且 enabled → tick 返回该 task_id。
        let engine = CronEngine::new();
        let past = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap();
        let mut def = make_def("due");
        def.next_run = Some(past);
        engine.register_task(def).expect("register ok");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
            .single()
            .unwrap();
        let due = engine.tick(now);
        assert_eq!(due, vec!["due".to_string()]);
    }

    #[test]
    fn tick_skips_disabled_tasks() {
        // disabled 任务即使 next_run <= now 也不触发。
        let engine = CronEngine::new();
        let past = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap();
        let mut def = make_def("disabled");
        def.next_run = Some(past);
        def.enabled = false;
        engine.register_task(def).expect("register ok");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
            .single()
            .unwrap();
        let due = engine.tick(now);
        assert!(due.is_empty(), "disabled task should not trigger tick");
    }

    #[test]
    fn tick_skips_future_tasks() {
        // next_run > now 的任务不触发。
        let engine = CronEngine::new();
        let future = Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).single().unwrap();
        let mut def = make_def("future");
        def.next_run = Some(future);
        engine.register_task(def).expect("register ok");
        let now = Utc
            .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
            .single()
            .unwrap();
        let due = engine.tick(now);
        assert!(due.is_empty(), "future task should not trigger tick");
    }

    // ---- RetryPolicy 退避策略 ----

    #[test]
    fn retry_policy_exponential() {
        // 指数退避：base=60 → 60, 120, 240。
        let policy = RetryPolicy {
            max_retries: 5,
            backoff: BackoffStrategy::Exponential,
            delay_base_secs: 60,
        };
        assert_eq!(policy.calculate_delay(0), 60);
        assert_eq!(policy.calculate_delay(1), 120);
        assert_eq!(policy.calculate_delay(2), 240);
    }

    #[test]
    fn retry_policy_linear() {
        // 线性退避：base=60 → 60, 120, 180。
        let policy = RetryPolicy {
            max_retries: 5,
            backoff: BackoffStrategy::Linear,
            delay_base_secs: 60,
        };
        assert_eq!(policy.calculate_delay(0), 60);
        assert_eq!(policy.calculate_delay(1), 120);
        assert_eq!(policy.calculate_delay(2), 180);
    }

    #[test]
    fn retry_policy_fixed() {
        // 固定退避：base=60 → 60, 60, 60。
        let policy = RetryPolicy {
            max_retries: 5,
            backoff: BackoffStrategy::Fixed,
            delay_base_secs: 60,
        };
        assert_eq!(policy.calculate_delay(0), 60);
        assert_eq!(policy.calculate_delay(1), 60);
        assert_eq!(policy.calculate_delay(2), 60);
    }

    #[test]
    fn retry_policy_default_is_fixed() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff, BackoffStrategy::Fixed);
        assert_eq!(policy.max_retries, 3);
        assert_eq!(policy.delay_base_secs, 60);
    }

    // ---- execution record 序列化 ----

    #[test]
    fn execution_record_serde_roundtrip() {
        let record = CronExecutionRecord {
            task_id: "t1".to_string(),
            started_at: Utc
                .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
                .single()
                .unwrap(),
            finished_at: Some(
                Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 5)
                    .single()
                    .unwrap(),
            ),
            status: CronTaskStatus::Success,
            exit_code: Some(0),
            output: "done".to_string(),
            error: None,
            retry_count: 2,
        };
        let json = serde_json::to_string(&record).expect("serialize ok");
        let back: CronExecutionRecord = serde_json::from_str(&json).expect("deserialize ok");
        assert_eq!(back.task_id, "t1");
        assert_eq!(back.status, CronTaskStatus::Success);
        assert_eq!(back.exit_code, Some(0));
        assert_eq!(back.output, "done");
        assert_eq!(back.retry_count, 2);
    }

    #[test]
    fn cron_task_status_serde_snake_case() {
        // 验证状态枚举序列化为 snake_case。
        assert_eq!(
            serde_json::to_string(&CronTaskStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&CronTaskStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&CronTaskStatus::Success).unwrap(),
            "\"success\""
        );
        assert_eq!(
            serde_json::to_string(&CronTaskStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&CronTaskStatus::Skipped).unwrap(),
            "\"skipped\""
        );
        assert_eq!(
            serde_json::to_string(&CronTaskStatus::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    // ---- 任务列表 / 并发注册（非多线程）----

    #[test]
    fn list_tasks_returns_all() {
        let engine = CronEngine::new();
        engine.register_task(make_def("t1")).expect("ok");
        engine.register_task(make_def("t2")).expect("ok");
        engine.register_task(make_def("t3")).expect("ok");
        let mut ids: Vec<String> = engine
            .list_tasks()
            .iter()
            .map(|t| t.task_id.clone())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn register_multiple_tasks_sequentially() {
        // 连续注册多个不同 task_id 的任务（非多线程并发，但验证批量注册）。
        let engine = CronEngine::new();
        for i in 0..10 {
            engine
                .register_task(make_def(&format!("task-{i}")))
                .expect("register ok");
        }
        assert_eq!(engine.list_tasks().len(), 10);
    }

    // ---- CronTaskType 各变体 ----

    #[test]
    fn cron_task_type_variants_serde() {
        // 验证三个变体的 snake_case 序列化与反序列化。
        let cases = vec![
            (CronTaskType::OneShot, "\"one_shot\""),
            (CronTaskType::Recurring, "\"recurring\""),
            (CronTaskType::Interval, "\"interval\""),
        ];
        for (variant, expected_json) in cases {
            let json = serde_json::to_string(&variant).expect("serialize ok");
            assert_eq!(json, expected_json);
            let back: CronTaskType = serde_json::from_str(&json).expect("deserialize ok");
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn cron_task_def_default_enabled_true() {
        // Default 的 enabled 应为 true（非 bool 默认的 false）。
        let def = CronTaskDef::default();
        assert!(def.enabled);
        assert_eq!(def.max_retries, 3);
        assert_eq!(def.retry_delay_secs, 60);
        assert_eq!(def.task_type, CronTaskType::OneShot);
    }

    #[test]
    fn cron_task_def_serde_with_defaults() {
        // 序列化时省略有默认值的字段，反序列化后应恢复默认值。
        let json = r#"{
            "task_id": "t1",
            "cron_expr": "* * * * *",
            "task_name": "test",
            "task_type": "recurring",
            "command": "echo"
        }"#;
        let def: CronTaskDef = serde_json::from_str(json).expect("deserialize ok");
        assert_eq!(def.task_id, "t1");
        assert_eq!(def.task_type, CronTaskType::Recurring);
        assert!(def.enabled, "enabled should default to true");
        assert_eq!(def.max_retries, 3, "max_retries should default to 3");
        assert_eq!(
            def.retry_delay_secs, 60,
            "retry_delay_secs should default to 60"
        );
        assert!(def.args.is_empty());
        assert!(def.tags.is_empty());
        assert!(def.timeout_secs.is_none());
    }
}
