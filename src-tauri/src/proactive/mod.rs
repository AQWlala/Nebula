//! T-E-D-05: Proactive Engine — 主动建议引擎。
//!
//! 监控用户活动模式,基于时间/频率/上下文主动建议操作。
//!
//! ## 核心概念
//!
//! * **活动记录**(`ActivityRecord`)— 每次用户操作(创建记忆/完成任务/运行
//!   Loop 等)记录一条,含时间戳 + 上下文字符串。
//! * **活动模式**(`ActivityPattern`)— 按 `ActivityType` 聚合后统计频率、
//!   时间跨度,推断出可能的建议动作。
//! * **规则**(`ProactiveRule`)— 用户/系统配置的触发规则,结合
//!   `RuleTrigger`(TimeBased/FrequencyBased/ContextBased/IdleBased)
//!   + `RuleCondition` 决定何时生成建议。
//! * **建议**(`ProactiveSuggestion`)— 引擎产出,含标题/描述/动作类型/
//!   置信度/触发原因;用户可 accept(执行动作)或 dismiss(忽略)。
//!
//! ## P0 范围
//!
//! 引擎在进程内运行(非持久化),活动记录和建议用 `HashMap` 存储。
//! `generate_suggestions()` 每次调用重新评估所有规则,清空旧建议后生成新集合。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// 活动类型
// ---------------------------------------------------------------------------

/// 用户活动类型。
///
/// 每种类型对应一类用户操作,引擎按类型分别记录和统计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    /// 创建记忆(写入 Memory 层)。
    MemoryCreation,
    /// 完成任务(Swarm/Plan 执行完毕)。
    TaskCompletion,
    /// 运行 Loop(触发器/自动化循环执行)。
    LoopExecution,
    /// 聊天交互(对话消息)。
    ChatInteraction,
    /// 文件操作(创建/编辑/删除)。
    FileOperation,
    /// 搜索查询(记忆/文件/Web 搜索)。
    SearchQuery,
}

impl ActivityType {
    /// 全部活动类型,按声明顺序。
    pub fn all() -> &'static [ActivityType; 6] {
        const ALL: [ActivityType; 6] = [
            ActivityType::MemoryCreation,
            ActivityType::TaskCompletion,
            ActivityType::LoopExecution,
            ActivityType::ChatInteraction,
            ActivityType::FileOperation,
            ActivityType::SearchQuery,
        ];
        &ALL
    }

    /// 英文标签(用于 UI)。
    pub fn label(self) -> &'static str {
        match self {
            ActivityType::MemoryCreation => "Memory Creation",
            ActivityType::TaskCompletion => "Task Completion",
            ActivityType::LoopExecution => "Loop Execution",
            ActivityType::ChatInteraction => "Chat Interaction",
            ActivityType::FileOperation => "File Operation",
            ActivityType::SearchQuery => "Search Query",
        }
    }

    /// 该活动类型对应的默认建议动作。
    pub fn default_action(self) -> SuggestionAction {
        match self {
            ActivityType::MemoryCreation => SuggestionAction::OpenMemory,
            ActivityType::TaskCompletion => SuggestionAction::StartTask,
            ActivityType::LoopExecution => SuggestionAction::RunLoop,
            ActivityType::ChatInteraction => SuggestionAction::StartTask,
            ActivityType::FileOperation => SuggestionAction::OptimizeMoc,
            ActivityType::SearchQuery => SuggestionAction::OpenMemory,
        }
    }
}

// ---------------------------------------------------------------------------
// 建议动作
// ---------------------------------------------------------------------------

/// 建议执行的动作类型。
///
/// 注意:使用外部标记表示(externally tagged),因为 `Custom(String)` 是
/// newtype 变体,内部标记(tag = "kind")不兼容 String 内部类型。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionAction {
    /// 打开记忆面板。
    OpenMemory,
    /// 启动新任务。
    StartTask,
    /// 运行 Loop 自动化。
    RunLoop,
    /// 优化 MOC(Map of Content)。
    OptimizeMoc,
    /// 调度备份。
    ScheduleBackup,
    /// 自定义动作(携带动作标识符)。
    Custom(String),
}

impl SuggestionAction {
    /// 动作 kind 字符串(用于持久化/日志)。
    pub fn kind_str(&self) -> &'static str {
        match self {
            SuggestionAction::OpenMemory => "open_memory",
            SuggestionAction::StartTask => "start_task",
            SuggestionAction::RunLoop => "run_loop",
            SuggestionAction::OptimizeMoc => "optimize_moc",
            SuggestionAction::ScheduleBackup => "schedule_backup",
            SuggestionAction::Custom(_) => "custom",
        }
    }
}

// ---------------------------------------------------------------------------
// 建议
// ---------------------------------------------------------------------------

/// 主动建议 — 引擎产出,展示给用户并由用户决定 accept/dismiss。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProactiveSuggestion {
    /// 建议唯一 ID(UUID v4)。
    pub id: String,
    /// 标题(简短,用于通知/列表项)。
    pub title: String,
    /// 描述(详细说明建议内容)。
    pub description: String,
    /// 建议执行的动作。
    pub action_type: SuggestionAction,
    /// 置信度 [0.0, 1.0] — 越高越优先展示。
    pub confidence: f64,
    /// 触发原因(人类可读,用于 UI 展示"为什么建议这个")。
    pub trigger_reason: String,
    /// 创建时间。
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// 活动模式
// ---------------------------------------------------------------------------

/// 活动模式 — 按 `ActivityType` 聚合后的统计结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityPattern {
    /// 活动类型。
    pub activity_type: ActivityType,
    /// 频率(次/秒)。
    pub frequency: f64,
    /// 时间跨度(秒)— 从首次到末次活动。
    pub timespan: i64,
    /// 最近一次触发时间。
    pub last_triggered: Option<DateTime<Utc>>,
    /// 推断的建议动作列表。
    pub suggested_actions: Vec<SuggestionAction>,
}

// ---------------------------------------------------------------------------
// 规则
// ---------------------------------------------------------------------------

/// 规则触发类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleTrigger {
    /// 基于时间(距上次活动超过 time_window 秒)。
    TimeBased,
    /// 基于频率(time_window 秒内活动数 >= min_count)。
    FrequencyBased,
    /// 基于上下文(活动 context 匹配 context_match)。
    ContextBased,
    /// 基于空闲(无任何活动超过 time_window 秒)。
    IdleBased,
}

/// 规则条件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleCondition {
    /// 最小活动次数(FrequencyBased 使用)。
    pub min_count: u32,
    /// 时间窗口(秒)。
    pub time_window: i64,
    /// 上下文匹配字符串(ContextBased 使用,`None` 表示不限制)。
    pub context_match: Option<String>,
}

impl Default for RuleCondition {
    fn default() -> Self {
        Self {
            min_count: 1,
            time_window: 3600, // 1 小时
            context_match: None,
        }
    }
}

/// 主动建议规则。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProactiveRule {
    /// 规则名称。
    pub name: String,
    /// 触发类型。
    pub trigger: RuleTrigger,
    /// 触发条件。
    pub condition: RuleCondition,
    /// 触发后执行的动作。
    pub action: SuggestionAction,
    /// 优先级(0-100,越高越优先)。
    pub priority: u32,
}

// ---------------------------------------------------------------------------
// 活动记录(内部)
// ---------------------------------------------------------------------------

/// 单条活动记录(内部使用,不暴露到外部 API)。
#[derive(Debug, Clone)]
struct ActivityRecord {
    timestamp: DateTime<Utc>,
    context: String,
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// 主动建议引擎。
///
/// 进程内运行,非持久化。活动记录和建议用 `HashMap` 存储。
///
/// 调用流程:
/// 1. `record_activity()` 记录用户活动;
/// 2. `set_rules()` 配置规则;
/// 3. `analyze_patterns()` 查看当前活动模式;
/// 4. `generate_suggestions()` 生成建议;
/// 5. `accept_suggestion()` / `dismiss_suggestion()` 处理用户反馈。
pub struct ProactiveEngine {
    /// 活动记录,key = ActivityType。
    activities: HashMap<ActivityType, Vec<ActivityRecord>>,
    /// 当前活跃的建议,key = suggestion id。
    suggestions: HashMap<String, ProactiveSuggestion>,
    /// 规则列表。
    rules: Vec<ProactiveRule>,
    /// 引擎创建时间(用于 idle 计算)。
    created_at: DateTime<Utc>,
    /// 最近一次活动时间(用于 idle 计算)。
    last_activity: Option<DateTime<Utc>>,
}

impl Default for ProactiveEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ProactiveEngine {
    /// 创建空引擎(无活动记录、无规则、无建议)。
    pub fn new() -> Self {
        Self {
            activities: HashMap::new(),
            suggestions: HashMap::new(),
            rules: Vec::new(),
            created_at: Utc::now(),
            last_activity: None,
        }
    }

    /// 记录一次用户活动。
    ///
    /// `context` 是活动上下文(如文件路径/任务描述/搜索关键词),
    /// 用于 ContextBased 规则匹配。
    pub fn record_activity(&mut self, activity: ActivityType, context: &str) {
        let now = Utc::now();
        let record = ActivityRecord {
            timestamp: now,
            context: context.to_string(),
        };
        self.activities.entry(activity).or_default().push(record);
        self.last_activity = Some(now);
    }

    /// 分析当前活动模式。
    ///
    /// 遍历所有已记录的活动类型,统计频率/时间跨度/推断建议动作。
    /// 无活动的类型不出现在返回列表中。
    pub fn analyze_patterns(&self) -> Vec<ActivityPattern> {
        let mut patterns = Vec::new();
        for activity_type in ActivityType::all() {
            if let Some(records) = self.activities.get(activity_type) {
                if records.is_empty() {
                    continue;
                }
                let count = records.len();
                let first = records.first().unwrap().timestamp;
                let last = records.last().unwrap().timestamp;
                let timespan = (last - first).num_seconds().max(0);
                // 频率 = 次数 / 跨度秒数;跨度为 0 时按次数/秒计。
                let frequency = if timespan > 0 {
                    count as f64 / timespan as f64
                } else {
                    count as f64
                };
                let suggested_actions = vec![activity_type.default_action()];
                patterns.push(ActivityPattern {
                    activity_type: *activity_type,
                    frequency,
                    timespan,
                    last_triggered: Some(last),
                    suggested_actions,
                });
            }
        }
        patterns
    }

    /// 生成建议。
    ///
    /// 清空旧建议,遍历所有规则,评估每条规则是否满足触发条件,
    /// 满足则生成 `ProactiveSuggestion`。返回当前活跃建议列表
    /// (按 confidence 降序排列)。
    pub fn generate_suggestions(&mut self) -> Vec<ProactiveSuggestion> {
        self.suggestions.clear();
        let now = Utc::now();
        for rule in &self.rules {
            if let Some(suggestion) = self.evaluate_rule(rule, now) {
                self.suggestions.insert(suggestion.id.clone(), suggestion);
            }
        }
        let mut list: Vec<ProactiveSuggestion> = self.suggestions.values().cloned().collect();
        list.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        list
    }

    /// 忽略建议。从活跃建议中移除指定 id。
    /// 若 id 不存在返回错误。
    pub fn dismiss_suggestion(&mut self, id: &str) -> Result<()> {
        if self.suggestions.remove(id).is_none() {
            return Err(anyhow!("suggestion not found: {}", id));
        }
        Ok(())
    }

    /// 接受建议。返回建议的动作并从活跃建议中移除。
    /// 若 id 不存在返回错误。
    pub fn accept_suggestion(&mut self, id: &str) -> Result<SuggestionAction> {
        let suggestion = self
            .suggestions
            .remove(id)
            .ok_or_else(|| anyhow!("suggestion not found: {}", id))?;
        Ok(suggestion.action_type)
    }

    /// 设置规则列表(替换现有规则)。
    pub fn set_rules(&mut self, rules: Vec<ProactiveRule>) {
        self.rules = rules;
    }

    // -- 内部辅助 ----------------------------------------------------------

    /// 评估单条规则,返回生成的建议(若触发)。
    fn evaluate_rule(
        &self,
        rule: &ProactiveRule,
        now: DateTime<Utc>,
    ) -> Option<ProactiveSuggestion> {
        let (triggered, confidence, reason) = match rule.trigger {
            RuleTrigger::FrequencyBased => {
                let count = self.count_activities_in_window(
                    now,
                    rule.condition.time_window,
                    rule.condition.context_match.as_deref(),
                );
                if count >= rule.condition.min_count {
                    let conf = (count as f64 / (rule.condition.min_count as f64 * 2.0)).min(1.0);
                    let reason = format!(
                        "在 {} 秒内记录了 {} 次活动(阈值 {})",
                        rule.condition.time_window, count, rule.condition.min_count
                    );
                    (true, conf, reason)
                } else {
                    (false, 0.0, String::new())
                }
            }
            RuleTrigger::TimeBased => {
                let elapsed = match self.last_activity {
                    Some(t) => (now - t).num_seconds(),
                    None => (now - self.created_at).num_seconds(),
                };
                if elapsed >= rule.condition.time_window {
                    let conf =
                        ((elapsed as f64 / rule.condition.time_window as f64) / 2.0).min(1.0);
                    let reason = format!(
                        "距上次活动已 {} 秒(阈值 {} 秒)",
                        elapsed, rule.condition.time_window
                    );
                    (true, conf, reason)
                } else {
                    (false, 0.0, String::new())
                }
            }
            RuleTrigger::ContextBased => {
                let matched = self.find_context_match(rule.condition.context_match.as_deref());
                if matched {
                    let conf = 0.7;
                    let reason = format!(
                        "检测到匹配上下文 \"{}\"",
                        rule.condition.context_match.as_deref().unwrap_or("*")
                    );
                    (true, conf, reason)
                } else {
                    (false, 0.0, String::new())
                }
            }
            RuleTrigger::IdleBased => {
                let idle_secs = match self.last_activity {
                    Some(t) => (now - t).num_seconds(),
                    None => (now - self.created_at).num_seconds(),
                };
                if idle_secs >= rule.condition.time_window {
                    let conf =
                        ((idle_secs as f64 / rule.condition.time_window as f64) / 2.0).min(1.0);
                    let reason = format!(
                        "用户已空闲 {} 秒(阈值 {} 秒)",
                        idle_secs, rule.condition.time_window
                    );
                    (true, conf, reason)
                } else {
                    (false, 0.0, String::new())
                }
            }
        };

        if !triggered {
            return None;
        }

        // 优先级因子:priority / 100。
        let priority_factor = rule.priority as f64 / 100.0;
        // 最终置信度 = (触发置信度 + 优先级因子) / 2,clamp [0, 1]。
        let final_confidence = ((confidence + priority_factor) / 2.0).clamp(0.0, 1.0);

        Some(ProactiveSuggestion {
            id: uuid::Uuid::new_v4().to_string(),
            title: rule.name.clone(),
            description: format!("规则 \"{}\" 触发:{}", rule.name, reason),
            action_type: rule.action.clone(),
            confidence: final_confidence,
            trigger_reason: reason,
            created_at: now,
        })
    }

    /// 统计 time_window 秒内的活动数(可选 context 过滤)。
    fn count_activities_in_window(
        &self,
        now: DateTime<Utc>,
        window_secs: i64,
        context_filter: Option<&str>,
    ) -> u32 {
        let cutoff = now - chrono::Duration::seconds(window_secs);
        let mut count = 0u32;
        for records in self.activities.values() {
            for r in records {
                if r.timestamp >= cutoff {
                    if let Some(filter) = context_filter {
                        if r.context.contains(filter) {
                            count += 1;
                        }
                    } else {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// 检查是否有活动上下文匹配指定字符串。
    /// `None` 表示不限制(有任何活动即匹配)。
    fn find_context_match(&self, pattern: Option<&str>) -> bool {
        let Some(pattern) = pattern else {
            return !self.activities.is_empty();
        };
        for records in self.activities.values() {
            for r in records {
                if r.context.contains(pattern) {
                    return true;
                }
            }
        }
        false
    }

    /// 当前活跃建议数量(测试/调试辅助)。
    pub fn active_suggestion_count(&self) -> usize {
        self.suggestions.len()
    }

    /// 当前规则数量(测试/调试辅助)。
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// 获取指定活动类型的记录数(测试/调试辅助)。
    pub fn activity_count(&self, activity_type: ActivityType) -> usize {
        self.activities
            .get(&activity_type)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- 引擎基础 ----------------------------------------------------------

    #[test]
    fn new_engine_is_empty() {
        let engine = ProactiveEngine::new();
        assert_eq!(engine.active_suggestion_count(), 0);
        assert_eq!(engine.rule_count(), 0);
        for at in ActivityType::all() {
            assert_eq!(engine.activity_count(*at), 0);
        }
    }

    #[test]
    fn default_trait_works() {
        let engine = ProactiveEngine::default();
        assert_eq!(engine.rule_count(), 0);
    }

    // -- record_activity ---------------------------------------------------

    #[test]
    fn record_activity_stores_single_record() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "created note");
        assert_eq!(engine.activity_count(ActivityType::MemoryCreation), 1);
        assert_eq!(engine.activity_count(ActivityType::ChatInteraction), 0);
    }

    #[test]
    fn record_activity_multiple_types() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "a");
        engine.record_activity(ActivityType::TaskCompletion, "b");
        engine.record_activity(ActivityType::ChatInteraction, "c");
        assert_eq!(engine.activity_count(ActivityType::MemoryCreation), 1);
        assert_eq!(engine.activity_count(ActivityType::TaskCompletion), 1);
        assert_eq!(engine.activity_count(ActivityType::ChatInteraction), 1);
    }

    #[test]
    fn record_activity_accumulates_same_type() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::FileOperation, "edit 1");
        engine.record_activity(ActivityType::FileOperation, "edit 2");
        engine.record_activity(ActivityType::FileOperation, "edit 3");
        assert_eq!(engine.activity_count(ActivityType::FileOperation), 3);
    }

    // -- analyze_patterns --------------------------------------------------

    #[test]
    fn analyze_patterns_empty_returns_empty() {
        let engine = ProactiveEngine::new();
        assert!(engine.analyze_patterns().is_empty());
    }

    #[test]
    fn analyze_patterns_returns_pattern_for_recorded_activity() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "note 1");
        let patterns = engine.analyze_patterns();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].activity_type, ActivityType::MemoryCreation);
        assert!(patterns[0].last_triggered.is_some());
        assert_eq!(
            patterns[0].suggested_actions,
            vec![SuggestionAction::OpenMemory]
        );
    }

    #[test]
    fn analyze_patterns_frequency_with_single_record() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::ChatInteraction, "hello");
        let patterns = engine.analyze_patterns();
        // 单条记录 timespan = 0,frequency = count = 1.0
        assert_eq!(patterns[0].timespan, 0);
        assert_eq!(patterns[0].frequency, 1.0);
    }

    #[test]
    fn analyze_patterns_covers_all_recorded_types() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "a");
        engine.record_activity(ActivityType::SearchQuery, "q");
        engine.record_activity(ActivityType::LoopExecution, "l");
        let patterns = engine.analyze_patterns();
        assert_eq!(patterns.len(), 3);
    }

    // -- generate_suggestions ---------------------------------------------

    #[test]
    fn generate_suggestions_empty_without_rules() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "a");
        let suggestions = engine.generate_suggestions();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn frequency_based_rule_fires_when_threshold_met() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::ChatInteraction, "msg 1");
        engine.record_activity(ActivityType::ChatInteraction, "msg 2");
        engine.record_activity(ActivityType::ChatInteraction, "msg 3");
        engine.set_rules(vec![ProactiveRule {
            name: "chat-heavy".to_string(),
            trigger: RuleTrigger::FrequencyBased,
            condition: RuleCondition {
                min_count: 3,
                time_window: 3600,
                context_match: None,
            },
            action: SuggestionAction::StartTask,
            priority: 50,
        }]);
        let suggestions = engine.generate_suggestions();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].action_type, SuggestionAction::StartTask);
        assert!(suggestions[0].confidence > 0.0);
        assert!(!suggestions[0].trigger_reason.is_empty());
    }

    #[test]
    fn frequency_based_rule_does_not_fire_below_threshold() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::ChatInteraction, "msg 1");
        engine.set_rules(vec![ProactiveRule {
            name: "chat-heavy".to_string(),
            trigger: RuleTrigger::FrequencyBased,
            condition: RuleCondition {
                min_count: 5,
                time_window: 3600,
                context_match: None,
            },
            action: SuggestionAction::StartTask,
            priority: 50,
        }]);
        let suggestions = engine.generate_suggestions();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn context_based_rule_matches_context() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::FileOperation, "edited /tmp/readme.md");
        engine.set_rules(vec![ProactiveRule {
            name: "markdown-edit".to_string(),
            trigger: RuleTrigger::ContextBased,
            condition: RuleCondition {
                min_count: 1,
                time_window: 3600,
                context_match: Some(".md".to_string()),
            },
            action: SuggestionAction::OptimizeMoc,
            priority: 70,
        }]);
        let suggestions = engine.generate_suggestions();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].action_type, SuggestionAction::OptimizeMoc);
    }

    #[test]
    fn context_based_rule_no_match_different_context() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::FileOperation, "edited /tmp/readme.txt");
        engine.set_rules(vec![ProactiveRule {
            name: "markdown-edit".to_string(),
            trigger: RuleTrigger::ContextBased,
            condition: RuleCondition {
                min_count: 1,
                time_window: 3600,
                context_match: Some(".md".to_string()),
            },
            action: SuggestionAction::OptimizeMoc,
            priority: 70,
        }]);
        let suggestions = engine.generate_suggestions();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn idle_based_rule_fires_when_no_activity() {
        let mut engine = ProactiveEngine::new();
        // 不记录任何活动 → idle 时间 = 引擎创建至今。
        // time_window = 0 确保触发。
        engine.set_rules(vec![ProactiveRule {
            name: "idle-backup".to_string(),
            trigger: RuleTrigger::IdleBased,
            condition: RuleCondition {
                min_count: 0,
                time_window: 0,
                context_match: None,
            },
            action: SuggestionAction::ScheduleBackup,
            priority: 60,
        }]);
        let suggestions = engine.generate_suggestions();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].action_type, SuggestionAction::ScheduleBackup);
    }

    #[test]
    fn generate_suggestions_sorted_by_confidence_desc() {
        let mut engine = ProactiveEngine::new();
        // 记录多次活动,让两条 FrequencyBased 规则都触发。
        for i in 0..10 {
            engine.record_activity(ActivityType::ChatInteraction, &format!("msg {}", i));
        }
        engine.record_activity(ActivityType::MemoryCreation, "note");
        engine.set_rules(vec![
            ProactiveRule {
                name: "low-priority".to_string(),
                trigger: RuleTrigger::FrequencyBased,
                condition: RuleCondition {
                    min_count: 1,
                    time_window: 3600,
                    context_match: None,
                },
                action: SuggestionAction::OpenMemory,
                priority: 10,
            },
            ProactiveRule {
                name: "high-priority".to_string(),
                trigger: RuleTrigger::FrequencyBased,
                condition: RuleCondition {
                    min_count: 1,
                    time_window: 3600,
                    context_match: None,
                },
                action: SuggestionAction::StartTask,
                priority: 90,
            },
        ]);
        let suggestions = engine.generate_suggestions();
        assert_eq!(suggestions.len(), 2);
        // 高优先级在前(confidence 更高)。
        assert!(suggestions[0].confidence >= suggestions[1].confidence);
        assert_eq!(suggestions[0].title, "high-priority");
    }

    // -- dismiss / accept --------------------------------------------------

    #[test]
    fn dismiss_suggestion_removes_it() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "a");
        engine.set_rules(vec![ProactiveRule {
            name: "suggest".to_string(),
            trigger: RuleTrigger::FrequencyBased,
            condition: RuleCondition {
                min_count: 1,
                time_window: 3600,
                context_match: None,
            },
            action: SuggestionAction::OpenMemory,
            priority: 50,
        }]);
        let suggestions = engine.generate_suggestions();
        let id = suggestions[0].id.clone();
        assert_eq!(engine.active_suggestion_count(), 1);
        engine.dismiss_suggestion(&id).unwrap();
        assert_eq!(engine.active_suggestion_count(), 0);
    }

    #[test]
    fn dismiss_suggestion_errors_on_unknown_id() {
        let mut engine = ProactiveEngine::new();
        let result = engine.dismiss_suggestion("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn accept_suggestion_returns_action_and_removes() {
        let mut engine = ProactiveEngine::new();
        engine.record_activity(ActivityType::MemoryCreation, "a");
        engine.set_rules(vec![ProactiveRule {
            name: "suggest".to_string(),
            trigger: RuleTrigger::FrequencyBased,
            condition: RuleCondition {
                min_count: 1,
                time_window: 3600,
                context_match: None,
            },
            action: SuggestionAction::OpenMemory,
            priority: 50,
        }]);
        let suggestions = engine.generate_suggestions();
        let id = suggestions[0].id.clone();
        let action = engine.accept_suggestion(&id).unwrap();
        assert_eq!(action, SuggestionAction::OpenMemory);
        assert_eq!(engine.active_suggestion_count(), 0);
    }

    #[test]
    fn accept_suggestion_errors_on_unknown_id() {
        let mut engine = ProactiveEngine::new();
        let result = engine.accept_suggestion("nonexistent");
        assert!(result.is_err());
    }

    // -- set_rules ---------------------------------------------------------

    #[test]
    fn set_rules_replaces_existing() {
        let mut engine = ProactiveEngine::new();
        engine.set_rules(vec![ProactiveRule {
            name: "rule1".to_string(),
            trigger: RuleTrigger::FrequencyBased,
            condition: RuleCondition::default(),
            action: SuggestionAction::StartTask,
            priority: 10,
        }]);
        assert_eq!(engine.rule_count(), 1);
        engine.set_rules(vec![
            ProactiveRule {
                name: "rule2".to_string(),
                trigger: RuleTrigger::TimeBased,
                condition: RuleCondition::default(),
                action: SuggestionAction::RunLoop,
                priority: 20,
            },
            ProactiveRule {
                name: "rule3".to_string(),
                trigger: RuleTrigger::IdleBased,
                condition: RuleCondition::default(),
                action: SuggestionAction::ScheduleBackup,
                priority: 30,
            },
        ]);
        assert_eq!(engine.rule_count(), 2);
    }

    // -- serde roundtrips --------------------------------------------------

    #[test]
    fn suggestion_action_serde_roundtrip() {
        let actions = vec![
            SuggestionAction::OpenMemory,
            SuggestionAction::StartTask,
            SuggestionAction::RunLoop,
            SuggestionAction::OptimizeMoc,
            SuggestionAction::ScheduleBackup,
            SuggestionAction::Custom("my-action".to_string()),
        ];
        for action in actions {
            let s = serde_json::to_string(&action).expect("serialize");
            let back: SuggestionAction = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(action, back);
        }
    }

    #[test]
    fn activity_type_serde_roundtrip() {
        for at in ActivityType::all() {
            let s = serde_json::to_string(at).expect("serialize");
            let back: ActivityType = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(*at, back);
        }
    }

    #[test]
    fn proactive_suggestion_serde_roundtrip() {
        let suggestion = ProactiveSuggestion {
            id: "test-id".to_string(),
            title: "Test".to_string(),
            description: "Test description".to_string(),
            action_type: SuggestionAction::OpenMemory,
            confidence: 0.85,
            trigger_reason: "test reason".to_string(),
            created_at: Utc::now(),
        };
        let s = serde_json::to_string(&suggestion).expect("serialize");
        let back: ProactiveSuggestion = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(suggestion, back);
    }

    #[test]
    fn proactive_rule_serde_roundtrip() {
        let rule = ProactiveRule {
            name: "test-rule".to_string(),
            trigger: RuleTrigger::FrequencyBased,
            condition: RuleCondition {
                min_count: 5,
                time_window: 1800,
                context_match: Some("meeting".to_string()),
            },
            action: SuggestionAction::Custom("notify".to_string()),
            priority: 80,
        };
        let s = serde_json::to_string(&rule).expect("serialize");
        let back: ProactiveRule = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(rule, back);
    }

    // -- 映射辅助 ----------------------------------------------------------

    #[test]
    fn default_action_mapping_covers_all_types() {
        assert_eq!(
            ActivityType::MemoryCreation.default_action(),
            SuggestionAction::OpenMemory
        );
        assert_eq!(
            ActivityType::TaskCompletion.default_action(),
            SuggestionAction::StartTask
        );
        assert_eq!(
            ActivityType::LoopExecution.default_action(),
            SuggestionAction::RunLoop
        );
        assert_eq!(
            ActivityType::ChatInteraction.default_action(),
            SuggestionAction::StartTask
        );
        assert_eq!(
            ActivityType::FileOperation.default_action(),
            SuggestionAction::OptimizeMoc
        );
        assert_eq!(
            ActivityType::SearchQuery.default_action(),
            SuggestionAction::OpenMemory
        );
    }

    #[test]
    fn suggestion_action_kind_str() {
        assert_eq!(SuggestionAction::OpenMemory.kind_str(), "open_memory");
        assert_eq!(SuggestionAction::StartTask.kind_str(), "start_task");
        assert_eq!(SuggestionAction::RunLoop.kind_str(), "run_loop");
        assert_eq!(SuggestionAction::OptimizeMoc.kind_str(), "optimize_moc");
        assert_eq!(
            SuggestionAction::ScheduleBackup.kind_str(),
            "schedule_backup"
        );
        assert_eq!(
            SuggestionAction::Custom("x".to_string()).kind_str(),
            "custom"
        );
    }
}
