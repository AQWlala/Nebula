-- =============================================================================
-- T-EVAL-003: Agent 评估体系 — Trace 持久化 (039_eval_traces.sql)
--
-- v2.4 add-agent-eval Phase 1: 将 TraceCollector 导出的 span 数据
-- 持久化到 SQLite, 供后续 LLM-as-Judge 评测与节点隔离测试使用。
--
-- 表结构:
--   * eval_traces — Trace 根表（每行 = 一次完整 trace）
--   * eval_spans  — Span 明细表（每行 = 一个 span, 外键关联 eval_traces）
--
-- 设计要点:
--   * trace_id / span id 用 UUID 字符串
--   * span_kind 枚举: master_decompose / master_synthesize / swarm_worker /
--     reviewer / evolution_pass / prompt_mutation / skill_exec / llm_call
--   * input_json / output_json / llm_meta_json / child_task_ids_json
--     均为 JSON 字符串（serde_json::to_string）
--   * 幂等性: CREATE TABLE IF NOT EXISTS / CREATE INDEX IF NOT EXISTS
--   * Feature 门控: 仅在 cargo build --features eval 时由 eval 模块读写;
--     表本身无条件创建（空表零开销, 不影响非 eval 用户）。
-- =============================================================================

CREATE TABLE IF NOT EXISTS eval_traces (
    trace_id      TEXT PRIMARY KEY,
    root_span_id  TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    label         TEXT,
    span_count    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS eval_spans (
    id                   TEXT PRIMARY KEY,
    trace_id             TEXT NOT NULL REFERENCES eval_traces(trace_id),
    parent_id            TEXT,
    span_kind            TEXT NOT NULL
                         CHECK(span_kind IN (
                             'master_decompose',
                             'master_synthesize',
                             'swarm_worker',
                             'reviewer',
                             'evolution_pass',
                             'prompt_mutation',
                             'skill_exec',
                             'llm_call'
                         )),
    started_at           TEXT NOT NULL,
    ended_at             TEXT,
    input_json           TEXT,
    output_json          TEXT,
    llm_meta_json        TEXT,
    child_task_ids_json  TEXT,
    error                TEXT
);

-- 按 trace_id 查询一次 trace 的所有 span（时间线回放）。
CREATE INDEX IF NOT EXISTS idx_eval_spans_trace ON eval_spans(trace_id);
-- 按 span_kind 过滤（如只看 llm_call 或 swarm_worker）。
CREATE INDEX IF NOT EXISTS idx_eval_spans_kind ON eval_spans(span_kind);
-- 按时间范围查询（默认按时间倒序）。
CREATE INDEX IF NOT EXISTS idx_eval_spans_started ON eval_spans(started_at);
