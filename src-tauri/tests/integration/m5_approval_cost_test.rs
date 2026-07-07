//! M5 #75-76: L4 审批门禁 + CostPolicy + CostTracker work_type 集成测试。
//!
//! 覆盖场景：
//! 1. **审批流端到端**：assess → ConfirmRequired → mark_confirmed → Confirmed
//! 2. **5 分钟超时**：注册 confirmation → 推进时间 → Expired
//! 3. **防重放**：同一 confirmation_id 二次确认 → AlreadyUsed
//! 4. **L5 后台 bypass**：AiSelfModify + L5Background + bypass=true → Allow
//! 5. **CostPolicy 本地豁免**：is_local=true → Allow（不消耗配额）
//! 6. **CostPolicy 任务级上限**：超出 max_tokens_per_task → TaskLimitExceeded
//! 7. **CostPolicy 每日上限**：超出 daily_task_limit → DailyLimitExceeded
//! 8. **CostRecord work_type 序列化**：含 work_type 字段的 JSON 往返
//! 9. **AiSelfModify 强制 High**：assess(AiSelfModify) → High（不可降级）
//! 10. **GC 清理**：gc() 移除已确认 + 已过期条目

use std::sync::Arc;

use nebula_lib::autonomy::{
    ApprovalGate, ApprovalVerdict, AutonomyLevel, ConfirmationRegistry, ConfirmationStatus,
    PendingConfirmation, RiskTier, WorkerRiskMap,
};
use nebula_lib::llm::{CostDecision, CostPolicy, CostRecord};
use nebula_lib::memory::values::risk_assessor::ActionKind;

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

/// 构造默认配置的 ApprovalGate（bypass_background_ai_self_modify = true）。
fn make_gate() -> ApprovalGate {
    let registry = Arc::new(ConfirmationRegistry::new());
    ApprovalGate::new(WorkerRiskMap::new(), registry)
}

// ---------------------------------------------------------------------------
// 审批流端到端
// ---------------------------------------------------------------------------

/// 验证完整审批流：
/// assess(High 风险动作) → ConfirmRequired → check → mark_confirmed → Confirmed
#[test]
fn approval_flow_high_risk_confirm_then_acknowledge() {
    let gate = make_gate();
    let autonomy = AutonomyLevel::L2Chat;

    // AiSelfModify 强制 High
    let verdict = gate.assess(
        ActionKind::AiSelfModify,
        "write SOUL.md evolution-append",
        autonomy,
        Some("@@ -1 +1 @@\n-old\n+new".to_string()),
    );

    match verdict {
        ApprovalVerdict::ConfirmRequired {
            risk_tier,
            confirmation_id,
            diff,
            ..
        } => {
            assert!(matches!(risk_tier, RiskTier::High));
            assert!(!confirmation_id.is_empty());
            assert!(diff.is_some(), "AiSelfModify should carry diff");
            let diff_str = diff.unwrap();
            assert!(
                diff_str.contains("@@ -1 +1 @@"),
                "diff should contain hunk header"
            );

            // 首次 check → Confirmed（未消费）
            assert_eq!(
                gate.check_confirmation(&confirmation_id),
                ConfirmationStatus::Confirmed
            );
            // mark_confirmed → Confirmed（首次消费）
            assert_eq!(
                gate.mark_confirmed(&confirmation_id),
                ConfirmationStatus::Confirmed
            );
        }
        other => panic!("expected ConfirmRequired, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 5 分钟超时
// ---------------------------------------------------------------------------

/// 验证 confirmation 在 5 分钟后过期。
#[test]
fn confirmation_expires_after_5_minutes() {
    let registry = Arc::new(ConfirmationRegistry::new());
    let old_id = "old-id".to_string();
    // 手动塞一条 6 分钟前创建的 pending。
    let six_min_ago = chrono::Utc::now().timestamp_millis() - (6 * 60 * 1000);
    registry.register(PendingConfirmation {
        confirmation_id: old_id.clone(),
        action_kind: ActionKind::AiSelfModify,
        risk_tier: RiskTier::High,
        prompt: "old".to_string(),
        diff: None,
        created_at: six_min_ago,
        confirmed_at: None,
    });

    // check → Expired
    assert_eq!(
        registry.check(&old_id),
        ConfirmationStatus::Expired,
        "6-minute-old confirmation should be Expired"
    );
    // mark → Expired
    assert_eq!(
        registry.mark_confirmed(&old_id),
        ConfirmationStatus::Expired,
        "mark_confirmed on expired should return Expired"
    );
}

// ---------------------------------------------------------------------------
// 防重放
// ---------------------------------------------------------------------------

/// 验证同一 confirmation_id 二次确认返回 AlreadyUsed。
#[test]
fn confirmation_replay_attack_blocked() {
    let gate = make_gate();
    let autonomy = AutonomyLevel::L2Chat;

    let verdict = gate.assess(ActionKind::Transfer, "transfer 1000 USD", autonomy, None);
    let confirmation_id = match verdict {
        ApprovalVerdict::ConfirmRequired {
            confirmation_id, ..
        } => confirmation_id,
        _ => panic!("expected ConfirmRequired"),
    };

    // 首次确认 → Confirmed
    let first = gate.mark_confirmed(&confirmation_id);
    assert_eq!(first, ConfirmationStatus::Confirmed);

    // 二次确认（重放攻击）→ AlreadyUsed
    let second = gate.mark_confirmed(&confirmation_id);
    assert_eq!(
        second,
        ConfirmationStatus::AlreadyUsed,
        "expected AlreadyUsed on replay, got {:?}",
        second
    );
}

// ---------------------------------------------------------------------------
// L5 后台 bypass
// ---------------------------------------------------------------------------

/// 验证 L5Background + AiSelfModify + bypass=true 时放行。
#[test]
fn l5_background_ai_self_modify_bypassed() {
    let gate = make_gate();
    let autonomy = AutonomyLevel::L5Background;

    let verdict = gate.assess(
        ActionKind::AiSelfModify,
        "background evolution phase 4",
        autonomy,
        Some("## [evolve_x] new lesson".to_string()),
    );

    match verdict {
        ApprovalVerdict::Allow { risk_tier, .. } => {
            // 仍然标记为 High 风险，但放行执行。
            assert!(
                matches!(risk_tier, RiskTier::High),
                "L5 bypass should still mark High risk"
            );
        }
        other => panic!("expected Allow for L5 background bypass, got {:?}", other),
    }
}

/// 验证 L5Background + 非 AiSelfModify 的 High 风险动作仍需确认。
#[test]
fn l5_background_bulk_delete_still_requires_confirm() {
    let gate = make_gate();
    let autonomy = AutonomyLevel::L5Background;

    let verdict = gate.assess(ActionKind::BulkDelete, "delete 100 files", autonomy, None);

    // bypass 只针对 AiSelfModify，BulkDelete 仍需确认。
    match verdict {
        ApprovalVerdict::ConfirmRequired { .. } => {}
        other => panic!(
            "expected ConfirmRequired for BulkDelete even at L5, got {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// CostPolicy 本地豁免
// ---------------------------------------------------------------------------

/// 验证 is_local=true 时 CostPolicy 直接 Allow（不消耗配额）。
#[test]
fn cost_policy_local_call_exempt_from_limits() {
    let policy = CostPolicy {
        max_tokens_per_task: Some(1000),
        daily_task_limit: Some(10),
    };

    // 即便超出 token 上限,本地调用仍 Allow。
    let decision = policy.check(true, false, 2000, 500, 20);
    assert!(matches!(decision, CostDecision::Allow));
}

/// 验证 is_local_only_work_type=true 时 CostPolicy 直接 Allow。
#[test]
fn cost_policy_local_only_work_type_exempt() {
    let policy = CostPolicy {
        max_tokens_per_task: Some(1000),
        daily_task_limit: Some(10),
    };

    // Evolution/SoulCompile/Classifier 是 local-only,即便远端 fallback 也豁免。
    let decision = policy.check(false, true, 2000, 500, 20);
    assert!(matches!(decision, CostDecision::Allow));
}

// ---------------------------------------------------------------------------
// CostPolicy 任务级上限
// ---------------------------------------------------------------------------

/// 验证超出 max_tokens_per_task 时返回 TaskLimitExceeded。
#[test]
fn cost_policy_task_limit_exceeded() {
    let policy = CostPolicy {
        max_tokens_per_task: Some(1000),
        daily_task_limit: None,
    };

    // 已用 800 + 新增 300 = 1100 > 1000
    let decision = policy.check(false, false, 800, 300, 0);
    match decision {
        CostDecision::TaskLimitExceeded { used, added, limit } => {
            assert_eq!(used, 800);
            assert_eq!(added, 300);
            assert_eq!(limit, 1000);
        }
        other => panic!("expected TaskLimitExceeded, got {:?}", other),
    }
}

/// 验证未超出 max_tokens_per_task 时 Allow。
#[test]
fn cost_policy_task_limit_not_exceeded() {
    let policy = CostPolicy {
        max_tokens_per_task: Some(1000),
        daily_task_limit: None,
    };

    // 已用 500 + 新增 300 = 800 < 1000
    let decision = policy.check(false, false, 500, 300, 0);
    assert!(matches!(decision, CostDecision::Allow));
}

// ---------------------------------------------------------------------------
// CostPolicy 每日上限
// ---------------------------------------------------------------------------

/// 验证超出 daily_task_limit 时返回 DailyLimitExceeded。
#[test]
fn cost_policy_daily_limit_exceeded() {
    let policy = CostPolicy {
        max_tokens_per_task: None,
        daily_task_limit: Some(10),
    };

    // 今日已 10 次 >= limit 10
    let decision = policy.check(false, false, 0, 100, 10);
    match decision {
        CostDecision::DailyLimitExceeded { today_count, limit } => {
            assert_eq!(today_count, 10);
            assert_eq!(limit, 10);
        }
        other => panic!("expected DailyLimitExceeded, got {:?}", other),
    }
}

/// 验证 task_limit 在 daily_limit 之前检查（顺序优先级）。
#[test]
fn cost_policy_task_limit_checked_before_daily_limit() {
    let policy = CostPolicy {
        max_tokens_per_task: Some(1000),
        daily_task_limit: Some(10),
    };

    // 两个上限都触发 → 返回 TaskLimitExceeded（优先）
    let decision = policy.check(false, false, 800, 300, 10);
    assert!(
        matches!(decision, CostDecision::TaskLimitExceeded { .. }),
        "task_limit should be checked before daily_limit, got {:?}",
        decision
    );
}

// ---------------------------------------------------------------------------
// CostRecord work_type 序列化
// ---------------------------------------------------------------------------

/// 验证 CostRecord 含 work_type 字段的 JSON 序列化/反序列化往返。
#[test]
fn cost_record_with_work_type_roundtrip() {
    let record = CostRecord::new_with_work_type(
        "qwen2.5:7b",
        100,
        50,
        Some("ollama".to_string()),
        Some("evolution".to_string()),
        Some("evolution_engine".to_string()),
        Some("evolution".to_string()),
    );

    let json = serde_json::to_string(&record).expect("serialize");
    assert!(
        json.contains("\"work_type\":\"evolution\""),
        "JSON should contain work_type field: {}",
        json
    );

    let deserialized: CostRecord = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized.work_type, Some("evolution".to_string()));
}

/// 验证旧 JSON（无 work_type 字段）反序列化时 work_type = None（向后兼容）。
#[test]
fn cost_record_old_json_without_work_type_defaults_none() {
    // 构造一个不含 work_type 字段的旧格式 JSON。
    let old_json = r#"{
        "model": "qwen2.5:7b",
        "input_tokens": 100,
        "output_tokens": 50,
        "cost_usd": 0.001,
        "timestamp": "2026-07-05T12:00:00Z",
        "source": "chat"
    }"#;

    let record: CostRecord = serde_json::from_str(old_json).expect("deserialize old JSON");
    assert!(
        record.work_type.is_none(),
        "old JSON without work_type should default to None"
    );
}

// ---------------------------------------------------------------------------
// AiSelfModify 强制 High
// ---------------------------------------------------------------------------

/// 验证 assess(AiSelfModify) 总是返回 High（不可降级）。
#[test]
fn ai_self_modify_always_high_risk() {
    let gate = make_gate();
    let autonomy = AutonomyLevel::L4Swarm;

    // 即便在 L4（较高自主级），AiSelfModify 仍需确认。
    let verdict = gate.assess(ActionKind::AiSelfModify, "harmless update", autonomy, None);

    match verdict {
        ApprovalVerdict::ConfirmRequired { risk_tier, .. } => {
            assert!(
                matches!(risk_tier, RiskTier::High),
                "AiSelfModify must always be High"
            );
        }
        ApprovalVerdict::Allow { risk_tier, .. } => {
            panic!(
                "AiSelfModify at L4 should require confirm, got Allow with {:?}",
                risk_tier
            );
        }
    }
}

// ---------------------------------------------------------------------------
// GC 清理
// ---------------------------------------------------------------------------

/// 验证 gc() 清理已确认和已过期的 confirmation。
#[test]
fn gc_cleans_confirmed_and_expired() {
    let registry = Arc::new(ConfirmationRegistry::new());
    let now = chrono::Utc::now().timestamp_millis();

    // 注入 3 个 confirmation：1 个已确认、1 个已过期、1 个 pending。
    let confirmed_id = "c1".to_string();
    let expired_id = "e1".to_string();
    let pending_id = "p1".to_string();

    registry.register(PendingConfirmation {
        confirmation_id: confirmed_id.clone(),
        action_kind: ActionKind::AiSelfModify,
        risk_tier: RiskTier::High,
        prompt: "x".to_string(),
        diff: None,
        created_at: now,
        confirmed_at: Some(now), // 已确认
    });
    registry.register(PendingConfirmation {
        confirmation_id: expired_id.clone(),
        action_kind: ActionKind::AiSelfModify,
        risk_tier: RiskTier::High,
        prompt: "x".to_string(),
        diff: None,
        created_at: now - (10 * 60 * 1000), // 10 分钟前，已过期
        confirmed_at: None,
    });
    registry.register(PendingConfirmation {
        confirmation_id: pending_id.clone(),
        action_kind: ActionKind::AiSelfModify,
        risk_tier: RiskTier::High,
        prompt: "x".to_string(),
        diff: None,
        created_at: now,
        confirmed_at: None, // pending
    });

    // GC 前有 3 个。
    assert_eq!(registry.pending_count(), 3);

    // GC。
    let removed = registry.gc();
    assert_eq!(removed, 2, "gc should remove 2 (confirmed + expired)");

    // GC 后应只剩 pending_id（1 个）。
    assert_eq!(registry.pending_count(), 1, "gc should leave only pending");

    // 验证剩余的是 pending_id（仍未消费）。
    assert_eq!(
        registry.check(&pending_id),
        ConfirmationStatus::Confirmed,
        "pending should still be Confirmed (not yet consumed)"
    );
}

// ---------------------------------------------------------------------------
// 流式接口集成（#73-74）
// ---------------------------------------------------------------------------

/// 验证 OllamaClient::chat_stream() 在死端口上返回错误流。
/// 这是流式 MVP 的最小集成测试（不依赖真实 Ollama 服务）。
#[tokio::test]
async fn chat_stream_dead_port_yields_error() {
    use futures::StreamExt;
    use nebula_lib::llm::ChatMessage;

    // Bind a port, then immediately release it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let base_url = format!("http://127.0.0.1:{port}");

    let client = nebula_lib::llm::OllamaClient::new_with_timeout(
        base_url,
        std::time::Duration::from_millis(200),
    );
    let mut stream = client.chat_stream("x", vec![ChatMessage::user("hi")]);
    let result = stream.next().await;
    assert!(result.is_some(), "stream should yield at least one item");
    match result.unwrap() {
        Ok(_) => panic!("expected Err on dead port"),
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("streaming chat request failed"),
                "error should mention request failure, got: {msg}"
            );
        }
    }
}
