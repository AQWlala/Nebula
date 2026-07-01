//! v1.8: observability layer.
//!
//! Currently houses the OpenTelemetry tracing bridge (`otel`).
//! When the `otel` cargo feature is enabled and the
//! `NINE_SNAKE_OTLP_ENDPOINT` env var is set, [`setup_otel_layer`]
//! returns a `tracing_opentelemetry::layer` that the caller adds to
//! the `tracing_subscriber` registry.  When either is absent, the
//! function returns `None` and the build stays dependency-free.

pub mod otel;
