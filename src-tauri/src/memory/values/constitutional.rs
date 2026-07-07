//! Constitutional AI 规则引擎。
//!
//! 维护一组 [`ConstitutionalRule`]，按 [`RuleSeverity`] 分级。
//! `Deny` 级规则命中即禁止；`Warn` 级规则命中仅记录。
//!
//! 默认规则集（`default_rules`）覆盖设计文档"风险表"中的灾难级操作：
//! 格式化磁盘、清空数据库、向不受信地址转账、批量删除用户数据等。

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 规则严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleSeverity {
    /// 命中即禁止（Deny）。
    Deny,
    /// 命中仅警告（记录但不阻断）。
    Warn,
}

/// 单条宪法规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionalRule {
    /// 规则名称（展示用）。
    pub name: String,
    /// 规则说明。
    pub description: String,
    /// 严重级别。
    pub severity: RuleSeverity,
    /// 匹配模式（正则表达式，大小写不敏感）。
    pub pattern: String,
    /// 编译后的正则（运行时构建，不序列化）。
    #[serde(skip)]
    pub compiled: Option<Regex>,
}

impl ConstitutionalRule {
    /// 构建一条规则并编译正则。
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        severity: RuleSeverity,
        pattern: &str,
    ) -> Self {
        let compiled = Regex::new(&format!("(?i){pattern}")).ok();
        Self {
            name: name.into(),
            description: description.into(),
            severity,
            pattern: pattern.to_string(),
            compiled,
        }
    }

    /// 判断描述是否命中本规则。
    pub fn matches(&self, description: &str) -> bool {
        self.compiled
            .as_ref()
            .map(|r| r.is_match(description))
            .unwrap_or(false)
    }
}

/// 宪法规则集。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConstitutionalRules {
    rules: Vec<ConstitutionalRule>,
}

impl ConstitutionalRules {
    /// 默认规则集（覆盖灾难级操作）。
    pub fn default_rules() -> Self {
        let deny = [
            // 磁盘/数据破坏
            (
                "disk_format",
                "格式化磁盘",
                r"格式化.*(磁盘|硬盘|[cd]\s*盘|系统盘)|(format|mkfs)\s+/",
            ),
            (
                "db_drop",
                "删除/清空数据库",
                r"(drop|truncate)\s+(database|table|schema)|清空(数据库|全部数据)|删除(所有|全部).*(记忆|数据)",
            ),
            // 不可逆批量删除
            (
                "bulk_delete",
                "批量删除用户数据",
                r"(rm\s+-rf|del\s+/[sqa]|删除).*(\/|\\).*\*|批量删除.*文件",
            ),
            // 转账/支付红线
            (
                "transfer_untrusted",
                "向不受信任地址转账/支付",
                r"(转账|付款|支付|汇款).*(陌生|未知|不受信任|untrusted).*地址|向.*钱包.*转账",
            ),
            // 隐私外泄
            (
                "exfiltrate_pii",
                "外泄敏感数据",
                r"(上传|发送|上传到|提交到).*(身份证|银行卡|密码|私钥|secret|private key).*(云端|服务器|第三方)",
            ),
            // 系统破坏
            (
                "system_wipe",
                "破坏操作系统",
                r"(删除|清空|覆盖).*(系统|system|注册表|registry|bootloader|引导)",
            ),
        ];
        let warn = [
            (
                "network_external",
                "访问外部网络",
                r"(curl|wget|http|https|fetch).*(api|endpoint|远程|external)",
            ),
            (
                "shell_exec",
                "执行 Shell 命令",
                r"(执行|运行)\s*(shell|bash|cmd|powershell|脚本)",
            ),
            (
                "large_batch",
                "大批量操作（>100 项）",
                r"(批量|batch).*(100|千|万|所有|all)",
            ),
        ];
        let rules = deny
            .iter()
            .map(|(n, d, p)| ConstitutionalRule::new(*n, *d, RuleSeverity::Deny, p))
            .chain(
                warn.iter()
                    .map(|(n, d, p)| ConstitutionalRule::new(*n, *d, RuleSeverity::Warn, p)),
            )
            .collect();
        Self { rules }
    }

    /// 返回命中的第一条 `Deny` 规则（若有）。
    pub fn match_deny(&self, description: &str) -> Option<&ConstitutionalRule> {
        self.rules
            .iter()
            .find(|r| r.severity == RuleSeverity::Deny && r.matches(description))
    }

    /// 返回所有命中的 `Warn` 规则。
    pub fn match_warnings(&self, description: &str) -> Vec<&ConstitutionalRule> {
        self.rules
            .iter()
            .filter(|r| r.severity == RuleSeverity::Warn && r.matches(description))
            .collect()
    }

    /// 追加一条自定义规则（用于运行时配置）。
    pub fn add(&mut self, rule: ConstitutionalRule) {
        self.rules.push(rule);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules_deny_format() {
        let rules = ConstitutionalRules::default_rules();
        assert!(rules.match_deny("请帮我格式化 C 盘").is_some());
        assert!(rules.match_deny("DROP TABLE memories").is_some());
    }

    #[test]
    fn default_rules_warn_network() {
        let rules = ConstitutionalRules::default_rules();
        assert!(rules.match_deny("帮我 curl 一个 api").is_none());
        assert!(!rules.match_warnings("帮我 curl 一个 api").is_empty());
    }

    #[test]
    fn default_rules_allow_safe() {
        let rules = ConstitutionalRules::default_rules();
        assert!(rules.match_deny("帮我写一份周报").is_none());
    }
}
