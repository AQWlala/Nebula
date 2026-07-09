//! T-E-C-19: 多端协同 — 设备发现、会话同步、冲突解决。
//!
//! 让多台设备上的 Nebula 实例可以协同工作。本模块提供:
//! - [`DeviceInfo`] / [`DeviceRegistry`]: 设备发现与注册表。
//! - [`SyncSession`]: 与对等设备建立同步会话,拉取/推送 memory 与 settings。
//! - [`SyncItem`] / [`SyncReport`]: 同步数据单元与同步报告。
//! - [`ConflictResolution`] / [`ConflictResolver`]: 冲突检测与解决。
//!
//! 设计说明:
//! - 仅依赖 `anyhow` + `serde` + `chrono` + `uuid` + `tracing`,
//!   不引入新依赖,不依赖任何 feature gate。
//! - 时间戳统一使用 `chrono::Utc::now().timestamp()`(Unix 秒, i64)。
//! - source = 远端(发起同步的对等设备);target = 本地。
//!   [`ConflictResolution::SourceWins`] 偏向远端,
//!   [`ConflictResolution::TargetWins`] 偏向本地。
//! - 本模块为纯逻辑骨架,不注册 Tauri 命令,不修改既有文件。

use std::collections::HashMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

/// 设备 ID 类型别名。
pub type DeviceId = String;

/// 设备信息。
///
/// 描述一个参与协同的 Nebula 实例。`id` 在整个协同拓扑中唯一,
/// 通常由 `uuid::Uuid::new_v4()` 生成。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// 设备唯一标识。
    pub id: String,
    /// 用户可读的设备名(如 "Alice 的 MacBook")。
    pub name: String,
    /// 平台标识: "windows" / "macos" / "linux" / "ios" / "android"。
    pub platform: String,
    /// Nebula 版本号,如 "2.0.0"。
    pub nebula_version: String,
    /// 最后一次见到该设备的时间(Unix 秒)。
    pub last_seen: i64,
    /// 设备在网络中的地址(IPv4/IPv6/hostname),用于直连发现。
    pub ip_addr: String,
}

/// 设备注册表。
///
/// 维护当前已知的所有协同设备。内存态实现;持久化由上层负责。
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    devices: HashMap<String, DeviceInfo>,
}

impl DeviceRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一台设备。若 `id` 已存在则覆盖(更新)。
    /// `last_seen` 会被刷新为当前时间。返回设备 ID。
    pub fn register(&mut self, mut device: DeviceInfo) -> Result<DeviceId> {
        if device.id.is_empty() {
            bail!("device id must not be empty");
        }
        if device.name.is_empty() {
            bail!("device name must not be empty");
        }
        device.last_seen = chrono::Utc::now().timestamp();
        let id = device.id.clone();
        if self.devices.contains_key(&id) {
            info!(
                target: "nebula.sync.multi_device",
                device_id = %id, "device re-registered (updated)"
            );
        }
        self.devices.insert(id.clone(), device);
        Ok(id)
    }

    /// 注销一台设备。设备不存在时返回错误。
    pub fn unregister(&mut self, device_id: &str) -> Result<()> {
        if self.devices.remove(device_id).is_none() {
            bail!("device not found: {}", device_id);
        }
        Ok(())
    }

    /// 列出所有已注册设备。
    pub fn list_devices(&self) -> Vec<DeviceInfo> {
        self.devices.values().cloned().collect()
    }

    /// 按名称查找设备(返回第一个匹配项)。
    pub fn find_by_name(&self, name: &str) -> Option<DeviceInfo> {
        self.devices.values().find(|d| d.name == name).cloned()
    }

    /// 按 ID 查找设备。
    pub fn find_by_id(&self, device_id: &str) -> Option<DeviceInfo> {
        self.devices.get(device_id).cloned()
    }

    /// 已注册设备数量。
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
}

/// 同步项 — 同步的最小数据单元。
///
/// 一个 `SyncItem` 代表 memory 条目或 settings 键值对的一个版本。
/// `timestamp` + `device_id` 共同标识该版本的来源与因果顺序。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    /// 数据键(如 memory id 或 settings 路径)。
    pub key: String,
    /// 数据值(JSON 可序列化)。
    pub value: serde_json::Value,
    /// 该版本的写入时间(Unix 秒)。
    pub timestamp: i64,
    /// 最初产生该数据的设备(source of truth)。
    pub source_device: String,
    /// 最近一次修改该数据的设备。
    pub device_id: String,
}

/// 冲突解决策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// 最后写入胜(LWW): 比较 `timestamp`,新者胜。
    LastWriteWins,
    /// 源端胜: 保留远端(source)版本。
    SourceWins,
    /// 目标端胜: 保留本地(target)版本。
    TargetWins,
    /// 合并: 字段级合并(对象字段合并,非对象退化为数组)。
    Merge,
    /// 手动: 不自动解决,保留本地版本,等待用户裁决。
    Manual,
}

impl Default for ConflictResolution {
    fn default() -> Self {
        ConflictResolution::LastWriteWins
    }
}

/// 冲突解决器。
#[derive(Debug, Clone)]
pub struct ConflictResolver {
    /// 解决策略。
    pub strategy: ConflictResolution,
}

impl Default for ConflictResolver {
    fn default() -> Self {
        Self {
            strategy: ConflictResolution::default(),
        }
    }
}

impl ConflictResolver {
    /// 创建指定策略的解决器。
    pub fn new(strategy: ConflictResolution) -> Self {
        Self { strategy }
    }

    /// 在本地(target)与远端(source)项之间应用解决策略,
    /// 返回解决后的 [`SyncItem`]。
    ///
    /// - [`ConflictResolution::LastWriteWins`]: `timestamp` 大者胜,
    ///   相等时偏向远端(让新数据流入)。
    /// - [`ConflictResolution::SourceWins`]: 返回远端。
    /// - [`ConflictResolution::TargetWins`]: 返回本地。
    /// - [`ConflictResolution::Merge`]: 合并值(见 [`merge_items`])。
    /// - [`ConflictResolution::Manual`]: 返回本地,标记需人工裁决。
    pub fn apply_resolution(&self, local: &SyncItem, remote: &SyncItem) -> SyncItem {
        match self.strategy {
            ConflictResolution::LastWriteWins => {
                if remote.timestamp >= local.timestamp {
                    remote.clone()
                } else {
                    local.clone()
                }
            }
            ConflictResolution::SourceWins => remote.clone(),
            ConflictResolution::TargetWins => local.clone(),
            ConflictResolution::Merge => merge_items(local, remote),
            ConflictResolution::Manual => local.clone(),
        }
    }
}

/// 合并两个同步项的值。
///
/// - 两者均为 JSON 对象时: 字段级合并,同名字段以远端为准。
/// - 否则: 退化为 `[local, remote]` 数组,保留两侧数据。
/// 合并后的 `timestamp` 取较大值,`device_id`/`source_device` 取本地
/// (因为合并产物写入本地)。
fn merge_items(local: &SyncItem, remote: &SyncItem) -> SyncItem {
    let merged_value = match (&local.value, &remote.value) {
        (serde_json::Value::Object(l), serde_json::Value::Object(r)) => {
            let mut m = l.clone();
            for (k, v) in r {
                m.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(m)
        }
        _ => serde_json::json!([local.value, remote.value]),
    };
    SyncItem {
        key: local.key.clone(),
        value: merged_value,
        timestamp: local.timestamp.max(remote.timestamp),
        source_device: local.source_device.clone(),
        device_id: local.device_id.clone(),
    }
}

/// 同步报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    /// 成功同步的条目数(含自动解决的冲突)。
    pub items_synced: usize,
    /// 自动解决的冲突数。
    pub conflicts_resolved: usize,
    /// 同步过程中遇到的错误(非致命,逐条记录)。
    pub errors: Vec<String>,
}

/// 同步会话状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// 活跃,可继续同步。
    Active,
    /// 已关闭,不可再同步。
    Closed,
}

/// 同步会话。
///
/// 一次 `SyncSession` 代表与某台对等设备的一次同步上下文。
/// `local_items` 为本地(target)侧数据,`remote_items` 为远端(source)
/// 侧待并入的数据。`sync_memory` / `sync_settings` 将远端项合并进本地。
pub struct SyncSession {
    /// 会话 ID(uuid v4)。
    pub id: String,
    /// 本地设备 ID(target)。
    pub local_device_id: String,
    /// 对等设备 ID(source)。
    pub peer_device_id: String,
    /// 会话开始时间(Unix 秒)。
    pub started_at: i64,
    /// 会话状态。
    pub state: SessionState,
    /// 本地侧同步项。
    pub local_items: HashMap<String, SyncItem>,
    /// 远端侧同步项(待并入)。
    pub remote_items: HashMap<String, SyncItem>,
    /// 冲突解决器。
    pub resolver: ConflictResolver,
    /// 最近一次同步报告。
    pub last_report: Option<SyncReport>,
}

impl SyncSession {
    /// 启动一个与 `peer_device_id` 的同步会话。
    pub fn start(peer_device_id: &str) -> Result<SyncSession> {
        if peer_device_id.is_empty() {
            bail!("peer device id must not be empty");
        }
        let now = chrono::Utc::now().timestamp();
        info!(
            target: "nebula.sync.multi_device",
            peer_device_id, "sync session started"
        );
        Ok(SyncSession {
            id: uuid::Uuid::new_v4().to_string(),
            local_device_id: String::new(),
            peer_device_id: peer_device_id.to_string(),
            started_at: now,
            state: SessionState::Active,
            local_items: HashMap::new(),
            remote_items: HashMap::new(),
            resolver: ConflictResolver::default(),
            last_report: None,
        })
    }

    /// 同步 memory 条目。将 `remote_items` 并入 `local_items`,
    /// 自动解决冲突,返回同步报告。
    pub fn sync_memory(session: &mut SyncSession) -> Result<SyncReport> {
        Self::sync_items(session)
    }

    /// 同步 settings。语义同 [`SyncSession::sync_memory`],但不返回报告。
    pub fn sync_settings(session: &mut SyncSession) -> Result<()> {
        Self::sync_items(session)?;
        Ok(())
    }

    /// 内部: 执行远端 → 本地的合并。
    fn sync_items(session: &mut SyncSession) -> Result<SyncReport> {
        if session.state == SessionState::Closed {
            bail!("session is closed");
        }
        let mut report = SyncReport::default();
        // 收集远端键,避免在迭代中修改 local_items 时的借用问题。
        let remote_keys: Vec<String> = session.remote_items.keys().cloned().collect();
        for key in remote_keys {
            let remote = match session.remote_items.get(&key) {
                Some(r) => r.clone(),
                None => continue,
            };
            match session.local_items.get(&key) {
                Some(local) => {
                    if local.value == remote.value {
                        // 值相同,无冲突,记一次同步。
                        report.items_synced += 1;
                    } else {
                        // 冲突: 应用解决策略。
                        let resolved = session.resolver.apply_resolution(local, &remote);
                        session.local_items.insert(key, resolved);
                        report.conflicts_resolved += 1;
                        report.items_synced += 1;
                    }
                }
                None => {
                    // 本地不存在,直接采纳远端。
                    session.local_items.insert(key, remote);
                    report.items_synced += 1;
                }
            }
        }
        session.last_report = Some(report.clone());
        Ok(report)
    }

    /// 关闭会话(消费之)。关闭后不可再同步。
    pub fn close(session: SyncSession) -> Result<()> {
        if session.state == SessionState::Closed {
            bail!("session already closed");
        }
        info!(
            target: "nebula.sync.multi_device",
            session_id = %session.id, "sync session closed"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造测试用 `DeviceInfo`。
    fn make_device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.to_string(),
            platform: "windows".to_string(),
            nebula_version: "2.0.0".to_string(),
            last_seen: 0,
            ip_addr: "127.0.0.1".to_string(),
        }
    }

    /// 构造测试用 `SyncItem`。
    fn make_item(key: &str, value: serde_json::Value, ts: i64, device: &str) -> SyncItem {
        SyncItem {
            key: key.to_string(),
            value,
            timestamp: ts,
            source_device: device.to_string(),
            device_id: device.to_string(),
        }
    }

    // ---- DeviceInfo ----

    #[test]
    fn device_info_round_trip_serde() {
        let d = make_device("dev-1", "Laptop");
        let json = serde_json::to_string(&d).expect("serialize");
        let back: DeviceInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, "dev-1");
        assert_eq!(back.name, "Laptop");
        assert_eq!(back.platform, "windows");
        assert_eq!(back.nebula_version, "2.0.0");
        assert_eq!(back.ip_addr, "127.0.0.1");
    }

    // ---- DeviceRegistry ----

    #[test]
    fn registry_register_returns_id() {
        let mut reg = DeviceRegistry::new();
        let id = reg
            .register(make_device("dev-1", "Laptop"))
            .expect("register");
        assert_eq!(id, "dev-1");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_register_rejects_empty_id() {
        let mut reg = DeviceRegistry::new();
        let mut d = make_device("", "NoId");
        d.id = String::new();
        assert!(reg.register(d).is_err());
    }

    #[test]
    fn registry_register_rejects_empty_name() {
        let mut reg = DeviceRegistry::new();
        let mut d = make_device("dev-1", "x");
        d.name = String::new();
        assert!(reg.register(d).is_err());
    }

    #[test]
    fn registry_register_updates_last_seen() {
        let mut reg = DeviceRegistry::new();
        let mut d = make_device("dev-1", "Laptop");
        d.last_seen = 0;
        reg.register(d).expect("register");
        let listed = reg.list_devices();
        assert_eq!(listed.len(), 1);
        // last_seen 应被刷新为当前时间(接近 now)。
        let now = chrono::Utc::now().timestamp();
        assert!(listed[0].last_seen > 0);
        assert!((now - listed[0].last_seen).abs() <= 5);
    }

    #[test]
    fn registry_unregister_removes_device() {
        let mut reg = DeviceRegistry::new();
        reg.register(make_device("dev-1", "Laptop"))
            .expect("register");
        assert!(reg.unregister("dev-1").is_ok());
        assert!(reg.is_empty());
    }

    #[test]
    fn registry_unregister_missing_errors() {
        let mut reg = DeviceRegistry::new();
        assert!(reg.unregister("ghost").is_err());
    }

    #[test]
    fn registry_list_devices_returns_all() {
        let mut reg = DeviceRegistry::new();
        reg.register(make_device("dev-1", "Laptop"))
            .expect("register");
        reg.register(make_device("dev-2", "Phone"))
            .expect("register");
        let mut ids: Vec<String> = reg.list_devices().into_iter().map(|d| d.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["dev-1".to_string(), "dev-2".to_string()]);
    }

    #[test]
    fn registry_find_by_name_found() {
        let mut reg = DeviceRegistry::new();
        reg.register(make_device("dev-1", "Laptop"))
            .expect("register");
        reg.register(make_device("dev-2", "Phone"))
            .expect("register");
        let found = reg.find_by_name("Phone").expect("should find");
        assert_eq!(found.id, "dev-2");
    }

    #[test]
    fn registry_find_by_name_missing_returns_none() {
        let reg = DeviceRegistry::new();
        assert!(reg.find_by_name("Nope").is_none());
    }

    #[test]
    fn registry_find_by_id() {
        let mut reg = DeviceRegistry::new();
        reg.register(make_device("dev-1", "Laptop"))
            .expect("register");
        assert!(reg.find_by_id("dev-1").is_some());
        assert!(reg.find_by_id("missing").is_none());
    }

    // ---- SyncSession ----

    #[test]
    fn sync_session_start_creates_active_session() {
        let s = SyncSession::start("peer-1").expect("start");
        assert!(!s.id.is_empty());
        assert_eq!(s.peer_device_id, "peer-1");
        assert_eq!(s.state, SessionState::Active);
        assert!(s.local_items.is_empty());
        assert!(s.remote_items.is_empty());
    }

    #[test]
    fn sync_session_start_rejects_empty_peer() {
        assert!(SyncSession::start("").is_err());
    }

    #[test]
    fn sync_memory_no_conflicts_merges_disjoint() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.local_items
            .insert("a".into(), make_item("a", json!("local-a"), 100, "me"));
        s.remote_items
            .insert("b".into(), make_item("b", json!("remote-b"), 100, "peer"));

        let report = SyncSession::sync_memory(&mut s).expect("sync");
        // 远端 1 项并入本地(本地无 b),无冲突。
        assert_eq!(report.items_synced, 1);
        assert_eq!(report.conflicts_resolved, 0);
        assert!(s.local_items.contains_key("a"));
        assert!(s.local_items.contains_key("b"));
        assert_eq!(s.local_items["b"].value, json!("remote-b"));
    }

    #[test]
    fn sync_memory_identical_items_no_conflict() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.local_items
            .insert("a".into(), make_item("a", json!("same"), 100, "me"));
        s.remote_items
            .insert("a".into(), make_item("a", json!("same"), 200, "peer"));

        let report = SyncSession::sync_memory(&mut s).expect("sync");
        assert_eq!(report.items_synced, 1);
        assert_eq!(report.conflicts_resolved, 0);
    }

    #[test]
    fn sync_memory_resolves_conflict_lww_newer_remote_wins() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.local_items
            .insert("a".into(), make_item("a", json!("local"), 100, "me"));
        s.remote_items
            .insert("a".into(), make_item("a", json!("remote"), 200, "peer"));

        let report = SyncSession::sync_memory(&mut s).expect("sync");
        assert_eq!(report.items_synced, 1);
        assert_eq!(report.conflicts_resolved, 1);
        // LWW: 远端 timestamp 更大,胜出。
        assert_eq!(s.local_items["a"].value, json!("remote"));
    }

    #[test]
    fn sync_memory_resolves_conflict_lww_older_remote_loses() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.local_items
            .insert("a".into(), make_item("a", json!("local"), 300, "me"));
        s.remote_items
            .insert("a".into(), make_item("a", json!("remote"), 200, "peer"));

        let report = SyncSession::sync_memory(&mut s).expect("sync");
        assert_eq!(report.conflicts_resolved, 1);
        assert_eq!(s.local_items["a"].value, json!("local"));
    }

    #[test]
    fn sync_memory_counts_items_synced_multiple() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.local_items
            .insert("a".into(), make_item("a", json!("la"), 100, "me"));
        s.remote_items
            .insert("b".into(), make_item("b", json!("rb"), 100, "peer"));
        s.remote_items
            .insert("c".into(), make_item("c", json!("rc"), 100, "peer"));
        s.remote_items
            .insert("d".into(), make_item("d", json!("rd"), 100, "peer"));

        let report = SyncSession::sync_memory(&mut s).expect("sync");
        assert_eq!(report.items_synced, 3);
        assert_eq!(report.conflicts_resolved, 0);
    }

    #[test]
    fn sync_memory_empty_remote_is_noop() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.local_items
            .insert("a".into(), make_item("a", json!("la"), 100, "me"));
        let report = SyncSession::sync_memory(&mut s).expect("sync");
        assert_eq!(report.items_synced, 0);
        assert_eq!(report.conflicts_resolved, 0);
        assert_eq!(s.local_items.len(), 1);
    }

    #[test]
    fn sync_settings_returns_ok_and_merges() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.remote_items.insert(
            "theme".into(),
            make_item("theme", json!("dark"), 100, "peer"),
        );
        let res = SyncSession::sync_settings(&mut s);
        assert!(res.is_ok());
        assert!(s.local_items.contains_key("theme"));
    }

    #[test]
    fn sync_on_closed_session_errors() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.state = SessionState::Closed;
        assert!(SyncSession::sync_memory(&mut s).is_err());
    }

    #[test]
    fn close_active_session_ok() {
        let s = SyncSession::start("peer-1").expect("start");
        assert!(SyncSession::close(s).is_ok());
    }

    #[test]
    fn close_already_closed_errors() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.state = SessionState::Closed;
        assert!(SyncSession::close(s).is_err());
    }

    #[test]
    fn sync_session_sets_last_report() {
        let mut s = SyncSession::start("peer-1").expect("start");
        s.remote_items
            .insert("a".into(), make_item("a", json!("ra"), 100, "peer"));
        assert!(s.last_report.is_none());
        let _ = SyncSession::sync_memory(&mut s).expect("sync");
        assert!(s.last_report.is_some());
        assert_eq!(s.last_report.as_ref().unwrap().items_synced, 1);
    }

    // ---- ConflictResolver ----

    #[test]
    fn conflict_resolver_last_write_wins_picks_newer() {
        let r = ConflictResolver::new(ConflictResolution::LastWriteWins);
        let local = make_item("k", json!("l"), 100, "me");
        let remote = make_item("k", json!("r"), 200, "peer");
        assert_eq!(r.apply_resolution(&local, &remote).value, json!("r"));
    }

    #[test]
    fn conflict_resolver_last_write_wins_tie_prefers_remote() {
        let r = ConflictResolver::new(ConflictResolution::LastWriteWins);
        let local = make_item("k", json!("l"), 100, "me");
        let remote = make_item("k", json!("r"), 100, "peer");
        // 相等时偏向远端(>=)。
        assert_eq!(r.apply_resolution(&local, &remote).value, json!("r"));
    }

    #[test]
    fn conflict_resolver_source_wins() {
        let r = ConflictResolver::new(ConflictResolution::SourceWins);
        let local = make_item("k", json!("l"), 300, "me");
        let remote = make_item("k", json!("r"), 100, "peer");
        assert_eq!(r.apply_resolution(&local, &remote).value, json!("r"));
    }

    #[test]
    fn conflict_resolver_target_wins() {
        let r = ConflictResolver::new(ConflictResolution::TargetWins);
        let local = make_item("k", json!("l"), 100, "me");
        let remote = make_item("k", json!("r"), 300, "peer");
        assert_eq!(r.apply_resolution(&local, &remote).value, json!("l"));
    }

    #[test]
    fn conflict_resolver_merge_objects_merges_fields() {
        let r = ConflictResolver::new(ConflictResolution::Merge);
        let local = make_item("k", json!({"a": 1, "b": 2}), 100, "me");
        let remote = make_item("k", json!({"b": 3, "c": 4}), 200, "peer");
        let resolved = r.apply_resolution(&local, &remote);
        assert_eq!(resolved.value, json!({"a": 1, "b": 3, "c": 4}));
        // timestamp 取较大值。
        assert_eq!(resolved.timestamp, 200);
    }

    #[test]
    fn conflict_resolver_merge_non_objects_makes_array() {
        let r = ConflictResolver::new(ConflictResolution::Merge);
        let local = make_item("k", json!("hello"), 100, "me");
        let remote = make_item("k", json!(42), 200, "peer");
        let resolved = r.apply_resolution(&local, &remote);
        assert_eq!(resolved.value, json!(["hello", 42]));
    }

    #[test]
    fn conflict_resolver_manual_keeps_local() {
        let r = ConflictResolver::new(ConflictResolution::Manual);
        let local = make_item("k", json!("l"), 100, "me");
        let remote = make_item("k", json!("r"), 300, "peer");
        assert_eq!(r.apply_resolution(&local, &remote).value, json!("l"));
    }

    #[test]
    fn conflict_resolver_default_is_lww() {
        let r = ConflictResolver::default();
        assert_eq!(r.strategy, ConflictResolution::LastWriteWins);
    }

    // ---- SyncReport ----

    #[test]
    fn sync_report_default_is_empty() {
        let r = SyncReport::default();
        assert_eq!(r.items_synced, 0);
        assert_eq!(r.conflicts_resolved, 0);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn sync_report_serde_round_trip() {
        let r = SyncReport {
            items_synced: 5,
            conflicts_resolved: 2,
            errors: vec!["boom".to_string()],
        };
        let json = serde_json::to_string(&r).expect("serialize");
        let back: SyncReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.items_synced, 5);
        assert_eq!(back.conflicts_resolved, 2);
        assert_eq!(back.errors, vec!["boom".to_string()]);
    }
}
