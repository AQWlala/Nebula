-- v016: memory_versions table for sync conflict resolution.
CREATE TABLE IF NOT EXISTS memory_versions (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    version INTEGER NOT NULL DEFAULT 1,
    device_id TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL DEFAULT 0,
    content_hash TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (memory_id, version)
);

CREATE INDEX IF NOT EXISTS idx_memory_versions_memory_id ON memory_versions(memory_id);