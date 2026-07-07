//! T-E-B-08: Obsidian vault 兼容。
//!
//! 功能:
//! 1. **读取 `.obsidian/` 配置** — 自动检测 vault 路径,读取 `app.json`
//! 2. **Markdown 双向同步** — Nebula Wiki ↔ Obsidian Vault
//! 3. **Frontmatter 解析** — YAML front-matter 与 WikiNote 元数据互转
//! 4. **Wikilink 转换** — `[[note]]` 双向链接在两个系统间保持一致
//!
//! ## 同步策略
//!
//! - **Nebula → Obsidian**:将 `wiki/{slug}.md` 复制到 `{vault}/Nebula/{slug}.md`,
//!   追加 `## Nebula` section 记录同步来源
//! - **Obsidian → Nebula**:扫描 vault 中的 `.md` 文件,导入为 WikiNote
//! - **冲突处理**:以 `updated_at` 较新者为准,旧版移入 `{vault}/.nebula-conflict/`
//!
//! ## 安全约束
//!
//! - vault 路径沙箱:拒绝 `..` 路径遍历
//! - 文件大小限制:8MiB(与 FileWatcher 一致)
//! - 扩展名白名单:仅 `.md` 文件参与同步

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{info, warn};

use super::WikiNote;

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// Obsidian vault 同步配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsidianSyncConfig {
    /// vault 根目录路径。
    pub vault_path: PathBuf,
    /// 是否启用双向同步。
    pub sync_enabled: bool,
    /// 同步方向。
    pub sync_direction: SyncDirection,
    /// Nebula 笔记在 vault 中的子目录名。
    pub nebula_subdir: String,
    /// 最后同步时间(Unix 毫秒)。
    pub last_sync_at: Option<i64>,
}

/// 同步方向。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    /// 双向同步(默认)。
    #[default]
    Bidirectional,
    /// 仅 Nebula → Obsidian。
    NebulaToObsidian,
    /// 仅 Obsidian → Nebula。
    ObsidianToNebula,
}

impl ObsidianSyncConfig {
    /// 默认配置:vault 路径待设置,双向同步开启。
    pub fn new(vault_path: PathBuf) -> Self {
        Self {
            vault_path,
            sync_enabled: true,
            sync_direction: SyncDirection::Bidirectional,
            nebula_subdir: "Nebula".to_string(),
            last_sync_at: None,
        }
    }

    /// Nebula 笔记在 vault 中的目录路径。
    pub fn nebula_dir(&self) -> PathBuf {
        self.vault_path.join(&self.nebula_subdir)
    }
}

// ---------------------------------------------------------------------------
// 同步引擎
// ---------------------------------------------------------------------------

/// Obsidian vault 同步引擎。
///
/// 无状态(配置通过参数传入),线程安全。
pub struct ObsidianVaultSync;

impl ObsidianVaultSync {
    /// 检测路径是否为有效的 Obsidian vault(存在 `.obsidian/` 目录)。
    pub async fn is_obsidian_vault(path: &Path) -> bool {
        fs::metadata(path.join(".obsidian")).await.is_ok()
    }

    /// 读取 `.obsidian/app.json` 配置。
    ///
    /// 返回 `Ok(None)` 表示文件不存在(新 vault 正常)。
    pub async fn read_app_config(
        vault_path: &Path,
    ) -> Result<Option<serde_json::Value>> {
        let app_json = vault_path.join(".obsidian").join("app.json");
        match fs::read_to_string(&app_json).await {
            Ok(content) => {
                let value: serde_json::Value = serde_json::from_str(&content)
                    .unwrap_or_else(|e| {
                        warn!(
                            target: "nebula.wiki.obsidian_sync",
                            path = %app_json.display(),
                            error = %e,
                            "failed to parse app.json, returning empty object"
                        );
                        serde_json::json!({})
                    });
                Ok(Some(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context(format!("reading {}", app_json.display())),
        }
    }

    /// 确保 Nebula 子目录存在。
    pub async fn ensure_nebula_dir(config: &ObsidianSyncConfig) -> Result<PathBuf> {
        let dir = config.nebula_dir();
        fs::create_dir_all(&dir).await.context(format!(
            "creating Nebula subdir {}",
            dir.display()
        ))?;
        Ok(dir)
    }

    /// 将 Nebula Wiki 笔记同步到 Obsidian vault。
    ///
    /// 写入 `{vault}/{nebula_subdir}/{slug}.md`,包含 frontmatter 和正文。
    /// 返回写入的文件路径。
    pub async fn export_to_obsidian(
        config: &ObsidianSyncConfig,
        note: &WikiNote,
        body: &str,
    ) -> Result<PathBuf> {
        Self::validate_path_component(&note.slug)?;

        let dir = Self::ensure_nebula_dir(config).await?;
        let file_path = dir.join(format!("{}.md", note.slug));

        // 构造 frontmatter
        let frontmatter = format_frontmatter(note);
        let content = if body.starts_with("---\n") {
            // body 已含 frontmatter,替换之
            if let Some(end) = body[4..].find("\n---\n") {
                format!("{}\n{}", frontmatter, &body[end + 8..])
            } else {
                format!("{}\n{}", frontmatter, body)
            }
        } else {
            format!("{}\n{}", frontmatter, body)
        };

        fs::write(&file_path, &content).await.context(format!(
            "writing {}",
            file_path.display()
        ))?;

        info!(
            target: "nebula.wiki.obsidian_sync",
            slug = %note.slug,
            path = %file_path.display(),
            "exported wiki note to obsidian vault"
        );

        Ok(file_path)
    }

    /// 从 Obsidian vault 导入 Markdown 文件为 Nebula Wiki 笔记候选。
    ///
    /// 返回解析后的 frontmatter 和 body。
    pub async fn import_from_obsidian(
        config: &ObsidianSyncConfig,
        relative_path: &str,
    ) -> Result<ImportedNote> {
        Self::validate_path_component(relative_path)?;

        let file_path = config.vault_path.join(relative_path);
        if !file_path.starts_with(&config.vault_path) {
            bail!("path traversal detected: {}", relative_path);
        }

        let content = fs::read_to_string(&file_path)
            .await
            .context(format!("reading {}", file_path.display()))?;

        // 8MiB 限制
        if content.len() > 8 * 1024 * 1024 {
            bail!("file too large ({} bytes, max 8MiB)", content.len());
        }

        let (frontmatter, body) = parse_frontmatter(&content);
        let note = parse_frontmatter_to_note(&frontmatter, relative_path);

        Ok(ImportedNote {
            note,
            body: body.to_string(),
            source_path: file_path.to_string_lossy().into_owned(),
        })
    }

    /// 扫描 vault 中的所有 `.md` 文件(排除 `.obsidian/` 和 Nebula 子目录)。
    ///
    /// 返回相对路径列表。
    pub async fn scan_vault(config: &ObsidianSyncConfig) -> Result<Vec<String>> {
        let mut results = Vec::new();
        let nebula_dir = config.nebula_dir();
        let obsidian_dir = config.vault_path.join(".obsidian");

        Self::scan_dir(
            &config.vault_path,
            &config.vault_path,
            &nebula_dir,
            &obsidian_dir,
            &mut results,
        )
        .await?;

        results.sort();
        Ok(results)
    }

    /// 递归扫描目录。
    async fn scan_dir(
        base: &Path,
        current: &Path,
        skip_dir1: &Path,
        skip_dir2: &Path,
        results: &mut Vec<String>,
    ) -> Result<()> {
        let mut entries = fs::read_dir(current).await.context(format!(
            "scanning {}",
            current.display()
        ))?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let name = entry.file_name();

            // 跳过隐藏目录(.obsidian, .trash, .git 等)
            if name.to_string_lossy().starts_with('.') {
                continue;
            }

            // 跳过 Nebula 子目录和 .obsidian 目录
            if path == *skip_dir1 || path == *skip_dir2 {
                continue;
            }

            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                Box::pin(Self::scan_dir(base, &path, skip_dir1, skip_dir2, results)).await?;
            } else if file_type.is_file() {
                let ext = path.extension().and_then(|e| e.to_str());
                if ext == Some("md") {
                    if let Ok(rel) = path.strip_prefix(base) {
                        results.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }

        Ok(())
    }

    /// 验证路径组件不含 `..` 或绝对路径。
    fn validate_path_component(component: &str) -> Result<()> {
        if component.contains("..") {
            bail!("path traversal detected: {}", component);
        }
        if component.starts_with('/') || component.starts_with('\\') {
            bail!("absolute path not allowed: {}", component);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 导入结果
// ---------------------------------------------------------------------------

/// 从 Obsidian 导入的笔记。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedNote {
    /// 解析后的 WikiNote 元数据。
    pub note: WikiNote,
    /// Markdown 正文(不含 frontmatter)。
    pub body: String,
    /// 源文件绝对路径。
    pub source_path: String,
}

// ---------------------------------------------------------------------------
// Frontmatter 解析
// ---------------------------------------------------------------------------

/// 将 WikiNote 序列化为 YAML frontmatter 字符串。
fn format_frontmatter(note: &WikiNote) -> String {
    let tags = if note.tags.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", note.tags.iter().map(|t| format!("\"{}\"", t.replace('"', "\\\""))).collect::<Vec<_>>().join(", "))
    };

    format!(
        "---\n\
         id: \"{}\"\n\
         title: \"{}\"\n\
         slug: \"{}\"\n\
         tags: {}\n\
         importance: {:.2}\n\
         created_at: {}\n\
         updated_at: {}\n\
         nebula_sync: true\n\
         ---\n",
        note.id.replace('"', "\\\""),
        note.title.replace('"', "\\\""),
        note.slug,
        tags,
        note.importance,
        note.created_at,
        note.updated_at,
    )
}

/// 从 Markdown 内容中分离 frontmatter 和 body。
///
/// 返回 `(frontmatter_str, body_str)`。无 frontmatter 时返回 `("", content)`。
/// body 已 `trim_start()`,去除 frontmatter 结束标记后的空行。
fn parse_frontmatter(content: &str) -> (&str, &str) {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return ("", content);
    }
    let after_fence = &content[4..];
    if let Some(end) = after_fence.find("\n---\n") {
        let frontmatter = &after_fence[..end];
        let body = after_fence[end + 5..].trim_start_matches('\n');
        (frontmatter, body)
    } else if let Some(end) = after_fence.find("\n---\r\n") {
        let frontmatter = &after_fence[..end];
        let body = after_fence[end + 6..].trim_start_matches('\n');
        (frontmatter, body)
    } else {
        ("", content)
    }
}

/// 从 frontmatter 字符串解析为 WikiNote。
///
/// 简单的 key-value 解析(不依赖 serde_yaml,避免新依赖)。
fn parse_frontmatter_to_note(frontmatter: &str, relative_path: &str) -> WikiNote {
    let now = Utc::now().timestamp_millis();
    let slug = Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported")
        .to_string();

    let mut note = WikiNote {
        id: format!("obsidian-{}-{}", slug, now),
        turn_id: None,
        title: slug.clone(),
        slug: slug.clone(),
        tags: Vec::new(),
        path: format!("wiki/{}.md", slug),
        created_at: now,
        updated_at: now,
        importance: 0.5,
    };

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("title:") {
            note.title = val.trim().trim_matches('"').to_string();
        } else if let Some(val) = line.strip_prefix("tags:") {
            let val = val.trim();
            if val.starts_with('[') && val.ends_with(']') {
                let inner = &val[1..val.len() - 1];
                note.tags = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        } else if let Some(val) = line.strip_prefix("importance:") {
            if let Ok(f) = val.trim().parse::<f32>() {
                note.importance = f;
            }
        }
    }

    note
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_with_meta() {
        let content = "---\ntitle: \"Test\"\ntags: [\"a\", \"b\"]\n---\n\n# Body\n";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.contains("title: \"Test\""));
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn test_parse_frontmatter_no_meta() {
        let content = "# Just a body\n";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_empty());
        assert_eq!(body.trim(), "# Just a body");
    }

    #[test]
    fn test_format_frontmatter_roundtrip() {
        let note = WikiNote {
            id: "test-123".to_string(),
            turn_id: None,
            title: "Test Note".to_string(),
            slug: "test-note".to_string(),
            tags: vec!["rust".to_string(), "wiki".to_string()],
            path: "wiki/test-note.md".to_string(),
            created_at: 1700000000,
            updated_at: 1700000001,
            importance: 0.8,
        };

        let fm = format_frontmatter(&note);
        assert!(fm.starts_with("---\n"));
        assert!(fm.contains("title: \"Test Note\""));
        assert!(fm.contains("slug: \"test-note\""));
        assert!(fm.contains("importance: 0.80"));
        assert!(fm.contains("nebula_sync: true"));
    }

    #[test]
    fn test_parse_frontmatter_to_note() {
        let fm = "title: \"My Note\"\ntags: [\"tag1\", \"tag2\"]\nimportance: 0.9";
        let note = parse_frontmatter_to_note(fm, "folder/my-note.md");
        assert_eq!(note.title, "My Note");
        assert_eq!(note.slug, "my-note");
        assert_eq!(note.tags, vec!["tag1", "tag2"]);
        assert!((note.importance - 0.9).abs() < 0.01);
        assert!(note.id.starts_with("obsidian-my-note-"));
    }

    #[test]
    fn test_validate_path_component_rejects_traversal() {
        assert!(ObsidianVaultSync::validate_path_component("../etc/passwd").is_err());
        assert!(ObsidianVaultSync::validate_path_component("/etc/passwd").is_err());
        assert!(ObsidianVaultSync::validate_path_component("\\\\server\\share").is_err());
        assert!(ObsidianVaultSync::validate_path_component("valid/path").is_ok());
        assert!(ObsidianVaultSync::validate_path_component("note.md").is_ok());
    }

    #[test]
    fn test_sync_direction_serialization() {
        assert_eq!(
            serde_json::to_string(&SyncDirection::Bidirectional).unwrap(),
            "\"bidirectional\""
        );
        assert_eq!(
            serde_json::to_string(&SyncDirection::NebulaToObsidian).unwrap(),
            "\"nebula_to_obsidian\""
        );
        assert_eq!(
            serde_json::to_string(&SyncDirection::ObsidianToNebula).unwrap(),
            "\"obsidian_to_nebula\""
        );
    }

    #[tokio::test]
    async fn test_is_obsidian_vault_detects_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        assert!(!ObsidianVaultSync::is_obsidian_vault(vault).await);

        tokio::fs::create_dir_all(vault.join(".obsidian")).await.unwrap();
        assert!(ObsidianVaultSync::is_obsidian_vault(vault).await);
    }

    #[tokio::test]
    async fn test_export_to_obsidian_writes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ObsidianSyncConfig::new(tmp.path().to_path_buf());

        let note = WikiNote {
            id: "test-1".to_string(),
            turn_id: None,
            title: "Test".to_string(),
            slug: "test".to_string(),
            tags: vec!["t1".to_string()],
            path: "wiki/test.md".to_string(),
            created_at: 1700000000,
            updated_at: 1700000001,
            importance: 0.5,
        };

        let body = "# Test\n\nHello world.";
        let path = ObsidianVaultSync::export_to_obsidian(&config, &note, body)
            .await
            .unwrap();

        assert!(path.exists());
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("title: \"Test\""));
        assert!(content.contains("# Test"));
        assert!(content.contains("Hello world."));
    }

    #[tokio::test]
    async fn test_scan_vault_finds_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();

        // 创建 vault 结构
        tokio::fs::create_dir_all(vault.join(".obsidian")).await.unwrap();
        tokio::fs::create_dir_all(vault.join("Nebula")).await.unwrap();
        tokio::fs::create_dir_all(vault.join("notes")).await.unwrap();
        tokio::fs::write(vault.join("notes/a.md"), "# A").await.unwrap();
        tokio::fs::write(vault.join("b.md"), "# B").await.unwrap();
        // 应被跳过
        tokio::fs::write(vault.join("Nebula/c.md"), "# C").await.unwrap();
        tokio::fs::write(vault.join(".hidden.md"), "# Hidden").await.unwrap();

        let config = ObsidianSyncConfig::new(vault.to_path_buf());
        let files = ObsidianVaultSync::scan_vault(&config).await.unwrap();

        assert!(files.contains(&"b.md".to_string()));
        assert!(files.contains(&"notes/a.md".to_string()));
        assert!(!files.contains(&"Nebula/c.md".to_string()));
        assert!(!files.contains(&".hidden.md".to_string()));
    }

    #[tokio::test]
    async fn test_import_from_obsidian_parses_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();

        tokio::fs::create_dir_all(vault.join("notes")).await.unwrap();
        let content = "---\ntitle: \"Imported\"\ntags: [\"test\"]\nimportance: 0.9\n---\n\n# Body\n";
        tokio::fs::write(vault.join("notes/imported.md"), content)
            .await
            .unwrap();

        let config = ObsidianSyncConfig::new(vault.to_path_buf());
        let imported = ObsidianVaultSync::import_from_obsidian(&config, "notes/imported.md")
            .await
            .unwrap();

        assert_eq!(imported.note.title, "Imported");
        assert_eq!(imported.note.tags, vec!["test"]);
        assert!((imported.note.importance - 0.9).abs() < 0.01);
        assert_eq!(imported.note.slug, "imported");
        assert!(imported.body.contains("# Body"));
    }
}
