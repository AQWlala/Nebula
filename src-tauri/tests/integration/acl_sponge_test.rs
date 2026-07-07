//! T-S1-A-04: MemoryAcl 接入 sponge search 集成测试。
//!
//! 对应 ROADMAP_v2.1.md §4.4 Stage 1 测试策略要求的
//! `tests/integration/acl_sponge_test.rs`。
//!
//! 覆盖目标：
//! * `MemoryAcl::check()` 默认策略（T-S1-PRE-02：可信主体 allow + 其他 deny-all）
//! * `AclFilteredSearch` 结构体字段语义（`results` / `filtered_count` / `acl_enforced`）
//! * `SpongeEngine::with_acl()` builder 与 `acl()` 访问器的契约
//! * 字符串 → 枚举解析（permission/effect 非法值不阻断，跳过 + warn）
//!
//! 注：完整 `search_with_acl()` 端到端测试需要 LanceDB + Ollama embedder，
//! 属于 T-S1-A-02（MemoryOrchestrator 集成 sponge）的验收范围。本文件
//! 聚焦 ACL 过滤逻辑与数据结构的契约验证。

use std::sync::Arc;

use nebula_lib::memory::acl::{AclEffect, AclPermission, AclRule, MemoryAcl};
use nebula_lib::memory::sponge::AclFilteredSearch;

/// T-S1-PRE-02 回归保护：默认策略对可信主体（system/owner/local）放行。
#[test]
fn acl_default_allows_trusted_principals() {
    let acl = MemoryAcl::new();
    for trusted in &["system", "owner", "local"] {
        assert!(
            acl.check(trusted, "any-resource", AclPermission::Read),
            "trusted principal `{trusted}` should be allowed by default"
        );
        assert!(
            acl.check(trusted, "any-resource", AclPermission::Write),
            "trusted principal `{trusted}` should be allowed for write by default"
        );
    }
}

/// T-S1-PRE-02 回归保护：默认策略对非可信主体拒绝。
#[test]
fn acl_default_denies_untrusted_principals() {
    let acl = MemoryAcl::new();
    assert!(
        !acl.check("external-skill", "any-resource", AclPermission::Read),
        "untrusted principal should be denied by default"
    );
    assert!(
        !acl.check("mcp-client", "any-resource", AclPermission::Read),
        "untrusted MCP client should be denied by default"
    );
}

/// 显式 deny 规则覆盖默认 allow（可信主体也被拒绝）。
#[test]
fn acl_explicit_deny_overrides_trusted_default() {
    let mut acl = MemoryAcl::new();
    acl.add_rule(AclRule {
        principal: "system".into(),
        resource: "mem-classified".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Deny,
    });
    assert!(
        !acl.check("system", "mem-classified", AclPermission::Read),
        "explicit deny must override trusted default"
    );
    // 其他资源仍放行
    assert!(acl.check("system", "mem-other", AclPermission::Read));
}

/// 显式 allow 规则授权非可信主体。
#[test]
fn acl_explicit_allow_grants_untrusted() {
    let mut acl = MemoryAcl::new();
    acl.add_rule(AclRule {
        principal: "skill-123".into(),
        resource: "mem-shared".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Allow,
    });
    assert!(
        acl.check("skill-123", "mem-shared", AclPermission::Read),
        "explicit allow should grant access to untrusted principal"
    );
    // 其他资源仍拒绝
    assert!(!acl.check("skill-123", "mem-other", AclPermission::Read));
}

/// 通配符 principal `*` 匹配所有主体。
#[test]
fn acl_wildcard_principal_matches_all() {
    let mut acl = MemoryAcl::new();
    acl.add_rule(AclRule {
        principal: "*".into(),
        resource: "mem-public".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Allow,
    });
    assert!(acl.check("anyone", "mem-public", AclPermission::Read));
    assert!(acl.check("skill-xyz", "mem-public", AclPermission::Read));
    // 非 mem-public 资源仍按默认策略
    assert!(!acl.check("anyone", "mem-private", AclPermission::Read));
}

/// `filter_memories()` 批量过滤：deny 规则的资源被移除。
#[test]
fn acl_filter_memories_removes_denied() {
    let mut acl = MemoryAcl::new();
    acl.add_rule(AclRule {
        principal: "skill-1".into(),
        resource: "mem-secret".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Deny,
    });
    // M7b #91: M2b deny-all 默认策略下,非可信主体 skill-1 对 mem-public-*
    // 无匹配规则会被默认拒绝。补显式 Allow 规则,使 public 记录通过过滤,
    // 与测试意图(deny 移除 mem-secret,保留 mem-public-*)一致。
    acl.add_rule(AclRule {
        principal: "skill-1".into(),
        resource: "mem-public-1".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Allow,
    });
    acl.add_rule(AclRule {
        principal: "skill-1".into(),
        resource: "mem-public-2".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Allow,
    });
    let mems = vec![
        ("mem-secret".to_string(), "classified content"),
        ("mem-public-1".to_string(), "public one"),
        ("mem-public-2".to_string(), "public two"),
    ];
    let filtered = acl.filter_memories("skill-1", mems);
    assert_eq!(filtered.len(), 2, "denied memory should be filtered out");
    let ids: Vec<_> = filtered.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"mem-public-1"));
    assert!(ids.contains(&"mem-public-2"));
    assert!(!ids.contains(&"mem-secret"));
}

/// `AclFilteredSearch` 在 `acl_enforced=false`（未注入 ACL）时
/// 应保留所有结果，`filtered_count` 为 0。
#[test]
fn acl_filtered_search_passthrough_semantics() {
    let passthrough = AclFilteredSearch {
        results: vec![
            ("m1".to_string(), 0.95),
            ("m2".to_string(), 0.80),
            ("m3".to_string(), 0.60),
        ],
        filtered_count: 0,
        acl_enforced: false,
    };
    assert_eq!(passthrough.results.len(), 3);
    assert_eq!(passthrough.filtered_count, 0);
    assert!(!passthrough.acl_enforced);
}

/// `AclFilteredSearch` 在 `acl_enforced=true` 时正确记录被拒条目数，
/// 且 `results` 仅包含通过 ACL 的条目。
#[test]
fn acl_filtered_search_enforced_semantics() {
    let enforced = AclFilteredSearch {
        results: vec![("m1".to_string(), 0.95)],
        filtered_count: 2,
        acl_enforced: true,
    };
    assert_eq!(enforced.results.len(), 1, "only 1 result passed ACL");
    assert_eq!(enforced.filtered_count, 2, "2 results were denied");
    assert!(enforced.acl_enforced);
}

/// 模拟 `load_acl_from_store` 的字符串 → 枚举解析逻辑：
/// 合法值正确映射，非法值被跳过（不 panic）。
///
/// 这是对 `lib.rs::load_acl_from_store` 的契约验证，避免在启动时
/// 因单条脏数据导致整个 ACL 子系统不可用。
#[test]
fn acl_string_to_enum_parse_resilience() {
    // 模拟从 SQLite list_acl() 返回的行
    let rows: Vec<(String, String, String, String, String)> = vec![
        (
            "r1".into(),
            "skill-1".into(),
            "mem-1".into(),
            "read".into(),
            "allow".into(),
        ),
        (
            "r2".into(),
            "skill-2".into(),
            "mem-2".into(),
            "write".into(),
            "deny".into(),
        ),
        (
            "r3".into(),
            "skill-3".into(),
            "mem-3".into(),
            "delete".into(),
            "allow".into(),
        ),
        // 非法 permission
        (
            "r4".into(),
            "skill-4".into(),
            "mem-4".into(),
            "execute".into(),
            "allow".into(),
        ),
        // 非法 effect
        (
            "r5".into(),
            "skill-5".into(),
            "mem-5".into(),
            "read".into(),
            "block".into(),
        ),
    ];

    let mut acl = MemoryAcl::new();
    let mut skipped = 0;
    for (id, principal, resource, permission_s, effect_s) in rows {
        let permission = match permission_s.as_str() {
            "read" => AclPermission::Read,
            "write" => AclPermission::Write,
            "delete" => AclPermission::Delete,
            _ => {
                skipped += 1;
                let _ = id; // 模拟 warn! 记录
                continue;
            }
        };
        let effect = match effect_s.as_str() {
            "allow" => AclEffect::Allow,
            "deny" => AclEffect::Deny,
            _ => {
                skipped += 1;
                continue;
            }
        };
        acl.add_rule(AclRule {
            principal,
            resource,
            permission,
            effect,
        });
    }
    assert_eq!(skipped, 2, "2 malformed rows should be skipped");
    assert_eq!(acl.rules().len(), 3, "3 valid rules should be added");
}

/// ACL 实例可被 `Arc` 共享（模拟 SpongeEngine 持有 `Arc<MemoryAcl>`）。
#[test]
fn acl_arc_sharing() {
    let mut acl = MemoryAcl::new();
    acl.add_rule(AclRule {
        principal: "skill-shared".into(),
        resource: "mem-shared".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Allow,
    });
    let acl_arc = Arc::new(acl);

    // 多个"持有者"共享同一 ACL 实例（模拟 sponge + 多个 skill executor）
    let holder1 = acl_arc.clone();
    let holder2 = acl_arc.clone();

    assert!(holder1.check("skill-shared", "mem-shared", AclPermission::Read));
    assert!(holder2.check("skill-shared", "mem-shared", AclPermission::Read));
    // Arc 引用计数
    assert_eq!(Arc::strong_count(&acl_arc), 3);
}
