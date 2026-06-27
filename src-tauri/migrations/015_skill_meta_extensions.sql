-- v015: SkillMeta extensions — activation_condition, platform, min_confidence.
ALTER TABLE skills ADD COLUMN activation_condition TEXT;
ALTER TABLE skills ADD COLUMN platform TEXT;
ALTER TABLE skills ADD COLUMN min_confidence REAL;