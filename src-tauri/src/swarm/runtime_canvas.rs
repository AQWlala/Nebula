//! T-E-S-11: 蜂群运行时画布 — 实时蜂群 agent 运行状态可视化数据流。
//!
//! 本模块提供蜂群运行时的全景快照（[`RuntimeCanvasSnapshot`]），供前端
//! 画布组件实时渲染 agent 节点、任务进度和 agent 间的连接关系。
//!
//! ## 核心设计
//!
//! * [`RuntimeCanvasCollector`] 从 [`SwarmOrchestrator`] 的公共 API 收集
//!   agent 列表与 Leader 信息，同时通过订阅 [`SwarmEvent`] 事件流追踪
//!   任务生命周期（启动/完成/协商），生成结构化快照。
//! * 快照通过 `tokio::sync::broadcast` 通道推送给所有订阅者，前端通过
//!   Tauri IPC Channel 接收并增量渲染画布。
//! * [`RuntimeCanvasCollector::start_periodic_collect`] 启动后台定时循环，
//!   周期性收集 + 广播快照，同时消费事件流更新内部追踪状态。
//!
//! ## 数据流
//!
//! ```text
//! SwarmOrchestrator ──list_agents()──┐
//!                   ──bus().subscribe_events()──┤
//!                                                ▼
//!                                    RuntimeCanvasCollector
//!                                          │
//!                          collect() + ingest_event()
//!                                          │
//!                                          ▼
//!                                 RuntimeCanvasSnapshot
//!                                          │
//!                                    broadcast::send
//!                                          │
//!                          ┌───────────────┼───────────────┐
//!                          ▼               ▼               ▼
//!                     Subscriber 1   Subscriber 2   Subscriber N
//!                     (前端画布)      (日志记录)     (指标采集)
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, info, instrument, warn};

use super::agents::AgentKind;
use super::events::SwarmEvent;
use super::orchestrator::SwarmOrchestrator;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// broadcast 通道容量 — 足够容纳若干轮快照，避免订阅者偶发 lag 时丢失数据。
const BROADCAST_CAPACITY: usize = 64;

// ---------------------------------------------------------------------------
// 辅助默认值函数（供 #[serde(default = "...")] 使用）
// ---------------------------------------------------------------------------

/// `AgentKind` 的默认值（`Generic`）。
fn default_agent_kind() -> AgentKind {
    AgentKind::Generic
}

/// `AgentStatus` 的默认值（`Idle`）。
fn default_agent_status() -> AgentStatus {
    AgentStatus::Idle
}

/// `TaskStatus` 的默认值（`Pending`）。
fn default_task_status() -> TaskStatus {
    TaskStatus::Pending
}

/// `ConnectionType` 的默认值（`Delegation`）。
fn default_connection_type() -> ConnectionType {
    ConnectionType::Delegation
}

// ---------------------------------------------------------------------------
// 枚举
// ---------------------------------------------------------------------------

/// Agent 运行时状态枚举 — 描述 agent 在画布上的视觉状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// 空闲 — 等待任务分配。
    Idle,
    /// 运行中 — 正在执行任务。
    Running,
    /// 等待中 — 等待协商/仲裁结果或其他 agent 完成。
    Waiting,
    /// 已完成 — 任务已成功结束。
    Completed,
}

/// 任务运行时状态枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 待执行。
    Pending,
    /// 执行中。
    Running,
    /// 已完成。
    Completed,
    /// 已失败。
    Failed,
    /// 已取消。
    Cancelled,
}

/// Agent 之间连接的类型 — 描述画布上连线的语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    /// 委派 — 上游 agent 将子任务委派给下游 agent。
    Delegation,
    /// 结果传递 — 上游 agent 的输出作为下游 agent 的输入。
    Result,
    /// 上下文共享 — agent 间共享团队上下文（非直接数据流）。
    Context,
}

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 画布元数据 — 快照级别的全局信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasMetadata {
    /// 画布会话 ID（标识当前蜂群会话）。
    #[serde(default)]
    pub session_id: String,
    /// 当前 Leader agent 名称（由 LeaderElector 选举产生）。
    #[serde(default)]
    pub current_leader: Option<String>,
    /// 画布上 agent 节点总数。
    #[serde(default)]
    pub total_agents: usize,
    /// 画布上活跃任务总数。
    #[serde(default)]
    pub total_tasks: usize,
}

impl Default for CanvasMetadata {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            current_leader: None,
            total_agents: 0,
            total_tasks: 0,
        }
    }
}

/// Agent 运行时状态 — 画布上单个 agent 节点的完整描述。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimeState {
    /// Agent 唯一标识（agent 名称）。
    pub agent_id: String,
    /// Agent 角色（`AgentKind`，描述 agent 实现类型）。
    #[serde(default = "default_agent_kind")]
    pub role: AgentKind,
    /// 当前运行状态。
    #[serde(default = "default_agent_status")]
    pub status: AgentStatus,
    /// 当前正在执行的任务 ID（空闲时为 None）。
    #[serde(default)]
    pub current_task_id: Option<String>,
    /// CPU 使用率（0.0-1.0，由系统监控填充，默认 0.0）。
    #[serde(default)]
    pub cpu_usage: f64,
    /// 内存使用率（0.0-1.0，由系统监控填充，默认 0.0）。
    #[serde(default)]
    pub memory_usage: f64,
    /// 最近心跳时间戳（Unix 毫秒）。
    #[serde(default)]
    pub last_heartbeat: i64,
}

impl AgentRuntimeState {
    /// 创建一个新的 `AgentRuntimeState`，默认状态为 `Idle`。
    pub fn new(agent_id: impl Into<String>, role: AgentKind) -> Self {
        Self {
            agent_id: agent_id.into(),
            role,
            status: AgentStatus::Idle,
            current_task_id: None,
            cpu_usage: 0.0,
            memory_usage: 0.0,
            last_heartbeat: 0,
        }
    }

    /// Builder：设置状态。
    pub fn with_status(mut self, status: AgentStatus) -> Self {
        self.status = status;
        self
    }

    /// Builder：设置当前任务 ID。
    pub fn with_current_task(mut self, task_id: impl Into<String>) -> Self {
        self.current_task_id = Some(task_id.into());
        self
    }

    /// Builder：设置资源使用率。
    pub fn with_usage(mut self, cpu: f64, memory: f64) -> Self {
        self.cpu_usage = cpu.clamp(0.0, 1.0);
        self.memory_usage = memory.clamp(0.0, 1.0);
        self
    }

    /// 状态转换：标记为运行中并关联任务。
    pub fn transition_to_running(&mut self, task_id: impl Into<String>) {
        self.status = AgentStatus::Running;
        self.current_task_id = Some(task_id.into());
    }

    /// 状态转换：标记为等待中（保留任务关联）。
    pub fn transition_to_waiting(&mut self) {
        if self.status == AgentStatus::Running {
            self.status = AgentStatus::Waiting;
        }
    }

    /// 状态转换：标记为已完成并清除任务关联。
    pub fn transition_to_completed(&mut self) {
        self.status = AgentStatus::Completed;
        self.current_task_id = None;
    }
}

/// 任务运行时状态 — 画布上单个任务节点的完整描述。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRuntimeState {
    /// 任务唯一标识。
    pub task_id: String,
    /// 父任务 ID（DAG 层级中上游任务，顶层任务为 None）。
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 被分配的 agent ID。
    #[serde(default)]
    pub assigned_agent_id: Option<String>,
    /// 任务状态。
    #[serde(default = "default_task_status")]
    pub status: TaskStatus,
    /// 完成进度（0.0-1.0）。
    #[serde(default)]
    pub progress: f32,
    /// 任务启动时间戳（Unix 毫秒）。
    #[serde(default)]
    pub started_at: i64,
    /// 预计完成时间戳（Unix 毫秒，None 表示未知）。
    #[serde(default)]
    pub eta: Option<i64>,
}

impl TaskRuntimeState {
    /// 创建一个新的 `TaskRuntimeState`，默认状态为 `Pending`。
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            parent_id: None,
            assigned_agent_id: None,
            status: TaskStatus::Pending,
            progress: 0.0,
            started_at: 0,
            eta: None,
        }
    }

    /// Builder：设置进度（自动 clamp 到 0.0-1.0）。
    pub fn with_progress(mut self, progress: f32) -> Self {
        self.progress = progress.clamp(0.0, 1.0);
        self
    }

    /// Builder：设置状态。
    pub fn with_status(mut self, status: TaskStatus) -> Self {
        self.status = status;
        self
    }

    /// Builder：设置启动时间。
    pub fn with_started_at(mut self, started_at: i64) -> Self {
        self.started_at = started_at;
        self
    }

    /// Builder：设置分配的 agent。
    pub fn with_assigned_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.assigned_agent_id = Some(agent_id.into());
        self
    }

    /// 更新进度（自动 clamp 到 0.0-1.0）。
    pub fn update_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }
}

/// Agent 之间的连接 — 画布上节点间的有向边。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    /// 源 agent ID。
    pub from_agent_id: String,
    /// 目标 agent ID。
    pub to_agent_id: String,
    /// 连接标签（画布上显示的文本）。
    #[serde(default)]
    pub label: String,
    /// 连接类型。
    #[serde(default = "default_connection_type")]
    pub connection_type: ConnectionType,
}

impl Connection {
    /// 创建一个新的 `Connection`。
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        connection_type: ConnectionType,
    ) -> Self {
        Self {
            from_agent_id: from.into(),
            to_agent_id: to.into(),
            label: String::new(),
            connection_type,
        }
    }

    /// Builder：设置标签。
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

/// 运行时画布快照 — 某一时刻蜂群的全景状态。
///
/// 由 [`RuntimeCanvasCollector::collect`] 生成，通过 `broadcast` 通道
/// 推送给所有订阅者。前端收到后增量更新画布渲染。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCanvasSnapshot {
    /// 快照时间戳（Unix 毫秒）。
    pub timestamp: i64,
    /// 所有 agent 的运行时状态。
    #[serde(default)]
    pub agents: Vec<AgentRuntimeState>,
    /// 活跃任务的运行时状态。
    #[serde(default)]
    pub active_tasks: Vec<TaskRuntimeState>,
    /// Agent 之间的连接。
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// 画布元数据。
    #[serde(default)]
    pub canvas_metadata: CanvasMetadata,
}

impl RuntimeCanvasSnapshot {
    /// 创建一个空快照（仅含时间戳）。
    pub fn empty() -> Self {
        Self {
            timestamp: chrono::Utc::now().timestamp_millis(),
            agents: Vec::new(),
            active_tasks: Vec::new(),
            connections: Vec::new(),
            canvas_metadata: CanvasMetadata::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// 内部追踪状态
// ---------------------------------------------------------------------------

/// 收集器内部追踪状态 — 由 [`SwarmEvent`] 事件流驱动更新。
///
/// `collect()` 时将此状态与 [`SwarmOrchestrator`] 的公共 API 数据合并，
/// 生成完整的 [`RuntimeCanvasSnapshot`]。
#[derive(Debug, Clone, Default)]
struct TrackedState {
    /// agent_id → 运行状态。
    agent_status: HashMap<String, AgentStatus>,
    /// agent_id → 当前关联的 task_id。
    agent_current_task: HashMap<String, String>,
    /// agent_id → 最近心跳时间戳。
    agent_heartbeats: HashMap<String, i64>,
    /// task_id → 任务运行时状态（含已完成但未清理的）。
    active_tasks: HashMap<String, TaskRuntimeState>,
    /// Agent 之间的连接（由委派/结果事件动态构建）。
    connections: Vec<Connection>,
}

// ---------------------------------------------------------------------------
// RuntimeCanvasCollector
// ---------------------------------------------------------------------------

/// 蜂群运行时画布收集器 — 从 [`SwarmOrchestrator`] 收集实时状态并生成快照。
///
/// ## 使用方式
///
/// ```rust,ignore
/// let collector = Arc::new(RuntimeCanvasCollector::new());
/// let rx = collector.subscribe();
/// let handle = collector.clone().start_periodic_collect(
///     orchestrator.clone(),
///     Duration::from_millis(500),
/// );
/// // rx 收到快照后渲染画布…
/// ```
pub struct RuntimeCanvasCollector {
    /// 广播发送端 — 向所有订阅者推送快照。
    sender: broadcast::Sender<RuntimeCanvasSnapshot>,
    /// 内部追踪状态（由事件驱动更新，collect 时读取）。
    state: RwLock<TrackedState>,
}

impl RuntimeCanvasCollector {
    /// 创建一个新的收集器，内部初始化 broadcast 通道。
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            sender,
            state: RwLock::new(TrackedState::default()),
        }
    }

    /// 从 [`SwarmOrchestrator`] 收集当前状态，生成 [`RuntimeCanvasSnapshot`]。
    ///
    /// 合并两个数据源：
    /// 1. **Orchestrator 公共 API**：`list_agents()` 提供 agent 列表，
    ///    `leader_elector()` 提供 Leader 信息。
    /// 2. **内部追踪状态**：由 [`ingest_event`](Self::ingest_event) 维护的
    ///    agent 状态、活跃任务和连接。
    #[instrument(
        target = "nebula.swarm.canvas",
        skip(self, orchestrator),
        fields(otel.kind = "swarm_canvas")
    )]
    pub fn collect(&self, orchestrator: &SwarmOrchestrator) -> RuntimeCanvasSnapshot {
        let now = chrono::Utc::now().timestamp_millis();
        let tracked = self.state.read().clone();

        // 从 orchestrator 公共 API 获取 agent 列表与 Leader 信息。
        let agent_list = orchestrator.list_agents();
        let leader = orchestrator.leader_elector().current_leader();

        let agents: Vec<AgentRuntimeState> = agent_list
            .iter()
            .map(|(kind_str, name, _prompt, _desc)| {
                let role = kind_str.parse::<AgentKind>().unwrap_or(AgentKind::Generic);
                let status = tracked
                    .agent_status
                    .get(name)
                    .copied()
                    .unwrap_or(AgentStatus::Idle);
                let current_task_id = tracked.agent_current_task.get(name).cloned();
                let last_heartbeat = tracked.agent_heartbeats.get(name).copied().unwrap_or(now);

                AgentRuntimeState {
                    agent_id: name.clone(),
                    role,
                    status,
                    current_task_id,
                    cpu_usage: 0.0,
                    memory_usage: 0.0,
                    last_heartbeat,
                }
            })
            .collect();

        // 从追踪状态提取活跃任务（仅 Running / Pending）。
        let active_tasks: Vec<TaskRuntimeState> = tracked
            .active_tasks
            .values()
            .filter(|t| t.status == TaskStatus::Running || t.status == TaskStatus::Pending)
            .cloned()
            .collect();

        let connections = tracked.connections.clone();

        let canvas_metadata = CanvasMetadata {
            session_id: format!("canvas-{now}"),
            current_leader: leader,
            total_agents: agents.len(),
            total_tasks: active_tasks.len(),
        };

        debug!(
            target: "nebula.swarm.canvas",
            agents = agents.len(),
            active_tasks = active_tasks.len(),
            connections = connections.len(),
            "canvas snapshot collected"
        );

        RuntimeCanvasSnapshot {
            timestamp: now,
            agents,
            active_tasks,
            connections,
            canvas_metadata,
        }
    }

    /// 订阅实时快照更新，返回 `broadcast::Receiver`。
    ///
    /// 调用方应在 `tokio::select!` 中处理 [`broadcast::error::RecvError::Lagged`]
    /// （表示消费过慢，跳过了部分快照）。
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeCanvasSnapshot> {
        self.sender.subscribe()
    }

    /// 将快照推送给所有活跃订阅者。
    ///
    /// 无订阅者时返回 `Ok(())`（非错误，仅记录 debug 日志）。
    #[instrument(target = "nebula.swarm.canvas", skip(self, snapshot))]
    pub fn publish(&self, snapshot: RuntimeCanvasSnapshot) -> Result<()> {
        match self.sender.send(snapshot) {
            Ok(n) => {
                debug!(
                    target: "nebula.swarm.canvas",
                    subscribers = n,
                    "canvas snapshot published"
                );
                Ok(())
            }
            Err(_) => {
                // 无活跃订阅者 — 非错误，仅记录。
                debug!(
                    target: "nebula.swarm.canvas",
                    "no active subscribers for canvas snapshot"
                );
                Ok(())
            }
        }
    }

    /// 摄入一个 [`SwarmEvent`]，更新内部追踪状态。
    ///
    /// 由 [`start_periodic_collect`](Self::start_periodic_collect) 内部调用，
    /// 也可由外部事件消费者手动调用。
    #[instrument(target = "nebula.swarm.canvas", skip(self, event))]
    pub fn ingest_event(&self, event: &SwarmEvent) -> Result<()> {
        let mut state = self.state.write();
        let now = chrono::Utc::now().timestamp_millis();

        match event {
            SwarmEvent::AgentStarted {
                agent_kind: _,
                task_id,
                timestamp,
            } => {
                // 创建或更新任务状态为 Running。
                state
                    .active_tasks
                    .entry(task_id.clone())
                    .or_insert_with(|| TaskRuntimeState {
                        task_id: task_id.clone(),
                        parent_id: None,
                        assigned_agent_id: None,
                        status: TaskStatus::Running,
                        progress: 0.0,
                        started_at: *timestamp,
                        eta: None,
                    });
                debug!(
                    target: "nebula.swarm.canvas",
                    task_id = %task_id,
                    "agent_started event ingested"
                );
            }
            SwarmEvent::AgentCompleted {
                agent_kind: _,
                task_id,
                success,
                ..
            } => {
                if let Some(task) = state.active_tasks.get_mut(task_id) {
                    task.progress = if *success { 1.0 } else { 0.0 };
                }
                debug!(
                    target: "nebula.swarm.canvas",
                    task_id = %task_id,
                    success = success,
                    "agent_completed event ingested"
                );
            }
            SwarmEvent::SwarmCompleted {
                task_id, approved, ..
            } => {
                if let Some(task) = state.active_tasks.get_mut(task_id) {
                    task.status = if *approved {
                        TaskStatus::Completed
                    } else {
                        TaskStatus::Failed
                    };
                    task.progress = 1.0;
                }
                // 已完成任务从活跃列表移除（collect 时已过滤，此处显式移除避免累积）。
                if state.active_tasks.get(task_id).is_some_and(|t| {
                    t.status == TaskStatus::Completed || t.status == TaskStatus::Failed
                }) {
                    state.active_tasks.remove(task_id);
                }
                info!(
                    target: "nebula.swarm.canvas",
                    task_id = %task_id,
                    approved = approved,
                    "swarm_completed event ingested"
                );
            }
            SwarmEvent::NegotiationStarted {
                task_id,
                candidate_count,
                ..
            } => {
                // 协商阶段：所有 Running 的 agent 转为 Waiting。
                for status in state.agent_status.values_mut() {
                    if *status == AgentStatus::Running {
                        *status = AgentStatus::Waiting;
                    }
                }
                debug!(
                    target: "nebula.swarm.canvas",
                    task_id = %task_id,
                    candidates = candidate_count,
                    "negotiation_started event ingested"
                );
            }
            SwarmEvent::AgentToolCall {
                agent_id, task_id, ..
            } => {
                // 工具调用：记录 agent 与 task 的关联 + 心跳。
                state
                    .agent_current_task
                    .insert(agent_id.clone(), task_id.clone());
                state
                    .agent_status
                    .insert(agent_id.clone(), AgentStatus::Running);
                state.agent_heartbeats.insert(agent_id.clone(), now);
            }
            SwarmEvent::AgentOutputChunk {
                agent_id, task_id, ..
            } => {
                // 输出块：更新心跳与任务关联。
                state.agent_heartbeats.insert(agent_id.clone(), now);
                state
                    .agent_current_task
                    .insert(agent_id.clone(), task_id.clone());
            }
            _ => {
                // 其他事件类型（DeadlockDetected / TreeOfThoughtsStarted / PathCompleted）
                // 暂不更新画布追踪状态。
            }
        }

        Ok(())
    }

    /// 手动记录 agent 状态（供外部系统或测试注入）。
    pub fn record_agent_status(&self, agent_id: &str, status: AgentStatus) {
        let mut state = self.state.write();
        state.agent_status.insert(agent_id.to_string(), status);
        state
            .agent_heartbeats
            .insert(agent_id.to_string(), chrono::Utc::now().timestamp_millis());
    }

    /// 手动添加 agent 间连接（供外部系统或测试注入）。
    pub fn record_connection(&self, connection: Connection) {
        let mut state = self.state.write();
        state.connections.push(connection);
    }

    /// 启动定时收集循环 — 周期性收集快照并广播，同时消费事件流更新追踪状态。
    ///
    /// 返回 `JoinHandle` 供调用方管理生命周期（abort / 等待结束）。
    /// 当事件总线关闭（所有 sender drop）时循环自动退出。
    #[instrument(
        target = "nebula.swarm.canvas",
        skip(self, orchestrator),
        fields(interval_ms = interval.as_millis() as u64)
    )]
    pub fn start_periodic_collect(
        self: Arc<Self>,
        orchestrator: Arc<SwarmOrchestrator>,
        interval: Duration,
    ) -> JoinHandle<()> {
        info!(
            target: "nebula.swarm.canvas",
            interval_ms = interval.as_millis() as u64,
            "starting periodic canvas collection loop"
        );

        tokio::spawn(async move {
            // 订阅事件流以驱动内部追踪状态更新。
            let mut event_rx = orchestrator.bus().subscribe_events();
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    // 定时收集 + 广播快照。
                    _ = ticker.tick() => {
                        let snapshot = self.collect(orchestrator.as_ref());
                        if let Err(e) = self.publish(snapshot) {
                            warn!(
                                target: "nebula.swarm.canvas",
                                error = %e,
                                "failed to publish canvas snapshot"
                            );
                        }
                    }
                    // 消费事件流更新追踪状态。
                    recv_result = event_rx.recv() => {
                        match recv_result {
                            Ok(event) => {
                                if let Err(e) = self.ingest_event(&event) {
                                    debug!(
                                        target: "nebula.swarm.canvas",
                                        error = %e,
                                        "failed to ingest swarm event"
                                    );
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(
                                    target: "nebula.swarm.canvas",
                                    lagged = n,
                                    "canvas event receiver lagged, some events skipped"
                                );
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                info!(
                                    target: "nebula.swarm.canvas",
                                    "event bus closed, stopping periodic collection loop"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        })
    }
}

impl Default for RuntimeCanvasCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // 数据结构序列化/反序列化测试
    // ===================================================================

    /// 测试 `RuntimeCanvasSnapshot` 的 JSON 序列化/反序列化 round-trip。
    #[test]
    fn test_snapshot_serde_round_trip() {
        let snapshot = RuntimeCanvasSnapshot {
            timestamp: 1_700_000_000_000,
            agents: vec![AgentRuntimeState::new("agent-1", AgentKind::Generic)
                .with_status(AgentStatus::Running)
                .with_current_task("task-1")
                .with_usage(0.3, 0.5)],
            active_tasks: vec![TaskRuntimeState::new("task-1")
                .with_status(TaskStatus::Running)
                .with_progress(0.5)
                .with_started_at(1_700_000_000_000)
                .with_assigned_agent("agent-1")],
            connections: vec![
                Connection::new("agent-1", "agent-2", ConnectionType::Delegation)
                    .with_label("delegate subtask"),
            ],
            canvas_metadata: CanvasMetadata {
                session_id: "canvas-1700000000000".to_string(),
                current_leader: Some("agent-1".to_string()),
                total_agents: 1,
                total_tasks: 1,
            },
        };

        let json = serde_json::to_string(&snapshot).expect("serialize should succeed");
        let de: RuntimeCanvasSnapshot =
            serde_json::from_str(&json).expect("deserialize should succeed");

        assert_eq!(de.timestamp, 1_700_000_000_000);
        assert_eq!(de.agents.len(), 1);
        assert_eq!(de.agents[0].agent_id, "agent-1");
        assert_eq!(de.agents[0].status, AgentStatus::Running);
        assert_eq!(de.agents[0].current_task_id.as_deref(), Some("task-1"));
        assert!((de.agents[0].cpu_usage - 0.3).abs() < f64::EPSILON);

        assert_eq!(de.active_tasks.len(), 1);
        assert_eq!(de.active_tasks[0].task_id, "task-1");
        assert_eq!(de.active_tasks[0].status, TaskStatus::Running);
        assert!((de.active_tasks[0].progress - 0.5).abs() < f32::EPSILON);

        assert_eq!(de.connections.len(), 1);
        assert_eq!(de.connections[0].from_agent_id, "agent-1");
        assert_eq!(de.connections[0].to_agent_id, "agent-2");
        assert_eq!(
            de.connections[0].connection_type,
            ConnectionType::Delegation
        );

        assert_eq!(
            de.canvas_metadata.current_leader.as_deref(),
            Some("agent-1")
        );
        assert_eq!(de.canvas_metadata.total_agents, 1);
    }

    /// 测试 `AgentStatus` 所有变体的序列化（snake_case）。
    #[test]
    fn test_agent_status_serde_variants() {
        let cases = [
            (AgentStatus::Idle, "idle"),
            (AgentStatus::Running, "running"),
            (AgentStatus::Waiting, "waiting"),
            (AgentStatus::Completed, "completed"),
        ];
        for (status, expected) in cases {
            let s = serde_json::to_string(&status).expect("serialize should succeed");
            assert!(s.contains(expected), "expected {expected} in {s}");
            let de: AgentStatus = serde_json::from_str(&s).expect("deserialize should succeed");
            assert_eq!(de, status);
        }
    }

    /// 测试 `ConnectionType` 所有变体的序列化（snake_case）。
    #[test]
    fn test_connection_type_serde_variants() {
        let cases = [
            (ConnectionType::Delegation, "delegation"),
            (ConnectionType::Result, "result"),
            (ConnectionType::Context, "context"),
        ];
        for (ct, expected) in cases {
            let s = serde_json::to_string(&ct).expect("serialize should succeed");
            assert!(s.contains(expected), "expected {expected} in {s}");
            let de: ConnectionType = serde_json::from_str(&s).expect("deserialize should succeed");
            assert_eq!(de, ct);
        }
    }

    /// 测试 `CanvasMetadata` 默认值。
    #[test]
    fn test_canvas_metadata_default() {
        let meta = CanvasMetadata::default();
        assert!(meta.session_id.is_empty());
        assert!(meta.current_leader.is_none());
        assert_eq!(meta.total_agents, 0);
        assert_eq!(meta.total_tasks, 0);
    }

    // ===================================================================
    // AgentRuntimeState 状态转换测试
    // ===================================================================

    /// 测试 `AgentRuntimeState` 的状态转换方法。
    #[test]
    fn test_agent_runtime_state_status_transitions() {
        let mut agent = AgentRuntimeState::new("agent-1", AgentKind::Generic);
        assert_eq!(agent.status, AgentStatus::Idle);
        assert!(agent.current_task_id.is_none());

        // Idle → Running
        agent.transition_to_running("task-1");
        assert_eq!(agent.status, AgentStatus::Running);
        assert_eq!(agent.current_task_id.as_deref(), Some("task-1"));

        // Running → Waiting
        agent.transition_to_waiting();
        assert_eq!(agent.status, AgentStatus::Waiting);
        // Waiting 仍保留任务关联
        assert_eq!(agent.current_task_id.as_deref(), Some("task-1"));

        // Waiting → Completed
        agent.transition_to_completed();
        assert_eq!(agent.status, AgentStatus::Completed);
        // Completed 清除任务关联
        assert!(agent.current_task_id.is_none());
    }

    /// 测试 `transition_to_waiting` 仅在 Running 时生效（非 Running 状态不变）。
    #[test]
    fn test_transition_to_waiting_only_from_running() {
        let mut agent = AgentRuntimeState::new("agent-1", AgentKind::Generic);
        agent.status = AgentStatus::Idle;
        agent.transition_to_waiting();
        assert_eq!(
            agent.status,
            AgentStatus::Idle,
            "Idle should not transition to Waiting"
        );

        agent.status = AgentStatus::Completed;
        agent.transition_to_waiting();
        assert_eq!(
            agent.status,
            AgentStatus::Completed,
            "Completed should not transition to Waiting"
        );
    }

    // ===================================================================
    // TaskRuntimeState 测试
    // ===================================================================

    /// 测试 `TaskRuntimeState` 的进度 clamp 行为。
    #[test]
    fn test_task_runtime_state_progress_clamp() {
        let mut task = TaskRuntimeState::new("task-1");
        assert!((task.progress - 0.0).abs() < f32::EPSILON);

        task.update_progress(0.5);
        assert!((task.progress - 0.5).abs() < f32::EPSILON);

        // 超出上限 clamp 到 1.0
        task.update_progress(1.5);
        assert!((task.progress - 1.0).abs() < f32::EPSILON);

        // 低于下限 clamp 到 0.0
        task.update_progress(-0.3);
        assert!((task.progress - 0.0).abs() < f32::EPSILON);
    }

    /// 测试 `with_progress` builder 的 clamp 行为。
    #[test]
    fn test_task_runtime_state_with_progress_builder() {
        let task = TaskRuntimeState::new("task-1").with_progress(2.0);
        assert!((task.progress - 1.0).abs() < f32::EPSILON);

        let task = TaskRuntimeState::new("task-2").with_progress(-1.0);
        assert!((task.progress - 0.0).abs() < f32::EPSILON);
    }

    // ===================================================================
    // Connection 测试
    // ===================================================================

    /// 测试 `Connection::new` + `with_label` builder。
    #[test]
    fn test_connection_builder() {
        let conn = Connection::new("a", "b", ConnectionType::Result).with_label("output");
        assert_eq!(conn.from_agent_id, "a");
        assert_eq!(conn.to_agent_id, "b");
        assert_eq!(conn.connection_type, ConnectionType::Result);
        assert_eq!(conn.label, "output");
    }

    // ===================================================================
    // RuntimeCanvasCollector 测试
    // ===================================================================

    /// 测试 `new()` 构造函数和 `subscribe()` — 返回可用的 Receiver。
    #[test]
    fn test_collector_new_and_subscribe() {
        let collector = RuntimeCanvasCollector::new();
        let rx = collector.subscribe();
        // Receiver 创建后应处于活跃状态（未关闭）。
        assert!(!rx.is_closed());
    }

    /// 测试 `publish()` 向订阅者推送快照。
    #[tokio::test]
    async fn test_collector_publish_and_receive() {
        let collector = RuntimeCanvasCollector::new();
        let mut rx = collector.subscribe();

        let snapshot = RuntimeCanvasSnapshot::empty();
        collector
            .publish(snapshot.clone())
            .expect("publish should succeed");

        let received = rx.recv().await.expect("should receive snapshot");
        assert_eq!(received.timestamp, snapshot.timestamp);
        assert!(received.agents.is_empty());
    }

    /// 测试 `publish()` 无订阅者时返回 Ok（非错误）。
    #[test]
    fn test_collector_publish_no_subscribers() {
        let collector = RuntimeCanvasCollector::new();
        // 无订阅者时 publish 应返回 Ok。
        let result = collector.publish(RuntimeCanvasSnapshot::empty());
        assert!(result.is_ok());
    }

    /// 测试 `ingest_event()` 摄入 `AgentStarted` 事件后追踪到任务。
    #[test]
    fn test_collector_ingest_agent_started_event() {
        let collector = RuntimeCanvasCollector::new();
        let event = SwarmEvent::agent_started(AgentKind::Generic, "task-ingest-1");
        collector
            .ingest_event(&event)
            .expect("ingest should succeed");

        // 验证内部状态：active_tasks 应包含 task-ingest-1。
        let state = collector.state.read();
        assert!(state.active_tasks.contains_key("task-ingest-1"));
        let task = state
            .active_tasks
            .get("task-ingest-1")
            .expect("task should exist");
        assert_eq!(task.status, TaskStatus::Running);
    }

    /// 测试 `ingest_event()` 摄入 `SwarmCompleted` 事件后任务被移除。
    #[test]
    fn test_collector_ingest_swarm_completed_removes_task() {
        let collector = RuntimeCanvasCollector::new();

        // 先摄入 AgentStarted 创建任务。
        let started = SwarmEvent::agent_started(AgentKind::Generic, "task-complete-1");
        collector
            .ingest_event(&started)
            .expect("ingest started should succeed");
        {
            let state = collector.state.read();
            assert!(state.active_tasks.contains_key("task-complete-1"));
        }

        // 摄入 SwarmCompleted 标记任务完成并移除。
        let completed = SwarmEvent::swarm_completed("task-complete-1", 1, 0, true);
        collector
            .ingest_event(&completed)
            .expect("ingest completed should succeed");
        {
            let state = collector.state.read();
            assert!(
                !state.active_tasks.contains_key("task-complete-1"),
                "completed task should be removed from active_tasks"
            );
        }
    }

    /// 测试 `ingest_event()` 摄入 `AgentToolCall` 事件后更新 agent 状态。
    #[test]
    fn test_collector_ingest_agent_tool_call_event() {
        let collector = RuntimeCanvasCollector::new();
        let start_ts = SwarmEvent::now_ts();
        let event = SwarmEvent::agent_tool_call(
            "agent-tc",
            "generic",
            "read_file",
            start_ts,
            start_ts + 100,
            100,
            true,
            Some("ok".to_string()),
            None,
            "task-tc",
        );
        collector
            .ingest_event(&event)
            .expect("ingest should succeed");

        let state = collector.state.read();
        // agent_status 应记录为 Running。
        assert_eq!(
            state.agent_status.get("agent-tc"),
            Some(&AgentStatus::Running)
        );
        // agent_current_task 应关联到 task-tc。
        assert_eq!(
            state.agent_current_task.get("agent-tc"),
            Some(&"task-tc".to_string())
        );
        // agent_heartbeats 应有记录。
        assert!(state.agent_heartbeats.contains_key("agent-tc"));
    }

    /// 测试 `ingest_event()` 摄入 `NegotiationStarted` 后 Running agent 转为 Waiting。
    #[test]
    fn test_collector_ingest_negotiation_started_transitions_to_waiting() {
        let collector = RuntimeCanvasCollector::new();

        // 手动设置两个 agent 为 Running。
        collector.record_agent_status("agent-a", AgentStatus::Running);
        collector.record_agent_status("agent-b", AgentStatus::Idle);

        // 摄入 NegotiationStarted。
        let event = SwarmEvent::negotiation_started("task-neg", 2);
        collector
            .ingest_event(&event)
            .expect("ingest should succeed");

        let state = collector.state.read();
        // agent-a (Running) → Waiting
        assert_eq!(
            state.agent_status.get("agent-a"),
            Some(&AgentStatus::Waiting)
        );
        // agent-b (Idle) 保持不变。
        assert_eq!(state.agent_status.get("agent-b"), Some(&AgentStatus::Idle));
    }

    /// 测试 `record_agent_status()` 和 `record_connection()` 手动注入。
    #[test]
    fn test_collector_record_agent_status_and_connection() {
        let collector = RuntimeCanvasCollector::new();
        collector.record_agent_status("agent-x", AgentStatus::Running);
        collector.record_connection(
            Connection::new("agent-x", "agent-y", ConnectionType::Context).with_label("shared ctx"),
        );

        let state = collector.state.read();
        assert_eq!(
            state.agent_status.get("agent-x"),
            Some(&AgentStatus::Running)
        );
        assert_eq!(state.connections.len(), 1);
        assert_eq!(
            state.connections[0].connection_type,
            ConnectionType::Context
        );
        assert_eq!(state.connections[0].label, "shared ctx");
    }

    // ===================================================================
    // collect() 与 SwarmOrchestrator 集成测试
    // ===================================================================

    /// 辅助：构造测试用 `SwarmOrchestrator`（LLM 端点不可达，仅用于 collect 测试）。
    fn make_orchestrator() -> Arc<SwarmOrchestrator> {
        use std::time::Duration;
        let client = Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_secs(2),
        ));
        let gw = Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        Arc::new(SwarmOrchestrator::new_without_memory(
            gw,
            Arc::new(crate::tools::ToolRegistry::new()),
        ))
    }

    /// 测试 `collect()` 从真实 `SwarmOrchestrator` 收集 agent 列表。
    #[tokio::test]
    async fn test_collector_collect_with_orchestrator() {
        let orch = make_orchestrator();
        let collector = RuntimeCanvasCollector::new();

        let snapshot = collector.collect(&orch);

        // 静态 agent 池有 6 个 agent。
        assert_eq!(
            snapshot.agents.len(),
            6,
            "static agent pool should have 6 agents"
        );
        // 每个 agent 应有非空 agent_id 和有效 role。
        for agent in &snapshot.agents {
            assert!(!agent.agent_id.is_empty(), "agent_id should not be empty");
            // 默认状态为 Idle（无事件摄入）。
            assert_eq!(agent.status, AgentStatus::Idle);
        }
        // 无活跃任务（无事件摄入）。
        assert!(snapshot.active_tasks.is_empty());
        // 画布元数据应反映 agent 数量。
        assert_eq!(snapshot.canvas_metadata.total_agents, 6);
        assert_eq!(snapshot.canvas_metadata.total_tasks, 0);
        // 时间戳应为正。
        assert!(snapshot.timestamp > 0);
    }

    /// 测试 `collect()` 合并追踪状态 — 摄入事件后 agent 状态应反映在快照中。
    #[tokio::test]
    async fn test_collector_collect_merges_tracked_state() {
        let orch = make_orchestrator();
        let collector = RuntimeCanvasCollector::new();

        // 获取第一个 agent 名称，手动设置其状态。
        let first_agent_name = orch.list_agents()[0].1.clone();
        collector.record_agent_status(&first_agent_name, AgentStatus::Running);

        let snapshot = collector.collect(&orch);

        // 找到该 agent，验证状态已合并。
        let target = snapshot
            .agents
            .iter()
            .find(|a| a.agent_id == first_agent_name)
            .expect("agent should be in snapshot");
        assert_eq!(target.status, AgentStatus::Running);
    }

    // ===================================================================
    // start_periodic_collect 测试
    // ===================================================================

    /// 测试 `start_periodic_collect()` 周期性广播快照。
    #[tokio::test]
    async fn test_collector_periodic_collect_broadcasts() {
        let orch = make_orchestrator();
        let collector = Arc::new(RuntimeCanvasCollector::new());
        let mut rx = collector.subscribe();

        // 启动定时收集，间隔 50ms。
        let handle = collector
            .clone()
            .start_periodic_collect(orch, Duration::from_millis(50));

        // 等待接收第一个快照（timeout 2s）。
        let snapshot = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("should receive snapshot within timeout")
            .expect("recv should succeed");

        // 验证快照内容。
        assert!(snapshot.timestamp > 0);
        assert_eq!(snapshot.agents.len(), 6);

        // 停止收集循环。
        handle.abort();
    }

    /// 测试 `start_periodic_collect()` 多次广播快照。
    #[tokio::test]
    async fn test_collector_periodic_collect_multiple_snapshots() {
        let orch = make_orchestrator();
        let collector = Arc::new(RuntimeCanvasCollector::new());
        let mut rx = collector.subscribe();

        let handle = collector
            .clone()
            .start_periodic_collect(orch, Duration::from_millis(30));

        // 接收 3 个快照。
        let mut count = 0;
        for _ in 0..3 {
            let snapshot = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("should receive snapshot within timeout")
                .expect("recv should succeed");
            assert_eq!(snapshot.agents.len(), 6);
            count += 1;
        }
        assert_eq!(count, 3, "should have received 3 snapshots");

        handle.abort();
    }

    // ===================================================================
    // 空快照与 Default 测试
    // ===================================================================

    /// 测试 `RuntimeCanvasSnapshot::empty()` 返回有效空快照。
    #[test]
    fn test_snapshot_empty() {
        let snapshot = RuntimeCanvasSnapshot::empty();
        assert!(snapshot.timestamp > 0);
        assert!(snapshot.agents.is_empty());
        assert!(snapshot.active_tasks.is_empty());
        assert!(snapshot.connections.is_empty());
        assert!(snapshot.canvas_metadata.session_id.is_empty());
    }

    /// 测试 `RuntimeCanvasCollector` 实现 `Default` trait。
    #[test]
    fn test_collector_default_impl() {
        let collector = RuntimeCanvasCollector::default();
        let rx = collector.subscribe();
        assert!(!rx.is_closed());
    }

    /// 测试快照反序列化时缺失字段使用默认值（向后兼容）。
    #[test]
    fn test_snapshot_deserialize_with_missing_fields() {
        // 仅含 timestamp 的最小 JSON，其他字段应回退到默认值。
        let json = r#"{"timestamp": 12345}"#;
        let de: RuntimeCanvasSnapshot =
            serde_json::from_str(json).expect("deserialize with missing fields should succeed");
        assert_eq!(de.timestamp, 12345);
        assert!(de.agents.is_empty());
        assert!(de.active_tasks.is_empty());
        assert!(de.connections.is_empty());
        assert_eq!(de.canvas_metadata.total_agents, 0);
    }

    /// 测试 `AgentRuntimeState` 反序列化时缺失可选字段使用默认值。
    #[test]
    fn test_agent_runtime_state_deserialize_with_defaults() {
        let json = r#"{"agent_id": "a1", "role": "generic"}"#;
        let de: AgentRuntimeState =
            serde_json::from_str(json).expect("deserialize with defaults should succeed");
        assert_eq!(de.agent_id, "a1");
        assert_eq!(de.role, AgentKind::Generic);
        assert_eq!(de.status, AgentStatus::Idle);
        assert!(de.current_task_id.is_none());
        assert!((de.cpu_usage - 0.0).abs() < f64::EPSILON);
        assert!((de.memory_usage - 0.0).abs() < f64::EPSILON);
        assert_eq!(de.last_heartbeat, 0);
    }
}
