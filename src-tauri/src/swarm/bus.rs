use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::warn;


const BUS_CAPACITY: usize = 256;
const BROADCAST_CAPACITY: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusMessage {
    pub from: String,
    pub to: Option<String>,
    pub content: String,
    pub timestamp: i64,
    pub msg_type: BusMessageType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BusMessageType {
    Request,
    Response,
    Notification,
    Capability,
}

pub struct AgentBus {
    mailboxes: Arc<tokio::sync::Mutex<HashMap<String, mpsc::Sender<BusMessage>>>>,
    broadcast_tx: broadcast::Sender<BusMessage>,
}

impl AgentBus {
    pub fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            mailboxes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            broadcast_tx,
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
        let target = message.to.as_deref().ok_or_else(|| anyhow!("message has no target"))?;
        let mailboxes = self.mailboxes.lock().await;
        let sender = mailboxes
            .get(target)
            .ok_or_else(|| anyhow!("agent '{target}' not found or not registered"))?;
        sender.send(message).await.map_err(|e| anyhow!("send failed: {e}"))
    }

    pub fn broadcast(&self, message: BusMessage) {
        if self.broadcast_tx.send(message).is_err() {
            warn!(target: "nine_snake.bus", "no active broadcast receivers");
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusMessage> {
        self.broadcast_tx.subscribe()
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
        };
        bus.send(msg).await.unwrap();
        let received = rx.recv().await.unwrap();
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
        };
        bus.broadcast(msg);
        assert_eq!(sub1.recv().await.unwrap().content, "broadcast!");
        assert_eq!(sub2.recv().await.unwrap().content, "broadcast!");
    }
}