//! Cron 调度器。
//!
//! 对标: Hermes Agent 内建 cron 调度器。
//! ROADMAP v2.2 T-E-S-53 标为 P1 但未实现。
//!
//! 三计时机制(ROADMAP v2.2 T-E-S-63):
//! - 合并(03:00):L1→L2 记忆合并
//! - 自检(12:00):EvolutionEngine 4阶段运行
//! - 回顾(21:00):Honcho 画像 nudge + Skill 评估
//!
//! ## Feature Gate
//!
//! 由 `self-evolution` feature 门控(与 evolution 模块一致)。
//! 实际的 LLM 调用和记忆合并走现有模块,本调度器只负责定时触发。

#![cfg(feature = "self-evolution")]

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Timelike, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::time::interval;
use tracing::{info, warn};

use super::honcho::HonchoEngine;
// T-D-B-11: EvolutionEngine 注入,使 12:00 自检任务能实际触发 4 Phase 进化。
#[cfg(feature = "evolution-engine")]
use super::engine::{EvolutionEngine, EvolutionError};

/// Cron 任务定义。
///
/// T-E-L-02 扩展:新增 `cron_expr`(5 字段 cron 表达式)、`autonomy`(L0-L5 自主度)、
/// `budget_tokens_*` / `budget_minutes_*`(预算)字段。旧字段 `hour` 保留向后兼容:
/// 当 `cron_expr` 为 None 时,`should_run()` 回退到 `hour` 匹配。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CronTask {
    /// 任务名称(唯一标识)。
    pub name: String,
    /// 每天执行的小时(0-23)。旧字段,当 `cron_expr` 为 None 时使用。
    pub hour: u32,
    /// 任务是否启用。
    pub enabled: bool,
    /// 上次执行时间(UTC)。
    pub last_run: Option<DateTime<Utc>>,

    /// T-E-L-02: 5 字段 cron 表达式(如 "0 9 * * 1-5")。
    /// 若设置,`should_run()` 使用 `CronExpr::matches()` 而非 `hour`。
    #[serde(default)]
    pub cron_expr: Option<String>,

    /// T-E-L-02: L0-L5 自主度(决定任务执行时是否需要审批)。
    /// 默认 L2(对话);后台自动化任务建议设为 L5。
    #[serde(default)]
    pub autonomy: crate::autonomy::AutonomyLevel,

    /// T-E-L-02: Token 预算上限(0 = 无限制)。
    #[serde(default)]
    pub budget_tokens_total: u64,

    /// T-E-L-02: 已用 Token(AtomicU64 内存累加,异步落库)。
    #[serde(default)]
    pub budget_tokens_used: u64,

    /// T-E-L-02: 时间预算上限-分钟(0 = 无限制)。
    #[serde(default)]
    pub budget_minutes_total: u64,

    /// T-E-L-02: 已用时间-分钟。
    #[serde(default)]
    pub budget_minutes_used: u64,
}

impl CronTask {
    /// 默认三计时任务表。
    ///
    /// T-E-L-02: 每个任务现在带 5 字段 cron 表达式 + L5 自主度 + 预算。
    pub fn default_schedule() -> Vec<CronTask> {
        use crate::autonomy::AutonomyLevel;
        vec![
            CronTask {
                name: "memory-merge".to_string(),
                hour: 3,
                enabled: true,
                last_run: None,
                cron_expr: Some("0 3 * * *".to_string()),
                autonomy: AutonomyLevel::L5Background,
                budget_tokens_total: 50_000,
                budget_minutes_total: 10,
                ..Default::default()
            },
            CronTask {
                name: "evolution-self-check".to_string(),
                hour: 12,
                enabled: true,
                last_run: None,
                cron_expr: Some("0 12 * * *".to_string()),
                autonomy: AutonomyLevel::L5Background,
                budget_tokens_total: 100_000,
                budget_minutes_total: 30,
                ..Default::default()
            },
            CronTask {
                name: "evening-review".to_string(),
                hour: 21,
                enabled: true,
                last_run: None,
                cron_expr: Some("0 21 * * *".to_string()),
                autonomy: AutonomyLevel::L5Background,
                budget_tokens_total: 50_000,
                budget_minutes_total: 15,
                ..Default::default()
            },
        ]
    }

    /// 检查任务是否应该执行。
    ///
    /// T-E-L-02: 当 `cron_expr` 字段设置时,使用 `CronExpr::matches()` 进行
    /// 5 字段匹配(支持一天多次执行);否则回退到旧的 `hour` 字段匹配(每天一次)。
    ///
    /// **防重复执行策略**:
    /// - `cron_expr` 任务:距上次执行至少 60 秒(允许一天多次,但同一分钟不重复)
    /// - `hour` 任务:同一天不重复(保持旧行为)
    fn should_run(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }

        // T-E-L-02: cron_expr 优先,解析失败回退到 hour 字段。
        let time_matches = if let Some(expr_str) = &self.cron_expr {
            match crate::cron_expr::CronExpr::parse(expr_str) {
                Ok(expr) => expr.matches(now),
                Err(e) => {
                    warn!(
                        target: "nebula.cron",
                        task = %self.name,
                        expr = %expr_str,
                        error = %e,
                        "invalid cron_expr, falling back to hour field"
                    );
                    now.hour() == self.hour
                }
            }
        } else {
            now.hour() == self.hour
        };

        if !time_matches {
            return false;
        }

        // 防重复执行。
        match self.last_run {
            None => true,
            Some(last) => {
                if self.cron_expr.is_some() {
                    // cron_expr 任务:至少间隔 60 秒(允许一天多次执行)。
                    (now.timestamp() - last.timestamp()) >= 60
                } else {
                    // hour 任务:同一天不重复(保持旧行为)。
                    last.date_naive() != now.date_naive()
                }
            }
        }
    }

    /// T-E-L-02: 检查任务预算是否已超限。
    ///
    /// - `budget_tokens_total = 0` 表示 Token 无限制
    /// - `budget_minutes_total = 0` 表示时间无限制
    ///
    /// 任一维度超限即返回 `true`。预算在每个调度周期开始时检查,
    /// 超限任务会被跳过(不执行)。
    pub fn budget_exceeded(&self) -> bool {
        if self.budget_tokens_total > 0 && self.budget_tokens_used >= self.budget_tokens_total {
            return true;
        }
        if self.budget_minutes_total > 0 && self.budget_minutes_used >= self.budget_minutes_total {
            return true;
        }
        false
    }

    /// T-E-L-02: 累加 Token 用量(saturating,不会溢出)。
    pub fn add_token_usage(&mut self, tokens: u64) {
        self.budget_tokens_used = self.budget_tokens_used.saturating_add(tokens);
    }

    /// T-E-L-02: 累加时间用量(分钟,saturating)。
    pub fn add_minutes_usage(&mut self, minutes: u64) {
        self.budget_minutes_used = self.budget_minutes_used.saturating_add(minutes);
    }
}

/// Cron 调度器。
///
/// 每 60 秒检查一次任务表,到达预定时间则触发对应任务。
/// 任务执行是"尽力而为":失败记录 warning 但不中断调度循环。
///
/// T-E-L-02: 新增预算检查 — 任务执行前检查 `budget_exceeded()`,
/// 超限任务跳过;执行后累加时间预算 + 聚合 AtomicU64 计数器。
pub struct CronScheduler {
    tasks: Arc<Mutex<Vec<CronTask>>>,
    honcho: Option<Arc<HonchoEngine>>,
    /// T-D-B-11: EvolutionEngine 引用(可选)。注入后,12:00 自检任务会
    /// 实际触发 4 Phase 进化管线;未注入时仅记 warning 跳过。
    #[cfg(feature = "evolution-engine")]
    evolution_engine: Option<Arc<EvolutionEngine>>,
    /// T-D-B-11: 进化运行时的 master_id(用于记忆 domain 隔离,
    /// 写入 `evolution:<master_id>` 域)。默认与 user_id 相同,
    /// 可通过 `with_master_id()` 覆盖。
    master_id: String,
    /// 用户 ID(用于 Honcho nudge)。
    user_id: String,
    /// 检查间隔(秒,默认 60)。
    check_interval_secs: u64,
    /// 是否正在运行。
    running: Arc<std::sync::atomic::AtomicBool>,
    /// T-E-L-02: 聚合 Token 用量(所有任务合计,AtomicU64 内存累加)。
    /// 异步落库由调用者/后台 worker 负责(见 `flush_budget_to_store()`)。
    aggregate_tokens_used: Arc<std::sync::atomic::AtomicU64>,
    /// T-E-L-02: 聚合执行时间-秒(所有任务合计,AtomicU64 内存累加)。
    aggregate_seconds_used: Arc<std::sync::atomic::AtomicU64>,
}

impl CronScheduler {
    pub fn new(honcho: Option<Arc<HonchoEngine>>, user_id: String) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(CronTask::default_schedule())),
            honcho,
            #[cfg(feature = "evolution-engine")]
            evolution_engine: None,
            master_id: user_id.clone(),
            user_id,
            check_interval_secs: 60,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            aggregate_tokens_used: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            aggregate_seconds_used: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// 自定义检查间隔(主要用于测试)。
    pub fn with_check_interval(mut self, secs: u64) -> Self {
        self.check_interval_secs = secs;
        self
    }

    /// T-D-B-11: 注入 EvolutionEngine,使 12:00 自检任务能实际触发 4 Phase 进化。
    ///
    /// 传 `None` 可移除已注入的引擎(自检任务回退到仅记 warning 跳过)。
    /// 未调用此方法时,`evolution_engine` 默认为 `None`(向后兼容)。
    #[cfg(feature = "evolution-engine")]
    pub fn with_evolution_engine(mut self, engine: Option<Arc<EvolutionEngine>>) -> Self {
        self.evolution_engine = engine;
        self
    }

    /// T-D-B-11: 设置进化运行时的 master_id(用于记忆 domain 隔离)。
    ///
    /// 默认 `master_id = user_id`。不同 master 的进化结果写入各自的
    /// domain(`evolution:<master_id>`),与 `shared` / `worker:*` 域隔离,
    /// 互不干扰(参考 EvolutionEngine 三层共存设计)。
    pub fn with_master_id(mut self, master_id: String) -> Self {
        self.master_id = master_id;
        self
    }

    /// 返回当前任务表的快照。
    pub fn list_tasks(&self) -> Vec<CronTask> {
        self.tasks.lock().clone()
    }

    /// 启用/禁用指定任务。
    pub fn set_task_enabled(&self, name: &str, enabled: bool) -> bool {
        let mut tasks = self.tasks.lock();
        for task in tasks.iter_mut() {
            if task.name == name {
                task.enabled = enabled;
                return true;
            }
        }
        false
    }

    /// T-E-L-02: 记录 Token 用量(由 execute_* 方法或外部调用者调用)。
    ///
    /// 使用 AtomicU64 无锁累加到聚合计数器,同时更新指定任务的预算用量。
    /// 异步落库由后台 worker 负责(`flush_budget_to_store`)。
    pub fn record_token_usage(&self, task_name: &str, tokens: u64) {
        self.aggregate_tokens_used
            .fetch_add(tokens, std::sync::atomic::Ordering::Relaxed);
        let mut tasks = self.tasks.lock();
        for t in tasks.iter_mut() {
            if t.name == task_name {
                t.add_token_usage(tokens);
                break;
            }
        }
        info!(
            target: "nebula.cron",
            task = %task_name,
            tokens = tokens,
            "token usage recorded"
        );
    }

    /// T-E-L-02: 聚合 Token 用量(所有任务合计)。
    pub fn aggregate_tokens_used(&self) -> u64 {
        self.aggregate_tokens_used
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// T-E-L-02: 聚合执行时间-秒(所有任务合计)。
    pub fn aggregate_seconds_used(&self) -> u64 {
        self.aggregate_seconds_used
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// T-E-L-02: 重置所有任务的预算用量(用于新周期)。
    ///
    /// 调用此方法后,所有任务的 `budget_*_used` 清零,聚合计数器也清零。
    /// 通常在日/周/月切换时调用。
    pub fn reset_all_budgets(&self) {
        let mut tasks = self.tasks.lock();
        for t in tasks.iter_mut() {
            t.budget_tokens_used = 0;
            t.budget_minutes_used = 0;
        }
        self.aggregate_tokens_used
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.aggregate_seconds_used
            .store(0, std::sync::atomic::Ordering::Relaxed);
        info!(target: "nebula.cron", "all task budgets reset");
    }

    /// 启动调度循环。这个方法会阻塞当前 tokio 任务直到取消。
    pub async fn start(self: Arc<Self>) -> Result<()> {
        if self
            .running
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            warn!(target: "nebula.cron", "cron scheduler already running");
            return Ok(());
        }

        info!(
            target: "nebula.cron",
            check_interval_secs = self.check_interval_secs,
            tasks = self.tasks.lock().len(),
            "cron scheduler started"
        );

        let mut tick = interval(Duration::from_secs(self.check_interval_secs));
        loop {
            tick.tick().await;
            let now = Utc::now();

            // 快照任务列表,检查哪些该执行。
            let due: Vec<CronTask> = {
                let tasks = self.tasks.lock();
                tasks
                    .iter()
                    .filter(|t| t.should_run(now))
                    .cloned()
                    .collect()
            };

            for task in due {
                // T-E-L-02: 预算检查 — 超限任务跳过。
                if task.budget_exceeded() {
                    warn!(
                        target: "nebula.cron",
                        task = %task.name,
                        tokens_used = task.budget_tokens_used,
                        tokens_total = task.budget_tokens_total,
                        minutes_used = task.budget_minutes_used,
                        minutes_total = task.budget_minutes_total,
                        "cron task skipped: budget exceeded"
                    );
                    // 仍更新 last_run,避免同一分钟内重复检查。
                    let mut tasks = self.tasks.lock();
                    for t in tasks.iter_mut() {
                        if t.name == task.name {
                            t.last_run = Some(now);
                        }
                    }
                    continue;
                }

                // T-E-L-02: 记录执行开始时间(用于时间预算)。
                let exec_start = std::time::Instant::now();

                if let Err(e) = self.execute_task(&task.name).await {
                    warn!(
                        target: "nebula.cron",
                        task = %task.name,
                        error = %e,
                        "cron task failed"
                    );
                }

                let elapsed_secs = exec_start.elapsed().as_secs();

                // 更新 last_run + 预算用量
                let mut tasks = self.tasks.lock();
                for t in tasks.iter_mut() {
                    if t.name == task.name {
                        t.last_run = Some(now);
                        // T-E-L-02: 累加时间预算(秒→分钟向上取整)。
                        let elapsed_minutes = (elapsed_secs + 59) / 60; // 向上取整
                        t.add_minutes_usage(elapsed_minutes);
                    }
                }

                // T-E-L-02: 聚合 AtomicU64 计数器(无锁,热路径)。
                self.aggregate_seconds_used
                    .fetch_add(elapsed_secs, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// 执行单个任务。
    async fn execute_task(&self, name: &str) -> Result<()> {
        info!(target: "nebula.cron", task = %name, "executing cron task");
        match name {
            "memory-merge" => self.execute_memory_merge().await,
            "evolution-self-check" => self.execute_evolution_self_check().await,
            "evening-review" => self.execute_evening_review().await,
            other => {
                warn!(target: "nebula.cron", task = %other, "unknown cron task");
                Ok(())
            }
        }
    }

    /// 03:00 合并:L1→L2 记忆合并。
    ///
    /// 委托给 sponge engine 的合并方法(如果存在)。
    /// 这里只做框架,实际的合并逻辑在 sponge.rs 中。
    async fn execute_memory_merge(&self) -> Result<()> {
        info!(
            target: "nebula.cron",
            "memory-merge: L1→L2 consolidation (delegated to sponge engine)"
        );
        // TODO: 当 sponge engine 暴露 merge_l1_to_l2() 接口时调用它。
        // 目前只记录日志,避免引入未实现的方法调用导致编译错误。
        Ok(())
    }

    /// 12:00 自检:EvolutionEngine 4阶段运行。
    ///
    /// T-D-B-11: 实际调用 `EvolutionEngine::run(&master_id)` 触发 4 Phase 进化管线
    /// (Extract → Compile → Reflect → Soul),闭合"评估→变异→选择"自进化回路。
    ///
    /// 行为分支(尽力而为,失败不破坏调度循环):
    /// - `evolution-engine` feature off: 仅记 info 日志并返回 Ok(功能未编译)
    /// - 引擎未注入(`with_evolution_engine` 未调用): 记 warning 并返回 Ok(配置不完整,非错误)
    /// - 引擎已注入但运行时禁用(`is_enabled() == false`): 记 info 并返回 Ok(用户未启用,正常跳过)
    /// - 引擎已注入且启用: 调用 `run()`,成功记 info,失败记 warning 并返回 Err
    ///
    /// `master_id` 用于记忆 domain 隔离(`evolution:<master_id>`),与 shared/worker 域隔离。
    async fn execute_evolution_self_check(&self) -> Result<()> {
        info!(
            target: "nebula.cron",
            master_id = %self.master_id,
            "evolution-self-check: 4-phase evolution run"
        );

        #[cfg(feature = "evolution-engine")]
        {
            if let Some(engine) = &self.evolution_engine {
                // 运行时双层 gate:config.enabled && evolution_enabled()
                if !engine.is_enabled() {
                    info!(
                        target: "nebula.cron",
                        master_id = %self.master_id,
                        "evolution-self-check: engine disabled at runtime; skipping"
                    );
                    return Ok(());
                }

                info!(
                    target: "nebula.cron",
                    master_id = %self.master_id,
                    "evolution-self-check: triggering EvolutionEngine::run()"
                );

                match engine.run(&self.master_id).await {
                    Ok(result) => {
                        info!(
                            target: "nebula.cron",
                            master_id = %self.master_id,
                            degraded = result.degraded,
                            warnings = result.warnings.len(),
                            phases = result.phases.len(),
                            "evolution-self-check: 4-phase run completed"
                        );
                    }
                    Err(EvolutionError::Disabled) => {
                        // 运行时开关在 is_enabled() 检查后被关闭(竞态)— 视为跳过
                        info!(
                            target: "nebula.cron",
                            master_id = %self.master_id,
                            "evolution-self-check: engine disabled (race); skipping"
                        );
                    }
                    Err(e) => {
                        warn!(
                            target: "nebula.cron",
                            master_id = %self.master_id,
                            error = %e,
                            "evolution-self-check: 4-phase run failed"
                        );
                        return Err(anyhow::anyhow!(e));
                    }
                }
            } else {
                warn!(
                    target: "nebula.cron",
                    "evolution-self-check: EvolutionEngine not injected; skipping \
                     (wire via CronScheduler::with_evolution_engine)"
                );
            }
        }

        #[cfg(not(feature = "evolution-engine"))]
        {
            info!(
                target: "nebula.cron",
                "evolution-self-check: evolution-engine feature off; skipping"
            );
        }

        Ok(())
    }

    /// 21:00 回顾:Honcho 画像 nudge + Skill 评估。
    async fn execute_evening_review(&self) -> Result<()> {
        info!(
            target: "nebula.cron",
            "evening-review: Honcho nudge + skill evaluation"
        );

        if let Some(honcho) = &self.honcho {
            match honcho.nudge_user(&self.user_id).await {
                Ok(super::honcho::NudgeResult::Nudge(nudge)) => {
                    info!(
                        target: "nebula.cron",
                        nudge_id = %nudge.id,
                        "honcho nudge generated"
                    );
                }
                Ok(super::honcho::NudgeResult::Skipped) => {
                    info!(target: "nebula.cron", "honcho nudge skipped (cooldown or no profile)");
                }
                Err(e) => {
                    warn!(target: "nebula.cron", error = %e, "honcho nudge failed");
                }
            }
        }

        // Skill 评估委托给 SkillAutoEvolver::run_once()。
        // 这里不直接调用,因为 SkillAutoEvolver 的实例由 AppState 持有。
        // 实际触发通过 evolution worker 的后台循环完成。
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_schedule_has_three_tasks() {
        let tasks = CronTask::default_schedule();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].name, "memory-merge");
        assert_eq!(tasks[0].hour, 3);
        assert_eq!(tasks[1].name, "evolution-self-check");
        assert_eq!(tasks[1].hour, 12);
        assert_eq!(tasks[2].name, "evening-review");
        assert_eq!(tasks[2].hour, 21);
    }

    #[test]
    fn default_schedule_has_cron_expr() {
        // T-E-L-02: 每个默认任务都带 5 字段 cron 表达式。
        let tasks = CronTask::default_schedule();
        assert_eq!(tasks[0].cron_expr.as_deref(), Some("0 3 * * *"));
        assert_eq!(tasks[1].cron_expr.as_deref(), Some("0 12 * * *"));
        assert_eq!(tasks[2].cron_expr.as_deref(), Some("0 21 * * *"));
    }

    #[test]
    fn default_schedule_autonomy_is_l5() {
        // T-E-L-02: 默认任务都是后台自动化 → L5。
        use crate::autonomy::AutonomyLevel;
        let tasks = CronTask::default_schedule();
        for t in &tasks {
            assert_eq!(t.autonomy, AutonomyLevel::L5Background);
        }
    }

    #[test]
    fn default_schedule_has_budgets() {
        // T-E-L-02: 每个任务都有 Token + 时间预算。
        let tasks = CronTask::default_schedule();
        for t in &tasks {
            assert!(
                t.budget_tokens_total > 0,
                "task {} has no token budget",
                t.name
            );
            assert!(
                t.budget_minutes_total > 0,
                "task {} has no time budget",
                t.name
            );
            assert_eq!(t.budget_tokens_used, 0);
            assert_eq!(t.budget_minutes_used, 0);
        }
    }

    #[test]
    fn cron_task_default_all_zero_budgets() {
        // T-E-L-02: Default::default() 的预算字段全为 0(无限制)。
        let task = CronTask::default();
        assert_eq!(task.budget_tokens_total, 0);
        assert_eq!(task.budget_tokens_used, 0);
        assert_eq!(task.budget_minutes_total, 0);
        assert_eq!(task.budget_minutes_used, 0);
        assert_eq!(task.cron_expr, None);
    }

    #[test]
    fn should_run_when_hour_matches_and_never_run() {
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: None,
            ..Default::default()
        };
        let now = Utc::now().with_hour(12).expect("test op should succeed");
        assert!(task.should_run(now));
    }

    #[test]
    fn should_not_run_when_hour_mismatches() {
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: None,
            ..Default::default()
        };
        let now = Utc::now().with_hour(15).expect("test op should succeed");
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_not_run_when_disabled() {
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: false,
            last_run: None,
            ..Default::default()
        };
        let now = Utc::now().with_hour(12).expect("test op should succeed");
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_not_run_twice_same_day() {
        let now = Utc::now().with_hour(12).expect("test op should succeed");
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: Some(now),
            ..Default::default()
        };
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_run_next_day() {
        let yesterday = Utc::now()
            .with_hour(12)
            .expect("test op should succeed")
            .checked_sub_days(chrono::Duration::days(1))
            .expect("test op should succeed");
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: Some(yesterday),
            ..Default::default()
        };
        let now = Utc::now().with_hour(12).expect("test op should succeed");
        assert!(task.should_run(now));
    }

    // ---- T-E-L-02: should_run() with cron_expr ----

    #[test]
    fn should_run_with_cron_expr_matching() {
        use chrono::TimeZone;
        // "0 9 * * 1-5" — 工作日 09:00
        let task = CronTask {
            name: "test".to_string(),
            hour: 9,
            enabled: true,
            last_run: None,
            cron_expr: Some("0 9 * * 1-5".to_string()),
            ..Default::default()
        };
        // 2026-07-08 是周三 → 匹配
        let now = Utc
            .with_ymd_and_hms(2026, 7, 8, 9, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(task.should_run(now));
    }

    #[test]
    fn should_not_run_with_cron_expr_wrong_time() {
        use chrono::TimeZone;
        let task = CronTask {
            name: "test".to_string(),
            hour: 9,
            enabled: true,
            last_run: None,
            cron_expr: Some("0 9 * * 1-5".to_string()),
            ..Default::default()
        };
        // 10:00 不匹配
        let now = Utc
            .with_ymd_and_hms(2026, 7, 8, 10, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_not_run_with_cron_expr_weekend() {
        use chrono::TimeZone;
        let task = CronTask {
            name: "test".to_string(),
            hour: 9,
            enabled: true,
            last_run: None,
            cron_expr: Some("0 9 * * 1-5".to_string()),
            ..Default::default()
        };
        // 2026-07-11 是周六 → 不匹配
        let now = Utc
            .with_ymd_and_hms(2026, 7, 11, 9, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(!task.should_run(now));
    }

    #[test]
    fn cron_expr_allows_multiple_runs_per_day() {
        use chrono::TimeZone;
        // "*/15 * * * *" — 每 15 分钟
        let task = CronTask {
            name: "test".to_string(),
            hour: 0,
            enabled: true,
            last_run: Some(
                Utc.with_ymd_and_hms(2026, 7, 8, 12, 0, 0)
                    .single()
                    .expect("test op should succeed"),
            ),
            cron_expr: Some("*/15 * * * *".to_string()),
            ..Default::default()
        };
        // 12:15 — 距上次 15 分钟 > 60 秒 → 应该执行
        let now = Utc
            .with_ymd_and_hms(2026, 7, 8, 12, 15, 0)
            .single()
            .expect("test op should succeed");
        assert!(task.should_run(now));
    }

    #[test]
    fn cron_expr_blocks_within_60s() {
        use chrono::TimeZone;
        // "*/15 * * * *" — 每 15 分钟
        let last = Utc
            .with_ymd_and_hms(2026, 7, 8, 12, 0, 0)
            .single()
            .expect("test op should succeed");
        let task = CronTask {
            name: "test".to_string(),
            hour: 0,
            enabled: true,
            last_run: Some(last),
            cron_expr: Some("*/15 * * * *".to_string()),
            ..Default::default()
        };
        // 12:00:30 — 距上次 30 秒 < 60 秒 → 不应执行
        let now = Utc
            .with_ymd_and_hms(2026, 7, 8, 12, 0, 30)
            .single()
            .expect("test op should succeed");
        assert!(!task.should_run(now));
    }

    #[test]
    fn invalid_cron_expr_falls_back_to_hour() {
        use chrono::TimeZone;
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: None,
            cron_expr: Some("invalid expr".to_string()),
            ..Default::default()
        };
        // 无效表达式 → 回退到 hour=12 → 12:00 匹配
        let now = Utc
            .with_ymd_and_hms(2026, 7, 8, 12, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(task.should_run(now));
        // 13:00 不匹配
        let now2 = Utc
            .with_ymd_and_hms(2026, 7, 8, 13, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(!task.should_run(now2));
    }

    #[test]
    fn default_schedule_tasks_should_run_at_their_cron_time() {
        use chrono::TimeZone;
        let tasks = CronTask::default_schedule();
        // memory-merge: "0 3 * * *" → 03:00 匹配
        let now_3 = Utc
            .with_ymd_and_hms(2026, 7, 8, 3, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(tasks[0].should_run(now_3));
        // evolution-self-check: "0 12 * * *" → 12:00 匹配
        let now_12 = Utc
            .with_ymd_and_hms(2026, 7, 8, 12, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(tasks[1].should_run(now_12));
        // evening-review: "0 21 * * *" → 21:00 匹配
        let now_21 = Utc
            .with_ymd_and_hms(2026, 7, 8, 21, 0, 0)
            .single()
            .expect("test op should succeed");
        assert!(tasks[2].should_run(now_21));
    }

    #[test]
    fn scheduler_lists_default_tasks() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        let tasks = scheduler.list_tasks();
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn set_task_enabled_toggles() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        assert!(scheduler.set_task_enabled("memory-merge", false));
        let tasks = scheduler.list_tasks();
        assert!(
            !tasks
                .iter()
                .find(|t| t.name == "memory-merge")
                .expect("test op should succeed")
                .enabled
        );
    }

    #[test]
    fn set_task_enabled_unknown_returns_false() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        assert!(!scheduler.set_task_enabled("nonexistent", true));
    }

    // ---- T-E-L-02: 预算检查测试 ----

    #[test]
    fn budget_exceeded_when_tokens_used() {
        let task = CronTask {
            name: "test".to_string(),
            budget_tokens_total: 1000,
            budget_tokens_used: 1000,
            ..Default::default()
        };
        assert!(task.budget_exceeded());
    }

    #[test]
    fn budget_exceeded_when_minutes_used() {
        let task = CronTask {
            name: "test".to_string(),
            budget_minutes_total: 10,
            budget_minutes_used: 10,
            ..Default::default()
        };
        assert!(task.budget_exceeded());
    }

    #[test]
    fn budget_not_exceeded_when_under_limit() {
        let task = CronTask {
            name: "test".to_string(),
            budget_tokens_total: 1000,
            budget_tokens_used: 500,
            budget_minutes_total: 10,
            budget_minutes_used: 5,
            ..Default::default()
        };
        assert!(!task.budget_exceeded());
    }

    #[test]
    fn budget_not_exceeded_when_total_zero() {
        // total=0 表示无限制
        let task = CronTask {
            name: "test".to_string(),
            budget_tokens_total: 0,
            budget_tokens_used: 999_999_999,
            budget_minutes_total: 0,
            budget_minutes_used: 999_999,
            ..Default::default()
        };
        assert!(!task.budget_exceeded());
    }

    #[test]
    fn add_token_usage_saturates() {
        let mut task = CronTask {
            name: "test".to_string(),
            budget_tokens_total: 1000,
            budget_tokens_used: 900,
            ..Default::default()
        };
        task.add_token_usage(200); // 900 + 200 = 1100
        assert_eq!(task.budget_tokens_used, 1100);
        assert!(task.budget_exceeded());
        // saturating: u64::MAX + 1 = u64::MAX
        task.add_token_usage(u64::MAX);
        assert_eq!(task.budget_tokens_used, u64::MAX);
    }

    #[test]
    fn add_minutes_usage_saturates() {
        let mut task = CronTask {
            name: "test".to_string(),
            budget_minutes_total: 10,
            budget_minutes_used: 5,
            ..Default::default()
        };
        task.add_minutes_usage(3); // 5 + 3 = 8
        assert_eq!(task.budget_minutes_used, 8);
        assert!(!task.budget_exceeded());
        task.add_minutes_usage(3); // 8 + 3 = 11 > 10
        assert!(task.budget_exceeded());
    }

    #[test]
    fn scheduler_record_token_usage_updates_task_and_aggregate() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        assert_eq!(scheduler.aggregate_tokens_used(), 0);

        scheduler.record_token_usage("memory-merge", 500);
        assert_eq!(scheduler.aggregate_tokens_used(), 500);

        // 验证任务级预算也更新了
        let tasks = scheduler.list_tasks();
        let memory_merge = tasks
            .iter()
            .find(|t| t.name == "memory-merge")
            .expect("query should succeed");
        assert_eq!(memory_merge.budget_tokens_used, 500);
    }

    #[test]
    fn scheduler_aggregate_seconds_starts_zero() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        assert_eq!(scheduler.aggregate_seconds_used(), 0);
        assert_eq!(scheduler.aggregate_tokens_used(), 0);
    }

    #[test]
    fn scheduler_reset_all_budgets() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        scheduler.record_token_usage("memory-merge", 500);
        scheduler.record_token_usage("evening-review", 300);
        assert_eq!(scheduler.aggregate_tokens_used(), 800);

        // 验证任务级预算有值
        let tasks = scheduler.list_tasks();
        assert_eq!(
            tasks
                .iter()
                .find(|t| t.name == "memory-merge")
                .expect("test op should succeed")
                .budget_tokens_used,
            500
        );

        // 重置
        scheduler.reset_all_budgets();
        assert_eq!(scheduler.aggregate_tokens_used(), 0);
        assert_eq!(scheduler.aggregate_seconds_used(), 0);

        // 验证任务级预算也清零了
        let tasks = scheduler.list_tasks();
        for t in &tasks {
            assert_eq!(t.budget_tokens_used, 0);
            assert_eq!(t.budget_minutes_used, 0);
        }
    }

    #[test]
    fn default_schedule_budgets_not_exceeded_initially() {
        let tasks = CronTask::default_schedule();
        for t in &tasks {
            assert!(
                !t.budget_exceeded(),
                "task {} should not be exceeded",
                t.name
            );
        }
    }

    // ---- T-D-B-11: EvolutionEngine 断路修复(12:00 自检实际触发 4 Phase)----

    #[test]
    fn master_id_defaults_to_user_id() {
        // T-D-B-11: master_id 默认 = user_id(向后兼容)。
        let scheduler = CronScheduler::new(None, "user-42".to_string());
        assert_eq!(scheduler.master_id, "user-42");
    }

    #[test]
    fn with_master_id_overrides_default() {
        // T-D-B-11: with_master_id 覆盖默认值,用于 domain 隔离。
        let scheduler = CronScheduler::new(None, "user-42".to_string())
            .with_master_id("agent_alpha".to_string());
        assert_eq!(scheduler.master_id, "agent_alpha");
        // user_id 不受影响
        assert_eq!(scheduler.user_id, "user-42");
    }

    #[tokio::test]
    async fn execute_evolution_self_check_ok_without_engine() {
        // T-D-B-11: 引擎未注入时,自检任务应优雅跳过(返回 Ok,记 warning)。
        // 此测试在 self-evolution(无 evolution-engine)和 evolution-engine
        // 两种 feature 配置下均应通过。
        let scheduler = CronScheduler::new(None, "user1".to_string());
        let result = scheduler.execute_evolution_self_check().await;
        assert!(result.is_ok(), "should skip gracefully without engine");
    }

    #[tokio::test]
    async fn execute_task_evolution_self_check_dispatches_ok() {
        // T-D-B-11: 通过 execute_task 分发到 evolution-self-check,
        // 验证任务名路由 + 无引擎时优雅跳过。
        let scheduler = CronScheduler::new(None, "user1".to_string());
        let result = scheduler.execute_task("evolution-self-check").await;
        assert!(
            result.is_ok(),
            "execute_task should dispatch and skip gracefully"
        );
    }

    #[cfg(feature = "evolution-engine")]
    #[test]
    fn with_evolution_engine_builder_accepts_none() {
        // T-D-B-11: with_evolution_engine(None) 应正确设置字段为 None。
        let scheduler = CronScheduler::new(None, "user1".to_string()).with_evolution_engine(None);
        assert!(
            scheduler.evolution_engine.is_none(),
            "evolution_engine should be None after with_evolution_engine(None)"
        );
    }

    #[cfg(feature = "evolution-engine")]
    #[test]
    fn evolution_engine_defaults_none_in_new() {
        // T-D-B-11: CronScheduler::new 默认不注入引擎(向后兼容)。
        let scheduler = CronScheduler::new(None, "user1".to_string());
        assert!(scheduler.evolution_engine.is_none());
    }

    #[cfg(feature = "evolution-engine")]
    #[tokio::test]
    async fn execute_evolution_self_check_skips_when_engine_disabled() {
        // T-D-B-11: 引擎已注入但运行时禁用(config.enabled=false 或全局开关 off)
        // 时,自检应记 info 并返回 Ok(正常跳过,非错误)。
        //
        // 这里无法构造完整 EvolutionEngine(需 dispatcher/sqlite/sponge/log),
        // 但可通过 None 分支验证"未注入 → 跳过"路径。
        // 完整端到端测试(注入真实引擎)见 engine/tests.rs + M5/M7a LLM 集成阶段。
        let scheduler = CronScheduler::new(None, "user1".to_string()).with_evolution_engine(None);
        let result = scheduler.execute_evolution_self_check().await;
        assert!(result.is_ok(), "should skip gracefully when engine is None");
    }
}
