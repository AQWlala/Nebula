//! T-E-S-12: 蜂群运行时画布节点交互层 — 支持拖拽、选中、连线、右键菜单等交互。
//!
//! 本模块提供画布节点交互的状态管理与事件处理，与 [`super::runtime_canvas`]
//! 模块配合使用：[`runtime_canvas`] 负责数据收集与快照广播，本模块负责
//! 用户交互语义的处理与布局管理。
//!
//! ## 核心组件
//!
//! * [`InteractionState`] — 交互状态机，处理 [`NodeInteraction`] 事件并
//!   返回 [`InteractionResult`]（含状态变化标志与副作用）。
//! * [`NodeLayout`] — 节点布局管理（位置存储、连接管理、自动布局算法）。
//! * [`NodeSelectionModel`] — 节点选择模型（单选/多选模式）。
//! * [`hit_test`] — 命中测试（判断点击位置命中的是节点、连接还是空白）。
//!
//! ## 交互流程
//!
//! ```text
//! 用户操作 ──▶ NodeInteraction
//!                │
//!                ▼
//!         InteractionState.handle_interaction()
//!                │
//!                ▼
//!         InteractionResult
//!           ├── state_changed   → 是否需要同步状态
//!           ├── requires_redraw → 是否需要重绘画布
//!           └── side_effect     → 连接添加/移除、节点移动、菜单开关
//! ```

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// 辅助默认值函数
// ---------------------------------------------------------------------------

/// `bool` 的默认值（`true`）— 供 `#[serde(default = "...")]` 使用。
fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

/// 二维坐标位置 — 画布上节点的坐标。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    /// X 坐标。
    pub x: f64,
    /// Y 坐标。
    pub y: f64,
}

impl Position {
    /// 创建一个新的 `Position`。
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// 计算与另一个位置之间的欧氏距离。
    pub fn distance_to(&self, other: &Position) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

impl Default for Position {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

// ---------------------------------------------------------------------------
// NodeInteraction 枚举
// ---------------------------------------------------------------------------

/// 节点交互事件 — 由前端画布组件产生，经 Tauri IPC 传递到后端处理。
///
/// 所有变体使用 `snake_case` 序列化，与前端 TypeScript 类型对齐。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeInteraction {
    /// 选中节点。
    Select { node_id: String },
    /// 取消选中节点。
    Deselect { node_id: String },
    /// 开始拖拽节点（记录拖拽起始位置）。
    DragStart { node_id: String, position: Position },
    /// 拖拽移动节点（更新节点位置）。
    DragMove {
        node_id: String,
        new_position: Position,
    },
    /// 结束拖拽节点（确认最终位置）。
    DragEnd {
        node_id: String,
        final_position: Position,
    },
    /// 连接两个节点（从 `from_node_id` 到 `to_node_id`）。
    Connect {
        from_node_id: String,
        to_node_id: String,
        label: Option<String>,
    },
    /// 断开两个节点之间的连接。
    Disconnect {
        from_node_id: String,
        to_node_id: String,
    },
    /// 右键点击节点（弹出上下文菜单）。
    RightClick { node_id: String, position: Position },
    /// 双击节点。
    DoubleClick { node_id: String },
    /// 鼠标悬停在节点上。
    Hover { node_id: String },
    /// 鼠标离开节点。
    Unhover { node_id: String },
}

impl NodeInteraction {
    /// 从 JSON 字符串解析 [`NodeInteraction`]。
    ///
    /// 用于从 Tauri IPC 前端传来的 JSON 消息中解析交互事件。
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| anyhow!("NodeInteraction 解析失败: {e}"))
    }
}

// ---------------------------------------------------------------------------
// InteractionSideEffect / InteractionResult
// ---------------------------------------------------------------------------

/// 交互副作用 — 需要外部系统（如 DAG、画布渲染器）响应的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionSideEffect {
    /// 连接已添加（需要更新 DAG / 布局）。
    ConnectionAdded,
    /// 连接已移除（需要更新 DAG / 布局）。
    ConnectionRemoved,
    /// 节点已移动（需要持久化位置）。
    NodeMoved,
    /// 右键菜单已打开。
    ContextMenuOpened,
    /// 右键菜单已关闭。
    ContextMenuClosed,
}

/// 交互处理结果 — 由 [`InteractionState::handle_interaction`] 返回。
///
/// 前端根据此结果决定是否同步状态、重绘画布以及执行副作用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionResult {
    /// 交互状态是否发生变化（用于判断是否需要同步状态到前端）。
    pub state_changed: bool,
    /// 是否需要重绘画布（用于判断是否需要触发渲染更新）。
    pub requires_redraw: bool,
    /// 交互副作用（如有）。
    #[serde(default)]
    pub side_effect: Option<InteractionSideEffect>,
}

impl InteractionResult {
    /// 创建一个新的 `InteractionResult`。
    pub fn new(
        state_changed: bool,
        requires_redraw: bool,
        side_effect: Option<InteractionSideEffect>,
    ) -> Self {
        Self {
            state_changed,
            requires_redraw,
            side_effect,
        }
    }

    /// 无状态变化的空结果。
    pub fn empty() -> Self {
        Self {
            state_changed: false,
            requires_redraw: false,
            side_effect: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ContextMenuItem / ContextMenu
// ---------------------------------------------------------------------------

/// 右键菜单项 — 菜单中的单个可点击项或分隔符。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextMenuItem {
    /// 菜单项标签（分隔符时为空）。
    pub label: String,
    /// 菜单项动作标识（前端据此执行对应操作）。
    pub action: String,
    /// 是否启用（禁用项显示为灰色）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 是否为分隔符（分隔符不显示文本，仅渲染分隔线）。
    #[serde(default)]
    pub separator: bool,
}

impl ContextMenuItem {
    /// 创建一个新的菜单项（默认启用，非分隔符）。
    pub fn new(label: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            action: action.into(),
            enabled: true,
            separator: false,
        }
    }

    /// 创建一个分隔符项。
    pub fn separator() -> Self {
        Self {
            label: String::new(),
            action: String::new(),
            enabled: false,
            separator: true,
        }
    }

    /// Builder：设置启用状态。
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// 右键菜单 — 右键点击节点时弹出的上下文菜单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextMenu {
    /// 关联的节点 ID。
    pub node_id: String,
    /// 菜单弹出位置（画布坐标）。
    pub position: Position,
    /// 菜单项列表。
    pub items: Vec<ContextMenuItem>,
}

impl ContextMenu {
    /// 创建一个新的右键菜单（含默认菜单项）。
    pub fn new(node_id: impl Into<String>, position: Position) -> Self {
        Self {
            node_id: node_id.into(),
            position,
            items: default_context_menu_items(),
        }
    }

    /// 创建一个无菜单项的空右键菜单（由调用方自行填充）。
    pub fn empty(node_id: impl Into<String>, position: Position) -> Self {
        Self {
            node_id: node_id.into(),
            position,
            items: Vec::new(),
        }
    }

    /// Builder：设置菜单项列表。
    pub fn with_items(mut self, items: Vec<ContextMenuItem>) -> Self {
        self.items = items;
        self
    }
}

/// 生成默认右键菜单项 — 编辑、复制、分隔符、删除、分隔符、属性。
fn default_context_menu_items() -> Vec<ContextMenuItem> {
    vec![
        ContextMenuItem::new("编辑", "edit"),
        ContextMenuItem::new("复制", "copy"),
        ContextMenuItem::separator(),
        ContextMenuItem::new("删除", "delete"),
        ContextMenuItem::separator(),
        ContextMenuItem::new("属性", "properties"),
    ]
}

// ---------------------------------------------------------------------------
// InteractionState
// ---------------------------------------------------------------------------

/// 交互状态 — 跟踪当前选中、悬停、拖拽、连接和右键菜单状态。
///
/// 由 [`handle_interaction`](Self::handle_interaction) 方法驱动状态转换，
/// 每次交互返回 [`InteractionResult`] 描述状态变化与副作用。
#[derive(Debug, Clone)]
pub struct InteractionState {
    /// 当前选中的节点 ID 列表（支持多选）。
    pub selected_nodes: Vec<String>,
    /// 当前悬停的节点 ID。
    pub hovered_node: Option<String>,
    /// 当前正在拖拽的节点 ID。
    pub dragging_node: Option<String>,
    /// 拖拽起始位置（用于计算偏移量）。
    pub drag_offset: Option<Position>,
    /// 连接操作的起始节点 ID（正在拖拽连线时）。
    pub connecting_from: Option<String>,
    /// 当前打开的右键菜单（None 表示无菜单打开）。
    pub context_menu: Option<ContextMenu>,
}

impl InteractionState {
    /// 创建一个新的空交互状态。
    pub fn new() -> Self {
        Self {
            selected_nodes: Vec::new(),
            hovered_node: None,
            dragging_node: None,
            drag_offset: None,
            connecting_from: None,
            context_menu: None,
        }
    }

    /// 处理一个交互事件，返回交互结果。
    ///
    /// 根据交互类型更新内部状态，并返回 [`InteractionResult`]：
    /// - `state_changed`：本次交互是否改变了内部状态。
    /// - `requires_redraw`：前端是否需要重绘画布。
    /// - `side_effect`：需要外部系统响应的副作用（如连接添加/移除）。
    #[instrument(target = "nebula.swarm.canvas_interaction", skip(self, interaction))]
    pub fn handle_interaction(&mut self, interaction: &NodeInteraction) -> InteractionResult {
        match interaction {
            NodeInteraction::Select { node_id } => {
                // 关闭已打开的右键菜单。
                let side_effect = self.close_context_menu();
                // 添加到选中列表（如未选中）。
                if !self.selected_nodes.iter().any(|s| s == node_id) {
                    self.selected_nodes.push(node_id.clone());
                }
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    selected = self.selected_nodes.len(),
                    "node selected"
                );
                InteractionResult::new(true, true, side_effect)
            }

            NodeInteraction::Deselect { node_id } => {
                let side_effect = self.close_context_menu();
                let before = self.selected_nodes.len();
                self.selected_nodes.retain(|s| s != node_id);
                let changed = self.selected_nodes.len() != before;
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    changed,
                    "node deselected"
                );
                InteractionResult::new(true, true, side_effect)
            }

            NodeInteraction::DragStart { node_id, position } => {
                let side_effect = self.close_context_menu();
                self.dragging_node = Some(node_id.clone());
                self.drag_offset = Some(*position);
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    "drag started"
                );
                InteractionResult::new(true, true, side_effect)
            }

            NodeInteraction::DragMove {
                node_id,
                new_position,
            } => {
                // 拖拽移动：更新偏移量（前端据此更新节点位置）。
                self.drag_offset = Some(*new_position);
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    "drag move"
                );
                InteractionResult::new(true, true, Some(InteractionSideEffect::NodeMoved))
            }

            NodeInteraction::DragEnd {
                node_id,
                final_position,
            } => {
                // 拖拽结束：清除拖拽状态，保留最终位置。
                self.dragging_node = None;
                self.drag_offset = Some(*final_position);
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    "drag ended"
                );
                InteractionResult::new(true, true, Some(InteractionSideEffect::NodeMoved))
            }

            NodeInteraction::Connect {
                from_node_id,
                to_node_id,
                ..
            } => {
                // 连接完成：清除连接起始状态。
                self.connecting_from = None;
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    from = %from_node_id,
                    to = %to_node_id,
                    "connection added"
                );
                InteractionResult::new(true, true, Some(InteractionSideEffect::ConnectionAdded))
            }

            NodeInteraction::Disconnect {
                from_node_id,
                to_node_id,
            } => {
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    from = %from_node_id,
                    to = %to_node_id,
                    "connection removed"
                );
                InteractionResult::new(true, true, Some(InteractionSideEffect::ConnectionRemoved))
            }

            NodeInteraction::RightClick { node_id, position } => {
                // 打开右键菜单。
                self.context_menu = Some(ContextMenu::new(node_id.clone(), *position));
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    "context menu opened"
                );
                InteractionResult::new(true, true, Some(InteractionSideEffect::ContextMenuOpened))
            }

            NodeInteraction::DoubleClick { node_id } => {
                let side_effect = self.close_context_menu();
                // 双击同时选中节点。
                if !self.selected_nodes.iter().any(|s| s == node_id) {
                    self.selected_nodes.push(node_id.clone());
                }
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    "double click"
                );
                InteractionResult::new(true, true, side_effect)
            }

            NodeInteraction::Hover { node_id } => {
                let changed = self.hovered_node.as_deref() != Some(node_id.as_str());
                self.hovered_node = Some(node_id.clone());
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    changed,
                    "hover"
                );
                InteractionResult::new(changed, changed, None)
            }

            NodeInteraction::Unhover { node_id } => {
                let changed = self.hovered_node.as_deref() == Some(node_id.as_str());
                if changed {
                    self.hovered_node = None;
                }
                debug!(
                    target: "nebula.swarm.canvas_interaction",
                    node_id = %node_id,
                    changed,
                    "unhover"
                );
                InteractionResult::new(changed, changed, None)
            }
        }
    }

    /// 关闭右键菜单（如已打开），返回对应的副作用。
    fn close_context_menu(&mut self) -> Option<InteractionSideEffect> {
        if self.context_menu.is_some() {
            self.context_menu = None;
            Some(InteractionSideEffect::ContextMenuClosed)
        } else {
            None
        }
    }

    /// 重置所有交互状态（清空选中、悬停、拖拽、连接、菜单）。
    pub fn reset(&mut self) {
        self.selected_nodes.clear();
        self.hovered_node = None;
        self.dragging_node = None;
        self.drag_offset = None;
        self.connecting_from = None;
        self.context_menu = None;
    }
}

impl Default for InteractionState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// LayoutConnection / LayoutAlgorithm
// ---------------------------------------------------------------------------

/// 布局连接 — 描述两个节点之间的有向连接（用于布局计算）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutConnection {
    /// 源节点 ID。
    pub from_node_id: String,
    /// 目标节点 ID。
    pub to_node_id: String,
    /// 连接标签（可选，画布上显示在连线中央）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl LayoutConnection {
    /// 创建一个新的 `LayoutConnection`（无标签）。
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from_node_id: from.into(),
            to_node_id: to.into(),
            label: None,
        }
    }

    /// Builder：设置连接标签。
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// 布局算法 — 用于 [`NodeLayout::layout_auto`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutAlgorithm {
    /// 力导向布局 — 节点间排斥 + 连接间吸引，自然分布。
    Force,
    /// 网格布局 — 节点排列为规则网格。
    Grid,
    /// 环形布局 — 节点均匀分布在圆周上。
    Circular,
    /// 层级布局 — 按依赖关系分层排列（拓扑排序）。
    Hierarchical,
}

// ---------------------------------------------------------------------------
// NodeLayout
// ---------------------------------------------------------------------------

/// 节点布局 — 管理画布上所有节点的位置和连接关系。
///
/// 提供位置存储、连接管理和自动布局算法（力导向/网格/环形/层级）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeLayout {
    /// 节点 ID → 位置映射。
    #[serde(default)]
    pub node_positions: HashMap<String, Position>,
    /// 布局中的连接列表。
    #[serde(default)]
    pub connections: Vec<LayoutConnection>,
}

impl NodeLayout {
    /// 创建一个空的节点布局。
    pub fn new() -> Self {
        Self {
            node_positions: HashMap::new(),
            connections: Vec::new(),
        }
    }

    /// 设置节点位置（如已存在则覆盖）。
    pub fn set_position(&mut self, node_id: &str, pos: Position) {
        self.node_positions.insert(node_id.to_string(), pos);
    }

    /// 获取节点位置。
    pub fn get_position(&self, node_id: &str) -> Option<&Position> {
        self.node_positions.get(node_id)
    }

    /// 添加连接（如已存在相同的 from→to 连接则忽略）。
    pub fn add_connection(&mut self, conn: LayoutConnection) {
        let exists = self
            .connections
            .iter()
            .any(|c| c.from_node_id == conn.from_node_id && c.to_node_id == conn.to_node_id);
        if !exists {
            self.connections.push(conn);
        }
    }

    /// 移除连接（匹配 from→to）。
    pub fn remove_connection(&mut self, from: &str, to: &str) {
        self.connections
            .retain(|c| !(c.from_node_id == from && c.to_node_id == to));
    }

    /// 自动布局 — 根据指定算法重新排列所有节点位置。
    ///
    /// 节点按 ID 排序后布局，保证同一组节点在不同运行中产生相同的布局结果
    /// （力导向算法除外，因其涉及浮点迭代，结果可能有微小差异）。
    #[instrument(target = "nebula.swarm.canvas_interaction", skip(self))]
    pub fn layout_auto(&mut self, algorithm: LayoutAlgorithm) {
        // 按 ID 排序保证确定性。
        let mut node_ids: Vec<String> = self.node_positions.keys().cloned().collect();
        node_ids.sort();
        if node_ids.is_empty() {
            return;
        }
        match algorithm {
            LayoutAlgorithm::Grid => self.layout_grid(&node_ids),
            LayoutAlgorithm::Circular => self.layout_circular(&node_ids),
            LayoutAlgorithm::Hierarchical => self.layout_hierarchical(&node_ids),
            LayoutAlgorithm::Force => self.layout_force(&node_ids),
        }
    }

    /// 网格布局 — 将节点排列为接近正方形的网格。
    fn layout_grid(&mut self, node_ids: &[String]) {
        let n = node_ids.len();
        let cols = ((n as f64).sqrt().ceil() as usize).max(1);
        let spacing = 200.0;
        for (i, id) in node_ids.iter().enumerate() {
            let row = i / cols;
            let col = i % cols;
            let pos = Position::new((col + 1) as f64 * spacing, (row + 1) as f64 * spacing);
            self.node_positions.insert(id.clone(), pos);
        }
    }

    /// 环形布局 — 将节点均匀分布在圆周上。
    fn layout_circular(&mut self, node_ids: &[String]) {
        let n = node_ids.len();
        let radius = 300.0;
        let center = Position::new(radius, radius);
        for (i, id) in node_ids.iter().enumerate() {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            let pos = Position::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            );
            self.node_positions.insert(id.clone(), pos);
        }
    }

    /// 层级布局 — 按依赖关系（连接）分层排列，同层节点水平展开。
    fn layout_hierarchical(&mut self, node_ids: &[String]) {
        // 构建入度表。
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for id in node_ids {
            in_degree.insert(id.clone(), 0);
        }
        for conn in &self.connections {
            if in_degree.contains_key(&conn.to_node_id) {
                *in_degree.get_mut(&conn.to_node_id).unwrap() += 1;
            }
        }

        // Kahn 分层算法：入度为 0 的节点归入当前层，移除后更新剩余入度。
        let mut layers: Vec<Vec<String>> = Vec::new();
        let mut assigned: std::collections::HashSet<String> = std::collections::HashSet::new();

        while assigned.len() < node_ids.len() {
            let current_layer: Vec<String> = node_ids
                .iter()
                .filter(|id| {
                    !assigned.contains(*id)
                        && self
                            .connections
                            .iter()
                            .filter(|c| c.to_node_id == **id && !assigned.contains(&c.from_node_id))
                            .count()
                            == 0
                })
                .cloned()
                .collect();

            if current_layer.is_empty() {
                // 防御性 break（可能存在环）。
                break;
            }

            for id in &current_layer {
                assigned.insert(id.clone());
            }
            layers.push(current_layer);
        }

        // 按层排列：第 0 层在最上方，y 递增。
        let layer_spacing = 200.0;
        let node_spacing = 200.0;
        for (layer_idx, layer) in layers.iter().enumerate() {
            let layer_len = layer.len();
            let total_width = layer_len.saturating_sub(1) as f64 * node_spacing;
            for (node_idx, id) in layer.iter().enumerate() {
                let x = (node_idx as f64 * node_spacing) - total_width / 2.0 + 500.0;
                let y = layer_idx as f64 * layer_spacing;
                self.node_positions.insert(id.clone(), Position::new(x, y));
            }
        }
    }

    /// 力导向布局 — 排斥力（所有节点对）+ 吸引力（连接节点对）迭代收敛。
    fn layout_force(&mut self, node_ids: &[String]) {
        let n = node_ids.len();

        // 初始化：先放在圆周上作为起点。
        let radius = 300.0;
        for (i, id) in node_ids.iter().enumerate() {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            let pos = Position::new(radius + radius * angle.cos(), radius + radius * angle.sin());
            self.node_positions.insert(id.clone(), pos);
        }

        // 力导向迭代参数。
        let iterations = 50;
        let ideal_distance = 150.0; // k — 理想距离。
        let repulsion = 5000.0; // 排斥力系数。
        let attraction = 0.1; // 吸引力系数。
        let max_step = 50.0; // 每次迭代最大位移（温度限制）。

        for _ in 0..iterations {
            // 计算每个节点的位移。
            let mut displacements: HashMap<String, Position> = HashMap::new();
            for id in node_ids {
                displacements.insert(id.clone(), Position::default());
            }

            // 排斥力 — 所有节点对之间的库仑式排斥。
            for i in 0..n {
                for j in (i + 1)..n {
                    let id_a = &node_ids[i];
                    let id_b = &node_ids[j];
                    let pos_a = self.node_positions.get(id_a).copied().unwrap_or_default();
                    let pos_b = self.node_positions.get(id_b).copied().unwrap_or_default();
                    let dx = pos_a.x - pos_b.x;
                    let dy = pos_a.y - pos_b.y;
                    let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                    let force = repulsion / (dist * dist);
                    let fx = (dx / dist) * force;
                    let fy = (dy / dist) * force;
                    if let Some(d) = displacements.get_mut(id_a) {
                        d.x += fx;
                        d.y += fy;
                    }
                    if let Some(d) = displacements.get_mut(id_b) {
                        d.x -= fx;
                        d.y -= fy;
                    }
                }
            }

            // 吸引力 — 连接节点对之间的弹簧式吸引。
            for conn in &self.connections {
                let pos_a = self
                    .node_positions
                    .get(&conn.from_node_id)
                    .copied()
                    .unwrap_or_default();
                let pos_b = self
                    .node_positions
                    .get(&conn.to_node_id)
                    .copied()
                    .unwrap_or_default();
                let dx = pos_b.x - pos_a.x;
                let dy = pos_b.y - pos_a.y;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = attraction * (dist - ideal_distance);
                let fx = (dx / dist) * force;
                let fy = (dy / dist) * force;
                if let Some(d) = displacements.get_mut(&conn.from_node_id) {
                    d.x += fx;
                    d.y += fy;
                }
                if let Some(d) = displacements.get_mut(&conn.to_node_id) {
                    d.x -= fx;
                    d.y -= fy;
                }
            }

            // 应用位移（带温度限制，防止节点飞出画布）。
            for id in node_ids {
                if let Some(d) = displacements.get(id) {
                    if let Some(pos) = self.node_positions.get_mut(id) {
                        let step = (d.x * d.x + d.y * d.y).sqrt().max(0.01);
                        let factor = (max_step / step).min(1.0);
                        pos.x += d.x * factor;
                        pos.y += d.y * factor;
                    }
                }
            }
        }
    }
}

impl Default for NodeLayout {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// NodeSelectionModel
// ---------------------------------------------------------------------------

/// 节点选择模型 — 支持单选和多选模式。
///
/// 单选模式下，`select` 会替换当前选中；多选模式下，`select` 追加到选中列表。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSelectionModel {
    /// 是否启用多选模式。
    #[serde(default)]
    pub multi_select: bool,
    /// 当前选中的节点 ID 列表。
    #[serde(default)]
    pub selected: Vec<String>,
}

impl NodeSelectionModel {
    /// 创建一个新的选择模型（默认单选模式）。
    pub fn new() -> Self {
        Self {
            multi_select: false,
            selected: Vec::new(),
        }
    }

    /// 创建一个多选模式的模型。
    pub fn multi() -> Self {
        Self {
            multi_select: true,
            selected: Vec::new(),
        }
    }

    /// 选中节点 — 单选模式替换当前选中，多选模式追加。
    pub fn select(&mut self, node_id: &str) {
        if self.multi_select {
            if !self.selected.iter().any(|s| s == node_id) {
                self.selected.push(node_id.to_string());
            }
        } else {
            self.selected.clear();
            self.selected.push(node_id.to_string());
        }
    }

    /// 取消选中节点。
    pub fn deselect(&mut self, node_id: &str) {
        self.selected.retain(|s| s != node_id);
    }

    /// 清空所有选中。
    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// 判断节点是否被选中。
    pub fn is_selected(&self, node_id: &str) -> bool {
        self.selected.iter().any(|s| s == node_id)
    }

    /// 全选 — 追加所有给定 ID 到选中列表（多选模式下使用）。
    pub fn select_all(&mut self, ids: &[String]) {
        for id in ids {
            if !self.selected.iter().any(|s| s == id) {
                self.selected.push(id.clone());
            }
        }
    }
}

impl Default for NodeSelectionModel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// HitTestResult / hit_test
// ---------------------------------------------------------------------------

/// 命中测试结果 — 描述点击位置命中的对象类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitTestResult {
    /// 命中节点（含节点 ID）。
    Node(String),
    /// 命中连接（含 from 和 to 节点 ID）。
    Connection(String, String),
    /// 命中空白区域。
    Empty,
}

/// 命中测试 — 判断点击位置命中的是节点、连接还是空白。
///
/// 检测顺序：先检测节点（优先级高），再检测连接，最后返回空。
///
/// # 参数
/// * `layout` — 节点布局（含节点位置和连接）。
/// * `point` — 点击位置（画布坐标）。
/// * `node_radius` — 节点命中半径（点击位置在此半径内视为命中节点）。
///
/// # 返回
/// * [`HitTestResult::Node`] — 点在某个节点的半径范围内。
/// * [`HitTestResult::Connection`] — 点靠近某条连接线段（阈值为 `node_radius * 0.3`，最小 5.0）。
/// * [`HitTestResult::Empty`] — 未命中任何对象。
pub fn hit_test(layout: &NodeLayout, point: Position, node_radius: f64) -> HitTestResult {
    // 优先检测节点命中。
    for (node_id, pos) in &layout.node_positions {
        if point.distance_to(pos) <= node_radius {
            return HitTestResult::Node(node_id.clone());
        }
    }

    // 检测连接命中 — 点到线段距离。
    let connection_threshold = (node_radius * 0.3).max(5.0);
    for conn in &layout.connections {
        if let (Some(from_pos), Some(to_pos)) = (
            layout.node_positions.get(&conn.from_node_id),
            layout.node_positions.get(&conn.to_node_id),
        ) {
            let dist = point_to_segment_distance(&point, from_pos, to_pos);
            if dist <= connection_threshold {
                return HitTestResult::Connection(
                    conn.from_node_id.clone(),
                    conn.to_node_id.clone(),
                );
            }
        }
    }

    HitTestResult::Empty
}

/// 计算点到线段的最短距离。
fn point_to_segment_distance(point: &Position, seg_start: &Position, seg_end: &Position) -> f64 {
    let dx = seg_end.x - seg_start.x;
    let dy = seg_end.y - seg_start.y;
    let length_sq = dx * dx + dy * dy;

    if length_sq < f64::EPSILON {
        // 线段退化为点。
        return point.distance_to(seg_start);
    }

    // 投影参数 t — 点在线段方向上的投影比例，clamp 到 [0, 1]。
    let t = ((point.x - seg_start.x) * dx + (point.y - seg_start.y) * dy) / length_sq;
    let t = t.clamp(0.0, 1.0);

    let projection = Position::new(seg_start.x + t * dx, seg_start.y + t * dy);
    point.distance_to(&projection)
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // InteractionState 交互处理测试
    // ===================================================================

    /// 测试 `Select` 交互 — 节点应被添加到选中列表。
    #[test]
    fn test_interaction_state_select() {
        let mut state = InteractionState::new();
        let result = state.handle_interaction(&NodeInteraction::Select {
            node_id: "node-1".to_string(),
        });
        assert!(result.state_changed);
        assert!(result.requires_redraw);
        assert!(state.selected_nodes.contains(&"node-1".to_string()));
    }

    /// 测试 `Deselect` 交互 — 节点应从选中列表移除。
    #[test]
    fn test_interaction_state_deselect() {
        let mut state = InteractionState::new();
        state.selected_nodes.push("node-1".to_string());
        let result = state.handle_interaction(&NodeInteraction::Deselect {
            node_id: "node-1".to_string(),
        });
        assert!(result.state_changed);
        assert!(result.requires_redraw);
        assert!(!state.selected_nodes.contains(&"node-1".to_string()));
    }

    /// 测试拖拽生命周期 — DragStart → DragMove → DragEnd。
    #[test]
    fn test_interaction_state_drag_lifecycle() {
        let mut state = InteractionState::new();

        // DragStart
        let result = state.handle_interaction(&NodeInteraction::DragStart {
            node_id: "node-1".to_string(),
            position: Position::new(100.0, 200.0),
        });
        assert!(result.state_changed);
        assert_eq!(state.dragging_node.as_deref(), Some("node-1"));
        assert_eq!(state.drag_offset, Some(Position::new(100.0, 200.0)));

        // DragMove
        let result = state.handle_interaction(&NodeInteraction::DragMove {
            node_id: "node-1".to_string(),
            new_position: Position::new(150.0, 250.0),
        });
        assert!(result.state_changed);
        assert_eq!(result.side_effect, Some(InteractionSideEffect::NodeMoved));
        assert_eq!(state.drag_offset, Some(Position::new(150.0, 250.0)));

        // DragEnd
        let result = state.handle_interaction(&NodeInteraction::DragEnd {
            node_id: "node-1".to_string(),
            final_position: Position::new(180.0, 280.0),
        });
        assert!(result.state_changed);
        assert_eq!(result.side_effect, Some(InteractionSideEffect::NodeMoved));
        assert!(state.dragging_node.is_none());
        assert_eq!(state.drag_offset, Some(Position::new(180.0, 280.0)));
    }

    /// 测试 `Connect` 交互 — 应返回 `ConnectionAdded` 副作用。
    #[test]
    fn test_interaction_state_connect() {
        let mut state = InteractionState::new();
        state.connecting_from = Some("node-1".to_string());
        let result = state.handle_interaction(&NodeInteraction::Connect {
            from_node_id: "node-1".to_string(),
            to_node_id: "node-2".to_string(),
            label: Some("delegate".to_string()),
        });
        assert!(result.state_changed);
        assert_eq!(
            result.side_effect,
            Some(InteractionSideEffect::ConnectionAdded)
        );
        // Connect 后 connecting_from 应被清除。
        assert!(state.connecting_from.is_none());
    }

    /// 测试 `Disconnect` 交互 — 应返回 `ConnectionRemoved` 副作用。
    #[test]
    fn test_interaction_state_disconnect() {
        let mut state = InteractionState::new();
        let result = state.handle_interaction(&NodeInteraction::Disconnect {
            from_node_id: "node-1".to_string(),
            to_node_id: "node-2".to_string(),
        });
        assert!(result.state_changed);
        assert_eq!(
            result.side_effect,
            Some(InteractionSideEffect::ConnectionRemoved)
        );
    }

    /// 测试 `Hover` / `Unhover` 交互 — 悬停状态应正确切换。
    #[test]
    fn test_interaction_state_hover_unhover() {
        let mut state = InteractionState::new();

        // Hover
        let result = state.handle_interaction(&NodeInteraction::Hover {
            node_id: "node-1".to_string(),
        });
        assert!(result.state_changed);
        assert_eq!(state.hovered_node.as_deref(), Some("node-1"));

        // 重复 hover 同一节点 — 状态未变化。
        let result = state.handle_interaction(&NodeInteraction::Hover {
            node_id: "node-1".to_string(),
        });
        assert!(!result.state_changed);

        // Unhover
        let result = state.handle_interaction(&NodeInteraction::Unhover {
            node_id: "node-1".to_string(),
        });
        assert!(result.state_changed);
        assert!(state.hovered_node.is_none());
    }

    /// 测试 `RightClick` 交互 — 应打开右键菜单。
    #[test]
    fn test_interaction_state_right_click() {
        let mut state = InteractionState::new();
        let result = state.handle_interaction(&NodeInteraction::RightClick {
            node_id: "node-1".to_string(),
            position: Position::new(300.0, 400.0),
        });
        assert!(result.state_changed);
        assert_eq!(
            result.side_effect,
            Some(InteractionSideEffect::ContextMenuOpened)
        );
        let menu = state
            .context_menu
            .as_ref()
            .expect("context menu should be open");
        assert_eq!(menu.node_id, "node-1");
        assert_eq!(menu.position, Position::new(300.0, 400.0));
        assert!(
            !menu.items.is_empty(),
            "default menu items should not be empty"
        );
    }

    /// 测试右键菜单关闭 — 其他交互应关闭已打开的菜单。
    #[test]
    fn test_interaction_state_context_menu_closes_on_select() {
        let mut state = InteractionState::new();
        // 先打开菜单。
        state.handle_interaction(&NodeInteraction::RightClick {
            node_id: "node-1".to_string(),
            position: Position::new(100.0, 100.0),
        });
        assert!(state.context_menu.is_some());

        // Select 交互应关闭菜单。
        let result = state.handle_interaction(&NodeInteraction::Select {
            node_id: "node-2".to_string(),
        });
        assert_eq!(
            result.side_effect,
            Some(InteractionSideEffect::ContextMenuClosed)
        );
        assert!(state.context_menu.is_none());
    }

    /// 测试 `DoubleClick` 交互 — 应选中节点并关闭菜单。
    #[test]
    fn test_interaction_state_double_click() {
        let mut state = InteractionState::new();
        // 先打开菜单。
        state.handle_interaction(&NodeInteraction::RightClick {
            node_id: "node-1".to_string(),
            position: Position::new(100.0, 100.0),
        });

        // DoubleClick 应关闭菜单并选中节点。
        let result = state.handle_interaction(&NodeInteraction::DoubleClick {
            node_id: "node-1".to_string(),
        });
        assert!(result.state_changed);
        assert_eq!(
            result.side_effect,
            Some(InteractionSideEffect::ContextMenuClosed)
        );
        assert!(state.context_menu.is_none());
        assert!(state.selected_nodes.contains(&"node-1".to_string()));
    }

    /// 测试 `reset` — 所有状态应被清空。
    #[test]
    fn test_interaction_state_reset() {
        let mut state = InteractionState::new();
        state.selected_nodes.push("node-1".to_string());
        state.hovered_node = Some("node-1".to_string());
        state.dragging_node = Some("node-1".to_string());
        state.drag_offset = Some(Position::new(100.0, 100.0));
        state.connecting_from = Some("node-1".to_string());
        state.context_menu = Some(ContextMenu::new("node-1", Position::new(50.0, 50.0)));

        state.reset();

        assert!(state.selected_nodes.is_empty());
        assert!(state.hovered_node.is_none());
        assert!(state.dragging_node.is_none());
        assert!(state.drag_offset.is_none());
        assert!(state.connecting_from.is_none());
        assert!(state.context_menu.is_none());
    }

    // ===================================================================
    // NodeInteraction 序列化测试
    // ===================================================================

    /// 测试 `NodeInteraction` 的 JSON 序列化/反序列化 round-trip。
    #[test]
    fn test_node_interaction_serde_round_trip() {
        let interactions = vec![
            NodeInteraction::Select {
                node_id: "n1".to_string(),
            },
            NodeInteraction::DragStart {
                node_id: "n1".to_string(),
                position: Position::new(10.0, 20.0),
            },
            NodeInteraction::DragMove {
                node_id: "n1".to_string(),
                new_position: Position::new(30.0, 40.0),
            },
            NodeInteraction::DragEnd {
                node_id: "n1".to_string(),
                final_position: Position::new(50.0, 60.0),
            },
            NodeInteraction::Connect {
                from_node_id: "n1".to_string(),
                to_node_id: "n2".to_string(),
                label: Some("delegate".to_string()),
            },
            NodeInteraction::Connect {
                from_node_id: "n1".to_string(),
                to_node_id: "n2".to_string(),
                label: None,
            },
            NodeInteraction::Disconnect {
                from_node_id: "n1".to_string(),
                to_node_id: "n2".to_string(),
            },
            NodeInteraction::RightClick {
                node_id: "n1".to_string(),
                position: Position::new(100.0, 200.0),
            },
            NodeInteraction::DoubleClick {
                node_id: "n1".to_string(),
            },
            NodeInteraction::Hover {
                node_id: "n1".to_string(),
            },
            NodeInteraction::Unhover {
                node_id: "n1".to_string(),
            },
            NodeInteraction::Deselect {
                node_id: "n1".to_string(),
            },
        ];

        for interaction in &interactions {
            let json = serde_json::to_string(interaction).expect("serialize should succeed");
            let de: NodeInteraction =
                serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(&de, interaction, "round-trip should preserve interaction");
        }
    }

    /// 测试 `NodeInteraction` 的 `snake_case` 序列化标签。
    #[test]
    fn test_node_interaction_serde_snake_case() {
        let json = serde_json::to_string(&NodeInteraction::DragStart {
            node_id: "n1".to_string(),
            position: Position::new(1.0, 2.0),
        })
        .expect("serialize should succeed");
        assert!(
            json.contains("\"drag_start\""),
            "expected snake_case tag in {json}"
        );

        let json = serde_json::to_string(&NodeInteraction::DoubleClick {
            node_id: "n1".to_string(),
        })
        .expect("serialize should succeed");
        assert!(
            json.contains("\"double_click\""),
            "expected snake_case tag in {json}"
        );
    }

    /// 测试 `NodeInteraction::from_json` 解析。
    #[test]
    fn test_node_interaction_from_json() {
        let json = r#"{"select":{"node_id":"n1"}}"#;
        let interaction = NodeInteraction::from_json(json).expect("parse should succeed");
        match interaction {
            NodeInteraction::Select { node_id } => {
                assert_eq!(node_id, "n1");
            }
            _ => panic!("expected Select variant"),
        }
    }

    /// 测试 `NodeInteraction::from_json` 解析失败返回错误。
    #[test]
    fn test_node_interaction_from_json_error() {
        let result = NodeInteraction::from_json("invalid json");
        assert!(result.is_err());
    }

    // ===================================================================
    // Position 测试
    // ===================================================================

    /// 测试 `Position` 的距离计算。
    #[test]
    fn test_position_distance() {
        let a = Position::new(0.0, 0.0);
        let b = Position::new(3.0, 4.0);
        assert!((a.distance_to(&b) - 5.0).abs() < f64::EPSILON);

        let c = Position::new(1.0, 1.0);
        assert!((a.distance_to(&c) - std::f64::consts::SQRT_2).abs() < 1e-10);

        // 同点距离为 0。
        assert!((a.distance_to(&a) - 0.0).abs() < f64::EPSILON);
    }

    // ===================================================================
    // NodeLayout 位置与连接测试
    // ===================================================================

    /// 测试 `NodeLayout` 的位置设置与获取。
    #[test]
    fn test_node_layout_set_get_position() {
        let mut layout = NodeLayout::new();
        layout.set_position("node-1", Position::new(100.0, 200.0));
        layout.set_position("node-2", Position::new(300.0, 400.0));

        let pos1 = layout.get_position("node-1").expect("node-1 should exist");
        assert_eq!(*pos1, Position::new(100.0, 200.0));

        let pos2 = layout.get_position("node-2").expect("node-2 should exist");
        assert_eq!(*pos2, Position::new(300.0, 400.0));

        assert!(layout.get_position("nonexistent").is_none());
    }

    /// 测试 `NodeLayout` 的连接添加与移除。
    #[test]
    fn test_node_layout_add_remove_connection() {
        let mut layout = NodeLayout::new();
        layout.add_connection(LayoutConnection::new("n1", "n2"));
        layout.add_connection(LayoutConnection::new("n2", "n3").with_label("result"));
        assert_eq!(layout.connections.len(), 2);

        // 重复添加相同连接应被忽略。
        layout.add_connection(LayoutConnection::new("n1", "n2"));
        assert_eq!(layout.connections.len(), 2);

        // 移除连接。
        layout.remove_connection("n1", "n2");
        assert_eq!(layout.connections.len(), 1);
        assert_eq!(layout.connections[0].from_node_id, "n2");
        assert_eq!(layout.connections[0].to_node_id, "n3");
        assert_eq!(layout.connections[0].label.as_deref(), Some("result"));

        // 移除不存在的连接 — 无影响。
        layout.remove_connection("n1", "n2");
        assert_eq!(layout.connections.len(), 1);
    }

    /// 测试 `LayoutConnection` 的 builder。
    #[test]
    fn test_layout_connection_builder() {
        let conn = LayoutConnection::new("a", "b").with_label("delegate");
        assert_eq!(conn.from_node_id, "a");
        assert_eq!(conn.to_node_id, "b");
        assert_eq!(conn.label.as_deref(), Some("delegate"));
    }

    // ===================================================================
    // NodeSelectionModel 测试
    // ===================================================================

    /// 测试单选模式 — select 替换当前选中。
    #[test]
    fn test_selection_model_single_select() {
        let mut model = NodeSelectionModel::new();
        assert!(!model.multi_select);

        model.select("n1");
        assert_eq!(model.selected.len(), 1);
        assert!(model.is_selected("n1"));

        // 单选模式下 select 新节点应替换。
        model.select("n2");
        assert_eq!(model.selected.len(), 1);
        assert!(model.is_selected("n2"));
        assert!(!model.is_selected("n1"));
    }

    /// 测试多选模式 — select 追加到选中列表。
    #[test]
    fn test_selection_model_multi_select() {
        let mut model = NodeSelectionModel::multi();
        assert!(model.multi_select);

        model.select("n1");
        model.select("n2");
        model.select("n3");
        assert_eq!(model.selected.len(), 3);
        assert!(model.is_selected("n1"));
        assert!(model.is_selected("n2"));
        assert!(model.is_selected("n3"));

        // 重复 select 同一节点不应增加。
        model.select("n1");
        assert_eq!(model.selected.len(), 3);
    }

    /// 测试 `deselect` 和 `clear`。
    #[test]
    fn test_selection_model_deselect_and_clear() {
        let mut model = NodeSelectionModel::multi();
        model.select("n1");
        model.select("n2");
        model.select("n3");

        model.deselect("n2");
        assert_eq!(model.selected.len(), 2);
        assert!(!model.is_selected("n2"));

        model.clear();
        assert!(model.selected.is_empty());
        assert!(!model.is_selected("n1"));
    }

    /// 测试 `select_all` — 批量追加。
    #[test]
    fn test_selection_model_select_all() {
        let mut model = NodeSelectionModel::multi();
        model.select("n0");

        let ids = vec!["n1".to_string(), "n2".to_string(), "n3".to_string()];
        model.select_all(&ids);

        assert_eq!(model.selected.len(), 4);
        assert!(model.is_selected("n0"));
        assert!(model.is_selected("n1"));
        assert!(model.is_selected("n2"));
        assert!(model.is_selected("n3"));

        // 再次 select_all 不应重复。
        model.select_all(&ids);
        assert_eq!(model.selected.len(), 4);
    }

    // ===================================================================
    // HitTest 命中测试
    // ===================================================================

    /// 测试命中节点。
    #[test]
    fn test_hit_test_node() {
        let mut layout = NodeLayout::new();
        layout.set_position("n1", Position::new(100.0, 100.0));
        layout.set_position("n2", Position::new(300.0, 300.0));

        // 点击在 n1 的半径内。
        let result = hit_test(&layout, Position::new(105.0, 105.0), 20.0);
        assert_eq!(result, HitTestResult::Node("n1".to_string()));

        // 点击恰好在半径边界上。
        let result = hit_test(&layout, Position::new(120.0, 100.0), 20.0);
        assert_eq!(result, HitTestResult::Node("n1".to_string()));
    }

    /// 测试命中连接。
    #[test]
    fn test_hit_test_connection() {
        let mut layout = NodeLayout::new();
        layout.set_position("n1", Position::new(0.0, 0.0));
        layout.set_position("n2", Position::new(100.0, 0.0));
        layout.add_connection(LayoutConnection::new("n1", "n2"));

        // 点击在连接线段附近（y 偏移在阈值内）。
        let result = hit_test(&layout, Position::new(50.0, 3.0), 20.0);
        assert_eq!(
            result,
            HitTestResult::Connection("n1".to_string(), "n2".to_string())
        );
    }

    /// 测试命中空白区域。
    #[test]
    fn test_hit_test_empty() {
        let mut layout = NodeLayout::new();
        layout.set_position("n1", Position::new(0.0, 0.0));
        layout.set_position("n2", Position::new(100.0, 0.0));
        layout.add_connection(LayoutConnection::new("n1", "n2"));

        // 点击在远离任何节点和连接的位置。
        let result = hit_test(&layout, Position::new(500.0, 500.0), 20.0);
        assert_eq!(result, HitTestResult::Empty);
    }

    /// 测试节点命中优先于连接命中。
    #[test]
    fn test_hit_test_node_priority_over_connection() {
        let mut layout = NodeLayout::new();
        layout.set_position("n1", Position::new(0.0, 0.0));
        layout.set_position("n2", Position::new(100.0, 0.0));
        layout.add_connection(LayoutConnection::new("n1", "n2"));

        // 点击在 n1 上（同时也在连接附近），应返回 Node。
        let result = hit_test(&layout, Position::new(0.0, 0.0), 20.0);
        assert_eq!(result, HitTestResult::Node("n1".to_string()));
    }

    // ===================================================================
    // ContextMenu 测试
    // ===================================================================

    /// 测试 `ContextMenu` 构建 — 默认菜单项。
    #[test]
    fn test_context_menu_default_items() {
        let menu = ContextMenu::new("node-1", Position::new(100.0, 200.0));
        assert_eq!(menu.node_id, "node-1");
        assert_eq!(menu.position, Position::new(100.0, 200.0));
        assert!(!menu.items.is_empty());

        // 验证包含分隔符。
        assert!(menu.items.iter().any(|item| item.separator));
        // 验证包含非分隔符项。
        assert!(menu.items.iter().any(|item| !item.separator));
        // 验证所有非分隔符项有非空 label。
        for item in &menu.items {
            if !item.separator {
                assert!(
                    !item.label.is_empty(),
                    "non-separator item should have label"
                );
                assert!(
                    !item.action.is_empty(),
                    "non-separator item should have action"
                );
            }
        }
    }

    /// 测试 `ContextMenu::empty` + `with_items` builder。
    #[test]
    fn test_context_menu_empty_and_with_items() {
        let items = vec![
            ContextMenuItem::new("自定义", "custom"),
            ContextMenuItem::separator(),
            ContextMenuItem::new("禁用项", "disabled").with_enabled(false),
        ];
        let menu = ContextMenu::empty("node-2", Position::new(50.0, 50.0)).with_items(items);

        assert_eq!(menu.node_id, "node-2");
        assert_eq!(menu.items.len(), 3);
        assert_eq!(menu.items[0].label, "自定义");
        assert!(menu.items[0].enabled);
        assert!(menu.items[1].separator);
        assert!(!menu.items[2].enabled);
    }

    /// 测试 `ContextMenuItem::separator`。
    #[test]
    fn test_context_menu_item_separator() {
        let sep = ContextMenuItem::separator();
        assert!(sep.separator);
        assert!(!sep.enabled);
        assert!(sep.label.is_empty());
        assert!(sep.action.is_empty());
    }

    // ===================================================================
    // InteractionResult 测试
    // ===================================================================

    /// 测试 `InteractionResult` 的状态变化检测。
    #[test]
    fn test_interaction_result_state_changed() {
        // 有状态变化的结果。
        let result = InteractionResult::new(true, true, Some(InteractionSideEffect::NodeMoved));
        assert!(result.state_changed);
        assert!(result.requires_redraw);
        assert_eq!(result.side_effect, Some(InteractionSideEffect::NodeMoved));

        // 空结果。
        let empty = InteractionResult::empty();
        assert!(!empty.state_changed);
        assert!(!empty.requires_redraw);
        assert!(empty.side_effect.is_none());
    }

    /// 测试 `InteractionResult` 序列化 round-trip。
    #[test]
    fn test_interaction_result_serde_round_trip() {
        let result =
            InteractionResult::new(true, false, Some(InteractionSideEffect::ConnectionAdded));
        let json = serde_json::to_string(&result).expect("serialize should succeed");
        let de: InteractionResult =
            serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(de, result);
    }

    // ===================================================================
    // LayoutAlgorithm 各变体测试
    // ===================================================================

    /// 辅助：构造含 N 个节点的布局。
    fn make_layout(n: usize) -> NodeLayout {
        let mut layout = NodeLayout::new();
        for i in 0..n {
            let id = format!("n{i}");
            layout.set_position(&id, Position::default());
        }
        layout
    }

    /// 测试网格布局 — 节点应排列为网格，位置不为全零。
    #[test]
    fn test_layout_algorithm_grid() {
        let mut layout = make_layout(9);
        layout.layout_auto(LayoutAlgorithm::Grid);

        // 9 个节点应有 9 个非默认位置。
        let non_default = layout
            .node_positions
            .values()
            .filter(|p| **p != Position::default())
            .count();
        assert_eq!(
            non_default, 9,
            "all 9 nodes should have non-default positions"
        );

        // 网格应有 3 列（sqrt(9) = 3）。
        let n0 = *layout.get_position("n0").unwrap();
        let n1 = *layout.get_position("n1").unwrap();
        let n3 = *layout.get_position("n3").unwrap();
        // n0 和 n1 在同一行（y 相同），x 不同。
        assert!((n0.y - n1.y).abs() < f64::EPSILON);
        assert!(n1.x > n0.x);
        // n0 和 n3 在不同行（y 不同）。
        assert!(n3.y > n0.y);
    }

    /// 测试环形布局 — 节点应均匀分布在圆周上。
    #[test]
    fn test_layout_algorithm_circular() {
        let mut layout = make_layout(4);
        layout.layout_auto(LayoutAlgorithm::Circular);

        // 所有节点到圆心的距离应相等。
        let center = Position::new(300.0, 300.0);
        let radius = 300.0;
        for i in 0..4 {
            let id = format!("n{i}");
            let pos = *layout.get_position(&id).unwrap();
            let dist = pos.distance_to(&center);
            assert!(
                (dist - radius).abs() < 1e-10,
                "node {id} should be on circle, dist={dist}, expected={radius}"
            );
        }
    }

    /// 测试层级布局 — 无入边的节点应在第一层。
    #[test]
    fn test_layout_algorithm_hierarchical() {
        let mut layout = make_layout(3);
        // n0 → n1 → n2
        layout.add_connection(LayoutConnection::new("n0", "n1"));
        layout.add_connection(LayoutConnection::new("n1", "n2"));

        layout.layout_auto(LayoutAlgorithm::Hierarchical);

        let y0 = layout.get_position("n0").unwrap().y;
        let y1 = layout.get_position("n1").unwrap().y;
        let y2 = layout.get_position("n2").unwrap().y;

        // n0 在第 0 层（y 最小），n1 在第 1 层，n2 在第 2 层。
        assert!(y0 < y1, "n0 (layer 0) should be above n1 (layer 1)");
        assert!(y1 < y2, "n1 (layer 1) should be above n2 (layer 2)");
    }

    /// 测试力导向布局 — 不应 panic，且节点位置应发生变化。
    #[test]
    fn test_layout_algorithm_force() {
        let mut layout = make_layout(5);
        layout.add_connection(LayoutConnection::new("n0", "n1"));
        layout.add_connection(LayoutConnection::new("n1", "n2"));

        // 不应 panic。
        layout.layout_auto(LayoutAlgorithm::Force);

        // 力导向布局后节点应有非零位置（从圆周初始化开始）。
        for i in 0..5 {
            let id = format!("n{i}");
            let pos = layout.get_position(&id).unwrap();
            // 初始圆周布局在 (300+300*cos, 300+300*sin) 附近，迭代后应有变化。
            assert!(
                pos.x != 0.0 || pos.y != 0.0,
                "node {id} should have non-zero position after force layout"
            );
        }
    }

    /// 测试 `LayoutAlgorithm` 的 `snake_case` 序列化。
    #[test]
    fn test_layout_algorithm_serde_snake_case() {
        let cases = [
            (LayoutAlgorithm::Force, "force"),
            (LayoutAlgorithm::Grid, "grid"),
            (LayoutAlgorithm::Circular, "circular"),
            (LayoutAlgorithm::Hierarchical, "hierarchical"),
        ];
        for (algo, expected) in cases {
            let s = serde_json::to_string(&algo).expect("serialize should succeed");
            assert!(s.contains(expected), "expected {expected} in {s}");
            let de: LayoutAlgorithm = serde_json::from_str(&s).expect("deserialize should succeed");
            assert_eq!(de, algo);
        }
    }

    // ===================================================================
    // Default trait 测试
    // ===================================================================

    /// 测试各类型的 `Default` 实现。
    #[test]
    fn test_default_impls() {
        let state = InteractionState::default();
        assert!(state.selected_nodes.is_empty());
        assert!(state.hovered_node.is_none());

        let layout = NodeLayout::default();
        assert!(layout.node_positions.is_empty());
        assert!(layout.connections.is_empty());

        let model = NodeSelectionModel::default();
        assert!(!model.multi_select);
        assert!(model.selected.is_empty());

        let pos = Position::default();
        assert!((pos.x - 0.0).abs() < f64::EPSILON);
        assert!((pos.y - 0.0).abs() < f64::EPSILON);
    }

    /// 测试 `NodeLayout` 反序列化时缺失字段使用默认值。
    #[test]
    fn test_node_layout_deserialize_with_defaults() {
        let json = r#"{}"#;
        let de: NodeLayout =
            serde_json::from_str(json).expect("deserialize with defaults should succeed");
        assert!(de.node_positions.is_empty());
        assert!(de.connections.is_empty());
    }
}
