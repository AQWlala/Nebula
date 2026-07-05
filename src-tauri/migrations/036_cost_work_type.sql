-- M5 #72: CostTracker 按 WorkType 分域统计 — cost_records 表新增 work_type 列。
--
-- work_type 取值:chat / swarm_worker / swarm_synthesize / master_task /
-- evolution / soul_compile / classifier (见 WorkType::as_str())。
-- 旧记录(无此列)在 SELECT 时通过 COALESCE 兜底为 NULL,反序列化时
-- CostRecord.work_type = None。
--
-- 与现有 task 字段的区别:
--   * task: 任意自由文本(可能含 "chat" / "swarm" 等);
--   * work_type: 来自 WorkType::as_str() 强类型枚举字符串,前端可精确分桶。
--
-- 幂等性: ALTER TABLE 在重复应用时报 "duplicate column name",
-- migration runner (migration.rs::is_idempotent_error) 静默忽略。
ALTER TABLE cost_records ADD COLUMN work_type TEXT;

-- 按 work_type 分桶查询索引(credits_overview group_by=work_type 高频访问)。
CREATE INDEX IF NOT EXISTS idx_cost_records_work_type ON cost_records(work_type);
