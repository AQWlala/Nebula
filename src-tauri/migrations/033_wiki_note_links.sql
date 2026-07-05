-- T-E-B-05: 双向链接 [[]] 语法 — wiki_note_links 关联表。
--
-- 存储笔记之间的双向链接关系(source → target):
--   * source_id: 链接来源笔记(含 [[target]] 的笔记)
--   * target_id: 链接目标笔记(被 [[target]] 引用的笔记)
--
-- 设计要点:
--   * ON DELETE CASCADE — 删除笔记时自动清除关联行(外键级联)。
--   * PRIMARY KEY (source_id, target_id) — 同一对链接只存一次。
--   * idx_wiki_note_links_target — 加速 get_backlinks(target_id) 查询。
--   * 只插目标存在的链接(悬空链接忽略,不在本表记录)。

CREATE TABLE IF NOT EXISTS wiki_note_links (
    source_id TEXT NOT NULL REFERENCES wiki_notes(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES wiki_notes(id) ON DELETE CASCADE,
    PRIMARY KEY (source_id, target_id)
);

CREATE INDEX IF NOT EXISTS idx_wiki_note_links_target ON wiki_note_links(target_id);
