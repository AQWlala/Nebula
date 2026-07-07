//! T-E-S-27: DiagnosticsLayer — 把 `nebula.diagnostic` target
//! 的 tracing 事件转发到 [`crate::diagnostics::bus::DiagnosticsBus`]。
//!
//! 设计参考 `observability/otel.rs::try_build_layer`:实现
//! `tracing_subscriber::Layer<Registry>`,可被 `.with(...)` 链式
//! 加入 subscriber。
//!
//! ## 过滤规则
//!
//! 只转发 `target = "nebula.diagnostic"`(或以其为前缀)且
//! level >= WARN 的事件。其他事件原样放过(由 fmt/otel 层处理)。
//!
//! ## 循环依赖避免
//!
//! `init_tracing` 在 `AppState::bootstrap` 之前调用,因此 layer
//! 通过 [`crate::diagnostics::bus::global`] 拿到全局单例 bus,
//! 而非通过 `AppState` 字段访问。

use tracing::{field::Visit, Event, Level, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

use crate::diagnostics::bus::{self, DiagnosticsBus};
use crate::diagnostics::events::DiagnosticEvent;

/// 诊断 target 前缀。所有以 `tracing::warn!(target = "nebula.diagnostic", ...)`
/// 等形式发出的事件都会被本层转发到 [`DiagnosticsBus`]。
pub const DIAGNOSTIC_TARGET_PREFIX: &str = "nebula.diagnostic";

/// T-E-S-27: 转发 `nebula.diagnostic` target 事件到
/// [`DiagnosticsBus`] 的 tracing layer。
///
/// 使用全局单例 bus(参考 `metrics::global()`),避免 `init_tracing`
/// 与 `AppState::bootstrap` 之间的循环依赖。
pub struct DiagnosticsLayer {
    bus: &'static DiagnosticsBus,
}

impl DiagnosticsLayer {
    /// 构造一个使用全局 [`DiagnosticsBus`] 单例的 layer。
    pub fn new() -> Self {
        // global() 返回 &'static Arc<DiagnosticsBus>,双重解引用拿到
        // &'static DiagnosticsBus。
        Self {
            bus: &**bus::global(),
        }
    }

    /// 构造一个使用指定 bus 引用的 layer。主要用于单元测试,
    /// 避免污染全局 bus。生产代码用 [`Self::new`]。
    pub fn with_bus(bus: &'static DiagnosticsBus) -> Self {
        Self { bus }
    }
}

impl Default for DiagnosticsLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for DiagnosticsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();
        if !target.starts_with(DIAGNOSTIC_TARGET_PREFIX) {
            return;
        }
        // 只转发 WARN/ERROR 级别的事件(INFO/DEBUG 留给 fmt 层)。
        let level = *metadata.level();
        if level != Level::WARN && level != Level::ERROR {
            return;
        }
        // 提取 message 字段(若存在)。
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();
        let diag_event = DiagnosticEvent::TracingWarn {
            target: target.to_string(),
            message,
            seq: 0,
        };
        self.bus.emit(diag_event);
    }
}

/// 从 tracing event 的字段中提取 `message` 字段值。
#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::Registry;

    /// 测试辅助:构造一个独立的 'static DiagnosticsBus(测试用,
    /// 避免污染全局 bus)。返回的引用在进程生命周期内有效。
    fn fresh_bus() -> &'static DiagnosticsBus {
        let bus = DiagnosticsBus::with_capacity(32);
        Box::leak(Box::new(bus))
    }

    #[test]
    fn layer_default_uses_global_bus() {
        let _layer = DiagnosticsLayer::default();
        // 全局 bus 已被初始化即可。
        assert!(bus::global().capacity() >= 1);
    }

    #[test]
    fn layer_implements_layer_for_registry() {
        // 编译期断言:DiagnosticsLayer: Layer<Registry>。
        fn assert_layer<L: Layer<Registry>>(_: &L) {}
        let layer = DiagnosticsLayer::new();
        assert_layer(&layer);
    }

    #[tokio::test]
    async fn warn_event_with_diagnostic_target_is_forwarded() {
        use tracing_subscriber::util::SubscriberInitExt as _;
        let bus = fresh_bus();
        let mut rx = bus.subscribe();
        let layer = DiagnosticsLayer::with_bus(bus);
        let _guard = tracing_subscriber::registry().with(layer).set_default();
        tracing::warn!(target: "nebula.diagnostic", "test warn message");
        let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("recv timed out")
            .expect("channel closed");
        match evt {
            DiagnosticEvent::TracingWarn {
                target, message, ..
            } => {
                assert_eq!(target, "nebula.diagnostic");
                assert!(message.contains("test warn message"));
            }
            _ => panic!("wrong variant: {:?}", evt),
        }
    }

    #[tokio::test]
    async fn info_event_with_diagnostic_target_is_not_forwarded() {
        use tracing_subscriber::util::SubscriberInitExt as _;
        let bus = fresh_bus();
        let mut rx = bus.subscribe();
        let layer = DiagnosticsLayer::with_bus(bus);
        let _guard = tracing_subscriber::registry().with(layer).set_default();
        tracing::info!(target: "nebula.diagnostic", "should be skipped");
        // 没有事件到达,recv 应该 pending → timeout。
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
        assert!(result.is_err(), "info event should not be forwarded");
    }

    #[tokio::test]
    async fn warn_event_with_other_target_is_not_forwarded() {
        use tracing_subscriber::util::SubscriberInitExt as _;
        let bus = fresh_bus();
        let mut rx = bus.subscribe();
        let layer = DiagnosticsLayer::with_bus(bus);
        let _guard = tracing_subscriber::registry().with(layer).set_default();
        tracing::warn!(target: "nebula.other", "should be skipped");
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
        assert!(
            result.is_err(),
            "other-target event should not be forwarded"
        );
    }
}
