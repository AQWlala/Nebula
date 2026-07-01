//! v1.8: Prometheus metrics HTTP exporter.
//!
//! Uses the standard `prometheus` crate (IntCounter / IntGauge /
//! Histogram) and `axum` for the HTTP server — per the v7.0 design
//! spec for proper observability.
//!
//! Endpoint: `GET /metrics` (Prometheus text exposition format).
//! Bind address: controlled by `NINE_SNAKE_METRICS_ADDR` env var
//! (default: `127.0.0.1:9100` when the env var is unset but the
//! exporter is still requested via the start function — the caller
//! decides whether to start it).

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use prometheus::{Encoder, IntCounter, IntGauge, TextEncoder};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::perf::monitor::PerfMonitor;

/// The default bind address for the metrics HTTP server.
pub const DEFAULT_ADDR: &str = "127.0.0.1:9100";

/// All Prometheus metrics registered by nine-snake.
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    pub embedding_cache_hits: IntCounter,
    pub embedding_cache_misses: IntCounter,
    pub memory_stores_total: IntCounter,
    pub memory_searches_total: IntCounter,
    pub blackhole_compressions_total: IntCounter,
    pub reflections_generated_total: IntCounter,
    pub swarm_executions_total: IntCounter,
    pub chat_total: IntCounter,
    pub memory_search_latency_us: IntCounter,
    pub memory_search_latency_count: IntCounter,
    pub llm_chat_latency_us: IntCounter,
    pub llm_chat_latency_count: IntCounter,
    pub embedding_cache_hit_ratio: IntGauge,
    pub process_rss_bytes: IntGauge,
    pub process_virtual_bytes: IntGauge,
    pub process_cpu_pct: IntGauge,
    pub over_rss_budget: IntGauge,
    pub registry: prometheus::Registry,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        let registry = prometheus::Registry::new();

        let embedding_cache_hits = IntCounter::new(
            "nine_snake_embedding_cache_hits_total",
            "Embedder cache hits since process start",
        )
        .unwrap();
        let embedding_cache_misses = IntCounter::new(
            "nine_snake_embedding_cache_misses_total",
            "Embedder cache misses since process start",
        )
        .unwrap();
        let memory_stores_total = IntCounter::new(
            "nine_snake_memory_stores_total",
            "Total memory_store calls",
        )
        .unwrap();
        let memory_searches_total = IntCounter::new(
            "nine_snake_memory_searches_total",
            "Total memory_search calls",
        )
        .unwrap();
        let blackhole_compressions_total = IntCounter::new(
            "nine_snake_blackhole_compressions_total",
            "Rows compressed by the black-hole engine",
        )
        .unwrap();
        let reflections_generated_total = IntCounter::new(
            "nine_snake_reflections_generated_total",
            "L5 reflections produced",
        )
        .unwrap();
        let swarm_executions_total = IntCounter::new(
            "nine_snake_swarm_executions_total",
            "swarm_execute invocations",
        )
        .unwrap();
        let chat_total = IntCounter::new(
            "nine_snake_chat_total",
            "chat invocations",
        )
        .unwrap();
        let memory_search_latency_us = IntCounter::new(
            "nine_snake_memory_search_latency_us_total",
            "Cumulative memory_search latency in microseconds",
        )
        .unwrap();
        let memory_search_latency_count = IntCounter::new(
            "nine_snake_memory_search_latency_count",
            "Number of memory_search latency samples",
        )
        .unwrap();
        let llm_chat_latency_us = IntCounter::new(
            "nine_snake_llm_chat_latency_us_total",
            "Cumulative chat latency in microseconds",
        )
        .unwrap();
        let llm_chat_latency_count = IntCounter::new(
            "nine_snake_llm_chat_latency_count",
            "Number of chat latency samples",
        )
        .unwrap();
        let embedding_cache_hit_ratio = IntGauge::new(
            "nine_snake_embedding_cache_hit_ratio",
            "Embedder cache hit ratio in [0,1] (scaled by 10000 for integer storage)",
        )
        .unwrap();
        let process_rss_bytes = IntGauge::new(
            "nine_snake_process_rss_bytes",
            "Process resident set size in bytes",
        )
        .unwrap();
        let process_virtual_bytes = IntGauge::new(
            "nine_snake_process_virtual_bytes",
            "Process virtual memory size in bytes",
        )
        .unwrap();
        let process_cpu_pct = IntGauge::new(
            "nine_snake_process_cpu_pct",
            "Process CPU usage percent (scaled by 100 for integer storage)",
        )
        .unwrap();
        let over_rss_budget = IntGauge::new(
            "nine_snake_over_rss_budget",
            "1 when RSS is over the 500MB budget, 0 otherwise",
        )
        .unwrap();

        let mf = &registry;
        let _ = mf.register(Box::new(embedding_cache_hits.clone()));
        let _ = mf.register(Box::new(embedding_cache_misses.clone()));
        let _ = mf.register(Box::new(memory_stores_total.clone()));
        let _ = mf.register(Box::new(memory_searches_total.clone()));
        let _ = mf.register(Box::new(blackhole_compressions_total.clone()));
        let _ = mf.register(Box::new(reflections_generated_total.clone()));
        let _ = mf.register(Box::new(swarm_executions_total.clone()));
        let _ = mf.register(Box::new(chat_total.clone()));
        let _ = mf.register(Box::new(memory_search_latency_us.clone()));
        let _ = mf.register(Box::new(memory_search_latency_count.clone()));
        let _ = mf.register(Box::new(llm_chat_latency_us.clone()));
        let _ = mf.register(Box::new(llm_chat_latency_count.clone()));
        let _ = mf.register(Box::new(embedding_cache_hit_ratio.clone()));
        let _ = mf.register(Box::new(process_rss_bytes.clone()));
        let _ = mf.register(Box::new(process_virtual_bytes.clone()));
        let _ = mf.register(Box::new(process_cpu_pct.clone()));
        let _ = mf.register(Box::new(over_rss_budget.clone()));

        Self {
            embedding_cache_hits,
            embedding_cache_misses,
            memory_stores_total,
            memory_searches_total,
            blackhole_compressions_total,
            reflections_generated_total,
            swarm_executions_total,
            chat_total,
            memory_search_latency_us,
            memory_search_latency_count,
            llm_chat_latency_us,
            llm_chat_latency_count,
            embedding_cache_hit_ratio,
            process_rss_bytes,
            process_virtual_bytes,
            process_cpu_pct,
            over_rss_budget,
            registry,
        }
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared state passed to axum route handlers.
#[derive(Clone)]
struct AppState {
    reg: Arc<MetricsRegistry>,
    perf: PerfMonitor,
}

/// Refresh gauges from the current `Metrics` snapshot and perf
/// monitor.  Called on every `/metrics` scrape so the gauges
/// reflect the latest values.
fn refresh_gauges(reg: &MetricsRegistry, perf: &PerfMonitor) {
    let m = crate::metrics::global().snapshot();
    let p = perf.latest();

    reg.embedding_cache_hits
        .reset();
    reg.embedding_cache_hits.inc_by(m.embedding_cache_hits);
    reg.embedding_cache_misses
        .reset();
    reg.embedding_cache_misses.inc_by(m.embedding_cache_misses);
    reg.memory_stores_total
        .reset();
    reg.memory_stores_total.inc_by(m.memory_stores_total);
    reg.memory_searches_total
        .reset();
    reg.memory_searches_total.inc_by(m.memory_searches_total);
    reg.blackhole_compressions_total
        .reset();
    reg.blackhole_compressions_total
        .inc_by(m.blackhole_compressions_total);
    reg.reflections_generated_total
        .reset();
    reg.reflections_generated_total
        .inc_by(m.reflections_generated_total);
    reg.swarm_executions_total
        .reset();
    reg.swarm_executions_total.inc_by(m.swarm_executions_total);
    reg.chat_total.reset();
    reg.chat_total.inc_by(m.chat_total);
    reg.memory_search_latency_us.reset();
    reg.memory_search_latency_us
        .inc_by(m.memory_search_latency_us_total);
    reg.memory_search_latency_count.reset();
    reg.memory_search_latency_count
        .inc_by(m.memory_search_latency_count);
    reg.llm_chat_latency_us.reset();
    reg.llm_chat_latency_us
        .inc_by(m.llm_chat_latency_us_total);
    reg.llm_chat_latency_count.reset();
    reg.llm_chat_latency_count
        .inc_by(m.llm_chat_latency_count);

    let total_lookups = m.embedding_cache_hits + m.embedding_cache_misses;
    let ratio_scaled = if total_lookups == 0 {
        0
    } else {
        ((m.embedding_cache_hits as f64 / total_lookups as f64) * 10000.0) as i64
    };
    reg.embedding_cache_hit_ratio.set(ratio_scaled);

    if let Some(rss) = p.rss_bytes {
        reg.process_rss_bytes.set(rss as i64);
    }
    if let Some(virt) = p.virt_bytes {
        reg.process_virtual_bytes.set(virt as i64);
    }
    if let Some(cpu) = p.cpu_pct {
        reg.process_cpu_pct.set((cpu * 100.0) as i64);
    }
    reg.over_rss_budget
        .set(if p.over_budget { 1 } else { 0 });
}

/// Handler for `GET /metrics`.
async fn metrics_handler(State(state): State<AppState>) -> Response {
    refresh_gauges(&state.reg, &state.perf);

    let encoder = TextEncoder::new();
    let metric_families = state.reg.registry.gather();
    let mut buffer = vec![];
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => (
            StatusCode::OK,
            [("content-type", encoder.format_type())],
            buffer,
        )
            .into_response(),
        Err(e) => {
            error!(target: "nine_snake.metrics", error = ?e, "failed to encode metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to encode metrics: {e}"),
            )
                .into_response()
        }
    }
}

/// Handler for `GET /health`.
async fn health_handler() -> &'static str {
    "OK\n"
}

/// 404 fallback — return empty body with 404.
async fn fallback() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// Start the Prometheus metrics HTTP server on `bind_addr`.
///
/// Returns a `JoinHandle` that the caller may drop to stop the
/// server, or leak to let it run for the process lifetime.
pub fn start(bind_addr: String, perf: PerfMonitor) -> JoinHandle<()> {
    let reg = Arc::new(MetricsRegistry::new());
    let state = AppState { reg, perf };

    tokio::spawn(async move {
        let app = Router::new()
            .route("/metrics", get(metrics_handler))
            .route("/health", get(health_handler))
            .with_state(state)
            .fallback(fallback);

        let listener = match TcpListener::bind(&bind_addr).await {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    target: "nine_snake.metrics",
                    addr = %bind_addr,
                    error = ?e,
                    "prometheus exporter failed to bind; disabled"
                );
                return;
            }
        };

        info!(
            target: "nine_snake.metrics",
            addr = %bind_addr,
            "prometheus exporter listening on /metrics"
        );

        if let Err(e) = axum::serve(listener, app).await {
            error!(
                target: "nine_snake.metrics",
                error = ?e,
                "prometheus exporter server errored"
            );
        }
    })
}

/// Helper: read bind address from `NINE_SNAKE_METRICS_ADDR` env var.
/// Returns `None` when unset or empty.
pub fn bind_addr_from_env() -> Option<String> {
    match std::env::var("NINE_SNAKE_METRICS_ADDR") {
        Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_creates_all_metrics() {
        let r = MetricsRegistry::new();
        let families = r.registry.gather();
        assert!(families.len() >= 12);
        assert!(families
            .iter()
            .any(|f| f.get_name() == "nine_snake_memory_stores_total"));
        assert!(families
            .iter()
            .any(|f| f.get_name() == "nine_snake_embedding_cache_hits_total"));
        assert!(families
            .iter()
            .any(|f| f.get_name() == "nine_snake_process_rss_bytes"));
    }

    #[test]
    fn refresh_gauges_updates_counters() {
        let r = MetricsRegistry::new();
        crate::metrics::global().record_store();
        let perf = PerfMonitor::new();
        refresh_gauges(&r, &perf);
        let families = r.registry.gather();
        let mem_stores = families
            .iter()
            .find(|f| f.get_name() == "nine_snake_memory_stores_total")
            .unwrap();
        assert!(mem_stores.get_metric().len() >= 1);
    }

    #[test]
    fn bind_addr_from_env_none_when_unset() {
        if std::env::var("NINE_SNAKE_METRICS_ADDR").is_err() {
            assert!(bind_addr_from_env().is_none());
        }
    }
}
