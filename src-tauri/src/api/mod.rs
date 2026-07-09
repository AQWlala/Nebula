//! `nebula::api` — internal Rust-side service surface.
//!
//! In v0.1 the only "API" exposed to the outside world is the Tauri
//! command layer in [`crate::commands`]. This module defines a
//! trait-based abstraction so future HTTP / MCP / gRPC transports can
//! re-use the same business logic.

pub mod server;

// P1-D: System daemon registration (systemd / launchd / Windows Service).
// Always compiled — no feature gate needed; the platform-specific code
// is selected via #[cfg(target_os = ...)] inside the module.
pub mod daemon;

// P1-D: Web frontend static file server for headless mode.
// Feature-gated by `rest-api` because it uses hyper::Response types
// that are only available when the hyper dependency is enabled.
#[cfg(feature = "rest-api")]
pub mod static_server;

// T-S2-B-03a: REST API 与 gRPC 解耦，使用独立 rest-api feature。
#[cfg(feature = "rest-api")]
pub mod auth;

#[cfg(feature = "rest-api")]
pub mod rest;

// T-E-S-60: Gateway 守护进程 — HTTP 代理网关 + 路由转发 + 认证 + 限流。
pub mod gateway_daemon;

pub use server::NebulaService;
