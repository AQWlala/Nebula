use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, warn};

use super::deadlock::WaitForGraph;
use super::events::SwarmEvent;

const BUS_CAPACITY: usize = 256;
const BROADCAST_CAPACITY: usize = 512;
/// T-S1-B-02: SwarmEvent 广播通道容量。
const EVENT_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusMessage {
    pub from: String,
    pub to: Option<String>,
    pub content: String,
    pub timestamp: i64,
    pub msg_type: BusMessageType,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BusMessageType {
    Request,
    Response,
    Notification,
    Capability,
    /// T-S4-A-03: Agent 间 CRDT 操作传播(负载为序列化的 CrdtVersion)。
    CrdtSync,
}

type PendingReply = tokio::sync::Mutex<HashMap<String, oneshot::Sender<BusMessage>>>;

pub struct AgentBus {
    mailboxes: Arc<tokio::sync::Mutex<HashMap<String, mpsc::Sender<BusMessage>>>>,
    broadcast_tx: broadcast::Sender<BusMessage>,
    /// T-S1-B-02: 独立的 SwarmEvent 通道,与 BusMessage 解耦。
    /// 前端通过 `subscribe_events` IPC 订阅此通道。
    event_tx: broadcast::Sender<SwarmEvent>,
    pending_replies: Arc<PendingReply>,
    /// T-E-S-05: 等待图(WFG),用于死锁检测。
    /// 记录 `A → waits_for → B` 关系(Agent A 正在等待 Agent B 回复)。
    wait_for_graph: Arc<RwLock<WaitForGraph>>,
}

impl AgentBus {
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (event_tx, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            mailboxes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            broadcast_tx,
            event_tx,
            pending_replies: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            wait_for_graph: Arc::new(RwLock::new(WaitForGraph::new())),
        }
    }

    pub async fn register(&self, agent_id: &str) -> mpsc::Receiver<BusMessage> {
        let (tx, rx) = mpsc::channel(BUS_CAPACITY);
        self.mailboxes.lock().await.insert(agent_id.to_string(), tx);
        rx
    }

    pub async fn unregister(&self, agent_id: &str) {
        self.mailboxes.lock().await.remove(agent_id);
    }

    pub async fn send(&self, message: BusMessage) -> Result<()> {
        let target = message
            .to
            .as_deref()
            .ok_or_else(|| anyhow!("message has no target"))?;
        let mailboxes = self.mailboxes.lock().await;
        let sender = mailboxes
            .get(target)
            .ok_or_else(|| anyhow!("agent '{target}' not found or not registered"))?;
        sender
            .send(message)
            .await
            .map_err(|e| anyhow!("send failed: {e}"))
    }

    /// T-E-S-05: 获取等待图(WFG)的 Arc 引用,供死锁检测器使用。
    pub fn wait_for_graph(&self) -> Arc<RwLock<WaitForGraph>> {
        Arc::clone(&self.wait_for_graph)
    }

    /// Send a request to a specific agent and wait for a response.
    /// This enables P2P request-response communication between agents.
    pub async fn request(
        &self,
        from: &str,
        to: &str,
        content: String,
        timeout: std::time::Duration,
    ) -> Result<BusMessage> {
        let correlation_id = uuid::Uuid::new_v4().to_string();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending_replies
            .lock()
            .await
            .insert(correlation_id.clone(), reply_tx);

        let msg = BusMessage {
            from: from.to_string(),
            to: Some(to.to_string()),
            content,
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: BusMessageType::Request,
            correlation_id: Some(correlation_id.clone()),
        };
        self.send(msg).await?;

        self.wait_for_graph.write().add_wait(from, to);

        let result = tokio::time::timeout(timeout, reply_rx).await;
        self.wait_for_graph.write().remove_wait(from, to);
        self.pending_replies.lock().await.remove(&correlation_id);
        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(anyhow!("reply channel closed for request to '{to}'")),
            Err(_) => Err(anyhow!("request to '{to}' timed out after {:?}", timeout)),
        }
    }

    /// Reply to a request using the correlation_id from the original message.
    pub async fn reply(&self, original: &BusMessage, content: String) -> Result<()> {
        let correlation_id = original
            .correlation_id
            .as_deref()
            .ok_or_else(|| anyhow!("cannot reply to a message without correlation_id"))?;

        let mut pending = self.pending_replies.lock().await;
        if let Some(reply_tx) = pending.remove(correlation_id) {
            let response = BusMessage {
                from: original.to.clone().unwrap_or_default(),
                to: Some(original.from.clone()),
                content,
                timestamp: chrono::Utc::now().timestamp_millis(),
                msg_type: BusMessageType::Response,
                correlation_id: Some(correlation_id.to_string()),
            };
            let _ = reply_tx.send(response);
            Ok(())
        } else {
            Err(anyhow!(
                "no pending reply for correlation_id '{correlation_id}'"
            ))
        }
    }

    pub fn broadcast(&self, message: BusMessage) {
        if self.broadcast_tx.send(message).is_err() {
            warn!(target: "nebula.bus", "no active broadcast receivers");
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusMessage> {
        self.broadcast_tx.subscribe()
    }

    /// T-S1-B-02: 广播一个结构化 SwarmEvent,供前端订阅可视化。
    /// T-E-S-26: 同时向全局 EventBus 广播(自动包装 EventEnvelope)。
    pub fn emit_event(&self, event: SwarmEvent) {
        // T-E-S-26: 向 EventBus 广播(协议化 EventEnvelope)。
        crate::swarm::event_bus::global().emit(event.clone());
        // 保留原有 event_tx 通道(向后兼容 subscribe_events 旧路径)。
        if self.event_tx.send(event).is_err() {
            // 没有订阅者不算错误:orchestrator 仍会正常执行,
            // 只是当前没有前端在监听。
            debug!(target: "nebula.bus", "no active SwarmEvent subscribers");
        }
    }

    /// T-S1-B-02: 订阅 SwarmEvent 流。返回的 Receiver 可在
    /// `subscribe_events` Tauri 命令中循环 `recv().await` 并通过
    /// `tauri::ipc::Channel::send` 推送给前端。
    pub fn subscribe_events(&self) -> broadcast::Receiver<SwarmEvent> {
        self.event_tx.subscribe()
    }

    /// T-S1-B-02: 返回 event sender 的克隆,供 orchestrator 在
    /// spawn 的 agent 任务中 emit 事件而不需要持有 `&self`。
    pub fn event_sender(&self) -> broadcast::Sender<SwarmEvent> {
        self.event_tx.clone()
    }
}

impl Default for AgentBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_and_send() {
        let bus = AgentBus::new();
        let mut rx = bus.register("agent-1").await;
        let msg = BusMessage {
            from: "agent-2".to_string(),
            to: Some("agent-1".to_string()),
            content: "hello".to_string(),
            timestamp: 0,
            msg_type: BusMessageType::Request,
            correlation_id: None,
        };
        bus.send(msg).await.expect("send should succeed");
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("recv timed out — message never arrived")
            .expect("channel closed");
        assert_eq!(received.content, "hello");
    }

    #[tokio::test]
    async fn send_to_unknown_fails() {
        let bus = AgentBus::new();
        let msg = BusMessage {
            from: "agent-1".to_string(),
            to: Some("unknown".to_string()),
            content: "hello".to_string(),
            timestamp: 0,
            msg_type: BusMessageType::Request,
            correlation_id: None,
        };
        assert!(bus.send(msg).await.is_err());
    }

    #[tokio::test]
    async fn broadcast_works() {
        let bus = AgentBus::new();
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();
        let msg = BusMessage {
            from: "agent-1".to_string(),
            to: None,
            content: "broadcast!".to_string(),
            timestamp: 0,
            msg_type: BusMessageType::Notification,
            correlation_id: None,
        };
        bus.broadcast(msg);
        let s1 = tokio::time::timeout(std::time::Duration::from_secs(5), sub1.recv())
            .await
            .expect("sub1 recv timed out")
            .expect("sub1 channel closed");
        let s2 = tokio::time::timeout(std::time::Duration::from_secs(5), sub2.recv())
            .await
            .expect("sub2 recv timed out")
            .expect("sub2 channel closed");
        assert_eq!(s1.content, "broadcast!");
        assert_eq!(s2.content, "broadcast!");
    }

    #[tokio::test]
    async fn request_reply_p2p() {
        let bus = AgentBus::new();
        let mut rx = bus.register("responder").await;

        let bus_clone = Arc::new(bus);
        let bus_for_task = bus_clone.clone();
        let handle = tokio::spawn(async move {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                .await
                .expect("responder recv timed out — request never arrived")
                .expect("responder channel closed");
            assert_eq!(msg.msg_type, BusMessageType::Request);
            bus_for_task
                .reply(&msg, "pong".to_string())
                .await
                .expect("serialize should succeed");
        });

        let response = bus_clone
            .request(
                "caller",
                "responder",
                "ping".to_string(),
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("test op should succeed");
        assert_eq!(response.content, "pong");
        tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("responder task join timed out")
            .expect("responder task panicked");
    }
}
