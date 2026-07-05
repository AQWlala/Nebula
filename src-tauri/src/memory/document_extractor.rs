//! T-E-B-12: PDF/DOCX 文档文本提取引擎。
//!
//! 为 [`super::sponge::SpongeEngine::absorb_file`] 提供二进制文档的
//! 文本提取能力,使 FileWatcher 可吸收 PDF/DOCX 文件。当前支持:
//!
//! * **PDF** — 通过 `pdf-extract`(纯 Rust,基于 lopdf)提取文本层。
//!   扫描版 PDF(无文本层)返回空字符串,视为成功。
//! * **DOCX** — 通过 `docx-rs` 解压并遍历 `word/document.xml` 段落,
//!   拼接 `<w:t>` 文本。
//!
//! 不实现 OCR / PPT/PPTX(超出 P1 范围)。
//!
//! 关键约束:
//! * 提取失败返回 `Err`,由上层(FileWatcher)决定跳过 + warn。
//! * 输出文本硬上限 1 MiB,超限截断 + warn(防止撑爆 embedding)。
//! * 扩展名检测大小写不敏感。

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// 提取后输出文本的硬上限(1 MiB)。超限截断并 warn。
const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// 受支持的文档类型。按扩展名分发。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    Pdf,
    Docx,
}

/// 按扩展名检测文档类型(大小写不敏感)。
/// 仅识别 `.pdf` / `.docx`,其余返回 `None`。
pub fn detect_kind(path: &Path) -> Option<DocumentKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Some(DocumentKind::Pdf),
        "docx" => Some(DocumentKind::Docx),
        _ => None,
    }
}

/// 主入口:按扩展名分发到对应提取器,返回 `(kind, text)`。
///
/// * 提取失败返回 `Err`(上层 FileWatcher 跳过 + warn)。
/// * 扫描版 PDF 提取空文本视为成功。
/// * 输出文本经清洗 + 1 MiB 截断。
pub fn extract_document_text(path: &Path) -> Result<(DocumentKind, String)> {
    let kind = detect_kind(path).ok_or_else(|| {
        anyhow::anyhow!("unsupported document extension: {}", path.display())
    })?;
    let raw = match kind {
        DocumentKind::Pdf => extract_pdf(path)?,
        DocumentKind::Docx => extract_docx(path)?,
    };
    let text = sanitize_and_truncate(raw);
    Ok((kind, text))
}

/// 调 `pdf-extract` 提取 PDF 文本层(纯 Rust,无系统依赖)。
/// 扫描版 PDF(无文本层)返回空字符串。
fn extract_pdf(path: &Path) -> Result<String> {
    let text = pdf_extract::extract_text(path).map_err(|e| {
        anyhow::anyhow!("pdf extract failed for {}: {:?}", path.display(), e)
    })?;
    Ok(text)
}

/// 调 `docx-rs` 读取 DOCX,遍历 `document.xml` 段落拼接文本。
fn extract_docx(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read docx {}: {}", path.display(), e))?;
    let docx = docx_rs::read_docx(&bytes)
        .map_err(|e| anyhow::anyhow!("docx parse failed for {}: {:?}", path.display(), e))?;
    let mut out = String::new();
    for child in &docx.document.children {
        if let docx_rs::DocumentChild::Paragraph(p) = child {
            collect_paragraph_text(p, &mut out);
            out.push('\n');
        }
        // TODO(见 ROADMAP): Table / TableOfContents 文本提取(P1 范围外)。
    }
    Ok(out)
}

/// 收集单个段落的文本(遍历 Run -> Text/Tab/Break)。
fn collect_paragraph_text(paragraph: &docx_rs::Paragraph, out: &mut String) {
    for pc in &paragraph.children {
        if let docx_rs::ParagraphChild::Run(run) = pc {
            collect_run_text(run, out);
        }
        // TODO: Insert/Delete/Hyperlink 内嵌的 Run 文本(track-changes / 超链接)。
    }
}

/// 收集单个 Run 的文本。
fn collect_run_text(run: &docx_rs::Run, out: &mut String) {
    for rc in &run.children {
        match rc {
            docx_rs::RunChild::Text(t) => out.push_str(&t.text),
            docx_rs::RunChild::Tab(_) => out.push('\t'),
            docx_rs::RunChild::Break(_) => out.push('\n'),
            _ => {}
        }
    }
}

/// 文本清洗:
/// 1. 控制字符(除 `\n` / `\t` / `\r`)替换为空格。
/// 2. 折叠连续空行为单个空行。
/// 3. 超过 1 MiB 时在 UTF-8 字符边界截断 + warn。
fn sanitize_and_truncate(text: String) -> String {
    // 1. 控制字符替换为空格(保留 \n \t \r)。
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c == '\n' || c == '\t' || c == '\r' {
                c
            } else if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();

    // 2. 折叠连续空行为单个空行,并统一行尾。
    let mut result = String::with_capacity(cleaned.len());
    let mut blank_run = 0usize;
    for line in cleaned.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                result.push('\n');
            }
        } else {
            blank_run = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    // 3. 1 MiB 截断(在 UTF-8 字符边界上,避免拆分多字节字符)。
    if result.len() > MAX_TEXT_BYTES {
        warn!(
            target: "nebula.document_extractor",
            size = result.len(),
            max = MAX_TEXT_BYTES,
            "document text exceeds 1MiB; truncating"
        );
        let mut end = MAX_TEXT_BYTES;
        while end > 0 && !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detect_kind_pdf_case_insensitive() {
        assert_eq!(detect_kind(&PathBuf::from("foo.pdf")), Some(DocumentKind::Pdf));
        assert_eq!(detect_kind(&PathBuf::from("FOO.PDF")), Some(DocumentKind::Pdf));
        assert_eq!(detect_kind(&PathBuf::from("a/b/c.PdF")), Some(DocumentKind::Pdf));
    }

    #[test]
    fn detect_kind_docx_case_insensitive() {
        assert_eq!(detect_kind(&PathBuf::from("bar.docx")), Some(DocumentKind::Docx));
        assert_eq!(detect_kind(&PathBuf::from("BAR.DOCX")), Some(DocumentKind::Docx));
    }

    #[test]
    fn detect_kind_unsupported_returns_none() {
        assert_eq!(detect_kind(&PathBuf::from("foo.txt")), None);
        assert_eq!(detect_kind(&PathBuf::from("foo.md")), None);
        assert_eq!(detect_kind(&PathBuf::from("noext")), None);
        assert_eq!(detect_kind(&PathBuf::from("foo.pptx")), None);
    }

    #[test]
    fn sanitize_collapses_blank_lines_and_control_chars() {
        let input = "a\x00b\n\n\n\nc\n\x07d";
        let out = sanitize_and_truncate(input.to_string());
        assert_eq!(out, "a b\n\nc\n d\n");
    }

    #[test]
    fn sanitize_truncates_at_char_boundary() {
        // 构造一个超过 1 MiB 的字符串(重复多字节字符 "中")。
        let big = "中".repeat(MAX_TEXT_BYTES);
        let out = sanitize_and_truncate(big);
        assert!(out.len() <= MAX_TEXT_BYTES);
        assert!(out.chars().all(|c| c == '中'));
    }
}
