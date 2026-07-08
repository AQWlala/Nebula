//! M6 #82: Master orchestrator + L4 approval Tauri 命令。
//!
//! ## 命令清单
//! - `master_run(input, mode, on_master_event)` — 启动 MasterOrchestrator 编排,
//!   通过 Tauri 2.0 `ipc::Channel` 实时推送 11 个 MasterEvent 变体给前端。
//!   返回 MasterReport(最终综合输出 + 统计)。仅在 `master-orchestrator` feature
//!   启用时编译;feature 关闭时前端调用会得到 "command not found" 错误。
//! - `master_confirm(confirmation_id)` — 用户确认 L4 审批请求(防重放 + 5min 超时)。
//! - `master_confirmation_status(confirmation_id)` — 查询 confirmation 状态(供 UI 显示倒计时)。
//! - `master_pending_confirmations()` — 列出当前 pending 的审批请求(供 UI 渲染待确认列表)。
//! - `loop_run(loop_md, workspace_id)` — 启动 Loop 执行模式(T-E-L-01)。
//! - `loop_state(output_path)` — 生成 STATE.md 只读投影(T-E-L-01)。
//! - `loop_templates_list()` — 列出 7 种 Loop 模板摘要(T-E-L-05)。
//! - `loop_template_get(name)` — 按 name 获取完整 Loop 模板(T-E-L-05)。
//!
//! `master_confirm*` 命令始终可用(autonomy 模块无 feature gate)。
//! `loop_*` 命令由 `master-orchestrator` feature 门控。

use tauri::State;
use tracing::instrument;

#[cfg(feature = "master-orchestrator")]
use crate::autonomy; // for autonomy::get_level() / autonomy::CONFIRMATION_TIMEOUT_MS
use crate::autonomy::{ConfirmationStatus, PendingConfirmation};
use crate::commands::error::CommandError;
#[cfg(feature = "master-orchestrator")]
use crate::memory::values::risk_assessor::ActionKind;
use crate::AppState;

/// M6 #82: 启动 MasterOrchestrator 编排 + 实时推送 MasterEvent。
///
/// 设计要点:
/// - 使用 Tauri 2.0 `ipc::Channel` 双向通道(与 subscribe_events 同模式)
/// - `MasterOrchestrator::set_event_sink` 接收 `std::sync::mpsc::Sender<MasterEvent>`
///   (sync channel),用 `spawn_blocking` 桥接到 async context
/// - `orchestrate()` 在独立 tokio task 中运行,与事件转发并行
/// - 流结束(tx 被 drop)时,转发循环自动退出
/// - 前端 abort Channel(关闭窗口 / 取消订阅)时 `on_master_event.send()` 失败,
///   转发循环退出;orchestrate 仍继续到完成
///
/// `mode` 控制子任务执行模式:
/// - `standard`: 完整 RAG + Negotiator 协商
/// - `bypass`: 选最高置信度(零 LLM 仲裁)
/// - `plan`: L4 门禁预检
#[cfg(feature = "master-orchestrator")]
#[tauri::command]
#[instrument(skip(state, on_master_event), fields(otel.kind = "master_run"))]
pub async fn master_run(
    state: State<'_, AppState>,
    input: String,
    mode: crate::swarm::ExecuteMode,
    on_master_event: tauri::ipc::Channel<crate::swarm::MasterEvent>,
) -> Result<crate::swarm::MasterReport, CommandError> {
    // 注入扫描(与 swarm_execute 一致)
    let scan = crate::security::injection_guard::full_injection_scan(&input);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nebula.cmd",
                hits = scan.injection_hits.len(),
                leaks = scan.credential_leaks.len(),
                "blocked critical injection / credential leak in master_run"
            );
            return Err(CommandError::validation("master_run").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string(),
            ));
        }
        if !scan.safe {
            tracing::warn!(
                target: "nebula.cmd",
                severity = %severity,
                "non-critical injection warning in master_run"
            );
        }
    }

    // M5 #71 / P1-15: 远端 LLM 隐私提示门。
    // MasterDecompose(现 MasterTask)默认走远端 provider,用户输入的 task description
    // 会被发送到 DeepSeek 等远端 LLM。在 orchestrate 之前提示用户确认。
    // 复用 ApprovalGate + ConfirmationRegistry + master_confirm 命令。
    // RemoteLlmDispatch 在 WorkerRiskMap 中强制 High,不受 autonomy 影响(隐私硬约束)。
    let autonomy_level = autonomy::get_level();
    let verdict =
        state
            .approval_gate
            .assess(ActionKind::RemoteLlmDispatch, &input, autonomy_level, None);
    if let crate::autonomy::ApprovalVerdict::ConfirmRequired {
        confirmation_id,
        created_at,
        prompt,
        ..
    } = verdict
    {
        tracing::info!(
            target: "nebula.master.privacy",
            confirmation_id = %confirmation_id,
            autonomy = ?autonomy_level,
            "privacy consent required for remote LLM dispatch"
        );
        // 通过 Channel 推送 UserConfirmationRequired 事件给前端。
        // 复用现有 MasterEvent::UserConfirmationRequired 变体(语义匹配)。
        let privacy_event = crate::swarm::MasterEvent::UserConfirmationRequired {
            task_id: "privacy_gate".to_string(),
            sub_task_id: "remote_llm_dispatch".to_string(),
            prompt: format!(
                "⚠️ 隐私提示:你的任务描述将被发送到远端 LLM provider。\n\n{prompt}\n\n\
                 确认发送? (5 分钟内有效)"
            ),
            confirmation_id: confirmation_id.clone(),
            created_at,
            timestamp: crate::swarm::MasterEvent::now_ts(),
        };
        if on_master_event.send(privacy_event).is_err() {
            return Err(CommandError::validation("master_run")
                .with_details("前端通道已关闭,无法推送隐私确认请求".to_string()));
        }

        // 轮询等待 master_confirm(5min 超时)。
        // 用 tokio::time::interval 每 500ms 检查一次 confirmation 状态。
        let deadline =
            chrono::Utc::now().timestamp_millis() + crate::autonomy::CONFIRMATION_TIMEOUT_MS;
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));
        let mut confirmed = false;
        loop {
            interval.tick().await;
            match state.confirmation_registry.check(&confirmation_id) {
                ConfirmationStatus::Confirmed => {
                    confirmed = true;
                    break;
                }
                ConfirmationStatus::Expired | ConfirmationStatus::NotFound => {
                    return Err(CommandError::validation("master_run")
                        .with_details("隐私确认超时或失效,请重新发起任务".to_string()));
                }
                ConfirmationStatus::AlreadyUsed => {
                    // 不应发生(check 不消费),防御性处理
                    confirmed = true;
                    break;
                }
            }
            if chrono::Utc::now().timestamp_millis() > deadline {
                return Err(CommandError::validation("master_run")
                    .with_details("隐私确认超时(5 分钟未响应),请重新发起任务".to_string()));
            }
        }
        if !confirmed {
            return Err(CommandError::validation("master_run")
                .with_details("隐私确认未通过,任务已取消".to_string()));
        }
        tracing::info!(
            target: "nebula.master.privacy",
            confirmation_id = %confirmation_id,
            "privacy consent granted, proceeding with orchestrate"
        );
    }

    let master = state.master_orchestrator.clone();
    // 同步 mpsc channel:MasterOrchestrator::emit 是同步方法,
    // 通过 std::sync::mpsc::Sender 推送事件。
    let (tx, rx) = std::sync::mpsc::channel::<crate::swarm::MasterEvent>();
    master.set_event_sink(tx);

    // orchestrate 在独立 tokio task 中运行(不阻塞当前命令 future)
    let master_clone = master.clone();
    let input_clone = input.clone();
    let orch_handle =
        tokio::spawn(async move { master_clone.orchestrate(&input_clone, mode).await });

    // 事件转发:用 spawn_blocking 阻塞 recv(避免阻塞 tokio executor),
    // 收到事件即同步调用 `on_master_event.send()`(Tauri Channel.send 是同步方法)。
    // tx 被 drop(orchestrate 退出)时 rx.recv() 返回 Err,循环自动退出。
    let forward_handle = tokio::task::spawn_blocking(move || {
        while let Ok(event) = rx.recv() {
            if on_master_event.send(event).is_err() {
                // 前端关闭 Channel(组件卸载),停止转发。
                break;
            }
        }
    });

    let report = orch_handle
        .await
        .map_err(|e| {
            CommandError::swarm(
                "master_run",
                &anyhow::anyhow!("orchestrate task panicked: {}", e),
            )
        })?
        .map_err(|e| CommandError::swarm("master_run", &e))?;

    // 等待转发循环退出(tx drop 后 recv 返回 Err)。
    let _ = forward_handle.await;
    Ok(report)
}

/// T-E-L-01: 启动 Loop 执行模式。
///
/// 接收 LOOP.md 内容（YAML frontmatter + Markdown body），
/// 解析为 LoopDef 后调用 MasterOrchestrator::execute_loop()。
///
/// 流程：
/// 1. `LoopDef::from_markdown(loop_md)` 解析 + `validate()`
/// 2. ValuesLayer 门禁（Deny/Confirm/Plan/Allow）
/// 3. Allow → 创建 + 启动 LongTask，返回 LoopRunReport
///
/// 仅在 `master-orchestrator` feature 启用时编译。
#[cfg(feature = "master-orchestrator")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "loop_run"))]
pub async fn loop_run(
    state: State<'_, AppState>,
    loop_md: String,
    workspace_id: Option<String>,
) -> Result<crate::swarm::LoopRunReport, CommandError> {
    use crate::swarm::loop_def::LoopDef;

    let loop_def = LoopDef::from_markdown(&loop_md).map_err(|e| {
        CommandError::validation("loop_run").with_details(format!("LOOP.md 解析失败: {e}"))
    })?;
    loop_def.validate().map_err(|e| {
        CommandError::validation("loop_run").with_details(format!("LOOP.md 校验失败: {e}"))
    })?;

    let master = state.master_orchestrator.clone();
    let engine = state.long_task_engine.clone();
    let report = master
        .execute_loop(&loop_def, &engine, workspace_id)
        .await
        .map_err(|e| CommandError::swarm("loop_run", &e))?;
    Ok(report)
}

/// T-E-L-01: 生成 STATE.md 只读投影。
///
/// 调用 `LongTaskEngine::state_projection()`，将所有长任务状态
/// 投影为 Markdown 文件（STATE.md），供 Loop Engine 观察当前状态。
///
/// 仅在 `master-orchestrator` feature 启用时编译。
#[cfg(feature = "master-orchestrator")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "loop_state"))]
pub async fn loop_state(
    state: State<'_, AppState>,
    output_path: String,
) -> Result<String, CommandError> {
    let engine = state.long_task_engine.clone();
    let path = std::path::PathBuf::from(&output_path);
    // state_projection 是同步方法（文件 I/O + SQLite 查询，通常 <100ms）。
    // 用 spawn_blocking 避免阻塞 tokio executor。
    let result = tokio::task::spawn_blocking(move || engine.state_projection(&path))
        .await
        .map_err(|e| {
            CommandError::swarm(
                "loop_state",
                &anyhow::anyhow!("state_projection task panicked: {e}"),
            )
        })?
        .map_err(|e| CommandError::swarm("loop_state", &e))?;
    Ok(result.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// T-E-L-05: Loop 模板库命令
// ---------------------------------------------------------------------------

/// Loop 模板摘要(列表项),供前端渲染模板卡片网格。
///
/// 来自 LOOP.md frontmatter,由 [`LoopDef::from_markdown`] 解析后提取。
#[cfg(feature = "master-orchestrator")]
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoopTemplateSummary {
    /// Loop 名称(唯一标识,如 "ci-sweeper")。
    pub name: String,
    /// Loop 描述(人类可读)。
    pub description: String,
    /// 自主度等级 L0-L5。
    pub autonomy: crate::swarm::loop_def::AutonomyLevel,
    /// cron 表达式(如 "0 * * * *")或 "on-webhook"。
    pub cadence: String,
    /// 单次执行 Token 预算。
    pub budget_tokens: u64,
    /// 单次执行时间预算(分钟)。
    pub budget_minutes: u32,
}

/// 完整 Loop 模板(含 LOOP.md 原文),供前端预览 + 传给 `loop_run`。
#[cfg(feature = "master-orchestrator")]
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoopTemplate {
    /// 摘要部分(frontmatter 字段)。
    #[serde(flatten)]
    pub summary: LoopTemplateSummary,
    /// LOOP.md 原文(YAML frontmatter + Markdown body),
    /// 可直接传给 `loop_run` 命令执行。
    pub content: String,
}

/// 内嵌的 7 种 Loop 模板(name, content)静态表。
///
/// 模板文件位于 `docs/skills/loop-engineering/templates/`,
/// 通过 `include_str!` 在编译时内嵌到二进制中,
/// 桌面应用无需携带 docs/ 目录。
///
/// 顺序固定,供 [`loop_templates_list`] 按稳定顺序返回。
#[cfg(feature = "master-orchestrator")]
static LOOP_TEMPLATES: &[(&str, &str)] = &[
    (
        "ci-sweeper",
        include_str!("../../../docs/skills/loop-engineering/templates/ci-sweeper.md"),
    ),
    (
        "pr-babysitter",
        include_str!("../../../docs/skills/loop-engineering/templates/pr-babysitter.md"),
    ),
    (
        "daily-triage",
        include_str!("../../../docs/skills/loop-engineering/templates/daily-triage.md"),
    ),
    (
        "code-review-loop",
        include_str!("../../../docs/skills/loop-engineering/templates/code-review-loop.md"),
    ),
    (
        "memory-consolidation",
        include_str!("../../../docs/skills/loop-engineering/templates/memory-consolidation.md"),
    ),
    (
        "skill-evolution",
        include_str!("../../../docs/skills/loop-engineering/templates/skill-evolution.md"),
    ),
    (
        "budget-guardian",
        include_str!("../../../docs/skills/loop-engineering/templates/budget-guardian.md"),
    ),
];

/// T-E-L-05: 列出所有 Loop 模板摘要。
///
/// 返回 7 种 Loop 模式的摘要列表(name / description / autonomy /
/// cadence / budget),供前端 TemplatesDialog 的 automation 类别渲染卡片网格。
///
/// 模板在编译时内嵌(`include_str!`),无运行时文件 I/O。
/// 仅在 `master-orchestrator` feature 启用时编译。
#[cfg(feature = "master-orchestrator")]
#[tauri::command]
#[instrument(fields(otel.kind = "loop_templates_list"))]
pub async fn loop_templates_list() -> Result<Vec<LoopTemplateSummary>, CommandError> {
    use crate::swarm::loop_def::LoopDef;

    let mut summaries = Vec::with_capacity(LOOP_TEMPLATES.len());
    for (name, content) in LOOP_TEMPLATES {
        let def = LoopDef::from_markdown(content).map_err(|e| {
            CommandError::internal(
                "loop_templates_list",
                &anyhow::anyhow!("内置模板 {name} 解析失败(编译时回归): {e}"),
            )
        })?;
        summaries.push(LoopTemplateSummary {
            name: def.name,
            description: def.description,
            autonomy: def.autonomy,
            cadence: def.cadence,
            budget_tokens: def.budget_tokens,
            budget_minutes: def.budget_minutes,
        });
    }
    Ok(summaries)
}

/// T-E-L-05: 按 name 获取完整 Loop 模板。
///
/// 返回 [`LoopTemplate`](含摘要 + LOOP.md 原文),前端可直接将
/// `content` 传给 `loop_run` 命令启动 Loop。
///
/// `name` 不存在时返回 `None`(前端展示"模板不存在"提示)。
/// 仅在 `master-orchestrator` feature 启用时编译。
#[cfg(feature = "master-orchestrator")]
#[tauri::command]
#[instrument(fields(otel.kind = "loop_template_get"))]
pub async fn loop_template_get(name: String) -> Result<Option<LoopTemplate>, CommandError> {
    use crate::swarm::loop_def::LoopDef;

    let content = LOOP_TEMPLATES
        .iter()
        .find(|(n, _)| *n == name.as_str())
        .map(|(_, c)| *c);
    match content {
        Some(md) => {
            let def = LoopDef::from_markdown(md).map_err(|e| {
                CommandError::internal(
                    "loop_template_get",
                    &anyhow::anyhow!("内置模板 {name} 解析失败(编译时回归): {e}"),
                )
            })?;
            Ok(Some(LoopTemplate {
                summary: LoopTemplateSummary {
                    name: def.name,
                    description: def.description,
                    autonomy: def.autonomy,
                    cadence: def.cadence,
                    budget_tokens: def.budget_tokens,
                    budget_minutes: def.budget_minutes,
                },
                content: md.to_string(),
            }))
        }
        None => Ok(None),
    }
}

/// M6 #82: 用户确认 L4 审批请求。
///
/// 调用 `ConfirmationRegistry::mark_confirmed`:
/// - 首次提交返回 `Confirmed`
/// - 已被消费返回 `AlreadyUsed`(防重放)
/// - 已过期(>5min)返回 `Expired`
/// - 不存在返回 `NotFound`
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "master_confirm"))]
pub async fn master_confirm(
    state: State<'_, AppState>,
    confirmation_id: String,
) -> Result<ConfirmationStatus, CommandError> {
    Ok(state.confirmation_registry.mark_confirmed(&confirmation_id))
}

/// M6 #82: 查询 confirmation 状态(供前端显示倒计时 / 防重放提示)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "master_confirmation_status"))]
pub async fn master_confirmation_status(
    state: State<'_, AppState>,
    confirmation_id: String,
) -> Result<ConfirmationStatus, CommandError> {
    Ok(state.confirmation_registry.check(&confirmation_id))
}

/// M6 #82: 列出当前 pending 的审批请求(供 UI 渲染待确认列表)。
///
/// 返回所有 pending(包含已确认 / 已过期),前端按 `created_at` + 5min 自行过滤。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "master_pending_confirmations"))]
pub async fn master_pending_confirmations(
    state: State<'_, AppState>,
) -> Result<Vec<PendingConfirmation>, CommandError> {
    Ok(state.confirmation_registry.all_pending())
}

// ---------------------------------------------------------------------------
// T-E-L-05 Commit 2: Loop 模板库回归测试
// ---------------------------------------------------------------------------

/// `loop_templates_list` / `loop_template_get` 命令的核心不变式:
/// 所有内嵌模板在编译时已固定(`include_str!`),运行时若任一模板
/// 解析失败会返回 `CommandError::internal`。此测试模块在 CI 阶段
/// 提前拦截模板格式回归(如 YAML 缩进 / 章节标题拼写错误)。
#[cfg(all(test, feature = "master-orchestrator"))]
mod loop_template_tests {
    use super::*;
    use crate::swarm::loop_def::{AutonomyLevel, LoopDef};

    /// 内嵌模板数量必须正好是 7 种(对应 7 种 Loop 模式)。
    /// 防止误删或误增模板。
    #[test]
    fn loop_templates_count_is_seven() {
        assert_eq!(
            LOOP_TEMPLATES.len(),
            7,
            "LOOP_TEMPLATES must contain exactly 7 templates (got {})",
            LOOP_TEMPLATES.len()
        );
    }

    /// 所有内嵌模板必须能被 `LoopDef::from_markdown` 成功解析。
    /// 这是 `loop_templates_list` 命令的核心不变式 —— 任一解析失败
    /// 会导致命令在运行时返回 InternalError。
    #[test]
    fn loop_templates_all_parse_successfully() {
        for (name, content) in LOOP_TEMPLATES {
            let def = LoopDef::from_markdown(content)
                .unwrap_or_else(|e| panic!("内置模板 {name} 解析失败(模板格式回归): {e}"));
            // 解析出的 name 应与静态表的 key 一致
            assert_eq!(
                def.name, *name,
                "模板 {name}: frontmatter.name 与 LOOP_TEMPLATES key 不一致"
            );
        }
    }

    /// 所有模板的 name 字段必须唯一(前端按 name 索引)。
    #[test]
    fn loop_templates_have_unique_names() {
        let mut names: Vec<&str> = LOOP_TEMPLATES.iter().map(|(n, _)| *n).collect();
        names.sort();
        let duplicates: Vec<&str> = names
            .windows(2)
            .filter(|w| w[0] == w[1])
            .map(|w| w[0])
            .collect();
        assert!(duplicates.is_empty(), "发现重复的模板 name: {duplicates:?}");
    }

    /// 所有模板必须包含非空的 description / cadence / intent,
    /// 以及至少一条 Action(否则 Loop 无法执行)。
    #[test]
    fn loop_templates_have_non_empty_required_fields() {
        for (name, content) in LOOP_TEMPLATES {
            let def = LoopDef::from_markdown(content)
                .unwrap_or_else(|e| panic!("模板 {name} 解析失败: {e}"));
            assert!(!def.description.is_empty(), "模板 {name}: description 为空");
            assert!(!def.cadence.is_empty(), "模板 {name}: cadence 为空");
            assert!(!def.intent.is_empty(), "模板 {name}: intent 为空");
            assert!(
                !def.action.is_empty(),
                "模板 {name}: action 为空(Loop 无法执行)"
            );
        }
    }

    /// 所有模板的 autonomy 必须在 L1-L5 范围内(L0 内联补全不适用于 Loop)。
    #[test]
    fn loop_templates_autonomy_in_valid_loop_range() {
        for (name, content) in LOOP_TEMPLATES {
            let def = LoopDef::from_markdown(content)
                .unwrap_or_else(|e| panic!("模板 {name} 解析失败: {e}"));
            assert!(
                !matches!(def.autonomy, AutonomyLevel::L0),
                "模板 {name}: autonomy=L0 不适用于 Loop(内联补全无需 Loop)"
            );
        }
    }

    /// `loop_template_get` 命令的查找逻辑 —— 存在的 name 应返回 Some,
    /// 不存在的应返回 None(命令层映射为 Ok(None) 给前端)。
    #[test]
    fn loop_templates_lookup_by_name() {
        let names: Vec<&str> = LOOP_TEMPLATES.iter().map(|(n, _)| *n).collect();
        for name in &names {
            let found = LOOP_TEMPLATES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, c)| *c);
            assert!(found.is_some(), "模板 {name} 应能被找到");
        }
        // 不存在的 name
        let missing = LOOP_TEMPLATES
            .iter()
            .find(|(n, _)| *n == "non-existent-template")
            .map(|(_, c)| *c);
        assert!(missing.is_none(), "不存在的模板应返回 None");
        // 确认所有期望的 7 个都在
        for expected in [
            "ci-sweeper",
            "pr-babysitter",
            "daily-triage",
            "code-review-loop",
            "memory-consolidation",
            "skill-evolution",
            "budget-guardian",
        ] {
            assert!(
                names.contains(&expected),
                "期望的模板 {expected} 不在 LOOP_TEMPLATES 中"
            );
        }
    }
}
