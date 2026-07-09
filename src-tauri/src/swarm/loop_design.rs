//! T-E-L-08b: Loop 设计节点 — 从自然语言描述生成 LOOP.md。
//!
//! 作为 Loop Engineering 的设计入口, [`LoopDesigner`] 接收自然语言描述,
//! 启发式提取触发条件 / 步骤 / 预算 / 自主度, 组装成 [`LoopDesign`],
//! 再由 [`LoopDesigner::generate_loop_md`] 渲染为符合 [`LoopDef`] 解析格式的
//! LOOP.md 文件 (YAML frontmatter + Markdown body)。
//!
//! ## 设计流程
//!
//! ```text
//! 自然语言描述
//!      │
//!      ▼
//! extract_trigger ──► TriggerSpec
//! extract_steps ────► Vec<StepSpec>
//! extract_budget ───► BudgetSpec
//! extract_autonomy ─► AutonomyLevel
//!      │
//!      ▼
//! LoopDesign ── validate_design ──► generate_loop_md ──► LOOP.md
//! ```
//!
//! ## 预置模板
//!
//! [`LoopDesignLibrary`] 提供 5 个开箱即用模板: 日报生成 / 代码审查 /
//! 记忆整理 / 趋势监控 / 定时备份, 用户可基于模板二次定制。
//!
//! ## Feature Gate
//!
//! 与 `loop_def.rs` 一致, 由 `master-orchestrator` feature 门控。

#![cfg(feature = "master-orchestrator")]

use anyhow::{bail, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

// 复用 loop_def 中已定义的 AutonomyLevel (L0-L5), 避免重复定义造成体系分裂。
use super::loop_def::AutonomyLevel;

// ---------------------------------------------------------------------------
// TriggerSpec — 触发规格
// ---------------------------------------------------------------------------

/// Loop 触发条件规格。
///
/// 对应 LOOP.md frontmatter 的 `cadence` 字段; 在 [`LoopDesigner::generate_loop_md`]
/// 中渲染为 cron 字符串或自定义标记。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerSpec {
    /// cron 表达式 (如 `"0 9 * * 1-5"`)。
    Cron(String),
    /// 固定间隔 (秒)。
    Interval(u64),
    /// 事件触发 (事件名称)。
    Event(String),
    /// 手动触发。
    Manual,
}

impl TriggerSpec {
    /// 渲染为 LOOP.md `cadence` 字段字符串。
    ///
    /// - `Cron(s)` → 原样返回
    /// - `Interval(secs)` → 转换为简化 cron:
    ///   - 60s → `"* * * * *"` (每分钟)
    ///   - 3600s → `"0 * * * *"` (每小时)
    ///   - 86400s → `"0 0 * * *"` (每天)
    ///   - 其他 → `"<secs>s"` 自定义标记 (T-E-L-02 解析器负责)
    /// - `Event(name)` → `"event:<name>"`
    /// - `Manual` → `"manual"`
    pub fn to_cadence(&self) -> String {
        match self {
            TriggerSpec::Cron(s) => s.clone(),
            TriggerSpec::Interval(secs) => match *secs {
                60 => "* * * * *".to_string(),
                3600 => "0 * * * *".to_string(),
                86400 => "0 0 * * *".to_string(),
                n => format!("{n}s"),
            },
            TriggerSpec::Event(name) => format!("event:{name}"),
            TriggerSpec::Manual => "manual".to_string(),
        }
    }

    /// 人类可读描述 (写入 LOOP.md `## Context` 帮助读者理解触发方式)。
    pub fn human_description(&self) -> String {
        match self {
            TriggerSpec::Cron(s) => format!("按 cron 表达式 `{s}` 定时触发"),
            TriggerSpec::Interval(secs) => {
                if *secs >= 86400 && *secs % 86400 == 0 {
                    format!("每 {} 天触发一次", secs / 86400)
                } else if *secs >= 3600 && *secs % 3600 == 0 {
                    format!("每 {} 小时触发一次", secs / 3600)
                } else if *secs >= 60 && *secs % 60 == 0 {
                    format!("每 {} 分钟触发一次", secs / 60)
                } else {
                    format!("每 {secs} 秒触发一次")
                }
            }
            TriggerSpec::Event(name) => format!("事件 `{name}` 触发"),
            TriggerSpec::Manual => "用户手动触发".to_string(),
        }
    }
}

impl Default for TriggerSpec {
    fn default() -> Self {
        TriggerSpec::Manual
    }
}

// ---------------------------------------------------------------------------
// StepActionType — 步骤动作类型
// ---------------------------------------------------------------------------

/// Loop 步骤的动作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepActionType {
    /// LLM 调用 (生成文本 / 摘要 / 分类等)。
    LlmCall,
    /// 工具调用 (文件读写 / shell / git 等)。
    ToolCall,
    /// 记忆查询 (向量检索 / STATE.md 读取)。
    MemoryQuery,
    /// 外部 API 调用 (GitHub / Slack / 自建服务)。
    ExternalApi,
    /// 条件分支 (根据上游输出选择路径)。
    Conditional,
}

impl StepActionType {
    pub fn as_str(self) -> &'static str {
        match self {
            StepActionType::LlmCall => "llm_call",
            StepActionType::ToolCall => "tool_call",
            StepActionType::MemoryQuery => "memory_query",
            StepActionType::ExternalApi => "external_api",
            StepActionType::Conditional => "conditional",
        }
    }
}

impl Default for StepActionType {
    fn default() -> Self {
        StepActionType::LlmCall
    }
}

// ---------------------------------------------------------------------------
// StepSpec — 步骤规格
// ---------------------------------------------------------------------------

/// Loop 步骤规格 (设计态, 未实例化为 LongTask)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSpec {
    /// 步骤名称 (唯一标识, 用于日志与 provenance)。
    pub name: String,
    /// 动作类型。
    pub action_type: StepActionType,
    /// 提示词模板 (可含 `{{prev_output}}` 等占位符)。
    pub prompt: String,
    /// 期望输出描述 (供 Checker Agent 校验)。
    pub expected_output: String,
    /// 重试次数 (失败后重试上限, 0 = 不重试)。
    pub retry_count: u32,
}

impl StepSpec {
    /// 创建一个简单 LlmCall 步骤。
    pub fn llm(name: &str, prompt: &str) -> Self {
        Self {
            name: name.to_string(),
            action_type: StepActionType::LlmCall,
            prompt: prompt.to_string(),
            expected_output: String::new(),
            retry_count: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// BudgetSpec — 预算规格
// ---------------------------------------------------------------------------

/// Loop 预算规格 (设计态)。
///
/// 与 [`crate::swarm::loop_budget::LoopBudgetConfig`] 的月度维度对齐,
/// 同时补充单次执行预算供 [`crate::swarm::master::MasterOrchestrator::execute_loop`] 门禁。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSpec {
    /// 月度美元上限 (0.0 = 不限制)。
    pub monthly_usd: f64,
    /// 月度 Token 上限 (0 = 不限制)。
    pub monthly_tokens: u64,
    /// 单次执行美元预算 (0.0 = 不限制)。
    pub per_run_usd: f64,
}

impl Default for BudgetSpec {
    fn default() -> Self {
        Self {
            monthly_usd: 10.0,
            monthly_tokens: 1_000_000,
            per_run_usd: 0.5,
        }
    }
}

impl BudgetSpec {
    /// 估算单次 Token 预算 (月度均摊到每天, 再按典型 Loop 一天 1-2 次估算)。
    ///
    /// 优先使用 `monthly_tokens / 30`; 若为 0 则返回保守默认 50000。
    pub fn estimated_per_run_tokens(&self) -> u64 {
        if self.monthly_tokens == 0 {
            50_000
        } else {
            (self.monthly_tokens / 30).max(10_000)
        }
    }

    /// 估算单次时间预算 (分钟), 基于 per_run_usd 粗略换算 (1 USD ≈ 10 分钟云端)。
    pub fn estimated_per_run_minutes(&self) -> u32 {
        if self.per_run_usd <= 0.0 {
            10
        } else {
            (self.per_run_usd * 10.0).round() as u32
        }
    }
}

// ---------------------------------------------------------------------------
// LoopDesign — 完整 Loop 设计
// ---------------------------------------------------------------------------

/// Loop 设计 (从自然语言或模板生成, 渲染为 LOOP.md)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDesign {
    /// Loop 名称 (唯一标识, kebab-case)。
    pub name: String,
    /// Loop 描述 (人类可读)。
    pub description: String,
    /// 触发条件。
    pub trigger: TriggerSpec,
    /// 步骤列表 (至少 1 条)。
    pub steps: Vec<StepSpec>,
    /// 预算规格。
    pub budget: BudgetSpec,
    /// 自主度等级 L0-L5。
    pub autonomy: AutonomyLevel,
    /// 自由元数据 (模板来源 / 标签 / 创建时间等)。
    pub metadata: serde_json::Value,
}

impl LoopDesign {
    /// 从设计中提取 Intent 段落 (优先 metadata.intent, 退化到 description)。
    pub fn intent(&self) -> String {
        if let Some(intent) = self.metadata.get("intent").and_then(|v| v.as_str()) {
            return intent.to_string();
        }
        self.description.clone()
    }

    /// 提取 Context 条目 (触发说明 + 各步骤的输入来源)。
    pub fn context_lines(&self) -> Vec<String> {
        let mut lines = vec![self.trigger.human_description()];
        for step in &self.steps {
            lines.push(format!("步骤 `{}` 输入: {}", step.name, step.prompt));
        }
        lines
    }

    /// 提取 Action 条目 (步骤名称 + 动作类型)。
    pub fn action_lines(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|s| format!("[{}] {}", s.action_type.as_str(), s.name))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// DesignTemplate — 预置设计模板
// ---------------------------------------------------------------------------

/// Loop 设计模板 (预置最佳实践)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignTemplate {
    /// 模板标识 (kebab-case)。
    pub id: String,
    /// 模板显示名称。
    pub title: String,
    /// 模板描述。
    pub description: String,
    /// 模板标签 (用于检索)。
    pub tags: Vec<String>,
    /// 模板对应的 Loop 设计 (用户可基于此二次定制)。
    pub design: LoopDesign,
}

// ---------------------------------------------------------------------------
// LoopDesignLibrary — 预置设计模板库
// ---------------------------------------------------------------------------

/// 预置 Loop 设计模板库。
///
/// 提供 5 个开箱即用的模板, 覆盖常见 Loop 场景。用户可通过 [`Self::find`]
/// 按 id 检索, 或通过 [`Self::list`] 列举全部。
pub struct LoopDesignLibrary;

impl LoopDesignLibrary {
    /// 列举全部预置模板。
    pub fn list() -> Vec<DesignTemplate> {
        vec![
            Self::daily_report_template(),
            Self::code_review_template(),
            Self::memory_tidy_template(),
            Self::trend_monitor_template(),
            Self::scheduled_backup_template(),
        ]
    }

    /// 按 id 检索模板。
    pub fn find(id: &str) -> Option<DesignTemplate> {
        Self::list().into_iter().find(|t| t.id == id)
    }

    /// 按标签检索模板 (任一标签匹配即返回)。
    pub fn find_by_tag(tag: &str) -> Vec<DesignTemplate> {
        Self::list()
            .into_iter()
            .filter(|t| t.tags.iter().any(|t2| t2 == tag))
            .collect()
    }

    // ---- 模板 1: 日报生成 ----
    fn daily_report_template() -> DesignTemplate {
        let design = LoopDesign {
            name: "daily-report".to_string(),
            description: "每个工作日 18:00 汇总当日 git 提交 / Issue / CI 状态, 生成日报 Markdown".to_string(),
            trigger: TriggerSpec::Cron("0 18 * * 1-5".to_string()),
            steps: vec![
                StepSpec {
                    name: "collect-activity".to_string(),
                    action_type: StepActionType::ToolCall,
                    prompt: "调用 git log --since='1 day ago' 与 GitHub API 拉取当日 issue/PR/CI 状态".to_string(),
                    expected_output: "结构化的当日活动列表 JSON".to_string(),
                    retry_count: 2,
                },
                StepSpec {
                    name: "draft-report".to_string(),
                    action_type: StepActionType::LlmCall,
                    prompt: "基于 collect-activity 输出, 撰写日报 Markdown, 包含: 完成项 / 进行中 / 阻塞 / 明日计划".to_string(),
                    expected_output: "日报 Markdown 文档".to_string(),
                    retry_count: 1,
                },
                StepSpec {
                    name: "save-report".to_string(),
                    action_type: StepActionType::ToolCall,
                    prompt: "将日报写入 reports/YYYY-MM-DD.md".to_string(),
                    expected_output: "文件路径".to_string(),
                    retry_count: 2,
                },
            ],
            budget: BudgetSpec {
                monthly_usd: 5.0,
                monthly_tokens: 500_000,
                per_run_usd: 0.2,
            },
            autonomy: AutonomyLevel::L2,
            metadata: serde_json::json!({
                "intent": "工作日下班前自动生成当日工作日报, 沉淀进度并暴露阻塞",
                "source": "preset-template",
                "category": "reporting",
            }),
        };
        DesignTemplate {
            id: "daily-report".to_string(),
            title: "日报生成".to_string(),
            description: "工作日 18:00 自动汇总 git/Issue/CI 并生成日报".to_string(),
            tags: vec!["reporting".to_string(), "daily".to_string()],
            design,
        }
    }

    // ---- 模板 2: 代码审查 ----
    fn code_review_template() -> DesignTemplate {
        let design = LoopDesign {
            name: "code-review".to_string(),
            description: "PR 打开事件触发, 自动审查变更并起草评审意见".to_string(),
            trigger: TriggerSpec::Event("pr_opened".to_string()),
            steps: vec![
                StepSpec {
                    name: "fetch-diff".to_string(),
                    action_type: StepActionType::ExternalApi,
                    prompt: "调用 GitHub API 拉取 PR diff 与元数据".to_string(),
                    expected_output: "PR diff + 元数据 JSON".to_string(),
                    retry_count: 3,
                },
                StepSpec {
                    name: "analyze".to_string(),
                    action_type: StepActionType::LlmCall,
                    prompt: "分析 diff, 识别: 风险点 / 风格问题 / 测试覆盖缺口, 输出结构化评审意见"
                        .to_string(),
                    expected_output: "结构化评审意见 (risk/style/test 三类)".to_string(),
                    retry_count: 1,
                },
                StepSpec {
                    name: "post-comment".to_string(),
                    action_type: StepActionType::ExternalApi,
                    prompt: "将评审意见作为 PR comment 发布".to_string(),
                    expected_output: "comment URL".to_string(),
                    retry_count: 2,
                },
            ],
            budget: BudgetSpec {
                monthly_usd: 20.0,
                monthly_tokens: 2_000_000,
                per_run_usd: 0.5,
            },
            autonomy: AutonomyLevel::L3,
            metadata: serde_json::json!({
                "intent": "PR 打开后自动审查, 在人工 review 前提供风险预判",
                "source": "preset-template",
                "category": "quality",
            }),
        };
        DesignTemplate {
            id: "code-review".to_string(),
            title: "代码审查".to_string(),
            description: "PR 打开事件触发, 自动审查并起草评审意见".to_string(),
            tags: vec!["quality".to_string(), "code-review".to_string()],
            design,
        }
    }

    // ---- 模板 3: 记忆整理 ----
    fn memory_tidy_template() -> DesignTemplate {
        let design = LoopDesign {
            name: "memory-tidy".to_string(),
            description: "每天凌晨整理记忆库: 去重 / 衰减旧条目 / 提炼摘要".to_string(),
            trigger: TriggerSpec::Interval(86400),
            steps: vec![
                StepSpec {
                    name: "scan-memories".to_string(),
                    action_type: StepActionType::MemoryQuery,
                    prompt: "扫描近 7 天写入的记忆条目, 按相似度聚类".to_string(),
                    expected_output: "聚类后的记忆簇列表".to_string(),
                    retry_count: 1,
                },
                StepSpec {
                    name: "dedupe".to_string(),
                    action_type: StepActionType::Conditional,
                    prompt: "对每簇相似度 > 0.9 的条目, 保留最新一条并合并内容".to_string(),
                    expected_output: "去重后的记忆条目数".to_string(),
                    retry_count: 1,
                },
                StepSpec {
                    name: "summarize".to_string(),
                    action_type: StepActionType::LlmCall,
                    prompt: "对每簇生成一句话摘要, 写回 metadata.summary 字段".to_string(),
                    expected_output: "摘要列表".to_string(),
                    retry_count: 1,
                },
            ],
            budget: BudgetSpec {
                monthly_usd: 3.0,
                monthly_tokens: 300_000,
                per_run_usd: 0.1,
            },
            autonomy: AutonomyLevel::L2,
            metadata: serde_json::json!({
                "intent": "定期整理记忆库, 控制膨胀并提升检索精度",
                "source": "preset-template",
                "category": "memory",
            }),
        };
        DesignTemplate {
            id: "memory-tidy".to_string(),
            title: "记忆整理".to_string(),
            description: "每天整理记忆库: 去重 / 衰减 / 提炼摘要".to_string(),
            tags: vec!["memory".to_string(), "maintenance".to_string()],
            design,
        }
    }

    // ---- 模板 4: 趋势监控 ----
    fn trend_monitor_template() -> DesignTemplate {
        let design = LoopDesign {
            name: "trend-monitor".to_string(),
            description: "每小时抓取关键指标, 检测异常趋势并告警".to_string(),
            trigger: TriggerSpec::Interval(3600),
            steps: vec![
                StepSpec {
                    name: "collect-metrics".to_string(),
                    action_type: StepActionType::ExternalApi,
                    prompt: "拉取系统指标 (CPU/内存/磁盘/QPS/错误率)".to_string(),
                    expected_output: "时序指标快照 JSON".to_string(),
                    retry_count: 3,
                },
                StepSpec {
                    name: "detect-anomaly".to_string(),
                    action_type: StepActionType::Conditional,
                    prompt: "对比历史基线, 若偏离 > 2σ 则标记为异常".to_string(),
                    expected_output: "异常项列表 (空 = 正常)".to_string(),
                    retry_count: 1,
                },
                StepSpec {
                    name: "alert".to_string(),
                    action_type: StepActionType::ToolCall,
                    prompt: "若存在异常, 通过 IM 渠道发送告警".to_string(),
                    expected_output: "告警发送结果".to_string(),
                    retry_count: 2,
                },
            ],
            budget: BudgetSpec {
                monthly_usd: 8.0,
                monthly_tokens: 800_000,
                per_run_usd: 0.05,
            },
            autonomy: AutonomyLevel::L1,
            metadata: serde_json::json!({
                "intent": "高频监控关键指标, 异常时主动告警而非被动发现",
                "source": "preset-template",
                "category": "observability",
            }),
        };
        DesignTemplate {
            id: "trend-monitor".to_string(),
            title: "趋势监控".to_string(),
            description: "每小时抓取指标并检测异常趋势, 异常时告警".to_string(),
            tags: vec!["observability".to_string(), "monitoring".to_string()],
            design,
        }
    }

    // ---- 模板 5: 定时备份 ----
    fn scheduled_backup_template() -> DesignTemplate {
        let design = LoopDesign {
            name: "scheduled-backup".to_string(),
            description: "每天凌晨 2:00 备份关键数据到外部存储".to_string(),
            trigger: TriggerSpec::Cron("0 2 * * *".to_string()),
            steps: vec![
                StepSpec {
                    name: "snapshot".to_string(),
                    action_type: StepActionType::ToolCall,
                    prompt: "对 SQLite + 向量库做一致性快照".to_string(),
                    expected_output: "快照文件路径".to_string(),
                    retry_count: 2,
                },
                StepSpec {
                    name: "upload".to_string(),
                    action_type: StepActionType::ExternalApi,
                    prompt: "上传快照到 S3 / WebDAV, 保留最近 30 天".to_string(),
                    expected_output: "上传结果 + 远端对象 key".to_string(),
                    retry_count: 3,
                },
                StepSpec {
                    name: "verify".to_string(),
                    action_type: StepActionType::Conditional,
                    prompt: "校验远端对象 ETag 与本地一致".to_string(),
                    expected_output: "校验通过 / 失败".to_string(),
                    retry_count: 1,
                },
            ],
            budget: BudgetSpec {
                monthly_usd: 2.0,
                monthly_tokens: 0,
                per_run_usd: 0.0,
            },
            autonomy: AutonomyLevel::L4,
            metadata: serde_json::json!({
                "intent": "每日离站备份, 确保灾难恢复 RPO <= 24h",
                "source": "preset-template",
                "category": "backup",
            }),
        };
        DesignTemplate {
            id: "scheduled-backup".to_string(),
            title: "定时备份".to_string(),
            description: "每天凌晨备份关键数据到外部存储并校验".to_string(),
            tags: vec!["backup".to_string(), "disaster-recovery".to_string()],
            design,
        }
    }
}

// ---------------------------------------------------------------------------
// LoopDesigner — 主设计器
// ---------------------------------------------------------------------------

/// Loop 设计器 — 从自然语言描述生成 [`LoopDesign`] 与 LOOP.md。
///
/// 使用启发式规则提取关键字段 (无 LLM 调用, 纯本地解析),
/// 适合作为设计入口快速起草, 后续可由用户手工精修。
pub struct LoopDesigner {
    /// 默认自主度 (无法从描述提取时使用)。
    pub default_autonomy: AutonomyLevel,
    /// 默认预算 (描述中未提及时使用)。
    pub default_budget: BudgetSpec,
}

impl Default for LoopDesigner {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopDesigner {
    /// 创建默认配置的设计器。
    pub fn new() -> Self {
        Self {
            default_autonomy: AutonomyLevel::L2,
            default_budget: BudgetSpec::default(),
        }
    }

    /// 主入口 — 从自然语言描述生成完整 Loop 设计。
    ///
    /// 步骤:
    /// 1. 提取触发条件 / 步骤 / 预算 / 自主度
    /// 2. 推导 name (从描述首句)
    /// 3. 组装 [`LoopDesign`] 并 validate
    pub fn design_from_natural_language(&self, description: &str) -> Result<LoopDesign> {
        if description.trim().is_empty() {
            bail!("description must not be empty");
        }

        let trigger = self.extract_trigger(description)?;
        let steps = self.extract_steps(description)?;
        let budget = self.extract_budget(description)?;
        let autonomy = self.extract_autonomy(description)?;

        let name = derive_name(description);
        let design = LoopDesign {
            name,
            description: description.trim().to_string(),
            trigger,
            steps,
            budget,
            autonomy,
            metadata: serde_json::json!({
                "intent": description.trim(),
                "source": "natural-language",
                "created_at": Utc::now().to_rfc3339(),
            }),
        };

        self.validate_design(&design)?;
        Ok(design)
    }

    /// 提取触发条件。
    ///
    /// 关键词识别 (大小写不敏感):
    /// - "每天" / "每日" / "daily" / "工作日" → 工作日 cron
    /// - "每小时" / "hourly" → 每小时 cron
    /// - "每周" / "weekly" → 每周 cron
    /// - "每月" / "monthly" → 每月 cron
    /// - "每 N 秒/分钟/小时/天" → Interval
    /// - "事件" / "event" / "当...时" / "触发" → Event
    /// - 缺失 → Manual
    pub fn extract_trigger(&self, description: &str) -> Result<TriggerSpec> {
        let lower = description.to_lowercase();
        let trimmed = description.trim();

        // 1. 显式 cron (含 `cron:` 前缀或 5 段空格分隔的 cron 字符)
        if let Some(cron) = extract_explicit_cron(description) {
            return Ok(TriggerSpec::Cron(cron));
        }

        // 2. "每 N <单位>" 间隔
        if let Some(secs) = extract_interval(&lower) {
            return Ok(TriggerSpec::Interval(secs));
        }

        // 3. 固定频率关键词
        if contains_any(&lower, &["工作日", "weekday"])
            || lower.contains("每天")
            || lower.contains("每日")
            || lower.contains("daily")
        {
            return Ok(TriggerSpec::Cron("0 9 * * 1-5".to_string()));
        }
        if lower.contains("每小时") || lower.contains("hourly") {
            return Ok(TriggerSpec::Cron("0 * * * *".to_string()));
        }
        if lower.contains("每周") || lower.contains("weekly") {
            return Ok(TriggerSpec::Cron("0 9 * * 1".to_string()));
        }
        if lower.contains("每月") || lower.contains("monthly") {
            return Ok(TriggerSpec::Cron("0 9 1 * *".to_string()));
        }

        // 4. 事件触发
        if lower.contains("事件")
            || lower.contains("event")
            || (trimmed.contains("当") && trimmed.contains("时"))
            || lower.contains("触发")
        {
            // 尝试提取事件名 (当 X 时 → event:X)
            if let Some(name) = extract_event_name(description) {
                return Ok(TriggerSpec::Event(name));
            }
            return Ok(TriggerSpec::Event("default".to_string()));
        }

        // 5. 默认手动
        Ok(TriggerSpec::Manual)
    }

    /// 提取步骤列表。
    ///
    /// 识别策略 (按优先级):
    /// 1. 显式 "步骤 N:" / "step N:" / "N. " 编号列表
    /// 2. 中文 "首先 / 然后 / 接着 / 最后" 等连接词切分
    /// 3. 整段作为单步骤
    pub fn extract_steps(&self, description: &str) -> Result<Vec<StepSpec>> {
        let numbered = extract_numbered_steps(description);
        if !numbered.is_empty() {
            return Ok(numbered
                .into_iter()
                .enumerate()
                .map(|(i, text)| StepSpec {
                    name: format!("step-{}", i + 1),
                    action_type: infer_action_type(&text),
                    prompt: text,
                    expected_output: String::new(),
                    retry_count: 1,
                })
                .collect());
        }

        let connector_split = split_by_connectors(description);
        if connector_split.len() >= 2 {
            return Ok(connector_split
                .into_iter()
                .enumerate()
                .map(|(i, text)| StepSpec {
                    name: format!("step-{}", i + 1),
                    action_type: infer_action_type(&text),
                    prompt: text,
                    expected_output: String::new(),
                    retry_count: 1,
                })
                .collect());
        }

        // 兜底: 整段作为单步骤
        Ok(vec![StepSpec {
            name: "step-1".to_string(),
            action_type: StepActionType::LlmCall,
            prompt: description.trim().to_string(),
            expected_output: String::new(),
            retry_count: 1,
        }])
    }

    /// 提取预算规格。
    ///
    /// 识别:
    /// - "$N" / "N usd" / "N 美元" / "每月 $N" → monthly_usd
    /// - "N token" / "N tokens" → monthly_tokens
    /// - "每次 $N" / "per run $N" → per_run_usd
    /// - 缺失 → 默认预算
    pub fn extract_budget(&self, description: &str) -> Result<BudgetSpec> {
        let lower = description.to_lowercase();
        let mut budget = self.default_budget.clone();

        // monthly_usd: 优先 "每月 $N" / "monthly $N", 其次任意 "$N" / "N usd"
        if let Some(usd) = parse_usd_with_prefix(&lower, &["每月", "monthly", "月度"]) {
            budget.monthly_usd = usd;
        } else if let Some(usd) = parse_any_usd(&lower) {
            budget.monthly_usd = usd;
        }

        // monthly_tokens: "N token" / "N tokens"
        if let Some(tokens) = parse_tokens(&lower) {
            budget.monthly_tokens = tokens;
        }

        // per_run_usd: "每次 $N" / "per run $N"
        if let Some(usd) = parse_usd_with_prefix(&lower, &["每次", "per run", "单次"]) {
            budget.per_run_usd = usd;
        }

        Ok(budget)
    }

    /// 提取自主度。
    ///
    /// 识别:
    /// - 显式 "L0"-"L5" (大小写不敏感)
    /// - "手动" / "manual" / "只读" → L1
    /// - "全自主" / "自动合并" / "全自动" → L5
    /// - "自动" / "auto" → L4
    /// - "审批" / "approval" / "确认" → L3 (需人工审批)
    /// - "起草" / "draft" → L2
    /// - 缺失 → default_autonomy
    pub fn extract_autonomy(&self, description: &str) -> Result<AutonomyLevel> {
        let lower = description.to_lowercase();

        // 1. 显式 L0-L5
        for level in [
            AutonomyLevel::L5,
            AutonomyLevel::L4,
            AutonomyLevel::L3,
            AutonomyLevel::L2,
            AutonomyLevel::L1,
            AutonomyLevel::L0,
        ] {
            if lower.contains(&format!("l{}", level_as_num(level))) {
                return Ok(level);
            }
        }

        // 2. 关键词推断 (高自主度优先匹配, 避免被 "自动" 误判)
        if lower.contains("全自主") || lower.contains("自动合并") || lower.contains("全自动")
        {
            return Ok(AutonomyLevel::L5);
        }
        if lower.contains("自动") || lower.contains("auto") {
            return Ok(AutonomyLevel::L4);
        }
        if lower.contains("审批") || lower.contains("approval") || lower.contains("需确认") {
            return Ok(AutonomyLevel::L3);
        }
        if lower.contains("起草") || lower.contains("draft") {
            return Ok(AutonomyLevel::L2);
        }
        if lower.contains("手动") || lower.contains("manual") || lower.contains("只读") {
            return Ok(AutonomyLevel::L1);
        }

        Ok(self.default_autonomy)
    }

    /// 生成 LOOP.md 字符串 (符合 [`super::loop_def::LoopDef::from_markdown`] 解析格式)。
    pub fn generate_loop_md(&self, design: &LoopDesign) -> String {
        let per_run_tokens = design.budget.estimated_per_run_tokens();
        let per_run_minutes = design.budget.estimated_per_run_minutes();

        let mut md = String::new();

        // ---- YAML frontmatter ----
        md.push_str("---\n");
        md.push_str(&format!("name: {}\n", yaml_escape(&design.name)));
        md.push_str(&format!(
            "description: {}\n",
            yaml_escape(&design.description)
        ));
        md.push_str(&format!("cadence: \"{}\"\n", design.trigger.to_cadence()));
        md.push_str(&format!("autonomy: {}\n", design.autonomy.as_str()));
        md.push_str(&format!("budget_tokens: {}\n", per_run_tokens));
        md.push_str(&format!("budget_minutes: {}\n", per_run_minutes));
        md.push_str("---\n\n");

        // ---- Markdown body ----
        md.push_str("## Intent\n");
        md.push_str(&design.intent());
        md.push_str("\n\n");

        md.push_str("## Context\n");
        for line in design.context_lines() {
            md.push_str(&format!("- {line}\n"));
        }
        md.push('\n');

        md.push_str("## Action\n");
        for line in design.action_lines() {
            md.push_str(&format!("- {line}\n"));
        }
        md.push('\n');

        md.push_str("## Observation\n");
        for step in &design.steps {
            if !step.expected_output.is_empty() {
                md.push_str(&format!(
                    "- `{}` 输出: {}\n",
                    step.name, step.expected_output
                ));
            } else {
                md.push_str(&format!("- `{}` 执行完成\n", step.name));
            }
        }
        md.push('\n');

        md.push_str("## Adjustment\n");
        md.push_str(&format!(
            "- 月度预算上限: ${:.2} / {} tokens\n",
            design.budget.monthly_usd, design.budget.monthly_tokens
        ));
        md.push_str(&format!("- 单次预算: ${:.2}\n", design.budget.per_run_usd));
        md.push_str(&format!(
            "- 自主度: {} ({})\n",
            design.autonomy.as_str(),
            autonomy_description(design.autonomy)
        ));
        md.push('\n');

        md.push_str("## Stop Condition\n");
        md.push_str("- 预算耗尽 或 所有步骤完成\n\n");

        md.push_str("## Safety\n");
        md.push_str(&format!(
            "- 严格遵循 {} 自主度约束\n",
            design.autonomy.as_str()
        ));
        md.push_str("- 关键写操作需 provenance 标注\n");

        md
    }

    /// 验证 Loop 设计。
    ///
    /// 规则:
    /// - `name` 非空且为 kebab-case (允许字母/数字/`-`)
    /// - `steps` 至少 1 条, 每条 name 非空, retry_count <= 10
    /// - `budget.monthly_usd` / `monthly_tokens` / `per_run_usd` 至少一个 > 0
    /// - `autonomy` 不为 L0 (Loop 不适用内联补全)
    pub fn validate_design(&self, design: &LoopDesign) -> Result<()> {
        if design.name.trim().is_empty() {
            bail!("LoopDesign `name` must not be empty");
        }
        if !is_valid_kebab(&design.name) {
            bail!(
                "LoopDesign `name` must be kebab-case (a-z0-9-), got: {}",
                design.name
            );
        }
        if design.steps.is_empty() {
            bail!("LoopDesign `steps` must have at least one item");
        }
        for (i, step) in design.steps.iter().enumerate() {
            if step.name.trim().is_empty() {
                bail!("LoopDesign step[{i}] `name` must not be empty");
            }
            if step.retry_count > 10 {
                bail!(
                    "LoopDesign step[{i}] `retry_count` must be <= 10, got {}",
                    step.retry_count
                );
            }
        }
        if design.budget.monthly_usd <= 0.0
            && design.budget.monthly_tokens == 0
            && design.budget.per_run_usd <= 0.0
        {
            bail!("LoopDesign `budget` must have at least one positive value");
        }
        if design.autonomy == AutonomyLevel::L0 {
            bail!("LoopDesign `autonomy` must not be L0 (inline completion is not applicable to Loops)");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 内部辅助函数
// ---------------------------------------------------------------------------

/// 判断 lower 字符串是否包含任一关键词。
fn contains_any(lower: &str, keys: &[&str]) -> bool {
    keys.iter().any(|k| lower.contains(k))
}

/// 把 AutonomyLevel 转为数字 (L0→0, L5→5)。
fn level_as_num(level: AutonomyLevel) -> u8 {
    match level {
        AutonomyLevel::L0 => 0,
        AutonomyLevel::L1 => 1,
        AutonomyLevel::L2 => 2,
        AutonomyLevel::L3 => 3,
        AutonomyLevel::L4 => 4,
        AutonomyLevel::L5 => 5,
    }
}

/// 自主度的人类可读说明。
fn autonomy_description(level: AutonomyLevel) -> &'static str {
    match level {
        AutonomyLevel::L0 => "内联补全",
        AutonomyLevel::L1 => "定向编辑(只读+STATE.md)",
        AutonomyLevel::L2 => "对话(起草+Shadow Workspace)",
        AutonomyLevel::L3 => "Plan(Draft PR+人工 merge)",
        AutonomyLevel::L4 => "蜂群(Maker+Checker+Confirm)",
        AutonomyLevel::L5 => "后台自动化(自动 merge)",
    }
}

/// 从描述首句推导 kebab-case 名称。
fn derive_name(description: &str) -> String {
    let first_sentence = description
        .split(|c: char| matches!(c, '。' | '.' | '\n' | ',' | '，'))
        .next()
        .unwrap_or("")
        .trim();
    let candidate = if first_sentence.is_empty() {
        "loop"
    } else {
        first_sentence
    };

    // 取前 30 字符, 转小写, 非 ASCII 字母数字替换为 -
    let mut name: String = candidate
        .chars()
        .take(30)
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // 去除首尾 - 并合并连续 -
    while name.starts_with('-') {
        name.remove(0);
    }
    while name.ends_with('-') {
        name.pop();
    }
    let mut collapsed = String::new();
    let mut prev_dash = false;
    for c in name.chars() {
        if c == '-' {
            if !prev_dash {
                collapsed.push('-');
            }
            prev_dash = true;
        } else {
            collapsed.push(c);
            prev_dash = false;
        }
    }
    if collapsed.is_empty() {
        "loop".to_string()
    } else {
        collapsed
    }
}

/// 校验 kebab-case (a-z0-9-, 不以 - 开头/结尾)。
fn is_valid_kebab(s: &str) -> bool {
    if s.is_empty() || s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// YAML 字符串转义 (含 `:` / `#` / 引号时加双引号)。
fn yaml_escape(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

/// 提取显式 cron 表达式。
///
/// 匹配 `cron:` 前缀, 或形如 "0 9 * * 1-5" 的 5 段空格分隔字符串。
fn extract_explicit_cron(description: &str) -> Option<String> {
    let lower = description.to_lowercase();
    if let Some(pos) = lower.find("cron:") {
        let rest = &description[pos + 5..];
        let token = rest.trim_start().split_whitespace().next()?;
        if token.len() >= 5 {
            return Some(token.trim_matches('"').to_string());
        }
    }
    // 形如 "0 9 * * 1-5" (5 段)
    for line in description.lines() {
        let trimmed = line.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 5 && parts.iter().all(|p| is_cron_field(p)) {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// 判断单个 cron 字段是否合法 (数字 / `*` / `*/n` / `1-5` / `1,2,3`)。
fn is_cron_field(s: &str) -> bool {
    if s == "*" {
        return true;
    }
    if let Some(rest) = s.strip_prefix("*/") {
        return rest.chars().all(|c| c.is_ascii_digit());
    }
    if s.contains('-') || s.contains(',') {
        return s
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == ',');
    }
    s.chars().all(|c| c.is_ascii_digit())
}

/// 提取 "每 N 秒/分钟/小时/天" 的间隔秒数。
fn extract_interval(lower: &str) -> Option<u64> {
    // 中文: 每 N 秒/分钟/小时/天
    let patterns: &[(&str, u64)] = &[
        ("秒", 1),
        ("分钟", 60),
        ("分", 60),
        ("小时", 3600),
        ("时", 3600),
        ("天", 86400),
        ("日", 86400),
    ];
    for (unit, mult) in patterns {
        if let Some(idx) = lower.find(unit) {
            // 向前找数字
            let prefix = &lower[..idx];
            if let Some(n) = trailing_number(prefix) {
                return Some(n * mult);
            }
        }
    }
    // 英文: every N seconds/minutes/hours/days
    let en_patterns: &[(&str, u64)] = &[
        ("seconds", 1),
        ("second", 1),
        ("minutes", 60),
        ("minute", 60),
        ("hours", 3600),
        ("hour", 3600),
        ("days", 86400),
        ("day", 86400),
    ];
    for (unit, mult) in en_patterns {
        if let Some(idx) = lower.find(unit) {
            let prefix = &lower[..idx];
            if let Some(n) = trailing_number(prefix) {
                return Some(n * mult);
            }
        }
    }
    None
}

/// 从字符串尾部提取连续数字 (跳过空格)。
fn trailing_number(s: &str) -> Option<u64> {
    let trimmed = s.trim_end();
    let digits: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// 提取事件名 (从 "当 X 时" / "event: X" 模式)。
fn extract_event_name(description: &str) -> Option<String> {
    let lower = description.to_lowercase();
    if let Some(pos) = lower.find("event:") {
        let rest = &description[pos + 6..];
        let token = rest.trim_start().split_whitespace().next()?;
        if !token.is_empty() {
            return Some(token.trim_matches('"').to_string());
        }
    }
    if let Some(start) = description.find("当") {
        if let Some(end) = description[start..].find("时") {
            let inner = &description[start + 3..start + end];
            let cleaned = inner.trim();
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

/// 提取编号步骤 (支持 "步骤 1:" / "step 1:" / "1. " / "1) ")。
fn extract_numbered_steps(description: &str) -> Vec<String> {
    let mut steps = Vec::new();
    for line in description.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // "步骤 N:" / "step N:"
        let lower = trimmed.to_lowercase();
        if let Some(rest) = lower
            .strip_prefix("步骤")
            .or_else(|| lower.strip_prefix("step"))
        {
            if let Some(after_num) = rest
                .trim_start()
                .find(':')
                .map(|i| &rest.trim_start()[i + 1..])
            {
                let text = after_num.trim();
                if !text.is_empty() {
                    steps.push(text.to_string());
                    continue;
                }
            }
        }
        // "N. " / "N) "
        if let Some(first) = trimmed.chars().next() {
            if first.is_ascii_digit() {
                let rest: String = trimmed.chars().skip(1).collect();
                let rest = rest.trim_start();
                if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
                    let text = text.trim();
                    if !text.is_empty() {
                        steps.push(text.to_string());
                    }
                }
            }
        }
    }
    steps
}

/// 按中文连接词切分步骤。
fn split_by_connectors(description: &str) -> Vec<String> {
    let connectors = [
        "首先", "然后", "接着", "其次", "最后", "同时", "first", "then", "next", "finally",
    ];
    let lower = description.to_lowercase();
    let mut segments = vec![lower.as_str()];
    for conn in connectors {
        let mut new_segments = Vec::new();
        for seg in segments {
            let mut last = 0;
            while let Some(pos) = seg[last..].find(conn) {
                new_segments.push(&seg[last..last + pos]);
                last += pos + conn.len();
            }
            new_segments.push(&seg[last..]);
        }
        segments = new_segments;
    }
    segments
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 根据步骤文本推断动作类型。
fn infer_action_type(text: &str) -> StepActionType {
    let lower = text.to_lowercase();
    if lower.contains("调用 api")
        || lower.contains("call api")
        || lower.contains("github")
        || lower.contains("slack")
        || lower.contains("s3")
        || lower.contains("webdav")
        || lower.contains("http")
    {
        return StepActionType::ExternalApi;
    }
    if lower.contains("工具")
        || lower.contains("tool")
        || lower.contains("文件")
        || lower.contains("file")
        || lower.contains("shell")
        || lower.contains("git")
        || lower.contains("写入")
        || lower.contains("读取")
    {
        return StepActionType::ToolCall;
    }
    if lower.contains("记忆")
        || lower.contains("memory")
        || lower.contains("检索")
        || lower.contains("向量")
        || lower.contains("recall")
    {
        return StepActionType::MemoryQuery;
    }
    if lower.contains("如果")
        || lower.contains("if ")
        || lower.contains("条件")
        || lower.contains("判断")
        || lower.contains("分支")
    {
        return StepActionType::Conditional;
    }
    StepActionType::LlmCall
}

/// 解析带前缀的 USD 金额 (如 "每月 $5" / "monthly $5" / "每月 5 美元")。
fn parse_usd_with_prefix(lower: &str, prefixes: &[&str]) -> Option<f64> {
    for prefix in prefixes {
        if let Some(pos) = lower.find(prefix) {
            let rest = &lower[pos + prefix.len()..];
            if let Some(val) = parse_leading_usd(rest) {
                return Some(val);
            }
        }
    }
    None
}

/// 解析任意 USD 金额 (无前缀, 取第一个匹配)。
fn parse_any_usd(lower: &str) -> Option<f64> {
    parse_leading_usd(lower)
}

/// 从字符串起始处解析 USD 金额 (支持 $N / N usd / N 美元)。
fn parse_leading_usd(s: &str) -> Option<f64> {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix('$') {
        return parse_leading_float(rest.trim_start()).map(|(v, _)| v);
    }
    if let Some(val) = parse_leading_float(trimmed) {
        let after = &trimmed[val.1..];
        let after = after.trim_start();
        if after.starts_with("usd") || after.starts_with("美元") {
            return Some(val.0);
        }
    }
    None
}

/// 解析字符串起始处的浮点数, 返回 (值, 消耗字节数)。
fn parse_leading_float(s: &str) -> Option<(f64, usize)> {
    let mut end = 0;
    let mut seen_dot = false;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() {
            end = i + c.len_utf8();
        } else if c == '.' && !seen_dot {
            seen_dot = true;
            end = i + 1;
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let val: f64 = s[..end].parse().ok()?;
    Some((val, end))
}

/// 解析 token 数量 (如 "500000 token" / "5m tokens")。
fn parse_tokens(lower: &str) -> Option<u64> {
    // 找 "token" / "tokens" 位置, 向前找数字
    let token_pos = lower.find("token")?;
    let prefix = &lower[..token_pos];
    let n = trailing_number(prefix)?;
    Some(n)
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::loop_def::LoopDef;

    // ---- TriggerSpec ----

    #[test]
    fn trigger_cron_to_cadence() {
        let t = TriggerSpec::Cron("0 9 * * 1-5".to_string());
        assert_eq!(t.to_cadence(), "0 9 * * 1-5");
    }

    #[test]
    fn trigger_interval_minute_hour_day() {
        assert_eq!(TriggerSpec::Interval(60).to_cadence(), "* * * * *");
        assert_eq!(TriggerSpec::Interval(3600).to_cadence(), "0 * * * *");
        assert_eq!(TriggerSpec::Interval(86400).to_cadence(), "0 0 * * *");
        assert_eq!(TriggerSpec::Interval(120).to_cadence(), "120s");
    }

    #[test]
    fn trigger_event_to_cadence() {
        assert_eq!(
            TriggerSpec::Event("pr_opened".to_string()).to_cadence(),
            "event:pr_opened"
        );
    }

    #[test]
    fn trigger_manual_to_cadence() {
        assert_eq!(TriggerSpec::Manual.to_cadence(), "manual");
    }

    #[test]
    fn trigger_human_description_formats() {
        assert!(TriggerSpec::Cron("0 9 * * *".to_string())
            .human_description()
            .contains("cron"));
        assert!(TriggerSpec::Interval(3600)
            .human_description()
            .contains("小时"));
        assert!(TriggerSpec::Interval(120)
            .human_description()
            .contains("分钟"));
        assert!(TriggerSpec::Interval(30).human_description().contains("秒"));
        assert!(TriggerSpec::Event("x".to_string())
            .human_description()
            .contains("事件"));
        assert!(TriggerSpec::Manual.human_description().contains("手动"));
    }

    // ---- extract_trigger ----

    #[test]
    fn extract_trigger_daily_keyword() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("每天早上扫描 CI 失败")
            .expect("test op should succeed");
        assert!(matches!(t, TriggerSpec::Cron(ref s) if s.contains("1-5")));
    }

    #[test]
    fn extract_trigger_hourly_keyword() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("每小时检查指标")
            .expect("test op should succeed");
        assert!(matches!(t, TriggerSpec::Cron(ref s) if s == "0 * * * *"));
    }

    #[test]
    fn extract_trigger_interval_chinese() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("每 30 分钟刷新一次缓存")
            .expect("test op should succeed");
        assert_eq!(t, TriggerSpec::Interval(1800));
    }

    #[test]
    fn extract_trigger_interval_english() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("every 2 hours poll the API")
            .expect("test op should succeed");
        assert_eq!(t, TriggerSpec::Interval(7200));
    }

    #[test]
    fn extract_trigger_explicit_cron() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("cron: 0 18 * * 1-5 生成日报")
            .expect("test op should succeed");
        assert_eq!(t, TriggerSpec::Cron("0 18 * * 1-5".to_string()));
    }

    #[test]
    fn extract_trigger_event_pattern() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("当 pr_opened 事件触发时审查代码")
            .expect("test op should succeed");
        assert_eq!(t, TriggerSpec::Event("pr_opened".to_string()));
    }

    #[test]
    fn extract_trigger_manual_default() {
        let d = LoopDesigner::new();
        let t = d
            .extract_trigger("整理一下我的笔记")
            .expect("test op should succeed");
        assert_eq!(t, TriggerSpec::Manual);
    }

    // ---- extract_steps ----

    #[test]
    fn extract_steps_numbered_chinese() {
        let d = LoopDesigner::new();
        let steps = d
            .extract_steps("步骤 1: 拉取数据\n步骤 2: 分析\n步骤 3: 生成报告")
            .expect("test op should succeed");
        assert_eq!(steps.len(), 3);
        assert!(steps[0].prompt.contains("拉取数据"));
        assert_eq!(steps[1].name, "step-2");
    }

    #[test]
    fn extract_steps_numbered_dot() {
        let d = LoopDesigner::new();
        let steps = d
            .extract_steps("1. fetch diff\n2. analyze\n3. post comment")
            .expect("test op should succeed");
        assert_eq!(steps.len(), 3);
        assert!(steps[2].prompt.contains("post comment"));
    }

    #[test]
    fn extract_steps_connector_split() {
        let d = LoopDesigner::new();
        let steps = d
            .extract_steps("首先拉取数据, 然后分析, 最后生成报告")
            .expect("test op should succeed");
        assert!(steps.len() >= 2);
    }

    #[test]
    fn extract_steps_fallback_single() {
        let d = LoopDesigner::new();
        let steps = d
            .extract_steps("一段没有编号也没有连接词的纯文本描述")
            .expect("test op should succeed");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].name, "step-1");
    }

    #[test]
    fn extract_steps_infer_action_type() {
        let d = LoopDesigner::new();
        let steps = d
            .extract_steps("1. 调用 GitHub API 拉取 PR\n2. 用 LLM 分析\n3. 如果有风险则告警")
            .expect("test op should succeed");
        assert_eq!(steps[0].action_type, StepActionType::ExternalApi);
        assert_eq!(steps[1].action_type, StepActionType::LlmCall);
        assert_eq!(steps[2].action_type, StepActionType::Conditional);
    }

    // ---- extract_budget ----

    #[test]
    fn extract_budget_monthly_usd_explicit() {
        let d = LoopDesigner::new();
        let b = d
            .extract_budget("每月 $5 用于日报生成")
            .expect("test op should succeed");
        assert_eq!(b.monthly_usd, 5.0);
    }

    #[test]
    fn extract_budget_tokens() {
        let d = LoopDesigner::new();
        let b = d
            .extract_budget("每月 500000 token 的预算")
            .expect("test op should succeed");
        assert_eq!(b.monthly_tokens, 500_000);
    }

    #[test]
    fn extract_budget_per_run() {
        let d = LoopDesigner::new();
        let b = d
            .extract_budget("每次 $0.5 的单次预算")
            .expect("test op should succeed");
        assert_eq!(b.per_run_usd, 0.5);
    }

    #[test]
    fn extract_budget_default_when_missing() {
        let d = LoopDesigner::new();
        let b = d
            .extract_budget("没有任何预算信息")
            .expect("test op should succeed");
        assert_eq!(b.monthly_usd, BudgetSpec::default().monthly_usd);
    }

    // ---- extract_autonomy ----

    #[test]
    fn extract_autonomy_explicit_level() {
        let d = LoopDesigner::new();
        assert_eq!(
            d.extract_autonomy("使用 L3 自主度")
                .expect("test op should succeed"),
            AutonomyLevel::L3
        );
        assert_eq!(
            d.extract_autonomy("自主度 l5")
                .expect("test op should succeed"),
            AutonomyLevel::L5
        );
    }

    #[test]
    fn extract_autonomy_keyword_inference() {
        let d = LoopDesigner::new();
        assert_eq!(
            d.extract_autonomy("全自动自动合并")
                .expect("test op should succeed"),
            AutonomyLevel::L5
        );
        assert_eq!(
            d.extract_autonomy("自动执行无需审批")
                .expect("test op should succeed"),
            AutonomyLevel::L4
        );
        assert_eq!(
            d.extract_autonomy("需审批后执行")
                .expect("test op should succeed"),
            AutonomyLevel::L3
        );
        assert_eq!(
            d.extract_autonomy("起草草稿")
                .expect("test op should succeed"),
            AutonomyLevel::L2
        );
        assert_eq!(
            d.extract_autonomy("手动只读模式")
                .expect("test op should succeed"),
            AutonomyLevel::L1
        );
    }

    #[test]
    fn extract_autonomy_default_when_unknown() {
        let d = LoopDesigner::new();
        assert_eq!(
            d.extract_autonomy("没有自主度关键词")
                .expect("test op should succeed"),
            AutonomyLevel::L2
        );
    }

    // ---- design_from_natural_language (集成) ----

    #[test]
    fn design_from_nl_full_pipeline() {
        let d = LoopDesigner::new();
        let design = d
            .design_from_natural_language(
                "每天工作日早上扫描 CI 失败\n1. 拉取 GitHub Actions 日志\n2. 用 LLM 分类\n每月 $3 预算, 自主度 L2",
            )
            .expect("design should succeed");
        assert!(!design.name.is_empty());
        assert!(matches!(design.trigger, TriggerSpec::Cron(_)));
        assert_eq!(design.steps.len(), 2);
        assert_eq!(design.budget.monthly_usd, 3.0);
        assert_eq!(design.autonomy, AutonomyLevel::L2);
    }

    #[test]
    fn design_from_nl_rejects_empty() {
        let d = LoopDesigner::new();
        assert!(d.design_from_natural_language("   ").is_err());
    }

    // ---- validate_design ----

    #[test]
    fn validate_accepts_template_design() {
        let d = LoopDesigner::new();
        let tmpl = LoopDesignLibrary::find("daily-report").expect("template should exist");
        d.validate_design(&tmpl.design)
            .expect("template should be valid");
    }

    #[test]
    fn validate_rejects_empty_name() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("daily-report")
            .expect("test op should succeed")
            .design;
        design.name = "".to_string();
        assert!(d.validate_design(&design).is_err());
    }

    #[test]
    fn validate_rejects_non_kebab_name() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("daily-report")
            .expect("test op should succeed")
            .design;
        design.name = "Daily Report".to_string();
        assert!(d.validate_design(&design).is_err());
    }

    #[test]
    fn validate_rejects_empty_steps() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("daily-report")
            .expect("test op should succeed")
            .design;
        design.steps.clear();
        assert!(d.validate_design(&design).is_err());
    }

    #[test]
    fn validate_rejects_excessive_retry() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("daily-report")
            .expect("test op should succeed")
            .design;
        design.steps[0].retry_count = 99;
        assert!(d.validate_design(&design).is_err());
    }

    #[test]
    fn validate_rejects_l0_autonomy() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("daily-report")
            .expect("test op should succeed")
            .design;
        design.autonomy = AutonomyLevel::L0;
        assert!(d.validate_design(&design).is_err());
    }

    #[test]
    fn validate_rejects_zero_budget() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("daily-report")
            .expect("test op should succeed")
            .design;
        design.budget.monthly_usd = 0.0;
        design.budget.monthly_tokens = 0;
        design.budget.per_run_usd = 0.0;
        assert!(d.validate_design(&design).is_err());
    }

    // ---- generate_loop_md ----

    #[test]
    fn generate_loop_md_round_trips_with_loop_def() {
        let d = LoopDesigner::new();
        let design = LoopDesignLibrary::find("daily-report").expect("template should exist");
        let md = d.generate_loop_md(&design.design);

        // 必须能被 LoopDef::from_markdown 解析回去
        let def = LoopDef::from_markdown(&md).expect("generated LOOP.md must be parseable");
        assert_eq!(def.name, "daily-report");
        assert_eq!(def.autonomy, AutonomyLevel::L2);
        assert!(!def.intent.is_empty());
        assert!(!def.action.is_empty());
        def.validate()
            .expect("generated LOOP.md must pass LoopDef::validate");
    }

    #[test]
    fn generate_loop_md_contains_frontmatter_and_sections() {
        let d = LoopDesigner::new();
        let design = LoopDesignLibrary::find("code-review").expect("template should exist");
        let md = d.generate_loop_md(&design.design);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: code-review"));
        assert!(md.contains("## Intent"));
        assert!(md.contains("## Context"));
        assert!(md.contains("## Action"));
        assert!(md.contains("## Observation"));
        assert!(md.contains("## Adjustment"));
        assert!(md.contains("## Stop Condition"));
        assert!(md.contains("## Safety"));
    }

    #[test]
    fn generate_loop_md_uses_trigger_cadence() {
        let d = LoopDesigner::new();
        let design = LoopDesignLibrary::find("trend-monitor").expect("template should exist");
        let md = d.generate_loop_md(&design.design);
        // Interval(3600) → "0 * * * *"
        assert!(md.contains("cadence: \"0 * * * *\""));
    }

    #[test]
    fn generate_loop_md_manual_trigger() {
        let d = LoopDesigner::new();
        let mut design = LoopDesignLibrary::find("memory-tidy")
            .expect("template should exist")
            .design;
        design.trigger = TriggerSpec::Manual;
        let md = d.generate_loop_md(&design);
        assert!(md.contains("cadence: \"manual\""));
    }

    // ---- LoopDesignLibrary ----

    #[test]
    fn library_has_at_least_five_templates() {
        let templates = LoopDesignLibrary::list();
        assert!(
            templates.len() >= 5,
            "expected >= 5 templates, got {}",
            templates.len()
        );
    }

    #[test]
    fn library_find_by_id() {
        let t = LoopDesignLibrary::find("code-review").expect("code-review template should exist");
        assert_eq!(t.title, "代码审查");
        assert!(matches!(t.design.trigger, TriggerSpec::Event(_)));
    }

    #[test]
    fn library_find_returns_none_for_unknown() {
        assert!(LoopDesignLibrary::find("non-existent").is_none());
    }

    #[test]
    fn library_find_by_tag() {
        let matched = LoopDesignLibrary::find_by_tag("memory");
        assert!(matched.iter().any(|t| t.id == "memory-tidy"));
    }

    #[test]
    fn library_all_templates_validate() {
        let d = LoopDesigner::new();
        for tmpl in LoopDesignLibrary::list() {
            d.validate_design(&tmpl.design)
                .unwrap_or_else(|e| panic!("template {} failed validation: {e}", tmpl.id));
        }
    }

    #[test]
    fn library_all_templates_round_trip_to_loop_def() {
        let d = LoopDesigner::new();
        for tmpl in LoopDesignLibrary::list() {
            let md = d.generate_loop_md(&tmpl.design);
            let def = LoopDef::from_markdown(&md)
                .unwrap_or_else(|e| panic!("template {} failed to round-trip: {e}", tmpl.id));
            def.validate()
                .unwrap_or_else(|e| panic!("template {} generated invalid LOOP.md: {e}", tmpl.id));
            assert_eq!(def.name, tmpl.design.name);
        }
    }

    // ---- 辅助函数测试 ----

    #[test]
    fn derive_name_kebab_case() {
        assert_eq!(
            derive_name("Daily Report Generator"),
            "daily-report-generator"
        );
        assert_eq!(derive_name("代码审查 Loop"), "loop"); // 全中文 → 无 ASCII → 兜底
        assert_eq!(derive_name("CI scan"), "ci-scan");
    }

    #[test]
    fn is_valid_kebab_checks() {
        assert!(is_valid_kebab("daily-report"));
        assert!(is_valid_kebab("loop-1"));
        assert!(!is_valid_kebab(""));
        assert!(!is_valid_kebab("-abc"));
        assert!(!is_valid_kebab("abc-"));
        assert!(!is_valid_kebab("DailyReport"));
        assert!(!is_valid_kebab("daily_report"));
    }

    #[test]
    fn budget_estimation_helpers() {
        let b = BudgetSpec {
            monthly_usd: 10.0,
            monthly_tokens: 3_000_000,
            per_run_usd: 1.0,
        };
        assert_eq!(b.estimated_per_run_tokens(), 100_000); // 3m / 30
        assert_eq!(b.estimated_per_run_minutes(), 10); // 1.0 * 10

        let b_zero = BudgetSpec {
            monthly_usd: 0.0,
            monthly_tokens: 0,
            per_run_usd: 0.0,
        };
        assert_eq!(b_zero.estimated_per_run_tokens(), 50_000); // 兜底
        assert_eq!(b_zero.estimated_per_run_minutes(), 10); // 兜底
    }

    #[test]
    fn step_spec_llm_helper() {
        let s = StepSpec::llm("draft", "写一段摘要");
        assert_eq!(s.name, "draft");
        assert_eq!(s.action_type, StepActionType::LlmCall);
        assert_eq!(s.retry_count, 1);
    }

    #[test]
    fn step_action_type_as_str() {
        assert_eq!(StepActionType::LlmCall.as_str(), "llm_call");
        assert_eq!(StepActionType::ToolCall.as_str(), "tool_call");
        assert_eq!(StepActionType::MemoryQuery.as_str(), "memory_query");
        assert_eq!(StepActionType::ExternalApi.as_str(), "external_api");
        assert_eq!(StepActionType::Conditional.as_str(), "conditional");
    }

    #[test]
    fn loop_design_intent_falls_back_to_description() {
        let design = LoopDesign {
            name: "test".to_string(),
            description: "描述文本".to_string(),
            trigger: TriggerSpec::Manual,
            steps: vec![StepSpec::llm("s", "p")],
            budget: BudgetSpec::default(),
            autonomy: AutonomyLevel::L2,
            metadata: serde_json::Value::Null,
        };
        assert_eq!(design.intent(), "描述文本");
    }

    #[test]
    fn loop_design_intent_uses_metadata_when_present() {
        let design = LoopDesign {
            name: "test".to_string(),
            description: "描述文本".to_string(),
            trigger: TriggerSpec::Manual,
            steps: vec![StepSpec::llm("s", "p")],
            budget: BudgetSpec::default(),
            autonomy: AutonomyLevel::L2,
            metadata: serde_json::json!({"intent": "显式 intent"}),
        };
        assert_eq!(design.intent(), "显式 intent");
    }
}
