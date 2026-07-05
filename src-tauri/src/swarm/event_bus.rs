//! T-E-S-26: EventBus — 基于 broadcast 的 SwarmEvent 事件总线。
//!
//! 复用 DiagnosticsBus 模式(`tokio::sync::broadcast`),
//! `emit` 方法自动将 `SwarmEvent` 包装为 `EventEnvelope<SwarmEvent>` 后广播。
//! 前端通过 `subscribe_events` Tauri 命令订阅 `EventEnvelope<serde_json::Value>`。

use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::debug;

use super::events::{EventEnvelope, SwarmEvent};

/// 默认 broadcast 容量(spec: 512)。
pub const DEFAULT_CAPACITY: usize = 512;

/// T-E-S-26: Swarm 事件总线。
///
/// 独立的 `broadcast::Sender<EventEnvelope<SwarmEvent>>`,
/// 专门承载协议化的 Swarm 事件流。`emit` 自动包装 `EventEnvelope`。
pub struct EventBus {
    tx: broadcast::Sender<EventEnvelope<SwarmEvent>>,
    capacity: usize,
}

impl EventBus {
    /// 用默认容量 512 构造。
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// 用指定容量构造。
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        let (tx, _) = broadcast::channel(cap);
        Self {
            tx,
            capacity: cap,
        }
    }

    /// 返回通道容量。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 返回 broadcast sender 的克隆(供 spawn 的任务 emit 事件)。
    pub fn sender(&self) -> broadcast::Sender<EventEnvelope<SwarmEvent>> {
        self.tx.clone()
    }

    /// 订阅事件流。返回的 Receiver 可在 Tauri 命令中循环
    /// `recv().await` 并通过 `tauri::ipc::Channel::send` 推送给前端。
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope<SwarmEvent>> {
        self.tx.subscribe()
    }

    /// 发出一个 SwarmEvent,自动包装为 `EventEnvelope` 后广播。
    ///
    /// 没有订阅者不算错误:事件仍被记录到 tracing 层的
    /// `nebula.event_bus` target 中,只是当前没有前端在监听。
    pub fn emit(&self, event: SwarmEvent) {
        let envelope = EventEnvelope::wrap_with_variant(event);
        if self.tx.send(envelope).is_err() {
            debug!(
                target: "nebula.event_bus",
                "no active event subscribers"
            );
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// 全局单例(EventBus 不需要像 DiagnosticsBus 那样在 init_tracing 中使用,
/// 但保持一致的模式)。用 `OnceLock` 确保 AppState 与各命令路径拿到同一实例。
pub fn global() -> &'static Arc<EventBus> {
    static GLOBAL: std::sync::OnceLock<Arc<EventBus>> = std::sync::OnceLock::new();
    GLOBAL.get_or_init(|| {
        let cap = std::env::var("NEBULA_EVENT_BUS_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_CAPACITY);
        Arc::new(EventBus::with_capacity(cap))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::AgentKind;
    use tokio::time::Duration;

    /// test_event_bus_emit_and_subscribe: emit 3 个事件 → 接收 3 个 EventEnvelope。
    #[tokio::test]
    async fn test_event_bus_emit_and_subscribe() {
        let bus = EventBus::with_capacity(64);
        let mut rx = bus.subscribe();

        bus.emit(SwarmEvent::agent_started(AgentKind::Coder, "t-1"));
        bus.emit(SwarmEvent::agent_completed(AgentKind::Writer, "t-2", true, None));
        bus.emit(SwarmEvent::swarm_completed("t-3", 1, 0, true));

        let mut received = Vec::new();
        for _ in 0..3 {
            let envelope = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("recv timed out")
                .expect("channel closed");
            received.push(envelope);
        }

        assert_eq!(received.len(), 3);
        assert_eq!(received[0].event_type, "AgentStarted");
        assert_eq!(received[1].event_type, "AgentCompleted");
        assert_eq!(received[2].event_type, "SwarmCompleted");

        // 验证每个 envelope 的 payload 类型
        match &received[0].payload {
            SwarmEvent::AgentStarted { task_id, .. } => assert_eq!(task_id, "t-1"),
            _ => panic!("expected AgentStarted"),
        }
        match &received[1].payload {
            SwarmEvent::AgentCompleted { task_id, .. } => assert_eq!(task_id, "t-2"),
            _ => panic!("expected AgentCompleted"),
        }
        match &received[2].payload {
            SwarmEvent::SwarmCompleted { task_id, .. } => assert_eq!(task_id, "t-3"),
            _ => panic!("expected SwarmCompleted"),
        }
    }

    /// emit 无订阅者时不出 panic。
    #[tokio::test]
    async fn test_emit_with_no_subscribers() {
        let bus = EventBus::with_capacity(4);
        bus.emit(SwarmEvent::agent_started(AgentKind::Generic, "t-no-sub"));
        // 不应 panic
    }

    /// 全局单例返回同一实例。
    #[test]
    fn test_global_returns_same_instance() {
        let a = global() as *const _;
        let b = global() as *const _;
        assert_eq!(a, b);
    }

    /// capacity 被 clamp 到 1。
    #[test]
    fn test_capacity_clamped_to_one() {
        let bus = EventBus::with_capacity(0);
        assert_eq!(bus.capacity(), 1);
    }
}
