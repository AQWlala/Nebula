//! LLM 驱动的多粒度摘要生成引擎。
//!
//! 设计文档 v7.0 §3.3 多粒度摘要：每条记忆在写入时生成 50/150/500/2000
//! 字符的四级摘要。短摘要用于列表预览和快速检索，长摘要用于深度阅读。
//!
//! ## 工作流程
//!
//! 1. 如果内容长度 ≤ 50 字符，直接用内容填充所有级别。
//! 2. 如果 LLM 可用，通过一次 LLM 调用生成四级摘要（用分隔符分割）。
//! 3. 如果 LLM 不可用或调用失败，回退到截断式摘要（与 sponge 原有逻辑一致）。
//!
//! ## 异步生成
//!
//! `generate` 是 async 方法，调用者可通过 `tokio::spawn` 在后台执行，
//! 不阻塞 sponge 的主写入流程。生成完成后通过 `update_summaries` 回写。

use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, warn};

use super::constants::SUMMARY_BUCKETS;
use super::types::MultiGranularity;
use crate::llm::LlmGateway;

/// 摘要生成引擎。
pub struct SummaryEngine {
    /// LLM 网关（可选：None 时回退到截断式摘要）。
    llm: Option<Arc<LlmGateway>>,
    /// 内容长度超过此阈值才调用 LLM（字符数）。
    /// 短内容直接截断即可，无需浪费 LLM 调用。
    llm_threshold: usize,
}

impl SummaryEngine {
    /// 创建一个使用指定 LLM 的摘要引擎。
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self {
            llm: Some(llm),
            llm_threshold: 200,
        }
    }

    /// 创建一个不使用 LLM 的摘要引擎（仅截断式）。
    pub fn without_llm() -> Self {
        Self {
            llm: None,
            llm_threshold: 200,
        }
    }

    /// 设置 LLM 调用阈值。
    pub fn with_llm_threshold(mut self, threshold: usize) -> Self {
        self.llm_threshold = threshold;
        self
    }

    /// 为一段内容生成多粒度摘要。
    ///
    /// 如果内容很短（≤ 50 字符）或 LLM 不可用，走快速路径；
    /// 否则调用 LLM 生成语义摘要，失败时回退到截断式。
    pub async fn generate(&self, content: &str) -> MultiGranularity {
        let char_count = content.chars().count();

        // 快速路径：内容本身就很短
        if char_count <= SUMMARY_BUCKETS[0] {
            let s = content.to_string();
            return MultiGranularity {
                s50: s.clone(),
                s150: s.clone(),
                s500: s.clone(),
                s2000: s,
            };
        }

        // 无 LLM 或内容不够长 → 截断式
        if self.llm.is_none() || char_count <= self.llm_threshold {
            return truncate_summaries(content);
        }

        // LLM 路径
        if let Some(ref llm) = self.llm {
            match self.generate_with_llm(llm, content).await {
                Ok(mg) => {
                    debug!(target: "nebula.summary", "LLM summaries generated");
                    return mg;
                }
                Err(e) => {
                    warn!(target: "nebula.summary", error = %e, "LLM summary failed, falling back to truncate");
                }
            }
        }

        truncate_summaries(content)
    }

    /// 调用 LLM 生成四级摘要。
    ///
    /// Prompt 要求 LLM 输出四个摘要，用 `===` 分隔。
    /// 解析时容错：如果 LLM 没有按格式输出，回退到截断式。
    async fn generate_with_llm(
        &self,
        llm: &Arc<LlmGateway>,
        content: &str,
    ) -> Result<MultiGranularity> {
        let prompt = build_prompt(content);
        let resp = llm.generate(&prompt).await?;

        Ok(parse_llm_response(&resp, content))
    }
}

/// 构造 LLM 摘要 prompt。
fn build_prompt(content: &str) -> String {
    format!(
        r#"请将以下内容生成四个不同长度的中文摘要。每个摘要必须独立成段，不要添加编号或标签。

要求：
1. 第一段：50字以内的极简摘要（核心要点）
2. 第二段：150字以内的简短摘要（主要信息）
3. 第三段：500字以内的中等摘要（关键细节）
4. 第四段：2000字以内的详细摘要（全面覆盖）

各摘要之间用一行三个等号（===）分隔。

内容：
{content}

请直接输出四个摘要，格式如下：
[50字摘要]
===
[150字摘要]
===
[500字摘要]
===
[2000字摘要]"#
    )
}

/// 解析 LLM 响应，提取四级摘要。
///
/// 期望格式：摘要1\n===\n摘要2\n===\n摘要3\n===\n摘要4
/// 如果解析失败，回退到截断式。
fn parse_llm_response(response: &str, original_content: &str) -> MultiGranularity {
    let parts: Vec<&str> = response.split("\n===\n").collect();
    if parts.len() >= 4 {
        let s50 = clamp_length(parts[0].trim(), SUMMARY_BUCKETS[0]);
        let s150 = clamp_length(parts[1].trim(), SUMMARY_BUCKETS[1]);
        let s500 = clamp_length(parts[2].trim(), SUMMARY_BUCKETS[2]);
        let s2000 = clamp_length(parts[3].trim(), SUMMARY_BUCKETS[3]);
        return MultiGranularity {
            s50,
            s150,
            s500,
            s2000,
        };
    }

    // 容错：尝试用 "===" 分隔（不带换行）
    let parts: Vec<&str> = response.split("===").collect();
    if parts.len() >= 4 {
        let s50 = clamp_length(parts[0].trim(), SUMMARY_BUCKETS[0]);
        let s150 = clamp_length(parts[1].trim(), SUMMARY_BUCKETS[1]);
        let s500 = clamp_length(parts[2].trim(), SUMMARY_BUCKETS[2]);
        let s2000 = clamp_length(parts[3].trim(), SUMMARY_BUCKETS[3]);
        return MultiGranularity {
            s50,
            s150,
            s500,
            s2000,
        };
    }

    // 解析失败 → 回退到截断式
    warn!(target: "nebula.summary", "LLM response not in expected format, falling back to truncate");
    truncate_summaries(original_content)
}

/// 将文本截断到指定字符数（与 sponge.rs 的 truncate_chars 逻辑一致）。
fn clamp_length(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 截断式摘要（回退方案，与 sponge.rs 的 derive_summaries 逻辑一致）。
fn truncate_summaries(content: &str) -> MultiGranularity {
    MultiGranularity {
        s50: clamp_length(content, SUMMARY_BUCKETS[0]),
        s150: clamp_length(content, SUMMARY_BUCKETS[1]),
        s500: clamp_length(content, SUMMARY_BUCKETS[2]),
        s2000: clamp_length(content, SUMMARY_BUCKETS[3]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_content_fills_all_buckets() {
        let mg = futures::executor::block_on(SummaryEngine::without_llm().generate("短文本"));
        assert_eq!(mg.s50, "短文本");
        assert_eq!(mg.s150, "短文本");
        assert_eq!(mg.s500, "短文本");
        assert_eq!(mg.s2000, "短文本");
    }

    #[test]
    fn truncate_respects_bucket_sizes() {
        let long: String = "a".repeat(3000);
        let mg = futures::executor::block_on(SummaryEngine::without_llm().generate(&long));
        assert!(mg.s50.chars().count() <= 50);
        assert!(mg.s150.chars().count() <= 150);
        assert!(mg.s500.chars().count() <= 500);
        assert!(mg.s2000.chars().count() <= 2000);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let long: String = "a".repeat(300);
        let mg = futures::executor::block_on(SummaryEngine::without_llm().generate(&long));
        // s50 应以省略号结尾
        assert!(mg.s50.ends_with('…'));
    }

    #[test]
    fn parse_llm_response_valid_format() {
        let resp =
            "这是50字摘要。\n===\n这是150字摘要。\n===\n这是500字摘要。\n===\n这是2000字摘要。";
        let mg = parse_llm_response(resp, "original");
        assert_eq!(mg.s50, "这是50字摘要。");
        assert_eq!(mg.s150, "这是150字摘要。");
        assert_eq!(mg.s500, "这是500字摘要。");
        assert_eq!(mg.s2000, "这是2000字摘要。");
    }

    #[test]
    fn parse_llm_response_without_newlines() {
        let resp = "短摘要1===短摘要2===短摘要3===短摘要4";
        let mg = parse_llm_response(resp, "original");
        assert_eq!(mg.s50, "短摘要1");
        assert_eq!(mg.s150, "短摘要2");
    }

    #[test]
    fn parse_llm_response_invalid_falls_back() {
        let resp = "只有一段摘要";
        let original = "这是一段原始内容，用于回退";
        let mg = parse_llm_response(resp, original);
        // 回退到截断式
        assert_eq!(mg.s50, original);
    }

    #[test]
    fn clamp_length_short_string_unchanged() {
        assert_eq!(clamp_length("abc", 50), "abc");
    }

    #[test]
    fn clamp_length_long_string_truncated() {
        let s = "x".repeat(100);
        let clamped = clamp_length(&s, 50);
        assert!(clamped.chars().count() <= 50);
        assert!(clamped.ends_with('…'));
    }

    #[test]
    fn build_prompt_contains_content() {
        let p = build_prompt("测试内容");
        assert!(p.contains("测试内容"));
        assert!(p.contains("==="));
        assert!(p.contains("50字"));
        assert!(p.contains("2000字"));
    }

    #[test]
    fn without_llm_does_not_panic() {
        let engine = SummaryEngine::without_llm();
        let mg = futures::executor::block_on(engine.generate("一些内容用于测试"));
        assert!(!mg.s50.is_empty());
    }

    #[test]
    fn llm_threshold_controls_llm_path() {
        // 阈值很大时，即使有 LLM 也不会调用（内容不够长）
        // 这里测试 without_llm + 高阈值的截断行为
        let engine = SummaryEngine::without_llm().with_llm_threshold(10000);
        let content = "a".repeat(500);
        let mg = futures::executor::block_on(engine.generate(&content));
        assert!(mg.s50.chars().count() <= 50);
    }
}
