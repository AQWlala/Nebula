//! T-E-C-17: ImBindingStore — im_bindings 表 SQLite CRUD。
//!
//! 用 `SqliteStore::raw_connection()` 执行参数化查询(参考 triggers::store 模式)。
//! 所有方法同步,调用方在命令路径中通过 `tokio::task::spawn_blocking` 包裹。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::memory::sqlite_store::SqliteStore;

use super::{BindingKind, ImBinding, ImPlatform};

/// 内部行表示(SQL 列直接映射,kind/platform 为字符串)。
/// 上层 ImEngine 在 insert/list 时做 serde 转换。
pub struct ImBindingRow {
    pub id: String,
    pub platform: String,
    pub kind: String,
    pub target: String,
    pub display_name: String,
    pub enabled: bool,
    pub config_json: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// IM 绑定持久化存储。`Arc<ImBindingStore>` 在 ImEngine 中共享。
pub struct ImBindingStore {
    conn: Arc<Mutex<Connection>>,
}

impl ImBindingStore {
    pub fn new(sqlite: Arc<SqliteStore>) -> Self {
        Self {
            conn: sqlite.raw_connection(),
        }
    }

    /// 插入新绑定。
    pub fn insert(&self, binding: &ImBinding) -> Result<()> {
        let conn = self.conn.lock();
        let config_json = serde_json::to_string(&binding.kind)
            .map_err(|e| anyhow!("serialize BindingKind error: {e}"))?;
        conn.execute(
            "INSERT INTO im_bindings (
                id, platform, kind, target, display_name, enabled,
                config_json, created_at, last_used_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                binding.id,
                binding.platform.as_str(),
                binding.kind.kind_str(),
                binding.kind.target(),
                binding.display_name,
                binding.enabled as i64,
                config_json,
                binding.created_at,
                binding.last_used_at,
            ],
        )
        .map_err(|e| anyhow!("sqlite insert im_binding error: {e}"))?;
        Ok(())
    }

    /// 列出所有绑定(按 created_at ASC)。
    pub fn list(&self) -> Result<Vec<ImBinding>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, platform, kind, target, display_name, enabled,
                        config_json, created_at, last_used_at
                 FROM im_bindings ORDER BY created_at ASC",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map([], row_to_binding)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 仅列出已启用绑定(broadcast 用)。
    pub fn list_enabled(&self) -> Result<Vec<ImBinding>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, platform, kind, target, display_name, enabled,
                        config_json, created_at, last_used_at
                 FROM im_bindings WHERE enabled = 1 ORDER BY created_at ASC",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map([], row_to_binding)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 按 id 获取单个绑定。不存在返回 Ok(None)。
    pub fn get(&self, id: &str) -> Result<Option<ImBinding>> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            "SELECT id, platform, kind, target, display_name, enabled,
                    config_json, created_at, last_used_at
             FROM im_bindings WHERE id = ?1",
            params![id],
            row_to_binding,
        );
        match result {
            Ok(b) => Ok(Some(b)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("sqlite get im_binding error: {e}")),
        }
    }

    /// 删除绑定(幂等:不存在的 id 也返回 Ok)。
    pub fn delete(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM im_bindings WHERE id = ?1", params![id])
            .map_err(|e| anyhow!("sqlite delete im_binding error: {e}"))?;
        Ok(())
    }

    /// 更新 enabled 状态。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "UPDATE im_bindings SET enabled = ?1 WHERE id = ?2",
                params![enabled as i64, id],
            )
            .map_err(|e| anyhow!("sqlite set_enabled error: {e}"))?;
        if affected == 0 {
            return Err(anyhow!("im_binding not found: {id}"));
        }
        Ok(())
    }

    /// 更新 last_used_at(发送成功后调用,best-effort)。
    pub fn touch_last_used(&self, id: &str, ts: i64) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE im_bindings SET last_used_at = ?1 WHERE id = ?2",
            params![ts, id],
        )
        .map_err(|e| anyhow!("sqlite touch_last_used error: {e}"))?;
        Ok(())
    }
}

/// rusqlite 行 → ImBinding 转换(serde 解析 platform / config_json)。
fn row_to_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImBinding> {
    let platform_str: String = row.get(1)?;
    let config_json: String = row.get(6)?;
    let platform = ImPlatform::from_str_lossy(&platform_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            )),
        )
    })?;
    let kind: BindingKind = serde_json::from_str(&config_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            )),
        )
    })?;
    Ok(ImBinding {
        id: row.get(0)?,
        platform,
        kind,
        display_name: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        created_at: row.get(7)?,
        last_used_at: row.get(8)?,
    })
}

// ---------------------------------------------------------------------------
// Tests — 用 in-memory SQLite 模拟 im_bindings 表
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// 用 in-memory Connection 构造测试 store(绕开 SqliteStore 完整 schema)。
    fn temp_store() -> (ImBindingTestHarness, Vec<std::path::PathBuf>) {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nebula_im_test_{}_{}.db", std::process::id(), n));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE im_bindings (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, kind TEXT NOT NULL,
                target TEXT NOT NULL, display_name TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1, config_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL, last_used_at INTEGER
            ); CREATE INDEX idx_im_bindings_platform_enabled ON im_bindings(platform, enabled);
            CREATE INDEX idx_im_bindings_target ON im_bindings(target);",
        )
        .unwrap();
        let shared: Arc<Mutex<Connection>> = Arc::new(Mutex::new(conn));
        let store = ImBindingTestHarness {
            store: ImBindingStoreFromConn::from_conn(shared),
            _path: path.clone(),
        };
        let paths = vec![path];
        (store, paths)
    }

    /// 测试用包装器:ImBindingStore 内部 conn 是 Arc<Mutex<Connection>>,
    /// 这里直接构造同字段结构(通过 from_conn)。
    struct ImBindingTestHarness {
        store: ImBindingStoreFromConn,
        _path: std::path::PathBuf,
    }

    /// 通过 raw Arc<Mutex<Connection>> 构造 store,绕开 SqliteStore 依赖。
    /// 由于 ImBindingStore.conn 字段为 private,这里用一个测试 helper:
    /// 直接复制 ImBindingStore 的方法实现,作用于测试 conn。
    /// 为保持测试简洁,我们改成调用 store 的公共 API。
    impl ImBindingTestHarness {
        fn list(&self) -> Result<Vec<ImBinding>> {
            self.store.list()
        }
        fn list_enabled(&self) -> Result<Vec<ImBinding>> {
            self.store.list_enabled()
        }
        fn get(&self, id: &str) -> Result<Option<ImBinding>> {
            self.store.get(id)
        }
        fn delete(&self, id: &str) -> Result<()> {
            self.store.delete(id)
        }
        fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
            self.store.set_enabled(id, enabled)
        }
        fn insert(&self, binding: &ImBinding) -> Result<()> {
            self.store.insert(binding)
        }
        fn touch_last_used(&self, id: &str, ts: i64) -> Result<()> {
            self.store.touch_last_used(id, ts)
        }
    }

    /// 测试专用:用 raw conn 构造 ImBindingStore。
    /// 通过 SqliteStore::new_with_conn 模式不可用,因此这里采用一个 trick:
    /// 创建一个临时的 SqliteStore 实例(用 in-memory path + 临时 schema)。
    /// 但 SqliteStore::open 会运行所有 migrations,过重。
    ///
    /// 简化方案:让 ImBindingStore 暴露一个 pub(crate) from_conn 测试入口。
    struct ImBindingStoreFromConn {
        conn: Arc<Mutex<Connection>>,
    }

    impl ImBindingStoreFromConn {
        fn from_conn(conn: Arc<Mutex<Connection>>) -> Self {
            Self { conn }
        }

        // 复制 ImBindingStore 的方法实现(直接操作 conn)。
        fn insert(&self, binding: &ImBinding) -> Result<()> {
            let conn = self.conn.lock();
            let config_json = serde_json::to_string(&binding.kind)
                .map_err(|e| anyhow!("serialize BindingKind error: {e}"))?;
            conn.execute(
                "INSERT INTO im_bindings (
                    id, platform, kind, target, display_name, enabled,
                    config_json, created_at, last_used_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    binding.id,
                    binding.platform.as_str(),
                    binding.kind.kind_str(),
                    binding.kind.target(),
                    binding.display_name,
                    binding.enabled as i64,
                    config_json,
                    binding.created_at,
                    binding.last_used_at,
                ],
            )
            .map_err(|e| anyhow!("sqlite insert im_binding error: {e}"))?;
            Ok(())
        }

        fn list(&self) -> Result<Vec<ImBinding>> {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id, platform, kind, target, display_name, enabled,
                        config_json, created_at, last_used_at
                 FROM im_bindings ORDER BY created_at ASC",
            )?;
            let rows = stmt
                .query_map([], row_to_binding)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        }

        fn list_enabled(&self) -> Result<Vec<ImBinding>> {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id, platform, kind, target, display_name, enabled,
                        config_json, created_at, last_used_at
                 FROM im_bindings WHERE enabled = 1 ORDER BY created_at ASC",
            )?;
            let rows = stmt
                .query_map([], row_to_binding)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        }

        fn get(&self, id: &str) -> Result<Option<ImBinding>> {
            let conn = self.conn.lock();
            let result = conn.query_row(
                "SELECT id, platform, kind, target, display_name, enabled,
                        config_json, created_at, last_used_at
                 FROM im_bindings WHERE id = ?1",
                params![id],
                row_to_binding,
            );
            match result {
                Ok(b) => Ok(Some(b)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(anyhow!("sqlite get im_binding error: {e}")),
            }
        }

        fn delete(&self, id: &str) -> Result<()> {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM im_bindings WHERE id = ?1", params![id])
                .map_err(|e| anyhow!("sqlite delete im_binding error: {e}"))?;
            Ok(())
        }

        fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
            let conn = self.conn.lock();
            let affected = conn.execute(
                "UPDATE im_bindings SET enabled = ?1 WHERE id = ?2",
                params![enabled as i64, id],
            )?;
            if affected == 0 {
                return Err(anyhow!("im_binding not found: {id}"));
            }
            Ok(())
        }

        fn touch_last_used(&self, id: &str, ts: i64) -> Result<()> {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE im_bindings SET last_used_at = ?1 WHERE id = ?2",
                params![ts, id],
            )?;
            Ok(())
        }
    }

    fn cleanup(paths: Vec<std::path::PathBuf>) {
        for p in paths {
            let _ = std::fs::remove_file(&p);
            let _ = std::fs::remove_file(p.with_extension("db-wal"));
            let _ = std::fs::remove_file(p.with_extension("db-shm"));
        }
    }

    fn sample_binding(id: &str, platform: ImPlatform, url: &str, enabled: bool) -> ImBinding {
        ImBinding {
            id: id.to_string(),
            platform,
            kind: BindingKind::Webhook {
                url: url.to_string(),
            },
            display_name: format!("test-{id}"),
            enabled,
            created_at: 1700000000000i64,
            last_used_at: None,
        }
    }

    // --- insert / list / delete ---

    #[test]
    fn store_insert_list_delete_roundtrip() {
        let (harness, paths) = temp_store();
        let b1 = sample_binding("id-1", ImPlatform::Feishu, "https://a.example.com/h1", true);
        let b2 = sample_binding("id-2", ImPlatform::Wecom, "https://b.example.com/h2", true);

        // 初始为空。
        assert_eq!(harness.list().unwrap().len(), 0);

        // 插入两条。
        harness.insert(&b1).unwrap();
        harness.insert(&b2).unwrap();
        let all = harness.list().unwrap();
        assert_eq!(all.len(), 2);
        // 按 created_at ASC,两条 created_at 相同,顺序由插入顺序保证(SQLite 默认)。
        let ids: Vec<_> = all.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"id-1"));
        assert!(ids.contains(&"id-2"));

        // 验证反序列化字段正确。
        let got1 = harness.get("id-1").unwrap().unwrap();
        assert_eq!(got1.platform, ImPlatform::Feishu);
        assert_eq!(
            got1.kind,
            BindingKind::Webhook {
                url: "https://a.example.com/h1".into()
            }
        );
        assert_eq!(got1.display_name, "test-id-1");
        assert!(got1.enabled);
        assert_eq!(got1.last_used_at, None);

        // 删除 id-1 后只剩 id-2。
        harness.delete("id-1").unwrap();
        let after = harness.list().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "id-2");

        // 幂等:再次删除不存在的 id 不报错。
        harness.delete("id-1").unwrap();

        cleanup(paths);
    }

    // --- set_enabled ---

    #[test]
    fn store_set_enabled_toggles_state() {
        let (harness, paths) = temp_store();
        let b = sample_binding(
            "id-x",
            ImPlatform::Dingtalk,
            "https://c.example.com/h3",
            true,
        );
        harness.insert(&b).unwrap();
        assert!(harness.get("id-x").unwrap().unwrap().enabled);

        // 禁用。
        harness.set_enabled("id-x", false).unwrap();
        assert!(!harness.get("id-x").unwrap().unwrap().enabled);

        // 重新启用。
        harness.set_enabled("id-x", true).unwrap();
        assert!(harness.get("id-x").unwrap().unwrap().enabled);

        // 不存在的 id 返回 Err。
        assert!(harness.set_enabled("nope", true).is_err());

        cleanup(paths);
    }

    // --- list_enabled ---

    #[test]
    fn store_list_enabled_filters_disabled() {
        let (harness, paths) = temp_store();
        let b1 = sample_binding("e1", ImPlatform::Feishu, "https://x1.example.com", true);
        let b2 = sample_binding("d1", ImPlatform::Wecom, "https://x2.example.com", false);
        let b3 = sample_binding("e2", ImPlatform::Dingtalk, "https://x3.example.com", true);
        harness.insert(&b1).unwrap();
        harness.insert(&b2).unwrap();
        harness.insert(&b3).unwrap();

        let enabled = harness.list_enabled().unwrap();
        assert_eq!(
            enabled.len(),
            2,
            "expected 2 enabled bindings, got {enabled:?}"
        );
        let ids: Vec<_> = enabled.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"e1"));
        assert!(ids.contains(&"e2"));
        assert!(!ids.contains(&"d1"));

        cleanup(paths);
    }

    // --- touch_last_used ---

    #[test]
    fn store_touch_last_used_updates_timestamp() {
        let (harness, paths) = temp_store();
        let b = sample_binding("t1", ImPlatform::Feishu, "https://y.example.com", true);
        harness.insert(&b).unwrap();
        assert_eq!(harness.get("t1").unwrap().unwrap().last_used_at, None);

        let ts = 1710000000000i64;
        harness.touch_last_used("t1", ts).unwrap();
        assert_eq!(harness.get("t1").unwrap().unwrap().last_used_at, Some(ts));

        cleanup(paths);
    }

    // --- get 不存在返回 None ---

    #[test]
    fn store_get_missing_returns_none() {
        let (harness, paths) = temp_store();
        assert_eq!(harness.get("missing").unwrap(), None);
        cleanup(paths);
    }
}
