//! Swarm commands — execute, list agents, get agent.

use serde::{Deserialize, Serialize};
use serde_json;
use tauri::State;
use tracing::instrument;

use crate::api::server::NebulaService;
use crate::commands::error::CommandError;
use crate::swarm::{DeadlockStatus, MoAConfig, OrchestrationReport, SwarmTask};
use crate::AppState;

/// Tauri command: dispatch a swarm task.
#[tauri::command]
#[instrument(skip(state, task), fields(otel.kind = "swarm_execute"))]
pub async fn swarm_execute(
    state: State<'_, AppState>,
    task: SwarmTask,
) -> Result<OrchestrationReport, CommandError> {
    // v1.1: Prompt injection scan before processing.
    let scan = crate::security::injection_guard::full_injection_scan(&task.description);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nebula.cmd",
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in swarm_execute"
            );
            return Err(CommandError::validation("swarm_execute").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string(),
            ));
        }
        if !scan.safe {
            tracing::warn!(
                target: "nebula.cmd",
                severity = %severity,
                "non-critical injection warning in swarm_execute"
            );
        }
    }

    let report = state
        .swarm_execute(task)
        .await
        .map_err(|e| CommandError::swarm("swarm_execute", &e))?;
    crate::metrics::global().record_swarm();
    Ok(report)
}

/// v0.3: list the available swarm agents as `(kind, name, system, description)`.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "swarm_list_agents"))]
pub async fn swarm_list_agents(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String, String, String)>, CommandError> {
    Ok(state.swarm.list_agents())
}

/// v0.3: fetch a single swarm agent by kind.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "swarm_get_agent"))]
pub async fn swarm_get_agent(
    state: State<'_, AppState>,
    kind: String,
) -> Result<Option<SwarmAgentInfo>, CommandError> {
    Ok(state.swarm.get_agent(&kind).map(|a| SwarmAgentInfo {
        name: a.name,
        system_prompt: a.system_prompt,
        description: a.description,
    }))
}

/// v0.3: agent descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmAgentInfo {
    pub name: String,
    pub system_prompt: String,
    pub description: String,
}

/// T-S1-B-02: 订阅 Swarm 执行事件流,供前端实时可视化。
/// T-E-S-26: 返回协议化 EventEnvelope<serde_json::Value>。
///
/// 使用 Tauri 2.0 `ipc::Channel` 双向通道:前端调用后立即开始监听,
/// 后端在 swarm 执行的 5 个关键节点(AgentStarted/AgentCompleted/
/// NegotiationStarted/ArbitrationResolved/SwarmCompleted)推送
/// `EventEnvelope<serde_json::Value>`。
///
/// 前端关闭通道(返回页面或取消订阅)时 `on_event.send()` 失败,
/// 后端循环自动退出,不会泄漏任务。
#[tauri::command]
#[instrument(skip(state, on_event), fields(otel.kind = "subscribe_events"))]
pub async fn subscribe_events(
    state: State<'_, AppState>,
    on_event: tauri::ipc::Channel<crate::swarm::EventEnvelope<serde_json::Value>>,
) -> Result<(), CommandError> {
    let mut rx = state.event_bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(envelope) => {
                // 将 EventEnvelope<SwarmEvent> 转为 EventEnvelope<serde_json::Value>
                let value = serde_json::to_value(&envelope)
                    .ok()
                    .and_then(|v| serde_json::from_value::<crate::swarm::EventEnvelope<serde_json::Value>>(v).ok());
                if let Some(v) = value {
                    if on_event.send(v).is_err() {
                        break;
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    target: "nebula.cmd",
                    lagged = n,
                    "subscribe_events lagged behind, skipping stale events"
                );
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}

/// T-E-D-07: 取消正在执行的 swarm 任务。
///
/// 通过 task_id 查找 `SwarmOrchestrator` 内部的 `CancellationToken` 并
/// 触发取消。各 agent 的 spawn 任务通过 `select!` 监听该 token,取消后
/// 立即中断返回。
///
/// 返回 `true` 表示该 task_id 存在并已取消;`false` 表示该 task_id 不存在
/// (可能已完成或从未创建)。无论返回值如何,前端浮动进度窗都应关闭。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "swarm_cancel"))]
pub async fn swarm_cancel(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, CommandError> {
    let cancelled = state.swarm.cancel(&task_id);
    if cancelled {
        tracing::info!(
            target: "nebula.cmd",
            task_id = %task_id,
            "swarm_cancel: task cancellation triggered"
        );
    } else {
        tracing::warn!(
            target: "nebula.cmd",
            task_id = %task_id,
            "swarm_cancel: task_id not found (already completed or unknown)"
        );
    }
    Ok(cancelled)
}

/// T-E-S-05: 查询死锁检测状态。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "deadlock_status"))]
pub async fn deadlock_status(
    state: State<'_, AppState>,
) -> Result<DeadlockStatus, CommandError> {
    Ok(state.deadlock_detector.status())
}

/// T-E-S-04: MoA(Mixture of Agents)执行命令。
///
/// 供前端直接调用 MoA 模式:多个 LLM provider 合议(Voting/Cascading/Arbitration)
/// 产出最优答案。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "moa_execute"))]
pub async fn moa_execute(
    prompt: String,
    config: MoAConfig,
    state: State<'_, AppState>,
) -> Result<crate::swarm::AgentOutput, CommandError> {
    // M7b #94: moa_execute 输入 injection_scan(与 swarm_execute 一致)。
    let scan = crate::security::injection_guard::full_injection_scan(&prompt);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nebula.cmd",
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in moa_execute"
            );
            return Err(CommandError::validation("moa_execute").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string(),
            ));
        }
        if !scan.safe {
            tracing::warn!(
                target: "nebula.cmd",
                severity = %severity,
                "non-critical injection warning in moa_execute"
            );
        }
    }
    let negotiator = crate::swarm::Negotiator::new();
    negotiator
        .negotiate_moa(&prompt, &config, &state.llm)
        .await
        .map_err(|e| CommandError::swarm("moa_execute", &e))
}
