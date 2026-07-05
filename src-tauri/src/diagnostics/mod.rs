//! T-E-S-27: Trusted Diagnostics Channels.
//!
//! 诊断信息走独立可信通道(OpenClaw),与普通日志/事件流分离,
//! 确保诊断信息不被污染。
//!
//! ## 架构
//!
//! * [`events::DiagnosticEvent`] — 结构化诊断事件枚举,序列化为
//!   JSON 后通过 `tauri::ipc::Channel` 推送给前端。
//! * [`bus::DiagnosticsBus`] — 独立的 `broadcast::Sender` 通道
//!   (容量 512),用 `OnceLock` 全局单例。这样 `init_tracing` 中
//!   的 [`layer::DiagnosticsLayer`] 与 `AppState::bootstrap` 都能
//!   拿到同一个实例,避免循环依赖。
//! * [`layer::DiagnosticsLayer`] — `tracing_subscriber::Layer`
//!   实现,过滤 `target = "nebula.diagnostic"` 的事件并转发
//!   到 `DiagnosticsBus`。
//!
//! ## 命名约束
//!
//! 严禁使用 `channel` 命名(与 v1.2 multi-channel 冲突),统一用
//! `diagnostics`。

pub mod bus;
pub mod doctor;
pub mod events;
pub mod layer;

pub use bus::{global, DiagnosticsBus};
pub use doctor::{run_doctor, CheckStatus, DoctorCheck, DoctorReport};
pub use events::{DiagnosticEvent, DiagnosticOrigin, TrustLevel};
pub use layer::DiagnosticsLayer;
