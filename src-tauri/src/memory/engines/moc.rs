//! T-E-B-15: AI 自动整理 MOC（Map of Content）引擎。
//!
//! 设计文档 v7.0 §3.5 MOC 自动整理 — 使用 LLM 分析记忆节点，
//! 自动生成层次化的 Map of Content 结构。
//!
//! ## 工作流程
//!
//! 1. `cluster_items` 按共享标签（首个 tag）对记忆项进行初步聚类。
//! 2. `merge_small_clusters` 将小于 `min_cluster_size` 的聚类合并到
//!    "misc" 顶层聚类中，避免噪声小簇。
//! 3. `generate_title`（可选）调用 LLM 为每个聚类生成简洁中文标题；
//!    LLM 不可用时回退到 `centroid_keyword`。
//! 4. `build_hierarchy` 按 `centroid_keyword` 前缀构建多层 MOC 树，
//!    深度受 `MocGeneratorConfig::max_depth` 约束。
//! 5. `export_moc` 将 `MocTree` 序列化为 Markdown / JSON / Obsidian
//!    三种格式之一。
//!
//! ## 设计约束
//!
//! * **LLM 抽象** — 通过 [`LlmClient`] trait 抽象 LLM 调用，不直接
//!   依赖具体实现（如 `LlmGateway`）。测试中可注入 stub 客户端，
//!   生产中可注入任意 `Arc<dyn LlmClient>`。
//! * **离线可用** — LLM 未配置或调用失败时回退到确定性算法，
//!   保证后台 worker 在离线环境不 panic。
//! * **确定性输出** — 聚类与层次构建内部使用 `BTreeMap` 与排序，
//!   相同输入产生相同输出（除 LLM 生成标题外）。

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// LLM 抽象 trait
// ---------------------------------------------------------------------------

/// LLM 客户端抽象。
///
/// 不直接依赖具体 LLM 实现（如 `crate::llm::LlmGateway`），便于在
/// 测试中注入 mock 客户端，并允许未来接入不同模型供应商。
///
/// 实现者需保证 `Send + Sync`（`Arc<dyn LlmClient>` 跨 await 边界）。
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// 根据给定 prompt 生成文本。
    async fn generate(&self, prompt: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 记忆项（MOC 生成的输入）。
///
/// 字段保持精简：仅包含聚类与导出所需信息。`embedding_id` 为可选，
/// 便于未来接入基于向量的聚类（当前实现使用 tag 聚类）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoItem {
    /// 记忆唯一 id（与 `Memory::id` 对应）。
    pub id: String,
    /// 记忆正文内容。
    pub content: String,
    /// 用户/系统打的标签集合。
    pub tags: Vec<String>,
    /// Unix 时间戳（秒）。
    pub timestamp: i64,
    /// 向量存储中的 embedding id（可选）。
    pub embedding_id: Option<String>,
}

impl MemoItem {
    /// 快速构造（无 embedding_id，timestamp=0）。
    pub fn new(id: impl Into<String>, content: impl Into<String>, tags: Vec<String>) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
            tags,
            timestamp: 0,
            embedding_id: None,
        }
    }
}

/// 聚类 — 一组语义相近的记忆项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    /// 聚类内的记忆项。
    pub items: Vec<MemoItem>,
    /// 聚类中心关键词（用于分组与回退标题）。
    pub centroid_keyword: String,
    /// LLM 生成或回退得到的建议标题。
    pub suggested_title: String,
}

impl Cluster {
    /// 创建聚类；`suggested_title` 初始化为 `centroid_keyword`。
    pub fn new(items: Vec<MemoItem>, centroid_keyword: String) -> Self {
        let suggested_title = if centroid_keyword.is_empty() {
            String::new()
        } else {
            centroid_keyword.clone()
        };
        Self {
            items,
            centroid_keyword,
            suggested_title,
        }
    }

    /// 聚类内记忆项数量。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否为空聚类。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 聚类内所有记忆的 id 列表。
    pub fn memory_ids(&self) -> Vec<String> {
        self.items.iter().map(|m| m.id.clone()).collect()
    }
}

/// MOC 节点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MocNode {
    /// 节点 id（如 `moc-0`、`moc-0-1`）。
    pub id: String,
    /// 节点标题。
    pub title: String,
    /// 子节点。
    pub children: Vec<MocNode>,
    /// 该节点直接关联的记忆 id（叶子节点非空，中间节点通常为空）。
    pub memory_ids: Vec<String>,
    /// 节点层级（root=1）。
    pub level: u32,
    /// 节点摘要（人类可读的简短说明）。
    pub summary: String,
}

impl MocNode {
    /// 创建空节点。
    pub fn new(id: String, title: String, level: u32) -> Self {
        Self {
            id,
            title,
            children: Vec::new(),
            memory_ids: Vec::new(),
            level,
            summary: String::new(),
        }
    }
}

/// MOC 树。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MocTree {
    /// 顶层节点列表。
    pub root_nodes: Vec<MocNode>,
    /// 树中节点总数（含根节点）。
    pub total_nodes: usize,
    /// 最大深度（空树为 0，单层为 1）。
    pub max_depth: u32,
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// MOC 生成器配置。
#[derive(Debug, Clone)]
pub struct MocGeneratorConfig {
    /// MOC 树最大深度（≥1）。
    pub max_depth: u32,
    /// 聚类最小尺寸；小于此值的聚类会被合并到 misc。
    pub min_cluster_size: usize,
    /// LLM 模型标识（用于日志与导出元数据，不影响实际调用）。
    pub llm_model: String,
}

impl Default for MocGeneratorConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            min_cluster_size: 2,
            llm_model: "default".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// 导出格式
// ---------------------------------------------------------------------------

/// MOC 导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MocExportFormat {
    /// 标准 Markdown（# 标题层级 + 列表）。
    Markdown,
    /// JSON（`MocTree` 序列化结果）。
    Json,
    /// Obsidian 友好格式（YAML frontmatter + `[[wikilink]]`）。
    Obsidian,
}

// ---------------------------------------------------------------------------
// MocGenerator
// ---------------------------------------------------------------------------

/// MOC 生成器。
///
/// 无状态：所有方法均不修改内部字段（`llm` 为 `Arc` 共享只读），
/// 可安全并发调用。
pub struct MocGenerator {
    config: MocGeneratorConfig,
    llm: Option<Arc<dyn LlmClient>>,
}

impl MocGenerator {
    /// 创建生成器（无 LLM，仅使用回退标题）。
    pub fn new(config: MocGeneratorConfig) -> Self {
        Self { config, llm: None }
    }

    /// 注入 LLM 客户端，启用 LLM 标题生成。
    pub fn with_llm(mut self, llm: Arc<dyn LlmClient>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// 返回配置引用。
    pub fn config(&self) -> &MocGeneratorConfig {
        &self.config
    }

    /// 主入口：从一组记忆项生成 MOC 树。
    ///
    /// 步骤：聚类 → 合并小簇 → LLM 标题（可选）→ 构建层次。
    /// 空输入返回空树。LLM 不可用时使用回退标题，不阻塞流程。
    pub async fn generate(&self, memo_items: &[MemoItem]) -> Result<MocTree> {
        if memo_items.is_empty() {
            return Ok(MocTree::default());
        }

        // 1. 聚类
        let clusters = self.cluster_items(memo_items);

        // 2. 合并小聚类
        let clusters = self.merge_small_clusters(clusters, self.config.min_cluster_size);

        // 3. 为每个聚类生成标题（LLM 可选）
        let mut titled_clusters: Vec<Cluster> = Vec::with_capacity(clusters.len());
        for mut c in clusters {
            if let Some(llm) = &self.llm {
                match self.generate_title(&c).await {
                    Ok(t) => c.suggested_title = t,
                    Err(e) => {
                        warn!(
                            target: "nebula.moc",
                            error = %e,
                            "LLM title generation failed; using fallback"
                        );
                    }
                }
            }
            if c.suggested_title.is_empty() {
                c.suggested_title = derive_fallback_title(&c);
            }
            titled_clusters.push(c);
        }

        // 4. 构建层次
        let tree = self.build_hierarchy(titled_clusters);
        info!(
            target: "nebula.moc",
            total_nodes = tree.total_nodes,
            max_depth = tree.max_depth,
            "MOC generated"
        );
        Ok(tree)
    }

    /// 按共享 tag 聚类。
    ///
    /// 取每条记忆的**首个 tag**（小写化）作为聚类键；无 tag 的记忆
    /// 归入 `uncategorized` 聚类。输出按 `centroid_keyword` 字典序
    /// 排序以保证确定性。
    pub fn cluster_items(&self, items: &[MemoItem]) -> Vec<Cluster> {
        let mut by_tag: HashMap<String, Vec<MemoItem>> = HashMap::new();
        let mut no_tag: Vec<MemoItem> = Vec::new();

        for item in items {
            if item.tags.is_empty() {
                no_tag.push(item.clone());
            } else {
                let key = item.tags[0].to_lowercase();
                by_tag.entry(key).or_default().push(item.clone());
            }
        }

        let mut clusters: Vec<Cluster> = by_tag
            .into_iter()
            .map(|(tag, items)| Cluster::new(items, tag))
            .collect();

        if !no_tag.is_empty() {
            clusters.push(Cluster::new(no_tag, "uncategorized".to_string()));
        }

        clusters.sort_by(|a, b| a.centroid_keyword.cmp(&b.centroid_keyword));
        clusters
    }

    /// 通过 LLM 为聚类生成标题。
    ///
    /// LLM 未配置时返回 `Err`（调用方应回退到 `derive_fallback_title`）。
    /// LLM 返回空字符串或纯空白时同样返回 `Err`。
    pub async fn generate_title(&self, cluster: &Cluster) -> Result<String> {
        let llm = self.llm.as_ref().context("LLM client not configured")?;
        if cluster.items.is_empty() {
            anyhow::bail!("cannot generate title for empty cluster");
        }
        let prompt = build_title_prompt(cluster);
        let raw = llm.generate(&prompt).await?;
        // 容错：去除引号、书名号、首尾空白
        let title = raw
            .trim()
            .trim_matches(|c| c == '"' || c == '「' || c == '」' || c == '《' || c == '》')
            .trim()
            .to_string();
        if title.is_empty() {
            anyhow::bail!("LLM returned empty title");
        }
        Ok(title)
    }

    /// 构建层次结构。
    ///
    /// - `max_depth <= 1` 或聚类数 ≤ 1：生成单层树。
    /// - 否则按 `centroid_keyword` 首字符分组，生成两层树。
    ///
    /// 输出确定性：分组使用 `BTreeMap`，组内按 `centroid_keyword` 排序。
    pub fn build_hierarchy(&self, clusters: Vec<Cluster>) -> MocTree {
        let mut tree = MocTree::default();
        let max_depth = self.config.max_depth.max(1);

        // 单层分支
        if clusters.len() <= 1 || max_depth == 1 {
            for (i, c) in clusters.into_iter().enumerate() {
                let memory_ids = c.memory_ids();
                let summary = derive_cluster_summary(&c);
                let node = MocNode {
                    id: format!("moc-{}", i),
                    title: if c.suggested_title.is_empty() {
                        derive_fallback_title(&c)
                    } else {
                        c.suggested_title
                    },
                    children: Vec::new(),
                    memory_ids,
                    level: 1,
                    summary,
                };
                tree.total_nodes += 1;
                tree.root_nodes.push(node);
            }
            tree.max_depth = if tree.root_nodes.is_empty() { 0 } else { 1 };
            return tree;
        }

        // 多层分支：按首字符分组
        let groups = group_clusters_by_prefix(clusters);
        for (i, (group_title, group_clusters)) in groups.into_iter().enumerate() {
            let mut root = MocNode::new(format!("moc-{}", i), group_title, 1);
            let mut sorted_children = group_clusters;
            sorted_children.sort_by(|a, b| a.centroid_keyword.cmp(&b.centroid_keyword));
            for (j, c) in sorted_children.into_iter().enumerate() {
                let memory_ids = c.memory_ids();
                let summary = derive_cluster_summary(&c);
                let child = MocNode {
                    id: format!("moc-{}-{}", i, j),
                    title: if c.suggested_title.is_empty() {
                        derive_fallback_title(&c)
                    } else {
                        c.suggested_title
                    },
                    children: Vec::new(),
                    memory_ids,
                    level: 2,
                    summary,
                };
                root.children.push(child);
                tree.total_nodes += 1;
            }
            root.summary = format!("包含 {} 个子主题", root.children.len());
            tree.total_nodes += 1;
            tree.root_nodes.push(root);
        }

        tree.max_depth = if tree.root_nodes.is_empty() { 0 } else { 2 };
        tree
    }

    /// 合并小于 `min_size` 的聚类。
    ///
    /// 大簇保留原样；小簇的记忆项合并到一个 `misc` 聚类中
    /// （若仅有一个小簇，沿用其 `centroid_keyword`）。
    /// `min_size == 0` 时直接返回原聚类列表（不合并）。
    pub fn merge_small_clusters(&self, clusters: Vec<Cluster>, min_size: usize) -> Vec<Cluster> {
        if min_size == 0 {
            return clusters;
        }
        let mut big: Vec<Cluster> = Vec::new();
        let mut small_items: Vec<MemoItem> = Vec::new();
        let mut small_keywords: Vec<String> = Vec::new();

        for c in clusters {
            if c.len() >= min_size {
                big.push(c);
            } else {
                small_items.extend(c.items);
                small_keywords.push(c.centroid_keyword);
            }
        }

        if !small_items.is_empty() {
            let keyword = if small_keywords.len() == 1 {
                small_keywords.remove(0)
            } else {
                "misc".to_string()
            };
            big.push(Cluster::new(small_items, keyword));
        }

        big.sort_by(|a, b| a.centroid_keyword.cmp(&b.centroid_keyword));
        big
    }
}

// ---------------------------------------------------------------------------
// 导出函数
// ---------------------------------------------------------------------------

/// 将 MOC 树导出为指定格式字符串。
pub fn export_moc(tree: &MocTree, format: MocExportFormat) -> String {
    match format {
        MocExportFormat::Json => serde_json::to_string_pretty(tree)
            .unwrap_or_else(|e| format!("{{\"error\": \"serialize failed: {e}\"}}")),
        MocExportFormat::Markdown => export_markdown(tree),
        MocExportFormat::Obsidian => export_obsidian(tree),
    }
}

fn export_markdown(tree: &MocTree) -> String {
    let mut out = String::new();
    out.push_str("# Map of Content\n\n");
    if tree.root_nodes.is_empty() {
        out.push_str("_(empty)_\n");
        return out;
    }
    out.push_str(&format!(
        "> 共 {} 个节点，最大深度 {}\n\n",
        tree.total_nodes, tree.max_depth
    ));
    for root in &tree.root_nodes {
        render_markdown_node(root, &mut out, 0);
    }
    out
}

fn render_markdown_node(node: &MocNode, out: &mut String, depth: usize) {
    // Markdown 标题最多 6 级
    let level = (depth + 2).min(6);
    let prefix = "#".repeat(level);
    out.push_str(&format!("{} {}\n\n", prefix, node.title));
    if !node.summary.is_empty() {
        out.push_str(&format!("{}\n\n", node.summary));
    }
    if !node.memory_ids.is_empty() {
        out.push_str(&format!("- 记忆数：{}\n", node.memory_ids.len()));
        for mid in &node.memory_ids {
            out.push_str(&format!("  - `[[{}]]`\n", mid));
        }
        out.push('\n');
    }
    for child in &node.children {
        render_markdown_node(child, out, depth + 1);
    }
}

fn export_obsidian(tree: &MocTree) -> String {
    let mut out = String::new();
    out.push_str("---\ntype: moc\ntags: [moc, auto-generated]\n---\n\n");
    out.push_str("# 🗺️ Map of Content\n\n");
    if tree.root_nodes.is_empty() {
        out.push_str("_(empty)_\n");
        return out;
    }
    for root in &tree.root_nodes {
        render_obsidian_node(root, &mut out, 0);
    }
    out
}

fn render_obsidian_node(node: &MocNode, out: &mut String, depth: usize) {
    let indent = "  ".repeat(depth);
    out.push_str(&format!("{}- **{}**\n", indent, node.title));
    if !node.summary.is_empty() {
        out.push_str(&format!("{}  {}\n", indent, node.summary));
    }
    for mid in &node.memory_ids {
        out.push_str(&format!("{}  - [[{}]]\n", indent, mid));
    }
    for child in &node.children {
        render_obsidian_node(child, out, depth + 1);
    }
}

// ---------------------------------------------------------------------------
// 内部辅助函数
// ---------------------------------------------------------------------------

/// 构造 LLM 标题生成 prompt。
fn build_title_prompt(cluster: &Cluster) -> String {
    let samples: Vec<&str> = cluster
        .items
        .iter()
        .take(5)
        .map(|m| m.content.as_str())
        .collect();
    let sample_text = samples.join("\n- ");
    format!(
        "请为以下记忆片段生成一个简洁的中文主题标题（不超过 10 个字，不要标点，不要引号）：\n- {}\n\n主题：",
        sample_text
    )
}

/// 回退标题：优先使用 `centroid_keyword`，否则 "未命名主题"。
fn derive_fallback_title(cluster: &Cluster) -> String {
    if cluster.items.is_empty() {
        return "空主题".to_string();
    }
    if !cluster.centroid_keyword.is_empty() {
        return cluster.centroid_keyword.clone();
    }
    "未命名主题".to_string()
}

/// 聚类摘要：记忆数 + 首条记忆前 60 字符预览。
fn derive_cluster_summary(cluster: &Cluster) -> String {
    if cluster.items.is_empty() {
        return String::new();
    }
    let n = cluster.items.len();
    let preview: String = cluster.items[0].content.chars().take(60).collect();
    format!("共 {} 条记忆；示例：{}", n, preview)
}

/// 按 `centroid_keyword` 首字符分组。
///
/// 返回 `Vec<(group_title, Vec<Cluster>)>`，按 `BTreeMap` 顺序排列
/// （确定性）。单元素分组以 `centroid_keyword` 自身作为顶层标题。
fn group_clusters_by_prefix(clusters: Vec<Cluster>) -> Vec<(String, Vec<Cluster>)> {
    let mut groups: BTreeMap<String, Vec<Cluster>> = BTreeMap::new();
    for c in clusters {
        let prefix = c
            .centroid_keyword
            .chars()
            .next()
            .map(|ch| ch.to_string())
            .unwrap_or_else(|| "其他".to_string());
        groups.entry(prefix).or_default().push(c);
    }
    groups
        .into_iter()
        .map(|(k, v)| {
            if v.len() == 1 {
                let real_title = v[0].centroid_keyword.clone();
                (real_title, v)
            } else {
                (format!("{} 类", k), v)
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 简单 stub LLM：总是返回固定字符串。
    struct StubLlm {
        response: String,
    }

    #[async_trait]
    impl LlmClient for StubLlm {
        async fn generate(&self, _prompt: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    /// 记录调用次数的 stub LLM。
    struct CountingLlm {
        response: String,
        count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl LlmClient for CountingLlm {
        async fn generate(&self, _prompt: &str) -> Result<String> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    fn make_item(id: &str, content: &str, tags: &[&str]) -> MemoItem {
        MemoItem {
            id: id.to_string(),
            content: content.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            timestamp: 0,
            embedding_id: None,
        }
    }

    // 1. cluster_items 按首个 tag 聚类
    #[test]
    fn cluster_items_groups_by_first_tag() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let items = vec![
            make_item("1", "Tauri 端口问题", &["tauri", "bug"]),
            make_item("2", "Tauri 权限问题", &["tauri"]),
            make_item("3", "数据库超时", &["db"]),
        ];
        let clusters = gen.cluster_items(&items);
        assert_eq!(clusters.len(), 2);
        let tauri = clusters
            .iter()
            .find(|c| c.centroid_keyword == "tauri")
            .expect("tauri cluster should exist");
        assert_eq!(tauri.len(), 2);
        let db = clusters
            .iter()
            .find(|c| c.centroid_keyword == "db")
            .expect("db cluster should exist");
        assert_eq!(db.len(), 1);
    }

    // 2. 无 tag 的项归入 uncategorized
    #[test]
    fn cluster_items_uncategorized_for_no_tag_items() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let items = vec![
            make_item("1", "内容 A", &[]),
            make_item("2", "内容 B", &["x"]),
        ];
        let clusters = gen.cluster_items(&items);
        let uncat = clusters
            .iter()
            .find(|c| c.centroid_keyword == "uncategorized")
            .expect("uncategorized cluster should exist");
        assert_eq!(uncat.len(), 1);
        assert_eq!(uncat.items[0].id, "1");
    }

    // 3. cluster_items 输出按 centroid_keyword 排序（确定性）
    #[test]
    fn cluster_items_output_is_sorted() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let items = vec![
            make_item("1", "z", &["zebra"]),
            make_item("2", "a", &["apple"]),
            make_item("3", "m", &["mango"]),
        ];
        let clusters = gen.cluster_items(&items);
        let keywords: Vec<&str> = clusters
            .iter()
            .map(|c| c.centroid_keyword.as_str())
            .collect();
        assert_eq!(keywords, vec!["apple", "mango", "zebra"]);
    }

    // 4. cluster_items 空输入返回空
    #[test]
    fn cluster_items_empty_input_returns_empty() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let clusters = gen.cluster_items(&[]);
        assert!(clusters.is_empty());
    }

    // 5. merge_small_clusters 合并小于阈值的簇
    #[test]
    fn merge_small_clusters_merges_small_ones() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let clusters = vec![
            Cluster::new(vec![make_item("1", "a", &["x"])], "x".to_string()),
            Cluster::new(
                vec![make_item("2", "b", &["y"]), make_item("3", "c", &["y"])],
                "y".to_string(),
            ),
            Cluster::new(vec![make_item("4", "d", &["z"])], "z".to_string()),
        ];
        let merged = gen.merge_small_clusters(clusters, 2);
        // 大簇 y 保留；小簇 x、z 合并为 misc
        assert_eq!(merged.len(), 2);
        let misc = merged
            .iter()
            .find(|c| c.centroid_keyword == "misc")
            .expect("misc cluster should exist");
        assert_eq!(misc.len(), 2);
        let big = merged
            .iter()
            .find(|c| c.centroid_keyword == "y")
            .expect("y cluster should exist");
        assert_eq!(big.len(), 2);
    }

    // 6. merge_small_clusters 单个小簇保留原 centroid_keyword
    #[test]
    fn merge_small_clusters_single_small_keeps_keyword() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let clusters = vec![
            Cluster::new(
                vec![make_item("1", "a", &["big"]), make_item("2", "b", &["big"])],
                "big".to_string(),
            ),
            Cluster::new(vec![make_item("3", "c", &["small"])], "small".to_string()),
        ];
        let merged = gen.merge_small_clusters(clusters, 2);
        assert_eq!(merged.len(), 2);
        assert!(
            merged.iter().any(|c| c.centroid_keyword == "small"),
            "single small cluster should keep its keyword"
        );
    }

    // 7. merge_small_clusters min_size=0 不合并
    #[test]
    fn merge_small_clusters_zero_min_size_no_merge() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let clusters = vec![
            Cluster::new(vec![make_item("1", "a", &["x"])], "x".to_string()),
            Cluster::new(vec![make_item("2", "b", &["y"])], "y".to_string()),
        ];
        let merged = gen.merge_small_clusters(clusters, 0);
        assert_eq!(merged.len(), 2);
    }

    // 8. build_hierarchy 单层（max_depth=1）
    #[test]
    fn build_hierarchy_flat_when_max_depth_one() {
        let cfg = MocGeneratorConfig {
            max_depth: 1,
            ..Default::default()
        };
        let gen = MocGenerator::new(cfg);
        let clusters = vec![
            Cluster::new(vec![make_item("1", "a", &["x"])], "x".to_string()),
            Cluster::new(vec![make_item("2", "b", &["y"])], "y".to_string()),
        ];
        let tree = gen.build_hierarchy(clusters);
        assert_eq!(tree.root_nodes.len(), 2);
        assert_eq!(tree.max_depth, 1);
        assert_eq!(tree.total_nodes, 2);
        assert!(tree.root_nodes.iter().all(|n| n.children.is_empty()));
    }

    // 9. build_hierarchy 多层（max_depth>=2，多聚类）
    #[test]
    fn build_hierarchy_two_level_when_multiple_clusters() {
        let cfg = MocGeneratorConfig {
            max_depth: 3,
            ..Default::default()
        };
        let gen = MocGenerator::new(cfg);
        let clusters = vec![
            Cluster::new(vec![make_item("1", "a", &["apple"])], "apple".to_string()),
            Cluster::new(
                vec![make_item("2", "b", &["apricot"])],
                "apricot".to_string(),
            ),
            Cluster::new(vec![make_item("3", "c", &["banana"])], "banana".to_string()),
        ];
        let tree = gen.build_hierarchy(clusters);
        assert_eq!(tree.max_depth, 2);
        // 3 个聚类按首字母分组：a 类（2 个子）+ b 类（1 个子，单元素分组用自身名）
        // total_nodes = root 数 + 所有叶子数
        assert!(tree.total_nodes >= 4);
        // 至少有一个 root 含 2 个 children（a 类）
        assert!(
            tree.root_nodes.iter().any(|n| n.children.len() == 2),
            "expected a group with 2 children"
        );
    }

    // 10. build_hierarchy 单聚类生成单层树
    #[test]
    fn build_hierarchy_single_cluster_flat() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let clusters = vec![Cluster::new(
            vec![make_item("1", "a", &["x"]), make_item("2", "b", &["x"])],
            "x".to_string(),
        )];
        let tree = gen.build_hierarchy(clusters);
        assert_eq!(tree.root_nodes.len(), 1);
        assert_eq!(tree.max_depth, 1);
        assert_eq!(tree.root_nodes[0].memory_ids.len(), 2);
    }

    // 11. build_hierarchy 空聚类输入返回空树
    #[test]
    fn build_hierarchy_empty_input_returns_empty_tree() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let tree = gen.build_hierarchy(vec![]);
        assert!(tree.root_nodes.is_empty());
        assert_eq!(tree.total_nodes, 0);
        assert_eq!(tree.max_depth, 0);
    }

    // 12. generate 空输入返回空树
    #[tokio::test]
    async fn generate_empty_input_returns_empty_tree() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let tree = gen.generate(&[]).await.expect("generate should succeed");
        assert!(tree.root_nodes.is_empty());
        assert_eq!(tree.total_nodes, 0);
    }

    // 13. generate 无 LLM 时使用回退标题
    #[tokio::test]
    async fn generate_without_llm_uses_fallback_title() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let items = vec![
            make_item("1", "Tauri 端口", &["tauri"]),
            make_item("2", "Tauri 权限", &["tauri"]),
            make_item("3", "DB 超时", &["db"]),
            make_item("4", "DB 死锁", &["db"]),
        ];
        let tree = gen.generate(&items).await.expect("generate should succeed");
        assert!(!tree.root_nodes.is_empty());
        // 回退标题应等于 centroid_keyword
        assert!(
            tree.root_nodes
                .iter()
                .any(|n| n.title == "tauri" || n.title == "db" || n.title.contains("tauri")),
            "expected fallback title from centroid_keyword"
        );
    }

    // 14. generate 有 LLM 时调用 LLM 生成标题
    #[tokio::test]
    async fn generate_with_llm_calls_llm_for_title() {
        let llm = Arc::new(CountingLlm {
            response: "LLM标题".to_string(),
            count: std::sync::atomic::AtomicUsize::new(0),
        });
        let gen = MocGenerator::new(MocGeneratorConfig::default()).with_llm(llm.clone());
        let items = vec![
            make_item("1", "Tauri 端口", &["tauri"]),
            make_item("2", "Tauri 权限", &["tauri"]),
            make_item("3", "DB 超时", &["db"]),
            make_item("4", "DB 死锁", &["db"]),
        ];
        let tree = gen.generate(&items).await.expect("generate should succeed");
        let calls = llm.count.load(std::sync::atomic::Ordering::SeqCst);
        assert!(calls >= 1, "LLM should be called at least once");
        assert!(
            tree.root_nodes.iter().any(|n| n.title == "LLM标题"),
            "expected LLM-generated title"
        );
    }

    // 15. generate_title 无 LLM 时返回错误
    #[tokio::test]
    async fn generate_title_without_llm_returns_error() {
        let gen = MocGenerator::new(MocGeneratorConfig::default());
        let cluster = Cluster::new(vec![make_item("1", "x", &["x"])], "x".to_string());
        let result = gen.generate_title(&cluster).await;
        assert!(result.is_err());
    }

    // 16. generate_title 空聚类返回错误
    #[tokio::test]
    async fn generate_title_empty_cluster_returns_error() {
        let llm = Arc::new(StubLlm {
            response: "title".to_string(),
        });
        let gen = MocGenerator::new(MocGeneratorConfig::default()).with_llm(llm);
        let cluster = Cluster::new(vec![], String::new());
        let result = gen.generate_title(&cluster).await;
        assert!(result.is_err());
    }

    // 17. generate_title 去除引号与书名号
    #[tokio::test]
    async fn generate_title_strips_quotes_and_brackets() {
        let llm = Arc::new(StubLlm {
            response: "「Tauri 排错」".to_string(),
        });
        let gen = MocGenerator::new(MocGeneratorConfig::default()).with_llm(llm);
        let cluster = Cluster::new(vec![make_item("1", "x", &["x"])], "x".to_string());
        let title = gen
            .generate_title(&cluster)
            .await
            .expect("title should succeed");
        assert_eq!(title, "Tauri 排错");
    }

    // 18. generate_title LLM 返回空白时报错
    #[tokio::test]
    async fn generate_title_empty_llm_response_returns_error() {
        let llm = Arc::new(StubLlm {
            response: "   ".to_string(),
        });
        let gen = MocGenerator::new(MocGeneratorConfig::default()).with_llm(llm);
        let cluster = Cluster::new(vec![make_item("1", "x", &["x"])], "x".to_string());
        let result = gen.generate_title(&cluster).await;
        assert!(result.is_err());
    }

    // 19. export_moc Markdown 格式包含标题与记忆 id
    #[test]
    fn export_moc_markdown_contains_title_and_ids() {
        let tree = MocTree {
            root_nodes: vec![MocNode {
                id: "moc-0".to_string(),
                title: "Tauri".to_string(),
                children: Vec::new(),
                memory_ids: vec!["m1".to_string(), "m2".to_string()],
                level: 1,
                summary: "共 2 条记忆".to_string(),
            }],
            total_nodes: 1,
            max_depth: 1,
        };
        let md = export_moc(&tree, MocExportFormat::Markdown);
        assert!(md.contains("# Map of Content"));
        assert!(md.contains("Tauri"));
        assert!(md.contains("[[m1]]"));
        assert!(md.contains("[[m2]]"));
        assert!(md.contains("共 1 个节点"));
    }

    // 20. export_moc JSON 格式可被反序列化
    #[test]
    fn export_moc_json_roundtrip() {
        let tree = MocTree {
            root_nodes: vec![MocNode {
                id: "moc-0".to_string(),
                title: "Test".to_string(),
                children: Vec::new(),
                memory_ids: vec!["m1".to_string()],
                level: 1,
                summary: "summary".to_string(),
            }],
            total_nodes: 1,
            max_depth: 1,
        };
        let json = export_moc(&tree, MocExportFormat::Json);
        let parsed: MocTree = serde_json::from_str(&json).expect("JSON should round-trip");
        assert_eq!(parsed.total_nodes, 1);
        assert_eq!(parsed.root_nodes[0].title, "Test");
        assert_eq!(parsed.root_nodes[0].memory_ids, vec!["m1".to_string()]);
    }

    // 21. export_moc Obsidian 格式包含 frontmatter 与 wikilink
    #[test]
    fn export_moc_obsidian_contains_frontmatter_and_wikilink() {
        let tree = MocTree {
            root_nodes: vec![MocNode {
                id: "moc-0".to_string(),
                title: "Tauri".to_string(),
                children: Vec::new(),
                memory_ids: vec!["m1".to_string()],
                level: 1,
                summary: "summary".to_string(),
            }],
            total_nodes: 1,
            max_depth: 1,
        };
        let obs = export_moc(&tree, MocExportFormat::Obsidian);
        assert!(obs.starts_with("---\ntype: moc"));
        assert!(obs.contains("tags: [moc, auto-generated]"));
        assert!(obs.contains("# 🗺️ Map of Content"));
        assert!(obs.contains("[[m1]]"));
    }

    // 22. export_moc 空树三种格式均不 panic
    #[test]
    fn export_moc_empty_tree_all_formats_no_panic() {
        let tree = MocTree::default();
        for fmt in [
            MocExportFormat::Markdown,
            MocExportFormat::Json,
            MocExportFormat::Obsidian,
        ] {
            let s = export_moc(&tree, fmt);
            assert!(!s.is_empty(), "export should produce non-empty output");
        }
    }

    // 23. MemoItem::new 构造正确
    #[test]
    fn memo_item_new_constructs_correctly() {
        let item = MemoItem::new("id1", "content", vec!["tag1".to_string()]);
        assert_eq!(item.id, "id1");
        assert_eq!(item.content, "content");
        assert_eq!(item.tags, vec!["tag1".to_string()]);
        assert_eq!(item.timestamp, 0);
        assert!(item.embedding_id.is_none());
    }

    // 24. Cluster::memory_ids 返回所有项 id
    #[test]
    fn cluster_memory_ids_returns_all() {
        let cluster = Cluster::new(
            vec![
                make_item("a", "x", &[]),
                make_item("b", "y", &[]),
                make_item("c", "z", &[]),
            ],
            "kw".to_string(),
        );
        assert_eq!(cluster.memory_ids(), vec!["a", "b", "c"]);
    }

    // 25. MocTree::default 字段均为空/零
    #[test]
    fn moc_tree_default_is_empty() {
        let tree = MocTree::default();
        assert!(tree.root_nodes.is_empty());
        assert_eq!(tree.total_nodes, 0);
        assert_eq!(tree.max_depth, 0);
    }

    // 26. MocGeneratorConfig::default 字段合理
    #[test]
    fn moc_generator_config_default_values() {
        let cfg = MocGeneratorConfig::default();
        assert!(cfg.max_depth >= 1);
        assert!(cfg.min_cluster_size >= 1);
        assert!(!cfg.llm_model.is_empty());
    }

    // 27. with_llm 启用 LLM 路径
    #[tokio::test]
    async fn with_llm_enables_llm_path() {
        let llm: Arc<dyn LlmClient> = Arc::new(StubLlm {
            response: "标题".to_string(),
        });
        let gen = MocGenerator::new(MocGeneratorConfig::default()).with_llm(llm);
        let cluster = Cluster::new(vec![make_item("1", "x", &["x"])], "x".to_string());
        let title = gen
            .generate_title(&cluster)
            .await
            .expect("LLM title should succeed");
        assert_eq!(title, "标题");
    }

    // 28. config() 返回配置引用
    #[test]
    fn config_accessor_returns_reference() {
        let cfg = MocGeneratorConfig {
            max_depth: 5,
            min_cluster_size: 3,
            llm_model: "test-model".to_string(),
        };
        let gen = MocGenerator::new(cfg);
        assert_eq!(gen.config().max_depth, 5);
        assert_eq!(gen.config().min_cluster_size, 3);
        assert_eq!(gen.config().llm_model, "test-model");
    }
}
