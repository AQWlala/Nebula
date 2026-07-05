-- M2a 任务 #29: Memory.domain 字段（P0-9 修复）。
--
-- 为 memories 表添加 `domain` 列，用于按"域"隔离记忆。
-- 域是一个字符串标识（如 "system"、"agent_a"、"worker:task_123"），
-- 与 CostSource（触发场景）和 SourceKind（来源类型）正交，构成第三维度。
--
-- 设计要点：
--   * 默认 'shared'：向后兼容旧记忆（无 domain 概念时归入公共域）。
--   * M2b 将引入 PrincipalDomainMap 实现 ACL 按 domain 过滤。
--   * M4 EvolutionEngine 写入时通过 absorb_with_principal() 指定 domain。
--   * 查询时通过 WHERE domain = ? 实现域隔离（M2a 任务 #31）。
--
-- 幂等模式参考 030_ingest_cost.sql：
--   * 重复应用时报 "duplicate column name"，
--     migration runner（见 migration.rs::is_idempotent_error）将其视为幂等错误静默忽略；
--   * 旧记忆读取 NULL → row_to_memory 容错回退为 "shared"。
--
-- 索引：加速 WHERE domain = ? 查询（list_recent / list_by_layer 等高频路径）。
ALTER TABLE memories ADD COLUMN domain TEXT NOT NULL DEFAULT 'shared';

-- 域隔离查询索引（覆盖 list_recent / list_by_layer / candidates_for_compression 等高频路径）。
CREATE INDEX IF NOT EXISTS idx_memories_domain ON memories(domain);
