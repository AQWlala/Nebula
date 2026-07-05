-- T-E-S-54: 事件触发器 — 文件/消息/Webhook 三种触发器持久化。
--
-- triggers 表存储触发器配置(condition/action 以 JSON 字符串保存),
-- trigger_fire_log 表记录每次触发的结果(成功/失败 + 错误信息)。
--
-- 设计参考 003_skills.sql::skills 表风格:
--   * id TEXT PRIMARY KEY(UUIDv4)
--   * created_at INTEGER(Unix 毫秒)
--   * 索引按 kind/enabled 分桶,便于 list 查询
CREATE TABLE IF NOT EXISTS triggers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    condition TEXT NOT NULL,
    action_kind TEXT NOT NULL,
    action_payload TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_fired_at INTEGER,
    fire_count INTEGER NOT NULL DEFAULT 0,
    debounce_ms INTEGER NOT NULL DEFAULT 1000,
    max_fires INTEGER
);

CREATE INDEX IF NOT EXISTS idx_triggers_kind ON triggers(kind);
CREATE INDEX IF NOT EXISTS idx_triggers_enabled ON triggers(enabled);

-- 触发器触发日志(每次 dispatch 一条记录)。
-- success=1 表示动作执行成功,success=0 表示失败(error 字段填充错误信息)。
-- payload 是触发时携带的 JSON 负载(可能为 NULL)。
CREATE TABLE IF NOT EXISTS trigger_fire_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    trigger_id TEXT NOT NULL,
    fired_at INTEGER NOT NULL,
    success INTEGER NOT NULL,
    error TEXT,
    payload TEXT
);

CREATE INDEX IF NOT EXISTS idx_fire_log_trigger ON trigger_fire_log(trigger_id, fired_at DESC);
