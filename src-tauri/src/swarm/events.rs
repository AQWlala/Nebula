//! T-S1-B-02: Swarm 事件 — 供前端可视化订阅的语义事件模型。
//!
//! 与 `BusMessage` 不同,`SwarmEvent` 是面向**蜂群生命周期**的
//! 结构化事件枚举:每个变体对应 orchestrator 执行流程的一个关键节点
//! (启动/完成/协商/裁决)。前端通过 `subscribe_events` IPC 订阅
//! `tauri::ipc::Channel<SwarmEvent>`,实现 Swarm 实时可视化。
//!
//! ## 事件触发点（orchestrator.rs::execute）
//!
//! 1. `AgentStarted` — fan-out 之前,每个 agent 入队
//! 2. `AgentCompleted` — 单个 agent 完成（含 success/failure 标志）
//! 3. `NegotiationStarted` — outputs > 1 时进入协商阶段
//! 4. `ArbitrationResolved` — LLM 仲裁完成,带最终 chosen agent 与 conflict 标志
//! 5. `SwarmCompleted` — 整轮 execute 结束,带统计

use serde::{Deserialize, Serialize};

use crate::swarm::{AgentKind, NegotiationMethod};
use crate::swarm::tot::ThoughtStrategy;

// ---------------------------------------------------------------------------
// T-E-S-26: EventEnvelope — 统一协议包装(event_type / payload / trace_id / timestamp)
// ---------------------------------------------------------------------------

/// 从当前 OTel span context 提取 trace_id。
///
/// - `otel` feature 启用时:从 `tracing::Span::current()` 提取
///   有效 span 的 trace_id(16 字节 hex)。
/// - `otel` feature 关闭时:返回 `None`,由 `EventEnvelope::wrap`
///   fallback 到 UUID。
fn get_current_trace_id() -> Option<String> {
    #[cfg(feature = "otel")]
    {
        use opentelemetry::trace::TraceContextExt as _;
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let ctx = tracing::Span::current().context();
        let span = ctx.span();
        let span_ctx = span.span_context();
        if span_ctx.is_valid() {
            return Some(span_ctx.trace_id().to_string());
        }
    }
    None
}

/// T-E-S-26: 事件协议信封 — 统一包装所有 SwarmEvent。
///
/// 每个事件携带:
/// - `event_type`: 事件类型名(从 SwarmEvent 变体名自动提取)
/// - `payload`: 原始事件数据
/// - `trace_id`: OTel trace_id(16 字节 hex)或 fallback UUID(32 字节 hex)
/// - `timestamp`: Unix 毫秒时间戳(由 wrap 时填充)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T: serde::Serialize + Clone> {
    /// 事件类型名(如 "AgentStarted", "OutputReady")
    pub event_type: String,
    /// 事件负载
    pub payload: T,
    /// OTel trace_id(16 字节 hex)或 fallback UUID(32 字节 hex)
    pub trace_id: String,
    /// Unix 毫秒时间戳
    pub timestamp: i64,
}

impl<T: serde::Serialize + Clone> EventEnvelope<T> {
    /// 将原始事件包装为 `EventEnvelope`。
    ///
    /// `event_type` 从 `T` 的类型名自动提取(取最后一个 `::` 段);
    /// `trace_id` 优先从 OTel span context 提取,fallback UUID;
    /// `timestamp` 取当前 Unix 毫秒。
    pub fn wrap(event: T) -> Self {
        let trace_id = get_current_trace_id()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace("-", ""));
        Self {
            event_type: std::any::type_name::<T>()
                .split("::")
                .last()
                .unwrap_or("unknown")
                .to_string(),
            payload: event,
            trace_id,
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

impl EventEnvelope<SwarmEvent> {
    /// 从 SwarmEvent 变体名提取事件类型(覆盖泛型的 type_name 行为)。
    ///
    /// SwarmEvent 用 `#[serde(tag = "kind")]` 标签,序列化后 `kind`
    /// 字段即为变体名。此方法返回变体的 PascalCase 名。
    pub fn wrap_with_variant(event: SwarmEvent) -> Self {
        let trace_id = get_current_trace_id()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace("-", ""));
        let event_type = match &event {
            SwarmEvent::AgentStarted { .. } => "AgentStarted",
            SwarmEvent::AgentCompleted { .. } => "AgentCompleted",
            SwarmEvent::NegotiationStarted { .. } => "NegotiationStarted",
            SwarmEvent::ArbitrationResolved { .. } => "ArbitrationResolved",
            SwarmEvent::AgentToolCall { .. } => "AgentToolCall",
            SwarmEvent::AgentOutputChunk { .. } => "AgentOutputChunk",
            SwarmEvent::SwarmCompleted { .. } => "SwarmCompleted",
            SwarmEvent::DeadlockDetected { .. } => "DeadlockDetected",
            SwarmEvent::TreeOfThoughtsStarted { .. } => "TreeOfThoughtsStarted",
            SwarmEvent::PathCompleted { .. } => "PathCompleted",
        };
        Self {
            event_type: event_type.to_string(),
            payload: event,
            trace_id,
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// Swarm 执行流程的结构化事件。
///
/// 序列化为 JSON 后通过 `tauri::ipc::Channel` 推送给前端;
/// 前端根据 `kind` 字段做分支渲染。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SwarmEvent {
    /// 单个 Agent 开始执行。
    AgentStarted {
        agent_kind: AgentKind,
        task_id: String,
        timestamp: i64,
    },
    /// 单个 Agent 完成（成功或失败）。
    AgentCompleted {
        agent_kind: AgentKind,
        task_id: String,
        success: bool,
        /// 失败时的错误摘要（成功为 None）。
        error: Option<String>,
        timestamp: i64,
    },
    /// 进入协商阶段（outputs > 1）。
    NegotiationStarted {
        task_id: String,
        candidate_count: usize,
        timestamp: i64,
    },
    /// LLM 仲裁完成。
    ArbitrationResolved {
        task_id: String,
        chosen_kind: AgentKind,
        method: NegotiationMethod,
        conflict_detected: bool,
        timestamp: i64,
    },
    /// 单个 Agent 调用工具（Skill 或内置工具）。
    AgentToolCall {
        agent_id: String,
        agent_role: String,
        tool_name: String,
        start_ts: i64,
        end_ts: i64,
        duration_ms: u64,
        success: bool,
        output_preview: Option<String>,
        error: Option<String>,
        task_id: String,
    },
    /// Agent 输出流的增量块（为未来流式做准备，本期不发实际 chunk）。
    AgentOutputChunk {
        agent_id: String,
        delta: String,
        ts: i64,
        task_id: String,
    },
    /// 整轮 Swarm 执行结束。
    SwarmCompleted {
        task_id: String,
        success_count: u32,
        failure_count: u32,
        approved: bool,
        timestamp: i64,
    },
    /// T-E-S-05: 检测到死锁（WFG 中发现环）。
    DeadlockDetected {
        cycle: Vec<String>,
        detected_at: i64,
    },
    /// T-E-B-18: 思维树模式启动 — fan-out N 个 ThoughtAgent。
    TreeOfThoughtsStarted {
        task_id: String,
        branches: u32,
        timestamp: i64,
    },
    /// T-E-B-18: 单个思维路径完成。
    PathCompleted {
        task_id: String,
        path_id: String,
        strategy: ThoughtStrategy,
        timestamp: i64,
    },
}

impl SwarmEvent {
    /// 当前时间的 Unix 毫秒时间戳（内部使用）。
    pub fn now_ts() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    pub fn agent_started(agent_kind: AgentKind, task_id: impl Into<String>) -> Self {
        Self::AgentStarted {
            agent_kind,
            task_id: task_id.into(),
            timestamp: Self::now_ts(),
        }
    }

    pub fn agent_completed(
        agent_kind: AgentKind,
        task_id: impl Into<String>,
        success: bool,
        error: Option<String>,
    ) -> Self {
        Self::AgentCompleted {
            agent_kind,
            task_id: task_id.into(),
            success,
            error,
            timestamp: Self::now_ts(),
        }
    }

    pub fn negotiation_started(task_id: impl Into<String>, candidate_count: usize) -> Self {
        Self::NegotiationStarted {
            task_id: task_id.into(),
            candidate_count,
            timestamp: Self::now_ts(),
        }
    }

    pub fn arbitration_resolved(
        task_id: impl Into<String>,
        chosen_kind: AgentKind,
        method: NegotiationMethod,
        conflict_detected: bool,
    ) -> Self {
        Self::ArbitrationResolved {
            task_id: task_id.into(),
            chosen_kind,
            method,
            conflict_detected,
            timestamp: Self::now_ts(),
        }
    }

    pub fn swarm_completed(
        task_id: impl Into<String>,
        success_count: u32,
        failure_count: u32,
        approved: bool,
    ) -> Self {
        Self::SwarmCompleted {
            task_id: task_id.into(),
            success_count,
            failure_count,
            approved,
            timestamp: Self::now_ts(),
        }
    }

    /// T-E-D-10: 工具调用事件构造函数。
    pub fn agent_tool_call(
        agent_id: impl Into<String>,
        agent_role: impl Into<String>,
        tool_name: impl Into<String>,
        start_ts: i64,
        end_ts: i64,
        duration_ms: u64,
        success: bool,
        output_preview: Option<String>,
        error: Option<String>,
        task_id: impl Into<String>,
    ) -> Self {
        Self::AgentToolCall {
            agent_id: agent_id.into(),
            agent_role: agent_role.into(),
            tool_name: tool_name.into(),
            start_ts,
            end_ts,
            duration_ms,
            success,
            output_preview,
            error,
            task_id: task_id.into(),
        }
    }

    /// T-E-D-10: Agent 输出流增量块（为未来流式准备）。
    pub fn agent_output_chunk(
        agent_id: impl Into<String>,
        delta: impl Into<String>,
        task_id: impl Into<String>,
    ) -> Self {
        Self::AgentOutputChunk {
            agent_id: agent_id.into(),
            delta: delta.into(),
            ts: Self::now_ts(),
            task_id: task_id.into(),
        }
    }

    /// T-E-S-05: 死锁检测事件构造函数。
    pub fn deadlock_detected(cycle: Vec<String>, detected_at: i64) -> Self {
        Self::DeadlockDetected {
            cycle,
            detected_at,
        }
    }

    /// T-E-B-18: 思维树启动事件构造函数。
    pub fn tree_of_thoughts_started(task_id: impl Into<String>, branches: u32) -> Self {
        Self::TreeOfThoughtsStarted {
            task_id: task_id.into(),
            branches,
            timestamp: Self::now_ts(),
        }
    }

    /// T-E-B-18: 单个思维路径完成事件构造函数。
    pub fn path_completed(
        task_id: impl Into<String>,
        path_id: impl Into<String>,
        strategy: ThoughtStrategy,
    ) -> Self {
        Self::PathCompleted {
            task_id: task_id.into(),
            path_id: path_id.into(),
            strategy,
            timestamp: Self::now_ts(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_started_serializes_with_kind_tag() {
        let evt = SwarmEvent::agent_started(AgentKind::Generic, "task-1");
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"agent_started\""), "got: {s}");
        assert!(s.contains("\"agent_kind\":\"generic\""));
        assert!(s.contains("\"task_id\":\"task-1\""));
    }

    #[test]
    fn agent_completed_with_error_field() {
        let evt = SwarmEvent::agent_completed(
            AgentKind::Coder,
            "task-2",
            false,
            Some("timeout".to_string()),
        );
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"agent_completed\""));
        assert!(s.contains("\"success\":false"));
        assert!(s.contains("\"error\":\"timeout\""));
    }

    #[test]
    fn agent_completed_success_omits_error_via_none() {
        let evt = SwarmEvent::agent_completed(AgentKind::Reviewer, "t", true, None);
        let v: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(v["kind"], "agent_completed");
        assert_eq!(v["success"], true);
        assert!(v["error"].is_null());
    }

    #[test]
    fn arbitration_resolved_serializes_method() {
        let evt = SwarmEvent::arbitration_resolved(
            "t-3",
            AgentKind::Writer,
            NegotiationMethod::LlmArbitration,
            true,
        );
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"arbitration_resolved\""));
        assert!(s.contains("\"method\":\"llm_arbitration\""));
        assert!(s.contains("\"conflict_detected\":true"));
    }

    #[test]
    fn swarm_completed_carries_counts() {
        let evt = SwarmEvent::swarm_completed("t-4", 3, 1, true);
        let v: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(v["kind"], "swarm_completed");
        assert_eq!(v["success_count"], 3);
        assert_eq!(v["failure_count"], 1);
        assert_eq!(v["approved"], true);
    }

    #[test]
    fn agent_tool_call_serializes_correctly() {
        let start = SwarmEvent::now_ts();
        let end = start + 150;
        let evt = SwarmEvent::agent_tool_call(
            "agent-1",
            "coder",
            "read_file",
            start,
            end,
            150,
            true,
            Some("file content preview".to_string()),
            None,
            "task-123",
        );
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"agent_tool_call\""));
        assert!(s.contains("\"agent_id\":\"agent-1\""));
        assert!(s.contains("\"agent_role\":\"coder\""));
        assert!(s.contains("\"tool_name\":\"read_file\""));
        assert!(s.contains("\"duration_ms\":150"));
        assert!(s.contains("\"success\":true"));
        assert!(s.contains("\"task_id\":\"task-123\""));
    }

    #[test]
    fn agent_tool_call_failure_has_error_field() {
        let start = SwarmEvent::now_ts();
        let end = start + 50;
        let evt = SwarmEvent::agent_tool_call(
            "agent-2",
            "writer",
            "write_file",
            start,
            end,
            50,
            false,
            None,
            Some("permission denied".to_string()),
            "task-456",
        );
        let v: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(v["kind"], "agent_tool_call");
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "permission denied");
        assert!(v["output_preview"].is_null());
    }

    #[test]
    fn agent_output_chunk_serializes() {
        let evt = SwarmEvent::agent_output_chunk("agent-1", "hello", "task-789");
        let v: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(v["kind"], "agent_output_chunk");
        assert_eq!(v["agent_id"], "agent-1");
        assert_eq!(v["delta"], "hello");
        assert_eq!(v["task_id"], "task-789");
    }

    // T-E-B-18: TreeOfThoughtsStarted / PathCompleted 序列化测试。

    #[test]
    fn tree_of_thoughts_started_serializes() {
        let evt = SwarmEvent::tree_of_thoughts_started("task-tot-1", 4);
        let s = serde_json::to_string(&evt).unwrap();
        assert!(s.contains("\"kind\":\"tree_of_thoughts_started\""), "got: {s}");
        assert!(s.contains("\"task_id\":\"task-tot-1\""));
        assert!(s.contains("\"branches\":4"));
        assert!(s.contains("\"timestamp\":"));
    }

    #[test]
    fn path_completed_serializes_with_strategy() {
        let evt = SwarmEvent::path_completed("task-tot-1", "path-0", ThoughtStrategy::Analytical);
        let v: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(v["kind"], "path_completed");
        assert_eq!(v["task_id"], "task-tot-1");
        assert_eq!(v["path_id"], "path-0");
        // ThoughtStrategy 用 snake_case 序列化。
        assert_eq!(v["strategy"], "analytical");
        assert!(v["timestamp"].is_i64());
    }

    #[test]
    fn path_completed_serializes_all_four_strategies() {
        for (strategy, expected) in [
            (ThoughtStrategy::Analytical, "analytical"),
            (ThoughtStrategy::Creative, "creative"),
            (ThoughtStrategy::Critical, "critical"),
            (ThoughtStrategy::Synthesis, "synthesis"),
        ] {
            let evt = SwarmEvent::path_completed("task-tot-1", "path-0", strategy);
            let v: serde_json::Value = serde_json::to_value(&evt).unwrap();
            assert_eq!(v["kind"], "path_completed");
            assert_eq!(v["strategy"], expected, "strategy {expected} mismatch");
        }
    }

    // -----------------------------------------------------------------------
    // T-E-S-26: EventEnvelope 单测
    // -----------------------------------------------------------------------

    /// test_event_envelope_wrap: wrap(SwarmEvent::AgentStarted{...}) →
    /// envelope 含正确 event_type / timestamp。
    #[test]
    fn test_event_envelope_wrap() {
        let evt = SwarmEvent::agent_started(AgentKind::Coder, "task-wrap");
        let envelope = EventEnvelope::wrap_with_variant(evt);
        assert_eq!(envelope.event_type, "AgentStarted");
        assert!(envelope.timestamp > 0, "timestamp should be positive");
        assert!(!envelope.trace_id.is_empty(), "trace_id should not be empty");
        // payload should be AgentStarted variant
        match &envelope.payload {
            SwarmEvent::AgentStarted { task_id, .. } => {
                assert_eq!(task_id, "task-wrap");
            }
            _ => panic!("expected AgentStarted variant"),
        }
    }

    /// test_event_envelope_trace_id_fallback: 无 OTel 时 → UUID 格式 trace_id。
    /// UUID v4 去掉连字符后为 32 字符 hex。
    #[test]
    fn test_event_envelope_trace_id_fallback() {
        let evt = SwarmEvent::agent_completed(AgentKind::Writer, "t-fallback", true, None);
        let envelope = EventEnvelope::wrap_with_variant(evt);
        // 无 OTel 时 trace_id 为 UUID v4 去连字符(32 字符 hex)
        assert_eq!(envelope.trace_id.len(), 32, "fallback trace_id should be 32 hex chars");
        assert!(
            envelope.trace_id.chars().all(|c| c.is_ascii_hexdigit()),
            "fallback trace_id should be hex: got {}",
            envelope.trace_id
        );
    }

    /// test_event_envelope_serialization: serialize + deserialize round-trip。
    #[test]
    fn test_event_envelope_serialization() {
        let evt = SwarmEvent::swarm_completed("t-serial", 2, 0, true);
        let envelope = EventEnvelope::wrap_with_variant(evt);
        let json = serde_json::to_string(&envelope).expect("serialize");
        let de: EventEnvelope<SwarmEvent> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.event_type, "SwarmCompleted");
        assert_eq!(de.trace_id, envelope.trace_id);
        assert_eq!(de.timestamp, envelope.timestamp);
        match &de.payload {
            SwarmEvent::SwarmCompleted { task_id, .. } => {
                assert_eq!(task_id, "t-serial");
            }
            _ => panic!("expected SwarmCompleted variant"),
        }
    }
}
