//! `nebula::grpc` — gRPC server (v2.1: real tonic wire layer).
//!
//! The server binds to `127.0.0.1:50051` by default (override with
//! `NEBULA_GRPC_ADDR`) and exposes the 22 RPCs from the design
//! document §13. It is opt-out: setting `NEBULA_GRPC=0` keeps the
//! server disabled.
//!
//! ## Architecture (v2.1 T-S2-B-01)
//!
//! * **Default**: Uses `tonic::transport::Server` with prost-generated
//!   types from `proto/nebula.v1.rs`. This is the real gRPC wire
//!   layer with HTTP/2 + protobuf framing.
//! * **Fallback** (`json-framing` feature): Uses the hand-rolled hyper
//!   HTTP/2 shim with JSON-over-gRPC framing. Kept for environments
//!   where prost codegen is unavailable.
//! * The `proto` module contains hand-rolled types for the JSON fallback.
//! * `tonic_server` module implements the 5 tonic-generated server traits
//!   on `TonicServiceImpl`, delegating to `AppState`.
//!
//! ## Feature flags
//!
//! * `grpc` — enables the gRPC server (default: off; add with `--features grpc`)
//! * `json-framing` — uses the hand-rolled JSON shim instead of tonic
//!   (default: off; only effective when `grpc` is also enabled)

pub mod proto;
#[cfg(feature = "grpc")]
pub mod server;

#[cfg(feature = "grpc")]
pub mod tonic_server;

// T-S2-B-01: 默认使用 tonic wire layer; json-framing feature 启用手写 JSON shim。
#[cfg(all(feature = "grpc", not(feature = "json-framing")))]
pub use tonic_server::{start_tonic_server as start_server, TonicGrpcHandle as GrpcHandle};

#[cfg(all(feature = "grpc", feature = "json-framing"))]
pub use server::{start_server, GrpcHandle};
