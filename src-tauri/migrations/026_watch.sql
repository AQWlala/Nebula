-- T-E-S-55: 条件监控 Watch — watch_state 表。
--
-- 每个 Watch 触发器持久化其轮询状态,用于 Diff 检测与去抖:
--   * Web     → last_url_hash(SHA-256 body hash,变化时触发)
--   * System  → last_value(上次读数,用于去重)
--   * Calendar → last_value(上次触发的 event UID,避免重复触发)
--
-- trigger_id UNIQUE 保证一个 trigger 只有一行状态(INSERT OR REPLACE upsert)。
CREATE TABLE IF NOT EXISTS watch_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trigger_id TEXT NOT NULL UNIQUE,
    last_url_hash TEXT,
    last_value TEXT,
    last_fire_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_watch_state_trigger ON watch_state(trigger_id);
