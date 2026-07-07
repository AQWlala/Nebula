//! T-E-S-54: 触发器 SQLite CRUD — `triggers` + `trigger_fire_log` 两张表。
//!
//! 用 `SqliteStore::raw_connection()` 执行参数化查询。所有方法同步,
//! 调用方在命令路径中通过 `tokio::task::spawn_blocking` 包裹(参考
//! `sqlite_store::insert_acl` 模式)。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::memory::sqlite_store::SqliteStore;

/// DB 行(对应 `triggers` 表)。
#[derive(Debug, Clone)]
pub struct TriggerRow {
    pub id: String,
    pub name: String,
    pub enabled: i64,
    pub kind: String,
    pub condition: String,
    pub action_kind: String,
    pub action_payload: String,
    pub created_at: i64,
    pub last_fired_at: Option<i64>,
    pub fire_count: i64,
    pub debounce_ms: i64,
    pub max_fires: Option<i64>,
}

/// DB 行(对应 `trigger_fire_log` 表)。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FireLogRow {
    pub id: i64,
    pub trigger_id: String,
    pub fired_at: i64,
    pub success: bool,
    pub error: Option<String>,
    pub payload: Option<String>,
}

/// DB 行(对应 `watch_state` 表) — T-E-S-55 Watch 触发器轮询状态。
#[derive(Debug, Clone)]
pub struct WatchStateRow {
    pub trigger_id: String,
    pub last_url_hash: Option<String>,
    pub last_value: Option<String>,
    pub last_fire_at: Option<i64>,
}

/// 触发器持久化存储。`Arc<TriggerStore>` 在 `TriggerEngine` 中共享。
pub struct TriggerStore {
    conn: Arc<Mutex<Connection>>,
}

impl TriggerStore {
    pub fn new(sqlite: Arc<SqliteStore>) -> Self {
        Self {
            conn: sqlite.raw_connection(),
        }
    }

    /// 插入新触发器。
    pub fn insert(&self, row: &TriggerRow) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO triggers (
                id, name, enabled, kind, condition, action_kind, action_payload,
                created_at, last_fired_at, fire_count, debounce_ms, max_fires
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                row.id,
                row.name,
                row.enabled,
                row.kind,
                row.condition,
                row.action_kind,
                row.action_payload,
                row.created_at,
                row.last_fired_at,
                row.fire_count,
                row.debounce_ms,
                row.max_fires,
            ],
        )
        .map_err(|e| anyhow!("sqlite insert trigger error: {e}"))?;
        Ok(())
    }

    /// 列出所有触发器(按 created_at ASC)。
    pub fn list(&self) -> Result<Vec<TriggerRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, enabled, kind, condition, action_kind, action_payload,
                        created_at, last_fired_at, fire_count, debounce_ms, max_fires
                 FROM triggers ORDER BY created_at ASC",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map([], row_to_trigger)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 按 id 获取单个触发器。
    pub fn get(&self, id: &str) -> Result<TriggerRow> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, name, enabled, kind, condition, action_kind, action_payload,
                    created_at, last_fired_at, fire_count, debounce_ms, max_fires
             FROM triggers WHERE id = ?1",
            params![id],
            row_to_trigger,
        )
        .map_err(|e| anyhow!("sqlite get trigger error: {e}"))
    }

    /// 删除触发器。
    pub fn delete(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM triggers WHERE id = ?1", params![id])
            .map_err(|e| anyhow!("sqlite delete trigger error: {e}"))?;
        Ok(())
    }

    /// 更新 enabled 状态。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE triggers SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )
        .map_err(|e| anyhow!("sqlite set_enabled error: {e}"))?;
        Ok(())
    }

    /// 记录一次触发:更新 fire_count + last_fired_at,并写 fire_log。
    pub fn record_fire(
        &self,
        trigger_id: &str,
        fired_at: i64,
        success: i64,
        error: Option<&str>,
        payload: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock();
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("sqlite begin transaction error: {e}"))?;
        tx.execute(
            "UPDATE triggers SET fire_count = fire_count + 1, last_fired_at = ?1 WHERE id = ?2",
            params![fired_at, trigger_id],
        )
        .map_err(|e| anyhow!("sqlite update fire_count error: {e}"))?;
        tx.execute(
            "INSERT INTO trigger_fire_log (trigger_id, fired_at, success, error, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![trigger_id, fired_at, success, error, payload],
        )
        .map_err(|e| anyhow!("sqlite insert fire_log error: {e}"))?;
        tx.commit()
            .map_err(|e| anyhow!("sqlite record_fire commit error: {e}"))?;
        Ok(())
    }

    /// 列出指定触发器的触发日志(按 fired_at DESC,最多 100 条)。
    pub fn list_fire_log(&self, trigger_id: &str) -> Result<Vec<FireLogRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, trigger_id, fired_at, success, error, payload
                 FROM trigger_fire_log WHERE trigger_id = ?1
                 ORDER BY fired_at DESC LIMIT 100",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map(params![trigger_id], |row| {
                Ok(FireLogRow {
                    id: row.get(0)?,
                    trigger_id: row.get(1)?,
                    fired_at: row.get(2)?,
                    success: row.get::<_, i64>(3)? != 0,
                    error: row.get(4)?,
                    payload: row.get(5)?,
                })
            })
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    // ---- watch_state CRUD(T-E-S-55) ----

    /// 获取 Watch 触发器的轮询状态。
    pub fn get_watch_state(&self, trigger_id: &str) -> Result<Option<WatchStateRow>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT trigger_id, last_url_hash, last_value, last_fire_at
             FROM watch_state WHERE trigger_id = ?1",
            params![trigger_id],
            |row| {
                Ok(WatchStateRow {
                    trigger_id: row.get(0)?,
                    last_url_hash: row.get(1)?,
                    last_value: row.get(2)?,
                    last_fire_at: row.get(3)?,
                })
            },
        );
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("sqlite get_watch_state error: {e}")),
        }
    }

    /// 写入/更新 Watch 触发器的轮询状态(INSERT OR REPLACE upsert)。
    /// `None` 字段表示不更新该列(用 COALESCE 保留原值)。
    pub fn set_watch_state(
        &self,
        trigger_id: &str,
        last_url_hash: Option<&str>,
        last_value: Option<&str>,
        last_fire_at: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO watch_state (trigger_id, last_url_hash, last_value, last_fire_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(trigger_id) DO UPDATE SET
                last_url_hash = COALESCE(excluded.last_url_hash, watch_state.last_url_hash),
                last_value = COALESCE(excluded.last_value, watch_state.last_value),
                last_fire_at = COALESCE(excluded.last_fire_at, watch_state.last_fire_at)",
            params![trigger_id, last_url_hash, last_value, last_fire_at],
        )
        .map_err(|e| anyhow!("sqlite set_watch_state error: {e}"))?;
        Ok(())
    }

    /// 删除 Watch 触发器的轮询状态(触发器删除时调用)。
    pub fn delete_watch_state(&self, trigger_id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM watch_state WHERE trigger_id = ?1",
            params![trigger_id],
        )
        .map_err(|e| anyhow!("sqlite delete_watch_state error: {e}"))?;
        Ok(())
    }
}

/// Row → TriggerRow 转换。
fn row_to_trigger(row: &rusqlite::Row<'_>) -> rusqlite::Result<TriggerRow> {
    Ok(TriggerRow {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get(2)?,
        kind: row.get(3)?,
        condition: row.get(4)?,
        action_kind: row.get(5)?,
        action_payload: row.get(6)?,
        created_at: row.get(7)?,
        last_fired_at: row.get(8)?,
        fire_count: row.get(9)?,
        debounce_ms: row.get(10)?,
        max_fires: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_store() -> (TriggerStore, std::path::PathBuf) {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nebula_trigger_test_{}_{}.db",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_file(&path);
        let sqlite = SqliteStore::open(&path).expect("open sqlite");
        // 跑 bundled migrations 以确保 triggers 表存在。
        {
            let conn = sqlite.raw_connection();
            let g = conn.lock();
            crate::memory::migration::run_bundled_migrations(&g).expect("run migrations");
        }
        let store = TriggerStore::new(Arc::new(sqlite));
        (store, path)
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    fn sample_row(id: &str) -> TriggerRow {
        TriggerRow {
            id: id.to_string(),
            name: format!("trigger-{id}"),
            enabled: 1,
            kind: "message".to_string(),
            condition: r#"{"kind":"message","event_kind":"agent_completed"}"#.to_string(),
            action_kind: "notify".to_string(),
            action_payload: r#"{"kind":"notify","title":"T","body":"B"}"#.to_string(),
            created_at: 1700000000_000,
            last_fired_at: None,
            fire_count: 0,
            debounce_ms: 1000,
            max_fires: Some(5),
        }
    }

    #[test]
    fn test_trigger_store_crud() {
        let (store, path) = temp_store();

        // 插入。
        store.insert(&sample_row("t1")).unwrap();
        store.insert(&sample_row("t2")).unwrap();

        // list。
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);

        // get。
        let got = store.get("t1").unwrap();
        assert_eq!(got.name, "trigger-t1");
        assert_eq!(got.fire_count, 0);

        // set_enabled。
        store.set_enabled("t1", false).unwrap();
        let got = store.get("t1").unwrap();
        assert_eq!(got.enabled, 0);

        // record_fire。
        store
            .record_fire("t1", 1700000001_000, 1, None, Some(r#"{"x":1}"#))
            .unwrap();
        let got = store.get("t1").unwrap();
        assert_eq!(got.fire_count, 1);
        assert_eq!(got.last_fired_at, Some(1700000001_000));

        // list_fire_log。
        let logs = store.list_fire_log("t1").unwrap();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].success);
        assert_eq!(logs[0].payload.as_deref(), Some(r#"{"x":1}"#));

        // delete。
        store.delete("t1").unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);

        cleanup(&path);
    }

    #[test]
    fn test_trigger_store_record_fire_multiple() {
        let (store, path) = temp_store();
        store.insert(&sample_row("t1")).unwrap();
        for i in 0..3 {
            store
                .record_fire("t1", 1700000000_000 + i, 1, None, None)
                .unwrap();
        }
        let got = store.get("t1").unwrap();
        assert_eq!(got.fire_count, 3);
        let logs = store.list_fire_log("t1").unwrap();
        assert_eq!(logs.len(), 3);
        // DESC 排序:最新的在前。
        assert_eq!(logs[0].fired_at, 1700000000002);
        cleanup(&path);
    }

    #[test]
    fn test_trigger_store_failed_fire_logs_error() {
        let (store, path) = temp_store();
        store.insert(&sample_row("t1")).unwrap();
        store.record_fire("t1", 1, 0, Some("boom"), None).unwrap();
        let logs = store.list_fire_log("t1").unwrap();
        assert_eq!(logs.len(), 1);
        assert!(!logs[0].success);
        assert_eq!(logs[0].error.as_deref(), Some("boom"));
        cleanup(&path);
    }

    #[test]
    fn test_watch_state_crud() {
        let (store, path) = temp_store();

        // 初始无状态。
        assert!(store.get_watch_state("w1").unwrap().is_none());

        // 写入 url_hash + fire_at。
        store
            .set_watch_state("w1", Some("hash_v1"), None, Some(1700000000_000))
            .unwrap();
        let row = store.get_watch_state("w1").unwrap().unwrap();
        assert_eq!(row.trigger_id, "w1");
        assert_eq!(row.last_url_hash.as_deref(), Some("hash_v1"));
        assert_eq!(row.last_value, None);
        assert_eq!(row.last_fire_at, Some(1700000000_000));

        // 更新 last_value(COALESCE 保留 last_url_hash + last_fire_at)。
        store
            .set_watch_state("w1", None, Some("evt_uid_1"), None)
            .unwrap();
        let row = store.get_watch_state("w1").unwrap().unwrap();
        assert_eq!(row.last_url_hash.as_deref(), Some("hash_v1")); // 保留
        assert_eq!(row.last_value.as_deref(), Some("evt_uid_1")); // 新值
        assert_eq!(row.last_fire_at, Some(1700000000_000)); // 保留

        // 更新 url_hash(其他列保留)。
        store
            .set_watch_state("w1", Some("hash_v2"), None, None)
            .unwrap();
        let row = store.get_watch_state("w1").unwrap().unwrap();
        assert_eq!(row.last_url_hash.as_deref(), Some("hash_v2"));
        assert_eq!(row.last_value.as_deref(), Some("evt_uid_1")); // 保留

        // 删除。
        store.delete_watch_state("w1").unwrap();
        assert!(store.get_watch_state("w1").unwrap().is_none());

        cleanup(&path);
    }
}
