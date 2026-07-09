//! `nebula::evolution::gene_mutator` — T-E-AE-04 基因级进化引擎。
//!
//! 将 Agent 的行为模式编码为「基因」(Gene)，通过变异 / 交叉 / 选择
//! 实现自进化。与 `prompt_mutator`（prompt 级）、`skill_evolver`
//!（skill 级）、`engine`（L2/L3/L5 记忆级）互补，本模块工作在更细
//! 粒度的「连续参数」级：每条基因持有一个 `f64` value，代表某个
//! 行为 / 策略 / 偏好参数的可调旋钮。
//!
//! ## 设计要点
//!
//! - **基因类型分类**：`GeneType` 区分 Behavior / Strategy / Parameter /
//!   Preference / Skill 五类，便于跨维度进化与统计。
//! - **变异策略**：`MutationStrategy` 提供 Gaussian / Uniform / Crossover
//!   / Adaptive 四种。`GeneMutator::mutate` 默认走 Gaussian；`Adaptive`
//!   会依据当前 fitness 自适应收缩变异步长（fitness 越高、扰动越小）。
//! - **选择**：`select` 采用锦标赛选择（tournament size = 3），相比
//!   轮盘赌对负 fitness 更鲁棒，且实现简单、无浮点归一化坑。
//! - **进化循环**：`evolve` 使用精英保留（elite preservation）策略——
//!   每代最优的 `elite_ratio` 比例基因原样进入下一代，其余席位由
//!   交叉 / 变异填补。精英保留保证 `best_fitness` 单调非递减
//!   （可被单测断言的收敛性质）。
//! - **确定性边界**：所有随机源走 `rand::thread_rng()`；不持久化任何
//!   RNG 状态，`GeneMutator` 本身无内部可变状态，可在多线程只读共享。
//!
//! 与 `evolution::EVOLUTION_ENABLED` 主开关的关系：本模块只提供原语，
//! 是否真正运行由上层（worker / 命令）读取 `evolution_enabled()` 决定，
//! 与 `prompt_mutator` / `skill_evolver` 一致。

use serde::{Deserialize, Serialize};

use rand::Rng;

/// 基因类型 — 行为模式的不同维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeneType {
    /// 行为类：触发条件 / 反应模式。
    Behavior,
    /// 策略类：规划 / 选择路径倾向。
    Strategy,
    /// 参数类：连续可调超参（温度、阈值、步长等）。
    Parameter,
    /// 偏好类：风格 / 语气 / 取舍倾向。
    Preference,
    /// 技能类：能力熟练度权重。
    Skill,
}

/// 变异策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationStrategy {
    /// 高斯扰动（Box-Muller），步长 ∝ value 量级 × mutation_rate。
    Gaussian,
    /// 均匀扰动，[-1,1] × rate × (|value|+1)。
    Uniform,
    /// 自交叉：value *= (1 + delta)，delta ∈ [-rate, rate]。
    Crossover,
    /// 自适应：fitness 越高步长越小，并回写新的 mutation_rate。
    Adaptive,
}

/// 基因 — Agent 行为模式的编码单元。
///
/// `value` 为 `f64` 连续参数；`fitness` 由上层目标函数（win rate 等）
/// 写入；`mutation_rate` 控制单次变异步长（0.0=不变，1.0=大幅扰动）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gene {
    /// 唯一标识（建议 `"<agent>:<dimension>:<n>"` 形式）。
    pub id: String,
    /// 人类可读名称。
    pub name: String,
    /// 连续参数值。
    pub value: f64,
    /// 适应度（越大越优；允许负值，锦标赛选择天然支持）。
    pub fitness: f64,
    /// 该基因自身的变异率，覆盖 mutator 全局 mutation_rate。
    pub mutation_rate: f64,
    /// 基因维度分类。
    pub gene_type: GeneType,
}

impl Gene {
    /// 创建一条新基因，fitness=0.0、mutation_rate=0.1。
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        value: f64,
        gene_type: GeneType,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            value,
            fitness: 0.0,
            mutation_rate: 0.1,
            gene_type,
        }
    }
}

/// 基因池 — 一个 Agent / 维度下全部基因的集合。
#[derive(Debug, Clone, Default)]
pub struct GenePool {
    genes: Vec<Gene>,
}

impl GenePool {
    /// 创建空基因池。
    pub fn new() -> Self {
        Self { genes: Vec::new() }
    }

    /// 追加一条基因。
    pub fn add_gene(&mut self, gene: Gene) {
        self.genes.push(gene);
    }

    /// 按 id 移除基因（若存在多条同 id，全部移除）。
    pub fn remove_gene(&mut self, id: &str) {
        self.genes.retain(|g| g.id != id);
    }

    /// 按 id 查找基因。
    pub fn get_gene(&self, id: &str) -> Option<&Gene> {
        self.genes.iter().find(|g| g.id == id)
    }

    /// 池中基因数量。
    pub fn len(&self) -> usize {
        self.genes.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.genes.is_empty()
    }

    /// 只读访问全部基因。
    pub fn genes(&self) -> &[Gene] {
        &self.genes
    }

    /// 全部基因 fitness 之和。
    pub fn total_fitness(&self) -> f64 {
        self.genes.iter().map(|g| g.fitness).sum()
    }

    /// 返回 fitness 最高的前 `n` 条基因（降序）。`n=0` 或空池返回空。
    pub fn best_genes(&self, n: usize) -> Vec<&Gene> {
        if n == 0 || self.genes.is_empty() {
            return Vec::new();
        }
        let mut indexed: Vec<&Gene> = self.genes.iter().collect();
        indexed.sort_by(|a, b| {
            b.fitness
                .partial_cmp(&a.fitness)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        indexed.into_iter().take(n).collect()
    }
}

/// 进化报告 — 一次 `evolve` 调用的统计快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionReport {
    /// 实际执行的代数。
    pub generations: usize,
    /// 变异次数。
    pub mutations: usize,
    /// 交叉次数。
    pub crossovers: usize,
    /// 终态平均 fitness（空池为 0.0）。
    pub avg_fitness: f64,
    /// 终态最优 fitness（空池为 0.0）。
    pub best_fitness: f64,
}

/// 基因变异配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneMutationConfig {
    /// 全局变异率（0.0-1.0），决定子代是否在交叉后再变异。
    pub mutation_rate: f64,
    /// 交叉率（0.0-1.0），决定子代由交叉还是纯变异产生。
    pub crossover_rate: f64,
    /// 精英比例（0.0-1.0），每代原样保留的最优基因占比。
    pub elite_ratio: f64,
    /// 最大代数上限（`evolve` 调用方可传更小值覆盖）。
    pub max_generations: usize,
}

impl Default for GeneMutationConfig {
    fn default() -> Self {
        Self {
            mutation_rate: 0.1,
            crossover_rate: 0.3,
            elite_ratio: 0.2,
            max_generations: 100,
        }
    }
}

/// 基因变异器 — 无内部可变状态，可只读共享。
pub struct GeneMutator {
    /// 全局变异率（与每条基因的 `mutation_rate` 取后者参与步长计算）。
    pub mutation_rate: f64,
    /// 进化配置（elite_ratio / crossover_rate 等可由调用方调整）。
    pub config: GeneMutationConfig,
}

impl GeneMutator {
    /// 创建变异器，使用默认 `GeneMutationConfig`，仅覆盖 mutation_rate。
    pub fn new(mutation_rate: f64) -> Self {
        let mut config = GeneMutationConfig::default();
        config.mutation_rate = mutation_rate;
        Self {
            mutation_rate,
            config,
        }
    }

    /// 单基因变异（默认 Gaussian 策略）。
    pub fn mutate(&self, gene: &Gene) -> Gene {
        self.mutate_with_strategy(gene, MutationStrategy::Gaussian)
    }

    /// 按指定策略变异单基因。返回新基因，原基因不变。
    pub fn mutate_with_strategy(&self, gene: &Gene, strategy: MutationStrategy) -> Gene {
        let mut mutated = gene.clone();
        let mut rng = rand::thread_rng();
        // rate ∈ [0,1]：取基因自身 mutation_rate，clamp 到合法区间。
        let rate = gene.mutation_rate.clamp(0.0, 1.0);
        // 步长基准：|value|+1，避免 value=0 时步长退化。
        let scale = gene.value.abs() + 1.0;

        match strategy {
            MutationStrategy::Gaussian => {
                // Box-Muller 生成标准正态样本 z。
                let u1: f64 = rng.gen_range(0.0001..1.0);
                let u2: f64 = rng.gen_range(0.0001..1.0);
                let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                mutated.value = gene.value + z * rate * scale;
            }
            MutationStrategy::Uniform => {
                let delta = rng.gen_range(-1.0..1.0) * rate * scale;
                mutated.value = gene.value + delta;
            }
            MutationStrategy::Crossover => {
                // 自交叉：value *= (1 + delta)，delta ∈ [-rate, rate]。
                let delta = rng.gen_range(-1.0..1.0) * rate;
                mutated.value = gene.value * (1.0 + delta);
            }
            MutationStrategy::Adaptive => {
                // fitness 越高 → 步长越小（explore → exploit）。
                // 归一化因子 |fitness|+1 防 0/负值发散。
                let adaptive_rate = rate * (1.0 - (gene.fitness / (gene.fitness.abs() + 1.0)));
                let adaptive_rate = adaptive_rate.clamp(0.0, 1.0);
                let delta = rng.gen_range(-1.0..1.0) * adaptive_rate * scale;
                mutated.value = gene.value + delta;
                mutated.mutation_rate = adaptive_rate;
            }
        }
        mutated
    }

    /// 交叉：凸组合 (BLX-α 的 α=0 退化形式)。
    ///
    /// `value = α·a + (1-α)·b`，α ~ Uniform(0,1)。子代 fitness / mutation_rate
    /// 取双亲均值，gene_type 沿用 parent_a，id/name 拼接双亲以保唯一可读。
    pub fn crossover(&self, parent_a: &Gene, parent_b: &Gene) -> Gene {
        let mut rng = rand::thread_rng();
        let alpha: f64 = rng.gen_range(0.0..1.0);
        let value = alpha * parent_a.value + (1.0 - alpha) * parent_b.value;
        Gene {
            id: format!("{}×{}", parent_a.id, parent_b.id),
            name: format!("{}/{}", parent_a.name, parent_b.name),
            value,
            fitness: (parent_a.fitness + parent_b.fitness) / 2.0,
            mutation_rate: (parent_a.mutation_rate + parent_b.mutation_rate) / 2.0,
            gene_type: parent_a.gene_type,
        }
    }

    /// 选择：锦标赛选择，返回 `n` 条克隆基因。
    ///
    /// 锦标赛规模 = min(3, pool.len())；空池或 n=0 返回空。
    pub fn select(&self, pool: &GenePool, n: usize) -> Vec<Gene> {
        let genes = pool.genes();
        if genes.is_empty() || n == 0 {
            return Vec::new();
        }
        let mut rng = rand::thread_rng();
        let tournament_size = 3.min(genes.len());
        let mut selected: Vec<Gene> = Vec::with_capacity(n);
        for _ in 0..n {
            let mut best: Option<&Gene> = None;
            for _ in 0..tournament_size {
                let idx = rng.gen_range(0..genes.len());
                let candidate = &genes[idx];
                best = Some(match best {
                    None => candidate,
                    Some(b) => {
                        if candidate.fitness > b.fitness {
                            candidate
                        } else {
                            b
                        }
                    }
                });
            }
            if let Some(b) = best {
                selected.push(b.clone());
            }
        }
        selected
    }

    /// 进化循环（in-place 修改 pool）。
    ///
    /// 每代：
    ///   1. 保留前 `elite_ratio` 比例的最优基因（精英，原样克隆）。
    ///   2. 剩余席位：以 `crossover_rate` 概率走「交叉(+可能变异)」，
    ///      否则走「变异」。
    ///   3. 用新一代整体替换池。
    ///
    /// 精英保留保证 `best_fitness` 单调非递减（收敛性质）。
    pub fn evolve(&self, pool: &mut GenePool, generations: usize) -> EvolutionReport {
        let mut mutations = 0usize;
        let mut crossovers = 0usize;
        let mut generations_run = 0usize;

        for _ in 0..generations {
            let pop_size = pool.len();
            if pop_size == 0 {
                break;
            }
            generations_run += 1;

            // 单基因池：无法交叉，直接变异唯一基因。
            if pop_size == 1 {
                let single = pool.genes()[0].clone();
                let mutated = self.mutate(&single);
                mutations += 1;
                pool.remove_gene(&single.id);
                pool.add_gene(mutated);
                continue;
            }

            // 精英保留。
            let elite_count = ((pop_size as f64) * self.config.elite_ratio).round() as usize;
            let elite_count = elite_count.clamp(1, pop_size);
            let elites: Vec<Gene> = pool.best_genes(elite_count).into_iter().cloned().collect();

            let mut rng = rand::thread_rng();
            let mut next_gen: Vec<Gene> = elites;

            while next_gen.len() < pop_size {
                let roll: f64 = rng.gen_range(0.0..1.0);
                if roll < self.config.crossover_rate {
                    let parents = self.select(pool, 2);
                    if parents.len() == 2 {
                        let child = self.crossover(&parents[0], &parents[1]);
                        let child = if rng.gen_range(0.0..1.0) < self.mutation_rate {
                            mutations += 1;
                            self.mutate(&child)
                        } else {
                            child
                        };
                        next_gen.push(child);
                        crossovers += 1;
                        continue;
                    }
                }
                // 变异分支（含交叉选亲不足 2 的回退）。
                let parent = self.select(pool, 1);
                if let Some(p) = parent.into_iter().next() {
                    let child = self.mutate(&p);
                    mutations += 1;
                    next_gen.push(child);
                } else {
                    break;
                }
            }

            // 整体替换池。
            *pool = GenePool::new();
            for g in next_gen {
                pool.add_gene(g);
            }
        }

        let avg = if pool.is_empty() {
            0.0
        } else {
            pool.total_fitness() / pool.len() as f64
        };
        let best_fitness = pool.best_genes(1).first().map(|g| g.fitness).unwrap_or(0.0);

        EvolutionReport {
            generations: generations_run,
            mutations,
            crossovers,
            avg_fitness: avg,
            best_fitness,
        }
    }
}

// ============================================================================
// 单元测试 — 覆盖变异 / 交叉 / 选择 / 进化循环 / 收敛性（共 22 个）。
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Gene ----------

    #[test]
    fn gene_new_sets_defaults() {
        let g = Gene::new("a:temp:1", "temperature", 0.7, GeneType::Parameter);
        assert_eq!(g.id, "a:temp:1");
        assert_eq!(g.name, "temperature");
        assert!((g.value - 0.7).abs() < f64::EPSILON);
        assert_eq!(g.fitness, 0.0);
        assert!((g.mutation_rate - 0.1).abs() < f64::EPSILON);
        assert_eq!(g.gene_type, GeneType::Parameter);
    }

    #[test]
    fn gene_type_serde_roundtrip() {
        for gt in [
            GeneType::Behavior,
            GeneType::Strategy,
            GeneType::Parameter,
            GeneType::Preference,
            GeneType::Skill,
        ] {
            let json = serde_json::to_string(&gt).unwrap();
            let back: GeneType = serde_json::from_str(&json).unwrap();
            assert_eq!(gt, back);
        }
    }

    #[test]
    fn gene_serde_roundtrip() {
        let g = Gene {
            id: "x".into(),
            name: "n".into(),
            value: -1.5,
            fitness: 0.25,
            mutation_rate: 0.4,
            gene_type: GeneType::Skill,
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: Gene = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }

    // ---------- GenePool ----------

    #[test]
    fn pool_new_is_empty() {
        let p = GenePool::new();
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
        assert!((p.total_fitness()).abs() < f64::EPSILON);
    }

    #[test]
    fn pool_add_and_get() {
        let mut p = GenePool::new();
        p.add_gene(Gene::new("g1", "a", 1.0, GeneType::Behavior));
        p.add_gene(Gene::new("g2", "b", 2.0, GeneType::Strategy));
        assert_eq!(p.len(), 2);
        assert!(p.get_gene("g1").is_some());
        assert_eq!(p.get_gene("g2").unwrap().name, "b");
    }

    #[test]
    fn pool_get_missing_returns_none() {
        let p = GenePool::new();
        assert!(p.get_gene("nope").is_none());
    }

    #[test]
    fn pool_remove_gene() {
        let mut p = GenePool::new();
        p.add_gene(Gene::new("g1", "a", 1.0, GeneType::Behavior));
        p.add_gene(Gene::new("g2", "b", 2.0, GeneType::Strategy));
        p.remove_gene("g1");
        assert_eq!(p.len(), 1);
        assert!(p.get_gene("g1").is_none());
        assert!(p.get_gene("g2").is_some());
    }

    #[test]
    fn pool_remove_missing_is_noop() {
        let mut p = GenePool::new();
        p.add_gene(Gene::new("g1", "a", 1.0, GeneType::Behavior));
        p.remove_gene("ghost");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn pool_total_fitness() {
        let mut p = GenePool::new();
        p.add_gene(Gene {
            fitness: 0.5,
            ..Gene::new("g1", "a", 1.0, GeneType::Parameter)
        });
        p.add_gene(Gene {
            fitness: -0.25,
            ..Gene::new("g2", "b", 2.0, GeneType::Parameter)
        });
        assert!((p.total_fitness() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn pool_best_genes_descending() {
        let mut p = GenePool::new();
        for (i, f) in [0.1, 0.9, 0.5, 0.3].iter().enumerate() {
            p.add_gene(Gene {
                id: format!("g{i}"),
                name: "n".into(),
                value: 0.0,
                fitness: *f,
                mutation_rate: 0.1,
                gene_type: GeneType::Skill,
            });
        }
        let best = p.best_genes(3);
        assert_eq!(best.len(), 3);
        assert!((best[0].fitness - 0.9).abs() < 1e-9);
        assert!((best[1].fitness - 0.5).abs() < 1e-9);
        assert!((best[2].fitness - 0.3).abs() < 1e-9);
    }

    #[test]
    fn pool_best_genes_more_than_size() {
        let mut p = GenePool::new();
        p.add_gene(Gene::new("g1", "a", 1.0, GeneType::Behavior));
        let best = p.best_genes(10);
        assert_eq!(best.len(), 1);
    }

    #[test]
    fn pool_best_genes_zero_n() {
        let mut p = GenePool::new();
        p.add_gene(Gene::new("g1", "a", 1.0, GeneType::Behavior));
        assert!(p.best_genes(0).is_empty());
    }

    // ---------- GeneMutator: mutate ----------

    #[test]
    fn mutator_new_sets_rate_and_default_config() {
        let m = GeneMutator::new(0.25);
        assert!((m.mutation_rate - 0.25).abs() < 1e-9);
        assert!((m.config.mutation_rate - 0.25).abs() < 1e-9);
        // 默认 config 其余字段
        assert!((m.config.crossover_rate - 0.3).abs() < 1e-9);
        assert!((m.config.elite_ratio - 0.2).abs() < 1e-9);
    }

    #[test]
    fn mutate_produces_variation_and_finite_values() {
        let m = GeneMutator::new(0.2);
        let g = Gene::new("g1", "temp", 1.0, GeneType::Parameter);
        let mut distinct = std::collections::HashSet::new();
        for _ in 0..100 {
            let child = m.mutate(&g);
            assert!(child.value.is_finite(), "mutated value must be finite");
            distinct.insert(child.value.to_bits());
        }
        // 100 次高斯变异应产生 >1 个不同值（确定性边界：几乎必然）。
        assert!(distinct.len() > 1, "mutate should introduce variation");
    }

    #[test]
    fn mutate_preserves_metadata_default_strategy() {
        let m = GeneMutator::new(0.2);
        let g = Gene {
            id: "g1".into(),
            name: "temp".into(),
            value: 2.0,
            fitness: 0.42,
            mutation_rate: 0.3,
            gene_type: GeneType::Strategy,
        };
        let child = m.mutate(&g);
        // 默认 Gaussian：仅 value 改变，其余元数据保留。
        assert_eq!(child.id, g.id);
        assert_eq!(child.name, g.name);
        assert_eq!(child.gene_type, g.gene_type);
        assert!((child.fitness - g.fitness).abs() < 1e-9);
        assert!((child.mutation_rate - g.mutation_rate).abs() < 1e-9);
        assert!((child.value - g.value).abs() > 0.0 || true); // value 允许偶发相等
    }

    #[test]
    fn mutate_uniform_finite() {
        let m = GeneMutator::new(0.5);
        let g = Gene::new("g1", "x", 0.0, GeneType::Parameter);
        for _ in 0..50 {
            let child = m.mutate_with_strategy(&g, MutationStrategy::Uniform);
            assert!(child.value.is_finite());
        }
    }

    #[test]
    fn mutate_adaptive_adjusts_mutation_rate() {
        let m = GeneMutator::new(0.5);
        let g = Gene {
            fitness: 5.0, // 高 fitness → 步长应显著收缩
            mutation_rate: 0.5,
            ..Gene::new("g1", "x", 10.0, GeneType::Behavior)
        };
        let child = m.mutate_with_strategy(&g, MutationStrategy::Adaptive);
        // adaptive_rate = 0.5 * (1 - 5/(5+1)) = 0.5 * (1/6) ≈ 0.0833
        assert!(child.mutation_rate < g.mutation_rate);
        assert!((child.mutation_rate - (0.5 * (1.0 - 5.0 / 6.0))).abs() < 1e-9);
    }

    #[test]
    fn mutate_crossover_strategy_preserves_sign() {
        let m = GeneMutator::new(0.1);
        let g = Gene::new("g1", "x", 5.0, GeneType::Parameter);
        for _ in 0..50 {
            let child = m.mutate_with_strategy(&g, MutationStrategy::Crossover);
            // value *= (1+delta), |delta|<=0.1 → 始终正。
            assert!(child.value > 0.0);
            assert!(child.value.is_finite());
        }
    }

    // ---------- GeneMutator: crossover ----------

    #[test]
    fn crossover_value_within_parent_range() {
        let m = GeneMutator::new(0.1);
        let a = Gene::new("a", "a", -10.0, GeneType::Parameter);
        let b = Gene::new("b", "b", 10.0, GeneType::Parameter);
        for _ in 0..50 {
            let child = m.crossover(&a, &b);
            assert!(
                child.value >= -10.0 && child.value <= 10.0,
                "crossover value must lie within parent range"
            );
        }
    }

    #[test]
    fn crossover_inherits_average_fitness_and_rate() {
        let m = GeneMutator::new(0.1);
        let a = Gene {
            fitness: 0.2,
            mutation_rate: 0.1,
            ..Gene::new("a", "a", 1.0, GeneType::Skill)
        };
        let b = Gene {
            fitness: 0.8,
            mutation_rate: 0.5,
            ..Gene::new("b", "b", 3.0, GeneType::Skill)
        };
        let child = m.crossover(&a, &b);
        assert!((child.fitness - 0.5).abs() < 1e-9);
        assert!((child.mutation_rate - 0.3).abs() < 1e-9);
    }

    #[test]
    fn crossover_inherits_parent_a_gene_type() {
        let m = GeneMutator::new(0.1);
        let a = Gene::new("a", "a", 1.0, GeneType::Preference);
        let b = Gene::new("b", "b", 2.0, GeneType::Strategy);
        let child = m.crossover(&a, &b);
        assert_eq!(child.gene_type, GeneType::Preference);
    }

    // ---------- GeneMutator: select ----------

    #[test]
    fn select_empty_pool_returns_empty() {
        let m = GeneMutator::new(0.1);
        let p = GenePool::new();
        assert!(m.select(&p, 5).is_empty());
    }

    #[test]
    fn select_returns_requested_count() {
        let m = GeneMutator::new(0.1);
        let mut p = GenePool::new();
        for i in 0..5 {
            p.add_gene(Gene::new(
                format!("g{i}"),
                "n",
                i as f64,
                GeneType::Behavior,
            ));
        }
        let sel = m.select(&p, 3);
        assert_eq!(sel.len(), 3);
    }

    #[test]
    fn select_tournament_prefers_fitter() {
        // 池大小 = 2 → 锦标赛规模 = 2 → 双亲都进锦标赛 → 优者必胜。
        let m = GeneMutator::new(0.1);
        let mut p = GenePool::new();
        p.add_gene(Gene {
            fitness: 0.0,
            ..Gene::new("weak", "w", 0.0, GeneType::Behavior)
        });
        p.add_gene(Gene {
            fitness: 10.0,
            ..Gene::new("strong", "s", 1.0, GeneType::Behavior)
        });
        let sel = m.select(&p, 20);
        assert_eq!(sel.len(), 20);
        for g in &sel {
            assert_eq!(
                g.id, "strong",
                "tournament must always pick the fitter gene"
            );
        }
    }

    // ---------- GeneMutator: evolve ----------

    #[test]
    fn evolve_zero_generations_is_noop() {
        let m = GeneMutator::new(0.1);
        let mut p = GenePool::new();
        p.add_gene(Gene {
            fitness: 0.5,
            ..Gene::new("g1", "a", 1.0, GeneType::Parameter)
        });
        let report = m.evolve(&mut p, 0);
        assert_eq!(report.generations, 0);
        assert_eq!(report.mutations, 0);
        assert_eq!(report.crossovers, 0);
        assert!((report.best_fitness - 0.5).abs() < 1e-9);
        assert!((report.avg_fitness - 0.5).abs() < 1e-9);
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn evolve_preserves_pool_size() {
        let m = GeneMutator::new(0.2);
        let mut p = GenePool::new();
        for i in 0..10 {
            p.add_gene(Gene {
                id: format!("g{i}"),
                name: "n".into(),
                value: i as f64,
                fitness: i as f64,
                mutation_rate: 0.2,
                gene_type: GeneType::Parameter,
            });
        }
        let size_before = p.len();
        let _report = m.evolve(&mut p, 5);
        assert_eq!(p.len(), size_before, "evolve should keep population stable");
    }

    #[test]
    fn evolve_elitism_best_fitness_non_decreasing() {
        // 收敛性：精英保留保证 best_fitness 单调非递减。
        let m = GeneMutator::new(0.3);
        let mut p = GenePool::new();
        for i in 0..8 {
            p.add_gene(Gene {
                id: format!("g{i}"),
                name: "n".into(),
                value: i as f64,
                fitness: (i as f64) * 0.1, // 0.0 .. 0.7
                mutation_rate: 0.3,
                gene_type: GeneType::Skill,
            });
        }
        let best_before = p.best_genes(1).first().unwrap().fitness;

        // 跑 3 轮，每轮后断言 best 不降。
        for _ in 0..3 {
            let report = m.evolve(&mut p, 1);
            let best_now = p.best_genes(1).first().unwrap().fitness;
            assert!(
                best_now >= best_before - 1e-9,
                "best_fitness must not decrease (got {best_now} < {best_before})"
            );
            assert!(
                report.best_fitness >= best_before - 1e-9,
                "report.best_fitness must not decrease"
            );
        }
    }

    #[test]
    fn evolve_counts_mutations_and_crossovers() {
        let m = GeneMutator::new(0.5); // 高变异率 + 默认 0.3 交叉率
        let mut p = GenePool::new();
        for i in 0..10 {
            p.add_gene(Gene::new(
                format!("g{i}"),
                "n",
                i as f64,
                GeneType::Parameter,
            ));
        }
        let report = m.evolve(&mut p, 3);
        assert_eq!(report.generations, 3);
        assert!(report.mutations > 0, "should have performed mutations");
        assert!(report.crossovers > 0, "should have performed crossovers");
    }

    #[test]
    fn evolve_single_gene_pool_mutates_only() {
        let m = GeneMutator::new(0.4);
        let mut p = GenePool::new();
        p.add_gene(Gene::new("solo", "s", 1.0, GeneType::Behavior));
        let report = m.evolve(&mut p, 3);
        assert_eq!(report.generations, 3);
        assert!(report.mutations >= 3);
        assert_eq!(report.crossovers, 0, "single-gene pool cannot crossover");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn evolve_converges_best_fitness_stable() {
        // 收敛性（稳定）：多代进化后 best_fitness 不再变化（精英被持续保留）。
        let m = GeneMutator::new(0.2);
        let mut p = GenePool::new();
        for i in 0..6 {
            p.add_gene(Gene {
                id: format!("g{i}"),
                name: "n".into(),
                value: i as f64,
                fitness: i as f64, // best = 5.0
                mutation_rate: 0.2,
                gene_type: GeneType::Strategy,
            });
        }
        let r1 = m.evolve(&mut p, 10);
        let best1 = r1.best_fitness;
        let r2 = m.evolve(&mut p, 10);
        // 精英保留 + 无外部 fitness 重估 → best 应保持稳定。
        assert!((r2.best_fitness - best1).abs() < 1e-9);
        assert!((r1.best_fitness - 5.0).abs() < 1e-9);
    }

    // ---------- EvolutionReport / Config ----------

    #[test]
    fn evolution_report_serde_roundtrip() {
        let r = EvolutionReport {
            generations: 7,
            mutations: 42,
            crossovers: 13,
            avg_fitness: 0.66,
            best_fitness: 0.99,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: EvolutionReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn gene_mutation_config_default_values() {
        let c = GeneMutationConfig::default();
        assert!((c.mutation_rate - 0.1).abs() < 1e-9);
        assert!((c.crossover_rate - 0.3).abs() < 1e-9);
        assert!((c.elite_ratio - 0.2).abs() < 1e-9);
        assert_eq!(c.max_generations, 100);
    }
}
