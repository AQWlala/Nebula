//! Skill auto-discovery — scans 4 layers for `SKILL.md` files.
//!
//! Layers (highest priority first):
//!
//! 1. **Project-level**: `<workspace>/.nebula/skills/`
//! 2. **User-level**: `~/.nebula/skills/`
//! 3. **System-level**: `/etc/nebula/skills/` (Linux) or
//!    `%ProgramData%\nebula\skills` (Windows)
//! 4. **Workspace-level**: `<workspace>/skills/`
//!
//! Each `SKILL.md` file uses YAML frontmatter to define skill metadata.
//!
//! ```markdown
//! ---
//! id: my-skill
//! name: My Skill
//! description: Does something useful
//! language: markdown
//! tags: ["utility", "test"]
//! permissions: ["file:read"]
//! ---
//!
//! # Instructions
//!
//! When the user asks to ..., do ...
//! ```

use std::path::{Path, PathBuf};

use anyhow::Context;
use tracing::{info, warn};

use crate::skills::types::{ActivationCondition, Skill};
use crate::skills::store::SkillStore;

/// Scans the 4-layer directory hierarchy for `SKILL.md` files and
/// registers any new skills into the [`SkillStore`].
pub struct SkillDiscoverer {
    scan_paths: Vec<PathBuf>,
}

impl SkillDiscoverer {
    /// Builds the discoverer with all existing scan paths.  Non-existent
    /// directories are silently skipped.
    pub fn new() -> Self {
        let mut paths = Vec::new();

        // Layer 4: workspace-level (`./skills/`)
        paths.push(PathBuf::from("skills"));

        // Layer 2: user-level (`~/.nebula/skills/`)
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            paths.push(PathBuf::from(home).join(".nebula").join("skills"));
        }

        // Layer 3: system-level
        #[cfg(target_os = "linux")]
        paths.push(PathBuf::from("/etc/nebula/skills"));

        #[cfg(target_os = "windows")]
        {
            if let Ok(prog_data) = std::env::var("ProgramData") {
                paths.push(PathBuf::from(prog_data).join("nebula").join("skills"));
            }
        }

        #[cfg(target_os = "macos")]
        paths.push(PathBuf::from("/Library/Application Support/nebula/skills"));

        // Layer 1: project-level (`<workspace>/.nebula/skills/`)
        // This is resolved relative to CWD at discovery time.
        paths.push(PathBuf::from(".nebula/skills"));

        // Retain only existing directories.
        paths.retain(|p| p.is_dir());
        Self { scan_paths: paths }
    }

    /// Returns the list of directories that will be scanned.
    pub fn scan_paths(&self) -> &[PathBuf] {
        &self.scan_paths
    }

    /// Walks all scan paths, parses `SKILL.md` files, and upserts new
    /// skills into the store.  Returns the number of skills discovered
    /// (not necessarily new — existing skills are updated in place).
    pub fn discover(&self, store: &SkillStore) -> anyhow::Result<usize> {
        let mut count = 0;
        for path in &self.scan_paths {
            match self.scan_directory(path, store) {
                Ok(n) => count += n,
                Err(e) => {
                    warn!(
                        target: "nebula.skills",
                        path = %path.display(),
                        error = %e,
                        "skill discovery failed for directory"
                    );
                }
            }
        }
        if count > 0 {
            info!(
                target: "nebula.skills",
                count,
                paths = self.scan_paths.len(),
                "skill auto-discovery complete"
            );
        }
        Ok(count)
    }

    /// Scans a single directory recursively for `SKILL.md` files.
    fn scan_directory(&self, dir: &Path, store: &SkillStore) -> anyhow::Result<usize> {
        let mut count = 0;
        for entry in walkdir::WalkDir::new(dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.file_name() != "SKILL.md" {
                continue;
            }

            match self.parse_and_register(entry.path(), store) {
                Ok(skill_id) => {
                    count += 1;
                    info!(
                        target: "nebula.skills",
                        skill_id = %skill_id,
                        path = %entry.path().display(),
                        "skill discovered"
                    );
                }
                Err(e) => {
                    warn!(
                        target: "nebula.skills",
                        path = %entry.path().display(),
                        error = %e,
                        "failed to parse SKILL.md"
                    );
                }
            }
        }
        Ok(count)
    }

    /// Parses a single `SKILL.md` file and upserts it into the store.
    fn parse_and_register(&self, path: &Path, store: &SkillStore) -> anyhow::Result<String> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading SKILL.md at {}", path.display()))?;

        let (frontmatter, body) = split_frontmatter(&content);
        let meta = parse_frontmatter(frontmatter)
            .with_context(|| format!("parsing frontmatter of {}", path.display()))?;

        let now = chrono::Utc::now().timestamp();
        let skill = Skill {
            id: meta.id,
            name: meta.name,
            description: meta.description,
            code: body.to_string(),
            language: meta.language.unwrap_or_else(|| "markdown".to_string()),
            tags: meta.tags.unwrap_or_default(),
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: now,
            updated_at: now,
            source_memory_id: None,
            activation_condition: meta
                .activation_keyword
                .map(|pattern| ActivationCondition::Keyword { pattern }),
            platform: meta.platform,
            min_confidence: meta.min_confidence,
            trust_level: 1, // Auto-discovered skills start at trust level 1 (user-confirmed)
            permissions: meta.permissions.unwrap_or_default(),
            capabilities: Default::default(),
        };

        let skill_id = skill.id.clone();

        // Upsert: insert if new, update if exists.
        store.upsert(&skill)?;

        Ok(skill_id)
    }
}

impl Default for SkillDiscoverer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SKILL.md frontmatter parsing
// ---------------------------------------------------------------------------

/// Parsed YAML frontmatter from a `SKILL.md` file.
#[derive(Debug, Default)]
struct SkillMeta {
    id: String,
    name: String,
    description: String,
    language: Option<String>,
    tags: Option<Vec<String>>,
    permissions: Option<Vec<String>>,
    platform: Option<Vec<String>>,
    min_confidence: Option<f32>,
    activation_keyword: Option<String>,
}

/// Splits a `SKILL.md` file into `(frontmatter, body)`.
/// If there is no frontmatter, returns `("", content)`.
fn split_frontmatter(content: &str) -> (&str, &str) {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return ("", content);
    }
    let after_first_fence = &content[4..];
    if let Some(end) = find_closing_fence(after_first_fence) {
        let frontmatter = &after_first_fence[..end];
        let body_start = end + 4; // skip closing "---\n"
        let body = after_first_fence.get(body_start..).unwrap_or("");
        (frontmatter, body.trim_start())
    } else {
        ("", content)
    }
}

/// Finds the position of the closing `---` fence.
fn find_closing_fence(s: &str) -> Option<usize> {
    s.find("\n---\n")
        .or_else(|| s.find("\n---\r\n"))
        .map(|p| p + 1) // +1 to skip the leading \n
}

/// Parses a minimal YAML subset for skill frontmatter.
///
/// Supported keys: `id`, `name`, `description`, `language`, `tags`,
/// `permissions`, `platform`, `min_confidence`, `activation_keyword`.
///
/// Lists are parsed as `["a", "b", "c"]` or `- a\n- b` style.
fn parse_frontmatter(yaml: &str) -> anyhow::Result<SkillMeta> {
    let mut meta = SkillMeta::default();
    let mut current_list_key: Option<String> = None;
    let mut current_list: Vec<String> = Vec::new();

    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Handle list items (- value)
        if trimmed.starts_with("- ") {
            if current_list_key.is_some() {
                let val = trimmed[2..].trim().trim_matches('"').to_string();
                current_list.push(val);
            }
            continue;
        }

        // If we were collecting a list, flush it
        if let Some(key) = current_list_key.take() {
            let list = std::mem::take(&mut current_list);
            set_list_field(&mut meta, &key, list);
        }

        // Handle key: value
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();

            // Check if value is empty (might be followed by list items)
            if value.is_empty() {
                current_list_key = Some(key.to_string());
                current_list.clear();
                continue;
            }

            // Parse inline list: ["a", "b"]
            if value.starts_with('[') && value.ends_with(']') {
                let inner = &value[1..value.len() - 1];
                let list: Vec<String> = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                set_list_field(&mut meta, key, list);
                continue;
            }

            // Parse scalar value
            let val_str = value.trim_matches('"').to_string();
            set_scalar_field(&mut meta, key, val_str);
        }
    }

    // Flush any remaining list
    if let Some(key) = current_list_key.take() {
        set_list_field(&mut meta, &key, current_list);
    }

    if meta.id.is_empty() {
        anyhow::bail!("SKILL.md frontmatter missing required field: id");
    }
    if meta.name.is_empty() {
        anyhow::bail!("SKILL.md frontmatter missing required field: name");
    }

    Ok(meta)
}

fn set_scalar_field(meta: &mut SkillMeta, key: &str, value: String) {
    match key {
        "id" => meta.id = value,
        "name" => meta.name = value,
        "description" => meta.description = value,
        "language" => meta.language = Some(value),
        "min_confidence" => {
            meta.min_confidence = value.parse().ok();
        }
        "activation_keyword" => meta.activation_keyword = Some(value),
        _ => {}
    }
}

fn set_list_field(meta: &mut SkillMeta, key: &str, list: Vec<String>) {
    match key {
        "tags" => meta.tags = Some(list),
        "permissions" => meta.permissions = Some(list),
        "platform" => meta.platform = Some(list),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter_with_meta() {
        let content = "---\nid: test-skill\nname: Test\n---\n\n# Body\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.contains("id: test-skill"));
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn test_split_frontmatter_no_meta() {
        let content = "# Just a body\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_empty());
        // 无 frontmatter 时返回原内容(含尾部换行符),trim 后比较
        assert_eq!(body.trim(), "# Just a body");
    }

    #[test]
    fn test_parse_frontmatter_required_fields() {
        let yaml = "id: my-skill\nname: My Skill\ndescription: A test\n";
        let meta = parse_frontmatter(yaml).unwrap();
        assert_eq!(meta.id, "my-skill");
        assert_eq!(meta.name, "My Skill");
        assert_eq!(meta.description, "A test");
    }

    #[test]
    fn test_parse_frontmatter_missing_id() {
        let yaml = "name: No ID\n";
        assert!(parse_frontmatter(yaml).is_err());
    }

    #[test]
    fn test_parse_frontmatter_inline_list() {
        let yaml = "id: s1\nname: S1\ntags: [\"a\", \"b\", \"c\"]\n";
        let meta = parse_frontmatter(yaml).unwrap();
        assert_eq!(meta.tags.unwrap(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_parse_frontmatter_yaml_list() {
        let yaml = "id: s1\nname: S1\ntags:\n  - alpha\n  - beta\n";
        let meta = parse_frontmatter(yaml).unwrap();
        assert_eq!(meta.tags.unwrap(), vec!["alpha", "beta"]);
    }

    #[test]
    fn test_split_frontmatter_with_bom() {
        let content = "\u{feff}---\nid: bom-test\nname: BOM\n---\nBody";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.contains("id: bom-test"));
        assert_eq!(body, "Body");
    }
}
