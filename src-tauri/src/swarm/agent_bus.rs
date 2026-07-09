//! T-E-AE-05: 主→子任务分派协议 — AgentBus 消息总线 + DelegatedTask 委派协议。
//!
//! 在 T-E-AE-01 的 [`super::primary_agent::PrimaryAgent`] 基础上,落地完整的
//! 主→子任务分派协议。本模块提供:
//!
//! * [`AgentBus`] — 基于主题(topic)的发布/订阅消息总线,底层使用
//!   `tokio::sync::broadcast`,每个 topic 拥有独立的 broadcast 通道。
//! * [`BusMessage`] — 总线消息载体(id/topic/payload/timestamp/sender/reply_to)。
//! * [`BusTopic`] — 预定义主题常量(委派/结果/状态/广播)。
//! * [`DelegatedTask`] — 完整版委派任务(含 parent_id/priority/deadline/context/
//!   status/result),是 T-E-AE-01 `primary_agent::DelegatedTask` 的协议化扩展。
//! * [`DelegationProtocol`] — 委派协议:delegate / collect_results / cancel_task /
//!   get_task_status,串联 PrimaryAgent 与蜂群 worker。
//!
//! ## 命名说明
//!
//! 本模块的 [`AgentBus`] / [`BusMessage`] / [`DelegatedTask`] 与 `bus::` /
//! `primary_agent::` 中的同名类型**有意区分**(分别承载 T-E-AE-05 的完整协议
//! 语义)。因此它们**不在 `swarm` 顶层 re-export**,通过全限定路径
//! `swarm::agent_bus::AgentBus` 访问,避免与既有导出冲突。
//!
//! ## 设计要点
//!
//! * **同步发布/订阅**:`AgentBus` 的 `publish` / `subscribe` / `topics` 均为
//!   同步方法(底层 `parking_lot::RwLock` + `broadcast` 的同步 API),只有
//!   [`DelegationProtocol::collect_results`] 因需 `tokio::time::timeout` 而
//!   为 `async`。
//! * **主题即通道**:每个 topic 对应一个 `broadcast::Sender<BusMessage>`,
//!   `subscribe` 按需创建,`publish` 广播给该 topic 的所有订阅者。
//! * **可测试性**:无 LLM / 蜂群依赖,测试通过模拟 worker 订阅 TASK_DELEGATE
//!   并回发 TASK_RESULT 验证端到端协议。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex as TokioMutex};
use tracing::debug;

// ---------------------------------------------------------------------------
// BusTopic — 预定义主题常量
// ---------------------------------------------------------------------------

/// 预定义总线主题常量。
///
/// 用结构体 + 关联常量表达,便于以 `BusTopic::TASK_DELEGATE` 形式引用,
/// 同时避免污染模块顶层命名空间。
pub struct BusTopic;

impl BusTopic {
    /// 主→子任务委派:PrimaryAgent 发布待执行子任务,worker 订阅领取。
    pub const TASK_DELEGATE: &'static str = "task.delegate";
    /// 子→主任务结果:worker 发布执行结果,PrimaryAgent 订阅收集。
    pub const TASK_RESULT: &'static str = "task.result";
    /// Agent 状态变更(上线/下线/取消等),供监控与调度使用。
    pub const AGENT_STATUS: &'static str = "agent.status";
    /// 全局广播:无差别推送给所有订阅者。
    pub const BROADCAST: &'static str = "broadcast";
}

// ---------------------------------------------------------------------------
// BusMessage — 总线消息载体
// ---------------------------------------------------------------------------

/// 总线消息载体。
///
/// 每条消息归属一个 `topic`,`payload` 为任意 JSON 值(通常是序列化后的
/// `DelegatedTask` / `TaskResult` / 状态对象)。`reply_to` 用于请求-响应
/// 模式下指明回复应发往的主题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusMessage {
    /// 消息唯一 ID(自动生成)。
    pub id: String,
    /// 主题(路由键),决定消息发往哪个 broadcast 通道。
    pub topic: String,
    /// 消息负载(任意 JSON)。
    pub payload: serde_json::Value,
    /// 发送时间戳(UTC)。
    pub timestamp: DateTime<Utc>,
    /// 发送方标识(如 "primary" / "worker-3")。
    pub sender: String,
    /// 回复主题:请求-响应模式下,接收方应将回复发往此主题。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

impl BusMessage {
    /// 创建一条新消息,自动生成 `id`(UUID v4)与 `timestamp`(当前 UTC)。
    pub fn new(
        topic: impl Into<String>,
        sender: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            topic: topic.into(),
            payload,
            timestamp: Utc::now(),
            sender: sender.into(),
            reply_to: None,
        }
    }

    /// 链式设置回复主题。
    pub fn with_reply_to(mut self, reply_to: impl Into<String>) -> Self {
        self.reply_to = Some(reply_to.into());
        self
    }
}

// ---------------------------------------------------------------------------
// AgentBus — 主题式发布/订阅消息总线
// ---------------------------------------------------------------------------

/// broadcast 通道接收端类型别名。
pub type BusReceiver = broadcast::Receiver<BusMessage>;

/// 单个 topic 的 broadcast 通道默认容量。
const DEFAULT_TOPIC_CAPACITY: usize = 256;

/// 主题式发布/订阅消息总线。
///
/// 每个 topic 对应一个独立的 `broadcast::Sender<BusMessage>`。`subscribe`
/// 按需创建通道并返回接收端;`publish` 将消息广播给该 topic 的所有活跃
/// 订阅者。无订阅者时 `publish` 仍返回 `Ok`(消息被丢弃,符合 pub/sub 语义)。
///
/// 内部状态全部包裹在 `Arc` 中,可低成本共享(虽然本类型未实现 `Clone`,
/// 但通过 `&self` 借用即可在多任务间使用)。
pub struct AgentBus {
    topics: Arc<RwLock<HashMap<String, broadcast::Sender<BusMessage>>>>,
    capacity: usize,
}

impl AgentBus {
    /// 创建空消息总线,使用默认 topic 通道容量(256)。
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_TOPIC_CAPACITY)
    }

    /// 创建空消息总线,指定每个 topic 的 broadcast 通道容量(最小为 1)。
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            topics: Arc::new(RwLock::new(HashMap::new())),
            capacity: capacity.max(1),
        }
    }

    /// 发布一条消息到其 `topic`。
    ///
    /// 若该 topic 尚不存在,会自动创建通道。无活跃订阅者时消息被丢弃,
    /// 仍返回 `Ok`(pub/sub 语义:发布不依赖订阅者存在)。
    pub fn publish(&self, msg: BusMessage) -> Result<()> {
        let topic = msg.topic.clone();
        let sender = {
            let map = self.topics.read();
            map.get(&topic).cloned()
        };
        let sender = match sender {
            Some(s) => s,
            None => {
                // 写锁内 get-or-create,避免重复创建。
                let mut map = self.topics.write();
                map.entry(topic.clone())
                    .or_insert_with(|| {
                        let (tx, _rx) = broadcast::channel(self.capacity);
                        tx
                    })
                    .clone()
            }
        };
        // broadcast::send 仅在无活跃接收端时返回 Err;按 pub/sub 语义视为成功。
        let _ = sender.send(msg);
        debug!(target: "nebula.agent_bus", %topic, "message published");
        Ok(())
    }

    /// 订阅指定主题,返回接收端。若 topic 不存在则自动创建。
    ///
    /// 同一 topic 可多次订阅(每个订阅者获得独立接收端,均能收到后续消息)。
    pub fn subscribe(&self, topic: &str) -> BusReceiver {
        let sender = {
            let map = self.topics.read();
            map.get(topic).cloned()
        };
        match sender {
            Some(s) => s.subscribe(),
            None => {
                let mut map = self.topics.write();
                let s = map
                    .entry(topic.to_string())
                    .or_insert_with(|| {
                        let (tx, _rx) = broadcast::channel(self.capacity);
                        tx
                    })
                    .clone();
                s.subscribe()
            }
        }
    }

    /// 退订:消费并丢弃接收端。
    ///
    /// `tokio::sync::broadcast` 无显式退订 API,接收端 `drop` 即自动退订。
    /// 本方法提供语义化入口,便于调用方表达意图。
    pub fn unsubscribe(&self, receiver: BusReceiver) {
        drop(receiver);
    }

    /// 返回当前所有已创建的主题(按字典序)。
    pub fn topics(&self) -> Vec<String> {
        let map = self.topics.read();
        let mut names: Vec<String> = map.keys().cloned().collect();
        names.sort();
        names
    }

    /// 返回每个 topic 的 broadcast 通道容量。
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for AgentBus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 委派协议数据类型
// ---------------------------------------------------------------------------

/// 委派任务类型。`Custom` 允许扩展自定义场景。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DelegatedTaskType {
    Research,
    Writing,
    Coding,
    Review,
    Analysis,
    /// 自定义场景(携带场景名)。
    Custom(String),
}

/// 任务优先级。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// 任务状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// 已创建,尚未分派。
    Pending,
    /// 已分派给 agent,尚未开始执行。
    Assigned,
    /// 正在执行中。
    Running,
    /// 已成功完成。
    Completed,
    /// 执行失败。
    Failed,
    /// 已取消。
    Cancelled,
}

/// 任务执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// 对应的 [`DelegatedTask::id`]。
    pub task_id: String,
    /// 输出内容(任意 JSON,通常是字符串)。
    pub output: serde_json::Value,
    /// 是否成功。
    pub success: bool,
    /// 失败原因(成功时为 `None`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 执行指标(如 token 数、迭代次数等,任意 JSON)。
    #[serde(default)]
    pub metrics: serde_json::Value,
    /// 执行耗时(毫秒)。
    #[serde(default)]
    pub duration_ms: u64,
}

impl TaskResult {
    /// 创建一个成功结果。
    pub fn ok(task_id: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            output: serde_json::Value::String(output.into()),
            success: true,
            error: None,
            metrics: serde_json::Value::Null,
            duration_ms: 0,
        }
    }

    /// 创建一个失败结果。
    pub fn failed(task_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            output: serde_json::Value::Null,
            success: false,
            error: Some(error.into()),
            metrics: serde_json::Value::Null,
            duration_ms: 0,
        }
    }
}

/// 主→子任务分派协议的完整任务载体(T-E-AE-05)。
///
/// 相比 T-E-AE-01 `primary_agent::DelegatedTask`(仅 id/description/
/// target_scenario/dependencies),本类型补充了 parent_id / priority /
/// deadline / context / status / result 等完整协议字段,支撑状态机化
/// 的委派-执行-回收流程。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedTask {
    /// 唯一标识。
    pub id: String,
    /// 父任务 ID(顶层任务为 `None`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// 任务类型。
    pub task_type: DelegatedTaskType,
    /// 任务描述(prompt 主体)。
    pub description: String,
    /// 已分配的 agent 标识(未分配为 `None`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_agent: Option<String>,
    /// 优先级。
    #[serde(default = "default_priority")]
    pub priority: TaskPriority,
    /// 截止时间(无截止为 `None`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<DateTime<Utc>>,
    /// 上下文(任意 JSON,可携带依赖任务输出、用户偏好等)。
    #[serde(default)]
    pub context: serde_json::Value,
    /// 当前状态。
    #[serde(default = "default_status")]
    pub status: TaskStatus,
    /// 执行结果(完成后填充)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<TaskResult>,
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

fn default_status() -> TaskStatus {
    TaskStatus::Pending
}

impl DelegatedTask {
    /// 创建一个新任务,默认优先级 `Medium`、状态 `Pending`、无父任务。
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        task_type: DelegatedTaskType,
    ) -> Self {
        Self {
            id: id.into(),
            parent_id: None,
            task_type,
            description: description.into(),
            assigned_agent: None,
            priority: TaskPriority::Medium,
            deadline: None,
            context: serde_json::Value::Null,
            status: TaskStatus::Pending,
            result: None,
        }
    }

    /// 链式设置父任务 ID。
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// 链式设置优先级。
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    /// 链式设置已分配 agent。
    pub fn with_assigned_agent(mut self, agent: impl Into<String>) -> Self {
        self.assigned_agent = Some(agent.into());
        self
    }
}

// ---------------------------------------------------------------------------
// DelegationProtocol — 委派协议
// ---------------------------------------------------------------------------

/// 主→子任务委派协议。
///
/// 串联 [`AgentBus`] 与 [`DelegatedTask`],提供:
/// - [`delegate`](Self::delegate):发布 `TASK_DELEGATE` 消息并登记任务。
/// - [`collect_results`](Self::collect_results):在 `TASK_RESULT` 主题上
///   收集指定任务的执行结果(带超时)。
/// - [`cancel_task`](Self::cancel_task):取消任务并发布 `AGENT_STATUS` 通知。
/// - [`get_task_status`](Self::get_task_status):查询任务当前状态。
///
/// 构造时通过 [`new`](Self::new) 订阅 `TASK_RESULT` 主题,后续
/// `collect_results` 从该订阅接收端拉取结果。因此结果消息必须在协议构造
/// **之后**发布才能被收到(broadcast 通道不缓存订阅前的消息)。
pub struct DelegationProtocol {
    /// 任务登记表:task_id → task(含最新 status)。
    tasks: Arc<RwLock<HashMap<String, DelegatedTask>>>,
    /// 已收集的结果缓存:task_id → result。
    results: Arc<RwLock<HashMap<String, TaskResult>>>,
    /// TASK_RESULT 主题订阅接收端(仅 collect_results 使用)。
    result_rx: TokioMutex<BusReceiver>,
}

impl DelegationProtocol {
    /// 创建委派协议,订阅指定总线的 `TASK_RESULT` 主题。
    pub fn new(bus: &AgentBus) -> Self {
        let rx = bus.subscribe(BusTopic::TASK_RESULT);
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            results: Arc::new(RwLock::new(HashMap::new())),
            result_rx: TokioMutex::new(rx),
        }
    }

    /// 委派任务:将任务状态置为 `Assigned`,发布 `TASK_DELEGATE` 消息,
    /// 并在内部登记表记录。返回任务 ID。
    pub fn delegate(&self, mut task: DelegatedTask, bus: &AgentBus) -> Result<String> {
        task.status = TaskStatus::Assigned;
        let id = task.id.clone();
        let payload = serde_json::to_value(&task)
            .map_err(|e| anyhow!("serialize DelegatedTask failed: {e}"))?;
        let msg = BusMessage::new(BusTopic::TASK_DELEGATE, "primary", payload);
        bus.publish(msg)?;
        self.tasks.write().insert(id.clone(), task);
        debug!(target: "nebula.agent_bus", %id, "task delegated");
        Ok(id)
    }

    /// 收集指定任务的结果,带超时。
    ///
    /// 在 `TASK_RESULT` 主题上拉取消息,匹配 `task_ids` 中的任务。超时后
    /// 未收到结果的任务以失败占位结果(`error = "timeout"`)返回。
    /// 返回顺序与 `task_ids` 一致,长度始终等于 `task_ids.len()`。
    pub async fn collect_results(&self, task_ids: &[String], timeout: Duration) -> Vec<TaskResult> {
        let needed: HashSet<String> = task_ids.iter().cloned().collect();
        let mut collected: HashMap<String, TaskResult> = HashMap::new();

        // 1. 先从已缓存的结果中提取(支持重复调用)。
        {
            let store = self.results.read();
            for id in &needed {
                if let Some(r) = store.get(id) {
                    collected.insert(id.clone(), r.clone());
                }
            }
        }

        // 2. 在 TASK_RESULT 接收端上拉取,直到收齐或超时。
        if collected.len() < needed.len() {
            let mut rx = self.result_rx.lock().await;
            let deadline = tokio::time::Instant::now() + timeout;
            while collected.len() < needed.len() {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match tokio::time::timeout(remaining, rx.recv()).await {
                    Ok(Ok(msg)) => {
                        if let Ok(result) =
                            serde_json::from_value::<TaskResult>(msg.payload.clone())
                        {
                            if needed.contains(&result.task_id) {
                                // 更新缓存与任务状态。
                                {
                                    let mut store = self.results.write();
                                    store.insert(result.task_id.clone(), result.clone());
                                }
                                let new_status = if result.success {
                                    TaskStatus::Completed
                                } else {
                                    TaskStatus::Failed
                                };
                                if let Some(task) = self.tasks.write().get_mut(&result.task_id) {
                                    task.status = new_status;
                                }
                                collected.insert(result.task_id.clone(), result);
                            }
                        }
                    }
                    // 通道关闭或滞后(Lagged):终止拉取。
                    _ => break,
                }
            }
        }

        // 3. 按输入顺序输出,缺失项以 timeout 失败占位。
        task_ids
            .iter()
            .map(|id| {
                collected
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| TaskResult::failed(id, "timeout"))
            })
            .collect()
    }

    /// 取消任务:将状态置为 `Cancelled`,发布 `AGENT_STATUS` 取消通知。
    pub fn cancel_task(&self, task_id: &str, bus: &AgentBus) -> Result<()> {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.status = TaskStatus::Cancelled;
        }
        let payload = serde_json::json!({ "task_id": task_id, "action": "cancel" });
        let msg = BusMessage::new(BusTopic::AGENT_STATUS, "primary", payload);
        bus.publish(msg)?;
        debug!(target: "nebula.agent_bus", %task_id, "task cancelled");
        Ok(())
    }

    /// 查询任务当前状态;未知任务返回 `None`。
    pub fn get_task_status(&self, task_id: &str) -> Option<TaskStatus> {
        self.tasks.read().get(task_id).map(|t| t.status)
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    // ===================================================================
    // BusMessage 测试
    // ===================================================================

    #[test]
    fn bus_message_new_sets_fields() {
        let msg = BusMessage::new(BusTopic::TASK_DELEGATE, "primary", serde_json::json!("hi"));
        assert!(!msg.id.is_empty(), "id 应自动生成");
        assert_eq!(msg.topic, BusTopic::TASK_DELEGATE);
        assert_eq!(msg.sender, "primary");
        assert_eq!(msg.payload, serde_json::json!("hi"));
        assert!(msg.reply_to.is_none(), "新消息默认无 reply_to");
    }

    #[test]
    fn bus_message_with_reply_to_sets_field() {
        let msg = BusMessage::new(BusTopic::TASK_RESULT, "worker", serde_json::json!(1))
            .with_reply_to("task.delegate");
        assert_eq!(msg.reply_to.as_deref(), Some("task.delegate"));
    }

    #[test]
    fn bus_message_serde_roundtrip() {
        let msg = BusMessage::new("custom.topic", "sender", serde_json::json!({"k": 42}))
            .with_reply_to("reply.here");
        let json = serde_json::to_string(&msg).expect("serialize");
        let de: BusMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.topic, "custom.topic");
        assert_eq!(de.sender, "sender");
        assert_eq!(de.payload, serde_json::json!({"k": 42}));
        assert_eq!(de.reply_to.as_deref(), Some("reply.here"));
    }

    // ===================================================================
    // AgentBus 发布/订阅测试
    // ===================================================================

    #[test]
    fn agent_bus_new_has_no_topics() {
        let bus = AgentBus::new();
        assert!(bus.topics().is_empty(), "新总线应无主题");
    }

    #[test]
    fn agent_bus_default_works() {
        let bus = AgentBus::default();
        assert!(bus.topics().is_empty());
        assert_eq!(bus.capacity(), DEFAULT_TOPIC_CAPACITY);
    }

    #[test]
    fn agent_bus_with_capacity_clamps_to_one() {
        let bus = AgentBus::with_capacity(0);
        assert_eq!(bus.capacity(), 1, "容量应被 clamp 到 1");
    }

    #[test]
    fn agent_bus_publish_creates_topic() {
        let bus = AgentBus::new();
        let msg = BusMessage::new(BusTopic::BROADCAST, "primary", serde_json::json!(null));
        bus.publish(msg).expect("publish");
        let topics = bus.topics();
        assert!(
            topics.contains(&BusTopic::BROADCAST.to_string()),
            "publish 应创建主题"
        );
    }

    #[test]
    fn agent_bus_subscribe_creates_topic() {
        let bus = AgentBus::new();
        let _rx = bus.subscribe(BusTopic::AGENT_STATUS);
        assert!(bus.topics().contains(&BusTopic::AGENT_STATUS.to_string()));
    }

    #[test]
    fn agent_bus_publish_with_no_subscriber_ok() {
        // 无订阅者时 publish 仍应返回 Ok(pub/sub 语义)。
        let bus = AgentBus::new();
        let msg = BusMessage::new(BusTopic::BROADCAST, "x", serde_json::json!(null));
        assert!(bus.publish(msg).is_ok());
    }

    #[tokio::test]
    async fn agent_bus_publish_and_receive() {
        let bus = AgentBus::new();
        let mut rx = bus.subscribe(BusTopic::TASK_DELEGATE);
        let msg = BusMessage::new(
            BusTopic::TASK_DELEGATE,
            "primary",
            serde_json::json!("payload"),
        );
        bus.publish(msg).expect("publish");
        let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("recv 超时")
            .expect("通道关闭");
        assert_eq!(received.topic, BusTopic::TASK_DELEGATE);
        assert_eq!(received.payload, serde_json::json!("payload"));
    }

    #[tokio::test]
    async fn agent_bus_multiple_subscribers_receive() {
        let bus = AgentBus::new();
        let mut rx1 = bus.subscribe(BusTopic::BROADCAST);
        let mut rx2 = bus.subscribe(BusTopic::BROADCAST);
        let msg = BusMessage::new(BusTopic::BROADCAST, "primary", serde_json::json!(42));
        bus.publish(msg).expect("publish");
        let r1 = tokio::time::timeout(Duration::from_secs(2), rx1.recv())
            .await
            .expect("rx1 超时")
            .expect("rx1 通道关闭");
        let r2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .expect("rx2 超时")
            .expect("rx2 通道关闭");
        assert_eq!(r1.payload, serde_json::json!(42));
        assert_eq!(r2.payload, serde_json::json!(42));
    }

    #[tokio::test]
    async fn agent_bus_fifo_order_preserved() {
        let bus = AgentBus::new();
        let mut rx = bus.subscribe(BusTopic::TASK_RESULT);
        for i in 0..5 {
            bus.publish(BusMessage::new(
                BusTopic::TASK_RESULT,
                "worker",
                serde_json::json!(i),
            ))
            .expect("publish");
        }
        for i in 0..5 {
            let m = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("recv 超时")
                .expect("通道关闭");
            assert_eq!(m.payload, serde_json::json!(i), "应保持 FIFO 顺序");
        }
    }

    #[test]
    fn agent_bus_topics_sorted_and_complete() {
        let bus = AgentBus::new();
        let _ = bus.subscribe(BusTopic::TASK_DELEGATE);
        let _ = bus.subscribe(BusTopic::TASK_RESULT);
        let _ = bus.subscribe(BusTopic::AGENT_STATUS);
        let topics = bus.topics();
        assert_eq!(topics.len(), 3);
        // 应为字典序。
        assert_eq!(
            topics,
            vec![
                BusTopic::AGENT_STATUS.to_string(),
                BusTopic::TASK_DELEGATE.to_string(),
                BusTopic::TASK_RESULT.to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn agent_bus_unsubscribe_drops_receiver() {
        // unsubscribe 消费接收端;之后该接收端无法再使用(drop 后编译期即可检测)。
        let bus = AgentBus::new();
        let rx = bus.subscribe(BusTopic::BROADCAST);
        bus.unsubscribe(rx);
        // 主题仍存在(发送端保留),不影响其他订阅者。
        assert!(bus.topics().contains(&BusTopic::BROADCAST.to_string()));
    }

    // ===================================================================
    // BusTopic 常量测试
    // ===================================================================

    #[test]
    fn bus_topic_constants_are_distinct_and_nonempty() {
        let consts = [
            BusTopic::TASK_DELEGATE,
            BusTopic::TASK_RESULT,
            BusTopic::AGENT_STATUS,
            BusTopic::BROADCAST,
        ];
        for c in &consts {
            assert!(!c.is_empty(), "主题常量不应为空");
        }
        // 4 个常量两两不同。
        let mut sorted = consts.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "4 个主题常量应两两不同");
    }

    // ===================================================================
    // DelegatedTask 数据结构测试
    // ===================================================================

    #[test]
    fn delegated_task_new_defaults() {
        let task = DelegatedTask::new("dt_1", "搜索资料", DelegatedTaskType::Research);
        assert_eq!(task.id, "dt_1");
        assert_eq!(task.description, "搜索资料");
        assert_eq!(task.task_type, DelegatedTaskType::Research);
        assert!(task.parent_id.is_none());
        assert!(task.assigned_agent.is_none());
        assert_eq!(task.priority, TaskPriority::Medium);
        assert!(task.deadline.is_none());
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.result.is_none());
    }

    #[test]
    fn delegated_task_builders_set_fields() {
        let task = DelegatedTask::new("dt_2", "写代码", DelegatedTaskType::Coding)
            .with_parent("dt_1")
            .with_priority(TaskPriority::High)
            .with_assigned_agent("worker-1");
        assert_eq!(task.parent_id.as_deref(), Some("dt_1"));
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.assigned_agent.as_deref(), Some("worker-1"));
    }

    #[test]
    fn delegated_task_serde_roundtrip() {
        let task = DelegatedTask::new("dt_3", "审查", DelegatedTaskType::Review)
            .with_parent("dt_0")
            .with_priority(TaskPriority::Critical);
        let json = serde_json::to_string(&task).expect("serialize");
        let de: DelegatedTask = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.id, "dt_3");
        assert_eq!(de.task_type, DelegatedTaskType::Review);
        assert_eq!(de.parent_id.as_deref(), Some("dt_0"));
        assert_eq!(de.priority, TaskPriority::Critical);
    }

    #[test]
    fn delegated_task_type_custom_roundtrip() {
        let ty = DelegatedTaskType::Custom("translate".to_string());
        let json = serde_json::to_string(&ty).expect("serialize");
        let de: DelegatedTaskType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de, ty);
        assert_eq!(ty, DelegatedTaskType::Custom("translate".to_string()));
    }

    #[test]
    fn task_priority_ord_orders_correctly() {
        // Low < Medium < High < Critical。
        assert!(TaskPriority::Low < TaskPriority::Medium);
        assert!(TaskPriority::Medium < TaskPriority::High);
        assert!(TaskPriority::High < TaskPriority::Critical);
    }

    #[test]
    fn task_status_all_variants_serde() {
        let variants = [
            TaskStatus::Pending,
            TaskStatus::Assigned,
            TaskStatus::Running,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize");
            let de: TaskStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(&de, v);
        }
    }

    #[test]
    fn task_result_ok_and_failed() {
        let ok = TaskResult::ok("dt_1", "成功输出");
        assert!(ok.success);
        assert_eq!(ok.task_id, "dt_1");
        assert_eq!(ok.output, serde_json::json!("成功输出"));
        assert!(ok.error.is_none());

        let fail = TaskResult::failed("dt_2", "超时");
        assert!(!fail.success);
        assert_eq!(fail.error.as_deref(), Some("超时"));
    }

    // ===================================================================
    // DelegationProtocol 委派/收集/取消测试
    // ===================================================================

    #[tokio::test]
    async fn protocol_delegate_publishes_message() {
        // delegate 应在 TASK_DELEGATE 主题上发布消息,worker 订阅可收到。
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        let mut worker_rx = bus.subscribe(BusTopic::TASK_DELEGATE);

        let task = DelegatedTask::new("dt_1", "搜索", DelegatedTaskType::Research);
        let id = protocol.delegate(task, &bus).expect("delegate");

        let msg = tokio::time::timeout(Duration::from_secs(2), worker_rx.recv())
            .await
            .expect("worker recv 超时")
            .expect("通道关闭");
        assert_eq!(msg.topic, BusTopic::TASK_DELEGATE);
        let payload_task: DelegatedTask =
            serde_json::from_value(msg.payload).expect("反序列化任务");
        assert_eq!(payload_task.id, "dt_1");
        assert_eq!(payload_task.status, TaskStatus::Assigned);
        assert_eq!(id, "dt_1");
    }

    #[tokio::test]
    async fn protocol_delegate_records_task_as_assigned() {
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        let task = DelegatedTask::new("dt_1", "写代码", DelegatedTaskType::Coding);
        protocol.delegate(task, &bus).expect("delegate");
        assert_eq!(
            protocol.get_task_status("dt_1"),
            Some(TaskStatus::Assigned),
            "delegate 后状态应为 Assigned"
        );
    }

    #[tokio::test]
    async fn protocol_get_task_status_unknown_returns_none() {
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        assert!(protocol.get_task_status("nope").is_none());
    }

    #[tokio::test]
    async fn protocol_collect_results_collects_published_result() {
        // 模拟 worker:订阅 TASK_DELEGATE,收到任务后回发 TASK_RESULT。
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus); // 订阅 TASK_RESULT
        let mut worker_rx = bus.subscribe(BusTopic::TASK_DELEGATE);

        let task = DelegatedTask::new("dt_1", "分析", DelegatedTaskType::Analysis);
        let id = protocol.delegate(task, &bus).expect("delegate");

        // worker 收到任务并回发结果。
        let _req = tokio::time::timeout(Duration::from_secs(2), worker_rx.recv())
            .await
            .expect("worker recv 超时")
            .expect("通道关闭");
        let result = TaskResult::ok("dt_1", "分析完成");
        bus.publish(BusMessage::new(
            BusTopic::TASK_RESULT,
            "worker",
            serde_json::to_value(&result).expect("serialize"),
        ))
        .expect("publish result");

        let results = protocol
            .collect_results(&[id], Duration::from_secs(2))
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success, "应收到成功结果");
        assert_eq!(results[0].task_id, "dt_1");
        // 收到结果后任务状态应变更为 Completed。
        assert_eq!(
            protocol.get_task_status("dt_1"),
            Some(TaskStatus::Completed)
        );
    }

    #[tokio::test]
    async fn protocol_collect_results_timeout_returns_failed_placeholder() {
        // 无 worker 回发结果 → 超时后返回失败占位。
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        let task = DelegatedTask::new("dt_x", "任务", DelegatedTaskType::Coding);
        protocol.delegate(task, &bus).expect("delegate");

        let results = protocol
            .collect_results(&["dt_x".to_string()], Duration::from_millis(100))
            .await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].success, "超时应返回失败占位");
        assert_eq!(results[0].error.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn protocol_collect_results_partial_success() {
        // 2 个任务,仅 1 个回发结果 → 1 成功 1 超时占位。
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);

        let t1 = DelegatedTask::new("dt_1", "a", DelegatedTaskType::Research);
        let t2 = DelegatedTask::new("dt_2", "b", DelegatedTaskType::Writing);
        protocol.delegate(t1, &bus).expect("delegate");
        protocol.delegate(t2, &bus).expect("delegate");

        // 仅回发 dt_1 的结果。
        let r1 = TaskResult::ok("dt_1", "done");
        bus.publish(BusMessage::new(
            BusTopic::TASK_RESULT,
            "worker",
            serde_json::to_value(&r1).expect("serialize"),
        ))
        .expect("publish");

        let results = protocol
            .collect_results(
                &["dt_1".to_string(), "dt_2".to_string()],
                Duration::from_millis(150),
            )
            .await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].task_id, "dt_1");
        assert!(results[0].success);
        assert_eq!(results[1].task_id, "dt_2");
        assert!(!results[1].success, "dt_2 应为超时占位");
    }

    #[tokio::test]
    async fn protocol_cancel_task_updates_status() {
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        let task = DelegatedTask::new("dt_1", "任务", DelegatedTaskType::Coding);
        protocol.delegate(task, &bus).expect("delegate");
        assert_eq!(protocol.get_task_status("dt_1"), Some(TaskStatus::Assigned));

        protocol.cancel_task("dt_1", &bus).expect("cancel");
        assert_eq!(
            protocol.get_task_status("dt_1"),
            Some(TaskStatus::Cancelled),
            "cancel 后状态应为 Cancelled"
        );
    }

    #[tokio::test]
    async fn protocol_cancel_task_publishes_status_message() {
        // cancel 应在 AGENT_STATUS 主题上发布取消通知。
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        let mut status_rx = bus.subscribe(BusTopic::AGENT_STATUS);

        let task = DelegatedTask::new("dt_1", "任务", DelegatedTaskType::Coding);
        protocol.delegate(task, &bus).expect("delegate");
        protocol.cancel_task("dt_1", &bus).expect("cancel");

        let msg = tokio::time::timeout(Duration::from_secs(2), status_rx.recv())
            .await
            .expect("status recv 超时")
            .expect("通道关闭");
        assert_eq!(msg.topic, BusTopic::AGENT_STATUS);
        assert_eq!(msg.payload["task_id"].as_str(), Some("dt_1"));
        assert_eq!(msg.payload["action"].as_str(), Some("cancel"));
    }

    #[tokio::test]
    async fn protocol_collect_results_after_prior_collection_uses_cache() {
        // 已收集过的结果在再次调用时应从缓存命中(无需再次发布)。
        let bus = AgentBus::new();
        let protocol = DelegationProtocol::new(&bus);
        let task = DelegatedTask::new("dt_1", "任务", DelegatedTaskType::Coding);
        protocol.delegate(task, &bus).expect("delegate");

        let r = TaskResult::ok("dt_1", "done");
        bus.publish(BusMessage::new(
            BusTopic::TASK_RESULT,
            "worker",
            serde_json::to_value(&r).expect("serialize"),
        ))
        .expect("publish");

        // 第一次收集(应收到并缓存)。
        let first = protocol
            .collect_results(&["dt_1".to_string()], Duration::from_secs(2))
            .await;
        assert!(first[0].success);
        // 第二次收集:即便无新消息,也应从缓存返回成功结果。
        let second = protocol
            .collect_results(&["dt_1".to_string()], Duration::from_millis(50))
            .await;
        assert_eq!(second.len(), 1);
        assert!(second[0].success, "缓存应命中");
    }
}
