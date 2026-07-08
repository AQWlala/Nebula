//! T-E-S-27: Diagnostic events schema.
//!
//! 镜像 `swarm::events::SwarmEvent` 的设计:结构化枚举 + serde tag,
//! 供前端通过 `tauri::ipc::Channel<DiagnosticEvent>` 订阅。
//!
//! 每个变体携带 `seq: u64` 单调递增序号,前端可据此去重/排序。

use serde::{Deserialize, Serialize};

/// 诊断事件来源层。标注每条诊断由哪个子系统产生,便于前端分类显示。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOrigin {
    /// 内核(orchestrator / bootstrap)。
    Kernel,
    /// L4 价值层(准奏 / 价值裁决)。
    L4ValueLayer,
    /// ACL 拒绝。
    Acl,
    /// 注入防护命中。
    InjectionGuard,
    /// Sidecar 进程崩溃。
    Sidecar,
    /// Tracing hook 转发。
    TracingHook,
}

/// 可信级别。标识诊断信息的可信程度。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// 签名验证通过(最高可信)。
    Signed,
    /// 来自可信来源(默认级别)。
    Trusted,
    /// 未验证(仅作参考)。
    Unverified,
}

/// T-E-S-27: 诊断事件枚举。
///
/// 序列化为 JSON 后通过 `tauri::ipc::Channel` 推送给前端;前端根据
/// `kind` 字段做分支渲染。`seq` 字段为单调递增序号,由
/// [`crate::diagnostics::bus::DiagnosticsBus::emit`] 自动填充。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticEvent {
    /// L4 价值层拒绝记忆(memory_id 为被拒绝的记忆 ID)。
    L4Deny {
        memory_id: String,
        reason: String,
        seq: u64,
    },
    /// ACL 拒绝访问。
    AclRejected {
        user: String,
        resource: String,
        seq: u64,
    },
    /// 注入防护命中。
    InjectionGuardHit {
        input: String,
        pattern: String,
        seq: u64,
    },
    /// Sidecar 进程崩溃。
    SidecarCrash {
        name: String,
        exit_code: i32,
        seq: u64,
    },
    /// Tracing hook 转发的告警。
    TracingWarn {
        target: String,
        message: String,
        seq: u64,
    },
    /// 消费端 Lagged 时发出的元事件(容量 512 满了之后丢消息)。
    Dropped { count: u64, seq: u64 },
}

impl DiagnosticEvent {
    /// 返回事件的可信级别(默认 Trusted,Dropped 为 Unverified)。
    pub fn trust_level(&self) -> TrustLevel {
        match self {
            // Dropped 元事件由系统自动发出,不来自可信源。
            DiagnosticEvent::Dropped { .. } => TrustLevel::Unverified,
            _ => TrustLevel::Trusted,
        }
    }

    /// 返回事件来源。
    pub fn origin(&self) -> DiagnosticOrigin {
        match self {
            DiagnosticEvent::L4Deny { .. } => DiagnosticOrigin::L4ValueLayer,
            DiagnosticEvent::AclRejected { .. } => DiagnosticOrigin::Acl,
            DiagnosticEvent::InjectionGuardHit { .. } => DiagnosticOrigin::InjectionGuard,
            DiagnosticEvent::SidecarCrash { .. } => DiagnosticOrigin::Sidecar,
            DiagnosticEvent::TracingWarn { .. } => DiagnosticOrigin::TracingHook,
            DiagnosticEvent::Dropped { .. } => DiagnosticOrigin::Kernel,
        }
    }

    /// 返回事件的 seq 序号。
    pub fn seq(&self) -> u64 {
        match self {
            DiagnosticEvent::L4Deny { seq, .. }
            | DiagnosticEvent::AclRejected { seq, .. }
            | DiagnosticEvent::InjectionGuardHit { seq, .. }
            | DiagnosticEvent::SidecarCrash { seq, .. }
            | DiagnosticEvent::TracingWarn { seq, .. }
            | DiagnosticEvent::Dropped { seq, .. } => *seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l4_deny_serializes_with_kind_tag() {
        let evt = DiagnosticEvent::L4Deny {
            memory_id: "mem-1".to_string(),
            reason: "value conflict".to_string(),
            seq: 1,
        };
        let s = serde_json::to_string(&evt).expect("serialize should succeed");
        assert!(s.contains("\"kind\":\"l4_deny\""), "got: {s}");
        assert!(s.contains("\"memory_id\":\"mem-1\""));
        assert!(s.contains("\"reason\":\"value conflict\""));
        assert!(s.contains("\"seq\":1"));
    }

    #[test]
    fn acl_rejected_serializes_with_kind_tag() {
        let evt = DiagnosticEvent::AclRejected {
            user: "alice".to_string(),
            resource: "mem://x".to_string(),
            seq: 2,
        };
        let s = serde_json::to_string(&evt).expect("serialize should succeed");
        assert!(s.contains("\"kind\":\"acl_rejected\""));
        assert!(s.contains("\"user\":\"alice\""));
    }

    #[test]
    fn injection_guard_hit_serializes_with_kind_tag() {
        let evt = DiagnosticEvent::InjectionGuardHit {
            input: "ignore previous".to_string(),
            pattern: "ignore_prev".to_string(),
            seq: 3,
        };
        let s = serde_json::to_string(&evt).expect("serialize should succeed");
        assert!(s.contains("\"kind\":\"injection_guard_hit\""));
        assert!(s.contains("\"pattern\":\"ignore_prev\""));
    }

    #[test]
    fn sidecar_crash_serializes_with_kind_tag() {
        let evt = DiagnosticEvent::SidecarCrash {
            name: "memory-sidecar".to_string(),
            exit_code: 137,
            seq: 4,
        };
        let s = serde_json::to_string(&evt).expect("serialize should succeed");
        assert!(s.contains("\"kind\":\"sidecar_crash\""));
        assert!(s.contains("\"exit_code\":137"));
    }

    #[test]
    fn tracing_warn_serializes_with_kind_tag() {
        let evt = DiagnosticEvent::TracingWarn {
            target: "nebula.diagnostic".to_string(),
            message: "slow query".to_string(),
            seq: 5,
        };
        let s = serde_json::to_string(&evt).expect("serialize should succeed");
        assert!(s.contains("\"kind\":\"tracing_warn\""));
        assert!(s.contains("\"target\":\"nebula.diagnostic\""));
    }

    #[test]
    fn dropped_serializes_with_kind_tag() {
        let evt = DiagnosticEvent::Dropped { count: 10, seq: 6 };
        let s = serde_json::to_string(&evt).expect("serialize should succeed");
        assert!(s.contains("\"kind\":\"dropped\""));
        assert!(s.contains("\"count\":10"));
    }

    #[test]
    fn origin_and_trust_level_match() {
        assert_eq!(
            DiagnosticEvent::L4Deny {
                memory_id: "x".into(),
                reason: "y".into(),
                seq: 1
            }
            .origin(),
            DiagnosticOrigin::L4ValueLayer
        );
        assert_eq!(
            DiagnosticEvent::Dropped { count: 1, seq: 1 }.trust_level(),
            TrustLevel::Unverified
        );
        assert_eq!(
            DiagnosticEvent::L4Deny {
                memory_id: "x".into(),
                reason: "y".into(),
                seq: 1
            }
            .trust_level(),
            TrustLevel::Trusted
        );
    }

    #[test]
    fn seq_accessor_works_for_all_variants() {
        assert_eq!(
            DiagnosticEvent::L4Deny {
                memory_id: "x".into(),
                reason: "y".into(),
                seq: 42
            }
            .seq(),
            42
        );
        assert_eq!(DiagnosticEvent::Dropped { count: 1, seq: 99 }.seq(), 99);
    }
}
