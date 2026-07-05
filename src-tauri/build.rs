// Build script for `nebula`.
// Two responsibilities:
//   1. Tauri build glue (icon, capabilities, etc.)
//   2. v0.3: compile the gRPC protobuf services into a Rust module
//      that the `nebula_lib::grpc` module re-exports. Skipped
//      when the `grpc` feature is off (minimal build).

fn main() {
    tauri_build::build();

    // v0.3: generate Rust types + tonic service traits from the
    // single source of truth in `proto/nebula.proto`. Gated by
    // the `grpc` feature so the minimal build doesn't pull in
    // tonic-build and doesn't try to compile the .proto file.
    #[cfg(feature = "grpc")]
    {
        let proto_path = "proto/nebula.proto";
        println!("cargo:rerun-if-changed={proto_path}");

        let mut config = tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .out_dir("src/grpc/proto");

        // A few message types intentionally have their own Rust
        // counterparts (Memory, Reflection, Skill, etc.) and we keep
        // tonic from clobbering the public API.
        config = config
            .server_attribute("grpc.module", env!("CARGO_PKG_NAME"))
            .client_attribute("grpc.module", env!("CARGO_PKG_NAME"));

        config
            .compile_protos(&[proto_path], &["proto"])
            .expect("failed to compile nebula.proto");
    }
}
