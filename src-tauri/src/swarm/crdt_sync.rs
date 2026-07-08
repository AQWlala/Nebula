//! T-S4-A-03: 蜂群内 CRDT 同步 — Agent 间通过 AgentBus 传播 CRDT 操作。
//!
//! 复用 [`crate::sync::crdt::CrdtEngine`] 的 LWW(Last-Writer-Wins)合并语义,
//! 在 swarm 内部维护一个 `memory_id -> CrdtVersion` 的本地副本。当任一 agent
//! 修改某条记忆时,通过 [`crate::swarm::bus::AgentBus`] 广播 `CrdtSync` 消息,
//! 其他 agent 接收后调用 `merge_remote` 进行 LWW 合并。
//!
//! ## ACL 过滤(关键约束)
//!
//! 根据 ROADMAP §2.4 隐式依赖链,**跨 Agent 记忆访问必须经过 ACL 过滤**。
//! 本模块在两个关键点强制执行 ACL:
//!
//! * `merge_remote` — 写入前检查 `Write` 权限,拒绝则跳过合并(返回 `false`)。
//! * `get_memory` / `list_memories` — 读取前检查 `Read` 权限,过滤不可见条目。
//!
//! ACL 主体(agent name)由调用方传入;默认规则集从 [`MemoryAcl`] 继承。
//!
//! ## 设计决策
//!
//! * CRDT 负载以 JSON 序列化形式放入 `BusMessage.content`(字符串),
//!   保持与现有 bus 协议兼容,无需扩展 BusMessage 结构。
//! * 合并采用 `merge_lww`(整版本级),而非 `merge_fields`(字段级),
//!   因为 swarm agent 通常产出完整记忆条目,字段级合并在 LLM 场景收益有限。
//!   `merge_fields` 仍可通过 `merge_remote_fields` 显式调用。

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{info, instrument, warn};

use crate::memory::acl::{AclPermission, MemoryAcl};
use crate::sync::crdt::{CrdtEngine, CrdtVersion, FieldChange};

use super::bus::{AgentBus, BusMessage, BusMessageType};

/// 蜂群内 CRDT 同步协调器。
///
/// 持有本地 CRDT 副本 + ACL 规则集,提供:
/// * `apply_local_change` — 本地修改,生成新版本,广播给其他 agent
/// * `merge_remote` — 接收远端版本,ACL 检查后 LWW 合并
/// * `get_memory` / `list_memories` — ACL 过滤的读取
pub struct SwarmCrdtSync {
    /// 本地 CRDT 副本:memory_id -> CrdtVersion。
    versions: RwLock<HashMap<String, CrdtVersion>>,
    /// CRDT 合并引擎(无状态)。
    engine: CrdtEngine,
    /// ACL 规则集(用于跨 agent 访问过滤)。
    acl: MemoryAcl,
    /// 本设备/agent 的标识(用于生成新版本)。
    device_id: String,
}

impl SwarmCrdtSync {
    /// 创建新的 CRDT 同步协调器。
    ///
    /// * `device_id` — 本 agent 的唯一标识(用于 LWW tie-breaker)
    /// * `acl` — ACL 规则集(通常从 SQLite `memory_acl` 表加载)
    pub fn new(device_id: impl Into<String>, acl: MemoryAcl) -> Self {
        Self {
            versions: RwLock::new(HashMap::new()),
            engine: CrdtEngine::new(),
            acl,
            device_id: device_id.into(),
        }
    }

    /// 本设备标识。
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// 当前本地副本中的记忆条数(未过滤 ACL)。
    pub fn len(&self) -> usize {
        self.versions.read().len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.versions.read().is_empty()
    }

    // ------------------------------------------------------------------
    // 写入路径
    // ------------------------------------------------------------------

    /// 应用本地变更:生成新版本,存入本地副本,并广播到 AgentBus。
    ///
    /// * `memory_id` — 记忆条目 ID
    /// * `field_changes` — 字段级变更列表
    /// * `bus` — AgentBus 引用(用于广播 CrdtSync 消息)
    ///
    /// 返回生成的新版本号。
    #[instrument(target = "nebula.swarm.crdt", skip(self, field_changes, bus), fields(otel.kind = "crdt", memory_id = %memory_id))]
    pub fn apply_local_change(
        &self,
        memory_id: &str,
        field_changes: Vec<FieldChange>,
        bus: &AgentBus,
    ) -> u64 {
        let now = chrono::Utc::now().timestamp();
        let new_version = {
            let versions = self.versions.read();
            let prev_version = versions.get(memory_id).map(|v| v.version).unwrap_or(0);
            prev_version + 1
        };

        let version = CrdtVersion {
            memory_id: memory_id.to_string(),
            version: new_version,
            device_id: self.device_id.clone(),
            timestamp: now,
            field_changes,
        };

        // 存入本地副本。
        self.versions
            .write()
            .insert(memory_id.to_string(), version.clone());

        // 广播到 AgentBus(序列化为 JSON 放入 content)。
        let payload = serde_json::to_string(&version).unwrap_or_else(|e| {
            warn!(
                target: "nebula.swarm.crdt",
                error = %e,
                "failed to serialize CrdtVersion for broadcast"
            );
            "{}".to_string()
        });

        bus.broadcast(BusMessage {
            from: self.device_id.clone(),
            to: None,
            content: payload,
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: BusMessageType::CrdtSync,
            correlation_id: None,
        });

        info!(
            target: "nebula.swarm.crdt",
            memory_id = %memory_id,
            version = new_version,
            "local change applied and broadcast"
        );

        new_version
    }

    /// 接收远端版本,ACL 检查后 LWW 合并到本地副本。
    ///
    /// **ACL 检查**:调用方(`from` agent)必须对 `memory_id` 拥有 `Write` 权限,
    /// 否则拒绝合并(返回 `false`)。这防止未授权 agent 覆盖他人记忆。
    ///
    /// 返回 `true` 表示合并成功(本地副本被更新或保持),`false` 表示被 ACL 拒绝。
    #[instrument(target = "nebula.swarm.crdt", skip(self, remote), fields(otel.kind = "crdt", memory_id = %remote.memory_id))]
    pub fn merge_remote(&self, remote: CrdtVersion, from: &str) -> bool {
        // ACL 检查:写权限。
        if !self
            .acl
            .check(from, &remote.memory_id, AclPermission::Write)
        {
            warn!(
                target: "nebula.swarm.crdt",
                agent = %from,
                memory_id = %remote.memory_id,
                "CRDT merge rejected by ACL (no Write permission)"
            );
            return false;
        }

        let mut versions = self.versions.write();
        match versions.get(&remote.memory_id) {
            Some(local) => {
                let result = self.engine.merge_lww(local, &remote);
                versions.insert(remote.memory_id.clone(), result.winner.clone());
                info!(
                    target: "nebula.swarm.crdt",
                    memory_id = %result.winner.memory_id,
                    winner_device = %result.winner.device_id,
                    conflict = result.loser.is_some(),
                    "remote version merged (LWW)"
                );
            }
            None => {
                // 本地无此条目,直接接受远端版本。
                versions.insert(remote.memory_id.clone(), remote.clone());
                info!(
                    target: "nebula.swarm.crdt",
                    memory_id = %remote.memory_id,
                    "remote version accepted (no local copy)"
                );
            }
        }
        true
    }

    /// 接收远端版本,使用字段级合并(`merge_fields`)。
    ///
    /// 仍受 ACL Write 权限检查。
    pub fn merge_remote_fields(&self, remote: CrdtVersion, from: &str) -> bool {
        if !self
            .acl
            .check(from, &remote.memory_id, AclPermission::Write)
        {
            warn!(
                target: "nebula.swarm.crdt",
                agent = %from,
                memory_id = %remote.memory_id,
                "CRDT field merge rejected by ACL (no Write permission)"
            );
            return false;
        }

        let mut versions = self.versions.write();
        match versions.get(&remote.memory_id) {
            Some(local) => {
                let result = self.engine.merge_fields(local, &remote);
                versions.insert(remote.memory_id.clone(), result.winner.clone());
            }
            None => {
                versions.insert(remote.memory_id.clone(), remote.clone());
            }
        }
        true
    }

    // ------------------------------------------------------------------
    // 读取路径(ACL 过滤)
    // ------------------------------------------------------------------

    /// 读取某条记忆的当前版本(**ACL Read 过滤**)。
    ///
    /// 若 `requester` 无 Read 权限,返回 `None`。
    pub fn get_memory(&self, memory_id: &str, requester: &str) -> Option<CrdtVersion> {
        if !self.acl.check(requester, memory_id, AclPermission::Read) {
            warn!(
                target: "nebula.swarm.crdt",
                agent = %requester,
                memory_id = %memory_id,
                "CRDT read rejected by ACL (no Read permission)"
            );
            return None;
        }
        self.versions.read().get(memory_id).cloned()
    }

    /// 列出所有可见记忆(**ACL Read 过滤**)。
    ///
    /// 仅返回 `requester` 有 Read 权限的条目。
    pub fn list_memories(&self, requester: &str) -> Vec<CrdtVersion> {
        let versions = self.versions.read();
        versions
            .values()
            .filter(|v| self.acl.check(requester, &v.memory_id, AclPermission::Read))
            .cloned()
            .collect()
    }

    // ------------------------------------------------------------------
    // Bus 消息处理
    // ------------------------------------------------------------------

    /// 处理从 AgentBus 接收到的 `CrdtSync` 消息。
    ///
    /// 反序列化 `content` 为 `CrdtVersion`,调用 `merge_remote`。
    /// 非 CrdtSync 消息会被忽略(返回 `false`)。
    pub fn handle_bus_message(&self, msg: &BusMessage) -> bool {
        if msg.msg_type != BusMessageType::CrdtSync {
            return false;
        }
        match serde_json::from_str::<CrdtVersion>(&msg.content) {
            Ok(version) => self.merge_remote(version, &msg.from),
            Err(e) => {
                warn!(
                    target: "nebula.swarm.crdt",
                    error = %e,
                    "failed to deserialize CrdtVersion from bus message"
                );
                false
            }
        }
    }

    /// 启动一个后台任务,订阅 AgentBus 广播,自动处理 CrdtSync 消息。
    ///
    /// 返回 `JoinHandle`,调用方可持有以便 shutdown 时 abort。
    pub fn start_sync_worker(self: Arc<Self>, bus: Arc<AgentBus>) -> tokio::task::JoinHandle<()> {
        let mut rx = bus.subscribe();
        tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                self.handle_bus_message(&msg);
            }
        })
    }
}

impl Default for SwarmCrdtSync {
    fn default() -> Self {
        Self::new("default-device", MemoryAcl::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::acl::{AclEffect, AclPermission, AclRule};
    use serde_json::json;

    fn make_acl() -> MemoryAcl {
        // 默认允许 system/owner/local;其他需显式规则。
        let mut acl = MemoryAcl::new();
        acl.add_rule(AclRule {
            principal: "agent-1".to_string(),
            resource: "*".to_string(),
            permission: AclPermission::Read,
            effect: AclEffect::Allow,
        });
        acl.add_rule(AclRule {
            principal: "agent-1".to_string(),
            resource: "*".to_string(),
            permission: AclPermission::Write,
            effect: AclEffect::Allow,
        });
        acl.add_rule(AclRule {
            principal: "agent-2".to_string(),
            resource: "shared-1".to_string(),
            permission: AclPermission::Read,
            effect: AclEffect::Allow,
        });
        acl.add_rule(AclRule {
            principal: "agent-2".to_string(),
            resource: "private-1".to_string(),
            permission: AclPermission::Read,
            effect: AclEffect::Deny,
        });
        acl
    }

    fn make_field_change(field: &str, new_value: serde_json::Value) -> FieldChange {
        FieldChange {
            field: field.to_string(),
            old_value: json!(null),
            new_value,
        }
    }

    #[test]
    fn apply_local_change_stores_and_increments_version() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        let bus = AgentBus::new();

        let v1 = sync.apply_local_change(
            "m1",
            vec![make_field_change("content", json!("hello"))],
            &bus,
        );
        assert_eq!(v1, 1);

        let v2 = sync.apply_local_change(
            "m1",
            vec![make_field_change("content", json!("world"))],
            &bus,
        );
        assert_eq!(v2, 2);

        assert_eq!(sync.len(), 1);
    }

    #[test]
    fn merge_remote_accepts_when_acl_allows() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        let remote = CrdtVersion {
            memory_id: "shared-1".to_string(),
            version: 1,
            device_id: "dev-b".to_string(),
            timestamp: 100,
            field_changes: vec![make_field_change("content", json!("remote"))],
        };
        // agent-1 对 * 有 Write 权限。
        assert!(sync.merge_remote(remote, "agent-1"));
        assert_eq!(sync.len(), 1);
    }

    #[test]
    fn merge_remote_rejects_when_acl_denies_write() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        let remote = CrdtVersion {
            memory_id: "m1".to_string(),
            version: 1,
            device_id: "dev-b".to_string(),
            timestamp: 100,
            field_changes: vec![make_field_change("content", json!("remote"))],
        };
        // agent-2 无 Write 权限(规则只给了 Read)。
        assert!(!sync.merge_remote(remote, "agent-2"));
        assert_eq!(sync.len(), 0);
    }

    #[test]
    fn merge_remote_lww_resolves_conflict() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        // 本地先写入 timestamp=100。
        sync.versions.write().insert(
            "shared-1".to_string(),
            CrdtVersion {
                memory_id: "shared-1".to_string(),
                version: 1,
                device_id: "dev-a".to_string(),
                timestamp: 100,
                field_changes: vec![make_field_change("content", json!("local-old"))],
            },
        );
        // 远端 timestamp=200,应胜出。
        let remote = CrdtVersion {
            memory_id: "shared-1".to_string(),
            version: 1,
            device_id: "dev-b".to_string(),
            timestamp: 200,
            field_changes: vec![make_field_change("content", json!("remote-new"))],
        };
        assert!(sync.merge_remote(remote, "agent-1"));
        let stored = sync.versions.read().get("shared-1").cloned().expect("get should succeed");
        assert_eq!(stored.device_id, "dev-b");
        assert_eq!(stored.timestamp, 200);
    }

    #[test]
    fn get_memory_acl_filters_read() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        sync.versions.write().insert(
            "private-1".to_string(),
            CrdtVersion {
                memory_id: "private-1".to_string(),
                version: 1,
                device_id: "dev-a".to_string(),
                timestamp: 100,
                field_changes: vec![],
            },
        );
        sync.versions.write().insert(
            "shared-1".to_string(),
            CrdtVersion {
                memory_id: "shared-1".to_string(),
                version: 1,
                device_id: "dev-a".to_string(),
                timestamp: 100,
                field_changes: vec![],
            },
        );

        // agent-1 对 * 有 Read 权限,可读全部。
        assert_eq!(sync.list_memories("agent-1").len(), 2);
        assert!(sync.get_memory("private-1", "agent-1").is_some());

        // agent-2 对 private-* 被 Deny,对 shared-* Allow。
        assert_eq!(sync.list_memories("agent-2").len(), 1);
        assert_eq!(sync.list_memories("agent-2")[0].memory_id, "shared-1");
        assert!(sync.get_memory("private-1", "agent-2").is_none());
        assert!(sync.get_memory("shared-1", "agent-2").is_some());
    }

    #[test]
    fn handle_bus_message_processes_crdt_sync() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        let version = CrdtVersion {
            memory_id: "shared-1".to_string(),
            version: 1,
            device_id: "dev-b".to_string(),
            timestamp: 100,
            field_changes: vec![make_field_change("content", json!("from-bus"))],
        };
        let msg = BusMessage {
            from: "agent-1".to_string(),
            to: None,
            content: serde_json::to_string(&version).expect("serialize should succeed"),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: BusMessageType::CrdtSync,
            correlation_id: None,
        };
        assert!(sync.handle_bus_message(&msg));
        assert_eq!(sync.len(), 1);
    }

    #[test]
    fn handle_bus_message_ignores_non_crdt() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        let msg = BusMessage {
            from: "agent-1".to_string(),
            to: None,
            content: "hello".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: BusMessageType::Notification,
            correlation_id: None,
        };
        assert!(!sync.handle_bus_message(&msg));
        assert_eq!(sync.len(), 0);
    }

    #[test]
    fn handle_bus_message_rejects_invalid_payload() {
        let sync = SwarmCrdtSync::new("dev-a", make_acl());
        let msg = BusMessage {
            from: "agent-1".to_string(),
            to: None,
            content: "not-json".to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: BusMessageType::CrdtSync,
            correlation_id: None,
        };
        assert!(!sync.handle_bus_message(&msg));
        assert_eq!(sync.len(), 0);
    }

    #[tokio::test]
    async fn start_sync_worker_auto_merges_broadcasts() {
        let acl = make_acl();
        let sync = Arc::new(SwarmCrdtSync::new("dev-a", acl));
        let bus = Arc::new(AgentBus::new());

        let _handle = sync.clone().start_sync_worker(bus.clone());

        // 通过 bus 广播一个 CrdtSync 消息(from=agent-1,有 Write 权限)。
        let version = CrdtVersion {
            memory_id: "shared-1".to_string(),
            version: 1,
            device_id: "dev-b".to_string(),
            timestamp: 100,
            field_changes: vec![make_field_change("content", json!("worker-test"))],
        };
        bus.broadcast(BusMessage {
            from: "agent-1".to_string(),
            to: None,
            content: serde_json::to_string(&version).expect("serialize should succeed"),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: BusMessageType::CrdtSync,
            correlation_id: None,
        });

        // 等待 worker 处理。
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(sync.len(), 1);
    }

    #[test]
    fn default_creates_empty_sync() {
        let sync = SwarmCrdtSync::default();
        assert!(sync.is_empty());
        assert_eq!(sync.device_id(), "default-device");
    }
}
