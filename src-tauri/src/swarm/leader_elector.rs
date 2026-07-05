//! T-S4-A-01: 领导轮值制 — 加权随机轮值算法。
//!
//! 根据 EXPERT_REVIEW §4.3 决议，不引入 Raft 共识算法，改用加权随机
//! 轮值。每个任务开始时根据 agent 的能力评分、历史成功率和当前负载
//! 计算综合分数，按分数加权随机选出 Leader。
//!
//! ## 评分公式
//!
//! ```text
//! score = capability_score * 0.5 + history_success_rate * 0.3 + (1 - current_load) * 0.2
//! ```
//!
//! * `capability_score` — agent 能力评分 [0, 1]，默认 0.5
//! * `history_success_rate` — 历史任务成功率 = successful / total
//! * `current_load` — 当前负载 [0, 1]，0 = 空闲，1 = 满载
//!
//! ## Leader 职责
//!
//! * 负责最终决策
//! * 触发协商（当多个 agent 输出冲突时）
//! * 其输出在协商阶段享有更高权重

use std::collections::HashMap;

use parking_lot::Mutex;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

/// 权重常量 — 来自 EXPERT_REVIEW §4.3 决议。
const W_CAPABILITY: f64 = 0.5;
const W_SUCCESS_RATE: f64 = 0.3;
const W_LOAD: f64 = 0.2;

/// 单个 agent 的统计信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentStats {
    /// 能力评分 [0, 1]，默认 0.5。可由外部系统更新。
    capability_score: f64,
    /// 总任务数。
    total_tasks: u32,
    /// 成功任务数。
    successful_tasks: u32,
    /// 当前负载 [0, 1]。0 = 空闲，1 = 满载。
    current_load: f64,
}

impl Default for AgentStats {
    fn default() -> Self {
        Self {
            capability_score: 0.5,
            total_tasks: 0,
            successful_tasks: 0,
            current_load: 0.0,
        }
    }
}

impl AgentStats {
    /// 历史成功率 = successful / total（无任务时返回 1.0 以避免新 agent 被低估）。
    fn success_rate(&self) -> f64 {
        if self.total_tasks == 0 {
            1.0
        } else {
            self.successful_tasks as f64 / self.total_tasks as f64
        }
    }

    /// 计算综合分数。
    fn score(&self) -> f64 {
        let cap = self.capability_score.clamp(0.0, 1.0);
        let sr = self.success_rate();
        let load = self.current_load.clamp(0.0, 1.0);
        cap * W_CAPABILITY + sr * W_SUCCESS_RATE + (1.0 - load) * W_LOAD
    }
}

/// 领导轮值选举器。
///
/// 维护所有 agent 的统计信息，在每次任务开始时通过加权随机算法
/// 选出 Leader。Leader 负责最终决策和协商触发。
pub struct LeaderElector {
    stats: Mutex<HashMap<String, AgentStats>>,
    /// 上一次选出的 Leader 名称（供调试和监控）。
    current_leader: Mutex<Option<String>>,
}

impl LeaderElector {
    pub fn new() -> Self {
        Self {
            stats: Mutex::new(HashMap::new()),
            current_leader: Mutex::new(None),
        }
    }

    /// 注册一个新 agent（如果已存在则不覆盖）。
    pub fn register(&self, agent_name: &str) {
        let mut stats = self.stats.lock();
        stats.entry(agent_name.to_string()).or_default();
    }

    /// 注销 agent。
    pub fn unregister(&self, agent_name: &str) {
        self.stats.lock().remove(agent_name);
    }

    /// 更新 agent 的能力评分。
    pub fn set_capability(&self, agent_name: &str, score: f64) {
        let mut stats = self.stats.lock();
        let entry = stats.entry(agent_name.to_string()).or_default();
        entry.capability_score = score.clamp(0.0, 1.0);
    }

    /// 更新 agent 的当前负载。
    pub fn update_load(&self, agent_name: &str, load: f64) {
        let mut stats = self.stats.lock();
        let entry = stats.entry(agent_name.to_string()).or_default();
        entry.current_load = load.clamp(0.0, 1.0);
    }

    /// 记录任务结果（成功或失败）。
    #[instrument(skip(self), fields(agent = %agent_name, success = success))]
    pub fn record_outcome(&self, agent_name: &str, success: bool) {
        let mut stats = self.stats.lock();
        let entry = stats.entry(agent_name.to_string()).or_default();
        entry.total_tasks += 1;
        if success {
            entry.successful_tasks += 1;
        }
    }

    /// 获取 agent 的当前分数。
    pub fn get_score(&self, agent_name: &str) -> f64 {
        let stats = self.stats.lock();
        stats
            .get(agent_name)
            .map(|s| s.score())
            .unwrap_or(0.0)
    }

    /// 获取所有已注册 agent 的名称和分数。
    pub fn list_scores(&self) -> Vec<(String, f64)> {
        let stats = self.stats.lock();
        stats
            .iter()
            .map(|(name, s)| (name.clone(), s.score()))
            .collect()
    }

    /// 加权随机选举 Leader。
    ///
    /// 算法：
    /// 1. 为每个候选 agent 计算分数
    /// 2. 将分数作为权重，执行加权随机选择
    /// 3. 返回被选中的 agent 名称
    ///
    /// 如果所有 agent 分数都为 0，则均匀随机选择。
    /// 如果候选列表为空，返回 `None`。
    pub fn elect(&self, candidates: &[String]) -> Option<String> {
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            let name = candidates[0].clone();
            *self.current_leader.lock() = Some(name.clone());
            return Some(name);
        }

        let stats = self.stats.lock();

        // 计算每个候选的分数
        let scores: Vec<(String, f64)> = candidates
            .iter()
            .map(|name| {
                let score = stats.get(name).map(|s| s.score()).unwrap_or(0.0);
                (name.clone(), score.max(0.0))
            })
            .collect();

        drop(stats); // 释放锁

        let total_weight: f64 = scores.iter().map(|(_, s)| *s).sum();

        let elected = if total_weight <= 0.0 {
            // 所有分数为 0，均匀随机选择
            let idx = rand::thread_rng().gen_range(0..candidates.len());
            candidates[idx].clone()
        } else {
            // 加权随机选择
            let mut rng = rand::thread_rng();
            let mut pick = rng.gen_range(0.0..total_weight);
            let mut chosen = &candidates[0];
            for (name, score) in &scores {
                pick -= score;
                if pick <= 0.0 {
                    chosen = name;
                    break;
                }
            }
            chosen.clone()
        };

        info!(
            target: "nebula.swarm.leader",
            leader = %elected,
            candidates = candidates.len(),
            "leader elected"
        );

        *self.current_leader.lock() = Some(elected.clone());
        Some(elected)
    }

    /// 获取当前 Leader 名称。
    pub fn current_leader(&self) -> Option<String> {
        self.current_leader.lock().clone()
    }
}

impl Default for LeaderElector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elect_single_candidate() {
        let elector = LeaderElector::new();
        let result = elector.elect(&["Agent-1".to_string()]);
        assert_eq!(result.as_deref(), Some("Agent-1"));
    }

    #[test]
    fn elect_empty_returns_none() {
        let elector = LeaderElector::new();
        assert!(elector.elect(&[]).is_none());
    }

    #[test]
    fn elect_multiple_candidates() {
        let elector = LeaderElector::new();
        elector.set_capability("Agent-1", 0.9);
        elector.set_capability("Agent-2", 0.5);
        elector.set_capability("Agent-3", 0.3);

        let result = elector.elect(&[
            "Agent-1".to_string(),
            "Agent-2".to_string(),
            "Agent-3".to_string(),
        ]);
        assert!(result.is_some());
        assert!(elector.current_leader().is_some());
    }

    #[test]
    fn record_outcome_updates_stats() {
        let elector = LeaderElector::new();
        elector.register("Agent-1");

        elector.record_outcome("Agent-1", true);
        elector.record_outcome("Agent-1", true);
        elector.record_outcome("Agent-1", false);

        let scores = elector.list_scores();
        assert_eq!(scores.len(), 1);
        // success_rate = 2/3 ≈ 0.667
        // score = 0.5*0.5 + 0.667*0.3 + 1.0*0.2 = 0.25 + 0.2 + 0.2 = 0.65
        let score = scores[0].1;
        assert!((0.64..0.66).contains(&score), "score was {score}");
    }

    #[test]
    fn higher_capability_wins_more_often() {
        // 统计测试：高能力 agent 应该被选中更多次
        let elector = LeaderElector::new();
        elector.set_capability("high", 1.0);
        elector.set_capability("low", 0.01);

        let mut high_count = 0;
        for _ in 0..1000 {
            if elector.elect(&["high".to_string(), "low".to_string()]) == Some("high".to_string()) {
                high_count += 1;
            }
        }
        // M7b #90 分类 A: 加权随机算法 score(high)=1.0, score(low)=0.505,
        // P(high)=1.0/1.505≈66.4%。原阈值 >900 假设绝对垄断,不符合加权随机语义。
        // 改为 >600(留 6.4% 余量),验证 high 确实更常被选中。
        assert!(high_count > 600, "high was only elected {high_count}/1000 times");
    }

    #[test]
    fn load_affects_selection() {
        let elector = LeaderElector::new();
        elector.set_capability("A", 0.8);
        elector.set_capability("B", 0.8);
        // A 满载，B 空闲
        elector.update_load("A", 1.0);
        elector.update_load("B", 0.0);

        let mut b_count = 0;
        for _ in 0..1000 {
            if elector.elect(&["A".to_string(), "B".to_string()]) == Some("B".to_string()) {
                b_count += 1;
            }
        }
        // M7b #90 分类 A: score(A)=0.7, score(B)=0.9, P(B)=0.9/1.6≈56.25%。
        // 原阈值 >600 假设 load 因子占主导,实际 capability 权重(0.5)更大。
        // 改为 >520(留 4.25% 余量),验证 B 确实因负载低更常被选中。
        assert!(b_count > 520, "B was only elected {b_count}/1000 times");
    }
}
