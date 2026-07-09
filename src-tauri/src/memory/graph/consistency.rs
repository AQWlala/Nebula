//! T-E-S-64: 反幻觉一致性检查器。
//!
//! 在 AI 回复渲染时提供"引用来源" badge,提示用户回复是否基于
//! 记忆上下文,以及上下文是否存在内部冲突 / 单一工具来源 / 空引用
//! 等风险,降低幻觉风险。
//!
//! ## 设计目标
//!
//! * `analyze()` 为同步函数,执行耗时 < 1ms,不阻塞主路径。
//! * 启发式规则保守,优先 false negative(宁可漏报不误报影响体验)。
//! * `ConsistencyReport` 通过 serde 透传前端,由前端决定如何渲染 badge。

use serde::{Deserialize, Serialize};

/// 一条被引用的记忆的精简视图(从 `Memory.metadata.provenance` 提取)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitedMemory {
    /// 原始记忆 id。
    pub id: String,
    /// 来源类型(与 `Memory.source` / `Provenance.source` 一致)。
    pub source: String,
    /// 触发工具 / agent 名(如 "writer" / "sponge" / "user"),未知为 None。
    pub tool: Option<String>,
    /// 内容 SHA-256 哈希(前 16 字符),用于修改链比对。
    pub content_hash: Option<String>,
    /// 内容片段(取 content 前 80 字符)。
    pub snippet: String,
}

/// 一致性风险警告(启发式判定)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ConsistencyWarning {
    /// 同一 id 来自 ≥2 不同 source(理论上的来源冲突)。
    SourceConflict { ids: Vec<String> },
    /// 仅 1 个 cited 且 tool 为某单一工具(单源风险)。
    SingleToolNegation { tool: String },
    /// cited 为空 + response 长度 > 200(可能凭空生成)。
    EmptyCitation,
}

/// 一致性报告(透传前端)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyReport {
    pub cited: Vec<CitedMemory>,
    pub warnings: Vec<ConsistencyWarning>,
    /// 风险分数,范围 `[0.0, 1.0]`。
    ///
    /// `risk_score = warnings.len() * 0.3 + (1.0 - cited.len().min(5) / 5.0) * 0.4`,
    /// clamp 到 `[0.0, 1.0]`。
    pub risk_score: f32,
}

/// 同步分析函数(<1ms)。
///
/// 启发式规则(保守,优先 false negative):
///
/// * **SourceConflict**:同一 id 来自 ≥2 不同 source
/// * **SingleToolNegation**:仅 1 个 cited 且 tool 为某单一工具
/// * **EmptyCitation**:cited 为空 + response 长度 > 200(按字符计数)
///
/// `risk_score` 公式:
/// `warnings.len() as f32 * 0.3 + (1.0 - cited.len().min(5) as f32 / 5.0) * 0.4`,
/// clamp 到 `[0.0, 1.0]`。
pub fn analyze(cited: &[CitedMemory], response_text: &str) -> ConsistencyReport {
    let mut warnings = Vec::new();

    // SourceConflict:同一 id 出现在 ≥2 不同 source。
    use std::collections::{HashMap, HashSet};
    let mut sources_by_id: HashMap<&str, HashSet<&str>> = HashMap::new();
    for c in cited {
        sources_by_id
            .entry(c.id.as_str())
            .or_default()
            .insert(c.source.as_str());
    }
    let conflict_ids: Vec<String> = sources_by_id
        .iter()
        .filter(|(_, srcs)| srcs.len() >= 2)
        .map(|(id, _)| id.to_string())
        .collect();
    if !conflict_ids.is_empty() {
        warnings.push(ConsistencyWarning::SourceConflict { ids: conflict_ids });
    }

    // SingleToolNegation:仅 1 个 cited 且 tool 为某单一工具。
    if cited.len() == 1 {
        if let Some(tool) = cited[0].tool.as_deref() {
            if !tool.is_empty() {
                warnings.push(ConsistencyWarning::SingleToolNegation {
                    tool: tool.to_string(),
                });
            }
        }
    }

    // EmptyCitation:cited 为空 + response 长度 > 200(按字符计数)。
    if cited.is_empty() && response_text.chars().count() > 200 {
        warnings.push(ConsistencyWarning::EmptyCitation);
    }

    // risk_score = warnings.len() * 0.3 + (1 - cited.len().min(5) / 5) * 0.4
    // 范围 [0.0, 1.0],clamp 防止超出。
    let cited_factor = 1.0 - (cited.len().min(5) as f32) / 5.0;
    let raw = warnings.len() as f32 * 0.3 + cited_factor * 0.4;
    let risk_score = raw.clamp(0.0, 1.0);

    ConsistencyReport {
        cited: cited.to_vec(),
        warnings,
        risk_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cited(id: &str, source: &str, tool: Option<&str>) -> CitedMemory {
        CitedMemory {
            id: id.to_string(),
            source: source.to_string(),
            tool: tool.map(|s| s.to_string()),
            content_hash: Some("abc123".to_string()),
            snippet: "test snippet".to_string(),
        }
    }

    /// EmptyCitation 在 response > 200 字符时触发,risk_score = 0.4。
    #[test]
    fn empty_citation_triggers_when_long_response() {
        let report = analyze(&[], &"x".repeat(250));
        assert!(report
            .warnings
            .iter()
            .any(|w| matches!(w, ConsistencyWarning::EmptyCitation)));
        // M7b #90 分类 A: EmptyCitation 触发后 warnings.len()=1,所以
        // risk_score = 1 * 0.3 + (1 - 0/5) * 0.4 = 0.7(原注释误算为 0.4)。
        assert!((report.risk_score - 0.7).abs() < 0.001);
    }

    /// EmptyCitation 在短 response 时不触发(保守,优先 false negative)。
    #[test]
    fn empty_citation_not_triggered_for_short_response() {
        let report = analyze(&[], "short");
        assert!(!report
            .warnings
            .iter()
            .any(|w| matches!(w, ConsistencyWarning::EmptyCitation)));
        // risk_score = 0 * 0.3 + (1 - 0/5) * 0.4 = 0.4(仅 cited 因子)
        assert!((report.risk_score - 0.4).abs() < 0.001);
    }

    /// SingleToolNegation:仅 1 个 cited 且 tool 非空时触发。
    #[test]
    fn single_tool_negation_triggers() {
        let cited = vec![make_cited("m1", "agent_output", Some("writer"))];
        let report = analyze(&cited, "response");
        assert!(report.warnings.iter().any(|w| matches!(
            w,
            ConsistencyWarning::SingleToolNegation { tool } if tool == "writer"
        )));
        // risk_score = 1 * 0.3 + (1 - 1/5) * 0.4 = 0.3 + 0.32 = 0.62
        assert!((report.risk_score - 0.62).abs() < 0.001);
    }

    /// SourceConflict:同一 id 来自 ≥2 不同 source 时触发。
    #[test]
    fn source_conflict_triggers_for_same_id_different_sources() {
        let cited = vec![
            CitedMemory {
                id: "m1".to_string(),
                source: "user_input".to_string(),
                tool: None,
                content_hash: None,
                snippet: "a".to_string(),
            },
            CitedMemory {
                id: "m1".to_string(),
                source: "agent_output".to_string(),
                tool: None,
                content_hash: None,
                snippet: "b".to_string(),
            },
        ];
        let report = analyze(&cited, "response");
        assert!(report.warnings.iter().any(|w| matches!(
            w,
            ConsistencyWarning::SourceConflict { ids } if ids.contains(&"m1".to_string())
        )));
    }

    /// 健康引用(多 cited、无单源、无冲突)不产生 warning。
    #[test]
    fn no_warnings_when_healthy_citations() {
        let cited = vec![
            make_cited("m1", "user_input", None),
            make_cited("m2", "agent_output", None),
        ];
        let report = analyze(&cited, "response");
        assert!(report.warnings.is_empty());
        // risk_score = 0 * 0.3 + (1 - 2/5) * 0.4 = 0.24
        assert!((report.risk_score - 0.24).abs() < 0.001);
    }

    /// risk_score 始终在 `[0.0, 1.0]` 范围内。
    #[test]
    fn risk_score_always_in_range() {
        // 0 cited + long response → EmptyCitation
        let report = analyze(&[], &"x".repeat(250));
        assert!(report.risk_score >= 0.0 && report.risk_score <= 1.0);

        // 5 cited,无 warning
        let cited: Vec<CitedMemory> = (0..5)
            .map(|i| make_cited(&format!("m{i}"), "user_input", None))
            .collect();
        let report = analyze(&cited, "response");
        assert!(report.risk_score >= 0.0 && report.risk_score <= 1.0);
        // 5 cited → cited_factor = 0 → risk = 0
        assert!((report.risk_score - 0.0).abs() < 0.001);
    }
}
