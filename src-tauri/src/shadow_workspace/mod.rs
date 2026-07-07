//! T-E-C-08 / T-E-C-09: Shadow Workspace — Agent 隔离执行环境 + 任务录屏回放。
//!
//! Agent 任务在独立 git worktree + 临时分支中执行,不影响用户当前工作区。
//! 完成后提供 diff 供用户审查,可合并或丢弃。借鉴 Cursor Cloud Agent,
//! 但本地化(无云端依赖)。
//!
//! - `engine`: worktree 生命周期管理(T-E-C-08)
//! - `recording`: 操作录屏日志,记录 Agent 每步操作供回放(T-E-C-09)
//! - `commands`: Tauri 命令层
//!
//! 详见 `engine::ShadowWorkspaceEngine`。

pub mod commands;
pub mod engine;
pub mod recording;

pub use engine::{ShadowConfig, ShadowStatus, ShadowWorkspace, ShadowWorkspaceEngine};
pub use recording::{OperationKind, OperationRecord, RecordingLog};
