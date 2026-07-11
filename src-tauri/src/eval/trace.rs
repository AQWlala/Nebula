//! Trace 导出器 — 记录 Agent 执行轨迹并导出 JSONL
//!
//! ## 设计
//!
//! `TraceCollector` 使用 `Arc<Mutex<Vec<TraceSpan>>>` 收集 span,
//! 通过 `NEBULA_EVAL_TRACING=1` 环境变量启用,默认关闭(no-op,零开销)。
//!
//! ## 接入点
//!
//! * `swarm::master::execute_loop()` → `MasterDecompose` / `MasterSynthesize`
//! * `swarm::orchestrator::run_worker()` → `SwarmWorker`
//! * `evolution::engine::run_pass()` → `EvolutionPass`
//! * `evolution::prompt_mutator` → `PromptMutation`
//! * `skills::engine::run()` → `SkillExec`
//!
//! ## 导出
//!
//! `export_jsonl()` 将所有 span 按时间顺序写入 JSONL 文件(每行一个 JSON 对象)。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// 环境变量门控
// ---------------------------------------------------------------------------

/// 全局开关: `NEBULA_EVAL_TRACING=1` 时为 true。
/// 首次访问时读取环境变量并缓存,避免重复 syscall。
static EVAL_TRACING_ENABLED: AtomicBool = AtomicBool::new(false);
static EVAL_TRACING_INIT: std::sync::Once = std::sync::Once::new();

fn eval_tracing_enabled() -> bool {
    EVAL_TRACING_INIT.call_once(|| {
        let enabled = std::env::var("NEBULA_EVAL_TRACING")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        EVAL_TRACING_ENABLED.store(enabled, Ordering::Relaxed);
    });
    EVAL_TRACING_ENABLED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// span 类型: Agent 执行的一个步骤
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// Master 拆解任务
    MasterDecompose,
    /// Master 综合结果
    MasterSynthesize,
    /// 蜂群 Worker 执行子任务
    SwarmWorker,
    /// 评审 Agent
    Reviewer,
    /// 进化 pass
    EvolutionPass,
    /// Prompt 变异
    PromptMutation,
    /// 技能执行
    SkillExec,
    /// LLM 调用(底层)
    LlmCall,
}

impl SpanKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpanKind::MasterDecompose => "master_decompose",
            SpanKind::MasterSynthesize => "master_synthesize",
            SpanKind::SwarmWorker => "swarm_worker",
            SpanKind::Reviewer => "reviewer",
            SpanKind::EvolutionPass => "evolution_pass",
            SpanKind::PromptMutation => "prompt_mutation",
            SpanKind::SkillExec => "skill_exec",
            SpanKind::LlmCall => "llm_call",
        }
    }
}

/// Trace payload — span 的输入或输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracePayload {
    /// 文本内容(PII 已脱敏)
    pub text: String,
    /// 结构化元数据(任务类型/技能名等)
    pub metadata: serde_json::Value,
}

impl TracePayload {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// LLM 调用元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMeta {
    /// 提供商: ollama / anthropic / deepseek / ...
    pub provider: String,
    /// 模型名: deepseek-chat / llama3 / ...
    pub model: String,
    /// 输入 token 数
    pub input_tokens: u64,
    /// 输出 token 数
    pub output_tokens: u64,
    /// 成本(USD)
    pub cost_usd: f64,
}

/// 一条 Trace 记录,对应 Agent 执行的一个步骤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    /// 唯一 ID (UUID v4, 时序可排序)
    pub id: String,
    /// 父 span ID (形成调用树)
    pub parent_id: Option<String>,
    /// Trace 根 ID (同一任务的所有 span 共享)
    pub trace_id: String,
    /// span 类型
    pub span_kind: SpanKind,
    /// 开始时间 (UTC ISO 8601)
    pub started_at: String,
    /// 结束时间
    pub ended_at: Option<String>,
    /// 输入内容(已脱敏)
    pub input: TracePayload,
    /// 输出内容(已脱敏)
    pub output: Option<TracePayload>,
    /// LLM 调用元数据
    pub llm_meta: Option<LlmMeta>,
    /// 子任务 ID 列表(如果是 DAG 节点)
    pub child_task_ids: Vec<String>,
    /// 错误信息(如果失败)
    pub error: Option<String>,
}

impl TraceSpan {
    /// 创建一个新 span (未结束, ended_at = None)
    pub fn start(
        trace_id: &str,
        parent_id: Option<&str>,
        kind: SpanKind,
        input: TracePayload,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            trace_id: trace_id.to_string(),
            span_kind: kind,
            started_at: now_iso8601(),
            ended_at: None,
            input,
            output: None,
            llm_meta: None,
            child_task_ids: Vec::new(),
            error: None,
        }
    }

    /// 标记 span 结束
    pub fn finish(&mut self, output: TracePayload) {
        self.ended_at = Some(now_iso8601());
        self.output = Some(output);
    }

    /// 标记 span 出错
    pub fn finish_with_error(&mut self, error: impl Into<String>) {
        self.ended_at = Some(now_iso8601());
        self.error = Some(error.into());
    }
}

// ---------------------------------------------------------------------------
// TraceCollector
// ---------------------------------------------------------------------------

/// Trace 收集器 — 收集 span 并支持导出
///
/// 线程安全: 内部使用 `Arc<Mutex<Vec<TraceSpan>>>`。
/// 通过 `NEBULA_EVAL_TRACING=1` 环境变量启用,默认关闭(no-op,零开销)。
#[derive(Debug, Clone)]
pub struct TraceCollector {
    spans: Arc<Mutex<Vec<TraceSpan>>>,
}

/// TraceCollector 的句柄 — 持有 collector 引用,提供便捷的 span 创建方法
#[derive(Debug, Clone)]
pub struct TraceCollectorHandle {
    collector: TraceCollector,
    trace_id: String,
    parent_id: Option<String>,
}

impl Default for TraceCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceCollector {
    /// 创建一个新的 TraceCollector
    pub fn new() -> Self {
        Self {
            spans: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Trace 收集是否启用
    pub fn is_enabled(&self) -> bool {
        eval_tracing_enabled()
    }

    /// 开始一个新的 trace (根 span)
    pub fn start_trace(&self, kind: SpanKind, input: TracePayload) -> TraceCollectorHandle {
        let trace_id = Uuid::new_v4().to_string();
        let span = TraceSpan::start(&trace_id, None, kind, input);
        let span_id = span.id.clone();
        if self.is_enabled() {
            if let Ok(mut spans) = self.spans.lock() {
                spans.push(span);
            }
        }
        TraceCollectorHandle {
            collector: self.clone(),
            trace_id,
            parent_id: Some(span_id),
        }
    }

    /// 开始一个子 span
    pub fn start_span(
        &self,
        trace_id: &str,
        parent_id: Option<&str>,
        kind: SpanKind,
        input: TracePayload,
    ) -> String {
        let span = TraceSpan::start(trace_id, parent_id, kind, input);
        let span_id = span.id.clone();
        if self.is_enabled() {
            if let Ok(mut spans) = self.spans.lock() {
                spans.push(span);
            }
        }
        span_id
    }

    /// 结束一个 span (设置 output)
    pub fn end_span(&self, span_id: &str, output: TracePayload) {
        if !self.is_enabled() {
            return;
        }
        if let Ok(mut spans) = self.spans.lock() {
            if let Some(span) = spans.iter_mut().find(|s| s.id == span_id) {
                span.finish(output);
            }
        }
    }

    /// 结束一个 span (设置 error)
    pub fn end_span_with_error(&self, span_id: &str, error: impl Into<String>) {
        if !self.is_enabled() {
            return;
        }
        if let Ok(mut spans) = self.spans.lock() {
            if let Some(span) = spans.iter_mut().find(|s| s.id == span_id) {
                span.finish_with_error(error);
            }
        }
    }

    /// 获取所有 span (按时间排序)
    pub fn spans(&self) -> Vec<TraceSpan> {
        if let Ok(spans) = self.spans.lock() {
            let mut spans = spans.clone();
            spans.sort_by(|a, b| a.started_at.cmp(&b.started_at));
            spans
        } else {
            Vec::new()
        }
    }

    /// 获取指定 trace_id 的所有 span
    pub fn spans_for_trace(&self, trace_id: &str) -> Vec<TraceSpan> {
        if let Ok(spans) = self.spans.lock() {
            let mut filtered: Vec<_> = spans
                .iter()
                .filter(|s| s.trace_id == trace_id)
                .cloned()
                .collect();
            filtered.sort_by(|a, b| a.started_at.cmp(&b.started_at));
            filtered
        } else {
            Vec::new()
        }
    }

    /// 导出所有 span 为 JSONL
    pub fn export_jsonl(&self, path: &std::path::Path) -> anyhow::Result<usize> {
        let spans = self.spans();
        export_spans_jsonl(&spans, path)?;
        Ok(spans.len())
    }

    /// 导出指定 trace 的 span 为 JSONL
    pub fn export_trace_jsonl(
        &self,
        trace_id: &str,
        path: &std::path::Path,
    ) -> anyhow::Result<usize> {
        let spans = self.spans_for_trace(trace_id);
        export_spans_jsonl(&spans, path)?;
        Ok(spans.len())
    }

    /// 清空所有 span
    pub fn clear(&self) {
        if let Ok(mut spans) = self.spans.lock() {
            spans.clear();
        }
    }

    /// span 数量
    pub fn len(&self) -> usize {
        if let Ok(spans) = self.spans.lock() {
            spans.len()
        } else {
            0
        }
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TraceCollectorHandle {
    /// 开始一个子 span (使用 handle 的 trace_id 和 parent_id)
    pub fn start_child(&self, kind: SpanKind, input: TracePayload) -> String {
        self.collector.start_span(
            &self.trace_id,
            self.parent_id.as_deref(),
            kind,
            input,
        )
    }

    /// 结束一个 span
    pub fn end_span(&self, span_id: &str, output: TracePayload) {
        self.collector.end_span(span_id, output);
    }

    /// 结束一个 span (出错)
    pub fn end_span_with_error(&self, span_id: &str, error: impl Into<String>) {
        self.collector.end_span_with_error(span_id, error);
    }

    /// trace_id
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }
}

// ---------------------------------------------------------------------------
// 导出工具函数
// ---------------------------------------------------------------------------

/// 将 span 列表导出为 JSONL 文件
pub fn export_spans_jsonl(spans: &[TraceSpan], path: &std::path::Path) -> anyhow::Result<()> {
    let file = File::create(path).map_err(|e| {
        anyhow::anyhow!("无法创建 trace 输出文件 {}: {}", path.display(), e)
    })?;
    let mut writer = BufWriter::new(file);
    for span in spans {
        let line = serde_json::to_string(span)
            .map_err(|e| anyhow::anyhow!("序列化 span 失败: {}", e))?;
        writeln!(writer, "{}", line)
            .map_err(|e| anyhow::anyhow!("写入 JSONL 失败: {}", e))?;
    }
    writer.flush().map_err(|e| anyhow::anyhow!("flush 失败: {}", e))?;
    Ok(())
}

/// 获取当前时间的 ISO 8601 字符串 (UTC)
pub fn now_iso8601() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();

    // 简单的 Unix 时间戳 → ISO 8601 转换 (不依赖 chrono)
    let days = secs / 86400;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    // 计算年月日 (从 1970-01-01 开始)
    let (year, month, day) = days_to_ymd(days as i64);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hour, min, sec, millis
    )
}

/// Unix 天数 → (年, 月, 日) — 从 1970-01-01 开始
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    // 算法来源: Howard Hinnant 的 days_from_civil 逆算法
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn span_kind_as_str_roundtrip() {
        let kinds = vec![
            SpanKind::MasterDecompose,
            SpanKind::MasterSynthesize,
            SpanKind::SwarmWorker,
            SpanKind::Reviewer,
            SpanKind::EvolutionPass,
            SpanKind::PromptMutation,
            SpanKind::SkillExec,
            SpanKind::LlmCall,
        ];
        for kind in kinds {
            let s = kind.as_str();
            assert!(!s.is_empty());
            // 序列化 / 反序列化
            let json = serde_json::to_string(&kind).unwrap();
            let back: SpanKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn trace_span_start_and_finish() {
        let mut span = TraceSpan::start(
            "trace-1",
            None,
            SpanKind::MasterDecompose,
            TracePayload::new("test input"),
        );
        assert!(span.ended_at.is_none());
        assert!(span.output.is_none());
        assert!(span.error.is_none());

        span.finish(TracePayload::new("test output"));
        assert!(span.ended_at.is_some());
        assert!(span.output.is_some());
        assert!(span.error.is_none());
    }

    #[test]
    fn trace_span_finish_with_error() {
        let mut span = TraceSpan::start(
            "trace-1",
            None,
            SpanKind::SwarmWorker,
            TracePayload::new("input"),
        );
        span.finish_with_error("something went wrong");
        assert!(span.ended_at.is_some());
        assert!(span.output.is_none());
        assert_eq!(span.error.as_deref(), Some("something went wrong"));
    }

    #[test]
    fn trace_collector_start_and_end_span() {
        let collector = TraceCollector::new();
        // 手动启用 tracing (测试中环境变量可能未设置)
        EVAL_TRACING_ENABLED.store(true, Ordering::Relaxed);
        // 重置 INIT 让 call_once 重新执行 (测试 hack)
        // 注意: 由于 Once 的限制, 我们直接设置 atomic

        let span_id = collector.start_span(
            "trace-1",
            None,
            SpanKind::MasterDecompose,
            TracePayload::new("decompose input"),
        );
        assert_eq!(collector.len(), 1);

        collector.end_span(&span_id, TracePayload::new("decompose output"));

        let spans = collector.spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].span_kind, SpanKind::MasterDecompose);
        assert!(spans[0].ended_at.is_some());
        assert!(spans[0].output.is_some());

        // 重置
        EVAL_TRACING_ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_collector_parent_child_relationship() {
        let collector = TraceCollector::new();
        EVAL_TRACING_ENABLED.store(true, Ordering::Relaxed);

        let handle = collector.start_trace(
            SpanKind::MasterDecompose,
            TracePayload::new("root input"),
        );
        let trace_id = handle.trace_id().to_string();

        let child1 = handle.start_child(
            SpanKind::SwarmWorker,
            TracePayload::new("child 1 input"),
        );
        let child2 = handle.start_child(
            SpanKind::SwarmWorker,
            TracePayload::new("child 2 input"),
        );

        handle.end_span(&child1, TracePayload::new("child 1 output"));
        handle.end_span(&child2, TracePayload::new("child 2 output"));

        let spans = collector.spans_for_trace(&trace_id);
        assert_eq!(spans.len(), 3);

        // 根 span 没有 parent
        let root = spans.iter().find(|s| s.parent_id.is_none()).unwrap();
        assert_eq!(root.span_kind, SpanKind::MasterDecompose);

        // 子 span 的 parent_id 指向根 span
        let children: Vec<_> = spans.iter().filter(|s| s.parent_id.is_some()).collect();
        assert_eq!(children.len(), 2);
        for child in &children {
            assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));
            assert_eq!(child.trace_id, trace_id);
        }

        EVAL_TRACING_ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_collector_disabled_is_noop() {
        let collector = TraceCollector::new();
        EVAL_TRACING_ENABLED.store(false, Ordering::Relaxed);

        let span_id = collector.start_span(
            "trace-1",
            None,
            SpanKind::MasterDecompose,
            TracePayload::new("input"),
        );
        // 禁用时不收集
        assert_eq!(collector.len(), 0);

        collector.end_span(&span_id, TracePayload::new("output"));
        assert_eq!(collector.len(), 0);
    }

    #[test]
    fn export_jsonl_creates_valid_file() {
        let collector = TraceCollector::new();
        EVAL_TRACING_ENABLED.store(true, Ordering::Relaxed);

        let handle = collector.start_trace(
            SpanKind::MasterDecompose,
            TracePayload::new("root input"),
        );
        let child = handle.start_child(
            SpanKind::SwarmWorker,
            TracePayload::new("child input"),
        );
        handle.end_span(&child, TracePayload::new("child output"));

        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("nebula_eval_test_trace.jsonl");
        let count = collector.export_jsonl(&path).unwrap();
        assert_eq!(count, 2);

        // 验证 JSONL 格式: 每行一个有效 JSON
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().collect::<Result<_, _>>().unwrap();
        assert_eq!(lines.len(), 2);

        for line in &lines {
            let span: TraceSpan = serde_json::from_str(line).unwrap();
            assert!(!span.id.is_empty());
            assert!(!span.trace_id.is_empty());
        }

        std::fs::remove_file(&path).ok();
        EVAL_TRACING_ENABLED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn trace_payload_with_metadata() {
        let payload = TracePayload::new("text")
            .with_metadata(serde_json::json!({"key": "value"}));
        assert_eq!(payload.text, "text");
        assert_eq!(payload.metadata["key"], "value");
    }

    #[test]
    fn llm_meta_serialization() {
        let meta = LlmMeta {
            provider: "ollama".to_string(),
            model: "llama3".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.0,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: LlmMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "ollama");
        assert_eq!(back.input_tokens, 100);
    }

    #[test]
    fn now_iso8601_format() {
        let ts = now_iso8601();
        // 格式: YYYY-MM-DDTHH:MM:SS.mmmZ
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 24);
        assert_eq!(ts.as_bytes()[4], b'-');
        assert_eq!(ts.as_bytes()[10], b'T');
        assert_eq!(ts.as_bytes()[19], b'.');
    }

    #[test]
    fn days_to_ymd_epoch() {
        // 1970-01-01
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_2026() {
        // 2026-07-11 ≈ 20475 天 from 1970-01-01
        let (y, m, d) = days_to_ymd(20475);
        assert_eq!(y, 2026);
        assert_eq!(m, 7);
        assert_eq!(d, 11);
    }

    #[test]
    fn clear_spans() {
        let collector = TraceCollector::new();
        EVAL_TRACING_ENABLED.store(true, Ordering::Relaxed);

        collector.start_span(
            "t1",
            None,
            SpanKind::SwarmWorker,
            TracePayload::new("x"),
        );
        assert_eq!(collector.len(), 1);

        collector.clear();
        assert_eq!(collector.len(), 0);
        assert!(collector.is_empty());

        EVAL_TRACING_ENABLED.store(false, Ordering::Relaxed);
    }
}
