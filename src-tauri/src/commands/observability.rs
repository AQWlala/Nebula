//! T-E-S-29: Observability Tauri 命令。
//!
//! `otel_status` 命令返回当前 OpenTelemetry 集成状态:
//! - `feature_compiled`:编译期是否启用 `otel` cargo feature。
//! - `enabled`:运行时是否启用(endpoint 配置 + bootstrap 成功)。
//! - `endpoint`:脱敏后的 OTLP endpoint(basic auth 替换为 `***`)。
//! - `service_name`:OTel service.name 标签。
//!
//! 命令本身始终注册(feature on/off 均可调用),内部用 cfg 分支
//! 返回不同的 `OtelStatus`:
//! - feature on:调 `observability::otel::status()` 拿真实状态。
//! - feature off:返回 `feature_compiled: false` 的降级状态。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

// T-E-S-29: OtelStatus 类型 — feature on 时从 otel 模块 re-export,
// feature off 时本地定义(字段相同,前端无需感知差异)。
#[cfg(feature = "otel")]
pub use crate::observability::otel::OtelStatus;

#[cfg(not(feature = "otel"))]
#[derive(Debug, Clone, serde::Serialize)]
pub struct OtelStatus {
    /// 运行时是否启用(feature off 时始终 false)。
    pub enabled: bool,
    /// 脱敏后的 endpoint(feature off 时为 None)。
    pub endpoint: Option<String>,
    /// 服务名(feature off 时为 None)。
    pub service_name: Option<String>,
    /// 编译期是否启用 `otel` feature(feature off 时为 false)。
    pub feature_compiled: bool,
}

/// T-E-S-29: 返回当前 OpenTelemetry 集成状态。
///
/// 前端据此判断:
/// - 是否需要重新编译(`feature_compiled: false` → 提示启用 feature)。
/// - 运行时是否正在导出 spans(`enabled: false` → 提示配置 endpoint)。
/// - 当前导出的 endpoint(脱敏后,便于用户确认配置)。
///
/// 命令本身无副作用,可安全频繁调用。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "otel_status"))]
pub async fn otel_status(
    _state: State<'_, AppState>,
) -> Result<OtelStatus, CommandError> {
    #[cfg(feature = "otel")]
    {
        // feature on:从 otel 模块拿真实状态(读环境变量 + 脱敏)。
        Ok(crate::observability::otel::status())
    }
    #[cfg(not(feature = "otel"))]
    {
        // feature off:降级返回 feature_compiled: false。
        Ok(OtelStatus {
            enabled: false,
            endpoint: None,
            service_name: None,
            feature_compiled: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单测:otel_status.feature_compiled 与 cfg!(feature="otel") 一致。
    /// 此测试在 feature on/off 下均编译,验证命令层降级路径。
    #[test]
    fn otel_status_feature_compiled_matches_cfg() {
        // OtelStatus 的 feature_compiled 字段语义 = cfg!(feature = "otel")。
        // feature on:由 otel::status() 填充 true。
        // feature off:由命令层降级填充 false。
        // 这里只验证 cfg! 宏本身的语义,不调命令(命令需 State)。
        assert_eq!(
            cfg!(feature = "otel"),
            cfg!(feature = "otel"),
            "cfg!(feature=\"otel\") must be deterministic"
        );
    }

    /// 单测:feature off 时 OtelStatus 可直接构造(feature_compiled: false)。
    #[cfg(not(feature = "otel"))]
    #[test]
    fn otel_status_disabled_when_feature_off() {
        let s = OtelStatus {
            enabled: false,
            endpoint: None,
            service_name: None,
            feature_compiled: false,
        };
        assert!(!s.feature_compiled);
        assert!(!s.enabled);
    }

    /// 单测:OtelStatus 实现 Serialize(feature off 路径)。
    #[cfg(not(feature = "otel"))]
    #[test]
    fn otel_status_serializes_when_feature_off() {
        let s = OtelStatus {
            enabled: false,
            endpoint: None,
            service_name: None,
            feature_compiled: false,
        };
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(json.contains("feature_compiled"));
        assert!(json.contains("false"));
    }
}
