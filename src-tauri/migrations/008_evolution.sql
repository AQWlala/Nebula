-- nine-snake v1.3 migration 008 — task_outcomes + prompt_snapshots.
--
-- Two tables supporting `nine_snake::evolution::*` (v1.3 self-evolution).
--
-- * `task_outcomes`: per-task outcome rows recorded by the three
--   `outcome_collectors::*` hooks (skill, swarm, chat).  Skill
--   archive policy + PromptSelfMutator both read from here.
-- * `prompt_snapshots`: every time an agent's `system_prompt` is
--   about to be rewritten, we capture the previous value here.  The
--   `restored_to_id` column is set when the user calls
--   `evolution_restore_snapshot`.
--
-- Both tables use additive CREATE TABLE IF NOT EXISTS so the
-- migration is safe to re-run and to back-out.

CREATE TABLE IF NOT EXISTS task_outcomes (
    id           TEXT PRIMARY KEY,
    source_id    TEXT NOT NULL,           -- skill_id / task_agent_id / conv_id
    source       TEXT NOT NULL,           -- 'skill' | 'swarm' | 'chat' | 'other'
    status       TEXT NOT NULL,           -- 'success' | 'fail' | 'timeout' | 'aborted' | 'cancelled'
    confidence   REAL NOT NULL DEFAULT 0.0,
    error        TEXT NOT NULL DEFAULT '',
    duration_ms  INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_outcomes_source ON task_outcomes(source, source_id, created_at);
CREATE INDEX IF NOT EXISTS idx_task_outcomes_recent   ON task_outcomes(created_at);

CREATE TABLE IF NOT EXISTS prompt_snapshots (
    id             TEXT PRIMARY KEY,
    target         TEXT NOT NULL,                  -- agent name
    prev_prompt    TEXT NOT NULL,
    replaced_at    INTEGER NOT NULL,
    reason         TEXT,
    restored_to_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_prompt_snapshots_target ON prompt_snapshots(target, replaced_at);
CREATE INDEX IF NOT EXISTS idx_prompt_snapshots_unrestored ON prompt_snapshots(target) WHERE restored_to_id IS NULL;

-- Version bookkeeping lives in PRAGMA user_version — managed by
-- memory::migration::run_migrations.  We deliberately do NOT touch
-- the legacy `schema_version` table from this migration to avoid
-- running two version-control systems side by side.
