//! T-E-S-25: 12 trace span types — 统一 `otel.kind` 标注枚举。
//!
//! 引入 [`SpanType`] 枚举(12 变体),提供 `as_otel_kind()` / `as_target()`
//! / `parse()` / `all()` / `from_target()` 辅助方法,使分布式追踪可按 span
//! 类型聚合分析。
//!
//! ## 设计约束
//!
//! * `SpanType` 独立无外部依赖(仅依赖 `serde`),不引用 `diagnostics` 模块,
//!   避免循环依赖。
//! * `parse` 容错:未知字符串返回 `None`(不 panic)。
//! * 12 个领域:chat / swarm / skill / memory / llm / reflect / acl / plan /
//!   crdt / sidecar / channel / export。
//! * 现有 213 个 `#[instrument]` 不强制重构,本枚举供后续新代码采用;
//!   `crdt_sync.rs` 已在本次补齐(12/12 领域均标注 `otel.kind`)。

use serde::{Deserialize, Serialize};

/// 12 个 trace span 领域类型,对应 `otel.kind` 标注的取值。
///
/// 每个变体映射到:
/// * 一个 `otel.kind` 字符串(如 `Chat` → `"chat"`)
/// * 一个 tracing target 前缀(如 `Chat` → `"nebula.chat"`)
///
/// 派生 `Copy + Clone + Debug + PartialEq + Eq + Serialize + Deserialize`,
/// 可作为常量求值上下文中的字段值(通过 `as_otel_kind()` 取 `&'static str`)。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanType {
    Chat,
    Swarm,
    Skill,
    Memory,
    Llm,
    Reflect,
    Acl,
    Plan,
    Crdt,
    Sidecar,
    Channel,
    Export,
}

impl SpanType {
    /// 返回该 span 类型的 `otel.kind` 字符串标识(如 `"chat"`)。
    ///
    /// 用于在 `#[instrument(fields(otel.kind = "..."))]` 中作为常量字段值。
    pub fn as_otel_kind(&self) -> &'static str {
        match self {
            SpanType::Chat => "chat",
            SpanType::Swarm => "swarm",
            SpanType::Skill => "skill",
            SpanType::Memory => "memory",
            SpanType::Llm => "llm",
            SpanType::Reflect => "reflect",
            SpanType::Acl => "acl",
            SpanType::Plan => "plan",
            SpanType::Crdt => "crdt",
            SpanType::Sidecar => "sidecar",
            SpanType::Channel => "channel",
            SpanType::Export => "export",
        }
    }

    /// 返回该 span 类型的 tracing target(如 `"nebula.chat"`)。
    ///
    /// 用于在 `#[instrument(target = "...")]` 中作为常量字段值。
    pub fn as_target(&self) -> &'static str {
        match self {
            SpanType::Chat => "nebula.chat",
            SpanType::Swarm => "nebula.swarm",
            SpanType::Skill => "nebula.skill",
            SpanType::Memory => "nebula.memory",
            SpanType::Llm => "nebula.llm",
            SpanType::Reflect => "nebula.reflect",
            SpanType::Acl => "nebula.acl",
            SpanType::Plan => "nebula.plan",
            SpanType::Crdt => "nebula.crdt",
            SpanType::Sidecar => "nebula.sidecar",
            SpanType::Channel => "nebula.channel",
            SpanType::Export => "nebula.export",
        }
    }

    /// 从字符串解析为 [`SpanType`]。
    ///
    /// 支持两种格式:
    /// * 短名:`"chat"` / `"swarm"` / ... / `"export"`
    /// * target 全名:`"nebula.chat"` / ... / `"nebula.export"`
    ///
    /// 未知字符串返回 `None`(容错,不 panic)。
    pub fn parse(s: &str) -> Option<SpanType> {
        // 优先匹配 target 全名(`nebula.<domain>`)。
        if let Some(rest) = s.strip_prefix("nebula.") {
            return Self::from_domain(rest);
        }
        // 否则按短名匹配。
        Self::from_domain(s)
    }

    /// 返回所有 12 个变体的静态切片。
    pub fn all() -> &'static [SpanType] {
        &[
            SpanType::Chat,
            SpanType::Swarm,
            SpanType::Skill,
            SpanType::Memory,
            SpanType::Llm,
            SpanType::Reflect,
            SpanType::Acl,
            SpanType::Plan,
            SpanType::Crdt,
            SpanType::Sidecar,
            SpanType::Channel,
            SpanType::Export,
        ]
    }

    /// 从 `nebula.<domain>` 形式的 target 反推 [`SpanType`]。
    ///
    /// 输入可以是完整 target(`"nebula.chat"`)或仅 domain(`"chat"`)。
    /// 未知 domain 返回 `None`。
    pub fn from_target(target: &str) -> Option<SpanType> {
        let domain = target.strip_prefix("nebula.").unwrap_or(target);
        Self::from_domain(domain)
    }

    /// 内部辅助:从 domain 短名(`"chat"`)解析。
    fn from_domain(domain: &str) -> Option<SpanType> {
        match domain {
            "chat" => Some(SpanType::Chat),
            "swarm" => Some(SpanType::Swarm),
            "skill" => Some(SpanType::Skill),
            "memory" => Some(SpanType::Memory),
            "llm" => Some(SpanType::Llm),
            "reflect" => Some(SpanType::Reflect),
            "acl" => Some(SpanType::Acl),
            "plan" => Some(SpanType::Plan),
            "crdt" => Some(SpanType::Crdt),
            "sidecar" => Some(SpanType::Sidecar),
            "channel" => Some(SpanType::Channel),
            "export" => Some(SpanType::Export),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `all()` 必须返回 12 个变体(对应 12 个领域)。
    #[test]
    fn all_returns_twelve_variants() {
        let all = SpanType::all();
        assert_eq!(all.len(), 12, "SpanType::all() must contain 12 variants");
    }

    /// `as_otel_kind` / `as_target` / `parse` / `from_target` 必须往返一致。
    #[test]
    fn roundtrip_consistency() {
        for &variant in SpanType::all() {
            // as_otel_kind → parse 往返。
            let kind = variant.as_otel_kind();
            assert_eq!(SpanType::parse(kind), Some(variant));
            // as_target → parse / from_target 往返。
            let target = variant.as_target();
            assert_eq!(SpanType::parse(target), Some(variant));
            assert_eq!(SpanType::from_target(target), Some(variant));
            // from_target 接受纯 domain 短名。
            assert_eq!(SpanType::from_target(kind), Some(variant));
        }
    }

    /// `parse` 对未知字符串返回 `None`(容错,不 panic)。
    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(SpanType::parse("unknown"), None);
        assert_eq!(SpanType::parse("nebula.unknown"), None);
        assert_eq!(SpanType::parse(""), None);
        assert_eq!(SpanType::parse("nebula."), None);
        // 大小写敏感(短名均为小写)。
        assert_eq!(SpanType::parse("Chat"), None);
        assert_eq!(SpanType::parse("CHAT"), None);
    }

    /// `as_otel_kind` 返回值各不相同(无重复)。
    #[test]
    fn as_otel_kind_unique() {
        let mut kinds: Vec<&'static str> =
            SpanType::all().iter().map(|v| v.as_otel_kind()).collect();
        let total = kinds.len();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), total, "otel.kind values must be unique");
    }

    /// `as_target` 返回值均以 `"nebula."` 前缀开头且各不相同。
    #[test]
    fn as_target_prefixed_and_unique() {
        let mut targets: Vec<&'static str> =
            SpanType::all().iter().map(|v| v.as_target()).collect();
        for t in &targets {
            assert!(
                t.starts_with("nebula."),
                "target {} must start with nebula.",
                t
            );
        }
        let total = targets.len();
        targets.sort();
        targets.dedup();
        assert_eq!(targets.len(), total, "target values must be unique");
    }

    /// spot-check 几个已知映射,防止 match 臂错位。
    #[test]
    fn known_mappings() {
        assert_eq!(SpanType::Chat.as_otel_kind(), "chat");
        assert_eq!(SpanType::Crdt.as_otel_kind(), "crdt");
        assert_eq!(SpanType::Export.as_otel_kind(), "export");
        assert_eq!(SpanType::Crdt.as_target(), "nebula.crdt");
        assert_eq!(SpanType::parse("crdt"), Some(SpanType::Crdt));
        assert_eq!(
            SpanType::parse("nebula.swarm.crdt"),
            None,
            "from_target 仅识别一级 domain,九段 target 返回 None"
        );
        // 注意:`nebula.swarm.crdt` strip_prefix 后得到 `swarm.crdt`,
        // 不匹配任何单一 domain,因此返回 None。这与 from_target 行为一致。
        assert_eq!(SpanType::from_target("nebula.swarm.crdt"), None);
    }

    /// 序列化/反序列化往返(serde derive)。
    #[test]
    fn serde_roundtrip() {
        for &variant in SpanType::all() {
            let json = serde_json::to_string(&variant).expect("serialize");
            let back: SpanType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant, "serde roundtrip failed for {:?}", variant);
        }
    }
}
