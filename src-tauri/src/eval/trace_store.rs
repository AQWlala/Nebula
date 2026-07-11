//! Trace 持久化 — 将 TraceCollector 收集的 span 写入 SQLite
//!
//! ## 设计
//!
//! 遵循 037_long_tasks / 038_loop_audit_log 的自管理模式:
//! 在 `TraceStore::new()` 时通过 `include_str!` + `execute_batch` 幂等应用
//! `039_eval_traces.sql` schema。
//!
//! ## Feature 门控
//!
//! 仅在 `#[cfg(feature = "eval")]` 时编译。表本身无条件创建
//! （`CREATE TABLE IF NOT EXISTS`，空表零开销）。

use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use tracing::{debug, instrument};

use super::trace::{LlmMeta, SpanKind, TraceCollector, TraceSpan};

// ---------------------------------------------------------------------------
// TraceStore
// ---------------------------------------------------------------------------

/// Trace SQLite 持久化层。
///
/// 持有 `Arc<Mutex<Connection>>`（来自 `SqliteStore::raw_connection()`），
/// 在 `new` 时幂等应用 `039_eval_traces.sql` schema。
///
/// 线程安全：`Arc<TraceStore>` 可跨线程共享，内部通过 `Mutex<Connection>` 串行化写入。
pub struct TraceStore {
    conn: Arc<Mutex<Connection>>,
}

impl TraceStore {
    /// 构造 Trace 持久化层，幂等应用 schema。
    pub fn new(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        let schema = include_str!("../../migrations/039_eval_traces.sql");
        {
            let c = conn.lock();
            c.execute_batch(schema)
                .context("applying 039_eval_traces.sql")?;
        }
        debug!(target: "nebula.eval.trace_store", "trace store initialized");
        Ok(Self { conn })
    }

    /// 从 `SqliteStore` 构造（便捷方法）。
    pub fn from_sqlite_store(store: &crate::memory::sqlite_store::SqliteStore) -> Result<Self> {
        Self::new(store.raw_connection())
    }

    /// 持久化一个完整的 trace（根 span + 所有子 span）。
    ///
    /// 先插入 eval_traces 根记录，再批量插入 eval_spans。
    /// 重复插入（相同 trace_id）会被 `INSERT OR REPLACE` 覆盖。
    #[instrument(skip(self, spans), fields(span_count = spans.len()))]
    pub fn save_trace(&self, spans: &[TraceSpan]) -> Result<()> {
        if spans.is_empty() {
            return Ok(());
        }

        let trace_id = spans[0].trace_id.clone();
        let root_span_id = spans
            .iter()
            .find(|s| s.parent_id.is_none())
            .map(|s| s.id.clone())
            .unwrap_or_else(|| spans[0].id.clone());
        let created_at = spans
            .iter()
            .map(|s| s.started_at.as_str())
            .min()
            .unwrap_or("")
            .to_string();
        let span_count = spans.len() as i64;

        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        // 插入 eval_traces 根记录
        tx.execute(
            "INSERT OR REPLACE INTO eval_traces \
             (trace_id, root_span_id, created_at, label, span_count) \
             VALUES (?1, ?2, ?3, NULL, ?4)",
            params![trace_id, root_span_id, created_at, span_count],
        )
        .context("inserting eval_traces row")?;

        // 批量插入 eval_spans
        for span in spans {
            let input_json = serde_json::to_string(&span.input).ok();
            let output_json = span
                .output
                .as_ref()
                .and_then(|o| serde_json::to_string(o).ok());
            let llm_meta_json = span
                .llm_meta
                .as_ref()
                .and_then(|m| serde_json::to_string(m).ok());
            let child_task_ids_json = serde_json::to_string(&span.child_task_ids).ok();

            tx.execute(
                "INSERT OR REPLACE INTO eval_spans \
                 (id, trace_id, parent_id, span_kind, started_at, ended_at, \
                  input_json, output_json, llm_meta_json, child_task_ids_json, error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    span.id,
                    span.trace_id,
                    span.parent_id,
                    span.span_kind.as_str(),
                    span.started_at,
                    span.ended_at,
                    input_json,
                    output_json,
                    llm_meta_json,
                    child_task_ids_json,
                    span.error,
                ],
            )
            .with_context(|| format!("inserting eval_spans row {}", span.id))?;
        }

        tx.commit()?;
        debug!(
            target: "nebula.eval.trace_store",
            trace_id = %trace_id,
            span_count,
            "trace saved"
        );
        Ok(())
    }

    /// 将 TraceCollector 中指定 trace 的所有 span 持久化。
    pub fn save_from_collector(&self, collector: &TraceCollector, trace_id: &str) -> Result<usize> {
        let spans = collector.spans_for_trace(trace_id);
        let count = spans.len();
        self.save_trace(&spans)?;
        Ok(count)
    }

    /// 查询指定 trace_id 的所有 span（从 SQLite 读取）。
    pub fn load_trace(&self, trace_id: &str) -> Result<Vec<TraceSpan>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, trace_id, parent_id, span_kind, started_at, ended_at, \
                    input_json, output_json, llm_meta_json, child_task_ids_json, error \
             FROM eval_spans WHERE trace_id = ?1 ORDER BY started_at ASC",
        )?;
        let rows = stmt.query_map(params![trace_id], row_to_span)?;
        let mut spans = Vec::new();
        for row in rows {
            spans.push(row?);
        }
        Ok(spans)
    }

    /// 列出所有 trace（按创建时间倒序）。
    pub fn list_traces(&self, limit: u32) -> Result<Vec<TraceRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT trace_id, root_span_id, created_at, label, span_count \
             FROM eval_traces ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(TraceRow {
                trace_id: row.get(0)?,
                root_span_id: row.get(1)?,
                created_at: row.get(2)?,
                label: row.get(3)?,
                span_count: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 删除指定 trace（级联删除其所有 span）。
    pub fn delete_trace(&self, trace_id: &str) -> Result<usize> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM eval_spans WHERE trace_id = ?1",
            params![trace_id],
        )?;
        let deleted = tx.execute(
            "DELETE FROM eval_traces WHERE trace_id = ?1",
            params![trace_id],
        )?;
        tx.commit()?;
        Ok(deleted)
    }
}

// ---------------------------------------------------------------------------
// TraceRow (列表查询结果)
// ---------------------------------------------------------------------------

/// `eval_traces` 表的一行（列表查询用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceRow {
    pub trace_id: String,
    pub root_span_id: String,
    pub created_at: String,
    pub label: Option<String>,
    pub span_count: i64,
}

// ---------------------------------------------------------------------------
// 行 → TraceSpan 反序列化
// ---------------------------------------------------------------------------

fn row_to_span(row: &rusqlite::Row<'_>) -> rusqlite::Result<TraceSpan> {
    let id: String = row.get(0)?;
    let trace_id: String = row.get(1)?;
    let parent_id: Option<String> = row.get(2)?;
    let span_kind_str: String = row.get(3)?;
    let started_at: String = row.get(4)?;
    let ended_at: Option<String> = row.get(5)?;
    let input_json: Option<String> = row.get(6)?;
    let output_json: Option<String> = row.get(7)?;
    let llm_meta_json: Option<String> = row.get(8)?;
    let child_task_ids_json: Option<String> = row.get(9)?;
    let error: Option<String> = row.get(10)?;

    let span_kind = parse_span_kind(&span_kind_str).unwrap_or(SpanKind::LlmCall);

    let input = input_json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| super::trace::TracePayload::new(""));

    let output = output_json.and_then(|s| serde_json::from_str(&s).ok());

    let llm_meta: Option<LlmMeta> = llm_meta_json.and_then(|s| serde_json::from_str(&s).ok());

    let child_task_ids: Vec<String> = child_task_ids_json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    Ok(TraceSpan {
        id,
        parent_id,
        trace_id,
        span_kind,
        started_at,
        ended_at,
        input,
        output,
        llm_meta,
        child_task_ids,
        error,
    })
}

fn parse_span_kind(s: &str) -> Option<SpanKind> {
    Some(match s {
        "master_decompose" => SpanKind::MasterDecompose,
        "master_synthesize" => SpanKind::MasterSynthesize,
        "swarm_worker" => SpanKind::SwarmWorker,
        "reviewer" => SpanKind::Reviewer,
        "evolution_pass" => SpanKind::EvolutionPass,
        "prompt_mutation" => SpanKind::PromptMutation,
        "skill_exec" => SpanKind::SkillExec,
        "llm_call" => SpanKind::LlmCall,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_store() -> TraceStore {
        let conn = Connection::open_in_memory().unwrap();
        let conn = Arc::new(Mutex::new(conn));
        TraceStore::new(conn).unwrap()
    }

    fn make_test_span(trace_id: &str, parent_id: Option<&str>, kind: SpanKind) -> TraceSpan {
        let mut span = TraceSpan::start(
            trace_id,
            parent_id,
            kind,
            super::super::trace::TracePayload::new("test input"),
        );
        span.finish(super::super::trace::TracePayload::new("test output"));
        span
    }

    #[test]
    fn trace_store_creates_schema() {
        let store = test_store();
        // 验证表存在
        let conn = store.conn.lock();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM eval_traces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM eval_spans", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn save_and_load_trace() {
        let store = test_store();

        let trace_id = "trace-save-1";
        let root = make_test_span(trace_id, None, SpanKind::MasterDecompose);
        let child1 = make_test_span(trace_id, Some(&root.id), SpanKind::SwarmWorker);
        let child2 = make_test_span(trace_id, Some(&root.id), SpanKind::SwarmWorker);
        let spans = vec![root, child1, child2];

        store.save_trace(&spans).unwrap();

        let loaded = store.load_trace(trace_id).unwrap();
        assert_eq!(loaded.len(), 3);

        // 验证根 span
        let root_loaded = loaded.iter().find(|s| s.parent_id.is_none()).unwrap();
        assert_eq!(root_loaded.span_kind, SpanKind::MasterDecompose);
        assert!(root_loaded.output.is_some());
    }

    #[test]
    fn list_traces_orders_by_created_at_desc() {
        let store = test_store();

        // 插入两个 trace
        let t1 = make_test_span("trace-list-1", None, SpanKind::MasterDecompose);
        let t2 = make_test_span("trace-list-2", None, SpanKind::EvolutionPass);
        store.save_trace(&[t1]).unwrap();
        store.save_trace(&[t2]).unwrap();

        let rows = store.list_traces(10).unwrap();
        assert_eq!(rows.len(), 2);
        // 倒序：后插入的在前
        assert_eq!(rows[0].trace_id, "trace-list-2");
        assert_eq!(rows[1].trace_id, "trace-list-1");
    }

    #[test]
    fn delete_trace_cascades() {
        let store = test_store();
        let trace_id = "trace-del-1";
        let root = make_test_span(trace_id, None, SpanKind::MasterDecompose);
        let child = make_test_span(trace_id, Some(&root.id), SpanKind::SwarmWorker);
        store.save_trace(&[root, child]).unwrap();

        assert_eq!(store.load_trace(trace_id).unwrap().len(), 2);
        let deleted = store.delete_trace(trace_id).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(store.load_trace(trace_id).unwrap().len(), 0);
    }

    #[test]
    fn save_empty_spans_is_noop() {
        let store = test_store();
        store.save_trace(&[]).unwrap();
        let rows = store.list_traces(10).unwrap();
        assert!(rows.is_empty());
    }
}
