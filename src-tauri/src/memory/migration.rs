//! Schema migration system for the nebula memory store.
//!
//! Migrations are stored as plain `.sql` files in the
//! `src-tauri/migrations/` directory. The file name MUST follow the
//! pattern `NNN_*.sql` where `NNN` is a monotonically increasing
//! version number. Files are applied in ascending order; the system
//! records the highest applied version in `PRAGMA user_version` so
//! re-runs are idempotent.
//!
//! ## Lifecycle
//!
//! 1. `current_version(&conn)` reads `PRAGMA user_version` (0 if unset).
//! 2. `run_migrations(&conn, dir)` discovers every `NNN_*.sql` file,
//!    parses the leading version number, and applies anything strictly
//!    greater than the current version.
//! 3. Each applied file is wrapped in a `BEGIN ... COMMIT` transaction
//!    so a partial failure leaves the database in its previous state.
//!
//! ## v0.1 → v0.2 transition
//!
//! The v0.1 `001_initial.sql` was applied via raw `execute_batch` in
//! `SqliteStore::open`. To stay backward-compatible, the v0.2 boot
//! sequence calls `bootstrap_v0_1_baseline` *before* `run_migrations`
//! — this stamps `PRAGMA user_version = 1` so 002+ are not skipped on
//! databases that pre-date this module.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// A single migration descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    /// Monotonically increasing version number.
    pub version: u32,
    /// Human-readable name (file name without the leading version
    /// prefix and without the extension).
    pub name: String,
    /// Raw SQL body.
    pub sql: String,
}

/// Snapshot of the migration state of a database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStatus {
    /// Highest applied version (0 if no migrations have run).
    pub current_version: u32,
    /// All migration files known to the migrator, with their applied
    /// state.
    pub applied: Vec<MigrationState>,
}

/// One entry of [`MigrationStatus::applied`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationState {
    pub version: u32,
    pub name: String,
    pub applied: bool,
}

/// Returns the highest migration version previously applied to the
/// database (`PRAGMA user_version`).
pub fn current_version(conn: &Connection) -> Result<u32> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v as u32)
}

/// Stamps `PRAGMA user_version = 1` when the v0.1 initial schema was
/// already applied by the legacy `SqliteStore::open` path. Idempotent.
pub fn bootstrap_v0_1_baseline(conn: &Connection) -> Result<()> {
    if current_version(conn)? == 0 {
        // The v0.1 schema includes its own `schema_version` row at
        // version 1, so we can safely assume 001 has run.
        let has: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_version WHERE version = 1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if has {
            conn.pragma_update(None, "user_version", 1i64)?;
            info!(target: "nebula.migration", "v0.1 baseline detected; user_version set to 1");
        }
    }
    Ok(())
}

/// Runs every migration in `migrations_dir` whose version is strictly
/// greater than the current `PRAGMA user_version`.
///
/// Returns the list of migrations that were applied during this call.
pub fn run_migrations(conn: &Connection, migrations_dir: &Path) -> Result<Vec<Migration>> {
    bootstrap_v0_1_baseline(conn)?;

    let all = discover_migrations(migrations_dir)?;
    if all.is_empty() {
        debug!(target: "nebula.migration", "no migration files found");
        return Ok(Vec::new());
    }

    let current = current_version(conn)?;
    let pending: Vec<Migration> = all.into_iter().filter(|m| m.version > current).collect();

    if pending.is_empty() {
        debug!(target: "nebula.migration", current, "no pending migrations");
        return Ok(Vec::new());
    }

    info!(
        target: "nebula.migration",
        from = current,
        to = pending.last().map(|m| m.version).unwrap_or(current),
        count = pending.len(),
        "applying migrations"
    );

    let mut applied: Vec<Migration> = Vec::new();
    for m in pending {
        apply_one(conn, &m).with_context(|| format!("applying migration {}", m.name))?;
        applied.push(m);
    }
    Ok(applied)
}

/// Builds a [`MigrationStatus`] without mutating the database.
pub fn migration_status(conn: &Connection, migrations_dir: &Path) -> Result<MigrationStatus> {
    let current = current_version(conn)?;
    let all = discover_migrations(migrations_dir)?;
    let applied = all
        .into_iter()
        .map(|m| MigrationState {
            version: m.version,
            name: m.name,
            applied: m.version <= current,
        })
        .collect();
    Ok(MigrationStatus {
        current_version: current,
        applied,
    })
}

/// 从内嵌的 migration 数据获取 migration 状态,不依赖文件系统路径。
pub fn bundled_migration_status(conn: &Connection) -> Result<MigrationStatus> {
    let current = current_version(conn)?;
    let applied = bundled_migrations()
        .iter()
        .map(|(v, name, _)| MigrationState {
            version: *v,
            name: (*name).to_string(),
            applied: *v <= current,
        })
        .collect();
    Ok(MigrationStatus {
        current_version: current,
        applied,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn apply_one(conn: &Connection, m: &Migration) -> Result<()> {
    debug!(target: "nebula.migration", version = m.version, name = %m.name, "applying");
    let stmts = split_sql(&m.sql);
    // PRAGMA statements (e.g. `PRAGMA journal_mode = WAL`) cannot
    // run inside a transaction on SQLite.  We execute them *before*
    // opening the transaction so the rest of the migration can run
    // atomically.
    let (pragmas, rest): (Vec<String>, Vec<String>) =
        stmts.into_iter().partition(|s| statement_is_pragma(s));
    for stmt in pragmas {
        if stmt.trim().is_empty() {
            continue;
        }
        if let Err(e) = conn.execute_batch(&stmt) {
            let msg = format!("{e}");
            if !is_idempotent_error(&msg) {
                return Err(e).with_context(|| format!("pragmas statement: {stmt}"));
            }
        }
    }
    let tx = conn.unchecked_transaction()?;
    // The migrator splits on `;` for granular error reporting so
    // that idempotent statements (e.g. `ALTER TABLE ... ADD COLUMN`
    // that fails with "duplicate column name") can be ignored.
    apply_statements_from_vec(&tx, &rest)
        .with_context(|| format!("executing migration body for version {}", m.version))?;
    // Bump user_version last so a failed migration leaves the previous
    // version intact.
    tx.pragma_update(None, "user_version", m.version as i64)?;
    tx.commit()?;
    info!(target: "nebula.migration", version = m.version, "applied");
    Ok(())
}

/// Returns true if the first non-comment SQL keyword in `stmt` is
/// `PRAGMA`.  Uses the same scanner as `split_sql` so line
/// comments (`--`) and block comments (`/* ... */`) are skipped.
fn statement_is_pragma(stmt: &str) -> bool {
    let mut chars = stmt.chars().peekable();
    let mut word_buf = String::new();
    loop {
        match chars.next() {
            None => return false,
            Some('-') if chars.peek() == Some(&'-') => {
                // Line comment — skip to end of line.
                for nc in chars.by_ref() {
                    if nc == '\n' {
                        break;
                    }
                }
                word_buf.clear();
            }
            Some('/') if chars.peek() == Some(&'*') => {
                // Block comment — skip until closing marker.
                chars.next(); // consume '*'
                while let Some(nc) = chars.next() {
                    if nc == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
                word_buf.clear();
            }
            Some(c) if c.is_alphabetic() => {
                word_buf.push(c.to_ascii_uppercase());
            }
            Some(c) if c.is_whitespace() => {
                if !word_buf.is_empty() {
                    return word_buf == "PRAGMA";
                }
            }
            Some(_) => {
                if !word_buf.is_empty() {
                    return word_buf == "PRAGMA";
                }
                return false;
            }
        }
    }
}

/// Splits a multi-statement SQL script and applies each statement
/// individually. Statements that fail with "duplicate column" or
/// "already exists" are silently ignored (idempotent re-runs).
fn apply_statements_from_vec(conn: &Connection, stmts: &[String]) -> Result<()> {
    for stmt in stmts {
        if stmt.trim().is_empty() {
            continue;
        }
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = format!("{e}");
            if is_idempotent_error(&msg) {
                debug!(target: "nebula.migration", error = %msg, "ignoring idempotent error");
            } else {
                return Err(e).with_context(|| format!("statement: {stmt}"));
            }
        }
    }
    Ok(())
}

/// SQL splitter that respects string literals, single-line `--` comments,
/// `/* ... */` block comments, and trigger boundaries.
///
/// v0.2 had a naive `sql.split(';')` that broke on:
///   * semicolons inside `'string'` or `"identifier"` literals,
///   * `BEGIN ... END;` blocks of triggers (we have none yet, but
///     future migrations might).
///
/// v0.3 implements a one-pass char-by-char scanner that:
///   * tracks `inside_string` (toggle on unescaped `'` / `"`),
///   * treats `--...` line comments and `/* ... */` block comments as
///     transparent,
///   * only emits a split when the semicolon sits at top-level.
fn split_sql(sql: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut chars = sql.chars().peekable();
    let mut inside_string: Option<char> = None;
    // Track BEGIN/END block depth so semicolons inside trigger
    // bodies do not cause splits. SQLite treats CREATE TRIGGER
    // ... BEGIN ... END; as a single compound statement.
    let mut begin_depth: u32 = 0;
    let mut word_buf: String = String::new();

    fn flush_word(word: &mut String, depth: &mut u32) {
        match word.as_str() {
            "BEGIN" => *depth += 1,
            "END" => *depth = depth.saturating_sub(1),
            _ => {}
        }
        word.clear();
    }

    while let Some(c) = chars.next() {
        match (c, inside_string) {
            ('\'', None) | ('"', None) => {
                flush_word(&mut word_buf, &mut begin_depth);
                buf.push(c);
                inside_string = Some(c);
            }
            (c2, Some(q)) if c2 == q => {
                buf.push(c2);
                inside_string = None;
            }
            ('-', None) if chars.peek() == Some(&'-') => {
                flush_word(&mut word_buf, &mut begin_depth);
                buf.push('-');
                buf.push(chars.next().unwrap()); // push second '-'
                for nc in chars.by_ref() {
                    buf.push(nc);
                    if nc == '\n' {
                        break;
                    }
                }
            }
            ('/', None) if chars.peek() == Some(&'*') => {
                flush_word(&mut word_buf, &mut begin_depth);
                buf.push('/');
                chars.next();
                buf.push('*');
                while let Some(nc) = chars.next() {
                    buf.push(nc);
                    if nc == '*' && chars.peek() == Some(&'/') {
                        buf.push(chars.next().unwrap());
                        break;
                    }
                }
            }
            (';', None) => {
                flush_word(&mut word_buf, &mut begin_depth);
                if begin_depth == 0 {
                    out.push(std::mem::take(&mut buf));
                } else {
                    buf.push(c);
                }
            }
            (other, None) => {
                buf.push(other);
                if other.is_alphabetic() {
                    word_buf.push(other.to_ascii_uppercase());
                } else {
                    flush_word(&mut word_buf, &mut begin_depth);
                }
            }
            (other, _) => buf.push(other),
        }
    }
    flush_word(&mut word_buf, &mut begin_depth);
    if !buf.trim().is_empty() {
        out.push(buf);
    }
    out
}
fn is_idempotent_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("duplicate column name") || m.contains("already exists")
}

fn discover_migrations(dir: &Path) -> Result<Vec<Migration>> {
    if !dir.exists() {
        warn!(target: "nebula.migration", path = %dir.display(), "migrations dir does not exist");
        return Ok(Vec::new());
    }
    let mut out: Vec<Migration> = Vec::new();
    for entry in
        fs::read_dir(dir).with_context(|| format!("reading migrations dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("sql") {
            continue;
        }
        let fname = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        let (version, name) = match parse_filename(fname) {
            Some(parts) => parts,
            None => {
                warn!(target: "nebula.migration", file = %fname, "skipping file without NNN_ prefix");
                continue;
            }
        };
        let sql = fs::read_to_string(&path)
            .with_context(|| format!("reading migration file {}", path.display()))?;
        out.push(Migration { version, name, sql });
    }
    out.sort_by_key(|m| m.version);
    Ok(out)
}

fn parse_filename(fname: &str) -> Option<(u32, String)> {
    let idx = fname.find('_')?;
    let n_str = &fname[..idx];
    let n: u32 = n_str.parse().ok()?;
    let rest = &fname[idx + 1..];
    let rest = rest.strip_suffix(".sql").unwrap_or(rest);
    Some((n, rest.to_string()))
}

/// Helper that returns the directory containing the bundled migration
/// files. Useful in tests that don't have a workspace root.
pub fn bundled_migrations_dir() -> &'static Path {
    // The path is relative to CARGO_MANIFEST_DIR at compile time.
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
}

/// 内嵌的 migration 列表 (version, name, sql)。
///
/// 用 `include_str!` 在编译时将所有 `.sql` 文件内嵌到二进制中,
/// 这样打包发布后不依赖 `CARGO_MANIFEST_DIR` 路径(该路径在用户
/// 机器上不存在)。新增 migration 时只需在此列表追加一行。
pub fn bundled_migrations() -> &'static [(u32, &'static str, &'static str)] {
    &[
        (
            1,
            "initial",
            include_str!("../../migrations/001_initial.sql"),
        ),
        (
            2,
            "reflections",
            include_str!("../../migrations/002_reflections.sql"),
        ),
        (3, "skills", include_str!("../../migrations/003_skills.sql")),
        (4, "v05", include_str!("../../migrations/004_v05.sql")),
        (5, "v10", include_str!("../../migrations/005_v10.sql")),
        (
            6,
            "documents_fk",
            include_str!("../../migrations/006_documents_fk.sql"),
        ),
        (
            7,
            "sensitive_mask",
            include_str!("../../migrations/007_sensitive_mask.sql"),
        ),
        (
            8,
            "evolution",
            include_str!("../../migrations/008_evolution.sql"),
        ),
        (
            9,
            "skill_archive",
            include_str!("../../migrations/009_skill_archive.sql"),
        ),
        (10, "fts5", include_str!("../../migrations/010_fts5.sql")),
        (
            11,
            "relation_evidence",
            include_str!("../../migrations/011_relation_evidence.sql"),
        ),
        (
            12,
            "skill_audit_log",
            include_str!("../../migrations/012_skill_audit_log.sql"),
        ),
        (
            13,
            "memory_acl",
            include_str!("../../migrations/013_memory_acl.sql"),
        ),
        (
            14,
            "memories_archived",
            include_str!("../../migrations/014_memories_archived.sql"),
        ),
        (
            15,
            "skill_meta_extensions",
            include_str!("../../migrations/015_skill_meta_extensions.sql"),
        ),
        (
            16,
            "memory_versions",
            include_str!("../../migrations/016_memory_versions.sql"),
        ),
        (
            17,
            "document_exports",
            include_str!("../../migrations/017_document_exports.sql"),
        ),
        (
            18,
            "fix_skill_ratings_pk",
            include_str!("../../migrations/018_fix_skill_ratings_pk.sql"),
        ),
        // v1.6: Git 风格记忆版本控制 — 分支管理。
        (
            19,
            "memory_branches",
            include_str!("../../migrations/019_memory_branches.sql"),
        ),
        // T-S1-A-06: self_reflections 表 — L5 真反思持久化。
        (
            20,
            "self_reflections",
            include_str!("../../migrations/020_self_reflections.sql"),
        ),
        // T-S3-A-01: agentskills.io SkillMeta 补全 — trust_level/permissions/capabilities
        (
            21,
            "skill_trust_meta",
            include_str!("../../migrations/021_skill_trust_meta.sql"),
        ),
        // T-S6-B-03: CRDT op 日志表 — 跨设备 CRDT op 传播落盘。
        (
            22,
            "crdt_op_log",
            include_str!("../../migrations/022_crdt_op_log.sql"),
        ),
        // T-E-S-28: 对话消息标注(good/bad)+ Dify 风格数据集导出。
        // UNIQUE(turn_id) 保证 upsert 幂等,created_at DESC 索引支撑 stats 聚合。
        (
            24,
            "chat_annotations",
            include_str!("../../migrations/024_chat_annotations.sql"),
        ),
        // T-E-S-54: 事件触发器 — 文件/消息/Webhook 三种触发器持久化。
        // triggers + trigger_fire_log 两张表,idx_triggers_kind/enabled 索引。
        (
            25,
            "triggers",
            include_str!("../../migrations/025_triggers.sql"),
        ),
        // T-E-S-55: 条件监控 Watch — watch_state 表(Web hash / System 值 / Calendar UID)。
        (
            26,
            "watch",
            include_str!("../../migrations/026_watch.sql"),
        ),
        // T-E-A-12: Automation Credits — cost_records 表新增 source / trigger_id 列。
        // source 默认 'chat'(CostSource::Chat),trigger_id 可空。前向兼容预留
        // 持久化层(T-E-A-13),当前 CostTracker 仍为内存态。
        (
            27,
            "cost_source",
            include_str!("../../migrations/027_cost_source.sql"),
        ),
        // T-E-C-17: IM 扫码绑定 — im_bindings 表(Feishu/WeCom/DingTalk webhook)。
        // Phase 1 Webhook 优先,Phase 2 OAuthUser 复用本表(由 kind 字段区分)。
        (
            28,
            "im_bindings",
            include_str!("../../migrations/028_im_bindings.sql"),
        ),
        // T-E-B-01: LLM Wiki 编译引擎 — wiki_notes 表 + FTS5 全文索引。
        // 每次对话后 AI "编译" 结构化 Markdown 笔记,UNIQUE(turn_id) 幂等,
        // FTS5 external content mode + 触发器自动同步。
        (
            29,
            "wiki_notes",
            include_str!("../../migrations/029_wiki_notes.sql"),
        ),
        // T-E-A-09: 记忆吸收成本(USD)— memories 表新增 ingest_cost REAL 列。
        // 默认 NULL(Option<f64>::None),Some(0.0) 表示已追踪但为零。
        // 幂等:重复应用报 "duplicate column name" 由 is_idempotent_error 忽略。
        (
            30,
            "ingest_cost",
            include_str!("../../migrations/030_ingest_cost.sql"),
        ),
        // T-E-D-01: 冷启动 < 3s 工程 — SemanticCache 响应正文持久化。
        // 重启后 prewarm_from_store 读取最近 256 条重建 entries map;
        // check() 在 entries map miss 时回退查本表,避免冷启动漏命中。
        (
            31,
            "semantic_cache_entries",
            include_str!("../../migrations/031_semantic_cache_entries.sql"),
        ),
        // T-E-B-06: Wiki index.md + log.md 自动维护 — wiki_notes 表新增 importance 列。
        // 默认 0.5(对齐 memories.importance);regenerate_index 按其降序稳定排序。
        // 幂等:重复应用报 "duplicate column name" 由 is_idempotent_error 忽略。
        (
            32,
            "wiki_notes_importance",
            include_str!("../../migrations/032_wiki_notes_importance.sql"),
        ),
        // T-E-B-05: 双向链接 [[]] 语法 — wiki_note_links 关联表(source_id / target_id)。
        // ON DELETE CASCADE 确保删除笔记时自动清除关联行;
        // idx_wiki_note_links_target 加速 get_backlinks(target_id) 查询。
        (
            33,
            "wiki_note_links",
            include_str!("../../migrations/033_wiki_note_links.sql"),
        ),
        // T-E-A-14: Arena A/B 测试 — arena_matches + model_elo_scores 表。
        // arena_matches 存单场对战记录(prompt/model_a/model_b/winner/auto_score),
        // model_elo_scores 累积 ELO(K=32, 初始 1200)。持久化模式参考 027_cost_source。
        (
            34,
            "arena",
            include_str!("../../migrations/034_arena.sql"),
        ),
        // M2a 任务 #29: Memory.domain 字段（P0-9 修复）。
        // 为 memories 表添加 domain 列，用于按"域"隔离记忆。
        // 默认 'shared'（向后兼容旧记忆）；idx_memories_domain 加速 WHERE domain = ? 查询。
        // 幂等模式参考 030_ingest_cost.sql。
        (
            35,
            "domain_column",
            include_str!("../../migrations/035_domain_column.sql"),
        ),
        // M5 #72 / M7b #96: CostTracker 按 WorkType 分域统计 — cost_records 表新增 work_type 列。
        // work_type 取值:chat / swarm_worker / swarm_synthesize / master_task /
        // evolution / soul_compile / classifier（见 WorkType::as_str()）。
        // 旧记录（无此列）SELECT 时通过 COALESCE 兜底为 NULL，反序列化时 CostRecord.work_type = None。
        // 幂等性: ALTER TABLE 重复应用报 "duplicate column name"，is_idempotent_error 静默忽略。
        (
            36,
            "cost_work_type",
            include_str!("../../migrations/036_cost_work_type.sql"),
        ),
    ]
}

/// 从内嵌的 migration 数据运行迁移,不依赖文件系统路径。
///
/// 打包发布后 `bundled_migrations_dir()` 返回的路径在用户机器上
/// 不存在,用此函数替代 `run_migrations(conn, bundled_migrations_dir())`。
///
/// M7b #96: 在应用 pending migrations 之前,若检测到数据库有文件路径
/// (非 `:memory:`),会先用 `VACUUM INTO` 创建一致性快照备份。
/// 备份失败仅记日志,不阻塞迁移流程。
pub fn run_bundled_migrations(conn: &Connection) -> Result<Vec<Migration>> {
    bootstrap_v0_1_baseline(conn)?;

    let bundled = bundled_migrations();
    if bundled.is_empty() {
        debug!(target: "nebula.migration", "no bundled migrations");
        return Ok(Vec::new());
    }

    let current = current_version(conn)?;
    let mut pending: Vec<Migration> = bundled
        .iter()
        .filter(|(v, _, _)| *v > current)
        .map(|(v, name, sql)| Migration {
            version: *v,
            name: (*name).to_string(),
            sql: (*sql).to_string(),
        })
        .collect();

    if pending.is_empty() {
        debug!(target: "nebula.migration", current, "no pending migrations");
        return Ok(Vec::new());
    }

    pending.sort_by_key(|m| m.version);
    let target_version = pending.last().map(|m| m.version).unwrap_or(current);

    // M7b #96: 迁移前备份策略。
    // 仅在有 pending migrations 且数据库有文件路径时备份。
    // :memory: 数据库跳过备份。备份失败仅 warn,不阻塞迁移。
    backup_before_migrate(conn, current, target_version);

    info!(
        target: "nebula.migration",
        from = current,
        to = target_version,
        count = pending.len(),
        "applying bundled migrations"
    );

    let mut applied: Vec<Migration> = Vec::new();
    for m in &pending {
        apply_one(conn, m).with_context(|| format!("applying migration {}", m.name))?;
        applied.push(m.clone());
    }
    Ok(applied)
}

/// M7b #96: 迁移前备份数据库。
///
/// 通过 `PRAGMA database_list` 获取主数据库文件路径,用 `VACUUM INTO`
/// 创建一致性快照(在线备份,不阻塞写入)。备份文件名格式:
/// `<db_name>.migrate_v{from}_to_v{to}.bak`,位于数据库同目录。
///
/// 跳过条件(不备份):
/// - `:memory:` 数据库(无文件路径)
/// - 获取路径失败
///
/// 失败处理(仅 warn,不返回 Err):
/// - `VACUUM INTO` SQL 执行失败(磁盘空间不足等)
///
/// 返回备份文件路径(成功时)/ None(跳过或失败时)。
fn backup_before_migrate(conn: &Connection, from: u32, to: u32) -> Option<PathBuf> {
    let db_path = match get_main_db_path(conn) {
        Some(p) if !p.as_os_str().is_empty() && p.to_str() != Some(":memory:") => p,
        _ => {
            debug!(
                target: "nebula.migration",
                "skipping pre-migration backup (in-memory db or no path)"
            );
            return None;
        }
    };

    let stem = db_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("nebula_db");
    let backup_name = format!("{stem}.migrate_v{from}_to_v{to}.bak");
    let backup_path = db_path.with_file_name(backup_name);

    // VACUUM INTO 创建一致性快照。
    // 注意:VACUUM INTO 不能在事务内执行,此处 conn 未在事务中(open 阶段)。
    // 路径用单引号包裹,防止路径中含特殊字符。
    // 用 execute_batch 而非 execute(后者需要 params 参数且返回影响的行数,
    // VACUUM INTO 不涉及行数概念)。
    let sql = format!("VACUUM INTO '{}'", backup_path.display());
    match conn.execute_batch(&sql) {
        Ok(_) => {
            info!(
                target: "nebula.migration",
                backup = %backup_path.display(),
                from,
                to,
                "pre-migration backup created"
            );
            Some(backup_path)
        }
        Err(e) => {
            warn!(
                target: "nebula.migration",
                error = %e,
                backup_path = %backup_path.display(),
                "pre-migration backup failed (VACUUM INTO); continuing migration without backup"
            );
            None
        }
    }
}

/// 通过 `PRAGMA database_list` 获取主数据库('main')的文件路径。
///
/// 返回 `None` 的情况:
/// - 查询失败
/// - 'main' 数据库行不存在
fn get_main_db_path(conn: &Connection) -> Option<PathBuf> {
    let mut stmt = conn.prepare("PRAGMA database_list").ok()?;
    let mut rows = stmt.query([]).ok()?;
    while let Some(row) = rows.next().ok()? {
        let name: String = row.get(1).ok()?;
        if name == "main" {
            let path: String = row.get(2).ok()?;
            return Some(PathBuf::from(path));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_db() -> (std::path::PathBuf, Connection) {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "nebula_mig_test_{}_{}.db",
            std::process::id(),
            n
        ));
        let conn = Connection::open(&path).unwrap();
        (path, conn)
    }

    fn temp_dir() -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nebula_mig_dir_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_dir(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    fn cleanup_file(p: &std::path::Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    #[test]
    fn current_version_defaults_to_zero() {
        let (path, conn) = temp_db();
        assert_eq!(current_version(&conn).unwrap(), 0);
        cleanup_file(&path);
    }

    #[test]
    fn bootstrap_v0_1_stamps_version_when_schema_present() {
        let (path, conn) = temp_db();
        // Simulate a v0.1 database that has the schema_version table
        // with version 1.
        conn.execute_batch(
            "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL, description TEXT NOT NULL DEFAULT '');\
             INSERT INTO schema_version(version, applied_at) VALUES (1, 0);",
        )
        .unwrap();
        bootstrap_v0_1_baseline(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 1);
        cleanup_file(&path);
    }

    #[test]
    fn bootstrap_v0_1_noop_when_already_set() {
        let (path, conn) = temp_db();
        conn.pragma_update(None, "user_version", 5i64).unwrap();
        bootstrap_v0_1_baseline(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 5);
        cleanup_file(&path);
    }

    #[test]
    fn discover_migrations_reads_files() {
        let dir = temp_dir();
        std::fs::write(dir.join("001_first.sql"), "SELECT 1;").unwrap();
        std::fs::write(dir.join("002_second.sql"), "SELECT 2;").unwrap();
        std::fs::write(dir.join("README"), "ignore me").unwrap();
        let all = discover_migrations(&dir).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].version, 1);
        assert_eq!(all[0].name, "first");
        assert_eq!(all[1].version, 2);
        assert_eq!(all[1].name, "second");
        cleanup_dir(&dir);
    }

    #[test]
    fn parse_filename_handles_edge_cases() {
        assert_eq!(parse_filename("001_a.sql"), Some((1, "a".to_string())));
        assert_eq!(
            parse_filename("123_xyz.sql"),
            Some((123, "xyz".to_string()))
        );
        assert_eq!(parse_filename("abc.sql"), None);
        assert_eq!(
            parse_filename("01_leading_zero.sql"),
            Some((1, "leading_zero".to_string()))
        );
    }

    #[test]
    fn split_sql_handles_string_semicolons() {
        let sql = "INSERT INTO t VALUES('a; b'); INSERT INTO t VALUES('c');";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2, "expected 2 statements, got {parts:?}");
        assert!(parts[0].contains("'a; b'"));
        assert!(parts[1].contains("'c'"));
    }

    #[test]
    fn split_sql_handles_line_comments() {
        let sql = "SELECT 1; -- this has a ; semicolon\nSELECT 2;";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2, "expected 2 statements, got {parts:?}");
        assert!(parts[0].starts_with("SELECT 1"));
        assert!(parts[1].contains("SELECT 2"));
    }

    #[test]
    fn split_sql_handles_block_comments() {
        let sql = "SELECT 1; /* block ; with ; semicolons */ SELECT 2;";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2, "expected 2 statements, got {parts:?}");
        assert!(parts[0].contains("SELECT 1"));
        assert!(parts[1].contains("SELECT 2"));
    }

    #[test]
    fn split_sql_handles_double_quoted_identifier() {
        let sql = "CREATE TABLE \"weird;name\" (id INT); SELECT 1;";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2, "expected 2 statements, got {parts:?}");
        assert!(parts[0].contains("\"weird;name\""));
    }

    #[test]
    fn split_sql_no_trailing_semicolon() {
        let sql = "SELECT 1; SELECT 2";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2);
    }
    #[test]
    fn split_sql_handles_trigger_body_semicolons() {
        let sql = "CREATE TRIGGER t AFTER INSERT ON x BEGIN INSERT INTO y VALUES(1); INSERT INTO y VALUES(2); END; SELECT 3;";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2, "expected 2 statements, got {parts:?}");
        assert!(parts[0].contains("CREATE TRIGGER"));
        assert!(parts[0].contains("INSERT INTO y VALUES(1)"));
        assert!(parts[0].contains("INSERT INTO y VALUES(2)"));
        assert!(parts[1].contains("SELECT 3"));
    }

    #[test]
    fn split_sql_handles_multiple_triggers() {
        let sql = "CREATE TRIGGER t1 AFTER INSERT ON x BEGIN INSERT INTO y VALUES(1); END; CREATE TRIGGER t2 AFTER INSERT ON x BEGIN INSERT INTO z VALUES(2); END;";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2, "expected 2 triggers, got {parts:?}");
        assert!(parts[0].contains("t1"));
        assert!(parts[1].contains("t2"));
    }

    #[test]
    fn run_migrations_applies_pending_only() {
        let dir = temp_dir();
        let (db_path, conn) = temp_db();

        // 001 creates a table; 002 adds an index.
        std::fs::write(
            dir.join("001_first.sql"),
            "CREATE TABLE t1 (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        std::fs::write(dir.join("002_second.sql"), "CREATE INDEX i1 ON t1(id);").unwrap();

        let applied = run_migrations(&conn, &dir).unwrap();
        assert_eq!(applied.len(), 2);
        assert_eq!(current_version(&conn).unwrap(), 2);

        // Re-running applies nothing.
        let applied2 = run_migrations(&conn, &dir).unwrap();
        assert!(applied2.is_empty());
        assert_eq!(current_version(&conn).unwrap(), 2);
        cleanup_dir(&dir);
        cleanup_file(&db_path);
    }

    #[test]
    fn run_migrations_skips_already_applied() {
        let dir = temp_dir();
        let (db_path, conn) = temp_db();
        std::fs::write(
            dir.join("001_first.sql"),
            "CREATE TABLE t1 (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        std::fs::write(dir.join("002_second.sql"), "CREATE INDEX i1 ON t1(id);").unwrap();

        // Simulate 001 already having been applied: create the table
        // manually and stamp user_version = 1.
        conn.execute_batch("CREATE TABLE t1 (id INTEGER PRIMARY KEY);")
            .unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        let applied = run_migrations(&conn, &dir).unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].version, 2);
        assert_eq!(current_version(&conn).unwrap(), 2);
        cleanup_dir(&dir);
        cleanup_file(&db_path);
    }

    #[test]
    fn migration_status_lists_all_with_applied_flag() {
        let dir = temp_dir();
        let (db_path, conn) = temp_db();
        std::fs::write(
            dir.join("001_first.sql"),
            "CREATE TABLE t1 (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        std::fs::write(dir.join("002_second.sql"), "CREATE INDEX i1 ON t1(id);").unwrap();

        let status = migration_status(&conn, &dir).unwrap();
        assert_eq!(status.applied.len(), 2);
        assert!(!status.applied[0].applied);
        assert!(!status.applied[1].applied);

        run_migrations(&conn, &dir).unwrap();
        let status = migration_status(&conn, &dir).unwrap();
        assert!(status.applied[0].applied);
        assert!(status.applied[1].applied);
        cleanup_dir(&dir);
        cleanup_file(&db_path);
    }

    // -----------------------------------------------------------------
    // v1.0 P0#9 regression tests.
    //
    // Approach A: 004_v05.sql no longer creates `e2ee_keys` and
    // 005_v10.sql drops the table on upgrade.  The two tests
    // below pin both halves of the contract.
    // -----------------------------------------------------------------

    #[test]
    fn p0_9_fresh_install_does_not_create_e2ee_keys_table() {
        // Run the full bundled migration set against a clean
        // database.  The `e2ee_keys` table MUST NOT exist.
        let (db_path, conn) = temp_db();
        run_migrations(&conn, bundled_migrations_dir()).unwrap_or_else(|e: anyhow::Error| {
            // 用 eprintln 输出完整错误链到 stderr,确保 nextest 捕获。
            // panic 消息用单行格式 (不换行),避免 GitHub annotations 截断。
            eprintln!("=== run_migrations failed (full error chain) ===");
            for cause in e.chain() {
                eprintln!("  cause: {cause}");
            }
            eprintln!("=== Debug ===");
            eprintln!("{e:#}");
            panic!("run_migrations failed: {}", format!("{e:#}").replace('\n', " | "));
        });
        let has: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='e2ee_keys'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        assert!(!has, "fresh install must not create e2ee_keys");
        cleanup_file(&db_path);
    }

    #[test]
    fn p0_9_005_v10_drops_orphan_e2ee_keys_table() {
        // Simulate a v0.5 database: run 004, then create the
        // orphan table, then run 005.  The table must be gone.
        let dir = temp_dir();
        let (db_path, conn) = temp_db();
        // Use only the bundled migrations we care about.
        std::fs::write(
            dir.join("004_v05.sql"),
            "CREATE TABLE e2ee_keys (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        std::fs::write(dir.join("005_v10.sql"), "DROP TABLE IF EXISTS e2ee_keys;").unwrap();
        run_migrations(&conn, &dir).unwrap();
        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='e2ee_keys'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 0, "e2ee_keys must be dropped by 005_v10");
        cleanup_dir(&dir);
        cleanup_file(&db_path);
    }

    // -----------------------------------------------------------------
    // M7b #96: 数据库迁移验证测试
    // -----------------------------------------------------------------

    #[test]
    fn bundled_migrations_includes_036_cost_work_type() {
        // M7b #96: 验证 036_cost_work_type.sql 已注册到 bundled_migrations()。
        // 这是 ADR-003 WorkType 维度成本统计的 schema 基础。
        let bundled = bundled_migrations();
        let has_036 = bundled
            .iter()
            .any(|(v, name, _)| *v == 36 && *name == "cost_work_type");
        assert!(
            has_036,
            "bundled_migrations() must include migration 036 (cost_work_type)"
        );
    }

    #[test]
    fn is_idempotent_error_catches_duplicate_column_name() {
        // M7b #96: 验证 is_idempotent_error 正确匹配 "duplicate column name"。
        // 这是 ALTER TABLE ADD COLUMN 重复应用时的典型错误。
        assert!(is_idempotent_error("duplicate column name: domain"));
        assert!(is_idempotent_error("DUPLICATE COLUMN NAME: x"));
        assert!(is_idempotent_error("table t1 already exists"));
        assert!(!is_idempotent_error("syntax error near ADD"));
        assert!(!is_idempotent_error("no such table: foo"));
    }

    #[test]
    fn alter_table_migration_tolerates_dirty_state() {
        // M7b #96: 验证 ALTER TABLE 迁移在脏状态下的幂等性。
        //
        // 场景:数据库已手动添加了列(如通过其他路径或迁移部分应用后 user_version 未 bump),
        // 但 user_version 仍指向旧版本。重新运行迁移时,ALTER TABLE ADD COLUMN 会报
        // "duplicate column name",is_idempotent_error 应兜底静默忽略,迁移继续。
        let dir = temp_dir();
        let (db_path, conn) = temp_db();

        // 001: 建表 t1
        std::fs::write(
            dir.join("001_create_t1.sql"),
            "CREATE TABLE t1 (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        // 002: 加列 x(TEXT) — 这是会被脏状态触发 "duplicate column name" 的迁移
        std::fs::write(
            dir.join("002_add_x.sql"),
            "ALTER TABLE t1 ADD COLUMN x TEXT;",
        )
        .unwrap();

        // 模拟脏状态:先应用 001 建表,然后手动 ALTER 添加 x 列(模拟 002 部分应用)
        run_migrations(&conn, &dir).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 2);

        // 重置 user_version 到 1(模拟 002 未完成 user_version bump)
        conn.pragma_update(None, "user_version", 1).unwrap();

        // 重新运行迁移:002 会尝试 ALTER TABLE ADD COLUMN x,但列已存在,
        // 应报 "duplicate column name",is_idempotent_error 兜底静默忽略。
        let applied = run_migrations(&conn, &dir).unwrap();
        assert_eq!(
            applied.len(),
            1,
            "migration 002 should be re-applied (idempotent)"
        );
        assert_eq!(current_version(&conn).unwrap(), 2);

        // 验证列确实存在且可写入
        conn.execute("INSERT INTO t1 (id, x) VALUES (1, 'hello')", [])
            .unwrap();
        let x: String = conn
            .query_row("SELECT x FROM t1 WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(x, "hello");

        cleanup_dir(&dir);
        cleanup_file(&db_path);
    }

    #[test]
    fn run_bundled_migrations_skips_backup_for_in_memory_db() {
        // M7b #96: 验证 :memory: 数据库跳过备份。
        // Connection::open_in_memory 创建纯内存数据库,无文件路径。
        let conn = Connection::open_in_memory().unwrap();
        // 手动 stamp user_version 到一个较低值,触发 pending migrations
        // (但不实际应用迁移,因为 001_initial.sql 需要文件系统)
        // 这里仅测试 backup_before_migrate 对 :memory: 的处理。
        let result = backup_before_migrate(&conn, 0, 36);
        assert!(
            result.is_none(),
            "in-memory db should skip backup (return None)"
        );
    }

    #[test]
    fn get_main_db_path_returns_path_for_file_db() {
        // M7b #96: 验证 get_main_db_path 对文件数据库返回路径。
        let (_db_path, conn) = temp_db();
        let path = get_main_db_path(&conn);
        assert!(path.is_some(), "file db should return Some(path)");
        let p = path.unwrap();
        assert!(
            !p.as_os_str().is_empty(),
            "path should not be empty for file db"
        );
        assert!(
            p.to_str() != Some(":memory:"),
            "path should not be :memory:"
        );
        // cleanup 由 temp_db 的调用者负责,但这里 db_path 被 _ 忽略了
        // 注意:conn drop 时会自动关闭,文件可清理
    }
}
