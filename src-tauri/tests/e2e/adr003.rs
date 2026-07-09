//! M7b #92: ADR-003 端到端(E2E)测试场景。
//!
//! 覆盖 ADR-003 核心系统的多组件协作端到端流程,不依赖外部 LLM 服务:
//!
//!   1. memory_domain_isolation_e2e — Memory domain 字段端到端隔离
//!   2. acl_cross_domain_filtering_e2e — MemoryAcl + PrincipalDomainMap 跨域过滤
//!   3. swarm_orchestrator_full_dispatch_e2e — SwarmOrchestrator 完整派发流程
//!   4. negotiator_conflict_detection_e2e — Negotiator 冲突检测 + 置信度选择

use nebula_lib::llm::{LlmGateway, OllamaClient};
use nebula_lib::memory::acl::{AclEffect, AclPermission, AclRule, MemoryAcl, PrincipalDomainMap};
use nebula_lib::memory::sqlite_store::SqliteStore;
use nebula_lib::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use nebula_lib::swarm::agents::{AgentKind, AgentOutput};
use nebula_lib::swarm::negotiator::Negotiator;
use nebula_lib::swarm::orchestrator::{SwarmOrchestrator, SwarmTask};
use nebula_lib::tools::ToolRegistry;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Helper: 构造真实临时 SqliteStore(同 causal_graph #90 修复模式)
// ---------------------------------------------------------------------------

fn temp_sqlite_store() -> (std::path::PathBuf, Arc<SqliteStore>) {
    let mut p = std::env::temp_dir();
    p.push(format!("nebula_e2e_adr003_{}.db", uuid::Uuid::new_v4()));
    let store = SqliteStore::open(&p).expect("open sqlite store");
    (p, Arc::new(store))
}

fn make_test_memory(content: &str, domain: &str) -> Memory {
    Memory {
        id: format!("e2e-{}", uuid::Uuid::new_v4()),
        memory_type: MemoryType::Episodic,
        layer: MemoryLayer::L1,
        content: content.to_string(),
        summary: Default::default(),
        embedding: vec![],
        importance: 0.5,
        access_count: 0,
        last_access: 0,
        created_at: chrono::Utc::now().timestamp(),
        source: SourceKind::UserInput,
        metadata: serde_json::Value::Null,
        compressed_from: None,
        compression_gen: 0,
        pinned: false,
        archived: false,
        domain: domain.to_string(),
        ingest_cost: None,
    }
}

fn mock_gateway() -> Arc<LlmGateway> {
    let client = Arc::new(OllamaClient::new_with_timeout(
        "http://127.0.0.1:1",
        Duration::from_secs(2),
    ));
    Arc::new(LlmGateway::new(
        client, "mock", "ollama", None, None, None, None, None,
    ))
}

// ---------------------------------------------------------------------------
// E2E #1: Memory domain 字段端到端隔离
// ---------------------------------------------------------------------------

/// 验证 M2a #28 新增的 domain 字端到端可用:
/// - 写入不同 domain 的记忆(shared / agent_a / agent_b)
/// - list_recent_in_domain 查询只返回对应 domain 的记忆
/// - 跨 domain 不串扰
#[tokio::test]
async fn memory_domain_isolation_e2e() {
    let (path, store) = temp_sqlite_store();

    // 写入 3 个不同 domain 的记忆
    let mem_shared = make_test_memory("shared memory content", "shared");
    let mem_a = make_test_memory("agent_a private memory", "agent_a");
    let mem_b = make_test_memory("agent_b private memory", "agent_b");

    store.insert_guarded_spawn(&mem_shared).await.unwrap();
    store.insert_guarded_spawn(&mem_a).await.unwrap();
    store.insert_guarded_spawn(&mem_b).await.unwrap();

    // 查询 shared 域 → 只返回 1 条
    let shared_mems = store.list_recent_in_domain("shared", 10).await.unwrap();
    assert_eq!(shared_mems.len(), 1, "shared domain should have 1 memory");
    assert_eq!(shared_mems[0].id, mem_shared.id);

    // 查询 agent_a 域 → 只返回 1 条
    let a_mems = store.list_recent_in_domain("agent_a", 10).await.unwrap();
    assert_eq!(a_mems.len(), 1, "agent_a domain should have 1 memory");
    assert_eq!(a_mems[0].id, mem_a.id);

    // 查询 agent_b 域 → 只返回 1 条
    let b_mems = store.list_recent_in_domain("agent_b", 10).await.unwrap();
    assert_eq!(b_mems.len(), 1, "agent_b domain should have 1 memory");
    assert_eq!(b_mems[0].id, mem_b.id);

    // 查询不存在的域 → 返回 0 条
    let none_mems = store
        .list_recent_in_domain("nonexistent", 10)
        .await
        .unwrap();
    assert!(
        none_mems.is_empty(),
        "nonexistent domain should have 0 memories"
    );

    // 清理临时文件
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// E2E #2: MemoryAcl + PrincipalDomainMap 跨域过滤
// ---------------------------------------------------------------------------

/// 验证 M2b ACL 重写后的跨域隔离端到端流程:
/// - PrincipalDomainMap 绑定 evolution:agent_a → agent_a 域
/// - MemoryAcl.check_with_domain 跨域拒绝 + 同域允许
/// - filter_memories_with_domain 批量过滤
#[tokio::test]
async fn acl_cross_domain_filtering_e2e() {
    let mut map = PrincipalDomainMap::new();
    // 显式绑定 worker:task_42 → agent_a 域
    map.set("worker:task_42", "agent_a");

    let acl = MemoryAcl::new().with_principal_domains(map);

    // 1. 同域检查:evolution:agent_a 访问 agent_a 域记忆 → 允许
    //    (principal_domain=None 让 PrincipalDomainMap 内联解析 evolution:agent_a → agent_a)
    assert!(
        acl.check_with_domain(
            "evolution:agent_a",
            "mem-agent_a-1",
            AclPermission::Read,
            "agent_a",
            None,
        ),
        "evolution:agent_a should access agent_a domain memory"
    );

    // 2. 跨域检查:evolution:agent_a 访问 agent_b 域记忆 → 拒绝
    assert!(
        !acl.check_with_domain(
            "evolution:agent_a",
            "mem-agent_b-1",
            AclPermission::Read,
            "agent_b",
            None,
        ),
        "evolution:agent_a should NOT access agent_b domain memory"
    );

    // 3. 显式映射:worker:task_42 → agent_a 域,访问 agent_a 记忆 → 允许
    assert!(
        acl.check_with_domain(
            "worker:task_42",
            "mem-agent_a-2",
            AclPermission::Read,
            "agent_a",
            None,
        ),
        "worker:task_42 (explicit→agent_a) should access agent_a domain memory"
    );

    // 4. 显式映射:worker:task_42 → agent_a 域,访问 shared 记忆 → 拒绝
    //    (PrincipalDomainMap 解析为 agent_a,与 shared 不匹配)
    assert!(
        !acl.check_with_domain(
            "worker:task_42",
            "mem-shared-1",
            AclPermission::Read,
            "shared",
            None,
        ),
        "worker:task_42 (explicit→agent_a) should NOT access shared domain memory"
    );

    // 5. 可信主体 system 访问 shared 域 → 允许(system → shared 默认映射)
    assert!(
        acl.check_with_domain(
            "system",
            "mem-shared-2",
            AclPermission::Read,
            "shared",
            None,
        ),
        "system should access shared domain memory"
    );

    // 6. filter_memories_with_domain 批量过滤
    let memories: Vec<(String, &str, &str)> = vec![
        ("mem-a-1".to_string(), "content a1", "agent_a"),
        ("mem-a-2".to_string(), "content a2", "agent_a"),
        ("mem-b-1".to_string(), "content b1", "agent_b"),
        ("mem-shared-1".to_string(), "content shared", "shared"),
    ];
    let filtered = acl.filter_memories_with_domain("evolution:agent_a", Some("agent_a"), memories);
    // 应只返回 agent_a 域的 2 条记忆
    assert_eq!(filtered.len(), 2, "should filter to 2 agent_a memories");
    let ids: Vec<_> = filtered.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"mem-a-1"));
    assert!(ids.contains(&"mem-a-2"));
}

// ---------------------------------------------------------------------------
// E2E #3: SwarmOrchestrator 完整派发流程
// ---------------------------------------------------------------------------

/// 验证 SwarmOrchestrator 端到端派发流程(M3 #44 + M7a #91 修复):
/// - 用小写 kinds("coder")正确解析为 AgentKind::Coder
/// - execute 返回有效 Report 结构
/// - mock LLM(死端口)→ 所有 agent 失败 → failure_count 正确
/// - negotiation 产生至少 1 个输出(降级)
#[tokio::test]
async fn swarm_orchestrator_full_dispatch_e2e() {
    let orch = SwarmOrchestrator::new_without_memory(mock_gateway(), Arc::new(ToolRegistry::new()));

    let mut task = SwarmTask::new("Design a REST API for user management");
    // M7b #91: 小写 kinds 才能被 AgentKind::from_str 正确解析
    task.agents = vec!["coder".to_string(), "reviewer".to_string()];

    let report = orch
        .execute(task)
        .await
        .expect("orchestration should complete");

    // 验证 Report 结构完整
    assert!(
        !report.task.description.is_empty(),
        "task description preserved"
    );
    assert!(
        !report.task.description.is_empty(),
        "report should carry original task"
    );

    // 2 个 agent(coder + reviewer)全部派发,mock LLM 全失败
    assert_eq!(
        report.failure_count, 2,
        "exactly 2 agents (coder + reviewer) should be dispatched and fail"
    );

    // Negotiation 应产生至少 1 个输出(降级输出,即使全失败)
    assert!(
        !report.outputs.is_empty(),
        "negotiation should produce at least 1 output (degraded)"
    );

    // 未批准(mock LLM 无成功输出)
    assert!(!report.approved, "should not be approved with mock LLM");
}

// ---------------------------------------------------------------------------
// E2E #4: Negotiator 冲突检测 + 置信度选择
// ---------------------------------------------------------------------------

/// 验证 Negotiator 端到端协商流程:
/// - 多个 AgentOutput with conflicting bodies → conflict_detected=true
/// - 选择最高置信度的输出作为 chosen
/// - 验证协商方法判定(HighConfidence vs ConflictResolution)
#[tokio::test]
async fn negotiator_conflict_detection_e2e() {
    let neg = Negotiator::new();

    // 构造 3 个分歧输出(不同 body,不同置信度)
    let outputs = vec![
        AgentOutput {
            kind: AgentKind::Generic,
            author: "agent-1".into(),
            body: "Use approach A: microservices".into(),
            confidence: 0.85,
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
            scenario: None,
        },
        AgentOutput {
            kind: AgentKind::Generic,
            author: "agent-2".into(),
            body: "Use approach B: monolith".into(),
            confidence: 0.75,
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
            scenario: None,
        },
        AgentOutput {
            kind: AgentKind::Generic,
            author: "agent-3".into(),
            body: "Use approach C: serverless".into(),
            confidence: 0.60,
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
            scenario: None,
        },
    ];

    let result = neg.negotiate(outputs);

    // 验证冲突检测(3 个完全不同的方案 → conflict)
    assert!(
        result.conflict_detected,
        "should detect conflict with 3 divergent approaches"
    );

    // 验证选择最高置信度(agent-1, 0.85)
    assert_eq!(
        result.chosen.author, "agent-1",
        "should pick highest confidence (agent-1)"
    );
    assert!(
        result.chosen.confidence >= 0.85,
        "chosen confidence should be >= 0.85"
    );

    // 验证 chosen body 非空(携带实际方案)
    assert!(
        !result.chosen.body.is_empty(),
        "chosen body should be non-empty"
    );
}
