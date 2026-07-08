//! T-E-A-06: Token 费用追踪。
//!
//! 每次远端 LLM 调用响应里都包含 `usage.prompt_tokens` /
//! `usage.completion_tokens`。本模块按模型单价把 token 用量换算成
//! 美元，记录到：
//!
//! * 全局原子计数器 `metrics::global().token_cost_usd`（以
//!   **micro-cent** 为单位，1 USD = 10^8 micro-cent，避免浮点）；
//! * 进程内 `CostTracker`，按日/月聚合查询。
//!
//! ## 单价表
//!
//! `model_price()` 内置几个常见模型的 (input_per_1M, output_per_1M)
//! USD 单价，未知模型返回 (0, 0)，记录但计费为 0。

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use parking_lot::Mutex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::memory::sqlite_store::SqliteStore;

/// micro-cent 与 USD 的换算因子：1 USD = 100_000_000 micro-cent。
pub const MICRO_CENTS_PER_USD: u64 = 100_000_000;

// ---------------------------------------------------------------------------
// T-E-A-12: Automation Credits — CostSource + task_local 上下文传播
// ---------------------------------------------------------------------------

/// T-E-A-12: 费用来源分类。用于把 LLM 调用费用按来源分组聚合,
/// 区分人工 Chat 与自动化(Automation/Cron/Background)产生的成本。
///
/// `#[serde(rename_all = "snake_case")]` 让序列化输出 `chat` / `automation`
/// / `cron` / `background`,与 SQLite migration 027 的 `source` 列默认值
/// `'chat'` 对齐。`#[derive(Default)]` 让 `Chat` 作为缺省值,保证旧 JSON
/// 反序列化(无 `source` 字段)时回退到 Chat。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    /// 人工 Chat 调用(用户主动发起的对话)。默认值。
    #[default]
    Chat,
    /// 事件触发器 / Swarm 等自动化动作产生的调用。
    Automation,
    /// 定时任务(Cron)产生的调用。
    Cron,
    /// 后台 worker(反思 / 黑洞压缩 / 嵌入回流等)产生的调用。
    Background,
}

impl CostSource {
    /// 返回与 SQLite `source` 列 / JSON 序列化一致的字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            CostSource::Chat => "chat",
            CostSource::Automation => "automation",
            CostSource::Cron => "cron",
            CostSource::Background => "background",
        }
    }

    /// 字符串反序列化(与 `#[serde(rename_all = "snake_case")]` 对齐)。
    /// 未知字符串回退到 Chat,保证旧数据兼容。
    pub fn from_str(s: &str) -> Self {
        match s {
            "automation" => CostSource::Automation,
            "cron" => CostSource::Cron,
            "background" => CostSource::Background,
            // "chat" 及任何未知值都回退到 Chat。
            _ => CostSource::Chat,
        }
    }
}

// T-E-A-12: task_local 容器 — 在异步任务上下文中传播当前 CostSource。
//
// `record()` / `record_with_context()` 通过 `COST_SOURCE.try_get()` 读取
// 当前来源;未设置(不在 `with_source` 上下文内)时回退到 `CostSource::Chat`。
// `COST_TRIGGER_ID` 携带触发器 ID,自动化动作执行期间设置,供 CostRecord
// 关联到具体 trigger。
//
// 注意:这两个 static 故意保持私有(不加 `pub`),因为 `pub static` 在
// `task_local!` 宏展开中会触发 rustc metadata encoder 的 ICE
// (rmeta/encoder "no entry found for key")。外部通过 `with_source` /
// `with_automation_trigger` 公共包装函数访问。
tokio::task_local! {
    /// 当前费用来源。未设置时 `try_get` 返回 Err → 默认 Chat。
    static COST_SOURCE: CostSource;
    /// 当前触发器 ID(自动化动作执行期间设置)。None 表示非触发器调用。
    static COST_TRIGGER_ID: Option<String>;
}

/// T-E-A-12: 在指定 `CostSource` 上下文中执行 `fut`。
///
/// `fut` 内部所有 `record()` 调用都会读取该 source 写入 CostRecord。
/// 用于 `ActionExecutor::dispatch` 把触发器动作执行标记为 `Automation`。
///
/// 注意:trigger_id 不通过此函数设置;如需同时关联 trigger_id,用
/// [`with_automation_trigger`]。
pub async fn with_source<F, R>(source: CostSource, fut: F) -> R
where
    F: std::future::Future<Output = R>,
{
    COST_SOURCE.scope(source, fut).await
}

/// T-E-A-12: 在 Automation 来源 + 指定 trigger_id 上下文中执行 `fut`。
///
/// 供 `ActionExecutor::dispatch` 包装触发器动作执行,让动作内部的 LLM
/// 调用经由 `CostTracker::record` 时自动归类为 Automation 来源并关联
/// trigger_id。等价于 `COST_TRIGGER_ID.scope(id, with_source(Automation, fut))`。
pub async fn with_automation_trigger<F, R>(trigger_id: Option<String>, fut: F) -> R
where
    F: std::future::Future<Output = R>,
{
    COST_TRIGGER_ID
        .scope(trigger_id, COST_SOURCE.scope(CostSource::Automation, fut))
        .await
}

/// T-E-A-12: 读取当前 task_local 的 CostSource;不在 task 上下文或未设置
/// 时返回 `CostSource::Chat`(默认值,保证向后兼容)。
fn current_source() -> CostSource {
    // tokio 1.35: `LocalKey<T>::try_get()` 对 `T: Copy` 类型直接返回
    // `Result<T, AccessError>`(owned);对非 Copy 类型返回 `Result<&T, _>`。
    // CostSource 实现 Copy,所以 `.ok()` → `Option<CostSource>`,
    // `.unwrap_or_default()` 回退到 Chat(默认值)。
    COST_SOURCE.try_get().ok().unwrap_or_default()
}

/// T-E-A-12: 读取当前 task_local 的 trigger_id;未设置时返回 None。
fn current_trigger_id() -> Option<String> {
    COST_TRIGGER_ID.try_get().ok().and_then(|v| v.clone())
}

/// T-E-A-12: 预算告警负载。当自动化当日累计费用超过阈值时由
/// `CostTracker` 通过回调发出(bootstrap 注入 `app.emit("budget_exceeded")`)。
#[derive(Debug, Clone, Serialize)]
pub struct BudgetAlert {
    /// 触发告警的来源(当前固定为 Automation)。
    pub source: CostSource,
    /// 当日(UTC)该来源累计费用(USD)。
    pub daily_cost_usd: f64,
    /// 配置的每日预算阈值(USD)。
    pub budget_usd: f64,
    /// 触发告警的 trigger_id(若适用)。
    pub trigger_id: Option<String>,
}

/// T-E-L-06: 月度 Loop 预算告警负载。当月度 Loop(Automation/Cron/Background)
/// 累计费用达到 80%(警告)或 100%(超限)时由 `CostTracker` 通过回调发出。
/// bootstrap 注入 `app.emit("loop_budget_warning" / "loop_budget_exceeded")`。
///
/// 100% 超限时 callback 仅 emit 事件;`pause_all` 由前端监听
/// `loop_budget_exceeded` 事件后调用 Tauri 命令(Task 8)执行,
/// 避免在 bootstrap 中持有 `LongTaskEngine` 引用导致循环依赖。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopBudgetAlert {
    /// 告警级别:"warning"(80%)或 "exceeded"(100%)。
    pub level: String,
    /// 当月已用 Token。
    pub used_tokens: u64,
    /// 当月已用 USD。
    pub used_usd: f64,
    /// 月度 Token 预算上限(未配置时为 0)。
    pub budget_tokens: u64,
    /// 月度 USD 预算上限(未配置时为 0.0)。
    pub budget_usd: f64,
    /// 已用百分比(0.0-1.0,可能 >1.0 表示超额)。
    pub ratio: f64,
}

/// T-E-A-12: 按来源(source)分桶的聚合结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceBucket {
    /// 来源字符串(`chat` / `automation` / `cron` / `background`)。
    pub source: String,
    /// 调用次数。
    pub calls: u64,
    /// 累计费用(USD)。
    pub cost_usd: f64,
}

/// 一条 LLM 调用费用记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    /// 模型名（如 `deepseek-chat`、`claude-3-5-haiku-20241022`）。
    pub model: String,
    /// 输入 token 数。
    pub input_tokens: u64,
    /// 输出 token 数。
    pub output_tokens: u64,
    /// 本次调用费用（USD，浮点保留 8 位精度）。
    pub cost_usd: f64,
    /// 调用时间（UTC）。
    pub timestamp: DateTime<Utc>,
    // T-E-A-07 新增（向后兼容，旧记录 None）。
    /// 调用所用的 provider（如 `deepseek`、`ollama`），未知为 None。
    #[serde(default)]
    pub provider: Option<String>,
    /// 触发本次调用的任务类型（如 `chat`、`swarm`），未知为 None。
    #[serde(default)]
    pub task: Option<String>,
    /// 触发本次调用的 agent 名，未知为 None。
    #[serde(default)]
    pub agent: Option<String>,
    // T-E-A-12 新增（向后兼容，旧记录默认 Chat / None）。
    /// 费用来源。`#[serde(default)]` 保证旧 JSON(无 source 字段)反序列化
    /// 时回退到 `CostSource::Chat`。
    #[serde(default)]
    pub source: CostSource,
    /// T-E-A-12: 关联的触发器 ID(自动化动作产生时填充),非触发器调用为 None。
    #[serde(default)]
    pub trigger_id: Option<String>,
    /// M5 #72: 关联的 WorkType 字符串("chat" / "swarm_worker" / "evolution" 等)。
    ///
    /// 与 `task` 字段区分:`task` 是任意自由文本(可能含 "chat" / "swarm" 等),
    /// `work_type` 来自 [`WorkType::as_str()`] 强类型枚举。前端可按此字段
    /// 聚合为 CreditsDashboard 分域展示(8 个 WorkType 桶)。
    ///
    /// `#[serde(default)]` 保证旧 JSON(无此字段)反序列化时回退到 None。
    /// 旧 SQLite 行(无此列)查询时通过 COALESCE 兜底为 NULL。
    #[serde(default)]
    pub work_type: Option<String>,
}

impl CostRecord {
    /// 构造一条记录并自动算出 `cost_usd`。新上下文字段设为 None
    ///（向后兼容）。source / trigger_id 由 task_local 上下文决定,
    /// 调用方通常不直接构造,而是通过 `CostTracker::record`。
    pub fn new(model: impl Into<String>, input_tokens: u64, output_tokens: u64) -> Self {
        let model = model.into();
        let cost_usd = compute_cost(&model, input_tokens, output_tokens);
        Self {
            model,
            input_tokens,
            output_tokens,
            cost_usd,
            timestamp: Utc::now(),
            provider: None,
            task: None,
            agent: None,
            source: current_source(),
            trigger_id: current_trigger_id(),
            work_type: None,
        }
    }

    /// T-E-A-07: 构造一条带上下文（provider/task/agent）的记录。
    /// T-E-A-12: source / trigger_id 自动从 task_local 读取。
    pub fn new_with_context(
        model: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        provider: Option<String>,
        task: Option<String>,
        agent: Option<String>,
    ) -> Self {
        let model = model.into();
        let cost_usd = compute_cost(&model, input_tokens, output_tokens);
        Self {
            model,
            input_tokens,
            output_tokens,
            cost_usd,
            timestamp: Utc::now(),
            provider,
            task,
            agent,
            source: current_source(),
            trigger_id: current_trigger_id(),
            work_type: None,
        }
    }

    /// M5 #72: 构造一条带 WorkType 上下文的记录（dispatch 路径专用）。
    ///
    /// `work_type_str` 来自 [`crate::llm::dispatcher::WorkType::as_str`]，
    /// 前端可按此字段聚合为 8 个 WorkType 桶（chat / swarm_worker /
    /// swarm_synthesize / master_task / evolution / soul_compile /
    /// classifier / embedding）。
    pub fn new_with_work_type(
        model: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
        provider: Option<String>,
        task: Option<String>,
        agent: Option<String>,
        work_type_str: Option<String>,
    ) -> Self {
        let mut rec =
            Self::new_with_context(model, input_tokens, output_tokens, provider, task, agent);
        rec.work_type = work_type_str;
        rec
    }

    /// 该记录所属的 UTC 日期（YYYY-MM-DD）。
    pub fn date(&self) -> NaiveDate {
        self.timestamp.date_naive()
    }

    /// 该记录所属的年月（YYYY-MM）。
    pub fn year_month(&self) -> (i32, u32) {
        (self.timestamp.year(), self.timestamp.month())
    }

    /// T-E-A-07: 该记录所属 ISO 周的周一日期（YYYY-MM-DD）。
    pub fn iso_week_start(&self) -> NaiveDate {
        let date = self.date();
        let days_since_monday = date.weekday().num_days_from_monday() as i64;
        date - chrono::Duration::days(days_since_monday)
    }
}
/// 按日聚合的结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyAggregate {
    pub date: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// 按月聚合的结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonthlyAggregate {
    pub year_month: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// T-E-A-07: 按周聚合的结果（ISO 周，周一为起点）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeeklyAggregate {
    /// 周一日期（YYYY-MM-DD）。
    pub week_start: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// T-E-A-07: 按 provider/agent 分桶的结果。
///
/// T-E-L-06: 扩展 `is_local` / `total_cost_usd` / `total_tokens` /
/// `count` 四个字段,供 `monthly_cost_by_source` 按 provider 拆分
/// 本地/云端消耗。旧字段 `calls` / `cost_usd` 保留供
/// `aggregate_by_provider` / `aggregate_by_agent` 向后兼容;新字段
/// 加 `#[serde(default)]` 保证旧 JSON 反序列化不出错。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderBucket {
    /// provider 或 agent 名（None → "unknown"）。
    pub provider: String,
    pub calls: u64,
    pub cost_usd: f64,
    /// T-E-L-06: 是否本地执行(true=ollama 本地,false=云端)。
    #[serde(default)]
    pub is_local: bool,
    /// T-E-L-06: 总成本(USD)。
    #[serde(default)]
    pub total_cost_usd: f64,
    /// T-E-L-06: 总 Token 数(input + output)。
    #[serde(default)]
    pub total_tokens: u64,
    /// T-E-L-06: 记录数。
    #[serde(default)]
    pub count: usize,
}

/// T-E-A-08: 按模型聚合的费用行（供 `cost report` 命令输出）。
#[derive(Debug, Clone, Serialize)]
pub struct ModelCostRow {
    /// 模型名（如 `deepseek-chat`）。
    pub model: String,
    /// provider 名（None → "unknown"）。
    pub provider: String,
    /// 输入 token 累计。
    pub input_tokens: u64,
    /// 输出 token 累计。
    pub output_tokens: u64,
    /// 总 token 数（input + output）。
    pub total_tokens: u64,
    /// 累计费用（USD）。
    pub cost_usd: f64,
    /// 调用次数。
    pub call_count: u32,
}

/// 进程内费用追踪器。`Arc<CostTracker>` 可挂在 `LlmGateway` 上，
/// 每次 provider 响应处理中调用 [`CostTracker::record`] 即可。
///
/// 所有记录在 `Mutex<Vec<CostRecord>>` 里。单进程生命周期内调用
/// 次数有限（每次 LLM 调用一条），无需 LRU 淘汰。
///
/// T-E-A-12: 新增 `automation_daily_budget_usd` + `budget_alert_callback`,
/// 每次 record 后检查当日 Automation 累计费用,超阈值时通过回调 emit
/// `budget_exceeded` 事件(由 bootstrap 注入 `app.emit`)。
pub struct CostTracker {
    records: Mutex<Vec<CostRecord>>,
    /// T-E-A-12: 自动化每日预算阈值(USD)。None 或 <=0 表示不限制。
    automation_daily_budget_usd: Option<f64>,
    /// T-E-A-12: 预算告警回调(由 bootstrap 注入 `app.emit`)。
    /// 用 `Arc<dyn Fn>` 而非直接持有 `tauri::AppHandle`,保持模块
    /// 与 tauri 运行时解耦,便于单测。
    budget_alert_callback: Option<Arc<dyn Fn(BudgetAlert) + Send + Sync>>,
    /// T-E-A-12: 当日已告警标记(UTC 日期),避免同一天重复 emit。
    budget_alerted_today: Mutex<Option<NaiveDate>>,
    /// T-E-A-13: 可选 SQLite 持久化后端。`None` 时退化为纯内存模式
    /// (与 T-E-A-06 行为一致,单测与未启用持久化路径使用)。
    /// `Some(store)` 时 `record_async()` 会 spawn_blocking 异步写
    /// `cost_records` 表(migration 027),`attach_store()` 启动时
    /// 回填内存。SqliteStore 内部为 `Arc<Mutex<Connection>>`,
    /// clone 廉价,与 SemanticCache::with_sqlite 同模式。
    store: Option<SqliteStore>,
    /// T-E-L-06: Loop 月度预算上限(Token)。None = 不限制。
    loop_monthly_budget_tokens: Option<u64>,
    /// T-E-L-06: Loop 月度预算上限(USD)。None = 不限制。
    loop_monthly_budget_usd: Option<f64>,
    /// T-E-L-06: Loop 月度预算告警 callback(80% 警告 / 100% 超限)。
    /// 用 `Arc<dyn Fn>` 保持与 tauri 运行时解耦(同 `budget_alert_callback`)。
    loop_budget_alert_callback: Option<Arc<dyn Fn(LoopBudgetAlert) + Send + Sync>>,
    /// T-E-L-06: 月度 warning 去重(本月已触发 warning,"YYYY-MM")。
    loop_budget_warned_this_month: Mutex<Option<String>>,
    /// T-E-L-06: 月度 exceeded 去重(本月已触发 exceeded,"YYYY-MM")。
    loop_budget_exceeded_this_month: Mutex<Option<String>>,
}

impl Default for CostTracker {
    fn default() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            automation_daily_budget_usd: None,
            budget_alert_callback: None,
            budget_alerted_today: Mutex::new(None),
            store: None,
            loop_monthly_budget_tokens: None,
            loop_monthly_budget_usd: None,
            loop_budget_alert_callback: None,
            loop_budget_warned_this_month: Mutex::new(None),
            loop_budget_exceeded_this_month: Mutex::new(None),
        }
    }
}

impl CostTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// T-E-A-12: builder 风格注入自动化每日预算 + 告警回调。
    /// bootstrap 阶段调用,把 `app.emit("budget_exceeded", alert)` 闭包传入。
    pub fn with_budget_alert(
        mut self,
        budget_usd: Option<f64>,
        callback: Arc<dyn Fn(BudgetAlert) + Send + Sync>,
    ) -> Self {
        self.automation_daily_budget_usd = budget_usd.filter(|v| *v > 0.0);
        self.budget_alert_callback = Some(callback);
        self
    }

    /// T-E-L-06: builder 风格注入 Loop 月度预算配置和告警 callback。
    ///
    /// bootstrap 阶段调用,把 `app.emit("loop_budget_warning" /
    /// "loop_budget_exceeded")` 闭包传入。`budget_tokens` / `budget_usd`
    /// 任一为 Some(且 >0)即启用对应维度的预算检查;两者都为 None
    /// 或 <=0 时即使注入了 callback 也不会触发告警(见
    /// [`check_loop_monthly_budget`](Self::check_loop_monthly_budget)
    /// 的早返回路径)。
    ///
    /// 100% 超限时 callback 仅负责 emit 事件;`pause_all` 由前端监听
    /// `loop_budget_exceeded` 事件后调用 Tauri 命令(Task 8)执行,
    /// 避免在 bootstrap 中持有 `LongTaskEngine` 引用的循环依赖。
    pub fn with_loop_budget(
        mut self,
        budget_tokens: Option<u64>,
        budget_usd: Option<f64>,
        callback: Arc<dyn Fn(LoopBudgetAlert) + Send + Sync>,
    ) -> Self {
        self.loop_monthly_budget_tokens = budget_tokens.filter(|&v| v > 0);
        self.loop_monthly_budget_usd = budget_usd.filter(|&v| v > 0.0);
        self.loop_budget_alert_callback = Some(callback);
        self
    }

    /// T-E-A-13: builder 风格注入 SQLite 持久化后端。
    ///
    /// `bootstrap_ai_core` 在构造 `CostTracker` 后调用 `.attach_store(sqlite)`,
    /// 把主 DB 的 `SqliteStore` 注入。注入后立即调用 `load_from_store_blocking()`
    /// 把 `cost_records` 表里的全部历史记录按 `id ASC` 回填到内存 `records`
    /// (id 是 AUTOINCREMENT,保证回填顺序与写入顺序一致)。load 失败仅
    /// `warn!` 不阻断启动(与 `SemanticCache::prewarm_from_store` 同模式:
    /// 持久化是 best-effort,失败时内存为空,后续 record 仍可写入)。
    ///
    /// SqliteStore 内部为 `Arc<Mutex<Connection>>`,clone 廉价,与
    /// `SemanticCache::with_sqlite` 同模式。`store: None` 时
    /// `record_async()` 退化为纯内存写入(单测路径)。
    pub fn attach_store(mut self, store: SqliteStore) -> Self {
        self.store = Some(store);
        if let Err(e) = self.load_from_store_blocking() {
            warn!(
                target: "nebula.cost",
                error = %e,
                "load cost records from store failed; starting with empty memory"
            );
        }
        self
    }

    /// T-E-A-13: 启动时从 SQLite `cost_records` 表回填内存 `records`。
    ///
    /// SELECT 全部行按 `id ASC`(AUTOINCREMENT 顺序 = 写入顺序)排序,
    /// 反序列化 `source`(JSON 字符串 → `CostSource` enum,未知值回退 Chat)
    /// 与 `timestamp`(RFC3339 字符串 → `DateTime<Utc>`,解析失败回退 now)。
    /// 直接覆盖 `inner.records`(启动时内存为空,无合并语义)。
    ///
    /// **同步**方法,仅在 `attach_store`(构造时,同步上下文)中调用。
    /// 后续 record 写入走 `record_async` 的 `spawn_blocking` 路径,
    /// 不调用此方法(避免读写竞争)。
    ///
    /// 返回回填的行数。`store: None` 时返回 Ok(0)。
    fn load_from_store_blocking(&self) -> Result<usize> {
        let store = match &self.store {
            Some(s) => s,
            None => return Ok(0),
        };
        let conn = store.raw_connection();
        let g = conn.lock();
        // M5 #72: 新增 work_type 列(migration 036)。
        // 旧库未应用 migration 036 时,查询会失败,捕获后回退到不含
        // work_type 的查询(向后兼容)。新库正常取 work_type 字段。
        let has_work_type_col: bool = g
            .prepare("SELECT work_type FROM cost_records LIMIT 0")
            .and_then(|mut s| s.execute([]).map(|_| true))
            .unwrap_or(false);
        let rows: Vec<CostRecord> = if has_work_type_col {
            // M5 #72: 用 match 替代 ? 避免 ControlFlow 临时值借用 stmt。
            // 见 lessons learned: stmt does not live long enough。
            let mut stmt = g.prepare(
                "SELECT model, input_tokens, output_tokens, cost_usd, timestamp, \
                    provider, task, agent, source, trigger_id, work_type \
             FROM cost_records ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                let ts_str: String = r.get(4)?;
                let timestamp = chrono::DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let source_str: String = r.get(8)?;
                Ok(CostRecord {
                    model: r.get(0)?,
                    input_tokens: r.get(1)?,
                    output_tokens: r.get(2)?,
                    cost_usd: r.get(3)?,
                    timestamp,
                    provider: r.get(5)?,
                    task: r.get(6)?,
                    agent: r.get(7)?,
                    source: CostSource::from_str(&source_str),
                    trigger_id: r.get(9)?,
                    work_type: r.get(10)?,
                })
            });
            match rows {
                Ok(iter) => iter.collect::<rusqlite::Result<Vec<_>>>()?,
                Err(e) => return Err(e.into()),
            }
        } else {
            tracing::warn!(
                target: "nebula.cost",
                "cost_records.work_type column missing (migration 036 not applied); \
                 loading without work_type field"
            );
            let mut stmt = g.prepare(
                "SELECT model, input_tokens, output_tokens, cost_usd, timestamp, \
                    provider, task, agent, source, trigger_id \
             FROM cost_records ORDER BY id ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                let ts_str: String = r.get(4)?;
                let timestamp = chrono::DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let source_str: String = r.get(8)?;
                Ok(CostRecord {
                    model: r.get(0)?,
                    input_tokens: r.get(1)?,
                    output_tokens: r.get(2)?,
                    cost_usd: r.get(3)?,
                    timestamp,
                    provider: r.get(5)?,
                    task: r.get(6)?,
                    agent: r.get(7)?,
                    source: CostSource::from_str(&source_str),
                    trigger_id: r.get(9)?,
                    work_type: None,
                })
            });
            match rows {
                Ok(iter) => iter.collect::<rusqlite::Result<Vec<_>>>()?,
                Err(e) => return Err(e.into()),
            }
        };
        let n = rows.len();
        let mut guard = self.records.lock();
        *guard = rows;
        Ok(n)
    }

    /// 记录一次调用。同时把费用累加进全局 `metrics::token_cost_usd`
    /// （micro-cent 单位）。
    ///
    /// T-E-A-12: source / trigger_id 从 task_local 读取(无则默认 Chat / None)。
    /// 记录后检查当日 Automation 累计费用是否超预算阈值,超则 emit 告警。
    pub fn record(&self, model: &str, input_tokens: u64, output_tokens: u64) {
        let rec = CostRecord::new(model, input_tokens, output_tokens);
        let micro_cent = usd_to_micro_cent(rec.cost_usd);
        crate::metrics::global().record_token_cost(micro_cent);
        let today_automation_cost = self.push_and_sum_today_automation(rec);
        self.maybe_emit_budget_alert(today_automation_cost);
    }

    /// T-E-A-07: 记录一次带上下文（provider/task/agent）的调用。
    /// T-E-A-12: source / trigger_id 同样从 task_local 读取。
    pub fn record_with_context(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        provider: Option<String>,
        task: Option<String>,
        agent: Option<String>,
    ) {
        let rec =
            CostRecord::new_with_context(model, input_tokens, output_tokens, provider, task, agent);
        let micro_cent = usd_to_micro_cent(rec.cost_usd);
        crate::metrics::global().record_token_cost(micro_cent);
        let today_automation_cost = self.push_and_sum_today_automation(rec);
        self.maybe_emit_budget_alert(today_automation_cost);
    }

    /// T-E-A-13: 异步持久化路径 — 写内存 + spawn_blocking 异步 INSERT
    /// 到 `cost_records` 表(migration 027)。
    ///
    /// 与现有同步 [`record`](Self::record) 区别:
    /// * 接受预构造的 `CostRecord`(调用方已设置 source / trigger_id /
    ///   timestamp),不再走 task_local;
    /// * `store: Some` 时 `spawn_blocking` 异步写 SQLite,失败仅 `warn!`
    ///   不传播(与 SemanticCache 同 best-effort 策略);
    /// * 不触发**日级** Automation 预算告警(由同步 `record` /
    ///   `record_with_context` 路径负责);
    /// * T-E-L-06: 末尾调用 [`check_loop_monthly_budget`](Self::check_loop_monthly_budget)
    ///   检查**月度** Loop 预算(80% warning / 100% exceeded)。Loop 的
    ///   Automation/Cron/Background 调用主要经由 dispatcher → `record_async`
    ///   路径写入,因此月度预算检查放在此处。
    ///
    /// **MutexGuard 不跨 await 点**:内存 push 在块作用域内完成 drop,
    /// 再进入 spawn_blocking。`parking_lot::MutexGuard` 是 `!Send`,
    /// 跨 await 会编译失败(参考 T-E-A-11 spawn_blocking 模式)。
    ///
    /// `store: None` 时退化为纯内存写入(单测 `test_cost_tracker_no_store`)。
    pub async fn record_async(&self, r: CostRecord) {
        {
            let mut guard = self.records.lock();
            guard.push(r.clone());
        }
        if let Some(store) = &self.store {
            let store = store.clone();
            let r2 = r.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = store.raw_connection();
                let g = conn.lock();
                let ts = r2.timestamp.to_rfc3339();
                // source 存储为 plain string("chat"/"automation"/"cron"/"background"),
                // 与 migration 027 的 `source TEXT NOT NULL DEFAULT 'chat'` 对齐
                // (而非 JSON 编码的 "\"automation\""),便于 SQL 查询过滤。
                let source_str = r2.source.as_str();
                // M5 #72: 尝试 INSERT 含 work_type 列;失败回退到不含 work_type 的
                // INSERT(旧库未应用 migration 036 的兼容路径)。
                let inserted_with_work_type: rusqlite::Result<usize> = g.execute(
                    "INSERT OR REPLACE INTO cost_records \
                     (model, input_tokens, output_tokens, cost_usd, timestamp, \
                      provider, task, agent, source, trigger_id, work_type) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        r2.model,
                        r2.input_tokens,
                        r2.output_tokens,
                        r2.cost_usd,
                        ts,
                        r2.provider,
                        r2.task,
                        r2.agent,
                        source_str,
                        r2.trigger_id,
                        r2.work_type,
                    ],
                );
                if let Err(e) = inserted_with_work_type {
                    // 回退到不含 work_type 的 INSERT(旧库)
                    let warn_msg = format!(
                        "sqlite insert cost_records (with work_type) failed: {e}; \
                         retrying without work_type column"
                    );
                    if let Err(e2) = g.execute(
                        "INSERT OR REPLACE INTO cost_records \
                         (model, input_tokens, output_tokens, cost_usd, timestamp, \
                          provider, task, agent, source, trigger_id) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            r2.model,
                            r2.input_tokens,
                            r2.output_tokens,
                            r2.cost_usd,
                            ts,
                            r2.provider,
                            r2.task,
                            r2.agent,
                            source_str,
                            r2.trigger_id,
                        ],
                    ) {
                        warn!(
                            target: "nebula.cost",
                            error = %e2,
                            "{warn_msg}; fallback also failed",
                        );
                    } else {
                        warn!(
                            target: "nebula.cost",
                            error = %e,
                            "sqlite insert cost_records without work_type column succeeded (legacy schema)",
                        );
                    }
                }
            })
            .await;
        }
        // T-E-L-06: 月度 Loop 预算检查(80% warning / 100% exceeded)。
        // 放在 SQLite 持久化之后,确保记录已落盘再检查。
        self.check_loop_monthly_budget();
    }

    /// T-E-A-12: 持锁 push 记录,并顺便计算当日 Automation 累计费用,
    /// 避免 parkint_lot::Mutex 重入(不调用其他用同一 records 锁的方法)。
    /// 返回 (当日 automation 费用, 今日日期)。
    fn push_and_sum_today_automation(&self, rec: CostRecord) -> (f64, NaiveDate) {
        let today = chrono::Utc::now().date_naive();
        let mut guard = self.records.lock();
        guard.push(rec);
        let sum: f64 = guard
            .iter()
            .filter(|r| r.source == CostSource::Automation && r.timestamp.date_naive() == today)
            .map(|r| r.cost_usd)
            .sum();
        (sum, today)
    }

    /// T-E-A-12: 检查当日 Automation 累计费用是否超预算阈值。
    /// 每日仅 emit 一次(用 `budget_alerted_today` 去重),UTC 跨天复位。
    /// 在 records 锁释放后调用,只锁 `budget_alerted_today`(无重入风险)。
    fn maybe_emit_budget_alert(&self, today_automation_cost: (f64, NaiveDate)) {
        let (cost, today) = today_automation_cost;
        let (budget, callback) = match (
            self.automation_daily_budget_usd,
            &self.budget_alert_callback,
        ) {
            (Some(b), Some(cb)) if b > 0.0 => (b, cb.clone()),
            _ => return,
        };
        if cost < budget {
            return;
        }
        // 每日去重:同一天只 emit 一次,跨天复位。
        let should_emit = {
            let mut guard = self.budget_alerted_today.lock();
            match *guard {
                Some(d) if d == today => false,
                _ => {
                    *guard = Some(today);
                    true
                }
            }
        };
        if should_emit {
            let trigger_id = current_trigger_id();
            callback(BudgetAlert {
                source: CostSource::Automation,
                daily_cost_usd: cost,
                budget_usd: budget,
                trigger_id,
            });
        }
    }

    /// T-E-L-06: 检查月度 Loop 预算,触发 80% 警告或 100% 超限 callback。
    ///
    /// 应在每次 [`record_async`](Self::record_async) 后调用(SQLite 持久化之后)。
    ///
    /// - **80%**(ratio ≥ 0.8):仅 emit `level="warning"`(不暂停);
    /// - **100%**(ratio ≥ 1.0):emit `level="exceeded"`,callback 内部
    ///   emit 事件;`pause_all` 由前端监听 `loop_budget_exceeded` 事件后
    ///   调用 Tauri 命令(Task 8)执行。
    ///
    /// 各级别每月去重(同月只触发一次,以 "YYYY-MM" 标记)。
    /// 若从 <80% 直接跳到 ≥100%,只 emit exceeded(优先级更高)。
    /// 无 callback 或无预算配置时直接返回,不 panic。
    fn check_loop_monthly_budget(&self) {
        let callback = match &self.loop_budget_alert_callback {
            Some(cb) => cb.clone(),
            None => return,
        };
        let budget_tokens = self.loop_monthly_budget_tokens;
        let budget_usd = self.loop_monthly_budget_usd;
        // 两个预算维度都未配置 → 不检查。
        if budget_tokens.is_none() && budget_usd.is_none() {
            return;
        }
        let (used_tokens, used_usd) = self.loop_cost_this_month();
        // 取 token 和 usd 中较高的比例作为触发依据。
        let token_ratio = budget_tokens
            .map(|b| used_tokens as f64 / b as f64)
            .unwrap_or(0.0);
        let usd_ratio = budget_usd
            .map(|b| if b > 0.0 { used_usd / b } else { 0.0 })
            .unwrap_or(0.0);
        let ratio = token_ratio.max(usd_ratio);

        // 当前年月字符串("YYYY-MM"),用于去重。跨月复位。
        let now = chrono::Utc::now();
        let this_month = format!("{:04}-{:02}", now.year(), now.month());

        // 100% 超限优先检查(避免先 emit warning 再 emit exceeded)。
        if ratio >= 1.0 {
            let should_emit = {
                let mut guard = self.loop_budget_exceeded_this_month.lock();
                match &*guard {
                    Some(m) if m == &this_month => false,
                    _ => {
                        *guard = Some(this_month.clone());
                        true
                    }
                }
            };
            if should_emit {
                callback(LoopBudgetAlert {
                    level: "exceeded".to_string(),
                    used_tokens,
                    used_usd,
                    budget_tokens: budget_tokens.unwrap_or(0),
                    budget_usd: budget_usd.unwrap_or(0.0),
                    ratio,
                });
            }
            return;
        }

        // 80% 警告检查。
        if ratio >= 0.8 {
            let should_emit = {
                let mut guard = self.loop_budget_warned_this_month.lock();
                match &*guard {
                    Some(m) if m == &this_month => false,
                    _ => {
                        *guard = Some(this_month.clone());
                        true
                    }
                }
            };
            if should_emit {
                callback(LoopBudgetAlert {
                    level: "warning".to_string(),
                    used_tokens,
                    used_usd,
                    budget_tokens: budget_tokens.unwrap_or(0),
                    budget_usd: budget_usd.unwrap_or(0.0),
                    ratio,
                });
            }
        }
    }

    /// 当前已记录的调用数。
    pub fn len(&self) -> usize {
        self.records.lock().len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.records.lock().is_empty()
    }

    /// 全部记录的快照（按时间升序）。
    pub fn all(&self) -> Vec<CostRecord> {
        self.records.lock().clone()
    }

    /// 累计总费用（USD）。
    pub fn total_cost_usd(&self) -> f64 {
        self.records.lock().iter().map(|r| r.cost_usd).sum()
    }

    /// T-E-A-05: 当日(UTC)累计费用(USD)。
    pub fn cost_today(&self) -> f64 {
        let today = chrono::Utc::now().date_naive();
        self.records
            .lock()
            .iter()
            .filter(|r| r.timestamp.date_naive() == today)
            .map(|r| r.cost_usd)
            .sum()
    }

    /// 累计 token 用量。
    pub fn total_tokens(&self) -> (u64, u64) {
        self.records.lock().iter().fold((0, 0), |(i, o), r| {
            (i + r.input_tokens, o + r.output_tokens)
        })
    }
}
impl CostTracker {
    /// 按日聚合（按日期升序）。
    pub fn aggregate_by_day(&self) -> Vec<DailyAggregate> {
        let mut map: std::collections::BTreeMap<String, DailyAggregate> =
            std::collections::BTreeMap::new();
        for r in self.records.lock().iter() {
            let key = r.date().format("%Y-%m-%d").to_string();
            let agg = map.entry(key.clone()).or_insert_with(|| DailyAggregate {
                date: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.input_tokens += r.input_tokens;
            agg.output_tokens += r.output_tokens;
            agg.cost_usd += r.cost_usd;
        }
        map.into_values().collect()
    }

    /// 按月聚合（按年月升序）。
    pub fn aggregate_by_month(&self) -> Vec<MonthlyAggregate> {
        let mut map: std::collections::BTreeMap<String, MonthlyAggregate> =
            std::collections::BTreeMap::new();
        for r in self.records.lock().iter() {
            let (y, m) = r.year_month();
            let key = format!("{y:04}-{m:02}");
            let agg = map.entry(key.clone()).or_insert_with(|| MonthlyAggregate {
                year_month: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.input_tokens += r.input_tokens;
            agg.output_tokens += r.output_tokens;
            agg.cost_usd += r.cost_usd;
        }
        map.into_values().collect()
    }

    /// T-E-A-07: 按 ISO 周聚合（以周一日期为 key，升序）。
    pub fn aggregate_by_week(&self) -> Vec<WeeklyAggregate> {
        let mut map: std::collections::BTreeMap<String, WeeklyAggregate> =
            std::collections::BTreeMap::new();
        for r in self.records.lock().iter() {
            let key = r.iso_week_start().format("%Y-%m-%d").to_string();
            let agg = map.entry(key.clone()).or_insert_with(|| WeeklyAggregate {
                week_start: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.input_tokens += r.input_tokens;
            agg.output_tokens += r.output_tokens;
            agg.cost_usd += r.cost_usd;
        }
        map.into_values().collect()
    }

    /// T-E-A-07: 按 provider 分桶（None → "unknown"），按 cost_usd 降序。
    pub fn aggregate_by_provider(&self) -> Vec<ProviderBucket> {
        let mut map: std::collections::HashMap<String, ProviderBucket> =
            std::collections::HashMap::new();
        for r in self.records.lock().iter() {
            let key = r.provider.clone().unwrap_or_else(|| "unknown".to_string());
            let agg = map.entry(key.clone()).or_insert_with(|| ProviderBucket {
                provider: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.cost_usd += r.cost_usd;
        }
        let mut out: Vec<ProviderBucket> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// T-E-A-07: 按 agent 分桶（None → "unknown"），按 cost_usd 降序。
    pub fn aggregate_by_agent(&self) -> Vec<ProviderBucket> {
        let mut map: std::collections::HashMap<String, ProviderBucket> =
            std::collections::HashMap::new();
        for r in self.records.lock().iter() {
            let key = r.agent.clone().unwrap_or_else(|| "unknown".to_string());
            let agg = map.entry(key.clone()).or_insert_with(|| ProviderBucket {
                provider: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.cost_usd += r.cost_usd;
        }
        let mut out: Vec<ProviderBucket> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// T-E-A-08: 按模型聚合费用，可按月份过滤。
    ///
    /// `month` 格式 "YYYY-MM"；`None` 表示当月。按 `CostRecord.ts` 的
    /// (year, month) 过滤。结果按 `cost_usd` 降序，便于费用报告展示。
    /// 同一模型下若存在多条记录的 provider 不一致，优先保留非 "unknown"
    /// 的 provider（避免老记录的 None 把整行打成 unknown）。
    pub fn aggregate_by_model(&self, month: Option<String>) -> Vec<ModelCostRow> {
        let now = Utc::now();
        let (target_year, target_month) = match &month {
            Some(m) => {
                let parts: Vec<&str> = m.split('-').collect();
                if parts.len() != 2 {
                    return Vec::new();
                }
                let y: i32 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                let mo: u32 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                (y, mo)
            }
            None => (now.year(), now.month()),
        };

        let mut map: std::collections::HashMap<String, ModelCostRow> =
            std::collections::HashMap::new();
        for r in self.records.lock().iter() {
            let (ry, rm) = r.year_month();
            if ry != target_year || rm != target_month {
                continue;
            }
            let provider = r.provider.clone().unwrap_or_else(|| "unknown".to_string());
            let agg = map.entry(r.model.clone()).or_insert_with(|| ModelCostRow {
                model: r.model.clone(),
                provider: provider.clone(),
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                cost_usd: 0.0,
                call_count: 0,
            });
            agg.input_tokens += r.input_tokens;
            agg.output_tokens += r.output_tokens;
            agg.total_tokens += r.input_tokens + r.output_tokens;
            agg.cost_usd += r.cost_usd;
            agg.call_count += 1;
            // 优先保留非 "unknown" 的 provider。
            if agg.provider == "unknown" && provider != "unknown" {
                agg.provider = provider;
            }
        }
        let mut out: Vec<ModelCostRow> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// T-E-A-12: 按来源(source)分桶聚合费用,可按月份过滤。
    ///
    /// `month` 格式 "YYYY-MM";`None` 表示当月。结果按 `cost_usd` 降序,
    /// 供 `cost_report group_by=source` 命令与前端 CreditsDashboard 的
    /// Chat/Automation 分栏展示使用。每个 source 桶包含 calls + cost_usd。
    pub fn aggregate_by_source(&self, month: Option<String>) -> Vec<SourceBucket> {
        let now = Utc::now();
        let (target_year, target_month) = match &month {
            Some(m) => {
                let parts: Vec<&str> = m.split('-').collect();
                if parts.len() != 2 {
                    return Vec::new();
                }
                let y: i32 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                let mo: u32 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                (y, mo)
            }
            None => (now.year(), now.month()),
        };

        let mut map: std::collections::HashMap<String, SourceBucket> =
            std::collections::HashMap::new();
        for r in self.records.lock().iter() {
            let (ry, rm) = r.year_month();
            if ry != target_year || rm != target_month {
                continue;
            }
            let key = r.source.as_str().to_string();
            let agg = map.entry(key.clone()).or_insert_with(|| SourceBucket {
                source: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.cost_usd += r.cost_usd;
        }
        let mut out: Vec<SourceBucket> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// T-E-A-12: 当日(UTC)Automation 来源累计费用(USD)。
    /// 供预算告警逻辑与外部查询使用。
    pub fn automation_cost_today(&self) -> f64 {
        let today = chrono::Utc::now().date_naive();
        self.records
            .lock()
            .iter()
            .filter(|r| r.source == CostSource::Automation && r.timestamp.date_naive() == today)
            .map(|r| r.cost_usd)
            .sum()
    }

    /// M5 #72: 按 WorkType(work_type)分桶聚合费用,可按月份过滤。
    ///
    /// `month` 格式 "YYYY-MM";`None` 表示当月。结果按 `cost_usd` 降序,
    /// 供 `cost_report group_by=work_type` 命令与前端 CreditsDashboard
    /// 分域展示使用。每个 work_type 桶包含 calls + cost_usd。
    /// work_type 为 None 的记录归入 "unknown" 桶(旧记录或未走 dispatch
    /// 路径的调用)。
    pub fn aggregate_by_work_type(&self, month: Option<String>) -> Vec<SourceBucket> {
        let now = Utc::now();
        let (target_year, target_month) = match &month {
            Some(m) => {
                let parts: Vec<&str> = m.split('-').collect();
                if parts.len() != 2 {
                    return Vec::new();
                }
                let y: i32 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                let mo: u32 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                (y, mo)
            }
            None => (now.year(), now.month()),
        };

        let mut map: std::collections::HashMap<String, SourceBucket> =
            std::collections::HashMap::new();
        for r in self.records.lock().iter() {
            let (ry, rm) = r.year_month();
            if ry != target_year || rm != target_month {
                continue;
            }
            let key = r.work_type.clone().unwrap_or_else(|| "unknown".to_string());
            let agg = map.entry(key.clone()).or_insert_with(|| SourceBucket {
                source: key,
                ..Default::default()
            });
            agg.calls += 1;
            agg.cost_usd += r.cost_usd;
        }
        let mut out: Vec<SourceBucket> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// M5 #72: 当日(UTC)指定 WorkType 累计调用次数(供 CostPolicy 的
    /// daily_task_limit 检查使用)。
    ///
    /// `work_type_str` 来自 `WorkType::as_str()`。返回当日该 work_type 的
    /// 远端调用次数(work_type 字段为 None 的旧记录不计入)。
    pub fn remote_calls_today_by_work_type(&self, work_type_str: &str) -> u32 {
        let today = chrono::Utc::now().date_naive();
        self.records
            .lock()
            .iter()
            .filter(|r| {
                r.timestamp.date_naive() == today && r.work_type.as_deref() == Some(work_type_str)
            })
            .count() as u32
    }

    /// T-E-L-06: 按月度聚合成本,按 provider 拆分本地/云端。
    ///
    /// 与 `aggregate_by_source` 不同,此方法按 provider 分桶
    /// (而非按 CostSource),用于本地/云端消耗占比分析。
    ///
    /// - provider="ollama" → 本地($0)
    /// - provider=其他 → 云端
    ///
    /// `year_month`: 格式 "YYYY-MM",None 表示当月。结果按
    /// `total_cost_usd` 降序(与 `aggregate_by_provider` 一致)。
    /// 非法月份格式返回空 Vec(不 panic)。
    pub fn monthly_cost_by_source(&self, year_month: Option<String>) -> Vec<ProviderBucket> {
        let now = Utc::now();
        let (target_year, target_month) = match &year_month {
            Some(m) => {
                let parts: Vec<&str> = m.split('-').collect();
                if parts.len() != 2 {
                    return Vec::new();
                }
                let y: i32 = match parts[0].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                let mo: u32 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => return Vec::new(),
                };
                (y, mo)
            }
            None => (now.year(), now.month()),
        };

        let mut map: std::collections::HashMap<String, ProviderBucket> =
            std::collections::HashMap::new();
        for r in self.records.lock().iter() {
            let (ry, rm) = r.year_month();
            if ry != target_year || rm != target_month {
                continue;
            }
            let key = r.provider.clone().unwrap_or_else(|| "unknown".to_string());
            let is_local = key == "ollama";
            let agg = map.entry(key.clone()).or_insert_with(|| ProviderBucket {
                provider: key.clone(),
                is_local,
                ..Default::default()
            });
            let tokens = r.input_tokens + r.output_tokens;
            // 同步旧字段,保证两套字段一致。
            agg.calls += 1;
            agg.cost_usd += r.cost_usd;
            // T-E-L-06 新字段。
            agg.total_cost_usd += r.cost_usd;
            agg.total_tokens += tokens;
            agg.count += 1;
        }
        let mut out: Vec<ProviderBucket> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.total_cost_usd
                .partial_cmp(&a.total_cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// T-E-L-06: 只读访问 Loop 月度预算配置(Task 8 命令使用)。
    ///
    /// 返回 (budget_tokens, budget_usd),两者均为 `Option`:
    /// - `Some(v)` 表示该维度已配置且 v > 0;
    /// - `None` 表示未配置或 ≤0(不限制)。
    ///
    /// 与 [`with_loop_budget`](Self::with_loop_budget) 注入的值一致
    /// (builder 内部已 filter ≤0),保证命令返回的 budget 与实际
    /// 触发告警的 threshold 同源。
    pub fn loop_budget_config(&self) -> (Option<u64>, Option<f64>) {
        (
            self.loop_monthly_budget_tokens,
            self.loop_monthly_budget_usd,
        )
    }

    /// T-E-L-06: 重置月度预算告警去重标记(Task 8 `loop_budget_reset` 命令调用)。
    ///
    /// 清零 `loop_budget_warned_this_month` / `loop_budget_exceeded_this_month`,
    /// 允许下月(或手动重置后)重新触发 warning / exceeded 事件。
    ///
    /// **不清空历史 CostRecord**(保留审计追溯),只重置告警状态。
    /// 因此重置后 `loop_cost_this_month()` 返回的累计值不变,
    /// 但 `is_warning` / `is_exceeded` 会重新基于当前累计比例计算。
    pub fn reset_loop_budget_alerts(&self) {
        *self.loop_budget_warned_this_month.lock() = None;
        *self.loop_budget_exceeded_this_month.lock() = None;
    }

    /// T-E-L-06: 当月 Loop 消耗(tokens + USD)。
    ///
    /// 聚合 CostSource::Automation + Cron + Background 三类来源
    /// (排除人工 Chat),用于月度预算检查。
    ///
    /// 返回 (total_tokens, total_cost_usd)。无记录或当月无 Loop
    /// 调用时返回 (0, 0.0)。
    pub fn loop_cost_this_month(&self) -> (u64, f64) {
        let now = Utc::now();
        let (target_year, target_month) = (now.year(), now.month());
        let mut total_tokens: u64 = 0;
        let mut total_cost_usd: f64 = 0.0;
        for r in self.records.lock().iter() {
            let (ry, rm) = r.year_month();
            if ry != target_year || rm != target_month {
                continue;
            }
            match r.source {
                CostSource::Automation | CostSource::Cron | CostSource::Background => {
                    total_tokens += r.input_tokens + r.output_tokens;
                    total_cost_usd += r.cost_usd;
                }
                CostSource::Chat => {}
            }
        }
        (total_tokens, total_cost_usd)
    }
}
/// T-E-S-41: 进程级 ModelsConfig 缓存。首次访问时从
/// `ModelsConfig::resolve_path()` 加载;后续直接复用。保存
/// models.json 后用户需重启进程(或通过命令触发——见
/// `models_config_save` 命令的实现,该命令会调用
/// `update_models_config_override()`)以让新配置生效。
static MODELS_CONFIG_CACHE: std::sync::OnceLock<crate::llm::models_config::ModelsConfig> =
    std::sync::OnceLock::new();

/// T-E-S-41: override 槽位。`models_config_save` 命令在保存后会把
/// 最新配置推进这里,让 `model_price()` 立即看到新 pricing,无需重启。
static MODELS_CONFIG_OVERRIDE: parking_lot::RwLock<
    Option<crate::llm::models_config::ModelsConfig>,
> = parking_lot::const_rwlock(None);

/// T-E-S-41: 由 `models_config_save` 命令调用,把最新保存的配置
/// 推进 override 槽位,让 `model_price()` 立即看到新 pricing。
pub fn update_models_config_override(cfg: crate::llm::models_config::ModelsConfig) {
    let mut guard = MODELS_CONFIG_OVERRIDE.write();
    *guard = Some(cfg);
}

/// 内部辅助:在缓存的 ModelsConfig 中查 model 的 pricing。
/// 跨所有 provider 查找(因为 caller 只传 model 名,不带 provider)。
fn lookup_price_in_models_config(model: &str) -> Option<(f64, f64)> {
    // 优先用 override(若 save 命令刚推过新配置)。
    let owned_override: Option<crate::llm::models_config::ModelsConfig> =
        MODELS_CONFIG_OVERRIDE.read().clone();
    let cfg_ref: &crate::llm::models_config::ModelsConfig = match owned_override.as_ref() {
        Some(c) => c,
        None => MODELS_CONFIG_CACHE.get_or_init(|| {
            let path = crate::llm::models_config::ModelsConfig::resolve_path();
            crate::llm::models_config::ModelsConfig::load(&path)
        }),
    };
    let m_lower = model.trim().to_ascii_lowercase();
    // 1) 精确匹配(大小写不敏感)。
    for p in &cfg_ref.providers {
        for m in &p.models {
            if m.id.eq_ignore_ascii_case(model.trim()) {
                if let Some(pr) = &m.pricing {
                    return Some((pr.input_usd_per_1m, pr.output_usd_per_1m));
                }
            }
        }
    }
    // 2) 前缀匹配(大小写不敏感,如 `claude-3-5-sonnet-20241022` → `claude-3-5-sonnet`)。
    for p in &cfg_ref.providers {
        for m in &p.models {
            let id_lower = m.id.to_ascii_lowercase();
            if m_lower.starts_with(&id_lower) {
                if let Some(pr) = &m.pricing {
                    return Some((pr.input_usd_per_1m, pr.output_usd_per_1m));
                }
            }
        }
    }
    None
}

/// 模型单价表：返回 (input_usd_per_1M_tokens, output_usd_per_1M_tokens)。
///
/// 价格为 2024-2025 公开牌价近似值，仅用于内部成本可观测性，
/// 不作为对账依据。未知模型返回 (0.0, 0.0)。
///
/// T-E-S-41: 优先查 `models.json`(`ModelsConfig`)里的 pricing 字段
/// ——该配置在进程首次调用时通过 `OnceLock` 缓存,允许用户在不重启
/// 的情况下通过 `models_config_save` 命令触发 override 立即生效。
/// 未命中则回退到下方硬编码表。
pub fn model_price(model: &str) -> (f64, f64) {
    // T-E-S-41: 先查 ModelsConfig 的 pricing 字段。
    if let Some((in_p, out_p)) = lookup_price_in_models_config(model) {
        return (in_p, out_p);
    }
    // 归一化：去掉前后空白，转小写比较前缀。
    let m = model.trim().to_ascii_lowercase();
    // DeepSeek
    if m.starts_with("deepseek-chat") || m.starts_with("deepseekcoder") {
        // deepseek-chat: $0.14 / 1M input, $0.28 / 1M output
        return (0.14, 0.28);
    }
    if m.starts_with("deepseek-reasoner") {
        // deepseek-reasoner: $0.55 / 1M input, $2.19 / 1M output
        return (0.55, 2.19);
    }
    // Anthropic Claude
    if m.starts_with("claude-3-5-sonnet") {
        return (3.00, 15.00);
    }
    if m.starts_with("claude-3-5-haiku") {
        return (0.80, 4.00);
    }
    if m.starts_with("claude-3-opus") {
        return (15.00, 75.00);
    }
    if m.starts_with("claude-3-haiku") {
        return (0.25, 1.25);
    }
    // OpenAI
    if m.starts_with("gpt-4o") {
        return (2.50, 10.00);
    }
    if m.starts_with("gpt-4-turbo") || m.starts_with("gpt-4-1106") {
        return (10.00, 30.00);
    }
    if m.starts_with("gpt-3.5") {
        return (0.50, 1.50);
    }
    // Ollama 本地模型：免费
    if m.starts_with("qwen")
        || m.starts_with("llama")
        || m.starts_with("mistral")
        || m.starts_with("bge")
    {
        return (0.0, 0.0);
    }
    // T-E-S-40: vLLM/LMStudio 常见本地模型前缀 — 本地部署免费。
    // llama-3.1- / llama-3.2- / gemma- 等开源权重本地推理零成本。
    if m.starts_with("llama-3.1-")
        || m.starts_with("llama-3.2-")
        || m.starts_with("llama-3.3-")
        || m.starts_with("gemma-")
        || m.starts_with("qwen2.5:")
        || m.starts_with("qwen2.5-")
    {
        return (0.0, 0.0);
    }
    // T-E-S-40: OpenRouter 远程定价(常见开源模型经 OpenRouter 转发)。
    // mistral-large 在 OpenRouter 上按 Mistral 官方价计费($2/$6 per 1M)。
    if m.starts_with("mistral-large") {
        return (2.00, 6.00);
    }
    (0.0, 0.0)
}

/// 计算一次调用的 USD 费用。
pub fn compute_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (in_p, out_p) = model_price(model);
    let i = input_tokens as f64 / 1_000_000.0;
    let o = output_tokens as f64 / 1_000_000.0;
    in_p * i + out_p * o
}

/// 把 USD 金额换算成 micro-cent（u64），向下取整。
pub fn usd_to_micro_cent(usd: f64) -> u64 {
    if usd <= 0.0 || usd.is_nan() {
        return 0;
    }
    if usd.is_infinite() {
        // +∞ 饱和到 u64::MAX(与下方 `v >= u64::MAX` 路径一致)。
        return u64::MAX;
    }
    let v = usd * (MICRO_CENTS_PER_USD as f64);
    if v >= (u64::MAX as f64) {
        u64::MAX
    } else {
        v as u64
    }
}

/// 把 micro-cent 换算回 USD（仅用于展示）。
pub fn micro_cent_to_usd(micro_cent: u64) -> f64 {
    (micro_cent as f64) / (MICRO_CENTS_PER_USD as f64)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_prices_nonzero() {
        let (i, o) = model_price("deepseek-chat");
        assert!(i > 0.0 && o > 0.0);
        let (i, o) = model_price("claude-3-5-haiku-20241022");
        assert!(i > 0.0 && o > 0.0);
    }

    #[test]
    fn unknown_model_returns_zero() {
        let (i, o) = model_price("some-internal-model");
        assert_eq!(i, 0.0);
        assert_eq!(o, 0.0);
    }

    #[test]
    fn local_models_are_free() {
        let (i, o) = model_price("qwen2.5:3b");
        assert_eq!(i, 0.0);
        assert_eq!(o, 0.0);
    }

    #[test]
    fn compute_cost_matches_table() {
        // 1M input + 1M output for deepseek-chat
        let cost = compute_cost("deepseek-chat", 1_000_000, 1_000_000);
        // 0.14 + 0.28 = 0.42
        assert!((cost - 0.42).abs() < 1e-9);
    }

    #[test]
    fn usd_to_micro_cent_roundtrip() {
        let usd = 0.42;
        let mc = usd_to_micro_cent(usd);
        assert_eq!(mc, 42_000_000);
        let back = micro_cent_to_usd(mc);
        assert!((back - usd).abs() < 1e-9);
    }

    #[test]
    fn record_accumulates_and_aggregates() {
        let tracker = CostTracker::new();
        tracker.record("deepseek-chat", 1_000_000, 500_000);
        tracker.record("deepseek-chat", 500_000, 500_000);
        assert_eq!(tracker.len(), 2);
        let (i, o) = tracker.total_tokens();
        assert_eq!(i, 1_500_000);
        assert_eq!(o, 1_000_000);
        // 0.14*1.5 + 0.28*1.0 = 0.21 + 0.28 = 0.49
        let total = tracker.total_cost_usd();
        assert!((total - 0.49).abs() < 1e-9);
        let daily = tracker.aggregate_by_day();
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].calls, 2);
        let monthly = tracker.aggregate_by_month();
        assert_eq!(monthly.len(), 1);
        assert_eq!(monthly[0].calls, 2);
    }

    #[test]
    fn micro_cent_zero_for_negative_or_nan() {
        assert_eq!(usd_to_micro_cent(-1.0), 0);
        assert_eq!(usd_to_micro_cent(f64::NAN), 0);
        assert_eq!(usd_to_micro_cent(f64::INFINITY), u64::MAX);
    }

    // ---- T-E-A-07 新增测试 ----

    #[test]
    fn test_aggregate_by_week_groups_by_iso_week() {
        // 跨两周的记录应分两组。构造三条记录分别落在两个不同的 ISO 周。
        let tracker = CostTracker::new();
        // 第一条：2025-01-06（周一）所在周
        let mut r1 = CostRecord::new("deepseek-chat", 1_000_000, 0);
        r1.timestamp = chrono::DateTime::parse_from_rfc3339("2025-01-06T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // 第二条：2025-01-13（下一周一）所在周
        let mut r2 = CostRecord::new("deepseek-chat", 1_000_000, 0);
        r2.timestamp = chrono::DateTime::parse_from_rfc3339("2025-01-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // 第三条：2025-01-08（与 r1 同周）
        let mut r3 = CostRecord::new("deepseek-chat", 1_000_000, 0);
        r3.timestamp = chrono::DateTime::parse_from_rfc3339("2025-01-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // 直接塞进 records，绕过 record() 以注入特定时间戳。
        {
            let mut guard = tracker.records.lock();
            guard.push(r1);
            guard.push(r2);
            guard.push(r3);
        }
        let weekly = tracker.aggregate_by_week();
        assert_eq!(weekly.len(), 2, "should group into 2 ISO weeks");
        // 升序：2025-01-06 在前，2025-01-13 在后
        assert_eq!(weekly[0].week_start, "2025-01-06");
        assert_eq!(weekly[0].calls, 2);
        assert_eq!(weekly[1].week_start, "2025-01-13");
        assert_eq!(weekly[1].calls, 1);
    }

    #[test]
    fn test_aggregate_by_provider_groups_unknown() {
        let tracker = CostTracker::new();
        // 两条 None provider 记录 → "unknown" 桶
        tracker.record("deepseek-chat", 100_000, 50_000);
        tracker.record("deepseek-chat", 100_000, 50_000);
        let by_provider = tracker.aggregate_by_provider();
        assert_eq!(by_provider.len(), 1);
        assert_eq!(by_provider[0].provider, "unknown");
        assert_eq!(by_provider[0].calls, 2);
    }

    #[test]
    fn test_aggregate_by_agent_groups_unknown() {
        let tracker = CostTracker::new();
        tracker.record("deepseek-chat", 100_000, 50_000);
        let by_agent = tracker.aggregate_by_agent();
        assert_eq!(by_agent.len(), 1);
        assert_eq!(by_agent[0].provider, "unknown");
        assert_eq!(by_agent[0].calls, 1);
    }

    #[test]
    fn test_record_with_context_stores_fields() {
        let tracker = CostTracker::new();
        tracker.record_with_context(
            "deepseek-chat",
            1_000_000,
            0,
            Some("deepseek".to_string()),
            Some("chat".to_string()),
            Some("orchestrator".to_string()),
        );
        let by_provider = tracker.aggregate_by_provider();
        assert!(
            by_provider
                .iter()
                .any(|b| b.provider == "deepseek" && b.calls == 1),
            "provider bucket should contain deepseek"
        );
        let by_agent = tracker.aggregate_by_agent();
        assert!(
            by_agent
                .iter()
                .any(|b| b.provider == "orchestrator" && b.calls == 1),
            "agent bucket should contain orchestrator"
        );
    }

    #[test]
    fn record_with_context_backward_compat_record_still_works() {
        // 老的 record() 接口仍可用，且 provider/agent 落到 unknown。
        let tracker = CostTracker::new();
        tracker.record("deepseek-chat", 100_000, 50_000);
        let by_provider = tracker.aggregate_by_provider();
        assert_eq!(by_provider.len(), 1);
        assert_eq!(by_provider[0].provider, "unknown");
    }

    // ---- T-E-A-05 新增测试 ----

    #[test]
    fn test_cost_today_empty() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.cost_today(), 0.0);
    }

    #[test]
    fn test_cost_today_filters_today() {
        let tracker = CostTracker::new();
        // record() 默认用 Utc::now() 作为时间戳,即当天。
        tracker.record("deepseek-chat", 1_000_000, 500_000);
        // 0.14*1 + 0.28*0.5 = 0.14 + 0.14 = 0.28
        let today = tracker.cost_today();
        assert!((today - 0.28).abs() < 1e-9, "expected 0.28, got {today}");
    }

    #[test]
    fn test_cost_today_excludes_other_days() {
        let tracker = CostTracker::new();
        // 昨天的记录(应被排除)。
        let mut r_yesterday = CostRecord::new("deepseek-chat", 1_000_000, 500_000);
        r_yesterday.timestamp = Utc::now() - chrono::Duration::days(1);
        // 今天的记录(应被计入)。
        let mut r_today = CostRecord::new("deepseek-chat", 500_000, 0);
        r_today.timestamp = Utc::now();
        {
            let mut guard = tracker.records.lock();
            guard.push(r_yesterday);
            guard.push(r_today);
        }
        // 只算今天: 0.14*0.5 + 0.28*0 = 0.07
        let today = tracker.cost_today();
        assert!((today - 0.07).abs() < 1e-9, "expected 0.07, got {today}");
    }

    // ---- T-E-A-08 新增测试：aggregate_by_model ----

    #[test]
    fn test_aggregate_by_model_empty() {
        let tracker = CostTracker::new();
        let rows = tracker.aggregate_by_model(None);
        assert!(rows.is_empty(), "empty tracker should yield no rows");
    }

    #[test]
    fn test_aggregate_by_model_single_model() {
        let tracker = CostTracker::new();
        // 两条同一模型的记录（当月）。
        tracker.record_with_context(
            "deepseek-chat",
            1_000_000,
            500_000,
            Some("deepseek".to_string()),
            None,
            None,
        );
        tracker.record_with_context(
            "deepseek-chat",
            500_000,
            500_000,
            Some("deepseek".to_string()),
            None,
            None,
        );
        let rows = tracker.aggregate_by_model(None);
        assert_eq!(rows.len(), 1, "should aggregate into 1 row");
        let r = &rows[0];
        assert_eq!(r.model, "deepseek-chat");
        assert_eq!(r.provider, "deepseek");
        assert_eq!(r.call_count, 2);
        assert_eq!(r.input_tokens, 1_500_000);
        assert_eq!(r.output_tokens, 1_000_000);
        assert_eq!(r.total_tokens, 2_500_000);
        // 0.14*1.5 + 0.28*1.0 = 0.21 + 0.28 = 0.49
        assert!(
            (r.cost_usd - 0.49).abs() < 1e-9,
            "expected 0.49, got {}",
            r.cost_usd
        );
    }

    #[test]
    fn test_aggregate_by_model_multiple_models() {
        let tracker = CostTracker::new();
        // deepseek-chat: 1M input → 0.14 USD
        tracker.record_with_context(
            "deepseek-chat",
            1_000_000,
            0,
            Some("deepseek".to_string()),
            None,
            None,
        );
        // claude-3-5-sonnet: 1M input → 3.00 USD（更贵，应排在前面）
        tracker.record_with_context(
            "claude-3-5-sonnet",
            1_000_000,
            0,
            Some("anthropic".to_string()),
            None,
            None,
        );
        let rows = tracker.aggregate_by_model(None);
        assert_eq!(rows.len(), 2, "should aggregate into 2 rows");
        // 按 cost_usd 降序：claude（3.00）在前，deepseek（0.14）在后。
        assert_eq!(rows[0].model, "claude-3-5-sonnet");
        assert_eq!(rows[0].provider, "anthropic");
        assert!((rows[0].cost_usd - 3.00).abs() < 1e-9);
        assert_eq!(rows[1].model, "deepseek-chat");
        assert_eq!(rows[1].provider, "deepseek");
        assert!((rows[1].cost_usd - 0.14).abs() < 1e-9);
    }

    #[test]
    fn test_aggregate_by_model_month_filter() {
        let tracker = CostTracker::new();
        // 当月记录（应被计入）。
        tracker.record_with_context(
            "deepseek-chat",
            1_000_000,
            0,
            Some("deepseek".to_string()),
            None,
            None,
        );
        // 上月记录（应被过滤掉）。
        let mut r_last_month = CostRecord::new_with_context(
            "deepseek-chat",
            1_000_000,
            0,
            Some("deepseek".to_string()),
            None,
            None,
        );
        let now = Utc::now();
        let last_month_date = if now.month() == 1 {
            // 1 月 → 去年 12 月
            chrono::NaiveDate::from_ymd_opt(now.year() - 1, 12, 15).unwrap()
        } else {
            chrono::NaiveDate::from_ymd_opt(now.year(), now.month() - 1, 15).unwrap()
        };
        r_last_month.timestamp = last_month_date.and_hms_opt(12, 0, 0).unwrap().and_utc();
        {
            let mut guard = tracker.records.lock();
            guard.push(r_last_month);
        }
        // 当月过滤：只保留 1 条。
        let rows_current = tracker.aggregate_by_model(None);
        assert_eq!(rows_current.len(), 1, "current month should have 1 row");
        assert_eq!(rows_current[0].call_count, 1);

        // 显式指定上月份字符串。
        let last_month_str = if now.month() == 1 {
            format!("{:04}-12", now.year() - 1)
        } else {
            format!("{:04}-{:02}", now.year(), now.month() - 1)
        };
        let rows_last = tracker.aggregate_by_model(Some(last_month_str));
        assert_eq!(rows_last.len(), 1, "last month should have 1 row");
        assert_eq!(rows_last[0].call_count, 1);
    }

    #[test]
    fn test_aggregate_by_model_invalid_month_returns_empty() {
        let tracker = CostTracker::new();
        tracker.record("deepseek-chat", 100_000, 50_000);
        // 非法月份格式应返回空 Vec（不 panic）。
        assert!(tracker
            .aggregate_by_model(Some("invalid".to_string()))
            .is_empty());
        assert!(tracker
            .aggregate_by_model(Some("2026-13".to_string()))
            .is_empty());
        assert!(tracker
            .aggregate_by_model(Some("2026".to_string()))
            .is_empty());
    }

    #[test]
    fn test_aggregate_by_model_unknown_provider_fallback() {
        let tracker = CostTracker::new();
        // 用老的 record() 接口，provider 为 None → "unknown"。
        tracker.record("deepseek-chat", 100_000, 50_000);
        let rows = tracker.aggregate_by_model(None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "unknown");
    }

    // -----------------------------------------------------------------
    // T-E-A-12: Automation Credits 新增测试
    // -----------------------------------------------------------------

    #[test]
    fn test_cost_source_serde_snake_case() {
        // 四变体序列化为 snake_case 字符串。
        let cases = [
            (CostSource::Chat, "\"chat\""),
            (CostSource::Automation, "\"automation\""),
            (CostSource::Cron, "\"cron\""),
            (CostSource::Background, "\"background\""),
        ];
        for (variant, expected) in cases {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected, "serialize {:?} → {}", variant, s);
            let back: CostSource = serde_json::from_str(&s).unwrap();
            assert_eq!(back, variant, "deserialize {}", s);
        }
    }

    #[test]
    fn test_cost_source_default_is_chat() {
        assert_eq!(CostSource::default(), CostSource::Chat);
        // from_str 对未知值回退到 Chat。
        assert_eq!(CostSource::from_str("unknown"), CostSource::Chat);
        assert_eq!(CostSource::from_str(""), CostSource::Chat);
    }

    #[test]
    fn test_cost_record_old_json_without_source_defaults_chat() {
        // 旧 JSON(无 source / trigger_id 字段)反序列化时 source 默认 Chat,
        // trigger_id 默认 None,保证向后兼容。
        let old_json = r#"{
            "model": "deepseek-chat",
            "input_tokens": 1000,
            "output_tokens": 500,
            "cost_usd": 0.001,
            "timestamp": "2025-01-06T12:00:00Z",
            "provider": "deepseek",
            "task": "chat",
            "agent": "orchestrator"
        }"#;
        let rec: CostRecord = serde_json::from_str(old_json).unwrap();
        assert_eq!(rec.model, "deepseek-chat");
        assert_eq!(
            rec.source,
            CostSource::Chat,
            "missing source must default to Chat"
        );
        assert!(
            rec.trigger_id.is_none(),
            "missing trigger_id must default to None"
        );
    }

    #[test]
    fn test_cost_record_roundtrip_with_source_and_trigger_id() {
        let mut rec = CostRecord::new("deepseek-chat", 100, 50);
        rec.source = CostSource::Automation;
        rec.trigger_id = Some("trig-123".to_string());
        let s = serde_json::to_string(&rec).unwrap();
        assert!(s.contains("\"source\":\"automation\""), "json: {s}");
        assert!(s.contains("\"trigger_id\":\"trig-123\""), "json: {s}");
        let back: CostRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back.source, CostSource::Automation);
        assert_eq!(back.trigger_id.as_deref(), Some("trig-123"));
    }

    #[test]
    fn test_record_without_task_local_defaults_chat() {
        // 不在 with_source 上下文内调用 record(),source 应为 Chat。
        let tracker = CostTracker::new();
        tracker.record("deepseek-chat", 100_000, 50_000);
        let records = tracker.all();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source, CostSource::Chat, "no task_local → Chat");
        assert!(
            records[0].trigger_id.is_none(),
            "no task_local → None trigger_id"
        );
    }

    #[tokio::test]
    async fn test_with_source_propagates_automation_to_record() {
        // 在 with_source(Automation) 上下文内 record() 应归类为 Automation。
        let tracker = Arc::new(CostTracker::new());
        let tracker_clone = Arc::clone(&tracker);
        with_source(CostSource::Automation, async move {
            tracker_clone.record("deepseek-chat", 1_000_000, 500_000);
        })
        .await;
        let records = tracker.all();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].source,
            CostSource::Automation,
            "with_source(Automation) must propagate to record"
        );
        // 聚合:source 分桶应含 automation 桶。
        let by_source = tracker.aggregate_by_source(None);
        assert!(
            by_source
                .iter()
                .any(|b| b.source == "automation" && b.calls == 1),
            "aggregate_by_source must contain automation bucket: {by_source:?}"
        );
    }

    #[tokio::test]
    async fn test_with_source_and_trigger_id_propagation() {
        // 嵌套 COST_TRIGGER_ID.scope + with_source,record 应同时记录两个字段。
        let tracker = Arc::new(CostTracker::new());
        let tracker_clone = Arc::clone(&tracker);
        let trigger_id = "trig-abc".to_string();
        COST_TRIGGER_ID
            .scope(Some(trigger_id.clone()), async {
                with_source(CostSource::Automation, async move {
                    tracker_clone.record("deepseek-chat", 1_000_000, 0);
                })
                .await
            })
            .await;
        let records = tracker.all();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source, CostSource::Automation);
        assert_eq!(records[0].trigger_id.as_deref(), Some("trig-abc"));
    }

    #[test]
    fn test_aggregate_by_source_groups_and_sorts() {
        // 构造 Chat + Automation 两类记录,验证 aggregate_by_source 分桶 + 降序。
        let tracker = CostTracker::new();
        // Chat 记录(record 无 task_local → Chat)。
        tracker.record("deepseek-chat", 1_000_000, 0);
        tracker.record("deepseek-chat", 500_000, 0);
        // 直接 push 一条 Automation 记录(绕过 task_local)。
        {
            let mut rec = CostRecord::new("claude-3-5-sonnet", 1_000_000, 0);
            rec.source = CostSource::Automation;
            rec.trigger_id = Some("t1".to_string());
            let mut guard = tracker.records.lock();
            guard.push(rec);
        }
        let by_source = tracker.aggregate_by_source(None);
        assert_eq!(
            by_source.len(),
            2,
            "should group into 2 sources: {by_source:?}"
        );
        // claude-3-5-sonnet 1M input = 3.00 USD (Automation) > deepseek 0.21 USD (Chat),
        // 所以 automation 桶应排在前。
        assert_eq!(by_source[0].source, "automation");
        assert_eq!(by_source[0].calls, 1);
        assert_eq!(by_source[1].source, "chat");
        assert_eq!(by_source[1].calls, 2);
    }

    #[test]
    fn test_aggregate_by_source_month_filter() {
        let tracker = CostTracker::new();
        // 当月 Chat 记录。
        tracker.record("deepseek-chat", 100_000, 0);
        // 上月记录(直接 push,绕过 record)。
        let mut r_last = CostRecord::new("deepseek-chat", 100_000, 0);
        let now = Utc::now();
        let last_month_date = if now.month() == 1 {
            chrono::NaiveDate::from_ymd_opt(now.year() - 1, 12, 15).unwrap()
        } else {
            chrono::NaiveDate::from_ymd_opt(now.year(), now.month() - 1, 15).unwrap()
        };
        r_last.timestamp = last_month_date.and_hms_opt(12, 0, 0).unwrap().and_utc();
        {
            let mut guard = tracker.records.lock();
            guard.push(r_last);
        }
        // 当月过滤:只保留 1 条。
        let cur = tracker.aggregate_by_source(None);
        assert_eq!(cur.len(), 1);
        assert_eq!(cur[0].source, "chat");
        assert_eq!(cur[0].calls, 1);
        // 非法月份返回空。
        assert!(tracker
            .aggregate_by_source(Some("invalid".to_string()))
            .is_empty());
    }

    #[tokio::test]
    async fn test_budget_alert_emits_when_threshold_exceeded() {
        // 注入预算 0.10 USD + 回调捕获告警。record 累计超过 0.10 时应 emit 一次。
        let alert = Arc::new(parking_lot::Mutex::new(None::<BudgetAlert>));
        let alert_cb = Arc::clone(&alert);
        let callback: Arc<dyn Fn(BudgetAlert) + Send + Sync> = Arc::new(move |a| {
            *alert_cb.lock() = Some(a);
        });
        let tracker = CostTracker::new().with_budget_alert(Some(0.10), callback);
        // Automation 来源 0.14 USD(deepseek-chat 1M input) > 0.10 阈值。
        let tracker_arc = Arc::new(tracker);
        let tracker_clone = Arc::clone(&tracker_arc);
        with_source(CostSource::Automation, async move {
            tracker_clone.record("deepseek-chat", 1_000_000, 0);
        })
        .await;
        let captured = alert.lock().clone();
        let captured =
            captured.expect("budget alert should fire when automation daily cost > 0.10");
        assert_eq!(captured.source, CostSource::Automation);
        assert!((captured.budget_usd - 0.10).abs() < 1e-9);
        assert!(
            captured.daily_cost_usd >= 0.10,
            "daily cost {} must >= budget 0.10",
            captured.daily_cost_usd
        );
    }

    #[tokio::test]
    async fn test_budget_alert_dedup_per_day() {
        // 同一天多次超阈值只 emit 一次。
        let alert_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_cb = Arc::clone(&alert_count);
        let callback: Arc<dyn Fn(BudgetAlert) + Send + Sync> = Arc::new(move |_| {
            count_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let tracker = CostTracker::new().with_budget_alert(Some(0.05), callback);
        let tracker_arc = Arc::new(tracker);
        for _ in 0..3 {
            let tracker_clone = Arc::clone(&tracker_arc);
            with_source(CostSource::Automation, async move {
                tracker_clone.record("deepseek-chat", 1_000_000, 0);
            })
            .await;
        }
        assert_eq!(
            alert_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "budget alert must emit only once per day"
        );
    }

    #[test]
    fn test_budget_alert_no_callback_no_panic() {
        // 未注入预算/回调时,record 不应 panic(默认 tracker)。
        let tracker = CostTracker::new();
        tracker.record("deepseek-chat", 1_000_000, 0); // 不 panic
        assert_eq!(tracker.len(), 1);
    }

    #[tokio::test]
    async fn test_chat_source_does_not_trigger_automation_budget_alert() {
        // Chat 来源的记录不应触发 Automation 预算告警(只统计 Automation)。
        let alert = Arc::new(parking_lot::Mutex::new(None::<BudgetAlert>));
        let alert_cb = Arc::clone(&alert);
        let callback: Arc<dyn Fn(BudgetAlert) + Send + Sync> = Arc::new(move |a| {
            *alert_cb.lock() = Some(a);
        });
        let tracker = CostTracker::new().with_budget_alert(Some(0.05), callback);
        let tracker_arc = Arc::new(tracker);
        let tracker_clone = Arc::clone(&tracker_arc);
        // 不在 with_source 内 → Chat 来源,即使费用高也不触发 Automation 告警。
        with_source(CostSource::Chat, async move {
            tracker_clone.record("deepseek-chat", 1_000_000, 0);
        })
        .await;
        assert!(
            alert.lock().is_none(),
            "Chat source must not trigger Automation budget alert"
        );
        // automation_cost_today 应为 0(Chat 记录不计入)。
        assert_eq!(tracker_arc.automation_cost_today(), 0.0);
    }

    // -----------------------------------------------------------------
    // T-E-A-13: 费用数据加密存储 单测
    // -----------------------------------------------------------------

    /// 辅助:构造一个临时 SqliteStore(migration 027 创建 cost_records 表)。
    /// 返回 (SqliteStore, PathBuf)。文件在 std::env::temp_dir() 下,
    /// 用 PID + nanos 命名保证并发安全;测试结束不主动清理(系统 temp 定期清)。
    fn make_temp_store() -> (SqliteStore, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!(
            "nebula_cost_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&tmp);
        let store = SqliteStore::open(&tmp).expect("open sqlite store for cost test");
        (store, tmp)
    }

    /// 辅助:查询 cost_records 表行数(供测试断言)。
    fn cost_records_count(store: &SqliteStore) -> i64 {
        let conn = store.raw_connection();
        let g = conn.lock();
        g.query_row("SELECT COUNT(*) FROM cost_records", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("count cost_records")
    }

    /// T1: 无 attach_store,record_async 仅入内存,不 panic。
    #[tokio::test]
    async fn test_cost_tracker_no_store() {
        let tracker = CostTracker::new();
        // store: None → record_async 只 push 内存,不 spawn_blocking,不 panic。
        let r = CostRecord::new("deepseek-chat", 1_000_000, 500_000);
        tracker.record_async(r).await;
        assert_eq!(tracker.len(), 1, "record should be in memory");
        // 再次 record 验证多次调用都安全。
        let r2 = CostRecord::new("claude-3-5-haiku", 100, 50);
        tracker.record_async(r2).await;
        assert_eq!(tracker.len(), 2, "second record should be in memory");
    }

    /// T2: attach_store 后 record_async,查询 cost_records 表有 1 行。
    #[tokio::test]
    async fn test_cost_tracker_with_store() {
        let (store, _tmp) = make_temp_store();
        let tracker = CostTracker::new().attach_store(store.clone());
        assert_eq!(tracker.len(), 0, "fresh store should have 0 records");
        let r = CostRecord::new("deepseek-chat", 1_000_000, 500_000);
        tracker.record_async(r).await;
        // spawn_blocking 完成后 SQLite 应有 1 行。
        assert_eq!(
            cost_records_count(&store),
            1,
            "cost_records table should have 1 row"
        );
        assert_eq!(tracker.len(), 1, "memory should have 1 record");
    }

    /// T3: 先 record_async 3 条,drop tracker,新构造 attach_store,
    /// load_from_store_blocking 后 records.len() == 3。
    #[tokio::test]
    async fn test_cost_tracker_load_from_store() {
        let (store, _tmp) = make_temp_store();
        // 第一阶段:写 3 条记录到 SQLite。
        {
            let tracker = CostTracker::new().attach_store(store.clone());
            tracker
                .record_async(CostRecord::new("deepseek-chat", 1_000_000, 0))
                .await;
            // 设置不同 source / trigger_id 验证反序列化 round-trip。
            let mut r2 = CostRecord::new("claude-3-5-sonnet", 500_000, 0);
            r2.source = CostSource::Automation;
            r2.trigger_id = Some("trig-abc".to_string());
            tracker.record_async(r2).await;
            let mut r3 = CostRecord::new("gpt-4o", 100_000, 50_000);
            r3.source = CostSource::Cron;
            tracker.record_async(r3).await;
            assert_eq!(
                tracker.len(),
                3,
                "first tracker should have 3 records in memory"
            );
            // tracker drop 在块结束 — SQLite 数据持久化在文件里。
        }
        // 验证 SQLite 里有 3 行。
        assert_eq!(
            cost_records_count(&store),
            3,
            "sqlite should persist 3 rows"
        );
        // 第二阶段:新构造 tracker,attach_store 触发 load_from_store_blocking。
        let tracker2 = CostTracker::new().attach_store(store.clone());
        assert_eq!(
            tracker2.len(),
            3,
            "load_from_store_blocking should backfill 3 records"
        );
        // 验证 source / trigger_id 反序列化正确(第 2 条是 Automation + trig-abc)。
        let records = tracker2.all();
        assert_eq!(records.len(), 3);
        assert_eq!(records[1].source, CostSource::Automation);
        assert_eq!(records[1].trigger_id.as_deref(), Some("trig-abc"));
        assert_eq!(records[2].source, CostSource::Cron);
        // 第 1 条 / 第 3 条 trigger_id 为 None(非触发器调用)。
        assert!(records[0].trigger_id.is_none());
        assert!(records[2].trigger_id.is_none());
    }

    /// T4: bootstrap_storage_plain — db_encryption_enabled=false 时走
    /// `SqliteStore::open` 路径(用临时 db 路径验证)。
    ///
    /// 验证:`SqliteStore::open(tmp)` 成功 + migration 027 已应用
    /// (`cost_records` 表存在,可 INSERT / SELECT)。这正是
    /// `bootstrap_storage` 在 `db_encryption_enabled=false` 分支的行为。
    #[test]
    fn test_bootstrap_storage_plain() {
        let (store, _tmp) = make_temp_store();
        // 验证 cost_records 表已建(migration 027 在 open 内部已应用)。
        assert_eq!(
            cost_records_count(&store),
            0,
            "fresh db should have 0 cost_records"
        );
        // 验证可 INSERT / SELECT(列名与 migration 027 对齐)。
        let conn = store.raw_connection();
        {
            let g = conn.lock();
            g.execute(
                "INSERT INTO cost_records \
                 (model, input_tokens, output_tokens, cost_usd, timestamp, \
                  provider, task, agent, source, trigger_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    "deepseek-chat",
                    100u64,
                    50u64,
                    0.001f64,
                    "2026-07-04T12:00:00Z",
                    None::<String>,
                    None::<String>,
                    None::<String>,
                    "chat",
                    None::<String>,
                ],
            )
            .expect("insert cost_records row");
        }
        assert_eq!(
            cost_records_count(&store),
            1,
            "after insert should have 1 row"
        );
    }

    /// T5: CostRecord 含 source / trigger_id 字段的序列化/反序列化
    /// round-trip(验证 JSON 与 SQLite TEXT 列双向兼容)。
    #[test]
    fn test_cost_record_serialization() {
        // 1. 默认 Chat 来源 + None trigger_id。
        let r1 = CostRecord::new("deepseek-chat", 100, 50);
        let s1 = serde_json::to_string(&r1).expect("serialize r1");
        let back1: CostRecord = serde_json::from_str(&s1).expect("deserialize r1");
        assert_eq!(back1.model, r1.model);
        assert_eq!(back1.input_tokens, r1.input_tokens);
        assert_eq!(back1.output_tokens, r1.output_tokens);
        assert_eq!(back1.cost_usd, r1.cost_usd);
        assert_eq!(
            back1.source,
            CostSource::Chat,
            "default source should be Chat"
        );
        assert!(
            back1.trigger_id.is_none(),
            "default trigger_id should be None"
        );

        // 2. Automation 来源 + trigger_id 全字段 round-trip。
        let mut r2 = CostRecord::new("claude-3-5-sonnet", 1_000_000, 500_000);
        r2.provider = Some("anthropic".to_string());
        r2.task = Some("swarm".to_string());
        r2.agent = Some("orchestrator".to_string());
        r2.source = CostSource::Automation;
        r2.trigger_id = Some("trig-xyz-789".to_string());
        let s2 = serde_json::to_string(&r2).expect("serialize r2");
        // JSON 应含 snake_case source 字符串 + trigger_id 字符串。
        assert!(
            s2.contains("\"source\":\"automation\""),
            "source should serialize as snake_case: {s2}"
        );
        assert!(
            s2.contains("\"trigger_id\":\"trig-xyz-789\""),
            "trigger_id should be in JSON: {s2}"
        );
        let back2: CostRecord = serde_json::from_str(&s2).expect("deserialize r2");
        assert_eq!(back2.provider.as_deref(), Some("anthropic"));
        assert_eq!(back2.task.as_deref(), Some("swarm"));
        assert_eq!(back2.agent.as_deref(), Some("orchestrator"));
        assert_eq!(back2.source, CostSource::Automation);
        assert_eq!(back2.trigger_id.as_deref(), Some("trig-xyz-789"));

        // 3. 旧 JSON(无 source / trigger_id)反序列化时回退 Chat / None。
        let old_json = r#"{
            "model": "deepseek-chat",
            "input_tokens": 1000,
            "output_tokens": 500,
            "cost_usd": 0.001,
            "timestamp": "2025-01-06T12:00:00Z",
            "provider": "deepseek",
            "task": "chat",
            "agent": "orchestrator"
        }"#;
        let back3: CostRecord = serde_json::from_str(old_json).expect("deserialize old json");
        assert_eq!(
            back3.source,
            CostSource::Chat,
            "missing source must default to Chat"
        );
        assert!(
            back3.trigger_id.is_none(),
            "missing trigger_id must default to None"
        );
    }

    // -----------------------------------------------------------------
    // T-E-L-06: monthly_cost_by_source + loop_cost_this_month 新增测试
    // -----------------------------------------------------------------

    /// 辅助:构造一条指定 provider + source + 当月时间戳的 CostRecord。
    fn make_record(
        provider: Option<&str>,
        source: CostSource,
        input: u64,
        output: u64,
    ) -> CostRecord {
        let mut r = CostRecord::new_with_context(
            "deepseek-chat",
            input,
            output,
            provider.map(|s| s.to_string()),
            None,
            None,
        );
        r.source = source;
        r
    }

    #[test]
    fn monthly_cost_by_source_groups_by_provider() {
        // 不同 provider 的记录应分到不同桶。
        let tracker = CostTracker::new();
        tracker.record_with_context(
            "deepseek-chat",
            100_000,
            50_000,
            Some("ollama".to_string()),
            None,
            None,
        );
        tracker.record_with_context(
            "deepseek-chat",
            100_000,
            50_000,
            Some("openai".to_string()),
            None,
            None,
        );
        let buckets = tracker.monthly_cost_by_source(None);
        assert_eq!(
            buckets.len(),
            2,
            "should group into 2 providers: {buckets:?}"
        );
        // 每个 provider 各 1 条记录。
        assert!(
            buckets
                .iter()
                .any(|b| b.provider == "ollama" && b.count == 1),
            "ollama bucket missing: {buckets:?}"
        );
        assert!(
            buckets
                .iter()
                .any(|b| b.provider == "openai" && b.count == 1),
            "openai bucket missing: {buckets:?}"
        );
    }

    #[test]
    fn monthly_cost_by_source_filters_by_month() {
        // 当月 + 上月记录,当月过滤只保留 1 条。
        let tracker = CostTracker::new();
        // 当月记录。
        tracker.record_with_context(
            "deepseek-chat",
            100_000,
            0,
            Some("deepseek".to_string()),
            None,
            None,
        );
        // 上月记录(直接 push,绕过 record)。
        let mut r_last = CostRecord::new_with_context(
            "deepseek-chat",
            100_000,
            0,
            Some("deepseek".to_string()),
            None,
            None,
        );
        let now = Utc::now();
        let last_month_date = if now.month() == 1 {
            chrono::NaiveDate::from_ymd_opt(now.year() - 1, 12, 15).unwrap()
        } else {
            chrono::NaiveDate::from_ymd_opt(now.year(), now.month() - 1, 15).unwrap()
        };
        r_last.timestamp = last_month_date.and_hms_opt(12, 0, 0).unwrap().and_utc();
        {
            let mut guard = tracker.records.lock();
            guard.push(r_last);
        }
        // 当月过滤:只保留 1 个桶。
        let cur = tracker.monthly_cost_by_source(None);
        assert_eq!(cur.len(), 1, "current month should have 1 bucket: {cur:?}");
        assert_eq!(cur[0].provider, "deepseek");
        assert_eq!(cur[0].count, 1);

        // 显式指定上月份字符串。
        let last_month_str = if now.month() == 1 {
            format!("{:04}-12", now.year() - 1)
        } else {
            format!("{:04}-{:02}", now.year(), now.month() - 1)
        };
        let last = tracker.monthly_cost_by_source(Some(last_month_str));
        assert_eq!(last.len(), 1, "last month should have 1 bucket: {last:?}");
        assert_eq!(last[0].count, 1);

        // 非法月份格式返回空。
        assert!(tracker
            .monthly_cost_by_source(Some("invalid".to_string()))
            .is_empty());
    }

    #[test]
    fn monthly_cost_by_source_identifies_local_vs_cloud() {
        // ollama → is_local=true,openai → is_local=false。
        let tracker = CostTracker::new();
        let r1 = make_record(Some("ollama"), CostSource::Chat, 100_000, 50_000);
        let r2 = make_record(Some("openai"), CostSource::Chat, 100_000, 50_000);
        {
            let mut guard = tracker.records.lock();
            guard.push(r1);
            guard.push(r2);
        }
        let buckets = tracker.monthly_cost_by_source(None);
        assert_eq!(buckets.len(), 2);
        let ollama_bucket = buckets
            .iter()
            .find(|b| b.provider == "ollama")
            .expect("ollama bucket should exist");
        assert!(
            ollama_bucket.is_local,
            "ollama should be local: {ollama_bucket:?}"
        );
        let openai_bucket = buckets
            .iter()
            .find(|b| b.provider == "openai")
            .expect("openai bucket should exist");
        assert!(
            !openai_bucket.is_local,
            "openai should be cloud: {openai_bucket:?}"
        );
    }

    #[test]
    fn loop_cost_this_month_excludes_chat() {
        // Chat + Automation + Cron + Background,结果只含后三者。
        let tracker = CostTracker::new();
        let r_chat = make_record(Some("deepseek"), CostSource::Chat, 1_000_000, 0);
        let r_auto = make_record(Some("deepseek"), CostSource::Automation, 1_000_000, 0);
        let r_cron = make_record(Some("deepseek"), CostSource::Cron, 1_000_000, 0);
        let r_bg = make_record(Some("deepseek"), CostSource::Background, 1_000_000, 0);
        {
            let mut guard = tracker.records.lock();
            guard.push(r_chat);
            guard.push(r_auto);
            guard.push(r_cron);
            guard.push(r_bg);
        }
        // deepseek-chat 1M input = 0.14 USD,tokens = 1M。
        // 排除 Chat 后:3 条 × 1M tokens = 3M tokens,3 × 0.14 = 0.42 USD。
        let (tokens, usd) = tracker.loop_cost_this_month();
        assert_eq!(tokens, 3_000_000, "tokens should exclude Chat: {tokens}");
        assert!(
            (usd - 0.42).abs() < 1e-9,
            "cost should be 0.42 (3 × 0.14), got {usd}"
        );
    }

    #[test]
    fn loop_cost_this_month_empty_returns_zero() {
        // 无 records 返回 (0, 0.0)。
        let tracker = CostTracker::new();
        let (tokens, usd) = tracker.loop_cost_this_month();
        assert_eq!(tokens, 0);
        assert!(
            (usd - 0.0).abs() < 1e-9,
            "empty should return 0.0 USD, got {usd}"
        );
    }

    #[test]
    fn loop_cost_this_month_aggregates_tokens_and_cost() {
        // Automation + Cron + Background 验证 tokens + usd 聚合正确。
        let tracker = CostTracker::new();
        // Automation: 1M input + 500K output → tokens=1.5M, cost=0.14+0.14=0.28
        let r_auto = make_record(Some("deepseek"), CostSource::Automation, 1_000_000, 500_000);
        // Cron: 500K input + 0 output → tokens=0.5M, cost=0.07
        let r_cron = make_record(Some("deepseek"), CostSource::Cron, 500_000, 0);
        // Background: 0 input + 500K output → tokens=0.5M, cost=0.14
        let r_bg = make_record(Some("deepseek"), CostSource::Background, 0, 500_000);
        {
            let mut guard = tracker.records.lock();
            guard.push(r_auto);
            guard.push(r_cron);
            guard.push(r_bg);
        }
        let (tokens, usd) = tracker.loop_cost_this_month();
        // 总 tokens: 1.5M + 0.5M + 0.5M = 2.5M
        assert_eq!(tokens, 2_500_000, "total tokens mismatch: {tokens}");
        // 总 cost: 0.28 + 0.07 + 0.14 = 0.49
        assert!(
            (usd - 0.49).abs() < 1e-9,
            "total cost should be 0.49, got {usd}"
        );
    }

    // -----------------------------------------------------------------
    // T-E-L-06: Loop 月度预算告警(80% warning / 100% exceeded)测试
    // -----------------------------------------------------------------

    /// 辅助:构造一个捕获 LoopBudgetAlert 的 callback,返回 (sink, callback)。
    /// sink 内部为 `Arc<Mutex<Vec<LoopBudgetAlert>>>`,callback 触发时 push。
    fn make_loop_alert_sink() -> (
        Arc<parking_lot::Mutex<Vec<LoopBudgetAlert>>>,
        Arc<dyn Fn(LoopBudgetAlert) + Send + Sync>,
    ) {
        let sink = Arc::new(parking_lot::Mutex::new(Vec::<LoopBudgetAlert>::new()));
        let sink_cb = Arc::clone(&sink);
        let callback: Arc<dyn Fn(LoopBudgetAlert) + Send + Sync> =
            Arc::new(move |alert| sink_cb.lock().push(alert));
        (sink, callback)
    }

    #[test]
    fn loop_budget_alert_serializes() {
        // LoopBudgetAlert 需实现 Serialize + Clone(任务约束)。
        let alert = LoopBudgetAlert {
            level: "warning".to_string(),
            used_tokens: 1_000_000,
            used_usd: 0.14,
            budget_tokens: 1_250_000,
            budget_usd: 0.175,
            ratio: 0.8,
        };
        let s = serde_json::to_string(&alert).expect("serialize LoopBudgetAlert");
        assert!(s.contains("\"level\":\"warning\""), "json: {s}");
        assert!(s.contains("\"used_tokens\":1000000"), "json: {s}");
        // Clone 可用(编译期保证)。
        let _cloned = alert.clone();
    }

    #[tokio::test]
    async fn loop_budget_warning_at_80_percent() {
        // budget_tokens = 1.25M → 1M tokens(1 条 Automation)= 80% → warning。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(Some(1_250_000), None, callback);
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        let alerts = sink.lock().clone();
        assert_eq!(alerts.len(), 1, "should emit 1 warning: {alerts:?}");
        assert_eq!(alerts[0].level, "warning");
        assert!(
            (alerts[0].ratio - 0.8).abs() < 1e-9,
            "ratio should be 0.8, got {}",
            alerts[0].ratio
        );
        assert_eq!(alerts[0].used_tokens, 1_000_000);
        assert_eq!(alerts[0].budget_tokens, 1_250_000);
    }

    #[tokio::test]
    async fn loop_budget_exceeded_at_100_percent() {
        // budget_tokens = 1M → 1M tokens(1 条 Automation)= 100% → exceeded。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(Some(1_000_000), None, callback);
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        let alerts = sink.lock().clone();
        assert_eq!(alerts.len(), 1, "should emit 1 exceeded: {alerts:?}");
        assert_eq!(alerts[0].level, "exceeded");
        assert!(
            (alerts[0].ratio - 1.0).abs() < 1e-9,
            "ratio should be 1.0, got {}",
            alerts[0].ratio
        );
    }

    #[tokio::test]
    async fn loop_budget_dedup_per_month() {
        // 同月 warning 只 emit 一次;后续 record 即使仍在 80-100% 区间也不重复。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(Some(1_250_000), None, callback);
        // 第 1 条:1M tokens → 80% → warning。
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        // 第 2 条:100K tokens(总 1.1M,88%)→ 仍在 warning 区间,dedup 不重复 emit。
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                100_000,
                0,
            ))
            .await;
        let alerts = sink.lock().clone();
        assert_eq!(
            alerts.len(),
            1,
            "warning should dedup per month: {alerts:?}"
        );
        assert_eq!(alerts[0].level, "warning");
    }

    #[tokio::test]
    async fn loop_budget_warning_then_exceeded_same_month() {
        // 先 80% warning,再累计到 100% exceeded(两个不同级别各 emit 一次);
        // 之后继续累计不再重复 emit exceeded(dedup)。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(Some(1_250_000), None, callback);
        // 第 1 条:1M tokens → 80% → warning。
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        // 第 2 条:500K tokens(总 1.5M,120%)→ exceeded。
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                500_000,
                0,
            ))
            .await;
        // 第 3 条:再 500K(总 2M,160%)→ dedup,不重复 emit exceeded。
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                500_000,
                0,
            ))
            .await;
        let alerts = sink.lock().clone();
        assert_eq!(
            alerts.len(),
            2,
            "should emit 1 warning + 1 exceeded: {alerts:?}"
        );
        assert_eq!(alerts[0].level, "warning");
        assert_eq!(alerts[1].level, "exceeded");
    }

    #[tokio::test]
    async fn loop_budget_no_callback_no_panic() {
        // 无 callback(默认 CostTracker),record_async 不 panic。
        let tracker = CostTracker::new();
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        assert_eq!(tracker.len(), 1);
    }

    #[tokio::test]
    async fn loop_budget_no_limit_no_check() {
        // 注入 callback 但两个预算维度都为 None → check 早返回,不 emit。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(None, None, callback);
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        assert!(
            sink.lock().is_empty(),
            "no budget configured → no alert emitted"
        );
    }

    #[tokio::test]
    async fn loop_budget_chat_source_does_not_trigger() {
        // Chat 来源不计入 loop_cost_this_month,不应触发告警。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(Some(1_000), None, callback);
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Chat,
                1_000_000,
                0,
            ))
            .await;
        assert!(
            sink.lock().is_empty(),
            "Chat source must not trigger loop budget alert"
        );
    }

    #[tokio::test]
    async fn loop_budget_usd_dimension_triggers() {
        // 仅配置 USD 预算(无 token 预算),USD 达 80% → warning。
        // deepseek-chat 1M input = 0.14 USD。budget_usd = 0.15 → 0.14/0.15 ≈ 0.933。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(None, Some(0.15), callback);
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Automation,
                1_000_000,
                0,
            ))
            .await;
        let alerts = sink.lock().clone();
        assert_eq!(
            alerts.len(),
            1,
            "USD dimension should trigger warning: {alerts:?}"
        );
        assert_eq!(alerts[0].level, "warning");
        assert!(
            alerts[0].ratio >= 0.8 && alerts[0].ratio < 1.0,
            "ratio should be in [0.8, 1.0), got {}",
            alerts[0].ratio
        );
        assert!(
            (alerts[0].budget_usd - 0.15).abs() < 1e-9,
            "budget_usd should be 0.15, got {}",
            alerts[0].budget_usd
        );
    }

    #[tokio::test]
    async fn loop_budget_cron_and_background_sources_trigger() {
        // Cron + Background 来源也计入 loop_cost_this_month,应触发告警。
        let (sink, callback) = make_loop_alert_sink();
        let tracker = CostTracker::new().with_loop_budget(Some(1_000_000), None, callback);
        // Cron 500K + Background 500K = 1M tokens → 100% → exceeded。
        tracker
            .record_async(make_record(Some("deepseek"), CostSource::Cron, 500_000, 0))
            .await;
        // 500K tokens → ratio 0.5 < 0.8,尚无告警。
        assert!(sink.lock().is_empty(), "500K tokens should not trigger yet");
        tracker
            .record_async(make_record(
                Some("deepseek"),
                CostSource::Background,
                500_000,
                0,
            ))
            .await;
        let alerts = sink.lock().clone();
        assert_eq!(alerts.len(), 1, "should emit exceeded: {alerts:?}");
        assert_eq!(alerts[0].level, "exceeded");
    }
}
