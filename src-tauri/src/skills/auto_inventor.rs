//! P0-6 SkillAutoInventor — Hermes 式自动发明技能机制。
//!
//! 灵感来自 Hermes 的"Agent 自动发明技能"能力:当 Agent 在工作中发现
//! 某个操作序列重复了多次,系统会自动检测该模式并生成一个 skill 草稿
//! (SKILL.md),存入 `~/.nebula/skills/auto-invented/<skill-name>/SKILL.md`。
//!
//! ## 设计要点
//!
//! * **环形缓冲区** —— [`RingBuffer`] 保存最近 N 个操作(默认 1000),
//!   滚动覆盖旧数据,内存占用恒定。
//! * **滑动窗口 + SHA256 hash** —— 在历史缓冲区上滑动长度 3..=10 的窗口,
//!   对 `op_type` 序列计算 SHA256,统计出现次数,超过阈值(默认 5)即标记为
//!   重复模式。
//! * **去重** —— `detected_patterns` 集合保存已发现的模式 ID,避免重复生成。
//! * **trust_level = 0** —— 自动发明的技能一律 `trust_level = 0`,用户必须
//!   手动提升后才能在沙箱外执行(安全红线,与 [`super::importer`] 行为一致)。
//! * **不 panic** —— 所有错误路径返回 `Result<_, String>`,file I/O 失败
//!   不会拖垮调用方。
//!
//! ## 调用关系
//!
//! * `SkillEngine` / `audit` 系统在每次操作后调用 [`SkillAutoInventor::record_operation`]。
//! * 后台任务或前台命令调用 [`SkillAutoInventor::detect_patterns`] 检测重复模式。
//! * 用户通过 Tauri 命令(`auto_invent_accept_pattern`)接受模式,
//!   触发 [`SkillAutoInventor::generate_skill_draft`] + [`SkillAutoInventor::save_skill_draft`]。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info};

use super::protocol::{SkillManifest, SkillTransport};

// ---------------------------------------------------------------------------
// RingBuffer — 简单环形缓冲区
// ---------------------------------------------------------------------------

/// 简单环形缓冲区,基于 `VecDeque` 实现。
///
/// 当容量满时,`push` 会丢弃队首元素,保证内存占用恒定。
/// 用于存储最近 N 个操作记录(见 [`SkillAutoInventor::operation_history`])。
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    data: VecDeque<T>,
    capacity: usize,
}

impl<T: Clone> RingBuffer<T> {
    /// 构造一个指定容量的空缓冲区。`capacity = 0` 表示永远为空(边界情形)。
    pub fn new(capacity: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// 追加一个元素。若已满,丢弃队首元素。
    pub fn push(&mut self, item: T) {
        if self.capacity == 0 {
            return;
        }
        if self.data.len() >= self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(item);
    }

    /// 当前元素数量。
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// 容量。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 按时间顺序(旧 → 新)迭代。
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }

    /// 取出快照(克隆所有元素,按时间顺序)。
    pub fn to_vec(&self) -> Vec<T> {
        self.data.iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

/// SkillAutoInventor 配置。
///
/// 通过 [`SkillAutoInventor::new`] 注入。所有字段都有合理的默认值
/// (见 [`AutoInventorConfig::default`])。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoInventorConfig {
    /// 触发阈值:相同操作序列重复多少次才生成技能(默认 5)。
    pub pattern_threshold: usize,
    /// 历史缓冲区大小(默认 1000)。
    pub history_size: usize,
    /// 最小序列长度(默认 3 个操作才算模式)。
    pub min_pattern_length: usize,
    /// 最大序列长度(滑动窗口上界,默认 10)。
    pub max_pattern_length: usize,
    /// 自动发明的技能存储根路径。
    /// 默认 `~/.nebula/skills/auto-invented/`。
    pub skills_dir: PathBuf,
    /// 是否启用自动发明(运行时开关)。
    pub enabled: bool,
}

impl Default for AutoInventorConfig {
    fn default() -> Self {
        Self {
            pattern_threshold: 5,
            history_size: 1000,
            min_pattern_length: 3,
            max_pattern_length: 10,
            skills_dir: default_skills_dir(),
            enabled: true,
        }
    }
}

/// 计算默认的自动发明技能存储目录:`~/.nebula/skills/auto-invented/`。
///
/// 跨平台:Unix 用 `$HOME`,Windows 用 `$USERPROFILE`。两者都缺失时
/// 退化为相对路径 `./.nebula/skills/auto-invented`(测试环境下可接受)。
fn default_skills_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        PathBuf::from(home)
            .join(".nebula")
            .join("skills")
            .join("auto-invented")
    } else {
        PathBuf::from(".nebula/skills/auto-invented")
    }
}

// ---------------------------------------------------------------------------
// 操作记录
// ---------------------------------------------------------------------------

/// 操作记录:SkillAutoInventor 的最小观测单元。
///
/// 由 `SkillEngine` / `audit` 系统在每次操作后调用
/// [`SkillAutoInventor::record_operation`] 注入。
///
/// **隐私**:只存 `params_hash`(操作参数的 SHA256 摘要前 16 位),
/// 不存原始参数,避免泄露用户数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationRecord {
    /// 操作类型(如 `"file.read"` / `"llm.call"` / `"code.search"`)。
    pub op_type: String,
    /// 操作参数摘要(SHA256 前 16 位十六进制)。仅用于去重统计,不还原原始数据。
    pub params_hash: String,
    /// 操作时间戳(Unix 毫秒)。
    pub timestamp: u64,
    /// 操作结果(`true` = 成功,`false` = 失败)。
    pub success: bool,
    /// 会话 ID(用于跨会话模式检测时区分来源)。
    pub session_id: String,
}

impl OperationRecord {
    /// 从操作类型 + 原始参数构造一条记录。
    ///
    /// `params` 会被 SHA256 摘要后取前 16 位十六进制存入 `params_hash`,
    /// 原始参数不保留。`now_ms` 由调用方传入(便于测试固定时间)。
    pub fn new(
        op_type: impl Into<String>,
        params: &str,
        now_ms: u64,
        session_id: impl Into<String>,
        success: bool,
    ) -> Self {
        Self {
            op_type: op_type.into(),
            params_hash: hash_params(params),
            timestamp: now_ms,
            success,
            session_id: session_id.into(),
        }
    }
}

/// 计算参数的 SHA256 摘要前 16 位十六进制。
fn hash_params(params: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(params.as_bytes());
    let digest = hasher.finalize();
    hex_encode_short(&digest[..8])
}

/// 把 8 字节摘要编码为 16 位小写十六进制字符串。
fn hex_encode_short(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(16);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ---------------------------------------------------------------------------
// 检测到的重复模式
// ---------------------------------------------------------------------------

/// 检测到的重复模式。
///
/// 一个 [`DetectedPattern`] 表示"某个操作序列在历史中出现了 N 次"。
/// 当 `occurrence_count >= config.pattern_threshold` 时,系统会调用
/// [`SkillAutoInventor::generate_skill_draft`] 生成草稿。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedPattern {
    /// 模式 ID(操作序列的 SHA256 前 16 位十六进制)。
    pub pattern_id: String,
    /// 操作序列(如 `["file.read", "code.search", "llm.call"]`)。
    pub operations: Vec<String>,
    /// 出现次数。
    pub occurrence_count: usize,
    /// 首次出现时间(Unix 毫秒)。
    pub first_seen: u64,
    /// 最近出现时间(Unix 毫秒)。
    pub last_seen: u64,
    /// 生成的技能草稿(若已生成)。
    pub generated_skill: Option<SkillManifest>,
    /// 审核状态:`pending` / `accepted` / `rejected`。
    pub review_status: String,
}

// ---------------------------------------------------------------------------
// SkillAutoInventor
// ---------------------------------------------------------------------------

/// SkillAutoInventor —— 自动发明技能机制的核心引擎。
///
/// 线程安全:所有可变状态用 `Arc<RwLock<...>>` 包裹,方法均为 `&self`。
/// 不直接持有数据库连接 —— 草稿只写文件系统,持久化到 SQLite 由调用方
/// (如 `SkillImporter` 或 `SkillDiscoverer`)在用户接受后完成。
pub struct SkillAutoInventor {
    /// 操作历史缓冲区(环形缓冲区,存储最近 N 个操作)。
    operation_history: Arc<RwLock<RingBuffer<OperationRecord>>>,
    /// 已检测到的重复模式 ID 集合(避免重复生成)。
    detected_patterns: Arc<RwLock<HashSet<String>>>,
    /// 已检测到的模式详情(按 pattern_id 索引,供 list_patterns 查询)。
    patterns: Arc<RwLock<HashMap<String, DetectedPattern>>>,
    /// 配置(运行时可更新)。
    config: Arc<RwLock<AutoInventorConfig>>,
}

impl SkillAutoInventor {
    /// 构造一个新的自动发明器。
    ///
    /// 配置中的 `history_size` 决定环形缓冲区容量;`skills_dir` 决定
    /// 草稿写入位置。两者都可在运行时通过 [`Self::set_config`] 修改
    /// (但 `history_size` 修改后需要重建缓冲区,本实现采取"懒重建"
    /// 策略 —— 下一次 `record_operation` 不会立即 resize,只在容量
    /// 不一致时按新容量约束 push)。
    pub fn new(config: AutoInventorConfig) -> Self {
        let history_size = config.history_size;
        Self {
            operation_history: Arc::new(RwLock::new(RingBuffer::new(history_size))),
            detected_patterns: Arc::new(RwLock::new(HashSet::new())),
            patterns: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(config)),
        }
    }

    /// 使用默认配置构造。
    pub fn with_defaults() -> Self {
        Self::new(AutoInventorConfig::default())
    }

    /// 记录一个操作(由 SkillEngine / audit 系统调用)。
    ///
    /// 当 `config.enabled == false` 时直接返回(no-op),避免在禁用
    /// 期间累积数据。线程安全,不阻塞读取方。
    pub async fn record_operation(&self, op: OperationRecord) {
        let cfg = self.config.read().clone();
        if !cfg.enabled {
            return;
        }
        // 若 history_size 与缓冲区容量不一致(运行时改过配置),
        // 重建一个新容量的缓冲区,丢弃旧数据(简化实现,避免逐元素迁移)。
        {
            let mut hist = self.operation_history.write();
            if hist.capacity() != cfg.history_size {
                let mut new_buf = RingBuffer::new(cfg.history_size);
                // 尽量保留旧数据(取最后 N 条)。
                let old: Vec<OperationRecord> = hist.to_vec();
                let take = old.len().min(cfg.history_size);
                for item in old[old.len().saturating_sub(take)..].iter().cloned() {
                    new_buf.push(item);
                }
                *hist = new_buf;
            }
            hist.push(op);
        }
    }

    /// 分析历史,检测重复模式。
    ///
    /// 算法:
    /// 1. 取历史快照(克隆所有 `OperationRecord`)。
    /// 2. 提取 `op_type` 序列(忽略 `params_hash` / `timestamp` /
    ///    `session_id`,只看操作类型序列)。
    /// 3. 对长度 `min_pattern_length..=max_pattern_length` 的每个窗口,
    ///    计算序列的 SHA256,统计出现次数。
    /// 4. 出现次数 >= `pattern_threshold` 的序列构造为 [`DetectedPattern`]。
    /// 5. 已在 `detected_patterns` 集合中的模式不重复加入(但会更新
    ///    `occurrence_count` / `last_seen`)。
    ///
    /// 返回新增的模式列表(不包含历史已检测到的)。
    pub async fn detect_patterns(&self) -> Vec<DetectedPattern> {
        let cfg = self.config.read().clone();
        if !cfg.enabled {
            return Vec::new();
        }

        // 1) 取快照。
        let snapshot: Vec<OperationRecord> = self.operation_history.read().to_vec();
        if snapshot.len() < cfg.min_pattern_length {
            return Vec::new();
        }

        // 2) 提取 op_type 序列。
        let op_types: Vec<&str> = snapshot.iter().map(|r| r.op_type.as_str()).collect();

        // 3) 滑动窗口统计。
        // key = pattern_id, value = (count, first_idx, last_idx)
        let mut counts: HashMap<String, (usize, usize, usize)> = HashMap::new();
        // 同时保存 pattern_id -> operations 序列(避免重复计算)。
        let mut patterns_ops: HashMap<String, Vec<String>> = HashMap::new();

        let max_len = cfg.max_pattern_length.min(op_types.len());
        for window_len in cfg.min_pattern_length..=max_len {
            if window_len > op_types.len() {
                break;
            }
            for start in 0..=(op_types.len() - window_len) {
                let window: Vec<String> = op_types[start..start + window_len]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                let pid = hash_op_sequence(&window);
                let entry = counts.entry(pid.clone()).or_insert((0, start, start));
                entry.0 += 1;
                entry.2 = start;
                patterns_ops.entry(pid).or_insert(window);
            }
        }

        // 4) 筛选达到阈值的模式。
        let mut new_patterns: Vec<DetectedPattern> = Vec::new();
        let mut detected = self.detected_patterns.write();
        let mut patterns_map = self.patterns.write();

        for (pid, (count, first_idx, last_idx)) in &counts {
            if *count < cfg.pattern_threshold {
                continue;
            }
            let operations = patterns_ops.get(pid).cloned().unwrap_or_default();
            if operations.is_empty() {
                continue;
            }
            let first_seen = snapshot.get(*first_idx).map(|r| r.timestamp).unwrap_or(0);
            let last_seen = snapshot.get(*last_idx).map(|r| r.timestamp).unwrap_or(0);

            if detected.contains(pid) {
                // 已检测过:更新 count / last_seen,但不重复返回。
                if let Some(existing) = patterns_map.get_mut(pid) {
                    existing.occurrence_count = *count;
                    existing.last_seen = last_seen;
                }
                continue;
            }

            // 新模式。
            detected.insert(pid.clone());
            let pattern = DetectedPattern {
                pattern_id: pid.clone(),
                operations: operations.clone(),
                occurrence_count: *count,
                first_seen,
                last_seen,
                generated_skill: None,
                review_status: "pending".to_string(),
            };
            patterns_map.insert(pid.clone(), pattern.clone());
            new_patterns.push(pattern);
        }

        if !new_patterns.is_empty() {
            debug!(
                target: "nebula.skills.auto_inventor",
                new_count = new_patterns.len(),
                total_detected = detected.len(),
                "detected new repeating patterns"
            );
        }
        new_patterns
    }

    /// 为检测到的模式生成 SKILL.md 草稿(只构造 [`SkillManifest`],
    /// 不写文件)。
    ///
    /// 字段映射:
    /// * `name` = `auto-invented-<pattern_id 前 8 位>`
    /// * `version` = `"0.1.0"`(草稿版本)
    /// * `status` = `"draft"`
    /// * `description` = 根据操作类型自动生成
    /// * `capabilities` = 从 `op_type` 推断(如 `"file.read"` → `"file:read"`)
    /// * `transport` = `Local`
    /// * `min_nebula_version` = `"2.0.0"`
    /// * `trust_level` = `0`(安全红线,不写入 manifest,但写文件时
    ///   frontmatter 会带 `trust_level: 0`)
    ///
    /// 若模式不存在或已生成过草稿,返回错误。
    pub async fn generate_skill_draft(
        &self,
        pattern: &DetectedPattern,
    ) -> Result<SkillManifest, String> {
        if pattern.operations.is_empty() {
            return Err("cannot generate skill draft: empty operation sequence".to_string());
        }

        let name = format!(
            "auto-invented-{}",
            &pattern.pattern_id[..8.min(pattern.pattern_id.len())]
        );
        let description = generate_description(&pattern.operations);
        let capabilities = infer_capabilities(&pattern.operations);

        let manifest = SkillManifest {
            name,
            version: "0.1.0".to_string(),
            description,
            capabilities,
            transport: SkillTransport::Local,
            author: Some("SkillAutoInventor".to_string()),
            source: None,
            status: Some("draft".to_string()),
            dependencies: Vec::new(),
            eligibility: super::protocol::SkillEligibility::none(),
            min_nebula_version: Some("2.0.0".to_string()),
        };

        // 把草稿缓存到 patterns map 中(供后续 review 时取用)。
        let mut patterns_map = self.patterns.write();
        if let Some(existing) = patterns_map.get_mut(&pattern.pattern_id) {
            existing.generated_skill = Some(manifest.clone());
        }
        Ok(manifest)
    }

    /// 将技能草稿写入文件系统。
    ///
    /// 路径:`<config.skills_dir>/<skill-name>/SKILL.md`。
    /// 文件格式:YAML frontmatter + Markdown body,与 agentskills.io
    /// 规范一致(可被 [`super::importer::SkillImporter::from_skill_md`]
    /// 反向解析)。
    ///
    /// frontmatter 中显式带 `trust_level: 0`(安全红线 —— 用户须手动
    /// 提升后才能在沙箱外执行)。
    ///
    /// 返回写入的文件绝对路径。
    pub async fn save_skill_draft(
        &self,
        manifest: &SkillManifest,
        body: &str,
    ) -> Result<PathBuf, String> {
        let cfg = self.config.read().clone();
        let skill_dir = cfg.skills_dir.join(&manifest.name);
        std::fs::create_dir_all(&skill_dir)
            .map_err(|e| format!("failed to create skill dir {}: {e}", skill_dir.display()))?;

        let skill_md = serialize_skill_md(manifest, body);
        let skill_md_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_md_path, skill_md.as_bytes()).map_err(|e| {
            format!(
                "failed to write SKILL.md at {}: {e}",
                skill_md_path.display()
            )
        })?;

        info!(
            target: "nebula.skills.auto_inventor",
            name = %manifest.name,
            path = %skill_md_path.display(),
            "auto-invented skill draft saved"
        );
        Ok(skill_md_path)
    }

    /// 获取所有已检测到的模式(不触发新一轮检测)。
    pub async fn list_patterns(&self) -> Vec<DetectedPattern> {
        let patterns_map = self.patterns.read();
        let mut all: Vec<DetectedPattern> = patterns_map.values().cloned().collect();
        // 按首次出现时间升序,便于前端按时间线展示。
        all.sort_by_key(|p| p.first_seen);
        all
    }

    /// 用户审核:接受或拒绝自动发明的技能。
    ///
    /// * `accepted = true` —— 若已生成草稿,则写入文件系统并返回路径;
    ///   若未生成草稿,先调用 [`Self::generate_skill_draft`] 生成后再写入。
    ///   审核状态置为 `accepted`。
    /// * `accepted = false` —— 仅标记 `rejected`,不写文件。
    ///   已生成的草稿文件(若有)不会被删除(避免误删用户后续编辑)。
    ///
    /// 返回值:`accepted = true` 时返回 `Ok(Some(path))`;
    /// `accepted = false` 时返回 `Ok(None)`。
    pub async fn review_pattern(
        &self,
        pattern_id: &str,
        accepted: bool,
    ) -> Result<Option<PathBuf>, String> {
        // 取出当前 pattern 的快照(避免跨锁持有)。
        let pattern_snapshot = {
            let patterns_map = self.patterns.read();
            patterns_map
                .get(pattern_id)
                .cloned()
                .ok_or_else(|| format!("pattern not found: {pattern_id}"))?
        };

        if !accepted {
            let mut patterns_map = self.patterns.write();
            if let Some(existing) = patterns_map.get_mut(pattern_id) {
                existing.review_status = "rejected".to_string();
            }
            debug!(
                target: "nebula.skills.auto_inventor",
                pattern_id,
                "pattern rejected by user"
            );
            return Ok(None);
        }

        // accepted:确保有草稿。
        let manifest = match pattern_snapshot.generated_skill.clone() {
            Some(m) => m,
            None => {
                // 未生成草稿 —— 现场生成。
                self.generate_skill_draft(&pattern_snapshot).await?
            }
        };

        let body = generate_body(&manifest, &pattern_snapshot.operations);
        let path = self.save_skill_draft(&manifest, &body).await?;

        let mut patterns_map = self.patterns.write();
        if let Some(existing) = patterns_map.get_mut(pattern_id) {
            existing.review_status = "accepted".to_string();
            existing.generated_skill = Some(manifest);
        }
        Ok(Some(path))
    }

    /// 获取当前配置的快照。
    pub fn config(&self) -> AutoInventorConfig {
        self.config.read().clone()
    }

    /// 更新配置。
    ///
    /// * `enabled` —— `Some(b)` 设置启用状态;`None` 保持不变。
    /// * `threshold` —— `Some(n)` 设置 `pattern_threshold`(必须 >= 2,
    ///   否则返回错误);`None` 保持不变。
    ///
    /// `history_size` / `skills_dir` 不通过本方法修改(避免运行时
    /// 频繁迁移缓冲区);如需修改,请通过 [`Self::new`] 重建实例。
    pub fn set_config(
        &self,
        enabled: Option<bool>,
        threshold: Option<usize>,
    ) -> Result<(), String> {
        if let Some(t) = threshold {
            if t < 2 {
                return Err(format!(
                    "pattern_threshold must be >= 2 (got {t}); \
                     a threshold of 1 would match every single operation"
                ));
            }
        }
        let mut cfg = self.config.write();
        if let Some(b) = enabled {
            cfg.enabled = b;
        }
        if let Some(t) = threshold {
            cfg.pattern_threshold = t;
        }
        Ok(())
    }
}

impl Default for SkillAutoInventor {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 计算操作序列的 SHA256,取前 16 位十六进制作为 pattern_id。
fn hash_op_sequence(operations: &[String]) -> String {
    let mut hasher = Sha256::new();
    for op in operations {
        hasher.update(op.as_bytes());
        hasher.update(b"|");
    }
    let digest = hasher.finalize();
    hex_encode_short(&digest[..8])
}

/// 根据操作类型序列生成自然语言描述。
///
/// 例如 `["file.read", "code.search", "llm.call"]` 会生成:
/// `"Auto-invented skill: file.read → code.search → llm.call (repeated workflow)"`
fn generate_description(operations: &[String]) -> String {
    let joined = operations.join(" → ");
    format!("Auto-invented skill: {joined} (repeated workflow)")
}

/// 从操作类型推断能力标签。
///
/// 映射规则(与 [`super::importer::SkillImporter::parse_skill_md_inner`]
/// 中的 capability 解析对齐):
///
/// | op_type 前缀/包含 | capability |
/// |---|---|
/// | `file.read` / `file.read.*` | `file:read` |
/// | `file.write` / `file.write.*` | `file:write` |
/// | `network.*` / `http.*` | `network` |
/// | `subprocess.*` / `shell.*` | `subprocess` |
/// | `env.read` / `env.*` | `env:read` |
/// | `clipboard.read` / `clipboard.*` | `clipboard:read` |
/// | `llm.call` / `llm.*` | `llm:call` |
/// | `db.*` / `database.*` | `db:access` |
fn infer_capabilities(operations: &[String]) -> Vec<String> {
    let mut caps: Vec<String> = Vec::new();
    let push_unique = |caps: &mut Vec<String>, c: &str| {
        if !caps.iter().any(|x| x == c) {
            caps.push(c.to_string());
        }
    };
    for op in operations {
        let lower = op.to_ascii_lowercase();
        if lower.starts_with("file.read") {
            push_unique(&mut caps, "file:read");
        } else if lower.starts_with("file.write") {
            push_unique(&mut caps, "file:write");
        } else if lower.starts_with("network") || lower.starts_with("http") {
            push_unique(&mut caps, "network");
        } else if lower.starts_with("subprocess") || lower.starts_with("shell") {
            push_unique(&mut caps, "subprocess");
        } else if lower.starts_with("env") {
            push_unique(&mut caps, "env:read");
        } else if lower.starts_with("clipboard") {
            push_unique(&mut caps, "clipboard:read");
        } else if lower.starts_with("llm") {
            push_unique(&mut caps, "llm:call");
        } else if lower.starts_with("db") || lower.starts_with("database") {
            push_unique(&mut caps, "db:access");
        }
    }
    caps
}

/// 生成 SKILL.md body 内容(自动生成的使用说明)。
///
/// body 描述:
/// * 这个技能是从什么操作序列自动发明的
/// * 触发条件(操作序列重复出现)
/// * 用户审核提示(trust_level = 0)
fn generate_body(manifest: &SkillManifest, operations: &[String]) -> String {
    let steps = operations
        .iter()
        .enumerate()
        .map(|(i, op)| format!("{}. `{}`", i + 1, op))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# {name}\n\n\
         > **Auto-invented by SkillAutoInventor.** This skill was detected from a\n\
         > repeated operation sequence and is currently a **draft** (status: `draft`,\n\
         > `trust_level: 0`). Review the steps below and manually promote the\n\
         > `trust_level` only after you have verified the workflow is safe.\n\n\
         ## Detected workflow\n\n\
         The following operation sequence was observed repeating in the agent's\n\
         history (pattern id: `{pid}`):\n\n\
         {steps}\n\n\
         ## Description\n\n\
         {desc}\n\n\
         ## Safety\n\n\
         - `trust_level = 0` — user must manually promote before this skill can be\n\
           executed outside the sandbox.\n\
         - Review each step carefully. If any step touches user data or the\n\
           network, verify the parameters are bounded.\n",
        name = manifest.name,
        pid = manifest.name.trim_start_matches("auto-invented-"),
        steps = steps,
        desc = manifest.description,
    )
}

/// 把 [`SkillManifest`] + body 序列化为 SKILL.md 字符串。
///
/// frontmatter 字段顺序与 [`super::importer::SkillImporter::parse_skill_md_inner`]
/// 解析逻辑对齐,保证无损往返。`trust_level: 0` 显式写入(安全红线)。
fn serialize_skill_md(manifest: &SkillManifest, body: &str) -> String {
    let caps_yaml = if manifest.capabilities.is_empty() {
        "[]".to_string()
    } else {
        let items: Vec<String> = manifest
            .capabilities
            .iter()
            .map(|c| format!("\"{}\"", c))
            .collect();
        format!("[{}]", items.join(", "))
    };
    let author_yaml = manifest
        .author
        .as_ref()
        .map(|a| format!("\"{}\"", a))
        .unwrap_or_else(|| "null".to_string());
    let status_yaml = manifest
        .status
        .as_ref()
        .map(|s| format!("\"{}\"", s))
        .unwrap_or_else(|| "null".to_string());
    let min_ver_yaml = manifest
        .min_nebula_version
        .as_ref()
        .map(|v| format!("\"{}\"", v))
        .unwrap_or_else(|| "null".to_string());

    format!(
        "---\n\
         name: \"{name}\"\n\
         version: \"{version}\"\n\
         description: \"{desc}\"\n\
         capabilities: {caps}\n\
         transport: local\n\
         author: {author}\n\
         status: {status}\n\
         min_nebula_version: {min_ver}\n\
         trust_level: 0\n\
         ---\n\n\
         {body}",
        name = manifest.name,
        version = manifest.version,
        desc = manifest.description.replace('"', "\\\""),
        caps = caps_yaml,
        author = author_yaml,
        status = status_yaml,
        min_ver = min_ver_yaml,
        body = body,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ----- RingBuffer -----

    #[test]
    fn ring_buffer_push_and_len() {
        let mut buf: RingBuffer<i32> = RingBuffer::new(3);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        buf.push(1);
        buf.push(2);
        assert_eq!(buf.len(), 2);
        buf.push(3);
        assert_eq!(buf.len(), 3);
        // 容量满后再 push,丢弃队首。
        buf.push(4);
        assert_eq!(buf.len(), 3);
        let v = buf.to_vec();
        assert_eq!(v, vec![2, 3, 4]);
    }

    #[test]
    fn ring_buffer_iter_order() {
        let mut buf: RingBuffer<&str> = RingBuffer::new(5);
        for s in ["a", "b", "c", "d"] {
            buf.push(s);
        }
        let collected: Vec<&&str> = buf.iter().collect();
        assert_eq!(collected, vec![&"a", &"b", &"c", &"d"]);
    }

    #[test]
    fn ring_buffer_zero_capacity_is_noop() {
        let mut buf: RingBuffer<u8> = RingBuffer::new(0);
        buf.push(1);
        buf.push(2);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.capacity(), 0);
    }

    #[test]
    fn ring_buffer_rolls_over_when_full() {
        let mut buf: RingBuffer<u32> = RingBuffer::new(2);
        buf.push(10);
        buf.push(20);
        buf.push(30);
        buf.push(40);
        assert_eq!(buf.to_vec(), vec![30, 40]);
    }

    // ----- hash_params / hash_op_sequence -----

    #[test]
    fn hash_params_is_stable_and_short() {
        let h1 = hash_params("foo");
        let h2 = hash_params("foo");
        let h3 = hash_params("bar");
        assert_eq!(h1, h2, "same input must hash to same value");
        assert_ne!(h1, h3, "different input must hash differently");
        assert_eq!(h1.len(), 16, "params_hash should be 16 hex chars");
    }

    #[test]
    fn hash_op_sequence_distinguishes_order() {
        let a = hash_op_sequence(&["x".to_string(), "y".to_string()]);
        let b = hash_op_sequence(&["y".to_string(), "x".to_string()]);
        assert_ne!(a, b, "order matters for op sequence hash");
        assert_eq!(a.len(), 16);
    }

    // ----- infer_capabilities -----

    #[test]
    fn infer_capabilities_maps_known_op_types() {
        let ops = vec![
            "file.read".to_string(),
            "code.search".to_string(),
            "llm.call".to_string(),
            "network.http".to_string(),
        ];
        let caps = infer_capabilities(&ops);
        assert!(caps.iter().any(|c| c == "file:read"));
        assert!(caps.iter().any(|c| c == "llm:call"));
        assert!(caps.iter().any(|c| c == "network"));
        // code.search 不映射到任何 capability。
        assert_eq!(caps.len(), 3);
    }

    #[test]
    fn infer_capabilities_dedupes() {
        let ops = vec![
            "file.read".to_string(),
            "file.read.again".to_string(),
            "file.write".to_string(),
        ];
        let caps = infer_capabilities(&ops);
        // file:read 只出现一次(去重),file:write 一次。
        let read_count = caps.iter().filter(|c| *c == "file:read").count();
        assert_eq!(read_count, 1);
        assert!(caps.iter().any(|c| c == "file:write"));
    }

    // ----- AutoInventorConfig -----

    #[test]
    fn config_default_has_sane_values() {
        let cfg = AutoInventorConfig::default();
        assert_eq!(cfg.pattern_threshold, 5);
        assert_eq!(cfg.history_size, 1000);
        assert_eq!(cfg.min_pattern_length, 3);
        assert_eq!(cfg.max_pattern_length, 10);
        assert!(cfg.enabled);
        // skills_dir 应包含 "auto-invented"。
        assert!(cfg.skills_dir.to_string_lossy().contains("auto-invented"));
    }

    // ----- record_operation + detect_patterns -----

    #[tokio::test]
    async fn record_operation_disabled_is_noop() {
        let mut cfg = AutoInventorConfig::default();
        cfg.enabled = false;
        cfg.history_size = 10;
        let inv = SkillAutoInventor::new(cfg);
        inv.record_operation(OperationRecord::new("file.read", "p", 1, "s", true))
            .await;
        let snap = inv.operation_history.read().to_vec();
        assert!(snap.is_empty(), "disabled inventor should not record");
    }

    #[tokio::test]
    async fn detect_patterns_finds_repeating_sequence() {
        // 构造一个 3 操作序列,重复 5 次(达到默认阈值)。
        let mut cfg = AutoInventorConfig::default();
        cfg.history_size = 100;
        cfg.pattern_threshold = 5;
        cfg.min_pattern_length = 3;
        cfg.max_pattern_length = 3; // 只看长度 3 的窗口,简化测试
        let inv = SkillAutoInventor::new(cfg);

        let seq = ["file.read", "code.search", "llm.call"];
        // 重复 5 次:file.read → code.search → llm.call × 5
        // 共 15 条记录,滑动窗口(长度 3)在每次完整序列出现时各贡献 1 次命中。
        for i in 0..5 {
            for (j, op) in seq.iter().enumerate() {
                let ts = (i * 3 + j) as u64;
                inv.record_operation(OperationRecord::new(*op, "p", ts, "s1", true))
                    .await;
            }
        }

        let new = inv.detect_patterns().await;
        assert!(!new.is_empty(), "should detect the repeating 3-op sequence");
        // 至少有一个模式的 operations == seq。
        let hit = new
            .iter()
            .find(|p| p.operations == seq.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(hit.is_some(), "expected to find the exact 3-op sequence");
        let hit = hit.unwrap();
        assert!(hit.occurrence_count >= 5, "occurrence_count should be >= 5");
        assert_eq!(hit.review_status, "pending");
        assert!(hit.generated_skill.is_none());
    }

    #[tokio::test]
    async fn detect_patterns_skips_below_threshold() {
        let mut cfg = AutoInventorConfig::default();
        cfg.history_size = 50;
        cfg.pattern_threshold = 5;
        cfg.min_pattern_length = 3;
        cfg.max_pattern_length = 3;
        let inv = SkillAutoInventor::new(cfg);

        // 只重复 2 次(低于阈值 5)。
        let seq = ["a", "b", "c"];
        for _ in 0..2 {
            for op in &seq {
                inv.record_operation(OperationRecord::new(*op, "p", 0, "s", true))
                    .await;
            }
        }
        let new = inv.detect_patterns().await;
        assert!(
            new.is_empty(),
            "below-threshold sequence should not be detected"
        );
    }

    #[tokio::test]
    async fn detect_patterns_does_not_return_duplicates() {
        let mut cfg = AutoInventorConfig::default();
        cfg.history_size = 100;
        cfg.pattern_threshold = 3;
        cfg.min_pattern_length = 2;
        cfg.max_pattern_length = 2;
        let inv = SkillAutoInventor::new(cfg);

        // 重复 3 次 ["x", "y"]。
        for _ in 0..3 {
            inv.record_operation(OperationRecord::new("x", "p", 0, "s", true))
                .await;
            inv.record_operation(OperationRecord::new("y", "p", 0, "s", true))
                .await;
        }
        let first = inv.detect_patterns().await;
        assert!(!first.is_empty(), "first detection should find patterns");

        // 第二次检测:不应再返回已检测到的模式。
        let second = inv.detect_patterns().await;
        let dup = second
            .iter()
            .filter(|p| first.iter().any(|f| f.pattern_id == p.pattern_id))
            .count();
        assert_eq!(dup, 0, "second detection should not return duplicates");
    }

    #[tokio::test]
    async fn detect_patterns_respects_min_length() {
        let mut cfg = AutoInventorConfig::default();
        cfg.history_size = 100;
        cfg.pattern_threshold = 3;
        cfg.min_pattern_length = 4; // 要求至少 4 个操作
        cfg.max_pattern_length = 4;
        let inv = SkillAutoInventor::new(cfg);

        // 只重复 3 操作序列,但 min_pattern_length = 4,不应被检测到。
        for _ in 0..5 {
            for op in &["a", "b", "c"] {
                inv.record_operation(OperationRecord::new(*op, "p", 0, "s", true))
                    .await;
            }
        }
        let new = inv.detect_patterns().await;
        // 由于 min_pattern_length=4,3 操作序列不会被检测到;
        // 但跨序列的 4 操作窗口(如 c,a,b,c)可能重复出现。这里只验证
        // 没有长度为 3 的模式被返回。
        for p in &new {
            assert!(
                p.operations.len() >= 4,
                "no pattern shorter than min_pattern_length should be returned"
            );
        }
    }

    // ----- generate_skill_draft -----

    #[tokio::test]
    async fn generate_skill_draft_produces_correct_frontmatter() {
        let inv = SkillAutoInventor::with_defaults();
        let pattern = DetectedPattern {
            pattern_id: "abcdef0123456789".to_string(),
            operations: vec![
                "file.read".to_string(),
                "code.search".to_string(),
                "llm.call".to_string(),
            ],
            occurrence_count: 7,
            first_seen: 100,
            last_seen: 200,
            generated_skill: None,
            review_status: "pending".to_string(),
        };
        let m = inv
            .generate_skill_draft(&pattern)
            .await
            .expect("draft should generate");
        assert_eq!(m.name, "auto-invented-abcdef01");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.status.as_deref(), Some("draft"));
        assert_eq!(m.transport, SkillTransport::Local);
        assert_eq!(m.min_nebula_version.as_deref(), Some("2.0.0"));
        assert_eq!(m.author.as_deref(), Some("SkillAutoInventor"));
        // capabilities 应包含 file:read 和 llm:call。
        assert!(m.capabilities.iter().any(|c| c == "file:read"));
        assert!(m.capabilities.iter().any(|c| c == "llm:call"));
        // description 应包含所有操作类型。
        for op in &pattern.operations {
            assert!(
                m.description.contains(op),
                "description should mention {op}"
            );
        }
    }

    #[tokio::test]
    async fn generate_skill_draft_rejects_empty_operations() {
        let inv = SkillAutoInventor::with_defaults();
        let pattern = DetectedPattern {
            pattern_id: "deadbeefdeadbeef".to_string(),
            operations: vec![],
            occurrence_count: 5,
            first_seen: 0,
            last_seen: 0,
            generated_skill: None,
            review_status: "pending".to_string(),
        };
        let res = inv.generate_skill_draft(&pattern).await;
        assert!(res.is_err(), "empty operations should error");
        let err = res.unwrap_err();
        assert!(err.contains("empty operation sequence"));
    }

    // ----- save_skill_draft -----

    #[tokio::test]
    async fn save_skill_draft_writes_file_with_trust_level_zero() {
        // 用临时目录,避免污染用户家目录。
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut cfg = AutoInventorConfig::default();
        cfg.skills_dir = tmp.path().to_path_buf();
        let inv = SkillAutoInventor::new(cfg);

        let manifest = SkillManifest {
            name: "auto-invented-test1234".to_string(),
            version: "0.1.0".to_string(),
            description: "test description".to_string(),
            capabilities: vec!["file:read".to_string()],
            transport: SkillTransport::Local,
            author: Some("SkillAutoInventor".to_string()),
            source: None,
            status: Some("draft".to_string()),
            dependencies: Vec::new(),
            eligibility: super::super::protocol::SkillEligibility::none(),
            min_nebula_version: Some("2.0.0".to_string()),
        };
        let body = "# Test body\n";
        let path = inv
            .save_skill_draft(&manifest, body)
            .await
            .expect("save should succeed");
        assert!(path.is_absolute() || path.starts_with(tmp.path()));
        assert!(path.ends_with("SKILL.md"));
        let content = std::fs::read_to_string(&path).expect("read back");
        // 安全红线:trust_level 必须为 0。
        assert!(
            content.contains("trust_level: 0"),
            "draft must explicitly carry trust_level: 0; got: {content}"
        );
        assert!(content.contains("name: \"auto-invented-test1234\""));
        assert!(content.contains("version: \"0.1.0\""));
        assert!(content.contains("status: \"draft\""));
        assert!(content.contains("transport: local"));
        assert!(content.contains("# Test body"));
    }

    // ----- review_pattern -----

    #[tokio::test]
    async fn review_pattern_accept_writes_file_and_updates_status() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut cfg = AutoInventorConfig::default();
        cfg.skills_dir = tmp.path().to_path_buf();
        cfg.pattern_threshold = 3;
        cfg.min_pattern_length = 2;
        cfg.max_pattern_length = 2;
        let inv = SkillAutoInventor::new(cfg);

        // 重复 ["x", "y"] 3 次以触发检测。
        for _ in 0..3 {
            inv.record_operation(OperationRecord::new("x", "p", 0, "s", true))
                .await;
            inv.record_operation(OperationRecord::new("y", "p", 0, "s", true))
                .await;
        }
        let new_patterns = inv.detect_patterns().await;
        assert!(!new_patterns.is_empty());
        let pid = new_patterns[0].pattern_id.clone();

        // 接受模式 —— 应生成草稿 + 写文件 + 状态置 accepted。
        let res = inv.review_pattern(&pid, true).await;
        let path = res
            .expect("accept should succeed")
            .expect("should return a path");
        assert!(path.exists(), "draft file should exist after accept");

        let all = inv.list_patterns().await;
        let accepted = all
            .iter()
            .find(|p| p.pattern_id == pid)
            .expect("pattern should exist");
        assert_eq!(accepted.review_status, "accepted");
        assert!(accepted.generated_skill.is_some());
    }

    #[tokio::test]
    async fn review_pattern_reject_does_not_write_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut cfg = AutoInventorConfig::default();
        cfg.skills_dir = tmp.path().to_path_buf();
        cfg.pattern_threshold = 3;
        cfg.min_pattern_length = 2;
        cfg.max_pattern_length = 2;
        let inv = SkillAutoInventor::new(cfg);

        for _ in 0..3 {
            inv.record_operation(OperationRecord::new("a", "p", 0, "s", true))
                .await;
            inv.record_operation(OperationRecord::new("b", "p", 0, "s", true))
                .await;
        }
        let new_patterns = inv.detect_patterns().await;
        let pid = new_patterns[0].pattern_id.clone();

        let res = inv.review_pattern(&pid, false).await;
        assert!(res.is_ok(), "reject should not error");
        assert!(res.unwrap().is_none(), "reject should not return a path");

        // 目录下不应有任何 SKILL.md。
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.is_empty(), "no files should be written on reject");

        let all = inv.list_patterns().await;
        let rejected = all
            .iter()
            .find(|p| p.pattern_id == pid)
            .expect("pattern should exist");
        assert_eq!(rejected.review_status, "rejected");
    }

    #[tokio::test]
    async fn review_pattern_unknown_id_returns_error() {
        let inv = SkillAutoInventor::with_defaults();
        let res = inv.review_pattern("nonexistent-pattern-id", true).await;
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("pattern not found"));
    }

    // ----- set_config -----

    #[test]
    fn set_config_updates_enabled_and_threshold() {
        let inv = SkillAutoInventor::with_defaults();
        inv.set_config(Some(false), Some(7))
            .expect("set should succeed");
        let cfg = inv.config();
        assert!(!cfg.enabled);
        assert_eq!(cfg.pattern_threshold, 7);
    }

    #[test]
    fn set_config_rejects_threshold_below_two() {
        let inv = SkillAutoInventor::with_defaults();
        let res = inv.set_config(None, Some(1));
        assert!(res.is_err(), "threshold=1 should be rejected");
        let res = inv.set_config(None, Some(0));
        assert!(res.is_err(), "threshold=0 should be rejected");
    }

    #[test]
    fn set_config_none_keeps_existing_values() {
        let inv = SkillAutoInventor::with_defaults();
        let before = inv.config();
        inv.set_config(None, None).expect("set should succeed");
        let after = inv.config();
        assert_eq!(before.enabled, after.enabled);
        assert_eq!(before.pattern_threshold, after.pattern_threshold);
    }

    // ----- list_patterns -----

    #[tokio::test]
    async fn list_patterns_returns_all_in_time_order() {
        let mut cfg = AutoInventorConfig::default();
        cfg.history_size = 100;
        cfg.pattern_threshold = 2;
        cfg.min_pattern_length = 2;
        cfg.max_pattern_length = 2;
        let inv = SkillAutoInventor::new(cfg);

        // 序列 1: ["a","b"] × 2 (timestamp 10, 20)
        for ts in [10u64, 20] {
            inv.record_operation(OperationRecord::new("a", "p", ts, "s", true))
                .await;
            inv.record_operation(OperationRecord::new("b", "p", ts, "s", true))
                .await;
        }
        // 序列 2: ["c","d"] × 2 (timestamp 30, 40)
        for ts in [30u64, 40] {
            inv.record_operation(OperationRecord::new("c", "p", ts, "s", true))
                .await;
            inv.record_operation(OperationRecord::new("d", "p", ts, "s", true))
                .await;
        }
        let _ = inv.detect_patterns().await;
        let all = inv.list_patterns().await;
        assert_eq!(all.len(), 2, "should have 2 patterns");
        // 按 first_seen 升序。
        assert!(all[0].first_seen <= all[1].first_seen);
    }

    // ----- serialize_skill_md round-trip -----

    #[test]
    fn serialize_skill_md_round_trips_through_importer_parser() {
        // 验证:serialize_skill_md 生成的 SKILL.md 可被
        // SkillImporter::from_skill_md 正确解析(字段无损往返)。
        let manifest = SkillManifest {
            name: "auto-invented-roundtrip".to_string(),
            version: "0.1.0".to_string(),
            description: "round trip test".to_string(),
            capabilities: vec!["file:read".to_string(), "llm:call".to_string()],
            transport: SkillTransport::Local,
            author: Some("SkillAutoInventor".to_string()),
            source: None,
            status: Some("draft".to_string()),
            dependencies: Vec::new(),
            eligibility: super::super::protocol::SkillEligibility::none(),
            min_nebula_version: Some("2.0.0".to_string()),
        };
        let body = "# Round trip\nbody text\n";
        let md = serialize_skill_md(&manifest, body);

        // 用 SkillSpecValidator 解析(协议层,不依赖 store)。
        let report = super::super::protocol::SkillSpecValidator::validate_skill_md(&md);
        assert!(
            report.errors.is_empty(),
            "expected no validation errors, got: {:?}",
            report.errors
        );
        let parsed = report.manifest.expect("manifest should parse");
        assert_eq!(parsed.name, manifest.name);
        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.description, manifest.description);
        assert_eq!(parsed.capabilities, manifest.capabilities);
        assert_eq!(parsed.transport, SkillTransport::Local);
        assert_eq!(parsed.status, manifest.status);
        assert_eq!(parsed.min_nebula_version, manifest.min_nebula_version);
    }

    // ----- path safety -----

    #[test]
    fn default_skills_dir_is_under_nebula() {
        let dir = default_skills_dir();
        let s = dir.to_string_lossy().to_string();
        assert!(
            s.contains(".nebula"),
            "skills_dir should be under .nebula: {s}"
        );
        assert!(
            s.contains("auto-invented"),
            "skills_dir should be in auto-invented: {s}"
        );
    }

    #[test]
    fn default_skills_dir_fallback_when_no_home() {
        // 此测试只在两个环境变量都缺失时验证回退路径。
        // 由于无法可靠地清除环境变量(其他测试可能依赖),这里仅验证
        // 函数能正常调用并返回一个非空路径。
        let dir: PathBuf = default_skills_dir();
        assert!(!dir.as_os_str().is_empty());
    }
}
