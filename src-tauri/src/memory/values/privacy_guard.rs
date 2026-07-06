//! 隐私保护守卫。
//!
//! 复用 [`crate::security::detectors::SensitiveScanner`] 进行 PII 检测，
//! 在此之上增加"泄露拦截"（Block）和"发送前脱敏"（redact）两个职责。
//!
//! ## 与 `security::detectors` 的区别
//!
//! * `security::detectors` 是底层扫描器（正则 + 替换），无业务语义；
//! * `privacy_guard` 是 L4 价值层的一部分，决定"是否禁止"和"如何脱敏后放行"。

use crate::security::detectors::SensitiveScanner;
use serde::{Deserialize, Serialize};

/// 隐私检查结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PrivacyVerdict {
    /// 放行（未检测到敏感数据）。
    Ok,
    /// 警告（检测到敏感数据，可脱敏后放行）。
    Warn(String),
    /// 阻断（检测到红线 PII，禁止该动作）。
    Block(String),
}

/// 隐私守卫。
#[derive(Debug, Clone)]
pub struct PrivacyGuard {
    scanner: SensitiveScanner,
}

impl Default for PrivacyGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PrivacyGuard {
    pub fn new() -> Self {
        Self {
            scanner: SensitiveScanner::new(),
        }
    }

    /// 检查描述中是否携带"红线 PII"（完整身份证号等），用于 L4 评估。
    ///
    /// 规则：
    /// * 完整身份证号（18 位）→ Block
    /// * 银行卡号 / API key / Bearer token → Warn（脱敏后放行）
    /// * 其他敏感命中 → Warn
    pub fn check_leak(&self, content: &str) -> PrivacyVerdict {
        let (redacted, names) = self.scanner.scan(content);
        if names.is_empty() {
            return PrivacyVerdict::Ok;
        }
        // 红线：身份证号出现即阻断（防止 PII 进 LLM 上下文）。
        if names.contains(&"china_id") {
            return PrivacyVerdict::Block(format!(
                "检测到完整身份证号，禁止将 PII 发送给 LLM：{}",
                summarize(&redacted)
            ));
        }
        PrivacyVerdict::Warn(format!(
            "检测到敏感数据（{}），已脱敏",
            names.join(", ")
        ))
    }

    /// 发送给 LLM 之前脱敏，返回脱敏后的内容。
    pub fn redact(&self, content: &str) -> String {
        self.scanner.scan(content).0
    }
}

/// 截断过长内容用于日志展示。
fn summarize(s: &str) -> String {
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_id_number() {
        let g = PrivacyGuard::new();
        let v = g.check_leak("我的身份证是 11010119900307888X");
        assert!(matches!(v, PrivacyVerdict::Block(_)), "{v:?}");
    }

    #[test]
    fn redacts_api_key() {
        let g = PrivacyGuard::new();
        let redacted = g.redact("api_key=sk-abcdefghijklmnopqrstuvwxyz1234567890");
        assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz1234567890"));
    }

    #[test]
    fn ok_for_clean_text() {
        let g = PrivacyGuard::new();
        assert_eq!(g.check_leak("今天天气不错"), PrivacyVerdict::Ok);
    }
}
