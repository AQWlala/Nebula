-- nine-snake v7.0 memory system: FTS5 full-text search fallback
--
-- Per the detailed design document (v7.0 详细设计 v1.0), SQLite FTS5
-- serves as the primary fallback when LanceDB vector search is
-- unavailable.  LanceDB → FTS5 → LIKE, in that order.
--
-- FTS5 is a virtual table; we create an external-content FTS index
-- that mirrors the `memories` table without duplicating storage.

-- v1.1: create the FTS5 virtual table using content-sync mode so the
-- index stays in sync with the real `memories` table automatically.
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    content,
    summary_50,
    summary_150,
    summary_500,
    summary_2000,
    memory_type,
    layer,
    tags,
    content='memories',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

-- Populate the FTS index for any existing rows (idempotent).
-- INSERT INTO memories_fts(memories_fts) VALUES ('rebuild');

-- v1.1: triggers to keep the FTS index in sync automatically.
-- These fire on INSERT, UPDATE, and DELETE against the `memories` table
-- and keep `memories_fts` consistent without application code changes.

-- After INSERT: insert a matching row into the FTS index.
CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content, summary_50, summary_150, summary_500, summary_2000, memory_type, layer, tags)
    VALUES (new.rowid, new.content, new.summary_50, new.summary_150, new.summary_500, new.summary_2000, new.memory_type, new.layer, new.tags);
END;

-- After DELETE: remove the corresponding FTS row.
CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, summary_50, summary_150, summary_500, summary_2000, memory_type, layer, tags)
    VALUES ('delete', old.rowid, old.content, old.summary_50, old.summary_150, old.summary_500, old.summary_2000, old.memory_type, old.layer, old.tags);
END;

-- After UPDATE: update the FTS row to match.
CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, summary_50, summary_150, summary_500, summary_2000, memory_type, layer, tags)
    VALUES ('delete', old.rowid, old.content, old.summary_50, old.summary_150, old.summary_500, old.summary_2000, old.memory_type, old.layer, old.tags);
    INSERT INTO memories_fts(rowid, content, summary_50, summary_150, summary_500, summary_2000, memory_type, layer, tags)
    VALUES (new.rowid, new.content, new.summary_50, new.summary_150, new.summary_500, new.summary_2000, new.memory_type, new.layer, new.tags);
END;
