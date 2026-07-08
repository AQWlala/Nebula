//! T-E-B-17: ReasoningChain — 记录每步推理 premise → inference → confidence → evidence。

use serde::{Deserialize, Serialize};

/// 单步推理记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningStep {
    /// 前提(输入/已知事实)。
    pub premise: String,
    /// 推论(推理结果)。
    pub inference: String,
    /// 置信度 [0.0, 1.0]。
    pub confidence: f32,
    /// 证据/引用(可选)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// 推理链(多步推理序列)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningChain {
    /// 推理步骤序列。
    pub steps: Vec<ReasoningStep>,
    /// 整体置信度(各步置信度之积或加权平均)。
    pub overall_confidence: f32,
}

impl ReasoningChain {
    /// 从单段推理文本构造(如 DeepSeek reasoning_content)。
    /// 整体置信度默认 0.8(无逐步置信度时)。
    pub fn from_text(text: &str) -> Self {
        let step = ReasoningStep {
            premise: "(model reasoning)".to_string(),
            inference: text.to_string(),
            confidence: 0.8,
            evidence: None,
        };
        Self {
            steps: vec![step],
            overall_confidence: 0.8,
        }
    }

    /// 空推理链。
    pub fn empty() -> Self {
        Self {
            steps: Vec::new(),
            overall_confidence: 0.0,
        }
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_text() {
        let chain = ReasoningChain::from_text("Let me think about this...");
        assert_eq!(chain.steps.len(), 1);
        assert_eq!(chain.steps[0].inference, "Let me think about this...");
        assert!(!chain.is_empty());
    }

    #[test]
    fn test_empty() {
        let chain = ReasoningChain::empty();
        assert!(chain.is_empty());
        assert_eq!(chain.overall_confidence, 0.0);
    }

    #[test]
    fn test_serialize_deserialize() {
        let chain = ReasoningChain::from_text("test");
        let json = serde_json::to_string(&chain).expect("serialize should succeed");
        let de: ReasoningChain = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(de.steps.len(), 1);
    }
}
