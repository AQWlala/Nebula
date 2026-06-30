-- =====================================================================
-- 017: document_exports table
-- =====================================================================
-- The `export()` method in `writing/mod.rs` records each rendered
-- export (Markdown / HTML) in a `document_exports` table.  The table
-- was originally part of the v0.5 writing feature but was lost when
-- 004_v05.sql was reduced to comments during the v1.0 P0#9 cleanup
-- (the `e2ee_keys` removal).  Migration 006 only recreated the
-- `documents` table, not `document_exports`, so every `export()` call
-- failed with "no such table: document_exports".
--
-- This migration restores the table.  `IF NOT EXISTS` makes it safe
-- on databases that somehow already have it.

CREATE TABLE IF NOT EXISTS document_exports (
    id            TEXT PRIMARY KEY,
    document_id   TEXT NOT NULL,
    format        TEXT NOT NULL,
    byte_size     INTEGER NOT NULL DEFAULT 0,
    exported_at   INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_document_exports_doc ON document_exports(document_id);
