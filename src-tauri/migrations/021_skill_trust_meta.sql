-- T-S3-A-01: agentskills.io SkillMeta 补全
-- 新增 trust_level / permissions / capabilities 三字段
ALTER TABLE skills ADD COLUMN trust_level INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN permissions TEXT NOT NULL DEFAULT '[]';
ALTER TABLE skills ADD COLUMN capabilities TEXT NOT NULL DEFAULT '{}';
