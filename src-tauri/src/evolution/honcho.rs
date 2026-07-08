//! Honcho 辩证式用户建模。
//!
//! 对标: Hermes Agent 的 Honcho 闭环。
//! 参考: plastic-labs/honcho GitHub。
//!
//! 核心机制:
//! 1. 从对话历史中归纳用户偏好(dialectic modeling)
//! 2. 建立跨会话用户画像(user profile)
//! 3. 定期 nudge 用户确认/修正画像
//! 4. 画像注入到 System Prompt 影响后续交互
//!
//! 辩证式建模流程:
//! Thesis(初始假设) → Antithesis(对话中的反驳证据) → Synthesis(修正后的画像)
//!
//! ## Feature Gate
//!
//! 与 `evolution` 模块一致,由 `self-evolution` feature 门控。
//! LLM 调用走 `UnifiedModelDispatcher::dispatch(WorkType::Evolution)`,
//! 强制本地模型(ADR-003 P0-2 修复),避免用户画像外泄。

#![cfg(feature = "self-evolution")]

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::llm::dispatcher::{UnifiedModelDispatcher, WorkType};
use crate::llm::ollama::ChatMessage;

/// Honcho 辩证式用户建模引擎。
pub struct HonchoEngine {
    sqlite: Arc<parking_lot::Mutex<Connection>>,
    llm: Arc<UnifiedModelDispatcher>,
    /// Profile归纳的最小对话数(低于此数不归纳)。
    min_sessions_for_induction: usize,
    /// Nudge 冷却时间(默认24小时)。
    nudge_cooldown: Duration,
}

impl HonchoEngine {
    pub fn new(
        sqlite: Arc<parking_lot::Mutex<Connection>>,
        llm: Arc<UnifiedModelDispatcher>,
    ) -> Self {
        Self {
            sqlite,
            llm,
            min_sessions_for_induction: 5,
            nudge_cooldown: Duration::hours(24),
        }
    }

    /// 从对话历史归纳用户画像。
    ///
    /// 对标 Hermes:FTS5 搜索 + LLM 摘要。
    ///
    /// 流程:
    /// 1. 查询用户最近 N 条对话(复用现有 SQLite messages 表)
    /// 2. LLM 摘要每个对话(对标 Hermes 的 LLM summarization)
    /// 3. 辩证式归纳(thesis → antithesis → synthesis)
    /// 4. 存入 SQLite honcho_profile 表
    pub async fn build_profile_from_sessions(
        &self,
        user_id: &str,
        session_limit: usize,
    ) -> Result<UserProfile> {
        // 1. 拉取最近对话
        let sessions = self.fetch_recent_sessions(user_id, session_limit)?;
        if sessions.len() < self.min_sessions_for_induction {
            info!(
                target: "nebula.evolution.honcho",
                user_id = %user_id,
                session_count = sessions.len(),
                min_required = self.min_sessions_for_induction,
                "not enough sessions for profile induction"
            );
            return Ok(UserProfile::empty(user_id));
        }

        // 2. LLM 归纳辩证层
        let layers = self.dialectic_induction(&sessions).await?;

        // 3. 构造并存储画像
        let profile = UserProfile {
            id: format!("profile-{user_id}"),
            user_id: user_id.to_string(),
            dialectic_layers: layers,
            confidence: 0.5,
            last_nudge: None,
            updated_at: Utc::now(),
        };

        self.store_profile(&profile)?;
        info!(
            target: "nebula.evolution.honcho",
            user_id = %user_id,
            layers = profile.dialectic_layers.len(),
            "profile built"
        );
        Ok(profile)
    }

    /// 获取用户画像(从 SQLite 读取)。
    pub fn get_profile(&self, user_id: &str) -> Result<Option<UserProfile>> {
        let conn = self.sqlite.lock();
        let mut stmt = conn.prepare(
            "SELECT user_id, confidence, last_nudge, updated_at
             FROM honcho_profile
             WHERE user_id = ?1",
        )?;
        let mut rows = stmt.query(params![user_id])?;
        let row = match rows.next()? {
            Some(r) => r,
            None => return Ok(None),
        };
        let confidence: f32 = row.get(1)?;
        let last_nudge_ts: Option<i64> = row.get(2)?;
        let updated_ts: i64 = row.get(3)?;
        let last_nudge = last_nudge_ts
            .map(|ts| DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_else(|| Utc::now()));
        let updated_at =
            DateTime::<Utc>::from_timestamp(updated_ts, 0).unwrap_or_else(|| Utc::now());

        // 拉取所有辩证层
        let mut stmt_layers = conn.prepare(
            "SELECT thesis, antithesis, synthesis, evidence_count, confidence, created_at
             FROM honcho_dialectic_layer
             WHERE user_id = ?1
             ORDER BY created_at ASC",
        )?;
        let layers: Vec<DialecticLayer> = stmt_layers
            .query_map(params![user_id], |row| {
                let created_ts: i64 = row.get(5)?;
                Ok(DialecticLayer {
                    thesis: row.get(0)?,
                    antithesis: row.get(1)?,
                    synthesis: row.get(2)?,
                    evidence_count: row.get(3)?,
                    confidence: row.get(4)?,
                    created_at: DateTime::<Utc>::from_timestamp(created_ts, 0)
                        .unwrap_or_else(|| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(Some(UserProfile {
            id: format!("profile-{user_id}"),
            user_id: user_id.to_string(),
            dialectic_layers: layers,
            confidence,
            last_nudge,
            updated_at,
        }))
    }

    /// Nudge 用户确认/修正画像。
    ///
    /// 对标 Hermes: agent-curated memory with periodic nudges。
    /// 如果上次 nudge 在冷却时间内,返回 Skipped。
    pub async fn nudge_user(&self, user_id: &str) -> Result<NudgeResult> {
        let profile = match self.get_profile(user_id)? {
            Some(p) => p,
            None => return Ok(NudgeResult::Skipped),
        };

        if let Some(last) = profile.last_nudge {
            if Utc::now() - last < self.nudge_cooldown {
                return Ok(NudgeResult::Skipped);
            }
        }

        if profile.dialectic_layers.is_empty() {
            return Ok(NudgeResult::Skipped);
        }

        let prompt = self.generate_nudge_prompt(&profile);
        let nudge = Nudge {
            id: format!("nudge-{}-{}", user_id, Utc::now().timestamp()),
            user_id: user_id.to_string(),
            prompt,
            created_at: Utc::now(),
        };

        // 更新 last_nudge 时间戳
        {
            let conn = self.sqlite.lock();
            conn.execute(
                "UPDATE honcho_profile SET last_nudge = ?1 WHERE user_id = ?2",
                params![Utc::now().timestamp(), user_id],
            )?;
        }

        Ok(NudgeResult::Nudge(nudge))
    }

    /// 确认或修正某个辩证层。
    pub fn confirm_layer(
        &self,
        user_id: &str,
        layer_index: usize,
        confirmed: bool,
        correction: Option<String>,
    ) -> Result<()> {
        let conn = self.sqlite.lock();
        let now = Utc::now().timestamp();
        if confirmed {
            // 增加证据计数和置信度
            conn.execute(
                "UPDATE honcho_dialectic_layer
                 SET evidence_count = evidence_count + 1,
                     confidence = MIN(1.0, confidence + 0.1)
                 WHERE user_id = ?1 AND rowid IN (
                     SELECT rowid FROM honcho_dialectic_layer
                     WHERE user_id = ?1
                     ORDER BY created_at ASC
                     LIMIT 1 OFFSET ?2
                 )",
                params![user_id, layer_index as i64],
            )?;
        } else if let Some(correction) = correction {
            // 用户修正:把修正作为新的 synthesis
            conn.execute(
                "UPDATE honcho_dialectic_layer
                 SET synthesis = ?1,
                     confidence = 0.8,
                     evidence_count = evidence_count + 1
                 WHERE user_id = ?2 AND rowid IN (
                     SELECT rowid FROM honcho_dialectic_layer
                     WHERE user_id = ?2
                     ORDER BY created_at ASC
                     LIMIT 1 OFFSET ?3
                 )",
                params![correction, user_id, layer_index as i64],
            )?;
        }
        info!(
            target: "nebula.evolution.honcho",
            user_id = %user_id,
            layer_index,
            confirmed,
            "layer confirmed"
        );
        let _ = now; // timestamp for future audit log
        Ok(())
    }

    // -----------------------------------------------------------------------
    // 内部方法
    // -----------------------------------------------------------------------

    /// 拉取用户最近的对话(从 messages 表)。
    fn fetch_recent_sessions(&self, user_id: &str, limit: usize) -> Result<Vec<SessionSummary>> {
        let conn = self.sqlite.lock();
        let mut stmt = conn.prepare(
            "SELECT role, content, timestamp
             FROM messages
             WHERE user_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![user_id, limit as i64], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            let ts: i64 = row.get(2)?;
            Ok((role, content, ts))
        })?;

        let mut messages: Vec<(String, String, i64)> = Vec::new();
        for r in rows {
            if let Ok(m) = r {
                messages.push(m);
            }
        }
        // 反转为时间正序
        messages.reverse();

        // 聚合成对话(以 user 消息开始,assistant 响应结束)
        let mut sessions = Vec::new();
        let mut current_user_msg = String::new();
        let mut current_assistant_msg = String::new();
        let mut current_ts = 0i64;
        for (role, content, ts) in messages {
            if role == "user" {
                if !current_user_msg.is_empty() {
                    sessions.push(SessionSummary {
                        user_message: std::mem::take(&mut current_user_msg),
                        assistant_response: std::mem::take(&mut current_assistant_msg),
                        timestamp: current_ts,
                    });
                }
                current_user_msg = content;
                current_ts = ts;
            } else if role == "assistant" {
                current_assistant_msg.push_str(&content);
                current_assistant_msg.push('\n');
            }
        }
        if !current_user_msg.is_empty() {
            sessions.push(SessionSummary {
                user_message: current_user_msg,
                assistant_response: current_assistant_msg,
                timestamp: current_ts,
            });
        }
        Ok(sessions)
    }

    /// 辩证式归纳:用 LLM 从对话摘要中提取 thesis/antithesis/synthesis。
    async fn dialectic_induction(
        &self,
        sessions: &[SessionSummary],
    ) -> Result<Vec<DialecticLayer>> {
        // 构造 LLM 提示
        let mut conversation_text = String::new();
        for (i, s) in sessions.iter().enumerate() {
            conversation_text.push_str(&format!(
                "## Session {}\nUser: {}\nAssistant: {}\n\n",
                i + 1,
                truncate_str(&s.user_message, 500),
                truncate_str(&s.assistant_response, 500)
            ));
        }

        let system_prompt = r#"你是 Nebula 的 Honcho 辩证式建模引擎。
从用户的对话历史中归纳用户偏好,使用辩证法:

1. **Thesis(初始假设)**:从对话中提取一个用户偏好假设。
2. **Antithesis(反驳证据)**:寻找对话中与 thesis 矛盾的证据。
3. **Synthesis(综合)**:如果有矛盾,生成修正后的偏好;如果没有矛盾,thesis 直接成为 synthesis。

输出 JSON 数组,每个元素包含:
- thesis: 初始偏好假设
- antithesis: 反驳证据(可为空字符串)
- synthesis: 修正后的偏好
- confidence: 0.0-1.0 的置信度

只输出 JSON,不要其他文字。示例:
[{"thesis":"用户偏好中文","antithesis":"用户最近用英文提问","synthesis":"用户中英双语,视话题而定","confidence":0.7}]"#;

        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "以下是用户最近的对话历史:\n\n{conversation_text}\n\n请归纳用户画像。"
                ),
                ..Default::default()
            },
        ];

        let resp = self
            .llm
            .dispatch(WorkType::Evolution, messages)
            .await
            .context("Honcho LLM dispatch failed")?;

        // 解析 JSON 响应
        let layers = parse_dialectic_response(&resp.message.content)?;
        Ok(layers)
    }

    /// 存储/更新用户画像。
    fn store_profile(&self, profile: &UserProfile) -> Result<()> {
        let conn = self.sqlite.lock();
        let now = Utc::now().timestamp();

        // Upsert profile
        conn.execute(
            "INSERT INTO honcho_profile (user_id, confidence, last_nudge, updated_at)
             VALUES (?1, ?2, NULL, ?3)
             ON CONFLICT(user_id) DO UPDATE SET
                confidence = excluded.confidence,
                updated_at = excluded.updated_at",
            params![profile.user_id, profile.confidence, now],
        )?;

        // Clear existing layers and insert new ones
        conn.execute(
            "DELETE FROM honcho_dialectic_layer WHERE user_id = ?1",
            params![profile.user_id],
        )?;
        for layer in &profile.dialectic_layers {
            conn.execute(
                "INSERT INTO honcho_dialectic_layer
                    (user_id, thesis, antithesis, synthesis, evidence_count, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    profile.user_id,
                    layer.thesis,
                    layer.antithesis,
                    layer.synthesis,
                    layer.evidence_count,
                    layer.confidence,
                    layer.created_at.timestamp(),
                ],
            )?;
        }
        Ok(())
    }

    /// 生成 nudge 提示词。
    fn generate_nudge_prompt(&self, profile: &UserProfile) -> String {
        let mut prompt = String::from("我根据你最近的对话归纳了一些偏好,请确认或修正:\n\n");
        for (i, layer) in profile.dialectic_layers.iter().enumerate() {
            prompt.push_str(&format!("{}. 偏好: {}\n", i + 1, layer.synthesis));
            if !layer.antithesis.is_empty() {
                prompt.push_str(&format!("   (检测到矛盾: {})\n", layer.antithesis));
            }
            prompt.push('\n');
        }
        prompt.push_str("请回复\"确认\"或提供修正。");
        prompt
    }
}

/// 解析 LLM 返回的 JSON 为辩证层列表。
fn parse_dialectic_response(content: &str) -> Result<Vec<DialecticLayer>> {
    // 尝试从 markdown 代码块中提取 JSON
    let json_str = if let Some(start) = content.find("```json") {
        let after = &content[start + 7..];
        if let Some(end) = after.find("```") {
            &after[..end]
        } else {
            content
        }
    } else if let Some(start) = content.find("```") {
        let after = &content[start + 3..];
        if let Some(end) = after.find("```") {
            &after[..end]
        } else {
            content
        }
    } else {
        content
    };

    let parsed: Vec<serde_json::Value> = serde_json::from_str(json_str.trim()).unwrap_or_default();

    let now = Utc::now();
    let layers = parsed
        .into_iter()
        .map(|v| DialecticLayer {
            thesis: v["thesis"].as_str().unwrap_or("").to_string(),
            antithesis: v["antithesis"].as_str().unwrap_or("").to_string(),
            synthesis: v["synthesis"].as_str().unwrap_or("").to_string(),
            evidence_count: 1,
            confidence: v["confidence"].as_f64().unwrap_or(0.5) as f32,
            created_at: now,
        })
        .filter(|l| !l.thesis.is_empty())
        .collect();

    Ok(layers)
}

/// 用户画像。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub user_id: String,
    pub dialectic_layers: Vec<DialecticLayer>,
    pub confidence: f32,
    pub last_nudge: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl UserProfile {
    pub fn empty(user_id: &str) -> Self {
        Self {
            id: format!("profile-{user_id}"),
            user_id: user_id.to_string(),
            dialectic_layers: Vec::new(),
            confidence: 0.0,
            last_nudge: None,
            updated_at: Utc::now(),
        }
    }
}

/// 单个辩证层(thesis → antithesis → synthesis)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialecticLayer {
    pub thesis: String,
    pub antithesis: String,
    pub synthesis: String,
    pub evidence_count: u32,
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
}

/// 对话摘要(供 LLM 归纳用)。
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub user_message: String,
    pub assistant_response: String,
    pub timestamp: i64,
}

/// Nudge 结果。
#[derive(Debug, Clone)]
pub enum NudgeResult {
    Nudge(Nudge),
    Skipped,
}

/// 一个 nudge 请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nudge {
    pub id: String,
    pub user_id: String,
    pub prompt: String,
    pub created_at: DateTime<Utc>,
}

/// 截断字符串到指定字符数(粗略,按字节)。
fn truncate_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_str_respects_boundary() {
        let s = "hello world!".repeat(100);
        let t = truncate_str(&s, 50);
        assert!(t.len() <= 52);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn truncate_str_short_unchanged() {
        let s = "short";
        assert_eq!(truncate_str(s, 50), s);
    }

    #[test]
    fn empty_profile_has_no_layers() {
        let p = UserProfile::empty("user1");
        assert!(p.dialectic_layers.is_empty());
        assert_eq!(p.confidence, 0.0);
    }

    #[test]
    fn parse_dialectic_response_handles_json_block() {
        let content = r#"```json
[{"thesis":"用户喜欢Rust","antithesis":"","synthesis":"用户偏好Rust","confidence":0.9}]
```"#;
        let layers = parse_dialectic_response(content).expect("parse should succeed");
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].thesis, "用户喜欢Rust");
        assert_eq!(layers[0].confidence, 0.9);
    }

    #[test]
    fn parse_dialectic_response_handles_plain_json() {
        let content = r#"[{"thesis":"A","antithesis":"B","synthesis":"C","confidence":0.5}]"#;
        let layers = parse_dialectic_response(content).expect("parse should succeed");
        assert_eq!(layers.len(), 1);
    }

    #[test]
    fn parse_dialectic_response_empty_on_garbage() {
        let layers = parse_dialectic_response("not json").expect("parse should succeed");
        assert!(layers.is_empty());
    }
}
