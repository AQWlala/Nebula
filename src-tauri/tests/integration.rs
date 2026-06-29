//! Integration test runner.
//!
//! Rust's standard `tests/` directory treats each top-level `.rs` file
//! as a separate test binary; subdirectories are *not* auto-discovered.
//! To keep the v0.3 spec's layout (one file per scenario under
//! `tests/integration/`), this top-level file re-includes them via
//! `#[path]` so `cargo test --test integration` runs the full suite.
//!
//! v0.3 layout: shared helpers live at `tests/integration/common.rs`
//! and are declared once here as `pub mod common;`. Each scenario file

//! accesses the helpers via `super::common`. This avoids the
//! "file-included-twice" error that would happen if every scenario
//! file declared its own `mod common;`.
//!
//! v1.0: e2e security tests live under `tests/e2e/` and are
//! included through a `#[path]` shim in this same file (Rust 2021
//! does not auto-pick up subdirectories).

#[path = "integration/common.rs"]
pub mod common;

#[path = "integration/bootstrap_test.rs"]
mod bootstrap_test;

#[path = "integration/memory_flow_test.rs"]
mod memory_flow_test;

#[path = "integration/documents_fk_test.rs"]
mod documents_fk_test;

#[path = "integration/icon_assets_test.rs"]
mod icon_assets_test;

#[path = "integration/updater_pubkey_test.rs"]
mod updater_pubkey_test;

// v1.0.1 P0#01: key rotation regression suite. Lives next to the
// existing `updater_pubkey_test` and is run by the same test binary.
#[path = "integration/key_rotation_test.rs"]
mod key_rotation_test;

#[cfg(feature = "grpc")]
#[path = "integration/grpc_wire_test.rs"]
mod grpc_wire_test;

#[path = "integration/swarm_test.rs"]
mod swarm_test;

#[path = "integration/swarm_e2e_test.rs"]
mod swarm_e2e_test;

#[path = "integration/reflect_test.rs"]
mod reflect_test;

#[path = "integration/skills_test.rs"]
mod skills_test;

#[path = "integration/llm_test.rs"]
mod llm_test;

// v0.5: editor / writing / work / OS / sync integration tests.
#[path = "integration/editor_test.rs"]
mod editor_test;

#[path = "integration/writing_test.rs"]
mod writing_test;

#[path = "integration/work_test.rs"]
mod work_test;

#[path = "integration/shell_test.rs"]
mod shell_test;

#[path = "integration/sync_test.rs"]
mod sync_test;

// v1.0.1 P0#10: blackhole + sponge concurrency regression.
#[path = "integration/compression_lock_test.rs"]
mod compression_lock_test;

// v1.0: end-to-end security audit suite.
#[path = "e2e/security.rs"]
mod security;

// v1.3: V2 integration tests.
#[path = "integration/v2_test.rs"]
mod v2_test;
