-- nebula migration 018 — fix skill_ratings PK collision.
--
-- The original PK (skill_id, created_at) caused INSERT failures when
-- two ratings for the same skill landed in the same second (and later,
-- even in the same millisecond).  This is fatal for the `rate()` path
-- because the test suite — and real users — can rate twice in quick
-- succession.
--
-- Fix: replace the composite PK with an auto-increment `id` column so
-- the INSERT in rate() can never fail on a uniqueness constraint.
-- `created_at` is retained as a regular indexed column for analytics.
--
-- SQLite cannot ALTER a PRIMARY KEY in place, so we use the standard
-- "create new, copy, drop old, rename" pattern.

CREATE TABLE IF NOT EXISTS skill_ratings_new (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    skill_id   TEXT NOT NULL,
    rating     REAL NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (skill_id) REFERENCES skills(id) ON DELETE CASCADE
);

INSERT INTO skill_ratings_new (skill_id, rating, created_at)
SELECT skill_id, rating, created_at FROM skill_ratings;

DROP TABLE skill_ratings;

ALTER TABLE skill_ratings_new RENAME TO skill_ratings;

CREATE INDEX IF NOT EXISTS idx_skill_ratings_skill ON skill_ratings(skill_id);

INSERT OR IGNORE INTO schema_version(version, applied_at, description)
VALUES (18, strftime('%s','now'), 'fix skill_ratings PK collision with autoincrement id');
