//! v1.3 L4 价值层（Values Layer）。
//!
//! 对应设计文档 v7.0 §2.1 的 L4 Values Layer，包含四个职责：
//!
//! * [`constitutional`] — Constitutional AI 规则引擎（宪法规则集 + 检查器）
//! * [`risk_assessor`] — 风险评估器（操作分级 Safe/Confirm/Plan/Forbidden）
//! * [`privacy_guard`] — 隐私保护（PII 检测 + 脱敏，复用 `security::detectors`）
//! * [`value_predictor`] — 价值预测（预期收益 vs 风险）
//!
//! ## 核心抽象
//!
//! 所有子模块围绕 [`Verdict`] 协作：对一个待执行的动作，L4 层给出
//! 四种裁定之一 —— 放行 / 需准奏 / 需 Plan / 禁止。
//!
//! ## 接入点
//!
//! `SwarmOrchestrator::execute` 在派发子智能体之前调用
//! [`ValuesLayer::evaluate`]，根据返回的 [`Verdict`] 决定是否继续。

pub mod constitutional;
pub mod privacy_guard;
pub mod risk_assessor;
pub mod value_predictor;

pub use constitutional::{ConstitutionalRule, ConstitutionalRules, RuleSeverity};
pub use privacy_guard::{PrivacyGuard, PrivacyVerdict};
pub use risk_assessor::{ActionKind, RiskAssessor, RiskLevel, RiskVerdict};
pub use value_predictor::{ValuePredictor, ValueVerdict};

/// L4 价值层对单个动作的最终裁定。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Verdict {
    /// 放行，无需额外确认。
    Allow,
    /// 需要用户准奏（不可逆操作：删除/发送/转账等）。
    Confirm {
        /// 准奏请求的说明（展示给用户）。
        prompt: String,
    },
    /// 需要 Plan 模式（高风险任务：先出方案，确认后执行）。
    Plan {
        /// Plan 请求的说明（展示给用户）。
        prompt: String,
    },
    /// 禁止（违反宪法规则或隐私红线）。
    Deny {
        /// 禁止理由。
        reason: String,
    },
}

impl Verdict {
    /// 是否允许直接执行（不需要任何用户介入）。
    pub fn is_allow(&self) -> bool {
        matches!(self, Verdict::Allow)
    }

    /// 是否被否决。
    pub fn is_deny(&self) -> bool {
        matches!(self, Verdict::Deny { .. })
    }
}

/// L4 价值层组合器：聚合宪法 + 风险 + 隐私 + 价值四个子评估器。
///
/// 评估顺序（短路）：
/// 1. **宪法** — 命中 `Deny` 规则直接禁止；
/// 2. **隐私** — 检测到红线 PII 泄露直接禁止；
/// 3. **风险** — 决定 `Allow` / `Confirm` / `Plan`；
/// 4. **价值** — 价值极低时可将 `Allow` 升级为 `Confirm`。
#[derive(Debug, Clone)]
pub struct ValuesLayer {
    constitutional: ConstitutionalRules,
    risk: RiskAssessor,
    privacy: PrivacyGuard,
    value: ValuePredictor,
}

impl ValuesLayer {
    /// 构建一个使用默认规则集的价值层。
    pub fn with_defaults() -> Self {
        Self {
            constitutional: ConstitutionalRules::default_rules(),
            risk: RiskAssessor::new(),
            privacy: PrivacyGuard::new(),
            value: ValuePredictor::new(),
        }
    }

    /// 对一个待执行动作进行完整评估。
    ///
    /// `description` 是任务/动作的自然语言描述（来自 `SwarmTask::description`），
    /// `kind` 是动作分类（可由前端/命令层推断，默认 [`ActionKind::Generic`]）。
    pub fn evaluate(&self, description: &str, kind: ActionKind) -> Verdict {
        // 1. 宪法规则：禁止级直接 Deny。
        if let Some(rule) = self.constitutional.match_deny(description) {
            return Verdict::Deny {
                reason: format!("违反宪法规则「{}」: {}", rule.name, rule.description),
            };
        }

        // 2. 隐私：检测描述中是否携带红线 PII（如完整身份证号）。
        match self.privacy.check_leak(description) {
            PrivacyVerdict::Block(reason) => {
                return Verdict::Deny { reason };
            }
            PrivacyVerdict::Warn(_) | PrivacyVerdict::Ok => {}
        }

        // 3. 风险评估：决定基础裁定。
        let risk_verdict = self.risk.assess(kind, description);
        let mut verdict = match risk_verdict.level {
            RiskLevel::Safe => Verdict::Allow,
            RiskLevel::NeedsConfirm => Verdict::Confirm {
                prompt: risk_verdict.reason,
            },
            RiskLevel::NeedsPlan => Verdict::Plan {
                prompt: risk_verdict.reason,
            },
            RiskLevel::Forbidden => Verdict::Deny {
                reason: risk_verdict.reason,
            },
        };

        // 4. 价值预测：价值极低时，Allow 升级为 Confirm（避免无意义执行）。
        if matches!(verdict, Verdict::Allow) {
            let value = self.value.predict(description);
            if value.score < value_predictor::VALUE_FLOOR {
                verdict = Verdict::Confirm {
                    prompt: format!(
                        "该任务预测价值极低（{:.2}），建议确认后再执行：{}",
                        value.score, value.reason
                    ),
                };
            }
        }

        verdict
    }

    /// 脱敏入口：在内容发送给 LLM 之前调用，返回脱敏后的内容。
    pub fn redact(&self, content: &str) -> String {
        self.privacy.redact(content)
    }
}

impl Default for ValuesLayer {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_layer_allows_safe_task() {
        let vl = ValuesLayer::with_defaults();
        let v = vl.evaluate("帮我写一份 Q3 工作总结", ActionKind::Generic);
        assert!(matches!(v, Verdict::Allow), "safe task should be allowed: {v:?}");
    }

    #[test]
    fn values_layer_denies_format_drive() {
        let vl = ValuesLayer::with_defaults();
        let v = vl.evaluate("格式化 C 盘并清除所有数据", ActionKind::Delete);
        assert!(v.is_deny(), "format drive should be denied: {v:?}");
    }

    #[test]
    fn values_layer_confirms_delete_file() {
        let vl = ValuesLayer::with_defaults();
        let v = vl.evaluate("删除用户配置文件 config.json", ActionKind::Delete);
        assert!(
            matches!(v, Verdict::Confirm { .. } | Verdict::Plan { .. }),
            "delete file should require confirm/plan: {v:?}"
        );
    }

    #[test]
    fn values_layer_blocks_id_leak() {
        let vl = ValuesLayer::with_defaults();
        // 18 位身份证号应被隐私守卫拦截。
        let v = vl.evaluate("把身份证 11010119900307888X 存到记忆里", ActionKind::Generic);
        assert!(v.is_deny(), "ID leak should be denied: {v:?}");
    }
}
