//! P1-1: 模型健康追踪器 — 记录每个 provider 的延迟 / 错误 / 断路器状态。
//!
//! `ModelHealthTracker` 是一个进程内的轻量级指标收集器,挂在 `LlmGateway`
//! 上,每次 provider 调用结束(成功或失败)时由 `record_request` 写入。
//! `get_model_health` Tauri 命令读取这些指标并合并 CostTracker / metrics
//! 数据,生成 `ModelHealthInfo` 供前端健康面板展示。
//!
//! ## 设计要点
//! - 使用 `parking_lot::RwLock<HashMap<String, ProviderMetrics>>` 保护并发读写。
//! - 仅保留最近一次延迟 / 错误 / 请求时间(非滑动窗口),满足面板展示需求。
//! - `total_requests` / `failed_requests` 是累计计数,用于计算错误率。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde::Serialize;

/// 单个 provider 的健康指标快照。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProviderMetrics {
    /// 最近一次请求延迟(毫秒)。None 表示尚无请求。
    pub latency_ms: Option<u64>,
    /// 最近一次错误信息(成功后清空)。None 表示无错误。
    pub last_error: Option<String>,
    /// 最近一次请求的 Unix 时间戳(秒)。None 表示尚无请求。
    pub last_request_at: Option<u64>,
    /// 断路器状态字符串("Closed" / "Open" / "HalfOpen")。
    pub circuit_breaker_status: String,
    /// 累计请求总数。
    pub total_requests: u64,
    /// 累计失败请求数。
    pub failed_requests: u64,
}

/// 模型健康追踪器。线程安全,可挂在 `LlmGateway` 上。
///
/// 通过 `Arc<ModelHealthTracker>` 共享,`record_request` 写入,
/// `get_metrics` / `get_all_metrics` 读取。
pub struct ModelHealthTracker {
    metrics: Arc<RwLock<HashMap<String, ProviderMetrics>>>,
}

impl Default for ModelHealthTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelHealthTracker {
    /// 创建一个空的追踪器。
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 记录一次 provider 请求的结果。
    ///
    /// - `provider_id`: provider 标识(如 "deepseek" / "ollama")。
    /// - `latency_ms`: 本次请求延迟(毫秒)。
    /// - `success`: 是否成功。
    /// - `error`: 失败时的错误信息(成功时传 None)。
    pub fn record_request(
        &self,
        provider_id: &str,
        latency_ms: u64,
        success: bool,
        error: Option<&str>,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut map = self.metrics.write();
        let entry = map.entry(provider_id.to_string()).or_default();
        entry.latency_ms = Some(latency_ms);
        entry.last_request_at = Some(now);
        entry.total_requests = entry.total_requests.saturating_add(1);
        if success {
            entry.last_error = None;
        } else {
            entry.failed_requests = entry.failed_requests.saturating_add(1);
            entry.last_error = error.map(|s| s.to_string());
        }
    }

    /// 更新指定 provider 的断路器状态字符串。
    /// 由 `LlmGateway` 在断路器状态变化时调用。
    pub fn update_circuit_breaker_status(&self, provider_id: &str, status: &str) {
        let mut map = self.metrics.write();
        let entry = map.entry(provider_id.to_string()).or_default();
        entry.circuit_breaker_status = status.to_string();
    }

    /// 读取指定 provider 的指标快照(不存在时返回 None)。
    pub fn get_metrics(&self, provider_id: &str) -> Option<ProviderMetrics> {
        self.metrics.read().get(provider_id).cloned()
    }

    /// 读取所有 provider 的指标快照。
    pub fn get_all_metrics(&self) -> HashMap<String, ProviderMetrics> {
        self.metrics.read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_success_updates_metrics() {
        let tracker = ModelHealthTracker::new();
        tracker.record_request("deepseek", 150, true, None);
        let m = tracker
            .get_metrics("deepseek")
            .expect("metrics should exist");
        assert_eq!(m.latency_ms, Some(150));
        assert!(m.last_error.is_none());
        assert_eq!(m.total_requests, 1);
        assert_eq!(m.failed_requests, 0);
        assert!(m.last_request_at.is_some());
    }

    #[test]
    fn record_failure_updates_metrics() {
        let tracker = ModelHealthTracker::new();
        tracker.record_request("ollama", 2000, false, Some("connection refused"));
        let m = tracker.get_metrics("ollama").expect("metrics should exist");
        assert_eq!(m.latency_ms, Some(2000));
        assert_eq!(m.last_error.as_deref(), Some("connection refused"));
        assert_eq!(m.total_requests, 1);
        assert_eq!(m.failed_requests, 1);
    }

    #[test]
    fn multiple_requests_accumulate() {
        let tracker = ModelHealthTracker::new();
        tracker.record_request("p1", 100, true, None);
        tracker.record_request("p1", 200, false, Some("timeout"));
        tracker.record_request("p1", 150, true, None);
        let m = tracker.get_metrics("p1").expect("metrics should exist");
        assert_eq!(m.total_requests, 3);
        assert_eq!(m.failed_requests, 1);
        // 最近一次成功,last_error 应被清空
        assert!(m.last_error.is_none());
        assert_eq!(m.latency_ms, Some(150));
    }

    #[test]
    fn get_all_metrics_returns_clone() {
        let tracker = ModelHealthTracker::new();
        tracker.record_request("a", 50, true, None);
        tracker.record_request("b", 100, false, Some("err"));
        let all = tracker.get_all_metrics();
        assert_eq!(all.len(), 2);
        assert!(all.contains_key("a"));
        assert!(all.contains_key("b"));
    }

    #[test]
    fn update_circuit_breaker_status() {
        let tracker = ModelHealthTracker::new();
        tracker.update_circuit_breaker_status("deepseek", "Open");
        let m = tracker
            .get_metrics("deepseek")
            .expect("metrics should exist");
        assert_eq!(m.circuit_breaker_status, "Open");
    }

    #[test]
    fn get_metrics_nonexistent_returns_none() {
        let tracker = ModelHealthTracker::new();
        assert!(tracker.get_metrics("nonexistent").is_none());
    }

    #[test]
    fn default_is_empty() {
        let tracker = ModelHealthTracker::default();
        assert!(tracker.get_all_metrics().is_empty());
    }
}
