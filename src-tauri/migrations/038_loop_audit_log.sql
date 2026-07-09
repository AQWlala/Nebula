-- =============================================================================
-- T-E-L-07: Loop 审计日志 (038_loop_audit_log.sql)
--
-- 记录每次 Loop 执行（execute_loop）的关键节点：开始时间、结束时间、
-- 阶段、输入、输出、耗时、状态、错误。用于审计追溯与可观测性。
--
-- 表结构:
--   * loop_audit_log — 审计日志主表（每行 = 一个阶段事件）
--
-- 设计要点:
--   * run_id 用 UUID 标识一次 execute_loop 调用，同一次调用产生多条记录
--     （values_check / budget_check / homogeneity_check / task_start / loop_started
--      或 loop_denied / loop_needs_confirmation / loop_failed）
--   * task_id 软引用 long_tasks.id（denied / needs_confirmation 时为 NULL）
--   * started_at / finished_at 用毫秒时间戳（与 MasterEvent 一致）
--   * metadata_json 存储额外上下文（autonomy_level / values_verdict 等）
--   * 幂等性: CREATE TABLE IF NOT EXISTS / CREATE INDEX IF NOT EXISTS
-- =============================================================================

CREATE TABLE IF NOT EXISTS loop_audit_log (
    id                 TEXT PRIMARY KEY,
    run_id             TEXT NOT NULL,
    loop_name          TEXT NOT NULL,
    task_id            TEXT,
    phase              TEXT NOT NULL
                       CHECK(phase IN (
                           'values_check',
                           'budget_check',
                           'homogeneity_check',
                           'task_creation',
                           'task_start',
                           'loop_started',
                           'loop_denied',
                           'loop_needs_confirmation',
                           'loop_failed'
                       )),
    status             TEXT NOT NULL,
    started_at         INTEGER NOT NULL,
    finished_at        INTEGER,
    elapsed_ms         INTEGER,
    input_summary      TEXT,
    output_summary     TEXT,
    error_message      TEXT,
    autonomy_level     TEXT,
    budget_status      TEXT,
    autonomy_downgraded TEXT,
    values_verdict     TEXT,
    metadata_json      TEXT NOT NULL DEFAULT '{}'
);

-- 按 run_id 查询同一次调用的所有事件（时间线回放）。
CREATE INDEX IF NOT EXISTS idx_loop_audit_log_run_id ON loop_audit_log(run_id);
-- 按 loop_name 查询某个 Loop 的历史执行记录。
CREATE INDEX IF NOT EXISTS idx_loop_audit_log_loop_name ON loop_audit_log(loop_name);
-- 按时间范围查询（审计面板默认按时间倒序）。
CREATE INDEX IF NOT EXISTS idx_loop_audit_log_started_at ON loop_audit_log(started_at);
-- 按状态过滤（如只看 denied / failed）。
CREATE INDEX IF NOT EXISTS idx_loop_audit_log_status ON loop_audit_log(status);
