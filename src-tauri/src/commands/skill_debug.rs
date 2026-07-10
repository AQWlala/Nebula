//! P1-7: 技能调试工具 — Inspector / TestRunner / Debugger / Profiler。
//!
//! 在 SkillEngine(create/use/rate/list/search)与 SkillSpecValidator(三层校验)
//! 之上,为技能开发者提供四个调试命令:
//!
//! * [`skill_inspect`] — 检查技能详情:manifest + body + 校验结果 + 依赖检查 + 使用统计。
//! * [`skill_test_run`] — 单技能测试运行(沙箱执行,30s 超时)。
//! * [`skill_debug_start`] / [`skill_debug_step`] / [`skill_debug_stop`] —
//!   逐步调试会话(变量状态 + 调用栈)。
//! * [`skill_profile`] — 性能分析(CPU/内存/IO/子调用 + 时间线)。
//!
//! ## 设计
//!
//! * 调试会话存在进程内全局 `Lazy<Mutex<HashMap>>`(once_cell),
//!   不污染 AppState 字段;会话 id 用 UUID v4。
//! * 所有命令返回 `Result<T, CommandError>`,不 panic。
//! * 测试运行复用 `SkillEngine::use_skill`(已含 5s 沙箱超时 + 网络阻断 +
//!   RLIMIT_AS),外层再加 30s tokio 超时作为安全网。
//! * 使用统计从 `SkillAuditLogger::list_for_skill` 聚合(call_count 仍以
//!   `skill.usage_count` 为准,审计日志用于成功率 / 最近使用 / 平均延迟)。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::{instrument, warn};

use crate::commands::error::CommandError;
use crate::skills::protocol::{SkillEligibility, SkillManifest, SkillSpecValidator, SkillTransport};
use crate::skills::types::{Skill, UseSkillRequest};
use crate::AppState;

// ---------------------------------------------------------------------------
// P1-7: 调试工具 DTO
// ---------------------------------------------------------------------------

/// 技能检查结果 — [`skill_inspect`] 命令的返回值。
///
/// 聚合 manifest(frontmatter)、body(SKILL.md 正文)、校验结果、依赖检查、使用统计。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInspection {
    /// 解析出的 SkillManifest(从 in-DB Skill 字段合成)。
    pub manifest: SkillManifest,
    /// SKILL.md 正文(从 skill.code + description 合成的 Markdown)。
    pub body: String,
    /// 三层校验结果(结构 / 规范 / 资格)。
    pub validation: ValidationResult,
    /// 依赖检查结果(bins / env / os)。
    pub dependency_check: DependencyCheckResult,
    /// 使用统计(调用次数 / 成功率 / 最近使用 / 平均延迟)。
    pub usage_stats: UsageStats,
}

/// 三层校验结果 — 结构层 / 规范层 / 资格层。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// 结构层:YAML frontmatter 可解析 + name/version 必填 + version 是 semver。
    pub structure_ok: bool,
    /// 规范层:transport 合法 + status 枚举值合法 + min_nebula_version 是 semver。
    pub spec_ok: bool,
    /// 资格层:bins/env/os 4 维资格全部满足。
    pub eligibility_ok: bool,
    /// 硬错误列表(规范违反,必须修复)。
    pub errors: Vec<String>,
    /// 软警告列表(不阻塞加载,但建议修复)。
    pub warnings: Vec<String>,
}

/// 依赖检查结果 — bins / env / os 三维。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCheckResult {
    /// 二进制可用性映射(bin 名 -> 是否在 PATH 中可找到)。
    pub bins_available: HashMap<String, bool>,
    /// 环境变量设置状态(变量名 -> 是否已设置且非空)。
    pub env_set: HashMap<String, bool>,
    /// 当前 OS 是否被允许(true = 无限制或当前 OS 在白名单内)。
    pub os_supported: bool,
}

/// 使用统计 — 从 audit log 聚合。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    /// 累计调用次数(来自 skill.usage_count)。
    pub call_count: u64,
    /// 成功率(0.0-1.0,来自 audit log;无记录时为 1.0)。
    pub success_rate: f64,
    /// 最近使用时间(Unix 毫秒;无记录时为 None)。
    pub last_used: Option<u64>,
    /// 平均延迟(毫秒;无记录时为 0)。
    pub avg_latency_ms: u64,
}

/// 单次测试运行结果 — [`skill_test_run`] 命令的返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTestResult {
    /// 是否成功。
    pub success: bool,
    /// 标准输出(成功时)或错误上下文(失败时)。
    pub output: String,
    /// 错误信息(失败时)。
    pub error: Option<String>,
    /// 端到端延迟(毫秒)。
    pub latency_ms: u64,
    /// 执行日志(逐步记录)。
    pub logs: Vec<String>,
}

/// 调试会话 — [`skill_debug_start`] 创建,逐步执行直到 [`skill_debug_stop`]。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSession {
    /// 会话 ID(UUID v4)。
    pub session_id: String,
    /// 目标技能 ID。
    pub skill_id: String,
    /// 测试输入(原始字符串)。
    pub test_input: String,
    /// 已执行的步骤列表(按顺序)。
    pub steps: Vec<String>,
    /// 当前变量状态(步骤可读写)。
    pub variables: HashMap<String, String>,
    /// 调用栈(步骤名栈)。
    pub call_stack: Vec<String>,
    /// 创建时间(Unix 毫秒)。
    pub created_at: u64,
}

/// 单步调试结果 — [`skill_debug_step`] 命令的返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugStepResult {
    /// 本次步骤名。
    pub step: String,
    /// 是否成功。
    pub success: bool,
    /// 步骤输出。
    pub output: String,
    /// 错误信息(失败时)。
    pub error: Option<String>,
    /// 步骤执行后的变量状态快照。
    pub variables: HashMap<String, String>,
    /// 步骤执行后的调用栈快照。
    pub call_stack: Vec<String>,
}

/// 性能分析结果 — [`skill_profile`] 命令的返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillProfile {
    /// CPU 时间(毫秒,近似为 wall-clock)。
    pub cpu_time_ms: u64,
    /// 内存占用(字节,从输出大小估算)。
    pub memory_bytes: u64,
    /// IO 操作次数。
    pub io_operations: u64,
    /// 子调用次数。
    pub sub_calls: u64,
    /// 时间线事件列表。
    pub timeline: Vec<ProfileEvent>,
}

/// 性能时间线事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEvent {
    /// 事件名(如 "start" / "execute" / "end")。
    pub name: String,
    /// 距开始的毫秒数。
    pub timestamp_ms: u64,
    /// 事件持续毫秒数。
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// P1-7: 调试会话全局注册表
// ---------------------------------------------------------------------------

/// 全局调试会话注册表 — 进程内单例,key 是 session_id。
///
/// 用 `Lazy<Mutex<HashMap>>` 而非 AppState 字段,避免污染 AppState 结构。
/// 会话在 [`skill_debug_stop`] 显式删除,或进程退出时自动回收。
static DEBUG_SESSIONS: Lazy<Mutex<HashMap<String, DebugSession>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

// ---------------------------------------------------------------------------
// P1-7: 命令实现
// ---------------------------------------------------------------------------

/// P1-7: 检查技能详情 — manifest + body + 校验 + 依赖 + 使用统计。
///
/// 从 in-DB Skill 合成 SkillManifest(填充 name/version/description/transport/
/// capabilities/status),合成 SKILL.md 后跑 [`SkillSpecValidator::validate_skill_md`]
/// 三层校验,再从 audit log 聚合使用统计。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_inspect"))]
pub async fn skill_inspect(
    state: State<'_, AppState>,
    skill_id: String,
) -> Result<SkillInspection, CommandError> {
    let skill = load_skill(&state, &skill_id)?;

    // 1. 合成 SkillManifest。
    let manifest = synthesize_manifest(&skill);

    // 2. 合成 SKILL.md body。
    let body = synthesize_body(&skill);

    // 3. 合成完整 SKILL.md 并跑三层校验。
    let skill_md = synthesize_skill_md(&skill);
    let report = SkillSpecValidator::validate_skill_md(&skill_md);

    // 拆分三层校验结果:结构层 / 规范层 / 资格层。
    let structure_ok = report.manifest.is_some()
        && !report
            .errors
            .iter()
            .any(|e| e.contains("name is required") || e.contains("version is required")
                || e.contains("frontmatter") || e.contains("semver"));
    let spec_ok = !report
        .errors
        .iter()
        .any(|e| e.contains("transport") || e.contains("status") || e.contains("min_nebula_version"));
    let eligibility_ok = report.eligible;

    let validation = ValidationResult {
        structure_ok,
        spec_ok,
        eligibility_ok,
        errors: report.errors,
        warnings: report.warnings,
    };

    // 4. 依赖检查:从 skill.language 派生 bins(env/os 无 in-DB 跟踪,默认通过)。
    let dependency_check = check_dependencies(&skill);

    // 5. 使用统计:从 audit log 聚合。
    let usage_stats = compute_usage_stats(&state, &skill);

    Ok(SkillInspection {
        manifest,
        body,
        validation,
        dependency_check,
        usage_stats,
    })
}

/// P1-7: 单技能测试运行 — 在沙箱中执行,30s 超时。
///
/// 复用 `SkillEngine::use_skill`(已含 5s 沙箱超时 + 网络阻断),外层加 30s
/// tokio 超时作为安全网。返回执行结果 / 错误 / 耗时 / 日志。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_test_run"))]
pub async fn skill_test_run(
    state: State<'_, AppState>,
    skill_id: String,
    test_input: String,
) -> Result<SkillTestResult, CommandError> {
    let skill = load_skill(&state, &skill_id)?;
    let params = parse_test_input(&test_input);
    let mut logs = Vec::new();
    logs.push(format!("[test_run] skill={} language={}", skill.name, skill.language));
    logs.push(format!("[test_run] params={} keys", params.len()));

    let start = Instant::now();
    let use_req = UseSkillRequest {
        id: skill_id.clone(),
        params,
    };
    // 外层 30s 超时(沙箱内部已有 5s 超时,这里只是安全网)。
    let result = match tokio::time::timeout(Duration::from_secs(30), state.swarm.skills.use_skill(use_req)).await {
        Ok(r) => r,
        Err(_) => {
            return Ok(SkillTestResult {
                success: false,
                output: String::new(),
                error: Some("test run exceeded 30s outer timeout".to_string()),
                latency_ms: start.elapsed().as_millis() as u64,
                logs,
            });
        }
    };

    let latency_ms = start.elapsed().as_millis() as u64;
    match result {
        Ok(skill_result) => {
            logs.push(format!("[test_run] success latency={}ms tokens={}", latency_ms, skill_result.tokens_used));
            Ok(SkillTestResult {
                success: true,
                output: skill_result.output,
                error: None,
                latency_ms,
                logs,
            })
        }
        Err(e) => {
            let msg = format!("{e:#}");
            logs.push(format!("[test_run] failed: {msg}"));
            Ok(SkillTestResult {
                success: false,
                output: String::new(),
                error: Some(msg),
                latency_ms,
                logs,
            })
        }
    }
}

/// P1-7: 启动调试会话 — 返回 session_id。
///
/// 创建一个调试会话,记录 skill_id + test_input,初始化空步骤列表 / 变量 / 调用栈。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_debug_start"))]
pub async fn skill_debug_start(
    state: State<'_, AppState>,
    skill_id: String,
    test_input: String,
) -> Result<String, CommandError> {
    // 校验技能存在(避免创建无效会话)。
    let _skill = load_skill(&state, &skill_id)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let session = DebugSession {
        session_id: session_id.clone(),
        skill_id,
        test_input,
        steps: Vec::new(),
        variables: HashMap::new(),
        call_stack: Vec::new(),
        created_at: chrono::Utc::now().timestamp_millis() as u64,
    };

    match DEBUG_SESSIONS.lock() {
        Ok(mut g) => {
            g.insert(session_id.clone(), session);
            Ok(session_id)
        }
        Err(e) => Err(CommandError::internal(
            "skill_debug_start",
            &anyhow::anyhow!("debug session lock poisoned: {e}"),
        )),
    }
}

/// P1-7: 逐步执行调试 — 执行单个步骤,返回步骤结果 + 变量状态 + 调用栈。
///
/// step 取值:
/// * `"load"` — 加载技能元数据(不执行),变量填充 skill 元信息。
/// * `"validate"` — 跑三层校验,变量填充校验结果。
/// * `"execute"` — 执行技能,变量填充执行结果。
/// * 其他 — 视为自定义步骤名,仅记录到 steps 列表。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_debug_step"))]
pub async fn skill_debug_step(
    state: State<'_, AppState>,
    session_id: String,
    step: String,
) -> Result<DebugStepResult, CommandError> {
    // 取出会话(克隆,避免长时间持锁)。
    let mut session = {
        let g = DEBUG_SESSIONS.lock().map_err(|e| {
            CommandError::internal(
                "skill_debug_step",
                &anyhow::anyhow!("debug session lock poisoned: {e}"),
            )
        })?;
        g.get(&session_id)
            .cloned()
            .ok_or_else(|| CommandError::not_found(format!("debug session: {session_id}")))?
    };

    session.steps.push(step.clone());
    session.call_stack.push(step.clone());

    let skill = load_skill(&state, &session.skill_id)?;
    let output: String;
    let mut success = true;
    let mut error: Option<String> = None;

    match step.as_str() {
        "load" => {
            // 加载元数据:填充 skill 元信息到变量。
            session.variables.insert("skill.name".into(), skill.name.clone());
            session.variables.insert("skill.language".into(), skill.language.clone());
            session.variables.insert("skill.usage_count".into(), skill.usage_count.to_string());
            session
                .variables
                .insert("skill.trust_level".into(), skill.trust_level.to_string());
            output = format!("loaded skill '{}' (language={}, usage={})",
                skill.name, skill.language, skill.usage_count);
        }
        "validate" => {
            // 跑三层校验。
            let skill_md = synthesize_skill_md(&skill);
            let report = SkillSpecValidator::validate_skill_md(&skill_md);
            session.variables.insert("validation.valid".into(), report.valid.to_string());
            session.variables.insert("validation.eligible".into(), report.eligible.to_string());
            session.variables.insert("validation.errors".into(), report.errors.join("; "));
            session.variables.insert("validation.warnings".into(), report.warnings.join("; "));
            output = format!(
                "validation: valid={} eligible={} errors={} warnings={}",
                report.valid, report.eligible, report.errors.len(), report.warnings.len()
            );
            if !report.valid {
                success = false;
                error = Some(report.errors.join("; "));
            }
        }
        "execute" => {
            // 执行技能。
            let params = parse_test_input(&session.test_input);
            let use_req = UseSkillRequest {
                id: session.skill_id.clone(),
                params,
            };
            match tokio::time::timeout(Duration::from_secs(30), state.swarm.skills.use_skill(use_req)).await {
                Ok(Ok(r)) => {
                    session.variables.insert("execute.output".into(), r.output.clone());
                    session
                        .variables
                        .insert("execute.latency_ms".into(), r.execution_time_ms.to_string());
                    session
                        .variables
                        .insert("execute.tokens".into(), r.tokens_used.to_string());
                    output = r.output;
                }
                Ok(Err(e)) => {
                    let msg = format!("{e:#}");
                    session.variables.insert("execute.error".into(), msg.clone());
                    success = false;
                    error = Some(msg.clone());
                    output = msg;
                }
                Err(_) => {
                    let msg = "execute exceeded 30s timeout".to_string();
                    session.variables.insert("execute.error".into(), msg.clone());
                    success = false;
                    error = Some(msg.clone());
                    output = msg;
                }
            }
        }
        _ => {
            // 自定义步骤:仅记录。
            output = format!("custom step '{}' recorded", step);
        }
    }

    // 步骤完成后弹出调用栈顶(保持栈深度 = 当前进行中的步骤)。
    session.call_stack.pop();

    // 写回会话。
    let variables = session.variables.clone();
    let call_stack = session.call_stack.clone();
    {
        let mut g = DEBUG_SESSIONS.lock().map_err(|e| {
            CommandError::internal(
                "skill_debug_step",
                &anyhow::anyhow!("debug session lock poisoned: {e}"),
            )
        })?;
        if let Some(s) = g.get_mut(&session_id) {
            *s = session;
        }
    }

    Ok(DebugStepResult {
        step,
        success,
        output,
        error,
        variables,
        call_stack,
    })
}

/// P1-7: 停止调试会话 — 删除会话。
#[tauri::command]
#[instrument(fields(otel.kind = "skill_debug_stop"))]
pub async fn skill_debug_stop(session_id: String) -> Result<(), CommandError> {
    match DEBUG_SESSIONS.lock() {
        Ok(mut g) => {
            g.remove(&session_id);
            Ok(())
        }
        Err(e) => Err(CommandError::internal(
            "skill_debug_stop",
            &anyhow::anyhow!("debug session lock poisoned: {e}"),
        )),
    }
}

/// P1-7: 性能分析 — 执行技能并收集性能数据。
///
/// 返回 CPU 时间(近似 wall-clock)、内存占用(从输出大小估算)、IO 次数、
/// 子调用次数、时间线事件列表。由于 Rust 后端无法直接 hook Python 子进程的
/// CPU/内存,这些指标是估算值;时间线基于实际执行阶段记录。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_profile"))]
pub async fn skill_profile(
    state: State<'_, AppState>,
    skill_id: String,
    test_input: String,
) -> Result<SkillProfile, CommandError> {
    let _skill = load_skill(&state, &skill_id)?;
    let params = parse_test_input(&test_input);

    let start = Instant::now();
    let mut timeline = Vec::new();
    let t0 = start.elapsed().as_millis() as u64;
    timeline.push(ProfileEvent {
        name: "start".into(),
        timestamp_ms: t0,
        duration_ms: 0,
    });

    let use_req = UseSkillRequest {
        id: skill_id.clone(),
        params,
    };
    let exec_start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(30), state.swarm.skills.use_skill(use_req)).await;
    let exec_ms = exec_start.elapsed().as_millis() as u64;

    let (output_len, error_msg) = match result {
        Ok(Ok(r)) => (r.output.len(), None),
        Ok(Err(e)) => (0, Some(format!("{e:#}"))),
        Err(_) => (0, Some("exceeded 30s timeout".into())),
    };

    let t1 = start.elapsed().as_millis() as u64;
    timeline.push(ProfileEvent {
        name: "execute".into(),
        timestamp_ms: t1 - exec_ms,
        duration_ms: exec_ms,
    });
    timeline.push(ProfileEvent {
        name: "end".into(),
        timestamp_ms: t1,
        duration_ms: 0,
    });

    // 估算内存:输出大小 + 1MB 基线(沙箱 Python 解释器固定开销)。
    let memory_bytes = (output_len as u64) * 2 + 1024 * 1024;
    // IO/子调用:无法精确测量,记为 0(占位;未来可通过 eBPF/strace 补齐)。
    let io_operations: u64 = 0;
    let sub_calls: u64 = 0;

    if let Some(ref msg) = error_msg {
        warn!(
            target: "nebula.skill_debug",
            skill_id = %skill_id,
            error = %msg,
            "profile run failed"
        );
    }

    Ok(SkillProfile {
        cpu_time_ms: exec_ms,
        memory_bytes,
        io_operations,
        sub_calls,
        timeline,
    })
}

// ---------------------------------------------------------------------------
// P1-7: 内部辅助函数
// ---------------------------------------------------------------------------

/// 从 store 加载技能,失败返回 NotFound。
fn load_skill(state: &State<'_, AppState>, skill_id: &str) -> Result<Skill, CommandError> {
    state
        .swarm
        .skills
        .store()
        .get(skill_id)
        .map_err(|e| CommandError::db("skill_debug", &e))?
        .ok_or_else(|| CommandError::not_found(format!("skill: {skill_id}")))
}

/// 从 in-DB Skill 合成 SkillManifest。
///
/// in-DB Skill 不携带 version/transport/eligibility 等扩展字段,这里填充合理默认值:
/// * version = "1.0.0"(in-DB skill 不跟踪版本)。
/// * transport = Local(本地执行)。
/// * capabilities = skill.permissions(声明式权限作为能力标签)。
/// * status = "stable"(trust_level >= 2)/ "draft"(否则)。
/// * eligibility = none()(无运行时要求)。
fn synthesize_manifest(skill: &Skill) -> SkillManifest {
    SkillManifest {
        name: skill.name.clone(),
        version: "1.0.0".to_string(),
        description: skill.description.clone(),
        capabilities: skill.permissions.clone(),
        transport: SkillTransport::Local,
        author: None,
        source: None,
        status: Some(if skill.trust_level >= 2 {
            "stable".to_string()
        } else {
            "draft".to_string()
        }),
        dependencies: Vec::new(),
        eligibility: SkillEligibility::none(),
        min_nebula_version: None,
    }
}

/// 合成 SKILL.md 正文(Markdown body,不含 frontmatter)。
fn synthesize_body(skill: &Skill) -> String {
    let mut body = String::new();
    body.push_str(&format!("# {}\n\n", skill.name));
    if !skill.description.is_empty() {
        body.push_str(&format!("{}\n\n", skill.description));
    }
    body.push_str("```");
    body.push_str(&skill.language);
    body.push('\n');
    body.push_str(&skill.code);
    if !skill.code.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("```\n");
    body
}

/// 合成完整 SKILL.md(frontmatter + body),用于跑 SkillSpecValidator。
fn synthesize_skill_md(skill: &Skill) -> String {
    let manifest = synthesize_manifest(skill);
    // 手工序列化 frontmatter(避免引入 serde_yaml 依赖到此处)。
    let mut fm = String::from("---\n");
    fm.push_str(&format!("name: {}\n", manifest.name));
    fm.push_str(&format!("version: {}\n", manifest.version));
    fm.push_str(&format!("description: {}\n", manifest.description));
    if !manifest.capabilities.is_empty() {
        fm.push_str("capabilities:\n");
        for cap in &manifest.capabilities {
            fm.push_str(&format!("  - {}\n", cap));
        }
    }
    fm.push_str("transport: local\n");
    if let Some(ref status) = manifest.status {
        fm.push_str(&format!("status: {}\n", status));
    }
    fm.push_str("---\n\n");
    fm.push_str(&synthesize_body(skill));
    fm
}

/// 依赖检查 — 从 skill.language 派生 bins。
///
/// in-DB Skill 不携带 eligibility 声明,这里按 language 推断:
/// * `python` → 检查 "python" 是否在 PATH。
/// * `wasm` / `llm` → 无 bins 依赖。
/// * 其他 → 尝试查找同名二进制。
fn check_dependencies(skill: &Skill) -> DependencyCheckResult {
    let mut bins_available = HashMap::new();
    let lang = skill.language.trim().to_ascii_lowercase();
    match lang.as_str() {
        "python" => {
            bins_available.insert("python".to_string(), find_in_path("python"));
        }
        "wasm" | "llm" => {
            // 无外部 bins 依赖。
        }
        _ => {
            // 尝试查找同名二进制(可能存在,如 node / ruby)。
            bins_available.insert(lang.clone(), find_in_path(&lang));
        }
    }
    DependencyCheckResult {
        bins_available,
        env_set: HashMap::new(),
        os_supported: true,
    }
}

/// 从 audit log 聚合使用统计。
fn compute_usage_stats(state: &State<'_, AppState>, skill: &Skill) -> UsageStats {
    let call_count = skill.usage_count as u64;
    let entries = state
        .swarm
        .skill_audit_logger
        .list_for_skill(&skill.id, 1000)
        .unwrap_or_default();

    if entries.is_empty() {
        // 无审计记录:成功率默认 1.0(无失败证据),last_used=None。
        return UsageStats {
            call_count,
            success_rate: 1.0,
            last_used: None,
            avg_latency_ms: 0,
        };
    }

    let total = entries.len() as u64;
    let successes = entries.iter().filter(|e| e.success).count() as u64;
    let success_rate = if total > 0 {
        successes as f64 / total as f64
    } else {
        1.0
    };
    // entries 按 executed_at DESC 排序,第一条是最近。
    let last_used = entries.first().map(|e| e.executed_at as u64);
    let avg_latency_ms = entries
        .iter()
        .map(|e| e.duration_ms)
        .sum::<u64>()
        .checked_div(total)
        .unwrap_or(0);

    UsageStats {
        call_count,
        success_rate,
        last_used,
        avg_latency_ms,
    }
}

/// 解析测试输入字符串为 params HashMap。
///
/// 支持两种格式:
/// * JSON 对象(`{"key":"value"}`)→ 直接反序列化为 HashMap。
/// * 纯文本 → 放入 `{"input": <text>}` 单键。
fn parse_test_input(test_input: &str) -> HashMap<String, String> {
    let trimmed = test_input.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }
    // 尝试 JSON 对象。
    if trimmed.starts_with('{') {
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(trimmed) {
            return map;
        }
    }
    // 降级:纯文本放入 "input" 键。
    let mut m = HashMap::new();
    m.insert("input".to_string(), test_input.to_string());
    m
}

/// 在 PATH 中查找二进制(跨平台)。
///
/// 与 `skills::protocol::find_in_path` 等价,但因 protocol 模块的 find_in_path
/// 是私有的,这里复制实现(避免修改 protocol.rs)。
fn find_in_path(bin: &str) -> bool {
    let path_env = match std::env::var("PATH") {
        Ok(v) => v,
        Err(_) => return false,
    };

    #[cfg(target_os = "windows")]
    let suffixes: &[&str] = &["", ".exe", ".bat", ".cmd", ".ps1"];
    #[cfg(not(target_os = "windows"))]
    let suffixes: &[&str] = &[""];

    for dir in std::env::split_paths(&path_env) {
        for suffix in suffixes {
            let candidate = dir.join(format!("{bin}{suffix}"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// P1-7: 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试 SkillInspection 序列化 — 所有字段应可无损往返。
    #[test]
    fn skill_inspection_serializes_round_trip() {
        let inspection = SkillInspection {
            manifest: SkillManifest::new(
                "test-skill",
                "1.0.0",
                "a test skill",
                vec!["io".to_string()],
                SkillTransport::Local,
            ),
            body: "# test-skill\n\ndesc\n\n```python\nprint(1)\n```\n".to_string(),
            validation: ValidationResult {
                structure_ok: true,
                spec_ok: true,
                eligibility_ok: true,
                errors: vec![],
                warnings: vec![],
            },
            dependency_check: DependencyCheckResult {
                bins_available: {
                    let mut m = HashMap::new();
                    m.insert("python".to_string(), true);
                    m
                },
                env_set: HashMap::new(),
                os_supported: true,
            },
            usage_stats: UsageStats {
                call_count: 42,
                success_rate: 0.95,
                last_used: Some(1700000000),
                avg_latency_ms: 123,
            },
        };
        let j = serde_json::to_string(&inspection).expect("serialize should succeed");
        // 关键字段应在 JSON 中出现。
        assert!(j.contains("\"manifest\""), "manifest field missing: {j}");
        assert!(j.contains("\"body\""), "body field missing: {j}");
        assert!(j.contains("\"validation\""), "validation field missing: {j}");
        assert!(j.contains("\"dependency_check\""), "dependency_check field missing: {j}");
        assert!(j.contains("\"usage_stats\""), "usage_stats field missing: {j}");
        // 反序列化还原。
        let back: SkillInspection =
            serde_json::from_str(&j).expect("deserialize should succeed");
        assert_eq!(back.manifest.name, "test-skill");
        assert_eq!(back.usage_stats.call_count, 42);
        assert!((back.usage_stats.success_rate - 0.95).abs() < 1e-9);
        assert_eq!(back.dependency_check.bins_available.get("python"), Some(&true));
    }

    /// 测试 ValidationResult 结构 — 三层布尔 + errors/warnings 列表。
    #[test]
    fn validation_result_structure() {
        let v = ValidationResult {
            structure_ok: true,
            spec_ok: false,
            eligibility_ok: true,
            errors: vec!["transport invalid".to_string()],
            warnings: vec!["description empty".to_string()],
        };
        let j = serde_json::to_string(&v).expect("serialize should succeed");
        assert!(j.contains("\"structure_ok\":true"));
        assert!(j.contains("\"spec_ok\":false"));
        assert!(j.contains("\"eligibility_ok\":true"));
        assert!(j.contains("\"errors\""));
        assert!(j.contains("\"warnings\""));
        assert!(j.contains("transport invalid"));
        assert!(j.contains("description empty"));
        let back: ValidationResult = serde_json::from_str(&j).expect("parse should succeed");
        assert!(back.structure_ok);
        assert!(!back.spec_ok);
        assert!(back.eligibility_ok);
        assert_eq!(back.errors.len(), 1);
        assert_eq!(back.warnings.len(), 1);
    }

    /// 测试 SkillTestResult 结构 — success/output/error/latency_ms/logs。
    #[test]
    fn skill_test_result_structure() {
        let r = SkillTestResult {
            success: true,
            output: "hello world".to_string(),
            error: None,
            latency_ms: 42,
            logs: vec!["[test_run] start".to_string(), "[test_run] success".to_string()],
        };
        let j = serde_json::to_string(&r).expect("serialize should succeed");
        assert!(j.contains("\"success\":true"));
        assert!(j.contains("\"output\":\"hello world\""));
        assert!(j.contains("\"error\":null"));
        assert!(j.contains("\"latency_ms\":42"));
        assert!(j.contains("\"logs\""));
        let back: SkillTestResult = serde_json::from_str(&j).expect("parse should succeed");
        assert!(back.success);
        assert_eq!(back.output, "hello world");
        assert!(back.error.is_none());
        assert_eq!(back.latency_ms, 42);
        assert_eq!(back.logs.len(), 2);

        // 失败场景:error 字段应非 null。
        let r_err = SkillTestResult {
            success: false,
            output: String::new(),
            error: Some("timeout".to_string()),
            latency_ms: 5000,
            logs: vec![],
        };
        let j_err = serde_json::to_string(&r_err).expect("serialize should succeed");
        assert!(j_err.contains("\"success\":false"));
        assert!(j_err.contains("\"error\":\"timeout\""));
    }

    /// 测试 SkillProfile 序列化 — cpu/memory/io/sub_calls/timeline。
    #[test]
    fn skill_profile_serializes() {
        let p = SkillProfile {
            cpu_time_ms: 100,
            memory_bytes: 2048,
            io_operations: 3,
            sub_calls: 2,
            timeline: vec![
                ProfileEvent {
                    name: "start".to_string(),
                    timestamp_ms: 0,
                    duration_ms: 0,
                },
                ProfileEvent {
                    name: "execute".to_string(),
                    timestamp_ms: 1,
                    duration_ms: 98,
                },
                ProfileEvent {
                    name: "end".to_string(),
                    timestamp_ms: 99,
                    duration_ms: 0,
                },
            ],
        };
        let j = serde_json::to_string(&p).expect("serialize should succeed");
        assert!(j.contains("\"cpu_time_ms\":100"));
        assert!(j.contains("\"memory_bytes\":2048"));
        assert!(j.contains("\"io_operations\":3"));
        assert!(j.contains("\"sub_calls\":2"));
        assert!(j.contains("\"timeline\""));
        let back: SkillProfile = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(back.cpu_time_ms, 100);
        assert_eq!(back.memory_bytes, 2048);
        assert_eq!(back.timeline.len(), 3);
        assert_eq!(back.timeline[1].name, "execute");
        assert_eq!(back.timeline[1].duration_ms, 98);
    }

    /// 测试 parse_test_input — JSON 对象 / 纯文本 / 空字符串。
    #[test]
    fn parse_test_input_handles_json_and_text() {
        // JSON 对象。
        let m = parse_test_input(r#"{"key":"value","num":"42"}"#);
        assert_eq!(m.get("key"), Some(&"value".to_string()));
        assert_eq!(m.get("num"), Some(&"42".to_string()));

        // 纯文本 → "input" 键。
        let m = parse_test_input("hello world");
        assert_eq!(m.get("input"), Some(&"hello world".to_string()));
        assert_eq!(m.len(), 1);

        // 空字符串 → 空 map。
        let m = parse_test_input("");
        assert!(m.is_empty());

        // 仅空白 → 空 map。
        let m = parse_test_input("   ");
        assert!(m.is_empty());

        // 非法 JSON(以 { 开头但格式错)→ 降级到 input 键。
        let m = parse_test_input("{invalid json}");
        assert_eq!(m.get("input"), Some(&"{invalid json}".to_string()));
    }

    /// 测试 synthesize_skill_md — 合成的 SKILL.md 应通过三层校验。
    #[test]
    fn synthesize_skill_md_passes_validation() {
        let skill = Skill {
            id: "test-1".to_string(),
            name: "test-skill".to_string(),
            description: "a test skill".to_string(),
            code: "print('hello')".to_string(),
            language: "python".to_string(),
            tags: vec![],
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 2,
            permissions: vec!["file:read".to_string()],
            capabilities: crate::skills::sandbox::CapabilitySet::new(),
        };
        let md = synthesize_skill_md(&skill);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: test-skill"));
        assert!(md.contains("version: 1.0.0"));
        assert!(md.contains("transport: local"));
        assert!(md.contains("status: stable"));
        assert!(md.contains("# test-skill"));

        // 合成的 SKILL.md 应通过校验(无硬错误)。
        let report = SkillSpecValidator::validate_skill_md(&md);
        assert!(
            report.errors.is_empty(),
            "expected no errors, got: {:?}",
            report.errors
        );
        assert!(report.valid, "expected valid, got failures: {:?}", report.eligibility_failures);
    }

    /// 测试 check_dependencies — python 语言技能应检查 python 二进制。
    #[test]
    fn check_dependencies_python_skill() {
        let mut skill = Skill {
            id: "test-1".to_string(),
            name: "test".to_string(),
            description: "".to_string(),
            code: "".to_string(),
            language: "python".to_string(),
            tags: vec![],
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: crate::skills::sandbox::CapabilitySet::new(),
        };
        let dep = check_dependencies(&skill);
        assert_eq!(dep.bins_available.len(), 1);
        assert!(dep.bins_available.contains_key("python"));
        assert!(dep.env_set.is_empty());
        assert!(dep.os_supported);

        // llm 语言:无 bins 依赖。
        skill.language = "llm".to_string();
        let dep = check_dependencies(&skill);
        assert!(dep.bins_available.is_empty());

        // wasm 语言:无 bins 依赖。
        skill.language = "wasm".to_string();
        let dep = check_dependencies(&skill);
        assert!(dep.bins_available.is_empty());
    }

    /// 测试 synthesize_manifest — trust_level 影响 status 字段。
    #[test]
    fn synthesize_manifest_status_reflects_trust_level() {
        let mut skill = Skill {
            id: "test-1".to_string(),
            name: "test".to_string(),
            description: "d".to_string(),
            code: "c".to_string(),
            language: "python".to_string(),
            tags: vec![],
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: crate::skills::sandbox::CapabilitySet::new(),
        };
        // trust_level=0 → draft。
        let m = synthesize_manifest(&skill);
        assert_eq!(m.status.as_deref(), Some("draft"));

        // trust_level=2 → stable。
        skill.trust_level = 2;
        let m = synthesize_manifest(&skill);
        assert_eq!(m.status.as_deref(), Some("stable"));

        // trust_level=3 → stable。
        skill.trust_level = 3;
        let m = synthesize_manifest(&skill);
        assert_eq!(m.status.as_deref(), Some("stable"));
    }

    /// 测试 DebugSession / DebugStepResult 序列化。
    #[test]
    fn debug_session_and_step_result_serialize() {
        let session = DebugSession {
            session_id: "sess-1".to_string(),
            skill_id: "skill-1".to_string(),
            test_input: "hello".to_string(),
            steps: vec!["load".to_string(), "execute".to_string()],
            variables: {
                let mut m = HashMap::new();
                m.insert("skill.name".to_string(), "test".to_string());
                m
            },
            call_stack: vec![],
            created_at: 1700000000,
        };
        let j = serde_json::to_string(&session).expect("serialize should succeed");
        assert!(j.contains("\"session_id\":\"sess-1\""));
        assert!(j.contains("\"skill_id\":\"skill-1\""));
        let back: DebugSession = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(back.steps.len(), 2);

        let step_result = DebugStepResult {
            step: "load".to_string(),
            success: true,
            output: "loaded".to_string(),
            error: None,
            variables: HashMap::new(),
            call_stack: vec!["load".to_string()],
        };
        let j = serde_json::to_string(&step_result).expect("serialize should succeed");
        assert!(j.contains("\"step\":\"load\""));
        assert!(j.contains("\"success\":true"));
        let back: DebugStepResult = serde_json::from_str(&j).expect("parse should succeed");
        assert_eq!(back.step, "load");
        assert!(back.success);
    }
}
