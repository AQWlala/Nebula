//! T-E-B-18: 思维树模式(Tree-of-Thoughts)。
//!
//! 在 SwarmOrchestrator 中支持多路径推理:fan-out N 个 ThoughtAgent,
//! 各自带不同的思维视角(分析/创意/批判/综合)与唯一 path_id,
//! 由 Negotiator 多视角综合仲裁选出最优结论。
//!
//! ## MVP 范围
//! * `depth=1` 单层 fan-out;`depth>1` clamp 到 1。
//! * `branches` 默认 4,与 4 种 ThoughtStrategy 一一对应;
//!   `branches > 4` 时按 mod 4 循环复用策略。
//! * 复用 GenericAgent,通过 `ThoughtAgentConfig::system_prompt_prefix`
//!   前缀注入到任务描述中,实现差异化思维视角。

use serde::{Deserialize, Serialize};

/// 推理策略:线性(默认既有路径)/ 思维树(多路径 fan-out + 仲裁)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReasoningStrategy {
    /// 既有路径:fan-out N 个 GenericAgent,Negotiator 走冲突仲裁。
    Linear,
    /// 思维树:fan-out N 个 ThoughtAgent(不同思维视角),
    /// Negotiator 走多视角综合仲裁。
    TreeOfThoughts {
        /// 分支数(并行路径数),默认 4。
        branches: u32,
        /// 深度(MVP 仅支持 1;>1 时 clamp 到 1)。
        depth: u32,
    },
}

impl Default for ReasoningStrategy {
    fn default() -> Self {
        ReasoningStrategy::Linear
    }
}

impl ReasoningStrategy {
    /// 实际生效的 branches 数(clamp 到 2..=6)。
    pub fn effective_branches(&self) -> u32 {
        match self {
            ReasoningStrategy::Linear => 0,
            ReasoningStrategy::TreeOfThoughts { branches, .. } => {
                (*branches).clamp(MIN_BRANCHES, MAX_BRANCHES)
            }
        }
    }

    /// 实际生效的 depth(MVP clamp 到 1)。
    pub fn effective_depth(&self) -> u32 {
        match self {
            ReasoningStrategy::Linear => 0,
            ReasoningStrategy::TreeOfThoughts { depth, .. } => (*depth).min(1),
        }
    }

    /// 是否为思维树模式。
    pub fn is_tree_of_thoughts(&self) -> bool {
        matches!(self, ReasoningStrategy::TreeOfThoughts { .. })
    }
}

/// 思维视角策略:每个 ThoughtAgent 采用一种独立思维风格。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtStrategy {
    /// 分析视角:逻辑拆解 + 证据驱动。
    Analytical,
    /// 创意视角:发散思维 + 备选方案。
    Creative,
    /// 批判视角:风险/边界条件/反例。
    Critical,
    /// 综合视角:融合多源信息 + 抽象升华。
    Synthesis,
}

impl ThoughtStrategy {
    /// 返回四种思维视角(顺序固定,供工厂轮转分配)。
    pub fn all_four() -> [Self; 4] {
        [
            ThoughtStrategy::Analytical,
            ThoughtStrategy::Creative,
            ThoughtStrategy::Critical,
            ThoughtStrategy::Synthesis,
        ]
    }

    /// 该思维视角对应的 system_prompt 前缀(注入到任务描述前)。
    pub fn system_prompt_prefix(&self) -> &'static str {
        match self {
            ThoughtStrategy::Analytical => {
                "[Analytical Perspective] You are an analytical thinker. \
                 Decompose the task into discrete logical steps. \
                 Cite evidence for each claim. \
                 Prefer structured, step-by-step reasoning over intuition."
            }
            ThoughtStrategy::Creative => {
                "[Creative Perspective] You are a creative thinker. \
                 Generate diverse alternative approaches. \
                 Explore non-obvious connections and edge cases. \
                 Favour breadth and novelty over convention."
            }
            ThoughtStrategy::Critical => {
                "[Critical Perspective] You are a critical thinker. \
                 Stress-test assumptions and identify failure modes. \
                 Surface boundary conditions and counterexamples. \
                 Quantify risks and propose mitigations."
            }
            ThoughtStrategy::Synthesis => {
                "[Synthesis Perspective] You are a synthesis thinker. \
                 Integrate multiple viewpoints into a coherent whole. \
                 Abstract beyond specifics to general principles. \
                 Reconcile tensions between competing objectives."
            }
        }
    }

    /// snake_case 字符串名(供事件/日志使用)。
    pub fn as_str(&self) -> &'static str {
        match self {
            ThoughtStrategy::Analytical => "analytical",
            ThoughtStrategy::Creative => "creative",
            ThoughtStrategy::Critical => "critical",
            ThoughtStrategy::Synthesis => "synthesis",
        }
    }
}

/// 单个 ThoughtAgent 的配置(由工厂生成,供 orchestrator 消费)。
///
/// orchestrator 拿到配置后:
/// 1. 从动态池 acquire 一个 GenericAgent;
/// 2. 把 `system_prompt_prefix` 前置到任务描述;
/// 3. agent 完成后,把 `path_id` 写入 AgentOutput.path_id。
#[derive(Debug, Clone)]
pub struct ThoughtAgentConfig {
    /// 唯一路径 ID,如 "path-0" / "path-1"。
    pub path_id: String,
    /// 该路径采用的思维视角。
    pub strategy: ThoughtStrategy,
    /// system_prompt 前缀(由 strategy.system_prompt_prefix() 生成)。
    pub system_prompt_prefix: &'static str,
}

/// branches clamp 范围(与 orchestrator 的 2..=6 一致)。
pub const MIN_BRANCHES: u32 = 2;
pub const MAX_BRANCHES: u32 = 6;
/// MVP 默认分支数(对应四种思维视角)。
pub const DEFAULT_BRANCHES: u32 = 4;
pub const DEFAULT_DEPTH: u32 = 1;

/// ThoughtAgent 工厂:生成 N 个 path_id 不同的 Agent 配置。
///
/// `branches` 会被 clamp 到 [MIN_BRANCHES, MAX_BRANCHES]。
/// 策略按 `ThoughtStrategy::all_four()` 循环分配,保证相邻 path
/// 视角不同。path_id 形如 "path-0" / "path-1" / ... 。
pub fn build_thought_agent_configs(branches: u32) -> Vec<ThoughtAgentConfig> {
    let n = branches.clamp(MIN_BRANCHES, MAX_BRANCHES);
    let strategies = ThoughtStrategy::all_four();
    (0..n)
        .map(|i| {
            let strategy = strategies[(i as usize) % strategies.len()];
            ThoughtAgentConfig {
                path_id: format!("path-{i}"),
                strategy,
                system_prompt_prefix: strategy.system_prompt_prefix(),
            }
        })
        .collect()
}

/// 构造默认的 TreeOfThoughts 策略(branches=4, depth=1)。
pub fn default_tree_of_thoughts() -> ReasoningStrategy {
    ReasoningStrategy::TreeOfThoughts {
        branches: DEFAULT_BRANCHES,
        depth: DEFAULT_DEPTH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ReasoningStrategy serde ---

    #[test]
    fn reasoning_strategy_linear_default() {
        // ReasoningStrategy 使用 #[serde(tag = "kind")] 内部标签,
        // Linear 序列化为 {"kind":"linear"}(而非裸字符串 "linear")。
        let s: ReasoningStrategy = serde_json::from_str(r#"{"kind":"linear"}"#).unwrap();
        assert!(matches!(s, ReasoningStrategy::Linear));
    }

    #[test]
    fn reasoning_strategy_linear_serde_roundtrip() {
        let s = ReasoningStrategy::Linear;
        let json = serde_json::to_string(&s).unwrap();
        // 默认变体序列化为 {"kind":"linear"}(内部标签)。
        assert!(json.contains("\"kind\":\"linear\""), "got: {json}");
        let de: ReasoningStrategy = serde_json::from_str(&json).unwrap();
        assert!(matches!(de, ReasoningStrategy::Linear));
    }

    #[test]
    fn reasoning_strategy_tree_of_thoughts_serde_roundtrip() {
        let s = ReasoningStrategy::TreeOfThoughts {
            branches: 4,
            depth: 1,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains("\"kind\":\"tree_of_thoughts\""),
            "got: {json}"
        );
        assert!(json.contains("\"branches\":4"));
        assert!(json.contains("\"depth\":1"));
        let de: ReasoningStrategy = serde_json::from_str(&json).unwrap();
        match de {
            ReasoningStrategy::TreeOfThoughts { branches, depth } => {
                assert_eq!(branches, 4);
                assert_eq!(depth, 1);
            }
            _ => panic!("expected TreeOfThoughts"),
        }
    }

    #[test]
    fn reasoning_strategy_default_is_linear() {
        assert!(matches!(
            ReasoningStrategy::default(),
            ReasoningStrategy::Linear
        ));
    }

    // --- effective_branches / effective_depth ---

    #[test]
    fn effective_branches_clamps_to_range() {
        let s = ReasoningStrategy::TreeOfThoughts {
            branches: 0,
            depth: 1,
        };
        assert_eq!(s.effective_branches(), MIN_BRANCHES);
        let s = ReasoningStrategy::TreeOfThoughts {
            branches: 100,
            depth: 1,
        };
        assert_eq!(s.effective_branches(), MAX_BRANCHES);
    }

    #[test]
    fn effective_depth_clamps_to_one_for_mvp() {
        let s = ReasoningStrategy::TreeOfThoughts {
            branches: 4,
            depth: 5,
        };
        assert_eq!(s.effective_depth(), 1);
    }

    #[test]
    fn linear_has_zero_branches_and_depth() {
        let s = ReasoningStrategy::Linear;
        assert_eq!(s.effective_branches(), 0);
        assert_eq!(s.effective_depth(), 0);
        assert!(!s.is_tree_of_thoughts());
    }

    // --- ThoughtStrategy ---

    #[test]
    fn thought_strategy_system_prompt_prefix_four_distinct() {
        let prefixes: Vec<&str> = ThoughtStrategy::all_four()
            .iter()
            .map(|s| s.system_prompt_prefix())
            .collect();
        // 四个前缀互不相同。
        for i in 0..prefixes.len() {
            for j in (i + 1)..prefixes.len() {
                assert_ne!(prefixes[i], prefixes[j], "prefix {i} == prefix {j}");
            }
        }
        // 每个前缀都非空且以视角名开头。
        assert!(prefixes[0].starts_with("[Analytical"));
        assert!(prefixes[1].starts_with("[Creative"));
        assert!(prefixes[2].starts_with("[Critical"));
        assert!(prefixes[3].starts_with("[Synthesis"));
    }

    #[test]
    fn thought_strategy_serde_snake_case() {
        let cases = [
            (ThoughtStrategy::Analytical, "analytical"),
            (ThoughtStrategy::Creative, "creative"),
            (ThoughtStrategy::Critical, "critical"),
            (ThoughtStrategy::Synthesis, "synthesis"),
        ];
        for (strategy, expected) in cases {
            let json = serde_json::to_string(&strategy).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "got: {json}");
            let de: ThoughtStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(de, strategy);
        }
    }

    #[test]
    fn thought_strategy_all_four_returns_correct_order() {
        let arr = ThoughtStrategy::all_four();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0], ThoughtStrategy::Analytical);
        assert_eq!(arr[1], ThoughtStrategy::Creative);
        assert_eq!(arr[2], ThoughtStrategy::Critical);
        assert_eq!(arr[3], ThoughtStrategy::Synthesis);
    }

    #[test]
    fn thought_strategy_as_str_matches_snake_case() {
        assert_eq!(ThoughtStrategy::Analytical.as_str(), "analytical");
        assert_eq!(ThoughtStrategy::Creative.as_str(), "creative");
        assert_eq!(ThoughtStrategy::Critical.as_str(), "critical");
        assert_eq!(ThoughtStrategy::Synthesis.as_str(), "synthesis");
    }

    // --- ThoughtAgent factory ---

    #[test]
    fn factory_generates_distinct_path_ids_for_default_branches() {
        let configs = build_thought_agent_configs(DEFAULT_BRANCHES);
        assert_eq!(configs.len(), DEFAULT_BRANCHES as usize);
        let path_ids: Vec<&str> = configs.iter().map(|c| c.path_id.as_str()).collect();
        assert_eq!(path_ids, vec!["path-0", "path-1", "path-2", "path-3"]);
        // path_id 互不相同。
        let unique: std::collections::HashSet<&str> = path_ids.iter().copied().collect();
        assert_eq!(unique.len(), configs.len());
    }

    #[test]
    fn factory_assigns_rotating_strategies() {
        let configs = build_thought_agent_configs(4);
        assert_eq!(configs[0].strategy, ThoughtStrategy::Analytical);
        assert_eq!(configs[1].strategy, ThoughtStrategy::Creative);
        assert_eq!(configs[2].strategy, ThoughtStrategy::Critical);
        assert_eq!(configs[3].strategy, ThoughtStrategy::Synthesis);
    }

    #[test]
    fn factory_clamps_branches_to_min() {
        let configs = build_thought_agent_configs(0);
        assert_eq!(configs.len(), MIN_BRANCHES as usize);
    }

    #[test]
    fn factory_clamps_branches_to_max() {
        let configs = build_thought_agent_configs(100);
        assert_eq!(configs.len(), MAX_BRANCHES as usize);
    }

    #[test]
    fn factory_wraps_strategies_when_branches_exceeds_four() {
        // branches=5 时,第 5 个 path 应复用 Analytical(mod 4 = 0)。
        let configs = build_thought_agent_configs(5);
        assert_eq!(configs.len(), 5);
        assert_eq!(configs[4].strategy, ThoughtStrategy::Analytical);
        assert_eq!(configs[4].path_id, "path-4");
    }

    #[test]
    fn factory_config_system_prompt_prefix_matches_strategy() {
        let configs = build_thought_agent_configs(4);
        for cfg in &configs {
            assert_eq!(
                cfg.system_prompt_prefix,
                cfg.strategy.system_prompt_prefix()
            );
        }
    }

    // --- default_tree_of_thoughts ---

    #[test]
    fn default_tree_of_thoughts_has_sensible_defaults() {
        let s = default_tree_of_thoughts();
        match s {
            ReasoningStrategy::TreeOfThoughts { branches, depth } => {
                assert_eq!(branches, DEFAULT_BRANCHES);
                assert_eq!(depth, DEFAULT_DEPTH);
            }
            _ => panic!("expected TreeOfThoughts"),
        }
        assert!(s.is_tree_of_thoughts());
        assert_eq!(s.effective_branches(), DEFAULT_BRANCHES);
        assert_eq!(s.effective_depth(), 1);
    }
}
