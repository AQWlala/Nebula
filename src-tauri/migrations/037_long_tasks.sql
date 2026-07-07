-- =============================================================================
-- T-E-C-10: 异步长任务模式 (037_long_tasks.sql)
--
-- 长任务允许 Agent 在后台分步执行跨小时/跨天的复杂任务,与 PlanEngine
-- 联动(可选)并在 Shadow Workspace 中隔离执行(可选)。状态持久化到
-- SQLite,进程重启后可恢复。
--
-- 表结构:
--   * long_tasks       — 任务主表(目标/状态/进度/关联)
--   * long_task_steps  — 步骤序列(每步一个命令执行)
--
-- 状态机:
--   任务: pending → running ⇄ paused → completed | failed | cancelled
--   步骤: pending → running → done | failed | skipped
--
-- 关联:
--   * workspace_id → shadow_workspaces.id (可选,隔离执行环境)
--   * plan_id      → plan_requests.id      (可选,PlanEngine 联动)
--   (注:不添加 FK 约束,因为 shadow_workspaces 是内存态无表,plan_requests
--    同样是内存态。关联仅作软引用。)
--
-- 幂等性: CREATE TABLE IF NOT EXISTS / CREATE INDEX IF NOT EXISTS。
-- =============================================================================

CREATE TABLE IF NOT EXISTS long_tasks (
    id           TEXT PRIMARY KEY,
    goal         TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending'
                 CHECK(status IN ('pending','running','paused','completed','failed','cancelled')),
    workspace_id TEXT,
    plan_id      TEXT,
    progress     INTEGER NOT NULL DEFAULT 0
                 CHECK(progress >= 0 AND progress <= 100),
    error        TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    started_at   INTEGER,
    finished_at  INTEGER
);

CREATE TABLE IF NOT EXISTS long_task_steps (
    task_id     TEXT NOT NULL,
    seq         INTEGER NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    program     TEXT NOT NULL,
    args_json   TEXT NOT NULL DEFAULT '[]',
    status      TEXT NOT NULL DEFAULT 'pending'
                CHECK(status IN ('pending','running','done','failed','skipped')),
    started_at  INTEGER,
    finished_at INTEGER,
    exit_code   INTEGER,
    output      TEXT,
    error       TEXT,
    PRIMARY KEY (task_id, seq)
);

-- 按状态过滤的高频查询索引(列表页常用)。
CREATE INDEX IF NOT EXISTS idx_long_tasks_status ON long_tasks(status);
-- 步骤按任务 + seq 顺序读取(回放时间线)。
CREATE INDEX IF NOT EXISTS idx_long_task_steps_task ON long_task_steps(task_id, seq);
