//! T-E-L-07: Loop 审计日志 — 记录每次 `execute_loop` 调用的关键节点。
//!
//! 与 [`crate::swarm::master::MasterOrchestrator::execute_loop`] 集成，
//! 在以下阶段写入审计记录：
//!
//! | 阶段 (`phase`)            | 触发条件                          | `status` 取值                  |
//! |---------------------------|-----------------------------------|--------------------------------|
//! | `values_check`            | ValuesLayer 门禁完成              | `allow` / `deny` / `confirm` / `plan` |
//! | `budget_check`            | 月度预算门禁完成                  | `ok` / `warning_80` / `exceeded` / `n/a` |
//! | `homogeneity_check`       | L4 同质检测完成                   | `ok` / `downgraded` / `skipped` |
//! | `task_creation`           | LongTask 创建结果                 | `ok` / `error`                 |
//! | `task_start`              | LongTask 启动结果                 | `ok` / `error`                 |
//! | `loop_started`            | execute_loop 成功返回             | `started`                      |
//! | `loop_denied`             | ValuesLayer Deny 短路返回         | `denied`                       |
//! | `loop_needs_confirmation` | ValuesLayer Confirm/Plan 短路返回 | `needs_confirmation`           |
//! | `loop_failed`             | 预算超限 / 内部错误返回 Err       | `failed`                       |
//!
//! ## 持久化
//!
//! 写入 SQLite `loop_audit_log` 表（migration `038_loop_audit_log.sql`）。
//! `LoopAuditLogger::new` 在构造时幂等应用 schema（`CREATE TABLE IF NOT EXISTS`），
//! 不依赖 migration runner，确保自包含。
//!
//! ## 性能
//!
//! `record_async` 通过 `tokio::task::spawn_blocking` 异步写入，不阻塞
//! `execute_loop` 的主流程。`record` 是同步写入，仅在测试中使用。
//!
//! ## Feature Gate
//!
//! 与 `master.rs` 一致，由 `master-orchestrator` feature 门控。

#![cfg(feature = "master-orchestrator")]

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// LoopAuditPhase — 审计日志阶段枚举
// ---------------------------------------------------------------------------

/// Loop 执行的阶段标识。
///
/// 与 SQL `CHECK` 约束中的字面量严格对齐（`#[serde(rename_all = "snake_case")]`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopAuditPhase {
    /// ValuesLayer 门禁阶段。
    ValuesCheck,
    /// 月度预算门禁阶段（T-E-L-06）。
    BudgetCheck,
    /// L4 同质检测阶段（T-E-L-06）。
    HomogeneityCheck,
    /// LongTask 创建阶段。
    TaskCreation,
    /// LongTask 启动阶段。
    TaskStart,
    /// execute_loop 成功返回。
    LoopStarted,
    /// ValuesLayer Deny 短路返回。
    LoopDenied,
    /// ValuesLayer Confirm/Plan 短路返回。
    LoopNeedsConfirmation,
    /// 预算超限或内部错误返回 Err。
    LoopFailed,
}

impl LoopAuditPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            LoopAuditPhase::ValuesCheck => "values_check",
            LoopAuditPhase::BudgetCheck => "budget_check",
            LoopAuditPhase::HomogeneityCheck => "homogeneity_check",
            LoopAuditPhase::TaskCreation => "task_creation",
            LoopAuditPhase::TaskStart => "task_start",
            LoopAuditPhase::LoopStarted => "loop_started",
            LoopAuditPhase::LoopDenied => "loop_denied",
            LoopAuditPhase::LoopNeedsConfirmation => "loop_needs_confirmation",
            LoopAuditPhase::LoopFailed => "loop_failed",
        }
    }
}

impl std::fmt::Display for LoopAuditPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// LoopAuditEntry — 单条审计记录
// ---------------------------------------------------------------------------

/// 单条 Loop 审计日志记录（对应 `loop_audit_log` 表一行）。
///
/// 由 [`LoopAuditLogger::record`] 写入 SQLite。`run_id` 标识一次
/// `execute_loop` 调用，同一次调用可产生多条不同 `phase` 的记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopAuditEntry {
    /// 记录 ID（UUID，主键）。
    pub id: String,
    /// 一次 execute_loop 调用的唯一标识（UUID），同一次调用的所有阶段共享。
    pub run_id: String,
    /// Loop 名称（来自 LOOP.md frontmatter `name`）。
    pub loop_name: String,
    /// 关联的 LongTask ID（`denied` / `needs_confirmation` / `failed` 时为 None）。
    pub task_id: Option<String>,
    /// 执行阶段。
    pub phase: LoopAuditPhase,
    /// 阶段状态（自由字符串，如 `allow` / `deny` / `ok` / `exceeded` / `started` / `error`）。
    pub status: String,
    /// 阶段开始时间（毫秒时间戳）。
    pub started_at: i64,
    /// 阶段结束时间（毫秒时间戳，可选 — 瞬时事件可与 started_at 相同）。
    pub finished_at: Option<i64>,
    /// 阶段耗时（毫秒，可选）。
    pub elapsed_ms: Option<u64>,
    /// 输入摘要（前 500 字符，用于审计追溯）。
    pub input_summary: Option<String>,
    /// 输出摘要（前 500 字符，用于审计追溯）。
    pub output_summary: Option<String>,
    /// 错误信息（阶段失败时填充）。
    pub error_message: Option<String>,
    /// Loop 自主度等级（如 "L1" / "L4"，来自 LoopDef.autonomy）。
    pub autonomy_level: Option<String>,
    /// 预算门禁状态（如 "ok" / "warning_80" / "exceeded" / "n/a"）。
    pub budget_status: Option<String>,
    /// 自主度降级标记（如 "L4→L2"，None 表示未降级）。
    pub autonomy_downgraded: Option<String>,
    /// ValuesLayer 裁定（如 "allow" / "deny" / "confirm" / "plan"）。
    pub values_verdict: Option<String>,
    /// 额外元数据（JSON 对象，用于扩展字段）。
    pub metadata: HashMap<String, String>,
}

impl LoopAuditEntry {
    /// 构造一条新的审计记录，自动生成 `id`（UUID）并设置 `started_at` 为当前时间。
    ///
    /// `finished_at` / `elapsed_ms` 留空，由调用方在阶段结束后通过
    /// [`LoopAuditEntryBuilder`] 设置，或直接在构造时填入。
    pub fn new(
        run_id: impl Into<String>,
        loop_name: impl Into<String>,
        phase: LoopAuditPhase,
        status: impl Into<String>,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: Uuid::new_v4().to_string(),
            run_id: run_id.into(),
            loop_name: loop_name.into(),
            task_id: None,
            phase,
            status: status.into(),
            started_at: now,
            finished_at: Some(now),
            elapsed_ms: Some(0),
            input_summary: None,
            output_summary: None,
            error_message: None,
            autonomy_level: None,
            budget_status: None,
            autonomy_downgraded: None,
            values_verdict: None,
            metadata: HashMap::new(),
        }
    }

    /// Builder: 设置 task_id。
    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Builder: 设置耗时（同时设置 finished_at = started_at + elapsed_ms）。
    pub fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self.finished_at = Some(self.started_at + elapsed_ms as i64);
        self
    }

    /// Builder: 设置输入摘要（截断到 500 字符）。
    pub fn with_input_summary(mut self, summary: impl Into<String>) -> Self {
        let s: String = summary.into();
        self.input_summary = Some(s.chars().take(500).collect());
        self
    }

    /// Builder: 设置输出摘要（截断到 500 字符）。
    pub fn with_output_summary(mut self, summary: impl Into<String>) -> Self {
        let s: String = summary.into();
        self.output_summary = Some(s.chars().take(500).collect());
        self
    }

    /// Builder: 设置错误信息。
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error_message = Some(error.into());
        self
    }

    /// Builder: 设置自主度等级。
    pub fn with_autonomy_level(mut self, level: impl Into<String>) -> Self {
        self.autonomy_level = Some(level.into());
        self
    }

    /// Builder: 设置预算状态。
    pub fn with_budget_status(mut self, status: impl Into<String>) -> Self {
        self.budget_status = Some(status.into());
        self
    }

    /// Builder: 设置自主度降级标记。
    pub fn with_autonomy_downgraded(mut self, downgrade: impl Into<String>) -> Self {
        self.autonomy_downgraded = Some(downgrade.into());
        self
    }

    /// Builder: 设置 ValuesLayer 裁定。
    pub fn with_values_verdict(mut self, verdict: impl Into<String>) -> Self {
        self.values_verdict = Some(verdict.into());
        self
    }

    /// Builder: 追加一个元数据键值对。
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// LoopAuditQuery — 查询过滤器
// ---------------------------------------------------------------------------

/// 审计日志查询过滤器。
///
/// 所有字段均为 `Option`，`None` 表示不过滤该维度。多个字段同时设置时取交集（AND 语义）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopAuditQuery {
    /// 按 run_id 过滤（查询某次 execute_loop 调用的完整时间线）。
    pub run_id: Option<String>,
    /// 按 loop_name 过滤（查询某个 Loop 的历史执行记录）。
    pub loop_name: Option<String>,
    /// 按 status 过滤（如只看 "denied" / "failed"）。
    pub status: Option<String>,
    /// 按 phase 过滤（如只看 "values_check"）。
    pub phase: Option<LoopAuditPhase>,
    /// 起始时间（毫秒时间戳，闭区间）。
    pub started_at_from: Option<i64>,
    /// 结束时间（毫秒时间戳，闭区间）。
    pub started_at_to: Option<i64>,
    /// 返回条数上限（默认 100）。
    pub limit: Option<usize>,
}

impl LoopAuditQuery {
    /// 构造一个空的查询过滤器（返回最近 100 条）。
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: 按 run_id 过滤。
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Builder: 按 loop_name 过滤。
    pub fn with_loop_name(mut self, loop_name: impl Into<String>) -> Self {
        self.loop_name = Some(loop_name.into());
        self
    }

    /// Builder: 按 status 过滤。
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    /// Builder: 按 phase 过滤。
    pub fn with_phase(mut self, phase: LoopAuditPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    /// Builder: 按时间范围过滤（闭区间）。
    pub fn with_time_range(mut self, from: i64, to: i64) -> Self {
        self.started_at_from = Some(from);
        self.started_at_to = Some(to);
        self
    }

    /// Builder: 设置返回条数上限。
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

// ---------------------------------------------------------------------------
// LoopAuditLogger — 审计日志记录器
// ---------------------------------------------------------------------------

/// Loop 审计日志记录器。
///
/// 持有 `Arc<Mutex<Connection>>`（来自 [`crate::memory::sqlite_store::SqliteStore::raw_connection`]），
/// 在 `new` 时幂等应用 `038_loop_audit_log.sql` schema。
///
/// 线程安全：`Arc<LoopAuditLogger>` 可跨线程共享，内部通过 `Mutex<Connection>` 串行化写入。
pub struct LoopAuditLogger {
    conn: Arc<Mutex<Connection>>,
}

impl LoopAuditLogger {
    /// 构造审计日志记录器，幂等应用 schema。
    ///
    /// 从 [`crate::memory::sqlite_store::SqliteStore`] 获取 `raw_connection()`，
    /// 执行 `038_loop_audit_log.sql`（`CREATE TABLE IF NOT EXISTS`，幂等）。
    pub fn new(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        let schema = include_str!("../../migrations/038_loop_audit_log.sql");
        {
            let c = conn.lock();
            c.execute_batch(schema)
                .context("applying 038_loop_audit_log.sql")?;
        }
        debug!(target: "nebula.loop.audit", "loop audit logger initialized");
        Ok(Self { conn })
    }

    /// 从 `SqliteStore` 构造（便捷方法）。
    ///
    /// 等价于 `LoopAuditLogger::new(store.raw_connection())`。
    pub fn from_sqlite_store(store: &crate::memory::sqlite_store::SqliteStore) -> Result<Self> {
        Self::new(store.raw_connection())
    }

    /// 同步写入一条审计记录。
    ///
    /// 在 `execute_loop` 的关键节点调用。写入失败仅记 `warn` 日志，
    /// 不影响 Loop 执行流程（审计日志是 best-effort，不阻塞主流程）。
    #[instrument(skip(self, entry), fields(phase = %entry.phase, status = %entry.status))]
    pub fn record(&self, entry: &LoopAuditEntry) -> Result<()> {
        let metadata_json =
            serde_json::to_string(&entry.metadata).unwrap_or_else(|_| "{}".to_string());
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO loop_audit_log \
             (id, run_id, loop_name, task_id, phase, status, started_at, finished_at, \
              elapsed_ms, input_summary, output_summary, error_message, \
              autonomy_level, budget_status, autonomy_downgraded, values_verdict, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                entry.id,
                entry.run_id,
                entry.loop_name,
                entry.task_id,
                entry.phase.as_str(),
                entry.status,
                entry.started_at,
                entry.finished_at,
                entry.elapsed_ms.map(|v| v as i64),
                entry.input_summary,
                entry.output_summary,
                entry.error_message,
                entry.autonomy_level,
                entry.budget_status,
                entry.autonomy_downgraded,
                entry.values_verdict,
                metadata_json,
            ],
        )
        .context("inserting loop audit log entry")?;
        debug!(
            target: "nebula.loop.audit",
            run_id = %entry.run_id,
            phase = %entry.phase,
            "audit entry recorded"
        );
        Ok(())
    }

    /// 异步写入一条审计记录（不阻塞调用方）。
    ///
    /// 通过 `tokio::task::spawn_blocking` 在独立线程执行 SQLite 写入。
    /// 写入失败仅记 `warn` 日志，不影响 Loop 执行。
    ///
    /// **重要**：此方法消费 `entry` 的所有权（`LoopAuditEntry` 是 `Clone` 的，
    /// 调用方可预先 clone）。`self` 以 `Arc` clone 传入 spawn 闭包。
    pub async fn record_async(self: &Arc<Self>, entry: LoopAuditEntry) {
        let logger = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = logger.record(&entry) {
                warn!(
                    target: "nebula.loop.audit",
                    error = %e,
                    "failed to record loop audit entry (non-fatal)"
                );
            }
        })
        .await
        .ok();
    }

    /// 触发即忘（fire-and-forget）异步写入一条审计记录。
    ///
    /// 与 [`record_async`](Self::record_async) 的区别：不 await spawn_blocking
    /// 的结果，立即返回。适用于 `execute_loop` 中不关心写入完成时机的阶段日志
    /// （如 `values_check` / `budget_check`），确保不影响 Loop 执行性能。
    ///
    /// **调用方需持有 `Arc<LoopAuditLogger>`**，因为 `spawn_blocking` 闭包需要
    /// `'static` 生命周期。
    pub fn spawn_record(self: &Arc<Self>, entry: LoopAuditEntry) {
        let logger = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            if let Err(e) = logger.record(&entry) {
                warn!(
                    target: "nebula.loop.audit",
                    error = %e,
                    phase = ?entry.phase,
                    "failed to record loop audit entry (non-fatal)"
                );
            }
        });
    }

    /// 按条件查询审计日志。
    ///
    /// 返回按 `started_at DESC` 排序的记录列表。默认上限 100 条。
    pub fn query(&self, filter: &LoopAuditQuery) -> Result<Vec<LoopAuditEntry>> {
        let limit = filter.limit.unwrap_or(100).min(1000) as i64;

        // 动态构造 WHERE 子句
        let mut where_clauses: Vec<String> = Vec::new();
        let mut param_values: Vec<rusqlite::types::Value> = Vec::new();
        let mut param_idx = 1usize;

        if let Some(ref run_id) = filter.run_id {
            where_clauses.push(format!("run_id = ?{param_idx}"));
            param_values.push(run_id.clone().into());
            param_idx += 1;
        }
        if let Some(ref loop_name) = filter.loop_name {
            where_clauses.push(format!("loop_name = ?{param_idx}"));
            param_values.push(loop_name.clone().into());
            param_idx += 1;
        }
        if let Some(ref status) = filter.status {
            where_clauses.push(format!("status = ?{param_idx}"));
            param_values.push(status.clone().into());
            param_idx += 1;
        }
        if let Some(phase) = filter.phase {
            where_clauses.push(format!("phase = ?{param_idx}"));
            param_values.push(phase.as_str().to_string().into());
            param_idx += 1;
        }
        if let Some(from) = filter.started_at_from {
            where_clauses.push(format!("started_at >= ?{param_idx}"));
            param_values.push(from.into());
            param_idx += 1;
        }
        if let Some(to) = filter.started_at_to {
            where_clauses.push(format!("started_at <= ?{param_idx}"));
            param_values.push(to.into());
            param_idx += 1;
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let sql = format!(
            "SELECT id, run_id, loop_name, task_id, phase, status, started_at, finished_at, \
                    elapsed_ms, input_summary, output_summary, error_message, \
                    autonomy_level, budget_status, autonomy_downgraded, values_verdict, metadata_json \
             FROM loop_audit_log{where_sql} \
             ORDER BY started_at DESC, rowid DESC LIMIT ?{param_idx}"
        );
        param_values.push(limit.into());

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(param_values.iter()),
            row_to_entry,
        )?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// 统计满足条件的记录数（不分页）。
    pub fn count(&self, filter: &LoopAuditQuery) -> Result<i64> {
        let mut where_clauses: Vec<String> = Vec::new();
        let mut param_values: Vec<rusqlite::types::Value> = Vec::new();
        let mut param_idx = 1usize;

        if let Some(ref run_id) = filter.run_id {
            where_clauses.push(format!("run_id = ?{param_idx}"));
            param_values.push(run_id.clone().into());
            param_idx += 1;
        }
        if let Some(ref loop_name) = filter.loop_name {
            where_clauses.push(format!("loop_name = ?{param_idx}"));
            param_values.push(loop_name.clone().into());
            param_idx += 1;
        }
        if let Some(ref status) = filter.status {
            where_clauses.push(format!("status = ?{param_idx}"));
            param_values.push(status.clone().into());
            param_idx += 1;
        }
        if let Some(phase) = filter.phase {
            where_clauses.push(format!("phase = ?{param_idx}"));
            param_values.push(phase.as_str().to_string().into());
            param_idx += 1;
        }
        if let Some(from) = filter.started_at_from {
            where_clauses.push(format!("started_at >= ?{param_idx}"));
            param_values.push(from.into());
            param_idx += 1;
        }
        if let Some(to) = filter.started_at_to {
            where_clauses.push(format!("started_at <= ?{param_idx}"));
            param_values.push(to.into());
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let sql = format!("SELECT COUNT(*) FROM loop_audit_log{where_sql}");
        let conn = self.conn.lock();
        let count: i64 =
            conn.query_row(&sql, rusqlite::params_from_iter(param_values.iter()), |r| {
                r.get(0)
            })?;
        Ok(count)
    }
}

/// 将 `rusqlite::Row` 映射为 `LoopAuditEntry`。
fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<LoopAuditEntry> {
    let phase_str: String = r.get(4)?;
    let phase = match phase_str.as_str() {
        "values_check" => LoopAuditPhase::ValuesCheck,
        "budget_check" => LoopAuditPhase::BudgetCheck,
        "homogeneity_check" => LoopAuditPhase::HomogeneityCheck,
        "task_creation" => LoopAuditPhase::TaskCreation,
        "task_start" => LoopAuditPhase::TaskStart,
        "loop_started" => LoopAuditPhase::LoopStarted,
        "loop_denied" => LoopAuditPhase::LoopDenied,
        "loop_needs_confirmation" => LoopAuditPhase::LoopNeedsConfirmation,
        "loop_failed" => LoopAuditPhase::LoopFailed,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                anyhow::anyhow!("unknown loop audit phase: {other}").into(),
            ))
        }
    };
    let metadata_json: String = r.get(16)?;
    let metadata: HashMap<String, String> =
        serde_json::from_str(&metadata_json).unwrap_or_default();
    let elapsed_ms_i64: Option<i64> = r.get(8)?;
    Ok(LoopAuditEntry {
        id: r.get(0)?,
        run_id: r.get(1)?,
        loop_name: r.get(2)?,
        task_id: r.get(3)?,
        phase,
        status: r.get(5)?,
        started_at: r.get(6)?,
        finished_at: r.get(7)?,
        elapsed_ms: elapsed_ms_i64.map(|v| v as u64),
        input_summary: r.get(9)?,
        output_summary: r.get(10)?,
        error_message: r.get(11)?,
        autonomy_level: r.get(12)?,
        budget_status: r.get(13)?,
        autonomy_downgraded: r.get(14)?,
        values_verdict: r.get(15)?,
        metadata,
    })
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sqlite_store::SqliteStore;

    /// 构造临时 SQLite + 应用 038 schema（通过 LoopAuditLogger::new）。
    fn make_logger() -> (Arc<LoopAuditLogger>, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!("nebula-loop-audit-test-{}", Uuid::new_v4()));
        let _ = std::fs::remove_file(&tmp);
        let store = SqliteStore::open(&tmp).expect("open sqlite");
        let logger = Arc::new(LoopAuditLogger::from_sqlite_store(&store).expect("init logger"));
        (logger, tmp)
    }

    fn cleanup(path: std::path::PathBuf) {
        let _ = std::fs::remove_file(&path);
    }

    fn make_sample_entry(
        run_id: &str,
        loop_name: &str,
        phase: LoopAuditPhase,
        status: &str,
    ) -> LoopAuditEntry {
        LoopAuditEntry::new(run_id, loop_name, phase, status)
            .with_input_summary("读取 README.md; 总结要点")
            .with_output_summary("Loop started successfully")
            .with_autonomy_level("L2")
    }

    // ---- LoopAuditPhase ----

    #[test]
    fn phase_as_str_matches_serde() {
        for (phase, expected) in [
            (LoopAuditPhase::ValuesCheck, "values_check"),
            (LoopAuditPhase::BudgetCheck, "budget_check"),
            (LoopAuditPhase::HomogeneityCheck, "homogeneity_check"),
            (LoopAuditPhase::TaskCreation, "task_creation"),
            (LoopAuditPhase::TaskStart, "task_start"),
            (LoopAuditPhase::LoopStarted, "loop_started"),
            (LoopAuditPhase::LoopDenied, "loop_denied"),
            (
                LoopAuditPhase::LoopNeedsConfirmation,
                "loop_needs_confirmation",
            ),
            (LoopAuditPhase::LoopFailed, "loop_failed"),
        ] {
            assert_eq!(phase.as_str(), expected);
            let json = serde_json::to_string(&phase).expect("serialize should succeed");
            assert_eq!(json, format!("\"{expected}\""));
            let back: LoopAuditPhase =
                serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(back, phase);
        }
    }

    #[test]
    fn phase_display_matches_as_str() {
        assert_eq!(format!("{}", LoopAuditPhase::ValuesCheck), "values_check");
        assert_eq!(format!("{}", LoopAuditPhase::LoopStarted), "loop_started");
    }

    // ---- LoopAuditEntry builder ----

    #[test]
    fn entry_new_generates_id_and_timestamp() {
        let entry = LoopAuditEntry::new("run-1", "test-loop", LoopAuditPhase::ValuesCheck, "allow");
        assert!(!entry.id.is_empty(), "id should be auto-generated");
        assert_eq!(entry.run_id, "run-1");
        assert_eq!(entry.loop_name, "test-loop");
        assert_eq!(entry.phase, LoopAuditPhase::ValuesCheck);
        assert_eq!(entry.status, "allow");
        assert!(entry.started_at > 0, "started_at should be set");
        assert!(
            entry.finished_at.is_some(),
            "finished_at should default to Some"
        );
        assert_eq!(entry.elapsed_ms, Some(0), "elapsed_ms should default to 0");
    }

    #[test]
    fn entry_builder_chains() {
        let entry = LoopAuditEntry::new("run-1", "test", LoopAuditPhase::LoopStarted, "started")
            .with_task_id("task-123")
            .with_elapsed_ms(500)
            .with_input_summary("input")
            .with_output_summary("output")
            .with_error("none")
            .with_autonomy_level("L4")
            .with_budget_status("ok")
            .with_autonomy_downgraded("L4→L2")
            .with_values_verdict("allow")
            .with_metadata("key", "value");
        assert_eq!(entry.task_id.as_deref(), Some("task-123"));
        assert_eq!(entry.elapsed_ms, Some(500));
        assert_eq!(entry.finished_at, Some(entry.started_at + 500));
        assert_eq!(entry.input_summary.as_deref(), Some("input"));
        assert_eq!(entry.output_summary.as_deref(), Some("output"));
        assert_eq!(entry.error_message.as_deref(), Some("none"));
        assert_eq!(entry.autonomy_level.as_deref(), Some("L4"));
        assert_eq!(entry.budget_status.as_deref(), Some("ok"));
        assert_eq!(entry.autonomy_downgraded.as_deref(), Some("L4→L2"));
        assert_eq!(entry.values_verdict.as_deref(), Some("allow"));
        assert_eq!(entry.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn entry_input_summary_truncated_to_500_chars() {
        let long_input = "x".repeat(1000);
        let entry = LoopAuditEntry::new("r", "l", LoopAuditPhase::ValuesCheck, "ok")
            .with_input_summary(&long_input);
        assert_eq!(
            entry
                .input_summary
                .as_ref()
                .expect("test op should succeed")
                .chars()
                .count(),
            500
        );
    }

    // ---- LoopAuditLogger::record + query ----

    #[test]
    fn record_and_query_single_entry() {
        let (logger, tmp) = make_logger();
        let entry = make_sample_entry(
            "run-1",
            "daily-triage",
            LoopAuditPhase::LoopStarted,
            "started",
        );
        logger.record(&entry).expect("record should succeed");

        let results = logger
            .query(&LoopAuditQuery::new().with_run_id("run-1"))
            .expect("query should succeed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].run_id, "run-1");
        assert_eq!(results[0].loop_name, "daily-triage");
        assert_eq!(results[0].phase, LoopAuditPhase::LoopStarted);
        assert_eq!(results[0].status, "started");
        assert_eq!(results[0].autonomy_level.as_deref(), Some("L2"));
        cleanup(tmp);
    }

    #[test]
    fn record_multiple_phases_for_same_run() {
        let (logger, tmp) = make_logger();
        let run_id = "run-multi";
        // 模拟一次 execute_loop 调用的完整时间线
        let phases = vec![
            (LoopAuditPhase::ValuesCheck, "allow"),
            (LoopAuditPhase::BudgetCheck, "ok"),
            (LoopAuditPhase::HomogeneityCheck, "skipped"),
            (LoopAuditPhase::TaskCreation, "ok"),
            (LoopAuditPhase::TaskStart, "ok"),
            (LoopAuditPhase::LoopStarted, "started"),
        ];
        for (phase, status) in &phases {
            let entry = LoopAuditEntry::new(run_id, "ci-sweeper", *phase, *status);
            logger.record(&entry).expect("record should succeed");
        }

        let results = logger
            .query(&LoopAuditQuery::new().with_run_id(run_id))
            .expect("query should succeed");
        assert_eq!(results.len(), 6, "should have 6 entries for this run");
        // 验证所有 entry 都属于同一个 run_id
        for r in &results {
            assert_eq!(r.run_id, run_id);
        }
        cleanup(tmp);
    }

    #[test]
    fn query_by_loop_name() {
        let (logger, tmp) = make_logger();
        // 两个不同的 loop
        logger
            .record(&make_sample_entry(
                "r1",
                "loop-a",
                LoopAuditPhase::LoopStarted,
                "started",
            ))
            .unwrap();
        logger
            .record(&make_sample_entry(
                "r2",
                "loop-b",
                LoopAuditPhase::LoopStarted,
                "started",
            ))
            .unwrap();
        logger
            .record(&make_sample_entry(
                "r3",
                "loop-a",
                LoopAuditPhase::LoopDenied,
                "denied",
            ))
            .unwrap();

        let results = logger
            .query(&LoopAuditQuery::new().with_loop_name("loop-a"))
            .expect("query should succeed");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.loop_name == "loop-a"));
        cleanup(tmp);
    }

    #[test]
    fn query_by_status() {
        let (logger, tmp) = make_logger();
        logger
            .record(&make_sample_entry(
                "r1",
                "l",
                LoopAuditPhase::LoopStarted,
                "started",
            ))
            .unwrap();
        logger
            .record(&make_sample_entry(
                "r2",
                "l",
                LoopAuditPhase::LoopDenied,
                "denied",
            ))
            .unwrap();
        logger
            .record(&make_sample_entry(
                "r3",
                "l",
                LoopAuditPhase::LoopFailed,
                "failed",
            ))
            .unwrap();

        let denied = logger
            .query(&LoopAuditQuery::new().with_status("denied"))
            .expect("query should succeed");
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].status, "denied");
        cleanup(tmp);
    }

    #[test]
    fn query_by_phase() {
        let (logger, tmp) = make_logger();
        logger
            .record(&make_sample_entry(
                "r1",
                "l",
                LoopAuditPhase::ValuesCheck,
                "allow",
            ))
            .unwrap();
        logger
            .record(&make_sample_entry(
                "r2",
                "l",
                LoopAuditPhase::BudgetCheck,
                "ok",
            ))
            .unwrap();
        logger
            .record(&make_sample_entry(
                "r3",
                "l",
                LoopAuditPhase::LoopStarted,
                "started",
            ))
            .unwrap();

        let results = logger
            .query(&LoopAuditQuery::new().with_phase(LoopAuditPhase::LoopStarted))
            .expect("query should succeed");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].phase, LoopAuditPhase::LoopStarted);
        cleanup(tmp);
    }

    #[test]
    fn query_by_time_range() {
        let (logger, tmp) = make_logger();
        let now = chrono::Utc::now().timestamp_millis();
        let past = now - 10_000; // 10 秒前
        let future = now + 10_000; // 10 秒后

        let mut entry1 = make_sample_entry("r1", "l", LoopAuditPhase::LoopStarted, "started");
        entry1.started_at = now - 5_000;
        let mut entry2 = make_sample_entry("r2", "l", LoopAuditPhase::LoopStarted, "started");
        entry2.started_at = now;
        let mut entry3 = make_sample_entry("r3", "l", LoopAuditPhase::LoopStarted, "started");
        entry3.started_at = now + 5_000;

        logger.record(&entry1).unwrap();
        logger.record(&entry2).unwrap();
        logger.record(&entry3).unwrap();

        let results = logger
            .query(&LoopAuditQuery::new().with_time_range(past, future))
            .expect("query should succeed");
        assert_eq!(results.len(), 3, "all 3 should be in range");

        let results = logger
            .query(&LoopAuditQuery::new().with_time_range(now - 6_000, now + 1_000))
            .expect("query should succeed");
        assert_eq!(results.len(), 2, "only entry1 and entry2 in range");
        cleanup(tmp);
    }

    #[test]
    fn query_results_ordered_by_started_at_desc() {
        let (logger, tmp) = make_logger();
        let mut entry_old = make_sample_entry("r-old", "l", LoopAuditPhase::LoopStarted, "started");
        entry_old.started_at = 1000;
        let mut entry_mid = make_sample_entry("r-mid", "l", LoopAuditPhase::LoopStarted, "started");
        entry_mid.started_at = 2000;
        let mut entry_new = make_sample_entry("r-new", "l", LoopAuditPhase::LoopStarted, "started");
        entry_new.started_at = 3000;

        logger.record(&entry_old).unwrap();
        logger.record(&entry_new).unwrap();
        logger.record(&entry_mid).unwrap();

        let results = logger
            .query(&LoopAuditQuery::new())
            .expect("query should succeed");
        assert_eq!(results.len(), 3);
        // 按 started_at DESC 排序
        assert!(results[0].started_at >= results[1].started_at);
        assert!(results[1].started_at >= results[2].started_at);
        assert_eq!(results[0].started_at, 3000);
        assert_eq!(results[2].started_at, 1000);
        cleanup(tmp);
    }

    #[test]
    fn query_limit_caps_results() {
        let (logger, tmp) = make_logger();
        for i in 0..10 {
            let entry = LoopAuditEntry::new(
                &format!("r-{i}"),
                "l",
                LoopAuditPhase::LoopStarted,
                "started",
            );
            logger.record(&entry).unwrap();
        }
        let results = logger
            .query(&LoopAuditQuery::new().with_limit(3))
            .expect("query should succeed");
        assert_eq!(results.len(), 3, "limit should cap results");
        cleanup(tmp);
    }

    #[test]
    fn count_returns_total_matching() {
        let (logger, tmp) = make_logger();
        for i in 0..5 {
            logger
                .record(&make_sample_entry(
                    &format!("r-{i}"),
                    "loop-a",
                    LoopAuditPhase::LoopStarted,
                    "started",
                ))
                .unwrap();
        }
        for i in 0..3 {
            logger
                .record(&make_sample_entry(
                    &format!("r-b-{i}"),
                    "loop-b",
                    LoopAuditPhase::LoopDenied,
                    "denied",
                ))
                .unwrap();
        }

        let total = logger
            .count(&LoopAuditQuery::new())
            .expect("count should succeed");
        assert_eq!(total, 8, "total should be 8");

        let loop_a = logger
            .count(&LoopAuditQuery::new().with_loop_name("loop-a"))
            .expect("count should succeed");
        assert_eq!(loop_a, 5);

        let denied = logger
            .count(&LoopAuditQuery::new().with_status("denied"))
            .expect("count should succeed");
        assert_eq!(denied, 3);
        cleanup(tmp);
    }

    #[test]
    fn empty_query_returns_empty_vec() {
        let (logger, tmp) = make_logger();
        let results = logger
            .query(&LoopAuditQuery::new())
            .expect("query should succeed");
        assert!(results.is_empty(), "no records → empty vec");
        let count = logger
            .count(&LoopAuditQuery::new())
            .expect("count should succeed");
        assert_eq!(count, 0);
        cleanup(tmp);
    }

    #[test]
    fn record_stores_metadata_json() {
        let (logger, tmp) = make_logger();
        let entry = LoopAuditEntry::new("r1", "l", LoopAuditPhase::ValuesCheck, "allow")
            .with_metadata("trigger", "cron")
            .with_metadata("model", "deepseek-chat");
        logger.record(&entry).expect("record should succeed");

        let results = logger
            .query(&LoopAuditQuery::new().with_run_id("r1"))
            .expect("query should succeed");
        assert_eq!(results.len(), 1);
        let metadata = &results[0].metadata;
        assert_eq!(metadata.get("trigger"), Some(&"cron".to_string()));
        assert_eq!(metadata.get("model"), Some(&"deepseek-chat".to_string()));
        cleanup(tmp);
    }

    #[test]
    fn record_stores_error_message() {
        let (logger, tmp) = make_logger();
        let entry = LoopAuditEntry::new("r1", "l", LoopAuditPhase::LoopFailed, "failed")
            .with_error("monthly budget exceeded: used 1M tokens (limit: 500K)");
        logger.record(&entry).expect("record should succeed");

        let results = logger
            .query(&LoopAuditQuery::new().with_run_id("r1"))
            .expect("query should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].error_message.is_some());
        assert!(results[0]
            .error_message
            .as_ref()
            .expect("test op should succeed")
            .contains("budget exceeded"));
        cleanup(tmp);
    }

    // ---- record_async ----

    #[tokio::test]
    async fn record_async_writes_without_blocking() {
        let (logger, tmp) = make_logger();
        let entry = make_sample_entry("r-async", "l", LoopAuditPhase::LoopStarted, "started");
        logger.record_async(entry).await;

        // spawn_blocking 完成后查询应能读到
        let results = logger
            .query(&LoopAuditQuery::new().with_run_id("r-async"))
            .expect("query should succeed");
        assert_eq!(results.len(), 1, "async record should be persisted");
        cleanup(tmp);
    }

    #[tokio::test]
    async fn record_async_failure_does_not_panic() {
        let (logger, tmp) = make_logger();
        // 正常记录一条 — 主要验证 record_async 不会 panic
        let entry = make_sample_entry("r-ok", "l", LoopAuditPhase::LoopStarted, "started");
        logger.record_async(entry).await;
        // 再次查询验证
        let results = logger
            .query(&LoopAuditQuery::new().with_run_id("r-ok"))
            .expect("query should succeed");
        assert_eq!(results.len(), 1);
        cleanup(tmp);
    }

    // ---- from_sqlite_store ----

    #[test]
    fn from_sqlite_store_applies_schema_idempotently() {
        let tmp = std::env::temp_dir().join(format!("nebula-loop-audit-idem-{}", Uuid::new_v4()));
        let _ = std::fs::remove_file(&tmp);
        let store = SqliteStore::open(&tmp).expect("open sqlite");
        // 连续构造两次 — schema 幂等
        let _logger1 = LoopAuditLogger::from_sqlite_store(&store).expect("first init");
        let _logger2 =
            LoopAuditLogger::from_sqlite_store(&store).expect("second init (idempotent)");
        cleanup(tmp);
    }

    // ---- LoopAuditQuery builder ----

    #[test]
    fn query_builder_chains() {
        let q = LoopAuditQuery::new()
            .with_run_id("r1")
            .with_loop_name("l1")
            .with_status("started")
            .with_phase(LoopAuditPhase::LoopStarted)
            .with_time_range(100, 200)
            .with_limit(50);
        assert_eq!(q.run_id.as_deref(), Some("r1"));
        assert_eq!(q.loop_name.as_deref(), Some("l1"));
        assert_eq!(q.status.as_deref(), Some("started"));
        assert_eq!(q.phase, Some(LoopAuditPhase::LoopStarted));
        assert_eq!(q.started_at_from, Some(100));
        assert_eq!(q.started_at_to, Some(200));
        assert_eq!(q.limit, Some(50));
    }

    #[test]
    fn query_default_limit_is_100() {
        let q = LoopAuditQuery::new();
        assert!(q.limit.is_none(), "default limit is None → query uses 100");
    }
}
