//! T-E-C-10: 异步长任务模式 — 后台分步执行跨小时/跨天的复杂任务。
//!
//! 与 PlanEngine 联动(可选,plan_id 软引用)并在 Shadow Workspace 中
//! 隔离执行(可选,workspace_id 软引用)。状态持久化到 SQLite,进程
//! 重启后可恢复(pending/running 自动重置为 paused 等待用户恢复)。
//!
//! - `engine`:  LongTaskEngine —— 任务/步骤 CRUD + 后台 runner
//! - `commands`: Tauri 命令层
//!
//! 详见 `engine::LongTaskEngine`。

pub mod engine;
pub mod commands;

pub use engine::{
    LongTask, LongTaskEngine, LongTaskStatus, LongTaskStep, StepInput, StepStatus,
};
