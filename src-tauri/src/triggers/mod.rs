//! T-E-S-54: 事件触发器引擎 — 文件/消息/Webhook 三种触发器统一调度。
//!
//! ## 模块结构
//!
//! * [`TriggerEngine`] — 统一调度器,管理三种触发器 worker + ActionExecutor
//! * [`TriggerConfig`] / [`TriggerCondition`] / [`TriggerAction`] — 配置模型
//! * [`store`] — SQLite CRUD(`triggers` + `trigger_fire_log` 表)
//! * [`message`] — 消息订阅器(监听 `AgentBus::subscribe_events`)
//! * [`file`] — 文件触发器(notify + mpsc + debounce,参考 `file_watcher.rs`)
//! * [`webhook`] — axum HTTP server(端口 8088,默认 127.0.0.1)
//!
//! ## 设计约束(spec §设计约束)
//!
//! 1. **去抖**:每个 trigger 有 `debounce_ms`(默认 1000ms),同 trigger 在
//!    去抖窗口内的多次匹配只触发一次。
//! 2. **递归防护**:`dispatch` 检查 payload 的 `source_trigger_id`,若等于
//!    trigger 自身 id 则跳过(防止 swarm → event → trigger → swarm 死循环)。
//! 3. **Webhook 安全**:HMAC-SHA256 签名校验(可选 secret)+ 默认仅绑
//!    `127.0.0.1` + body 1MiB 限制。
//! 4. **exec 类动作安全**:触发器触发的 Skill 自动继承 ExecApproval 流程
//!    (由 `SkillEngine` 内部处理,触发器层不再重复实现)。

pub mod file;
pub mod message;
pub mod store;
pub mod watch;
pub mod webhook;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::memory::sqlite_store::SqliteStore;
use crate::notify::NotificationService;
use crate::skills::engine::SkillEngine;
use crate::skills::types::UseSkillRequest;
use crate::swarm::bus::AgentBus;
use crate::swarm::events::SwarmEvent;
use crate::swarm::orchestrator::{SwarmOrchestrator, SwarmTask};
// T-E-A-12: 触发器动作执行期间通过 task_local 把费用归类为 Automation 来源。
// 使用 with_automation_trigger 公共包装函数(内部同时 scope COST_SOURCE +
// COST_TRIGGER_ID 两个 task_local),避免直接引用私有 static 触发 rustc ICE。
use crate::llm::cost_tracker::with_automation_trigger;

pub use store::{FireLogRow, TriggerRow, TriggerStore};

/// 默认去抖窗口(毫秒)。
const DEFAULT_DEBOUNCE_MS: u64 = 1000;

/// 默认 webhook 监听地址(避开 8080 REST / 50051 gRPC)。
pub const DEFAULT_WEBHOOK_ADDR: &str = "127.0.0.1:8088";

// ---------------------------------------------------------------------------
// 配置模型
// ---------------------------------------------------------------------------

/// 触发器种类(对应四种 worker)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TriggerKind {
    File,
    Message,
    Webhook,
    Watch,
}

impl TriggerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerKind::File => "file",
            TriggerKind::Message => "message",
            TriggerKind::Webhook => "webhook",
            TriggerKind::Watch => "watch",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "file" => Some(TriggerKind::File),
            "message" => Some(TriggerKind::Message),
            "webhook" => Some(TriggerKind::Webhook),
            "watch" => Some(TriggerKind::Watch),
            _ => None,
        }
    }
}

/// 触发条件 — 三种触发器各有一种条件变体。
///
/// 序列化为 JSON 存储在 `triggers.condition` 列。`tag = "kind"` 让前端
/// 按字段名分支渲染。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerCondition {
    /// 文件触发器:监听 `paths` 下匹配 `patterns` 的文件 `events` 类型变更。
    File {
        /// 监听目录列表(canonicalized 路径字符串)。
        paths: Vec<String>,
        /// glob 模式列表,如 `["*.md", "*.txt"]`。空表示匹配所有文件。
        #[serde(default)]
        patterns: Vec<String>,
        /// 关心的事件类型,如 `["create", "modify"]`。空表示全部。
        #[serde(default)]
        events: Vec<String>,
    },
    /// 消息触发器:监听 `AgentBus` 广播的 `SwarmEvent`,按字段匹配。
    Message {
        /// 匹配的事件 kind 字符串(如 `"agent_completed"`)。
        /// `None` 表示匹配所有事件。
        #[serde(default)]
        event_kind: Option<String>,
        /// 匹配的 agent kind(如 `"coder"`)。`None` 表示不限制。
        #[serde(default)]
        agent_kind: Option<String>,
        /// 仅匹配成功事件(`AgentCompleted.success == true` 等)。默认 false。
        #[serde(default)]
        success_only: bool,
    },
    /// Webhook 触发器:接收外部 HTTP POST 请求。
    Webhook {
        /// HMAC-SHA256 校验密钥。`None` 表示不校验签名(仅用于本地测试)。
        #[serde(default)]
        secret: Option<String>,
        /// 允许的 HTTP method(默认 `"POST"`)。
        #[serde(default)]
        method: Option<String>,
    },
    /// Watch 触发器:条件监控(Web 抓取 / System 指标 / Calendar 日历)。
    /// 周期轮询 `source`,条件满足时经 `dispatch` 派发动作。
    Watch { source: watch::WatchSource },
}

/// 触发动作 — 三种异步派发目标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerAction {
    /// 调用已注册的 Skill(走 SkillEngine,自动继承 ExecApproval)。
    Skill {
        skill_id: String,
        #[serde(default)]
        params: HashMap<String, String>,
    },
    /// 触发一次 Swarm 执行。
    Swarm {
        description: String,
        #[serde(default)]
        agent_count: Option<u32>,
        #[serde(default)]
        agents: Vec<String>,
    },
    /// 弹出系统通知(走 NotificationService)。
    Notify { title: String, body: String },
}

impl TriggerAction {
    /// 返回动作 kind 字符串,用于持久化 `action_kind` 列。
    pub fn kind_str(&self) -> &'static str {
        match self {
            TriggerAction::Skill { .. } => "skill",
            TriggerAction::Swarm { .. } => "swarm",
            TriggerAction::Notify { .. } => "notify",
        }
    }

    pub fn from_kind_str(kind: &str, payload: &str) -> Option<Self> {
        match kind.to_ascii_lowercase().as_str() {
            "skill" => serde_json::from_str(payload).ok(),
            "swarm" => serde_json::from_str(payload).ok(),
            "notify" => serde_json::from_str(payload).ok(),
            _ => None,
        }
    }
}

/// 触发器配置(运行时模型,与 DB 行互换)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub kind: TriggerKind,
    pub condition: TriggerCondition,
    pub action: TriggerAction,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default)]
    pub max_fires: Option<u32>,
}

fn default_debounce_ms() -> u64 {
    DEFAULT_DEBOUNCE_MS
}

impl TriggerConfig {
    /// 从 DB 行重构配置。condition/action JSON 字符串反序列化为枚举。
    pub fn from_row(row: &TriggerRow) -> Option<Self> {
        let kind = TriggerKind::from_str(&row.kind)?;
        let condition: TriggerCondition = serde_json::from_str(&row.condition).ok()?;
        let action = TriggerAction::from_kind_str(&row.action_kind, &row.action_payload)?;
        Some(Self {
            id: row.id.clone(),
            name: row.name.clone(),
            enabled: row.enabled != 0,
            kind,
            condition,
            action,
            debounce_ms: row.debounce_ms as u64,
            max_fires: row.max_fires.map(|n| n as u32),
        })
    }

    /// 序列化 condition + action 为 JSON 字符串,供 DB 持久化。
    pub fn to_row_parts(&self) -> (String, String, String) {
        let condition = serde_json::to_string(&self.condition).unwrap_or_default();
        let action_kind = self.action.kind_str().to_string();
        let action_payload = serde_json::to_string(&self.action).unwrap_or_default();
        (condition, action_kind, action_payload)
    }
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// 触发器引擎 — 在 `AppState` 中以 `Arc<TriggerEngine>` 共享。
///
/// 持有所有 trigger 的运行时配置 + 三种 worker 的生命周期管理。
/// `start()` 启动消息订阅 + webhook server + 从 DB 加载文件触发器。
pub struct TriggerEngine {
    /// 内存中的触发器配置(与 DB 同步)。key = trigger_id。
    config: Arc<parking_lot::RwLock<HashMap<String, TriggerConfig>>>,
    bus: Arc<AgentBus>,
    skills: Arc<SkillEngine>,
    swarm: Arc<SwarmOrchestrator>,
    notify: Arc<NotificationService>,
    store: Arc<TriggerStore>,
    cancel: CancellationToken,

    /// 每 trigger 的最近触发时间(用于去抖)。
    last_fired: Arc<Mutex<HashMap<String, Instant>>>,
    /// 文件触发器 worker 集合(key = trigger_id)。
    file_workers: Arc<Mutex<HashMap<String, file::FileTriggerWorker>>>,
    /// Watch 触发器 worker 集合(key = trigger_id)。
    watch_workers: Arc<Mutex<HashMap<String, watch::WatchWorkerHandle>>>,
    /// 消息订阅 worker handle。
    message_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// webhook server handle。
    webhook_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// webhook 监听地址(可由 env 覆盖)。
    webhook_addr: String,
}

impl TriggerEngine {
    /// 构造引擎(未启动)。调用方随后通过 `start()` 启动所有 worker。
    pub fn new(
        sqlite: Arc<SqliteStore>,
        bus: Arc<AgentBus>,
        skills: Arc<SkillEngine>,
        swarm: Arc<SwarmOrchestrator>,
        notify: Arc<NotificationService>,
    ) -> Self {
        let store = Arc::new(TriggerStore::new(sqlite));
        let webhook_addr = std::env::var("NEBULA_TRIGGER_WEBHOOK_ADDR")
            .unwrap_or_else(|_| DEFAULT_WEBHOOK_ADDR.to_string());
        Self {
            config: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            bus,
            skills,
            swarm,
            notify,
            store,
            cancel: CancellationToken::new(),
            last_fired: Arc::new(Mutex::new(HashMap::new())),
            file_workers: Arc::new(Mutex::new(HashMap::new())),
            watch_workers: Arc::new(Mutex::new(HashMap::new())),
            message_handle: Arc::new(Mutex::new(None)),
            webhook_handle: Arc::new(Mutex::new(None)),
            webhook_addr,
        }
    }

    /// 暴露给 worker 用的配置快照(Arc clone)。
    pub fn config_handle(&self) -> Arc<parking_lot::RwLock<HashMap<String, TriggerConfig>>> {
        Arc::clone(&self.config)
    }

    /// 暴露给 webhook / file worker 用的 self Arc。
    /// 调用方需在外部 `Arc::clone(&engine)` 后传入。
    pub fn store(&self) -> Arc<TriggerStore> {
        Arc::clone(&self.store)
    }

    /// 启动所有 worker:消息订阅 + webhook server + 文件触发器。
    /// 从 DB 加载已持久化的触发器配置并填充内存 map。
    pub fn start(self: &Arc<Self>) {
        // 1. 从 DB 加载所有触发器到内存
        self.reload_from_store();

        // 2. 启动消息订阅 worker
        let engine = Arc::clone(self);
        let handle = message::spawn_message_subscriber(
            Arc::clone(&self.bus),
            Arc::clone(&self.config),
            engine,
        );
        *self.message_handle.lock() = Some(handle);
        info!(target: "nebula.triggers", "message subscriber started");

        // 3. 启动 webhook server(失败 warn + 降级,不阻断启动)
        let triggers = Arc::clone(&self.config);
        let addr = self.webhook_addr.clone();
        let cancel = self.cancel.clone();
        let webhook_engine = Arc::clone(self);
        let webhook_handle = tokio::spawn(async move {
            match webhook::start_webhook_server(addr, triggers, webhook_engine, cancel).await {
                Ok(h) => {
                    info!(target: "nebula.triggers", "webhook server started");
                    Some(h)
                }
                Err(e) => {
                    warn!(
                        target: "nebula.triggers",
                        error = %e,
                        "webhook server failed to start; degraded mode"
                    );
                    None
                }
            }
        });
        let engine_clone = Arc::clone(self);
        tokio::spawn(async move {
            match webhook_handle.await {
                Ok(Some(h)) => {
                    *engine_clone.webhook_handle.lock() = Some(h);
                }
                Ok(None) => {
                    // start_webhook_server 失败,已在 spawn 内 warn。
                }
                Err(e) => {
                    warn!(
                        target: "nebula.triggers",
                        error = %e,
                        "webhook server task panicked"
                    );
                }
            }
        });

        // 4. 启动所有文件触发器 worker
        let file_triggers: Vec<(String, TriggerConfig)> = {
            let cfg = self.config.read();
            cfg.iter()
                .filter(|(_, c)| c.enabled && c.kind == TriggerKind::File)
                .map(|(id, c)| (id.clone(), c.clone()))
                .collect()
        };
        for (id, cfg) in file_triggers {
            self.start_file_worker(&id, &cfg);
        }

        // 5. 启动所有 Watch 触发器 worker
        let watch_triggers: Vec<(String, TriggerConfig)> = {
            let cfg = self.config.read();
            cfg.iter()
                .filter(|(_, c)| c.enabled && c.kind == TriggerKind::Watch)
                .map(|(id, c)| (id.clone(), c.clone()))
                .collect()
        };
        for (id, cfg) in watch_triggers {
            self.start_watch_worker(&id, &cfg);
        }

        info!(target: "nebula.triggers", "trigger engine started");
    }

    /// 从 DB 重新加载所有触发器配置到内存(覆盖现有 map)。
    /// 调用后文件 worker 集合会按新配置重建。
    pub fn reload_from_store(&self) {
        let rows = match self.store.list() {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    target: "nebula.triggers",
                    error = %e,
                    "failed to load triggers from store"
                );
                return;
            }
        };
        let mut map = self.config.write();
        map.clear();
        for row in rows {
            if let Some(cfg) = TriggerConfig::from_row(&row) {
                map.insert(cfg.id.clone(), cfg);
            }
        }
        debug!(target: "nebula.triggers", count = map.len(), "loaded triggers from store");
    }

    /// 启动单个文件触发器 worker(若已存在则先停止)。
    fn start_file_worker(self: &Arc<Self>, id: &str, cfg: &TriggerConfig) {
        let TriggerCondition::File {
            ref paths,
            ref patterns,
            ref events,
        } = cfg.condition
        else {
            return;
        };
        let path_bufs: Vec<std::path::PathBuf> =
            paths.iter().map(std::path::PathBuf::from).collect();
        let mut worker = file::FileTriggerWorker::new();
        worker.start(
            path_bufs,
            patterns.clone(),
            events.clone(),
            id.to_string(),
            Arc::clone(self),
        );
        self.file_workers.lock().insert(id.to_string(), worker);
        info!(target: "nebula.triggers", trigger_id = %id, "file trigger worker started");
    }

    /// 停止单个文件触发器 worker。
    pub fn stop_file_worker(&self, id: &str) {
        if let Some(w) = self.file_workers.lock().remove(id) {
            w.stop();
            info!(target: "nebula.triggers", trigger_id = %id, "file trigger worker stopped");
        }
    }

    /// 启动单个 Watch 触发器 worker(若已存在则先停止)。
    fn start_watch_worker(self: &Arc<Self>, id: &str, cfg: &TriggerConfig) {
        let TriggerCondition::Watch { ref source } = cfg.condition else {
            return;
        };
        let worker = watch::spawn_watch_worker(
            id.to_string(),
            source.clone(),
            Arc::clone(self),
            Arc::clone(&self.store),
        );
        self.watch_workers.lock().insert(id.to_string(), worker);
        info!(target: "nebula.triggers", trigger_id = %id, "watch trigger worker started");
    }

    /// 停止单个 Watch 触发器 worker。
    pub fn stop_watch_worker(&self, id: &str) {
        if let Some(mut w) = self.watch_workers.lock().remove(id) {
            w.stop();
            info!(target: "nebula.triggers", trigger_id = %id, "watch trigger worker stopped");
        }
    }

    /// 新增触发器:持久化到 DB + 加入内存 map + 启动 worker(若 file 类型)。
    pub fn create(self: &Arc<Self>, cfg: TriggerConfig) -> anyhow::Result<()> {
        let row = TriggerRow {
            id: cfg.id.clone(),
            name: cfg.name.clone(),
            enabled: cfg.enabled as i64,
            kind: cfg.kind.as_str().to_string(),
            condition: serde_json::to_string(&cfg.condition)?,
            action_kind: cfg.action.kind_str().to_string(),
            action_payload: serde_json::to_string(&cfg.action)?,
            created_at: chrono::Utc::now().timestamp_millis(),
            last_fired_at: None,
            fire_count: 0,
            debounce_ms: cfg.debounce_ms as i64,
            max_fires: cfg.max_fires.map(|n| n as i64),
        };
        self.store.insert(&row)?;
        let needs_worker =
            cfg.enabled && matches!(cfg.kind, TriggerKind::File | TriggerKind::Watch);
        let id = cfg.id.clone();
        self.config.write().insert(cfg.id.clone(), cfg);
        if needs_worker {
            let cfg_snap = self.config.read().get(&id).cloned().expect("just inserted");
            match cfg_snap.kind {
                TriggerKind::File => self.start_file_worker(&id, &cfg_snap),
                TriggerKind::Watch => self.start_watch_worker(&id, &cfg_snap),
                _ => {}
            }
        }
        Ok(())
    }

    /// 删除触发器:停止 worker + 从内存 map 移除 + 从 DB 删除。
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.stop_file_worker(id);
        self.stop_watch_worker(id);
        self.config.write().remove(id);
        self.last_fired.lock().remove(id);
        self.store.delete(id)?;
        // 清理 watch_state(Watch 触发器独有,非 Watch 类型删除是无害 no-op)。
        let _ = self.store.delete_watch_state(id);
        Ok(())
    }

    /// 启用/禁用触发器:更新 DB + 内存 map + 启停 file/watch worker。
    pub fn set_enabled(self: &Arc<Self>, id: &str, enabled: bool) -> anyhow::Result<()> {
        self.store.set_enabled(id, enabled)?;
        let (need_restart_file, need_restart_watch) = {
            let mut map = self.config.write();
            if let Some(cfg) = map.get_mut(id) {
                let was_file = cfg.enabled && cfg.kind == TriggerKind::File;
                let was_watch = cfg.enabled && cfg.kind == TriggerKind::Watch;
                cfg.enabled = enabled;
                let now_file = cfg.enabled && cfg.kind == TriggerKind::File;
                let now_watch = cfg.enabled && cfg.kind == TriggerKind::Watch;
                (was_file != now_file, was_watch != now_watch)
            } else {
                (false, false)
            }
        };
        if need_restart_file {
            self.stop_file_worker(id);
            if enabled {
                let cfg_snap = self.config.read().get(id).cloned();
                if let Some(cfg) = cfg_snap {
                    self.start_file_worker(id, &cfg);
                }
            }
        }
        if need_restart_watch {
            self.stop_watch_worker(id);
            if enabled {
                let cfg_snap = self.config.read().get(id).cloned();
                if let Some(cfg) = cfg_snap {
                    self.start_watch_worker(id, &cfg);
                }
            }
        }
        Ok(())
    }

    /// 列出所有触发器(DB 读取,保证与持久化一致)。
    pub fn list(&self) -> anyhow::Result<Vec<TriggerConfig>> {
        let rows = self.store.list()?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(cfg) = TriggerConfig::from_row(&row) {
                out.push(cfg);
            }
        }
        Ok(out)
    }

    /// 查询触发日志。
    pub fn fire_log(&self, id: &str) -> anyhow::Result<Vec<FireLogRow>> {
        self.store.list_fire_log(id)
    }

    /// 触发器派发入口 — 由各 worker 调用。
    ///
    /// 流程:
    /// 1. 取 trigger 配置;不存在或 disabled → 跳过。
    /// 2. **递归防护**:若 payload.source_trigger_id == trigger_id → 跳过。
    /// 3. **去抖**:若距上次触发 < debounce_ms → 跳过。
    /// 4. **max_fires**:若已达上限 → 跳过并自动 disable。
    /// 5. 更新内存中的 last_fired,异步 spawn 执行 action。
    /// 6. 异步任务完成后写 fire_log。
    pub async fn dispatch(&self, trigger_id: &str, payload: serde_json::Value) {
        let cfg = {
            let map = self.config.read();
            map.get(trigger_id).cloned()
        };
        let Some(cfg) = cfg else {
            debug!(target: "nebula.triggers", trigger_id, "dispatch: trigger not found");
            return;
        };
        if !cfg.enabled {
            debug!(target: "nebula.triggers", trigger_id, "dispatch: trigger disabled");
            return;
        }

        // 递归防护:payload.source_trigger_id == trigger_id → 跳过。
        if let Some(src) = payload.get("source_trigger_id").and_then(|v| v.as_str()) {
            if src == trigger_id {
                warn!(
                    target: "nebula.triggers",
                    trigger_id,
                    "recursion guard: source_trigger_id matches self; skipping"
                );
                return;
            }
        }

        // 去抖检查。
        let now = Instant::now();
        {
            let last = self.last_fired.lock();
            if let Some(t) = last.get(trigger_id) {
                let elapsed = now.duration_since(*t);
                if elapsed < Duration::from_millis(cfg.debounce_ms) {
                    debug!(
                        target: "nebula.triggers",
                        trigger_id,
                        elapsed_ms = elapsed.as_millis(),
                        debounce_ms = cfg.debounce_ms,
                        "dispatch: debounced; skipping"
                    );
                    return;
                }
            }
        }

        // max_fires 检查(读 DB fire_count)。
        if let Some(max) = cfg.max_fires {
            if let Ok(row) = self.store.get(trigger_id) {
                if row.fire_count as u32 >= max {
                    warn!(
                        target: "nebula.triggers",
                        trigger_id,
                        fire_count = row.fire_count,
                        max_fires = max,
                        "trigger reached max_fires; disabling"
                    );
                    let _ = self.store.set_enabled(trigger_id, false);
                    let mut map = self.config.write();
                    if let Some(c) = map.get_mut(trigger_id) {
                        c.enabled = false;
                    }
                    return;
                }
            }
        }

        // 更新去抖时间戳。
        self.last_fired.lock().insert(trigger_id.to_string(), now);

        // 异步执行 action(不阻塞 worker)。
        let skills = Arc::clone(&self.skills);
        let swarm = Arc::clone(&self.swarm);
        let notify = Arc::clone(&self.notify);
        let store = Arc::clone(&self.store);
        let action = cfg.action.clone();
        let trigger_id_owned = trigger_id.to_string();
        let payload_str = serde_json::to_string(&payload).ok();
        let fired_at = chrono::Utc::now().timestamp_millis();

        // T-E-A-12: 用 with_automation_trigger 包装动作执行,同时设置
        // COST_SOURCE=Automation + COST_TRIGGER_ID=trigger_id,让动作内部
        // 的 LLM 调用经由 CostTracker.record 时自动归类为 Automation 来源
        // 并关联 trigger_id,供前端 Chat vs Automation 分栏展示 + 每日
        // 自动化预算告警使用。with_automation_trigger 是公共包装函数,
        // 避免直接引用私有 task_local static(会触发 rustc metadata ICE)。
        let trigger_id_for_scope = trigger_id_owned.clone();
        tokio::spawn(async move {
            let result = with_automation_trigger(Some(trigger_id_for_scope), async move {
                execute_action(&action, &skills, &swarm, &notify, &payload).await
            })
            .await;
            let (success, error_msg) = match result {
                Ok(_) => (1i64, None),
                Err(e) => {
                    warn!(
                        target: "nebula.triggers",
                        trigger_id = %trigger_id_owned,
                        error = %e,
                        "action execution failed"
                    );
                    (0i64, Some(format!("{e}")))
                }
            };
            // 更新 DB:fire_count + last_fired_at,并写 fire_log。
            if let Err(e) = store.record_fire(
                &trigger_id_owned,
                fired_at,
                success,
                error_msg.as_deref(),
                payload_str.as_deref(),
            ) {
                warn!(
                    target: "nebula.triggers",
                    trigger_id = %trigger_id_owned,
                    error = %e,
                    "failed to record fire log"
                );
            }
        });
    }

    /// 停止所有 worker(消息订阅 + webhook + 文件/watch 触发器)。
    pub fn stop(&self) {
        self.cancel.cancel();
        if let Some(h) = self.message_handle.lock().take() {
            h.abort();
        }
        if let Some(h) = self.webhook_handle.lock().take() {
            h.abort();
        }
        let mut file_w = self.file_workers.lock();
        for (_, w) in file_w.drain() {
            w.stop();
        }
        let mut watch_w = self.watch_workers.lock();
        for (_, mut w) in watch_w.drain() {
            w.stop();
        }
        info!(target: "nebula.triggers", "trigger engine stopped");
    }
}

// ---------------------------------------------------------------------------
// ActionExecutor
// ---------------------------------------------------------------------------

/// 执行触发动作。三种动作各自异步派发,不阻塞 worker。
async fn execute_action(
    action: &TriggerAction,
    skills: &Arc<SkillEngine>,
    swarm: &Arc<SwarmOrchestrator>,
    notify: &Arc<NotificationService>,
    _payload: &serde_json::Value,
) -> anyhow::Result<()> {
    match action {
        TriggerAction::Skill { skill_id, params } => {
            let req = UseSkillRequest {
                id: skill_id.clone(),
                params: params.clone(),
            };
            let result = skills.use_skill(req).await?;
            info!(
                target: "nebula.triggers",
                skill_id,
                execution_time_ms = result.execution_time_ms,
                "skill action executed"
            );
            Ok(())
        }
        TriggerAction::Swarm {
            description,
            agent_count,
            agents,
        } => {
            let mut task = SwarmTask::new(description.clone());
            if let Some(n) = agent_count {
                task.agent_count = *n;
            }
            if !agents.is_empty() {
                task.agents = agents.clone();
            }
            let report = swarm.execute(task).await?;
            info!(
                target: "nebula.triggers",
                success_count = report.success_count,
                failure_count = report.failure_count,
                approved = report.approved,
                "swarm action executed"
            );
            Ok(())
        }
        TriggerAction::Notify { title, body } => {
            // NotificationService 没有暴露通用 notify 方法,这里用
            // notify_task_completed 复用系统通知通道(task_id 用 trigger
            // 标识符,触发 5s 去重)。
            notify.notify_task_completed(title, &format!("trigger:{body}"));
            info!(
                target: "nebula.triggers",
                title,
                "notify action executed"
            );
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// 条件匹配辅助
// ---------------------------------------------------------------------------

/// 消息触发器条件匹配 — 由 `message::spawn_message_subscriber` 调用。
///
/// 规则:
/// * `event_kind`:`Some(s)` 时,SwarmEvent 的 kind 字符串必须等于 `s`。
/// * `agent_kind`:`Some(s)` 时,事件携带的 agent_kind 必须等于 `s`。
/// * `success_only`:`true` 时,仅匹配 success 事件(AgentCompleted.success=true)。
pub fn message_condition_matches(condition: &TriggerCondition, event: &SwarmEvent) -> bool {
    let TriggerCondition::Message {
        event_kind,
        agent_kind,
        success_only,
    } = condition
    else {
        return false;
    };
    let event_kind_str = swarm_event_kind_str(event);
    if let Some(k) = event_kind {
        if k != event_kind_str {
            return false;
        }
    }
    if let Some(k) = agent_kind {
        if !swarm_event_has_agent_kind(event, k) {
            return false;
        }
    }
    if *success_only && !swarm_event_is_success(event) {
        return false;
    }
    true
}

/// 返回 SwarmEvent 的 kind 字符串(与序列化 tag 一致)。
pub fn swarm_event_kind_str(event: &SwarmEvent) -> &'static str {
    match event {
        SwarmEvent::AgentStarted { .. } => "agent_started",
        SwarmEvent::AgentCompleted { .. } => "agent_completed",
        SwarmEvent::NegotiationStarted { .. } => "negotiation_started",
        SwarmEvent::ArbitrationResolved { .. } => "arbitration_resolved",
        SwarmEvent::AgentToolCall { .. } => "agent_tool_call",
        SwarmEvent::AgentOutputChunk { .. } => "agent_output_chunk",
        SwarmEvent::SwarmCompleted { .. } => "swarm_completed",
        SwarmEvent::DeadlockDetected { .. } => "deadlock_detected",
        // T-E-B-18: 思维树事件。
        SwarmEvent::TreeOfThoughtsStarted { .. } => "tree_of_thoughts_started",
        SwarmEvent::PathCompleted { .. } => "path_completed",
    }
}

/// 检查事件是否携带指定的 agent_kind(字符串比对)。
fn swarm_event_has_agent_kind(event: &SwarmEvent, kind: &str) -> bool {
    match event {
        SwarmEvent::AgentStarted { agent_kind, .. } => agent_kind.as_str() == kind,
        SwarmEvent::AgentCompleted { agent_kind, .. } => agent_kind.as_str() == kind,
        SwarmEvent::ArbitrationResolved { chosen_kind, .. } => chosen_kind.as_str() == kind,
        _ => false,
    }
}

/// 检查事件是否是"成功"事件(用于 success_only 过滤)。
fn swarm_event_is_success(event: &SwarmEvent) -> bool {
    match event {
        SwarmEvent::AgentCompleted { success, .. } => *success,
        SwarmEvent::SwarmCompleted { failure_count, .. } => *failure_count == 0,
        SwarmEvent::AgentToolCall { success, .. } => *success,
        _ => true,
    }
}

/// 把 SwarmEvent 序列化为 payload(含 event_kind + 关键字段)。
pub fn swarm_event_to_payload(event: &SwarmEvent) -> serde_json::Value {
    let kind = swarm_event_kind_str(event);
    let mut payload = serde_json::to_value(event).unwrap_or(serde_json::json!({}));
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "event_kind".to_string(),
            serde_json::Value::String(kind.to_string()),
        );
    }
    payload
}

// ---------------------------------------------------------------------------
// 去抖逻辑(独立函数,便于单测)
// ---------------------------------------------------------------------------

/// 检查去抖:若 `last` 距 `now` 不足 `debounce_ms`,返回 false(应跳过)。
/// 否则返回 true(应触发)。纯函数,无副作用。
pub fn debounce_should_fire(last: Option<Instant>, now: Instant, debounce_ms: u64) -> bool {
    match last {
        Some(t) => now.duration_since(t) >= Duration::from_millis(debounce_ms),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::agents::AgentKind;

    #[test]
    fn test_trigger_config_serde() {
        let cfg = TriggerConfig {
            id: "t1".to_string(),
            name: "test".to_string(),
            enabled: true,
            kind: TriggerKind::Message,
            condition: TriggerCondition::Message {
                event_kind: Some("agent_completed".to_string()),
                agent_kind: Some("coder".to_string()),
                success_only: true,
            },
            action: TriggerAction::Notify {
                title: "Done".to_string(),
                body: "task finished".to_string(),
            },
            debounce_ms: 500,
            max_fires: Some(3),
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: TriggerConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, back);
        // condition 序列化带 kind tag。
        assert!(s.contains("\"kind\":\"message\""));
        // action 序列化带 kind tag。
        assert!(s.contains("\"action\":{"));
        assert!(s.contains("\"kind\":\"notify\""));
    }

    #[test]
    fn test_trigger_condition_file_serde() {
        let c = TriggerCondition::File {
            paths: vec!["/tmp".to_string()],
            patterns: vec!["*.md".to_string()],
            events: vec!["create".to_string()],
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"kind\":\"file\""));
        let back: TriggerCondition = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn test_trigger_action_skill_serde() {
        let mut params = HashMap::new();
        params.insert("k".to_string(), "v".to_string());
        let a = TriggerAction::Skill {
            skill_id: "s1".to_string(),
            params,
        };
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"kind\":\"skill\""));
        let back: TriggerAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn test_trigger_action_swarm_serde() {
        let a = TriggerAction::Swarm {
            description: "do something".to_string(),
            agent_count: Some(3),
            agents: vec!["coder".to_string()],
        };
        let s = serde_json::to_string(&a).unwrap();
        assert!(s.contains("\"kind\":\"swarm\""));
        let back: TriggerAction = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn test_message_trigger_condition_match_event_kind() {
        let condition = TriggerCondition::Message {
            event_kind: Some("agent_completed".to_string()),
            agent_kind: None,
            success_only: false,
        };
        let event = SwarmEvent::agent_completed(AgentKind::Coder, "t1", true, None);
        assert!(message_condition_matches(&condition, &event));

        let condition_mismatch = TriggerCondition::Message {
            event_kind: Some("agent_started".to_string()),
            agent_kind: None,
            success_only: false,
        };
        assert!(!message_condition_matches(&condition_mismatch, &event));
    }

    #[test]
    fn test_message_trigger_condition_match_agent_kind() {
        let condition = TriggerCondition::Message {
            event_kind: None,
            agent_kind: Some("coder".to_string()),
            success_only: false,
        };
        let event = SwarmEvent::agent_completed(AgentKind::Coder, "t1", true, None);
        assert!(message_condition_matches(&condition, &event));

        let event_writer = SwarmEvent::agent_completed(AgentKind::Writer, "t1", true, None);
        assert!(!message_condition_matches(&condition, &event_writer));
    }

    #[test]
    fn test_message_trigger_condition_success_only() {
        let condition = TriggerCondition::Message {
            event_kind: None,
            agent_kind: None,
            success_only: true,
        };
        let ok = SwarmEvent::agent_completed(AgentKind::Coder, "t1", true, None);
        assert!(message_condition_matches(&condition, &ok));

        let fail =
            SwarmEvent::agent_completed(AgentKind::Coder, "t1", false, Some("err".to_string()));
        assert!(!message_condition_matches(&condition, &fail));
    }

    #[test]
    fn test_debounce_logic() {
        let now = Instant::now();
        // 无上次触发时间 → 应触发。
        assert!(debounce_should_fire(None, now, 1000));
        // 距上次 500ms,去抖 1000ms → 不应触发。
        let last = now - Duration::from_millis(500);
        assert!(!debounce_should_fire(Some(last), now, 1000));
        // 距上次 1500ms,去抖 1000ms → 应触发。
        let last = now - Duration::from_millis(1500);
        assert!(debounce_should_fire(Some(last), now, 1000));
    }

    #[test]
    fn test_recursion_guard_source_trigger_id() {
        // 模拟 dispatch 的递归防护逻辑(不依赖完整 engine 构造)。
        let trigger_id = "t1";
        let payload_self = serde_json::json!({
            "source_trigger_id": "t1",
            "event_kind": "agent_completed",
        });
        let payload_other = serde_json::json!({
            "source_trigger_id": "t2",
            "event_kind": "agent_completed",
        });
        let payload_none = serde_json::json!({
            "event_kind": "agent_completed",
        });

        // source_trigger_id == trigger_id → 应跳过(返回 true 表示"应跳过")。
        let should_skip = |payload: &serde_json::Value| -> bool {
            if let Some(src) = payload.get("source_trigger_id").and_then(|v| v.as_str()) {
                src == trigger_id
            } else {
                false
            }
        };

        assert!(should_skip(&payload_self));
        assert!(!should_skip(&payload_other));
        assert!(!should_skip(&payload_none));
    }

    #[test]
    fn test_trigger_kind_roundtrip() {
        for k in [
            TriggerKind::File,
            TriggerKind::Message,
            TriggerKind::Webhook,
            TriggerKind::Watch,
        ] {
            let s = k.as_str();
            let back = TriggerKind::from_str(s).unwrap();
            assert_eq!(k, back);
        }
        assert!(TriggerKind::from_str("unknown").is_none());
    }

    #[test]
    fn test_trigger_config_to_row_parts() {
        let cfg = TriggerConfig {
            id: "t1".to_string(),
            name: "n".to_string(),
            enabled: true,
            kind: TriggerKind::Webhook,
            condition: TriggerCondition::Webhook {
                secret: Some("s".to_string()),
                method: None,
            },
            action: TriggerAction::Notify {
                title: "T".to_string(),
                body: "B".to_string(),
            },
            debounce_ms: 1000,
            max_fires: None,
        };
        let (condition, action_kind, action_payload) = cfg.to_row_parts();
        assert_eq!(action_kind, "notify");
        assert!(condition.contains("\"kind\":\"webhook\""));
        assert!(action_payload.contains("\"kind\":\"notify\""));
    }

    #[test]
    fn test_swarm_event_kind_str() {
        assert_eq!(
            swarm_event_kind_str(&SwarmEvent::agent_started(AgentKind::Generic, "t")),
            "agent_started"
        );
        assert_eq!(
            swarm_event_kind_str(&SwarmEvent::agent_completed(
                AgentKind::Coder,
                "t",
                true,
                None
            )),
            "agent_completed"
        );
        assert_eq!(
            swarm_event_kind_str(&SwarmEvent::swarm_completed("t", 1, 0, true)),
            "swarm_completed"
        );
    }

    #[test]
    fn test_swarm_event_to_payload_has_event_kind() {
        let event = SwarmEvent::agent_completed(AgentKind::Coder, "t1", true, None);
        let payload = swarm_event_to_payload(&event);
        assert_eq!(payload["event_kind"], "agent_completed");
        assert_eq!(payload["kind"], "agent_completed");
    }
}
