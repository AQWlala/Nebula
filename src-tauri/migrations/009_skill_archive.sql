-- nine-snake v1.3 migration 009 — skill_archive.
--
-- `skill_evolver::SqliteSkillAutoEvolver` writes here when a skill
-- crosses the archive threshold.  Original rows in `skills` are
-- untouched — this is a soft archive that the user can undo via
-- `evolution_restore_archived`.
--
-- DECISION: deliberately did NOT add `archived` column to `skills`
-- because v0.3 ships no bulk-recovery story for partial-state rows
-- and because our reader paths (`SkillStore::list`) filter by
-- `language` / `tag` only — keeping the table single-source lets
-- callers audit "what was archived when and why" without joining.

CREATE TABLE IF NOT EXISTS skill_archive (
    skill_id      TEXT PRIMARY KEY,
    skill_name    TEXT NOT NULL,
    usage_count   INTEGER NOT NULL,
    avg_rating    REAL NOT NULL,
    archived_at   INTEGER NOT NULL,
    reason        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_skill_archive_archived_at ON skill_archive(archived_at);

-- See 008_evolution.sql comment for the schema_version policy: version
-- bookkeeping is owned by PRAGMA user_version, not by this table.
