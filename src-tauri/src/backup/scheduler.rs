//! T-S6-A-03: 自动备份调度器 — 每日 02:00 备份 SQLite + LanceDB。
//!
//! 调度器在后台线程中运行,计算到下一个 02:00 (本地时间) 的秒数并 sleep,
//! 到点后执行一次完整备份,然后清理超过 7 份的旧备份。线程为守护线程,
//! 进程退出时自动终止。
//!
//! ## 备份目录结构
//!
//! ```text
//! %LOCALAPPDATA%\nebula\backups\
//! ├── 20260703\
//! │   ├── nebula.db          # SQLite 主库
//! │   ├── nebula.db-wal      # WAL sidecar (若存在)
//! │   ├── nebula.db-shm      # SHM sidecar (若存在)
//! │   ├── lance\                 # LanceDB 目录递归复制
//! │   └── meta.json              # 备份元信息
//! └── 20260704\
//!     └── ...
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tracing::{error, info, warn};
use walkdir::WalkDir;

/// 备份保留份数(超过此数量的旧备份会被清理)。
const MAX_BACKUPS: usize = 7;

/// 备份目录中的元信息文件名。
const META_FILENAME: &str = "meta.json";

/// 一天的秒数。
const SECS_PER_DAY: u64 = 86_400;

/// 02:00 (本地时间) 的秒数偏移(从午夜起)。
const BACKUP_HOUR_SECS: u64 = 2 * 3_600;

/// 单次备份的元信息,写入 `backups/YYYYMMDD/meta.json`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMeta {
    /// 备份日期(YYYYMMDD)。
    pub date: String,
    /// SQLite 数据库文件大小(字节,含 sidecar)。
    pub sqlite_bytes: u64,
    /// LanceDB 目录总大小(字节)。
    pub lance_bytes: u64,
    /// 备份创建时间(Unix 时间戳,秒)。
    pub created_at: i64,
    /// 备份创建时间(ISO 8601 字符串)。
    pub created_at_iso: String,
}

/// 列出备份时返回的摘要信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    /// 备份日期(YYYYMMDD)。
    pub date: String,
    /// 备份目录绝对路径。
    pub path: String,
    /// 备份总大小(字节)。
    pub size_bytes: u64,
    /// 备份创建时间(Unix 时间戳,秒)。
    pub created_at: i64,
}

/// 自动备份调度器。
///
/// 每日 02:00 触发一次完整备份:
/// - 复制 SQLite 数据库文件(含 `-wal` / `-shm` sidecar)
/// - 递归复制 LanceDB 目录
/// - 写入 `meta.json` 元信息
/// - 清理超过 [`MAX_BACKUPS`] 份的旧备份
pub struct BackupScheduler {
    /// Tauri 应用句柄(用于后台事件通知)。
    app: tauri::AppHandle,
    /// SQLite 数据库文件路径。
    db_path: PathBuf,
    /// LanceDB 数据目录。
    lance_dir: PathBuf,
    /// 备份根目录(`%LOCALAPPDATA%\nebula\backups\`)。
    backup_root: PathBuf,
}

impl BackupScheduler {
    /// 创建调度器。
    ///
    /// `backup_root` 自动解析为 `%LOCALAPPDATA%\nebula\backups\`(Windows),
    /// 与日志目录 `%LOCALAPPDATA%\nebula\logs\` 同级。
    pub fn new(app: tauri::AppHandle, db_path: PathBuf, lance_dir: PathBuf) -> Self {
        let backup_root = default_backup_root();
        Self {
            app,
            db_path,
            lance_dir,
            backup_root,
        }
    }

    /// 启动后台调度线程。
    ///
    /// 线程计算到下一个 02:00 (本地时间) 的秒数,sleep 后触发备份,然后循环。
    /// 备份完成或失败时通过 Tauri 事件 `backup-completed` / `backup-failed`
    /// 通知前端。线程为守护线程,进程退出时自动终止。
    pub fn start(self) -> Result<()> {
        let app = self.app;
        let db_path = self.db_path.clone();
        let lance_dir = self.lance_dir.clone();
        let backup_root = self.backup_root.clone();

        std::thread::Builder::new()
            .name("nebula-backup".to_string())
            .spawn(move || {
                info!(target: "nebula.backup", "scheduler thread started");
                loop {
                    let secs = seconds_until_next_02am();
                    info!(
                        target: "nebula.backup",
                        secs_to_next = secs,
                        "next backup scheduled"
                    );
                    std::thread::sleep(Duration::from_secs(secs));

                    let sched = BackupScheduler {
                        app: app.clone(),
                        db_path: db_path.clone(),
                        lance_dir: lance_dir.clone(),
                        backup_root: backup_root.clone(),
                    };
                    match sched.backup_once() {
                        Ok(path) => {
                            info!(
                                target: "nebula.backup",
                                path = %path.display(),
                                "scheduled backup completed"
                            );
                            let _ =
                                app.emit("backup-completed", path.to_string_lossy().to_string());
                        }
                        Err(e) => {
                            error!(
                                target: "nebula.backup",
                                error = ?e,
                                "scheduled backup failed"
                            );
                            let _ = app.emit("backup-failed", format!("{e:#}"));
                        }
                    }
                    if let Err(e) = sched.prune_old_backups() {
                        warn!(
                            target: "nebula.backup",
                            error = ?e,
                            "prune old backups failed"
                        );
                    }
                }
            })
            .context("spawning backup scheduler thread")?;

        Ok(())
    }

    /// 执行一次备份并返回备份目录路径。
    ///
    /// 步骤:
    /// 1. 创建 `backups\YYYYMMDD\` 目录(UTC 日期)
    /// 2. 复制 SQLite 文件(含 `-wal` / `-shm`)
    /// 3. 递归复制 LanceDB 目录到 `backups\YYYYMMDD\lance\`
    /// 4. 写入 `meta.json` 元信息
    pub fn backup_once(&self) -> Result<PathBuf> {
        let now = Utc::now();
        let date_str = now.format("%Y%m%d").to_string();
        let backup_dir = self.backup_root.join(&date_str);

        // 确保备份根目录存在
        fs::create_dir_all(&self.backup_root)
            .with_context(|| format!("creating backup root: {}", self.backup_root.display()))?;
        fs::create_dir_all(&backup_dir)
            .with_context(|| format!("creating backup dir: {}", backup_dir.display()))?;

        // 复制 SQLite 数据库文件(含 sidecar)
        let mut sqlite_bytes = 0u64;
        if self.db_path.exists() {
            sqlite_bytes = copy_file_with_sidecars(&self.db_path, &backup_dir)?;
        } else {
            warn!(
                target: "nebula.backup",
                path = %self.db_path.display(),
                "sqlite db not found, skipping"
            );
        }

        // 递归复制 LanceDB 目录
        let lance_bytes = if self.lance_dir.exists() {
            let lance_dest = backup_dir.join("lance");
            copy_dir_recursive(&self.lance_dir, &lance_dest)?
        } else {
            warn!(
                target: "nebula.backup",
                path = %self.lance_dir.display(),
                "lance dir not found, skipping"
            );
            0
        };

        // 写入 meta.json
        let meta = BackupMeta {
            date: date_str.clone(),
            sqlite_bytes,
            lance_bytes,
            created_at: now.timestamp(),
            created_at_iso: now.to_rfc3339(),
        };
        let meta_path = backup_dir.join(META_FILENAME);
        let meta_json = serde_json::to_string_pretty(&meta).context("serializing backup meta")?;
        fs::write(&meta_path, meta_json)
            .with_context(|| format!("writing meta.json: {}", meta_path.display()))?;

        info!(
            target: "nebula.backup",
            path = %backup_dir.display(),
            sqlite_bytes,
            lance_bytes,
            "backup completed"
        );

        Ok(backup_dir)
    }

    /// 列出所有备份(按日期降序)。
    pub fn list_backups(&self) -> Result<Vec<BackupInfo>> {
        let mut backups = Vec::new();

        if !self.backup_root.exists() {
            return Ok(backups);
        }

        for entry in fs::read_dir(&self.backup_root)
            .with_context(|| format!("reading backup root: {}", self.backup_root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // 只识别 YYYYMMDD 格式的目录(8 位纯数字)
            if !(name.len() == 8 && name.chars().all(|c| c.is_ascii_digit())) {
                continue;
            }

            let path = entry.path();
            let size_bytes = dir_size(&path);
            let created_at = read_backup_timestamp(&path).unwrap_or(0);

            backups.push(BackupInfo {
                date: name,
                path: path.to_string_lossy().to_string(),
                size_bytes,
                created_at,
            });
        }

        // 按日期降序排序(最新的在前)
        backups.sort_by(|a, b| b.date.cmp(&a.date));

        Ok(backups)
    }

    /// 清理超过 [`MAX_BACKUPS`] 份的旧备份。
    ///
    /// 读取 `backup_root` 下所有 `YYYYMMDD` 目录,按日期降序排序,
    /// 保留前 [`MAX_BACKUPS`] 个,删除其余。
    pub fn prune_old_backups(&self) -> Result<()> {
        if !self.backup_root.exists() {
            return Ok(());
        }

        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&self.backup_root)
            .with_context(|| format!("reading backup root: {}", self.backup_root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.len() == 8 && name.chars().all(|c| c.is_ascii_digit()) {
                dirs.push((name, entry.path()));
            }
        }

        // 按日期降序排序(最新的在前)
        dirs.sort_by(|a, b| b.0.cmp(&a.0));

        // 删除超出保留份数的旧备份
        for (_, path) in dirs.iter().skip(MAX_BACKUPS) {
            info!(
                target: "nebula.backup",
                path = %path.display(),
                "pruning old backup"
            );
            fs::remove_dir_all(path)
                .with_context(|| format!("removing old backup: {}", path.display()))?;
        }

        Ok(())
    }
}

/// 返回平台默认的备份根目录。
///
/// Windows: `%LOCALAPPDATA%\nebula\backups\`
/// macOS:   `~/Library/Application Support/nebula/backups/`
/// Linux:   `~/.local/share/nebula/backups/`
pub fn default_backup_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|d| PathBuf::from(d).join("nebula").join("backups"))
            .unwrap_or_else(|| PathBuf::from("nebula-backups"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| PathBuf::from(d).join("Library/Application Support/nebula/backups"))
            .unwrap_or_else(|| PathBuf::from("nebula-backups"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| PathBuf::from(d).join(".local/share/nebula/backups"))
            .unwrap_or_else(|| PathBuf::from("nebula-backups"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        PathBuf::from("nebula-backups")
    }
}

/// 计算从现在到下一个 02:00 (本地时间) 的秒数。
///
/// 若当前时间在 02:00 之前,返回到今天 02:00 的差值;
/// 否则返回到明天 02:00 的差值。最小返回 60 秒以避免边界 tight loop。
fn seconds_until_next_02am() -> u64 {
    let now = Local::now();
    let now_secs = now.num_seconds_from_midnight() as u64;

    let diff = if now_secs < BACKUP_HOUR_SECS {
        BACKUP_HOUR_SECS - now_secs
    } else {
        SECS_PER_DAY - now_secs + BACKUP_HOUR_SECS
    };

    diff.max(60)
}

/// 复制 SQLite 数据库文件及其 WAL sidecar(`-wal` / `-shm`)。
///
/// 主文件复制失败则返回错误;sidecar 文件复制失败仅 warn 不中断
/// (运行中的数据库可能随时创建/删除 WAL 文件)。
fn copy_file_with_sidecars(db_path: &Path, dest_dir: &Path) -> Result<u64> {
    let mut total = 0u64;

    let filename = db_path.file_name().context("db_path has no file_name")?;
    let dest = dest_dir.join(filename);
    total += fs::copy(db_path, &dest)
        .with_context(|| format!("copying sqlite db: {}", db_path.display()))?;

    // WAL sidecar: <dbname>-wal, <dbname>-shm
    for suffix in &["-wal", "-shm"] {
        let sidecar_name = format!("{}{}", filename.to_string_lossy(), suffix);
        let sidecar_src = db_path.with_file_name(&sidecar_name);
        if sidecar_src.exists() {
            let sidecar_dest = dest_dir.join(&sidecar_name);
            match fs::copy(&sidecar_src, &sidecar_dest) {
                Ok(n) => total += n,
                Err(e) => warn!(
                    target: "nebula.backup",
                    file = %sidecar_src.display(),
                    error = %e,
                    "failed to copy sqlite sidecar"
                ),
            }
        }
    }

    Ok(total)
}

/// 递归复制目录,返回所有文件总字节数。
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<u64> {
    let mut total = 0u64;
    fs::create_dir_all(dest).with_context(|| format!("creating dest dir: {}", dest.display()))?;

    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let entry_path = entry.path();
        let rel = entry_path.strip_prefix(src)?;
        let dest_path = dest.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest_path)
                .with_context(|| format!("creating dir: {}", dest_path.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            let n = fs::copy(entry_path, &dest_path)
                .with_context(|| format!("copying file: {}", entry_path.display()))?;
            total += n;
        }
    }

    Ok(total)
}

/// 计算目录下所有文件的总大小(字节)。
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

/// 从备份目录的 `meta.json` 读取创建时间戳。
fn read_backup_timestamp(backup_dir: &Path) -> Option<i64> {
    let meta_path = backup_dir.join(META_FILENAME);
    let content = fs::read_to_string(&meta_path).ok()?;
    let meta: BackupMeta = serde_json::from_str(&content).ok()?;
    Some(meta.created_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_until_next_02am_returns_at_least_60() {
        let secs = seconds_until_next_02am();
        assert!(secs >= 60);
        assert!(secs <= SECS_PER_DAY);
    }

    #[test]
    fn default_backup_root_ends_with_backups() {
        let root = default_backup_root();
        assert!(root.ends_with("backups"));
    }

    #[test]
    fn backup_info_serializes() {
        let info = BackupInfo {
            date: "20260703".to_string(),
            path: "/tmp/backup".to_string(),
            size_bytes: 1024,
            created_at: 1_700_000_000,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("20260703"));
    }
}
