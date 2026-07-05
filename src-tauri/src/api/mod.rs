//! `nebula::api` — internal Rust-side service surface.
//!
//! In v0.1 the only "API" exposed to the outside world is the Tauri
//! command layer in [`crate::commands`]. This module defines a
//! trait-based abstraction so future HTTP / MCP / gRPC transports can
//! re-use the same business logic.

pub mod server;

// T-S2-B-03a: REST API 与 gRPC 解耦，使用独立 rest-api feature。
#[cfg(feature = "rest-api")]
pub mod auth;

#[cfg(feature = "rest-api")]
pub mod rest;

pub use server::NebulaService;
