// M7b #91: channel 模块受 `channels` feature 门控(默认关闭)。
// 用 cfg gate 包裹导入与对应测试,避免默认构建下编译失败。
#[cfg(feature = "channels")]
use nebula_lib::channel::router::WebChatAdapter;
#[cfg(feature = "channels")]
use nebula_lib::channel::ChannelRouter;
#[cfg(feature = "channels")]
use nebula_lib::channel::WebChatService;
use nebula_lib::identity::{DidDocument, DidKey};
use nebula_lib::memory::acl::{AclEffect, AclPermission, AclRule, MemoryAcl};
use nebula_lib::memory::forgetting::{ForgettingConfig, ForgettingEngine};
use nebula_lib::memory::layers::check_auto_promote;
use nebula_lib::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use nebula_lib::security::ssrf_guard::SsrfGuard;
use nebula_lib::skills::audit;
use nebula_lib::swarm::bus::AgentBus;
use nebula_lib::swarm::negotiator::Negotiator;
use nebula_lib::sync::crdt::CrdtEngine;
use nebula_lib::sync::device_manager::DeviceManager;
use std::sync::Arc;

#[test]
#[ignore = "requires SQLite + LanceDB runtime"]
fn test_sponge_absorb_and_search() {}

#[test]
fn test_ssrf_guard_rejects_private_ips() {
    let guard = SsrfGuard::new();
    assert!(guard.validate_url("http://192.168.1.1/api").is_err());
    assert!(guard.validate_url("http://127.0.0.1/api").is_err());
    assert!(guard.validate_url("http://10.0.0.1/api").is_err());
    assert!(guard
        .validate_url("http://169.254.169.254/metadata")
        .is_err());
}

#[test]
fn test_ssrf_guard_allows_public() {
    let guard = SsrfGuard::new();
    assert!(guard.validate_url("https://api.openai.com").is_ok());
    assert!(guard.validate_url("https://api.anthropic.com").is_ok());
}

#[test]
fn test_agent_bus_broadcast() {
    let bus = AgentBus::new();
    let mut sub = bus.subscribe();
    bus.broadcast(nebula_lib::swarm::bus::BusMessage {
        from: "test".to_string(),
        to: None,
        content: "hello".to_string(),
        timestamp: 0,
        msg_type: nebula_lib::swarm::bus::BusMessageType::Notification,
        correlation_id: None,
    });
    let msg = sub.try_recv().unwrap();
    assert_eq!(msg.content, "hello");
}

#[test]
fn test_agent_bus_point_to_point() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let bus = AgentBus::new();
        let mut rx = bus.register("agent-1").await;
        bus.send(nebula_lib::swarm::bus::BusMessage {
            from: "agent-2".to_string(),
            to: Some("agent-1".to_string()),
            content: "ping".to_string(),
            timestamp: 0,
            msg_type: nebula_lib::swarm::bus::BusMessageType::Request,
            correlation_id: None,
        })
        .await
        .unwrap();
        // 包裹 timeout 防止裸 recv 永久挂起导致 nextest 60s 超时。
        // 理论上 send 完成后 recv 立即返回,但加 5s 兜底。
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("recv timed out — message never arrived")
            .expect("channel closed");
        assert_eq!(msg.content, "ping");
    });
}

#[test]
fn test_negotiator_high_confidence() {
    let negotiator = Negotiator::new();
    let outputs = vec![
        nebula_lib::swarm::agents::AgentOutput {
            kind: nebula_lib::swarm::agents::AgentKind::Coder,
            author: "coder".to_string(),
            body: "solution A".to_string(),
            confidence: 0.9,
            // M7b #91: AgentOutput 新增字段(T-E-B-17/18/S-02)。
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
        },
        nebula_lib::swarm::agents::AgentOutput {
            kind: nebula_lib::swarm::agents::AgentKind::Writer,
            author: "writer".to_string(),
            body: "solution B".to_string(),
            confidence: 0.5,
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
        },
    ];
    let result = negotiator.negotiate(outputs);
    assert!(result.chosen.confidence >= 0.9);
}

#[test]
fn test_memory_acl_default_allow() {
    // M7b #91: M2b ACL 重写后默认策略为 deny-all(可信主体 system/owner/local
    // 放行 + 其他拒绝)。user1 非可信主体 → 默认 deny。
    let acl = MemoryAcl::new();
    assert!(!acl.check("user1", "mem1", AclPermission::Read));
}

#[test]
fn test_memory_acl_deny() {
    let mut acl = MemoryAcl::new();
    acl.add_rule(AclRule {
        principal: "user1".into(),
        resource: "mem1".into(),
        permission: AclPermission::Read,
        effect: AclEffect::Deny,
    });
    assert!(!acl.check("user1", "mem1", AclPermission::Read));
}

#[test]
fn test_auto_promote_l3_to_l4() {
    let mem = Memory {
        id: "test".to_string(),
        memory_type: MemoryType::Semantic,
        layer: MemoryLayer::L3,
        content: "test".to_string(),
        summary: Default::default(),
        importance: 0.8,
        access_count: 15,
        last_access: 0,
        created_at: 0,
        source: SourceKind::UserInput,
        metadata: Default::default(),
        compressed_from: None,
        compression_gen: 0,
        pinned: false,
        archived: false,
        embedding: vec![],
        // M7b #91: Memory 新增字段(M2a #28 domain + T-E-A-09 ingest_cost)。
        domain: "shared".to_string(),
        ingest_cost: None,
    };
    let result = check_auto_promote(mem.layer, mem.access_count, mem.importance, mem.pinned);
    assert!(result.is_some());
    assert_eq!(result.unwrap(), MemoryLayer::L4);
}

#[test]
fn test_auto_promote_pinned_no_promote() {
    let mem = Memory {
        id: "test".to_string(),
        memory_type: MemoryType::Semantic,
        layer: MemoryLayer::L3,
        content: "test".to_string(),
        summary: Default::default(),
        importance: 0.9,
        access_count: 50,
        last_access: 0,
        created_at: 0,
        source: SourceKind::UserInput,
        metadata: Default::default(),
        compressed_from: None,
        compression_gen: 0,
        pinned: true,
        archived: false,
        embedding: vec![],
        // M7b #91: Memory 新增字段(M2a #28 domain + T-E-A-09 ingest_cost)。
        domain: "shared".to_string(),
        ingest_cost: None,
    };
    assert!(check_auto_promote(mem.layer, mem.access_count, mem.importance, mem.pinned).is_none());
}

#[test]
fn test_auto_promote_l7_no_promote() {
    let mem = Memory {
        id: "test".to_string(),
        memory_type: MemoryType::Semantic,
        layer: MemoryLayer::L7,
        content: "test".to_string(),
        summary: Default::default(),
        importance: 1.0,
        access_count: 100,
        last_access: 0,
        created_at: 0,
        source: SourceKind::UserInput,
        metadata: Default::default(),
        compressed_from: None,
        compression_gen: 0,
        pinned: false,
        archived: false,
        embedding: vec![],
        // M7b #91: Memory 新增字段(M2a #28 domain + T-E-A-09 ingest_cost)。
        domain: "shared".to_string(),
        ingest_cost: None,
    };
    assert!(check_auto_promote(mem.layer, mem.access_count, mem.importance, mem.pinned).is_none());
}

#[test]
fn test_forgetting_engine_marks_low_importance() {
    let engine = ForgettingEngine::new(ForgettingConfig::default());
    let mem = Memory {
        id: "test".to_string(),
        memory_type: MemoryType::Semantic,
        layer: MemoryLayer::L1,
        content: "test".to_string(),
        summary: Default::default(),
        importance: 0.1,
        access_count: 0,
        last_access: 0,
        created_at: 0,
        source: SourceKind::UserInput,
        metadata: Default::default(),
        compressed_from: None,
        compression_gen: 0,
        pinned: false,
        archived: false,
        embedding: vec![],
        // M7b #91: Memory 新增字段(M2a #28 domain + T-E-A-09 ingest_cost)。
        domain: "shared".to_string(),
        ingest_cost: None,
    };
    let candidates = engine.scan_for_archive(
        vec![(
            mem.id.clone(),
            mem.layer,
            mem.importance,
            mem.last_access,
            mem.pinned,
        )],
        2 * 86400, // now = 2 days; age = 2 days > L1 TTL of 1 day
    );
    assert!(!candidates.is_empty());
}

#[test]
fn test_forgetting_engine_pinned_never() {
    let engine = ForgettingEngine::new(ForgettingConfig::default());
    let mem = Memory {
        id: "test".to_string(),
        memory_type: MemoryType::Semantic,
        layer: MemoryLayer::L1,
        content: "test".to_string(),
        summary: Default::default(),
        importance: 0.1,
        access_count: 0,
        last_access: 0,
        created_at: 0,
        source: SourceKind::UserInput,
        metadata: Default::default(),
        compressed_from: None,
        compression_gen: 0,
        pinned: true,
        archived: false,
        embedding: vec![],
        // M7b #91: Memory 新增字段(M2a #28 domain + T-E-A-09 ingest_cost)。
        domain: "shared".to_string(),
        ingest_cost: None,
    };
    let candidates = engine.scan_for_archive(
        vec![(
            mem.id.clone(),
            mem.layer,
            mem.importance,
            mem.last_access,
            mem.pinned,
        )],
        0,
    );
    assert!(candidates.is_empty());
}

#[test]
fn test_did_key_from_public_key() {
    let pk = [0u8; 32];
    let did_key = DidKey::from_public_key(&pk);
    assert!(did_key.did.starts_with("did:key:"));
    assert!(!did_key.public_key_b64().is_empty());
}

#[test]
fn test_did_document_from_did_key() {
    let pk = [0u8; 32];
    let did_key = DidKey::from_public_key(&pk);
    let doc = DidDocument::from_did_key(&did_key);
    assert_eq!(doc.id, did_key.did);
    assert!(!doc.context.is_empty());
}

#[test]
fn test_crdt_lww_newer_wins() {
    let engine = CrdtEngine::new();
    let local = nebula_lib::sync::crdt::CrdtVersion {
        memory_id: "m1".to_string(),
        version: 1,
        device_id: "dev-1".to_string(),
        timestamp: 1000,
        field_changes: vec![],
    };
    let remote = nebula_lib::sync::crdt::CrdtVersion {
        memory_id: "m1".to_string(),
        version: 2,
        device_id: "dev-2".to_string(),
        timestamp: 2000,
        field_changes: vec![],
    };
    let winner = engine.merge_lww(&local, &remote);
    assert_eq!(winner.winner.device_id, "dev-2");
}

#[test]
fn test_device_manager_revoke() {
    let conn = Arc::new(parking_lot::Mutex::new(
        rusqlite::Connection::open_in_memory().unwrap(),
    ));
    let mut mgr = DeviceManager::new(conn);
    mgr.register_device("dev-1".to_string(), "pk1".to_string(), 1000);
    let result = mgr.revoke_device("dev-1");
    assert!(result.success);
    assert!(mgr.is_device_revoked("dev-1"));
}

#[test]
fn test_skill_audit_redaction() {
    let redacted = audit::redact_if_sensitive("key=sk-abc123def456ghi789jkl012mno345pqr678");
    assert!(!redacted.contains("sk-abc123"));
}

#[cfg(feature = "channels")]
#[test]
fn test_channel_router_register() {
    let router = ChannelRouter::new();
    router.register(std::sync::Arc::new(WebChatAdapter::new()));
    let channels = router.list_channels();
    assert_eq!(channels.len(), 1);
}

#[cfg(feature = "channels")]
#[test]
fn test_webchat_service_session() {
    let svc = WebChatService::new();
    let token = svc.create_session();
    assert!(svc.validate_session(&token));
    assert!(!svc.validate_session("invalid"));
}
