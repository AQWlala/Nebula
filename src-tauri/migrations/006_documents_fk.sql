-- =====================================================================
-- 006: documents table + foreign key (P0#8 fix)
-- =====================================================================
-- SQLite does not support ALTER TABLE ... ADD CONSTRAINT.
-- We recreate the table with the FK using the standard pattern:
--   create new table with FK -> copy data -> drop old -> rename new.
-- The migrator runs this in a single transaction so it is atomic.

-- 1. Ensure the table exists (for fresh installs where a previous
--    migration version may not have created it).
CREATE TABLE IF NOT EXISTS documents (
    id            TEXT PRIMARY KEY,
    title         TEXT NOT NULL DEFAULT '',
    template_id   TEXT NOT NULL DEFAULT 'blank',
    content       TEXT NOT NULL DEFAULT '',
    word_count    INTEGER NOT NULL DEFAULT 0,
    memory_id     TEXT,
    created_at    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0,
    metadata      TEXT NOT NULL DEFAULT '{}'
);

-- 2. Recreate with foreign key constraint.
PRAGMA foreign_keys = OFF;
BEGIN TRANSACTION;

CREATE TABLE documents_new (
    id            TEXT PRIMARY KEY,
    title         TEXT NOT NULL DEFAULT '',
    template_id   TEXT NOT NULL DEFAULT 'blank',
    content       TEXT NOT NULL DEFAULT '',
    word_count    INTEGER NOT NULL DEFAULT 0,
    memory_id     TEXT,
    created_at    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0,
    metadata      TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE SET NULL
);

INSERT INTO documents_new
    SELECT id, title, template_id, content, word_count, memory_id,
           created_at, updated_at, metadata
    FROM documents;

DROP TABLE documents;
ALTER TABLE documents_new RENAME TO documents;

COMMIT;
PRAGMA foreign_keys = ON;

-- 3. Indexes
CREATE INDEX IF NOT EXISTS idx_documents_template ON documents(template_id);
CREATE INDEX IF NOT EXISTS idx_documents_updated  ON documents(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_documents_memory_id
    ON documents (memory_id) WHERE memory_id IS NOT NULL;
