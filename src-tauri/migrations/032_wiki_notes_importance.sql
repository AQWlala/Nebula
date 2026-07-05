-- T-E-B-06: Wiki index.md + log.md 自动维护。
--
-- 为 wiki_notes 表追加 importance 列(0.0..1.0,默认 0.5),
-- 供 regenerate_index() 按 importance 降序 + created_at 升序稳定排序。
--
-- 设计要点:
--   * 默认 0.5 — 与 memories.importance 默认值对齐(spec §设计约束)。
--   * NOT NULL — 避免 NULL 排序不确定性。
--   * idx_wiki_notes_importance 索引 — 加速 regenerate_index 全量 ORDER BY。
--
-- 幂等:重复应用报 "duplicate column name" 由 is_idempotent_error 忽略。

ALTER TABLE wiki_notes ADD COLUMN importance REAL NOT NULL DEFAULT 0.5;

CREATE INDEX IF NOT EXISTS idx_wiki_notes_importance
    ON wiki_notes(importance DESC);
