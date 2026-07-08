//! Lightweight in-process metrics.
//!
//! v0.2 deliberately avoids pulling in the full Prometheus client
//! stack. The [`Metrics`] struct is a collection of `AtomicU64`
//! counters accessible via the [`global`] singleton. The Tauri
//! command `metrics()` snapshots the counters into a
//! [`MetricsSnapshot`] for transport to the front-end.
//!
//! All counters are monotonic: increments are positive, resets are
//! not exposed. Snapshotting reads with `Ordering::Relaxed` is
//! safe because the counters are independent.

// v1.8: Prometheus text-exposition HTTP endpoint (gated by env var
// `NEBULA_METRICS_ADDR`). Lives in `metrics/exporter.rs`.
pub mod exporter;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Atomic counter bag. Field naming uses snake_case so the JSON
/// snapshot is directly consumable by the front-end.
pub struct Metrics {
    /// Number of embedder cache hits since process start.
    pub embedding_cache_hits: AtomicU64,
    /// Number of embedder cache misses since process start.
    pub embedding_cache_misses: AtomicU64,
    /// Number of `memory_store` calls that returned `Inserted`.
    pub memory_stores_total: AtomicU64,
    /// Number of `memory_search` calls.
    pub memory_searches_total: AtomicU64,
    /// Number of memory rows absorbed by the black-hole engine.
    pub blackhole_compressions_total: AtomicU64,
    /// Number of L5 reflections produced by the reflection engine.
    pub reflections_generated_total: AtomicU64,
    /// Number of `swarm_execute` invocations.
    pub swarm_executions_total: AtomicU64,
    /// Number of `chat` invocations.
    pub chat_total: AtomicU64,
    // v1.8: 延迟累加器（微秒），配合 count 可算平均延迟。
    /// 向量检索总耗时（微秒）。
    pub memory_search_latency_us_total: AtomicU64,
    /// 向量检索调用次数（含延迟记录的）。
    pub memory_search_latency_count: AtomicU64,
    /// LLM chat 总耗时（微秒）。
    pub llm_chat_latency_us_total: AtomicU64,
    /// LLM chat 调用次数（含延迟记录的）。
    pub llm_chat_latency_count: AtomicU64,
    // T-S1-B-03: 5 项可观测性指标补全。
    /// LLM prompt token 累计（来自 provider usage 字段）。
    pub token_prompt_total: AtomicU64,
    /// LLM completion token 累计。
    pub token_completion_total: AtomicU64,
    /// L0 热缓存命中数（与 L0Cache 内部计数器镜像）。
    pub l0_hits: AtomicU64,
    /// L0 热缓存未命中数。
    pub l0_misses: AtomicU64,
    /// L4 价值层 Allow 裁定数。
    pub l4_allow_total: AtomicU64,
    /// L4 价值层 Confirm 裁定数（需准奏）。
    pub l4_confirm_total: AtomicU64,
    /// L4 价值层 Plan 裁定数（需 Plan 模式）。
    pub l4_plan_total: AtomicU64,
    /// L4 价值层 Deny 裁定数（禁止）。
    pub l4_deny_total: AtomicU64,
    /// ACL 放行数。
    pub acl_allow_total: AtomicU64,
    /// ACL 拒绝数。
    pub acl_deny_total: AtomicU64,
    /// 反思引擎被 RoundGuard skip 的次数。
    pub reflections_skipped_total: AtomicU64,
    // T-E-A-01: L0.5 语义缓存命中/未命中。
    /// L0.5 SemanticCache 命中数。
    pub semantic_cache_hits: AtomicU64,
    /// L0.5 SemanticCache 未命中数。
    pub semantic_cache_misses: AtomicU64,
    // T-E-A-06: Token 费用累加器（micro-cent 单位，1 USD = 10^8 micro-cent）。
    /// LLM 调用累计费用（micro-cent）。`f64` 在并发累加下会丢精度，
    /// 改用整数 micro-cent。
    pub token_cost_usd: AtomicU64,
    // T-E-A-04: Prefix-Cache 指标。
    /// T-E-A-04: Prefix-Cache 命中数(cache_read_input_tokens > 0 的请求数)。
    pub prefix_cache_hits: AtomicU64,
    /// T-E-A-04: Prefix-Cache 累计命中 token 数。
    pub prefix_cache_cached_tokens: AtomicU64,
    // T-E-A-10: 估算节省的金额（micro-cent，1 USD = 10^8 micro-cent）。
    /// T-E-A-10: Prefix-Cache 命中累计节省的金额（micro-cent）。
    pub cost_saved_usd: AtomicU64,
    // T-E-D-02: 首响时间(Time To First Token)累加器(微秒)。
    /// T-E-D-02: TTFT 累计耗时（微秒），配合 ttft_count 可算平均首响。
    pub ttft_us_total: AtomicU64,
    /// T-E-D-02: TTFT 采样次数。
    pub ttft_count: AtomicU64,
}

impl Metrics {
    /// Builds a zeroed metrics bag. The result is *not* a singleton
    /// — for the process-wide counters use [`global`].
    pub const fn new() -> Self {
        Self {
            embedding_cache_hits: AtomicU64::new(0),
            embedding_cache_misses: AtomicU64::new(0),
            memory_stores_total: AtomicU64::new(0),
            memory_searches_total: AtomicU64::new(0),
            blackhole_compressions_total: AtomicU64::new(0),
            reflections_generated_total: AtomicU64::new(0),
            swarm_executions_total: AtomicU64::new(0),
            chat_total: AtomicU64::new(0),
            memory_search_latency_us_total: AtomicU64::new(0),
            memory_search_latency_count: AtomicU64::new(0),
            llm_chat_latency_us_total: AtomicU64::new(0),
            llm_chat_latency_count: AtomicU64::new(0),
            token_prompt_total: AtomicU64::new(0),
            token_completion_total: AtomicU64::new(0),
            l0_hits: AtomicU64::new(0),
            l0_misses: AtomicU64::new(0),
            l4_allow_total: AtomicU64::new(0),
            l4_confirm_total: AtomicU64::new(0),
            l4_plan_total: AtomicU64::new(0),
            l4_deny_total: AtomicU64::new(0),
            acl_allow_total: AtomicU64::new(0),
            acl_deny_total: AtomicU64::new(0),
            reflections_skipped_total: AtomicU64::new(0),
            semantic_cache_hits: AtomicU64::new(0),
            semantic_cache_misses: AtomicU64::new(0),
            token_cost_usd: AtomicU64::new(0),
            prefix_cache_hits: AtomicU64::new(0),
            prefix_cache_cached_tokens: AtomicU64::new(0),
            cost_saved_usd: AtomicU64::new(0),
            ttft_us_total: AtomicU64::new(0),
            ttft_count: AtomicU64::new(0),
        }
    }

    /// Records an embedding cache hit.
    pub fn record_embedding_hit(&self) {
        self.embedding_cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    /// Records an embedding cache miss.
    pub fn record_embedding_miss(&self) {
        self.embedding_cache_misses.fetch_add(1, Ordering::Relaxed);
    }
    /// Records a successful memory store.
    pub fn record_store(&self) {
        self.memory_stores_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Records a memory search.
    pub fn record_search(&self) {
        self.memory_searches_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Records `n` rows compressed by the black-hole engine.
    pub fn record_blackhole(&self, n: u64) {
        self.blackhole_compressions_total
            .fetch_add(n, Ordering::Relaxed);
    }
    /// Records a single reflection.
    pub fn record_reflection(&self) {
        self.reflections_generated_total
            .fetch_add(1, Ordering::Relaxed);
    }
    /// Records a swarm execution.
    pub fn record_swarm(&self) {
        self.swarm_executions_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Records a chat turn.
    pub fn record_chat(&self) {
        self.chat_total.fetch_add(1, Ordering::Relaxed);
    }
    /// v1.8: 记录一次向量检索延迟（微秒）。
    pub fn record_search_latency(&self, us: u64) {
        self.memory_search_latency_us_total
            .fetch_add(us, Ordering::Relaxed);
        self.memory_search_latency_count
            .fetch_add(1, Ordering::Relaxed);
    }
    /// v1.8: 记录一次 LLM chat 延迟（微秒）。
    pub fn record_chat_latency(&self, us: u64) {
        self.llm_chat_latency_us_total
            .fetch_add(us, Ordering::Relaxed);
        self.llm_chat_latency_count.fetch_add(1, Ordering::Relaxed);
    }

    // ---- T-S1-B-03: 5 项新指标采集方法 ----

    /// T-S1-B-03: 记录一次 LLM 调用的 token 用量（来自 provider usage 字段）。
    pub fn record_token_usage(&self, prompt: u64, completion: u64) {
        self.token_prompt_total.fetch_add(prompt, Ordering::Relaxed);
        self.token_completion_total
            .fetch_add(completion, Ordering::Relaxed);
    }
    /// T-S1-B-03: 记录一次 L0 热缓存命中。
    pub fn record_l0_hit(&self) {
        self.l0_hits.fetch_add(1, Ordering::Relaxed);
    }
    /// T-S1-B-03: 记录一次 L0 热缓存未命中。
    pub fn record_l0_miss(&self) {
        self.l0_misses.fetch_add(1, Ordering::Relaxed);
    }
    /// T-S1-B-03: 记录一次 L4 价值层裁定。按 Verdict 变体分桶计数。
    pub fn record_l4_verdict(&self, verdict: &crate::memory::values::Verdict) {
        use crate::memory::values::Verdict;
        match verdict {
            Verdict::Allow => {
                self.l4_allow_total.fetch_add(1, Ordering::Relaxed);
            }
            Verdict::Confirm { .. } => {
                self.l4_confirm_total.fetch_add(1, Ordering::Relaxed);
            }
            Verdict::Plan { .. } => {
                self.l4_plan_total.fetch_add(1, Ordering::Relaxed);
            }
            Verdict::Deny { .. } => {
                self.l4_deny_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    /// T-S1-B-03: 记录一次 ACL 裁定。
    pub fn record_acl_verdict(&self, allowed: bool) {
        if allowed {
            self.acl_allow_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.acl_deny_total.fetch_add(1, Ordering::Relaxed);
        }
    }
    /// T-S1-B-03: 记录一次反思被 RoundGuard skip。
    pub fn record_reflection_skipped(&self) {
        self.reflections_skipped_total
            .fetch_add(1, Ordering::Relaxed);
    }

    // ---- T-E-A-01 / T-E-A-06 新指标采集方法 ----

    /// T-E-A-01: 记录一次 L0.5 语义缓存命中。
    pub fn record_semantic_cache_hit(&self) {
        self.semantic_cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    /// T-E-A-01: 记录一次 L0.5 语义缓存未命中。
    pub fn record_semantic_cache_miss(&self) {
        self.semantic_cache_misses.fetch_add(1, Ordering::Relaxed);
    }
    /// T-E-A-06: 记录一次 LLM 调用的费用（micro-cent 单位）。
    pub fn record_token_cost(&self, micro_cent: u64) {
        self.token_cost_usd.fetch_add(micro_cent, Ordering::Relaxed);
    }

    /// T-E-A-04: 记录一次 Prefix-Cache 命中。
    pub fn record_prefix_cache_hit(&self) {
        self.prefix_cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    /// T-E-A-04: 记录 Prefix-Cache 命中的 token 数。
    pub fn record_prefix_cache_cached_tokens(&self, tokens: u64) {
        self.prefix_cache_cached_tokens
            .fetch_add(tokens, Ordering::Relaxed);
    }

    /// T-E-A-10: 记录一次 Prefix-Cache 命中节省的金额（USD）。
    /// 内部转 micro-cent（1 USD = 10^8 micro-cent）后原子累加。
    pub fn record_cost_saved(&self, usd: f64) {
        if !usd.is_finite() || usd <= 0.0 {
            return;
        }
        let micro_cent = (usd * 100_000_000.0) as u64;
        self.cost_saved_usd.fetch_add(micro_cent, Ordering::Relaxed);
    }

    /// T-E-A-10: 累计节省金额（USD，由 micro-cent 换算）。
    pub fn cost_saved_usd(&self) -> f64 {
        // 1 USD = 10^8 micro-cent
        (self.cost_saved_usd.load(Ordering::Relaxed) as f64) / 100_000_000.0
    }

    // ---- T-E-D-02: TTFT(首响时间)指标 ----

    /// T-E-D-02: 记录一次首响时间（微秒）。
    /// 在 chat_stream 命令收到首个非空 token 时调用。
    pub fn record_ttft(&self, us: u64) {
        self.ttft_us_total.fetch_add(us, Ordering::Relaxed);
        self.ttft_count.fetch_add(1, Ordering::Relaxed);
    }

    /// T-E-D-02: 平均首响时间（微秒）。无数据时返回 0（避免除零）。
    pub fn ttft_avg_us(&self) -> u64 {
        let n = self.ttft_count.load(Ordering::Relaxed);
        self.ttft_us_total
            .load(Ordering::Relaxed)
            .checked_div(n)
            .unwrap_or(0)
    }

    /// Atomically snapshots all counters into a transport-friendly
    /// struct.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            embedding_cache_hits: self.embedding_cache_hits.load(Ordering::Relaxed),
            embedding_cache_misses: self.embedding_cache_misses.load(Ordering::Relaxed),
            memory_stores_total: self.memory_stores_total.load(Ordering::Relaxed),
            memory_searches_total: self.memory_searches_total.load(Ordering::Relaxed),
            blackhole_compressions_total: self.blackhole_compressions_total.load(Ordering::Relaxed),
            reflections_generated_total: self.reflections_generated_total.load(Ordering::Relaxed),
            swarm_executions_total: self.swarm_executions_total.load(Ordering::Relaxed),
            chat_total: self.chat_total.load(Ordering::Relaxed),
            memory_search_latency_us_total: self
                .memory_search_latency_us_total
                .load(Ordering::Relaxed),
            memory_search_latency_count: self.memory_search_latency_count.load(Ordering::Relaxed),
            llm_chat_latency_us_total: self.llm_chat_latency_us_total.load(Ordering::Relaxed),
            llm_chat_latency_count: self.llm_chat_latency_count.load(Ordering::Relaxed),
            token_prompt_total: self.token_prompt_total.load(Ordering::Relaxed),
            token_completion_total: self.token_completion_total.load(Ordering::Relaxed),
            l0_hits: self.l0_hits.load(Ordering::Relaxed),
            l0_misses: self.l0_misses.load(Ordering::Relaxed),
            l4_allow_total: self.l4_allow_total.load(Ordering::Relaxed),
            l4_confirm_total: self.l4_confirm_total.load(Ordering::Relaxed),
            l4_plan_total: self.l4_plan_total.load(Ordering::Relaxed),
            l4_deny_total: self.l4_deny_total.load(Ordering::Relaxed),
            acl_allow_total: self.acl_allow_total.load(Ordering::Relaxed),
            acl_deny_total: self.acl_deny_total.load(Ordering::Relaxed),
            reflections_skipped_total: self.reflections_skipped_total.load(Ordering::Relaxed),
            semantic_cache_hits: self.semantic_cache_hits.load(Ordering::Relaxed),
            semantic_cache_misses: self.semantic_cache_misses.load(Ordering::Relaxed),
            token_cost_usd: self.token_cost_usd.load(Ordering::Relaxed),
            prefix_cache_hits: self.prefix_cache_hits.load(Ordering::Relaxed),
            prefix_cache_cached_tokens: self.prefix_cache_cached_tokens.load(Ordering::Relaxed),
            cost_saved_usd: self.cost_saved_usd.load(Ordering::Relaxed),
        }
    }
}

/// Plain-data snapshot used for transport to the front-end.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricsSnapshot {
    /// Embedder cache hits at snapshot time.
    pub embedding_cache_hits: u64,
    /// Embedder cache misses at snapshot time.
    pub embedding_cache_misses: u64,
    /// Successful memory stores at snapshot time.
    pub memory_stores_total: u64,
    /// Memory searches at snapshot time.
    pub memory_searches_total: u64,
    /// Black-hole compression rows at snapshot time.
    pub blackhole_compressions_total: u64,
    /// Reflections generated at snapshot time.
    pub reflections_generated_total: u64,
    /// Swarm executions at snapshot time.
    pub swarm_executions_total: u64,
    /// Chat turns at snapshot time.
    pub chat_total: u64,
    /// v1.8: 向量检索累计耗时（微秒）。
    pub memory_search_latency_us_total: u64,
    /// v1.8: 向量检索延迟采样次数。
    pub memory_search_latency_count: u64,
    /// v1.8: LLM chat 累计耗时（微秒）。
    pub llm_chat_latency_us_total: u64,
    /// v1.8: LLM chat 延迟采样次数。
    pub llm_chat_latency_count: u64,
    // T-S1-B-03: 5 项新指标
    /// LLM prompt token 累计。
    pub token_prompt_total: u64,
    /// LLM completion token 累计。
    pub token_completion_total: u64,
    /// L0 热缓存命中数。
    pub l0_hits: u64,
    /// L0 热缓存未命中数。
    pub l0_misses: u64,
    /// L4 价值层 Allow 裁定数。
    pub l4_allow_total: u64,
    /// L4 价值层 Confirm 裁定数。
    pub l4_confirm_total: u64,
    /// L4 价值层 Plan 裁定数。
    pub l4_plan_total: u64,
    /// L4 价值层 Deny 裁定数。
    pub l4_deny_total: u64,
    /// ACL 放行数。
    pub acl_allow_total: u64,
    /// ACL 拒绝数。
    pub acl_deny_total: u64,
    /// 反思被 RoundGuard skip 的次数。
    pub reflections_skipped_total: u64,
    // T-E-A-01 / T-E-A-06
    /// T-E-A-01: L0.5 语义缓存命中数。
    pub semantic_cache_hits: u64,
    /// T-E-A-01: L0.5 语义缓存未命中数。
    pub semantic_cache_misses: u64,
    /// T-E-A-06: LLM 调用累计费用（micro-cent，1 USD = 10^8）。
    pub token_cost_usd: u64,
    /// T-E-A-04: Prefix-Cache 命中数。
    pub prefix_cache_hits: u64,
    /// T-E-A-04: Prefix-Cache 累计命中 token 数。
    pub prefix_cache_cached_tokens: u64,
    /// T-E-A-10: Prefix-Cache 命中累计节省的金额（micro-cent，1 USD = 10^8）。
    pub cost_saved_usd: u64,
}

impl MetricsSnapshot {
    /// 向量检索平均延迟（毫秒），无数据返回 0.0。
    pub fn memory_search_avg_latency_ms(&self) -> f64 {
        if self.memory_search_latency_count == 0 {
            0.0
        } else {
            (self.memory_search_latency_us_total as f64)
                / (self.memory_search_latency_count as f64)
                / 1000.0
        }
    }
    /// LLM chat 平均延迟（毫秒），无数据返回 0.0。
    pub fn llm_chat_avg_latency_ms(&self) -> f64 {
        if self.llm_chat_latency_count == 0 {
            0.0
        } else {
            (self.llm_chat_latency_us_total as f64) / (self.llm_chat_latency_count as f64) / 1000.0
        }
    }
}

impl MetricsSnapshot {
    /// Embedder cache hit ratio in `[0.0, 1.0]`. Returns `0.0` when
    /// no lookups have been performed.
    pub fn embedding_cache_hit_ratio(&self) -> f32 {
        let total = self.embedding_cache_hits + self.embedding_cache_misses;
        if total == 0 {
            0.0
        } else {
            self.embedding_cache_hits as f32 / total as f32
        }
    }

    /// T-S1-B-03: L0 热缓存命中率 `[0.0, 1.0]`,无数据返回 0.0。
    pub fn l0_hit_ratio(&self) -> f32 {
        let total = self.l0_hits + self.l0_misses;
        if total == 0 {
            0.0
        } else {
            self.l0_hits as f32 / total as f32
        }
    }

    /// T-S1-B-03: L4 价值层拦截率 = (Confirm + Plan + Deny) / 总裁定数。
    /// "拦截"定义为"非直接放行",即需要用户介入或被禁止。
    pub fn l4_block_ratio(&self) -> f32 {
        let total =
            self.l4_allow_total + self.l4_confirm_total + self.l4_plan_total + self.l4_deny_total;
        if total == 0 {
            0.0
        } else {
            let blocked = self.l4_confirm_total + self.l4_plan_total + self.l4_deny_total;
            blocked as f32 / total as f32
        }
    }

    /// T-S1-B-03: ACL 拒绝率 `[0.0, 1.0]`。
    pub fn acl_deny_ratio(&self) -> f32 {
        let total = self.acl_allow_total + self.acl_deny_total;
        if total == 0 {
            0.0
        } else {
            self.acl_deny_total as f32 / total as f32
        }
    }

    /// T-S1-B-03: token 总数 = prompt + completion。
    pub fn token_total(&self) -> u64 {
        self.token_prompt_total + self.token_completion_total
    }

    /// T-E-A-01: L0.5 语义缓存命中率 `[0.0, 1.0]`。
    pub fn semantic_cache_hit_ratio(&self) -> f32 {
        let total = self.semantic_cache_hits + self.semantic_cache_misses;
        if total == 0 {
            0.0
        } else {
            self.semantic_cache_hits as f32 / total as f32
        }
    }

    /// T-E-A-06: 累计 LLM 费用（USD，由 micro-cent 换算）。
    pub fn token_cost_usd(&self) -> f64 {
        // 1 USD = 10^8 micro-cent
        (self.token_cost_usd as f64) / 100_000_000.0
    }

    /// T-E-A-04: Prefix-Cache 命中率 [0.0, 1.0](命中请求数 / chat 总数)。
    /// 注意:分母使用 chat_total,包含所有 provider 的 chat 调用。
    pub fn prefix_cache_hit_ratio(&self) -> f32 {
        let total = self.chat_total;
        if total == 0 {
            0.0
        } else {
            self.prefix_cache_hits as f32 / total as f32
        }
    }
}

/// Process-wide metrics singleton. Initialised lazily on first access.
pub fn global() -> &'static Metrics {
    static GLOBAL: OnceLock<Metrics> = OnceLock::new();
    GLOBAL.get_or_init(Metrics::new)
}

// ---- T-E-D-02: TTFT 命令返回结构 ----

/// T-E-D-02: metrics_ttft 命令返回的 TTFT 统计快照。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TtftStats {
    /// 平均首响时间（微秒）。
    pub avg_us: u64,
    /// 采样次数。
    pub count: u64,
}

/// T-E-D-02: 从给定 Metrics 构造 TTFT 统计快照。
/// `metrics_ttft` Tauri 命令调用此函数(传入 global()),
/// 单测传入本地 Metrics 实例避免共享全局状态。
pub fn build_ttft_stats(metrics: &Metrics) -> TtftStats {
    TtftStats {
        avg_us: metrics.ttft_avg_us(),
        count: metrics.ttft_count.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_snapshot_is_zero() {
        // We cannot use `global()` here because tests share state; use
        // a local bag instead.
        let m = Metrics::new();
        let s = m.snapshot();
        assert_eq!(s.embedding_cache_hits, 0);
        assert_eq!(s.embedding_cache_misses, 0);
        assert_eq!(s.memory_stores_total, 0);
        assert_eq!(s.memory_searches_total, 0);
        assert_eq!(s.blackhole_compressions_total, 0);
        assert_eq!(s.reflections_generated_total, 0);
        assert_eq!(s.swarm_executions_total, 0);
        assert_eq!(s.chat_total, 0);
    }

    #[test]
    fn recorders_increment_correctly() {
        let m = Metrics::new();
        m.record_embedding_hit();
        m.record_embedding_hit();
        m.record_embedding_miss();
        m.record_store();
        m.record_search();
        m.record_blackhole(3);
        m.record_reflection();
        m.record_swarm();
        m.record_chat();
        m.record_search_latency(1500);
        m.record_chat_latency(800_000);
        m.record_token_usage(120, 40);
        m.record_l0_hit();
        m.record_l0_miss();
        m.record_acl_verdict(true);
        m.record_reflection_skipped();
        let s = m.snapshot();
        assert_eq!(s.embedding_cache_hits, 2);
        assert_eq!(s.embedding_cache_misses, 1);
        assert_eq!(s.memory_stores_total, 1);
        assert_eq!(s.memory_searches_total, 1);
        assert_eq!(s.blackhole_compressions_total, 3);
        assert_eq!(s.reflections_generated_total, 1);
        assert_eq!(s.swarm_executions_total, 1);
        assert_eq!(s.chat_total, 1);
        assert_eq!(s.memory_search_latency_us_total, 1500);
        assert_eq!(s.memory_search_latency_count, 1);
        assert_eq!(s.llm_chat_latency_us_total, 800_000);
        assert_eq!(s.llm_chat_latency_count, 1);
        assert_eq!(s.token_prompt_total, 120);
        assert_eq!(s.token_completion_total, 40);
        assert_eq!(s.l0_hits, 1);
        assert_eq!(s.l0_misses, 1);
        assert_eq!(s.acl_allow_total, 1);
        assert_eq!(s.reflections_skipped_total, 1);
    }

    #[test]
    fn hit_ratio_handles_zero_total() {
        let s = MetricsSnapshot::default();
        assert_eq!(s.embedding_cache_hit_ratio(), 0.0);
    }

    #[test]
    fn hit_ratio_computes_correctly() {
        let s = MetricsSnapshot {
            embedding_cache_hits: 3,
            embedding_cache_misses: 1,
            ..Default::default()
        };
        assert!((s.embedding_cache_hit_ratio() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn global_singleton_is_stable() {
        let a = global();
        let b = global();
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn snapshot_serialises_to_json() {
        let s = MetricsSnapshot {
            memory_stores_total: 5,
            ..Default::default()
        };
        let j = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(j.contains("\"memory_stores_total\":5"));
    }

    // ---- T-S1-B-03 新指标测试 ----

    #[test]
    fn record_token_usage_accumulates() {
        let m = Metrics::new();
        m.record_token_usage(100, 50);
        m.record_token_usage(200, 80);
        let s = m.snapshot();
        assert_eq!(s.token_prompt_total, 300);
        assert_eq!(s.token_completion_total, 130);
        assert_eq!(s.token_total(), 430);
    }

    #[test]
    fn record_l0_hit_miss_counts() {
        let m = Metrics::new();
        m.record_l0_hit();
        m.record_l0_hit();
        m.record_l0_miss();
        let s = m.snapshot();
        assert_eq!(s.l0_hits, 2);
        assert_eq!(s.l0_misses, 1);
        assert!((m.snapshot().l0_hit_ratio() - (2.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn record_l4_verdict_buckets() {
        use crate::memory::values::Verdict;
        let m = Metrics::new();
        m.record_l4_verdict(&Verdict::Allow);
        m.record_l4_verdict(&Verdict::Allow);
        m.record_l4_verdict(&Verdict::Confirm { prompt: "p".into() });
        m.record_l4_verdict(&Verdict::Plan { prompt: "p".into() });
        m.record_l4_verdict(&Verdict::Deny { reason: "r".into() });
        let s = m.snapshot();
        assert_eq!(s.l4_allow_total, 2);
        assert_eq!(s.l4_confirm_total, 1);
        assert_eq!(s.l4_plan_total, 1);
        assert_eq!(s.l4_deny_total, 1);
        // 拦截率 = (1+1+1) / 5 = 0.6
        assert!((m.snapshot().l4_block_ratio() - 0.6).abs() < 1e-6);
    }

    #[test]
    fn record_acl_verdict_counts() {
        let m = Metrics::new();
        m.record_acl_verdict(true);
        m.record_acl_verdict(true);
        m.record_acl_verdict(false);
        let s = m.snapshot();
        assert_eq!(s.acl_allow_total, 2);
        assert_eq!(s.acl_deny_total, 1);
        assert!((m.snapshot().acl_deny_ratio() - (1.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn record_reflection_skipped_increments() {
        let m = Metrics::new();
        m.record_reflection_skipped();
        m.record_reflection_skipped();
        let s = m.snapshot();
        assert_eq!(s.reflections_skipped_total, 2);
    }

    #[test]
    fn l0_hit_ratio_zero_when_no_data() {
        let s = MetricsSnapshot::default();
        assert_eq!(s.l0_hit_ratio(), 0.0);
        assert_eq!(s.l4_block_ratio(), 0.0);
        assert_eq!(s.acl_deny_ratio(), 0.0);
        assert_eq!(s.token_total(), 0);
    }

    // ---- T-E-A-01 / T-E-A-06 新指标测试 ----

    #[test]
    fn record_semantic_cache_hit_miss_counts() {
        let m = Metrics::new();
        m.record_semantic_cache_hit();
        m.record_semantic_cache_hit();
        m.record_semantic_cache_miss();
        let s = m.snapshot();
        assert_eq!(s.semantic_cache_hits, 2);
        assert_eq!(s.semantic_cache_misses, 1);
        assert!((s.semantic_cache_hit_ratio() - (2.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn semantic_cache_hit_ratio_zero_when_no_data() {
        let s = MetricsSnapshot::default();
        assert_eq!(s.semantic_cache_hit_ratio(), 0.0);
    }

    #[test]
    fn record_token_cost_accumulates_micro_cent() {
        let m = Metrics::new();
        m.record_token_cost(42_000_000); // $0.42
        m.record_token_cost(7_000_000); //  $0.07
        let s = m.snapshot();
        assert_eq!(s.token_cost_usd, 49_000_000);
        assert!((s.token_cost_usd() - 0.49).abs() < 1e-9);
    }

    #[test]
    fn token_cost_zero_when_no_data() {
        let s = MetricsSnapshot::default();
        assert_eq!(s.token_cost_usd(), 0.0);
    }

    // ---- T-E-D-02: TTFT 指标测试 ----

    /// T-E-D-02: record_ttft 累加 3 次后,avg_us = total / count 正确。
    #[test]
    fn test_record_ttft() {
        let m = Metrics::new();
        m.record_ttft(1_000);
        m.record_ttft(3_000);
        m.record_ttft(5_000);
        assert_eq!(m.ttft_count.load(Ordering::Relaxed), 3);
        assert_eq!(m.ttft_us_total.load(Ordering::Relaxed), 9_000);
        // avg = 9000 / 3 = 3000
        assert_eq!(m.ttft_avg_us(), 3_000);
    }

    /// T-E-D-02: 0 次采样时 ttft_avg_us 返回 0,不 panic(避免除零)。
    #[test]
    fn test_ttft_avg_empty() {
        let m = Metrics::new();
        assert_eq!(m.ttft_count.load(Ordering::Relaxed), 0);
        assert_eq!(m.ttft_avg_us(), 0);
    }

    /// T-E-D-02: build_ttft_stats 返回正确 TtftStats(metrics_ttft 命令核心逻辑)。
    #[test]
    fn test_metrics_ttft_command() {
        let m = Metrics::new();
        m.record_ttft(2_000);
        m.record_ttft(4_000);
        let stats = build_ttft_stats(&m);
        assert_eq!(stats.count, 2);
        assert_eq!(stats.avg_us, 3_000);
        // 空状态
        let empty = Metrics::new();
        let empty_stats = build_ttft_stats(&empty);
        assert_eq!(empty_stats.count, 0);
        assert_eq!(empty_stats.avg_us, 0);
    }
}
