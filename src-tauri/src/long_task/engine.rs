//! T-E-C-10: 异步长任务引擎。
//!
//! 用户描述目标后,后台分步执行(跨小时/跨天),与 PlanEngine 联动
//! (可选)并在 Shadow Workspace 中隔离执行(可选)。状态持久化到
//! SQLite,进程重启后可恢复。
//!
//! 生命周期:
//!   Pending → Running ⇄ Paused → Completed | Failed | Cancelled
//!
//! 关键设计:
//! - SQLite 持久化(long_tasks + long_task_steps 表),每步执行后更新状态
//! - tokio::spawn 后台执行步骤序列,每步用 spawn_blocking 调用
//!   `shadow_engine.run_command(workspace_id, program, args)`
//! - AtomicBool 暂停/取消信号(协同式中断,当前步完成后生效)
//! - 重启恢复:bootstrap 时加载所有任务,Running 状态重置为 Paused
//!   (用户手动 resume 继续)
//!
//! 与 PlanEngine 的联动:
//! - PlanEngine 是同步审批流(Pending→Approved→Executing→Done)
//! - LongTaskEngine 是异步执行引擎,可携带 plan_id 软引用
//! - PlanEngine.approve() 后可调用 LongTaskEngine.start() 启动后台执行
//!
//! 与 Shadow Workspace 的联动:
//! - 每个长任务可关联一个 shadow workspace(workspace_id)
//! - 步骤命令在 shadow workspace 内执行,与用户工作区隔离
//! - 任务完成后用户可审查 diff + 录屏回放(复用 T-E-C-08/09 基础设施)

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use parking_lot::RwLock;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::{error, info, instrument, warn};
use uuid::Uuid;

use crate::memory::sqlite_store::SqliteStore;
use crate::shadow_workspace::ShadowWorkspaceEngine;

/// 长任务状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LongTaskStatus {
    /// 已创建,等待启动。
    Pending,
    /// 后台执行中。
    Running,
    /// 已暂停(用户暂停或进程重启后)。
    Paused,
    /// 全部步骤成功完成。
    Completed,
    /// 某步执行失败。
    Failed,
    /// 用户取消。
    Cancelled,
}

impl LongTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LongTaskStatus::Pending => "pending",
            LongTaskStatus::Running => "running",
            LongTaskStatus::Paused => "paused",
            LongTaskStatus::Completed => "completed",
            LongTaskStatus::Failed => "failed",
            LongTaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(LongTaskStatus::Pending),
            "running" => Ok(LongTaskStatus::Running),
            "paused" => Ok(LongTaskStatus::Paused),
            "completed" => Ok(LongTaskStatus::Completed),
            "failed" => Ok(LongTaskStatus::Failed),
            "cancelled" => Ok(LongTaskStatus::Cancelled),
            other => Err(anyhow!("unknown long task status: {other}")),
        }
    }

    /// 是否处于终态(不可再变更)。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            LongTaskStatus::Completed | LongTaskStatus::Failed | LongTaskStatus::Cancelled
        )
    }
}

/// 步骤状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    /// 未开始。
    Pending,
    /// 执行中。
    Running,
    /// 成功完成。
    Done,
    /// 执行失败。
    Failed,
    /// 已跳过(取消后剩余步骤)。
    Skipped,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Done => "done",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(StepStatus::Pending),
            "running" => Ok(StepStatus::Running),
            "done" => Ok(StepStatus::Done),
            "failed" => Ok(StepStatus::Failed),
            "skipped" => Ok(StepStatus::Skipped),
            other => Err(anyhow!("unknown step status: {other}")),
        }
    }
}

/// 长任务主记录(序列化给前端)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongTask {
    pub id: String,
    pub goal: String,
    pub status: LongTaskStatus,
    /// 关联的 Shadow Workspace ID(可选)。
    pub workspace_id: Option<String>,
    /// 关联的 PlanEngine 请求 ID(可选)。
    pub plan_id: Option<String>,
    /// 0-100 完成百分比。
    pub progress: i32,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// 长任务步骤(序列化给前端)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongTaskStep {
    pub task_id: String,
    pub seq: u32,
    pub description: String,
    pub program: String,
    /// 命令参数(JSON 数组)。
    pub args: Vec<String>,
    pub status: StepStatus,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    /// 截断后的输出(stdout+stderr)。
    pub output: Option<String>,
    pub error: Option<String>,
}

/// 创建任务时的步骤输入(由前端传入)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepInput {
    pub description: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// 长任务管理引擎。
///
/// 持有:
/// - `sqlite: Arc<SqliteStore>` — 持久化任务/步骤
/// - `shadow_engine: Arc<ShadowWorkspaceEngine>` — 步骤命令执行的隔离环境
/// - `runners: RwLock<HashMap<id, JoinHandle<()>>>` — 后台 tokio 任务句柄
/// - `pause_flags: RwLock<HashMap<id, Arc<AtomicBool>>>` — 暂停信号
/// - `cancel_flags: RwLock<HashMap<id, Arc<AtomicBool>>>` — 取消信号
pub struct LongTaskEngine {
    sqlite: Arc<SqliteStore>,
    shadow_engine: Arc<ShadowWorkspaceEngine>,
    runners: RwLock<HashMap<String, JoinHandle<()>>>,
    pause_flags: RwLock<HashMap<String, Arc<AtomicBool>>>,
    cancel_flags: RwLock<HashMap<String, Arc<AtomicBool>>>,
}

impl LongTaskEngine {
    pub fn new(sqlite: Arc<SqliteStore>, shadow_engine: Arc<ShadowWorkspaceEngine>) -> Self {
        Self {
            sqlite,
            shadow_engine,
            runners: RwLock::new(HashMap::new()),
            pause_flags: RwLock::new(HashMap::new()),
            cancel_flags: RwLock::new(HashMap::new()),
        }
    }

    /// 创建新任务(状态 = Pending)。
    ///
    /// 步骤序列号从 1 开始递增。若 workspace_id 为 None 且步骤非空,
    /// 调用方应在 start() 前显式创建 Shadow Workspace 并传入。
    #[instrument(skip(self, steps))]
    pub fn create_task(
        &self,
        goal: String,
        steps: Vec<StepInput>,
        workspace_id: Option<String>,
        plan_id: Option<String>,
    ) -> Result<LongTask> {
        if goal.trim().is_empty() {
            return Err(anyhow!("task goal must not be empty"));
        }
        if steps.is_empty() {
            return Err(anyhow!("task must have at least one step"));
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        {
            let conn = self.sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute(
                "INSERT INTO long_tasks (id, goal, status, workspace_id, plan_id, progress, created_at, updated_at) \
                 VALUES (?1, ?2, 'pending', ?3, ?4, 0, ?5, ?5)",
                params![id, goal, workspace_id, plan_id, now],
            )
            .context("inserting long task")?;
            for (i, step) in steps.iter().enumerate() {
                let seq = (i + 1) as u32;
                let args_json = serde_json::to_string(&step.args).unwrap_or_else(|_| "[]".into());
                conn.execute(
                    "INSERT INTO long_task_steps (task_id, seq, description, program, args_json, status) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                    params![id, seq, step.description, step.program, args_json],
                )
                .context(format!("inserting step {seq}"))?;
            }
        }
        info!(target: "nebula.long_task", id = %id, steps = steps.len(), "long task created");
        self.get_task(&id)?
            .ok_or_else(|| anyhow!("task vanished after create: {id}"))
    }

    /// 获取任务。
    pub fn get_task(&self, id: &str) -> Result<Option<LongTask>> {
        let conn = self.sqlite.raw_connection();
        let conn = conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, goal, status, workspace_id, plan_id, progress, error, \
                    created_at, updated_at, started_at, finished_at \
             FROM long_tasks WHERE id = ?1",
        )?;
        // 用元组收集字段,避免返回 rusqlite::Row 引用(生命周期问题)。
        let row = stmt.query_row(params![id], |r| {
            let status_str: String = r.get(2)?;
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                status_str,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i32>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<i64>>(10)?,
            ))
        });
        match row {
            Ok((
                id,
                goal,
                status_str,
                ws,
                plan,
                progress,
                error,
                created,
                updated,
                started,
                finished,
            )) => {
                let status = LongTaskStatus::from_str(&status_str)?;
                Ok(Some(LongTask {
                    id,
                    goal,
                    status,
                    workspace_id: ws,
                    plan_id: plan,
                    progress,
                    error,
                    created_at: created,
                    updated_at: updated,
                    started_at: started,
                    finished_at: finished,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 获取任务的所有步骤(按 seq 升序)。
    pub fn get_steps(&self, id: &str) -> Result<Vec<LongTaskStep>> {
        let conn = self.sqlite.raw_connection();
        let conn = conn.lock();
        let mut stmt = conn.prepare(
            "SELECT task_id, seq, description, program, args_json, status, \
                    started_at, finished_at, exit_code, output, error \
             FROM long_task_steps WHERE task_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            let status_str: String = r.get(5)?;
            let args_json: String = r.get(4)?;
            let args: Vec<String> = serde_json::from_str(&args_json).unwrap_or_default();
            let status = StepStatus::from_str(&status_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
            })?;
            Ok(LongTaskStep {
                task_id: r.get(0)?,
                seq: r.get(1)?,
                description: r.get(2)?,
                program: r.get(3)?,
                args,
                status,
                started_at: r.get(6)?,
                finished_at: r.get(7)?,
                exit_code: r.get(8)?,
                output: r.get(9)?,
                error: r.get(10)?,
            })
        })?;
        let mut steps = Vec::new();
        for row in rows {
            steps.push(row?);
        }
        Ok(steps)
    }

    /// 列出所有任务(按创建时间降序),可按状态过滤。
    pub fn list_tasks(&self, status: Option<LongTaskStatus>) -> Result<Vec<LongTask>> {
        let conn = self.sqlite.raw_connection();
        let conn = conn.lock();
        let mut stmt = if let Some(s) = status {
            let mut stmt = conn.prepare(
                "SELECT id, goal, status, workspace_id, plan_id, progress, error, \
                        created_at, updated_at, started_at, finished_at \
                 FROM long_tasks WHERE status = ?1 ORDER BY created_at DESC, rowid DESC",
            )?;
            let rows = stmt.query_map(params![s.as_str()], row_to_task)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            return Ok(out);
        } else {
            conn.prepare(
                "SELECT id, goal, status, workspace_id, plan_id, progress, error, \
                        created_at, updated_at, started_at, finished_at \
                 FROM long_tasks ORDER BY created_at DESC, rowid DESC",
            )?
        };
        let rows = stmt.query_map([], row_to_task)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// 启动任务(Pending 或 Paused → Running)。
    ///
    /// 重新 spawn 后台 runner,从第一个 Pending 步骤继续。
    /// 若任务已 Running,返回错误。
    #[instrument(skip(self))]
    pub fn start(&self, id: &str) -> Result<LongTask> {
        let task = self
            .get_task(id)?
            .ok_or_else(|| anyhow!("task {id} not found"))?;
        match task.status {
            LongTaskStatus::Pending | LongTaskStatus::Paused => {}
            LongTaskStatus::Running => return Err(anyhow!("task {id} already running")),
            LongTaskStatus::Completed => return Err(anyhow!("task {id} already completed")),
            LongTaskStatus::Failed => return Err(anyhow!("task {id} failed, cannot start")),
            LongTaskStatus::Cancelled => return Err(anyhow!("task {id} cancelled, cannot start")),
        }
        // 设置 Running 状态
        let now = Utc::now().timestamp();
        let started_at = task.started_at.or(Some(now));
        {
            let conn = self.sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute(
                "UPDATE long_tasks SET status = 'running', updated_at = ?1, started_at = ?2 WHERE id = ?3",
                params![now, started_at, id],
            )?;
        }
        // 创建暂停/取消标志(若不存在)
        let pause_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.pause_flags
            .write()
            .insert(id.to_string(), pause_flag.clone());
        self.cancel_flags
            .write()
            .insert(id.to_string(), cancel_flag.clone());

        // spawn 后台 runner
        let runner = self.spawn_runner(id.to_string(), pause_flag, cancel_flag);
        self.runners.write().insert(id.to_string(), runner);

        self.get_task(id)?
            .ok_or_else(|| anyhow!("task vanished after start: {id}"))
    }

    /// 暂停任务(Running → Paused)。协同式:当前步骤完成后停止。
    pub fn pause(&self, id: &str) -> Result<LongTask> {
        let task = self
            .get_task(id)?
            .ok_or_else(|| anyhow!("task {id} not found"))?;
        if task.status != LongTaskStatus::Running {
            return Err(anyhow!(
                "task {id} not running (status={}), cannot pause",
                task.status.as_str()
            ));
        }
        // 设置暂停标志(runner 检查后退出)
        if let Some(flag) = self.pause_flags.read().get(id) {
            flag.store(true, Ordering::SeqCst);
        }
        // 立即更新 DB 状态(runner 退出时也会再次更新)
        let now = Utc::now().timestamp();
        {
            let conn = self.sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute(
                "UPDATE long_tasks SET status = 'paused', updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        // 等待 runner 退出(它会在下次检查时退出并从 runners map 移除)
        // 这里不主动 abort,让 runner 协同式退出
        self.get_task(id)?
            .ok_or_else(|| anyhow!("task vanished after pause: {id}"))
    }

    /// 恢复任务(Paused → Running)。等价于 start()。
    pub fn resume(&self, id: &str) -> Result<LongTask> {
        let task = self
            .get_task(id)?
            .ok_or_else(|| anyhow!("task {id} not found"))?;
        if task.status != LongTaskStatus::Paused {
            return Err(anyhow!(
                "task {id} not paused (status={}), cannot resume",
                task.status.as_str()
            ));
        }
        // 清除暂停标志(以防残留)
        if let Some(flag) = self.pause_flags.read().get(id) {
            flag.store(false, Ordering::SeqCst);
        }
        self.start(id)
    }

    /// T-E-L-06: 批量暂停所有运行中任务。
    ///
    /// 遍历所有 status=Running 的任务,逐个设置 pause_flag=true 并更新
    /// DB 状态为 Paused。已暂停/已完成/已失败/已取消的任务自动跳过
    /// (不在 Running 列表中)。
    ///
    /// 返回被暂停的 task_id 列表(按 created_at 升序,最早的在前)。
    ///
    /// 使用场景:月度预算超限 100% 时,批量暂停所有 Loop。
    pub fn pause_all(&self) -> Vec<String> {
        // 1. 取出所有 Running 任务快照(避免长时间持锁)
        let running = self.list_running();
        if running.is_empty() {
            return Vec::new();
        }
        // 2. 逐个设置 pause_flag + 更新 DB 状态(参考 pause(id) 实现)
        let now = Utc::now().timestamp();
        let mut paused_ids = Vec::with_capacity(running.len());
        for task in &running {
            // 设置暂停标志(若存在,runner 才能感知并退出)
            if let Some(flag) = self.pause_flags.read().get(&task.id) {
                flag.store(true, Ordering::SeqCst);
            }
            // 立即更新 DB 状态(WHERE status='running' 防止并发状态变更导致重复暂停)
            let conn = self.sqlite.raw_connection();
            let conn = conn.lock();
            match conn.execute(
                "UPDATE long_tasks SET status = 'paused', updated_at = ?1 \
                 WHERE id = ?2 AND status = 'running'",
                params![now, &task.id],
            ) {
                Ok(_) => paused_ids.push(task.id.clone()),
                Err(e) => warn!(
                    target: "nebula.long_task",
                    task_id = %task.id,
                    error = %e,
                    "pause_all: failed to update task status"
                ),
            }
        }
        if !paused_ids.is_empty() {
            info!(
                target: "nebula.long_task",
                count = paused_ids.len(),
                "pause_all: paused running tasks"
            );
        }
        paused_ids
    }

    /// T-E-L-06: 列出所有运行中(status=Running)的任务。
    ///
    /// 供 pause_all 前的预览/UI 展示用。按 created_at 升序返回(最早的在前),
    /// 便于调用方优先关注运行时间最长的任务。
    pub fn list_running(&self) -> Vec<LongTask> {
        // 复用 list_tasks(Running 过滤),返回 DESC;反转为 ASC(最早的在前)
        let mut tasks = self
            .list_tasks(Some(LongTaskStatus::Running))
            .unwrap_or_default();
        tasks.reverse();
        tasks
    }

    /// 取消任务(Running/Paused/Pending → Cancelled)。
    ///
    /// 设置取消标志 + abort runner(立即停止当前命令的进程需更深层
    /// 集成,当前实现是协同式:当前步骤完成后停止)。
    pub fn cancel(&self, id: &str) -> Result<LongTask> {
        let task = self
            .get_task(id)?
            .ok_or_else(|| anyhow!("task {id} not found"))?;
        if task.status.is_terminal() {
            return Err(anyhow!(
                "task {id} already terminal (status={}), cannot cancel",
                task.status.as_str()
            ));
        }
        // 设置取消标志
        if let Some(flag) = self.cancel_flags.read().get(id) {
            flag.store(true, Ordering::SeqCst);
        }
        // 立即更新 DB 状态
        let now = Utc::now().timestamp();
        {
            let conn = self.sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute(
                "UPDATE long_tasks SET status = 'cancelled', updated_at = ?1, finished_at = ?2 WHERE id = ?3",
                params![now, now, id],
            )?;
            // 剩余 pending 步骤标记为 skipped
            conn.execute(
                "UPDATE long_task_steps SET status = 'skipped', finished_at = ?1 \
                 WHERE task_id = ?2 AND status = 'pending'",
                params![now, id],
            )?;
        }
        // abort runner(立即停止 tokio 任务)
        if let Some(handle) = self.runners.write().remove(id) {
            handle.abort();
        }
        self.cleanup_flags(id);
        self.get_task(id)?
            .ok_or_else(|| anyhow!("task vanished after cancel: {id}"))
    }

    /// 删除任务(硬删除,级联删除步骤)。
    pub fn delete_task(&self, id: &str) -> Result<bool> {
        // 先取消 runner
        if let Some(handle) = self.runners.write().remove(id) {
            handle.abort();
        }
        self.cleanup_flags(id);
        let conn = self.sqlite.raw_connection();
        let conn = conn.lock();
        let n = conn.execute("DELETE FROM long_tasks WHERE id = ?1", params![id])?;
        // long_task_steps 通过 ON DELETE CASCADE 或手动删除
        let _ = conn.execute(
            "DELETE FROM long_task_steps WHERE task_id = ?1",
            params![id],
        );
        Ok(n > 0)
    }

    /// bootstrap 时调用:加载所有任务,Running 状态重置为 Paused。
    ///
    /// 进程异常退出后,Running 状态的任务没有活跃 runner,重置为 Paused
    /// 让用户手动 resume。Pending/Completed/Failed/Cancelled/Paused 保持不变。
    pub fn bootstrap(&self) -> Result<usize> {
        let conn = self.sqlite.raw_connection();
        let conn = conn.lock();
        let now = Utc::now().timestamp();
        let n = conn.execute(
            "UPDATE long_tasks SET status = 'paused', updated_at = ?1 \
             WHERE status = 'running'",
            params![now],
        )?;
        if n > 0 {
            warn!(target: "nebula.long_task", count = n, "reset running tasks to paused on bootstrap");
        }
        // 也重置 Running 状态的步骤为 Pending(避免卡在 running)
        let _ = conn.execute(
            "UPDATE long_task_steps SET status = 'pending', started_at = NULL \
             WHERE status = 'running'",
            [],
        );
        Ok(n)
    }

    // ---- 内部辅助 ----

    fn cleanup_flags(&self, id: &str) {
        self.pause_flags.write().remove(id);
        self.cancel_flags.write().remove(id);
    }

    /// spawn 后台 runner 执行剩余步骤。
    ///
    /// runner 逻辑:
    /// 1. 加载所有步骤
    /// 2. 对每个 Pending 步骤:
    ///    a. 检查 cancel_flag → 若 true,break
    ///    b. 等待 pause_flag 清除(spin yield)
    ///    c. 标记步骤 Running
    ///    d. 执行命令(spawn_blocking 调用 shadow_engine.run_command)
    ///    e. 标记步骤 Done/Failed + 更新任务 progress
    /// 3. 全部完成 → 标记任务 Completed
    /// 4. 失败 → 标记任务 Failed
    /// 5. 取消 → 标记任务 Cancelled + 剩余步骤 Skipped
    fn spawn_runner(
        &self,
        task_id: String,
        pause_flag: Arc<AtomicBool>,
        cancel_flag: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        let sqlite = self.sqlite.clone();
        let shadow = self.shadow_engine.clone();
        let id = task_id.clone();
        tokio::spawn(async move {
            let result = run_task_loop(sqlite, shadow, id, pause_flag, cancel_flag).await;
            // runner 退出后清理(从 runners map 移除)
            // 注意:无法直接访问 self.runners(无 Arc 包装),清理由 cancel/ pause 等方法
            // 在下次调用时顺带完成。这里只记录日志。
            match &result {
                Ok(()) => info!(target: "nebula.long_task", "runner exited cleanly"),
                Err(e) => error!(target: "nebula.long_task", "runner exited with error: {e}"),
            }
        })
    }

    /// T-E-L-01: 生成 STATE.md 只读投影（SQLite → Markdown）。
    ///
    /// 将 long_tasks 表的内容投影为人类可读的 Markdown，供 Loop Agent
    /// 和用户审视当前状态。**只读 SQLite，不修改任何状态**。
    ///
    /// 输出格式（参考 NEBULA_LOOP_DESIGN.md §2.4）：
    /// - 头部：最后更新时间 + 数据源说明
    /// - `## 进行中`：Running/Paused 任务
    /// - `## 待处理`：Pending 任务
    /// - `## 已完成`：Completed 任务（最近 20 个）
    /// - `## 失败/取消`：Failed/Cancelled 任务（最近 20 个）
    /// - 每个任务项含：goal + progress + provenance 占位
    ///
    /// 原子写入：先写 `output_path.tmp`，再 `rename` 到 `output_path`，
    /// 避免读端看到半写文件。
    ///
    /// `provenance` 字段先用占位 `loop:unknown | autonomy:L2`，
    /// 真实填充由 T-E-L-07（审计日志）实现。
    #[instrument(skip(self, output_path))]
    pub fn state_projection(&self, output_path: &Path) -> Result<PathBuf> {
        // 1. 确保父目录存在
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).context("creating STATE.md parent dir")?;
            }
        }

        // 2. 读取所有任务
        let all_tasks = self
            .list_tasks(None)
            .context("listing tasks for STATE.md")?;

        // 3. 按状态分组
        let mut in_progress: Vec<&LongTask> = Vec::new();
        let mut pending: Vec<&LongTask> = Vec::new();
        let mut completed: Vec<&LongTask> = Vec::new();
        let mut failed_cancelled: Vec<&LongTask> = Vec::new();

        for task in &all_tasks {
            match task.status {
                LongTaskStatus::Running | LongTaskStatus::Paused => in_progress.push(task),
                LongTaskStatus::Pending => pending.push(task),
                LongTaskStatus::Completed => completed.push(task),
                LongTaskStatus::Failed | LongTaskStatus::Cancelled => {
                    failed_cancelled.push(task);
                }
            }
        }

        // 4. 已完成 + 失败/取消 截断到最近 20 个（list_tasks 已按 created_at DESC 排序）
        const MAX_HISTORY: usize = 20;
        if completed.len() > MAX_HISTORY {
            completed.truncate(MAX_HISTORY);
        }
        if failed_cancelled.len() > MAX_HISTORY {
            failed_cancelled.truncate(MAX_HISTORY);
        }

        // 5. 生成 Markdown
        let now_ts = Utc::now().timestamp();
        let now_str = chrono::DateTime::<chrono::Utc>::from_timestamp(now_ts, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| now_ts.to_string());

        let mut md = String::new();
        md.push_str("# STATE.md — Nebula 长任务只读投影\n\n");
        md.push_str(&format!("> 最后更新：{now_str}  \n"));
        md.push_str("> 数据源：SQLite long_tasks 表（只读投影，请勿手动编辑）\n\n");

        // 进行中
        md.push_str("## 进行中\n\n");
        if in_progress.is_empty() {
            md.push_str("_无_\n\n");
        } else {
            for t in &in_progress {
                push_task_item(&mut md, t);
            }
        }

        // 待处理
        md.push_str("## 待处理\n\n");
        if pending.is_empty() {
            md.push_str("_无_\n\n");
        } else {
            for t in &pending {
                push_task_item(&mut md, t);
            }
        }

        // 已完成
        md.push_str("## 已完成\n\n");
        if completed.is_empty() {
            md.push_str("_无_\n\n");
        } else {
            for t in &completed {
                push_task_item(&mut md, t);
            }
        }

        // 失败/取消
        md.push_str("## 失败/取消\n\n");
        if failed_cancelled.is_empty() {
            md.push_str("_无_\n\n");
        } else {
            for t in &failed_cancelled {
                push_task_item(&mut md, t);
            }
        }

        // 6. 原子写入：.tmp + rename
        let tmp_path = output_path.with_extension("md.tmp");
        fs::write(&tmp_path, &md).context("writing STATE.md.tmp")?;
        fs::rename(&tmp_path, output_path).context("renaming STATE.md.tmp → STATE.md")?;

        info!(
            target: "nebula.long_task",
            path = %output_path.display(),
            total = all_tasks.len(),
            "STATE.md projection written"
        );

        Ok(output_path.to_path_buf())
    }
}

/// T-E-L-01: 向 STATE.md Markdown 追加单个任务项。
///
/// 格式：
/// ```text
/// - **[status]** goal (progress%)
///   - provenance: loop:unknown | autonomy:L2
///   - error: <error> (仅 Failed 时)
/// ```
fn push_task_item(md: &mut String, task: &LongTask) {
    md.push_str(&format!(
        "- **[{}]** {} ({}%)\n",
        task.status.as_str(),
        task.goal,
        task.progress
    ));
    md.push_str("  - provenance: loop:unknown | autonomy:L2\n");
    if let Some(err) = &task.error {
        md.push_str(&format!("  - error: {err}\n"));
    }
    md.push('\n');
}

/// 后台 runner 主循环(独立 async fn,便于 spawn)。
///
/// 接受 `Arc` 而非引用,因为 `spawn_blocking` 闭包需要 `'static`。
async fn run_task_loop(
    sqlite: Arc<SqliteStore>,
    shadow: Arc<ShadowWorkspaceEngine>,
    task_id: String,
    pause_flag: Arc<AtomicBool>,
    cancel_flag: Arc<AtomicBool>,
) -> Result<()> {
    loop {
        // 1. 检查取消
        if cancel_flag.load(Ordering::SeqCst) {
            mark_task_cancelled(&sqlite, &task_id)?;
            return Ok(());
        }
        // 2. 检查暂停(自旋等待恢复)
        while pause_flag.load(Ordering::SeqCst) {
            if cancel_flag.load(Ordering::SeqCst) {
                mark_task_cancelled(&sqlite, &task_id)?;
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // 3. 取下一个 Pending 步骤
        let next = match fetch_next_pending_step(&sqlite, &task_id)? {
            Some(s) => s,
            None => {
                // 无更多步骤 → 任务完成
                mark_task_completed(&sqlite, &task_id)?;
                return Ok(());
            }
        };
        // 4. 标记步骤 Running
        mark_step_running(&sqlite, &task_id, next.seq)?;
        // 5. 执行命令(spawn_blocking 包裹同步 IO)
        let task = match load_task(&sqlite, &task_id)? {
            Some(t) => t,
            None => return Err(anyhow!("task {task_id} vanished during run")),
        };
        let program = next.program.clone();
        let args = next.args.clone();
        let ws_id = task.workspace_id.clone();
        let shadow_clone = shadow.clone();
        let exec_result = tokio::task::spawn_blocking(move || {
            // 若无 workspace_id,返回错误(步骤必须有执行环境)
            let ws_id = ws_id
                .as_deref()
                .ok_or_else(|| anyhow!("long task has no workspace_id, cannot run step"))?;
            shadow_clone.run_command(ws_id, &program, &args)
        })
        .await
        .context("spawn_blocking run_command")?;
        // 6. 根据结果更新步骤状态
        let now = Utc::now().timestamp();
        match exec_result {
            Ok(output) => {
                let truncated = truncate_output(&output);
                let conn = sqlite.raw_connection();
                let conn = conn.lock();
                conn.execute(
                    "UPDATE long_task_steps SET status = 'done', finished_at = ?1, exit_code = 0, output = ?2 \
                     WHERE task_id = ?3 AND seq = ?4",
                    params![now, truncated, task_id, next.seq],
                )?;
            }
            Err(e) => {
                let msg = e.to_string();
                let truncated = truncate_output(&msg);
                let conn = sqlite.raw_connection();
                let conn = conn.lock();
                conn.execute(
                    "UPDATE long_task_steps SET status = 'failed', finished_at = ?1, exit_code = 1, error = ?2 \
                     WHERE task_id = ?3 AND seq = ?4",
                    params![now, truncated, task_id, next.seq],
                )?;
                // 任务失败
                drop(conn);
                mark_task_failed(&sqlite, &task_id, &msg)?;
                return Ok(());
            }
        }
        // 7. 更新进度
        update_progress(&sqlite, &task_id)?;
    }
}

fn fetch_next_pending_step(sqlite: &SqliteStore, task_id: &str) -> Result<Option<LongTaskStep>> {
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT task_id, seq, description, program, args_json, status, \
                started_at, finished_at, exit_code, output, error \
         FROM long_task_steps WHERE task_id = ?1 AND status = 'pending' \
         ORDER BY seq ASC LIMIT 1",
    )?;
    let row = stmt.query_row(params![task_id], |r| {
        let status_str: String = r.get(5)?;
        let args_json: String = r.get(4)?;
        let args: Vec<String> = serde_json::from_str(&args_json).unwrap_or_default();
        let status = StepStatus::from_str(&status_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
        })?;
        Ok(LongTaskStep {
            task_id: r.get(0)?,
            seq: r.get(1)?,
            description: r.get(2)?,
            program: r.get(3)?,
            args,
            status,
            started_at: r.get(6)?,
            finished_at: r.get(7)?,
            exit_code: r.get(8)?,
            output: r.get(9)?,
            error: r.get(10)?,
        })
    });
    match row {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn mark_step_running(sqlite: &SqliteStore, task_id: &str, seq: u32) -> Result<()> {
    let now = Utc::now().timestamp();
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    conn.execute(
        "UPDATE long_task_steps SET status = 'running', started_at = ?1 \
         WHERE task_id = ?2 AND seq = ?3",
        params![now, task_id, seq],
    )?;
    Ok(())
}

fn load_task(sqlite: &SqliteStore, task_id: &str) -> Result<Option<LongTask>> {
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT id, goal, status, workspace_id, plan_id, progress, error, \
                created_at, updated_at, started_at, finished_at \
         FROM long_tasks WHERE id = ?1",
    )?;
    let row = stmt.query_row(params![task_id], row_to_task);
    match row {
        Ok(t) => Ok(Some(t)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn mark_task_completed(sqlite: &SqliteStore, task_id: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    conn.execute(
        "UPDATE long_tasks SET status = 'completed', progress = 100, \
         updated_at = ?1, finished_at = ?2 WHERE id = ?3",
        params![now, now, task_id],
    )?;
    info!(target: "nebula.long_task", id = %task_id, "task completed");
    Ok(())
}

fn mark_task_failed(sqlite: &SqliteStore, task_id: &str, error: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    conn.execute(
        "UPDATE long_tasks SET status = 'failed', error = ?1, \
         updated_at = ?2, finished_at = ?3 WHERE id = ?4",
        params![truncate_output(error), now, now, task_id],
    )?;
    warn!(target: "nebula.long_task", id = %task_id, "task failed: {error}");
    Ok(())
}

fn mark_task_cancelled(sqlite: &SqliteStore, task_id: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    conn.execute(
        "UPDATE long_tasks SET status = 'cancelled', \
         updated_at = ?1, finished_at = ?2 WHERE id = ?3",
        params![now, now, task_id],
    )?;
    conn.execute(
        "UPDATE long_task_steps SET status = 'skipped', finished_at = ?1 \
         WHERE task_id = ?2 AND status = 'pending'",
        params![now, task_id],
    )?;
    info!(target: "nebula.long_task", id = %task_id, "task cancelled");
    Ok(())
}

fn update_progress(sqlite: &SqliteStore, task_id: &str) -> Result<()> {
    let conn = sqlite.raw_connection();
    let conn = conn.lock();
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM long_task_steps WHERE task_id = ?1",
        params![task_id],
        |r| r.get(0),
    )?;
    let done: i64 = conn.query_row(
        "SELECT COUNT(*) FROM long_task_steps WHERE task_id = ?1 AND status IN ('done','failed','skipped')",
        params![task_id],
        |r| r.get(0),
    )?;
    let progress = if total > 0 {
        ((done as f64 / total as f64) * 100.0) as i32
    } else {
        0
    };
    let now = Utc::now().timestamp();
    conn.execute(
        "UPDATE long_tasks SET progress = ?1, updated_at = ?2 WHERE id = ?3",
        params![progress, now, task_id],
    )?;
    Ok(())
}

/// 截断输出到 5000 字符(避免 DB 膨胀)。
fn truncate_output(s: &str) -> String {
    const MAX: usize = 5000;
    if s.len() > MAX {
        format!("{}…", &s[..MAX])
    } else {
        s.to_string()
    }
}

/// rusqlite 行 → LongTask 映射(供 list/get 复用)。
fn row_to_task(r: &rusqlite::Row) -> rusqlite::Result<LongTask> {
    let status_str: String = r.get(2)?;
    let status = LongTaskStatus::from_str(&status_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, e.into())
    })?;
    Ok(LongTask {
        id: r.get(0)?,
        goal: r.get(1)?,
        status,
        workspace_id: r.get(3)?,
        plan_id: r.get(4)?,
        progress: r.get(5)?,
        error: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        started_at: r.get(9)?,
        finished_at: r.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造内存态 SQLite + 真实 schema(用于测试)。
    fn make_engine() -> (LongTaskEngine, std::path::PathBuf) {
        use std::fs;
        // 用临时文件 SQLite(不能用纯内存,因为 raw_connection 返回的 MutexGuard
        // 在多线程 spawn_blocking 时需要持久化)
        let tmp = std::env::temp_dir().join(format!("nebula-long-task-test-{}", Uuid::new_v4()));
        let _ = fs::remove_file(&tmp);
        let sqlite = Arc::new(SqliteStore::open(&tmp).expect("open sqlite"));
        // 应用 long_tasks schema(模拟 migration)
        let conn = sqlite.raw_connection();
        let conn = conn.lock();
        conn.execute_batch(include_str!("../../migrations/037_long_tasks.sql"))
            .expect("apply migration");
        drop(conn);
        // shadow_engine 用默认(不设 repo_root,run_command 会失败但测试不依赖执行)
        let shadow = Arc::new(ShadowWorkspaceEngine::with_default());
        let engine = LongTaskEngine::new(sqlite, shadow);
        (engine, tmp)
    }

    fn cleanup(path: std::path::PathBuf) {
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn create_task_persists_with_steps() {
        let (engine, tmp) = make_engine();
        let steps = vec![
            StepInput {
                description: "step 1".into(),
                program: "echo".into(),
                args: vec!["hello".into()],
            },
            StepInput {
                description: "step 2".into(),
                program: "echo".into(),
                args: vec!["world".into()],
            },
        ];
        let task = engine
            .create_task("test goal".into(), steps, Some("ws123".into()), None)
            .expect("create");
        assert_eq!(task.goal, "test goal");
        assert_eq!(task.status, LongTaskStatus::Pending);
        assert_eq!(task.progress, 0);
        assert_eq!(task.workspace_id.as_deref(), Some("ws123"));

        // 步骤持久化
        let steps = engine.get_steps(&task.id).expect("get_steps");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].seq, 1);
        assert_eq!(steps[0].program, "echo");
        assert_eq!(steps[0].args, vec!["hello".to_string()]);
        assert_eq!(steps[0].status, StepStatus::Pending);
        assert_eq!(steps[1].seq, 2);

        // get_task 也能取回
        let fetched = engine
            .get_task(&task.id)
            .expect("get_task")
            .expect("get should succeed");
        assert_eq!(fetched.goal, "test goal");

        cleanup(tmp);
    }

    #[test]
    fn create_rejects_empty_goal() {
        let (engine, tmp) = make_engine();
        let err = engine
            .create_task(
                "  ".into(),
                vec![StepInput {
                    description: "x".into(),
                    program: "echo".into(),
                    args: vec![],
                }],
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("empty"));
        cleanup(tmp);
    }

    #[test]
    fn create_rejects_empty_steps() {
        let (engine, tmp) = make_engine();
        let err = engine
            .create_task("goal".into(), vec![], None, None)
            .unwrap_err();
        assert!(err.to_string().contains("at least one step"));
        cleanup(tmp);
    }

    #[test]
    fn list_tasks_orders_by_created_desc() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec![],
        };
        let _ = engine
            .create_task("a".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = engine
            .create_task("b".into(), vec![step], None, None)
            .expect("test op should succeed");

        let list = engine.list_tasks(None).expect("list");
        assert_eq!(list.len(), 2);
        // 降序:b 在前
        assert_eq!(list[0].goal, "b");
        assert_eq!(list[1].goal, "a");
        cleanup(tmp);
    }

    #[test]
    fn list_tasks_filters_by_status() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec![],
        };
        let _ = engine
            .create_task("a".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let _ = engine
            .create_task("b".into(), vec![step], None, None)
            .expect("test op should succeed");

        let pending = engine
            .list_tasks(Some(LongTaskStatus::Pending))
            .expect("list");
        assert_eq!(pending.len(), 2);
        let running = engine
            .list_tasks(Some(LongTaskStatus::Running))
            .expect("list");
        assert_eq!(running.len(), 0);
        cleanup(tmp);
    }

    #[tokio::test]
    async fn start_pending_transitions_to_running() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], Some("ws".into()), None)
            .expect("test op should succeed");
        let started = engine.start(&task.id).expect("start");
        assert_eq!(started.status, LongTaskStatus::Running);
        assert!(started.started_at.is_some());
        // runner 会因为 workspace "ws" 不存在而失败,但状态会变 Running → Failed
        // 等待一下让 runner 执行
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // 应该已经失败(workspace 不存在)
        let after = engine
            .get_task(&task.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(
            after.status == LongTaskStatus::Failed || after.status == LongTaskStatus::Running,
            "expected failed or running, got {}",
            after.status.as_str()
        );
        cleanup(tmp);
    }

    #[tokio::test]
    async fn start_already_running_returns_error() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], Some("ws".into()), None)
            .expect("test op should succeed");
        engine.start(&task.id).expect("engine op should succeed");
        let err = engine.start(&task.id).unwrap_err();
        assert!(err.to_string().contains("already running"));
        cleanup(tmp);
    }

    #[test]
    fn start_completed_returns_error() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], Some("ws".into()), None)
            .expect("test op should succeed");
        // 手动标记为 completed
        {
            let conn = engine.sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute(
                "UPDATE long_tasks SET status = 'completed' WHERE id = ?1",
                params![task.id],
            )
            .expect("test op should succeed");
        }
        let err = engine.start(&task.id).unwrap_err();
        assert!(err.to_string().contains("already completed"));
        cleanup(tmp);
    }

    #[test]
    fn cancel_pending_transitions_to_cancelled() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        let cancelled = engine.cancel(&task.id).expect("cancel");
        assert_eq!(cancelled.status, LongTaskStatus::Cancelled);
        assert!(cancelled.finished_at.is_some());
        // 步骤应被标记为 skipped
        let steps = engine.get_steps(&task.id).expect("get should succeed");
        assert_eq!(steps[0].status, StepStatus::Skipped);
        cleanup(tmp);
    }

    #[test]
    fn cancel_terminal_returns_error() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        engine.cancel(&task.id).expect("engine op should succeed");
        let err = engine.cancel(&task.id).unwrap_err();
        assert!(err.to_string().contains("already terminal"));
        cleanup(tmp);
    }

    #[test]
    fn pause_non_running_returns_error() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        let err = engine.pause(&task.id).unwrap_err();
        assert!(err.to_string().contains("not running"));
        cleanup(tmp);
    }

    #[test]
    fn resume_non_paused_returns_error() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        let err = engine.resume(&task.id).unwrap_err();
        assert!(err.to_string().contains("not paused"));
        cleanup(tmp);
    }

    #[test]
    fn delete_task_removes_task_and_steps() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        assert!(engine.delete_task(&task.id).expect("delete"));
        assert!(engine
            .get_task(&task.id)
            .expect("get should succeed")
            .is_none());
        assert!(engine
            .get_steps(&task.id)
            .expect("get should succeed")
            .is_empty());
        cleanup(tmp);
    }

    #[test]
    fn bootstrap_resets_running_to_paused() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        // 手动标记为 running(模拟进程崩溃前的状态)
        {
            let conn = engine.sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute(
                "UPDATE long_tasks SET status = 'running' WHERE id = ?1",
                params![task.id],
            )
            .expect("test op should succeed");
        }
        let n = engine.bootstrap().expect("bootstrap");
        assert_eq!(n, 1, "should reset 1 running task");
        let after = engine
            .get_task(&task.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(after.status, LongTaskStatus::Paused);
        cleanup(tmp);
    }

    #[test]
    fn bootstrap_leaves_other_statuses_unchanged() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let task = engine
            .create_task("g".into(), vec![step], None, None)
            .expect("test op should succeed");
        // pending → 应保持不变
        let n = engine.bootstrap().expect("bootstrap");
        assert_eq!(n, 0);
        let after = engine
            .get_task(&task.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(after.status, LongTaskStatus::Pending);
        cleanup(tmp);
    }

    #[test]
    fn status_is_terminal_works() {
        assert!(!LongTaskStatus::Pending.is_terminal());
        assert!(!LongTaskStatus::Running.is_terminal());
        assert!(!LongTaskStatus::Paused.is_terminal());
        assert!(LongTaskStatus::Completed.is_terminal());
        assert!(LongTaskStatus::Failed.is_terminal());
        assert!(LongTaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn status_round_trip() {
        for s in [
            LongTaskStatus::Pending,
            LongTaskStatus::Running,
            LongTaskStatus::Paused,
            LongTaskStatus::Completed,
            LongTaskStatus::Failed,
            LongTaskStatus::Cancelled,
        ] {
            let s2 = LongTaskStatus::from_str(s.as_str()).expect("round trip");
            assert_eq!(s, s2);
        }
    }

    #[test]
    fn truncate_output_caps_at_max() {
        let long = "x".repeat(10000);
        let t = truncate_output(&long);
        assert!(t.len() <= 5100); // 5000 + "…"
        assert!(t.ends_with('…'));
        let short = "hello";
        assert_eq!(truncate_output(short), "hello");
    }

    // ---- T-E-L-01: state_projection 测试 ----

    /// 直接在 SQLite 中设置任务状态（测试辅助）。
    fn set_task_status(engine: &LongTaskEngine, task_id: &str, status: &str) {
        let conn = engine.sqlite.raw_connection();
        let conn = conn.lock();
        conn.execute(
            "UPDATE long_tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, Utc::now().timestamp(), task_id],
        )
        .expect("update status");
    }

    #[test]
    fn state_projection_empty() {
        let (engine, tmp) = make_engine();
        let out = std::env::temp_dir().join(format!("nebula-state-empty-{}.md", Uuid::new_v4()));
        let result = engine.state_projection(&out).expect("projection");
        assert_eq!(result, out);

        let content = fs::read_to_string(&out).expect("read");
        assert!(content.contains("# STATE.md"));
        assert!(content.contains("## 进行中"));
        assert!(content.contains("## 待处理"));
        assert!(content.contains("## 已完成"));
        assert!(content.contains("## 失败/取消"));
        // 空列表应显示 _无_
        assert!(content.contains("_无_"));

        let _ = fs::remove_file(&out);
        cleanup(tmp);
    }

    #[test]
    fn state_projection_pending() {
        let (engine, tmp) = make_engine();
        let task = engine
            .create_task(
                "pending goal".into(),
                vec![StepInput {
                    description: "s1".into(),
                    program: "echo".into(),
                    args: vec![],
                }],
                None,
                None,
            )
            .expect("create");

        let out = std::env::temp_dir().join(format!("nebula-state-pending-{}.md", Uuid::new_v4()));
        engine.state_projection(&out).expect("projection");
        let content = fs::read_to_string(&out).expect("read");

        // Pending 任务应出现在 ## 待处理
        assert!(content.contains("## 待处理"));
        assert!(content.contains("pending goal"));
        assert!(content.contains("[pending]"));
        assert!(content.contains("provenance: loop:unknown | autonomy:L2"));

        // 不应出现在已完成
        let completed_section = content.split("## 已完成").nth(1).unwrap_or("");
        assert!(!completed_section.contains("pending goal"));

        let _ = fs::remove_file(&out);
        let _ = engine.delete_task(&task.id);
        cleanup(tmp);
    }

    #[test]
    fn state_projection_completed() {
        let (engine, tmp) = make_engine();
        let task = engine
            .create_task(
                "completed goal".into(),
                vec![StepInput {
                    description: "s1".into(),
                    program: "echo".into(),
                    args: vec![],
                }],
                None,
                None,
            )
            .expect("create");
        set_task_status(&engine, &task.id, "completed");

        let out = std::env::temp_dir().join(format!("nebula-state-done-{}.md", Uuid::new_v4()));
        engine.state_projection(&out).expect("projection");
        let content = fs::read_to_string(&out).expect("read");

        // Completed 任务应出现在 ## 已完成
        let completed_section = content.split("## 已完成").nth(1).unwrap_or("");
        assert!(completed_section.contains("completed goal"));
        assert!(completed_section.contains("[completed]"));

        let _ = fs::remove_file(&out);
        let _ = engine.delete_task(&task.id);
        cleanup(tmp);
    }

    #[test]
    fn state_projection_atomic_write() {
        let (engine, tmp) = make_engine();
        let out = std::env::temp_dir().join(format!("nebula-state-atomic-{}.md", Uuid::new_v4()));
        engine.state_projection(&out).expect("projection");

        // .tmp 文件不应残留
        let tmp_file = out.with_extension("md.tmp");
        assert!(!tmp_file.exists(), "tmp file should not linger");

        // 目标文件应存在
        assert!(out.exists());

        let _ = fs::remove_file(&out);
        cleanup(tmp);
    }

    #[test]
    fn state_projection_creates_parent_dir() {
        let (engine, tmp) = make_engine();
        let nested = std::env::temp_dir().join(format!(
            "nebula-state-nested-{}/sub/dir/STATE.md",
            Uuid::new_v4()
        ));
        engine
            .state_projection(&nested)
            .expect("projection with nested dirs");
        assert!(nested.exists(), "nested STATE.md should be created");

        // 清理整个嵌套目录
        let parent = nested
            .ancestors()
            .nth(3)
            .expect("test op should succeed")
            .to_path_buf();
        let _ = fs::remove_dir_all(&parent);
        cleanup(tmp);
    }

    #[test]
    fn state_projection_truncates() {
        let (engine, tmp) = make_engine();

        // 创建 100 个 Completed 任务
        let mut ids = Vec::new();
        for i in 0..100 {
            let task = engine
                .create_task(
                    format!("goal-{i}").into(),
                    vec![StepInput {
                        description: "s".into(),
                        program: "echo".into(),
                        args: vec![],
                    }],
                    None,
                    None,
                )
                .expect("create");
            set_task_status(&engine, &task.id, "completed");
            ids.push(task.id);
        }

        let out = std::env::temp_dir().join(format!("nebula-state-trunc-{}.md", Uuid::new_v4()));
        engine.state_projection(&out).expect("projection");
        let content = fs::read_to_string(&out).expect("read");

        // ## 已完成 段落应最多 20 个任务项
        let completed_section = content
            .split("## 已完成")
            .nth(1)
            .unwrap_or("")
            .split("## 失败/取消")
            .next()
            .unwrap_or("");
        let count = completed_section.matches("- **[completed]**").count();
        assert!(count <= 20, "should truncate to <= 20, got {count}");

        // 清理
        for id in &ids {
            let _ = engine.delete_task(id);
        }
        let _ = fs::remove_file(&out);
        cleanup(tmp);
    }

    // ---- T-E-L-06: pause_all / list_running 测试 ----

    #[test]
    fn pause_all_pauses_running_tasks() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        // 创建 3 个任务:2 个 Running,1 个 Completed
        let t1 = engine
            .create_task("running-1".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t2 = engine
            .create_task("running-2".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t3 = engine
            .create_task("completed-1".into(), vec![step], None, None)
            .expect("test op should succeed");
        set_task_status(&engine, &t1.id, "running");
        set_task_status(&engine, &t2.id, "running");
        set_task_status(&engine, &t3.id, "completed");

        let paused = engine.pause_all();
        // 应返回 2 个 ID(t1, t2)
        assert_eq!(paused.len(), 2, "should pause 2 running tasks");
        assert!(paused.contains(&t1.id), "t1 should be paused");
        assert!(paused.contains(&t2.id), "t2 should be paused");
        // t3 不应被暂停(Completed 跳过)
        assert!(!paused.contains(&t3.id), "t3 should not be paused");

        // 验证状态:t1, t2 → Paused,t3 仍为 Completed
        let after1 = engine
            .get_task(&t1.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(after1.status, LongTaskStatus::Paused);
        let after2 = engine
            .get_task(&t2.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(after2.status, LongTaskStatus::Paused);
        let after3 = engine
            .get_task(&t3.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(after3.status, LongTaskStatus::Completed);

        cleanup(tmp);
    }

    #[test]
    fn pause_all_skips_already_paused() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let t1 = engine
            .create_task("running".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t2 = engine
            .create_task("paused".into(), vec![step], None, None)
            .expect("test op should succeed");
        // t1 Running,t2 已 Paused
        set_task_status(&engine, &t1.id, "running");
        set_task_status(&engine, &t2.id, "paused");

        let paused = engine.pause_all();
        // 只应返回 t1(已暂停的 t2 跳过)
        assert_eq!(paused.len(), 1, "should skip already paused task");
        assert_eq!(paused[0], t1.id, "only t1 should be in paused list");

        // t2 状态应保持 Paused(不变)
        let after2 = engine
            .get_task(&t2.id)
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(after2.status, LongTaskStatus::Paused);

        cleanup(tmp);
    }

    #[test]
    fn pause_all_returns_paused_ids() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        // 创建 3 个 Running 任务(按 created_at 升序)
        let t1 = engine
            .create_task("r1".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t2 = engine
            .create_task("r2".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t3 = engine
            .create_task("r3".into(), vec![step], None, None)
            .expect("test op should succeed");
        set_task_status(&engine, &t1.id, "running");
        set_task_status(&engine, &t2.id, "running");
        set_task_status(&engine, &t3.id, "running");

        let paused = engine.pause_all();
        // 应返回 3 个 ID,顺序为 created_at 升序(t1, t2, t3)
        assert_eq!(paused.len(), 3, "should pause all 3 running tasks");
        assert_eq!(paused[0], t1.id, "earliest task should be first");
        assert_eq!(paused[1], t2.id);
        assert_eq!(paused[2], t3.id, "latest task should be last");

        cleanup(tmp);
    }

    #[test]
    fn list_running_filters_correctly() {
        let (engine, tmp) = make_engine();
        let step = StepInput {
            description: "s".into(),
            program: "echo".into(),
            args: vec!["x".into()],
        };
        let t_pending = engine
            .create_task("pending".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t_running = engine
            .create_task("running".into(), vec![step.clone()], None, None)
            .expect("test op should succeed");
        let t_completed = engine
            .create_task("completed".into(), vec![step], None, None)
            .expect("test op should succeed");
        // t_pending 保持 Pending
        set_task_status(&engine, &t_running.id, "running");
        set_task_status(&engine, &t_completed.id, "completed");

        let running = engine.list_running();
        // 只应返回 Running 状态的任务
        assert_eq!(running.len(), 1, "should only return running tasks");
        assert_eq!(running[0].id, t_running.id);
        assert_eq!(running[0].status, LongTaskStatus::Running);
        // 不应包含 Pending / Completed
        assert!(
            !running.iter().any(|t| t.id == t_pending.id),
            "should not include pending task"
        );
        assert!(
            !running.iter().any(|t| t.id == t_completed.id),
            "should not include completed task"
        );

        cleanup(tmp);
    }

    #[test]
    fn pause_all_empty_returns_empty_vec() {
        let (engine, tmp) = make_engine();
        // 无任务场景
        let paused = engine.pause_all();
        assert!(paused.is_empty(), "should return empty vec when no tasks");

        // list_running 也应返回空
        let running = engine.list_running();
        assert!(running.is_empty(), "list_running should also be empty");

        cleanup(tmp);
    }
}
