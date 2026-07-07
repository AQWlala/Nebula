//! T-E-L-01: Loop Definition (LOOP.md) 解析层。
//!
//! 将 LOOP.md 文件(YAML frontmatter + Markdown body)解析为 [`LoopDef`] 结构,
//! 供 [`MasterOrchestrator::execute_loop`] 使用。
//!
//! ## LOOP.md 格式(参考 NEBULA_LOOP_DESIGN.md §2.3)
//!
//! ```text
//! ---
//! name: daily-triage
//! description: 每日扫描 CI 失败和 Issue
//! cadence: "0 9 * * 1-5"          # cron 表达式(T-E-L-02 解析,本任务仅存储)
//! autonomy: L1                     # Nebula L0-L5 枚举
//! budget_tokens: 50000
//! budget_minutes: 10
//! ---
//!
//! ## Intent
//! <意图描述>
//!
//! ## Context
//! - <上下文条目 1>
//! - <上下文条目 2>
//!
//! ## Action
//! - <动作条目 1>
//!
//! ## Observation
//! - <观察信号 1>
//!
//! ## Adjustment
//! - <调整策略 1>
//!
//! ## Stop Condition
//! <停止条件>
//!
//! ## Connectors
//! - github: required
//!
//! ## Safety
//! - <安全约束>
//! ```
//!
//! ## Feature Gate
//!
//! 与 `master.rs` 一致,由 `master-orchestrator` feature 门控。

#![cfg(feature = "master-orchestrator")]

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AutonomyLevel — 与前端 AutonomySlider.tsx 对齐的 L0-L5 枚举
// ---------------------------------------------------------------------------

/// Nebula 自主度等级 L0-L5。
///
/// **重要**:不自建 Loop 专属自主度阶梯,直接复用现有 L0-L5 体系。
/// 来源:T-E-S-50 AutonomySlider(已上线) + SKILL.md §5。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AutonomyLevel {
    /// 内联补全(Loop 不适用)。
    L0,
    /// 定向编辑(只读 + 写 STATE.md 投影)。
    L1,
    /// 对话(起草修复 + 写 Shadow Workspace 分支,人工 push)。
    L2,
    /// Plan 模式(开 Draft PR + CI + IM 通知,人工 merge)。
    L3,
    /// 蜂群 + ApprovalGate(Maker + Checker 双 Agent,需 ValuesLayer.Confirm)。
    L4,
    /// 后台自动化(CI 通过后自动合并,需强 Checker + 审计日志)。
    L5,
}

impl AutonomyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            AutonomyLevel::L0 => "L0",
            AutonomyLevel::L1 => "L1",
            AutonomyLevel::L2 => "L2",
            AutonomyLevel::L3 => "L3",
            AutonomyLevel::L4 => "L4",
            AutonomyLevel::L5 => "L5",
        }
    }

    /// 从字符串解析,大小写不敏感(接受 "L1"/"l1"/"l4")。
    pub fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_uppercase().as_str() {
            "L0" => Ok(AutonomyLevel::L0),
            "L1" => Ok(AutonomyLevel::L1),
            "L2" => Ok(AutonomyLevel::L2),
            "L3" => Ok(AutonomyLevel::L3),
            "L4" => Ok(AutonomyLevel::L4),
            "L5" => Ok(AutonomyLevel::L5),
            other => bail!("invalid autonomy level: {other} (expected L0-L5)"),
        }
    }
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        AutonomyLevel::L2
    }
}

impl std::fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// LoopDef — 解析后的完整 Loop 定义
// ---------------------------------------------------------------------------

/// LOOP.md YAML frontmatter 部分(内部反序列化用)。
#[derive(Debug, Clone, Deserialize)]
struct LoopFrontmatter {
    name: String,
    #[serde(default)]
    description: String,
    cadence: String,
    autonomy: String,
    #[serde(default)]
    budget_tokens: u64,
    #[serde(default)]
    budget_minutes: u32,
}

/// Loop 定义(解析后的完整结构,序列化给前端)。
///
/// 来源:LOOP.md 文件经 [`LoopDef::from_markdown`] 解析。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDef {
    // ---- YAML frontmatter ----
    /// Loop 名称(唯一标识)。
    pub name: String,
    /// Loop 描述(人类可读)。
    pub description: String,
    /// cron 表达式(T-E-L-02 实现完整解析,本任务仅存储字符串)。
    pub cadence: String,
    /// 自主度等级 L0-L5。
    pub autonomy: AutonomyLevel,
    /// 单次执行 Token 预算。
    pub budget_tokens: u64,
    /// 单次执行时间预算(分钟)。
    pub budget_minutes: u32,

    // ---- Markdown body 五阶段 + 元信息 ----
    /// Intent 段落(目标描述,自由文本)。
    pub intent: String,
    /// Context 条目列表(每条一行)。
    pub context: Vec<String>,
    /// Action 条目列表(每条是一个可执行动作)。
    pub action: Vec<String>,
    /// Observation 信号列表。
    pub observation: Vec<String>,
    /// Adjustment 策略列表。
    pub adjustment: Vec<String>,
    /// 停止条件(自由文本,可选)。
    pub stop_condition: Option<String>,
    /// Connectors 列表(如 "github: required")。
    pub connectors: Vec<String>,
    /// Safety 约束列表。
    pub safety: Vec<String>,
}

impl LoopDef {
    /// 从 LOOP.md markdown 字符串解析。
    ///
    /// 步骤:
    /// 1. 分离 YAML frontmatter(`---` 包裹)和 Markdown body
    /// 2. 用 `serde_yaml` 反序列化 frontmatter
    /// 3. 用状态机扫描 body 的 `## 章节` 标题,收集每节内容
    /// 4. 组装 [`LoopDef`]
    pub fn from_markdown(md: &str) -> Result<Self> {
        let (frontmatter_str, body) = split_frontmatter(md)?;

        let fm: LoopFrontmatter = serde_yaml::from_str(frontmatter_str)
            .map_err(|e| anyhow!("failed to parse LOOP.md frontmatter: {e}"))?;

        let sections = parse_body_sections(body);

        let autonomy = AutonomyLevel::from_str(&fm.autonomy)?;

        Ok(LoopDef {
            name: fm.name,
            description: fm.description,
            cadence: fm.cadence,
            autonomy,
            budget_tokens: fm.budget_tokens,
            budget_minutes: fm.budget_minutes,
            intent: sections
                .get("intent")
                .map(|lines| lines.join("\n"))
                .unwrap_or_default(),
            context: sections.get("context").cloned().unwrap_or_default(),
            action: sections.get("action").cloned().unwrap_or_default(),
            observation: sections.get("observation").cloned().unwrap_or_default(),
            adjustment: sections.get("adjustment").cloned().unwrap_or_default(),
            stop_condition: sections.get("stop condition").map(|lines| lines.join("\n")),
            connectors: sections.get("connectors").cloned().unwrap_or_default(),
            safety: sections.get("safety").cloned().unwrap_or_default(),
        })
    }

    /// 校验 LoopDef 必填字段。
    ///
    /// 规则:
    /// - `name` 非空
    /// - `intent` 非空(Loop 必须有目标)
    /// - `action` 至少 1 条(Loop 必须有动作)
    /// - `budget_tokens` 或 `budget_minutes` 至少一个 > 0(防无限运行)
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("LOOP.md `name` must not be empty");
        }
        if self.intent.trim().is_empty() {
            bail!("LOOP.md `## Intent` section must not be empty");
        }
        if self.action.is_empty() {
            bail!("LOOP.md `## Action` section must have at least one item");
        }
        if self.budget_tokens == 0 && self.budget_minutes == 0 {
            bail!("LOOP.md `budget_tokens` or `budget_minutes` must be > 0");
        }
        Ok(())
    }

    /// 生成 provenance 字段(数据主权追溯用)。
    ///
    /// 格式:`loop:<name> | autonomy:<L?>`
    ///
    /// 写入 STATE.md 每个任务项后,让用户能追溯每行代码来源。
    /// (评审新增,缓解"认知投降"风险)
    pub fn provenance(&self) -> String {
        format!("loop:{} | autonomy:{}", self.name, self.autonomy.as_str())
    }
}

// ---------------------------------------------------------------------------
// 内部解析辅助
// ---------------------------------------------------------------------------

/// 分离 YAML frontmatter 和 Markdown body。
///
/// 输入格式:
/// ```text
/// ---
/// <yaml content>
/// ---
/// <markdown body>
/// ```
///
/// 返回 `(yaml_str, body_str)`,失败情况:
/// - 不以 `---` 开头
/// - 缺少第二个 `---`(frontmatter 未闭合)
fn split_frontmatter(md: &str) -> Result<(&str, &str)> {
    let trimmed = md.trim_start();
    if !trimmed.starts_with("---") {
        bail!("LOOP.md must start with YAML frontmatter (---)");
    }
    // 跳过第一个 "---" + 换行
    let after_first = trimmed[3..].trim_start_matches(['\r', '\n']);

    // 找第二个 "---"(行首)
    let end_pos = after_first
        .find("\n---")
        .ok_or_else(|| anyhow!("LOOP.md frontmatter not closed (missing second ---)"))?;
    let frontmatter = &after_first[..end_pos];
    // 跳过 "\n---" + 换行
    let body = after_first[end_pos + 4..].trim_start_matches(['\r', '\n']);
    Ok((frontmatter, body))
}

/// 解析 Markdown body 的 `## 章节`。
///
/// 返回 `HashMap<section_name_lowercase, Vec<line>>`。
///
/// 规则:
/// - `## Title` 标记新章节(Title 转小写作为 key)
/// - 章节内容按行收集,过滤空行和注释(`//` 开头)
/// - 列表项前的 `-` / `*` / 缩进空格被去除
/// - 不在任何 `##` 下的内容被丢弃
fn parse_body_sections(body: &str) -> HashMap<String, Vec<String>> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current_section: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            // 保存上一个 section
            if let Some(name) = current_section.take() {
                sections.insert(name.to_lowercase(), std::mem::take(&mut current_lines));
            }
            // 提取 section 名(去掉 "## " 前缀,trim 尾部空白)
            current_section = Some(trimmed[3..].trim().to_string());
        } else if let Some(_) = &current_section {
            // 跳过空行
            if trimmed.is_empty() {
                continue;
            }
            // 去除列表项前缀(- / * / 数字.)和缩进
            let cleaned = trimmed.trim_start_matches(|c| c == '-' || c == '*').trim();
            // 跳过注释
            if cleaned.starts_with("//") {
                continue;
            }
            // 跳过数字列表前缀(如 "1. " / "2. ")
            let cleaned = if let Some(rest) = cleaned
                .strip_prefix(|c: char| c.is_ascii_digit())
                .and_then(|s| s.strip_prefix(". ").or_else(|| s.strip_prefix(") ")))
            {
                rest
            } else {
                cleaned
            };
            if !cleaned.is_empty() {
                current_lines.push(cleaned.to_string());
            }
        }
    }
    // 保存最后一个 section
    if let Some(name) = current_section {
        sections.insert(name.to_lowercase(), current_lines);
    }
    sections
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LOOP_MD: &str = r#"---
name: daily-triage
description: 每日扫描 CI 失败和 Issue,分类并起草 quick-win 修复
cadence: "0 9 * * 1-5"
autonomy: L1
budget_tokens: 50000
budget_minutes: 10
---

## Intent
扫描昨日的 CI 失败和 GitHub Issue,按优先级分类

## Context
- 读取 GitHub Actions 失败记录
- 读取 open issues labeled "bug"
- 加载 SKILL.md 中的项目规范

## Action
- 分类发现项(quick-win / 需深入分析 / 阻塞)
- 为 quick-win 项在 Shadow Workspace 起草修复

## Observation
- CI 失败的根因分析
- Issue 的复现路径

## Adjustment
- 将发现写入 STATE.md
- 若 quick-win 修复测试通过,提升自主度到 L2

## Stop Condition
- STATE.md 已更新,或
- Token/时间预算耗尽

## Connectors
- github: required
- filesystem: required

## Safety
- L1: 只读 CI 日志 + 只写 STATE.md
- 不触碰源码文件
"#;

    // ---- AutonomyLevel ----

    #[test]
    fn autonomy_round_trip() {
        for level in [
            AutonomyLevel::L0,
            AutonomyLevel::L1,
            AutonomyLevel::L2,
            AutonomyLevel::L3,
            AutonomyLevel::L4,
            AutonomyLevel::L5,
        ] {
            let s = level.as_str();
            let back = AutonomyLevel::from_str(s).expect("round trip");
            assert_eq!(level, back, "round trip for {s}");
        }
    }

    #[test]
    fn autonomy_case_insensitive() {
        assert_eq!(AutonomyLevel::from_str("l1").unwrap(), AutonomyLevel::L1);
        assert_eq!(AutonomyLevel::from_str("L4").unwrap(), AutonomyLevel::L4);
        assert_eq!(AutonomyLevel::from_str(" l5 ").unwrap(), AutonomyLevel::L5);
    }

    #[test]
    fn autonomy_rejects_invalid() {
        assert!(AutonomyLevel::from_str("L6").is_err());
        assert!(AutonomyLevel::from_str("L9").is_err());
        assert!(AutonomyLevel::from_str("XL").is_err());
        assert!(AutonomyLevel::from_str("").is_err());
    }

    #[test]
    fn autonomy_serde_uppercase() {
        let json = serde_json::to_string(&AutonomyLevel::L3).unwrap();
        assert_eq!(json, "\"L3\"");
        let back: AutonomyLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AutonomyLevel::L3);
    }

    #[test]
    fn autonomy_default_is_l2() {
        assert_eq!(AutonomyLevel::default(), AutonomyLevel::L2);
    }

    #[test]
    fn autonomy_display_matches_as_str() {
        assert_eq!(format!("{}", AutonomyLevel::L1), "L1");
    }

    // ---- LoopDef::from_markdown ----

    #[test]
    fn parse_full_loop_md() {
        let def = LoopDef::from_markdown(SAMPLE_LOOP_MD).expect("parse");

        assert_eq!(def.name, "daily-triage");
        assert_eq!(
            def.description,
            "每日扫描 CI 失败和 Issue,分类并起草 quick-win 修复"
        );
        assert_eq!(def.cadence, "0 9 * * 1-5");
        assert_eq!(def.autonomy, AutonomyLevel::L1);
        assert_eq!(def.budget_tokens, 50000);
        assert_eq!(def.budget_minutes, 10);

        assert!(def.intent.contains("扫描昨日"));
        assert!(def.intent.contains("GitHub Issue"));

        assert_eq!(def.context.len(), 3);
        assert!(def.context[0].contains("GitHub Actions"));

        assert_eq!(def.action.len(), 2);
        assert!(def.action[0].contains("分类发现项"));
        assert!(def.action[1].contains("Shadow Workspace"));

        assert_eq!(def.observation.len(), 2);
        assert_eq!(def.adjustment.len(), 2);

        assert!(def.stop_condition.is_some());
        let sc = def.stop_condition.as_ref().unwrap();
        assert!(sc.contains("STATE.md 已更新"));
        assert!(sc.contains("预算耗尽"));

        assert_eq!(def.connectors.len(), 2);
        assert!(def.connectors[0].contains("github"));

        assert_eq!(def.safety.len(), 2);
        assert!(def.safety[0].contains("L1"));
    }

    #[test]
    fn parse_missing_frontmatter_fails() {
        let md = "## Intent\nfoo\n";
        let err = LoopDef::from_markdown(md).unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
    }

    #[test]
    fn parse_unclosed_frontmatter_fails() {
        let md = "---\nname: foo\nautonomy: L1\ncadence: \"0 9 * * 1-5\"\n## Intent\nfoo\n";
        let err = LoopDef::from_markdown(md).unwrap_err();
        assert!(err.to_string().contains("not closed"));
    }

    #[test]
    fn parse_invalid_autonomy_fails() {
        let md = "---\nname: foo\nautonomy: L9\ncadence: \"0 9 * * 1-5\"\nbudget_tokens: 100\n---\n## Intent\nx\n## Action\n- y\n";
        let err = LoopDef::from_markdown(md).unwrap_err();
        assert!(err.to_string().contains("autonomy"));
    }

    #[test]
    fn parse_invalid_yaml_fails() {
        let md = "---\nname: [unterminated\ncadence: \"0 9 * * 1-5\"\nautonomy: L1\n---\n## Intent\nx\n## Action\n- y\n";
        let err = LoopDef::from_markdown(md).unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
    }

    #[test]
    fn parse_section_order_independent() {
        // 5 阶段顺序无关
        let md = "---\nname: foo\nautonomy: L2\ncadence: \"0 9 * * 1-5\"\nbudget_tokens: 100\n---\n## Action\n- act1\n## Intent\nmy intent\n## Stop Condition\ncond\n";
        let def = LoopDef::from_markdown(md).unwrap();
        assert_eq!(def.intent, "my intent");
        assert_eq!(def.action, vec!["act1".to_string()]);
        assert_eq!(def.stop_condition.as_deref(), Some("cond"));
    }

    #[test]
    fn parse_missing_optional_sections() {
        // Stop Condition / Connectors / Safety 可选
        let md = "---\nname: foo\nautonomy: L2\ncadence: \"0 9 * * 1-5\"\nbudget_tokens: 100\n---\n## Intent\nmy intent\n## Action\n- act1\n";
        let def = LoopDef::from_markdown(md).unwrap();
        assert!(def.stop_condition.is_none());
        assert!(def.connectors.is_empty());
        assert!(def.safety.is_empty());
        assert!(def.context.is_empty());
        assert!(def.observation.is_empty());
        assert!(def.adjustment.is_empty());
    }

    #[test]
    fn parse_filters_comments_and_blanks() {
        let md = "---\nname: foo\nautonomy: L2\ncadence: \"0 9 * * 1-5\"\nbudget_tokens: 100\n---\n## Action\n\n// 这是注释\n- real action\n\n## Intent\n\nreal intent\n";
        let def = LoopDef::from_markdown(md).unwrap();
        assert_eq!(def.action, vec!["real action".to_string()]);
        assert_eq!(def.intent, "real intent");
    }

    #[test]
    fn parse_supports_star_and_numbered_lists() {
        let md = "---\nname: foo\nautonomy: L2\ncadence: \"0 9 * * 1-5\"\nbudget_tokens: 100\n---\n## Action\n* star item\n1. numbered item\n2) paren item\nplain item\n";
        let def = LoopDef::from_markdown(md).unwrap();
        assert_eq!(def.action.len(), 4);
        assert_eq!(def.action[0], "star item");
        assert_eq!(def.action[1], "numbered item");
        assert_eq!(def.action[2], "paren item");
        assert_eq!(def.action[3], "plain item");
    }

    #[test]
    fn parse_handles_crlf_line_endings() {
        let md = "---\r\nname: foo\r\nautonomy: L2\r\ncadence: \"0 9 * * 1-5\"\r\nbudget_tokens: 100\r\n---\r\n## Intent\r\nintent\r\n## Action\r\n- act\r\n";
        let def = LoopDef::from_markdown(md).unwrap();
        assert_eq!(def.name, "foo");
        assert_eq!(def.intent, "intent");
        assert_eq!(def.action, vec!["act".to_string()]);
    }

    // ---- LoopDef::validate ----

    #[test]
    fn validate_accepts_full_def() {
        let def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.validate().expect("should pass");
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.name = "  ".to_string();
        let err = def.validate().unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn validate_rejects_empty_intent() {
        let mut def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.intent = "".to_string();
        let err = def.validate().unwrap_err();
        assert!(err.to_string().contains("Intent"));
    }

    #[test]
    fn validate_rejects_empty_action() {
        let mut def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.action.clear();
        let err = def.validate().unwrap_err();
        assert!(err.to_string().contains("Action"));
    }

    #[test]
    fn validate_rejects_zero_budget() {
        let mut def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.budget_tokens = 0;
        def.budget_minutes = 0;
        let err = def.validate().unwrap_err();
        assert!(err.to_string().contains("budget"));
    }

    #[test]
    fn validate_accepts_one_zero_budget() {
        let mut def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.budget_tokens = 0;
        def.budget_minutes = 5;
        def.validate().expect("one > 0 is enough");

        let mut def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        def.budget_tokens = 100;
        def.budget_minutes = 0;
        def.validate().expect("one > 0 is enough");
    }

    // ---- provenance ----

    #[test]
    fn provenance_format() {
        let def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        let p = def.provenance();
        assert!(p.contains("loop:daily-triage"));
        assert!(p.contains("autonomy:L1"));
    }

    // ---- serde round-trip ----

    #[test]
    fn loop_def_serde_round_trip() {
        let def = LoopDef::from_markdown(SAMPLE_LOOP_MD).unwrap();
        let json = serde_json::to_string(&def).expect("serialize");
        let back: LoopDef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.name, def.name);
        assert_eq!(back.autonomy, def.autonomy);
        assert_eq!(back.action, def.action);
        assert_eq!(back.intent, def.intent);
        assert_eq!(back.stop_condition, def.stop_condition);
    }

    // ---- split_frontmatter (内部辅助测试) ----

    #[test]
    fn split_frontmatter_basic() {
        let md = "---\nfoo: bar\n---\nbody text";
        let (fm, body) = split_frontmatter(md).unwrap();
        assert_eq!(fm, "foo: bar");
        assert_eq!(body, "body text");
    }

    #[test]
    fn split_frontmatter_no_frontmatter() {
        let md = "just body";
        assert!(split_frontmatter(md).is_err());
    }

    #[test]
    fn split_frontmatter_unclosed() {
        let md = "---\nfoo: bar\nbody";
        assert!(split_frontmatter(md).is_err());
    }

    #[test]
    fn parse_minimal_loop_md() {
        let md = "---\nname: minimal\nautonomy: L1\ncadence: \"0 0 * * *\"\nbudget_tokens: 100\n---\n## Intent\ngoal\n## Action\n- step1\n";
        let def = LoopDef::from_markdown(md).unwrap();
        def.validate().unwrap();
        assert_eq!(def.name, "minimal");
        assert_eq!(def.intent, "goal");
        assert_eq!(def.action, vec!["step1".to_string()]);
    }
}
