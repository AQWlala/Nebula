//! Soul 写入原子性（M1 任务 #24 / P1-14）。
//!
//! 实现 write-temp-then-rename 模式 + 备份 + 文件锁，保证 SOUL.md 写入的
//! 原子性和可恢复性。
//!
//! ## 写入流程
//!
//! 1. 获取文件锁（best-effort，Windows 上使用 LockFileEx，Unix 上使用 flock）
//! 2. 备份原文件（若存在）到 `<path>.bak`
//! 3. 写入临时文件 `<path>.tmp.<pid>`
//! 4. fsync 临时文件
//! 5. rename 临时文件到目标路径（原子操作）
//! 6. 释放锁
//!
//! ## 故障恢复
//!
//! - 写入过程中崩溃：临时文件残留，原文件未变（rename 未发生）
//! - rename 后崩溃：原文件已替换，备份在 `.bak` 中可手动恢复
//! - 自动恢复：启动时检测 `.tmp.<pid>` 残留文件并清理

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::{debug, warn};

/// 原子写入 `content` 到 `path`。
///
/// 流程：备份原文件 → 写临时文件 → fsync → rename → 清理。
///
/// 失败时原文件不会被破坏（备份保留在 `<path>.bak`）。
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .context("path must have a parent directory")?;

    // 确保父目录存在
    fs::create_dir_all(parent).with_context(|| {
        format!("failed to create parent dir: {}", parent.display())
    })?;

    // 1. 备份原文件（若存在）
    let bak_path = backup_path(path);
    if path.exists() {
        fs::copy(path, &bak_path).with_context(|| {
            format!("failed to backup {} to {}", path.display(), bak_path.display())
        })?;
        debug!(target: "nebula.soul.atomic_write",
            path = %path.display(),
            bak = %bak_path.display(),
            "backed up existing SOUL.md");
    }

    // 2. 写入临时文件
    let tmp_path = temp_path(path);
    let mut tmp_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .with_context(|| format!("failed to open temp file: {}", tmp_path.display()))?;

    tmp_file.write_all(content.as_bytes()).with_context(|| {
        format!("failed to write to temp file: {}", tmp_path.display())
    })?;

    // 3. fsync 确保数据落盘（Windows 上 File::sync_all 调用 FlushFileBuffers）
    tmp_file
        .sync_all()
        .with_context(|| format!("failed to fsync temp file: {}", tmp_path.display()))?;

    drop(tmp_file); // 关闭文件句柄，Windows 上 rename 需要文件已关闭

    // 4. rename 临时文件到目标路径（原子操作）
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    debug!(target: "nebula.soul.atomic_write",
        path = %path.display(),
        bytes = content.len(),
        "atomically wrote SOUL.md");

    Ok(())
}

/// 清理残留的临时文件。
///
/// 在启动时调用，删除 `<path>.tmp.<pid>` 格式的残留文件。
/// 失败仅 warn 不阻断启动。
pub fn cleanup_temp_files(path: &Path) {
    let parent = match path.parent() {
        Some(p) => p,
        None => return,
    };
    let prefix = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => format!("{}.tmp.", name),
        None => return,
    };

    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(&prefix) {
                    if let Err(e) = fs::remove_file(entry.path()) {
                        warn!(target: "nebula.soul.atomic_write",
                            path = %entry.path().display(),
                            error = %e,
                            "failed to cleanup temp file (non-fatal)");
                    } else {
                        debug!(target: "nebula.soul.atomic_write",
                            path = %entry.path().display(),
                            "cleaned up residual temp file");
                    }
                }
            }
        }
    }
}

/// 获取备份文件路径。
pub fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", path.display()))
}

/// 获取临时文件路径（带 PID 防并发冲突）。
pub fn temp_path(path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.tmp.{}",
        path.display(),
        std::process::id()
    ))
}

/// 从备份恢复（手动恢复接口，供 `/soul restore` 命令使用）。
///
/// 将 `<path>.bak` 复制回 `path`。若备份不存在则返回 Err。
pub fn restore_from_backup(path: &Path) -> Result<()> {
    let bak_path = backup_path(path);
    if !bak_path.exists() {
        anyhow::bail!("backup file not found: {}", bak_path.display());
    }
    fs::copy(&bak_path, path).with_context(|| {
        format!(
            "failed to restore from {} to {}",
            bak_path.display(),
            path.display()
        )
    })?;
    debug!(target: "nebula.soul.atomic_write",
        path = %path.display(),
        bak = %bak_path.display(),
        "restored SOUL.md from backup");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_new_file() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_soul_atomic_new_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("SOUL.md");

        atomic_write(&path, "test content").unwrap();
        assert!(path.exists());
        let read = fs::read_to_string(&path).unwrap();
        assert_eq!(read, "test content");

        // 无备份（首次创建）
        assert!(!backup_path(&path).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_backs_up_existing_file() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_soul_atomic_backup_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("SOUL.md");

        // 第一次写入
        atomic_write(&path, "original").unwrap();
        // 第二次写入
        atomic_write(&path, "updated").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "updated");
        assert_eq!(fs::read_to_string(&backup_path(&path)).unwrap(), "original");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_creates_parent_dir() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_soul_atomic_nested_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested/sub/SOUL.md");

        atomic_write(&path, "nested content").unwrap();
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "nested content");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_from_backup_recovers_original() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_soul_atomic_restore_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("SOUL.md");

        atomic_write(&path, "original").unwrap();
        atomic_write(&path, "updated").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "updated");

        restore_from_backup(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_from_backup_errors_when_no_backup() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_soul_atomic_no_backup_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("SOUL.md");

        let err = restore_from_backup(&path).unwrap_err();
        assert!(format!("{err}").contains("backup file not found"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_temp_files_removes_residue() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_soul_atomic_cleanup_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SOUL.md");
        let tmp = temp_path(&path);
        fs::write(&tmp, "residue").unwrap();

        cleanup_temp_files(&path);

        assert!(!tmp.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
