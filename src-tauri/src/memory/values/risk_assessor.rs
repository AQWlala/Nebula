//! 风险评估器。
//!
//! 根据 [`ActionKind`] + 动作描述，给出 [`RiskLevel`] 分级。
//! 分级对应 L4 价值层的四种裁定：
//!
//! | RiskLevel      | Verdict    |
//! |----------------|------------|
//! | Safe           | Allow      |
//! | NeedsConfirm   | Confirm    |
//! | NeedsPlan      | Plan       |
//! | Forbidden      | Deny       |

use serde::{Deserialize, Serialize};

/// 动作分类（由命令层/前端推断，默认 [`Generic`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// 读取/查询（只读）。
    Read,
    /// 写入/创建（新建文件、写入记忆）。
    Write,
    /// 修改/更新（编辑已有内容）。
    Modify,
    /// 删除（单文件/单条记忆）。
    Delete,
    /// 批量删除（>1 项）。
    BulkDelete,
    /// 发送消息/邮件。
    Send,
    /// 转账/支付。
    Transfer,
    /// 执行 Shell/脚本。
    Execute,
    /// 访问外部网络。
    Network,
    /// AI 自我修改（写 SOUL.md / EvolutionEngine 写入 L2/L3/L5）。
    ///
    /// M5 #69 新增：用于 L4 审批门禁强制 High 风险分级（不可逆、
    /// 影响系统人格），需用户 diff 确认。`WorkerRiskMap` 强制映射为
    /// `RiskTier::High`，`RiskAssessor` 兜底返回 `NeedsPlan`。
    AiSelfModify,
    /// 远端 LLM 调度（用户输入将发送到远端 provider）。
    ///
    /// M5 #71 / P1-15 新增：用于 MasterDecompose（现 MasterTask）
    /// 隐私提示。`WorkerRiskMap` 强制映射为 `RiskTier::High`，
    /// 不受 autonomy_level 影响（隐私是硬约束，L5 也要提示）。
    RemoteLlmDispatch,
    /// 通用/未分类。
    Generic,
}

impl Default for ActionKind {
    fn default() -> Self {
        ActionKind::Generic
    }
}

/// 风险级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// 安全（只读、纯计算）。
    Safe,
    /// 需要准奏（不可逆但影响范围小）。
    NeedsConfirm,
    /// 需要 Plan 模式（高风险、影响范围大）。
    NeedsPlan,
    /// 禁止（应由宪法规则拦截，这里作兜底）。
    Forbidden,
}

/// 风险评估结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskVerdict {
    pub level: RiskLevel,
    pub reason: String,
    /// 0-1 风险分（供价值预测器参考）。
    pub score: f32,
}

/// 风险评估器。
#[derive(Debug, Clone, Default)]
pub struct RiskAssessor;

impl RiskAssessor {
    pub fn new() -> Self {
        Self
    }

    /// 评估单个动作的风险级别。
    pub fn assess(&self, kind: ActionKind, description: &str) -> RiskVerdict {
        use ActionKind::*;
        match kind {
            // 只读永远安全。
            Read => RiskVerdict {
                level: RiskLevel::Safe,
                reason: "只读操作".to_string(),
                score: 0.05,
            },
            // 写入/修改：默认安全，但批量或破坏性关键词升级。
            Write | Modify | Generic => {
                if has_bulk_signal(description) {
                    RiskVerdict {
                        level: RiskLevel::NeedsPlan,
                        reason: "检测到批量/大规模操作信号，建议先出 Plan".to_string(),
                        score: 0.7,
                    }
                } else {
                    RiskVerdict {
                        level: RiskLevel::Safe,
                        reason: "常规写入".to_string(),
                        score: 0.15,
                    }
                }
            }
            // 单项删除：需准奏。
            Delete => RiskVerdict {
                level: RiskLevel::NeedsConfirm,
                reason: "删除操作不可逆，需确认".to_string(),
                score: 0.6,
            },
            // 批量删除：需 Plan。
            BulkDelete => RiskVerdict {
                level: RiskLevel::NeedsPlan,
                reason: "批量删除影响范围大，需 Plan 模式".to_string(),
                score: 0.85,
            },
            // 发送：需准奏（防止误发）。
            Send => RiskVerdict {
                level: RiskLevel::NeedsConfirm,
                reason: "发送消息/邮件不可撤回，需确认".to_string(),
                score: 0.55,
            },
            // 转账/支付：需 Plan（最高风险之一）。
            Transfer => RiskVerdict {
                level: RiskLevel::NeedsPlan,
                reason: "涉及资金转账，必须 Plan 模式 + 准奏".to_string(),
                score: 0.95,
            },
            // 执行 Shell：需准奏（沙箱内可降级，但默认谨慎）。
            Execute => {
                if has_destructive_signal(description) {
                    RiskVerdict {
                        level: RiskLevel::NeedsPlan,
                        reason: "Shell 命令含破坏性信号，需 Plan".to_string(),
                        score: 0.8,
                    }
                } else {
                    RiskVerdict {
                        level: RiskLevel::NeedsConfirm,
                        reason: "执行 Shell 命令需确认".to_string(),
                        score: 0.5,
                    }
                }
            }
            // 外部网络：需准奏。
            Network => RiskVerdict {
                level: RiskLevel::NeedsConfirm,
                reason: "访问外部网络，需确认".to_string(),
                score: 0.45,
            },
            // M5 #69: AI 自我修改 — 强制 NeedsPlan + 0.9 分（高风险）
            // WorkerRiskMap 会基于 0.9 分强制映射为 RiskTier::High
            AiSelfModify => RiskVerdict {
                level: RiskLevel::NeedsPlan,
                reason: "AI 自我修改（SOUL/进化），必须 L4 审批 + 用户 diff 确认".to_string(),
                score: 0.9,
            },
            // M5 #71 / P1-15: 远端 LLM 调度 — 强制 NeedsPlan + 0.85 分（高隐私风险）
            // WorkerRiskMap 会强制映射为 RiskTier::High（不受 autonomy 影响）
            RemoteLlmDispatch => RiskVerdict {
                level: RiskLevel::NeedsPlan,
                reason: "用户输入将发送到远端 LLM provider，需隐私确认".to_string(),
                score: 0.85,
            },
        }
    }
}

/// 检测描述中是否含批量/大规模信号。
fn has_bulk_signal(desc: &str) -> bool {
    let lower = desc.to_lowercase();
    ["批量", "全部", "所有", "all", "batch", "千", "万"]
        .iter()
        .any(|k| lower.contains(k))
}

/// 检测描述中是否含破坏性 Shell 信号。
fn has_destructive_signal(desc: &str) -> bool {
    let lower = desc.to_lowercase();
    ["rm -rf", "del /", "format", "mkfs", "dd if", "shutdown", "reboot", "> /dev/sd"]
        .iter()
        .any(|k| lower.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_is_safe() {
        let a = RiskAssessor::new();
        assert_eq!(a.assess(ActionKind::Read, "x").level, RiskLevel::Safe);
    }

    #[test]
    fn delete_needs_confirm() {
        let a = RiskAssessor::new();
        assert_eq!(a.assess(ActionKind::Delete, "x").level, RiskLevel::NeedsConfirm);
    }

    #[test]
    fn transfer_needs_plan() {
        let a = RiskAssessor::new();
        assert_eq!(a.assess(ActionKind::Transfer, "x").level, RiskLevel::NeedsPlan);
    }

    #[test]
    fn bulk_write_needs_plan() {
        let a = RiskAssessor::new();
        assert_eq!(
            a.assess(ActionKind::Write, "批量更新所有记忆").level,
            RiskLevel::NeedsPlan
        );
    }

    #[test]
    fn destructive_shell_needs_plan() {
        let a = RiskAssessor::new();
        assert_eq!(
            a.assess(ActionKind::Execute, "rm -rf /tmp/old").level,
            RiskLevel::NeedsPlan
        );
    }
}
