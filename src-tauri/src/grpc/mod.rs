//! `nebula::grpc` — gRPC server (标准 tonic wire layer)。
//!
//! 服务默认绑定到 `127.0.0.1:50051`（可用 `NEBULA_GRPC_ADDR` 覆盖），
//! 暴露设计文档 §13 中的 22 个 RPC。它是可选关闭的：设置 `NEBULA_GRPC=0`
//! 可保持服务禁用。
//!
//! ## 架构 (T-D-B-08: 已移除手写 JSON framing shim)
//!
//! * **唯一 wire 实现**：`tonic::transport::Server` + prost 生成的类型
//!   （位于 `proto/nebula.v1.rs`）。使用标准 HTTP/2 + protobuf 二进制帧，
//!   可被 grpcurl 等标准工具直接调用。
//! * `tonic_server` 模块在 `TonicServiceImpl` 上实现 5 个 prost 生成的
//!   server trait，委托到 `AppState`。
//!
//! ## Feature flags
//!
//! * `grpc` — 启用 gRPC server（默认关闭；通过 `--features grpc` 开启）

#[cfg(feature = "grpc")]
pub mod tonic_server;

// T-D-B-08: tonic 是唯一的 gRPC wire 实现（移除了手写 JSON shim）。
#[cfg(feature = "grpc")]
pub use tonic_server::{start_tonic_server as start_server, TonicGrpcHandle as GrpcHandle};
