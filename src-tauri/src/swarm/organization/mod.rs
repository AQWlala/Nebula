//! T-E-S-06: Organization Orchestration — 多组织编排。
//!
//! 支持多个 Agent 组织（团队）的编排。每个组织拥有独立的 Agent 池、
//! 任务队列和协作策略。`OrganizationRegistry` 管理所有组织的生命周期，
//! `OrgOrchestrator` 负责跨组织的任务分配、结果收集和冲突解决。
//!
//! ## 协作策略
//!
//! | 策略 | 决策方式 | 任务分配 |
//! |------|----------|----------|
//! | `Hierarchical` | Leader 决策 | 分配给 Leader |
//! | `Democratic` | 投票决策 | 分配给多数票获胜者（模拟：首个成员） |
//! | `Competitive` | 最优胜出 | 分配给表现最优者（模拟：末位成员） |
//! | `Collaborative` | 分工合作 | 分配给首个 Worker |
//!
//! ## 冲突解决
//!
//! 冲突解决策略与组织的协作策略对齐：
//! - `Hierarchical` → Leader 裁决
//! - `Democratic` → 多数投票
//! - `Competitive` → 最优者胜出
//! - `Collaborative` → 协商共识

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// 枚举类型
// ---------------------------------------------------------------------------

/// 组织成员角色。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    /// 领导者 — 负责决策和任务分配。
    Leader,
    /// 执行者 — 负责具体任务执行。
    Worker,
    /// 审查者 — 负责结果审查和质量把关。
    Reviewer,
    /// 观察者 — 只读权限，不参与执行。
    Observer,
}

/// 组织编排策略。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OrgStrategy {
    /// 层级式 — Leader 统一分配任务和决策。
    Hierarchical,
    /// 民主式 — 成员投票决策，多数票胜出。
    Democratic,
    /// 竞争式 — 成员竞争执行，最优结果胜出。
    Competitive,
    /// 协作式 — 成员分工合作，共同完成任务。
    Collaborative,
}

/// 组织生命周期状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OrgStatus {
    /// 活跃 — 可接受新任务和成员变更。
    Active,
    /// 暂停 — 暂不接受新任务，但保留所有数据。
    Paused,
    /// 已解散 — 不可恢复，拒绝所有操作。
    Disbanded,
}

/// 任务优先级。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Critical,
}

/// 任务生命周期状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 待分配。
    Pending,
    /// 已分配。
    Assigned,
    /// 执行中。
    InProgress,
    /// 已完成。
    Completed,
    /// 已失败。
    Failed,
    /// 已取消。
    Cancelled,
}

/// 冲突类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// 任务分配冲突 — 多个成员争夺同一任务。
    TaskAssignment,
    /// 资源竞争冲突 — 多个任务竞争同一资源。
    ResourceContention,
    /// 输出分歧 — 成员对结果存在不同意见。
    OutputDisagreement,
    /// 优先级争议 — 任务优先级排序不一致。
    PriorityDispute,
}

/// 冲突解决方式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionType {
    /// Leader 裁决。
    LeaderDecides,
    /// 投票表决。
    Vote,
    /// 竞争胜出。
    Competition,
    /// 协商共识。
    Consensus,
    /// 仲裁。
    Arbitration,
    /// 解决失败。
    Failed,
}

// ---------------------------------------------------------------------------
// 核心结构体
// ---------------------------------------------------------------------------

/// 组织成员 — 组织中的一个 Agent 实例。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgMember {
    /// Agent 标识符。
    pub agent_id: String,
    /// 成员角色。
    pub role: OrgRole,
    /// 加入时间。
    pub joined_at: chrono::DateTime<chrono::Utc>,
    /// 权限列表（如 `["read", "write", "execute"]`）。
    pub permissions: Vec<String>,
}

impl OrgMember {
    /// 创建新成员，自动设置 `joined_at` 为当前时间。
    pub fn new(agent_id: &str, role: OrgRole) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            role,
            joined_at: Utc::now(),
            permissions: Vec::new(),
        }
    }

    /// 创建带权限的新成员。
    pub fn with_permissions(agent_id: &str, role: OrgRole, permissions: Vec<String>) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            role,
            joined_at: Utc::now(),
            permissions,
        }
    }

    /// 检查是否拥有指定权限。
    pub fn has_permission(&self, perm: &str) -> bool {
        self.permissions.iter().any(|p| p == perm)
    }
}

/// 组织任务。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgTask {
    /// 任务唯一标识。
    pub id: String,
    /// 任务描述。
    pub description: String,
    /// 分配给的 Agent（`None` 表示未分配）。
    pub assigned_to: Option<String>,
    /// 优先级。
    pub priority: TaskPriority,
    /// 当前状态。
    pub status: TaskStatus,
    /// 创建时间。
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl OrgTask {
    /// 创建新任务，自动生成 ID 和时间戳，状态为 `Pending`。
    pub fn new(description: &str, priority: TaskPriority) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            description: description.to_string(),
            assigned_to: None,
            priority,
            status: TaskStatus::Pending,
            created_at: Utc::now(),
        }
    }
}

/// 组织 — 一个独立的 Agent 团队。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    /// 组织唯一标识。
    pub id: String,
    /// 组织名称。
    pub name: String,
    /// 组织描述。
    pub description: String,
    /// 成员列表。
    pub members: Vec<OrgMember>,
    /// 任务队列。
    pub task_queue: Vec<OrgTask>,
    /// 编排策略。
    pub strategy: OrgStrategy,
    /// 创建时间。
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// 当前状态。
    pub status: OrgStatus,
}

impl Organization {
    /// 成员数量。
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// 是否包含指定 Agent。
    pub fn has_member(&self, agent_id: &str) -> bool {
        self.members.iter().any(|m| m.agent_id == agent_id)
    }

    /// 是否活跃。
    pub fn is_active(&self) -> bool {
        self.status == OrgStatus::Active
    }

    /// 获取指定角色的成员列表。
    pub fn members_by_role(&self, role: OrgRole) -> Vec<&OrgMember> {
        self.members.iter().filter(|m| m.role == role).collect()
    }
}

/// 任务执行结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskResult {
    /// 关联的任务 ID。
    pub task_id: String,
    /// 执行者 Agent ID。
    pub agent_id: String,
    /// 执行输出。
    pub output: String,
    /// 是否成功。
    pub success: bool,
    /// 完成时间。
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

/// 冲突描述。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    /// 冲突唯一标识。
    pub id: String,
    /// 所属组织 ID。
    pub org_id: String,
    /// 冲突类型。
    pub conflict_type: ConflictType,
    /// 冲突描述。
    pub description: String,
    /// 涉及的 Agent ID 列表。
    pub parties: Vec<String>,
    /// 创建时间。
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Conflict {
    /// 创建新冲突。
    pub fn new(
        org_id: &str,
        conflict_type: ConflictType,
        description: &str,
        parties: Vec<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            org_id: org_id.to_string(),
            conflict_type,
            description: description.to_string(),
            parties,
            created_at: Utc::now(),
        }
    }
}

/// 冲突解决方案。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Resolution {
    /// 解决方式。
    pub resolution_type: ResolutionType,
    /// 胜出方 Agent ID（`None` 表示无明确胜出方）。
    pub winner: Option<String>,
    /// 解决说明。
    pub explanation: String,
    /// 决策时间。
    pub decided_at: chrono::DateTime<chrono::Utc>,
}

impl Resolution {
    /// 创建一个失败的解决方案。
    pub fn failed(explanation: &str) -> Self {
        Self {
            resolution_type: ResolutionType::Failed,
            winner: None,
            explanation: explanation.to_string(),
            decided_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// OrganizationRegistry — 组织注册表
// ---------------------------------------------------------------------------

/// 组织注册表 — 管理所有组织的创建、查询和生命周期。
pub struct OrganizationRegistry {
    orgs: HashMap<String, Organization>,
}

impl OrganizationRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self {
            orgs: HashMap::new(),
        }
    }

    /// 创建新组织并注册。
    ///
    /// 自动生成 UUID 和时间戳，初始状态为 `Active`，无成员和任务。
    pub fn create_org(&mut self, name: &str, strategy: OrgStrategy) -> Result<Organization> {
        let org = Organization {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: String::new(),
            members: Vec::new(),
            task_queue: Vec::new(),
            strategy,
            created_at: Utc::now(),
            status: OrgStatus::Active,
        };
        self.orgs.insert(org.id.clone(), org.clone());
        Ok(org)
    }

    /// 获取组织（不可变引用）。
    pub fn get_org(&self, id: &str) -> Option<&Organization> {
        self.orgs.get(id)
    }

    /// 获取组织（可变引用）。
    pub fn get_org_mut(&mut self, id: &str) -> Option<&mut Organization> {
        self.orgs.get_mut(id)
    }

    /// 列出所有组织。
    pub fn list_orgs(&self) -> Vec<&Organization> {
        self.orgs.values().collect()
    }

    /// 向组织添加成员。
    ///
    /// 组织必须存在且未解散。不允许重复添加同一 Agent。
    pub fn add_member(&mut self, org_id: &str, member: OrgMember) -> Result<()> {
        let org = self
            .orgs
            .get_mut(org_id)
            .ok_or_else(|| anyhow!("organization not found: {}", org_id))?;
        if org.status == OrgStatus::Disbanded {
            bail!("organization is disbanded: {}", org_id);
        }
        if org.members.iter().any(|m| m.agent_id == member.agent_id) {
            bail!("member already exists in organization: {}", member.agent_id);
        }
        org.members.push(member);
        Ok(())
    }

    /// 从组织移除成员。
    ///
    /// 组织必须存在且未解散。成员不存在则返回错误。
    pub fn remove_member(&mut self, org_id: &str, agent_id: &str) -> Result<()> {
        let org = self
            .orgs
            .get_mut(org_id)
            .ok_or_else(|| anyhow!("organization not found: {}", org_id))?;
        if org.status == OrgStatus::Disbanded {
            bail!("organization is disbanded: {}", org_id);
        }
        let before = org.members.len();
        org.members.retain(|m| m.agent_id != agent_id);
        if org.members.len() == before {
            bail!("member not found in organization: {}", agent_id);
        }
        Ok(())
    }

    /// 解散组织。
    ///
    /// 解散后组织状态变为 `Disbanded`，拒绝所有后续操作。
    /// 已解散的组织不能再次解散。
    pub fn disband(&mut self, org_id: &str) -> Result<()> {
        let org = self
            .orgs
            .get_mut(org_id)
            .ok_or_else(|| anyhow!("organization not found: {}", org_id))?;
        if org.status == OrgStatus::Disbanded {
            bail!("organization already disbanded: {}", org_id);
        }
        org.status = OrgStatus::Disbanded;
        Ok(())
    }

    /// 组织数量。
    pub fn len(&self) -> usize {
        self.orgs.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.orgs.is_empty()
    }
}

impl Default for OrganizationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// OrgOrchestrator — 组织编排器
// ---------------------------------------------------------------------------

/// 组织编排器 — 负责跨组织的任务分配、结果收集和冲突解决。
///
/// 持有 `OrganizationRegistry` 的所有权，提供高层编排接口。
pub struct OrgOrchestrator {
    registry: OrganizationRegistry,
    /// 已完成但尚未收集的结果缓冲（org_id → results）。
    results: HashMap<String, Vec<TaskResult>>,
}

impl OrgOrchestrator {
    /// 创建编排器，接管注册表所有权。
    pub fn new(registry: OrganizationRegistry) -> Self {
        Self {
            registry,
            results: HashMap::new(),
        }
    }

    /// 获取注册表（不可变引用）。
    pub fn registry(&self) -> &OrganizationRegistry {
        &self.registry
    }

    /// 获取注册表（可变引用）。
    pub fn registry_mut(&mut self) -> &mut OrganizationRegistry {
        &mut self.registry
    }

    /// 分配任务到指定组织。
    ///
    /// 根据组织的协作策略选择执行者，将任务加入队列，并生成模拟执行结果。
    ///
    /// 返回被分配的 Agent ID。
    pub fn assign_task(&mut self, org_id: &str, mut task: OrgTask) -> Result<String> {
        // 读取组织信息（不可变借用，作用域结束即释放）
        let (strategy, members) = {
            let org = self
                .registry
                .get_org(org_id)
                .ok_or_else(|| anyhow!("organization not found: {}", org_id))?;
            if org.status == OrgStatus::Disbanded {
                bail!("organization is disbanded: {}", org_id);
            }
            if org.status == OrgStatus::Paused {
                bail!("organization is paused: {}", org_id);
            }
            if org.members.is_empty() {
                bail!("organization has no members: {}", org_id);
            }
            (org.strategy, org.members.clone())
        };

        // 根据策略选择执行者
        let assignee = select_assignee(strategy, &members);

        // 更新任务并加入队列（可变借用，作用域结束即释放）
        task.assigned_to = Some(assignee.clone());
        task.status = TaskStatus::Assigned;
        {
            let org = self
                .registry
                .get_org_mut(org_id)
                .ok_or_else(|| anyhow!("organization not found: {}", org_id))?;
            org.task_queue.push(task.clone());
        }

        // 生成模拟执行结果
        let result = TaskResult {
            task_id: task.id.clone(),
            agent_id: assignee.clone(),
            output: format!("[simulated] completed task: {}", task.description),
            success: true,
            completed_at: Utc::now(),
        };
        self.results
            .entry(org_id.to_string())
            .or_default()
            .push(result);

        Ok(assignee)
    }

    /// 收集指定组织的所有已完成结果。
    ///
    /// 收集后结果从缓冲区中移除（drain 语义），重复调用返回空列表。
    pub fn collect_results(&mut self, org_id: &str) -> Vec<TaskResult> {
        self.results.remove(org_id).unwrap_or_default()
    }

    /// 解决组织内的冲突。
    ///
    /// 解决策略与组织的协作策略对齐：
    /// - `Hierarchical` → Leader 裁决
    /// - `Democratic` → 多数投票（首个方胜出）
    /// - `Competitive` → 最优者胜出（末位方胜出）
    /// - `Collaborative` → 协商共识
    pub fn resolve_conflict(&self, org_id: &str, conflict: &Conflict) -> Resolution {
        let org = match self.registry.get_org(org_id) {
            Some(o) => o,
            None => return Resolution::failed(&format!("organization not found: {}", org_id)),
        };

        match org.strategy {
            OrgStrategy::Hierarchical => {
                let leader = org
                    .members
                    .iter()
                    .find(|m| m.role == OrgRole::Leader)
                    .map(|m| m.agent_id.clone());
                Resolution {
                    resolution_type: ResolutionType::LeaderDecides,
                    winner: leader.clone(),
                    explanation: format!(
                        "leader {} resolved the conflict",
                        leader.as_deref().unwrap_or("none")
                    ),
                    decided_at: Utc::now(),
                }
            }
            OrgStrategy::Democratic => {
                let winner = conflict.parties.first().cloned();
                Resolution {
                    resolution_type: ResolutionType::Vote,
                    winner: winner.clone(),
                    explanation: format!(
                        "majority vote selected {}",
                        winner.as_deref().unwrap_or("none")
                    ),
                    decided_at: Utc::now(),
                }
            }
            OrgStrategy::Competitive => {
                let winner = conflict.parties.last().cloned();
                Resolution {
                    resolution_type: ResolutionType::Competition,
                    winner: winner.clone(),
                    explanation: format!(
                        "best performer {} wins",
                        winner.as_deref().unwrap_or("none")
                    ),
                    decided_at: Utc::now(),
                }
            }
            OrgStrategy::Collaborative => Resolution {
                resolution_type: ResolutionType::Consensus,
                winner: conflict.parties.first().cloned(),
                explanation: "consensus reached through collaboration".to_string(),
                decided_at: Utc::now(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// 内部辅助函数
// ---------------------------------------------------------------------------

/// 根据协作策略从成员列表中选择任务执行者。
///
/// - `Hierarchical` — 选择 Leader（无 Leader 则选首个成员）
/// - `Democratic` — 选择首个成员（模拟多数票胜出）
/// - `Competitive` — 选择末位成员（模拟最优者胜出）
/// - `Collaborative` — 选择首个 Worker（无 Worker 则选首个成员）
///
/// 调用者保证 `members` 非空。
fn select_assignee(strategy: OrgStrategy, members: &[OrgMember]) -> String {
    match strategy {
        OrgStrategy::Hierarchical => members
            .iter()
            .find(|m| m.role == OrgRole::Leader)
            .map(|m| m.agent_id.clone())
            .unwrap_or_else(|| members[0].agent_id.clone()),
        OrgStrategy::Democratic => members[0].agent_id.clone(),
        OrgStrategy::Competitive => members[members.len() - 1].agent_id.clone(),
        OrgStrategy::Collaborative => members
            .iter()
            .find(|m| m.role == OrgRole::Worker)
            .map(|m| m.agent_id.clone())
            .unwrap_or_else(|| members[0].agent_id.clone()),
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- 测试辅助函数 ---

    fn make_member(agent_id: &str, role: OrgRole) -> OrgMember {
        OrgMember::with_permissions(
            agent_id,
            role,
            vec!["read".to_string(), "write".to_string()],
        )
    }

    /// 创建包含 3 个成员（1 Leader + 2 Worker）的组织，返回 (orchestrator, org_id)。
    fn setup_org(strategy: OrgStrategy) -> (OrgOrchestrator, String) {
        let mut registry = OrganizationRegistry::new();
        let org = registry.create_org("test-org", strategy).unwrap();
        registry
            .add_member(&org.id, make_member("agent-1", OrgRole::Leader))
            .unwrap();
        registry
            .add_member(&org.id, make_member("agent-2", OrgRole::Worker))
            .unwrap();
        registry
            .add_member(&org.id, make_member("agent-3", OrgRole::Worker))
            .unwrap();
        let orchestrator = OrgOrchestrator::new(registry);
        (orchestrator, org.id)
    }

    // --- OrganizationRegistry 测试 ---

    #[test]
    fn test_create_org() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("engineering", OrgStrategy::Hierarchical)
            .unwrap();
        assert_eq!(org.name, "engineering");
        assert_eq!(org.strategy, OrgStrategy::Hierarchical);
        assert_eq!(org.status, OrgStatus::Active);
        assert!(org.members.is_empty());
        assert!(org.task_queue.is_empty());
        assert!(!org.id.is_empty());
    }

    #[test]
    fn test_create_org_generates_unique_ids() {
        let mut registry = OrganizationRegistry::new();
        let org1 = registry
            .create_org("team-a", OrgStrategy::Democratic)
            .unwrap();
        let org2 = registry
            .create_org("team-b", OrgStrategy::Competitive)
            .unwrap();
        assert_ne!(org1.id, org2.id);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_get_org_existing() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("research", OrgStrategy::Collaborative)
            .unwrap();
        let found = registry.get_org(&org.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "research");
    }

    #[test]
    fn test_get_org_nonexistent() {
        let registry = OrganizationRegistry::new();
        assert!(registry.get_org("nonexistent").is_none());
    }

    #[test]
    fn test_list_orgs_empty() {
        let registry = OrganizationRegistry::new();
        assert!(registry.list_orgs().is_empty());
        assert!(registry.is_empty());
    }

    #[test]
    fn test_list_orgs_multiple() {
        let mut registry = OrganizationRegistry::new();
        registry
            .create_org("team-a", OrgStrategy::Hierarchical)
            .unwrap();
        registry
            .create_org("team-b", OrgStrategy::Democratic)
            .unwrap();
        registry
            .create_org("team-c", OrgStrategy::Competitive)
            .unwrap();
        assert_eq!(registry.list_orgs().len(), 3);
    }

    #[test]
    fn test_add_member_success() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        let member = make_member("agent-x", OrgRole::Worker);
        registry.add_member(&org.id, member).unwrap();
        let org = registry.get_org(&org.id).unwrap();
        assert_eq!(org.member_count(), 1);
        assert!(org.has_member("agent-x"));
    }

    #[test]
    fn test_add_member_duplicate() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        registry
            .add_member(&org.id, make_member("agent-1", OrgRole::Leader))
            .unwrap();
        let result = registry.add_member(&org.id, make_member("agent-1", OrgRole::Worker));
        assert!(result.is_err());
    }

    #[test]
    fn test_add_member_to_nonexistent_org() {
        let mut registry = OrganizationRegistry::new();
        let result = registry.add_member("fake-org", make_member("agent-1", OrgRole::Worker));
        assert!(result.is_err());
    }

    #[test]
    fn test_add_member_to_disbanded_org() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        registry.disband(&org.id).unwrap();
        let result = registry.add_member(&org.id, make_member("agent-1", OrgRole::Worker));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_member_success() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        registry
            .add_member(&org.id, make_member("agent-1", OrgRole::Leader))
            .unwrap();
        registry
            .add_member(&org.id, make_member("agent-2", OrgRole::Worker))
            .unwrap();
        registry.remove_member(&org.id, "agent-1").unwrap();
        let org = registry.get_org(&org.id).unwrap();
        assert_eq!(org.member_count(), 1);
        assert!(!org.has_member("agent-1"));
        assert!(org.has_member("agent-2"));
    }

    #[test]
    fn test_remove_member_not_found() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        registry
            .add_member(&org.id, make_member("agent-1", OrgRole::Leader))
            .unwrap();
        let result = registry.remove_member(&org.id, "nonexistent-agent");
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_member_from_nonexistent_org() {
        let mut registry = OrganizationRegistry::new();
        let result = registry.remove_member("fake-org", "agent-1");
        assert!(result.is_err());
    }

    #[test]
    fn test_disband_success() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        registry.disband(&org.id).unwrap();
        let org = registry.get_org(&org.id).unwrap();
        assert_eq!(org.status, OrgStatus::Disbanded);
        assert!(!org.is_active());
    }

    #[test]
    fn test_disband_nonexistent() {
        let mut registry = OrganizationRegistry::new();
        let result = registry.disband("fake-org");
        assert!(result.is_err());
    }

    #[test]
    fn test_disband_already_disbanded() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("team", OrgStrategy::Hierarchical)
            .unwrap();
        registry.disband(&org.id).unwrap();
        let result = registry.disband(&org.id);
        assert!(result.is_err());
    }

    // --- OrgOrchestrator.assign_task 测试 ---

    #[test]
    fn test_assign_task_hierarchical_picks_leader() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        let task = OrgTask::new("implement feature X", TaskPriority::High);
        let assignee = orchestrator.assign_task(&org_id, task).unwrap();
        assert_eq!(assignee, "agent-1"); // Leader
    }

    #[test]
    fn test_assign_task_democratic_picks_first() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Democratic);
        let task = OrgTask::new("design API", TaskPriority::Medium);
        let assignee = orchestrator.assign_task(&org_id, task).unwrap();
        assert_eq!(assignee, "agent-1"); // 首个成员（多数票模拟）
    }

    #[test]
    fn test_assign_task_competitive_picks_last() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Competitive);
        let task = OrgTask::new("optimize algorithm", TaskPriority::Critical);
        let assignee = orchestrator.assign_task(&org_id, task).unwrap();
        assert_eq!(assignee, "agent-3"); // 末位成员（最优者模拟）
    }

    #[test]
    fn test_assign_task_collaborative_picks_first_worker() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Collaborative);
        let task = OrgTask::new("write tests", TaskPriority::Low);
        let assignee = orchestrator.assign_task(&org_id, task).unwrap();
        assert_eq!(assignee, "agent-2"); // 首个 Worker
    }

    #[test]
    fn test_assign_task_nonexistent_org() {
        let (mut orchestrator, _) = setup_org(OrgStrategy::Hierarchical);
        let task = OrgTask::new("task", TaskPriority::Medium);
        let result = orchestrator.assign_task("fake-org", task);
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_task_disbanded_org() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        orchestrator.registry_mut().disband(&org_id).unwrap();
        let task = OrgTask::new("task", TaskPriority::Medium);
        let result = orchestrator.assign_task(&org_id, task);
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_task_no_members() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("empty-team", OrgStrategy::Hierarchical)
            .unwrap();
        let mut orchestrator = OrgOrchestrator::new(registry);
        let task = OrgTask::new("task", TaskPriority::Medium);
        let result = orchestrator.assign_task(&org.id, task);
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_task_adds_to_queue() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        let task = OrgTask::new("task-1", TaskPriority::Medium);
        orchestrator.assign_task(&org_id, task).unwrap();
        let org = orchestrator.registry().get_org(&org_id).unwrap();
        assert_eq!(org.task_queue.len(), 1);
        assert_eq!(org.task_queue[0].status, TaskStatus::Assigned);
        assert!(org.task_queue[0].assigned_to.is_some());
    }

    // --- OrgOrchestrator.collect_results 测试 ---

    #[test]
    fn test_collect_results_returns_results() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        let task = OrgTask::new("task-1", TaskPriority::Medium);
        orchestrator.assign_task(&org_id, task).unwrap();
        let results = orchestrator.collect_results(&org_id);
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].agent_id, "agent-1");
    }

    #[test]
    fn test_collect_results_empty() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        let results = orchestrator.collect_results(&org_id);
        assert!(results.is_empty());
    }

    #[test]
    fn test_collect_results_drains() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        let task = OrgTask::new("task-1", TaskPriority::Medium);
        orchestrator.assign_task(&org_id, task).unwrap();
        let first = orchestrator.collect_results(&org_id);
        let second = orchestrator.collect_results(&org_id);
        assert_eq!(first.len(), 1);
        assert!(second.is_empty(), "collect_results should drain the buffer");
    }

    #[test]
    fn test_collect_results_multiple_tasks() {
        let (mut orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        for i in 0..5 {
            let task = OrgTask::new(&format!("task-{}", i), TaskPriority::Medium);
            orchestrator.assign_task(&org_id, task).unwrap();
        }
        let results = orchestrator.collect_results(&org_id);
        assert_eq!(results.len(), 5);
        assert!(results.iter().all(|r| r.success));
    }

    // --- OrgOrchestrator.resolve_conflict 测试 ---

    #[test]
    fn test_resolve_conflict_hierarchical() {
        let (orchestrator, org_id) = setup_org(OrgStrategy::Hierarchical);
        let conflict = Conflict::new(
            &org_id,
            ConflictType::TaskAssignment,
            "two agents want the same task",
            vec!["agent-2".to_string(), "agent-3".to_string()],
        );
        let resolution = orchestrator.resolve_conflict(&org_id, &conflict);
        assert_eq!(resolution.resolution_type, ResolutionType::LeaderDecides);
        assert_eq!(resolution.winner.as_deref(), Some("agent-1")); // Leader 裁决
    }

    #[test]
    fn test_resolve_conflict_democratic() {
        let (orchestrator, org_id) = setup_org(OrgStrategy::Democratic);
        let conflict = Conflict::new(
            &org_id,
            ConflictType::OutputDisagreement,
            "agents disagree on output",
            vec!["agent-2".to_string(), "agent-3".to_string()],
        );
        let resolution = orchestrator.resolve_conflict(&org_id, &conflict);
        assert_eq!(resolution.resolution_type, ResolutionType::Vote);
        assert_eq!(resolution.winner.as_deref(), Some("agent-2")); // 首个方（多数票模拟）
    }

    #[test]
    fn test_resolve_conflict_competitive() {
        let (orchestrator, org_id) = setup_org(OrgStrategy::Competitive);
        let conflict = Conflict::new(
            &org_id,
            ConflictType::ResourceContention,
            "agents compete for resource",
            vec![
                "agent-1".to_string(),
                "agent-2".to_string(),
                "agent-3".to_string(),
            ],
        );
        let resolution = orchestrator.resolve_conflict(&org_id, &conflict);
        assert_eq!(resolution.resolution_type, ResolutionType::Competition);
        assert_eq!(resolution.winner.as_deref(), Some("agent-3")); // 末位方（最优者模拟）
    }

    #[test]
    fn test_resolve_conflict_collaborative() {
        let (orchestrator, org_id) = setup_org(OrgStrategy::Collaborative);
        let conflict = Conflict::new(
            &org_id,
            ConflictType::PriorityDispute,
            "agents disagree on priority",
            vec!["agent-1".to_string(), "agent-2".to_string()],
        );
        let resolution = orchestrator.resolve_conflict(&org_id, &conflict);
        assert_eq!(resolution.resolution_type, ResolutionType::Consensus);
        assert!(resolution.winner.is_some());
    }

    #[test]
    fn test_resolve_conflict_nonexistent_org() {
        let (orchestrator, _) = setup_org(OrgStrategy::Hierarchical);
        let conflict = Conflict::new(
            "fake-org",
            ConflictType::TaskAssignment,
            "test",
            vec!["agent-1".to_string()],
        );
        let resolution = orchestrator.resolve_conflict("fake-org", &conflict);
        assert_eq!(resolution.resolution_type, ResolutionType::Failed);
        assert!(resolution.winner.is_none());
    }

    // --- 辅助类型测试 ---

    #[test]
    fn test_org_task_new() {
        let task = OrgTask::new("do something", TaskPriority::High);
        assert_eq!(task.description, "do something");
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.assigned_to.is_none());
        assert!(!task.id.is_empty());
    }

    #[test]
    fn test_org_member_new() {
        let member = OrgMember::new("agent-1", OrgRole::Leader);
        assert_eq!(member.agent_id, "agent-1");
        assert_eq!(member.role, OrgRole::Leader);
        assert!(member.permissions.is_empty());
    }

    #[test]
    fn test_org_member_has_permission() {
        let member = OrgMember::with_permissions(
            "agent-1",
            OrgRole::Worker,
            vec!["read".to_string(), "write".to_string()],
        );
        assert!(member.has_permission("read"));
        assert!(member.has_permission("write"));
        assert!(!member.has_permission("execute"));
    }

    #[test]
    fn test_organization_members_by_role() {
        let mut org = Organization {
            id: "test".to_string(),
            name: "test".to_string(),
            description: String::new(),
            members: vec![
                make_member("agent-1", OrgRole::Leader),
                make_member("agent-2", OrgRole::Worker),
                make_member("agent-3", OrgRole::Worker),
                make_member("agent-4", OrgRole::Reviewer),
            ],
            task_queue: Vec::new(),
            strategy: OrgStrategy::Hierarchical,
            created_at: Utc::now(),
            status: OrgStatus::Active,
        };
        assert_eq!(org.members_by_role(OrgRole::Leader).len(), 1);
        assert_eq!(org.members_by_role(OrgRole::Worker).len(), 2);
        assert_eq!(org.members_by_role(OrgRole::Reviewer).len(), 1);
        assert_eq!(org.members_by_role(OrgRole::Observer).len(), 0);
        // 修改状态并验证 is_active
        org.status = OrgStatus::Paused;
        assert!(!org.is_active());
    }

    #[test]
    fn test_task_priority_ordering() {
        assert!(TaskPriority::Critical > TaskPriority::High);
        assert!(TaskPriority::High > TaskPriority::Medium);
        assert!(TaskPriority::Medium > TaskPriority::Low);
    }

    #[test]
    fn test_conflict_new() {
        let conflict = Conflict::new(
            "org-1",
            ConflictType::ResourceContention,
            "agents compete for GPU",
            vec!["agent-1".to_string(), "agent-2".to_string()],
        );
        assert_eq!(conflict.org_id, "org-1");
        assert_eq!(conflict.conflict_type, ConflictType::ResourceContention);
        assert_eq!(conflict.parties.len(), 2);
        assert!(!conflict.id.is_empty());
    }

    #[test]
    fn test_hierarchical_without_leader_falls_back_to_first() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("no-leader", OrgStrategy::Hierarchical)
            .unwrap();
        // 只有 Worker，没有 Leader
        registry
            .add_member(&org.id, make_member("worker-1", OrgRole::Worker))
            .unwrap();
        registry
            .add_member(&org.id, make_member("worker-2", OrgRole::Worker))
            .unwrap();
        let mut orchestrator = OrgOrchestrator::new(registry);
        let task = OrgTask::new("task", TaskPriority::Medium);
        let assignee = orchestrator.assign_task(&org.id, task).unwrap();
        // 无 Leader 时回退到首个成员
        assert_eq!(assignee, "worker-1");
    }

    #[test]
    fn test_collaborative_without_worker_falls_back_to_first() {
        let mut registry = OrganizationRegistry::new();
        let org = registry
            .create_org("no-worker", OrgStrategy::Collaborative)
            .unwrap();
        // 只有 Leader 和 Reviewer，没有 Worker
        registry
            .add_member(&org.id, make_member("leader-1", OrgRole::Leader))
            .unwrap();
        registry
            .add_member(&org.id, make_member("reviewer-1", OrgRole::Reviewer))
            .unwrap();
        let mut orchestrator = OrgOrchestrator::new(registry);
        let task = OrgTask::new("task", TaskPriority::Medium);
        let assignee = orchestrator.assign_task(&org.id, task).unwrap();
        // 无 Worker 时回退到首个成员
        assert_eq!(assignee, "leader-1");
    }
}
