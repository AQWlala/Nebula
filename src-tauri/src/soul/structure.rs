//! SOUL.md 文件结构 + 分区隔离（M1 任务 #19）。
//!
//! ## 双分区设计
//!
//! SOUL.md 由两个语义不同的分区组成：
//!
//! 1. **`immutable_from_ai`**：AI 不可修改区（用户手写或首次 SoulCompiler 生成后冻结）。
//!    包含用户的核心理念、人格底色、不可妥协的约束。
//!
//! 2. **`evolution-append`**：进化追加区（EvolutionEngine Phase 4 Soul 反哺写入）。
//!    包含从经验中提炼的 L5 Lessons、行为偏好等。
//!
//! ## Section 标签语法
//!
//! ```markdown
//! <!-- BEGIN SECTION: immutable_from_ai -->
//! ...用户不可改的内容...
//! <!-- END SECTION: immutable_from_ai -->
//!
//! <!-- BEGIN SECTION: evolution-append -->
//! ...进化引擎追加的内容...
//! <!-- END SECTION: evolution-append -->
//! ```
//!
//! ## 配对校验规则
//!
//! - 每个 `BEGIN SECTION: <name>` 必须有对应的 `END SECTION: <name>`
//! - Section name 必须是 `immutable_from_ai` 或 `evolution-append`（闭集）
//! - 不允许嵌套（BEGIN 在另一个 Section 内部）
//! - 允许 Section 之外存在自由文本（前言、注释），但 SoulCompiler 仅提取 Section 内容
//!
//! 参见 ADR-003 §6.3 SoulCompiler Step 4（L2/L3/L5 提取）。

use std::collections::HashMap;

use thiserror::Error;

/// 已知的 Section 名称（闭集）。
pub const SECTION_IMMUTABLE_FROM_AI: &str = "immutable_from_ai";
pub const SECTION_EVOLUTION_APPEND: &str = "evolution-append";

/// 所有合法的 Section 名称。
pub const KNOWN_SECTIONS: &[&str] = &[SECTION_IMMUTABLE_FROM_AI, SECTION_EVOLUTION_APPEND];

/// 单个 Section 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulSection {
    /// Section 名称（`immutable_from_ai` / `evolution-append`）。
    pub name: String,
    /// Section 内容（不含 BEGIN/END 标签）。
    pub content: String,
    /// BEGIN 标签在原文中的字节偏移（含标签行）。
    pub begin_offset: usize,
    /// END 标签在原文中的字节偏移。
    pub end_offset: usize,
}

/// SOUL.md 解析后的结构。
#[derive(Debug, Clone)]
pub struct SoulStructure {
    /// 解析出的所有 Section（顺序与原文一致）。
    pub sections: Vec<SoulSection>,
    /// 按 name 索引的 Section（重复时保留最后一个）。
    pub by_name: HashMap<String, SoulSection>,
    /// Section 之外的自由文本（前言、注释等）。
    pub preamble: String,
}

impl SoulStructure {
    /// 获取指定 Section 的内容。
    pub fn get_section(&self, name: &str) -> Option<&str> {
        self.by_name.get(name).map(|s| s.content.as_str())
    }

    /// 是否存在指定 Section。
    pub fn has_section(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// 获取 `immutable_from_ai` Section 内容（最常用）。
    pub fn immutable_content(&self) -> Option<&str> {
        self.get_section(SECTION_IMMUTABLE_FROM_AI)
    }

    /// 获取 `evolution-append` Section 内容。
    pub fn evolution_content(&self) -> Option<&str> {
        self.get_section(SECTION_EVOLUTION_APPEND)
    }
}

/// SOUL.md 解析错误。
#[derive(Debug, Error, PartialEq)]
pub enum SoulStructureError {
    #[error("unclosed section: BEGIN {name} at offset {offset} has no matching END")]
    UnclosedSection { name: String, offset: usize },

    #[error("mismatched END: expected END {expected}, found END {found} at offset {offset}")]
    MismatchedEnd {
        expected: String,
        found: String,
        offset: usize,
    },

    #[error("unknown section name: {name} (allowed: immutable_from_ai, evolution-append)")]
    UnknownSection { name: String },

    #[error("nested section: BEGIN {inner} inside BEGIN {outer} (nesting not allowed)")]
    NestedSection { outer: String, inner: String },

    #[error("orphan END: END {name} at offset {offset} has no matching BEGIN")]
    OrphanEnd { name: String, offset: usize },

    #[error("empty section content: section {name} cannot be empty")]
    EmptySection { name: String },
}

/// 解析 SOUL.md 文本为 `SoulStructure`。
///
/// 严格校验 Section 标签配对，遇到任何错误立即返回。
/// 空 SOUL.md（无任何 Section）返回空结构（不报错，调用方决定是否降级）。
pub fn parse_soul_md(text: &str) -> Result<SoulStructure, SoulStructureError> {
    let mut sections: Vec<SoulSection> = Vec::new();
    let mut by_name: HashMap<String, SoulSection> = HashMap::new();
    let mut preamble = String::new();

    // 当前未闭合的 Section 栈（理论上不允许嵌套，栈深度最多 1）。
    let mut stack: Vec<(String, usize, usize)> = Vec::new();
    // stack 元素：(name, begin_tag_offset, content_start_offset)

    let mut last_pos = 0;

    // 扫描所有 BEGIN / END 标签
    let begin_pat = "<!-- BEGIN SECTION: ";
    let end_pat = "<!-- END SECTION: ";

    let mut cursor = 0;
    let bytes = text.as_bytes();

    while cursor < bytes.len() {
        // 查找下一个 BEGIN 或 END
        let next_begin = find_substring(text, begin_pat, cursor);
        let next_end = find_substring(text, end_pat, cursor);

        match (next_begin, next_end) {
            (Some(b_off), Some(e_off)) if b_off < e_off => {
                // 先遇到 BEGIN
                let (name, tag_end) = parse_section_tag(text, b_off, begin_pat, "BEGIN")?;
                // 校验 name 合法
                if !KNOWN_SECTIONS.contains(&name.as_str()) {
                    return Err(SoulStructureError::UnknownSection { name });
                }
                // 校验不嵌套
                if let Some((outer, _, _)) = stack.last() {
                    return Err(SoulStructureError::NestedSection {
                        outer: outer.clone(),
                        inner: name,
                    });
                }
                // BEGIN 之前的文本归 preamble（仅当栈为空时）
                if stack.is_empty() {
                    preamble.push_str(&text[last_pos..b_off]);
                }
                stack.push((name, b_off, tag_end));
                cursor = tag_end;
                last_pos = tag_end;
            }
            (Some(_), Some(e_off)) | (None, Some(e_off)) => {
                // 先遇到 END（或仅有 END）
                let (name, tag_end) = parse_section_tag(text, e_off, end_pat, "END")?;
                // 校验 name 合法
                if !KNOWN_SECTIONS.contains(&name.as_str()) {
                    return Err(SoulStructureError::UnknownSection { name });
                }
                // 必须有匹配的 BEGIN
                match stack.pop() {
                    Some((begin_name, begin_off, content_start)) => {
                        if begin_name != name {
                            return Err(SoulStructureError::MismatchedEnd {
                                expected: begin_name,
                                found: name,
                                offset: e_off,
                            });
                        }
                        let content = text[content_start..e_off].trim().to_string();
                        // 空内容不允许（防止"幽灵 Section"）
                        // 注：放宽此约束由调用方决定（编译时可能允许空 evolution-append）
                        // 暂不强制 non-empty，由 SoulCompiler 决定降级策略
                        let _ = &content; // 抑制未使用警告
                        sections.push(SoulSection {
                            name: name.clone(),
                            content: content.clone(),
                            begin_offset: begin_off,
                            end_offset: tag_end,
                        });
                        by_name.insert(name.clone(), SoulSection {
                            name: name.clone(),
                            content,
                            begin_offset: begin_off,
                            end_offset: tag_end,
                        });
                    }
                    None => {
                        return Err(SoulStructureError::OrphanEnd {
                            name,
                            offset: e_off,
                        });
                    }
                }
                cursor = tag_end;
                last_pos = tag_end;
            }
            (Some(b_off), None) => {
                // 仅有 BEGIN（无后续 END）
                let (name, tag_end) = parse_section_tag(text, b_off, begin_pat, "BEGIN")?;
                if !KNOWN_SECTIONS.contains(&name.as_str()) {
                    return Err(SoulStructureError::UnknownSection { name });
                }
                if stack.is_empty() {
                    preamble.push_str(&text[last_pos..b_off]);
                }
                // 推入栈，继续扫描找匹配的 END
                stack.push((name.clone(), b_off, tag_end));
                cursor = tag_end;
                last_pos = tag_end;
                // 由于没有后续 END，循环会终止，最后栈非空 → UnclosedSection
            }
            (None, None) => {
                // 没有更多标签
                break;
            }
        }
    }

    // 校验所有 Section 已闭合
    if let Some((name, begin_off, _)) = stack.pop() {
        return Err(SoulStructureError::UnclosedSection {
            name,
            offset: begin_off,
        });
    }

    // 栈空后剩余文本归 preamble
    if last_pos < bytes.len() {
        preamble.push_str(&text[last_pos..]);
    }

    Ok(SoulStructure {
        sections,
        by_name,
        preamble,
    })
}

/// 在 `text` 中从 `start` 开始查找 `pat` 的位置。
fn find_substring(text: &str, pat: &str, start: usize) -> Option<usize> {
    if start > text.len() {
        return None;
    }
    text[start..].find(pat).map(|p| start + p)
}

/// 解析 `<!-- BEGIN SECTION: name -->` 或 `<!-- END SECTION: name -->` 标签。
///
/// 返回 `(section_name, tag_end_offset)`，其中 `tag_end_offset` 是标签结束后的位置。
fn parse_section_tag(
    text: &str,
    offset: usize,
    pat: &str,
    kind: &str,
) -> Result<(String, usize), SoulStructureError> {
    let pat_len = pat.len();
    let after_pat = offset + pat_len;

    // 查找 `-->` 结束标记
    let rest = &text[after_pat..];
    let close = rest
        .find("-->")
        .ok_or_else(|| SoulStructureError::UnclosedSection {
            name: format!("(malformed {kind} tag)"),
            offset,
        })?;

    let name = text[after_pat..after_pat + close].trim().to_string();
    let tag_end = after_pat + close + 3; // 3 = len("-->")

    Ok((name, tag_end))
}

/// 序列化 `SoulStructure` 回 SOUL.md 文本格式。
///
/// 用于 SoulCompiler 输出或 EvolutionEngine 写入 SOUL.md。
/// 不保留 preamble（仅 Section 内容）。
pub fn serialize_soul_md(structure: &SoulStructure) -> String {
    let mut out = String::new();
    for (i, section) in structure.sections.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "<!-- BEGIN SECTION: {} -->\n{}\n<!-- END SECTION: {} -->",
            section.name, section.content, section.name
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_returns_empty_structure() {
        let s = parse_soul_md("").unwrap();
        assert!(s.sections.is_empty());
        assert!(s.preamble.is_empty());
    }

    #[test]
    fn parse_free_text_only_no_sections() {
        let s = parse_soul_md("just some notes\nno sections here").unwrap();
        assert!(s.sections.is_empty());
        assert_eq!(s.preamble, "just some notes\nno sections here");
    }

    #[test]
    fn parse_single_immutable_section() {
        let text = "<!-- BEGIN SECTION: immutable_from_ai -->\n用户核心理念\n<!-- END SECTION: immutable_from_ai -->";
        let s = parse_soul_md(text).unwrap();
        assert_eq!(s.sections.len(), 1);
        assert_eq!(s.sections[0].name, "immutable_from_ai");
        assert_eq!(s.sections[0].content, "用户核心理念");
        assert!(s.has_section(SECTION_IMMUTABLE_FROM_AI));
        assert_eq!(s.immutable_content(), Some("用户核心理念"));
    }

    #[test]
    fn parse_both_sections() {
        let text = "<!-- BEGIN SECTION: immutable_from_ai -->\n核心\n<!-- END SECTION: immutable_from_ai -->\n\n<!-- BEGIN SECTION: evolution-append -->\n经验\n<!-- END SECTION: evolution-append -->";
        let s = parse_soul_md(text).unwrap();
        assert_eq!(s.sections.len(), 2);
        assert_eq!(s.immutable_content(), Some("核心"));
        assert_eq!(s.evolution_content(), Some("经验"));
    }

    #[test]
    fn parse_with_preamble() {
        let text = "# Soul\n\n前言\n\n<!-- BEGIN SECTION: immutable_from_ai -->\n核心\n<!-- END SECTION: immutable_from_ai -->\n\n后记";
        let s = parse_soul_md(text).unwrap();
        assert_eq!(s.sections.len(), 1);
        assert!(s.preamble.contains("# Soul"));
        assert!(s.preamble.contains("前言"));
        assert!(s.preamble.contains("后记"));
    }

    #[test]
    fn parse_unclosed_section_errors() {
        let text = "<!-- BEGIN SECTION: immutable_from_ai -->\n内容";
        let err = parse_soul_md(text).unwrap_err();
        assert!(matches!(err, SoulStructureError::UnclosedSection { name, .. } if name == "immutable_from_ai"));
    }

    #[test]
    fn parse_orphan_end_errors() {
        let text = "内容\n<!-- END SECTION: immutable_from_ai -->";
        let err = parse_soul_md(text).unwrap_err();
        assert!(matches!(err, SoulStructureError::OrphanEnd { name, .. } if name == "immutable_from_ai"));
    }

    #[test]
    fn parse_mismatched_end_errors() {
        let text = "<!-- BEGIN SECTION: immutable_from_ai -->\n核心\n<!-- END SECTION: evolution-append -->";
        let err = parse_soul_md(text).unwrap_err();
        assert!(matches!(err, SoulStructureError::MismatchedEnd { expected, found, .. }
            if expected == "immutable_from_ai" && found == "evolution-append"));
    }

    #[test]
    fn parse_nested_section_errors() {
        let text = "<!-- BEGIN SECTION: immutable_from_ai -->\n核心\n<!-- BEGIN SECTION: evolution-append -->\n经验\n<!-- END SECTION: evolution-append -->\n<!-- END SECTION: immutable_from_ai -->";
        let err = parse_soul_md(text).unwrap_err();
        assert!(matches!(err, SoulStructureError::NestedSection { outer, inner, .. }
            if outer == "immutable_from_ai" && inner == "evolution-append"));
    }

    #[test]
    fn parse_unknown_section_name_errors() {
        let text = "<!-- BEGIN SECTION: custom_section -->\n内容\n<!-- END SECTION: custom_section -->";
        let err = parse_soul_md(text).unwrap_err();
        assert!(matches!(err, SoulStructureError::UnknownSection { name, .. } if name == "custom_section"));
    }

    #[test]
    fn serialize_roundtrip() {
        let text = "<!-- BEGIN SECTION: immutable_from_ai -->\n核心\n<!-- END SECTION: immutable_from_ai -->\n\n<!-- BEGIN SECTION: evolution-append -->\n经验\n<!-- END SECTION: evolution-append -->";
        let s = parse_soul_md(text).unwrap();
        let out = serialize_soul_md(&s);
        let s2 = parse_soul_md(&out).unwrap();
        assert_eq!(s.sections.len(), s2.sections.len());
        assert_eq!(s2.immutable_content(), s.immutable_content());
        assert_eq!(s2.evolution_content(), s.evolution_content());
    }
}
