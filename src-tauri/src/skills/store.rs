//! SQLite-backed CRUD over the `skills` table.
//!
//! The v0.1 schema reserved a `skills` table but no API ever wrote to
//! it. v0.3 promotes it to a first-class subsystem: this module
//! provides typed insert/get/list/rate primitives that the
//! [`SkillEngine`](crate::skills::engine::SkillEngine) and the
//! Tauri/gRPC command layers share.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use tracing::debug;

use super::types::{Skill, TagCount, TagMatch};
use crate::memory::sqlite_store::SqliteStore;

/// Thread-safe CRUD wrapper for the `skills` table.
#[derive(Clone)]
pub struct SkillStore {
    conn: Arc<Mutex<Connection>>,
}

impl SkillStore {
    /// Opens (or re-uses) the `skills` table on the given [`SqliteStore`].
    /// The store is shared with the rest of the system so the
    /// connection + WAL mode are reused.
    pub fn new(sqlite: SqliteStore) -> Result<Self> {
        let conn = sqlite.raw_connection();
        // Quick sanity: the v0.1 schema must already include the
        // `skills` table. We don't re-run migrations here — the
        // bootstrap pipeline handles that.
        {
            let g = conn.lock();
            let present: bool = g
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='skills'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n > 0)
                .unwrap_or(false);
            if !present {
                return Err(anyhow!(
                    "skills table not initialised; run migrations first"
                ));
            }
        }
        Ok(Self { conn })
    }

    /// Construct a `SkillStore` without checking for the `skills` table.
    /// Used as a panic-free fallback when [`new`](Self::new) fails (e.g.
    /// migrations not yet run). Query operations will return errors
    /// instead of panicking at construction.
    pub(crate) fn from_sqlite_unchecked(sqlite: SqliteStore) -> Self {
        Self {
            conn: sqlite.raw_connection(),
        }
    }

    /// Convenience for tests: opens a fresh DB file with the v0.1 +
    /// v0.2 + v0.3 migrations applied.
    pub fn open_test<P: AsRef<Path>>(path: P) -> Result<Self> {
        let sqlite = SqliteStore::open(&path)?;
        let conn = sqlite.raw_connection();
        {
            let g = conn.lock();
            crate::memory::migration::run_bundled_migrations(&g)?;
        }
        Self::new(sqlite)
    }

    /// Inserts a new skill. `now` is used as both `created_at` and
    /// `updated_at`. Returns the stored row.
    pub fn insert(&self, s: &Skill) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let tags_json = serde_json::to_string(&s.tags).unwrap_or_else(|_| "[]".to_string());
        let activation_json = s
            .activation_condition
            .as_ref()
            .map(|a| serde_json::to_string(a).unwrap_or_default());
        let platform_json = s
            .platform
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_default());
        let g = self.conn.lock();
        // FK integrity: skills.memory_id REFERENCES memories(id).
        // When source_memory_id is None, create a minimal placeholder
        // memory so the FK constraint is satisfied.
        let memory_id = s.source_memory_id.clone().unwrap_or_else(|| s.id.clone());
        if s.source_memory_id.is_none() {
            g.execute(
                "INSERT OR IGNORE INTO memories (id, memory_type, layer, content, last_access, created_at) VALUES (?1, 'Procedural', 'L3', '', ?2, ?2)",
                params![memory_id, now],
            )?;
        }
        g.execute(
            "INSERT INTO skills
                (id, memory_id, name, description, steps, trigger,
                 success_count, failure_count, last_used, created_at,
                 code, language, tags, usage_count, avg_rating,
                 rating_count, updated_at, source_memory_id,
                 activation_condition, platform, min_confidence,
                 trust_level, permissions, capabilities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                     ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13, ?14, ?15,
                     ?16, ?17, ?18, ?19, ?20, ?21,
                     ?22, ?23, ?24)",
            params![
                s.id,
                memory_id,
                s.name,
                s.description,
                "[]",
                "",
                0,
                0,
                0,
                now,
                s.code,
                s.language,
                tags_json,
                s.usage_count,
                s.avg_rating,
                s.rating_count,
                now,
                s.source_memory_id,
                activation_json,
                platform_json,
                s.min_confidence,
                s.trust_level,
                serde_json::to_string(&s.permissions).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&s.capabilities).unwrap_or_else(|_| "{}".to_string()),
            ],
        )?;
        debug!(target: "nebula.skills", id = %s.id, name = %s.name, "inserted skill");
        Ok(())
    }

    /// Upserts a skill: inserts if new, updates if the id already exists.
    /// Used by the auto-discovery system to refresh discovered skills.
    pub fn upsert(&self, s: &Skill) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let tags_json = serde_json::to_string(&s.tags).unwrap_or_else(|_| "[]".to_string());
        let activation_json = s
            .activation_condition
            .as_ref()
            .map(|a| serde_json::to_string(a).unwrap_or_default());
        let platform_json = s
            .platform
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_default());
        let g = self.conn.lock();
        let memory_id = s.source_memory_id.clone().unwrap_or_else(|| s.id.clone());
        if s.source_memory_id.is_none() {
            g.execute(
                "INSERT OR IGNORE INTO memories (id, memory_type, layer, content, last_access, created_at) VALUES (?1, 'Procedural', 'L3', '', ?2, ?2)",
                params![memory_id, now],
            )?;
        }
        g.execute(
            "INSERT OR REPLACE INTO skills
                (id, memory_id, name, description, steps, trigger,
                 success_count, failure_count, last_used, created_at,
                 code, language, tags, usage_count, avg_rating,
                 rating_count, updated_at, source_memory_id,
                 activation_condition, platform, min_confidence,
                 trust_level, permissions, capabilities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                     ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13, ?14, ?15,
                     ?16, ?17, ?18, ?19, ?20, ?21,
                     ?22, ?23, ?24)",
            params![
                s.id,
                memory_id,
                s.name,
                s.description,
                "[]",
                "",
                0,
                0,
                0,
                now,
                s.code,
                s.language,
                tags_json,
                s.usage_count,
                s.avg_rating,
                s.rating_count,
                now,
                s.source_memory_id,
                activation_json,
                platform_json,
                s.min_confidence,
                s.trust_level,
                serde_json::to_string(&s.permissions).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&s.capabilities).unwrap_or_else(|_| "{}".to_string()),
            ],
        )?;
        debug!(target: "nebula.skills", id = %s.id, name = %s.name, "upserted skill");
        Ok(())
    }

    /// Fetches a skill by id.
    pub fn get(&self, id: &str) -> Result<Option<Skill>> {
        let g = self.conn.lock();
        g.query_row(
            "SELECT id, name, description, code, language, tags,
                    usage_count, avg_rating, rating_count,
                    created_at, updated_at, source_memory_id,
                    activation_condition, platform, min_confidence,
                    trust_level, permissions, capabilities
             FROM skills WHERE id = ?1",
            params![id],
            row_to_skill,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Lists skills, optionally filtered by language and tag(s).
    ///
    /// **T-E-S-37 扩展**:新增 `tags: &[String]` + `tag_match: TagMatch` 参数,
    /// 支持多 tag 的 OR(Any)/AND(All)匹配。当 `tags` 非空时,优先使用多 tag
    /// 过滤逻辑;否则降级到旧 `tag: Option<&str>` 单 tag 路径(向后兼容)。
    ///
    /// 所有 tag 值都用参数化绑定(`?` 占位符)防 SQL 注入,不用字符串拼接。
    pub fn list(
        &self,
        language: Option<&str>,
        single_tag: Option<&str>,
        tags: &[String],
        tag_match: TagMatch,
        limit: u32,
    ) -> Result<Vec<Skill>> {
        let g = self.conn.lock();
        // Build a dynamic WHERE clause. The `tags` column stores a
        // JSON array; we use `like '%"tag"%' as a cheap, index-free
        // filter — fine for the marketplace scale we expect.
        let mut sql = String::from(
            "SELECT id, name, description, code, language, tags,
                    usage_count, avg_rating, rating_count,
                    created_at, updated_at, source_memory_id,
                    activation_condition, platform, min_confidence,
                    trust_level, permissions, capabilities
             FROM skills WHERE 1=1",
        );
        if language.is_some() {
            sql.push_str(" AND language = ?");
        }

        // T-E-S-37: 多 tag 优先于单 tag。当 tags 非空时使用多 tag 逻辑
        // (按 tag_match 模式 OR / AND 拼接),否则降级到旧单 tag 路径。
        let use_multi_tags = !tags.is_empty();
        if use_multi_tags {
            // 多 tag 模式:每个 tag 用一个 `?` 占位符 + `LIKE ?` 表达式,
            // 防止 SQL 注入。OR 模式用括号包裹避免与前置条件歧义。
            let likes: Vec<&str> = tags.iter().map(|_| "tags LIKE ?").collect();
            let joined = likes.join(match tag_match {
                TagMatch::Any => " OR ",
                TagMatch::All => " AND ",
            });
            // Any 模式必须用括号包裹,否则 `AND (cond1 OR cond2)` 语义会被前置条件
            // 破坏;All 模式括号无害但更清晰。
            sql.push_str(&format!(" AND ({joined})"));
        } else if single_tag.is_some() {
            sql.push_str(" AND tags LIKE ?");
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");

        let mut stmt = g.prepare(&sql)?;

        // 收集所有参数到堆上,然后转为 `&dyn ToSql` 引用序列。
        // 注意顺序必须与 SQL 占位符顺序一致:language, [tag(s)], limit。
        let lang_p = language.map(|s| s.to_string());
        let single_tag_p = if !use_multi_tags {
            single_tag.map(|s| format!("%\"{}\"%", s))
        } else {
            None
        };
        // 多 tag 模式:每个 tag 序列化为 `%"tagN"%`(与单 tag 一致的 LIKE 模式)。
        let multi_tag_p: Vec<String> = tags.iter().map(|t| format!("%\"{}\"%", t)).collect();
        let lim_p = limit.max(1) as i64;

        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        if let Some(ref s) = lang_p {
            params_vec.push(s as &dyn rusqlite::ToSql);
        }
        if use_multi_tags {
            for s in multi_tag_p.iter() {
                params_vec.push(s as &dyn rusqlite::ToSql);
            }
        } else if let Some(ref s) = single_tag_p {
            params_vec.push(s as &dyn rusqlite::ToSql);
        }
        params_vec.push(&lim_p);

        let rows = stmt
            .query_map(params_vec.as_slice(), row_to_skill)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// T-E-S-37: 聚合所有 tag 的频次,按 count 降序返回。
    ///
    /// 用 `json_each(skills.tags)` 把每个 skill 的 JSON 数组展开为多行,
    /// 然后按 tag 字符串分组计数。无任何 skill 的 tag 返回空 Vec。
    ///
    /// SQL 形如:
    /// ```sql
    /// SELECT tag, COUNT(*) FROM (
    ///   SELECT json_each.value AS tag FROM skills, json_each(skills.tags)
    /// ) GROUP BY tag ORDER BY COUNT(*) DESC
    /// ```
    pub fn all_tags(&self) -> Vec<TagCount> {
        let g = self.conn.lock();
        let mut stmt = match g.prepare(
            "SELECT tag, COUNT(*) AS cnt FROM (
                SELECT json_each.value AS tag FROM skills, json_each(skills.tags)
             ) GROUP BY tag ORDER BY cnt DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| {
            Ok(TagCount {
                tag: r.get::<_, String>(0)?,
                count: r.get::<_, i64>(1)? as usize,
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Atomically updates `usage_count`, `avg_rating` and
    /// `rating_count`. The new `avg_rating` is computed as a
    /// weighted average: `(old_avg * old_count + new_rating) /
    /// (old_count + 1)`.
    pub fn rate(&self, id: &str, rating: f32) -> Result<Skill> {
        let now = chrono::Utc::now().timestamp();
        // Millisecond precision for analytics; the PK is an
        // auto-increment id (migration 018), so there is no
        // collision risk regardless of timestamp resolution.
        let rating_ts = chrono::Utc::now().timestamp_millis();
        {
            let g = self.conn.lock();
            let tx = g.unchecked_transaction()?;
            // Insert the raw rating first.
            tx.execute(
                "INSERT INTO skill_ratings(skill_id, rating, created_at) VALUES (?1, ?2, ?3)",
                params![id, rating, rating_ts],
            )?;
            let (old_count, old_avg): (i64, f64) = tx
                .query_row(
                    "SELECT rating_count, avg_rating FROM skills WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| anyhow!("skill not found: {id}"))?;
            let new_count = old_count + 1;
            let new_avg = if old_count == 0 {
                rating as f64
            } else {
                (old_avg * old_count as f64 + rating as f64) / new_count as f64
            };
            tx.execute(
                "UPDATE skills SET rating_count = ?2, avg_rating = ?3, updated_at = ?4
                 WHERE id = ?1",
                params![id, new_count, new_avg, now],
            )?;
            tx.commit()?;
        } // g dropped here — lock released before calling self.get()
        self.get(id)?
            .ok_or_else(|| anyhow!("skill disappeared after update: {id}"))
            .context("rate")
    }

    /// Increments `usage_count` (called after a successful execution).
    pub fn bump_usage(&self, id: &str) -> Result<()> {
        let g = self.conn.lock();
        let n = g.execute(
            "UPDATE skills SET usage_count = usage_count + 1, last_used = ?2 WHERE id = ?1",
            params![id, chrono::Utc::now().timestamp()],
        )?;
        if n == 0 {
            return Err(anyhow!("skill not found: {id}"));
        }
        Ok(())
    }

    /// Searches skills by name / description substring. Vector search
    /// lives in [`crate::skills::engine::SkillEngine`] — this method
    /// is the cheap fallback.
    pub fn text_search(&self, query: &str, limit: u32) -> Result<Vec<Skill>> {
        let g = self.conn.lock();
        let mut stmt = g.prepare(
            "SELECT id, name, description, code, language, tags,
                    usage_count, avg_rating, rating_count,
                    created_at, updated_at, source_memory_id,
                    activation_condition, platform, min_confidence
             FROM skills
             WHERE name LIKE ?1 OR description LIKE ?1 OR tags LIKE ?1
             ORDER BY usage_count DESC, avg_rating DESC
             LIMIT ?2",
        )?;
        let pat = format!("%{}%", query);
        let rows = stmt
            .query_map(params![pat, limit.max(1) as i64], row_to_skill)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns the total number of skills.
    pub fn count(&self) -> Result<i64> {
        let g = self.conn.lock();
        let n: i64 = g.query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))?;
        Ok(n)
    }
}

fn row_to_skill(row: &Row<'_>) -> rusqlite::Result<Skill> {
    let tags_s: String = row.get(5)?;
    let tags: Vec<String> = serde_json::from_str(&tags_s).unwrap_or_default();
    let updated_at: i64 = row.get(10)?;
    let source_memory_id: Option<String> = row.get(11)?;
    let activation_condition: Option<super::types::ActivationCondition> = row
        .get::<_, Option<String>>(12)?
        .and_then(|s| serde_json::from_str(&s).ok());
    let platform: Option<Vec<String>> = row
        .get::<_, Option<String>>(13)?
        .and_then(|s| serde_json::from_str(&s).ok());
    let min_confidence: Option<f32> = row.get(14)?;
    let trust_level: u8 = row.get::<_, i64>(15).unwrap_or(0) as u8;
    let permissions: Vec<String> = row
        .get::<_, Option<String>>(16)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let capabilities: super::sandbox::CapabilitySet = row
        .get::<_, Option<String>>(17)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Ok(Skill {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        code: row.get(3)?,
        language: row.get(4)?,
        tags,
        usage_count: row
            .get::<_, u32>(6)
            .or_else(|_| row.get::<_, i64>(6).map(|v| v as u32))?,
        avg_rating: row.get(7)?,
        rating_count: row
            .get::<_, u32>(8)
            .or_else(|_| row.get::<_, i64>(8).map(|v| v as u32))?,
        created_at: row.get(9)?,
        updated_at: if updated_at == 0 {
            row.get(9)?
        } else {
            updated_at
        },
        source_memory_id,
        activation_condition,
        platform,
        min_confidence,
        trust_level,
        permissions,
        capabilities,
    })
}

// Silence the unused-import warning for `FromStr` when the test module
// is compiled out.
#[allow(dead_code)]
fn _fromstr_keep(s: &str) -> std::result::Result<(), String> {
    let _ = <String as FromStr>::from_str(s);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> (PathBuf, SkillStore) {
        let mut p = std::env::temp_dir();
        p.push(format!("nebula_skill_test_{}.db", uuid::Uuid::new_v4()));
        let s = SkillStore::open_test(&p).expect("create should succeed");
        (p, s)
    }

    fn sample() -> Skill {
        Skill {
            id: "sk-1".to_string(),
            name: "palindrome".to_string(),
            description: "checks if a string is a palindrome".to_string(),
            code: "fn is_pal(s: &str) -> bool { s.chars().rev().collect::<String>() == s }"
                .to_string(),
            language: "rust".to_string(),
            tags: vec!["string".to_string(), "utility".to_string()],
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: super::super::sandbox::CapabilitySet::new(),
        }
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    #[test]
    fn insert_and_get_round_trip() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed");
        let got = s
            .get("sk-1")
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got.name, "palindrome");
        assert_eq!(got.tags, vec!["string".to_string(), "utility".to_string()]);
        assert!(got.created_at > 0);
        cleanup(&p);
    }

    #[test]
    fn list_filters_by_language_and_tag() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed");
        let mut b = sample();
        b.id = "sk-2".to_string();
        b.name = "math_utils".to_string();
        b.language = "python".to_string();
        b.tags = vec!["math".to_string()];
        s.insert(&b).expect("insert should succeed");

        // T-E-S-37: list() 签名已扩展,旧调用方需传入 tags=[] + tag_match=Any。
        let all = s
            .list(None, None, &[], TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(all.len(), 2);
        let rust_only = s
            .list(Some("rust"), None, &[], TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(rust_only.len(), 1);
        assert_eq!(rust_only[0].id, "sk-1");
        let math_only = s
            .list(None, Some("math"), &[], TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(math_only.len(), 1);
        assert_eq!(math_only[0].id, "sk-2");
        cleanup(&p);
    }

    /// T-E-S-37: 多 tag OR(Any)匹配 — 任一 tag 命中即返回。
    #[test]
    fn list_filters_by_multiple_tags_any() {
        let (p, s) = temp_db();
        // sk-1: tags = [string, utility]
        s.insert(&sample()).expect("insert should succeed");
        // sk-2: tags = [math]
        let mut b = sample();
        b.id = "sk-2".to_string();
        b.name = "math_utils".to_string();
        b.language = "python".to_string();
        b.tags = vec!["math".to_string()];
        s.insert(&b).expect("insert should succeed");
        // sk-3: tags = [string, math]
        let mut c = sample();
        c.id = "sk-3".to_string();
        c.name = "string_math".to_string();
        c.tags = vec!["string".to_string(), "math".to_string()];
        s.insert(&c).expect("insert should succeed");

        // tags=[string, math] + Any:应返回 3 条(每条都至少命中一个)。
        let tags = vec!["string".to_string(), "math".to_string()];
        let hits = s
            .list(None, None, &tags, TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(hits.len(), 3, "Any match should return all 3");

        // tags=[string, utility] + Any:应返回 2 条(sk-1 命中两个,sk-3 命中 string)。
        let tags = vec!["string".to_string(), "utility".to_string()];
        let hits = s
            .list(None, None, &tags, TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(hits.len(), 2, "Any match should return sk-1 + sk-3");

        // tags=[nonexistent] + Any:应返回 0 条。
        let tags = vec!["nonexistent".to_string()];
        let hits = s
            .list(None, None, &tags, TagMatch::Any, 10)
            .expect("test op should succeed");
        assert!(hits.is_empty(), "no skill should match nonexistent tag");
        cleanup(&p);
    }

    /// T-E-S-37: 多 tag AND(All)匹配 — 所有 tag 都必须命中。
    #[test]
    fn list_filters_by_multiple_tags_all() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed"); // tags = [string, utility]
        let mut b = sample();
        b.id = "sk-2".to_string();
        b.name = "math_utils".to_string();
        b.language = "python".to_string();
        b.tags = vec!["math".to_string()];
        s.insert(&b).expect("insert should succeed");
        let mut c = sample();
        c.id = "sk-3".to_string();
        c.name = "string_math".to_string();
        c.tags = vec!["string".to_string(), "math".to_string()];
        s.insert(&c).expect("insert should succeed");

        // tags=[string, math] + All:只有 sk-3 同时有两个 tag,应返回 1 条。
        let tags = vec!["string".to_string(), "math".to_string()];
        let hits = s
            .list(None, None, &tags, TagMatch::All, 10)
            .expect("test op should succeed");
        assert_eq!(hits.len(), 1, "All match should return only sk-3");
        assert_eq!(hits[0].id, "sk-3");

        // tags=[string, utility] + All:只有 sk-1 同时有两个 tag,应返回 1 条。
        let tags = vec!["string".to_string(), "utility".to_string()];
        let hits = s
            .list(None, None, &tags, TagMatch::All, 10)
            .expect("test op should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "sk-1");

        // tags=[math, nonexistent] + All:应返回 0 条(无人同时有)。
        let tags = vec!["math".to_string(), "nonexistent".to_string()];
        let hits = s
            .list(None, None, &tags, TagMatch::All, 10)
            .expect("test op should succeed");
        assert!(
            hits.is_empty(),
            "All match with nonexistent tag should return 0"
        );
        cleanup(&p);
    }

    /// T-E-S-37: 多 tag 与 language 过滤可叠加。
    #[test]
    fn list_filters_by_language_and_multiple_tags() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed"); // rust, [string, utility]
        let mut c = sample();
        c.id = "sk-3".to_string();
        c.name = "string_math".to_string();
        c.language = "python".to_string();
        c.tags = vec!["string".to_string(), "math".to_string()];
        s.insert(&c).expect("insert should succeed");

        // language=rust + tags=[string, math] + Any:只匹配 sk-1(string 命中)。
        let tags = vec!["string".to_string(), "math".to_string()];
        let hits = s
            .list(Some("rust"), None, &tags, TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "sk-1");
        cleanup(&p);
    }

    /// T-E-S-37: tags 非空时优先于单 tag 字段(单 tag 被忽略)。
    #[test]
    fn list_multi_tags_takes_precedence_over_single_tag() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed"); // sk-1: [string, utility]

        // single_tag="nonexistent" + tags=["string"]:应返回 sk-1,
        // 因为 tags 非空时使用多 tag 路径,single_tag 被忽略。
        let tags = vec!["string".to_string()];
        let hits = s
            .list(None, Some("nonexistent"), &tags, TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "sk-1");
        cleanup(&p);
    }

    /// T-E-S-37: SQL 注入防护 — tag 值含特殊字符(引号 / 分号)应被安全绑定。
    /// 不会改变 SQL 语义,只会作为 LIKE 模式的一部分匹配(此处应无匹配)。
    #[test]
    fn list_tags_use_parameterized_binding_no_sql_injection() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed");
        // tag 值含单引号 + 分号 + DROP TABLE:应被 `?` 绑定为字面量 LIKE 模式,
        // 不会执行注入的 SQL。预期 0 匹配(tag 不存在)。
        let evil_tag = "'; DROP TABLE skills; --".to_string();
        let hits = s
            .list(None, None, &[evil_tag], TagMatch::Any, 10)
            .expect("test op should succeed");
        assert!(hits.is_empty(), "evil tag should match nothing");
        // 验证 skills 表仍存在(未被 DROP)。
        let after = s
            .list(None, None, &[], TagMatch::Any, 10)
            .expect("test op should succeed");
        assert_eq!(after.len(), 1, "skills table must still exist");
        cleanup(&p);
    }

    /// T-E-S-37: all_tags() 正确聚合 tag 频次,按 count 降序返回。
    #[test]
    fn all_tags_aggregates_correctly() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed"); // sk-1: [string, utility]
        let mut b = sample();
        b.id = "sk-2".to_string();
        b.name = "string_math".to_string();
        b.tags = vec!["string".to_string(), "math".to_string()];
        s.insert(&b).expect("insert should succeed");
        let mut c = sample();
        c.id = "sk-3".to_string();
        c.name = "string_only".to_string();
        c.tags = vec!["string".to_string()];
        s.insert(&c).expect("insert should succeed");

        let counts = s.all_tags();
        // string 出现 3 次,math 1 次,utility 1 次。
        // 顺序:string(3) > math(1) == utility(1)(后者顺序不稳定)。
        assert_eq!(counts.len(), 3, "should have 3 unique tags");
        assert_eq!(counts[0].tag, "string");
        assert_eq!(counts[0].count, 3, "string should appear 3 times");
        // 验证 math + utility 各 1 次(顺序无保证,用 find)。
        let math = counts
            .iter()
            .find(|t| t.tag == "math")
            .expect("math missing");
        assert_eq!(math.count, 1);
        let utility = counts
            .iter()
            .find(|t| t.tag == "utility")
            .expect("utility missing");
        assert_eq!(utility.count, 1);
        cleanup(&p);
    }

    /// T-E-S-37: 无 skill 时 all_tags() 返回空 Vec。
    #[test]
    fn all_tags_returns_empty_when_no_skills() {
        let (p, s) = temp_db();
        assert!(s.all_tags().is_empty());
        cleanup(&p);
    }

    /// T-E-S-37: skill.tags = [](空数组)时 all_tags() 不应报错也不应贡献行。
    #[test]
    fn all_tags_skips_empty_tag_arrays() {
        let (p, s) = temp_db();
        let mut b = sample();
        b.id = "sk-empty-tags".to_string();
        b.tags = vec![];
        s.insert(&b).expect("insert should succeed");
        let counts = s.all_tags();
        assert!(
            counts.is_empty(),
            "empty tags array should not contribute rows"
        );
        cleanup(&p);
    }

    #[test]
    fn rate_updates_avg_atomically() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed");
        s.rate("sk-1", 5.0).expect("test op should succeed");
        s.rate("sk-1", 3.0).expect("test op should succeed");
        let got = s
            .get("sk-1")
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got.rating_count, 2);
        assert!((got.avg_rating - 4.0).abs() < 1e-6);
        cleanup(&p);
    }

    #[test]
    fn text_search_matches_name_and_tags() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed");
        let hits = s
            .text_search("palindrome", 10)
            .expect("query should succeed");
        assert_eq!(hits.len(), 1);
        let hits = s.text_search("string", 10).expect("query should succeed");
        assert_eq!(hits.len(), 1);
        cleanup(&p);
    }

    #[test]
    fn bump_usage_increments() {
        let (p, s) = temp_db();
        s.insert(&sample()).expect("insert should succeed");
        s.bump_usage("sk-1").expect("test op should succeed");
        s.bump_usage("sk-1").expect("test op should succeed");
        let got = s
            .get("sk-1")
            .expect("get should succeed")
            .expect("get should succeed");
        assert_eq!(got.usage_count, 2);
        cleanup(&p);
    }
}
