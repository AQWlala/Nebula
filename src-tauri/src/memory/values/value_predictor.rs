//! 价值预测器。
//!
//! v1.3 版本使用基于规则的轻量启发式（与 L5 v0 假意识同思路）。
//! 后续（v2.0）可接入 LLM + 历史任务结局（`evolution::outcome`）做真正的价值预测。
//!
//! 用途：当一个动作被风险评估为 `Allow` 时，若价值预测极低，
//! [`crate::memory::values::ValuesLayer`] 会将其升级为 `Confirm`，
//! 避免执行无意义的任务。

use serde::{Deserialize, Serialize};

/// 价值低于此阈值时，Allow 升级为 Confirm。
pub const VALUE_FLOOR: f32 = 0.15;

/// 价值预测结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueVerdict {
    /// 0-1 价值分。
    pub score: f32,
    /// 预测理由。
    pub reason: String,
}

/// 价值预测器。
#[derive(Debug, Clone, Default)]
pub struct ValuePredictor;

impl ValuePredictor {
    pub fn new() -> Self {
        Self
    }

    /// 预测一个任务的价值分。
    ///
    /// v1.3 启发式：
    /// * 空描述 / 纯噪声 → 极低价值
    /// * 含明确动词 + 宾语（写/分析/总结/规划 + 对象）→ 中高价值
    /// * 含"测试""test""hello"等噪声词 → 低价值
    pub fn predict(&self, description: &str) -> ValueVerdict {
        let trimmed = description.trim();
        if trimmed.is_empty() {
            return ValueVerdict {
                score: 0.0,
                reason: "任务描述为空".to_string(),
            };
        }
        let lower = trimmed.to_lowercase();
        // 噪声任务
        let noise = [
            "test",
            "hello",
            "ping",
            "测试一下",
            "你好",
            "hi",
            "asdf",
            "xxx",
        ];
        if noise.iter().any(|n| lower == *n || lower.starts_with(n)) && trimmed.chars().count() < 12
        {
            return ValueVerdict {
                score: 0.08,
                reason: "疑似测试/噪声任务".to_string(),
            };
        }
        // 有明确动作动词
        let verbs = [
            "写",
            "分析",
            "总结",
            "规划",
            "实现",
            "修复",
            "重构",
            "设计",
            "生成",
            "整理",
            "write",
            "analyze",
            "summarize",
            "plan",
            "implement",
            "fix",
            "refactor",
            "design",
        ];
        let has_verb = verbs.iter().any(|v| lower.contains(v));
        // 有宾语（长度可作为代理指标）
        let len = trimmed.chars().count();
        let score = if has_verb && len >= 8 {
            0.7
        } else if has_verb {
            0.45
        } else if len >= 16 {
            0.4
        } else {
            0.2
        };
        ValueVerdict {
            score,
            reason: format!("启发式评估（动词={}，长度={}）", has_verb, len),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        let p = ValuePredictor::new();
        assert_eq!(p.predict("").score, 0.0);
    }

    #[test]
    fn noise_is_low() {
        let p = ValuePredictor::new();
        assert!(p.predict("test").score < VALUE_FLOOR);
    }

    #[test]
    fn real_task_is_high() {
        let p = ValuePredictor::new();
        assert!(p.predict("帮我写一份 Q3 工作总结").score >= 0.5);
    }
}
