//! T-S6-B-03: CRDT op 传播与 LocalTransport 落盘。
//!
//! [`CrdtEngine`](super::crdt::CrdtEngine) 是零 Sized 纯计算单元,不与任何
//! 传输层串联。本模块实现 CRDT op 落盘到 SQLite,并暴露 relay_client 可
//! 消费的 op 流,是 U-08 云中继的隐式前置依赖。
//!
//! ## 数据流
//!
//! 1. 本地记忆变更时,调用 [`CrdtOpLog::record_op`] 将 [`CrdtVersion`]
//!    信息落盘(status='pending')。
//! 2. relay_client 调用 [`CrdtOpLog::fetch_pending_ops`] 拉取未消费的 op。
//! 3. relay_client 成功推送后调用 [`CrdtOpLog::mark_consumed`] 标记已消费;
//!    推送失败时调用 [`CrdtOpLog::mark_failed`],可重试。
//!
//! 表结构见 `migrations/022_crdt_op_log.sql`。

use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

use super::crdt::{CrdtVersion, FieldChange};

/// CRDT op 日志条目 — 对应 `crdt_op_log` 表的一行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdtOpLogEntry {
    pub id: i64,
    pub op_id: String,
    pub memory_id: String,
    pub device_id: String,
    pub version: u64,
    pub timestamp: i64,
    pub field_changes: Vec<FieldChange>,
    /// `pending` / `consumed` / `failed`
    pub status: String,
    pub created_at: i64,
    pub consumed_at: Option<i64>,
}

/// CRDT op 日志统计(供前端仪表盘展示)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdtOpStats {
    pub pending: i64,
    pub consumed: i64,
    pub failed: i64,
    pub total: i64,
}

/// CRDT op 日志存储 — 包装 SqliteStore 的连接,提供 op 记录/消费 API。
///
/// 这是 [`CrdtEngine`](super::crdt::CrdtEngine)(纯计算)与 relay_client
/// (传输)之间的桥梁。所有方法都是同步的,调用方应在 `spawn_blocking`
/// 中执行以避免阻塞 tokio worker 线程(参见 Tauri 命令实现)。
pub struct CrdtOpLog {
    conn: Arc<Mutex<Connection>>,
    /// 本设备标识。当前仅存储供调用方区分 op 来源;未来 relay_client
    /// 可据此过滤本设备已产生的 op 以避免回环。`record_op` 实际写入
    /// 的是 `CrdtVersion.device_id`,与此字段独立。
    device_id: String,
}

impl CrdtOpLog {
    /// 创建新的 CRDT op 日志存储。
    ///
    /// * `conn` — 共享的 SQLite 连接(通常来自 `SqliteStore::raw_connection`)
    /// * `device_id` — 本设备标识
    pub fn new(conn: Arc<Mutex<Connection>>, device_id: String) -> Self {
        Self { conn, device_id }
    }

    /// 本设备标识。
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// 记录一个 CRDT op(本地记忆变更时调用)。
    ///
    /// 生成新的 `op_id`(uuid v4),将 [`CrdtVersion`] 信息落盘为
    /// `status='pending'`。返回生成的 `op_id`。
    #[instrument(skip(self, version), fields(memory_id = %version.memory_id))]
    pub fn record_op(&self, version: &CrdtVersion) -> Result<String> {
        let op_id = uuid::Uuid::new_v4().to_string();
        let field_changes_json =
            serde_json::to_string(&version.field_changes).context("serializing field_changes")?;
        let now = chrono::Utc::now().timestamp();

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO crdt_op_log (op_id, memory_id, device_id, version, timestamp, field_changes, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                op_id,
                version.memory_id,
                version.device_id,
                version.version as i64,
                version.timestamp,
                field_changes_json,
                now
            ],
        )
        .context("inserting crdt op log")?;
        info!(
            target: "nebula.crdt",
            op_id,
            memory_id = %version.memory_id,
            "crdt op recorded"
        );
        Ok(op_id)
    }

    /// 拉取所有 `pending` 状态的 op(供 relay_client 消费)。
    ///
    /// 返回按 `created_at` 升序排列的待处理 op 列表,最多 `limit` 条。
    pub fn fetch_pending_ops(&self, limit: usize) -> Result<Vec<CrdtOpLogEntry>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, op_id, memory_id, device_id, version, timestamp, field_changes, status, created_at, consumed_at
                 FROM crdt_op_log
                 WHERE status = 'pending'
                 ORDER BY created_at ASC
                 LIMIT ?1",
            )
            .context("preparing fetch_pending_ops")?;

        let entries = stmt
            .query_map(params![limit as i64], |row| {
                let field_changes_json: String = row.get(6)?;
                let field_changes: Vec<FieldChange> =
                    serde_json::from_str(&field_changes_json).unwrap_or_default();
                Ok(CrdtOpLogEntry {
                    id: row.get(0)?,
                    op_id: row.get(1)?,
                    memory_id: row.get(2)?,
                    device_id: row.get(3)?,
                    version: row.get::<_, i64>(4)? as u64,
                    timestamp: row.get(5)?,
                    field_changes,
                    status: row.get(7)?,
                    created_at: row.get(8)?,
                    consumed_at: row.get(9)?,
                })
            })
            .context("querying pending ops")?;

        let mut out = Vec::new();
        for entry in entries {
            out.push(entry?);
        }
        Ok(out)
    }

    /// 标记 op 为已消费(relay_client 成功推送后调用)。
    pub fn mark_consumed(&self, op_id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE crdt_op_log SET status = 'consumed', consumed_at = ?1 WHERE op_id = ?2",
            params![now, op_id],
        )
        .context("marking op consumed")?;
        Ok(())
    }

    /// 标记 op 为失败(relay_client 推送失败时调用,可重试)。
    ///
    /// 不写入 `consumed_at`,status 置为 `failed`。失败的 op 不会被
    /// `fetch_pending_ops` 返回;如需重试,调用方可直接 UPDATE 回
    /// `pending`(后续版本可提供 `mark_pending` API)。
    pub fn mark_failed(&self, op_id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE crdt_op_log SET status = 'failed' WHERE op_id = ?1",
            params![op_id],
        )
        .context("marking op failed")?;
        Ok(())
    }

    /// 获取 op 日志统计(供前端仪表盘展示)。
    pub fn stats(&self) -> Result<CrdtOpStats> {
        let conn = self.conn.lock();
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crdt_op_log WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let consumed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crdt_op_log WHERE status = 'consumed'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crdt_op_log WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(CrdtOpStats {
            pending,
            consumed,
            failed,
            total: pending + consumed + failed,
        })
    }

    /// 清理已消费超过 N 天的 op(定期维护)。
    ///
    /// 返回被删除的行数。
    pub fn prune_consumed_older_than(&self, days: i64) -> Result<usize> {
        let cutoff = chrono::Utc::now().timestamp() - days * 86400;
        let conn = self.conn.lock();
        let deleted = conn
            .execute(
                "DELETE FROM crdt_op_log WHERE status = 'consumed' AND consumed_at < ?1",
                params![cutoff],
            )
            .context("pruning old consumed ops")?;
        if deleted > 0 {
            info!(
                target: "nebula.crdt",
                deleted, "pruned old consumed crdt ops"
            );
        }
        Ok(deleted)
    }
}

// ---------------------------------------------------------------------------
// Tauri 命令(未注册到 invoke_handler,使用 #[allow(dead_code)] 标注)
// ---------------------------------------------------------------------------

use crate::commands::error::CommandError;
use crate::AppState;

/// 查询 CRDT op 日志统计。前端仪表盘用于展示待同步/已同步/失败计数。
#[tauri::command]
#[allow(dead_code)]
pub async fn crdt_op_stats(state: tauri::State<'_, AppState>) -> Result<CrdtOpStats, CommandError> {
    let conn = state.memory.sqlite.raw_connection();
    tokio::task::spawn_blocking(move || {
        let log = CrdtOpLog::new(conn, String::new());
        log.stats()
            .map_err(|e| CommandError::db("crdt_op_stats", &e))
    })
    .await
    .map_err(|e| CommandError::internal("crdt_op_stats", &anyhow::anyhow!("{e}")))?
}

/// 拉取 pending 状态的 CRDT op(最多 `limit` 条,按入库时间升序)。
/// 供调试/仪表盘观察待同步队列,relay_client 应直接使用 [`CrdtOpLog`] API。
#[tauri::command]
#[allow(dead_code)]
pub async fn crdt_op_pending(
    state: tauri::State<'_, AppState>,
    limit: usize,
) -> Result<Vec<CrdtOpLogEntry>, CommandError> {
    let conn = state.memory.sqlite.raw_connection();
    tokio::task::spawn_blocking(move || {
        let log = CrdtOpLog::new(conn, String::new());
        log.fetch_pending_ops(limit)
            .map_err(|e| CommandError::db("crdt_op_pending", &e))
    })
    .await
    .map_err(|e| CommandError::internal("crdt_op_pending", &anyhow::anyhow!("{e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造一个内存 SQLite 并手动建表(不依赖 migration runner)。
    fn make_log() -> CrdtOpLog {
        let conn = Connection::open_in_memory().expect("create should succeed");
        conn.execute_batch(
            "CREATE TABLE crdt_op_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                op_id TEXT NOT NULL UNIQUE,
                memory_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                field_changes TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                consumed_at INTEGER
            );
            CREATE INDEX idx_crdt_op_log_status ON crdt_op_log(status);
            CREATE INDEX idx_crdt_op_log_memory ON crdt_op_log(memory_id);",
        )
        .expect("test op should succeed");
        CrdtOpLog::new(Arc::new(Mutex::new(conn)), "dev-test".to_string())
    }

    fn make_version(memory_id: &str, version: u64, ts: i64) -> CrdtVersion {
        CrdtVersion {
            memory_id: memory_id.to_string(),
            version,
            device_id: "dev-test".to_string(),
            timestamp: ts,
            field_changes: vec![FieldChange {
                field: "content".to_string(),
                old_value: json!("old"),
                new_value: json!("new"),
            }],
        }
    }

    #[test]
    fn record_and_fetch_round_trip() {
        let log = make_log();

        let op_id = log
            .record_op(&make_version("m1", 1, 100))
            .expect("test op should succeed");
        assert!(!op_id.is_empty());

        // 第二条 op,created_at 更晚(用稍后的调用时刻)。
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _op_id2 = log
            .record_op(&make_version("m2", 1, 200))
            .expect("test op should succeed");

        let pending = log.fetch_pending_ops(10).expect("get should succeed");
        assert_eq!(pending.len(), 2);
        // 按 created_at 升序:m1 在前。
        assert_eq!(pending[0].memory_id, "m1");
        assert_eq!(pending[1].memory_id, "m2");
        // field_changes 正确反序列化。
        assert_eq!(pending[0].field_changes.len(), 1);
        assert_eq!(pending[0].field_changes[0].new_value, json!("new"));
        // op_id 与返回值一致。
        assert_eq!(pending[0].op_id, op_id);
        // pending 状态正确。
        assert_eq!(pending[0].status, "pending");
        assert!(pending[0].consumed_at.is_none());
    }

    #[test]
    fn mark_consumed_hides_from_pending() {
        let log = make_log();
        let op_id = log
            .record_op(&make_version("m1", 1, 100))
            .expect("test op should succeed");
        let _op_id2 = log
            .record_op(&make_version("m2", 1, 200))
            .expect("test op should succeed");

        // 标记第一条已消费。
        log.mark_consumed(&op_id).expect("test op should succeed");

        let pending = log.fetch_pending_ops(10).expect("get should succeed");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].memory_id, "m2");

        // consumed_at 已写入。
        let stats = log.stats().expect("test op should succeed");
        assert_eq!(stats.consumed, 1);
        assert_eq!(stats.pending, 1);
    }

    #[test]
    fn mark_failed_hides_from_pending() {
        let log = make_log();
        let op_id = log
            .record_op(&make_version("m1", 1, 100))
            .expect("test op should succeed");

        log.mark_failed(&op_id).expect("test op should succeed");

        let pending = log.fetch_pending_ops(10).expect("get should succeed");
        assert!(pending.is_empty());

        let stats = log.stats().expect("test op should succeed");
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.total, 1);
    }

    #[test]
    fn stats_counts_all_states() {
        let log = make_log();
        let a = log
            .record_op(&make_version("m1", 1, 100))
            .expect("test op should succeed");
        let b = log
            .record_op(&make_version("m2", 1, 200))
            .expect("test op should succeed");
        let _c = log
            .record_op(&make_version("m3", 1, 300))
            .expect("test op should succeed");

        log.mark_consumed(&a).expect("test op should succeed");
        log.mark_failed(&b).expect("test op should succeed");
        // c 保持 pending。

        let stats = log.stats().expect("test op should succeed");
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.consumed, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.total, 3);
    }

    #[test]
    fn prune_removes_old_consumed() {
        let log = make_log();
        let op_id = log
            .record_op(&make_version("m1", 1, 100))
            .expect("test op should succeed");
        log.mark_consumed(&op_id).expect("test op should succeed");

        // 手动把 consumed_at 改到 30 天前,模拟过期数据。
        {
            let conn = log.conn.lock();
            let old_ts = chrono::Utc::now().timestamp() - 30 * 86400;
            conn.execute(
                "UPDATE crdt_op_log SET consumed_at = ?1 WHERE op_id = ?2",
                params![old_ts, op_id],
            )
            .expect("test op should succeed");
        }

        // 清理 7 天前的已消费 op。
        let deleted = log
            .prune_consumed_older_than(7)
            .expect("delete should succeed");
        assert_eq!(deleted, 1);

        // 统计应不再含此条。
        let stats = log.stats().expect("test op should succeed");
        assert_eq!(stats.consumed, 0);
        assert_eq!(stats.total, 0);
    }

    #[test]
    fn fetch_pending_respects_limit() {
        let log = make_log();
        for i in 0..5 {
            let _ = log
                .record_op(&make_version(&format!("m{i}"), 1, 100 + i))
                .expect("test op should succeed");
        }
        let pending = log.fetch_pending_ops(3).expect("get should succeed");
        assert_eq!(pending.len(), 3);
    }

    #[test]
    fn device_id_accessor_returns_stored_value() {
        let log = make_log();
        assert_eq!(log.device_id(), "dev-test");
    }
}
