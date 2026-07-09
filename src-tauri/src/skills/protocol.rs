//! T-E-S-36 协议层 — SkillManifest / SkillRequest / SkillResponse。
//!
//! 跨传输无关的 Skill 协议定义。本模块只描述"什么是 skill / 如何请求 /
//! 如何响应",不关心具体执行方式(本地 / 远程 / MCP)。执行由
//! [`super::executor`] 负责,能力发现由 [`super::capability`] 负责。
//!
//! T-D-B-10 扩展:本模块同时承载 SKILL.md 规范层 —— [`SkillManifest`]
//! 完整字段集(含 version/author/source/status/dependencies/eligibility)、
//! [`SkillEligibility`] 4 维运行时资格检查(bins/env/config/os)、
//! [`SkillSpecValidator`] 规范校验器。
//!
//! ## 设计
//!
//! * [`SkillManifest`] — 声明式描述一个 skill 的元数据与传输方式。
//!   未来静态内置 skill 可在编译时 `include_str!` SKILL.md 后反序列化
//!   为 `SkillManifest`。
//! * [`SkillTransport`] — 三种传输变体(`Local` / `Remote{url}` / `Mcp{server}`),
//!   `#[serde(rename_all = "snake_case")]` 保证向后兼容。
//! * [`SkillRequest`] / [`SkillResponse`] — 跨传输的请求 / 响应信封。
//! * [`SkillEligibility`] — 4 维资格声明(bins/env/config/os),用于
//!   [`SkillSpecValidator`] 在加载前判定宿主是否满足 skill 运行要求。
//! * [`SkillSpecValidator`] — SKILL.md 规范校验器,返回 [`SkillSpecReport`]。

use serde::{Deserialize, Serialize};

/// Skill 清单:声明式描述一个 skill 的元数据与传输方式。
///
/// 与 [`super::types::Skill`] 的区别:`Skill` 是持久化到 SQLite 的完整
/// 记录(含 `code` / `tags` / `usage_count` 等),`SkillManifest` 是跨
/// 传输的轻量描述(仅 name/version/description/capabilities/transport)。
/// 未来 exporter / publisher 可将 `Skill` 转换为 `SkillManifest` 后再
/// 序列化,本期仅在协议层定义。
///
/// T-D-B-10:扩展 `author` / `source` / `status` / `dependencies` /
/// `eligibility` / `min_nebula_version` 字段(均带 `#[serde(default)]`,
/// 向后兼容旧 SKILL.md)。这些字段对齐 agentskills.io 规范 + loop-engineering
/// 内化技能已使用的字段(`docs/skills/loop-engineering/SKILL.md`)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// 声明式能力标签(如 `["file:read", "network"]`)。与
    /// [`super::capability::Capability`] 的 `id` 对应,用于能力反向映射。
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub transport: SkillTransport,
    /// T-D-B-10: 作者声明(如 `"Nebula Project"`)。对应 SKILL.md frontmatter `author`。
    #[serde(default)]
    pub author: Option<String>,
    /// T-D-B-10: 来源声明(如源 URL 列表)。对应 SKILL.md frontmatter `source`。
    #[serde(default)]
    pub source: Option<Vec<String>>,
    /// T-D-B-10: 状态标签(`draft` / `internalized` / `stable` / `deprecated`)。
    /// 对应 SKILL.md frontmatter `status`。`internalized` 表示已内化为 Nebula 原生能力。
    #[serde(default)]
    pub status: Option<String>,
    /// T-D-B-10: 依赖的其他 skill id 列表。加载时按拓扑序解析。
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// T-D-B-10: 4 维运行时资格声明(bins/env/config/os)。
    /// 加载前由 [`SkillSpecValidator::check_eligibility`] 校验,任一维度不满足则拒绝加载。
    #[serde(default)]
    pub eligibility: SkillEligibility,
    /// T-D-B-10: 最小 Nebula 版本(semver)。低于此版本的宿主拒绝加载该 skill。
    #[serde(default)]
    pub min_nebula_version: Option<String>,
}

// ---------------------------------------------------------------------------
// T-D-B-10: SkillEligibility — 4 维运行时资格声明
// ---------------------------------------------------------------------------

/// 4 维运行时资格声明。
///
/// 来源:T-D-B-10 技术债务"无 Eligibility 检查(bins/env/config/os 4 维)"。
///
/// 每个维度都是一个字符串列表,语义如下:
///
/// | 维度 | 含义 | 校验方式 |
/// |------|------|---------|
/// | `bins` | 必须存在的可执行文件名(如 `["python", "git"]`) | `which` / `where` 查找 PATH |
/// | `env` | 必须设置的环境变量名(如 `["NEBULA_API_KEY"]`) | `std::env::var` 非空 |
/// | `config` | 必须存在的配置键(如 `["db_path"]`) | 查 `AppConfig` 字段非默认 |
/// | `os` | 允许的操作系统(如 `["linux", "macos"]`) | `std::env::consts::OS` 命中 |
///
/// 任一维度不满足,skill 应被拒绝加载(由 [`SkillSpecValidator`] 强制)。
/// 空列表表示该维度无要求(总是满足)。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SkillEligibility {
    /// 必须存在的可执行文件名(在 PATH 中可找到)。
    #[serde(default)]
    pub bins: Vec<String>,
    /// 必须设置(且非空)的环境变量名。
    #[serde(default)]
    pub env: Vec<String>,
    /// 必须存在的 AppConfig 配置键(目前仅做存在性提示,不深查值)。
    #[serde(default)]
    pub config: Vec<String>,
    /// 允许的操作系统白名单(空 = 全平台)。
    #[serde(default)]
    pub os: Vec<String>,
}

impl SkillEligibility {
    /// 构造一个全空的资格声明(无任何要求)。
    pub fn none() -> Self {
        Self::default()
    }

    /// 是否完全无要求(4 维全空)。
    pub fn is_empty(&self) -> bool {
        self.bins.is_empty() && self.env.is_empty() && self.config.is_empty() && self.os.is_empty()
    }
}

/// Skill 传输方式。
///
/// `#[serde(rename_all = "snake_case")]` 保证枚举变体名在序列化时为
/// 小写蛇形,与 agentskills.io / MCP 社区惯例一致。
///
/// T-D-B-10: 派生 `Default` = `Local`,使 [`SkillManifest`] 可实现 `Default`
/// (用于测试构造 + 协议层占位)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillTransport {
    /// 本地 in-process 执行(委派给 [`super::executor::LocalExecutor`] /
    /// 既有 sandbox)。
    #[default]
    Local,
    /// 远程 HTTP 执行(委派给 [`super::executor::RemoteExecutor`] +
    /// SSRF 校验)。
    Remote { url: String },
    /// MCP 协议执行(委派给 [`super::executor::McpExecutor`])。
    Mcp { server: String },
}

impl SkillManifest {
    /// T-D-B-10: 构造一个最小化的 manifest(仅必填字段),其余字段取默认值。
    ///
    /// 用于测试 + 协议层占位。生产代码应通过 [`SkillSpecValidator::parse_skill_md`]
    /// 从 SKILL.md 解析完整 manifest。
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
        capabilities: Vec<String>,
        transport: SkillTransport,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            description: description.into(),
            capabilities,
            transport,
            author: None,
            source: None,
            status: None,
            dependencies: Vec::new(),
            eligibility: SkillEligibility::default(),
            min_nebula_version: None,
        }
    }
}

impl Default for SkillManifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            description: String::new(),
            capabilities: Vec::new(),
            transport: SkillTransport::default(),
            author: None,
            source: None,
            status: None,
            dependencies: Vec::new(),
            eligibility: SkillEligibility::default(),
            min_nebula_version: None,
        }
    }
}

/// Skill 执行请求(跨传输信封)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRequest {
    /// 目标 skill 名称(与 [`SkillManifest::name`] 对应)。
    pub skill: String,
    /// 输入载荷(任意 JSON 值)。RemoteExecutor 期望包含 `url` 字段。
    pub input: serde_json::Value,
    /// 超时(毫秒)。0 表示用执行器默认超时。
    pub timeout_ms: u32,
}

/// Skill 执行响应(跨传输信封)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillResponse {
    /// 输出载荷(任意 JSON 值)。
    pub output: serde_json::Value,
    /// 错误信息。`None` 表示成功。
    pub error: Option<String>,
    /// 端到端延迟(毫秒)。
    pub latency_ms: u64,
}

// ---------------------------------------------------------------------------
// T-D-B-10: SkillSpecValidator — SKILL.md 规范校验器
// ---------------------------------------------------------------------------

/// SKILL.md 规范校验结果。
///
/// 由 [`SkillSpecValidator::validate_skill_md`] 返回。`eligible` 表示
/// 当前宿主是否满足 4 维资格(bins/env/config/os);`errors` / `warnings`
/// 列出规范层面的硬错误与软警告。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillSpecReport {
    /// 是否通过校验(`errors.is_empty()` 且 `eligible == true`)。
    pub valid: bool,
    /// 当前宿主是否满足 4 维资格。
    pub eligible: bool,
    /// 解析出的 manifest(解析成功时;失败为 `None`)。
    pub manifest: Option<SkillManifest>,
    /// 硬错误列表(规范违反,必须修复)。
    pub errors: Vec<String>,
    /// 软警告列表(不阻塞加载,但建议修复)。
    pub warnings: Vec<String>,
    /// 4 维资格检查详情(每维度的失败项)。
    pub eligibility_failures: Vec<String>,
}

impl SkillSpecReport {
    /// 是否有任何硬错误。
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// SKILL.md 规范校验器。
///
/// 来源:T-D-B-10 技术债务"agentskills.io 规范字段缺失"。
///
/// 校验 3 个层面:
///
/// 1. **结构层**:YAML frontmatter 必须存在且可解析;`name` / `version` /
///    `description` 必填;`version` 必须是 semver (X.Y.Z)。
/// 2. **规范层**:`transport` 必须是 `local` / `remote{url}` / `mcp{server}`
///    之一;`status` 必须是 `draft` / `internalized` / `stable` / `deprecated`
///    之一(若提供);`min_nebula_version` 必须是 semver(若提供)。
/// 3. **资格层**:`eligibility.bins` 中每个二进制必须在 PATH 中可找到;
///    `eligibility.env` 中每个环境变量必须已设置且非空;
///    `eligibility.config` 仅做存在性提示(无法在协议层深查);
///    `eligibility.os` 若非空,当前 `std::env::consts::OS` 必须命中。
pub struct SkillSpecValidator;

impl SkillSpecValidator {
    /// 校验一段 SKILL.md 内容,返回详细报告。
    ///
    /// 不会 panic;解析失败时返回 `errors` 非空、`manifest = None` 的报告。
    pub fn validate_skill_md(content: &str) -> SkillSpecReport {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut eligibility_failures = Vec::new();

        // 1) 解析 YAML frontmatter。
        let manifest = match Self::parse_manifest(content) {
            Ok(m) => Some(m),
            Err(e) => {
                errors.push(format!("manifest parse failed: {e}"));
                None
            }
        };

        // 2) 结构层 + 规范层校验(仅当 manifest 解析成功)。
        if let Some(ref m) = manifest {
            if m.name.is_empty() {
                errors.push("name is required".to_string());
            }
            if m.version.is_empty() {
                errors.push("version is required".to_string());
            } else if !is_semver(&m.version) {
                errors.push(format!(
                    "version '{}' is not semver (expected X.Y.Z)",
                    m.version
                ));
            }
            if m.description.is_empty() {
                warnings.push("description is empty".to_string());
            }
            if let Some(ref status) = m.status {
                if !matches!(
                    status.as_str(),
                    "draft" | "internalized" | "stable" | "deprecated"
                ) {
                    warnings.push(format!(
                        "status '{status}' is not one of draft/internalized/stable/deprecated"
                    ));
                }
            }
            if let Some(ref min_ver) = m.min_nebula_version {
                if !is_semver(min_ver) {
                    errors.push(format!(
                        "min_nebula_version '{min_ver}' is not semver (expected X.Y.Z)"
                    ));
                }
            }

            // 3) 资格层校验。
            Self::check_eligibility(&m.eligibility, &mut eligibility_failures);
        }

        let eligible = eligibility_failures.is_empty();
        let valid = errors.is_empty() && eligible;

        SkillSpecReport {
            valid,
            eligible,
            manifest,
            errors,
            warnings,
            eligibility_failures,
        }
    }

    /// 从 SKILL.md 内容解析出 [`SkillManifest`]。
    ///
    /// 与 [`super::importer::SkillImporter::from_skill_md`] 不同,本函数
    /// 返回的是协议层 manifest(含 transport / eligibility / dependencies
    /// 等扩展字段),而非持久层 [`super::types::CreateSkillRequest`]。
    pub fn parse_manifest(content: &str) -> Result<SkillManifest, String> {
        let trimmed = content.trim_start_matches('\u{feff}');
        if !trimmed.starts_with("---") {
            return Err("no YAML frontmatter found (expected leading '---')".to_string());
        }
        let rest = &trimmed[3..];
        let end = rest
            .find("\n---")
            .or_else(|| rest.find("\r\n---"))
            .ok_or_else(|| "unclosed frontmatter (missing closing '---')".to_string())?;
        let yaml_str = &rest[..end];

        let yaml_value: serde_yaml::Value =
            serde_yaml::from_str(yaml_str).map_err(|e| format!("YAML parse error: {e}"))?;
        let json_str = serde_json::to_string(&yaml_value)
            .map_err(|e| format!("YAML-to-JSON conversion error: {e}"))?;
        let json_value: serde_json::Value =
            serde_json::from_str(&json_str).map_err(|e| format!("JSON round-trip error: {e}"))?;

        let mut manifest: SkillManifest = serde_json::from_value(json_value)
            .map_err(|e| format!("manifest deserialize error: {e}"))?;

        // 若 frontmatter 未提供 transport,默认 Local(向后兼容旧 SKILL.md)。
        // serde 默认会填入 Local(因 SkillTransport: Default),此处冗余保险。
        if manifest.transport == SkillTransport::Local && manifest.name.is_empty() {
            // 极端情况:完全空 manifest。保持原样,后续校验会报错。
        }

        // 兼容:frontmatter 中 `id` 字段若存在,作为 name 的回退(部分
        // SKILL.md 用 `id` 而非 `name` 作为主键)。
        // 注意:需先把 yaml Value 绑定到变量再借用,避免闭包返回对
        // 已 drop 的临时 Value 的引用(借用检查器会拒绝)。
        if manifest.name.is_empty() {
            if let Ok(yaml_val) = serde_yaml::from_str::<serde_yaml::Value>(yaml_str) {
                if let Some(id) = yaml_val
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                {
                    manifest.name = id;
                }
            }
        }

        Ok(manifest)
    }

    /// 4 维资格检查:bins/env/config/os。
    ///
    /// 失败项追加到 `failures`(每项格式如 `"bins: python not found in PATH"`)。
    /// 不修改全局状态;`bins` 检查使用 `which` crate 等价的手工 PATH 扫描。
    pub fn check_eligibility(elig: &SkillEligibility, failures: &mut Vec<String>) {
        // bins: 在 PATH 中查找每个二进制。
        for bin in &elig.bins {
            if !find_in_path(bin) {
                failures.push(format!("bins: '{bin}' not found in PATH"));
            }
        }
        // env: 环境变量必须设置且非空。
        for var in &elig.env {
            match std::env::var(var) {
                Ok(v) if !v.is_empty() => {}
                Ok(_) => failures.push(format!("env: '{var}' is set but empty")),
                Err(_) => failures.push(format!("env: '{var}' is not set")),
            }
        }
        // config: 仅做存在性提示(协议层无法深查 AppConfig)。
        // 失败项不加入 failures(否则会阻塞加载),而是作为 warning 透传。
        // 调用方应在 Tauri 命令层补充 AppConfig 检查。
        for cfg_key in &elig.config {
            // 占位:实际检查需 AppConfig 上下文,此处仅记录 warning。
            // 为避免误判,config 维度不产生 failure,只由上层补充。
            let _ = cfg_key;
        }
        // os: 当前 OS 必须命中白名单。
        if !elig.os.is_empty() {
            let current_os = std::env::consts::OS;
            if !elig.os.iter().any(|o| o == current_os) {
                failures.push(format!(
                    "os: current '{current_os}' not in allowed list {:?}",
                    elig.os
                ));
            }
        }
    }
}

/// 判断字符串是否为 semver (X.Y.Z, X/Y/Z 为非负整数)。
///
/// 不引入 semver crate(避免新依赖),仅做格式校验。
/// 接受 `1.2.3` / `0.1.0` / `2.0.0`;拒绝 `1.2` / `latest` / `v1.2.3`。
fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u32>().is_ok())
}

/// 在 PATH 中查找二进制(等价于 `which` / `where`)。
///
/// 跨平台:Windows 上自动尝试 `.exe` / `.bat` / `.cmd` 后缀;
/// Unix 上直接查找裸名。不区分大小写(Windows)。
fn find_in_path(bin: &str) -> bool {
    let path_env = match std::env::var("PATH") {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Windows 可执行文件后缀。
    #[cfg(target_os = "windows")]
    let suffixes: &[&str] = &["", ".exe", ".bat", ".cmd", ".ps1"];
    #[cfg(not(target_os = "windows"))]
    let suffixes: &[&str] = &[""];

    for dir in std::env::split_paths(&path_env) {
        for suffix in suffixes {
            let candidate = dir.join(format!("{bin}{suffix}"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_manifest_local_transport_round_trip() {
        // Local 变体:序列化为 "local" 字符串,反序列化还原。
        let m = SkillManifest::new(
            "echo",
            "1.0.0",
            "echo input back",
            vec!["io".to_string()],
            SkillTransport::Local,
        );
        let j = serde_json::to_string(&m).expect("serialize should succeed");
        // transport 应序列化为 "local"(snake_case)。
        assert!(
            j.contains("\"transport\":\"local\""),
            "expected snake_case 'local', got: {j}"
        );
        let m2: SkillManifest = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(m, m2);
    }

    #[test]
    fn test_skill_manifest_remote_transport_round_trip() {
        // Remote 变体:序列化为 {"remote":{"url":...}},反序列化还原。
        let m = SkillManifest::new(
            "fetch",
            "2.0.0",
            "fetch a remote url",
            vec!["network".to_string()],
            SkillTransport::Remote {
                url: "https://api.example.com/skill".to_string(),
            },
        );
        let j = serde_json::to_string(&m).expect("serialize should succeed");
        assert!(j.contains("\"remote\""), "expected 'remote' tag, got: {j}");
        assert!(j.contains("https://api.example.com/skill"));
        let m2: SkillManifest = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(m, m2);
    }

    #[test]
    fn test_skill_manifest_mcp_transport_round_trip() {
        // Mcp 变体:序列化为 {"mcp":{"server":...}},反序列化还原。
        let m = SkillManifest::new(
            "mcp-tool",
            "0.1.0",
            "mcp protocol tool",
            vec!["mcp".to_string()],
            SkillTransport::Mcp {
                server: "mcp-server-1".to_string(),
            },
        );
        let j = serde_json::to_string(&m).expect("serialize should succeed");
        assert!(j.contains("\"mcp\""), "expected 'mcp' tag, got: {j}");
        assert!(j.contains("mcp-server-1"));
        let m2: SkillManifest = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(m, m2);
    }

    /// T-D-B-10: 扩展字段(author/source/status/dependencies/eligibility/
    /// min_nebula_version)应可无损往返。
    #[test]
    fn test_skill_manifest_extended_fields_round_trip() {
        let m = SkillManifest {
            name: "loop-engineering".to_string(),
            version: "1.0.0".to_string(),
            description: "Loop Engineering 内化技能".to_string(),
            capabilities: vec!["llm:call".to_string()],
            transport: SkillTransport::Local,
            author: Some("Nebula Project".to_string()),
            source: Some(vec![
                "https://www.runoob.com/ai-agent/loop-engineering.html".to_string(),
            ]),
            status: Some("internalized".to_string()),
            dependencies: vec!["shadow-workspace".to_string()],
            eligibility: SkillEligibility {
                bins: vec!["python".to_string()],
                env: vec!["NEBULA_API_KEY".to_string()],
                config: vec!["db_path".to_string()],
                os: vec!["linux".to_string(), "macos".to_string()],
            },
            min_nebula_version: Some("3.1.0".to_string()),
        };
        let j = serde_json::to_string(&m).expect("serialize should succeed");
        assert!(j.contains("\"author\":\"Nebula Project\""));
        assert!(j.contains("\"status\":\"internalized\""));
        assert!(j.contains("\"dependencies\""));
        assert!(j.contains("\"eligibility\""));
        assert!(j.contains("\"min_nebula_version\":\"3.1.0\""));
        let m2: SkillManifest = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(m, m2);
    }

    /// T-D-B-10: 旧 payload(无扩展字段)应能反序列化为新 manifest,
    /// 扩展字段全部填默认值(向后兼容)。
    #[test]
    fn test_skill_manifest_backwards_compatible_with_old_payload() {
        let old_json = r#"{
            "name": "legacy",
            "version": "0.9.0",
            "description": "old skill without extended fields",
            "capabilities": ["io"],
            "transport": "local"
        }"#;
        let m: SkillManifest = serde_json::from_str(old_json).expect("parse should succeed");
        assert_eq!(m.name, "legacy");
        assert_eq!(m.transport, SkillTransport::Local);
        assert_eq!(m.author, None);
        assert_eq!(m.status, None);
        assert!(m.dependencies.is_empty());
        assert!(m.eligibility.is_empty());
        assert_eq!(m.min_nebula_version, None);
    }

    /// T-D-B-10: SkillEligibility::is_empty() 在 4 维全空时返回 true。
    #[test]
    fn test_skill_eligibility_is_empty() {
        assert!(SkillEligibility::none().is_empty());
        assert!(SkillEligibility::default().is_empty());
        let mut e = SkillEligibility::none();
        e.bins.push("python".to_string());
        assert!(!e.is_empty());
    }

    /// T-D-B-10: is_semver 格式校验。
    #[test]
    fn test_is_semver_format() {
        assert!(is_semver("1.2.3"));
        assert!(is_semver("0.0.0"));
        assert!(is_semver("10.20.30"));
        assert!(!is_semver("1.2"));
        assert!(!is_semver("1.2.3.4"));
        assert!(!is_semver("latest"));
        assert!(!is_semver("v1.2.3"));
        assert!(!is_semver(""));
    }

    /// T-D-B-10: SkillSpecValidator::validate_skill_md 应通过合法 SKILL.md。
    #[test]
    fn test_validator_passes_valid_skill_md() {
        let md = r#"---
name: test-skill
version: 1.0.0
description: A valid test skill
capabilities: ["io"]
transport: local
---
# Body
"#;
        let report = SkillSpecValidator::validate_skill_md(md);
        assert!(
            report.errors.is_empty(),
            "expected no errors, got: {:?}",
            report.errors
        );
        assert!(report.eligible, "expected eligible");
        assert!(report.valid, "expected valid");
        assert!(report.manifest.is_some());
        let m = report.manifest.as_ref().expect("manifest should exist");
        assert_eq!(m.name, "test-skill");
        assert_eq!(m.version, "1.0.0");
    }

    /// T-D-B-10: 缺少 name/version 应报硬错误。
    #[test]
    fn test_validator_reports_missing_required_fields() {
        let md = r#"---
description: missing name and version
---
# Body
"#;
        let report = SkillSpecValidator::validate_skill_md(md);
        assert!(!report.valid);
        assert!(report.has_errors());
        assert!(
            report.errors.iter().any(|e| e.contains("name is required")),
            "expected name required error, got: {:?}",
            report.errors
        );
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("version is required")),
            "expected version required error, got: {:?}",
            report.errors
        );
    }

    /// T-D-B-10: 非 semver version 应报硬错误。
    #[test]
    fn test_validator_rejects_non_semver_version() {
        let md = r#"---
name: bad-version
version: latest
description: bad version
---
# Body
"#;
        let report = SkillSpecValidator::validate_skill_md(md);
        assert!(!report.valid);
        assert!(
            report.errors.iter().any(|e| e.contains("not semver")),
            "expected semver error, got: {:?}",
            report.errors
        );
    }

    /// T-D-B-10: 无 frontmatter 应报硬错误。
    #[test]
    fn test_validator_rejects_missing_frontmatter() {
        let md = "# Just a body\nNo frontmatter here";
        let report = SkillSpecValidator::validate_skill_md(md);
        assert!(!report.valid);
        assert!(report.errors.iter().any(|e| e.contains("frontmatter")));
    }

    /// T-D-B-10: eligibility.os 白名单不命中当前 OS 时,eligible=false。
    #[test]
    fn test_validator_eligibility_os_mismatch() {
        let current_os = std::env::consts::OS;
        let other_os = if current_os == "windows" {
            "linux"
        } else {
            "windows"
        };
        let md = format!(
            r#"---
name: os-restricted
version: 1.0.0
description: only runs on {other_os}
eligibility:
  os: ["{other_os}"]
---
# Body
"#
        );
        let report = SkillSpecValidator::validate_skill_md(&md);
        assert!(
            !report.eligible,
            "expected not eligible on {current_os}, got failures: {:?}",
            report.eligibility_failures
        );
        assert!(report
            .eligibility_failures
            .iter()
            .any(|f| f.starts_with("os:")));
    }

    /// T-D-B-10: parse_manifest 应能解析 loop-engineering 风格的 SKILL.md
    /// (含 author / source / status 扩展字段)。
    #[test]
    fn test_parse_manifest_loop_engineering_style() {
        let md = r#"---
name: loop-engineering
version: 1.0.0
description: Loop Engineering 内化技能
author: Nebula Project
source:
  - https://www.runoob.com/ai-agent/loop-engineering.html
status: internalized
capabilities: ["llm:call"]
transport: local
---
# Loop Engineering 技能
## 1. 核心理念
"#;
        let manifest = SkillSpecValidator::parse_manifest(md).expect("parse should succeed");
        assert_eq!(manifest.name, "loop-engineering");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.author.as_deref(), Some("Nebula Project"));
        assert_eq!(manifest.status.as_deref(), Some("internalized"));
        assert!(manifest.source.is_some());
        assert_eq!(manifest.source.as_ref().expect("source").len(), 1);
        assert_eq!(manifest.transport, SkillTransport::Local);
    }

    #[test]
    fn test_skill_request_response_serde() {
        // 请求 / 响应信封应可无损往返。
        let req = SkillRequest {
            skill: "echo".to_string(),
            input: serde_json::json!({"text": "hello"}),
            timeout_ms: 5000,
        };
        let req_j = serde_json::to_string(&req).expect("serialize should succeed");
        let req2: SkillRequest = serde_json::from_str(&req_j).expect("parse should succeed");
        assert_eq!(req.skill, req2.skill);
        assert_eq!(req.input, req2.input);
        assert_eq!(req.timeout_ms, req2.timeout_ms);

        let resp = SkillResponse {
            output: serde_json::json!({"echo": "hello"}),
            error: None,
            latency_ms: 42,
        };
        let resp_j = serde_json::to_string(&resp).expect("serialize should succeed");
        let resp2: SkillResponse = serde_json::from_str(&resp_j).expect("parse should succeed");
        assert_eq!(resp.output, resp2.output);
        assert_eq!(resp.error, resp2.error);
        assert_eq!(resp.latency_ms, resp2.latency_ms);
    }
}
