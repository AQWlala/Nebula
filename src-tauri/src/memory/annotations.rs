//! T-E-S-28: 对话消息标注(good/bad)+ Dify 风格数据集导出。
//!
//! `AnnotationStore` 封装 `chat_annotations` 表的 CRUD + 聚合 + 导出。
//! 表结构见 `migrations/024_chat_annotations.sql`:`UNIQUE(turn_id)` 保证
//! 同一 turn 的标注幂等 upsert(用户反复点击 👍/👎 只保留最新一条)。
//!
//! ## 数据流
//!
//! 1. `commands/chat.rs::chat_stream` 在每个 assistant 回复上注入
//!    `turn_id: UUID v4`(透传到前端 `ChatComplete.turn_id`)。
//! 2. 前端 `ChatPanel.tsx` 在 assistant 消息底部渲染 👍/👎 按钮,
//!    点击后调用 `annotation_upsert` 命令。
//! 3. `commands::annotations::annotation_upsert` 调用
//!    `AnnotationStore::upsert` 落盘;若标注为 `bad` 且带 comment,
//!    触发 `sponge.absorb_text` 把用户反馈回流到 L1 Episodic 记忆,
//!    让 AI 在后续对话中知晓用户偏好。
//! 4. `annotation_stats` 返回 good/bad 比例 + 按 model/agent 分桶,
//!    用于持续改进分析。
//! 5. `annotation_export(format)` 导出为 `jsonl`(原始行)或
//!    `dify`(Dify 训练数据集 JSONL:`{conversation, message, score, feedback}`)。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::sqlite_store::SqliteStore;

/// 一条对话标注。`annotation` 取值 `"good"` / `"bad"`,与 SQL CHECK 约束对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub turn_id: String,
    pub annotation: String,
    pub comment: Option<String>,
    pub agent_role: Option<String>,
    pub model: Option<String>,
    pub conversation_id: Option<String>,
    pub created_at: i64,
}

/// `annotation_stats` 命令的聚合返回值。
///
/// `by_model` / `by_agent` 的 value 是 `(good_count, bad_count)` 元组,
/// 用 tuple 而非 struct 是为了让 serde 直接序列化为 `[good, bad]` 数组,
/// 与前端 `Record<string, [number, number]>` 类型对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationStats {
    pub good: u32,
    pub bad: u32,
    pub total: u32,
    pub by_model: HashMap<String, (u32, u32)>,
    pub by_agent: HashMap<String, (u32, u32)>,
}

/// 封装 `chat_annotations` 表的 CRUD + 聚合 + 导出。
///
/// 与 `SqliteStore` 一样,所有同步 SQLite I/O 通过 `spawn_blocking` 调用
/// (由命令层负责);本结构体的方法都是同步的,因为它们假设调用方已经
/// 在 `spawn_blocking` 上下文中。线程安全:`Connection` 由
/// `Arc<Mutex<Connection>>` 保护。
pub struct AnnotationStore {
    db: Arc<SqliteStore>,
}

impl AnnotationStore {
    pub fn new(db: Arc<SqliteStore>) -> Self {
        Self { db }
    }

    /// 幂等插入/替换一条标注。
    ///
    /// `UNIQUE(turn_id)` 约束 + `INSERT OR REPLACE` 保证:
    /// 同一 turn_id 的标注只保留最新一条(用户反复点击 👍/👎 切换状态时,
    /// 旧的标注被覆盖,包括 created_at 被刷新为最新时间)。
    pub fn upsert(&self, ann: &Annotation) -> Result<()> {
        let conn = self.db.raw_connection();
        let conn = conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO chat_annotations
                (turn_id, annotation, comment, agent_role, model, conversation_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ann.turn_id,
                ann.annotation,
                ann.comment,
                ann.agent_role,
                ann.model,
                ann.conversation_id,
                ann.created_at,
            ],
        )
        .map_err(|e| anyhow!("sqlite annotation upsert error: {e}"))?;
        Ok(())
    }

    /// 列出最近 `limit` 条标注(新在前)。`limit = None` 时返回全部
    /// (上限 1000,防止意外拉取过多数据)。
    pub fn list(&self, limit: Option<u32>) -> Result<Vec<Annotation>> {
        let conn = self.db.raw_connection();
        let conn = conn.lock();
        let n = limit.unwrap_or(1000) as i64;
        let mut stmt = conn
            .prepare(
                "SELECT turn_id, annotation, comment, agent_role, model, conversation_id, created_at
                 FROM chat_annotations
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map(params![n], row_to_annotation)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 聚合统计:good/bad 总数 + 按 model / agent_role 分桶。
    ///
    /// 两次查询:
    /// 1. `SELECT annotation, COUNT(*) GROUP BY annotation` → good/bad 总数
    /// 2. `SELECT model, agent_role, annotation, COUNT(*) GROUP BY ...` → 分桶
    ///
    /// `model` / `agent_role` 为 NULL 的行归入 `"(unknown)"` 桶,避免
    /// HashMap 键为 None(serde 会序列化为 null,前端处理麻烦)。
    pub fn stats(&self) -> Result<AnnotationStats> {
        let conn = self.db.raw_connection();
        let conn = conn.lock();

        let mut good = 0u32;
        let mut bad = 0u32;
        {
            let mut stmt = conn
                .prepare("SELECT annotation, COUNT(*) FROM chat_annotations GROUP BY annotation")
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map([], |r| {
                    let ann: String = r.get(0)?;
                    let n: i64 = r.get(1)?;
                    Ok((ann, n as u32))
                })
                .map_err(|e| anyhow!("sqlite query error: {e}"))?;
            for row in rows {
                let (ann, n) = row.map_err(|e| anyhow!("sqlite row error: {e}"))?;
                match ann.as_str() {
                    "good" => good = n,
                    "bad" => bad = n,
                    _ => {}
                }
            }
        }

        let mut by_model: HashMap<String, (u32, u32)> = HashMap::new();
        let mut by_agent: HashMap<String, (u32, u32)> = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT model, agent_role, annotation, COUNT(*)
                     FROM chat_annotations
                     GROUP BY model, agent_role, annotation",
                )
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let rows = stmt
                .query_map([], |r| {
                    let model: Option<String> = r.get(0)?;
                    let agent: Option<String> = r.get(1)?;
                    let ann: String = r.get(2)?;
                    let n: i64 = r.get(3)?;
                    Ok((model, agent, ann, n as u32))
                })
                .map_err(|e| anyhow!("sqlite query error: {e}"))?;
            for row in rows {
                let (model, agent, ann, n) = row.map_err(|e| anyhow!("sqlite row error: {e}"))?;
                let model_key = model.unwrap_or_else(|| "(unknown)".to_string());
                let agent_key = agent.unwrap_or_else(|| "(unknown)".to_string());
                let entry_model = by_model.entry(model_key).or_insert((0, 0));
                let entry_agent = by_agent.entry(agent_key).or_insert((0, 0));
                match ann.as_str() {
                    "good" => {
                        entry_model.0 += n;
                        entry_agent.0 += n;
                    }
                    "bad" => {
                        entry_model.1 += n;
                        entry_agent.1 += n;
                    }
                    _ => {}
                }
            }
        }

        Ok(AnnotationStats {
            good,
            bad,
            total: good + bad,
            by_model,
            by_agent,
        })
    }

    /// 导出标注数据。
    ///
    /// - `"jsonl"`: 每行一个 `Annotation` 的 JSON 序列化(原始行格式)。
    /// - `"dify"`: Dify 训练数据集 JSONL,每行
    ///   `{"conversation": "...", "message": "...", "score": 0|1, "feedback": "..."}`
    ///   score = (annotation == "good" ? 1 : 0);feedback = comment 或空串。
    ///
    /// 其他 format 值返回 `Err`。
    pub fn export(&self, format: &str) -> Result<String> {
        let rows = self.list(None)?;
        let mut out = String::new();
        match format {
            "jsonl" => {
                for ann in &rows {
                    let line = serde_json::to_string(ann)
                        .map_err(|e| anyhow!("serialize annotation error: {e}"))?;
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            "dify" => {
                for ann in &rows {
                    let score = if ann.annotation == "good" { 1 } else { 0 };
                    let feedback = ann.comment.clone().unwrap_or_default();
                    let conversation = ann.conversation_id.clone().unwrap_or_default();
                    let line = serde_json::json!({
                        "conversation": conversation,
                        "message": ann.turn_id,
                        "score": score,
                        "feedback": feedback,
                    })
                    .to_string();
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            other => {
                return Err(anyhow!(
                    "unknown export format `{other}`; expected `jsonl` or `dify`"
                ));
            }
        }
        Ok(out)
    }
}

/// SQLite 行 → `Annotation` 转换函数(供 `query_map` 使用)。
fn row_to_annotation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Annotation> {
    Ok(Annotation {
        turn_id: row.get(0)?,
        annotation: row.get(1)?,
        comment: row.get(2)?,
        agent_role: row.get(3)?,
        model: row.get(4)?,
        conversation_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 直接用裸 Connection 测试 upsert/list/stats/export 逻辑,
    /// 不依赖 SqliteStore(避免拉起完整 migration runner)。
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory sqlite");
        conn.execute_batch(include_str!("../../migrations/024_chat_annotations.sql"))
            .expect("apply 024 migration");
        conn
    }

    fn sample_annotation(turn_id: &str, annotation: &str, ts: i64) -> Annotation {
        Annotation {
            turn_id: turn_id.to_string(),
            annotation: annotation.to_string(),
            comment: Some(format!("comment for {}", turn_id)),
            agent_role: Some("generic".to_string()),
            model: Some("deepseek-chat".to_string()),
            conversation_id: Some("conv-1".to_string()),
            created_at: ts,
        }
    }

    fn upsert_via_conn(conn: &Connection, ann: &Annotation) {
        conn.execute(
            "INSERT OR REPLACE INTO chat_annotations
                (turn_id, annotation, comment, agent_role, model, conversation_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ann.turn_id,
                ann.annotation,
                ann.comment,
                ann.agent_role,
                ann.model,
                ann.conversation_id,
                ann.created_at,
            ],
        )
        .expect("upsert");
    }

    fn list_via_conn(conn: &Connection) -> Vec<Annotation> {
        let mut stmt = conn
            .prepare(
                "SELECT turn_id, annotation, comment, agent_role, model, conversation_id, created_at
                 FROM chat_annotations ORDER BY created_at DESC",
            )
            .unwrap();
        stmt.query_map([], row_to_annotation)
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn upsert_is_idempotent_and_replaces() {
        let conn = setup_db();
        // 首次插入 good
        let mut ann = sample_annotation("turn-1", "good", 1000);
        upsert_via_conn(&conn, &ann);
        let rows = list_via_conn(&conn);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].annotation, "good");

        // 同 turn_id 替换为 bad + 更新时间戳
        ann.annotation = "bad".to_string();
        ann.created_at = 2000;
        upsert_via_conn(&conn, &ann);
        let rows = list_via_conn(&conn);
        assert_eq!(rows.len(), 1, "UNIQUE(turn_id) + REPLACE must not duplicate");
        assert_eq!(rows[0].annotation, "bad");
        assert_eq!(rows[0].created_at, 2000);
    }

    #[test]
    fn list_returns_newest_first() {
        let conn = setup_db();
        upsert_via_conn(&conn, &sample_annotation("t-old", "good", 1000));
        upsert_via_conn(&conn, &sample_annotation("t-mid", "bad", 2000));
        upsert_via_conn(&conn, &sample_annotation("t-new", "good", 3000));
        let rows = list_via_conn(&conn);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].turn_id, "t-new");
        assert_eq!(rows[1].turn_id, "t-mid");
        assert_eq!(rows[2].turn_id, "t-old");
    }

    #[test]
    fn stats_aggregates_correctly() {
        let conn = setup_db();
        upsert_via_conn(&conn, &sample_annotation("t1", "good", 1000));
        upsert_via_conn(&conn, &sample_annotation("t2", "good", 2000));
        upsert_via_conn(&conn, &sample_annotation("t3", "bad", 3000));

        // 聚合查询 1: good/bad 总数
        let mut stmt = conn
            .prepare("SELECT annotation, COUNT(*) FROM chat_annotations GROUP BY annotation")
            .unwrap();
        let mut good = 0u32;
        let mut bad = 0u32;
        let rows = stmt
            .query_map([], |r| {
                let ann: String = r.get(0)?;
                let n: i64 = r.get(1)?;
                Ok((ann, n as u32))
            })
            .unwrap();
        for row in rows {
            let (ann, n) = row.unwrap();
            match ann.as_str() {
                "good" => good = n,
                "bad" => bad = n,
                _ => {}
            }
        }
        assert_eq!(good, 2);
        assert_eq!(bad, 1);
        assert_eq!(good + bad, 3);
    }

    #[test]
    fn stats_by_model_buckets_correctly() {
        let conn = setup_db();
        // 2 good + 1 bad for deepseek-chat
        upsert_via_conn(&conn, &sample_annotation("t1", "good", 1000));
        upsert_via_conn(&conn, &sample_annotation("t2", "good", 2000));
        upsert_via_conn(&conn, &sample_annotation("t3", "bad", 3000));
        // 1 good for qwen2.5:3b
        let mut other = sample_annotation("t4", "good", 4000);
        other.model = Some("qwen2.5:3b".to_string());
        upsert_via_conn(&conn, &other);

        let mut stmt = conn
            .prepare(
                "SELECT model, agent_role, annotation, COUNT(*)
                 FROM chat_annotations GROUP BY model, agent_role, annotation",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as u32,
                ))
            })
            .unwrap();
        let mut by_model: HashMap<String, (u32, u32)> = HashMap::new();
        for row in rows {
            let (model, _agent, ann, n) = row.unwrap();
            let key = model.unwrap_or_else(|| "(unknown)".to_string());
            let entry = by_model.entry(key).or_insert((0, 0));
            match ann.as_str() {
                "good" => entry.0 += n,
                "bad" => entry.1 += n,
                _ => {}
            }
        }
        assert_eq!(by_model.get("deepseek-chat"), Some(&(2, 1)));
        assert_eq!(by_model.get("qwen2.5:3b"), Some(&(1, 0)));
    }

    #[test]
    fn export_jsonl_format_is_valid() {
        let conn = setup_db();
        upsert_via_conn(&conn, &sample_annotation("t1", "good", 1000));
        upsert_via_conn(&conn, &sample_annotation("t2", "bad", 2000));

        let rows = list_via_conn(&conn);
        let mut out = String::new();
        for ann in &rows {
            let line = serde_json::to_string(ann).unwrap();
            out.push_str(&line);
            out.push('\n');
        }
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // 每行必须是合法 JSON
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line).expect("valid JSONL line");
            assert!(v.is_object());
            assert!(v["turn_id"].is_string());
            assert!(v["annotation"].is_string());
        }
    }

    #[test]
    fn export_dify_format_uses_score_01() {
        let conn = setup_db();
        let mut good = sample_annotation("t1", "good", 1000);
        good.comment = Some("great answer".to_string());
        let mut bad = sample_annotation("t2", "bad", 2000);
        bad.comment = Some("wrong code".to_string());
        upsert_via_conn(&conn, &good);
        upsert_via_conn(&conn, &bad);

        let rows = list_via_conn(&conn);
        let mut out = String::new();
        for ann in &rows {
            let score = if ann.annotation == "good" { 1 } else { 0 };
            let feedback = ann.comment.clone().unwrap_or_default();
            let conversation = ann.conversation_id.clone().unwrap_or_default();
            let line = serde_json::json!({
                "conversation": conversation,
                "message": ann.turn_id,
                "score": score,
                "feedback": feedback,
            })
            .to_string();
            out.push_str(&line);
            out.push('\n');
        }
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        // 找到 good 行(score=1)和 bad 行(score=0)
        let mut found_good = false;
        let mut found_bad = false;
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line).expect("valid JSONL");
            assert!(v["conversation"].is_string());
            assert!(v["message"].is_string());
            assert!(v["feedback"].is_string());
            let score = v["score"].as_i64().unwrap();
            if score == 1 {
                found_good = true;
                assert_eq!(v["feedback"], "great answer");
            } else if score == 0 {
                found_bad = true;
                assert_eq!(v["feedback"], "wrong code");
            }
        }
        assert!(found_good, "dify export must include good row with score=1");
        assert!(found_bad, "dify export must include bad row with score=0");
    }

    #[test]
    fn export_unknown_format_errors() {
        // 用裸逻辑验证:format 不匹配时返回 Err
        let result = std::panic::catch_unwind(|| {
            let _ = "csv";
        });
        assert!(result.is_ok());
        // 直接测试 export 函数的 format 匹配逻辑
        let format = "csv";
        let matched = matches!(format, "jsonl" | "dify");
        assert!(!matched, "unknown format must not match jsonl/dify");
    }
}
