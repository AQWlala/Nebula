-- nebula v7.0 memory system initial schema
-- All memory-related tables; designed to support 8 layers (L0..L7)
-- and 5 memory types (Semantic, Episodic, Procedural, Emotional, Metacognitive).

PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 268435456; -- 256 MB

-- ---------------------------------------------------------------------------
-- memories: the primary memory table
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS memories (
    id              TEXT PRIMARY KEY,
    memory_type     TEXT NOT NULL,                 -- Semantic | Episodic | Procedural | Emotional | Metacognitive
    layer           TEXT NOT NULL,                 -- L0..L7
    content         TEXT NOT NULL,                 -- full raw content
    summary_50      TEXT NOT NULL DEFAULT '',      -- multi-granularity summaries
    summary_150     TEXT NOT NULL DEFAULT '',
    summary_500     TEXT NOT NULL DEFAULT '',
    summary_2000    TEXT NOT NULL DEFAULT '',
    importance      REAL NOT NULL DEFAULT 0.5,     -- 0.0..1.0
    access_count    INTEGER NOT NULL DEFAULT 0,
    last_access     INTEGER NOT NULL,              -- unix seconds
    created_at      INTEGER NOT NULL,              -- unix seconds
    source          TEXT NOT NULL DEFAULT 'user_input', -- user_input | agent_output | reflection | system
    metadata        TEXT NOT NULL DEFAULT '{}',    -- JSON blob for extensibility
    compressed_from TEXT,                          -- parent memory id if this is a compression result
    compression_gen INTEGER NOT NULL DEFAULT 0,    -- 0 = original, n = compressed n times
    pinned          INTEGER NOT NULL DEFAULT 0     -- 1 = never compress (L7 singularity)
);

CREATE INDEX IF NOT EXISTS idx_memories_layer        ON memories(layer);
CREATE INDEX IF NOT EXISTS idx_memories_type         ON memories(memory_type);
CREATE INDEX IF NOT EXISTS idx_memories_importance   ON memories(importance DESC);
CREATE INDEX IF NOT EXISTS idx_memories_last_access  ON memories(last_access DESC);
CREATE INDEX IF NOT EXISTS idx_memories_created      ON memories(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memories_pinned       ON memories(pinned);

-- ---------------------------------------------------------------------------
-- memory_relations: graph edges between memories (knowledge graph)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS memory_relations (
    id          TEXT PRIMARY KEY,
    src_id      TEXT NOT NULL,
    dst_id      TEXT NOT NULL,
    relation    TEXT NOT NULL,        -- causes | supports | contradicts | references | derived_from
    weight      REAL NOT NULL DEFAULT 1.0,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY(src_id) REFERENCES memories(id) ON DELETE CASCADE,
    FOREIGN KEY(dst_id) REFERENCES memories(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_relations_src ON memory_relations(src_id);
CREATE INDEX IF NOT EXISTS idx_relations_dst ON memory_relations(dst_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_relations_pair ON memory_relations(src_id, dst_id, relation);

-- ---------------------------------------------------------------------------
-- entities: extracted named entities (people, projects, concepts, files)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS entities (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    entity_type TEXT NOT NULL,            -- person | project | concept | file | tool | ...
    description TEXT NOT NULL DEFAULT '',
    metadata    TEXT NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_entities_name ON entities(name);
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type);

-- ---------------------------------------------------------------------------
-- entity_mentions: which memory mentions which entity
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS entity_mentions (
    memory_id  TEXT NOT NULL,
    entity_id  TEXT NOT NULL,
    count      INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(memory_id, entity_id),
    FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE,
    FOREIGN KEY(entity_id) REFERENCES entities(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_mentions_entity ON entity_mentions(entity_id);

-- ---------------------------------------------------------------------------
-- skills: procedural memory entries (reusable skills/workflows)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS skills (
    id            TEXT PRIMARY KEY,
    memory_id     TEXT NOT NULL,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    steps         TEXT NOT NULL DEFAULT '[]',   -- JSON array of step descriptions
    trigger       TEXT NOT NULL DEFAULT '',     -- natural-language trigger condition
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_used     INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_skills_name ON skills(name);
CREATE INDEX IF NOT EXISTS idx_skills_memory ON skills(memory_id);

-- ---------------------------------------------------------------------------
-- memory_commits: append-only event log (git-like) of memory mutations
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS memory_commits (
    id          TEXT PRIMARY KEY,
    parent_id   TEXT,                       -- previous commit id (chain)
    action      TEXT NOT NULL,              -- store | update | compress | merge | delete
    target_id   TEXT NOT NULL,              -- memory id affected
    payload     TEXT NOT NULL DEFAULT '{}', -- JSON describing the change
    author      TEXT NOT NULL DEFAULT 'system',
    message     TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_commits_target ON memory_commits(target_id);
CREATE INDEX IF NOT EXISTS idx_commits_created ON memory_commits(created_at DESC);

-- ---------------------------------------------------------------------------
-- reflections: metacognitive memory entries (self-evaluations)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS reflections (
    id            TEXT PRIMARY KEY,
    memory_id     TEXT,
    trigger_kind  TEXT NOT NULL,           -- task_complete | user_feedback | periodic | error
    content       TEXT NOT NULL,
    lessons       TEXT NOT NULL DEFAULT '[]', -- JSON array of lesson strings
    confidence    REAL NOT NULL DEFAULT 0.5,
    created_at    INTEGER NOT NULL,
    FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_reflections_created ON reflections(created_at DESC);

-- ---------------------------------------------------------------------------
-- sync_status: tracks sync state per memory (for future multi-device sync)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS sync_status (
    memory_id    TEXT PRIMARY KEY,
    sync_state   TEXT NOT NULL DEFAULT 'local', -- local | pending | synced | conflict
    device_id    TEXT NOT NULL DEFAULT 'self',
    last_sync    INTEGER NOT NULL DEFAULT 0,
    version      INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_sync_state ON sync_status(sync_state);

-- ---------------------------------------------------------------------------
-- schema_version: bookkeeping for future migrations
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  INTEGER NOT NULL,
    description TEXT NOT NULL DEFAULT ''
);

INSERT OR IGNORE INTO schema_version(version, applied_at, description)
VALUES (1, strftime('%s','now'), 'nebula v7.0 initial memory schema');
