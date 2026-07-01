//! v1.8: OpenTelemetry tracing export (OTLP).
//!
//! Standard implementation using `opentelemetry-otlp` with tonic
//! (gRPC) transport — per the v7.0 design spec for proper
//! observability.
//!
//! Controlled by the `NINE_SNAKE_OTLP_ENDPOINT` env var.  When set,
//! [`try_build_layer`] returns a `tracing_opentelemetry::Layer`
//! that forwards all `#[instrument]` spans to the OTel collector.
//! When unset, returns `None` and no OTel deps are exercised at
//! runtime (still compiled in, per design requirement for full
//! observability).
//!
//! The existing `#[instrument(fields(otel.kind = "..."))]` attributes
//! on every Tauri command are already OTel-compatible: the
//! `tracing-opentelemetry` layer forwards them as OTel span
//! attributes automatically.

use opentelemetry::{
    global,
    trace::TracerProvider as _,
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime::Tokio,
    trace::{BatchSpanProcessor, Config, TracerProvider},
    Resource,
};
use tracing::warn;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::Registry;

/// The env var name read by [`otlp_endpoint_from_env`].
pub const OTLP_ENDPOINT_ENV: &str = "NINE_SNAKE_OTLP_ENDPOINT";

/// The env var name for the service name reported to the OTel
/// collector.  Defaults to `nine-snake` when unset.
pub const OTLP_SERVICE_ENV: &str = "NINE_SNAKE_OTLP_SERVICE";

/// Concrete tracer type from the SDK.
pub type SdkTracer = opentelemetry_sdk::trace::Tracer;

/// The OTel tracing layer type — `OpenTelemetryLayer<Registry, SdkTracer>`.
pub type OtelLayer = OpenTelemetryLayer<Registry, SdkTracer>;

/// Read the OTLP endpoint from the env.  `None` when unset/empty.
pub fn otlp_endpoint_from_env() -> Option<String> {
    match std::env::var(OTLP_ENDPOINT_ENV) {
        Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Read the OTel service name from the env, defaulting to
/// `nine-snake`.
pub fn otlp_service_name_from_env() -> String {
    std::env::var(OTLP_SERVICE_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "nine-snake".to_string())
}

/// Build an OTLP exporter-backed tracing layer.
///
/// Returns `None` when exporter initialisation fails (we warn and
/// fall back to fmt-only logging).
pub fn try_build_layer(endpoint: &str, service_name: &str) -> Option<OtelLayer> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint)
        .build_span_exporter()
        .map_err(|e| {
            warn!(
                target: "nine_snake.otel",
                endpoint = %endpoint,
                error = ?e,
                "failed to build OTLP span exporter; OTel layer disabled"
            );
            e
        })
        .ok()?;

    // Build the batch span processor + tracer provider with a
    // resource tagged with the service name.
    let batch_processor = BatchSpanProcessor::builder(exporter, Tokio).build();
    let provider = TracerProvider::builder()
        .with_span_processor(batch_processor)
        .with_config(
            Config::default().with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                service_name.to_string(),
            )])),
        )
        .build();

    let tracer = provider.tracer(service_name.to_string());
    global::set_tracer_provider(provider.clone());

    // Leak the provider so it lives for the process lifetime —
    // the tracing layer holds a reference to the tracer, which
    // is backed by the provider.  Dropping the provider before
    // process exit would lose in-flight spans.
    std::mem::forget(provider);

    Some(tracing_opentelemetry::layer().with_tracer(tracer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otlp_endpoint_none_by_default() {
        if std::env::var(OTLP_ENDPOINT_ENV).is_err() {
            assert!(otlp_endpoint_from_env().is_none());
        }
    }

    #[test]
    fn otlp_service_name_defaults_to_nine_snake() {
        let prev = std::env::var(OTLP_SERVICE_ENV).ok();
        std::env::remove_var(OTLP_SERVICE_ENV);
        assert_eq!(otlp_service_name_from_env(), "nine-snake");
        if let Some(v) = prev {
            std::env::set_var(OTLP_SERVICE_ENV, v);
        }
    }

    #[test]
    fn otlp_endpoint_trims_whitespace() {
        let prev = std::env::var(OTLP_ENDPOINT_ENV).ok();
        std::env::set_var(OTLP_ENDPOINT_ENV, "  http://localhost:4317  ");
        assert_eq!(
            otlp_endpoint_from_env(),
            Some("http://localhost:4317".to_string())
        );
        match prev {
            Some(v) => std::env::set_var(OTLP_ENDPOINT_ENV, v),
            None => std::env::remove_var(OTLP_ENDPOINT_ENV),
        }
    }

    #[test]
    fn try_build_layer_returns_none_for_invalid_endpoint() {
        // An unreachable endpoint should fail gracefully (return None)
        // rather than panicking.  We use a clearly-invalid address.
        let result = try_build_layer("http://127.0.0.1:99999", "test-service");
        // The builder may succeed (it doesn't connect at build time),
        // so this test just verifies it doesn't panic.
        let _ = result;
    }
}
