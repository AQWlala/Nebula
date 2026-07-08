//! T-S4-A-02: Team Context Pool — 跨任务共享上下文区。
//!
//! 与 [`super::context::TeamContext`]（单次 `execute()` 内的临时上下文）不同,
//! `TeamContextPool` 是一个**跨任务持久**的共享内存区,允许不同 swarm 执行
//! 之间通过 topic 发布/订阅上下文片段。
//!
//! ## 核心能力
//!
//! * `publish(topic, author, body)` — 向某 topic 发布一条上下文条目,
//!   所有订阅该 topic 的 receiver 会收到通知。
//! * `subscribe(topic)` — 订阅 topic,返回 `broadcast::Receiver<PoolEntry>`,
//!   后续 publish 会异步推送到 receiver。
//! * `get(topic)` — 拉取某 topic 当前全部条目快照(同步)。
//! * 自动 GC:条目 30 分钟未被访问即被回收(可在 `new_with_ttl` 自定义)。
//!
//! ## 设计决策
//!
//! * 采用 `tokio::sync::broadcast` 而非 `mpsc`:多订阅者可同时监听同一 topic。
//! * GC 采用**惰性 + 后台**双策略:每次 publish/get 触发轻量惰性 GC,
//!   `start_gc_worker` 可选启动后台周期 GC(避免长期不访问的 topic 堆积)。
//! * `PoolEntry` 复用 `ContextEntry` 结构,增加 `topic`/`published_at`/`last_accessed`。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, info, instrument};

use super::context::ContextEntry;

/// 默认 GC TTL:30 分钟(1800 秒)。
const DEFAULT_GC_TTL_SECS: i64 = 30 * 60;
/// 默认 broadcast channel 容量:每个 topic 缓存 64 条历史消息。
const DEFAULT_CHANNEL_CAPACITY: usize = 64;
/// 后台 GC 默认检查间隔:5 分钟。
const DEFAULT_GC_INTERVAL_SECS: u64 = 5 * 60;

/// 池中一条上下文记录(在 ContextEntry 基础上增加 topic/时间戳)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolEntry {
    /// 所属 topic。
    pub topic: String,
    /// 复用 ContextEntry(author/label/body/created_at)。
    pub entry: ContextEntry,
    /// 发布时间(unix 秒)。
    pub published_at: i64,
    /// 最后一次被访问(通过 get/snapshot)的时间(unix 秒)。
    pub last_accessed: i64,
}

impl PoolEntry {
    fn new(topic: impl Into<String>, entry: ContextEntry) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            topic: topic.into(),
            entry,
            published_at: now,
            last_accessed: now,
        }
    }
}

/// 跨任务共享上下文池。
///
/// 线程安全(`Arc<RwLock<...>>`),可被 `SwarmOrchestrator` 持有并跨多次
/// `execute()` 调用复用。Agent 可通过 `publish`/`subscribe` 协作共享中间发现。
pub struct TeamContextPool {
    /// topic -> 条目列表。
    entries: RwLock<HashMap<String, Vec<PoolEntry>>>,
    /// topic -> broadcast sender(惰性创建)。
    subscribers: RwLock<HashMap<String, broadcast::Sender<PoolEntry>>>,
    /// GC TTL(秒)。
    gc_ttl_secs: i64,
    /// broadcast channel 容量。
    channel_capacity: usize,
}

impl TeamContextPool {
    /// 创建默认池(30 分钟 TTL,64 channel 容量)。
    pub fn new() -> Self {
        Self::new_with_ttl(DEFAULT_GC_TTL_SECS)
    }

    /// 创建自定义 TTL 的池。
    pub fn new_with_ttl(gc_ttl_secs: i64) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            subscribers: RwLock::new(HashMap::new()),
            gc_ttl_secs,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }

    /// 发布一条上下文到指定 topic。
    ///
    /// - 添加到 `entries[topic]`
    /// - 通知所有 `subscribe(topic)` 的 receiver(若有)
    /// - 触发惰性 GC(仅清理本 topic 过期条目,轻量)
    #[instrument(skip(self, body), fields(topic = %topic, author = %author))]
    pub fn publish(&self, topic: &str, author: &str, body: &str) {
        let entry = ContextEntry::new(author, topic, body);
        let pool_entry = PoolEntry::new(topic, entry);

        {
            let mut map = self.entries.write();
            let vec = map.entry(topic.to_string()).or_default();
            vec.push(pool_entry.clone());
            // 惰性 GC:清理本 topic 过期条目。
            let now = chrono::Utc::now().timestamp();
            vec.retain(|e| now - e.last_accessed < self.gc_ttl_secs);
        }

        // 通知订阅者(若 sender 不存在则惰性跳过;subscribe 时会创建)。
        if let Some(sender) = self.subscribers.read().get(topic) {
            // broadcast send 失败仅表示无 receiver,不算错误。
            let _ = sender.send(pool_entry);
        }

        debug!(
            target: "nebula.swarm.context_pool",
            topic = %topic,
            "context published"
        );
    }

    /// 订阅 topic,返回 broadcast receiver。
    ///
    /// 若 topic 尚无 sender,会惰性创建。后续 `publish` 会推送到 receiver。
    /// 注意:broadcast 仅推送**订阅之后**发布的条目;历史条目请用 `get`。
    pub fn subscribe(&self, topic: &str) -> broadcast::Receiver<PoolEntry> {
        let sender = {
            let subs = self.subscribers.read();
            if let Some(s) = subs.get(topic) {
                s.clone()
            } else {
                drop(subs);
                let mut subs = self.subscribers.write();
                // 双检:另一线程可能已创建。
                subs.entry(topic.to_string())
                    .or_insert_with(|| {
                        let (tx, _rx) = broadcast::channel(self.channel_capacity);
                        tx
                    })
                    .clone()
            }
        };
        sender.subscribe()
    }

    /// 拉取某 topic 当前全部条目快照(同步)。
    ///
    /// 同时更新这些条目的 `last_accessed`(重置 GC 计时)。
    pub fn get(&self, topic: &str) -> Vec<PoolEntry> {
        let now = chrono::Utc::now().timestamp();
        let mut map = self.entries.write();
        let vec = map.entry(topic.to_string()).or_default();
        // 惰性 GC + 更新 last_accessed。
        vec.retain(|e| now - e.last_accessed < self.gc_ttl_secs);
        for e in vec.iter_mut() {
            e.last_accessed = now;
        }
        vec.clone()
    }

    /// 列出当前所有 topic 名称。
    pub fn list_topics(&self) -> Vec<String> {
        let map = self.entries.read();
        let mut topics: Vec<String> = map.keys().cloned().collect();
        topics.sort();
        topics
    }

    /// 全量 GC:遍历所有 topic,清理过期条目。
    ///
    /// 由后台 GC worker 周期调用,也可手动触发。
    #[instrument(skip(self))]
    pub fn gc(&self) -> usize {
        let now = chrono::Utc::now().timestamp();
        let mut map = self.entries.write();
        let mut removed = 0;
        for (_topic, vec) in map.iter_mut() {
            let before = vec.len();
            vec.retain(|e| now - e.last_accessed < self.gc_ttl_secs);
            removed += before - vec.len();
        }
        // 清理空 topic。
        map.retain(|_, v| !v.is_empty());
        if removed > 0 {
            info!(
                target: "nebula.swarm.context_pool",
                removed,
                "GC completed"
            );
        }
        removed
    }

    /// 清空所有条目(保留 subscriber sender)。
    pub fn clear(&self) {
        self.entries.write().clear();
    }

    /// 当前总条目数(所有 topic 求和)。
    pub fn len(&self) -> usize {
        let map = self.entries.read();
        map.values().map(|v| v.len()).sum()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for TeamContextPool {
    fn default() -> Self {
        Self::new()
    }
}

/// 启动后台 GC worker,周期性清理过期条目。
///
/// 返回 `JoinHandle`,调用方可持有以便 shutdown 时 abort。
/// 默认间隔 5 分钟。
pub fn start_gc_worker(pool: Arc<TeamContextPool>) -> tokio::task::JoinHandle<()> {
    start_gc_worker_with_interval(pool, Duration::from_secs(DEFAULT_GC_INTERVAL_SECS))
}

/// 启动后台 GC worker(自定义间隔)。
pub fn start_gc_worker_with_interval(
    pool: Arc<TeamContextPool>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            pool.gc();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_and_get_round_trip() {
        let pool = TeamContextPool::new();
        pool.publish("topic-a", "agent-1", "发现 A");
        pool.publish("topic-a", "agent-2", "确认 A");
        let entries = pool.get("topic-a");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry.author, "agent-1");
        assert_eq!(entries[1].entry.body, "确认 A");
    }

    #[test]
    fn list_topics_sorted() {
        let pool = TeamContextPool::new();
        pool.publish("zeta", "a", "x");
        pool.publish("alpha", "a", "x");
        pool.publish("mid", "a", "x");
        let topics = pool.list_topics();
        assert_eq!(topics, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn get_unknown_topic_returns_empty() {
        let pool = TeamContextPool::new();
        assert!(pool.get("nonexistent").is_empty());
    }

    #[test]
    fn gc_removes_expired_entries() {
        // 使用 TTL=1 秒的池,手动构造过期条目。
        let pool = TeamContextPool::new_with_ttl(1);
        pool.publish("t", "a", "fresh");
        // 手动将条目 last_accessed 回拨到 100 秒前模拟过期。
        {
            let mut map = pool.entries.write();
            let vec = map.get_mut("t").expect("get should succeed");
            for e in vec.iter_mut() {
                e.last_accessed -= 100;
            }
        }
        let removed = pool.gc();
        assert_eq!(removed, 1);
        assert!(pool.get("t").is_empty());
    }

    #[test]
    fn gc_cleans_empty_topics() {
        let pool = TeamContextPool::new_with_ttl(1);
        pool.publish("t1", "a", "x");
        pool.publish("t2", "a", "x");
        // 全部回拨过期。
        {
            let mut map = pool.entries.write();
            for vec in map.values_mut() {
                for e in vec.iter_mut() {
                    e.last_accessed -= 100;
                }
            }
        }
        pool.gc();
        assert!(pool.list_topics().is_empty());
    }

    #[test]
    fn get_resets_last_accessed() {
        let pool = TeamContextPool::new_with_ttl(2);
        pool.publish("t", "a", "x");
        // 回拨到 1 秒前(未过期,接近 TTL)。
        {
            let mut map = pool.entries.write();
            let vec = map.get_mut("t").expect("get should succeed");
            vec[0].last_accessed -= 1;
        }
        // get 会重置 last_accessed。
        let _ = pool.get("t");
        // 再 GC,不应被清理(因为刚被访问)。
        pool.gc();
        assert_eq!(pool.get("t").len(), 1);
    }

    #[tokio::test]
    async fn subscribe_receives_published_entries() {
        let pool = Arc::new(TeamContextPool::new());
        let mut rx = pool.subscribe("live");

        pool.publish("live", "agent-1", "hello");
        pool.publish("live", "agent-2", "world");

        let e1 = rx.recv().await.expect("first entry");
        assert_eq!(e1.entry.author, "agent-1");
        assert_eq!(e1.entry.body, "hello");
        let e2 = rx.recv().await.expect("second entry");
        assert_eq!(e2.entry.body, "world");
    }

    #[tokio::test]
    async fn subscribe_does_not_receive_historical_entries() {
        // broadcast 仅推送订阅之后的消息。
        let pool = TeamContextPool::new();
        pool.publish("t", "a", "before-subscribe");
        let mut rx = pool.subscribe("t");
        pool.publish("t", "a", "after-subscribe");

        let e = rx.recv().await.expect("entry after subscribe");
        assert_eq!(e.entry.body, "after-subscribe");
    }

    #[tokio::test]
    async fn gc_worker_periodically_cleans() {
        let pool = Arc::new(TeamContextPool::new_with_ttl(1));
        pool.publish("t", "a", "x");
        // 回拨过期。
        {
            let mut map = pool.entries.write();
            let vec = map.get_mut("t").expect("get should succeed");
            for e in vec.iter_mut() {
                e.last_accessed -= 100;
            }
        }
        // 启动 100ms 间隔的 GC worker。
        let handle = start_gc_worker_with_interval(pool.clone(), Duration::from_millis(100));
        // 等待 300ms 确保至少触发一次 GC。
        tokio::time::sleep(Duration::from_millis(300)).await;
        handle.abort();
        assert!(pool.get("t").is_empty());
    }

    #[test]
    fn len_and_is_empty() {
        let pool = TeamContextPool::new();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
        pool.publish("t1", "a", "x");
        pool.publish("t1", "a", "y");
        pool.publish("t2", "a", "z");
        assert_eq!(pool.len(), 3);
        assert!(!pool.is_empty());
    }

    #[test]
    fn clear_empties_entries() {
        let pool = TeamContextPool::new();
        pool.publish("t", "a", "x");
        pool.clear();
        assert!(pool.is_empty());
    }
}
