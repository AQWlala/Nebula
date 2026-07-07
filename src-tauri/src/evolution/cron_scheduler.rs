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

/// Cron 任务定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTask {
    /// 任务名称(唯一标识)。
    pub name: String,
    /// 每天执行的小时(0-23)。
    pub hour: u32,
    /// 任务是否启用。
    pub enabled: bool,
    /// 上次执行时间(UTC)。
    pub last_run: Option<DateTime<Utc>>,
}

impl CronTask {
    /// 默认三计时任务表。
    pub fn default_schedule() -> Vec<CronTask> {
        vec![
            CronTask {
                name: "memory-merge".to_string(),
                hour: 3,
                enabled: true,
                last_run: None,
            },
            CronTask {
                name: "evolution-self-check".to_string(),
                hour: 12,
                enabled: true,
                last_run: None,
            },
            CronTask {
                name: "evening-review".to_string(),
                hour: 21,
                enabled: true,
                last_run: None,
            },
        ]
    }

    /// 检查任务是否应该执行(当前小时匹配且当天未执行过)。
    fn should_run(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        if now.hour() != self.hour {
            return false;
        }
        match self.last_run {
            None => true,
            Some(last) => {
                // 如果上次执行在同一天,则跳过。
                last.date_naive() != now.date_naive()
            }
        }
    }
}

/// Cron 调度器。
///
/// 每 60 秒检查一次任务表,到达预定时间则触发对应任务。
/// 任务执行是"尽力而为":失败记录 warning 但不中断调度循环。
pub struct CronScheduler {
    tasks: Arc<Mutex<Vec<CronTask>>>,
    honcho: Option<Arc<HonchoEngine>>,
    /// 用户 ID(用于 Honcho nudge)。
    user_id: String,
    /// 检查间隔(秒,默认 60)。
    check_interval_secs: u64,
    /// 是否正在运行。
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl CronScheduler {
    pub fn new(honcho: Option<Arc<HonchoEngine>>, user_id: String) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(CronTask::default_schedule())),
            honcho,
            user_id,
            check_interval_secs: 60,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 自定义检查间隔(主要用于测试)。
    pub fn with_check_interval(mut self, secs: u64) -> Self {
        self.check_interval_secs = secs;
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
                if let Err(e) = self.execute_task(&task.name).await {
                    warn!(
                        target: "nebula.cron",
                        task = %task.name,
                        error = %e,
                        "cron task failed"
                    );
                }

                // 更新 last_run
                let mut tasks = self.tasks.lock();
                for t in tasks.iter_mut() {
                    if t.name == task.name {
                        t.last_run = Some(now);
                    }
                }
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
    /// 委托给 EvolutionEngine::run()。
    async fn execute_evolution_self_check(&self) -> Result<()> {
        info!(
            target: "nebula.cron",
            "evolution-self-check: 4-phase evolution run (delegated to EvolutionEngine)"
        );
        // TODO: 当 AppState 注入 EvolutionEngine 后,通过 state.evolution_engine.run() 触发。
        // 目前只记录日志,实际的引擎调用通过 evolution_run Tauri 命令手动触发。
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
    fn should_run_when_hour_matches_and_never_run() {
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: None,
        };
        let now = Utc::now().with_hour(12).unwrap();
        assert!(task.should_run(now));
    }

    #[test]
    fn should_not_run_when_hour_mismatches() {
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: None,
        };
        let now = Utc::now().with_hour(15).unwrap();
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_not_run_when_disabled() {
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: false,
            last_run: None,
        };
        let now = Utc::now().with_hour(12).unwrap();
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_not_run_twice_same_day() {
        let now = Utc::now().with_hour(12).unwrap();
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: Some(now),
        };
        assert!(!task.should_run(now));
    }

    #[test]
    fn should_run_next_day() {
        let yesterday = Utc::now()
            .with_hour(12)
            .unwrap()
            .checked_sub_days(chrono::Duration::days(1))
            .unwrap();
        let task = CronTask {
            name: "test".to_string(),
            hour: 12,
            enabled: true,
            last_run: Some(yesterday),
        };
        let now = Utc::now().with_hour(12).unwrap();
        assert!(task.should_run(now));
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
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn set_task_enabled_unknown_returns_false() {
        let scheduler = CronScheduler::new(None, "user1".to_string());
        assert!(!scheduler.set_task_enabled("nonexistent", true));
    }
}
