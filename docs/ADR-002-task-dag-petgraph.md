# ADR-002: TaskDag 依赖管理（petgraph）

> **状态**: 已接受  
> **日期**: 2026-07-05  
> **决策者**: 架构组  
> **关联**: SWARM_EVOLUTION_DESIGN_v2.md §3  
> **解决**: P0-10 — petgraph 依赖未引入；P1-2 — DAG 节点缺少 work_type_hint

---

## Context（背景）

### 问题

蜂群进化设计 v2.0 §3 提出了 DAG 依赖管理方案，但：

1. **petgraph 依赖未引入**：`Cargo.toml` 中没有 `petgraph`，M3 启动即阻塞
2. **SubTask 缺少 `work_type_hint` 字段**：无法按子任务指定模型路由（EA-1 FINDING-1.4）
3. **WorkerCapability 枚举未定义**：v2.0 引入但未列出成员（EA-1 FINDING-1.13）
4. **SubTaskResultMap placeholder 注入风险**：直接字符串替换存在 prompt 注入风险（EA-4 P0-3）
5. **DAG 缓存与 SemanticCache 关系未定义**（EA-2 P1-2）

### 现有代码

项目当前无 DAG 相关代码。`SwarmOrchestrator::execute()` 是线性 fan-out（2-6 Worker 并行执行同一任务），无子任务依赖管理。

---

## Decision（决策）

### 1. 引入 petgraph 0.6

```toml
# Cargo.toml [dependencies]
petgraph = { version = "0.6", default-features = false }
```

> `default-features = false` 排除不必要的 `quickcheck` / `serde` 依赖，减小二进制体积。

### 2. TaskDag 结构

```rust
use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};

/// DAG 节点：子任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// 唯一标识（如 "st_1", "st_2"）
    pub id: String,
    /// 任务描述（prompt 主体）
    pub prompt: String,
    /// Worker 能力需求
    pub capabilities: Vec<WorkerCapability>,
    /// 模型路由提示（P1-2 修复）
    /// MasterAgent 拆解时根据任务性质标注：
    /// - 写作类 → 远端 WorkType
    /// - 搜索/整理类 → 本地 WorkType
    /// - None → 回退到 SwarmWorker 默认 + ModelRouter 动态升级
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_type_hint: Option<WorkType>,
    /// Worker 数量（默认 3，clamped 2..=6）
    #[serde(default = "default_worker_count")]
    pub worker_count: u32,
    /// 最大重试次数（默认 1）
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// 指定 Agent 类型（如 ["generic", "generic"]）
    #[serde(default)]
    pub agent_kinds: Vec<String>,
    /// 失败策略
    #[serde(default)]
    pub on_failure: FailureStrategy,
}

/// DAG 边：依赖关系
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// 依赖类型
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// 必须等上游完成后才能开始
    FinishToStart,
    /// 上游启动后即可开始（目前未使用，预留）
    StartToStart,
}

/// 失败策略
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FailureStrategy {
    /// 重试（默认，最多 max_retries 次）
    #[default]
    Retry,
    /// 跳过此子任务，下游用空结果继续
    Skip,
    /// 整个 DAG 失败
    Fail,
    /// 等待用户手动决定
    Manual,
}

/// Worker 能力枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WorkerCapability {
    Summarize,
    WriteShort,
    WriteLong,
    Search,
    Generate,
    CodeExecute,
    FileOperate,
    MediaProcess,
}

/// 任务 DAG
pub struct TaskDag {
    /// petgraph 有向图
    graph: DiGraph<SubTask, DependencyEdge>,
    /// 入口节点（无入边的节点，按拓扑排序）
    roots: Vec<NodeIndex>,
}

impl TaskDag {
    /// 从 JSON 解析 DAG（MasterAgent LLM 拆解输出）
    pub fn from_json(json: &str) -> Result<Self> {
        let parsed: TaskDagJson = serde_json::from_str(json)?;
        Self::from_parsed(parsed)
    }

    /// 拓扑排序：返回可并行执行的层级
    pub fn topological_layers(&self) -> Result<Vec<Vec<NodeIndex>>> {
        petgraph::algo::toposort(&self.graph, None)
            .map_err(|_| anyhow::anyhow!("DAG contains cycle"))
            .map(|sorted| {
                // 按入度分层：同层节点可并行
                self.group_by_layers(&sorted)
            })
    }

    /// 循环检测
    pub fn has_cycle(&self) -> bool {
        petgraph::algo::is_cyclic_directed(&self.graph)
    }

    /// 获取节点
    pub fn node(&self, idx: NodeIndex) -> Option<&SubTask> {
        self.graph.node_weight(idx)
    }
}
```

### 3. SubTaskResultMap + 注入防护

```rust
/// 子任务结果映射表
pub struct SubTaskResultMap {
    results: HashMap<String, SubTaskResult>,
}

impl SubTaskResultMap {
    /// 替换 prompt 中的 {{st_xxx.output}} 占位符
    ///
    /// 安全措施（P0-3 EA-4 修复）：
    /// 1. 上游输出包装在 <upstream_result> 标签中
    /// 2. 替换前对上游输出调用 injection_guard 扫描
    /// 3. 命中 Critical/High 时替换为 [BLOCKED: injection detected]
    pub fn resolve_placeholders(&self, prompt: &str) -> String {
        let mut resolved = prompt.to_string();
        for (id, result) in &self.results {
            let placeholder = format!("{{{{{}.output}}}}}", id);

            // 注入扫描
            let safe_output = if injection_guard::full_injection_scan(&result.output).is_clean() {
                // 包装在结构化标签中
                format!(
                    "<upstream_result sub_task_id=\"{}\">\n{}\n</upstream_result>",
                    id, result.output
                )
            } else {
                tracing::warn!(sub_task_id = %id, "injection detected in upstream output, blocked");
                "[BLOCKED: injection detected]".to_string()
            };

            resolved = resolved.replace(&placeholder, &safe_output);
        }
        resolved
    }
}
```

### 4. DAG 缓存（独立于 SemanticCache）

```rust
/// DAG 拆解结果缓存（P1-2 EA-2 修复）
///
/// 独立于 SemanticCache：
/// - 值类型是 TaskDag（不是 ChatResponse）
/// - 阈值 0.85（比 SemanticCache 的 0.92 宽松）
/// - 复用 Embedder + LanceStore 但独立管理 entries
pub struct DecompositionCache {
    embedder: Arc<Embedder>,
    store: Arc<dyn VectorStore>,
    /// 缓存阈值（相似度 > 0.85 时复用之前的 DAG）
    threshold: f32,
}

impl DecompositionCache {
    /// 查询缓存的 DAG
    pub async fn get(&self, query: &str) -> Option<TaskDag> {
        let embedding = self.embedder.embed(query).await.ok()?;
        let hits = self.store.search(&embedding, 1, 0.85).await.ok()?;
        // ...
    }

    /// 存入 DAG 拆解结果
    pub async fn store(&self, query: &str, dag: &TaskDag) -> Result<()> {
        let embedding = self.embedder.embed(query).await?;
        let dag_json = serde_json::to_string(dag)?;
        // ...
    }
}
```

---

## Consequences（后果）

### 正面

- **petgraph 0.6 成熟稳定**，与 Rust 1.75 / edition 2021 兼容
- **拓扑排序 + 循环检测**用 petgraph 内置算法，零手写
- **work_type_hint** 让 MasterAgent 可按子任务指定模型路由
- **注入防护**在 placeholder 替换时执行，阻断跨子任务注入
- **DAG 缓存独立**，不污染 SemanticCache

### 负面

- **新增依赖**：petgraph 0.6（~50KB 编译产物）
- **SubTask 字段较多**：8 个字段，但从 JSON 解析时大部分有默认值
- **DecompositionCache 额外内存**：LanceDB 中多一个 collection

---

## Alternatives（备选方案）

### 方案 B：手写 DAG（已拒绝）

不引入 petgraph，用 `HashMap<String, Vec<String>>` 手写邻接表。

**拒绝原因**：手写拓扑排序 + 循环检测容易出 bug；petgraph 已经过广泛验证。

### 方案 C：DAG 缓存复用 SemanticCache（已拒绝）

把 TaskDag 序列化为 JSON 塞进 SemanticCache 的 `response` 字段。

**拒绝原因**（EA-2 P1-2）：
- 值类型不同（TaskDag vs ChatResponse）
- 阈值不同（0.85 vs 0.92）
- 缓存语义不同（结构化 vs 语义近邻）

---

## 实施清单

1. `Cargo.toml` 添加 `petgraph = { version = "0.6", default-features = false }`
2. 新建 `src-tauri/src/swarm/dag.rs`：TaskDag + SubTask + DependencyEdge + FailureStrategy
3. 新建 `src-tauri/src/swarm/result_map.rs`：SubTaskResultMap + 注入防护
4. 新建 `src-tauri/src/swarm/decomp_cache.rs`：DecompositionCache
5. 单元测试：拓扑排序 / 循环检测 / placeholder 替换 / 注入防护 / DAG 缓存命中
