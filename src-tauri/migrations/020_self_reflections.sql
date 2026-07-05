-- T-S1-A-06: self_reflections 表 — 持久化 L5 Self-Reflection 结果。
--
-- 设计文档 v7.0 §2.1 L5 Metacognitive Layer。
-- EXPERT_REVIEW §2.2.3 指出 v2.0 的 SelfReflectionEngine.reflect_all()
-- 不写库,L5 无法历史回溯,是 L6 的前置阻塞。
--
-- 与旧的 `reflections` 表（v0.2 的模板摘要）共存：
-- - `reflections` 表：v0 摘要式反思（template/periodic），单条 memory → 单条 reflection
-- - `self_reflections` 表（本表）：v2.0 批判性反思（value_alignment/outcome_review/self_improvement），
--   跨多条 memory 的综合性反思，含 action_items 和 severity
--
-- 字段说明：
--   id              — UUIDv4 主键
--   kind            — ReflectionKind 字符串（value_alignment/outcome_review/self_improvement）
--   title           — 一句话总结
--   content         — 详细内容（100-500 字）
--   insights        — JSON 数组，关键洞见/教训
--   action_items    — JSON 数组，具体行动建议
--   confidence      — [0,1] 反思置信度
--   severity        — [0,1] 严重程度/重要性
--   related_memory_ids — JSON 数组，相关记忆 ID（溯源链）
--   created_at      — Unix 时间戳（秒）

CREATE TABLE IF NOT EXISTS self_reflections (
    id                  TEXT PRIMARY KEY NOT NULL,
    kind                TEXT NOT NULL,
    title               TEXT NOT NULL,
    content             TEXT NOT NULL,
    insights            TEXT NOT NULL DEFAULT '[]',
    action_items        TEXT NOT NULL DEFAULT '[]',
    confidence          REAL NOT NULL DEFAULT 0.5,
    severity            REAL NOT NULL DEFAULT 0.5,
    related_memory_ids  TEXT NOT NULL DEFAULT '[]',
    created_at          INTEGER NOT NULL
);

-- 按时间倒序查询（供 UI 历史回溯面板使用）
CREATE INDEX IF NOT EXISTS idx_self_reflections_created_at
    ON self_reflections(created_at DESC);

-- 按 kind 过滤（供按反思类型筛选）
CREATE INDEX IF NOT EXISTS idx_self_reflections_kind
    ON self_reflections(kind);
