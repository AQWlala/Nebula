//! v1.8: OpenTelemetry tracing export (OTLP).
//!
//! Standard implementation using `opentelemetry-otlp` with tonic
//! (gRPC) transport — per the v7.0 design spec for proper
//! observability.
//!
//! Controlled by the `NEBULA_OTLP_ENDPOINT` env var.  When set,
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
//!
//! T-E-S-29: 整文件 `otel` feature 守卫 — 4 个 OTel 依赖改 optional 后,
//! feature off 时不编译,避免引入未使用依赖。

#![cfg(feature = "otel")]

use opentelemetry::{global, trace::TracerProvider as _, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime::Tokio,
    trace::{BatchSpanProcessor, Config, TracerProvider},
    Resource,
};
use serde::{Deserialize, Serialize};
use tracing::warn;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::Registry;

/// The env var name read by [`otlp_endpoint_from_env`] / [`OtelConfig::from_env`]。
pub const OTLP_ENDPOINT_ENV: &str = "NEBULA_OTLP_ENDPOINT";

/// The env var name for the service name reported to the OTel
/// collector.  Defaults to `nebula` when unset.
pub const OTLP_SERVICE_ENV: &str = "NEBULA_OTLP_SERVICE";

/// Concrete tracer type from the SDK.
pub type SdkTracer = opentelemetry_sdk::trace::Tracer;

/// The OTel tracing layer type — `OpenTelemetryLayer<Registry, SdkTracer>`.
pub type OtelLayer = OpenTelemetryLayer<Registry, SdkTracer>;

// ---------------------------------------------------------------------------
// T-E-S-29: OtelConfig — AppConfig 集成用配置 DTO
// ---------------------------------------------------------------------------

/// T-E-S-29: OTel 配置 DTO,序列化进 `AppConfig.otel` 字段。
///
/// 由 [`OtelConfig::from_env`] 从 `NEBULA_OTLP_ENDPOINT` /
/// `NEBULA_OTLP_SERVICE` 解析;主 agent 在 `AppConfig::from_env`
/// 中调用并填入 `AppConfig.otel: Option<OtelConfig>`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtelConfig {
    /// OTLP gRPC endpoint,如 `"http://localhost:4317"`。
    pub endpoint: String,
    /// 服务名,默认 `"nebula"`。
    pub service_name: String,
    /// 是否启用(由 endpoint 非空推导,或显式配置)。
    pub enabled: bool,
}

impl OtelConfig {
    /// 从环境变量解析配置。
    ///
    /// - `NEBULA_OTLP_ENDPOINT` 未设/空 → 返回 `None`(OTel 禁用)。
    /// - `NEBULA_OTLP_ENDPOINT` 设 → 返回 `Some`,endpoint 非空即 `enabled: true`。
    /// - `NEBULA_OTLP_SERVICE` 未设 → 默认 `"nebula"`。
    pub fn from_env() -> Option<Self> {
        let endpoint = otlp_endpoint_from_env()?;
        let service_name = otlp_service_name_from_env();
        Some(Self {
            enabled: true,
            endpoint,
            service_name,
        })
    }
}

// ---------------------------------------------------------------------------
// T-E-S-29: bootstrap_otel — 从配置构建 OtelLayer
// ---------------------------------------------------------------------------

/// T-E-S-29: 根据配置构建 OTel layer。
///
/// **不直接引用 `AppConfig`**(避免循环依赖),主 agent 在 `lib.rs`
/// 的 `init_tracing_with_config` 中传入 `config.otel` 的具体字段。
///
/// - `enabled = false` 或 `endpoint` 为空 → 返回 `None`(降级到 fmt-only)。
/// - 否则调 [`try_build_layer`] 构建 OTLP exporter + tracer provider。
pub fn bootstrap_otel(endpoint: &str, service_name: &str, enabled: bool) -> Option<OtelLayer> {
    if !enabled || endpoint.trim().is_empty() {
        return None;
    }
    try_build_layer(endpoint, service_name)
}

// ---------------------------------------------------------------------------
// T-E-S-29: OtelStatus + status() — otel_status 命令返回 DTO
// ---------------------------------------------------------------------------

/// T-E-S-29: OTel 当前状态快照,由 `otel_status` 命令返回前端。
///
/// `feature_compiled` 反映编译期 `cfg!(feature = "otel")`,前端据此
/// 提示用户是否需要重新编译以启用 OTel。
#[derive(Debug, Clone, Serialize)]
pub struct OtelStatus {
    /// 运行时是否启用(endpoint 配置 + bootstrap 成功)。
    pub enabled: bool,
    /// 脱敏后的 endpoint(basic auth 部分被替换为 `***`)。
    pub endpoint: Option<String>,
    /// 服务名(无敏感信息,原样返回)。
    pub service_name: Option<String>,
    /// 编译期是否启用 `otel` feature。
    pub feature_compiled: bool,
}

/// T-E-S-29: 返回当前 OTel 状态。
///
/// - `feature_compiled` = `cfg!(feature = "otel")`(本文件仅在 feature on 时编译,
///   故始终为 `true`;feature off 时由 `commands::observability` 提供降级分支)。
/// - `enabled` / `endpoint` / `service_name` 从环境变量推导。
/// - `endpoint` 脱敏 basic auth(见 [`redact_endpoint_basic_auth`])。
pub fn status() -> OtelStatus {
    let endpoint = otlp_endpoint_from_env();
    let service_name = otlp_service_name_from_env();
    let enabled = endpoint.is_some();
    OtelStatus {
        enabled,
        endpoint: endpoint.as_ref().map(|e| redact_endpoint_basic_auth(e)),
        service_name: Some(service_name),
        feature_compiled: true,
    }
}

/// 脱敏 endpoint 中的 basic auth 凭据。
///
/// `"http://user:pass@host:4317"` → `"http://***@host:4317"`
/// `"http://host:4317"` → `"http://host:4317"`(无变化)
///
/// 仅处理 `userinfo@` 部分,不解析 query/fragment。失败时原样返回
/// (脱敏是尽力而为,不应阻塞状态查询)。
pub fn redact_endpoint_basic_auth(endpoint: &str) -> String {
    // 简单解析:寻找 "://" 后到下一个 '@' 之间的 userinfo。
    // 不引入 url crate(避免新依赖),用字符串操作。
    let scheme_end = endpoint.find("://");
    let Some(scheme_end) = scheme_end else {
        return endpoint.to_string();
    };
    let after_scheme = &endpoint[scheme_end + 3..];
    // 在 scheme 之后到第一个 '/' 之间寻找 '@'(userinfo 只能在 authority 中)。
    let auth_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    let authority = &after_scheme[..auth_end];
    let Some(at_pos) = authority.find('@') else {
        return endpoint.to_string();
    };
    // 有 userinfo:替换为 "***"。
    let userinfo = &authority[..at_pos];
    let host = &authority[at_pos + 1..];
    let redacted = format!(
        "{}://***@{}{}",
        &endpoint[..scheme_end],
        host,
        &after_scheme[auth_end..]
    );
    debug_assert_eq!(redacted.len(), endpoint.len() - userinfo.len() + 3);
    redacted
}

// ---------------------------------------------------------------------------
// 原有:env 读取 + layer 构建(保持向后兼容,lib.rs 仍引用)
// ---------------------------------------------------------------------------

/// Read the OTLP endpoint from the env.  `None` when unset/empty.
pub fn otlp_endpoint_from_env() -> Option<String> {
    match std::env::var(OTLP_ENDPOINT_ENV) {
        Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

/// Read the OTel service name from the env, defaulting to
/// `nebula`.
pub fn otlp_service_name_from_env() -> String {
    std::env::var(OTLP_SERVICE_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "nebula".to_string())
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
                target: "nebula.otel",
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

    // T-D-B-03: 不再使用 std::mem::forget 泄露 TracerProvider。
    // 存入 static OnceLock,保持 provider 存活至进程退出,避免
    // Drop 触发 shutdown 丢失 in-flight spans。进程退出时 static
    // 自动 drop,与 forget 的效果一致(进程生命周期存活)但不泄露内存。
    static TRACER_PROVIDER: std::sync::OnceLock<TracerProvider> = std::sync::OnceLock::new();
    let _ = TRACER_PROVIDER.set(provider);

    Some(tracing_opentelemetry::layer().with_tracer(tracer))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单测:OtelConfig::from_env 无 env 返回 None。
    #[test]
    fn otel_config_from_env_none_when_unset() {
        let prev_ep = std::env::var(OTLP_ENDPOINT_ENV).ok();
        std::env::remove_var(OTLP_ENDPOINT_ENV);
        assert!(OtelConfig::from_env().is_none());
        if let Some(v) = prev_ep {
            std::env::set_var(OTLP_ENDPOINT_ENV, v);
        }
    }

    /// 单测:OtelConfig::from_env 有 env 解析正确。
    #[test]
    fn otel_config_from_env_parses_when_set() {
        let prev_ep = std::env::var(OTLP_ENDPOINT_ENV).ok();
        let prev_svc = std::env::var(OTLP_SERVICE_ENV).ok();
        std::env::set_var(OTLP_ENDPOINT_ENV, "http://localhost:4317");
        std::env::set_var(OTLP_SERVICE_ENV, "test-svc");
        let cfg = OtelConfig::from_env().expect("endpoint set → Some");
        assert_eq!(cfg.endpoint, "http://localhost:4317");
        assert_eq!(cfg.service_name, "test-svc");
        assert!(cfg.enabled);
        match prev_ep {
            Some(v) => std::env::set_var(OTLP_ENDPOINT_ENV, v),
            None => std::env::remove_var(OTLP_ENDPOINT_ENV),
        }
        match prev_svc {
            Some(v) => std::env::set_var(OTLP_SERVICE_ENV, v),
            None => std::env::remove_var(OTLP_SERVICE_ENV),
        }
    }

    /// 单测:bootstrap_otel 禁用时返回 None。
    #[test]
    fn bootstrap_otel_returns_none_when_disabled() {
        assert!(bootstrap_otel("http://localhost:4317", "svc", false).is_none());
    }

    /// 单测:bootstrap_otel endpoint 为空时返回 None。
    #[test]
    fn bootstrap_otel_returns_none_when_endpoint_empty() {
        assert!(bootstrap_otel("", "svc", true).is_none());
        assert!(bootstrap_otel("   ", "svc", true).is_none());
    }

    /// 单测(otel feature):bootstrap_otel 启用且 endpoint 有效时返回 Some。
    /// 注意:try_build_layer 不实际连接,仅构建 exporter,通常返回 Some。
    #[test]
    fn bootstrap_otel_returns_some_when_enabled() {
        let result = bootstrap_otel("http://127.0.0.1:4317", "test-service", true);
        assert!(
            result.is_some(),
            "enabled + valid endpoint should build layer"
        );
    }

    /// 单测:otel_status.feature_compiled 与 cfg!(feature="otel") 一致。
    #[test]
    fn status_feature_compiled_matches_cfg() {
        let s = status();
        assert_eq!(s.feature_compiled, cfg!(feature = "otel"));
        // 本文件仅在 otel feature on 时编译,故 feature_compiled 必为 true。
        assert!(s.feature_compiled);
    }

    /// 单测:status 脱敏 endpoint basic auth。
    #[test]
    fn status_redacts_endpoint_basic_auth() {
        let redacted = redact_endpoint_basic_auth("http://user:pass@host:4317");
        assert_eq!(redacted, "http://***@host:4317");
        assert!(!redacted.contains("user"));
        assert!(!redacted.contains("pass"));
    }

    /// 单测:无 basic auth 的 endpoint 不被改动。
    #[test]
    fn status_no_redact_when_no_userinfo() {
        let ep = "http://localhost:4317";
        assert_eq!(redact_endpoint_basic_auth(ep), ep);
        let ep2 = "https://collector.example.com:4318";
        assert_eq!(redact_endpoint_basic_auth(ep2), ep2);
    }

    /// 单测:status 无 endpoint 时 enabled=false,endpoint=None。
    #[test]
    fn status_disabled_when_no_endpoint() {
        let prev = std::env::var(OTLP_ENDPOINT_ENV).ok();
        std::env::remove_var(OTLP_ENDPOINT_ENV);
        let s = status();
        assert!(!s.enabled);
        assert!(s.endpoint.is_none());
        match prev {
            Some(v) => std::env::set_var(OTLP_ENDPOINT_ENV, v),
            None => std::env::remove_var(OTLP_ENDPOINT_ENV),
        }
    }

    // --- 原有测试(保留)---

    #[test]
    fn otlp_endpoint_none_by_default() {
        if std::env::var(OTLP_ENDPOINT_ENV).is_err() {
            assert!(otlp_endpoint_from_env().is_none());
        }
    }

    #[test]
    fn otlp_service_name_defaults_to_nebula() {
        let prev = std::env::var(OTLP_SERVICE_ENV).ok();
        std::env::remove_var(OTLP_SERVICE_ENV);
        assert_eq!(otlp_service_name_from_env(), "nebula");
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
