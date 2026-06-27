//! `nine_snake::api` — internal Rust-side service surface.
//!
//! In v0.1 the only "API" exposed to the outside world is the Tauri
//! command layer in [`crate::commands`]. This module defines a
//! trait-based abstraction so future HTTP / MCP / gRPC transports can
//! re-use the same business logic.

pub mod server;

#[cfg(feature = "grpc")]
pub mod rest;

pub use server::NineSnakeService;
