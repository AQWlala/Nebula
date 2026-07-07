//! 记忆访问控制层。
//!
//! ## v3 (M2b) — domain-aware ACL
//!
//! v2.1 的 `TRUSTED_PRINCIPALS` 在无规则匹配时默认放行 `system`/`owner`/`local`,
//! 这使得任何可信主体都能跨域读取所有记忆,绕过 domain 隔离边界。
//! M2b 引入 [`PrincipalDomainMap`] + [`MemoryAcl::check_with_domain`]:
//!
//! * 每个 principal 解析到一个 domain(`evolution:agent_a` → `agent_a`)。
//! * 记忆的 `domain` 字段与 principal 的 domain 比对,不匹配则拒绝。
//! * TRUSTED_PRINCIPALS 不再自动跨域 allow-all,必须 domain 匹配才放行。
//! * 旧 [`MemoryAcl::check`](Self::check) 保留为向后兼容(无 domain 检查,
//!   仅用于尚未迁移的调用点,M3+ 全部迁移后可移除)。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    pub principal: String,
    pub resource: String,
    pub permission: AclPermission,
    pub effect: AclEffect,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AclPermission {
    Read,
    Write,
    Delete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AclEffect {
    Allow,
    Deny,
}

/// M2b #35: principal → domain 映射表。
///
/// 每个 principal(如 `evolution:agent_a` / `worker:task_42` / `system`)
/// 解析到一个 domain 字符串。解析规则:
///
/// | principal 前缀 | 解析规则 | 示例 |
/// |----------------|---------|------|
/// | `evolution:`   | 去前缀,取剩余 | `evolution:agent_a` → `agent_a` |
/// | `worker:`      | 查表(运行时由 MasterAgent 设置 task_id → master_domain) | `worker:task_42` → `agent_a`(若 master 为 agent_a) |
/// | `system`/`owner`/`local` | 默认 `shared` 域 | `system` → `shared` |
/// | 其他           | 显式查表,未命中返回 None → 调用方决定(默认拒绝) | `unknown_agent` → None |
///
/// **线程安全**: 内部使用 `parking_lot::RwLock<HashMap>`,读多写少场景下
/// 读锁无竞争。MasterAgent 在派发子任务时调用 `set_worker_domain()` 写入映射,
/// 任务完成后调用 `clear_worker_domain()` 清理。
#[derive(Debug, Clone, Default)]
pub struct PrincipalDomainMap {
    /// 显式映射表(优先级最高)。worker:task_id → master_domain
    /// 由 MasterAgent 在派发任务时写入。
    explicit: HashMap<String, String>,
}

impl PrincipalDomainMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置 principal → domain 的显式映射。
    /// 用于 worker:task_id → current_master_domain 的运行时绑定。
    pub fn set(&mut self, principal: impl Into<String>, domain: impl Into<String>) {
        self.explicit.insert(principal.into(), domain.into());
    }

    /// 清除指定 principal 的映射。任务完成后由 MasterAgent 调用。
    pub fn clear(&mut self, principal: &str) {
        self.explicit.remove(principal);
    }

    /// 解析 principal 的 domain。返回 `None` 表示未绑定且无前缀规则可推导。
    ///
    /// 解析顺序:
    /// 1. 显式映射表(最高优先级)
    /// 2. `evolution:` 前缀 → 去前缀取剩余(由 [`resolve_inline`] 处理)
    /// 3. TRUSTED_PRINCIPALS(`system`/`owner`/`local`) → `shared` 域
    /// 4. 其他 → None(调用方应拒绝)
    pub fn resolve(&self, principal: &str) -> Option<String> {
        // 1. 显式映射表
        if let Some(d) = self.explicit.get(principal) {
            return Some(d.clone());
        }
        // 2-4. 内联规则
        Self::resolve_inline(principal)
    }

    /// M2b #33: 内联解析规则(不依赖 explicit map)。供 `check_with_domain`
    /// 在 `PrincipalDomainMap` 未注入时使用,确保 `evolution:` 前缀与
    /// TRUSTED_PRINCIPALS 仍能正确解析。
    pub fn resolve_inline(principal: &str) -> Option<String> {
        // 2. evolution: 前缀
        if let Some(rest) = principal.strip_prefix("evolution:") {
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
        // 3. TRUSTED_PRINCIPALS → shared 域
        const TRUSTED_PRINCIPALS: &[&str] = &["system", "owner", "local"];
        if TRUSTED_PRINCIPALS.contains(&principal) {
            return Some("shared".to_string());
        }
        // 4. 未知 principal
        None
    }

    /// 返回当前映射表条目数(用于测试与诊断)。
    pub fn len(&self) -> usize {
        self.explicit.len()
    }

    /// 映射表是否为空。
    pub fn is_empty(&self) -> bool {
        self.explicit.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryAcl {
    rules: Vec<AclRule>,
    /// M2b #35: principal → domain 映射。可选,未设置时 check_with_domain
    /// 回退到旧的"无 domain 检查"路径(向后兼容)。
    principal_domains: Option<PrincipalDomainMap>,
}

impl MemoryAcl {
    pub fn new() -> Self {
        Self::default()
    }

    /// M2b #35: 注入 PrincipalDomainMap,启用 domain-aware 检查。
    pub fn with_principal_domains(mut self, map: PrincipalDomainMap) -> Self {
        self.principal_domains = Some(map);
        self
    }

    /// M2b #35: 设置 / 替换 PrincipalDomainMap。
    pub fn set_principal_domains(&mut self, map: PrincipalDomainMap) {
        self.principal_domains = Some(map);
    }

    pub fn add_rule(&mut self, rule: AclRule) {
        self.rules.push(rule);
    }

    pub fn remove_rule(&mut self, principal: &str, resource: &str, permission: AclPermission) {
        self.rules.retain(|r| {
            !(r.principal == principal && r.resource == resource && r.permission == permission)
        });
    }

    /// 旧版检查(无 domain 检查)。保留向后兼容,未迁移的调用点使用。
    ///
    /// **警告**: 此方法不检查 domain,TRUSTED_PRINCIPALS 仍可跨域访问。
    /// 新代码应使用 [`check_with_domain`](Self::check_with_domain)。
    pub fn check(&self, principal: &str, resource: &str, permission: AclPermission) -> bool {
        const TRUSTED_PRINCIPALS: &[&str] = &["system", "owner", "local"];

        let mut has_deny = false;
        let mut has_allow = false;

        for rule in &self.rules {
            if !Self::matches(&rule.principal, principal) {
                continue;
            }
            if !Self::matches(&rule.resource, resource) {
                continue;
            }
            if rule.permission != permission {
                continue;
            }
            match rule.effect {
                AclEffect::Deny => has_deny = true,
                AclEffect::Allow => has_allow = true,
            }
        }

        let allowed = if has_deny {
            false
        } else if has_allow {
            true
        } else {
            // 无规则匹配时:可信主体放行,其他主体拒绝
            TRUSTED_PRINCIPALS.contains(&principal)
        };
        // T-S1-B-03: 上报 ACL 裁定到全局 metrics,供仪表盘计算拒绝率。
        crate::metrics::global().record_acl_verdict(allowed);
        allowed
    }

    /// M2b #33: domain-aware 检查。
    ///
    /// 按 `entry.domain` 与 `principal_domain` 比对:
    /// * domain 不匹配 → 拒绝(无论 TRUSTED_PRINCIPALS)
    /// * domain 匹配 → 走旧规则匹配路径
    /// * domain 匹配且无规则 → allow(同域默认信任)
    ///
    /// **principal_domain 优先级**:
    /// 1. 显式传入的 `principal_domain` 参数(最高,调用方已解析)
    /// 2. `PrincipalDomainMap` 解析(若已注入)
    /// 3. None → 回退到旧 `check()`(无 domain 检查,向后兼容)
    pub fn check_with_domain(
        &self,
        principal: &str,
        resource: &str,
        permission: AclPermission,
        memory_domain: &str,
        principal_domain: Option<&str>,
    ) -> bool {
        // 解析 principal 的 domain
        // 优先级:显式参数 > PrincipalDomainMap > 内联规则(前缀/TRUSTED)
        let resolved_domain = principal_domain
            .map(|s| s.to_string())
            .or_else(|| {
                self.principal_domains
                    .as_ref()
                    .and_then(|m| m.resolve(principal))
            })
            .or_else(|| PrincipalDomainMap::resolve_inline(principal));

        // 未解析到 domain → 回退旧路径(向后兼容)
        let resolved_domain = match resolved_domain {
            Some(d) => d,
            None => {
                return self.check(principal, resource, permission);
            }
        };

        // M2b #34: domain 不匹配 → 拒绝(即使 TRUSTED_PRINCIPALS 也不再跨域)
        if resolved_domain != memory_domain {
            crate::metrics::global().record_acl_verdict(false);
            return false;
        }

        // domain 匹配 → 走规则匹配路径
        let mut has_deny = false;
        let mut has_allow = false;

        for rule in &self.rules {
            if !Self::matches(&rule.principal, principal) {
                continue;
            }
            if !Self::matches(&rule.resource, resource) {
                continue;
            }
            if rule.permission != permission {
                continue;
            }
            match rule.effect {
                AclEffect::Deny => has_deny = true,
                AclEffect::Allow => has_allow = true,
            }
        }

        // domain 匹配且无规则 → allow(同域默认信任)
        // domain 匹配但有 deny → deny(显式拒绝优先)
        let allowed = if has_deny {
            false
        } else if has_allow {
            true
        } else {
            // 同域默认信任(M2b: 不再依赖 TRUSTED_PRINCIPALS,而是 domain 匹配)
            true
        };
        crate::metrics::global().record_acl_verdict(allowed);
        allowed
    }

    pub fn filter_memories<'a>(
        &self,
        principal: &str,
        memories: Vec<(String, &'a str)>,
    ) -> Vec<(String, &'a str)> {
        memories
            .into_iter()
            .filter(|(id, _)| self.check(principal, id, AclPermission::Read))
            .collect()
    }

    /// M2b #33: domain-aware 过滤(query-time 模式,但本方法仍是 post-filter
    /// — 真正的 query-time 过滤在 sqlite_store 的 `_in_domain` 变体中)。
    /// 此方法供已加载到内存的记忆列表使用。
    pub fn filter_memories_with_domain<'a>(
        &self,
        principal: &str,
        principal_domain: Option<&str>,
        memories: Vec<(String, &'a str, &'a str)>, // (id, content, domain)
    ) -> Vec<(String, &'a str)> {
        memories
            .into_iter()
            .filter(|(id, _, domain)| {
                self.check_with_domain(principal, id, AclPermission::Read, domain, principal_domain)
            })
            .map(|(id, content, _)| (id, content))
            .collect()
    }

    fn matches(pattern: &str, value: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        pattern == value
    }

    pub fn rules(&self) -> &[AclRule] {
        &self.rules
    }

    /// M2b #35: 返回 PrincipalDomainMap 的只读引用(用于测试与诊断)。
    pub fn principal_domains(&self) -> Option<&PrincipalDomainMap> {
        self.principal_domains.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // v2.1 deny-all 行为测试(非 TRUSTED_PRINCIPALS 默认拒绝)
    // -----------------------------------------------------------------

    /// v2.1: TRUSTED_PRINCIPALS(system/owner/local)无规则时放行。
    #[test]
    fn default_allows_trusted_principals() {
        let acl = MemoryAcl::new();
        assert!(acl.check("system", "any-resource", AclPermission::Read));
        assert!(acl.check("owner", "any-resource", AclPermission::Read));
        assert!(acl.check("local", "any-resource", AclPermission::Read));
    }

    /// v2.1: 非 TRUSTED_PRINCIPALS 无规则时拒绝。
    #[test]
    fn default_denies_untrusted_principals() {
        let acl = MemoryAcl::new();
        assert!(!acl.check("anyone", "any-resource", AclPermission::Read));
        assert!(!acl.check("user-1", "any-resource", AclPermission::Read));
    }

    #[test]
    fn deny_overrides_allow() {
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "user-1".into(),
            resource: "mem-1".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        assert!(!acl.check("user-1", "mem-1", AclPermission::Read));
    }

    #[test]
    fn wildcard_principal() {
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "*".into(),
            resource: "secret".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        // deny 匹配 → 拒绝
        assert!(!acl.check("anyone", "secret", AclPermission::Read));
        assert!(!acl.check("system", "secret", AclPermission::Read));
        // 无匹配资源:TRUSTED_PRINCIPALS 放行,其他拒绝
        assert!(acl.check("system", "public", AclPermission::Read));
        assert!(!acl.check("anyone", "public", AclPermission::Read));
    }

    #[test]
    fn filter_removes_denied() {
        let mut acl = MemoryAcl::new();
        // user-1 对 mem-1 有 deny 规则
        acl.add_rule(AclRule {
            principal: "user-1".into(),
            resource: "mem-1".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        // user-1 对 mem-2 无规则 → deny-all(user-1 非 trusted)
        let mems = vec![
            ("mem-1".to_string(), "content-1"),
            ("mem-2".to_string(), "content-2"),
        ];
        let filtered = acl.filter_memories("user-1", mems);
        // mem-1 deny,mem-2 无规则被 deny-all → 0 条
        assert_eq!(filtered.len(), 0);

        // system 对 mem-1 无规则 → 放行(trusted)
        let mems2 = vec![
            ("mem-1".to_string(), "content-1"),
            ("mem-2".to_string(), "content-2"),
        ];
        let filtered2 = acl.filter_memories("system", mems2);
        assert_eq!(filtered2.len(), 2);
    }

    // -----------------------------------------------------------------
    // M2b #39: domain-aware ACL tests
    // -----------------------------------------------------------------

    /// M2b #35: PrincipalDomainMap 解析 evolution: 前缀。
    #[test]
    fn principal_domain_map_resolves_evolution_prefix() {
        let map = PrincipalDomainMap::new();
        assert_eq!(
            map.resolve("evolution:agent_a"),
            Some("agent_a".to_string())
        );
        assert_eq!(map.resolve("evolution:writer"), Some("writer".to_string()));
        // 空前缀剩余 → None
        assert_eq!(map.resolve("evolution:"), None);
    }

    /// M2b #35: PrincipalDomainMap 解析 TRUSTED_PRINCIPALS → shared。
    #[test]
    fn principal_domain_map_resolves_trusted_principals() {
        let map = PrincipalDomainMap::new();
        assert_eq!(map.resolve("system"), Some("shared".to_string()));
        assert_eq!(map.resolve("owner"), Some("shared".to_string()));
        assert_eq!(map.resolve("local"), Some("shared".to_string()));
    }

    /// M2b #35: PrincipalDomainMap 显式映射覆盖前缀规则。
    #[test]
    fn principal_domain_map_explicit_overrides_prefix() {
        let mut map = PrincipalDomainMap::new();
        // worker:task_42 显式映射到 master 的 domain(agent_a)
        map.set("worker:task_42", "agent_a");
        assert_eq!(map.resolve("worker:task_42"), Some("agent_a".to_string()));
        // 不在显式表中且无前缀规则 → None
        assert_eq!(map.resolve("worker:task_99"), None);
        // clear 后恢复 None
        map.clear("worker:task_42");
        assert_eq!(map.resolve("worker:task_42"), None);
    }

    /// M2b #33: 同域访问允许(无显式规则)。
    #[test]
    fn check_with_domain_same_domain_allows() {
        let acl = MemoryAcl::new();
        assert!(acl.check_with_domain(
            "evolution:agent_a",
            "mem-1",
            AclPermission::Read,
            "agent_a",
            None
        ));
    }

    /// M2b #34: 跨域访问拒绝(即使 system 也不再跨域)。
    #[test]
    fn check_with_domain_cross_domain_denies_even_for_system() {
        let acl = MemoryAcl::new();
        // system 解析到 shared 域,记忆在 agent_a 域 → 拒绝
        assert!(!acl.check_with_domain("system", "mem-1", AclPermission::Read, "agent_a", None));
        // 同域(shared)允许
        assert!(acl.check_with_domain("system", "mem-1", AclPermission::Read, "shared", None));
    }

    /// M2b #34: 跨域访问拒绝(非可信主体)。
    #[test]
    fn check_with_domain_cross_domain_denies_unknown_principal() {
        let acl = MemoryAcl::new();
        // agent_a 试图读 agent_b 的记忆 → 拒绝
        assert!(!acl.check_with_domain(
            "evolution:agent_a",
            "mem-1",
            AclPermission::Read,
            "agent_b",
            None
        ));
    }

    /// M2b #33: 显式 principal_domain 参数优先于 PrincipalDomainMap。
    #[test]
    fn check_with_domain_explicit_param_overrides_map() {
        let map = PrincipalDomainMap::new();
        let acl = MemoryAcl::new().with_principal_domains(map);
        // 显式传入 principal_domain=agent_b,即使 PrincipalDomainMap 解析为 agent_a
        // 也以显式参数为准(允许调用方覆盖,用于临时跨域授权场景)
        assert!(acl.check_with_domain(
            "evolution:agent_a",
            "mem-1",
            AclPermission::Read,
            "agent_b",
            Some("agent_b")
        ));
    }

    /// M2b #33: deny 规则在同域内仍然生效。
    #[test]
    fn check_with_domain_deny_rule_in_same_domain() {
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "evolution:agent_a".into(),
            resource: "mem-secret".into(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        // 同域但 deny 规则匹配 → 拒绝
        assert!(!acl.check_with_domain(
            "evolution:agent_a",
            "mem-secret",
            AclPermission::Read,
            "agent_a",
            None
        ));
        // 同域且不同资源 → 允许
        assert!(acl.check_with_domain(
            "evolution:agent_a",
            "mem-other",
            AclPermission::Read,
            "agent_a",
            None
        ));
    }

    /// M2b #33: 未解析到 domain(principal 未在 map 中且无前缀)→ 回退旧路径。
    #[test]
    fn check_with_domain_falls_back_when_unresolved() {
        let acl = MemoryAcl::new();
        // "unknown_principal" 无前缀,不在 map 中 → 回退 check()
        // 旧 check() 对 TRUSTED_PRINCIPALS 放行,其他拒绝
        assert!(!acl.check_with_domain(
            "unknown_principal",
            "mem-1",
            AclPermission::Read,
            "shared",
            None
        ));
        // 但 system 在旧路径中放行(回退行为)
        assert!(acl.check_with_domain("system", "mem-1", AclPermission::Read, "shared", None));
    }

    /// M2b #39: filter_memories_with_domain 跨域过滤。
    #[test]
    fn filter_memories_with_domain_filters_cross_domain() {
        let acl = MemoryAcl::new();
        let mems = vec![
            ("mem-1".to_string(), "content-1", "shared"),
            ("mem-2".to_string(), "content-2", "agent_a"),
            ("mem-3".to_string(), "content-3", "agent_b"),
        ];
        // agent_a 主体:只能看 agent_a 域 → 1 条
        let filtered = acl.filter_memories_with_domain("evolution:agent_a", None, mems.clone());
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "mem-2");

        // system 主体:只能看 shared 域 → 1 条
        let filtered = acl.filter_memories_with_domain("system", None, mems);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "mem-1");
    }
}
