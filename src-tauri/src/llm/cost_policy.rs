//! M5 任务 #71: CostPolicy 统一 — 任务级 + 每日双上限，本地调用不计入双上限。
//!
//! ## 设计
//!
//! - **max_tokens_per_task**：单次任务（一次 chat / 一次 master orchestration）的
//!   总 token 上限。`check_task_limit(used, added)` 返回 `CostDecision`。
//! - **daily_task_limit**：当日远端 LLM 调用次数上限。`check_daily_limit(count)`
//!   返回 `CostDecision`。**仅远端调用计入**（本地 Ollama 零成本，不计入）。
//!
//! ## 本地不计入双上限
//!
//! `is_local = true` 时，[`CostPolicy::check`] 直接返回 `Allow`，
//! 不消耗任何配额。这与 [`CostTracker::record`] 不同（CostTracker 仍记录
//! 本地调用，只是 model_price 返回 0），与 `WorkType::is_local_only`
//! （强制本地路由）配合：
//!
//! | WorkType          | is_local_only | 计入 daily_task_limit |
//! |------------------|--------------|----------------------|
//! | Evolution / Soul / Classifier | ✅ | ❌（永远本地） |
//! | Chat / MasterTask / SwarmSynthesize | ❌ | ✅（远端时计入） |
//! | SwarmWorker | ❌ | ✅（远端时计入；本地时不计入） |
//!
//! ## 与 CostTracker 的关系
//!
//! `CostPolicy` 是**预算门禁**（dispatch 前检查，是否允许调用），
//! `CostTracker` 是**事后记录**（调用完后记账）。两者独立：
//!
//! - CostTracker 仍记录所有调用（含本地，用于审计）
//! - CostPolicy 只关心是否允许远端调用（门禁逻辑）
//!
//! ## 不做的事
//!
//! - **不持久化**：policy 是进程内运行时状态，重启从配置文件重读。
//! - **不做预算告警 emit**：那是 `CostTracker::with_budget_alert` 的职责。
//!
//! ## 不依赖 dispatcher 模块
//!
//! 为避免 cfg-gate 依赖（dispatcher 仅在 `unified-dispatcher` feature
//! 开启时编译），本模块不直接引用 `WorkType`。调用方（dispatcher.rs）
//! 自行把 `WorkType::is_local_only()` 转为 `is_local_only_work_type: bool`
//! 传入，保持 cost_policy 在最小构建中也可用。

use serde::{Deserialize, Serialize};

/// CostPolicy 检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostDecision {
    /// 允许调用。
    Allow,
    /// 任务级 token 上限已达。
    TaskLimitExceeded {
        used: u64,
        added: u64,
        limit: u64,
    },
    /// 每日远端调用次数上限已达。
    DailyLimitExceeded {
        today_count: u32,
        limit: u32,
    },
}

impl CostDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, CostDecision::Allow)
    }
    pub fn is_denied(&self) -> bool {
        !self.is_allow()
    }
}

/// CostPolicy 配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPolicy {
    /// 单任务 token 上限（input + output）。None 或 0 表示不限制。
    pub max_tokens_per_task: Option<u64>,
    /// 每日远端调用次数上限。None 或 0 表示不限制。
    pub daily_task_limit: Option<u32>,
}

impl Default for CostPolicy {
    fn default() -> Self {
        Self::unlimited()
    }
}

impl CostPolicy {
    /// 无限制（默认）。
    pub fn unlimited() -> Self {
        Self {
            max_tokens_per_task: None,
            daily_task_limit: None,
        }
    }

    /// builder 风格设置单任务 token 上限。
    pub fn with_max_tokens_per_task(mut self, n: u64) -> Self {
        self.max_tokens_per_task = if n > 0 { Some(n) } else { None };
        self
    }

    /// builder 风格设置每日远端调用上限。
    pub fn with_daily_task_limit(mut self, n: u32) -> Self {
        self.daily_task_limit = if n > 0 { Some(n) } else { None };
        self
    }

    /// 检查本次调用是否允许。
    ///
    /// - `is_local`：本次实际路由是否本地（与 `WorkType::is_local_only` 可不同——
    ///   SwarmWorker 可能在本地 Ollama，也可能远端）
    /// - `is_local_only_work_type`：work_type 是否强制本地（Evolution/Soul/Classifier）。
    ///   调用方传入 `work_type.is_local_only()`，避免本模块依赖 dispatcher。
    /// - `task_used_tokens`：当前任务已用 token 数（input + output 累计）
    /// - `task_added_tokens`：本次预计增加 token 数（预估）
    /// - `today_remote_count`：当日已发生的远端调用次数
    pub fn check(
        &self,
        is_local: bool,
        is_local_only_work_type: bool,
        task_used_tokens: u64,
        task_added_tokens: u64,
        today_remote_count: u32,
    ) -> CostDecision {
        // 本地调用永远放行（不计入双上限）
        if is_local {
            return CostDecision::Allow;
        }
        // 强制本地路由的 WorkType（Evolution/Soul/Classifier）— 即使
        // is_local=false（理论不会发生，因 dispatcher.resolve 强制本地），
        // 也按本地处理，避免误计费。
        if is_local_only_work_type {
            return CostDecision::Allow;
        }
        // 1. 任务级 token 上限
        if let Some(limit) = self.max_tokens_per_task {
            if limit > 0 && task_used_tokens + task_added_tokens > limit {
                return CostDecision::TaskLimitExceeded {
                    used: task_used_tokens,
                    added: task_added_tokens,
                    limit,
                };
            }
        }
        // 2. 每日远端调用上限
        if let Some(limit) = self.daily_task_limit {
            if limit > 0 && today_remote_count >= limit {
                return CostDecision::DailyLimitExceeded {
                    today_count: today_remote_count,
                    limit,
                };
            }
        }
        CostDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助：模拟 Chat work_type（非强制本地）
    const IS_LOCAL_ONLY_FALSE: bool = false;
    /// 辅助：模拟 Evolution/Soul/Classifier work_type（强制本地）
    const IS_LOCAL_ONLY_TRUE: bool = true;

    #[test]
    fn unlimited_policy_always_allows() {
        let p = CostPolicy::unlimited();
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 1_000_000, 1_000_000, 9999);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn local_call_always_allows_regardless_of_limits() {
        let p = CostPolicy::unlimited()
            .with_max_tokens_per_task(1000)
            .with_daily_task_limit(5);
        // is_local=true → 放行（即使超 token 上限）
        let d = p.check(true, IS_LOCAL_ONLY_FALSE, 100_000, 100_000, 9999);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn local_only_worktype_always_allows() {
        let p = CostPolicy::unlimited()
            .with_max_tokens_per_task(1000)
            .with_daily_task_limit(5);
        // Evolution / SoulCompile / Classifier 永远放行
        let d = p.check(false, IS_LOCAL_ONLY_TRUE, 100_000, 100_000, 9999);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn task_limit_exceeded() {
        let p = CostPolicy::unlimited().with_max_tokens_per_task(10_000);
        // used=8000 + added=3000 = 11000 > 10000
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 8000, 3000, 0);
        match d {
            CostDecision::TaskLimitExceeded {
                used,
                added,
                limit,
            } => {
                assert_eq!(used, 8000);
                assert_eq!(added, 3000);
                assert_eq!(limit, 10000);
            }
            other => panic!("expected TaskLimitExceeded, got {other:?}"),
        }
    }

    #[test]
    fn task_limit_not_exceeded_when_under() {
        let p = CostPolicy::unlimited().with_max_tokens_per_task(10_000);
        // used=5000 + added=3000 = 8000 < 10000 → Allow
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 5000, 3000, 0);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn daily_limit_exceeded() {
        let p = CostPolicy::unlimited().with_daily_task_limit(100);
        // today_count=100 >= 100 → 拒绝
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 0, 0, 100);
        match d {
            CostDecision::DailyLimitExceeded {
                today_count,
                limit,
            } => {
                assert_eq!(today_count, 100);
                assert_eq!(limit, 100);
            }
            other => panic!("expected DailyLimitExceeded, got {other:?}"),
        }
    }

    #[test]
    fn daily_limit_zero_means_unlimited() {
        // with_daily_task_limit(0) → None → 不限制
        let p = CostPolicy::unlimited().with_daily_task_limit(0);
        assert!(p.daily_task_limit.is_none());
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 0, 0, u32::MAX);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn max_tokens_zero_means_unlimited() {
        let p = CostPolicy::unlimited().with_max_tokens_per_task(0);
        assert!(p.max_tokens_per_task.is_none());
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, u64::MAX, u64::MAX, 0);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn task_limit_checked_before_daily_limit() {
        // 两个上限都设，task 优先返回（task_used_tokens 超）
        let p = CostPolicy::unlimited()
            .with_max_tokens_per_task(1000)
            .with_daily_task_limit(10);
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 10_000, 1, 0);
        assert!(matches!(d, CostDecision::TaskLimitExceeded { .. }));
    }

    #[test]
    fn swarm_worker_local_allows() {
        // SwarmWorker 不在 is_local_only 中，但当 is_local=true 时仍放行
        let p = CostPolicy::unlimited()
            .with_max_tokens_per_task(1000)
            .with_daily_task_limit(5);
        let d = p.check(true, IS_LOCAL_ONLY_FALSE, 100_000, 100_000, 9999);
        assert_eq!(d, CostDecision::Allow);
    }

    #[test]
    fn swarm_worker_remote_checks_limits() {
        let p = CostPolicy::unlimited()
            .with_max_tokens_per_task(1000)
            .with_daily_task_limit(5);
        // 远端 SwarmWorker 走限制检查
        let d = p.check(false, IS_LOCAL_ONLY_FALSE, 2000, 100, 0);
        assert!(matches!(d, CostDecision::TaskLimitExceeded { .. }));
    }

    #[test]
    fn decision_serde_roundtrip() {
        let cases = vec![
            CostDecision::Allow,
            CostDecision::TaskLimitExceeded {
                used: 100,
                added: 200,
                limit: 250,
            },
            CostDecision::DailyLimitExceeded {
                today_count: 50,
                limit: 50,
            },
        ];
        for c in cases {
            let s = serde_json::to_string(&c).unwrap();
            let back: CostDecision = serde_json::from_str(&s).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn decision_helpers() {
        assert!(CostDecision::Allow.is_allow());
        assert!(!CostDecision::Allow.is_denied());
        assert!(CostDecision::TaskLimitExceeded {
            used: 1,
            added: 1,
            limit: 1
        }
        .is_denied());
        assert!(CostDecision::DailyLimitExceeded {
            today_count: 1,
            limit: 1
        }
        .is_denied());
    }

    #[test]
    fn default_is_unlimited() {
        let p = CostPolicy::default();
        assert!(p.max_tokens_per_task.is_none());
        assert!(p.daily_task_limit.is_none());
    }
}
