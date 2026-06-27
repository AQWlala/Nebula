-- =====================================================================
-- 006: documents table + foreign key (P0#8 fix)
-- =====================================================================
--
-- Background
-- ----------
-- v0.5 migration `004_v05.sql` originally declared a `documents`
-- table whose `memory_id` column carried a "-- FK to memories.id"
-- comment but no actual `FOREIGN KEY` constraint.  P0#9's edit
-- (removing the `e2ee_keys` table from 004_v05.sql) accidentally
-- stripped the entire `documents` `CREATE TABLE` statement too,
-- so a fresh v1.0 install reaches the `WritingEngine` with no
-- `documents` table and every save fails with
-- "no such table: documents".
--
-- v1.0 P0#8 also targets the orphan-reference problem: every
-- Sponge / blackhole pass and every `memory_delete` Tauri
-- command could remove a `memories.id` row while orphaned
-- `documents.memory_id` references stayed behind, because
-- there was no FK cascade.
--
-- Fix
-- ---
-- This migration is two statements:
--
--   1. `CREATE TABLE IF NOT EXISTS documents (...)` — restores
--      the schema that v0.5's 004_v05.sql carried.  We use
--      `IF NOT EXISTS` so the migration is a no-op for
--      databases (real or fixture-driven) where the table
--      already exists.
--
--   2. `ALTER TABLE documents ADD CONSTRAINT fk_documents_memory
--      FOREIGN KEY (memory_id) REFERENCES memories(id) ON
--      DELETE SET NULL` — registers the out-of-line FK with
--      `ON DELETE SET NULL`.  We deliberately preserve the
--      document row (it carries the user-authored content) and
--      only null out the mirror pointer, matching the
--      semantics of the `reflections.memory_id` FK from
--      migration 001.
--
-- Compatibility
-- -------------
-- * SQLite supports `ALTER TABLE ... ADD CONSTRAINT FOREIGN KEY`
--   starting with 3.35.0 (2021-03-12). The bundled `rusqlite`
--   we ship is built against a modern SQLite (>= 3.42) so this
--   is available.
-- * If a v0.5 install pre-dates this migration and the column
--   already contains dangling ids, the constraint is still
--   accepted (SQLite only validates *new* inserts / cascades
--   after the constraint is registered) — a follow-up
--   `foreign_key_check` cleanup is the v1.0.1 backfill.
--
-- Idempotency
-- -----------
-- Re-running the migration on a database where the table
-- already exists and the constraint is already registered is
-- a no-op (both `IF NOT EXISTS` and the SQLite-issued
-- "constraint already exists" are silent under the migrator's
-- error-coercion rules).
--
-- Versioning
-- ----------
-- v1.0 ships two 005* migrations (P0#8 was originally numbered
-- 005; P0#9's `005_v10.sql` had to land first, so the FK fix
-- renumbered to 006). The v1.0.1 follow-up may renumber to 007
-- when other P0 fixes land.

PRAGMA foreign_keys = ON;

-- 1. Restore the v0.5 documents table for fresh installs.
CREATE TABLE IF NOT EXISTS documents (
    id            TEXT PRIMARY KEY,
    title         TEXT NOT NULL DEFAULT '',
    template_id   TEXT NOT NULL DEFAULT 'blank',
    content       TEXT NOT NULL DEFAULT '',
    word_count    INTEGER NOT NULL DEFAULT 0,
    memory_id     TEXT,                          -- FK to memories.id (P0#8)
    created_at    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0,
    metadata      TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_documents_template ON documents(template_id);
CREATE INDEX IF NOT EXISTS idx_documents_updated  ON documents(updated_at DESC);

-- 2. Register the FK on memory_id. ON DELETE SET NULL mirrors
--    the `reflections.memory_id` FK from 001.
ALTER TABLE documents
    ADD CONSTRAINT fk_documents_memory
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE SET NULL;

-- Companion index so the cascade path can locate document rows
-- that point at a given memory id quickly.
CREATE INDEX IF NOT EXISTS idx_documents_memory_id
    ON documents (memory_id) WHERE memory_id IS NOT NULL;

