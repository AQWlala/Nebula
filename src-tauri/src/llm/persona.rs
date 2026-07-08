//! T-E-S-39: SOUL.md / AGENTS.md / TOOLS.md persona injection.
//!
//! Reads up to three Markdown files from the workspace root and
//! assembles them into an XML-tagged system-prompt prefix that is
//! prepended to every LLM system prompt (both the main chat path and
//! the swarm GenericAgent path).
//!
//! ## File semantics
//! * `SOUL.md`   — overall persona / values / tone.
//! * `AGENTS.md` — role behaviour rules.
//! * `TOOLS.md`  — tool-usage guidelines.
//!
//! All three files are optional; a missing file simply yields `None`
//! and is skipped in [`PersonaConfig::to_system_prefix`].
//!
//! ## Size guard
//! Each file is capped at 64 KiB (prevents a runaway SOUL.md from
//! consuming the entire context window). Oversized files are
//! truncated at the nearest UTF-8 char boundary.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Maximum size of a single persona file in bytes (64 KiB).
const MAX_FILE_SIZE: usize = 64 * 1024;

/// T-E-S-39: Loaded persona configuration.
///
/// Each field holds the raw Markdown content of the corresponding
/// workspace-root file, or `None` if the file does not exist / could
/// not be read.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaConfig {
    /// Content of `SOUL.md` — overall persona / values.
    pub soul_md: Option<String>,
    /// Content of `AGENTS.md` — role behaviour rules.
    pub agents_md: Option<String>,
    /// Content of `TOOLS.md` — tool-usage guidelines.
    pub tools_md: Option<String>,
}

impl PersonaConfig {
    /// Loads the three persona files from the workspace root in
    /// parallel.
    ///
    /// Missing files are silently treated as `None` (not an error).
    /// I/O errors other than "not found" are logged at `warn` level
    /// and the field is left as `None` so chat is never blocked by
    /// persona loading failures.
    pub async fn load(workspace_root: &Path) -> Result<Self> {
        let soul_path = workspace_root.join("SOUL.md");
        let agents_path = workspace_root.join("AGENTS.md");
        let tools_path = workspace_root.join("TOOLS.md");

        let (soul, agents, tools) = tokio::join!(
            read_persona_file(&soul_path),
            read_persona_file(&agents_path),
            read_persona_file(&tools_path),
        );

        Ok(Self {
            soul_md: soul,
            agents_md: agents,
            tools_md: tools,
        })
    }

    /// Renders the persona as an XML-tagged system-prompt prefix.
    ///
    /// Empty (`None`) fields are skipped entirely. If all three
    /// fields are `None` the result is an empty string.
    ///
    /// Output format (only non-empty sections appear):
    /// ```text
    /// <soul>
    /// {soul_md}
    /// </soul>
    /// <agents>
    /// {agents_md}
    /// </agents>
    /// <tools>
    /// {tools_md}
    /// </tools>
    /// ```
    pub fn to_system_prefix(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(soul) = &self.soul_md {
            parts.push(format!("<soul>\n{soul}\n</soul>"));
        }
        if let Some(agents) = &self.agents_md {
            parts.push(format!("<agents>\n{agents}\n</agents>"));
        }
        if let Some(tools) = &self.tools_md {
            parts.push(format!("<tools>\n{tools}\n</tools>"));
        }
        parts.join("\n")
    }

    /// Returns `true` when all three fields are `None`.
    pub fn is_empty(&self) -> bool {
        self.soul_md.is_none() && self.agents_md.is_none() && self.tools_md.is_none()
    }
}

/// Reads a single persona file, applying the 64 KiB size cap.
///
/// Returns `None` for "file not found" and for any other I/O error
/// (after logging a warning). Oversized files are truncated at the
/// nearest valid UTF-8 char boundary at or before `MAX_FILE_SIZE`
/// bytes.
async fn read_persona_file(path: &Path) -> Option<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            if content.len() > MAX_FILE_SIZE {
                warn!(
                    target: "nebula.persona",
                    path = %path.display(),
                    size = content.len(),
                    max = MAX_FILE_SIZE,
                    "persona file exceeds 64 KiB, truncating"
                );
                Some(truncate_to_char_boundary(&content, MAX_FILE_SIZE))
            } else {
                Some(content)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!(
                target: "nebula.persona",
                path = %path.display(),
                error = %e,
                "failed to read persona file; skipping"
            );
            None
        }
    }
}

/// Truncates `s` to at most `max_bytes` bytes, cutting at the last
/// valid UTF-8 char boundary at or before that offset.
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_system_prefix_includes_all_sections() {
        let p = PersonaConfig {
            soul_md: Some("be helpful".to_string()),
            agents_md: Some("be concise".to_string()),
            tools_md: Some("use shell".to_string()),
        };
        let prefix = p.to_system_prefix();
        assert!(prefix.contains("<soul>"));
        assert!(prefix.contains("be helpful"));
        assert!(prefix.contains("</soul>"));
        assert!(prefix.contains("<agents>"));
        assert!(prefix.contains("be concise"));
        assert!(prefix.contains("</agents>"));
        assert!(prefix.contains("<tools>"));
        assert!(prefix.contains("use shell"));
        assert!(prefix.contains("</tools>"));
    }

    #[test]
    fn to_system_prefix_empty_when_all_none() {
        let p = PersonaConfig::default();
        assert!(p.to_system_prefix().is_empty());
        assert!(p.is_empty());
    }

    #[test]
    fn to_system_prefix_skips_none_fields() {
        let p = PersonaConfig {
            soul_md: Some("soul-only".to_string()),
            agents_md: None,
            tools_md: None,
        };
        let prefix = p.to_system_prefix();
        assert!(prefix.contains("<soul>"));
        assert!(prefix.contains("soul-only"));
        assert!(!prefix.contains("<agents>"));
        assert!(!prefix.contains("<tools>"));
        assert!(!p.is_empty());
    }

    #[test]
    fn truncates_large_file_to_max_size() {
        // A string just over the 64 KiB limit (all ASCII so byte == char).
        let large = "x".repeat(MAX_FILE_SIZE + 200);
        let truncated = truncate_to_char_boundary(&large, MAX_FILE_SIZE);
        assert_eq!(truncated.len(), MAX_FILE_SIZE);
        assert!(truncated.chars().all(|c| c == 'x'));
    }

    #[test]
    fn truncate_respects_utf8_char_boundary() {
        // "中" is 3 bytes in UTF-8. Place one right at the boundary.
        let prefix = "a".repeat(MAX_FILE_SIZE - 1); // boundary - 1 bytes of ASCII
        let s = format!("{prefix}中b"); // inserting a 3-byte char at offset max-1
        let truncated = truncate_to_char_boundary(&s, MAX_FILE_SIZE);
        // The cut must not split the multibyte char.
        assert!(truncated.len() <= MAX_FILE_SIZE);
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn load_returns_empty_when_files_missing() {
        // A temp directory is guaranteed to exist but unlikely to
        // contain SOUL.md / AGENTS.md / TOOLS.md.
        let tmp = std::env::temp_dir();
        let p = PersonaConfig::load(&tmp).await.expect("get should succeed");
        assert!(p.is_empty());
        assert!(p.soul_md.is_none());
        assert!(p.agents_md.is_none());
        assert!(p.tools_md.is_none());
    }

    #[tokio::test]
    async fn load_reads_existing_files() {
        let dir = std::env::temp_dir().join(format!("nebula-persona-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create should succeed");
        std::fs::write(dir.join("SOUL.md"), "soul-content").expect("update should succeed");

        let p = PersonaConfig::load(&dir).await.expect("get should succeed");
        assert_eq!(p.soul_md.as_deref(), Some("soul-content"));
        assert!(p.agents_md.is_none());
        assert!(p.tools_md.is_none());
        assert!(!p.is_empty());

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
