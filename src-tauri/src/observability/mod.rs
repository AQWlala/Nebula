//! v1.8: observability layer.
//!
//! Currently houses the OpenTelemetry tracing bridge (`otel`).
//! When the `otel` cargo feature is enabled and the
//! `NEBULA_OTLP_ENDPOINT` env var is set, [`setup_otel_layer`]
//! returns a `tracing_opentelemetry::layer` that the caller adds to
//! the `tracing_subscriber` registry.  When either is absent, the
//! function returns `None` and the build stays dependency-free.

// T-E-S-29: OTel 模块在 `otel` feature 关闭时不编译(4 个 OTel 依赖改 optional)。
// Cargo.toml 已将 opentelemetry / opentelemetry_sdk / opentelemetry-otlp /
// tracing-opentelemetry 标记为 optional,由 `otel` feature 启用。
// lib.rs init_tracing 中的 otel 调用也由 `#[cfg(feature = "otel")]` 守卫。
#[cfg(feature = "otel")]
pub mod otel;
// T-E-S-25: 12 trace span types — 统一 otel.kind 标注枚举(始终编译,无外部依赖)。
pub mod span_type;
