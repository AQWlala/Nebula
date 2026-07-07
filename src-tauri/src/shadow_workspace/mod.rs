//! T-E-C-08: Shadow Workspace — Agent 隔离执行环境。
//!
//! Agent 任务在独立 git worktree + 临时分支中执行,不影响用户当前工作区。
//! 完成后提供 diff 供用户审查,可合并或丢弃。借鉴 Cursor Cloud Agent,
//! 但本地化(无云端依赖)。
//!
//! 详见 `engine::ShadowWorkspaceEngine`。

pub mod engine;
pub mod commands;

pub use engine::{ShadowConfig, ShadowStatus, ShadowWorkspace, ShadowWorkspaceEngine};
