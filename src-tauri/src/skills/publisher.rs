//! T-E-S-46: 技能发布 — `SkillPublisher` trait + `GistPublisher` + `FilePublisher`。
//!
//! P2 MVP 范围:社区市场尚不存在,用 **GitHub Gist + 本地文件导出** 替代。
//!
//! * `--target gist`(默认):上传 `SKILL.md` 到 GitHub Gist,返回 `html_url`。
//! * `--target file`:导出 `SKILL.md` 到本地目录。
//! * `--target clawhub`:预留(等 T-E-S-45 完成 exporter.rs 后接入)。
//! * `--dry-run`:仅打印 `SKILL.md` 到 stdout。
//!
//! ## 与 T-E-S-45 的协调
//!
//! T-E-S-45 负责 `skills/exporter.rs`(`to_skill_md()`)。**本模块不依赖
//! exporter.rs** —— 在 [`skill_to_skill_md`] 中内联实现一份最小 SKILL.md
//! 序列化器,字段契约与 `importer::parse_skill_md_inner` 严格对称,保证
//! 发布→导入 round-trip。若未来 T-E-S-45 注册了 `pub mod exporter`,
//! 可在调用层切换为 `SkillExporter::to_skill_md`,本模块无需改动。
//!
//! ## 安全
//!
//! * `GistPublisher` 用 [`SsrfGuard::build_safe_client`] 构造 reqwest 客户端,
//!   重定向链每跳 SSRF 校验(GitHub API 走公网,默认通过)。
//! * GitHub PAT 存 OS keychain(`publisher:github` slot,见
//!   [`crate::security::keychain::set_publisher_token`]),不落盘。
//! * `GistPublisher` 默认创建 **public** Gist(`"public": true`)—— 与
//!   agentskills.io 开放注册表语义一致;社区市场天然公开。

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tracing::info;

use super::marketplace::PublishManifest;
use super::sandbox::{Capability, CapabilitySet};
use super::types::Skill;
use crate::security::SsrfGuard;

// T-E-S-36: re-export SkillManifest 供未来 skill_to_skill_md 改用协议层类型。
// 当前 skill_to_skill_md 仍接受 &Skill(类型不兼容,保留旧逻辑);未来 publisher
// 可增加 From<&Skill> for SkillManifest 转换后切换。
pub use super::protocol::SkillManifest;

/// GitHub Gist API 端点。
const GIST_API_URL: &str = "https://api.github.com/gists";

/// 发布结果。
///
/// `target` 标识发布器类型(`gist` / `file` / `dry_run`)。`url` 仅 gist
/// 模式有值;`file_path` 仅 file 模式有值。
#[derive(Debug, Clone, Serialize)]
pub struct PublishResult {
    /// 发布目标:`gist` / `file` / `dry_run`。
    pub target: String,
    /// Gist `html_url`(gist 模式)。
    pub url: Option<String>,
    /// 本地文件绝对路径(file 模式)。
    pub file_path: Option<String>,
}

/// 技能发布器 trait。
///
/// 单一方法 [`publish`]:接收已序列化的 `SKILL.md` + `PublishManifest` +
/// 可选 token,返回 [`PublishResult`]。
#[async_trait::async_trait]
pub trait SkillPublisher: Send + Sync {
    /// 发布 SKILL.md 到目标。`token` 仅 gist 模式需要(file 模式忽略)。
    async fn publish(
        &self,
        skill_md: &str,
        manifest: &PublishManifest,
        token: Option<&str>,
    ) -> Result<PublishResult>;
}

// ---------------------------------------------------------------------------
// GistPublisher
// ---------------------------------------------------------------------------

/// GitHub Gist 发布器。
///
/// 用 [`SsrfGuard::build_safe_client`] 构造 reqwest 客户端;`publish` 时
/// POST 到 `https://api.github.com/gists`,Header `Authorization: Bearer
/// <token>`,Body 见 [`build_request_body`]。返回 `html_url`。
pub struct GistPublisher {
    client: reqwest::Client,
}

impl GistPublisher {
    /// 构造发布器(SSRF 安全客户端)。
    pub fn new() -> Result<Self> {
        let guard = SsrfGuard::new();
        let client = guard.build_safe_client()?;
        Ok(Self { client })
    }

    /// 构造 Gist API 请求体(JSON)。
    ///
    /// 测试可见:便于断言 body 格式而不发真实 HTTP 请求。
    pub fn build_request_body(skill_md: &str, manifest: &PublishManifest) -> serde_json::Value {
        let description = format!("nebula skill: {} (v{})", manifest.name, manifest.version);
        serde_json::json!({
            "description": description,
            "public": true,
            "files": {
                "SKILL.md": {
                    "content": skill_md,
                }
            }
        })
    }
}

impl Default for GistPublisher {
    fn default() -> Self {
        // T-D-B-07: build_safe_client 在 SsrfGuard::new() 下不可能失败(reqwest
        // builder 仅在 redirect policy 自定义闭包内 panic 才会出错,我们的闭包不
        // panic)。若极端情况下仍失败,降级为默认 reqwest::Client::new()(reqwest
        // 0.12 的 Client::new 返回 Client 而非 Result)。
        Self::new().unwrap_or_else(|e| {
            tracing::warn!("GistPublisher::new failed: {e}; falling back to default client");
            Self {
                client: reqwest::Client::new(),
            }
        })
    }
}

#[async_trait::async_trait]
impl SkillPublisher for GistPublisher {
    async fn publish(
        &self,
        skill_md: &str,
        manifest: &PublishManifest,
        token: Option<&str>,
    ) -> Result<PublishResult> {
        let token = token.ok_or_else(|| {
            anyhow!(
                "GitHub token required for gist publish; \
                 configure via keychain `publisher:github` slot"
            )
        })?;

        let body = Self::build_request_body(skill_md, manifest);
        let resp = self
            .client
            .post(GIST_API_URL)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "nebula-skill-publisher")
            .json(&body)
            .send()
            .await
            .context("gist POST request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("gist publish failed: HTTP {status}: {text}"));
        }

        let resp_json: serde_json::Value =
            resp.json().await.context("parsing gist response JSON")?;
        let url = resp_json
            .get("html_url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        info!(
            target: "nebula.skills.publisher",
            skill_id = %manifest.id,
            gist_url = ?url,
            "skill published to GitHub Gist"
        );

        Ok(PublishResult {
            target: "gist".to_string(),
            url,
            file_path: None,
        })
    }
}

// ---------------------------------------------------------------------------
// FilePublisher
// ---------------------------------------------------------------------------

/// 本地文件发布器。
///
/// 写 `SKILL.md` 到 `<out_dir>/<skill_id>.md`。自动创建 `out_dir`(含父目录)。
/// 文件名取自 `manifest.id`,特殊字符替换为 `_`(见 [`sanitize_filename`])。
pub struct FilePublisher {
    out_dir: PathBuf,
}

impl FilePublisher {
    pub fn new<P: Into<PathBuf>>(out_dir: P) -> Self {
        Self {
            out_dir: out_dir.into(),
        }
    }
}

#[async_trait::async_trait]
impl SkillPublisher for FilePublisher {
    async fn publish(
        &self,
        skill_md: &str,
        manifest: &PublishManifest,
        _token: Option<&str>,
    ) -> Result<PublishResult> {
        std::fs::create_dir_all(&self.out_dir)
            .with_context(|| format!("creating out_dir: {}", self.out_dir.display()))?;

        let safe_id = sanitize_filename(&manifest.id);
        let file_name = format!("{safe_id}.md");
        let file_path = self.out_dir.join(&file_name);

        std::fs::write(&file_path, skill_md)
            .with_context(|| format!("writing SKILL.md to {}", file_path.display()))?;

        info!(
            target: "nebula.skills.publisher",
            skill_id = %manifest.id,
            path = %file_path.display(),
            "skill exported to local file"
        );

        Ok(PublishResult {
            target: "file".to_string(),
            url: None,
            file_path: Some(file_path.to_string_lossy().to_string()),
        })
    }
}

// ---------------------------------------------------------------------------
// 内联 SKILL.md 序列化器(不依赖 exporter.rs)
// ---------------------------------------------------------------------------

/// YAML front-matter 序列化结构。
///
/// 字段名与 [`super::importer::SkillImporter::parse_skill_md_inner`] 读取的
/// 键严格对称,确保发布→导入 round-trip。仅写入 7 个核心字段;扩展字段
/// (`platform` / `min_confidence` / `activation_condition`)由 T-E-S-45 的
/// exporter.rs 处理,本模块不负责。
#[derive(Serialize)]
struct SkillFrontMatter<'a> {
    name: &'a str,
    description: &'a str,
    category: &'a str,
    #[serde(default)]
    tags: &'a [String],
    #[serde(default)]
    trust_level: u8,
    #[serde(default)]
    permissions: &'a [String],
    #[serde(default)]
    capabilities: Vec<String>,
}

/// `CapabilitySet` → `Vec<String>` 反向映射。
///
/// 使用 [`Capability::Display`](`std::fmt::Display` for `Capability`)输出
/// `"file:read"` / `"network"` 等 —— 与 `importer.rs` 的字符串匹配项
/// 完全对称。
fn capabilities_to_strings(caps: &CapabilitySet) -> Vec<String> {
    caps.granted()
        .iter()
        .map(|c: &Capability| c.to_string())
        .collect()
}

/// 内联 SKILL.md 生成(T-E-S-45 协调:不依赖 exporter.rs)。
///
/// 输出格式:
/// ```text
/// ---
/// name: ...
/// description: ...
/// category: ...
/// tags:
///   - ...
/// trust_level: ...
/// permissions:
///   - ...
/// capabilities:
///   - ...
/// ---
///
/// <skill.code>
/// ```
pub fn skill_to_skill_md(skill: &Skill) -> Result<String> {
    let caps_strings = capabilities_to_strings(&skill.capabilities);
    let front_matter = SkillFrontMatter {
        name: &skill.name,
        description: &skill.description,
        category: &skill.language,
        tags: &skill.tags,
        trust_level: skill.trust_level,
        permissions: &skill.permissions,
        capabilities: caps_strings,
    };

    let yaml = serde_yaml::to_string(&front_matter)
        .context("failed to serialize SKILL.md front-matter as YAML")?;
    // serde_yaml::to_string 追加尾部换行;trim_end 后重新拼接分隔符。
    let yaml_trimmed = yaml.trim_end();

    let body = if skill.code.is_empty() {
        String::new()
    } else {
        format!("\n{}", skill.code)
    };

    Ok(format!("---\n{yaml_trimmed}\n---\n{body}"))
}

/// 把可疑字符替换为 `_`,保留可读性。
///
/// 仅允许 ASCII 字母数字 + `-` + `_`;其他字符(包括路径分隔符、中文、
/// 空格)统一替换为 `_`,避免 `out_dir/<skill_id>.md` 落到非预期目录。
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_skill() -> Skill {
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::FileRead);
        caps.grant(Capability::LlmCall);

        Skill {
            id: "sk-publish-1".to_string(),
            name: "text-summarizer".to_string(),
            description: "Summarize long texts into concise bullet points.".to_string(),
            code: "# Text Summarizer\n\n## Instructions\n1. Read input\n2. Identify key points\n3. Output summary".to_string(),
            language: "text".to_string(),
            tags: vec!["nlp".to_string(), "utility".to_string()],
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 1,
            permissions: vec!["file:read".to_string()],
            capabilities: caps,
        }
    }

    fn sample_manifest() -> PublishManifest {
        PublishManifest {
            manifest_version: "1.0".to_string(),
            id: "sk-publish-1".to_string(),
            name: "text-summarizer".to_string(),
            version: "0.1.0".to_string(),
            description: "Summarize long texts into concise bullet points.".to_string(),
            author: String::new(),
            tags: vec!["nlp".to_string(), "utility".to_string()],
            source_url: None,
            dependencies: vec![],
            min_nebula_version: Some("1.3.0".to_string()),
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_skill_to_skill_md_generates_yaml() {
        let md = skill_to_skill_md(&sample_skill()).expect("test op should succeed");

        // 必须以 YAML front-matter 起始分隔符开头。
        assert!(
            md.starts_with("---\n"),
            "SKILL.md must start with '---\\n', got: {md:?}"
        );

        // 必须包含闭合分隔符。
        let rest = &md[4..];
        let end = rest
            .find("---")
            .expect("SKILL.md must contain closing '---'");

        // front-matter 段必须包含全部 7 个核心字段键。
        let front_matter = &rest[..end];
        assert!(front_matter.contains("name: text-summarizer"));
        assert!(front_matter.contains("description:"));
        assert!(front_matter.contains("category: text"));
        assert!(front_matter.contains("trust_level: 1"));
        // tags / permissions / capabilities 是 YAML 列表。
        assert!(front_matter.contains("tags:"));
        assert!(front_matter.contains("permissions:"));
        assert!(front_matter.contains("capabilities:"));

        // body 段(skill.code)必须出现在闭合分隔符之后。
        let body = rest[end + 3..].trim();
        assert!(
            body.contains("# Text Summarizer"),
            "body must contain skill.code, got: {body:?}"
        );
    }

    #[test]
    fn test_skill_to_skill_md_round_trips_with_importer() {
        // SKILL.md → from_skill_md → 8 个核心字段一致。
        use crate::skills::importer::SkillImporter;

        let original = sample_skill();
        let md = skill_to_skill_md(&original).expect("test op should succeed");
        let parsed = SkillImporter::from_skill_md(&md).expect("parse should succeed");

        assert_eq!(parsed.name, original.name);
        assert_eq!(parsed.description, original.description);
        assert_eq!(parsed.language, original.language);
        assert_eq!(parsed.code, original.code);
        assert_eq!(parsed.tags, original.tags);
        assert_eq!(parsed.trust_level, original.trust_level);
        assert_eq!(parsed.permissions, original.permissions);
        assert_eq!(parsed.capabilities, original.capabilities);
    }

    #[tokio::test]
    async fn test_file_publisher_writes_file() {
        let out_dir = std::env::temp_dir().join(format!("nebula_pub_dir_{}", uuid::Uuid::new_v4()));
        let publisher = FilePublisher::new(&out_dir);
        let manifest = sample_manifest();
        let skill_md = skill_to_skill_md(&sample_skill()).expect("test op should succeed");

        let result = publisher
            .publish(&skill_md, &manifest, None)
            .await
            .expect("send should succeed");
        assert_eq!(result.target, "file");
        assert!(result.url.is_none());

        let path = result.file_path.expect("file_path must be set");
        let content = std::fs::read_to_string(&path).expect("get should succeed");
        assert!(content.contains("name: text-summarizer"));
        assert!(content.contains("# Text Summarizer"));
        // 文件名应以 skill_id 开头并以 .md 结尾。
        assert!(path.ends_with("sk-publish-1.md"));

        // 清理。
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&out_dir);
    }

    #[test]
    fn test_gist_publisher_request_body_format() {
        let skill_md = "---\nname: demo\n---\n# Demo\nbody";
        let manifest = sample_manifest();
        let body = GistPublisher::build_request_body(skill_md, &manifest);

        // 必须包含 description / public / files.SKILL.md.content 字段。
        assert_eq!(body["public"], serde_json::Value::Bool(true));
        assert!(body["description"]
            .as_str()
            .expect("test op should succeed")
            .contains("text-summarizer"));
        assert!(body["description"]
            .as_str()
            .expect("assertion value")
            .contains("0.1.0"));
        assert_eq!(body["files"]["SKILL.md"]["content"], skill_md);
    }

    #[test]
    fn test_gist_publisher_request_body_includes_all_required_fields() {
        // 防止意外删除 public/files/description 字段。
        let body = GistPublisher::build_request_body("x", &sample_manifest());
        let obj = body.as_object().expect("test op should succeed");
        assert!(obj.contains_key("description"));
        assert!(obj.contains_key("public"));
        assert!(obj.contains_key("files"));
        assert!(obj["files"]
            .as_object()
            .expect("assertion value")
            .contains_key("SKILL.md"));
    }

    #[test]
    fn test_gist_publisher_constructs() {
        // build_safe_client 在 SsrfGuard::new() 下应成功(GitHub API 走公网)。
        let publisher = GistPublisher::new();
        assert!(publisher.is_ok(), "GistPublisher::new should succeed");
    }

    #[test]
    fn test_sanitize_filename_replaces_special_chars() {
        assert_eq!(sanitize_filename("sk-1_2"), "sk-1_2");
        assert_eq!(sanitize_filename("a/b:c"), "a_b_c");
        // 非 ASCII 字符替换为 `_`。
        assert_eq!(sanitize_filename("中文"), "__");
        assert_eq!(sanitize_filename("a b"), "a_b");
    }
}
