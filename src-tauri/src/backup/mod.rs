//! T-S6-A-03: 自动备份 — 每日 02:00 cron 任务备份 SQLite + LanceDB。
//!
//! 备份目标目录: `%LOCALAPPDATA%\nebula\backups\YYYYMMDD\`(Windows)。
//! 保留最近 7 份备份,超过的旧备份自动清理。
//!
//! 模块组成:
//! - [`scheduler`] — `BackupScheduler` 调度器,负责定时触发与备份执行
//! - [`commands`] — Tauri 命令(`backup_now` / `backup_list` / `backup_restore`)

pub mod commands;
pub mod scheduler;

pub use commands::{backup_list, backup_now, backup_restore};
pub use scheduler::{BackupInfo, BackupScheduler};
