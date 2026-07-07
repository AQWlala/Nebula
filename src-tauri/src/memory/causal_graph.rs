//! 因果图谱推理引擎。
//!
//! 基于 `memory_relations` 表中的 `Causes` / `DerivedFrom` / `References`
//! 关系，提供因果链追踪和根本原因分析。与通用的 `graph_search` 模块不同，
//! 本模块专注于因果语义：
//!
//! * `trace_root_causes` — 从一个记忆出发，沿 `Causes` / `DerivedFrom`
//!   边向上追溯，找到根本原因链。
//! * `find_effects` — 从一个记忆出发，沿 `Causes` 边向下查找所有下游
//!   效果。
//! * `explain` — 生成一条最可能的因果解释路径。
//!
//! 关系语义约定（参见 `types::RelationKind`）：
//!
//! * `Causes`：`src → dst` 表示 "src 导致了 dst"。
//!   - 找 dst 的原因 → 查 `dst_id == id && kind == Causes`，src 即原因。
//!   - 找 src 的效果 → 查 `src_id == id && kind == Causes`，dst 即效果。
//! * `DerivedFrom`：`src → dst` 表示 "src 派生自 dst"。
//!   - 找 src 的来源 → 查 `src_id == id && kind == DerivedFrom`，dst 即来源。
//!
//! 设计文档 v7.0 §3.2 L2 认知层 — 因果图谱推理。

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::sqlite_store::SqliteStore;
use super::types::RelationKind;

/// 因果链中的一个节点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalNode {
    /// 记忆 ID。
    pub memory_id: String,
    /// 从根到本节点的深度（根节点 depth=0）。
    pub depth: u32,
    /// 从父节点到本节点的关系类型（根节点为 None）。
    pub relation: Option<RelationKind>,
    /// 关系权重（0.0-1.0），用于排序因果链。
    pub weight: f32,
}

/// 一条完整的因果链。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalChain {
    /// 从根到叶的节点序列。
    pub nodes: Vec<CausalNode>,
    /// 根节点 ID（nodes[0].memory_id 的快捷引用）。
    pub root_id: String,
    /// 叶节点 ID（nodes.last().memory_id 的快捷引用）。
    pub leaf_id: String,
    /// 链路总权重（各边权重之积）。
    pub confidence: f32,
}

impl CausalChain {
    /// 链路长度（节点数）。
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 是否为空链。
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 返回链路上所有记忆 ID（按从根到叶的顺序）。
    pub fn path_ids(&self) -> Vec<&str> {
        self.nodes.iter().map(|n| n.memory_id.as_str()).collect()
    }
}

/// 因果图查询配置。
#[derive(Debug, Clone)]
pub struct CausalGraphConfig {
    /// 最大追溯深度（边数）。
    pub max_depth: u32,
    /// 最多返回的因果链数。
    pub max_chains: usize,
    /// 每层最大分支数（防止爆炸）。
    pub max_branch_per_level: usize,
    /// 关系权重低于此值的边被忽略。
    pub min_weight: f32,
}

impl Default for CausalGraphConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_chains: 10,
            max_branch_per_level: 20,
            min_weight: 0.1,
        }
    }
}

/// 因果图谱引擎。
///
/// 持有一个 `SqliteStore` 的克隆，通过 `get_relations` 同步查询
/// `memory_relations` 表。引擎本身是无状态的，每次查询独立。
pub struct CausalGraphEngine {
    store: SqliteStore,
}

impl CausalGraphEngine {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    /// 追溯一个记忆的根本原因链。
    ///
    /// 从 `memory_id` 出发，沿 `Causes`（反向）和 `DerivedFrom`（正向）
    /// 边向上追溯，找到所有根本原因链。一条链的 "根" 是没有更上游
    /// 原因的节点。
    ///
    /// 返回的链按 confidence 降序排列。
    pub fn trace_root_causes(
        &self,
        memory_id: &str,
        config: &CausalGraphConfig,
    ) -> Vec<CausalChain> {
        let mut chains: Vec<CausalChain> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(memory_id.to_string());

        // BFS 队列：(当前节点, 已构建的链)
        let initial = CausalChain {
            nodes: vec![CausalNode {
                memory_id: memory_id.to_string(),
                depth: 0,
                relation: None,
                weight: 1.0,
            }],
            root_id: memory_id.to_string(),
            leaf_id: memory_id.to_string(),
            confidence: 1.0,
        };
        let mut queue: VecDeque<CausalChain> = VecDeque::new();
        queue.push_back(initial);

        while let Some(chain) = queue.pop_front() {
            let current = chain.leaf_id.clone();
            let current_depth = chain.nodes.last().map(|n| n.depth).unwrap_or(0);

            if current_depth >= config.max_depth {
                chains.push(chain);
                continue;
            }

            let relations = match self.store.get_relations(&current) {
                Ok(r) => r,
                Err(_) => {
                    chains.push(chain);
                    continue;
                }
            };

            // 找上游原因：
            // - dst_id == current && kind == Causes → src 是原因
            // - src_id == current && kind == DerivedFrom → dst 是来源
            let mut upstream: Vec<(String, RelationKind, f32)> = Vec::new();
            for rel in relations.iter() {
                if rel.weight < config.min_weight {
                    continue;
                }
                if rel.dst_id == current && rel.kind == RelationKind::Causes {
                    upstream.push((rel.src_id.clone(), rel.kind, rel.weight));
                } else if rel.src_id == current && rel.kind == RelationKind::DerivedFrom {
                    upstream.push((rel.dst_id.clone(), rel.kind, rel.weight));
                }
            }

            if upstream.is_empty() {
                // 没有更上游的原因 → 这是一条完整的根因链
                chains.push(chain);
                continue;
            }

            upstream.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            let mut branch_count = 0usize;
            for (cause_id, rel_kind, weight) in upstream {
                if branch_count >= config.max_branch_per_level {
                    break;
                }
                if visited.contains(&cause_id) {
                    continue;
                }
                visited.insert(cause_id.clone());

                let mut new_chain = chain.clone();
                new_chain.nodes.push(CausalNode {
                    memory_id: cause_id.clone(),
                    depth: current_depth + 1,
                    relation: Some(rel_kind),
                    weight,
                });
                new_chain.leaf_id = cause_id.clone();
                new_chain.confidence *= weight;
                queue.push_back(new_chain);
                branch_count += 1;
            }

            // 如果 chain 被扩展了（leaf 变了），原始 chain 不入结果；
            // 否则（所有上游都已访问过），保留原始链。
            if chain.leaf_id == current && branch_count == 0 {
                chains.push(chain);
            }
        }

        chains.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        chains.truncate(config.max_chains);
        chains
    }

    /// 查找一个记忆的所有下游效果。
    ///
    /// 从 `memory_id` 出发，沿 `Causes`（正向）边向下查找所有效果。
    /// 返回的链按 confidence 降序排列。
    pub fn find_effects(&self, memory_id: &str, config: &CausalGraphConfig) -> Vec<CausalChain> {
        let mut chains: Vec<CausalChain> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(memory_id.to_string());

        let initial = CausalChain {
            nodes: vec![CausalNode {
                memory_id: memory_id.to_string(),
                depth: 0,
                relation: None,
                weight: 1.0,
            }],
            root_id: memory_id.to_string(),
            leaf_id: memory_id.to_string(),
            confidence: 1.0,
        };
        let mut queue: VecDeque<CausalChain> = VecDeque::new();
        queue.push_back(initial);

        while let Some(chain) = queue.pop_front() {
            let current = chain.leaf_id.clone();
            let current_depth = chain.nodes.last().map(|n| n.depth).unwrap_or(0);

            if current_depth >= config.max_depth {
                chains.push(chain);
                continue;
            }

            let relations = match self.store.get_relations(&current) {
                Ok(r) => r,
                Err(_) => {
                    chains.push(chain);
                    continue;
                }
            };

            // 找下游效果：src_id == current && kind == Causes → dst 是效果
            let mut downstream: Vec<(String, RelationKind, f32)> = Vec::new();
            for rel in relations.iter() {
                if rel.weight < config.min_weight {
                    continue;
                }
                if rel.src_id == current && rel.kind == RelationKind::Causes {
                    downstream.push((rel.dst_id.clone(), rel.kind, rel.weight));
                }
            }

            if downstream.is_empty() {
                chains.push(chain);
                continue;
            }

            downstream.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            let mut branch_count = 0usize;
            for (effect_id, rel_kind, weight) in downstream {
                if branch_count >= config.max_branch_per_level {
                    break;
                }
                if visited.contains(&effect_id) {
                    continue;
                }
                visited.insert(effect_id.clone());

                let mut new_chain = chain.clone();
                new_chain.nodes.push(CausalNode {
                    memory_id: effect_id.clone(),
                    depth: current_depth + 1,
                    relation: Some(rel_kind),
                    weight,
                });
                new_chain.leaf_id = effect_id.clone();
                new_chain.confidence *= weight;
                queue.push_back(new_chain);
                branch_count += 1;
            }

            if chain.leaf_id == current && branch_count == 0 {
                chains.push(chain);
            }
        }

        chains.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        chains.truncate(config.max_chains);
        chains
    }

    /// 生成一条最可能的因果解释路径。
    ///
    /// 先追溯根因，取 confidence 最高的根因链；如果根因链长度 > 1，
    /// 再从根因出发查找效果链，拼接成完整的 "根因 → ... → 当前 → ... → 效果"
    /// 解释路径。
    pub fn explain(&self, memory_id: &str) -> Option<CausalChain> {
        let config = CausalGraphConfig::default();
        let root_chains = self.trace_root_causes(memory_id, &config);
        let best_root = root_chains.into_iter().max_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 如果根因链只有当前节点本身（无上游原因），尝试只返回效果链。
        let root = best_root?;
        if root.len() <= 1 {
            // 无上游原因，查找效果链
            let effect_chains = self.find_effects(memory_id, &config);
            return effect_chains.into_iter().next();
        }

        // 根因链是从 leaf(memory_id) 到 root 的顺序，反转得到 root → memory_id
        let mut nodes: Vec<CausalNode> = root.nodes.into_iter().rev().collect();
        // 修正 depth
        for (i, node) in nodes.iter_mut().enumerate() {
            node.depth = i as u32;
        }
        let root_id = nodes.first()?.memory_id.clone();
        let confidence = root.confidence;

        // 拼接效果链（从 memory_id 向下）
        let effect_chains = self.find_effects(memory_id, &config);
        if let Some(best_effect) = effect_chains.into_iter().next() {
            // 跳过效果链的第一个节点（它是 memory_id 本身，已在根因链中）
            if best_effect.len() > 1 {
                for node in best_effect.nodes.into_iter().skip(1) {
                    let mut n = node;
                    n.depth = nodes.len() as u32;
                    nodes.push(n);
                }
            }
        }

        let leaf_id = nodes.last()?.memory_id.clone();
        Some(CausalChain {
            nodes,
            root_id,
            leaf_id,
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::Memory;
    use crate::memory::types::{MemoryLayer, MemoryRelation, MemoryType, SourceKind};

    // M7b #90 分类 C:原实现返回 `file:causal_test_<nanos>?mode=memory&cache=shared`
    // URI 字符串,但 `SqliteStore::open()` 用的是 `Connection::open(path)`(非 URI
    // 模式)。Windows 文件名不能含 `?`,导致 SqliteStore::open 失败。改用 sqlite_store.rs
    // 测试模式:真实临时文件 + UUID,每个测试独立 DB 文件。
    fn temp_db_path() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("nebula_causal_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    fn seed_chain(store: &SqliteStore) -> (String, String, String) {
        // A → Causes → B → Causes → C
        let a = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            "root cause A",
            SourceKind::UserInput,
        );
        let b = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            "intermediate B",
            SourceKind::AgentOutput,
        );
        let c = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            "effect C",
            SourceKind::System,
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // 直接用 insert_guarded_spawn（绕过 sponge 以隔离测试）
            for m in [&a, &b, &c] {
                store.insert_guarded_spawn(m).await.unwrap();
            }
            // M7b #90 分类 A: add_relation 是 async fn,必须在 async 上下文中
            // .await 才会执行。原实现 `let _ = store.add_relation(&rel_ab);`
            // 只创建了 Future 未 await,关系从未写入 DB,导致 trace_root_causes
            // / find_effects / explain 都找不到链条。
            let rel_ab = MemoryRelation::new(a.id.clone(), b.id.clone(), RelationKind::Causes);
            let rel_bc = MemoryRelation::new(b.id.clone(), c.id.clone(), RelationKind::Causes);
            store.add_relation(&rel_ab).await.unwrap();
            store.add_relation(&rel_bc).await.unwrap();
        });

        (a.id, b.id, c.id)
    }

    #[test]
    fn trace_root_causes_finds_chain() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let (a_id, b_id, c_id) = seed_chain(&store);

        let engine = CausalGraphEngine::new(store.clone());
        let config = CausalGraphConfig::default();
        let chains = engine.trace_root_causes(&c_id, &config);

        // 应至少找到一条链: C → B → A
        assert!(
            !chains.is_empty(),
            "should find at least one root cause chain"
        );
        let best = &chains[0];
        // M7b #90 分类 A: trace_root_causes 语义是从 memory_id(C)反向追踪到根因(A)。
        // 链 nodes 顺序 = [C, B, A],root_id = C(被查询节点/效果),
        // leaf_id = A(根因/链终点)。原断言 `leaf_id == c_id` 混淆了 root/leaf。
        assert_eq!(best.root_id, c_id, "root should be C (queried node)");
        assert_eq!(best.leaf_id, a_id, "leaf should be root cause A");
        assert!(best.len() >= 2, "chain should have at least 2 nodes");
        // 路径中应包含 B 和 A
        let path_ids: Vec<&str> = best.path_ids();
        assert!(
            path_ids.contains(&a_id.as_str()),
            "chain should include root A"
        );
        assert!(
            path_ids.contains(&b_id.as_str()),
            "chain should include intermediate B"
        );
    }

    #[test]
    fn find_effects_traces_downstream() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let (a_id, b_id, c_id) = seed_chain(&store);

        let engine = CausalGraphEngine::new(store.clone());
        let config = CausalGraphConfig::default();
        let chains = engine.find_effects(&a_id, &config);

        // 应找到 A → B → C
        assert!(!chains.is_empty(), "should find at least one effect chain");
        let best = &chains[0];
        assert_eq!(best.root_id, a_id, "root should be A");
        let path_ids: Vec<&str> = best.path_ids();
        assert!(path_ids.contains(&b_id.as_str()), "chain should include B");
        assert!(path_ids.contains(&c_id.as_str()), "chain should include C");
    }

    #[test]
    fn explain_provides_full_path() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let (a_id, _b_id, c_id) = seed_chain(&store);

        let engine = CausalGraphEngine::new(store.clone());
        let chain = engine.explain(&c_id);

        assert!(chain.is_some(), "explain should return a chain");
        let ch = chain.unwrap();
        let path_ids: Vec<&str> = ch.path_ids();
        assert!(
            path_ids.contains(&a_id.as_str()),
            "explain should include root A"
        );
        assert!(
            path_ids.contains(&c_id.as_str()),
            "explain should include C"
        );
    }

    #[test]
    fn default_config_values() {
        let cfg = CausalGraphConfig::default();
        assert_eq!(cfg.max_depth, 5);
        assert_eq!(cfg.max_chains, 10);
        assert_eq!(cfg.max_branch_per_level, 20);
        assert!(cfg.min_weight > 0.0);
    }
}
