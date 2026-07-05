-- T-E-B-01: LLM Wiki 编译引擎 — wiki_notes 表 + FTS5 全文索引。
--
-- 每次对话后 AI "编译" 结构化 Markdown 笔记写入 wiki/ 目录,
-- 元数据持久化到 wiki_notes 表,body 列缓存正文供 FTS5 全文检索。
--
-- 字段说明:
--   id          — UUID v4,主键
--   turn_id     — 关联对话 turn_id(幂等键,UNIQUE WHERE NOT NULL,
--                 允许多条 raw 笔记 turn_id 为 NULL)
--   title       — 笔记标题(LLM 生成,≤60 字)
--   slug        — 文件名安全 slug(小写中划线,UNIQUE)
--   tags_json   — 标签 JSON 数组(如 ["rust","tauri"])
--   path        — 相对 storage 路径 "wiki/{slug}.md"
--   body        — Markdown 正文(与文件系统内容同步,供 FTS5 索引)
--   created_at  — 创建时间(Unix 毫秒)
--   updated_at  — 更新时间(Unix 毫秒)
CREATE TABLE IF NOT EXISTS wiki_notes (
    id TEXT PRIMARY KEY,
    turn_id TEXT,
    title TEXT NOT NULL,
    slug TEXT NOT NULL,
    tags_json TEXT NOT NULL DEFAULT '[]',
    path TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- 幂等键:同 turn_id 不重复编译。
-- WHERE turn_id IS NOT NULL 允许多条 raw 笔记(turn_id 为 NULL)共存。
CREATE UNIQUE INDEX IF NOT EXISTS idx_wiki_notes_turn_id
    ON wiki_notes(turn_id) WHERE turn_id IS NOT NULL;

-- slug 唯一:文件名冲突防护(ensure_unique_slug 查询此索引)。
CREATE UNIQUE INDEX IF NOT EXISTS idx_wiki_notes_slug ON wiki_notes(slug);

-- 按创建时间倒序查询(list 命令分页)。
CREATE INDEX IF NOT EXISTS idx_wiki_notes_created_at ON wiki_notes(created_at DESC);

-- FTS5 全文索引(external content mode,引用 wiki_notes 表)。
-- title + body 两列索引;body 与文件系统 Markdown 内容由 WikiCompiler 同步。
-- 参考 010_fts5.sql memories_fts 的 content-sync 模式。
CREATE VIRTUAL TABLE IF NOT EXISTS wiki_notes_fts USING fts5(
    title, body, content='wiki_notes', content_rowid='rowid'
);

-- FTS5 同步触发器(参考 010_fts5.sql memories_fts 模式)。
-- INSERT:同步插入 FTS 索引。
CREATE TRIGGER IF NOT EXISTS wiki_notes_fts_ai AFTER INSERT ON wiki_notes BEGIN
    INSERT INTO wiki_notes_fts(rowid, title, body)
    VALUES (new.rowid, new.title, new.body);
END;

-- DELETE:从 FTS 索引删除。
CREATE TRIGGER IF NOT EXISTS wiki_notes_fts_ad AFTER DELETE ON wiki_notes BEGIN
    INSERT INTO wiki_notes_fts(wiki_notes_fts, rowid, title, body)
    VALUES ('delete', old.rowid, old.title, old.body);
END;

-- UPDATE:先删后插保持 FTS 索引一致。
CREATE TRIGGER IF NOT EXISTS wiki_notes_fts_au AFTER UPDATE ON wiki_notes BEGIN
    INSERT INTO wiki_notes_fts(wiki_notes_fts, rowid, title, body)
    VALUES ('delete', old.rowid, old.title, old.body);
    INSERT INTO wiki_notes_fts(rowid, title, body)
    VALUES (new.rowid, new.title, new.body);
END;
