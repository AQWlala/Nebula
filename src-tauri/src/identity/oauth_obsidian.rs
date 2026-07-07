//! Obsidian Vault adapter — bidirectional sync with a local Obsidian vault.
//!
//! Unlike the OAuth providers, this adapter reads/writes `.md` files
//! directly on disk (Obsidian vaults are plain folders).  It detects
//! changes via polling the file mtime (a real notify watcher is wired
//! in `crate::memory::file_watcher` and can be layered on top).
//!
//! 对标: OpenHuman — Memory Tree → Obsidian Vault 双向同步。
//! 复用已有 wiki 模块和 FileWatcher。
//!
//! 同步流程:
//! 1. 拉取(pull):扫描 `vault_path` 下所有 `.md` 文件,解析 frontmatter
//!    + markdown + wikilinks,返回 [`VaultPage`] 列表供记忆管线入库。
//! 2. 推送(push):把 Nebula L3 compiled knowledge 写入 Vault `.md` 文件,
//!    保留 frontmatter,追加到 `## Nebula` section。
//!
//! 设计约束:文件是人类可读的 .md 格式,不破坏 Obsidian 的索引。

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{info, warn};

/// Obsidian Vault adapter — reads/writes `.md` files in a local vault.
pub struct ObsidianVaultAdapter {
    vault_path: PathBuf,
    /// Max file size to read (default 256 KB) — larger files are skipped.
    max_file_bytes: u64,
}

impl ObsidianVaultAdapter {
    /// Creates a new adapter pointing at the given vault root.
    pub fn new(vault_path: PathBuf) -> Self {
        Self {
            vault_path,
            max_file_bytes: 256 * 1024,
        }
    }

    /// Returns the vault root path.
    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    /// Sets the max file size (in bytes) to read.
    pub fn with_max_file_bytes(mut self, bytes: u64) -> Self {
        self.max_file_bytes = bytes;
        self
    }

    /// Pulls all `.md` files from the vault, returning parsed pages.
    ///
    /// Walks the vault recursively, skipping `.obsidian/`, `.trash/`,
    /// and any file larger than `max_file_bytes`.
    pub async fn pull_all(&self) -> Result<Vec<VaultPage>> {
        if !self.vault_path.is_dir() {
            anyhow::bail!(
                "Obsidian vault path does not exist or is not a directory: {}",
                self.vault_path.display()
            );
        }
        let mut pages = Vec::new();
        self.walk_dir(&self.vault_path.clone(), &mut pages).await?;
        info!(
            target: "nebula.oauth.obsidian",
            vault = %self.vault_path.display(),
            count = pages.len(),
            "pull_all complete"
        );
        Ok(pages)
    }

    /// Recursive directory walker.
    async fn walk_dir(&self, dir: &Path, pages: &mut Vec<VaultPage>) -> Result<()> {
        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip Obsidian internal dirs and trash.
            if path.is_dir() {
                if name_str.starts_with('.') && name_str != ".obsidian" {
                    continue;
                }
                if name_str == ".obsidian" || name_str == ".trash" || name_str == "node_modules" {
                    continue;
                }
                Box::pin(self.walk_dir(&path, pages)).await?;
                continue;
            }

            if !path.is_file() || name_str.len() < 3 || !name_str.ends_with(".md") {
                continue;
            }

            // Skip oversized files.
            if let Ok(meta) = entry.metadata().await {
                if meta.len() > self.max_file_bytes {
                    warn!(
                        target: "nebula.oauth.obsidian",
                        path = %path.display(),
                        size = meta.len(),
                        "skipping oversized file"
                    );
                    continue;
                }
            }

            match self.parse_page(&path).await {
                Ok(page) => pages.push(page),
                Err(e) => {
                    warn!(
                        target: "nebula.oauth.obsidian",
                        path = %path.display(),
                        error = %e,
                        "failed to parse page"
                    );
                }
            }
        }
        Ok(())
    }

    /// Parses a single `.md` file into a [`VaultPage`].
    async fn parse_page(&self, path: &Path) -> Result<VaultPage> {
        let raw = fs::read_to_string(path).await?;
        let rel_path = path
            .strip_prefix(&self.vault_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let (frontmatter, body) = split_frontmatter(&raw);
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| rel_path.clone());
        let wikilinks = extract_wikilinks(&body);

        Ok(VaultPage {
            path: rel_path,
            title,
            frontmatter,
            body,
            wikilinks,
        })
    }

    /// Pushes a Nebula knowledge update to a vault page.
    ///
    /// Writes/updates `<vault>/<relative_path>`, preserving any existing
    /// frontmatter and appending the update under a `## Nebula` section.
    pub async fn push_update(
        &self,
        relative_path: &str,
        update: &str,
    ) -> Result<PathBuf> {
        let target = self.vault_path.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }

        let existing = fs::read_to_string(&target).await.unwrap_or_default();
        let (frontmatter, body) = split_frontmatter(&existing);

        let new_body = append_nebula_section(&body, update);
        let mut output = String::new();
        if !frontmatter.is_empty() {
            output.push_str("---\n");
            output.push_str(&frontmatter);
            output.push_str("\n---\n\n");
        }
        output.push_str(&new_body);

        fs::write(&target, output).await?;
        info!(
            target: "nebula.oauth.obsidian",
            path = %target.display(),
            "push_update complete"
        );
        Ok(target)
    }
}

/// A single parsed vault page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPage {
    /// Path relative to vault root, using `/` separators.
    pub path: String,
    pub title: String,
    /// Raw frontmatter YAML (without the `---` fences), empty if none.
    pub frontmatter: String,
    /// Markdown body (without frontmatter).
    pub body: String,
    /// Wikilink targets extracted from the body (`[[Target]]`).
    pub wikilinks: Vec<String>,
}

/// Splits a markdown file into (frontmatter, body).
fn split_frontmatter(raw: &str) -> (String, String) {
    if !raw.starts_with("---") {
        return (String::new(), raw.to_string());
    }
    // Find the closing `---` fence.
    let after_first_fence = &raw[3..];
    if let Some(end) = after_first_fence.find("\n---") {
        let frontmatter = after_first_fence[..end].trim().to_string();
        let body_start = 3 + end + 4; // 3 for opening, +4 for `\n---`
        let body = if body_start >= raw.len() {
            String::new()
        } else {
            raw[body_start..].trim_start_matches('\n').to_string()
        };
        (frontmatter, body)
    } else {
        (String::new(), raw.to_string())
    }
}

/// Extracts `[[ wikilink ]]` targets from markdown body.
fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' && chars.peek() == Some(&'[') {
            chars.next();
            let mut target = String::new();
            let mut closed = false;
            for inner in chars.by_ref() {
                if inner == ']' {
                    if chars.peek() == Some(&']') {
                        chars.next();
                        closed = true;
                    }
                    break;
                }
                target.push(inner);
            }
            if closed && !target.is_empty() {
                // Wikilinks can have aliases: `[[Target|Alias]]`
                let target = target.split('|').next().unwrap_or("").trim();
                if !target.is_empty() {
                    links.push(target.to_string());
                }
            }
        }
    }
    links
}

/// Appends a Nebula update under a `## Nebula` section, creating the
/// section if it doesn't exist.
fn append_nebula_section(body: &str, update: &str) -> String {
    let section_marker = "## Nebula";
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let entry = format!("\n### {timestamp}\n\n{update}\n");

    if let Some(pos) = body.find(section_marker) {
        // Insert after the section header.
        let mut out = body[..pos].to_string();
        out.push_str(section_marker);
        out.push('\n');
        out.push_str(&entry);
        out.push('\n');
        out.push_str(&body[pos + section_marker.len()..]);
        out
    } else {
        let mut out = body.to_string();
        if !out.is_empty() && !out.ends_with("\n\n") {
            if out.ends_with('\n') {
                out.push('\n');
            } else {
                out.push_str("\n\n");
            }
        }
        out.push_str(section_marker);
        out.push('\n');
        out.push_str(&entry);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_frontmatter_extracts_yaml() {
        let raw = "---\ntitle: Hello\n---\n\nBody text";
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm, "title: Hello");
        assert_eq!(body, "Body text");
    }

    #[test]
    fn split_frontmatter_no_frontmatter() {
        let raw = "Just body";
        let (fm, body) = split_frontmatter(raw);
        assert!(fm.is_empty());
        assert_eq!(body, "Just body");
    }

    #[test]
    fn extract_wikilinks_basic() {
        let body = "See [[Note A]] and [[Note B|alias]] and [[Note C]]";
        let links = extract_wikilinks(body);
        assert_eq!(links, vec!["Note A", "Note B", "Note C"]);
    }

    #[test]
    fn extract_wikilinks_none() {
        let body = "No links here";
        let links = extract_wikilinks(body);
        assert!(links.is_empty());
    }

    #[test]
    fn append_nebula_section_creates_new() {
        let body = "Existing content";
        let out = append_nebula_section(body, "New update");
        assert!(out.contains("## Nebula"));
        assert!(out.contains("New update"));
        assert!(out.contains("Existing content"));
    }

    #[test]
    fn append_nebula_section_appends_to_existing() {
        let body = "## Nebula\n\nOld entry\n\n## Other\n\nText";
        let out = append_nebula_section(body, "New update");
        assert!(out.contains("## Nebula"));
        assert!(out.contains("New update"));
        assert!(out.contains("Old entry"));
        assert!(out.contains("## Other"));
    }
}
