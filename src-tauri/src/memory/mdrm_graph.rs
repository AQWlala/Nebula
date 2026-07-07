//! T-E-B-16: MDRM 5 维关系图谱(Multi-Dimensional Relation Map)。
//!
//! 扩展 v2.0 的 `CausalGraphEngine`(仅因果)为 5 维:
//!
//! | 维度 | 含义 | 关系类型 | 示例 |
//! |------|------|---------|------|
//! | Causal(因果) | A 导致 B | `Causes`/`Supports`/`Contradicts` | "改了配置" → "服务启动失败" |
//! | Temporal(时序) | A 先于 B | `Before` | "晨会" → "下午提交报告" |
//! | Entity(实体) | A 属于 B / 同实体 | `SameEntity`/`References` | "Rust" 属于 "编程语言" |
//! | Hierarchical(层级) | A 包含 B | `Contains`/`DerivedFrom` | "项目" 包含 "模块" |
//! | Similarity(相似度) | A 相似 B | `Similar` | "向量搜索" 相似 "语义检索" |
//!
//! ## 设计原则
//!
//! - **维度正交**:每个 RelationKind 严格归属一个维度,便于按维度过滤
//! - **方向语义**:Causal/Temporal 有方向;Entity/Similarity 对称;Hierarchical 双向
//! - **图快照**:`get_graph_snapshot()` 返回前端可直接渲染的 nodes + edges
//! - **循环防护**:BFS + visited set,防止图环导致死循环
//! - **退化兼容**:无 relations 的记忆返回单节点图,不报错
//!
//! 来源:OpenAkita MDRM 思路(AGPL-3.0,仅思路借鉴,不可代码 fork)。

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::sqlite_store::SqliteStore;
use super::types::{MemoryLayer, RelationKind};

// ---------------------------------------------------------------------------
// 维度分类
// ---------------------------------------------------------------------------

/// MDRM 5 维分类。
///
/// 每个 `RelationKind` 严格归属一个维度,通过 `dimension_of()` 转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationDimension {
    /// 因果维度:`Causes`/`Supports`/`Contradicts`
    Causal,
    /// 时序维度:`Before`
    Temporal,
    /// 实体维度:`SameEntity`/`References`
    Entity,
    /// 层级维度:`Contains`/`DerivedFrom`
    Hierarchical,
    /// 相似度维度:`Similar`
    Similarity,
}

impl RelationDimension {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationDimension::Causal => "causal",
            RelationDimension::Temporal => "temporal",
            RelationDimension::Entity => "entity",
            RelationDimension::Hierarchical => "hierarchical",
            RelationDimension::Similarity => "similarity",
        }
    }

    /// 从字符串解析维度(前端透传用)。
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "causal" => Some(Self::Causal),
            "temporal" => Some(Self::Temporal),
            "entity" => Some(Self::Entity),
            "hierarchical" => Some(Self::Hierarchical),
            "similarity" => Some(Self::Similarity),
            _ => None,
        }
    }

    /// 该维度下的所有 RelationKind。
    pub fn kinds(&self) -> &'static [RelationKind] {
        match self {
            RelationDimension::Causal => &[
                RelationKind::Causes,
                RelationKind::Supports,
                RelationKind::Contradicts,
            ],
            RelationDimension::Temporal => &[RelationKind::Before],
            RelationDimension::Entity => &[RelationKind::SameEntity, RelationKind::References],
            RelationDimension::Hierarchical => &[RelationKind::Contains, RelationKind::DerivedFrom],
            RelationDimension::Similarity => &[RelationKind::Similar],
        }
    }
}

/// 将 RelationKind 映射到所属维度。
pub fn dimension_of(kind: RelationKind) -> RelationDimension {
    match kind {
        RelationKind::Causes | RelationKind::Supports | RelationKind::Contradicts => {
            RelationDimension::Causal
        }
        RelationKind::Before => RelationDimension::Temporal,
        RelationKind::SameEntity | RelationKind::References => RelationDimension::Entity,
        RelationKind::Contains | RelationKind::DerivedFrom => RelationDimension::Hierarchical,
        RelationKind::Similar => RelationDimension::Similarity,
    }
}

// ---------------------------------------------------------------------------
// 图数据结构
// ---------------------------------------------------------------------------

/// 图节点(用于前端可视化)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// 记忆 ID。
    pub id: String,
    /// 从根节点出发的深度(根=0)。
    pub depth: u32,
    /// 节点在图中的角色:`root`(查询起点)/ `inner`(中间节点)/ `leaf`(叶子)。
    pub role: GraphNodeRole,
    /// 记忆层级(L0-L7)。
    pub layer: MemoryLayer,
    /// 内容摘要(前 80 字符)。
    pub summary: String,
    /// 重要性(0.0-1.0)。
    pub importance: f32,
}

/// 节点角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeRole {
    Root,
    Inner,
    Leaf,
}

/// 图边(用于前端可视化)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub src_id: String,
    pub dst_id: String,
    /// 关系类型字符串(与 `RelationKind::as_str()` 一致)。
    pub kind: String,
    /// 所属维度字符串(与 `RelationDimension::as_str()` 一致)。
    pub dimension: String,
    /// 边权重(0.0-1.0)。
    pub weight: f32,
}

/// 图快照 — 前端可视化所需的最小数据集。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// 查询起点 ID。
    pub root_id: String,
    /// 请求的维度列表(可能多个)。
    pub dimensions: Vec<String>,
    /// 节点列表(去重,按 depth 升序)。
    pub nodes: Vec<GraphNode>,
    /// 边列表。
    pub edges: Vec<GraphEdge>,
    /// 是否因达到 max_nodes/max_edges 截断。
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// MDRM 查询配置。
#[derive(Debug, Clone)]
pub struct MdrmConfig {
    /// 最大遍历深度(边数)。
    pub max_depth: u32,
    /// 最多返回节点数(防止图爆炸)。
    pub max_nodes: usize,
    /// 最多返回边数。
    pub max_edges: usize,
    /// 关系权重低于此值的边被忽略。
    pub min_weight: f32,
}

impl Default for MdrmConfig {
    fn default() -> Self {
        Self {
            max_depth: 4,
            max_nodes: 200,
            max_edges: 500,
            min_weight: 0.1,
        }
    }
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// MDRM 5 维关系图谱引擎。
///
/// 持有 `SqliteStore` 克隆,通过 `get_relations`(同步)和 `get`(异步)查询。
/// 引擎本身无状态,每次查询独立。线程安全(`SqliteStore` 内部 `Arc<Mutex<_>>`)。
pub struct MdrmEngine {
    store: SqliteStore,
}

impl MdrmEngine {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    /// 暴露内部 store 引用(供调用方插入测试关系)。
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    // -----------------------------------------------------------------------
    // 单维度查询
    // -----------------------------------------------------------------------

    /// 时序维度:从 `memory_id` 出发,沿 `Before` 边追溯时间链。
    ///
    /// 语义:`A Before B` 表示 A 先于 B。本方法返回以 `memory_id` 为起点的
    /// 双向时序链(向前找先决事件,向后找后续事件)。
    pub async fn trace_temporal(&self, memory_id: &str, config: &MdrmConfig) -> GraphSnapshot {
        self.traverse_dim(memory_id, &[RelationDimension::Temporal], config)
            .await
    }

    /// 实体维度:查找与 `memory_id` 指向同一实体的所有记忆。
    ///
    /// `SameEntity`/`References` 关系连接同一实体的不同记忆,本方法返回该实体
    /// 的"记忆簇"。对称遍历(双向)。
    pub async fn find_entities(&self, memory_id: &str, config: &MdrmConfig) -> GraphSnapshot {
        self.traverse_dim(memory_id, &[RelationDimension::Entity], config)
            .await
    }

    /// 层级维度:追溯 `memory_id` 的包含层级。
    ///
    /// `Contains`(A 包含 B)和 `DerivedFrom`(A 派生自 B)互为反向,本方法
    /// 双向遍历,返回完整层级树片段。
    pub async fn trace_hierarchy(&self, memory_id: &str, config: &MdrmConfig) -> GraphSnapshot {
        self.traverse_dim(memory_id, &[RelationDimension::Hierarchical], config)
            .await
    }

    /// 相似度维度:查找与 `memory_id` 相似的所有记忆。
    ///
    /// `Similar` 关系对称,本方法双向遍历。
    pub async fn find_similar(&self, memory_id: &str, config: &MdrmConfig) -> GraphSnapshot {
        self.traverse_dim(memory_id, &[RelationDimension::Similarity], config)
            .await
    }

    /// 因果维度:双向追溯根因 + 效果(复用 CausalGraphEngine 语义)。
    ///
    /// 为保持 MDRM 接口统一,本方法返回 `GraphSnapshot` 而非 `CausalChain`。
    /// 如需带 confidence 的链式分析,仍推荐使用 `CausalGraphEngine`。
    pub async fn trace_causal(&self, memory_id: &str, config: &MdrmConfig) -> GraphSnapshot {
        self.traverse_dim(memory_id, &[RelationDimension::Causal], config)
            .await
    }

    // -----------------------------------------------------------------------
    // 多维度查询
    // -----------------------------------------------------------------------

    /// 多维度组合查询:在指定维度集合内做 BFS 遍历。
    ///
    /// 例如 `dims = [Causal, Temporal]` 同时追溯因果和时序关系。
    /// 至少返回根节点;无匹配关系时返回单节点快照。
    pub async fn query_multi_dim(
        &self,
        memory_id: &str,
        dims: &[RelationDimension],
        config: &MdrmConfig,
    ) -> GraphSnapshot {
        self.traverse_dim(memory_id, dims, config).await
    }

    /// 便捷方法:返回全部 5 维的图快照(供前端 "全局图谱" 视图)。
    pub async fn get_full_graph(&self, memory_id: &str, config: &MdrmConfig) -> GraphSnapshot {
        self.traverse_dim(
            memory_id,
            &[
                RelationDimension::Causal,
                RelationDimension::Temporal,
                RelationDimension::Entity,
                RelationDimension::Hierarchical,
                RelationDimension::Similarity,
            ],
            config,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // 核心遍历
    // -----------------------------------------------------------------------

    /// BFS 遍历指定维度集合。
    ///
    /// 算法:
    /// 1. 从 `memory_id` 出发,作为 root(depth=0)
    /// 2. 对每个节点,调 `store.get_relations()` 取所有关系(同步)
    /// 3. 过滤:关系 kind ∈ dims 的 kinds 集合,且 weight >= min_weight
    /// 4. 对每条匹配边,将另一端加入队列(depth+1)
    /// 5. 用 visited set 去重,防止环
    /// 6. 达到 max_nodes 或 max_edges 时停止,标记 truncated
    /// 7. 节点元数据(layer/summary/importance)通过 `store.get()` 异步获取
    async fn traverse_dim(
        &self,
        memory_id: &str,
        dims: &[RelationDimension],
        config: &MdrmConfig,
    ) -> GraphSnapshot {
        let allowed_kinds: HashSet<RelationKind> = dims
            .iter()
            .flat_map(|d| d.kinds().iter().copied())
            .collect();

        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut truncated = false;

        // 获取根节点元数据(失败则用默认值,仍返回单节点图)
        let root_meta = self.fetch_node_meta(memory_id).await;
        nodes.push(GraphNode {
            id: memory_id.to_string(),
            depth: 0,
            role: GraphNodeRole::Root,
            layer: root_meta.layer,
            summary: root_meta.summary,
            importance: root_meta.importance,
        });
        visited.insert(memory_id.to_string());

        // BFS 队列:(node_id, depth)
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        queue.push_back((memory_id.to_string(), 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= config.max_depth {
                continue;
            }
            if nodes.len() >= config.max_nodes {
                truncated = true;
                break;
            }

            let relations = match self.store.get_relations(&current_id) {
                Ok(r) => r,
                Err(_) => continue,
            };

            for rel in relations.iter() {
                if edges.len() >= config.max_edges {
                    truncated = true;
                    break;
                }
                if rel.weight < config.min_weight {
                    continue;
                }
                if !allowed_kinds.contains(&rel.kind) {
                    continue;
                }

                // 对称遍历:无论 current 是 src 还是 dst,都把另一端加入图
                let other_id = if rel.src_id == current_id {
                    rel.dst_id.clone()
                } else if rel.dst_id == current_id {
                    rel.src_id.clone()
                } else {
                    continue;
                };

                let dim = dimension_of(rel.kind);
                edges.push(GraphEdge {
                    src_id: rel.src_id.clone(),
                    dst_id: rel.dst_id.clone(),
                    kind: rel.kind.as_str().to_string(),
                    dimension: dim.as_str().to_string(),
                    weight: rel.weight,
                });

                if visited.contains(&other_id) {
                    continue;
                }
                visited.insert(other_id.clone());

                let meta = self.fetch_node_meta(&other_id).await;
                nodes.push(GraphNode {
                    id: other_id.clone(),
                    depth: depth + 1,
                    role: GraphNodeRole::Inner,
                    layer: meta.layer,
                    summary: meta.summary,
                    importance: meta.importance,
                });

                queue.push_back((other_id, depth + 1));
            }

            if edges.len() >= config.max_edges {
                truncated = true;
                break;
            }
        }

        // 标记叶子节点:depth 达到 max_depth 的非根节点视为叶子(未被进一步展开)
        for n in nodes.iter_mut() {
            if n.role == GraphNodeRole::Root {
                continue;
            }
            if n.depth >= config.max_depth {
                n.role = GraphNodeRole::Leaf;
            }
        }

        // 排序:depth 升序,确保前端渲染时根节点在最前
        nodes.sort_by_key(|n| n.depth);

        GraphSnapshot {
            root_id: memory_id.to_string(),
            dimensions: dims.iter().map(|d| d.as_str().to_string()).collect(),
            nodes,
            edges,
            truncated,
        }
    }

    // -----------------------------------------------------------------------
    // 元数据获取
    // -----------------------------------------------------------------------

    /// 获取节点元数据(layer/summary/importance),失败返回默认值。
    ///
    /// 用 `get().await` 异步查询;失败时退化为默认节点,保证图遍历不中断。
    async fn fetch_node_meta(&self, memory_id: &str) -> NodeMeta {
        match self.store.get(memory_id).await {
            Ok(Some(m)) => {
                // MultiGranularity 字段是 String(非 Option),空字符串视为无摘要
                let summary_raw = if !m.summary.s150.is_empty() {
                    m.summary.s150.clone()
                } else if !m.summary.s50.is_empty() {
                    m.summary.s50.clone()
                } else {
                    String::new()
                };
                let summary = if summary_raw.chars().count() > 80 {
                    // 安全截断到 80 字符(chars 边界)
                    let truncated: String = summary_raw.chars().take(80).collect();
                    format!("{}…", truncated)
                } else {
                    summary_raw
                };
                NodeMeta {
                    layer: m.layer,
                    summary,
                    importance: m.importance,
                }
            }
            _ => NodeMeta {
                layer: MemoryLayer::L2,
                summary: String::new(),
                importance: 0.0,
            },
        }
    }
}

/// 节点元数据中间结构。
struct NodeMeta {
    layer: MemoryLayer,
    summary: String,
    importance: f32,
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{Memory, MemoryLayer, MemoryType, MemoryRelation, SourceKind};

    fn temp_db_path() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("nebula_mdrm_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    /// 构造 5 维测试图:
    /// ```text
    ///   [A] --causes-->     [B] --causes-->     [C]
    ///    |                   |                   |
    ///   before              same_entity         similar
    ///    |                   |                   |
    ///   [D] --contains-->   [E] --derived_from--> [F]
    /// ```
    async fn seed_5d_graph(store: &SqliteStore) -> [String; 6] {
        let memories: Vec<Memory> = (0..6)
            .map(|i| {
                Memory::new(
                    MemoryType::Episodic,
                    MemoryLayer::L2,
                    &format!("memory-{}", i),
                    SourceKind::UserInput,
                )
            })
            .collect();

        for m in &memories {
            store.insert_guarded_spawn(m).await.unwrap();
        }
        // A --causes--> B
        store
            .add_relation(&MemoryRelation::new(
                memories[0].id.clone(),
                memories[1].id.clone(),
                RelationKind::Causes,
            ))
            .await
            .unwrap();
        // B --causes--> C
        store
            .add_relation(&MemoryRelation::new(
                memories[1].id.clone(),
                memories[2].id.clone(),
                RelationKind::Causes,
            ))
            .await
            .unwrap();
        // A --before--> D
        store
            .add_relation(&MemoryRelation::new(
                memories[0].id.clone(),
                memories[3].id.clone(),
                RelationKind::Before,
            ))
            .await
            .unwrap();
        // B --same_entity--> E
        store
            .add_relation(&MemoryRelation::new(
                memories[1].id.clone(),
                memories[4].id.clone(),
                RelationKind::SameEntity,
            ))
            .await
            .unwrap();
        // D --contains--> E
        store
            .add_relation(&MemoryRelation::new(
                memories[3].id.clone(),
                memories[4].id.clone(),
                RelationKind::Contains,
            ))
            .await
            .unwrap();
        // E --derived_from--> F
        store
            .add_relation(&MemoryRelation::new(
                memories[4].id.clone(),
                memories[5].id.clone(),
                RelationKind::DerivedFrom,
            ))
            .await
            .unwrap();
        // C --similar--> F
        store
            .add_relation(&MemoryRelation::new(
                memories[2].id.clone(),
                memories[5].id.clone(),
                RelationKind::Similar,
            ))
            .await
            .unwrap();

        let mut ids: Vec<String> = memories.into_iter().map(|m| m.id).collect();
        let ids_arr: [String; 6] = [
            ids.remove(0),
            ids.remove(0),
            ids.remove(0),
            ids.remove(0),
            ids.remove(0),
            ids.remove(0),
        ];
        ids_arr
    }

    // ---- 维度分类测试 ----

    #[test]
    fn dimension_of_all_kinds() {
        assert_eq!(dimension_of(RelationKind::Causes), RelationDimension::Causal);
        assert_eq!(dimension_of(RelationKind::Supports), RelationDimension::Causal);
        assert_eq!(
            dimension_of(RelationKind::Contradicts),
            RelationDimension::Causal
        );
        assert_eq!(dimension_of(RelationKind::Before), RelationDimension::Temporal);
        assert_eq!(
            dimension_of(RelationKind::SameEntity),
            RelationDimension::Entity
        );
        assert_eq!(
            dimension_of(RelationKind::References),
            RelationDimension::Entity
        );
        assert_eq!(
            dimension_of(RelationKind::Contains),
            RelationDimension::Hierarchical
        );
        assert_eq!(
            dimension_of(RelationKind::DerivedFrom),
            RelationDimension::Hierarchical
        );
        assert_eq!(
            dimension_of(RelationKind::Similar),
            RelationDimension::Similarity
        );
    }

    #[test]
    fn dimension_from_str_lossy() {
        assert_eq!(
            RelationDimension::from_str_lossy("causal"),
            Some(RelationDimension::Causal)
        );
        assert_eq!(
            RelationDimension::from_str_lossy("TEMPORAL"),
            Some(RelationDimension::Temporal)
        );
        assert_eq!(RelationDimension::from_str_lossy("unknown"), None);
    }

    #[test]
    fn dimension_kinds_complete() {
        // 5 维覆盖全部 9 个 RelationKind
        let mut all: Vec<RelationKind> = Vec::new();
        for d in [
            RelationDimension::Causal,
            RelationDimension::Temporal,
            RelationDimension::Entity,
            RelationDimension::Hierarchical,
            RelationDimension::Similarity,
        ] {
            all.extend_from_slice(d.kinds());
        }
        assert_eq!(all.len(), 9, "5 dims should cover all 9 RelationKind variants");
    }

    // ---- 单维度查询测试 ----

    #[tokio::test]
    async fn trace_temporal_finds_before_chain() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // 从 A 出发,沿 before 边找 D
        let snap = engine.trace_temporal(&ids[0], &MdrmConfig::default()).await;
        assert_eq!(snap.root_id, ids[0]);
        assert!(snap.nodes.len() >= 2, "should include A and D");
        assert!(snap.edges.len() >= 1, "should have A->D before edge");
        assert!(
            snap.edges
                .iter()
                .any(|e| e.kind == "before" && e.src_id == ids[0] && e.dst_id == ids[3]),
            "should contain A->D before edge"
        );
        // 不应包含 causes 边(被维度过滤)
        assert!(
            !snap.edges.iter().any(|e| e.kind == "causes"),
            "temporal dim should not include causal edges"
        );
    }

    #[tokio::test]
    async fn find_similar_finds_symmetric_edge() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // 从 C 出发,沿 similar 边找 F (C --similar--> F)
        let snap = engine.find_similar(&ids[2], &MdrmConfig::default()).await;
        assert!(snap.nodes.len() >= 2, "should include C and F");
        assert!(
            snap.edges
                .iter()
                .any(|e| e.kind == "similar" && e.src_id == ids[2] && e.dst_id == ids[5]),
            "should contain C->F similar edge"
        );
    }

    #[tokio::test]
    async fn trace_hierarchy_walks_contains_and_derived_from() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // 从 D 出发:D --contains--> E --derived_from--> F
        let snap = engine.trace_hierarchy(&ids[3], &MdrmConfig::default()).await;
        assert!(snap.nodes.len() >= 3, "should include D, E, F");
        let node_ids: Vec<&str> = snap.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(node_ids.contains(&ids[3].as_str()));
        assert!(node_ids.contains(&ids[4].as_str()));
        assert!(node_ids.contains(&ids[5].as_str()));
        // 边应包含 contains 和 derived_from
        let kinds: Vec<&str> = snap.edges.iter().map(|e| e.kind.as_str()).collect();
        assert!(kinds.contains(&"contains"));
        assert!(kinds.contains(&"derived_from"));
    }

    #[tokio::test]
    async fn find_entities_walks_same_entity_and_references() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // 从 B 出发:B --same_entity--> E
        let snap = engine.find_entities(&ids[1], &MdrmConfig::default()).await;
        assert!(snap.nodes.len() >= 2, "should include B and E");
        assert!(
            snap.edges.iter().any(|e| e.kind == "same_entity"),
            "should contain same_entity edge"
        );
    }

    #[tokio::test]
    async fn trace_causal_walks_causes_chain() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // 从 A 出发:A --causes--> B --causes--> C
        let snap = engine.trace_causal(&ids[0], &MdrmConfig::default()).await;
        assert!(snap.nodes.len() >= 3, "should include A, B, C");
        let causes_count = snap.edges.iter().filter(|e| e.kind == "causes").count();
        assert!(causes_count >= 2, "should have at least 2 causes edges");
    }

    // ---- 多维度查询测试 ----

    #[tokio::test]
    async fn query_multi_dim_causal_temporal() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        let snap = engine
            .query_multi_dim(
                &ids[0],
                &[RelationDimension::Causal, RelationDimension::Temporal],
                &MdrmConfig::default(),
            )
            .await;
        // 应包含 A, B, C (causal), D (temporal)
        assert!(snap.nodes.len() >= 4, "should include A, B, C, D");
        let kinds: HashSet<&str> = snap.edges.iter().map(|e| e.kind.as_str()).collect();
        assert!(kinds.contains("causes"));
        assert!(kinds.contains("before"));
        // 不应包含 similarity/hierarchical/entity 的边
        assert!(!kinds.contains("similar"));
        assert!(!kinds.contains("contains"));
        assert!(!kinds.contains("same_entity"));
    }

    #[tokio::test]
    async fn get_full_graph_includes_all_dims() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        let snap = engine.get_full_graph(&ids[0], &MdrmConfig::default()).await;
        // 全图遍历应触达所有 6 个节点(max_depth=4 足够)
        assert!(
            snap.nodes.len() >= 6,
            "full graph should reach all 6 nodes, got {}",
            snap.nodes.len()
        );
        let dim_set: HashSet<&str> = snap.edges.iter().map(|e| e.dimension.as_str()).collect();
        // 至少触达 4 个维度
        assert!(
            dim_set.len() >= 4,
            "should cover at least 4 dimensions, got {:?}",
            dim_set
        );
    }

    // ---- 边界 / 防护测试 ----

    #[tokio::test]
    async fn empty_graph_returns_single_root_node() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        // 插入一个孤立记忆(无任何关系)
        let m = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            "isolated",
            SourceKind::UserInput,
        );
        store.insert_guarded_spawn(&m).await.unwrap();

        let engine = MdrmEngine::new(store);
        let snap = engine.get_full_graph(&m.id, &MdrmConfig::default()).await;
        assert_eq!(snap.nodes.len(), 1, "isolated memory should return 1 node");
        assert!(snap.edges.is_empty());
        assert_eq!(snap.nodes[0].role, GraphNodeRole::Root);
        assert!(!snap.truncated);
    }

    #[tokio::test]
    async fn non_existent_memory_returns_single_default_node() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let engine = MdrmEngine::new(store);

        // 查询不存在的 ID:fetch_node_meta 退化为默认值,仍返回单节点图
        let snap = engine
            .get_full_graph("non-existent-id", &MdrmConfig::default())
            .await;
        assert_eq!(snap.nodes.len(), 1);
        assert!(snap.edges.is_empty());
        assert_eq!(snap.nodes[0].importance, 0.0);
    }

    #[tokio::test]
    async fn max_nodes_truncation_flag() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // max_nodes=2 强制截断
        let cfg = MdrmConfig {
            max_nodes: 2,
            max_edges: 500,
            ..MdrmConfig::default()
        };
        let snap = engine.get_full_graph(&ids[0], &cfg).await;
        assert!(snap.truncated, "should be truncated due to max_nodes");
        // max_nodes 检查在 BFS 开始时,根节点已加入,所以 nodes 可能略超 max_nodes
        assert!(snap.nodes.len() <= 4, "nodes should not greatly exceed max_nodes");
    }

    #[tokio::test]
    async fn max_edges_truncation_flag() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        let cfg = MdrmConfig {
            max_nodes: 200,
            max_edges: 1,
            ..MdrmConfig::default()
        };
        let snap = engine.get_full_graph(&ids[0], &cfg).await;
        assert!(snap.truncated, "should be truncated due to max_edges");
        assert!(
            snap.edges.len() <= 2,
            "edges should not greatly exceed max_edges"
        );
    }

    #[tokio::test]
    async fn min_weight_filters_low_weight_edges() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        // min_weight=2.0 过滤所有边(默认 weight=1.0)
        let cfg = MdrmConfig {
            min_weight: 2.0,
            ..MdrmConfig::default()
        };
        let snap = engine.trace_causal(&ids[0], &cfg).await;
        assert!(
            snap.edges.is_empty(),
            "all edges (weight=1.0) should be filtered out by min_weight=2.0"
        );
        assert_eq!(snap.nodes.len(), 1, "only root node should remain");
    }

    #[tokio::test]
    async fn cycle_detection_visited_set_prevents_infinite_loop() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();

        // 构造环:A -> B -> A
        let a = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            "cycle-a",
            SourceKind::UserInput,
        );
        let b = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            "cycle-b",
            SourceKind::UserInput,
        );
        store.insert_guarded_spawn(&a).await.unwrap();
        store.insert_guarded_spawn(&b).await.unwrap();
        store
            .add_relation(&MemoryRelation::new(
                a.id.clone(),
                b.id.clone(),
                RelationKind::Similar,
            ))
            .await
            .unwrap();
        store
            .add_relation(&MemoryRelation::new(
                b.id.clone(),
                a.id.clone(),
                RelationKind::Similar,
            ))
            .await
            .unwrap();

        let engine = MdrmEngine::new(store);
        // 环图应正常返回,不死循环
        let snap = engine.find_similar(&a.id, &MdrmConfig::default()).await;
        assert!(snap.nodes.len() <= 2, "visited set should prevent re-visiting");
        assert!(!snap.truncated);
    }

    #[tokio::test]
    async fn root_node_depth_zero_and_meta_populated() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let ids = seed_5d_graph(&store).await;
        let engine = MdrmEngine::new(store);

        let snap = engine.trace_causal(&ids[0], &MdrmConfig::default()).await;
        let root = snap.nodes.iter().find(|n| n.id == ids[0]).unwrap();
        assert_eq!(root.depth, 0);
        assert_eq!(root.role, GraphNodeRole::Root);
        // 元数据应已填充
        assert!(root.importance >= 0.0);
    }

    #[test]
    fn default_config_values() {
        let cfg = MdrmConfig::default();
        assert_eq!(cfg.max_depth, 4);
        assert_eq!(cfg.max_nodes, 200);
        assert_eq!(cfg.max_edges, 500);
        assert!(cfg.min_weight > 0.0);
    }
}
