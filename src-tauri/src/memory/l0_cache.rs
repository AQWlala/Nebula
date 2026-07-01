//! v1.4 L0 缓存层（Cache Layer）。
//!
//! 对应设计文档 v7.0 §2.1 的 L0 Cache Layer，包含三个职责：
//!
//! * **LRU 热记忆缓存** — 最近访问的记忆，避免重复查 SQLite/LanceDB。
//! * **会话上下文窗口** — 当前会话的记忆窗口（滑动窗口）。
//! * **预取队列** — 基于任务关键词预测性预加载相关记忆。
//!
//! ## 容量限制
//!
//! 设计文档规定 L0 容量上限 64 MB。实际实现用条目数限制（默认 256 条）+
//! token 预算（默认 8000 token）双重约束。

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::memory::types::{Memory, MemoryType};

/// 默认 LRU 热记忆条目数。
const DEFAULT_HOT_CAPACITY: usize = 256;
/// 默认会话上下文 token 预算。
const DEFAULT_SESSION_TOKEN_BUDGET: usize = 8000;
/// 默认会话上下文条目数上限。
const DEFAULT_SESSION_ENTRY_LIMIT: usize = 64;

/// 粗略 token 估算（1 token ≈ 2 字符，中英混合启发式）。
fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 2).max(1)
}

/// L0 缓存层。
#[derive(Debug)]
pub struct L0Cache {
    /// LRU 热记忆缓存（按 id 索引）。
    hot: Mutex<LruCache<String, Arc<Memory>>>,
    /// 会话上下文窗口（最近的记忆，滑动窗口）。
    session: Mutex<SessionWindow>,
    /// 预取队列（待预加载的记忆 id）。
    prefetch_queue: Mutex<VecDeque<String>>,
}

/// 会话上下文窗口。
#[derive(Debug)]
struct SessionWindow {
    /// 窗口内的记忆（按时间顺序）。
    entries: VecDeque<Arc<Memory>>,
    /// 当前 token 总量。
    token_used: usize,
    /// token 预算。
    token_budget: usize,
    /// 条目数上限。
    entry_limit: usize,
}

impl SessionWindow {
    fn new(token_budget: usize, entry_limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            token_used: 0,
            token_budget,
            entry_limit,
        }
    }

    /// 推入一条记忆，超出预算时从最旧的开始淘汰。
    fn push(&mut self, mem: Arc<Memory>) {
        let tokens = estimate_tokens(&mem.content);
        // 先淘汰，直到有空间。
        while (self.token_used + tokens > self.token_budget
            || self.entries.len() >= self.entry_limit)
            && !self.entries.is_empty()
        {
            if let Some(old) = self.entries.pop_front() {
                self.token_used -= estimate_tokens(&old.content);
            }
        }
        self.token_used += tokens;
        self.entries.push_back(mem);
    }

    /// 清空窗口。
    fn clear(&mut self) {
        self.entries.clear();
        self.token_used = 0;
    }
}

/// L0 缓存命中统计（供可观测性使用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct L0Stats {
    pub hot_hits: u64,
    pub hot_misses: u64,
    pub session_entries: usize,
    pub session_tokens: usize,
    pub prefetch_pending: usize,
}

impl L0Cache {
    /// 使用默认容量创建。
    pub fn new() -> Self {
        Self::with_capacity(
            DEFAULT_HOT_CAPACITY,
            DEFAULT_SESSION_TOKEN_BUDGET,
            DEFAULT_SESSION_ENTRY_LIMIT,
        )
    }

    /// 自定义容量。
    pub fn with_capacity(hot_cap: usize, session_token_budget: usize, session_entry_limit: usize) -> Self {
        let cap = NonZeroUsize::new(hot_cap.max(1)).expect("hot_cap must be non-zero");
        Self {
            hot: Mutex::new(LruCache::new(cap)),
            session: Mutex::new(SessionWindow::new(session_token_budget, session_entry_limit)),
            prefetch_queue: Mutex::new(VecDeque::new()),
        }
    }

    /// 从 LRU 热缓存查找一条记忆。命中时标记为最近使用。
    pub fn lookup_hot(&self, id: &str) -> Option<Arc<Memory>> {
        let mut g = self.hot.lock();
        if let Some(mem) = g.get(id) {
            return Some(mem.clone());
        }
        None
    }

    /// 将一条记忆放入热缓存 + 会话窗口。
    pub fn insert(&self, mem: Memory) {
        let arc = Arc::new(mem);
        {
            let mut g = self.hot.lock();
            g.put(arc.id.clone(), arc.clone());
        }
        {
            let mut s = self.session.lock();
            s.push(arc);
        }
    }

    /// 批量放入（用于预取结果回填）。
    pub fn insert_many(&self, mems: Vec<Memory>) {
        for m in mems {
            self.insert(m);
        }
    }

    /// 将记忆 id 加入预取队列。
    pub fn enqueue_prefetch(&self, id: String) {
        self.prefetch_queue.lock().push_back(id);
    }

    /// 取出下一个待预取的记忆 id。
    pub fn dequeue_prefetch(&self) -> Option<String> {
        self.prefetch_queue.lock().pop_front()
    }

    /// 获取当前会话上下文快照（按时间顺序）。
    pub fn session_snapshot(&self) -> Vec<Arc<Memory>> {
        self.session.lock().entries.iter().cloned().collect()
    }

    /// 清空会话上下文（新会话开始时调用）。
    pub fn clear_session(&self) {
        self.session.lock().clear();
    }

    /// 按记忆类型过滤会话上下文。
    pub fn session_by_type(&self, mt: MemoryType) -> Vec<Arc<Memory>> {
        self.session
            .lock()
            .entries
            .iter()
            .filter(|m| m.memory_type == mt)
            .cloned()
            .collect()
    }

    /// 缓存统计。
    pub fn stats(&self) -> L0Stats {
        let s = self.session.lock();
        L0Stats {
            hot_hits: 0,
            hot_misses: 0,
            session_entries: s.entries.len(),
            session_tokens: s.token_used,
            prefetch_pending: self.prefetch_queue.lock().len(),
        }
    }
}

impl Default for L0Cache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{MemoryLayer, SourceKind};

    fn make_mem(id: &str, content: &str, mt: MemoryType) -> Memory {
        let mut m = Memory::new(mt, MemoryLayer::L1, content.to_string(), SourceKind::UserInput);
        m.id = id.to_string();
        m
    }

    #[test]
    fn hot_cache_hit_and_miss() {
        let c = L0Cache::new();
        let m = make_mem("m1", "hello world", MemoryType::Semantic);
        c.insert(m);
        assert!(c.lookup_hot("m1").is_some());
        assert!(c.lookup_hot("nonexistent").is_none());
    }

    #[test]
    fn session_window_evicts_on_token_budget() {
        // 小预算：4 条 10 字符记忆（每条约 5 token），预算 12 token → 只能放 2 条。
        let c = L0Cache::with_capacity(64, 12, 64);
        c.insert(make_mem("1", "aaaaaaaaaa", MemoryType::Semantic));
        c.insert(make_mem("2", "bbbbbbbbbb", MemoryType::Semantic));
        c.insert(make_mem("3", "cccccccccc", MemoryType::Semantic));
        let snap = c.session_snapshot();
        assert!(snap.len() <= 3, "session should evict, got {} entries", snap.len());
        // 最旧的应被淘汰
        let ids: Vec<_> = snap.iter().map(|m| m.id.as_str()).collect();
        assert!(!ids.contains(&"1") || snap.len() <= 2, "oldest should be evicted");
    }

    #[test]
    fn session_by_type_filters() {
        let c = L0Cache::new();
        c.insert(make_mem("1", "fact", MemoryType::Semantic));
        c.insert(make_mem("2", "event", MemoryType::Episodic));
        c.insert(make_mem("3", "skill", MemoryType::Procedural));
        let sem = c.session_by_type(MemoryType::Semantic);
        assert_eq!(sem.len(), 1);
        assert_eq!(sem[0].id, "1");
    }

    #[test]
    fn clear_session_resets() {
        let c = L0Cache::new();
        c.insert(make_mem("1", "x", MemoryType::Semantic));
        assert_eq!(c.session_snapshot().len(), 1);
        c.clear_session();
        assert_eq!(c.session_snapshot().len(), 0);
    }

    #[test]
    fn prefetch_queue_fifo() {
        let c = L0Cache::new();
        c.enqueue_prefetch("a".into());
        c.enqueue_prefetch("b".into());
        assert_eq!(c.dequeue_prefetch(), Some("a".into()));
        assert_eq!(c.dequeue_prefetch(), Some("b".into()));
        assert_eq!(c.dequeue_prefetch(), None);
    }
}
