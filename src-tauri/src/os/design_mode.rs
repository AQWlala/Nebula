//! T-E-C-12: Design Mode — 可视化设计画布。
//!
//! 允许用户通过可视化设计界面创建自动化流程:在画布上添加节点、连接节点、
//! 验证设计、转换为执行计划。支持模板库快速实例化常见工作流。
//!
//! ## 架构
//!
//! * [`DesignCanvas`] — 可视化设计画布,管理节点与连接。
//! * [`DesignNode`] — 画布上的节点(触发器 / 条件 / 动作 / 延时 / 循环 / 分支 / 输出 / 输入 / 变量 / 注释)。
//! * [`DesignConnection`] — 节点间的有向连接(可选条件与标签)。
//! * [`ValidationResult`] — 设计验证结果(错误 + 警告)。
//! * [`DesignAction`] — 执行计划中的单步动作(由画布拓扑排序生成)。
//! * [`DesignTemplateLibrary`] — 内置模板库(6+ 个预置工作流)。
//!
//! ## 注册
//!
//! 本模块当前未在 `os/mod.rs` 中注册(遵循"只创建新文件"约束)。注册时添加:
//! ```ignore
//! // in src-tauri/src/os/mod.rs
//! pub mod design_mode;
//! ```

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------
// DesignNodeType — 节点类型枚举
// ----------------------------------------------------------------------

/// 设计节点类型(serde snake_case 序列化)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DesignNodeType {
    /// 触发器(流程起点)。
    Trigger,
    /// 条件判断。
    Condition,
    /// 动作执行。
    Action,
    /// 延时。
    Delay,
    /// 循环。
    Loop,
    /// 分支。
    Branch,
    /// 输出。
    Output,
    /// 输入。
    Input,
    /// 变量。
    Variable,
    /// 注释(不参与执行)。
    Comment,
}

impl DesignNodeType {
    /// 返回节点类型名(对应 serde snake_case 变体名)。
    pub fn type_name(&self) -> &'static str {
        match self {
            DesignNodeType::Trigger => "trigger",
            DesignNodeType::Condition => "condition",
            DesignNodeType::Action => "action",
            DesignNodeType::Delay => "delay",
            DesignNodeType::Loop => "loop",
            DesignNodeType::Branch => "branch",
            DesignNodeType::Output => "output",
            DesignNodeType::Input => "input",
            DesignNodeType::Variable => "variable",
            DesignNodeType::Comment => "comment",
        }
    }
}

// ----------------------------------------------------------------------
// NodePosition — 节点画布坐标
// ----------------------------------------------------------------------

/// 节点在画布上的坐标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NodePosition {
    /// X 坐标。
    pub x: f64,
    /// Y 坐标。
    pub y: f64,
}

impl NodePosition {
    /// 创建新的节点坐标。
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

// ----------------------------------------------------------------------
// DesignNode — 设计节点
// ----------------------------------------------------------------------

/// 设计节点 — 画布上的一个可视化节点。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DesignNode {
    /// 节点 ID(画布内唯一)。
    pub node_id: String,
    /// 节点类型。
    pub node_type: DesignNodeType,
    /// 节点标签(显示名)。
    pub label: String,
    /// 节点画布坐标。
    pub position: NodePosition,
    /// 节点配置(自由 key-value,值为 JSON)。
    pub config: HashMap<String, serde_json::Value>,
    /// 是否启用(禁用的节点在执行计划中被跳过)。
    pub enabled: bool,
}

impl DesignNode {
    /// 创建新的设计节点(默认 enabled=true,空 config,坐标 (0,0))。
    pub fn new(
        node_id: impl Into<String>,
        node_type: DesignNodeType,
        label: impl Into<String>,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            node_type,
            label: label.into(),
            position: NodePosition::default(),
            config: HashMap::new(),
            enabled: true,
        }
    }
}

// ----------------------------------------------------------------------
// DesignConnection — 节点连接
// ----------------------------------------------------------------------

/// 节点连接 — 从 `from_node` 到 `to_node` 的有向边(可选条件与标签)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesignConnection {
    /// 起始节点 ID。
    pub from_node: String,
    /// 目标节点 ID。
    pub to_node: String,
    /// 连接条件(如分支条件表达式)。
    pub condition: Option<String>,
    /// 连接标签(显示名)。
    pub label: Option<String>,
}

impl DesignConnection {
    /// 创建新的节点连接(无条件、无标签)。
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from_node: from.into(),
            to_node: to.into(),
            condition: None,
            label: None,
        }
    }
}

// ----------------------------------------------------------------------
// ValidationErrorType — 验证错误类型
// ----------------------------------------------------------------------

/// 验证错误类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorType {
    /// 缺少输入。
    MissingInput,
    /// 配置无效。
    InvalidConfig,
    /// 循环依赖。
    CircularDependency,
    /// 节点未连接。
    DisconnectedNode,
    /// 重复 ID。
    DuplicateId,
    /// 无效连接。
    InvalidConnection,
}

// ----------------------------------------------------------------------
// ValidationError / ValidationWarning / ValidationResult
// ----------------------------------------------------------------------

/// 验证错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    /// 相关节点 ID(全局错误时为 None)。
    pub node_id: Option<String>,
    /// 错误类型。
    pub error_type: ValidationErrorType,
    /// 错误信息。
    pub message: String,
}

/// 验证警告。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationWarning {
    /// 相关节点 ID(全局警告时为 None)。
    pub node_id: Option<String>,
    /// 警告信息。
    pub message: String,
}

/// 验证结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ValidationResult {
    /// 是否有效(无错误即有效)。
    pub valid: bool,
    /// 错误列表。
    pub errors: Vec<ValidationError>,
    /// 警告列表。
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationResult {
    /// 创建新的验证结果(默认 valid=false,需在检查完毕后置 true)。
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加错误(自动将 valid 置为 false)。
    pub fn add_error(
        &mut self,
        node_id: Option<String>,
        error_type: ValidationErrorType,
        message: impl Into<String>,
    ) {
        self.errors.push(ValidationError {
            node_id,
            error_type,
            message: message.into(),
        });
        self.valid = false;
    }

    /// 添加警告(不影响 valid)。
    pub fn add_warning(&mut self, node_id: Option<String>, message: impl Into<String>) {
        self.warnings.push(ValidationWarning {
            node_id,
            message: message.into(),
        });
    }
}

// ----------------------------------------------------------------------
// DesignAction — 执行计划单步动作
// ----------------------------------------------------------------------

/// 执行计划单步动作(由画布拓扑排序生成)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesignAction {
    /// 步骤序号(1 基)。
    pub step: u32,
    /// 动作类型(对应节点 type_name)。
    pub action_type: String,
    /// 源节点 ID。
    pub node_id: String,
    /// 命令(从节点 config["command"] 或 label 派生)。
    pub command: String,
    /// 命令参数(从节点 config["args"] 派生)。
    pub args: Vec<String>,
    /// 条件(从入边 condition 派生)。
    pub condition: Option<String>,
}

// ----------------------------------------------------------------------
// DesignCanvas — 可视化设计画布
// ----------------------------------------------------------------------

/// 可视化设计画布 — 管理节点与连接,支持验证与执行计划生成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DesignCanvas {
    /// 画布上的节点列表。
    nodes: Vec<DesignNode>,
    /// 节点间的连接列表。
    connections: Vec<DesignConnection>,
}

impl Default for DesignCanvas {
    fn default() -> Self {
        Self::new()
    }
}

impl DesignCanvas {
    /// 创建空画布。
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            connections: Vec::new(),
        }
    }

    /// 添加节点(若 node_id 已存在则返回 Err)。
    pub fn add_node(&mut self, node: DesignNode) -> Result<()> {
        if self.nodes.iter().any(|n| n.node_id == node.node_id) {
            anyhow::bail!("node_id already exists: {}", node.node_id);
        }
        tracing::debug!(node_id = %node.node_id, node_type = ?node.node_type, "add_node");
        self.nodes.push(node);
        Ok(())
    }

    /// 移除节点(同时移除相关连接;若节点不存在则返回 Err)。
    pub fn remove_node(&mut self, node_id: &str) -> Result<()> {
        let before = self.nodes.len();
        self.nodes.retain(|n| n.node_id != node_id);
        if self.nodes.len() == before {
            anyhow::bail!("node_id not found: {}", node_id);
        }
        // 移除涉及该节点的所有连接。
        self.connections
            .retain(|c| c.from_node != node_id && c.to_node != node_id);
        tracing::debug!(node_id = %node_id, "remove_node");
        Ok(())
    }

    /// 连接两个节点(若节点不存在、自连接或连接已存在则返回 Err)。
    pub fn connect(&mut self, from: &str, to: &str) -> Result<()> {
        // 校验节点存在。
        if !self.nodes.iter().any(|n| n.node_id == from) {
            anyhow::bail!("from_node not found: {}", from);
        }
        if !self.nodes.iter().any(|n| n.node_id == to) {
            anyhow::bail!("to_node not found: {}", to);
        }
        // 禁止自连接。
        if from == to {
            anyhow::bail!("cannot connect node to itself: {}", from);
        }
        // 校验连接未重复。
        if self
            .connections
            .iter()
            .any(|c| c.from_node == from && c.to_node == to)
        {
            anyhow::bail!("connection already exists: {} -> {}", from, to);
        }
        self.connections.push(DesignConnection {
            from_node: from.to_string(),
            to_node: to.to_string(),
            condition: None,
            label: None,
        });
        tracing::debug!(from = %from, to = %to, "connect");
        Ok(())
    }

    /// 断开两个节点的连接(若连接不存在则返回 Err)。
    pub fn disconnect(&mut self, from: &str, to: &str) -> Result<()> {
        let before = self.connections.len();
        self.connections
            .retain(|c| !(c.from_node == from && c.to_node == to));
        if self.connections.len() == before {
            anyhow::bail!("connection not found: {} -> {}", from, to);
        }
        tracing::debug!(from = %from, to = %to, "disconnect");
        Ok(())
    }

    /// 获取所有节点。
    pub fn nodes(&self) -> &[DesignNode] {
        &self.nodes
    }

    /// 获取所有连接。
    pub fn connections(&self) -> &[DesignConnection] {
        &self.connections
    }

    /// 验证设计(检查重复 ID、无效连接、循环依赖、断开节点、缺少输入)。
    pub fn validate(&self) -> ValidationResult {
        let mut result = ValidationResult::new();

        // 1. 检查重复 ID。
        let mut seen: HashSet<&str> = HashSet::new();
        for n in &self.nodes {
            if !seen.insert(n.node_id.as_str()) {
                result.add_error(
                    Some(n.node_id.clone()),
                    ValidationErrorType::DuplicateId,
                    format!("duplicate node id: {}", n.node_id),
                );
            }
        }

        // 2. 检查无效连接(引用不存在的节点)。
        let id_set: HashSet<&str> = self.nodes.iter().map(|n| n.node_id.as_str()).collect();
        for c in &self.connections {
            if !id_set.contains(c.from_node.as_str()) {
                result.add_error(
                    Some(c.from_node.clone()),
                    ValidationErrorType::InvalidConnection,
                    format!("connection from unknown node: {}", c.from_node),
                );
            }
            if !id_set.contains(c.to_node.as_str()) {
                result.add_error(
                    Some(c.to_node.clone()),
                    ValidationErrorType::InvalidConnection,
                    format!("connection to unknown node: {}", c.to_node),
                );
            }
        }

        // 3. 检查循环依赖(DFS)。
        if let Some(cycle_node) = self.detect_cycle() {
            result.add_error(
                Some(cycle_node.clone()),
                ValidationErrorType::CircularDependency,
                format!(
                    "circular dependency detected involving node: {}",
                    cycle_node
                ),
            );
        }

        // 4. 检查断开节点(无任何入边或出边的非注释节点)。
        for n in &self.nodes {
            // 注释节点不参与连接,跳过。
            if n.node_type == DesignNodeType::Comment {
                continue;
            }
            let has_in = self.connections.iter().any(|c| c.to_node == n.node_id);
            let has_out = self.connections.iter().any(|c| c.from_node == n.node_id);
            if !has_in && !has_out {
                result.add_error(
                    Some(n.node_id.clone()),
                    ValidationErrorType::DisconnectedNode,
                    format!("node is disconnected: {}", n.node_id),
                );
            }
        }

        // 5. 检查缺少输入(Action / Output / Condition 节点应有入边)。
        for n in &self.nodes {
            match n.node_type {
                DesignNodeType::Action | DesignNodeType::Output | DesignNodeType::Condition => {
                    let has_in = self.connections.iter().any(|c| c.to_node == n.node_id);
                    if !has_in {
                        result.add_error(
                            Some(n.node_id.clone()),
                            ValidationErrorType::MissingInput,
                            format!("node has no input: {}", n.node_id),
                        );
                    }
                }
                _ => {}
            }
        }

        // 若无错误则 valid=true(空画布自然有效)。
        if result.errors.is_empty() {
            result.valid = true;
        }

        result
    }

    /// 检测循环依赖(DFS);返回环中任意一个节点 ID,无环返回 None。
    fn detect_cycle(&self) -> Option<String> {
        // 构建邻接表(用 String 避免 lifetime 复杂性)。
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        for c in &self.connections {
            adj.entry(c.from_node.clone())
                .or_default()
                .push(c.to_node.clone());
        }

        // 0=未访问,1=访问中(栈上),2=已完成。
        let mut color: HashMap<String, u8> = HashMap::new();
        for n in &self.nodes {
            color.insert(n.node_id.clone(), 0);
        }

        for n in &self.nodes {
            if color.get(&n.node_id).copied().unwrap_or(0) == 0 {
                if let Some(cycle) = Self::dfs_cycle(&n.node_id, &adj, &mut color) {
                    return Some(cycle);
                }
            }
        }
        None
    }

    /// DFS 环检测辅助(递归)。
    fn dfs_cycle(
        u: &str,
        adj: &HashMap<String, Vec<String>>,
        color: &mut HashMap<String, u8>,
    ) -> Option<String> {
        color.insert(u.to_string(), 1);
        if let Some(neighbors) = adj.get(u) {
            for v in neighbors {
                let cv = color.get(v).copied().unwrap_or(0);
                if cv == 1 {
                    // 找到环(v 在当前 DFS 栈上)。
                    return Some(v.clone());
                }
                if cv == 0 {
                    if let Some(cycle) = Self::dfs_cycle(v, adj, color) {
                        return Some(cycle);
                    }
                }
            }
        }
        color.insert(u.to_string(), 2);
        None
    }

    /// 转换为执行计划(拓扑排序;存在环或无效设计时返回 Err)。
    pub fn to_execution_plan(&self) -> Result<Vec<DesignAction>> {
        // 先验证。
        let validation = self.validate();
        if !validation.valid {
            let msgs: Vec<String> = validation
                .errors
                .iter()
                .map(|e| e.message.clone())
                .collect();
            anyhow::bail!(
                "design is invalid, cannot generate execution plan: {}",
                msgs.join("; ")
            );
        }

        // 拓扑排序(Kahn 算法)。
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for n in &self.nodes {
            in_degree.insert(n.node_id.as_str(), 0);
        }
        for c in &self.connections {
            adj.entry(c.from_node.as_str())
                .or_default()
                .push(c.to_node.as_str());
            *in_degree.entry(c.to_node.as_str()).or_insert(0) += 1;
        }

        // 入度为 0 的节点入队(按 nodes 顺序以保证确定性)。
        let mut queue: Vec<&str> = self
            .nodes
            .iter()
            .map(|n| n.node_id.as_str())
            .filter(|id| in_degree.get(id).copied().unwrap_or(0) == 0)
            .collect();

        let mut order: Vec<&str> = Vec::with_capacity(self.nodes.len());
        while let Some(u) = queue.first().copied() {
            queue.remove(0);
            order.push(u);
            if let Some(neighbors) = adj.get(u) {
                for &v in neighbors {
                    if let Some(d) = in_degree.get_mut(v) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push(v);
                        }
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            // 理论上 validate() 已检测环,此处为防御性检查。
            anyhow::bail!("circular dependency prevents topological sort");
        }

        // 生成执行计划。
        let mut plan: Vec<DesignAction> = Vec::with_capacity(order.len());
        for &node_id in &order {
            let node = self
                .nodes
                .iter()
                .find(|n| n.node_id == node_id)
                .expect("node must exist in topo order");

            // 跳过禁用节点与注释节点。
            if !node.enabled || node.node_type == DesignNodeType::Comment {
                continue;
            }

            // 派生 command:优先 config["command"],其次 label。
            let command = node
                .config
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| node.label.clone());

            // 派生 args:config["args"] 为字符串数组。
            let args: Vec<String> = node
                .config
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            // 派生 condition:首条入边的 condition。
            let condition = self
                .connections
                .iter()
                .find(|c| c.to_node == node_id)
                .and_then(|c| c.condition.clone());

            plan.push(DesignAction {
                step: 0, // 下方统一编号。
                action_type: node.node_type.type_name().to_string(),
                node_id: node.node_id.clone(),
                command,
                args,
                condition,
            });
        }

        // 重新编号 step(连续,跳过的节点不占号)。
        for (i, a) in plan.iter_mut().enumerate() {
            a.step = (i + 1) as u32;
        }

        Ok(plan)
    }

    /// 序列化为 JSON 字符串(美化格式)。
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// 从 JSON 字符串反序列化。
    pub fn from_json(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json)?)
    }
}

// ----------------------------------------------------------------------
// DesignTemplate — 设计模板
// ----------------------------------------------------------------------

/// 设计模板 — 预置的画布工作流,可通过 template_id 实例化。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DesignTemplate {
    /// 模板 ID(全局唯一)。
    pub template_id: String,
    /// 模板名称。
    pub name: String,
    /// 模板描述。
    pub description: String,
    /// 模板分类。
    pub category: String,
    /// 模板画布。
    pub canvas: DesignCanvas,
}

// ----------------------------------------------------------------------
// DesignTemplateLibrary — 模板库
// ----------------------------------------------------------------------

/// 设计模板库 — 内置 6+ 个预置工作流模板。
pub struct DesignTemplateLibrary {
    templates: Vec<DesignTemplate>,
}

impl Default for DesignTemplateLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl DesignTemplateLibrary {
    /// 创建模板库(内置 6 个模板)。
    pub fn new() -> Self {
        let templates = vec![
            build_data_entry_automation(),
            build_file_processing(),
            build_web_scraping(),
            build_report_generation(),
            build_batch_rename(),
            build_screenshot_workflow(),
        ];
        Self { templates }
    }

    /// 列出所有模板。
    pub fn list(&self) -> Vec<&DesignTemplate> {
        self.templates.iter().collect()
    }

    /// 按 template_id 获取模板。
    pub fn get(&self, template_id: &str) -> Option<&DesignTemplate> {
        self.templates.iter().find(|t| t.template_id == template_id)
    }

    /// 实例化模板(返回模板画布的克隆)。
    pub fn instantiate(&self, template_id: &str) -> Result<DesignCanvas> {
        let tmpl = self
            .get(template_id)
            .ok_or_else(|| anyhow::anyhow!("template not found: {}", template_id))?;
        Ok(tmpl.canvas.clone())
    }
}

// ----------------------------------------------------------------------
// 内置模板构建函数
// ----------------------------------------------------------------------

/// 辅助:创建带坐标的节点。
fn node(id: &str, node_type: DesignNodeType, label: &str, x: f64, y: f64) -> DesignNode {
    DesignNode {
        node_id: id.to_string(),
        node_type,
        label: label.to_string(),
        position: NodePosition::new(x, y),
        config: HashMap::new(),
        enabled: true,
    }
}

/// 辅助:创建带 config 的节点。
fn node_with_config(
    id: &str,
    node_type: DesignNodeType,
    label: &str,
    x: f64,
    y: f64,
    config: HashMap<String, serde_json::Value>,
) -> DesignNode {
    DesignNode {
        node_id: id.to_string(),
        node_type,
        label: label.to_string(),
        position: NodePosition::new(x, y),
        config,
        enabled: true,
    }
}

/// 辅助:从节点列表与连接列表构建画布。
fn build_canvas(nodes: Vec<DesignNode>, conns: Vec<(&str, &str)>) -> DesignCanvas {
    let mut canvas = DesignCanvas::new();
    for n in nodes {
        canvas
            .add_node(n)
            .expect("add_node should succeed in template");
    }
    for (from, to) in conns {
        canvas
            .connect(from, to)
            .expect("connect should succeed in template");
    }
    canvas
}

/// 辅助:构建 config HashMap。
fn config(pairs: Vec<(&str, serde_json::Value)>) -> HashMap<String, serde_json::Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// 模板 1:数据录入自动化。
fn build_data_entry_automation() -> DesignTemplate {
    let nodes = vec![
        node("t1", DesignNodeType::Trigger, "启动", 0.0, 0.0),
        node_with_config(
            "a1",
            DesignNodeType::Action,
            "打开应用",
            200.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("open_app")),
                ("args", serde_json::json!(["notepad.exe"])),
            ]),
        ),
        node_with_config(
            "a2",
            DesignNodeType::Action,
            "点击输入框",
            400.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("click")),
                ("args", serde_json::json!(["#input_field"])),
            ]),
        ),
        node_with_config(
            "a3",
            DesignNodeType::Action,
            "输入数据",
            600.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("type_text")),
                ("args", serde_json::json!(["hello world"])),
            ]),
        ),
        node_with_config(
            "a4",
            DesignNodeType::Action,
            "保存",
            800.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("save")),
                ("args", serde_json::json!(["Ctrl+S"])),
            ]),
        ),
        node("o1", DesignNodeType::Output, "完成", 1000.0, 0.0),
    ];
    let conns = vec![
        ("t1", "a1"),
        ("a1", "a2"),
        ("a2", "a3"),
        ("a3", "a4"),
        ("a4", "o1"),
    ];
    DesignTemplate {
        template_id: "data_entry_automation".into(),
        name: "数据录入自动化".into(),
        description: "自动打开应用、定位输入框、录入数据并保存的完整流程".into(),
        category: "办公自动化".into(),
        canvas: build_canvas(nodes, conns),
    }
}

/// 模板 2:文件处理流程。
fn build_file_processing() -> DesignTemplate {
    let nodes = vec![
        node("t1", DesignNodeType::Trigger, "启动", 0.0, 0.0),
        node_with_config(
            "a1",
            DesignNodeType::Action,
            "列出文件",
            200.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("list_files")),
                ("args", serde_json::json!(["C:\\data"])),
            ]),
        ),
        node("l1", DesignNodeType::Loop, "遍历文件", 400.0, 0.0),
        node_with_config(
            "a2",
            DesignNodeType::Action,
            "处理文件",
            600.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("process_file")),
                ("args", serde_json::json!([])),
            ]),
        ),
        node("o1", DesignNodeType::Output, "完成", 800.0, 0.0),
    ];
    let conns = vec![("t1", "a1"), ("a1", "l1"), ("l1", "a2"), ("a2", "o1")];
    DesignTemplate {
        template_id: "file_processing".into(),
        name: "文件处理流程".into(),
        description: "列出目录文件并逐一处理的循环工作流".into(),
        category: "文件操作".into(),
        canvas: build_canvas(nodes, conns),
    }
}

/// 模板 3:网页抓取。
fn build_web_scraping() -> DesignTemplate {
    let nodes = vec![
        node("t1", DesignNodeType::Trigger, "启动", 0.0, 0.0),
        node_with_config(
            "a1",
            DesignNodeType::Action,
            "打开网页",
            200.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("open_url")),
                ("args", serde_json::json!(["https://example.com"])),
            ]),
        ),
        node_with_config(
            "a2",
            DesignNodeType::Action,
            "提取数据",
            400.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("extract_data")),
                ("args", serde_json::json!(["table.data"])),
            ]),
        ),
        node_with_config(
            "a3",
            DesignNodeType::Action,
            "翻页",
            600.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("next_page")),
                ("args", serde_json::json!([])),
            ]),
        ),
        node("o1", DesignNodeType::Output, "输出结果", 800.0, 0.0),
    ];
    let conns = vec![("t1", "a1"), ("a1", "a2"), ("a2", "a3"), ("a3", "o1")];
    DesignTemplate {
        template_id: "web_scraping".into(),
        name: "网页抓取".into(),
        description: "打开网页、提取数据、翻页继续抓取的工作流".into(),
        category: "数据采集".into(),
        canvas: build_canvas(nodes, conns),
    }
}

/// 模板 4:报告生成。
fn build_report_generation() -> DesignTemplate {
    let nodes = vec![
        node("t1", DesignNodeType::Trigger, "启动", 0.0, 0.0),
        node_with_config(
            "a1",
            DesignNodeType::Action,
            "收集数据",
            200.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("collect_data")),
                ("args", serde_json::json!([])),
            ]),
        ),
        node_with_config(
            "a2",
            DesignNodeType::Action,
            "格式化报告",
            400.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("format_report")),
                ("args", serde_json::json!(["markdown"])),
            ]),
        ),
        node_with_config(
            "a3",
            DesignNodeType::Action,
            "导出 PDF",
            600.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("export_pdf")),
                ("args", serde_json::json!(["report.pdf"])),
            ]),
        ),
        node("o1", DesignNodeType::Output, "报告已生成", 800.0, 0.0),
    ];
    let conns = vec![("t1", "a1"), ("a1", "a2"), ("a2", "a3"), ("a3", "o1")];
    DesignTemplate {
        template_id: "report_generation".into(),
        name: "报告生成".into(),
        description: "收集数据、格式化报告并导出 PDF 的完整流程".into(),
        category: "办公自动化".into(),
        canvas: build_canvas(nodes, conns),
    }
}

/// 模板 5:批量重命名。
fn build_batch_rename() -> DesignTemplate {
    let nodes = vec![
        node("t1", DesignNodeType::Trigger, "启动", 0.0, 0.0),
        node_with_config(
            "i1",
            DesignNodeType::Input,
            "选择文件",
            200.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("select_files")),
                ("args", serde_json::json!([])),
            ]),
        ),
        node("l1", DesignNodeType::Loop, "遍历文件", 400.0, 0.0),
        node_with_config(
            "a1",
            DesignNodeType::Action,
            "重命名文件",
            600.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("rename_file")),
                ("args", serde_json::json!(["{index}_{name}"])),
            ]),
        ),
        node("o1", DesignNodeType::Output, "重命名完成", 800.0, 0.0),
    ];
    let conns = vec![("t1", "i1"), ("i1", "l1"), ("l1", "a1"), ("a1", "o1")];
    DesignTemplate {
        template_id: "batch_rename".into(),
        name: "批量重命名".into(),
        description: "选择一批文件并按模板批量重命名的工作流".into(),
        category: "文件操作".into(),
        canvas: build_canvas(nodes, conns),
    }
}

/// 模板 6:截图工作流。
fn build_screenshot_workflow() -> DesignTemplate {
    let nodes = vec![
        node("t1", DesignNodeType::Trigger, "启动", 0.0, 0.0),
        node_with_config(
            "a1",
            DesignNodeType::Action,
            "截取屏幕",
            200.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("capture_screen")),
                ("args", serde_json::json!([])),
            ]),
        ),
        node_with_config(
            "a2",
            DesignNodeType::Action,
            "标注截图",
            400.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("annotate")),
                ("args", serde_json::json!(["arrow"])),
            ]),
        ),
        node_with_config(
            "a3",
            DesignNodeType::Action,
            "保存图片",
            600.0,
            0.0,
            config(vec![
                ("command", serde_json::json!("save_image")),
                ("args", serde_json::json!(["screenshot.png"])),
            ]),
        ),
        node("o1", DesignNodeType::Output, "截图已保存", 800.0, 0.0),
    ];
    let conns = vec![("t1", "a1"), ("a1", "a2"), ("a2", "a3"), ("a3", "o1")];
    DesignTemplate {
        template_id: "screenshot_workflow".into(),
        name: "截图工作流".into(),
        description: "截取屏幕、标注并保存图片的完整流程".into(),
        category: "屏幕操作".into(),
        canvas: build_canvas(nodes, conns),
    }
}

// ----------------------------------------------------------------------
// 单元测试
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- DesignCanvas add/remove node ----

    #[test]
    fn canvas_add_node() {
        let mut canvas = DesignCanvas::new();
        let n = DesignNode::new("n1", DesignNodeType::Trigger, "启动");
        canvas.add_node(n).expect("add_node");
        assert_eq!(canvas.nodes().len(), 1);
        assert_eq!(canvas.nodes()[0].node_id, "n1");
        assert_eq!(canvas.nodes()[0].node_type, DesignNodeType::Trigger);
    }

    #[test]
    fn canvas_add_node_duplicate_returns_err() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node first");
        let err = canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Action, "重复"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("already exists"), "unexpected err: {}", err);
        // 仍然只有一个节点。
        assert_eq!(canvas.nodes().len(), 1);
    }

    #[test]
    fn canvas_remove_node() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("n2", DesignNodeType::Action, "动作"))
            .expect("add_node");
        canvas.connect("n1", "n2").expect("connect");

        // 移除 n2,连接也应被清除。
        canvas.remove_node("n2").expect("remove_node");
        assert_eq!(canvas.nodes().len(), 1);
        assert!(canvas.connections().is_empty());
    }

    #[test]
    fn canvas_remove_node_not_found_returns_err() {
        let mut canvas = DesignCanvas::new();
        let err = canvas.remove_node("nope").unwrap_err().to_string();
        assert!(err.contains("not found"), "unexpected err: {}", err);
    }

    // ---- DesignCanvas connect/disconnect ----

    #[test]
    fn canvas_connect() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("n2", DesignNodeType::Action, "动作"))
            .expect("add_node");

        canvas.connect("n1", "n2").expect("connect");
        assert_eq!(canvas.connections().len(), 1);
        assert_eq!(canvas.connections()[0].from_node, "n1");
        assert_eq!(canvas.connections()[0].to_node, "n2");
    }

    #[test]
    fn canvas_connect_unknown_node_returns_err() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");

        let err = canvas.connect("n1", "nope").unwrap_err().to_string();
        assert!(err.contains("not found"), "unexpected err: {}", err);

        let err = canvas.connect("nope", "n1").unwrap_err().to_string();
        assert!(err.contains("not found"), "unexpected err: {}", err);
    }

    #[test]
    fn canvas_connect_self_returns_err() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        let err = canvas.connect("n1", "n1").unwrap_err().to_string();
        assert!(err.contains("itself"), "unexpected err: {}", err);
    }

    #[test]
    fn canvas_disconnect() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("n2", DesignNodeType::Action, "动作"))
            .expect("add_node");
        canvas.connect("n1", "n2").expect("connect");
        assert_eq!(canvas.connections().len(), 1);

        canvas.disconnect("n1", "n2").expect("disconnect");
        assert!(canvas.connections().is_empty());
    }

    #[test]
    fn canvas_disconnect_not_found_returns_err() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("n1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("n2", DesignNodeType::Action, "动作"))
            .expect("add_node");
        let err = canvas.disconnect("n1", "n2").unwrap_err().to_string();
        assert!(err.contains("not found"), "unexpected err: {}", err);
    }

    // ---- validate 各种场景 ----

    #[test]
    fn validate_empty_canvas() {
        let canvas = DesignCanvas::new();
        let result = canvas.validate();
        assert!(result.valid, "empty canvas should be valid");
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn validate_circular_dependency() {
        // n1 -> n2 -> n3 -> n1 (环)
        let json = r#"{
            "nodes": [
                {"node_id": "n1", "node_type": "trigger", "label": "T", "position": {"x": 0.0, "y": 0.0}, "config": {}, "enabled": true},
                {"node_id": "n2", "node_type": "action", "label": "A", "position": {"x": 1.0, "y": 0.0}, "config": {}, "enabled": true},
                {"node_id": "n3", "node_type": "action", "label": "B", "position": {"x": 2.0, "y": 0.0}, "config": {}, "enabled": true}
            ],
            "connections": [
                {"from_node": "n1", "to_node": "n2", "condition": null, "label": null},
                {"from_node": "n2", "to_node": "n3", "condition": null, "label": null},
                {"from_node": "n3", "to_node": "n1", "condition": null, "label": null}
            ]
        }"#;
        let canvas = DesignCanvas::from_json(json).expect("from_json");
        let result = canvas.validate();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.error_type == ValidationErrorType::CircularDependency),
            "should detect circular dependency"
        );
    }

    #[test]
    fn validate_disconnected_node() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("a1", DesignNodeType::Action, "已连接"))
            .expect("add_node");
        // 孤立节点(无任何连接)。
        canvas
            .add_node(DesignNode::new("orphan", DesignNodeType::Action, "孤立"))
            .expect("add_node");
        canvas.connect("t1", "a1").expect("connect");

        let result = canvas.validate();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.error_type == ValidationErrorType::DisconnectedNode
                    && e.node_id.as_deref() == Some("orphan")),
            "should detect disconnected node 'orphan'"
        );
    }

    #[test]
    fn validate_duplicate_id() {
        // 通过 from_json 绕过 add_node 的去重检查。
        let json = r#"{
            "nodes": [
                {"node_id": "n1", "node_type": "trigger", "label": "T", "position": {"x": 0.0, "y": 0.0}, "config": {}, "enabled": true},
                {"node_id": "n1", "node_type": "action", "label": "A", "position": {"x": 1.0, "y": 0.0}, "config": {}, "enabled": true}
            ],
            "connections": []
        }"#;
        let canvas = DesignCanvas::from_json(json).expect("from_json");
        let result = canvas.validate();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.error_type == ValidationErrorType::DuplicateId),
            "should detect duplicate id"
        );
    }

    #[test]
    fn validate_valid_chain() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("a1", DesignNodeType::Action, "动作"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("o1", DesignNodeType::Output, "输出"))
            .expect("add_node");
        canvas.connect("t1", "a1").expect("connect");
        canvas.connect("a1", "o1").expect("connect");

        let result = canvas.validate();
        assert!(result.valid, "valid chain should pass: {:?}", result.errors);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn validate_missing_input() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        // Action 无入边 → MissingInput + DisconnectedNode。
        canvas
            .add_node(DesignNode::new("a1", DesignNodeType::Action, "孤立动作"))
            .expect("add_node");
        // 连接 t1 -> nothing,a1 无入边。
        let result = canvas.validate();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.error_type == ValidationErrorType::MissingInput
                    && e.node_id.as_deref() == Some("a1")),
            "should detect missing input for a1"
        );
    }

    // ---- to_execution_plan 顺序 ----

    #[test]
    fn to_execution_plan_order() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("a1", DesignNodeType::Action, "动作一"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("a2", DesignNodeType::Action, "动作二"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("o1", DesignNodeType::Output, "输出"))
            .expect("add_node");
        canvas.connect("t1", "a1").expect("connect");
        canvas.connect("a1", "a2").expect("connect");
        canvas.connect("a2", "o1").expect("connect");

        let plan = canvas.to_execution_plan().expect("execution plan");
        assert_eq!(plan.len(), 4);
        // 步骤序号连续 1..4。
        assert_eq!(plan[0].step, 1);
        assert_eq!(plan[1].step, 2);
        assert_eq!(plan[2].step, 3);
        assert_eq!(plan[3].step, 4);
        // 拓扑顺序:trigger -> action -> action -> output。
        assert_eq!(plan[0].node_id, "t1");
        assert_eq!(plan[0].action_type, "trigger");
        assert_eq!(plan[1].node_id, "a1");
        assert_eq!(plan[1].action_type, "action");
        assert_eq!(plan[2].node_id, "a2");
        assert_eq!(plan[3].node_id, "o1");
        assert_eq!(plan[3].action_type, "output");
    }

    #[test]
    fn to_execution_plan_with_config() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(node_with_config(
                "a1",
                DesignNodeType::Action,
                "打开应用",
                0.0,
                0.0,
                config(vec![
                    ("command", serde_json::json!("open_app")),
                    ("args", serde_json::json!(["notepad.exe", "--maximized"])),
                ]),
            ))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("o1", DesignNodeType::Output, "完成"))
            .expect("add_node");
        canvas.connect("t1", "a1").expect("connect");
        canvas.connect("a1", "o1").expect("connect");

        let plan = canvas.to_execution_plan().expect("plan");
        // a1 是 step 2(command/args 从 config 派生)。
        assert_eq!(plan[1].node_id, "a1");
        assert_eq!(plan[1].command, "open_app");
        assert_eq!(plan[1].args, vec!["notepad.exe", "--maximized"]);
    }

    #[test]
    fn to_execution_plan_invalid_returns_err() {
        // 孤立节点 → 验证失败 → Err。
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("orphan", DesignNodeType::Action, "孤立"))
            .expect("add_node");
        let err = canvas.to_execution_plan().unwrap_err().to_string();
        assert!(err.contains("invalid"), "unexpected err: {}", err);
    }

    // ---- to_json / from_json 往返 ----

    #[test]
    fn to_json_from_json_roundtrip() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("a1", DesignNodeType::Action, "动作"))
            .expect("add_node");
        canvas.connect("t1", "a1").expect("connect");

        let json = canvas.to_json().expect("to_json");
        assert!(json.contains("\"nodes\""));
        assert!(json.contains("\"connections\""));
        assert!(json.contains("\"t1\""));
        assert!(json.contains("\"trigger\""));

        let back = DesignCanvas::from_json(&json).expect("from_json");
        assert_eq!(canvas, back);
    }

    #[test]
    fn from_json_invalid_returns_err() {
        let result = DesignCanvas::from_json("not valid json {");
        assert!(result.is_err());
    }

    // ---- DesignNodeType 所有变体 serde ----

    #[test]
    fn design_node_type_all_variants_serde() {
        let variants = vec![
            (DesignNodeType::Trigger, "trigger"),
            (DesignNodeType::Condition, "condition"),
            (DesignNodeType::Action, "action"),
            (DesignNodeType::Delay, "delay"),
            (DesignNodeType::Loop, "loop"),
            (DesignNodeType::Branch, "branch"),
            (DesignNodeType::Output, "output"),
            (DesignNodeType::Input, "input"),
            (DesignNodeType::Variable, "variable"),
            (DesignNodeType::Comment, "comment"),
        ];
        for (variant, expected) in &variants {
            let json = serde_json::to_string(variant).expect("serialize");
            assert!(
                json.contains(&format!("\"{}\"", expected)),
                "expected snake_case \"{}\" in {}",
                expected,
                json
            );
            let back: DesignNodeType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(variant, &back);
            // type_name 一致。
            assert_eq!(variant.type_name(), *expected);
        }
    }

    // ---- DesignConnection 结构 serde ----

    #[test]
    fn design_connection_structure_serde() {
        // 无条件无标签。
        let c1 = DesignConnection::new("n1", "n2");
        let json = serde_json::to_string(&c1).expect("serialize");
        assert!(json.contains("\"from_node\":\"n1\""));
        assert!(json.contains("\"to_node\":\"n2\""));
        assert!(json.contains("\"condition\":null"));
        assert!(json.contains("\"label\":null"));
        let back: DesignConnection = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(c1, back);

        // 带条件与标签。
        let c2 = DesignConnection {
            from_node: "a".into(),
            to_node: "b".into(),
            condition: Some("x > 0".into()),
            label: Some("正向分支".into()),
        };
        let json2 = serde_json::to_string(&c2).expect("serialize");
        assert!(json2.contains("\"condition\":\"x > 0\""));
        assert!(json2.contains("\"label\":\"正向分支\""));
        let back2: DesignConnection = serde_json::from_str(&json2).expect("deserialize");
        assert_eq!(c2, back2);
    }

    // ---- ValidationResult errors / warnings ----

    #[test]
    fn validation_result_errors_and_warnings() {
        let mut result = ValidationResult::new();
        // 初始 valid=false(Default)。
        assert!(!result.valid);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());

        // 添加错误 → valid=false。
        result.add_error(
            Some("n1".into()),
            ValidationErrorType::MissingInput,
            "missing input",
        );
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].node_id.as_deref(), Some("n1"));
        assert_eq!(
            result.errors[0].error_type,
            ValidationErrorType::MissingInput
        );

        // 添加警告 → 不影响 valid。
        result.add_warning(Some("n2".into()), "deprecated node type");
        assert!(!result.valid);
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].node_id.as_deref(), Some("n2"));

        // 空结果的 validate 自然 valid=true。
        let empty = DesignCanvas::new().validate();
        assert!(empty.valid);
        assert!(empty.errors.is_empty());

        // 序列化往返。
        let json = serde_json::to_string(&result).expect("serialize");
        let back: ValidationResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result, back);
    }

    // ---- DesignTemplateLibrary list / get / instantiate ----

    #[test]
    fn template_library_list() {
        let lib = DesignTemplateLibrary::new();
        let list = lib.list();
        assert!(!list.is_empty());
        // 每个模板的 template_id 非空且唯一。
        let mut ids: Vec<&str> = list.iter().map(|t| t.template_id.as_str()).collect();
        let len_before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "template ids should be unique");
    }

    #[test]
    fn template_library_get() {
        let lib = DesignTemplateLibrary::new();
        // 已知模板。
        let t = lib.get("data_entry_automation").expect("template exists");
        assert_eq!(t.template_id, "data_entry_automation");
        assert!(!t.name.is_empty());
        assert!(!t.description.is_empty());
        assert!(!t.category.is_empty());

        // 不存在的模板。
        assert!(lib.get("nonexistent").is_none());
    }

    #[test]
    fn template_library_instantiate() {
        let lib = DesignTemplateLibrary::new();
        let canvas = lib.instantiate("file_processing").expect("instantiate");
        assert!(!canvas.nodes().is_empty());
        assert!(!canvas.connections().is_empty());
        // 实例化的画布应是有效的。
        let validation = canvas.validate();
        assert!(
            validation.valid,
            "template canvas should be valid: {:?}",
            validation.errors
        );

        // 不存在的模板 → Err。
        let err = lib.instantiate("nope").unwrap_err().to_string();
        assert!(err.contains("not found"), "unexpected err: {}", err);
    }

    // ---- 内置模板数量 ----

    #[test]
    fn built_in_templates_count() {
        let lib = DesignTemplateLibrary::new();
        assert!(
            lib.list().len() >= 6,
            "should have at least 6 built-in templates, got {}",
            lib.list().len()
        );
        // 验证所有 6 个已知模板都存在。
        let expected_ids = [
            "data_entry_automation",
            "file_processing",
            "web_scraping",
            "report_generation",
            "batch_rename",
            "screenshot_workflow",
        ];
        for id in &expected_ids {
            assert!(lib.get(id).is_some(), "template '{}' should exist", id);
        }
    }

    #[test]
    fn all_templates_produce_valid_execution_plan() {
        let lib = DesignTemplateLibrary::new();
        for tmpl in lib.list() {
            let canvas = lib.instantiate(&tmpl.template_id).expect("instantiate");
            let validation = canvas.validate();
            assert!(
                validation.valid,
                "template '{}' should be valid: {:?}",
                tmpl.template_id, validation.errors
            );
            let plan = canvas
                .to_execution_plan()
                .unwrap_or_else(|e| panic!("template '{}' plan failed: {}", tmpl.template_id, e));
            assert!(
                !plan.is_empty(),
                "template '{}' should produce non-empty plan",
                tmpl.template_id
            );
            // step 从 1 开始连续。
            for (i, a) in plan.iter().enumerate() {
                assert_eq!(
                    a.step,
                    (i + 1) as u32,
                    "step numbering mismatch in template {}",
                    tmpl.template_id
                );
            }
        }
    }

    // ---- DesignNode serde 往返 ----

    #[test]
    fn design_node_serde_roundtrip() {
        let mut config = HashMap::new();
        config.insert("command".to_string(), serde_json::json!("click"));
        config.insert("args".to_string(), serde_json::json!(["#btn", "twice"]));
        let n = DesignNode {
            node_id: "n1".into(),
            node_type: DesignNodeType::Action,
            label: "点击按钮".into(),
            position: NodePosition::new(128.5, 256.0),
            config,
            enabled: true,
        };
        let json = serde_json::to_string(&n).expect("serialize");
        let back: DesignNode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(n, back);
        // 禁用节点也往返。
        let n2 = DesignNode {
            node_id: "n2".into(),
            node_type: DesignNodeType::Comment,
            label: "注释".into(),
            position: NodePosition::new(0.0, 0.0),
            config: HashMap::new(),
            enabled: false,
        };
        let json2 = serde_json::to_string(&n2).expect("serialize");
        let back2: DesignNode = serde_json::from_str(&json2).expect("deserialize");
        assert_eq!(n2, back2);
    }

    // ---- to_execution_plan 跳过禁用节点与注释节点 ----

    #[test]
    fn to_execution_plan_skips_disabled_and_comment() {
        let mut canvas = DesignCanvas::new();
        canvas
            .add_node(DesignNode::new("t1", DesignNodeType::Trigger, "启动"))
            .expect("add_node");
        // 禁用的 Action(仍参与拓扑排序但不在计划中)。
        let mut disabled = DesignNode::new("d1", DesignNodeType::Action, "禁用");
        disabled.enabled = false;
        canvas.add_node(disabled).expect("add_node");
        // 注释节点(连接在链中,但不出现在计划中)。
        canvas
            .add_node(DesignNode::new("c1", DesignNodeType::Comment, "注释"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("a1", DesignNodeType::Action, "动作"))
            .expect("add_node");
        canvas
            .add_node(DesignNode::new("o1", DesignNodeType::Output, "输出"))
            .expect("add_node");

        // t1 -> d1(disabled) -> c1(comment) -> a1 -> o1
        canvas.connect("t1", "d1").expect("connect");
        canvas.connect("d1", "c1").expect("connect");
        canvas.connect("c1", "a1").expect("connect");
        canvas.connect("a1", "o1").expect("connect");

        let plan = canvas.to_execution_plan().expect("plan");
        // 仅 t1 / a1 / o1 出现(d1 禁用,c1 注释)。
        let ids: Vec<&str> = plan.iter().map(|a| a.node_id.as_str()).collect();
        assert_eq!(ids, vec!["t1", "a1", "o1"]);
        // step 连续 1,2,3。
        assert_eq!(plan[0].step, 1);
        assert_eq!(plan[1].step, 2);
        assert_eq!(plan[2].step, 3);
    }
}
