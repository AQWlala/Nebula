CREATE TABLE IF NOT EXISTS memory_acl (
    id          TEXT PRIMARY KEY,
    principal   TEXT NOT NULL,
    resource    TEXT NOT NULL,
    permission  TEXT NOT NULL,
    effect      TEXT NOT NULL DEFAULT 'allow',
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_acl_principal ON memory_acl(principal);
CREATE INDEX IF NOT EXISTS idx_acl_resource ON memory_acl(resource);