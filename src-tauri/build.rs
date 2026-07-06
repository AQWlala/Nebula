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

        // v2.0: 确保 out_dir 存在。该目录下的 *.v1.rs 被 .gitignore
        // 忽略（tonic-build 构建产物），git 不跟踪空目录，CI checkout
        // 后目录不存在会导致 compile_protos 失败 (Os code 3 NotFound)。
        let out_dir = "src/grpc/proto";
        std::fs::create_dir_all(out_dir).expect("failed to create grpc proto out_dir");

        let mut config = tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .out_dir(out_dir);

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
