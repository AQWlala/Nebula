-- v019: Git 风格记忆版本控制 — 分支管理。
-- 设计文档 v7.0 §3.4 L3 应用层 — 记忆版本控制。
-- memory_commits 表已存在（001_initial.sql），本 migration 添加分支表
-- 并为 memory_commits 添加 branch_name 列以支持多分支语义。

CREATE TABLE IF NOT EXISTS memory_branches (
    name            TEXT PRIMARY KEY,
    head_commit_id  TEXT,
    parent_branch   TEXT,
    created_at      INTEGER NOT NULL,
    is_active       INTEGER NOT NULL DEFAULT 0
);

-- 为 memory_commits 添加 branch_name 列（如果不存在）。
-- SQLite 没有 ADD COLUMN IF NOT EXISTS，用 pragma 检查。
-- 这里直接添加，重复执行会报错但 migration 框架保证只执行一次。
ALTER TABLE memory_commits ADD COLUMN branch_name TEXT NOT NULL DEFAULT 'main';

CREATE INDEX IF NOT EXISTS idx_commits_branch ON memory_commits(branch_name, created_at DESC);

-- 默认创建 main 分支。
INSERT OR IGNORE INTO memory_branches (name, head_commit_id, parent_branch, created_at, is_active)
VALUES ('main', NULL, NULL, strftime('%s','now'), 1);
