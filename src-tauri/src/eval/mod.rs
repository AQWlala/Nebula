//! Agent 评估体系 (v2.4.0 add-agent-eval)
//!
//! 四大子系统:
//! * [`trace`] — Trace 导出器,记录 Master/Swarm/Evolution 执行轨迹
//! * [`evalset`] — 评测集管理,YAML 格式样本 + 二元标准
//! * [`judge`] — LLM-as-Judge 评估器,本地 Ollama 评委
//! * [`isolation`] — 节点隔离测试,单节点运行节省成本
//! * [`scrub`] — PII 脱敏,评估前脱敏敏感信息
//!
//! ## Feature 门控
//!
//! `eval` feature 默认关闭。启用方式: `cargo build --features eval`。
//! 不启用时, TraceCollector 是 no-op, 零开销。
//!
//! ## 数据主权
//!
//! Judge LLM 强制本地 Ollama (WorkType::EvalJudge, is_local_only() = true)。
//! 评估前 PII 脱敏 (Scrubber)。评估结果不写入记忆层。
//!
//! ## 环境变量
//!
//! * `NEBULA_EVAL_TRACING=1` — 启用 Trace 收集 (默认关闭, 零开销)

pub mod trace;
pub mod trace_store;

pub use trace::{
    export_spans_jsonl, LlmMeta, SpanKind, TraceCollector, TraceCollectorHandle, TracePayload,
    TraceSpan,
};
pub use trace_store::{TraceRow, TraceStore};
