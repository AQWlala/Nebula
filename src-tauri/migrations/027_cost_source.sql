-- T-E-A-12: Automation Credits — cost_records 表新增 source / trigger_id 列。
--
-- CostTracker 当前为内存态(Mutex<Vec<CostRecord>>),持久化留作 T-E-A-13。
-- 此 migration 为前向兼容预留:
--   * CREATE TABLE IF NOT EXISTS 保证首次应用时不报 "no such table";
--   * ALTER TABLE 在重复应用时报 "duplicate column name",migration runner
--     (见 migration.rs::is_idempotent_error)将其视为幂等错误静默忽略;
--   * source 列默认 'chat',与 CostSource::Chat 默认值对齐;
--   * trigger_id 列可空,非触发器调用为 NULL。
--
-- source 取值:chat / automation / cron / background
-- (对应 CostSource 枚举 #[serde(rename_all = "snake_case")])。
CREATE TABLE IF NOT EXISTS cost_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    timestamp TEXT NOT NULL,
    provider TEXT,
    task TEXT,
    agent TEXT
);

ALTER TABLE cost_records ADD COLUMN source TEXT NOT NULL DEFAULT 'chat';
ALTER TABLE cost_records ADD COLUMN trigger_id TEXT;

-- 按来源分桶查询索引(credits_overview group_by=source 高频访问)。
CREATE INDEX IF NOT EXISTS idx_cost_records_source ON cost_records(source);
-- 按时间范围查询索引(每日预算聚合 / 趋势图)。
CREATE INDEX IF NOT EXISTS idx_cost_records_timestamp ON cost_records(timestamp);
