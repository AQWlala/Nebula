//! `nine_snake::grpc` — v0.3 in-process gRPC server.
//!
//! The server binds to `127.0.0.1:50051` by default (override with
//! `NINE_SNAKE_GRPC_ADDR`) and exposes the 22 RPCs from the design
//! document §13. It is opt-out: setting `NINE_SNAKE_GRPC=0` keeps the
//! server disabled.
//!
//! ## Architecture
//!
//! * The protobuf types are generated from `proto/nine_snake.proto` at
//!   build time via the `tonic-build` build-script in `build.rs`.
//! * The `proto` module below is `pub` so the integration tests can
//!   dial the in-process server and assert on the wire shape.
//! * Each RPC delegates to the corresponding Tauri-command handler
//!   (see `crate::commands`). The gRPC server *does not* duplicate
//!   business logic — it's a thin wire layer.
//!
//! ## Feature flag
//!
//! v0.3 ships gRPC enabled by default. To disable it at compile time,
//! build with `--no-default-features --features grpc` (the
//! `grpc` feature is implicit in the default feature set).

pub mod proto;
#[cfg(feature = "grpc")]
pub mod server;

#[cfg(feature = "grpc")]
pub use server::{start_server, GrpcHandle};
