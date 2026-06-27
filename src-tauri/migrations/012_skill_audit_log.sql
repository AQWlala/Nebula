CREATE TABLE IF NOT EXISTS skill_audit_log (
    id                    TEXT PRIMARY KEY,
    skill_id              TEXT NOT NULL,
    executed_at           INTEGER NOT NULL,
    input_summary         TEXT NOT NULL DEFAULT '',
    output_summary        TEXT NOT NULL DEFAULT '',
    duration_ms           INTEGER NOT NULL DEFAULT 0,
    sandbox_type          TEXT NOT NULL DEFAULT 'python',
    security_scan_result  TEXT NOT NULL DEFAULT '',
    success               INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_audit_skill ON skill_audit_log(skill_id);
CREATE INDEX IF NOT EXISTS idx_audit_executed ON skill_audit_log(executed_at DESC);