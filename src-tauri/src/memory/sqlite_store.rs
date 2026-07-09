//! SQLite-backed structured store for [`Memory`] records.
//!
//! The store uses `rusqlite` which is synchronous, so all database
//! I/O operations are wrapped in `tokio::task::spawn_blocking` to
//! avoid blocking the tokio worker threads. WAL mode is enabled for
//! concurrent readers.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use tracing::{debug, info};

use super::types::{
    Memory, MemoryLayer, MemoryRelation, MemoryType, MultiGranularity, RelationKind, SourceKind,
};

// T-E-B-14: exposed pub(crate) so the query_dsl translator can reuse
// the canonical column list when building SELECT statements. Keeping
// a single source of truth avoids drift between the two code paths.
//
// T-E-A-09: 追加 ingest_cost 列(向后兼容:旧记忆为 NULL)。
// M2a #28: 追加 domain 列(向后兼容:旧记忆为 NULL → 容错回退 "shared")。
pub(crate) const MEMORY_COLUMNS: &str = "id, memory_type, layer, content, summary_50, summary_150, summary_500, summary_2000, importance, access_count, last_access, created_at, source, metadata, compressed_from, compression_gen, pinned, archived, ingest_cost, domain";

macro_rules! sel_mem {
    ($rest:expr) => {
        concat!("SELECT id, memory_type, layer, content, summary_50, summary_150, summary_500, summary_2000, importance, access_count, last_access, created_at, source, metadata, compressed_from, compression_gen, pinned, archived, ingest_cost, domain FROM ", $rest)
    };
}

/// Thread-safe SQLite store for memory records.
///
/// Cloning is cheap: the underlying [`Connection`] is wrapped in an
/// `Arc<Mutex<_>>` so multiple Tauri commands can issue queries
/// concurrently.
#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
    /// v1.0.1 P0#10: an additional process-wide lock acquired
    /// around the *whole* blackhole compression pass and the
    /// sponge `absorb` write path.  This is **not** the same as
    /// `conn` — `conn` is per-statement, this is a cross-call
    /// guard that ensures a sponge read cannot observe a row
    /// that's in the middle of being rewritten by a blackhole
    /// pass (the "partial compression" race).
    ///
    /// Trade-off: we briefly serialise absorb / compress, which
    /// is a small latency cost.  In exchange, the sponge
    /// `absorb` reader can no longer race against a `compress`
    /// writer for the same `memories.content` cell.  The cost is
    /// paid in milliseconds at most (the compress_group inner
    /// work is local); the alternative — a partial read of a
    /// half-compressed cell — is a correctness bug.
    compression_lock: Arc<Mutex<()>>,
}

impl SqliteStore {
    /// Opens (or creates) the database at `path` and runs the bundled
    /// migration SQL.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("creating parent dir for sqlite db: {}", parent.display())
                })?;
            }
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite db at {}", path.display()))?;
        // Performance & correctness pragmas.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;

        // Apply the bundled migration. We embed the SQL at compile time.
        const SCHEMA: &str = include_str!("../../migrations/001_initial.sql");
        conn.execute_batch(SCHEMA)
            .context("applying initial migration")?;

        // v1.1: apply all pending migrations (002..016) so that
        // columns like `archived` (migration 014) exist.  The
        // migration runner is idempotent — it stamps
        // `PRAGMA user_version = 1` via `bootstrap_v0_1_baseline`
        // (since 001_initial.sql already inserted the
        // `schema_version` row) and then applies every migration
        // whose version is strictly greater than 1.
        super::migration::run_bundled_migrations(&conn).context("applying pending migrations")?;

        info!(target: "nebula.memory", path = %path.display(), "sqlite store ready");
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            compression_lock: Arc::new(Mutex::new(())),
        })
    }

    /// T-E-S-43: 打开(或创建)加密的 SQLite 数据库。
    ///
    /// 与 [`open`](Self::open) 相同的 PRAGMA + migration 流程,但在任何
    /// 其他 SQL 之前注入 `PRAGMA key = '<key>'`(SQLCipher 要求 key 在
    /// 任何 SQL 之前)。验证 `cipher_version` 返回非空(确认 SQLCipher
    /// 已编译),并验证 key 正确(`SELECT count(*) FROM sqlite_master`,
    /// key 错则 "file is not a database")。
    ///
    /// **PRAGMA 顺序**:key → cipher_version 验证 → key 验证 → WAL →
    /// synchronous → foreign_keys → temp_store → mmap_size → migrations。
    ///
    /// **feature gate**:仅在 `sqlcipher` feature 启用时编译。无 feature
    /// 时调用方应回退到 [`open`](Self::open)。`bootstrap_storage` 根据
    /// `db_encryption_enabled` + key 是否存在决定走哪条路径。
    ///
    /// **不破坏现有 `open(path)` 签名**:30+ 测试依赖 `open`,本方法
    /// 是新增的独立签名。
    #[cfg(feature = "sqlcipher")]
    pub fn open_encrypted<P: AsRef<Path>>(path: P, key: &str) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("creating parent dir for sqlite db: {}", parent.display())
                })?;
            }
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite db at {}", path.display()))?;

        // 1. PRAGMA key — 必须在任何其他 SQL 之前(SQLCipher 要求)。
        //    若 sqlcipher 未编译,此 pragma_update 会失败。
        conn.pragma_update(None, "key", key)
            .context("setting PRAGMA key (sqlcipher not compiled?)")?;

        // 2. 验证 cipher_version 非空(确认 SQLCipher 已编译)。
        //    PRAGMA cipher_version 返回类似 "4.5.5 community" 的版本字符串。
        let cipher_ver: String = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .context("querying PRAGMA cipher_version (sqlcipher not compiled?)")?;
        if cipher_ver.is_empty() {
            return Err(anyhow!(
                "PRAGMA cipher_version returned empty — sqlcipher not compiled"
            ));
        }

        // 3. 验证 key 正确:`SELECT count(*) FROM sqlite_master`。
        //    key 错则 "file is not a database" 错误(SQLCipher 无法解密页)。
        //    新 DB(首次创建)此查询返回 0(schema 为空),也通过验证。
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .context("verifying db key (wrong key? file is not a database)")?;

        // 4. 设置现有 PRAGMA(WAL / synchronous / foreign_keys / temp_store / mmap_size)。
        //    与 open() 保持一致,确保加密 DB 与明文 DB 行为相同。
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "mmap_size", 268_435_456_i64)?;

        // 5. 运行 migrations(001_initial.sql + run_bundled_migrations)。
        const SCHEMA: &str = include_str!("../../migrations/001_initial.sql");
        conn.execute_batch(SCHEMA)
            .context("applying initial migration")?;
        super::migration::run_bundled_migrations(&conn).context("applying pending migrations")?;

        info!(
            target: "nebula.memory",
            path = %path.display(),
            cipher_version = %cipher_ver,
            "encrypted sqlite store ready"
        );
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            compression_lock: Arc::new(Mutex::new(())),
        })
    }

    /// v1.0.1 P0#10: process-wide compression lock.
    ///
    /// Held by `BlackholeEngine::run_pass` for the duration of a
    /// compression pass, and by `sponge::absorb` for the
    /// duration of a merge write.  Acquired as a `MutexGuard` so
    /// callers can use `let _g = store.compression_lock();` to
    /// scope the critical section.
    pub fn compression_lock(&self) -> parking_lot::MutexGuard<'_, ()> {
        self.compression_lock.lock()
    }

    /// v1.1: Insert a memory under the compression lock.
    /// This is a synchronous method intended to be called inside
    /// `spawn_blocking` so the lock is held for the duration of
    /// the SQLite write and released before any `.await` point.
    pub fn insert_guarded(&self, m: &Memory) -> Result<()> {
        let _g = self.compression_lock.lock();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO memories (
                id, memory_type, layer, content,
                summary_50, summary_150, summary_500, summary_2000,
                importance, access_count, last_access, created_at,
                source, metadata, compressed_from, compression_gen, pinned, archived, ingest_cost, domain
            ) VALUES (
                ?1, ?2, ?3, ?4,
                ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
            )",
            params![
                m.id,
                m.memory_type.as_str(),
                m.layer.as_str(),
                m.content,
                m.summary.s50,
                m.summary.s150,
                m.summary.s500,
                m.summary.s2000,
                m.importance,
                m.access_count,
                m.last_access,
                m.created_at,
                m.source.as_str(),
                m.metadata.to_string(),
                m.compressed_from,
                m.compression_gen,
                m.pinned as i32,
                m.archived as i32,
                m.ingest_cost,
                m.domain,
            ],
        )
        .map_err(|e| anyhow!("sqlite insert_guarded error: {e}"))?;
        debug!(target: "nebula.memory", id = %m.id, layer = %m.layer, "inserted memory (guarded)");
        Ok(())
    }

    /// v1.1: Update a memory under the compression lock.
    /// Same rationale as `insert_guarded`.
    pub fn update_guarded(&self, m: &Memory) -> Result<()> {
        let _g = self.compression_lock.lock();
        let conn = self.conn.lock();
        let affected = conn
            .execute(
                "UPDATE memories SET
                memory_type = ?2,
                layer = ?3,
                content = ?4,
                summary_50 = ?5,
                summary_150 = ?6,
                summary_500 = ?7,
                summary_2000 = ?8,
                importance = ?9,
                access_count = ?10,
                last_access = ?11,
                source = ?12,
                metadata = ?13,
                compressed_from = ?14,
                compression_gen = ?15,
                pinned = ?16,
                ingest_cost = ?17,
                domain = ?18
             WHERE id = ?1",
                params![
                    m.id,
                    m.memory_type.as_str(),
                    m.layer.as_str(),
                    m.content,
                    m.summary.s50,
                    m.summary.s150,
                    m.summary.s500,
                    m.summary.s2000,
                    m.importance,
                    m.access_count,
                    m.last_access,
                    m.source.as_str(),
                    m.metadata.to_string(),
                    m.compressed_from,
                    m.compression_gen,
                    m.pinned as i32,
                    m.ingest_cost,
                    m.domain,
                ],
            )
            .map_err(|e| anyhow!("sqlite update_guarded error: {e}"))?;
        if affected == 0 {
            return Err(anyhow!("memory not found"));
        }
        Ok(())
    }

    pub async fn insert_guarded_spawn(&self, m: &Memory) -> Result<()> {
        let this = self.clone();
        let m = m.clone();
        tokio::task::spawn_blocking(move || this.insert_guarded(&m))
            .await
            .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    pub async fn update_guarded_spawn(&self, m: &Memory) -> Result<()> {
        let this = self.clone();
        let m = m.clone();
        tokio::task::spawn_blocking(move || this.update_guarded(&m))
            .await
            .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }
    // spawn_blocking to avoid blocking tokio worker threads.
    // The blocking SQLite calls are isolated in spawn_blocking closures.

    /// Inserts a new memory. The caller is expected to have already filled in the
    /// embedding and summaries.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn insert(&self, m: &Memory) -> Result<()> {
        let conn = self.conn.clone();
        let m_id = m.id.clone();
        let m_type = m.memory_type.as_str().to_string();
        let m_layer = m.layer.as_str().to_string();
        let m_content = m.content.clone();
        let m_s50 = m.summary.s50.clone();
        let m_s150 = m.summary.s150.clone();
        let m_s500 = m.summary.s500.clone();
        let m_s2000 = m.summary.s2000.clone();
        let m_importance = m.importance;
        let m_access_count = m.access_count;
        let m_last_access = m.last_access;
        let m_created_at = m.created_at;
        let m_source = m.source.as_str().to_string();
        let m_metadata = m.metadata.to_string();
        let m_compressed_from = m.compressed_from.clone();
        let m_compression_gen = m.compression_gen;
        let m_pinned = m.pinned as i32;
        let m_archived = m.archived as i32;
        let m_ingest_cost = m.ingest_cost;
        let m_domain = m.domain.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.execute(
                "INSERT INTO memories (
                    id, memory_type, layer, content,
                    summary_50, summary_150, summary_500, summary_2000,
                    importance, access_count, last_access, created_at,
                    source, metadata, compressed_from, compression_gen, pinned, archived, ingest_cost, domain
                ) VALUES (
                    ?1, ?2, ?3, ?4,
                    ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
                )",
                params![
                    m_id,
                    m_type,
                    m_layer,
                    m_content,
                    m_s50,
                    m_s150,
                    m_s500,
                    m_s2000,
                    m_importance,
                    m_access_count,
                    m_last_access,
                    m_created_at,
                    m_source,
                    m_metadata,
                    m_compressed_from,
                    m_compression_gen,
                    m_pinned,
                    m_archived,
                    m_ingest_cost,
                    m_domain,
                ],
            )
            .map_err(|e| anyhow!("sqlite insert error: {e}"))?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))??;
        debug!(target: "nebula.memory", id = %m.id, layer = %m.layer, "inserted memory");
        Ok(())
    }

    /// Updates an existing record in place. Returns `Err` if the row
    /// does not exist.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn update(&self, m: &Memory) -> Result<()> {
        let conn = self.conn.clone();
        let m_id = m.id.clone();
        let m_type = m.memory_type.as_str().to_string();
        let m_layer = m.layer.as_str().to_string();
        let m_content = m.content.clone();
        let m_s50 = m.summary.s50.clone();
        let m_s150 = m.summary.s150.clone();
        let m_s500 = m.summary.s500.clone();
        let m_s2000 = m.summary.s2000.clone();
        let m_importance = m.importance;
        let m_access_count = m.access_count;
        let m_last_access = m.last_access;
        let m_source = m.source.as_str().to_string();
        let m_metadata = m.metadata.to_string();
        let m_compressed_from = m.compressed_from.clone();
        let m_compression_gen = m.compression_gen;
        let m_pinned = m.pinned as i32;
        let m_ingest_cost = m.ingest_cost;
        let m_domain = m.domain.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let affected = conn
                .execute(
                    "UPDATE memories SET
                    memory_type = ?2,
                    layer = ?3,
                    content = ?4,
                    summary_50 = ?5,
                    summary_150 = ?6,
                    summary_500 = ?7,
                    summary_2000 = ?8,
                    importance = ?9,
                    access_count = ?10,
                    last_access = ?11,
                    source = ?12,
                    metadata = ?13,
                    compressed_from = ?14,
                    compression_gen = ?15,
                    pinned = ?16,
                    ingest_cost = ?17,
                    domain = ?18
                 WHERE id = ?1",
                    params![
                        m_id,
                        m_type,
                        m_layer,
                        m_content,
                        m_s50,
                        m_s150,
                        m_s500,
                        m_s2000,
                        m_importance,
                        m_access_count,
                        m_last_access,
                        m_source,
                        m_metadata,
                        m_compressed_from,
                        m_compression_gen,
                        m_pinned,
                        m_ingest_cost,
                        m_domain,
                    ],
                )
                .map_err(|e| anyhow!("sqlite update error: {e}"))?;
            if affected == 0 {
                return Err(anyhow!("memory not found"));
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))??;
        Ok(())
    }

    /// Fetches a memory by id. Returns `Ok(None)` if absent.
    // v1.1 P1#3: async + spawn_blocking.
    //
    // v1.3 fix (cargo compat): the pthread-style `let conn = self.conn.lock();`
    // pattern holds a `MutexGuard` (`*mut ()`) across `.await`, which is
    // rejected by the post-tokio-1.36 `Send` bound for `JoinHandle`. The
    // guard must only live on the worker thread. We instead clone the
    // `Arc<Mutex<Connection>>` outside the future and re-acquire the lock
    // inside `spawn_blocking`'s closure, so the guard is born-and-destroyed
    // on a single thread.
    pub async fn get(&self, id: &str) -> Result<Option<Memory>> {
        let id_owned = id.to_string();
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let row = conn
                .query_row(
                    sel_mem!("memories WHERE id = ?1"),
                    params![id_owned],
                    row_to_memory,
                )
                .optional()
                .map_err(|e| anyhow!("sqlite get error: {e}"))?;
            Ok(row)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Fetches many memories in a single `WHERE id IN (...)` query.
    ///
    /// v0.1 used a per-hit `get()` round-trip from `memory_search`
    /// which was O(N) and blocked the async runtime; this method is the
    /// fix for both. The order of the returned vector follows the
    /// natural `IN` order (which is implementation-defined for SQLite
    /// but consistent within a version), so callers should not rely on
    /// a specific position.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn get_many(&self, ids: &[String]) -> Result<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.clone();
        let ids_owned = ids.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            // Build "?, ?, ?" placeholders dynamically.
            let placeholders = std::iter::repeat("?")
                .take(ids_owned.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE id IN ({placeholders}) \
                 AND compressed_from IS NULL"
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let params_vec: Vec<&dyn rusqlite::ToSql> = ids_owned
                .iter()
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();
            let rows = stmt
                .query_map(params_vec.as_slice(), row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// M2a #31: 与 [`get_many`](Self::get_many) 相同,但额外按 `domain`
    /// 过滤。供 M2b ACL 的 query-time 过滤使用 —— 旧的 `get_many` 保留
    /// 为跨域查询(向后兼容),新代码应优先使用本方法以避免 post-filter 开销。
    ///
    /// SQL 形如 `WHERE id IN (...) AND compressed_from IS NULL AND domain = ?`,
    /// 占位符顺序为 `[id_1, id_2, ..., id_n, domain]`。
    pub async fn get_many_in_domain(&self, ids: &[String], domain: &str) -> Result<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.clone();
        let ids_owned = ids.to_vec();
        let domain_owned = domain.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let placeholders = std::iter::repeat("?")
                .take(ids_owned.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {MEMORY_COLUMNS} FROM memories WHERE id IN ({placeholders}) \
                 AND compressed_from IS NULL AND domain = ?"
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            // 前 N 个占位符绑定 id,最后一个绑定 domain。
            let mut params_vec: Vec<&dyn rusqlite::ToSql> = ids_owned
                .iter()
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();
            params_vec.push(&domain_owned);
            let rows = stmt
                .query_map(params_vec.as_slice(), row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Marks a memory row as "compressed from this id" by setting
    /// `compressed_from = summary_id`. The original record is *not*
    /// deleted — the v0.1 black-hole contract is "density-preserving
    /// compression". Subsequent `get_many` / `list_recent` /
    /// `list_by_layer` calls will exclude the row.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn update_compressed_from(&self, source_id: &str, summary_id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let src_owned = source_id.to_string();
        let sum_owned = summary_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let n = conn
                .execute(
                    "UPDATE memories SET compressed_from = ?2 WHERE id = ?1",
                    params![src_owned, sum_owned],
                )
                .map_err(|e| anyhow!("sqlite update_compressed_from error: {e}"))?;
            if n == 0 {
                return Err(anyhow!("memory not found for compress"));
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Deletes a memory by id. The black-hole engine never calls this;
    /// it is reserved for explicit user actions.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.clone();
        let id_owned = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let n = conn
                .execute("DELETE FROM memories WHERE id = ?1", params![id_owned])
                .map_err(|e| anyhow!("sqlite delete error: {e}"))?;
            Ok(n > 0)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Lists the most recent memories (newest first), limited to `limit`.
    /// Excludes rows that have been absorbed by the black-hole engine
    /// (`compressed_from IS NOT NULL`).
    // v1.1 P1#3: async + spawn_blocking
    pub async fn list_recent(&self, limit: usize) -> Result<Vec<Memory>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories WHERE compressed_from IS NULL \
                 ORDER BY created_at DESC LIMIT ?1"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![limit as i64], row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// M2a #31: 与 [`list_recent`](Self::list_recent) 相同,但额外按 `domain`
    /// 过滤。供 M2b ACL 的 query-time 过滤使用 —— 旧的 `list_recent` 保留
    /// 为跨域查询(向后兼容),新代码应优先使用本方法以避免 post-filter 开销。
    pub async fn list_recent_in_domain(&self, domain: &str, limit: usize) -> Result<Vec<Memory>> {
        let conn = self.conn.clone();
        let domain_owned = domain.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories WHERE compressed_from IS NULL \
                     AND domain = ?1 \
                     ORDER BY created_at DESC LIMIT ?2"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![domain_owned, limit as i64], row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// T-S1-A-03a: 列出可归档候选记忆的元组（id, layer, importance,
    /// last_access, pinned）。
    ///
    /// 查询条件：
    /// - `archived = 0`（未归档）
    /// - `pinned = 0`（未固定）
    /// - `compressed_from IS NULL`（未被黑洞压缩）
    /// - `importance < threshold`（低于重要性阈值）
    /// - `layer != 'L7'`（奇点层永不归档）
    ///
    /// 返回的 `last_access` 是 Unix 时间戳（秒）。调用方（
    /// `ForgettingEngine::tick()`）在应用层用 TTL 策略进一步过滤。
    pub async fn list_forgettable_candidates(
        &self,
        importance_threshold: f32,
    ) -> Result<Vec<(String, MemoryLayer, f32, i64, bool)>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT id, layer, importance, last_access, pinned \
                     FROM memories \
                     WHERE archived = 0 \
                       AND pinned = 0 \
                       AND compressed_from IS NULL \
                       AND importance < ?1 \
                       AND layer != 'L7'",
                )
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![importance_threshold as f64], |r| {
                    let id: String = r.get(0)?;
                    let layer_s: String = r.get(1)?;
                    let importance: f32 = r.get(2)?;
                    let last_access: i64 = r.get(3)?;
                    let pinned: i32 = r.get(4)?;
                    let layer = MemoryLayer::from_str(&layer_s).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::<dyn std::error::Error + Send + Sync>::from(e),
                        )
                    })?;
                    Ok((id, layer, importance, last_access, pinned != 0))
                })
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// T-S1-A-03a: 批量将记忆标记为 `archived = 1`。
    ///
    /// 返回实际更新的行数。不存在的 id 会被静默忽略（不报错）。
    /// 该方法只做 UPDATE，不删除记忆内容 —— 黑洞引擎（T-S1-A-03b）
    /// 后续会扫描 `archived = 1` 的行进行压缩。
    pub async fn archive_memories(&self, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.clone();
        let ids_owned = ids.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut total = 0usize;
            for id in &ids_owned {
                let n = conn
                    .execute(
                        "UPDATE memories SET archived = 1 WHERE id = ?1 AND archived = 0",
                        params![id],
                    )
                    .map_err(|e| anyhow!("sqlite archive_memories error: {e}"))?;
                total += n;
            }
            Ok(total)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// v0.3: update a memory's `importance` in-place. Returns the
    /// refreshed row. Errors if the id is unknown.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn update_importance(&self, id: &str, importance: f32) -> Result<Memory> {
        let importance = importance.clamp(0.0, 1.0);
        let conn = self.conn.clone();
        let id_owned = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let n = conn
                .execute(
                    "UPDATE memories SET importance = ?2 WHERE id = ?1",
                    params![id_owned, importance],
                )
                .map_err(|e| anyhow!("sqlite update_importance error: {e}"))?;
            if n == 0 {
                return Err(anyhow!("memory not found"));
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))??;

        // Re-fetch the updated row (this is a second async call but importance updates are rare)
        self.get(id)
            .await?
            .ok_or_else(|| anyhow!("memory {id} disappeared after update"))
    }

    /// v0.3: per-layer memory counts. Returns a `MemoryLayer -> count`
    /// map. Rows that have been absorbed by the black-hole engine are
    /// excluded.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn counts_per_layer(&self) -> Result<std::collections::HashMap<MemoryLayer, u64>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT layer, COUNT(*) FROM memories WHERE compressed_from IS NULL GROUP BY layer",
            ).map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map([], |r| {
                    let layer_s: String = r.get(0)?;
                    let n: i64 = r.get(1)?;
                    Ok((layer_s, n as u64))
                })
                .map_err(|e| anyhow!("sqlite query error: {e}"))?;
            let mut out = std::collections::HashMap::new();
            for row in rows {
                let (layer_s, n) = row.map_err(|e| anyhow!("sqlite row error: {e}"))?;
                if let Ok(layer) = MemoryLayer::from_str(&layer_s) {
                    out.insert(layer, n);
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Lists memories within a given layer, newest first. Excludes rows
    /// that have been absorbed by the black-hole engine.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn list_by_layer(&self, layer: MemoryLayer, limit: usize) -> Result<Vec<Memory>> {
        let conn = self.conn.clone();
        let layer_str = layer.as_str().to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories WHERE layer = ?1 \
                 AND compressed_from IS NULL \
                 ORDER BY created_at DESC LIMIT ?2"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![layer_str, limit as i64], row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// M2a #31: 与 [`list_by_layer`](Self::list_by_layer) 相同,但额外按 `domain`
    /// 过滤。供 M2b ACL 的 query-time 过滤使用。
    pub async fn list_by_layer_in_domain(
        &self,
        layer: MemoryLayer,
        domain: &str,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let conn = self.conn.clone();
        let layer_str = layer.as_str().to_string();
        let domain_owned = domain.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories WHERE layer = ?1 \
                     AND compressed_from IS NULL \
                     AND domain = ?2 \
                     ORDER BY created_at DESC LIMIT ?3"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(
                    params![layer_str, domain_owned, limit as i64],
                    row_to_memory,
                )
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Returns memories older than `now - threshold_secs` whose
    /// importance is at or below `importance_ceiling`. The black-hole
    /// engine uses this to find candidates.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn candidates_for_compression(
        &self,
        threshold_secs: i64,
        importance_ceiling: f32,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let now = chrono::Utc::now().timestamp();
        let cutoff = now - threshold_secs;
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories
                 WHERE pinned = 0
                   AND compressed_from IS NULL
                   AND importance <= ?1
                   AND last_access <= ?2
                 ORDER BY last_access ASC
                 LIMIT ?3"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(
                    params![importance_ceiling, cutoff, limit as i64],
                    row_to_memory,
                )
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// M2a #31: 与 [`candidates_for_compression`](Self::candidates_for_compression)
    /// 相同,但额外按 `domain` 过滤。供 M2b ACL 的 query-time 过滤使用 ——
    /// 黑洞压缩应仅作用于同域记忆,避免跨域压缩破坏隔离边界。
    pub async fn candidates_for_compression_in_domain(
        &self,
        threshold_secs: i64,
        importance_ceiling: f32,
        domain: &str,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let now = chrono::Utc::now().timestamp();
        let cutoff = now - threshold_secs;
        let conn = self.conn.clone();
        let domain_owned = domain.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories
                     WHERE pinned = 0
                       AND compressed_from IS NULL
                       AND importance <= ?1
                       AND last_access <= ?2
                       AND domain = ?3
                     ORDER BY last_access ASC
                     LIMIT ?4"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(
                    params![importance_ceiling, cutoff, domain_owned, limit as i64],
                    row_to_memory,
                )
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// T-S1-A-03b: 查询已归档(`archived = 1`)且尚未被压缩(`compressed_from IS NULL`)
    /// 的记忆,供 `BlackholeEngine::run_pass_archived()` 使用。
    ///
    /// 与 `candidates_for_compression` 的区别:
    /// - 不加 `importance` / `last_access` 过滤(已归档记忆已通过 ForgettingEngine
    ///   的 TTL 策略筛选,无需重复过滤)。
    /// - 只扫 `archived = 1`,与 `run_pass()` 的全扫描(`archived` 未过滤)分工。
    pub async fn list_archived_for_compression(&self, limit: usize) -> Result<Vec<Memory>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(sel_mem!(
                    "memories
                 WHERE archived = 1
                   AND pinned = 0
                   AND compressed_from IS NULL
                 ORDER BY last_access ASC
                 LIMIT ?1"
                ))
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![limit as i64], row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Inserts a graph edge between two memories.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn add_relation(&self, rel: &MemoryRelation) -> Result<()> {
        let conn = self.conn.clone();
        let rel_id = rel.id.clone();
        let rel_src = rel.src_id.clone();
        let rel_dst = rel.dst_id.clone();
        let rel_kind = rel.kind.as_str().to_string();
        let rel_weight = rel.weight;
        let rel_created = rel.created_at;
        let rel_evidence = rel.evidence.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.execute(
                "INSERT OR REPLACE INTO memory_relations
                    (id, src_id, dst_id, relation, weight, created_at, evidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    rel_id,
                    rel_src,
                    rel_dst,
                    rel_kind,
                    rel_weight,
                    rel_created,
                    rel_evidence,
                ],
            )
            .map_err(|e| anyhow!("sqlite add_relation error: {e}"))?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    pub fn insert_relation(&self, rel: &MemoryRelation) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO memory_relations
                (id, src_id, dst_id, relation, weight, created_at, evidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rel.id,
                rel.src_id,
                rel.dst_id,
                rel.kind.as_str(),
                rel.weight,
                rel.created_at,
                rel.evidence,
            ],
        )
        .map_err(|e| anyhow!("sqlite insert_relation error: {e}"))?;
        Ok(())
    }

    pub async fn insert_relation_spawn(&self, rel: &MemoryRelation) -> Result<()> {
        let store = self.clone();
        let rel = rel.clone();
        tokio::task::spawn_blocking(move || store.insert_relation(&rel))
            .await
            .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    pub fn get_relations(&self, memory_id: &str) -> Result<Vec<MemoryRelation>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, src_id, dst_id, relation, weight, created_at, evidence
             FROM memory_relations WHERE src_id = ?1 OR dst_id = ?1",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let id_owned = memory_id.to_string();
        let rows = stmt
            .query_map(params![id_owned], |row| {
                Ok(MemoryRelation {
                    id: row.get(0)?,
                    src_id: row.get(1)?,
                    dst_id: row.get(2)?,
                    kind: RelationKind::from_str(row.get::<_, String>(3)?.as_str())
                        .unwrap_or(RelationKind::References),
                    weight: row.get(4)?,
                    created_at: row.get(5)?,
                    evidence: row.get(6)?,
                })
            })
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    pub fn list_all_relations(&self) -> Result<Vec<MemoryRelation>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, src_id, dst_id, relation, weight, created_at, evidence
             FROM memory_relations",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(MemoryRelation {
                    id: row.get(0)?,
                    src_id: row.get(1)?,
                    dst_id: row.get(2)?,
                    kind: RelationKind::from_str(row.get::<_, String>(3)?.as_str())
                        .unwrap_or(RelationKind::References),
                    weight: row.get(4)?,
                    created_at: row.get(5)?,
                    evidence: row.get(6)?,
                })
            })
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// Lists outgoing relations for a given memory.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn relations_from(&self, src_id: &str) -> Result<Vec<MemoryRelation>> {
        let conn = self.conn.clone();
        let src_owned = src_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT id, src_id, dst_id, relation, weight, created_at, evidence
                 FROM memory_relations WHERE src_id = ?1",
                )
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![src_owned], |row| {
                    Ok(MemoryRelation {
                        id: row.get(0)?,
                        src_id: row.get(1)?,
                        dst_id: row.get(2)?,
                        kind: RelationKind::from_str(row.get::<_, String>(3)?.as_str())
                            .unwrap_or(RelationKind::References),
                        weight: row.get(4)?,
                        created_at: row.get(5)?,
                        evidence: row.get(6)?,
                    })
                })
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Records a `memory_commits` row. Used as an append-only audit log.
    // TODO(v0.5, 见 ROADMAP): add automatic commit creation on every insert / update
    // and a batch reconciliation worker that replays the log to rebuild
    // derived state (e.g. importance aggregates, layer counters).
    // v1.1 P1#3: async + spawn_blocking
    pub async fn log_commit(
        &self,
        commit_id: &str,
        parent_id: Option<&str>,
        action: &str,
        target_id: &str,
        payload: &serde_json::Value,
        author: &str,
        message: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let cid = commit_id.to_string();
        let pid = parent_id.map(String::from);
        let act = action.to_string();
        let tid = target_id.to_string();
        let pay = payload.to_string();
        let auth = author.to_string();
        let msg = message.to_string();
        let now = chrono::Utc::now().timestamp();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.execute(
                "INSERT INTO memory_commits
                    (id, parent_id, action, target_id, payload, author, message, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![cid, pid, act, tid, pay, auth, msg, now],
            )
            .map_err(|e| anyhow!("sqlite log_commit error: {e}"))?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Returns the total number of stored memories.
    // v1.1 P1#3: async + spawn_blocking
    pub async fn count(&self) -> Result<i64> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                .map_err(|e| anyhow!("sqlite count error: {e}"))?;
            Ok(n)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// Returns a clone of the inner `Arc<Mutex<Connection>>`. Useful
    /// for callers (e.g. the reflection engine, the migration runner)
    /// that need to issue queries outside the public API surface.
    pub fn raw_connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    /// T-E-B-14: Execute a Dataview-style DSL query against the
    /// `memories` table.
    ///
    /// The [`QueryAst`] is translated by [`super::query_dsl::translate`]
    /// into a parameterised SQL `SELECT` (with `compressed_from IS NULL`
    /// force-injected). All values are bound as positional `?`
    /// parameters — no user-supplied text is ever inlined into the SQL
    /// string, which is the primary SQL-injection guard.
    ///
    /// Blocking SQLite I/O is wrapped in `spawn_blocking` so the tokio
    /// runtime is never starved.
    pub async fn query_dsl(&self, ast: &super::query_dsl::QueryAst) -> Result<Vec<Memory>> {
        let (sql, params) = super::query_dsl::translate(ast);
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let param_refs: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), row_to_memory)
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    pub fn insert_acl(
        &self,
        id: &str,
        principal: &str,
        resource: &str,
        permission: &str,
        effect: &str,
    ) -> Result<()> {
        let conn = self.conn.lock();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO memory_acl (id, principal, resource, permission, effect, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, principal, resource, permission, effect, now],
        ).map_err(|e| anyhow!("sqlite insert_acl error: {e}"))?;
        Ok(())
    }

    pub fn list_acl(&self) -> Result<Vec<(String, String, String, String, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT id, principal, resource, permission, effect FROM memory_acl")
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    pub fn remove_acl(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM memory_acl WHERE id = ?1", params![id])
            .map_err(|e| anyhow!("sqlite remove_acl error: {e}"))?;
        Ok(())
    }

    /// T-E-D-01: 查询单条 SemanticCache 响应正文。
    ///
    /// 在 `SemanticCache::check()` 中,当 LanceDB 命中但进程内 entries map
    /// 缺失(进程重启后)时,调用本方法回退查 SQLite。命中则把响应回填到
    /// entries map,后续命中走内存路径(无需再查 SQLite)。
    ///
    /// 与 [`insert_semantic_cache_entry`](Self::insert_semantic_cache_entry)
    /// 配套使用。query_hash 形如 `sem:0123abcd...`(`stable_id` 输出)。
    pub async fn query_semantic_cache_entry(&self, query_hash: &str) -> Result<Option<String>> {
        let conn = self.conn.clone();
        let key = query_hash.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let row = conn
                .query_row(
                    "SELECT response FROM semantic_cache_entries WHERE query_hash = ?1",
                    params![key],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| anyhow!("sqlite query_semantic_cache_entry error: {e}"))?;
            Ok(row)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// T-E-D-01: 写入/更新单条 SemanticCache 响应正文。
    ///
    /// 在 `SemanticCache::store()` 中,与 LanceDB upsert + entries map insert
    /// 同步写入。`INSERT OR REPLACE` 保证幂等:同 query_hash 重复写入只更新
    /// `inserted_at` 与 `response`,不报错。
    ///
    /// 失败不阻塞调用方:`SemanticCache::store()` 内部 `debug!` 后吞掉错误
    /// (缓存写入失败不影响主调用链)。
    pub async fn insert_semantic_cache_entry(
        &self,
        query_hash: &str,
        response: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let key = query_hash.to_string();
        let resp = response.to_string();
        let now = chrono::Utc::now().timestamp();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            conn.execute(
                "INSERT OR REPLACE INTO semantic_cache_entries \
                 (query_hash, response, inserted_at) VALUES (?1, ?2, ?3)",
                params![key, resp, now],
            )
            .map_err(|e| anyhow!("sqlite insert_semantic_cache_entry error: {e}"))?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }

    /// T-E-D-01: 读取最近 `limit` 条 SemanticCache 响应正文,用于重启后预热。
    ///
    /// 按 `inserted_at DESC` 排序(最新优先),返回 `(query_hash, response)`
    /// 元组列表。调用方 `SemanticCache::prewarm_from_store` 把结果回填到
    /// 进程内 entries map。
    ///
    /// 注意:`limit` 一般为 256(LRU 容量),避免一次性拉全表。
    pub async fn list_recent_semantic_cache_entries(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT query_hash, response FROM semantic_cache_entries \
                     ORDER BY inserted_at DESC LIMIT ?1",
                )
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![limit as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }
}

/// Row-to-memory conversion shared by all `SELECT` paths.
fn row_to_memory(row: &Row<'_>) -> rusqlite::Result<Memory> {
    let memory_type_s: String = row.get("memory_type")?;
    let layer_s: String = row.get("layer")?;
    let source_s: String = row.get("source")?;
    let metadata_s: String = row.get("metadata")?;
    let pinned: i32 = row.get("pinned")?;
    let archived: i32 = row.get("archived")?;

    let memory_type = MemoryType::from_str(&memory_type_s).map_err(|e| {
        rusqlite::Error::InvalidColumnType(1, e.to_string(), rusqlite::types::Type::Text)
    })?;
    let layer = MemoryLayer::from_str(&layer_s).map_err(|e| {
        rusqlite::Error::InvalidColumnType(2, e.to_string(), rusqlite::types::Type::Text)
    })?;
    let source = SourceKind::from_str(&source_s).map_err(|e| {
        rusqlite::Error::InvalidColumnType(12, e.to_string(), rusqlite::types::Type::Text)
    })?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata_s).map_err(|e| {
        rusqlite::Error::InvalidColumnType(13, e.to_string(), rusqlite::types::Type::Text)
    })?;

    Ok(Memory {
        id: row.get("id")?,
        memory_type,
        layer,
        content: row.get("content")?,
        summary: MultiGranularity {
            s50: row.get("summary_50")?,
            s150: row.get("summary_150")?,
            s500: row.get("summary_500")?,
            s2000: row.get("summary_2000")?,
        },
        embedding: Vec::new(), // embeddings live in LanceDB; SQLite is metadata only.
        importance: row.get("importance")?,
        access_count: row.get("access_count")?,
        last_access: row.get("last_access")?,
        created_at: row.get("created_at")?,
        source,
        metadata,
        compressed_from: row.get("compressed_from")?,
        compression_gen: row.get("compression_gen")?,
        pinned: pinned != 0,
        archived: archived != 0,
        // T-E-A-09: 旧记忆列可能不存在(migration 未应用)或为 NULL。
        // `.ok().flatten()` 容错:`row.get` 失败时返回 Err,flatten 后变 None。
        ingest_cost: row.get::<_, Option<f64>>("ingest_cost").ok().flatten(),
        // M2a #28: domain 列向后兼容。migration 035 应用前该列不存在,
        // migration 应用后旧记忆为 NULL。两者均回退到默认 "shared"。
        domain: row
            .get::<_, Option<String>>("domain")
            .ok()
            .flatten()
            .unwrap_or_else(|| "shared".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_db_path() -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("nebula_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    fn sample() -> Memory {
        let mut m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "the quick brown fox",
            SourceKind::UserInput,
        );
        m.summary = MultiGranularity::new(
            "fox",
            "the quick brown fox",
            "the quick brown fox jumps over",
            "the quick brown fox jumps over the lazy dog",
        );
        m.embedding = vec![0.0; 4];
        m.importance = 0.42;
        m
    }

    #[tokio::test]
    async fn insert_and_get_round_trip() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let m = sample();
        store.insert(&m).await.expect("insert should succeed");
        let got = store
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got.id, m.id);
        assert_eq!(got.layer, MemoryLayer::L3);
        assert!((got.importance - 0.42).abs() < 1e-6);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn candidates_for_compression_skips_pinned() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut m = sample();
        m.pinned = true;
        store.insert(&m).await.expect("insert should succeed");
        let cands = store
            .candidates_for_compression(0, 1.0, 100)
            .await
            .expect("update should succeed");
        assert!(
            cands.is_empty(),
            "pinned memories must never be compression candidates"
        );
        let _ = std::fs::remove_file(path);
    }

    /// M2a #31: 验证 `_in_domain` 变体仅返回同域记忆,且旧 `Memory::new()`
    /// 默认 domain = "shared"。这是 domain 隔离的端到端测试。
    #[tokio::test]
    async fn list_recent_in_domain_filters_correctly() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");

        // shared 域(默认)
        let mut a = sample();
        a.id = "shared-1".to_string();
        a.domain = "shared".to_string();
        // agent_a 域
        let mut b = sample();
        b.id = "agent_a-1".to_string();
        b.domain = "agent_a".to_string();
        // agent_b 域
        let mut c = sample();
        c.id = "agent_b-1".to_string();
        c.domain = "agent_b".to_string();

        store.insert(&a).await.expect("insert should succeed");
        store.insert(&b).await.expect("insert should succeed");
        store.insert(&c).await.expect("insert should succeed");

        // 跨域查询(旧方法,向后兼容):返回全部 3 条
        let all = store.list_recent(100).await.expect("update should succeed");
        assert_eq!(
            all.len(),
            3,
            "list_recent (no domain filter) must return all 3"
        );

        // agent_a 域查询:仅返回 1 条
        let agent_a_only = store
            .list_recent_in_domain("agent_a", 100)
            .await
            .expect("update should succeed");
        assert_eq!(agent_a_only.len(), 1);
        assert_eq!(agent_a_only[0].id, "agent_a-1");

        // shared 域查询:仅返回 1 条
        let shared_only = store
            .list_recent_in_domain("shared", 100)
            .await
            .expect("update should succeed");
        assert_eq!(shared_only.len(), 1);
        assert_eq!(shared_only[0].id, "shared-1");

        // 不存在的域:返回 0 条
        let empty = store
            .list_recent_in_domain("nonexistent", 100)
            .await
            .expect("test op should succeed");
        assert!(empty.is_empty());

        let _ = std::fs::remove_file(path);
    }

    /// M2a #31: 验证 `Memory::new()` 默认 domain = "shared",且旧数据库
    /// (domain 列为 NULL)经 row_to_memory 回退后也读取为 "shared"。
    #[tokio::test]
    async fn default_domain_is_shared() {
        let m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "test content",
            SourceKind::UserInput,
        );
        assert_eq!(
            m.domain, "shared",
            "Memory::new() must default domain to 'shared'"
        );
    }

    #[tokio::test]
    async fn candidates_for_compression_skips_already_compressed() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut a = sample();
        a.id = "absorbed".to_string();
        a.importance = 0.1;
        a.last_access = 0;
        let mut b = sample();
        b.id = "fresh".to_string();
        b.importance = 0.1;
        b.last_access = 0;
        store.insert(&a).await.expect("insert should succeed");
        store.insert(&b).await.expect("insert should succeed");
        store
            .update_compressed_from("absorbed", "summary-a")
            .await
            .expect("test op should succeed");
        let cands = store
            .candidates_for_compression(0, 1.0, 100)
            .await
            .expect("update should succeed");
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].id, "fresh");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_by_layer_returns_matching_rows() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut a = sample();
        a.layer = MemoryLayer::L2;
        let mut b = sample();
        b.layer = MemoryLayer::L3;
        store.insert(&a).await.expect("insert should succeed");
        store.insert(&b).await.expect("insert should succeed");
        let l2 = store
            .list_by_layer(MemoryLayer::L2, 10)
            .await
            .expect("update should succeed");
        assert_eq!(l2.len(), 1);
        assert_eq!(l2[0].id, a.id);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn get_many_returns_only_uncompressed() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut a = sample();
        a.id = "id-a".to_string();
        let mut b = sample();
        b.id = "id-b".to_string();
        store.insert(&a).await.expect("insert should succeed");
        store.insert(&b).await.expect("insert should succeed");
        store
            .update_compressed_from("id-a", "summary-x")
            .await
            .expect("test op should succeed");

        let hits = store
            .get_many(&["id-a".to_string(), "id-b".to_string()])
            .await
            .expect("test op should succeed");
        assert_eq!(hits.len(), 1, "compressed source must be excluded");
        assert_eq!(hits[0].id, "id-b");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_recent_excludes_compressed_rows() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut a = sample();
        a.id = "kept".to_string();
        let mut b = sample();
        b.id = "gone".to_string();
        store.insert(&a).await.expect("insert should succeed");
        store.insert(&b).await.expect("insert should succeed");
        store
            .update_compressed_from("gone", "summary-z")
            .await
            .expect("test op should succeed");
        let recent = store.list_recent(10).await.expect("update should succeed");
        assert!(recent.iter().all(|m| m.id != "gone"));
        assert!(recent.iter().any(|m| m.id == "kept"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_by_layer_excludes_compressed_rows() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut a = sample();
        a.id = "alive-l2".to_string();
        a.layer = MemoryLayer::L2;
        let mut b = sample();
        b.id = "dead-l2".to_string();
        b.layer = MemoryLayer::L2;
        store.insert(&a).await.expect("insert should succeed");
        store.insert(&b).await.expect("insert should succeed");
        store
            .update_compressed_from("dead-l2", "summary-l2")
            .await
            .expect("test op should succeed");
        let l2 = store
            .list_by_layer(MemoryLayer::L2, 10)
            .await
            .expect("update should succeed");
        assert_eq!(l2.len(), 1);
        assert_eq!(l2[0].id, "alive-l2");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn update_compressed_from_unknown_errors() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let res = store.update_compressed_from("nope", "sum").await;
        assert!(res.is_err());
        let _ = std::fs::remove_file(path);
    }

    // -----------------------------------------------------------------------
    // T-E-S-43: open_encrypted 测试(sqlcipher feature-gated)。
    // 无 sqlcipher feature 时整个块不参与编译,确保降级路径 cargo check 通过。
    // -----------------------------------------------------------------------
    #[cfg(feature = "sqlcipher")]
    #[tokio::test]
    async fn open_encrypted_creates_db_with_cipher_version() {
        let path = temp_db_path();
        let key = crate::security::keychain::generate_db_encryption_key();
        let store = SqliteStore::open_encrypted(&path, &key).expect("open_encrypted");

        // cipher_version 非空(open_encrypted 内部已验证,此处再读确认)。
        let conn = store.raw_connection();
        let lock = conn.lock();
        let v: String = lock
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .expect("cipher_version");
        assert!(!v.is_empty(), "cipher_version must be non-empty");
        drop(lock);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "sqlcipher")]
    #[tokio::test]
    async fn open_encrypted_rejects_wrong_key() {
        let path = temp_db_path();
        let key = crate::security::keychain::generate_db_encryption_key();
        // 首次创建加密 DB。
        let store = SqliteStore::open_encrypted(&path, &key).expect("open_encrypted first");
        drop(store);

        // 用错误 key 重新打开应失败(key 错 → "file is not a database")。
        let wrong_key = crate::security::keychain::generate_db_encryption_key();
        let result = SqliteStore::open_encrypted(&path, &wrong_key);
        assert!(
            result.is_err(),
            "wrong key must be rejected (file is not a database)"
        );
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "sqlcipher")]
    #[tokio::test]
    async fn open_encrypted_round_trip_data() {
        let path = temp_db_path();
        let key = crate::security::keychain::generate_db_encryption_key();
        let store = SqliteStore::open_encrypted(&path, &key).expect("open_encrypted");

        // 插入 + 读取往返。
        let m = sample();
        store.insert(&m).await.expect("insert should succeed");
        let got = store
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got.id, m.id);
        assert_eq!(got.layer, MemoryLayer::L3);
        assert!((got.importance - 0.42).abs() < 1e-6);
        assert_eq!(got.content, m.content);

        drop(store);
        // 重新打开(用正确 key)验证持久化。
        let store2 = SqliteStore::open_encrypted(&path, &key).expect("reopen encrypted");
        let got2 = store2
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got2.id, m.id, "data must persist across reopens");
        drop(store2);
        let _ = std::fs::remove_file(path);
    }

    // -----------------------------------------------------------------------
    // T-E-D-01: SemanticCache 持久化 CRUD 测试。
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn insert_and_query_semantic_cache_entry_round_trip() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");

        // 初始查询返回 None。
        let got = store
            .query_semantic_cache_entry("sem:abc")
            .await
            .expect("query should succeed");
        assert!(got.is_none(), "expected None for missing entry");

        // 插入后再查,返回 Some(response)。
        store
            .insert_semantic_cache_entry("sem:abc", "hello world")
            .await
            .expect("test op should succeed");
        let got = store
            .query_semantic_cache_entry("sem:abc")
            .await
            .expect("test op should succeed")
            .expect("entry must exist after insert");
        assert_eq!(got, "hello world");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn insert_semantic_cache_entry_is_idempotent() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");

        // 同 query_hash 重复写入,后写覆盖前写(INSERT OR REPLACE)。
        store
            .insert_semantic_cache_entry("sem:dup", "first")
            .await
            .expect("test op should succeed");
        store
            .insert_semantic_cache_entry("sem:dup", "second")
            .await
            .expect("test op should succeed");
        let got = store
            .query_semantic_cache_entry("sem:dup")
            .await
            .expect("test op should succeed")
            .expect("entry must exist");
        assert_eq!(got, "second", "second write must overwrite first");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_recent_semantic_cache_entries_orders_by_inserted_at_desc() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");

        // 插入三条,中间隔 1 秒保证 inserted_at 不同。
        store
            .insert_semantic_cache_entry("sem:old", "old")
            .await
            .expect("test op should succeed");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        store
            .insert_semantic_cache_entry("sem:mid", "mid")
            .await
            .expect("test op should succeed");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        store
            .insert_semantic_cache_entry("sem:new", "new")
            .await
            .expect("test op should succeed");

        let rows = store
            .list_recent_semantic_cache_entries(10)
            .await
            .expect("update should succeed");
        assert_eq!(rows.len(), 3, "should have 3 entries");
        // 最新插入的应排第一。
        assert_eq!(rows[0].0, "sem:new", "newest must come first");
        assert_eq!(rows[1].0, "sem:mid", "middle must come second");
        assert_eq!(rows[2].0, "sem:old", "oldest must come last");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn list_recent_semantic_cache_entries_respects_limit() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");

        for i in 0..5 {
            store
                .insert_semantic_cache_entry(&format!("sem:{i}"), &format!("resp-{i}"))
                .await
                .expect("test op should succeed");
        }
        let rows = store
            .list_recent_semantic_cache_entries(3)
            .await
            .expect("update should succeed");
        assert_eq!(rows.len(), 3, "limit=3 must clamp to 3 rows");
        let _ = std::fs::remove_file(path);
    }

    // ---- T-E-A-09: ingest_cost round-trip 测试 ----

    /// `ingest_cost = Some(v)` 时 insert + get 应保留原值。
    #[tokio::test]
    async fn insert_and_get_preserves_ingest_cost() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut m = sample();
        m.ingest_cost = Some(0.0072);
        store.insert(&m).await.expect("insert should succeed");
        let got = store
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(
            (got.ingest_cost.expect("test op should succeed") - 0.0072).abs() < 1e-9,
            "ingest_cost round-trip failed"
        );
        let _ = std::fs::remove_file(path);
    }

    /// `ingest_cost = None` 时 insert + get 应保持 None(列存 NULL)。
    #[tokio::test]
    async fn insert_and_get_preserves_ingest_cost_none() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let m = sample();
        // sample() 默认 ingest_cost = None
        store.insert(&m).await.expect("insert should succeed");
        let got = store
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(got.ingest_cost.is_none(), "ingest_cost should remain None");
        let _ = std::fs::remove_file(path);
    }

    /// `ingest_cost = Some(0.0)` 表示已追踪但为零(本地 Ollama 场景),
    /// 应与 Some(非零) 走相同的 round-trip 路径。
    #[tokio::test]
    async fn insert_and_get_preserves_ingest_cost_zero() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut m = sample();
        m.ingest_cost = Some(0.0);
        store.insert(&m).await.expect("insert should succeed");
        let got = store
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got.ingest_cost, Some(0.0));
        let _ = std::fs::remove_file(path);
    }

    /// `update_guarded` 应能更新 ingest_cost 字段。
    #[tokio::test]
    async fn update_guarded_updates_ingest_cost() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).expect("create should succeed");
        let mut m = sample();
        m.ingest_cost = Some(0.001);
        store.insert(&m).await.expect("insert should succeed");
        // 更新为新的成本值
        m.ingest_cost = Some(0.005);
        store.update(&m).await.expect("update should succeed");
        let got = store
            .get(&m.id)
            .await
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(
            (got.ingest_cost.expect("test op should succeed") - 0.005).abs() < 1e-9,
            "update_guarded should update ingest_cost"
        );
        let _ = std::fs::remove_file(path);
    }
}
