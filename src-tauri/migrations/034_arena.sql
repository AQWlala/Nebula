-- T-E-A-14: Arena A/B 测试 — arena_matches + model_elo_scores 表。
--
-- 背景:
--   Nebula缺少 LLM 模型评测组件。Arena 通过相同 prompt 让两个模型
--   并行生成响应,自动评分或人工投票决定 winner,基于胜负更新
--   ELO 评分(K=32, 初始 1200),为模型选型提供量化依据。
--
--   持久化模式参考 cost_records(027_cost_source.sql),CREATE TABLE
--   IF NOT EXISTS + ALTER TABLE 重复应用由 migration runner 的
--   is_idempotent_error 兜底。
--
-- arena_matches 表:单场对战记录。
--   id            — UUID v4,主键
--   prompt        — 对战 prompt(两模型共用)
--   model_a/b     — 参赛模型名(如 "deepseek-chat" / "qwen2.5:7b")
--   response_a/b  — 各自响应正文(若 gateway 不可用为 NULL)
--   winner        — "a" / "b" / "tie"(NULL 表示未判定)
--   auto_score_a/b — 自动评分(0.0-1.0,NULL 表示未自动评分)
--   created_at    — Unix 毫秒时间戳
CREATE TABLE IF NOT EXISTS arena_matches (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    model_a TEXT NOT NULL,
    model_b TEXT NOT NULL,
    response_a TEXT,
    response_b TEXT,
    winner TEXT,
    auto_score_a REAL,
    auto_score_b REAL,
    created_at INTEGER NOT NULL
);

-- 按模型组合查询索引(同两模型历史对战记录)。
CREATE INDEX IF NOT EXISTS idx_arena_matches_models ON arena_matches(model_a, model_b);

-- model_elo_scores 表:模型 ELO 评分累积。
--   model          — 模型名,主键
--   elo            — 当前 ELO(默认 1200,与 new() 中 1200.0 对齐)
--   matches_played — 累计对战次数
--   updated_at     — 最近更新时间(Unix 毫秒)
CREATE TABLE IF NOT EXISTS model_elo_scores (
    model TEXT PRIMARY KEY,
    elo REAL NOT NULL DEFAULT 1200,
    matches_played INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
