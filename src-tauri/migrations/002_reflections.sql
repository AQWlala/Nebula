-- nebula v0.2 migration 002 — reflection engine support
-- Adds the `importance` column to `reflections` (v0.1 was missing it),
-- supporting indexes, and a join table linking each reflection to
-- the memories that triggered it.
--
-- NOTE: `ALTER TABLE ... ADD COLUMN` is not idempotent in SQLite, so
-- the Rust migration runner is responsible for ignoring
-- "duplicate column name" errors when re-running 002.

-- 1. Extend the existing `reflections` table with the importance
--    column that the v0.2 reflection engine uses to order output.
ALTER TABLE reflections ADD COLUMN importance REAL NOT NULL DEFAULT 0.5;

-- 2. Indexes used by the reflection list / search paths.
CREATE INDEX IF NOT EXISTS idx_reflections_created_at ON reflections(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_reflections_importance  ON reflections(importance DESC);

-- 3. Join table: many-to-many between `memories` and `reflections`.
--    A single reflection may cite several source memories, and a
--    single memory may participate in several reflections.
CREATE TABLE IF NOT EXISTS memory_reflections (
    memory_id     TEXT NOT NULL,
    reflection_id TEXT NOT NULL,
    created_at    INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (memory_id, reflection_id),
    FOREIGN KEY (memory_id)     REFERENCES memories(id)     ON DELETE CASCADE,
    FOREIGN KEY (reflection_id) REFERENCES reflections(id)  ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_reflections_reflection ON memory_reflections(reflection_id);
CREATE INDEX IF NOT EXISTS idx_memory_reflections_memory     ON memory_reflections(memory_id);
