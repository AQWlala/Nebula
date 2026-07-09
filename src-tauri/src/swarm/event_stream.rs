//! T-E-S-26: Event Stream 协议化模块。
//!
//! 将蜂群事件流标准化为协议化格式,支持版本化、过滤、回放。
//!
//! 本模块定义与 `swarm::events::EventEnvelope<T>`(泛型、面向前端推送)
//! 不同的**协议化**信封(非泛型、传输层包装):payload 序列化为
//! `serde_json::Value`,携带 `protocol_version` / `event_id` / `metadata`
//! 等字段,用于事件流的版本化、过滤、回放与跨进程传输。
//!
//! ## 组成
//!
//! - [`EventEnvelope`] — 协议化事件信封(版本化、可追溯)
//! - [`EventStreamFilter`] — 事件过滤条件(source / event_type / trace_id / since / limit)
//! - [`EventStreamCodec`] — NDJSON 编解码器(单条 / 批量)
//! - [`EventStreamReplay`] — 事件回放器(全量 / 按过滤条件)
//! - [`EventStreamBuffer`] — 线程安全环形缓冲区(基于 `tokio::sync::RwLock`)

use std::collections::{HashMap, VecDeque};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use uuid::Uuid;

/// 当前协议版本。
pub const CURRENT_PROTOCOL_VERSION: u8 = 2;

// ---------------------------------------------------------------------------
// EventEnvelope — 协议化事件信封
// ---------------------------------------------------------------------------

/// T-E-S-26: 协议化事件信封。
///
/// 与 `swarm::events::EventEnvelope<T>`(泛型、面向前端 `ipc::Channel` 推送)
/// 不同,本信封为**非泛型**传输层包装:`payload` 序列化为 `serde_json::Value`,
/// 携带 `protocol_version`、`event_id`、`metadata` 等字段,用于事件流的
/// 版本化、过滤、回放与跨进程传输。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// 协议版本(当前 = [`CURRENT_PROTOCOL_VERSION`] = 2)。
    pub protocol_version: u8,
    /// 事件唯一 ID(UUID v4)。
    pub event_id: Uuid,
    /// 事件产生时间(UTC)。
    pub timestamp: DateTime<Utc>,
    /// 事件来源(如 "orchestrator"、"agent:coder")。
    pub source: String,
    /// 事件类型名(如 "AgentStarted"、"SwarmCompleted")。
    pub event_type: String,
    /// 事件负载(任意 JSON 值)。
    pub payload: Value,
    /// 附加元数据(键值对,如 "task_id"、"session_id")。
    pub metadata: HashMap<String, String>,
    /// OTel trace_id(可选)。`None` 表示无追踪上下文。
    pub trace_id: Option<String>,
}

impl EventEnvelope {
    /// 用当前协议版本构造一个新信封,自动生成 `event_id` 与 `timestamp`。
    pub fn new(source: impl Into<String>, event_type: impl Into<String>, payload: Value) -> Self {
        Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            source: source.into(),
            event_type: event_type.into(),
            payload,
            metadata: HashMap::new(),
            trace_id: None,
        }
    }

    /// 设置 metadata(覆盖)。
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// 设置 trace_id。
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// 设置 protocol_version(主要用于测试旧版本兼容)。
    pub fn with_protocol_version(mut self, version: u8) -> Self {
        self.protocol_version = version;
        self
    }
}

// ---------------------------------------------------------------------------
// EventStreamFilter — 事件过滤条件
// ---------------------------------------------------------------------------

/// T-E-S-26: 事件流过滤条件。
///
/// 所有字段均为 `Option`;`None` 表示该维度不参与过滤(即接受任意值)。
/// `limit` 是结果集级别的约束,在 [`EventStreamFilter::matches`] 之外
/// 由调用方(replay / buffer query)应用。
#[derive(Debug, Clone, Default)]
pub struct EventStreamFilter {
    /// 按来源精确匹配。
    pub by_source: Option<String>,
    /// 按事件类型匹配(任一命中即可,IN 语义)。
    pub by_event_type: Option<Vec<String>>,
    /// 按 trace_id 精确匹配。
    pub by_trace_id: Option<String>,
    /// 仅保留 `timestamp >= since` 的事件。
    pub since: Option<DateTime<Utc>>,
    /// 结果最多返回的条数(在 matches 之后应用)。
    pub limit: Option<usize>,
}

impl EventStreamFilter {
    /// 构造一个空过滤条件(匹配所有事件)。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置 by_source。
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.by_source = Some(source.into());
        self
    }

    /// 设置 by_event_type。
    pub fn event_types(mut self, types: Vec<String>) -> Self {
        self.by_event_type = Some(types);
        self
    }

    /// 设置 by_trace_id。
    pub fn trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.by_trace_id = Some(trace_id.into());
        self
    }

    /// 设置 since。
    pub fn since(mut self, since: DateTime<Utc>) -> Self {
        self.since = Some(since);
        self
    }

    /// 设置 limit。
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// 判断单个信封是否满足过滤条件。
    ///
    /// 注意:本方法只评估 per-envelope 谓词(source / event_type / trace_id /
    /// since);`limit` 是结果集级别的约束,由调用方(replay / buffer query)应用。
    pub fn matches(&self, envelope: &EventEnvelope) -> bool {
        if let Some(ref src) = self.by_source {
            if &envelope.source != src {
                return false;
            }
        }
        if let Some(ref types) = self.by_event_type {
            if !types.iter().any(|t| t == &envelope.event_type) {
                return false;
            }
        }
        if let Some(ref tid) = self.by_trace_id {
            match &envelope.trace_id {
                Some(eid) if eid == tid => {}
                _ => return false,
            }
        }
        if let Some(since) = self.since {
            if envelope.timestamp < since {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// EventStreamCodec — NDJSON 编解码器
// ---------------------------------------------------------------------------

/// T-E-S-26: 事件流 NDJSON 编解码器。
///
/// - `encode` / `encode_batch` 产出以 `\n` 分隔的 JSON 行(NDJSON)。
/// - `decode` 解析单行(允许尾随 `\n` 或空白)。
/// - 解码时校验 `protocol_version`:接受 `<= CURRENT_PROTOCOL_VERSION` 的版本
///   (向后兼容旧版本,拒绝未来版本)。
#[derive(Debug, Clone, Default)]
pub struct EventStreamCodec;

impl EventStreamCodec {
    /// 构造编解码器。
    pub fn new() -> Self {
        Self
    }

    /// 编码单个信封为 NDJSON 行(以 `\n` 结尾)。
    pub fn encode(&self, envelope: &EventEnvelope) -> Result<Vec<u8>> {
        let mut json =
            serde_json::to_vec(envelope).map_err(|e| anyhow!("序列化 EventEnvelope 失败: {e}"))?;
        json.push(b'\n');
        Ok(json)
    }

    /// 解码单行 NDJSON(允许尾随 `\n` 或空白)为信封。
    ///
    /// 版本校验:接受 `protocol_version <= CURRENT_PROTOCOL_VERSION`,
    /// 拒绝未来版本(返回错误)。
    pub fn decode(&self, data: &[u8]) -> Result<EventEnvelope> {
        let trimmed = std::str::from_utf8(data)
            .map_err(|e| anyhow!("NDJSON 行非 UTF-8: {e}"))?
            .trim();
        let envelope: EventEnvelope = serde_json::from_str(trimmed)
            .map_err(|e| anyhow!("反序列化 EventEnvelope 失败: {e}"))?;
        if envelope.protocol_version > CURRENT_PROTOCOL_VERSION {
            warn!(
                version = envelope.protocol_version,
                current = CURRENT_PROTOCOL_VERSION,
                "拒绝未来协议版本的事件信封"
            );
            return Err(anyhow!(
                "不支持的协议版本: {} > 当前版本 {}",
                envelope.protocol_version,
                CURRENT_PROTOCOL_VERSION
            ));
        }
        Ok(envelope)
    }

    /// 批量编码为 NDJSON(每行一个信封,均以 `\n` 结尾)。
    pub fn encode_batch(&self, envelopes: &[EventEnvelope]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for env in envelopes {
            out.extend_from_slice(&self.encode(env)?);
        }
        Ok(out)
    }

    /// 批量解码 NDJSON(每行一个信封,跳过空行)。供回放/测试使用。
    pub fn decode_batch(&self, data: &[u8]) -> Result<Vec<EventEnvelope>> {
        let text =
            std::str::from_utf8(data).map_err(|e| anyhow!("NDJSON 批量数据非 UTF-8: {e}"))?;
        let mut result = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            result.push(self.decode(line.as_bytes())?);
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// EventStreamReplay — 事件回放器
// ---------------------------------------------------------------------------

/// T-E-S-26: 事件回放器。
///
/// 持有一段已捕获的事件序列,支持全量回放或按过滤条件回放。
/// 不可变结构,适合在快照上做确定性回放。
#[derive(Debug, Clone)]
pub struct EventStreamReplay {
    events: Vec<EventEnvelope>,
}

impl EventStreamReplay {
    /// 用事件序列构造回放器。
    pub fn new(events: Vec<EventEnvelope>) -> Self {
        debug!(count = events.len(), "构造 EventStreamReplay");
        Self { events }
    }

    /// 按过滤条件回放,返回匹配事件的引用切片。
    ///
    /// `limit` 在过滤之后应用。
    pub fn replay_with_filter(&self, filter: &EventStreamFilter) -> Vec<&EventEnvelope> {
        let mut matched: Vec<&EventEnvelope> =
            self.events.iter().filter(|e| filter.matches(e)).collect();
        if let Some(limit) = filter.limit {
            matched.truncate(limit);
        }
        debug!(
            total = self.events.len(),
            matched = matched.len(),
            "replay_with_filter 完成"
        );
        matched
    }

    /// 全量回放,返回事件切片引用。
    pub fn replay_all(&self) -> &[EventEnvelope] {
        &self.events
    }

    /// 返回事件总数。
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// ---------------------------------------------------------------------------
// EventStreamBuffer — 线程安全环形缓冲区
// ---------------------------------------------------------------------------

/// T-E-S-26: 事件流环形缓冲区。
///
/// 基于 `tokio::sync::RwLock` 的线程安全环形缓冲区。容量固定,超出时
/// 淘汰最旧的事件(头部)。支持并发 `push` 与 `query`。
///
/// 注:同步方法内部使用 `tokio::sync::RwLock::blocking_read` /
/// `blocking_write`,适用于在异步运行时之外(如 OS 线程)调用。
pub struct EventStreamBuffer {
    /// 内部环形队列(VecDeque,头部为最旧,尾部为最新)。
    inner: RwLock<VecDeque<EventEnvelope>>,
    /// 容量上限。
    capacity: usize,
}

impl EventStreamBuffer {
    /// 用指定容量构造缓冲区。`capacity = 0` 会被 clamp 到 1。
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// 返回容量上限。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 推入一个事件。若缓冲区已满,淘汰最旧的事件。
    pub fn push(&self, envelope: EventEnvelope) {
        let mut guard = self.inner.blocking_write();
        if guard.len() >= self.capacity {
            if let Some(evicted) = guard.pop_front() {
                debug!(
                    event_id = %evicted.event_id,
                    capacity = self.capacity,
                    "缓冲区已满,淘汰最旧事件"
                );
            }
        }
        guard.push_back(envelope);
    }

    /// 按过滤条件查询事件(返回克隆,保持内部所有权)。
    ///
    /// `limit` 在过滤之后应用。结果按插入顺序返回(最旧在前)。
    pub fn query(&self, filter: &EventStreamFilter) -> Vec<EventEnvelope> {
        let guard = self.inner.blocking_read();
        let mut matched: Vec<EventEnvelope> = guard
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect();
        if let Some(limit) = filter.limit {
            matched.truncate(limit);
        }
        matched
    }

    /// 当前缓冲区中的事件数量。
    pub fn len(&self) -> usize {
        self.inner.blocking_read().len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.inner.blocking_read().is_empty()
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::Arc;
    use std::thread;

    /// 构造一个测试用信封。
    fn make_envelope(source: &str, event_type: &str) -> EventEnvelope {
        EventEnvelope::new(source, event_type, serde_json::json!({"task_id": "t-1"}))
    }

    /// 构造一个带指定时间戳的测试信封。
    fn make_envelope_at(source: &str, event_type: &str, ts: DateTime<Utc>) -> EventEnvelope {
        let mut env = make_envelope(source, event_type);
        env.timestamp = ts;
        env
    }

    // 1. envelope 序列化往返
    #[test]
    fn test_envelope_serialization_roundtrip() {
        let mut env = make_envelope("orchestrator", "AgentStarted");
        env.metadata.insert("task_id".into(), "t-1".into());
        env.trace_id = Some("abcdef0123456789".into());
        let json = serde_json::to_string(&env).expect("序列化");
        let de: EventEnvelope = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(de.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(de.event_id, env.event_id);
        assert_eq!(de.timestamp, env.timestamp);
        assert_eq!(de.source, "orchestrator");
        assert_eq!(de.event_type, "AgentStarted");
        assert_eq!(de.trace_id, env.trace_id);
        assert_eq!(de.metadata.get("task_id").map(|s| s.as_str()), Some("t-1"));
        assert_eq!(de.payload, env.payload);
    }

    // 2. filter 各种条件匹配
    #[test]
    fn test_filter_matches_all_conditions() {
        let early = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let mut env = make_envelope("orchestrator", "AgentStarted");
        env.trace_id = Some("trace-1".into());
        env.timestamp = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 10).unwrap();

        // 空过滤器匹配所有
        assert!(EventStreamFilter::new().matches(&env));
        // source 匹配
        assert!(EventStreamFilter::new()
            .source("orchestrator")
            .matches(&env));
        assert!(!EventStreamFilter::new().source("other").matches(&env));
        // event_type 匹配(IN 语义)
        assert!(EventStreamFilter::new()
            .event_types(vec!["AgentStarted".into(), "SwarmCompleted".into()])
            .matches(&env));
        assert!(!EventStreamFilter::new()
            .event_types(vec!["SwarmCompleted".into()])
            .matches(&env));
        // trace_id 匹配
        assert!(EventStreamFilter::new().trace_id("trace-1").matches(&env));
        assert!(!EventStreamFilter::new().trace_id("trace-2").matches(&env));
        // since 匹配(事件时间 >= since)
        assert!(EventStreamFilter::new().since(early).matches(&env));
        let later = Utc.with_ymd_and_hms(2024, 1, 1, 0, 5, 0).unwrap();
        assert!(!EventStreamFilter::new().since(later).matches(&env));
        // 组合条件全部满足
        let combo = EventStreamFilter::new()
            .source("orchestrator")
            .event_types(vec!["AgentStarted".into()])
            .trace_id("trace-1")
            .since(early);
        assert!(combo.matches(&env));
        // 组合条件中一项不满足
        let combo_fail = EventStreamFilter::new()
            .source("orchestrator")
            .event_types(vec!["SwarmCompleted".into()]);
        assert!(!combo_fail.matches(&env));
    }

    // 3. trace_id 过滤专项
    #[test]
    fn test_filter_by_trace_id() {
        let mut env_with = make_envelope("src", "E");
        env_with.trace_id = Some("trace-xyz".into());
        let env_without = make_envelope("src", "E"); // trace_id = None

        let f = EventStreamFilter::new().trace_id("trace-xyz");
        assert!(f.matches(&env_with));
        assert!(
            !f.matches(&env_without),
            "无 trace_id 的事件不应匹配 by_trace_id 过滤"
        );
        // 不同的 trace_id 不匹配
        let f2 = EventStreamFilter::new().trace_id("other");
        assert!(!f2.matches(&env_with));
    }

    // 4. codec 编解码往返
    #[test]
    fn test_codec_encode_decode_roundtrip() {
        let codec = EventStreamCodec::new();
        let mut env = make_envelope("orchestrator", "AgentCompleted");
        env.metadata.insert("k".into(), "v".into());
        env.trace_id = Some("trace-1".into());
        let encoded = codec.encode(&env).expect("encode");
        // NDJSON 行以 \n 结尾
        assert!(encoded.ends_with(b"\n"), "NDJSON 行应以 \\n 结尾");
        let decoded = codec.decode(&encoded).expect("decode");
        assert_eq!(decoded.event_id, env.event_id);
        assert_eq!(decoded.source, env.source);
        assert_eq!(decoded.event_type, env.event_type);
        assert_eq!(decoded.trace_id, env.trace_id);
        assert_eq!(decoded.metadata, env.metadata);
        assert_eq!(decoded.protocol_version, env.protocol_version);
        assert_eq!(decoded.timestamp, env.timestamp);
    }

    // 5. batch 编解码
    #[test]
    fn test_codec_batch_encode_decode() {
        let codec = EventStreamCodec::new();
        let envelopes: Vec<EventEnvelope> = (0..5)
            .map(|i| EventEnvelope::new("src", "E", serde_json::json!({"i": i})))
            .collect();
        let batch = codec.encode_batch(&envelopes).expect("encode_batch");
        // 5 行,每行以 \n 结尾(serde_json 默认紧凑输出,JSON 内部无 \n)
        let line_count = batch.iter().filter(|&&b| b == b'\n').count();
        assert_eq!(line_count, 5);
        let decoded = codec.decode_batch(&batch).expect("decode_batch");
        assert_eq!(decoded.len(), 5);
        for (a, b) in envelopes.iter().zip(decoded.iter()) {
            assert_eq!(a.event_id, b.event_id);
            assert_eq!(a.payload, b.payload);
            assert_eq!(a.event_type, b.event_type);
        }
    }

    // 6. protocol_version 兼容性
    #[test]
    fn test_protocol_version_compatibility() {
        let codec = EventStreamCodec::new();
        // 当前版本(v2)可正常解码
        let env_v2 = make_envelope("src", "E");
        let enc2 = codec.encode(&env_v2).unwrap();
        assert!(codec.decode(&enc2).is_ok());

        // 旧版本(v1)向后兼容,可解码
        let env_v1 = make_envelope("src", "E").with_protocol_version(1);
        let json_v1 = serde_json::to_string(&env_v1).unwrap();
        let decoded_v1 = codec.decode(json_v1.as_bytes()).expect("v1 应可解码");
        assert_eq!(decoded_v1.protocol_version, 1);

        // 未来版本(v3)被拒绝
        let env_v3 = make_envelope("src", "E").with_protocol_version(3);
        let json_v3 = serde_json::to_string(&env_v3).unwrap();
        let err = codec.decode(json_v3.as_bytes());
        assert!(err.is_err(), "未来版本应被拒绝");
    }

    // 7. replay 过滤
    #[test]
    fn test_replay_with_filter() {
        let base = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let events = vec![
            make_envelope_at("orchestrator", "AgentStarted", base),
            make_envelope_at(
                "agent:coder",
                "AgentCompleted",
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap(),
            ),
            make_envelope_at(
                "orchestrator",
                "SwarmCompleted",
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 2).unwrap(),
            ),
            make_envelope_at(
                "agent:writer",
                "AgentStarted",
                Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 3).unwrap(),
            ),
        ];
        let replay = EventStreamReplay::new(events);

        // 全量回放
        assert_eq!(replay.replay_all().len(), 4);
        assert_eq!(replay.len(), 4);
        assert!(!replay.is_empty());

        // 按 source 过滤
        let f = EventStreamFilter::new().source("orchestrator");
        let r = replay.replay_with_filter(&f);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|e| e.source == "orchestrator"));

        // 按 event_type 过滤
        let f = EventStreamFilter::new().event_types(vec!["AgentStarted".into()]);
        assert_eq!(replay.replay_with_filter(&f).len(), 2);

        // 按 since 过滤(>= 00:00:02)
        let f = EventStreamFilter::new().since(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 2).unwrap());
        assert_eq!(replay.replay_with_filter(&f).len(), 2);

        // limit 应用
        let f = EventStreamFilter::new().source("orchestrator").limit(1);
        assert_eq!(replay.replay_with_filter(&f).len(), 1);

        // 空过滤返回全部
        assert_eq!(
            replay.replay_with_filter(&EventStreamFilter::new()).len(),
            4
        );
    }

    // 8. buffer 容量淘汰
    #[test]
    fn test_buffer_eviction() {
        let buf = EventStreamBuffer::new(3);
        for i in 0..5 {
            buf.push(EventEnvelope::new("src", "E", serde_json::json!({"i": i})));
        }
        // 容量 3,push 5 次后应只剩最后 3 个(i=2,3,4)
        assert_eq!(buf.len(), 3);
        let all = buf.query(&EventStreamFilter::new());
        assert_eq!(all.len(), 3);
        // 验证保留的是最新的(按 payload i)
        let ids: Vec<i64> = all
            .iter()
            .map(|e| e.payload["i"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![2, 3, 4], "应淘汰最旧的 i=0,1");
    }

    // 9. 空 buffer 查询
    #[test]
    fn test_empty_buffer_query() {
        let buf = EventStreamBuffer::new(16);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        let result = buf.query(&EventStreamFilter::new());
        assert!(result.is_empty(), "空 buffer 查询应返回空");
        // 带过滤条件的空 buffer 查询也应返回空
        let result2 = buf.query(&EventStreamFilter::new().source("x"));
        assert!(result2.is_empty());
    }

    // 10. 并发读写(OS 线程,避免 tokio runtime blocking 限制)
    #[test]
    fn test_buffer_concurrent_push_and_query() {
        let buf = Arc::new(EventStreamBuffer::new(200));
        let mut handles = Vec::new();

        // 4 个写线程,各 push 50 条,总计 200 条
        for t in 0..4u32 {
            let buf = Arc::clone(&buf);
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    buf.push(EventEnvelope::new(
                        format!("writer-{t}"),
                        "E",
                        serde_json::json!({"t": t, "i": i}),
                    ));
                }
            }));
        }
        // 1 个读线程在写的同时不断查询
        let buf_r = Arc::clone(&buf);
        let reader = thread::spawn(move || {
            for _ in 0..20 {
                let n = buf_r.len();
                assert!(n <= 200, "不应超过容量");
                let _ = buf_r.query(&EventStreamFilter::new());
            }
        });

        for h in handles {
            h.join().expect("writer thread panicked");
        }
        reader.join().expect("reader thread panicked");

        // 总共 push 4*50=200,容量 200,最终应正好 200(无淘汰)
        assert_eq!(buf.len(), 200);
        let all = buf.query(&EventStreamFilter::new());
        assert_eq!(all.len(), 200);
    }

    // 11. 容量 clamp 到 1
    #[test]
    fn test_buffer_capacity_clamped_to_one() {
        let buf = EventStreamBuffer::new(0);
        assert_eq!(buf.capacity(), 1);
        buf.push(make_envelope("s", "E"));
        buf.push(make_envelope("s", "E"));
        // 容量 1,只保留最新
        assert_eq!(buf.len(), 1);
    }

    // 12. codec decode 错误处理
    #[test]
    fn test_codec_decode_invalid_input() {
        let codec = EventStreamCodec::new();
        // 非 UTF-8
        assert!(codec.decode(&[0xFF, 0xFE]).is_err());
        // 非法 JSON
        assert!(codec.decode(b"not json").is_err());
        // 空行(trim 后为空,serde_json 报 EOF)
        assert!(codec.decode(b"   \n").is_err());
    }

    // 13. EventEnvelope builder 链式构造
    #[test]
    fn test_envelope_builder() {
        let mut meta = HashMap::new();
        meta.insert("session_id".into(), "s-1".into());
        let env = EventEnvelope::new("src", "E", serde_json::json!({"x": 1}))
            .with_metadata(meta)
            .with_trace_id("trace-abc");
        assert_eq!(env.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(
            env.metadata.get("session_id").map(|s| s.as_str()),
            Some("s-1")
        );
        assert_eq!(env.trace_id.as_deref(), Some("trace-abc"));
    }
}
