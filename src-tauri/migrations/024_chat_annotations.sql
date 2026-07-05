-- T-E-S-28: 对话消息标注(good/bad)+ Dify 风格数据集导出。
--
-- 每条标注关联一个 turn_id(由 commands/chat.rs::ChatComplete 注入的
-- UUID v4)。UNIQUE(turn_id) 保证同一 turn 的标注幂等 upsert ——
-- 用户可以反复点击 👍/👎 切换状态,后端只保留最新一条。
--
-- 设计参考 003_skills.sql::skill_ratings 的 rating history 表风格:
--   * created_at INTEGER(Unix 时间戳,毫秒)
--   * CHECK 约束限制 annotation 取值
--   * 索引按 created_at DESC + annotation 分桶,便于 stats 聚合查询
CREATE TABLE IF NOT EXISTS chat_annotations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id TEXT NOT NULL,
    annotation TEXT NOT NULL CHECK(annotation IN ('good', 'bad')),
    comment TEXT,
    agent_role TEXT,
    model TEXT,
    conversation_id TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE(turn_id)
);

CREATE INDEX IF NOT EXISTS idx_annotations_created ON chat_annotations(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_annotations_annotation ON chat_annotations(annotation);
