-- =============================================================================
-- v0.5 schema additions (004_v05.sql)
--
-- History:
--   * v0.5 introduced the `e2ee_keys` table together with a v0.5
--     end-to-end-encryption feature.  v0.5 ships an in-memory
--     `E2eeIdentity` only — the table was created in anticipation
--     of a v0.5.x follow-up that never landed.
--
-- v1.0 P0#9 fix (Approach A — "drop, redesign in v1.1"):
--   The v0.5 `e2ee_keys` table was never read or written by the
--   application code, which means it was dead weight on disk
--   (and a future source of "stale schema" confusion if a
--     v0.5.x build ever tries to consume it).  Per the P0 audit
--   decision, the table definition has been *removed* from this
--   migration and the migration `005_v10.sql` drops the table
--   on existing v0.5 databases (idempotent `DROP TABLE IF EXISTS`).
--
--   v1.1 is on the roadmap to re-introduce E2EE identity
--   persistence with a v1.1-shaped table (encrypted-at-rest
--   secret, version field, fingerprint, device label, etc.)
--   and the corresponding `E2eeIdentity::persist()` /
--   `E2eeIdentity::load()` methods.
-- =============================================================================

-- Work engine task table (v0.5).
-- Used by WorkEngine for task tracking, timer management, and
-- priority recommendation.
CREATE TABLE IF NOT EXISTS work_tasks (
    id           TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'todo'
                 CHECK(status IN ('todo','doing','done')),
    priority     INTEGER NOT NULL DEFAULT 0,
    due_at       INTEGER,
    time_spent_ms INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    completed_at INTEGER,
    metadata     TEXT
);
