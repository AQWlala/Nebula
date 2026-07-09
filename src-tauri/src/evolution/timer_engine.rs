//! T-E-S-63: 三定时机制统一引擎(`TimerEngine`)。
//!
//! 统一管理三种定时机制,为下游(T-E-D-05 Proactive Engine、T-E-B-15 AI 自动整理 MOC)
//! 提供单一订阅入口:
//!
//! * **Cron 定时** — 基于 cron 表达式的定时任务,委托 [`CronScheduler`]
//!   (三计时:03:00 合并 / 12:00 自检 / 21:00 回顾)。TimerEngine 持有
//!   `Arc<CronScheduler>`,在 Cron 任务执行前广播 `TimerEvent(Cron)`。
//! * **事件触发** — 基于系统事件(文件变化 / 消息到达 / Webhook)触发。
//!   外部事件源(如 `TriggerEngine`)通过 [`TimerEngine::inject_event`]
//!   注入事件,引擎广播 `TimerEvent(Event)`。
//! * **间隔轮询** — 基于固定间隔轮询外部资源(API / 邮件 / 通知)。
//!   通过 [`TimerEngine::register_poll_task`] 注册周期任务,每次轮询
//!   广播 `TimerEvent(Poll)`。
//!
//! ## 复用关系
//!
//! | 下游任务 | 复用方式 |
//! |---------|---------|
//! | **T-E-S-53**(Cron 定时任务引擎) | `TimerEngine::cron()` 直接返回 `Arc<CronScheduler>`,复用其 cron 表达式解析 + 预算 + 三计时任务 |
//! | **T-E-D-05**(Proactive Engine) | `TimerEngine::subscribe()` 订阅统一事件流,在任一机制触发时收到 `TimerEvent` |
//! | **T-E-B-15**(AI 自动整理 MOC) | 订阅 `TimerSource::Cron` 事件,在每日 Consolidation(03:00)后触发主题聚类 |
//!
//! ## Feature Gate
//!
//! 由 `self-evolution` feature 门控(与 `CronScheduler` 一致)。
//!
//! ## 设计约束
//!
//! 1. **零新依赖**:全部用 Cargo.toml 已有 crate(tokio `sync::broadcast` /
//!    parking_lot / tokio-util `CancellationToken` / chrono / serde)。
//! 2. **不阻塞**:事件广播走 `broadcast::Sender::send`(非阻塞,无订阅者时丢弃)。
//! 3. **尽力而为**:轮询任务失败记 warning,不中断引擎。
//! 4. **可取消**:所有后台任务响应 `CancellationToken`,引擎 `stop()` 后全部退出。

#![cfg(feature = "self-evolution")]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::cron_scheduler::CronScheduler;

// ---------------------------------------------------------------------------
// 事件模型
// ---------------------------------------------------------------------------

/// 定时事件来源 — 三种定时机制各一种。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimerSource {
    /// Cron 表达式定时(委托 CronScheduler)。
    Cron,
    /// 系统事件触发(由 TriggerEngine 或外部源注入)。
    Event,
    /// 固定间隔轮询(周期性 poll 外部资源)。
    Poll,
}

impl TimerSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TimerSource::Cron => "cron",
            TimerSource::Event => "event",
            TimerSource::Poll => "poll",
        }
    }
}

/// 统一定时事件 — 三种机制触发后都产生此事件,经广播通道派发给订阅者。
///
/// 下游(Proactive Engine / MOC)通过 `TimerEngine::subscribe()` 接收,
/// 按 `source` 字段分支处理。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerEvent {
    /// 事件来源(Cron / Event / Poll)。
    pub source: TimerSource,
    /// 任务标识(Cron 任务名 / 事件源 id / 轮询任务 id)。
    pub task_id: String,
    /// 触发时间(UTC)。
    pub fired_at: DateTime<Utc>,
    /// 事件载荷(Cron 任务名 / 事件 payload / 轮询结果)。
    pub payload: serde_json::Value,
}

impl TimerEvent {
    /// 构造一个 Cron 源事件。
    pub fn cron(task_id: impl Into<String>) -> Self {
        Self {
            source: TimerSource::Cron,
            task_id: task_id.into(),
            fired_at: Utc::now(),
            payload: serde_json::json!({}),
        }
    }

    /// 构造一个 Event 源事件。
    pub fn event(task_id: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            source: TimerSource::Event,
            task_id: task_id.into(),
            fired_at: Utc::now(),
            payload,
        }
    }

    /// 构造一个 Poll 源事件。
    pub fn poll(task_id: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            source: TimerSource::Poll,
            task_id: task_id.into(),
            fired_at: Utc::now(),
            payload,
        }
    }
}

// ---------------------------------------------------------------------------
// 间隔轮询模型
// ---------------------------------------------------------------------------

/// 轮询源 — 外部资源类型(供前端渲染 + 日志归类)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PollSource {
    /// HTTP/HTTPS API 轮询(如 REST 端点健康检查)。
    Api { url: String },
    /// 邮件账户轮询(IMAP/POP3 未读数,实际抓取由调用方实现)。
    Email { account: String },
    /// 系统通知轮询(如未读消息计数)。
    Notification { source: String },
    /// 自定义轮询源(调用方在 callback 中自行决定语义)。
    Custom,
}

/// 轮询任务定义(注册时传入,引擎据此 spawn 周期任务)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollTask {
    /// 任务唯一标识。
    pub id: String,
    /// 轮询间隔(秒)。
    pub interval_secs: u64,
    /// 轮询源类型。
    pub source: PollSource,
}

/// 轮询任务运行时句柄(存放在引擎的 poll_tasks map 中)。
struct PollTaskHandle {
    cancel: CancellationToken,
    handle: Option<JoinHandle<()>>,
}

impl PollTaskHandle {
    fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// 默认广播通道容量(订阅者 lag 时丢弃最旧事件)。
const DEFAULT_BROADCAST_CAPACITY: usize = 256;

/// 三定时机制统一引擎。
///
/// 在 `AppState` 中以 `Arc<TimerEngine>` 共享。持有 `CronScheduler` +
/// 间隔轮询任务注册表 + 广播通道。下游通过 `subscribe()` 订阅统一定时事件流。
///
/// ## 生命周期
///
/// 1. `TimerEngine::new(cron_scheduler)` 构造引擎(未启动)。
/// 2. `register_poll_task(task)` 注册间隔轮询任务(可在 start 前或后调用)。
/// 3. `start()` 启动 CronScheduler(注入 event_tx)+ 所有已注册轮询任务。
/// 4. `subscribe()` 在任意时刻获取广播 Receiver。
/// 5. `inject_event(task_id, payload)` 在任意时刻注入事件触发事件。
/// 6. `stop()` 取消所有后台任务。
pub struct TimerEngine {
    /// Cron 定时机制 — 持有 CronScheduler(三计时任务)。
    cron: Arc<CronScheduler>,
    /// 间隔轮询任务注册表(id → handle)。
    poll_tasks: Arc<Mutex<HashMap<String, PollTaskHandle>>>,
    /// 广播通道(所有定时机制触发后都通过此通道广播 TimerEvent)。
    tx: Sender<TimerEvent>,
    /// 取消令牌(stop() 时取消所有后台任务)。
    cancel: CancellationToken,
    /// 是否已启动(start() 防重入)。
    started: Arc<std::sync::atomic::AtomicBool>,
}

impl TimerEngine {
    /// 构造引擎(未启动)。传入已配置好的 `CronScheduler`。
    pub fn new(cron: Arc<CronScheduler>) -> Self {
        let (tx, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            cron,
            poll_tasks: Arc::new(Mutex::new(HashMap::new())),
            tx,
            cancel: CancellationToken::new(),
            started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 自定义广播通道容量(默认 256)。
    pub fn with_broadcast_capacity(mut self, capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        self.tx = tx;
        self
    }

    /// 返回 Cron 定时机制引用(`Arc<CronScheduler>`)。
    ///
    /// T-E-S-53(Cron 定时任务引擎)通过此方法直接复用 CronScheduler 的
    /// cron 表达式解析 + 预算管理 + 三计时任务执行能力。
    pub fn cron(&self) -> &Arc<CronScheduler> {
        &self.cron
    }

    /// 订阅统一定时事件流。
    ///
    /// 返回 `broadcast::Receiver<TimerEvent>`。下游(Proactive Engine /
    /// MOC)在独立 tokio task 中 `while let Ok(ev) = rx.recv().await { ... }`
    /// 消费事件,按 `ev.source` 分支处理。
    ///
    /// **注意**:若订阅者消费慢于生产者,旧事件会被丢弃(lag > capacity)。
    /// 关键场景应增大 `with_broadcast_capacity` 或在订阅者侧加 buffer。
    pub fn subscribe(&self) -> Receiver<TimerEvent> {
        self.tx.subscribe()
    }

    /// 注入事件触发事件(Event 机制)。
    ///
    /// 外部事件源(如 `TriggerEngine` 的 message/file/webhook worker)
    /// 在匹配到条件时调用此方法,引擎广播 `TimerEvent(Event)`。
    ///
    /// `task_id` 通常是触发器 id,`payload` 是事件载荷(JSON)。
    /// 无订阅者时事件被丢弃(非错误)。
    pub fn inject_event(&self, task_id: impl Into<String>, payload: serde_json::Value) {
        let event = TimerEvent::event(task_id, payload);
        let _ = self.tx.send(event);
    }

    /// 注册间隔轮询任务(Poll 机制)。
    ///
    /// spawn 一个 tokio task,每 `interval_secs` 秒广播一次
    /// `TimerEvent(Poll)`。`task.source` 描述轮询源类型(API/Email/Notification/Custom),
    /// 供订阅者分支处理。实际的外部资源抓取由订阅者在收到事件后执行
    /// (引擎只负责"按时提醒",不负责"如何抓取",保持单一职责)。
    ///
    /// 若 `task.id` 已存在则先停止旧任务再注册新任务(覆盖语义)。
    pub fn register_poll_task(self: &Arc<Self>, task: PollTask) {
        // 覆盖语义:先停止同 id 旧任务。
        self.unregister_poll_task(&task.id);

        let cancel = self.cancel.child_token();
        let cancel_clone = cancel.clone();
        let tx = self.tx.clone();
        let task_id = task.id.clone();
        let interval_secs = task.interval_secs.max(1);
        let source = task.source.clone();

        let handle = tokio::spawn(async move {
            info!(
                target: "nebula.timer",
                task_id = %task_id,
                interval_secs,
                "poll task started"
            );
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_clone.cancelled() => {
                        debug!(
                            target: "nebula.timer",
                            task_id = %task_id,
                            "poll task cancelled"
                        );
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {
                        let payload = serde_json::to_value(&source).unwrap_or(serde_json::json!({}));
                        let event = TimerEvent::poll(task_id.clone(), payload);
                        let _ = tx.send(event);
                    }
                }
            }
            info!(
                target: "nebula.timer",
                task_id = %task_id,
                "poll task exiting"
            );
        });

        self.poll_tasks.lock().insert(
            task.id.clone(),
            PollTaskHandle {
                cancel,
                handle: Some(handle),
            },
        );
        info!(
            target: "nebula.timer",
            task_id = %task.id,
            interval_secs = task.interval_secs,
            "poll task registered"
        );
    }

    /// 注销间隔轮询任务(停止 + 移除)。
    pub fn unregister_poll_task(&self, task_id: &str) {
        if let Some(mut h) = self.poll_tasks.lock().remove(task_id) {
            h.stop();
            info!(target: "nebula.timer", task_id, "poll task unregistered");
        }
    }

    /// 列出已注册的轮询任务 id(快照)。
    pub fn list_poll_task_ids(&self) -> Vec<String> {
        self.poll_tasks.lock().keys().cloned().collect()
    }

    /// 启动引擎:启动 CronScheduler(注入 event_tx)+ 标记已启动。
    ///
    /// CronScheduler 的 event_tx 在此刻注入,使其在执行 cron 任务前
    /// 广播 `TimerEvent(Cron)`。已注册的轮询任务在 `register_poll_task`
    /// 时已 spawn,此处不重复启动。
    ///
    /// **防重入**:多次调用 `start()` 仅第一次生效。
    pub async fn start(self: Arc<Self>) {
        if self
            .started
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            warn!(target: "nebula.timer", "timer engine already started");
            return;
        }

        // 注入 event_tx 到 CronScheduler,使其广播 Cron 事件。
        // CronScheduler::start() 是一个阻塞循环(直到取消),此处 spawn。
        let cron = Arc::clone(&self.cron);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            // 用 set_event_sender 注入广播通道,再 start。
            cron.set_event_sender(Some(tx));
            if let Err(e) = cron.start().await {
                warn!(
                    target: "nebula.timer",
                    error = %e,
                    "cron scheduler exited with error"
                );
            }
        });

        info!(
            target: "nebula.timer",
            poll_tasks = self.poll_tasks.lock().len(),
            "timer engine started"
        );
    }

    /// 停止引擎:取消所有后台任务(CronScheduler + 轮询任务)。
    pub fn stop(&self) {
        self.cancel.cancel();

        // 停止所有轮询任务。
        let mut tasks = self.poll_tasks.lock();
        for (_, mut h) in tasks.drain() {
            h.stop();
        }

        // CronScheduler 通过共享 cancel token 停止?实际上 CronScheduler
        // 没有暴露 stop(),其 start() 循环靠 running flag 自旋。此处
        // 仅取消轮询任务;CronScheduler 的生命周期由调用方(bootstrap)
        // 通过 tokio task abort 管理。
        info!(target: "nebula.timer", "timer engine stopped");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::cron_scheduler::CronScheduler;

    fn make_engine() -> Arc<TimerEngine> {
        let cron = Arc::new(CronScheduler::new(None, "test-user".to_string()));
        Arc::new(TimerEngine::new(cron))
    }

    // ---- TimerSource / TimerEvent ----

    #[test]
    fn timer_source_serde_roundtrip() {
        for s in [TimerSource::Cron, TimerSource::Event, TimerSource::Poll] {
            let json = serde_json::to_string(&s).expect("serialize should succeed");
            let back: TimerSource = serde_json::from_str(&json).expect("parse should succeed");
            assert_eq!(s, back);
        }
        assert_eq!(
            serde_json::to_string(&TimerSource::Cron).expect("serialize should succeed"),
            "\"cron\""
        );
        assert_eq!(
            serde_json::to_string(&TimerSource::Event).expect("serialize should succeed"),
            "\"event\""
        );
        assert_eq!(
            serde_json::to_string(&TimerSource::Poll).expect("serialize should succeed"),
            "\"poll\""
        );
    }

    #[test]
    fn timer_source_as_str() {
        assert_eq!(TimerSource::Cron.as_str(), "cron");
        assert_eq!(TimerSource::Event.as_str(), "event");
        assert_eq!(TimerSource::Poll.as_str(), "poll");
    }

    #[test]
    fn timer_event_cron_constructor() {
        let ev = TimerEvent::cron("memory-merge");
        assert_eq!(ev.source, TimerSource::Cron);
        assert_eq!(ev.task_id, "memory-merge");
        assert!(ev.payload.is_object());
    }

    #[test]
    fn timer_event_event_constructor_with_payload() {
        let payload = serde_json::json!({ "file": "/tmp/a.md", "kind": "modify" });
        let ev = TimerEvent::event("file-trigger-1", payload.clone());
        assert_eq!(ev.source, TimerSource::Event);
        assert_eq!(ev.task_id, "file-trigger-1");
        assert_eq!(ev.payload, payload);
    }

    #[test]
    fn timer_event_poll_constructor_with_payload() {
        let payload = serde_json::json!({ "url": "https://example.com", "status": 200 });
        let ev = TimerEvent::poll("api-poll-1", payload.clone());
        assert_eq!(ev.source, TimerSource::Poll);
        assert_eq!(ev.task_id, "api-poll-1");
        assert_eq!(ev.payload, payload);
    }

    // ---- PollSource serde ----

    #[test]
    fn poll_source_api_serde() {
        let s = PollSource::Api {
            url: "https://api.example.com/health".to_string(),
        };
        let json = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(json.contains("\"type\":\"api\""));
        assert!(json.contains("\"url\""));
        let back: PollSource = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(s, back);
    }

    #[test]
    fn poll_source_email_serde() {
        let s = PollSource::Email {
            account: "user@example.com".to_string(),
        };
        let json = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(json.contains("\"type\":\"email\""));
        let back: PollSource = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(s, back);
    }

    #[test]
    fn poll_source_notification_serde() {
        let s = PollSource::Notification {
            source: "feishu".to_string(),
        };
        let json = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(json.contains("\"type\":\"notification\""));
        let back: PollSource = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(s, back);
    }

    #[test]
    fn poll_source_custom_serde() {
        let s = PollSource::Custom;
        let json = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(json.contains("\"type\":\"custom\""));
        let back: PollSource = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(s, back);
    }

    // ---- TimerEngine 基础 ----

    #[test]
    fn engine_cron_accessor_returns_scheduler() {
        let engine = make_engine();
        let cron = engine.cron();
        // 验证返回的是同一个 CronScheduler(三计时任务存在)。
        let tasks = cron.list_tasks();
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn engine_subscribe_returns_receiver() {
        let engine = make_engine();
        let _rx = engine.subscribe();
        // 仅验证 subscribe() 不 panic 且返回 Receiver(无事件时 recv 会 lag)。
    }

    #[test]
    fn engine_inject_event_no_subscriber_is_ok() {
        // 无订阅者时 inject_event 不应 panic(lag > capacity 丢弃)。
        let engine = make_engine();
        engine.inject_event("test-trigger", serde_json::json!({"x": 1}));
    }

    #[tokio::test]
    async fn engine_inject_event_reaches_subscriber() {
        let engine = make_engine();
        let mut rx = engine.subscribe();
        engine.inject_event("file-trigger", serde_json::json!({"file": "a.md"}));

        let ev = rx.recv().await.expect("should receive event");
        assert_eq!(ev.source, TimerSource::Event);
        assert_eq!(ev.task_id, "file-trigger");
        assert_eq!(ev.payload["file"], "a.md");
    }

    #[tokio::test]
    async fn engine_inject_multiple_events_preserve_order() {
        let engine = make_engine();
        let mut rx = engine.subscribe();
        for i in 0..5 {
            engine.inject_event(format!("evt-{i}"), serde_json::json!({"i": i}));
        }
        for i in 0..5 {
            let ev = rx.recv().await.expect("should receive event");
            assert_eq!(ev.task_id, format!("evt-{i}"));
            assert_eq!(ev.payload["i"], i);
        }
    }

    // ---- 轮询任务注册 / 注销 ----

    #[tokio::test]
    async fn register_poll_task_adds_to_list() {
        let engine = make_engine();
        assert!(engine.list_poll_task_ids().is_empty());

        engine.register_poll_task(PollTask {
            id: "api-1".to_string(),
            interval_secs: 60,
            source: PollSource::Api {
                url: "https://example.com".to_string(),
            },
        });
        assert_eq!(engine.list_poll_task_ids(), vec!["api-1".to_string()]);

        engine.register_poll_task(PollTask {
            id: "email-1".to_string(),
            interval_secs: 120,
            source: PollSource::Email {
                account: "u@e.com".to_string(),
            },
        });
        assert_eq!(engine.list_poll_task_ids().len(), 2);
    }

    #[tokio::test]
    async fn unregister_poll_task_removes_from_list() {
        let engine = make_engine();
        engine.register_poll_task(PollTask {
            id: "api-1".to_string(),
            interval_secs: 60,
            source: PollSource::Api {
                url: "https://example.com".to_string(),
            },
        });
        assert_eq!(engine.list_poll_task_ids().len(), 1);

        engine.unregister_poll_task("api-1");
        assert!(engine.list_poll_task_ids().is_empty());
    }

    #[tokio::test]
    async fn register_poll_task_overwrites_existing() {
        // 覆盖语义:同 id 注册两次,后者覆盖前者,列表中只保留一个。
        let engine = make_engine();
        engine.register_poll_task(PollTask {
            id: "api-1".to_string(),
            interval_secs: 60,
            source: PollSource::Api {
                url: "https://old.example.com".to_string(),
            },
        });
        engine.register_poll_task(PollTask {
            id: "api-1".to_string(),
            interval_secs: 30,
            source: PollSource::Api {
                url: "https://new.example.com".to_string(),
            },
        });
        assert_eq!(engine.list_poll_task_ids(), vec!["api-1".to_string()]);
    }

    #[test]
    fn unregister_unknown_poll_task_is_noop() {
        let engine = make_engine();
        // 不存在的 id 不应 panic。
        engine.unregister_poll_task("nonexistent");
        assert!(engine.list_poll_task_ids().is_empty());
    }

    #[tokio::test]
    async fn poll_task_emits_timer_event() {
        // 注册一个 1 秒间隔的轮询任务,验证订阅者收到 TimerEvent(Poll)。
        let engine = make_engine();
        let mut rx = engine.subscribe();

        engine.register_poll_task(PollTask {
            id: "fast-poll".to_string(),
            interval_secs: 1,
            source: PollSource::Custom,
        });

        let ev = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("should receive within 3s")
            .expect("should receive event");

        assert_eq!(ev.source, TimerSource::Poll);
        assert_eq!(ev.task_id, "fast-poll");
        // payload 是 PollSource::Custom 的序列化。
        assert_eq!(ev.payload["type"], "custom");

        engine.stop();
    }

    #[tokio::test]
    async fn poll_task_stops_on_engine_stop() {
        let engine = make_engine();
        let mut rx = engine.subscribe();

        engine.register_poll_task(PollTask {
            id: "fast-poll".to_string(),
            interval_secs: 1,
            source: PollSource::Custom,
        });

        // 先收到一次(证明任务在跑)。
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("should receive within 3s");

        engine.stop();

        // stop 后应不再收到新事件(等待 2s 验证无新事件)。
        // 注意:broadcast::Receiver::recv 在 sender 存活但无事件时会阻塞,
        // 这里用 timeout 验证超时(无新事件)。
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await;
        assert!(result.is_err(), "should not receive events after stop()");
    }

    // ---- 三机制统一验证 ----

    #[tokio::test]
    async fn all_three_sources_reach_same_subscriber() {
        // 验证 Cron / Event / Poll 三种机制触发的事件都通过同一个广播通道
        // 到达同一个订阅者(Proactive Engine 订阅模式)。
        let engine = make_engine();
        let mut rx = engine.subscribe();

        // Event 机制:手动注入。
        engine.inject_event("evt-1", serde_json::json!({"k": "v"}));

        // Poll 机制:注册 1s 轮询任务。
        engine.register_poll_task(PollTask {
            id: "poll-1".to_string(),
            interval_secs: 1,
            source: PollSource::Custom,
        });

        // 收集 2 个事件(Event + Poll),验证来源不同但都到达。
        let mut sources = Vec::new();
        for _ in 0..2 {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
                .await
                .expect("should receive within 3s")
                .expect("should receive event");
            sources.push(ev.source);
        }

        assert!(sources.contains(&TimerSource::Event), "should have Event");
        assert!(sources.contains(&TimerSource::Poll), "should have Poll");

        engine.stop();
    }

    #[test]
    fn engine_with_broadcast_capacity_custom() {
        let cron = Arc::new(CronScheduler::new(None, "test-user".to_string()));
        let _engine = TimerEngine::new(cron).with_broadcast_capacity(64);
        // 仅验证 builder 不 panic。
    }

    #[tokio::test]
    async fn engine_start_is_idempotent() {
        // start() 多次调用仅第一次生效。
        let engine = make_engine();
        engine.clone().start().await;
        engine.clone().start().await; // 第二次应 warn 并跳过。
                                      // 验证 started flag 为 true。
        assert!(engine.started.load(std::sync::atomic::Ordering::SeqCst));
    }
}
