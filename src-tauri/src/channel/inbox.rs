//! T-E-S-59 统一收件箱 — Unified inbox for all channel messages.
//!
//! 所有渠道(Telegram / Discord / WebChat / JiuwenSwarm)的入站消息
//! 汇入同一张 `inbox_messages` 表,前端可通过 `InboxView` 组件统一
//! 浏览与回复。回复时根据原消息的 `source_channel` 字段路由出站。
//!
//! 设计要点:
//! * **统一存储**:`InboxStore` 复用 `SqliteStore::raw_connection()`
//!   返回的 `Arc<Mutex<Connection>>`,不另开连接。
//! * **统一消息**:`UnifiedMessage` 同时兼容 `ChannelMessage`(v1,
//!   JiuwenSwarm 桥)和 `ChannelMessageV2`(v2,原生直连)。
//! * **路由回复**:`InboxManager::send_reply` 根据 `source_channel`
//!   字符串匹配 `ChannelKind`,调 `ChannelRouter::send`。
//! * **幂等建表**:`InboxStore::new()` 通过 `include_str!` 内嵌
//!   `023_inbox.sql` 并用 `CREATE TABLE IF NOT EXISTS` 幂等执行,
//!   不依赖 `bundled_migrations()` 注册(避免修改 `memory/migration.rs`)。

#![cfg(feature = "channels")]

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::router::ChannelRouter;
use super::types::{ChannelKind, ChannelMessage, ChannelMessageV2};

/// 内嵌的 migration SQL,用于 `InboxStore::new()` 幂等建表。
const INBOX_SCHEMA_SQL: &str = include_str!("../../migrations/023_inbox.sql");

// ---------------------------------------------------------------------------
// UnifiedMessage
// ---------------------------------------------------------------------------

/// 跨渠道统一消息表示。
///
/// 兼容 v1 `ChannelMessage`(JiuwenSwarm 桥)和 v2 `ChannelMessageV2`
/// (原生直连),由 `from_channel_message` / `from_channel_message_v2`
/// 转换而来。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedMessage {
    /// 全局唯一 ID(由调用方生成,通常 uuid v4)。
    pub id: String,
    /// 来源渠道字符串(对应 `ChannelKind::as_str`)。
    pub source_channel: String,
    /// 发送者标识。
    pub sender: String,
    /// 消息正文。
    pub content: String,
    /// Unix 毫秒时间戳。
    pub timestamp_ms: i64,
    /// 可选会话 ID。
    pub conversation_id: Option<String>,
    /// `true` = 入站(用户发来),`false` = 出站(我方发出)。
    pub inbound: bool,
    /// `false` = 未读,`true` = 已读。
    pub read: bool,
    /// 出站回复时,记录被回复的原消息 ID。
    pub original_message_id: Option<String>,
}

impl UnifiedMessage {
    /// 从 v1 `ChannelMessage`(JiuwenSwarm 桥接消息)转换。
    ///
    /// `source_channel` 取 `msg.channel.as_str()`(可能为
    /// web/feishu/telegram/wechat/dingtalk/wecom/desktop/discord)。
    pub fn from_channel_message(msg: &ChannelMessage) -> Self {
        Self {
            id: format!("inbox-{}", msg.timestamp_ms),
            source_channel: msg.channel.as_str().to_string(),
            sender: msg.sender.clone(),
            content: msg.body.clone(),
            timestamp_ms: msg.timestamp_ms,
            conversation_id: msg.conversation_id.clone(),
            inbound: true,
            read: false,
            original_message_id: None,
        }
    }

    /// 从 v2 `ChannelMessageV2`(原生直连消息)转换。
    ///
    /// `source_channel` 取 `msg.channel.as_str()`
    /// (jiuwenswarm/telegram/discord/webchat)。
    pub fn from_channel_message_v2(msg: &ChannelMessageV2) -> Self {
        Self {
            id: format!("inbox-{}", msg.timestamp),
            source_channel: msg.channel.as_str().to_string(),
            sender: msg.sender_id.clone(),
            content: msg.content.clone(),
            timestamp_ms: msg.timestamp,
            conversation_id: msg.reply_to.clone(),
            inbound: true,
            read: false,
            original_message_id: msg.reply_to.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// InboxStore
// ---------------------------------------------------------------------------

/// SQLite 持久化的统一收件箱。
///
/// 持有 `SqliteStore::raw_connection()` 返回的 `Arc<Mutex<Connection>>`,
/// 所有操作都在锁内同步执行(SQLite 是同步 API)。`new()` 会幂等建表。
#[derive(Clone)]
pub struct InboxStore {
    conn: Arc<Mutex<Connection>>,
}

impl InboxStore {
    /// 创建 `InboxStore` 并幂等执行 `023_inbox.sql` 建表。
    ///
    /// 该方法可安全多次调用:`CREATE TABLE IF NOT EXISTS` 保证幂等。
    pub fn new(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        {
            let g = conn.lock();
            g.execute_batch(INBOX_SCHEMA_SQL)
                .map_err(|e| anyhow!("InboxStore::new — apply 023_inbox.sql failed: {e}"))?;
        }
        debug!(target: "nebula.inbox", "InboxStore initialised (schema applied)");
        Ok(Self { conn })
    }

    /// 插入一条消息。如果 `id` 已存在则报错。
    pub fn insert(&self, msg: &UnifiedMessage) -> Result<()> {
        let g = self.conn.lock();
        g.execute(
            "INSERT INTO inbox_messages (
                id, source_channel, sender, content, timestamp_ms,
                conversation_id, inbound, read, original_message_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg.id,
                msg.source_channel,
                msg.sender,
                msg.content,
                msg.timestamp_ms,
                msg.conversation_id,
                msg.inbound as i32,
                msg.read as i32,
                msg.original_message_id,
            ],
        )
        .map_err(|e| anyhow!("InboxStore::insert error: {e}"))?;
        debug!(target: "nebula.inbox", id = %msg.id, channel = %msg.source_channel, "inserted message");
        Ok(())
    }

    /// 列出消息,按时间戳降序。可选渠道过滤。
    pub fn list(
        &self,
        limit: u32,
        offset: u32,
        channel_filter: Option<&str>,
    ) -> Result<Vec<UnifiedMessage>> {
        let g = self.conn.lock();
        let mut out: Vec<UnifiedMessage> = Vec::new();
        match channel_filter {
            Some(ch) => {
                let mut stmt = g
                    .prepare(
                        "SELECT id, source_channel, sender, content, timestamp_ms,
                                conversation_id, inbound, read, original_message_id
                         FROM inbox_messages
                         WHERE source_channel = ?1
                         ORDER BY timestamp_ms DESC
                         LIMIT ?2 OFFSET ?3",
                    )
                    .map_err(|e| anyhow!("InboxStore::list prepare error: {e}"))?;
                let rows = stmt
                    .query_map(params![ch, limit as i64, offset as i64], row_to_unified)
                    .map_err(|e| anyhow!("InboxStore::list query error: {e}"))?;
                for r in rows {
                    out.push(r.map_err(|e| anyhow!("InboxStore::list row error: {e}"))?);
                }
            }
            None => {
                let mut stmt = g
                    .prepare(
                        "SELECT id, source_channel, sender, content, timestamp_ms,
                                conversation_id, inbound, read, original_message_id
                         FROM inbox_messages
                         ORDER BY timestamp_ms DESC
                         LIMIT ?1 OFFSET ?2",
                    )
                    .map_err(|e| anyhow!("InboxStore::list prepare error: {e}"))?;
                let rows = stmt
                    .query_map(params![limit as i64, offset as i64], row_to_unified)
                    .map_err(|e| anyhow!("InboxStore::list query error: {e}"))?;
                for r in rows {
                    out.push(r.map_err(|e| anyhow!("InboxStore::list row error: {e}"))?);
                }
            }
        }
        Ok(out)
    }

    /// 标记一组消息为已读。不存在的 id 静默忽略。
    pub fn mark_read(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let g = self.conn.lock();
        for id in ids {
            g.execute(
                "UPDATE inbox_messages SET read = 1 WHERE id = ?1",
                params![id],
            )
            .map_err(|e| anyhow!("InboxStore::mark_read error: {e}"))?;
        }
        debug!(target: "nebula.inbox", count = ids.len(), "marked messages as read");
        Ok(())
    }

    /// 返回未读(`read = 0`)消息数。
    pub fn unread_count(&self) -> Result<u32> {
        let g = self.conn.lock();
        let n: i64 = g
            .query_row(
                "SELECT COUNT(*) FROM inbox_messages WHERE read = 0",
                [],
                |r| r.get(0),
            )
            .map_err(|e| anyhow!("InboxStore::unread_count error: {e}"))?;
        Ok(n.max(0) as u32)
    }

    /// 按 ID 查询单条消息。返回 `Ok(None)` 表示不存在。
    pub fn get_by_id(&self, id: &str) -> Result<Option<UnifiedMessage>> {
        let g = self.conn.lock();
        let row = g
            .query_row(
                "SELECT id, source_channel, sender, content, timestamp_ms,
                        conversation_id, inbound, read, original_message_id
                 FROM inbox_messages WHERE id = ?1",
                params![id],
                row_to_unified,
            )
            .optional()
            .map_err(|e| anyhow!("InboxStore::get_by_id error: {e}"))?;
        Ok(row)
    }
}

/// 行到 `UnifiedMessage` 的转换函数。
fn row_to_unified(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnifiedMessage> {
    let inbound: i32 = row.get(6)?;
    let read: i32 = row.get(7)?;
    Ok(UnifiedMessage {
        id: row.get(0)?,
        source_channel: row.get(1)?,
        sender: row.get(2)?,
        content: row.get(3)?,
        timestamp_ms: row.get(4)?,
        conversation_id: row.get(5)?,
        inbound: inbound != 0,
        read: read != 0,
        original_message_id: row.get(8)?,
    })
}

// ---------------------------------------------------------------------------
// InboxManager
// ---------------------------------------------------------------------------

/// 统一收件箱管理器,持有 `InboxStore` + `Arc<ChannelRouter>`。
///
/// `ingest` 将任意来源的消息写入收件箱;`send_reply` / `send_new`
/// 通过 `ChannelRouter` 路由出站消息。
pub struct InboxManager {
    store: InboxStore,
    router: Arc<ChannelRouter>,
}

impl InboxManager {
    pub fn new(store: InboxStore, router: Arc<ChannelRouter>) -> Self {
        Self { store, router }
    }

    /// 将一条入站(或出站回执)消息写入收件箱,返回消息 ID。
    pub fn ingest(&self, mut msg: UnifiedMessage) -> Result<String> {
        // 入站消息默认未读;出站消息默认已读(自己发的)。
        if !msg.inbound {
            msg.read = true;
        }
        let id = msg.id.clone();
        self.store.insert(&msg)?;
        info!(
            target: "nebula.inbox",
            id = %id,
            channel = %msg.source_channel,
            inbound = msg.inbound,
            "ingested message"
        );
        Ok(id)
    }

    /// 回复原消息:根据原消息 `source_channel` 路由出站。
    ///
    /// 流程:
    /// 1. 从 InboxStore 读出原消息(若不存在则报错)
    /// 2. 解析 `source_channel` → `ChannelKind`(未知渠道报错)
    /// 3. 调 `ChannelRouter::send(kind, body, Some(original_id))`
    /// 4. 写一条出站记录到收件箱(`inbound=false`、`read=true`)
    pub async fn send_reply(&self, message_id: &str, body: &str) -> Result<()> {
        let original = self
            .store
            .get_by_id(message_id)?
            .ok_or_else(|| anyhow!("send_reply: original message {message_id} not found"))?;
        let kind = parse_channel_kind(&original.source_channel)?;
        self.router
            .send(&kind, body, Some(message_id))
            .await
            .map_err(|e| anyhow!("send_reply router.send error: {e}"))?;

        // 写一条出站记录,便于前端在收件箱中显示对话历史。
        let reply_msg = UnifiedMessage {
            id: format!(
                "reply-{}-{}",
                message_id,
                chrono::Utc::now().timestamp_millis()
            ),
            source_channel: original.source_channel.clone(),
            sender: "nebula".to_string(),
            content: body.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            conversation_id: original.conversation_id.clone(),
            inbound: false,
            read: true,
            original_message_id: Some(message_id.to_string()),
        };
        if let Err(e) = self.store.insert(&reply_msg) {
            // 路由已成功,出站记录写入失败不应阻断主流程。
            warn!(target: "nebula.inbox", error = %e, "failed to persist outbound reply record");
        }
        Ok(())
    }

    /// 主动发起新消息到指定渠道。
    ///
    /// `target_channel` 必须是 `ChannelKind::as_str()` 之一
    /// (jiuwenswarm / telegram / discord / webchat)。
    pub async fn send_new(&self, target_channel: &str, body: &str) -> Result<()> {
        let kind = parse_channel_kind(target_channel)?;
        self.router
            .send(&kind, body, None)
            .await
            .map_err(|e| anyhow!("send_new router.send error: {e}"))?;

        let out_msg = UnifiedMessage {
            id: format!("out-{}", chrono::Utc::now().timestamp_millis()),
            source_channel: target_channel.to_string(),
            sender: "nebula".to_string(),
            content: body.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            conversation_id: None,
            inbound: false,
            read: true,
            original_message_id: None,
        };
        if let Err(e) = self.store.insert(&out_msg) {
            warn!(target: "nebula.inbox", error = %e, "failed to persist outbound new message record");
        }
        Ok(())
    }
}

/// 将 `source_channel` 字符串解析为 `ChannelKind`。
///
/// 接受 `ChannelKind::as_str()` 的全部输出,以及 v1 `Channel::as_str()`
/// 的部分别名(telegram / discord / web → webchat)。
fn parse_channel_kind(s: &str) -> Result<ChannelKind> {
    match s.to_lowercase().as_str() {
        "telegram" => Ok(ChannelKind::Telegram),
        "discord" => Ok(ChannelKind::Discord),
        "webchat" | "web" => Ok(ChannelKind::WebChat),
        "jiuwenswarm" => Ok(ChannelKind::JiuwenSwarm),
        other => Err(anyhow!("unknown channel kind: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个内存 SQLite 连接(已应用 inbox schema),用 Arc<Mutex> 包裹。
    fn mem_store() -> InboxStore {
        let conn = Connection::open_in_memory().expect("open_in_memory");
        let conn = Arc::new(Mutex::new(conn));
        InboxStore::new(conn).expect("InboxStore::new on in-memory db")
    }

    fn sample_msg(id: &str, channel: &str, ts: i64) -> UnifiedMessage {
        UnifiedMessage {
            id: id.to_string(),
            source_channel: channel.to_string(),
            sender: "alice".to_string(),
            content: format!("hello from {channel} at {ts}"),
            timestamp_ms: ts,
            conversation_id: None,
            inbound: true,
            read: false,
            original_message_id: None,
        }
    }

    #[test]
    fn inbox_store_insert_and_list() {
        let store = mem_store();
        store
            .insert(&sample_msg("m1", "telegram", 1000))
            .expect("insert should succeed");
        store
            .insert(&sample_msg("m2", "discord", 2000))
            .expect("insert should succeed");
        store
            .insert(&sample_msg("m3", "webchat", 1500))
            .expect("insert should succeed");

        // 全量列表(按 ts 降序):m2(2000) > m3(1500) > m1(1000)
        let all = store.list(100, 0, None).expect("update should succeed");
        assert_eq!(all.len(), 3, "expected 3 messages, got {}", all.len());
        assert_eq!(all[0].id, "m2");
        assert_eq!(all[1].id, "m3");
        assert_eq!(all[2].id, "m1");

        // 渠道过滤:只取 telegram
        let tg = store
            .list(100, 0, Some("telegram"))
            .expect("update should succeed");
        assert_eq!(tg.len(), 1);
        assert_eq!(tg[0].id, "m1");

        // limit + offset
        let paged = store.list(1, 1, None).expect("update should succeed");
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].id, "m3");
    }

    #[test]
    fn inbox_store_mark_read() {
        let store = mem_store();
        store
            .insert(&sample_msg("r1", "telegram", 1000))
            .expect("insert should succeed");
        store
            .insert(&sample_msg("r2", "telegram", 2000))
            .expect("insert should succeed");
        store
            .insert(&sample_msg("r3", "telegram", 3000))
            .expect("insert should succeed");

        assert_eq!(store.unread_count().expect("get should succeed"), 3);
        store
            .mark_read(&["r1".to_string(), "r2".to_string()])
            .expect("test op should succeed");
        assert_eq!(store.unread_count().expect("get should succeed"), 1);

        // 已读消息的 read 字段应为 true
        let r1 = store
            .get_by_id("r1")
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(r1.read);
        let r3 = store
            .get_by_id("r3")
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(!r3.read);

        // mark_read 不存在的 id 不报错
        store
            .mark_read(&["nonexistent".to_string()])
            .expect("get should succeed");
    }

    #[test]
    fn inbox_store_unread_count() {
        let store = mem_store();
        assert_eq!(store.unread_count().expect("get should succeed"), 0);

        store
            .insert(&sample_msg("u1", "telegram", 1000))
            .expect("insert should succeed");
        store
            .insert(&sample_msg("u2", "discord", 2000))
            .expect("insert should succeed");
        assert_eq!(store.unread_count().expect("get should succeed"), 2);

        store
            .mark_read(&["u1".to_string()])
            .expect("get should succeed");
        assert_eq!(store.unread_count().expect("get should succeed"), 1);
    }

    #[test]
    fn inbox_manager_ingest_then_list() {
        let store = mem_store();
        // 用一个空的 ChannelRouter(无适配器注册),ingest 不需要 router
        let router = Arc::new(ChannelRouter::new());
        let mgr = InboxManager::new(store.clone(), router);

        let msg = sample_msg("mgr-1", "telegram", 12345);
        let id = mgr.ingest(msg).expect("test op should succeed");
        assert_eq!(id, "mgr-1");

        // ingest 后立即可 list
        let list = store.list(100, 0, None).expect("update should succeed");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "mgr-1");
        assert!(
            !list[0].read,
            "inbound message should be unread after ingest"
        );

        // 出站消息(inbound=false)ingest 后应自动标记为已读
        let out_msg = UnifiedMessage {
            id: "mgr-out".to_string(),
            source_channel: "telegram".to_string(),
            sender: "nebula".to_string(),
            content: "outbound".to_string(),
            timestamp_ms: 13000,
            conversation_id: None,
            inbound: false,
            read: false, // 故意设为 false,InboxManager 应纠正
            original_message_id: None,
        };
        mgr.ingest(out_msg).expect("test op should succeed");
        let out = store
            .get_by_id("mgr-out")
            .expect("get should succeed")
            .expect("get should succeed");
        assert!(
            out.read,
            "outbound message should be marked read after ingest"
        );
        assert_eq!(
            store.unread_count().expect("get should succeed"),
            1,
            "only the inbound one is unread"
        );
    }

    #[test]
    fn from_channel_message_v2_converts_fields() {
        let v2 = ChannelMessageV2 {
            channel: ChannelKind::Telegram,
            sender_id: "user-42".to_string(),
            content: "hi from tg".to_string(),
            timestamp: 1700000000,
            reply_to: Some("orig-1".to_string()),
        };
        let u = UnifiedMessage::from_channel_message_v2(&v2);
        assert_eq!(u.source_channel, "telegram");
        assert_eq!(u.sender, "user-42");
        assert_eq!(u.content, "hi from tg");
        assert_eq!(u.timestamp_ms, 1700000000);
        assert!(u.inbound);
        assert!(!u.read);
        assert_eq!(u.original_message_id.as_deref(), Some("orig-1"));
    }

    #[test]
    fn parse_channel_kind_handles_known_and_unknown() {
        assert_eq!(
            parse_channel_kind("telegram").expect("parse should succeed"),
            ChannelKind::Telegram
        );
        assert_eq!(
            parse_channel_kind("discord").expect("parse should succeed"),
            ChannelKind::Discord
        );
        assert_eq!(
            parse_channel_kind("webchat").expect("parse should succeed"),
            ChannelKind::WebChat
        );
        assert_eq!(
            parse_channel_kind("web").expect("parse should succeed"),
            ChannelKind::WebChat
        );
        assert_eq!(
            parse_channel_kind("jiuwenswarm").expect("parse should succeed"),
            ChannelKind::JiuwenSwarm
        );
        // 大小写不敏感
        assert_eq!(
            parse_channel_kind("Telegram").expect("parse should succeed"),
            ChannelKind::Telegram
        );
        // 未知渠道
        assert!(parse_channel_kind("slack").is_err());
    }
}
