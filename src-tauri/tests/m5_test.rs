//! M5 独立测试二进制。
//!
//! 与 `tests/integration.rs` 分离，避免受其他集成测试模块的预存在编译错误影响。
//! 运行：`cargo test --test m5_test`

#[path = "integration/m5_approval_cost_test.rs"]
mod m5_approval_cost_test;
