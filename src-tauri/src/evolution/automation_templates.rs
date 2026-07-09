//! T-E-S-56: Automation 模板库。
//!
//! 预定义的 Automation 模板库，结合 Cron 引擎提供开箱即用的自动化场景。
//!
//! ## 能力
//!
//! - **模板库** — 内置 12+ 个常用自动化模板（记忆合并/进化自检/每日回顾/健康检查/
//!   数据备份/技能清理/缓存清理/费用报告/MOC 更新/主动推送/同步检查/日志轮转）。
//! - **分类与搜索** — 按 `AutomationCategory` 筛选，按关键词搜索
//!   name/description/tags/template_id（大小写不敏感）。
//! - **自定义模板** — `add()` / `remove()` 支持用户扩展模板库。
//! - **Cron 转换** — `to_cron_task_def()` 将模板转换为 `CronTaskDef`，可直接
//!   注册到 `CronEngine`。
//! - **配置覆盖** — `TemplateConfig` 支持对模板字段（cron_expr/command/args/tags 等）
//!   进行覆盖，`apply_to()` 返回覆盖后的新模板。
//! - **智能推荐** — `AutomationSuggestion::suggest_for_user()` 基于用户活动模式
//!   推荐合适的模板。
//!
//! ## 设计约束
//!
//! 1. **feature 门控** — 与 `cron_engine` / `cron_scheduler` 一致，由
//!    `self-evolution` feature 门控（文件顶部 `#![cfg(...)]`）。
//! 2. **尽力而为** — `to_cron_task_def()` 对未知 template_id 返回错误，不 panic；
//!    `TemplateConfig::apply_to()` 对类型不匹配的 override 静默跳过（记 warning）。
//! 3. **单一 Vec 存储** — 内置与自定义模板存于同一 `Vec`，`remove()` 可移除
//!    任意模板（含内置）；如需保留内置模板不可移除，调用方自行判断。

#![cfg(feature = "self-evolution")]

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::evolution::cron_engine::{CronTaskDef, CronTaskType};

// ---------------------------------------------------------------------------
// 分类
// ---------------------------------------------------------------------------

/// Automation 模板分类。
///
/// 每个模板归属一个分类，用于 `list_by_category()` 筛选与 UI 分组展示。
/// 序列化为 snake_case（如 `"maintenance"` / `"evolution"` 等）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AutomationCategory {
    /// 维护类（日志轮转 / 系统 upkeep）。
    Maintenance,
    /// 同步类（多端同步检查）。
    Sync,
    /// 备份类（数据备份 / 恢复）。
    Backup,
    /// 监控类（健康检查 / 费用报告）。
    Monitoring,
    /// 进化类（记忆合并 / 进化自检 / MOC 更新）。
    Evolution,
    /// 通知类（主动推送 / 每日回顾）。
    Notification,
    /// 清理类（缓存 / 技能 / 临时文件清理）。
    Cleanup,
    /// 自定义类（用户扩展）。
    Custom,
}

// ---------------------------------------------------------------------------
// 模板
// ---------------------------------------------------------------------------

/// Automation 模板定义。
///
/// 描述一个可自动化的场景：何时触发（`cron_expr`）、执行什么（`command` + `args`）、
/// 归属分类、标签、默认是否启用、所需环境变量等。可通过 `to_cron_task_def()`
/// 转换为 `CronTaskDef` 注册到 `CronEngine`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationTemplate {
    /// 模板唯一标识（如 `"daily_memory_consolidation"`）。
    pub template_id: String,
    /// 人类可读的模板名称。
    pub name: String,
    /// 模板描述（详细说明触发时机与执行内容）。
    pub description: String,
    /// 模板分类。
    pub category: AutomationCategory,
    /// 5 字段 cron 表达式（如 `"0 3 * * *"`）。
    pub cron_expr: String,
    /// 要执行的命令（如 `"nebula"`）。
    pub command: String,
    /// 命令参数。
    #[serde(default)]
    pub args: Vec<String>,
    /// 标签（供搜索与分组）。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 是否默认启用（用户可在 `TemplateConfig` 中覆盖）。
    #[serde(default)]
    pub default_enabled: bool,
    /// 所需环境变量列表（执行前检查，缺失则警告）。
    #[serde(default)]
    pub required_env_vars: Vec<String>,
    /// 配置 schema（可选，描述可覆盖的字段及其类型，供 UI 生成配置表单）。
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// 模板库
// ---------------------------------------------------------------------------

/// Automation 模板库。
///
/// 内置 12+ 个常用自动化模板，支持按分类筛选、关键词搜索、自定义添加/移除，
/// 以及转换为 `CronTaskDef` 注册到 `CronEngine`。
///
/// # 示例
///
/// ```ignore
/// use crate::evolution::automation_templates::{AutomationTemplateLibrary, AutomationCategory};
///
/// let lib = AutomationTemplateLibrary::new();
/// // 列出所有模板
/// let all = lib.list();
/// // 按分类筛选
/// let evolution = lib.list_by_category(&AutomationCategory::Evolution);
/// // 转换为 CronTaskDef
/// let def = lib.to_cron_task_def("daily_memory_consolidation").unwrap();
/// ```
pub struct AutomationTemplateLibrary {
    /// 模板列表（内置 + 用户自定义）。
    templates: Vec<AutomationTemplate>,
}

impl Default for AutomationTemplateLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl AutomationTemplateLibrary {
    /// 构造模板库，内置 12+ 个常用自动化模板。
    pub fn new() -> Self {
        let templates = vec![
            // 1. 每天 03:00 记忆合并（L1→L2→L3）
            AutomationTemplate {
                template_id: "daily_memory_consolidation".to_string(),
                name: "每日记忆合并".to_string(),
                description: "每天 03:00 执行 L1→L2→L3 记忆合并，将短期记忆沉淀为长期记忆"
                    .to_string(),
                category: AutomationCategory::Evolution,
                cron_expr: "0 3 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec![
                    "memory".to_string(),
                    "consolidate".to_string(),
                    "--levels=L1,L2,L3".to_string(),
                ],
                tags: vec![
                    "memory".to_string(),
                    "consolidation".to_string(),
                    "l1-l2-l3".to_string(),
                ],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "levels": { "type": "string", "default": "L1,L2,L3" }
                    }
                })),
            },
            // 2. 每天 12:00 进化自检
            AutomationTemplate {
                template_id: "daily_evolution_self_check".to_string(),
                name: "每日进化自检".to_string(),
                description: "每天 12:00 执行 EvolutionEngine 4 Phase 进化自检（Extract→Compile→Reflect→Soul）"
                    .to_string(),
                category: AutomationCategory::Evolution,
                cron_expr: "0 12 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["evolution".to_string(), "self-check".to_string()],
                tags: vec![
                    "evolution".to_string(),
                    "self-check".to_string(),
                    "4-phase".to_string(),
                ],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 3. 每天 21:00 每日回顾
            AutomationTemplate {
                template_id: "daily_review".to_string(),
                name: "每日回顾".to_string(),
                description: "每天 21:00 执行每日回顾：Honcho 画像 nudge + Skill 评估".to_string(),
                category: AutomationCategory::Notification,
                cron_expr: "0 21 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["review".to_string(), "daily".to_string()],
                tags: vec!["review".to_string(), "honcho".to_string(), "skill".to_string()],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 4. 每小时健康检查
            AutomationTemplate {
                template_id: "hourly_health_check".to_string(),
                name: "每小时健康检查".to_string(),
                description: "每小时执行系统健康检查，监控 Memory/Evolution/Cron 引擎运行状态"
                    .to_string(),
                category: AutomationCategory::Monitoring,
                cron_expr: "0 * * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["health".to_string(), "check".to_string()],
                tags: vec!["health".to_string(), "monitoring".to_string(), "hourly".to_string()],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 5. 每周一 02:00 数据备份
            AutomationTemplate {
                template_id: "weekly_backup".to_string(),
                name: "每周数据备份".to_string(),
                description: "每周一 02:00 执行全量数据备份（Memory/Skills/Config）".to_string(),
                category: AutomationCategory::Backup,
                cron_expr: "0 2 * * 1".to_string(),
                command: "nebula".to_string(),
                args: vec!["backup".to_string(), "full".to_string()],
                tags: vec!["backup".to_string(), "weekly".to_string(), "full".to_string()],
                default_enabled: true,
                required_env_vars: vec!["NEBULA_BACKUP_DIR".to_string()],
                config_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target_dir": { "type": "string" },
                        "compress": { "type": "boolean", "default": true }
                    }
                })),
            },
            // 6. 每周日 23:00 清理过期技能
            AutomationTemplate {
                template_id: "weekly_skill_cleanup".to_string(),
                name: "每周技能清理".to_string(),
                description: "每周日 23:00 清理过期/低使用率技能（归档至 SkillArchive）".to_string(),
                category: AutomationCategory::Cleanup,
                cron_expr: "0 23 * * 0".to_string(),
                command: "nebula".to_string(),
                args: vec!["skill".to_string(), "cleanup".to_string(), "--archive".to_string()],
                tags: vec!["skill".to_string(), "cleanup".to_string(), "weekly".to_string()],
                default_enabled: false,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 7. 每天 04:00 清理过期缓存
            AutomationTemplate {
                template_id: "daily_cache_cleanup".to_string(),
                name: "每日缓存清理".to_string(),
                description: "每天 04:00 清理过期缓存条目（LLM 响应缓存 / 临时文件）".to_string(),
                category: AutomationCategory::Cleanup,
                cron_expr: "0 4 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["cache".to_string(), "cleanup".to_string()],
                tags: vec!["cache".to_string(), "cleanup".to_string(), "daily".to_string()],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 8. 每天 23:00 生成费用报告
            AutomationTemplate {
                template_id: "daily_cost_report".to_string(),
                name: "每日费用报告".to_string(),
                description: "每天 23:00 汇总当日 Token 消耗与 API 费用，生成费用报告".to_string(),
                category: AutomationCategory::Monitoring,
                cron_expr: "0 23 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["cost".to_string(), "report".to_string(), "--daily".to_string()],
                tags: vec!["cost".to_string(), "report".to_string(), "token".to_string()],
                default_enabled: false,
                required_env_vars: vec!["NEBULA_LLM_API_KEY".to_string()],
                config_schema: None,
            },
            // 9. 每天 06:00 更新 MOC 聚类
            AutomationTemplate {
                template_id: "daily_moc_update".to_string(),
                name: "每日 MOC 更新".to_string(),
                description: "每天 06:00 更新 MOC（Map of Content）聚类，重组记忆主题索引".to_string(),
                category: AutomationCategory::Evolution,
                cron_expr: "0 6 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["moc".to_string(), "update".to_string()],
                tags: vec!["moc".to_string(), "clustering".to_string(), "daily".to_string()],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 10. 每小时主动推送
            AutomationTemplate {
                template_id: "hourly_proactive_nudge".to_string(),
                name: "每小时主动推送".to_string(),
                description: "每小时触发 Proactive Engine 评估活动模式，生成主动建议并推送".to_string(),
                category: AutomationCategory::Notification,
                cron_expr: "0 * * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["proactive".to_string(), "nudge".to_string()],
                tags: vec!["proactive".to_string(), "nudge".to_string(), "hourly".to_string()],
                default_enabled: false,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 11. 每天 08:00 检查多端同步状态
            AutomationTemplate {
                template_id: "daily_sync_check".to_string(),
                name: "每日同步检查".to_string(),
                description: "每天 08:00 检查多端同步状态，报告冲突与待同步条目".to_string(),
                category: AutomationCategory::Sync,
                cron_expr: "0 8 * * *".to_string(),
                command: "nebula".to_string(),
                args: vec!["sync".to_string(), "check".to_string()],
                tags: vec!["sync".to_string(), "multi-device".to_string(), "daily".to_string()],
                default_enabled: true,
                required_env_vars: vec![],
                config_schema: None,
            },
            // 12. 每周日 01:00 日志轮转
            AutomationTemplate {
                template_id: "weekly_log_rotation".to_string(),
                name: "每周日志轮转".to_string(),
                description: "每周日 01:00 执行日志轮转，压缩归档旧日志并清理过期文件".to_string(),
                category: AutomationCategory::Maintenance,
                cron_expr: "0 1 * * 0".to_string(),
                command: "nebula".to_string(),
                args: vec!["log".to_string(), "rotate".to_string()],
                tags: vec!["log".to_string(), "rotation".to_string(), "weekly".to_string()],
                default_enabled: true,
                required_env_vars: vec!["NEBULA_LOG_DIR".to_string()],
                config_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "keep_days": { "type": "integer", "default": 30 },
                        "compress": { "type": "boolean", "default": true }
                    }
                })),
            },
        ];
        info!(
            target: "nebula.automation_templates",
            count = templates.len(),
            "automation template library initialized"
        );
        Self { templates }
    }

    /// 列出所有模板（按插入顺序）。
    pub fn list(&self) -> Vec<&AutomationTemplate> {
        self.templates.iter().collect()
    }

    /// 按分类筛选模板。
    pub fn list_by_category(&self, category: &AutomationCategory) -> Vec<&AutomationTemplate> {
        self.templates
            .iter()
            .filter(|t| &t.category == category)
            .collect()
    }

    /// 获取指定 template_id 的模板。
    pub fn get(&self, template_id: &str) -> Option<&AutomationTemplate> {
        self.templates.iter().find(|t| t.template_id == template_id)
    }

    /// 搜索模板（大小写不敏感），匹配范围：name / description / tags / template_id。
    ///
    /// 任意字段包含查询子串即返回。空查询返回空列表（避免返回全部）。
    pub fn search(&self, query: &str) -> Vec<&AutomationTemplate> {
        if query.is_empty() {
            return Vec::new();
        }
        let q = query.to_lowercase();
        self.templates
            .iter()
            .filter(|t| {
                t.name.to_lowercase().contains(&q)
                    || t.description.to_lowercase().contains(&q)
                    || t.template_id.to_lowercase().contains(&q)
                    || t.tags.iter().any(|tag| tag.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// 添加自定义模板。
    ///
    /// 若 `template_id` 已存在，覆盖旧模板（调用方如需拒绝重复应自行检查 `get()`）。
    pub fn add(&mut self, template: AutomationTemplate) {
        // 移除同 id 的旧模板（覆盖语义）。
        if let Some(pos) = self
            .templates
            .iter()
            .position(|t| t.template_id == template.template_id)
        {
            self.templates.remove(pos);
        }
        self.templates.push(template);
    }

    /// 移除指定 template_id 的模板。
    ///
    /// 返回 `true` 表示移除成功，`false` 表示模板不存在。
    pub fn remove(&mut self, template_id: &str) -> bool {
        if let Some(pos) = self
            .templates
            .iter()
            .position(|t| t.template_id == template_id)
        {
            self.templates.remove(pos);
            true
        } else {
            false
        }
    }

    /// 将模板转换为 `CronTaskDef`，可直接注册到 `CronEngine`。
    ///
    /// - `task_id` = `template_id`
    /// - `task_name` = `name`
    /// - `task_type` = `Recurring`
    /// - `enabled` = `default_enabled`
    /// - `max_retries` = 3（默认）
    /// - `retry_delay_secs` = 60（默认）
    /// - `tags` 透传
    ///
    /// 未知 template_id 返回错误。
    pub fn to_cron_task_def(&self, template_id: &str) -> Result<CronTaskDef> {
        let template = self
            .get(template_id)
            .ok_or_else(|| anyhow!("template not found: {template_id}"))?;
        Ok(CronTaskDef {
            task_id: template.template_id.clone(),
            cron_expr: template.cron_expr.clone(),
            task_name: template.name.clone(),
            task_type: CronTaskType::Recurring,
            command: template.command.clone(),
            args: template.args.clone(),
            enabled: template.default_enabled,
            max_retries: 3,
            retry_delay_secs: 60,
            timeout_secs: None,
            last_run: None,
            next_run: None,
            tags: template.tags.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// 模板配置
// ---------------------------------------------------------------------------

/// 模板配置 — 用户对某个模板的定制（字段覆盖 + 启用状态）。
///
/// `apply_to()` 将配置应用到模板上，返回覆盖后的新模板。
/// `overrides` 是字段名 → JSON 值的映射，支持的 key：
/// - `cron_expr`（string）— 覆盖 cron 表达式
/// - `command`（string）— 覆盖命令
/// - `args`（array of string）— 覆盖命令参数
/// - `name`（string）— 覆盖名称
/// - `description`（string）— 覆盖描述
/// - `tags`（array of string）— 覆盖标签
/// - `default_enabled`（bool）— 覆盖默认启用状态
///
/// 类型不匹配的 override 会被静默跳过（记 warning）。
/// `enabled` 字段独立于 `overrides`，最终覆盖 `default_enabled`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    /// 对应的模板 ID。
    pub template_id: String,
    /// 字段覆盖映射（key = 字段名，value = JSON 值）。
    #[serde(default)]
    pub overrides: HashMap<String, serde_json::Value>,
    /// 是否启用（覆盖模板的 `default_enabled`）。
    pub enabled: bool,
}

impl TemplateConfig {
    /// 将配置应用到模板上，返回覆盖后的新模板。
    ///
    /// 1. 克隆模板；
    /// 2. 遍历 `overrides`，对类型匹配的已知字段进行覆盖；
    /// 3. 用 `self.enabled` 覆盖 `default_enabled`。
    pub fn apply_to(&self, template: &AutomationTemplate) -> AutomationTemplate {
        let mut out = template.clone();

        // 覆盖字符串字段。
        if let Some(v) = self.overrides.get("cron_expr") {
            if let Some(s) = v.as_str() {
                out.cron_expr = s.to_string();
            } else {
                warn!(
                    target: "nebula.automation_templates",
                    field = "cron_expr",
                    "override type mismatch, expected string; skipping"
                );
            }
        }
        if let Some(v) = self.overrides.get("command") {
            if let Some(s) = v.as_str() {
                out.command = s.to_string();
            } else {
                warn!(
                    target: "nebula.automation_templates",
                    field = "command",
                    "override type mismatch, expected string; skipping"
                );
            }
        }
        if let Some(v) = self.overrides.get("name") {
            if let Some(s) = v.as_str() {
                out.name = s.to_string();
            }
        }
        if let Some(v) = self.overrides.get("description") {
            if let Some(s) = v.as_str() {
                out.description = s.to_string();
            }
        }

        // 覆盖 args（array of string）。
        if let Some(v) = self.overrides.get("args") {
            if let Some(arr) = v.as_array() {
                let parsed: Option<Vec<String>> = arr
                    .iter()
                    .map(|item| item.as_str().map(|s| s.to_string()))
                    .collect();
                if let Some(args) = parsed {
                    out.args = args;
                } else {
                    warn!(
                        target: "nebula.automation_templates",
                        field = "args",
                        "override array contains non-string element; skipping"
                    );
                }
            }
        }

        // 覆盖 tags（array of string）。
        if let Some(v) = self.overrides.get("tags") {
            if let Some(arr) = v.as_array() {
                let parsed: Option<Vec<String>> = arr
                    .iter()
                    .map(|item| item.as_str().map(|s| s.to_string()))
                    .collect();
                if let Some(tags) = parsed {
                    out.tags = tags;
                }
            }
        }

        // 覆盖 default_enabled（bool）。
        if let Some(v) = self.overrides.get("default_enabled") {
            if let Some(b) = v.as_bool() {
                out.default_enabled = b;
            }
        }

        // config.enabled 最终覆盖 default_enabled。
        out.default_enabled = self.enabled;

        out
    }
}

// ---------------------------------------------------------------------------
// 模板推荐
// ---------------------------------------------------------------------------

/// Automation 模板推荐 — 基于用户活动模式推荐合适的模板。
///
/// 由 `suggest_for_user()` 静态方法根据活动模式字符串中的关键词生成。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationSuggestion {
    /// 推荐的模板 ID。
    pub template_id: String,
    /// 推荐理由（人类可读）。
    pub reason: String,
    /// 置信度 [0.0, 1.0] — 越高越优先展示。
    pub confidence: f32,
    /// 推荐的调度表达式（可选，覆盖模板默认 cron_expr）。
    pub recommended_schedule: Option<String>,
}

impl AutomationSuggestion {
    /// 基于用户活动模式推荐模板。
    ///
    /// 分析 `activity_pattern` 中的关键词（大小写不敏感），返回匹配的推荐列表。
    /// 关键词与模板的映射：
    /// - `memory` / `记忆` → `daily_memory_consolidation`
    /// - `evolution` / `进化` → `daily_evolution_self_check`
    /// - `review` / `回顾` → `daily_review`
    /// - `health` / `健康` → `hourly_health_check`
    /// - `backup` / `备份` → `weekly_backup`
    /// - `skill` / `技能` → `weekly_skill_cleanup`
    /// - `cache` / `缓存` → `daily_cache_cleanup`
    /// - `cost` / `费用` → `daily_cost_report`
    /// - `moc` → `daily_moc_update`
    /// - `nudge` / `推送` → `hourly_proactive_nudge`
    /// - `sync` / `同步` → `daily_sync_check`
    /// - `log` / `日志` → `weekly_log_rotation`
    ///
    /// 无匹配关键词时返回空列表。
    pub fn suggest_for_user(activity_pattern: &str) -> Vec<AutomationSuggestion> {
        let p = activity_pattern.to_lowercase();

        // 关键词 → (template_id, reason, confidence, recommended_schedule)
        // recommended_schedule = None 表示使用模板默认 cron_expr。
        let rules: &[(&str, &str, &str, f32, Option<&str>)] = &[
            (
                "memory",
                "daily_memory_consolidation",
                "检测到记忆相关活动，建议启用每日记忆合并",
                0.9,
                None,
            ),
            (
                "记忆",
                "daily_memory_consolidation",
                "检测到记忆相关活动，建议启用每日记忆合并",
                0.9,
                None,
            ),
            (
                "evolution",
                "daily_evolution_self_check",
                "检测到进化相关活动，建议启用每日进化自检",
                0.85,
                None,
            ),
            (
                "进化",
                "daily_evolution_self_check",
                "检测到进化相关活动，建议启用每日进化自检",
                0.85,
                None,
            ),
            (
                "review",
                "daily_review",
                "建议启用每日回顾以总结当日活动",
                0.7,
                None,
            ),
            (
                "回顾",
                "daily_review",
                "建议启用每日回顾以总结当日活动",
                0.7,
                None,
            ),
            (
                "health",
                "hourly_health_check",
                "建议启用每小时健康检查监控系统状态",
                0.75,
                None,
            ),
            (
                "健康",
                "hourly_health_check",
                "建议启用每小时健康检查监控系统状态",
                0.75,
                None,
            ),
            (
                "backup",
                "weekly_backup",
                "检测到备份需求，建议启用每周数据备份",
                0.95,
                None,
            ),
            (
                "备份",
                "weekly_backup",
                "检测到备份需求，建议启用每周数据备份",
                0.95,
                None,
            ),
            (
                "skill",
                "weekly_skill_cleanup",
                "建议启用每周技能清理归档过期技能",
                0.6,
                None,
            ),
            (
                "技能",
                "weekly_skill_cleanup",
                "建议启用每周技能清理归档过期技能",
                0.6,
                None,
            ),
            (
                "cache",
                "daily_cache_cleanup",
                "建议启用每日缓存清理释放存储空间",
                0.65,
                None,
            ),
            (
                "缓存",
                "daily_cache_cleanup",
                "建议启用每日缓存清理释放存储空间",
                0.65,
                None,
            ),
            (
                "cost",
                "daily_cost_report",
                "建议启用每日费用报告跟踪 Token 消耗",
                0.7,
                None,
            ),
            (
                "费用",
                "daily_cost_report",
                "建议启用每日费用报告跟踪 Token 消耗",
                0.7,
                None,
            ),
            (
                "moc",
                "daily_moc_update",
                "建议启用每日 MOC 更新重组主题索引",
                0.6,
                None,
            ),
            (
                "nudge",
                "hourly_proactive_nudge",
                "建议启用每小时主动推送提升交互体验",
                0.5,
                None,
            ),
            (
                "推送",
                "hourly_proactive_nudge",
                "建议启用每小时主动推送提升交互体验",
                0.5,
                None,
            ),
            (
                "sync",
                "daily_sync_check",
                "建议启用每日同步检查保证多端一致",
                0.8,
                None,
            ),
            (
                "同步",
                "daily_sync_check",
                "建议启用每日同步检查保证多端一致",
                0.8,
                None,
            ),
            (
                "log",
                "weekly_log_rotation",
                "建议启用每周日志轮转控制日志体积",
                0.55,
                None,
            ),
            (
                "日志",
                "weekly_log_rotation",
                "建议启用每周日志轮转控制日志体积",
                0.55,
                None,
            ),
        ];

        let mut suggestions = Vec::new();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (kw, tid, reason, conf, sched) in rules {
            if p.contains(kw) && seen.insert(tid) {
                suggestions.push(AutomationSuggestion {
                    template_id: tid.to_string(),
                    reason: reason.to_string(),
                    confidence: *conf,
                    recommended_schedule: sched.map(|s| s.to_string()),
                });
            }
        }

        // 按置信度降序排列。
        suggestions.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        suggestions
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- AutomationCategory 序列化 ----

    #[test]
    fn automation_category_all_variants_serde_snake_case() {
        // 验证所有 8 个变体序列化为 snake_case，并能正确反序列化往返。
        let cases = vec![
            (AutomationCategory::Maintenance, "\"maintenance\""),
            (AutomationCategory::Sync, "\"sync\""),
            (AutomationCategory::Backup, "\"backup\""),
            (AutomationCategory::Monitoring, "\"monitoring\""),
            (AutomationCategory::Evolution, "\"evolution\""),
            (AutomationCategory::Notification, "\"notification\""),
            (AutomationCategory::Cleanup, "\"cleanup\""),
            (AutomationCategory::Custom, "\"custom\""),
        ];
        for (variant, expected_json) in cases {
            let json = serde_json::to_string(&variant).expect("serialize ok");
            assert_eq!(json, expected_json, "serialize mismatch for {:?}", variant);
            let back: AutomationCategory = serde_json::from_str(&json).expect("deserialize ok");
            assert_eq!(back, variant, "deserialize mismatch for {:?}", variant);
        }
    }

    // ---- AutomationTemplate 序列化往返 ----

    #[test]
    fn automation_template_serde_roundtrip() {
        let template = AutomationTemplate {
            template_id: "test_template".to_string(),
            name: "测试模板".to_string(),
            description: "用于序列化测试的模板".to_string(),
            category: AutomationCategory::Custom,
            cron_expr: "0 9 * * 1-5".to_string(),
            command: "nebula".to_string(),
            args: vec!["run".to_string(), "--flag".to_string()],
            tags: vec!["test".to_string(), "serde".to_string()],
            default_enabled: true,
            required_env_vars: vec!["FOO".to_string()],
            config_schema: Some(serde_json::json!({"type": "object"})),
        };
        let json = serde_json::to_string(&template).expect("serialize ok");
        let back: AutomationTemplate = serde_json::from_str(&json).expect("deserialize ok");
        assert_eq!(back.template_id, "test_template");
        assert_eq!(back.name, "测试模板");
        assert_eq!(back.category, AutomationCategory::Custom);
        assert_eq!(back.cron_expr, "0 9 * * 1-5");
        assert_eq!(back.command, "nebula");
        assert_eq!(back.args, vec!["run", "--flag"]);
        assert_eq!(back.tags, vec!["test", "serde"]);
        assert!(back.default_enabled);
        assert_eq!(back.required_env_vars, vec!["FOO"]);
        assert!(back.config_schema.is_some());
    }

    #[test]
    fn automation_template_serde_with_defaults() {
        // 省略有默认值的字段，反序列化后应恢复默认值。
        let json = r#"{
            "template_id": "t1",
            "name": "T1",
            "description": "desc",
            "category": "custom",
            "cron_expr": "* * * * *",
            "command": "echo"
        }"#;
        let t: AutomationTemplate = serde_json::from_str(json).expect("deserialize ok");
        assert_eq!(t.template_id, "t1");
        assert!(t.args.is_empty(), "args should default to empty");
        assert!(t.tags.is_empty(), "tags should default to empty");
        assert!(
            !t.default_enabled,
            "default_enabled should default to false"
        );
        assert!(t.required_env_vars.is_empty());
        assert!(t.config_schema.is_none());
    }

    // ---- Library new 包含 12+ 模板 ----

    #[test]
    fn library_new_has_at_least_12_templates() {
        let lib = AutomationTemplateLibrary::new();
        assert!(
            lib.list().len() >= 12,
            "expected at least 12 built-in templates, got {}",
            lib.list().len()
        );
    }

    #[test]
    fn library_new_contains_all_expected_template_ids() {
        let lib = AutomationTemplateLibrary::new();
        let expected_ids = [
            "daily_memory_consolidation",
            "daily_evolution_self_check",
            "daily_review",
            "hourly_health_check",
            "weekly_backup",
            "weekly_skill_cleanup",
            "daily_cache_cleanup",
            "daily_cost_report",
            "daily_moc_update",
            "hourly_proactive_nudge",
            "daily_sync_check",
            "weekly_log_rotation",
        ];
        for id in &expected_ids {
            assert!(
                lib.get(id).is_some(),
                "expected built-in template '{id}' to exist"
            );
        }
    }

    // ---- list_by_category 筛选 ----

    #[test]
    fn library_list_by_category_filters_correctly() {
        let lib = AutomationTemplateLibrary::new();
        let evolution = lib.list_by_category(&AutomationCategory::Evolution);
        // Evolution 分类应至少包含 3 个模板（记忆合并/进化自检/MOC 更新）。
        assert!(
            evolution.len() >= 3,
            "expected at least 3 evolution templates, got {}",
            evolution.len()
        );
        // 所有返回的模板都应是 Evolution 分类。
        for t in &evolution {
            assert_eq!(t.category, AutomationCategory::Evolution);
        }
    }

    #[test]
    fn library_list_by_category_custom_is_empty_initially() {
        // 内置模板不含 Custom 分类。
        let lib = AutomationTemplateLibrary::new();
        let custom = lib.list_by_category(&AutomationCategory::Custom);
        assert!(custom.is_empty(), "no built-in custom templates expected");
    }

    // ---- get 获取单个 ----

    #[test]
    fn library_get_returns_template() {
        let lib = AutomationTemplateLibrary::new();
        let t = lib
            .get("daily_memory_consolidation")
            .expect("template exists");
        assert_eq!(t.template_id, "daily_memory_consolidation");
        assert_eq!(t.name, "每日记忆合并");
        assert_eq!(t.category, AutomationCategory::Evolution);
        assert_eq!(t.cron_expr, "0 3 * * *");
    }

    #[test]
    fn library_get_returns_none_for_unknown() {
        let lib = AutomationTemplateLibrary::new();
        assert!(lib.get("nonexistent_template").is_none());
    }

    // ---- search 搜索匹配 ----

    #[test]
    fn library_search_matches_name() {
        let lib = AutomationTemplateLibrary::new();
        // 搜索中文名 "记忆" 应匹配 "每日记忆合并"。
        let results = lib.search("记忆");
        assert!(!results.is_empty(), "search '记忆' should match");
        assert!(results
            .iter()
            .any(|t| t.template_id == "daily_memory_consolidation"));
    }

    #[test]
    fn library_search_matches_tags() {
        let lib = AutomationTemplateLibrary::new();
        // 搜索 tag "backup" 应匹配 weekly_backup。
        let results = lib.search("backup");
        assert!(!results.is_empty());
        assert!(results.iter().any(|t| t.template_id == "weekly_backup"));
    }

    #[test]
    fn library_search_matches_template_id() {
        let lib = AutomationTemplateLibrary::new();
        let results = lib.search("hourly");
        // 应匹配 hourly_health_check 和 hourly_proactive_nudge。
        assert!(results.len() >= 2);
        assert!(results
            .iter()
            .any(|t| t.template_id == "hourly_health_check"));
        assert!(results
            .iter()
            .any(|t| t.template_id == "hourly_proactive_nudge"));
    }

    #[test]
    fn library_search_case_insensitive() {
        let lib = AutomationTemplateLibrary::new();
        let upper = lib.search("BACKUP");
        let lower = lib.search("backup");
        assert_eq!(
            upper.len(),
            lower.len(),
            "search should be case-insensitive"
        );
        assert!(!upper.is_empty());
    }

    #[test]
    fn library_search_empty_query_returns_empty() {
        let lib = AutomationTemplateLibrary::new();
        assert!(lib.search("").is_empty(), "empty query should return empty");
    }

    #[test]
    fn library_search_no_match_returns_empty() {
        let lib = AutomationTemplateLibrary::new();
        let results = lib.search("zzz_no_such_keyword_zzz");
        assert!(results.is_empty(), "no match should return empty");
    }

    // ---- add / remove 自定义模板 ----

    #[test]
    fn library_add_increases_count() {
        let mut lib = AutomationTemplateLibrary::new();
        let initial = lib.list().len();
        let custom = AutomationTemplate {
            template_id: "my_custom_template".to_string(),
            name: "自定义模板".to_string(),
            description: "用户自定义".to_string(),
            category: AutomationCategory::Custom,
            cron_expr: "0 0 * * *".to_string(),
            command: "echo".to_string(),
            args: vec![],
            tags: vec!["custom".to_string()],
            default_enabled: false,
            required_env_vars: vec![],
            config_schema: None,
        };
        lib.add(custom);
        assert_eq!(lib.list().len(), initial + 1);
        assert!(lib.get("my_custom_template").is_some());
    }

    #[test]
    fn library_add_overrides_existing_id() {
        let mut lib = AutomationTemplateLibrary::new();
        let custom = AutomationTemplate {
            template_id: "daily_memory_consolidation".to_string(),
            name: "覆盖的记忆合并".to_string(),
            description: "用户覆盖".to_string(),
            category: AutomationCategory::Custom,
            cron_expr: "0 5 * * *".to_string(),
            command: "echo".to_string(),
            args: vec![],
            tags: vec![],
            default_enabled: false,
            required_env_vars: vec![],
            config_schema: None,
        };
        lib.add(custom);
        // 数量不变（覆盖而非新增）。
        let t = lib.get("daily_memory_consolidation").expect("exists");
        assert_eq!(t.name, "覆盖的记忆合并");
        assert_eq!(t.cron_expr, "0 5 * * *");
    }

    #[test]
    fn library_remove_returns_true_for_existing() {
        let mut lib = AutomationTemplateLibrary::new();
        let initial = lib.list().len();
        assert!(lib.remove("daily_review"));
        assert_eq!(lib.list().len(), initial - 1);
        assert!(lib.get("daily_review").is_none());
    }

    #[test]
    fn library_remove_returns_false_for_unknown() {
        let mut lib = AutomationTemplateLibrary::new();
        assert!(!lib.remove("nonexistent_template"));
    }

    // ---- to_cron_task_def 转换正确性 ----

    #[test]
    fn to_cron_task_def_converts_correctly() {
        let lib = AutomationTemplateLibrary::new();
        let def = lib
            .to_cron_task_def("daily_memory_consolidation")
            .expect("convert ok");
        assert_eq!(def.task_id, "daily_memory_consolidation");
        assert_eq!(def.cron_expr, "0 3 * * *");
        assert_eq!(def.task_name, "每日记忆合并");
        assert_eq!(def.task_type, CronTaskType::Recurring);
        assert_eq!(def.command, "nebula");
        assert!(!def.args.is_empty());
        assert!(def.enabled, "enabled should follow default_enabled");
        assert_eq!(def.max_retries, 3);
        assert_eq!(def.retry_delay_secs, 60);
        assert!(def.timeout_secs.is_none());
        assert!(def.last_run.is_none());
        assert!(def.next_run.is_none());
        assert!(!def.tags.is_empty());
    }

    #[test]
    fn to_cron_task_def_unknown_fails() {
        let lib = AutomationTemplateLibrary::new();
        let err = lib
            .to_cron_task_def("nonexistent")
            .expect_err("should fail");
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn to_cron_task_def_disabled_template_produces_disabled_task() {
        // weekly_skill_cleanup 的 default_enabled = false。
        let lib = AutomationTemplateLibrary::new();
        let def = lib
            .to_cron_task_def("weekly_skill_cleanup")
            .expect("convert ok");
        assert!(
            !def.enabled,
            "disabled template should produce disabled task"
        );
    }

    // ---- TemplateConfig apply_to 覆盖 ----

    #[test]
    fn template_config_apply_to_overrides_cron_expr() {
        let lib = AutomationTemplateLibrary::new();
        let template = lib.get("daily_review").expect("exists");
        let mut overrides = HashMap::new();
        overrides.insert("cron_expr".to_string(), serde_json::json!("30 22 * * *"));
        let config = TemplateConfig {
            template_id: "daily_review".to_string(),
            overrides,
            enabled: true,
        };
        let applied = config.apply_to(template);
        assert_eq!(applied.cron_expr, "30 22 * * *");
        assert_eq!(applied.template_id, "daily_review");
    }

    #[test]
    fn template_config_apply_to_overrides_args() {
        let lib = AutomationTemplateLibrary::new();
        let template = lib.get("daily_review").expect("exists");
        let mut overrides = HashMap::new();
        overrides.insert(
            "args".to_string(),
            serde_json::json!(["new", "args", "--flag"]),
        );
        let config = TemplateConfig {
            template_id: "daily_review".to_string(),
            overrides,
            enabled: true,
        };
        let applied = config.apply_to(template);
        assert_eq!(applied.args, vec!["new", "args", "--flag"]);
    }

    #[test]
    fn template_config_apply_to_enabled_overrides_default_enabled() {
        // 模板 weekly_skill_cleanup 的 default_enabled = false，
        // config.enabled = true 应覆盖为 true。
        let lib = AutomationTemplateLibrary::new();
        let template = lib.get("weekly_skill_cleanup").expect("exists");
        assert!(!template.default_enabled, "precondition: default disabled");
        let config = TemplateConfig {
            template_id: "weekly_skill_cleanup".to_string(),
            overrides: HashMap::new(),
            enabled: true,
        };
        let applied = config.apply_to(template);
        assert!(
            applied.default_enabled,
            "enabled flag should override default"
        );
    }

    #[test]
    fn template_config_apply_to_skips_type_mismatch() {
        let lib = AutomationTemplateLibrary::new();
        let template = lib.get("daily_review").expect("exists");
        let original_cron = template.cron_expr.clone();
        let mut overrides = HashMap::new();
        // 传入数字而非字符串，应被跳过。
        overrides.insert("cron_expr".to_string(), serde_json::json!(12345));
        let config = TemplateConfig {
            template_id: "daily_review".to_string(),
            overrides,
            enabled: true,
        };
        let applied = config.apply_to(template);
        assert_eq!(
            applied.cron_expr, original_cron,
            "type mismatch should be skipped"
        );
    }

    // ---- AutomationSuggestion 推荐逻辑 ----

    #[test]
    fn suggest_for_user_matches_memory_keyword() {
        let suggestions = AutomationSuggestion::suggest_for_user("user creates memory frequently");
        assert!(suggestions
            .iter()
            .any(|s| s.template_id == "daily_memory_consolidation"));
        // memory 关键词置信度 0.9。
        let memory_sug = suggestions
            .iter()
            .find(|s| s.template_id == "daily_memory_consolidation")
            .expect("should exist");
        assert!((memory_sug.confidence - 0.9).abs() < 1e-6);
        assert!(!memory_sug.reason.is_empty());
    }

    #[test]
    fn suggest_for_user_matches_chinese_keyword() {
        let suggestions = AutomationSuggestion::suggest_for_user("用户频繁进行备份操作");
        assert!(suggestions.iter().any(|s| s.template_id == "weekly_backup"));
    }

    #[test]
    fn suggest_for_user_sorted_by_confidence_desc() {
        let suggestions = AutomationSuggestion::suggest_for_user("memory backup sync cost health");
        assert!(suggestions.len() >= 2);
        for i in 1..suggestions.len() {
            assert!(
                suggestions[i - 1].confidence >= suggestions[i].confidence,
                "suggestions should be sorted by confidence desc"
            );
        }
    }

    #[test]
    fn suggest_for_user_no_match_returns_empty() {
        let suggestions = AutomationSuggestion::suggest_for_user("zzz_no_matching_keyword_zzz");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn suggest_for_user_deduplicates_template_ids() {
        // "memory" 和 "记忆" 都映射到 daily_memory_consolidation，应只出现一次。
        let suggestions = AutomationSuggestion::suggest_for_user("memory 记忆");
        let count = suggestions
            .iter()
            .filter(|s| s.template_id == "daily_memory_consolidation")
            .count();
        assert_eq!(count, 1, "duplicate template_id should be deduplicated");
    }

    // ---- default_enabled 检查 ----

    #[test]
    fn library_default_enabled_check() {
        // 验证部分模板的 default_enabled 字段值符合预期。
        let lib = AutomationTemplateLibrary::new();
        let enabled_ids = [
            "daily_memory_consolidation",
            "daily_evolution_self_check",
            "daily_review",
            "hourly_health_check",
            "weekly_backup",
            "daily_cache_cleanup",
            "daily_moc_update",
            "daily_sync_check",
            "weekly_log_rotation",
        ];
        for id in &enabled_ids {
            let t = lib.get(id).expect("template exists");
            assert!(
                t.default_enabled,
                "template '{id}' should be default_enabled=true"
            );
        }
        // 这两个默认禁用。
        assert!(
            !lib.get("weekly_skill_cleanup")
                .expect("exists")
                .default_enabled
        );
        assert!(
            !lib.get("daily_cost_report")
                .expect("exists")
                .default_enabled
        );
        assert!(
            !lib.get("hourly_proactive_nudge")
                .expect("exists")
                .default_enabled
        );
    }

    #[test]
    fn library_default_template_has_valid_cron_expr() {
        // 所有内置模板的 cron_expr 应为 5 字段非空字符串。
        let lib = AutomationTemplateLibrary::new();
        for t in lib.list() {
            assert!(
                !t.cron_expr.is_empty(),
                "cron_expr empty for {}",
                t.template_id
            );
            let fields: Vec<&str> = t.cron_expr.split_whitespace().collect();
            assert_eq!(
                fields.len(),
                5,
                "cron_expr '{}' should have 5 fields",
                t.cron_expr
            );
        }
    }
}
