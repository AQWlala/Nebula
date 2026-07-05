-- nebula v0.3 migration 003 — skill CRUD support.
--
-- The v0.1 `skills` table was reserved (no rows ever written) but
-- lacked the columns the v0.3 engine needs. This migration:
--   * extends `skills` with code/language/tags/usage/avg_rating/etc.
--   * adds an index on `language` for the marketplace filter.
--   * adds a `skill_ratings` join table for raw rating history.
--   * records this migration in `schema_version`.
--
-- `ALTER TABLE ... ADD COLUMN` is not idempotent in SQLite; the Rust
-- migration runner already ignores "duplicate column name" errors.

-- 1. Extend `skills` with v0.3 columns. Sensible defaults so existing
--    rows (if any pre-existed) remain valid.
ALTER TABLE skills ADD COLUMN code           TEXT NOT NULL DEFAULT '';
ALTER TABLE skills ADD COLUMN language       TEXT NOT NULL DEFAULT 'rust';
ALTER TABLE skills ADD COLUMN tags           TEXT NOT NULL DEFAULT '[]';  -- JSON array
ALTER TABLE skills ADD COLUMN usage_count    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN avg_rating     REAL NOT NULL DEFAULT 0.0;
ALTER TABLE skills ADD COLUMN rating_count   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN updated_at     INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN source_memory_id TEXT;

-- 2. Indexes.
CREATE INDEX IF NOT EXISTS idx_skills_language ON skills(language);
CREATE INDEX IF NOT EXISTS idx_skills_usage    ON skills(usage_count DESC);
CREATE INDEX IF NOT EXISTS idx_skills_avg      ON skills(avg_rating DESC);

-- 3. Raw rating history. The denormalised `avg_rating`/`rating_count`
--    on the parent row is updated transactionally; this table keeps
--    the per-rating trail for analytics.
CREATE TABLE IF NOT EXISTS skill_ratings (
    skill_id  TEXT NOT NULL,
    rating    REAL NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (skill_id, created_at),
    FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_skill_ratings_skill ON skill_ratings(skill_id);

-- 4. Bookkeeping.
INSERT OR IGNORE INTO schema_version(version, applied_at, description)
VALUES (3, strftime('%s','now'), 'nebula v0.3 skill CRUD support');
