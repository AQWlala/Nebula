//! Tracing subscriber setup.
//!
//! [`init_tracing`] installs the `tracing` subscriber with optional
//! JSON formatting, OpenTelemetry OTLP export, Trusted Diagnostics
//! layer, and daily-rotated log files.
//!
//! T-D-B-01: 8 路 match 组合爆炸重构为 builder pattern。
//! 原实现用 `match (otel_layer, nb_writer, use_json)` 产生 8 个分支，
//! 现在用 `Vec<Box<dyn Layer>>` 收集所有可选层，最后统一 with() 一次。

use std::path::PathBuf;

use tracing_subscriber::{
    fmt,
    layer::{Layer, SubscriberExt as _},
    registry,
    util::SubscriberInitExt as _,
    EnvFilter, Registry,
};

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// 构建可选的 fmt layer，统一处理 `use_json` 和 `nb_writer` 两个维度。
///
/// 返回 `BoxedLayer` 以便调用方统一 with()。
fn build_fmt_layer(
    use_json: bool,
    nb_writer: Option<tracing_appender::non_blocking::NonBlocking>,
) -> BoxedLayer {
    // 先构造基础 layer，再按需附加 writer 和 json 格式。
    // tracing_subscriber 的 fmt::Layer 支持 with_writer + json 链式调用，
    // 但返回类型不同（fmt::Layer vs fmt::Layer<JsonFields>），
    // 所以用 boxed trait object 统一类型。
    if use_json {
        match nb_writer {
            Some(nb) => Box::new(fmt::layer().with_writer(nb).json()),
            None => Box::new(fmt::layer().json()),
        }
    } else {
        match nb_writer {
            Some(nb) => Box::new(fmt::layer().with_writer(nb)),
            None => Box::new(fmt::layer()),
        }
    }
}

/// Installs the `tracing` subscriber. Safe to call multiple times.
///
/// v0.2: writes structured JSON to stdout when the
/// `NEBULA_LOG_FORMAT=json` environment variable is set; the
/// default remains human-readable pretty output.
///
/// v1.0: when `NEBULA_LOG_DIR` is set we also write a
/// daily-rotated JSON log file via `tracing_appender`.  This is
/// what the user-facing "Open logs folder" menu points at.
///
/// v1.1.9: 默认日志目录。即使未设置 `NEBULA_LOG_DIR`,也写入
/// 平台默认的 app data 目录,以便用户在遇到启动崩溃时能找到日志。
///
/// T-D-B-01: 8 路 match 组合爆炸重构为 builder pattern。
/// 原实现用 `match (otel_layer, nb_writer, use_json)` 产生 8 个分支，
/// 现在用 `Vec<BoxedLayer>` 收集所有可选层，最后统一 with() 一次。
pub fn init_tracing() {
    static INIT: once_cell::sync::OnceCell<()> = once_cell::sync::OnceCell::new();
    INIT.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,nebula=debug"));
        let use_json = std::env::var("NEBULA_LOG_FORMAT")
            .map(|v| v.eq_ignore_ascii_case("json"))
            .unwrap_or(false);

        // v1.8: 尝试构建 OpenTelemetry OTLP 层。
        // 由 NEBULA_OTLP_ENDPOINT 环境变量控制；未设置则返回 None。
        // T-E-S-29: 整个 OTel 路径门控 `otel` feature — feature off 时
        // 不编译 opentelemetry 依赖,otel_layer 始终为 None。
        #[cfg(feature = "otel")]
        let otel_layer: Option<BoxedLayer> = {
            let otel_endpoint = crate::observability::otel::otlp_endpoint_from_env();
            let otel_service = crate::observability::otel::otlp_service_name_from_env();
            otel_endpoint
                .as_ref()
                .and_then(|ep| crate::observability::otel::try_build_layer(ep, &otel_service))
                .map(|l| Box::new(l) as BoxedLayer)
        };
        #[cfg(not(feature = "otel"))]
        let otel_layer: Option<BoxedLayer> = None;

        // T-E-S-27: Trusted Diagnostics Layer。
        // 用全局单例 bus 避免与 AppState::bootstrap 的循环依赖:
        // init_tracing 在 bootstrap 之前调用,layer 通过
        // `diagnostics::bus::global()` 拿到 bus,bootstrap 中
        // `AppState.diagnostics = bus::global()` 拿到同一实例。
        // 当 NEBULA_DIAGNOSTICS=0 时不安装 layer(转发无效)。
        let diagnostics_enabled = std::env::var("NEBULA_DIAGNOSTICS")
            .ok()
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let diagnostics_layer: Option<crate::diagnostics::DiagnosticsLayer> = if diagnostics_enabled
        {
            Some(crate::diagnostics::DiagnosticsLayer::new())
        } else {
            None
        };

        // 日志目录:优先用 NEBULA_LOG_DIR,否则用平台默认目录。
        let log_dir = std::env::var("NEBULA_LOG_DIR").ok().map(PathBuf::from);
        let log_dir = log_dir.or_else(default_log_dir);

        let nb_writer: Option<tracing_appender::non_blocking::NonBlocking> =
            if let Some(dir) = &log_dir {
                let _ = std::fs::create_dir_all(dir);
                // 安装 panic hook:将 panic 信息写入日志文件,避免
                // `windows_subsystem = "windows"` 下 panic 被静默吞掉。
                let panic_dir = dir.clone();
                std::panic::set_hook(Box::new(move |info| {
                    let panic_file = panic_dir.join("nebula-panic.log");
                    let msg = format!(
                        "[{}] PANIC: {}\n",
                        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                        info
                    );
                    let _ = std::fs::OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&panic_file)
                        .and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
                    eprintln!("{msg}");
                }));
                let appender = tracing_appender::rolling::daily(dir, "nebula.log");
                let (nb, _guard) = tracing_appender::non_blocking(appender);
                let _ = Box::leak(Box::new(_guard));
                Some(nb)
            } else {
                None
            };

        // T-D-B-01: 统一构建 subscriber，不再 8 路 match。
        // 用 Vec<BoxedLayer> 收集所有可选层，Vec<L> 实现了 Layer<S>，
        // 这样避免了 Box<dyn Layer<Registry>> 不实现 Layer<Layered<...>> 的问题。
        // OTel 层放在 Vec 第一个位置（它实现 Layer<Registry> 而非 Layer<Layered<...>>），
        // 通过 Vec 整体加到 bare Registry 上等价于直接 with(otel)。
        let fmt_layer = build_fmt_layer(use_json, nb_writer);

        let mut layers: Vec<BoxedLayer> = Vec::new();
        if let Some(otel) = otel_layer {
            layers.push(otel);
        }
        if let Some(diag) = diagnostics_layer {
            layers.push(Box::new(diag));
        }
        layers.push(fmt_layer);

        // EnvFilter 不 box：它对所有 S: Subscriber 实现 Layer<S>，
        // 所以可以加到 Layered<Vec<BoxedLayer>, Registry> 上。
        let _ = registry().with(layers).with(filter).try_init();
    });
}

/// 返回平台默认的日志目录。
pub(crate) fn default_log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|d| PathBuf::from(d).join("nebula").join("logs"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| PathBuf::from(d).join("Library/Logs/nebula"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| PathBuf::from(d).join(".local/share/nebula/logs"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    None
}
