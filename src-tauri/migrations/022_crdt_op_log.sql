-- T-S6-B-03: CRDT op 日志表 — 记录本地产生的 CRDT op 供 relay_client 消费。
-- 这是 CrdtEngine(纯计算)与 relay_client(传输)之间的落盘桥梁:
--   1. 本地记忆变更时,record_op() 将 CrdtVersion 信息写入此表(status='pending')
--   2. relay_client 调用 fetch_pending_ops() 拉取未消费的 op
--   3. relay_client 成功推送后调用 mark_consumed() 标记已消费(status='consumed')
CREATE TABLE IF NOT EXISTS crdt_op_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id TEXT NOT NULL UNIQUE,              -- uuid v4,全局唯一
    memory_id TEXT NOT NULL,                 -- 关联的记忆 ID
    device_id TEXT NOT NULL,                 -- 产生 op 的设备
    version INTEGER NOT NULL,                -- CrdtVersion.version
    timestamp INTEGER NOT NULL,              -- CrdtVersion.timestamp (unix secs)
    field_changes TEXT NOT NULL,             -- JSON 序列化的 Vec<FieldChange>
    status TEXT NOT NULL DEFAULT 'pending',  -- pending / consumed / failed
    created_at INTEGER NOT NULL,             -- 入库时间 (unix secs)
    consumed_at INTEGER                      -- 消费时间
);

CREATE INDEX IF NOT EXISTS idx_crdt_op_log_status ON crdt_op_log(status);
CREATE INDEX IF NOT EXISTS idx_crdt_op_log_memory ON crdt_op_log(memory_id);
