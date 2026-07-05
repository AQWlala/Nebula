-- T-E-D-01: SemanticCache 响应正文持久化(重启后可恢复)。
--
-- 背景:
--   SemanticCache 在进程内用 `Mutex<HashMap<query_hash, CacheEntry>>` 保存
--   响应正文 + 插入时刻。LanceDB 只存向量(用于语义近邻检索),进程
--   重启后 entries map 清空,LanceDB 命中但本地映射缺失 → 误判为 miss。
--
--   本表把 (query_hash, response) 持久化到 SQLite,重启后由
--   `SemanticCache::prewarm_from_store` 读取最近 256 条重建 entries map。
--   `check()` 在 entries map miss 时也会回退查本表,避免单次冷启动漏命中。
--
-- 字段说明:
--   query_hash   — stable_id(query) 的 hex 字符串(形如 "sem:0123abcd..."),
--                   主键,与 LanceDB 的 id 一致
--   response     — 缓存的 LLM 响应正文
--   inserted_at  — Unix 时间戳(秒),用于按时间倒序预热 + TTL 过期(可选)
--
-- 幂等:`CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS`,
-- 重复执行不报错(已被 migration runner 的 is_idempotent_error 兜底)。
CREATE TABLE IF NOT EXISTS semantic_cache_entries (
    query_hash   TEXT PRIMARY KEY,
    response     TEXT NOT NULL,
    inserted_at  INTEGER NOT NULL
);

-- 按插入时间倒序索引:prewarm_from_store 用 ORDER BY inserted_at DESC LIMIT 256
-- 读取最近热点。索引覆盖 ORDER BY,无需排序扫描。
CREATE INDEX IF NOT EXISTS idx_semantic_cache_inserted
    ON semantic_cache_entries(inserted_at);
