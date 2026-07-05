-- T-E-S-59: 统一收件箱 — 所有渠道(Telegram/Discord/Web/JiuwenSwarm)消息汇入一张表。
-- UnifiedMessage 持久化层,由 InboxStore 操作,InboxManager 调用 ingest/send_reply。
-- 字段说明:
--   id                   — 全局唯一消息 ID(由调用方生成,通常 uuid v4)
--   source_channel       — 来源渠道字符串(对应 ChannelKind::as_str: telegram/discord/webchat/jiuwenswarm)
--   sender               — 发送者标识(Telegram user_id / Discord handle / WebChat session)
--   content              — 消息正文
--   timestamp_ms         — Unix 毫秒时间戳
--   conversation_id      — 可选会话 ID(用于跨消息合并对话)
--   inbound              — 1=入站(用户发来),0=出站(我方发出)
--   read                 — 0=未读,1=已读(仅对 inbound 有意义)
--   original_message_id  — 可选,出站回复时记录被回复的原消息 id
CREATE TABLE IF NOT EXISTS inbox_messages (
    id TEXT PRIMARY KEY,
    source_channel TEXT NOT NULL,
    sender TEXT NOT NULL,
    content TEXT NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    conversation_id TEXT,
    inbound INTEGER NOT NULL,
    read INTEGER NOT NULL DEFAULT 0,
    original_message_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_inbox_ts ON inbox_messages(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_inbox_channel ON inbox_messages(source_channel, timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_inbox_unread ON inbox_messages(read, timestamp_ms DESC);
