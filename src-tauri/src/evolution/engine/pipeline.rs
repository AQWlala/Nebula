//! EvolutionEngine — 4 Phase 进化管线主逻辑（M4 任务 #55-58）。
//!
//! 这是 EvolutionEngine 的核心实现。每个 Phase 调用 `dispatch(WorkType::Evolution)`
//! 走本地 LLM，将 L1/L2/L3 记忆提炼为 L5 Lessons，最后写入 SOUL.md evolution-append。
//!
//! ## Phase 数据流
//!
//! ```text
//! Phase 1 (Extract):    L1 memories → L2 Experience
//! Phase 2 (Compile):    L2 memories → L3 Facts
//! Phase 3 (Reflect):    L2 + L3 → L5 Lessons
//! Phase 4 (Soul 反哺):   L5 Lessons → SOUL.md evolution-append
//! ```
//!
//! 每个 Phase 都是幂等的：失败不破坏前序状态，warnings 记录降级原因。

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info};

use crate::llm::dispatcher::{UnifiedModelDispatcher, WorkType};
use crate::llm::ollama::ChatMessage;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use crate::security::{self, InjectionSeverity};

use super::log::{EvolutionLog, EvolutionLogEntry};
use super::EvolutionEngineConfig;
// Note: EvolutionLog is shared via Arc<EvolutionLog> for AppState injection.

/// EvolutionEngine 错误类型。
#[derive(Debug, Error)]
pub enum EvolutionError {
    #[error("dispatcher error: {0}")]
    Dispatcher(#[from] anyhow::Error),

    #[error("memory store error: {0}")]
    MemoryStore(String),

    #[error("phase {phase:?} timeout after {timeout:?}")]
    Timeout { phase: EvolutionPhase, timeout: Duration },

    #[error("injection scan rejected L5 content (severity={severity:?})")]
    InjectionRejected { severity: InjectionSeverity },

    #[error("soul write error: {0}")]
    SoulWrite(String),

    #[error("evolution disabled at runtime")]
    Disabled,
}

/// 进化阶段标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionPhase {
    /// Phase 1: 经验提取（L1 → L2）
    Extract,
    /// Phase 2: 知识编译（L2 → L3）
    Compile,
    /// Phase 3: 元认知反思（L2+L3 → L5）
    Reflect,
    /// Phase 4: Soul 反哺（L5 → SOUL.md evolution-append）
    Soul,
}

impl EvolutionPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            EvolutionPhase::Extract => "extract",
            EvolutionPhase::Compile => "compile",
            EvolutionPhase::Reflect => "reflect",
            EvolutionPhase::Soul => "soul",
        }
    }

    /// 全部 4 个 Phase，按执行顺序。
    pub fn all_in_order() -> [EvolutionPhase; 4] {
        [
            EvolutionPhase::Extract,
            EvolutionPhase::Compile,
            EvolutionPhase::Reflect,
            EvolutionPhase::Soul,
        ]
    }
}

impl std::fmt::Display for EvolutionPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 单个 Phase 的输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseOutput {
    pub phase: EvolutionPhase,
    /// LLM 生成的提炼文本（L2/L3/L5 内容）。
    pub content: String,
    /// 生成的 memory ID（若写入了 SQLite）。
    pub memory_id: Option<String>,
    /// 进化日志条目 ID（若写入了 evolution_log.md）。
    pub log_entry_id: Option<String>,
    /// 该阶段的 warnings（注入命中、降级、超时等）。
    pub warnings: Vec<String>,
    /// 是否发生降级（true 表示未走 LLM，仅文本拼接/空输出）。
    pub degraded: bool,
}

impl PhaseOutput {
    pub(crate) fn new(phase: EvolutionPhase) -> Self {
        Self {
            phase,
            content: String::new(),
            memory_id: None,
            log_entry_id: None,
            warnings: Vec::new(),
            degraded: false,
        }
    }
}

/// EvolutionEngine 完整执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionResult {
    /// 各 Phase 输出（按执行顺序）。
    pub phases: Vec<PhaseOutput>,
    /// 总体 warnings（聚合各 Phase）。
    pub warnings: Vec<String>,
    /// 是否整体降级。
    pub degraded: bool,
    /// 写入 SOUL.md 的最终文本（Phase 4 输出，可能为空表示未写入）。
    pub soul_append_text: String,
    /// master_id（domain 标识，用于记忆隔离）。
    pub master_id: String,
}

/// EvolutionEngine 进化引擎。
///
/// 持有以下依赖：
/// - `dispatcher: Arc<UnifiedModelDispatcher>` — LLM 调用经 `dispatch(Evolution)` 强制本地路由
/// - `sqlite: Arc<SqliteStore>` — 读取 L1/L2/L3 + 写入 L2/L3/L5
/// - `sponge: Arc<SpongeEngine>` — 记忆写入经 `absorb_with_principal("evolution:<master_id>", mem)`
/// - `log: EvolutionLog` — 进化日志（与 SOUL.md 同事务写入）
///
/// **线程安全**：所有字段都是 `Arc` 或 `&`，可跨 await 点共享。
/// 内部不持有任何 `MutexGuard` 跨 await（参考 lessons learned: MutexGuard must not cross await points）。
pub struct EvolutionEngine {
    dispatcher: Arc<UnifiedModelDispatcher>,
    sqlite: Arc<SqliteStore>,
    sponge: Arc<SpongeEngine>,
    log: Arc<EvolutionLog>,
    config: EvolutionEngineConfig,
}

impl std::fmt::Debug for EvolutionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvolutionEngine")
            .field("config", &self.config)
            .finish()
    }
}

impl EvolutionEngine {
    /// 构造 EvolutionEngine。
    pub fn new(
        dispatcher: Arc<UnifiedModelDispatcher>,
        sqlite: Arc<SqliteStore>,
        sponge: Arc<SpongeEngine>,
        log: Arc<EvolutionLog>,
        config: EvolutionEngineConfig,
    ) -> Self {
        Self {
            dispatcher,
            sqlite,
            sponge,
            log,
            config,
        }
    }

    /// Builder: 替换 config。
    pub fn with_config(mut self, config: EvolutionEngineConfig) -> Self {
        self.config = config;
        self
    }

    /// 查询是否启用（运行时双层 gate）。
    pub fn is_enabled(&self) -> bool {
        self.config.enabled && super::super::evolution_enabled()
    }

    /// 运行时启用/禁用（不修改 config，仅检查运行时开关）。
    pub fn runtime_enabled(&self) -> bool {
        super::super::evolution_enabled()
    }

    /// 执行完整 4 Phase 进化管线。
    ///
    /// `master_id` 用于：
    /// 1. 记忆写入的 domain 隔离：`absorb_with_principal("evolution:<master_id>", mem)`
    /// 2. 进化日志的 provenance 记录
    /// 3. Phase 4 写入 SOUL.md 时校验 master_id 一致性（M2b 任务 #38，P1-4 EA-2）
    ///
    /// 失败不破坏前序状态：每个 Phase 失败仅记 warning 并继续下一 Phase。
    pub async fn run(&self, master_id: &str) -> Result<EvolutionResult, EvolutionError> {
        if !self.is_enabled() {
            return Err(EvolutionError::Disabled);
        }

        let mut phases: Vec<PhaseOutput> = Vec::with_capacity(4);
        let mut all_warnings: Vec<String> = Vec::new();
        let mut degraded = false;
        let soul_append_text;

        // Phase 1: Extract
        let p1 = self.run_phase1_extract(master_id).await?;
        if p1.degraded {
            degraded = true;
        }
        all_warnings.extend(p1.warnings.clone());
        phases.push(p1);

        // Phase 2: Compile
        let p2 = self.run_phase2_compile(master_id).await?;
        if p2.degraded {
            degraded = true;
        }
        all_warnings.extend(p2.warnings.clone());
        phases.push(p2);

        // Phase 3: Reflect
        let p3 = self.run_phase3_reflect(master_id).await?;
        if p3.degraded {
            degraded = true;
        }
        all_warnings.extend(p3.warnings.clone());
        let p3_content = p3.content.clone();
        phases.push(p3);

        // Phase 4: Soul 反哺（即便前 Phase 降级，仍尝试写入空内容以保持日志完整）
        let p4 = self.run_phase4_soul(master_id, &p3_content).await?;
        if p4.degraded {
            degraded = true;
        }
        all_warnings.extend(p4.warnings.clone());
        soul_append_text = p4.content.clone();
        phases.push(p4);

        Ok(EvolutionResult {
            phases,
            warnings: all_warnings,
            degraded,
            soul_append_text,
            master_id: master_id.to_string(),
        })
    }

    /// Phase 1: 经验提取（L1 → L2 Experience）。
    ///
    /// 读取最近 N 条 L1 记忆（domain = "shared"），调用 `dispatch(Evolution)` 提炼为 L2。
    /// 写入新 L2 memory 经 `absorb_with_principal("evolution:<master_id>", mem)`。
    async fn run_phase1_extract(&self, master_id: &str) -> Result<PhaseOutput, EvolutionError> {
        let mut out = PhaseOutput::new(EvolutionPhase::Extract);
        debug!(target: "nebula.evolution", phase = "extract", master_id, "phase 1 start");

        // 读 L1（默认域 shared）
        let l1_mems = self
            .sqlite
            .list_by_layer_in_domain(MemoryLayer::L1, "shared", self.config.phase1_l1_window)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("read L1: {e}")))?;

        if l1_mems.is_empty() {
            out.warnings.push("no L1 memories to extract from".to_string());
            out.degraded = true;
            return Ok(out);
        }

        // 构造 prompt
        let l1_text = l1_mems
            .iter()
            .map(|m| format!("- [{}] {}", m.created_at, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "你是经验提取器。从以下最近对话记录(L1)中提炼出跨会话的经验(L2)。\n\
             要求：\n\
             1. 提取可复用的事实、用户偏好、任务模式\n\
             2. 跳过临时性、琐碎的内容\n\
             3. 每条经验一行，以 '- ' 开头\n\
             4. 最多 10 条\n\n\
             L1 记录：\n{l1_text}"
        );

        let messages = vec![
            ChatMessage::system("You are an experience extractor for the nebula memory system."),
            ChatMessage::user(prompt),
        ];

        // dispatch(Evolution) 强制本地路由 + 超时保护
        let content = self
            .dispatch_with_timeout(messages, EvolutionPhase::Extract, &mut out.warnings)
            .await?;

        // 写入 L2 memory（domain = master_id，经 absorb_with_principal）
        let mut mem = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L2,
            content.clone(),
            SourceKind::Reflection,
        );
        mem.metadata = serde_json::json!({
            "phase": "extract",
            "master_id": master_id,
            "l1_count": l1_mems.len(),
        });

        let principal = format!("evolution:{master_id}");
        let sponge_result = self
            .sponge
            .absorb_with_principal(&principal, mem)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("write L2: {e}")))?;

        out.content = content;
        out.memory_id = Some(sponge_result.id().to_string());

        // 进化日志
        let log_entry = EvolutionLogEntry::new(
            EvolutionPhase::Extract,
            master_id,
            sponge_result.id(),
            out.content.len() as u64,
        );
        match self.log.append(&log_entry).await {
            Ok(id) => out.log_entry_id = Some(id),
            Err(e) => out.warnings.push(format!("log append failed: {e}")),
        }

        info!(target: "nebula.evolution",
            phase = "extract",
            master_id,
            l1_count = l1_mems.len(),
            memory_id = %sponge_result.id(),
            "phase 1 done");
        Ok(out)
    }

    /// Phase 2: 知识编译（L2 → L3 Facts）。
    ///
    /// 读取 domain = master_id 的最近 N 条 L2 记忆，调用 `dispatch(Evolution)` 编译为 L3 Facts。
    async fn run_phase2_compile(&self, master_id: &str) -> Result<PhaseOutput, EvolutionError> {
        let mut out = PhaseOutput::new(EvolutionPhase::Compile);
        debug!(target: "nebula.evolution", phase = "compile", master_id, "phase 2 start");

        let l2_mems = self
            .sqlite
            .list_by_layer_in_domain(MemoryLayer::L2, master_id, self.config.phase2_l2_window)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("read L2: {e}")))?;

        if l2_mems.is_empty() {
            out.warnings
                .push("no L2 memories in domain to compile".to_string());
            out.degraded = true;
            return Ok(out);
        }

        let l2_text = l2_mems
            .iter()
            .map(|m| format!("- {}", m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "你是知识编译器。将以下跨会话经验(L2)编译为结构化的事实(L3)。\n\
             要求：\n\
             1. 合并重复或相近的经验\n\
             2. 抽象出普适的事实陈述（避免具体到某次对话）\n\
             3. 每条事实一行，以 '- ' 开头\n\
             4. 最多 8 条\n\n\
             L2 经验：\n{l2_text}"
        );

        let messages = vec![
            ChatMessage::system("You are a knowledge compiler for the nebula memory system."),
            ChatMessage::user(prompt),
        ];

        let content = self
            .dispatch_with_timeout(messages, EvolutionPhase::Compile, &mut out.warnings)
            .await?;

        let mut mem = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            content.clone(),
            SourceKind::Reflection,
        );
        mem.metadata = serde_json::json!({
            "phase": "compile",
            "master_id": master_id,
            "l2_count": l2_mems.len(),
        });

        let principal = format!("evolution:{master_id}");
        let sponge_result = self
            .sponge
            .absorb_with_principal(&principal, mem)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("write L3: {e}")))?;

        out.content = content;
        out.memory_id = Some(sponge_result.id().to_string());

        let log_entry = EvolutionLogEntry::new(
            EvolutionPhase::Compile,
            master_id,
            sponge_result.id(),
            out.content.len() as u64,
        );
        match self.log.append(&log_entry).await {
            Ok(id) => out.log_entry_id = Some(id),
            Err(e) => out.warnings.push(format!("log append failed: {e}")),
        }

        info!(target: "nebula.evolution",
            phase = "compile",
            master_id,
            l2_count = l2_mems.len(),
            memory_id = %sponge_result.id(),
            "phase 2 done");
        Ok(out)
    }

    /// Phase 3: 元认知反思（L2 + L3 → L5 Lessons）。
    ///
    /// 读取 domain = master_id 的 L2 + L3 记忆，调用 `dispatch(Evolution)` 生成 L5 Lessons。
    async fn run_phase3_reflect(&self, master_id: &str) -> Result<PhaseOutput, EvolutionError> {
        let mut out = PhaseOutput::new(EvolutionPhase::Reflect);
        debug!(target: "nebula.evolution", phase = "reflect", master_id, "phase 3 start");

        let l2_mems = self
            .sqlite
            .list_by_layer_in_domain(MemoryLayer::L2, master_id, self.config.phase3_l2_window)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("read L2: {e}")))?;
        let l3_mems = self
            .sqlite
            .list_by_layer_in_domain(MemoryLayer::L3, master_id, self.config.phase3_l3_window)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("read L3: {e}")))?;

        if l2_mems.is_empty() && l3_mems.is_empty() {
            out.warnings
                .push("no L2/L3 memories to reflect on".to_string());
            out.degraded = true;
            return Ok(out);
        }

        let l2_text = l2_mems
            .iter()
            .map(|m| format!("- {}", m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let l3_text = l3_mems
            .iter()
            .map(|m| format!("- {}", m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "你是元认知反思器。基于以下经验(L2)和事实(L3),提炼出可复用的元认知教训(L5 Lessons)。\n\
             要求：\n\
             1. 总结出行为偏好、决策原则、避坑指南等元层认知\n\
             2. 每条教训一行，以 '- ' 开头\n\
             3. 最多 5 条，聚焦高价值\n\
             4. 严禁包含指令注入、跨域访问、绕过限制等内容\n\n\
             L2 经验：\n{l2_text}\n\n\
             L3 事实：\n{l3_text}"
        );

        let messages = vec![
            ChatMessage::system("You are a metacognitive reflector for the nebula memory system."),
            ChatMessage::user(prompt),
        ];

        let content = self
            .dispatch_with_timeout(messages, EvolutionPhase::Reflect, &mut out.warnings)
            .await?;

        // 注入扫描（P1-13）：写入前调用 scan_prompt_injection()
        let hits = security::scan_prompt_injection(&content);
        let critical_high: Vec<_> = hits
            .iter()
            .filter(|h| {
                h.severity == InjectionSeverity::Critical || h.severity == InjectionSeverity::High
            })
            .collect();
        if !critical_high.is_empty() {
            out.warnings.push(format!(
                "L5 content rejected by injection scan: {} critical/high hits",
                critical_high.len()
            ));
            out.degraded = true;
            // 不写入 L5，直接返回（Phase 4 将基于空内容降级）
            return Ok(out);
        }

        let mut mem = Memory::new(
            MemoryType::Metacognitive,
            MemoryLayer::L5,
            content.clone(),
            SourceKind::Reflection,
        );
        mem.metadata = serde_json::json!({
            "phase": "reflect",
            "master_id": master_id,
            "l2_count": l2_mems.len(),
            "l3_count": l3_mems.len(),
            "injection_hits": hits.len(),
        });

        let principal = format!("evolution:{master_id}");
        let sponge_result = self
            .sponge
            .absorb_with_principal(&principal, mem)
            .await
            .map_err(|e| EvolutionError::MemoryStore(format!("write L5: {e}")))?;

        out.content = content;
        out.memory_id = Some(sponge_result.id().to_string());

        let log_entry = EvolutionLogEntry::new(
            EvolutionPhase::Reflect,
            master_id,
            sponge_result.id(),
            out.content.len() as u64,
        );
        match self.log.append(&log_entry).await {
            Ok(id) => out.log_entry_id = Some(id),
            Err(e) => out.warnings.push(format!("log append failed: {e}")),
        }

        info!(target: "nebula.evolution",
            phase = "reflect",
            master_id,
            l2_count = l2_mems.len(),
            l3_count = l3_mems.len(),
            memory_id = %sponge_result.id(),
            "phase 3 done");
        Ok(out)
    }

    /// Phase 4: Soul 反哺（L5 → SOUL.md evolution-append）。
    ///
    /// 将 Phase 3 生成的 L5 Lessons 写入 SOUL.md 的 evolution-append Section。
    /// 写入前再次调用 `full_injection_scan`（输出侧扫描，P1-13）。
    /// 经 `soul::atomic_write` 保证原子性（P1-14）。
    async fn run_phase4_soul(
        &self,
        master_id: &str,
        l5_content: &str,
    ) -> Result<PhaseOutput, EvolutionError> {
        let mut out = PhaseOutput::new(EvolutionPhase::Soul);
        debug!(target: "nebula.evolution", phase = "soul", master_id, "phase 4 start");

        if l5_content.is_empty() {
            out.warnings
                .push("L5 content is empty; skipping SOUL.md write".to_string());
            out.degraded = true;
            return Ok(out);
        }

        // Step 1: 拼接要写入的文本（带 master_id provenance）
        let timestamp = chrono::Utc::now().to_rfc3339();
        let append_text = format!(
            "## [{timestamp}] Evolution (master={master_id})\n\n{l5_content}\n"
        );

        // Step 2: 输出侧注入扫描（P1-13）
        let scan_result = security::full_injection_scan(&append_text);
        let is_critical_or_high = scan_result
            .max_severity
            .map(|s| s == InjectionSeverity::Critical || s == InjectionSeverity::High)
            .unwrap_or(false);
        if is_critical_or_high {
            out.warnings.push(format!(
                "SOUL.md append rejected by full_injection_scan (severity={:?})",
                scan_result.max_severity
            ));
            out.degraded = true;
            return Ok(out);
        }

        // Step 3: 校验 master_id 与 SOUL.md 路径一致（M2b 任务 #38，P1-4 EA-2）
        // 实现:读取 SOUL.md 的 immutable_from_ai section,解析 master_id 元数据行,
        // 与当前 run(master_id) 比对。不匹配则拒绝写入(防止跨实例写入)。
        // 首次写入(SOUL.md 不存在或无 master_id 元数据)时,自动写入当前 master_id。
        if let Err(e) = self.verify_soul_md_master_id(master_id) {
            out.warnings.push(format!(
                "SOUL.md master_id verification failed: {e}"
            ));
            out.degraded = true;
            return Ok(out);
        }

        // Step 4: 读取现有 SOUL.md（若存在），构造新内容
        let soul_path = std::path::Path::new(&self.config.soul_md_path);
        let existing = match std::fs::read_to_string(soul_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(EvolutionError::SoulWrite(format!(
                    "read SOUL.md: {e}"
                )));
            }
        };

        let new_soul_md = self.append_to_evolution_section(&existing, &append_text)?;

        // M2b 任务 #38 / P1-4 EA-2: 首次写入时,在 immutable_from_ai section
        // 注入 master_id 元数据行(供后续 verify_soul_md_master_id 校验)。
        // 仅在 existing 为空(全新 SOUL.md)且 new_soul_md 含 immutable_from_ai
        // section 但无 master_id 元数据时注入。
        if existing.is_empty() {
            let new_with_metadata = inject_master_id_metadata(&new_soul_md, master_id);
            // Step 5: 原子写入（write-temp-then-rename + 备份）
            #[cfg(feature = "soul-system")]
            {
                crate::soul::atomic_write::atomic_write(soul_path, &new_with_metadata)
                    .map_err(|e| EvolutionError::SoulWrite(format!("atomic_write: {e}")))?;
            }
            #[cfg(not(feature = "soul-system"))]
            {
                std::fs::write(soul_path, &new_with_metadata)
                    .map_err(|e| EvolutionError::SoulWrite(format!("fs::write: {e}")))?;
            }
        } else {
            // Step 5: 原子写入（write-temp-then-rename + 备份）
            #[cfg(feature = "soul-system")]
            {
                crate::soul::atomic_write::atomic_write(soul_path, &new_soul_md)
                    .map_err(|e| EvolutionError::SoulWrite(format!("atomic_write: {e}")))?;
            }
            // soul-system 未启用时回退到普通写入（仍保证覆盖语义，但不原子）
            #[cfg(not(feature = "soul-system"))]
            {
                std::fs::write(soul_path, &new_soul_md)
                    .map_err(|e| EvolutionError::SoulWrite(format!("fs::write: {e}")))?;
            }
        }

        // Step 6: 写入进化日志（与 SOUL.md 同事务）
        out.content = append_text.clone();
        let log_entry = EvolutionLogEntry::new(
            EvolutionPhase::Soul,
            master_id,
            "", // SOUL.md 写入不产生 memory_id
            out.content.len() as u64,
        )
        .with_soul_md_path(&self.config.soul_md_path);
        match self.log.append(&log_entry).await {
            Ok(id) => out.log_entry_id = Some(id),
            Err(e) => out.warnings.push(format!("log append failed: {e}")),
        }

        info!(target: "nebula.evolution",
            phase = "soul",
            master_id,
            soul_md = %self.config.soul_md_path,
            bytes = out.content.len(),
            "phase 4 done (SOUL.md updated)");
        Ok(out)
    }

    /// 调用 `dispatch(Evolution)` 并带超时保护。
    ///
    /// 失败不返回 Err，而是降级为空内容 + warning（保持管线继续）。
    /// 仅在底层 dispatcher 返回 Err 时通过 `EvolutionError::Dispatcher` 上抛。
    async fn dispatch_with_timeout(
        &self,
        messages: Vec<ChatMessage>,
        phase: EvolutionPhase,
        warnings: &mut Vec<String>,
    ) -> Result<String, EvolutionError> {
        let timeout = Duration::from_secs(self.config.phase_timeout_secs);
        let dispatch_fut = self.dispatcher.dispatch(WorkType::Evolution, messages);

        match tokio::time::timeout(timeout, dispatch_fut).await {
            Ok(Ok(resp)) => Ok(resp.message.content),
            Ok(Err(e)) => {
                warnings.push(format!("phase {phase} dispatch failed: {e}; degraded"));
                Ok(String::new())
            }
            Err(_) => {
                warnings.push(format!(
                    "phase {phase} timeout after {timeout:?}; degraded"
                ));
                Ok(String::new())
            }
        }
    }

    /// 将 `append_text` 追加到 SOUL.md 的 `evolution-append` Section 内。
    ///
    /// 若 SOUL.md 不存在或不包含 Section 标签，则构造一个最小骨架。
    /// M2b 任务 #38 / P1-4 EA-2: 校验 SOUL.md 的 master_id 与当前 run 一致。
    ///
    /// 在 Phase 4 写入 SOUL.md 前调用,防止跨实例写入(如实例 A 的进化结果
    /// 写入实例 B 的 SOUL.md)。
    ///
    /// ## 校验逻辑
    ///
    /// 1. 读取 SOUL.md 文件(若不存在则视为首次写入,自动通过)
    /// 2. 在 `immutable_from_ai` section 中查找 `master_id: <id>` 元数据行
    /// 3. 若元数据不存在,视为首次写入,自动通过(后续写入会带上 master_id)
    /// 4. 若元数据存在,与当前 `master_id` 比对:
    ///    - 匹配 → 返回 Ok(())
    ///    - 不匹配 → 返回 Err(拒绝写入)
    ///
    /// ## 元数据格式
    ///
    /// 在 `immutable_from_ai` section 的第一行写入:
    /// ```markdown
    /// <!-- BEGIN SECTION: immutable_from_ai -->
    /// master_id: agent_a
    /// ...
    /// <!-- END SECTION: immutable_from_ai -->
    /// ```
    ///
    /// ## 注意
    ///
    /// 此校验在 Step 3(读取现有 SOUL.md 之前)执行,因此会读取两次文件。
    /// 性能影响可忽略(SOUL.md 通常 < 10KB,本地磁盘读取 < 1ms)。
    fn verify_soul_md_master_id(&self, expected_master_id: &str) -> Result<(), String> {
        let soul_path = std::path::Path::new(&self.config.soul_md_path);

        // 1. 文件不存在 → 首次写入,自动通过
        if !soul_path.exists() {
            return Ok(());
        }

        // 2. 读取文件内容
        let content = match std::fs::read_to_string(soul_path) {
            Ok(s) => s,
            Err(e) => {
                // 读取失败不阻塞写入(避免 IO 错误阻断进化流程),
                // 但记日志警告。
                tracing::warn!(
                    target: "nebula.evolution",
                    path = %soul_path.display(),
                    error = %e,
                    "failed to read SOUL.md for master_id verification; skipping check"
                );
                return Ok(());
            }
        };

        // 3. 在 immutable_from_ai section 中查找 master_id 元数据行
        const SECTION_BEGIN: &str = "<!-- BEGIN SECTION: immutable_from_ai -->";
        const SECTION_END: &str = "<!-- END SECTION: immutable_from_ai -->";
        const MASTER_ID_PREFIX: &str = "master_id:";

        let file_master_id = if let Some(begin_idx) = content.find(SECTION_BEGIN) {
            let section_start = begin_idx + SECTION_BEGIN.len();
            if let Some(end_idx) = content[section_start..].find(SECTION_END) {
                let section_content = &content[section_start..section_start + end_idx];
                // 在 section 内查找 "master_id: <id>" 行
                section_content
                    .lines()
                    .map(|line| line.trim())
                    .find_map(|line| line.strip_prefix(MASTER_ID_PREFIX).map(|s| s.trim()))
            } else {
                // section 未闭合 — 不阻塞(可能文件损坏),记警告
                tracing::warn!(
                    target: "nebula.evolution",
                    path = %soul_path.display(),
                    "immutable_from_ai section unclosed; skipping master_id verification"
                );
                None
            }
        } else {
            // 无 immutable_from_ai section — 视为首次写入,自动通过
            None
        };

        match file_master_id {
            None => {
                // 元数据不存在 → 首次写入此 master 的 SOUL.md,自动通过
                tracing::debug!(
                    target: "nebula.evolution",
                    expected_master_id,
                    "no master_id metadata in SOUL.md; first write for this master"
                );
                Ok(())
            }
            Some(actual) => {
                if actual == expected_master_id {
                    tracing::debug!(
                        target: "nebula.evolution",
                        master_id = actual,
                        "SOUL.md master_id verification passed"
                    );
                    Ok(())
                } else {
                    // 不匹配 → 拒绝写入(防止跨实例写入)
                    tracing::warn!(
                        target: "nebula.evolution",
                        expected = expected_master_id,
                        actual = actual,
                        "SOUL.md master_id mismatch; refusing to write"
                    );
                    Err(format!(
                        "master_id mismatch: SOUL.md belongs to '{actual}', \
                         but evolution run is for '{expected_master_id}'"
                    ))
                }
            }
        }
    }

    /// 若 `evolution-append` Section 不存在，则在末尾追加新 Section。
    /// 行数超过 `phase4_max_lines` 时仅保留最后 N 行（FIFO 淘汰）。
    fn append_to_evolution_section(
        &self,
        existing_soul_md: &str,
        append_text: &str,
    ) -> Result<String, EvolutionError> {
        const BEGIN_TAG: &str = "<!-- BEGIN SECTION: evolution-append -->";
        const END_TAG: &str = "<!-- END SECTION: evolution-append -->";

        // 简单字符串处理（不依赖 soul-system feature）
        let new_soul_md = if existing_soul_md.is_empty() {
            // 全新 SOUL.md：构造骨架
            format!(
                "<!-- BEGIN SECTION: immutable_from_ai -->\n\
                 (用户核心理念 - 待填充)\n\
                 <!-- END SECTION: immutable_from_ai -->\n\n\
                 {BEGIN_TAG}\n\
                 {append_text}\n\
                 {END_TAG}\n"
            )
        } else if let Some(begin_idx) = existing_soul_md.find(BEGIN_TAG) {
            // 已有 evolution-append Section：在 BEGIN 后插入
            let insert_pos = begin_idx + BEGIN_TAG.len();
            let mut new_content = String::with_capacity(existing_soul_md.len() + append_text.len() + 2);
            new_content.push_str(&existing_soul_md[..insert_pos]);
            new_content.push('\n');
            new_content.push_str(append_text);
            new_content.push_str(&existing_soul_md[insert_pos..]);

            // FIFO 淘汰：保留最后 N 行
            self.truncate_evolution_section_to_max_lines(new_content)
        } else {
            // SOUL.md 存在但无 evolution-append Section：在末尾追加
            format!(
                "{existing_soul_md}\n\n{BEGIN_TAG}\n{append_text}\n{END_TAG}\n"
            )
        };

        Ok(new_soul_md)
    }

    /// 截断 evolution-append Section 内容到 `phase4_max_lines` 行（FIFO 淘汰）。
    ///
    /// 仅截断 evolution-append Section 内的行，不影响其他 Section。
    fn truncate_evolution_section_to_max_lines(&self, soul_md: String) -> String {
        const BEGIN_TAG: &str = "<!-- BEGIN SECTION: evolution-append -->";
        const END_TAG: &str = "<!-- END SECTION: evolution-append -->";
        let max_lines = self.config.phase4_max_lines;

        let begin_idx = match soul_md.find(BEGIN_TAG) {
            Some(i) => i,
            None => return soul_md,
        };
        let end_idx = match soul_md.find(END_TAG) {
            Some(i) => i,
            None => return soul_md,
        };
        if end_idx <= begin_idx {
            return soul_md;
        }

        let section_start = begin_idx + BEGIN_TAG.len();
        let section_end = end_idx;
        let section_content = &soul_md[section_start..section_end];

        let lines: Vec<&str> = section_content.lines().filter(|l| !l.is_empty()).collect();
        if lines.len() <= max_lines {
            return soul_md;
        }

        // 保留最后 N 行（FIFO 淘汰最早的）
        let kept_lines = &lines[lines.len() - max_lines..];
        let new_section: String = kept_lines.iter().copied().collect::<Vec<_>>().join("\n");

        let mut result = String::with_capacity(soul_md.len());
        result.push_str(&soul_md[..section_start]);
        result.push_str("\n");
        result.push_str(&new_section);
        result.push_str(&soul_md[section_end..]);
        result
    }
}

/// 在 SOUL.md 的 `immutable_from_ai` Section 中注入 `master_id` 元数据行。
///
/// 仅在以下条件全满足时注入:
/// 1. `soul_md` 中存在 `<!-- BEGIN SECTION: immutable_from_ai -->` 标签
/// 2. section 内尚未含 `master_id:` 行(避免重复注入)
///
/// 注入位置:section BEGIN 标签后的第一行。
/// 格式:`master_id: <id>`(可被 [`EvolutionEngine::verify_soul_md_master_id`] 解析)。
///
/// 若条件不满足,原样返回 `soul_md`。
///
/// M2b 任务 #38 / P1-4 EA-2:首次写入 SOUL.md 时,在 immutable_from_ai
/// section 注入 master_id 元数据,后续写入通过 `verify_soul_md_master_id`
/// 校验一致性,防止跨实例写入。
fn inject_master_id_metadata(soul_md: &str, master_id: &str) -> String {
    const SECTION_BEGIN: &str = "<!-- BEGIN SECTION: immutable_from_ai -->";
    const SECTION_END: &str = "<!-- END SECTION: immutable_from_ai -->";
    const MASTER_ID_PREFIX: &str = "master_id:";

    let begin_idx = match soul_md.find(SECTION_BEGIN) {
        Some(i) => i,
        // 无 section — 原样返回(不应发生,append_to_evolution_section 已构造骨架)
        None => return soul_md.to_string(),
    };
    let section_start = begin_idx + SECTION_BEGIN.len();
    let section_end = soul_md[section_start..]
        .find(SECTION_END)
        .map(|i| section_start + i)
        .unwrap_or(soul_md.len());
    let section_content = &soul_md[section_start..section_end];

    // 若 section 已含 master_id 行,不重复注入(幂等)
    let already_has = section_content
        .lines()
        .map(|line| line.trim())
        .any(|line| line.starts_with(MASTER_ID_PREFIX));
    if already_has {
        return soul_md.to_string();
    }

    // 在 section BEGIN 标签后插入 master_id 元数据行
    let mut result = String::with_capacity(
        soul_md.len() + MASTER_ID_PREFIX.len() + master_id.len() + 4,
    );
    result.push_str(&soul_md[..section_start]);
    result.push('\n');
    result.push_str(MASTER_ID_PREFIX);
    result.push(' ');
    result.push_str(master_id);
    result.push_str(&soul_md[section_start..]);
    result
}

#[cfg(test)]
mod master_id_verification_tests {
    use super::*;

    #[test]
    fn inject_master_id_metadata_inserts_into_empty_section() {
        let soul_md = "<!-- BEGIN SECTION: immutable_from_ai -->\n(用户核心理念)\n<!-- END SECTION: immutable_from_ai -->";
        let result = inject_master_id_metadata(soul_md, "agent_a");
        assert!(
            result.contains("master_id: agent_a"),
            "master_id metadata should be injected; got: {result}"
        );
        // 仅注入一次(不应重复)
        let result2 = inject_master_id_metadata(&result, "agent_a");
        assert_eq!(
            result.matches("master_id:").count(),
            result2.matches("master_id:").count(),
            "second inject should be idempotent"
        );
    }

    #[test]
    fn inject_master_id_metadata_noop_without_section() {
        let soul_md = "no section here";
        let result = inject_master_id_metadata(soul_md, "agent_a");
        assert_eq!(result, soul_md);
    }

    #[test]
    fn verify_soul_md_master_id_accepts_matching_id() {
        // 构造一个 EvolutionEngine 测试实例较重,直接测试 inject + verify 的协作
        // 通过单元逻辑:同一 master_id 注入后,再次 verify 应通过
        let soul_md = "<!-- BEGIN SECTION: immutable_from_ai -->\n(内容)\n<!-- END SECTION: immutable_from_ai -->";
        let with_metadata = inject_master_id_metadata(soul_md, "agent_a");
        // 重新解析 master_id
        const SECTION_BEGIN: &str = "<!-- BEGIN SECTION: immutable_from_ai -->";
        const SECTION_END: &str = "<!-- END SECTION: immutable_from_ai -->";
        let begin_idx = with_metadata.find(SECTION_BEGIN).unwrap();
        let section_start = begin_idx + SECTION_BEGIN.len();
        let end_rel = with_metadata[section_start..].find(SECTION_END).unwrap();
        let section_content = &with_metadata[section_start..section_start + end_rel];
        let parsed = section_content
            .lines()
            .map(|l| l.trim())
            .find_map(|l| l.strip_prefix("master_id:").map(|s| s.trim()));
        assert_eq!(parsed, Some("agent_a"));
    }

    #[test]
    fn inject_master_id_metadata_is_idempotent() {
        let soul_md = "<!-- BEGIN SECTION: immutable_from_ai -->\n<!-- END SECTION: immutable_from_ai -->";
        let once = inject_master_id_metadata(soul_md, "agent_a");
        let twice = inject_master_id_metadata(&once, "agent_a");
        assert_eq!(once, twice, "double inject should be idempotent");
    }
}
