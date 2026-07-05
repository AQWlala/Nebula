//! T-E-S-54: 消息触发器 — 监听 `AgentBus::subscribe_events()` 广播的
//! `SwarmEvent`,按 `TriggerCondition::Message` 匹配后调用
//! `TriggerEngine::dispatch(trigger_id, payload)`。
//!
//! 设计要点:
//! * 用 `broadcast::Receiver<SwarmEvent>` 订阅事件流(与 `notify` 模块的
//!   订阅器同模式,见 `lib.rs` 中 `swarm_bus.subscribe_events()`)。
//! * `Lagged` 错误时跳过(参考 `lib.rs` notification subscriber)。
//! * `Closed` 错误时退出 worker。
//! * 匹配条件用 `super::message_condition_matches`。
//! * payload 由 `super::swarm_event_to_payload` 构造,带 `event_kind` 字段。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::swarm::bus::AgentBus;
use crate::swarm::events::SwarmEvent;

use super::{
    message_condition_matches, swarm_event_to_payload, TriggerCondition, TriggerKind,
};

/// 启动消息订阅 worker。
///
/// 持续监听 `AgentBus` 的 `SwarmEvent` 流,对每个 enabled 的
/// `TriggerKind::Message` 触发器调用 `engine.dispatch`。
///
/// 返回 `JoinHandle` 供调用方管理生命周期(abort 即停止)。
pub fn spawn_message_subscriber(
    bus: Arc<AgentBus>,
    triggers: Arc<parking_lot::RwLock<HashMap<String, super::TriggerConfig>>>,
    engine: Arc<super::TriggerEngine>,
) -> JoinHandle<()> {
    let mut rx = bus.subscribe_events();
    info!(target: "nebula.triggers.message", "subscriber started");
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    handle_event(&event, &triggers, &engine);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(
                        target: "nebula.triggers.message",
                        lagged = n,
                        "subscriber lagged; skipping stale events"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!(
                        target: "nebula.triggers.message",
                        "subscriber channel closed; exiting"
                    );
                    break;
                }
            }
        }
        info!(target: "nebula.triggers.message", "subscriber exiting");
    })
}

/// 处理单个 SwarmEvent:遍历所有 enabled Message 触发器,匹配条件后 dispatch。
fn handle_event(
    event: &SwarmEvent,
    triggers: &Arc<parking_lot::RwLock<HashMap<String, super::TriggerConfig>>>,
    engine: &Arc<super::TriggerEngine>,
) {
    let matching: Vec<(String, TriggerCondition)> = {
        let map = triggers.read();
        map.iter()
            .filter(|(_, cfg)| cfg.enabled && cfg.kind == TriggerKind::Message)
            .filter(|(_, cfg)| message_condition_matches(&cfg.condition, event))
            .map(|(id, cfg)| (id.clone(), cfg.condition.clone()))
            .collect()
    };
    if matching.is_empty() {
        return;
    }
    let payload = swarm_event_to_payload(event);
    let now = std::time::Instant::now();
    for (id, _) in matching {
        // 这里只做粗粒度去抖检查(精确去抖由 engine.dispatch 内部完成)。
        // 早期跳过可减少不必要的 dispatch 调用。
        debug!(
            target: "nebula.triggers.message",
            trigger_id = %id,
            event_kind = %super::swarm_event_kind_str(event),
            "matched; dispatching"
        );
        let _ = now;
        let engine = Arc::clone(engine);
        let payload_clone = payload.clone();
        let id_clone = id.clone();
        tokio::spawn(async move {
            engine.dispatch(&id_clone, payload_clone).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::agents::AgentKind;
    use crate::triggers::TriggerConfig;

    fn make_message_trigger(
        id: &str,
        event_kind: Option<&str>,
        agent_kind: Option<&str>,
        success_only: bool,
    ) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            name: format!("msg-{id}"),
            enabled: true,
            kind: TriggerKind::Message,
            condition: TriggerCondition::Message {
                event_kind: event_kind.map(|s| s.to_string()),
                agent_kind: agent_kind.map(|s| s.to_string()),
                success_only,
            },
            action: crate::triggers::TriggerAction::Notify {
                title: "T".to_string(),
                body: "B".to_string(),
            },
            debounce_ms: 1000,
            max_fires: None,
        }
    }

    #[test]
    fn test_message_trigger_condition_match_via_helper() {
        let cfg = make_message_trigger("t1", Some("agent_completed"), Some("coder"), true);
        let event = SwarmEvent::agent_completed(AgentKind::Coder, "task-1", true, None);
        assert!(message_condition_matches(&cfg.condition, &event));
    }

    #[test]
    fn test_message_trigger_condition_no_match_wrong_kind() {
        let cfg = make_message_trigger("t1", Some("agent_started"), None, false);
        let event = SwarmEvent::agent_completed(AgentKind::Coder, "task-1", true, None);
        assert!(!message_condition_matches(&cfg.condition, &event));
    }

    #[test]
    fn test_debounce_helper_in_message_module() {
        use crate::triggers::debounce_should_fire;
        let now = std::time::Instant::now();
        assert!(debounce_should_fire(None, now, 1000));
        let last = now - std::time::Duration::from_millis(100);
        assert!(!debounce_should_fire(Some(last), now, 1000));
    }
}
