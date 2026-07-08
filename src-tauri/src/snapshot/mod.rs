//! T-E-S-24 文件快照回滚引擎(MVP)。
//!
//! 提供两种后端:
//! - `GitBackend`: 利用系统 git 命令做 stash 快照
//! - `CopyBackend`: 通过 `StorageBackend` 抽象将文件备份到 Local/S3/WebDAV
//!
//! `SnapshotEngine` 自动选择后端(git 可用则 git,否则 copy),
//! 并管理所有活跃快照的元数据。
//!
//! T-E-S-44: CopyBackend 改为持有 `Arc<dyn StorageBackend>`,所有 std::fs
//! 调用替换为 backend 异步调用,实现 Local/S3/WebDAV 统一存储。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::DynStorageBackend;

pub type SnapshotId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Git,
    Copy,
}

#[async_trait]
pub trait SnapshotBackend: Send + Sync {
    async fn create(&self, working_dir: &Path, files: &[PathBuf]) -> Result<SnapshotId>;
    async fn rollback(&self, id: &SnapshotId, working_dir: &Path, files: &[PathBuf]) -> Result<()>;
    async fn discard(&self, id: &SnapshotId, working_dir: &Path) -> Result<()>;
}

// ---------------------------------------------------------------------------
// GitBackend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct GitBackend;

impl GitBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn is_git_repo(&self, working_dir: &Path) -> bool {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(working_dir)
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        matches!(output, Ok(s) if s.success())
    }

    fn run_git(&self, working_dir: &Path, args: &[&str]) -> Result<std::process::Output> {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(working_dir);
        for a in args {
            cmd.arg(a);
        }
        let output = cmd
            .output()
            .with_context(|| format!("failed to spawn git process with args: {:?}", args))?;
        Ok(output)
    }
}

impl Default for GitBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SnapshotBackend for GitBackend {
    async fn create(&self, working_dir: &Path, files: &[PathBuf]) -> Result<SnapshotId> {
        let id = Uuid::new_v4().to_string();
        let message = format!("nebula-snapshot-{}", id);

        let mut args = vec!["stash", "create", "-m", &message, "--"];
        let file_strs: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect();
        let file_strs_ref: Vec<&str> = file_strs.iter().map(|s| s.as_str()).collect();
        args.extend(file_strs_ref.iter().copied());

        let output = self
            .run_git(working_dir, &args)
            .context("git stash create failed")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("git stash create failed: {}", stderr.trim()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let commit_hash = stdout.trim();

        if commit_hash.is_empty() {
            Ok(format!("clean:{}", id))
        } else {
            Ok(format!("commit:{}", commit_hash))
        }
    }

    async fn rollback(&self, id: &SnapshotId, working_dir: &Path, files: &[PathBuf]) -> Result<()> {
        let target = if let Some(hash) = id.strip_prefix("commit:") {
            hash.to_string()
        } else if id.strip_prefix("clean:").is_some() {
            "HEAD".to_string()
        } else {
            return Err(anyhow!("invalid snapshot id format: {}", id));
        };

        let file_strs: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect();
        let mut args = vec!["checkout", &target, "--"];
        let file_strs_ref: Vec<&str> = file_strs.iter().map(|s| s.as_str()).collect();
        args.extend(file_strs_ref.iter().copied());

        let output = self
            .run_git(working_dir, &args)
            .context("git checkout failed")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("git checkout failed: {}", stderr.trim()));
        }

        Ok(())
    }

    async fn discard(&self, _id: &SnapshotId, _working_dir: &Path) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CopyBackend
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CopyBackend {
    storage: DynStorageBackend,
}

impl std::fmt::Debug for CopyBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopyBackend")
            .field("storage_kind", &self.storage.kind())
            .finish()
    }
}

/// 计算相对路径(用于 backend 存储 key)。canonicalize 解析符号链接。
async fn pathdiff(path: &Path, base: &Path) -> Option<PathBuf> {
    let path = tokio::fs::canonicalize(path).await.ok()?;
    let base = tokio::fs::canonicalize(base).await.ok()?;

    let mut path_iter = path.components();
    let mut base_iter = base.components();

    loop {
        match (path_iter.next(), base_iter.next()) {
            (Some(p), Some(b)) if p == b => continue,
            (Some(p), Some(_)) => {
                let mut remaining = PathBuf::new();
                remaining.push(p);
                remaining.extend(path_iter);
                return Some(remaining);
            }
            (Some(p), None) => {
                let mut remaining = PathBuf::new();
                remaining.push(p);
                remaining.extend(path_iter);
                return Some(remaining);
            }
            (None, None) => return Some(PathBuf::new()),
            (None, Some(_)) => return None,
        }
    }
}

impl CopyBackend {
    pub fn new(storage: DynStorageBackend) -> Result<Self> {
        Ok(Self { storage })
    }
}

#[async_trait]
impl SnapshotBackend for CopyBackend {
    async fn create(&self, working_dir: &Path, files: &[PathBuf]) -> Result<SnapshotId> {
        let id = Uuid::new_v4().to_string();
        // 在 backend 中创建 snapshot 根目录(backend 路径:`{id}/`)。
        // create_dir 幂等,且 LocalBackend::write 也会自动创建父目录,
        // 但显式调用保证 WebDAV/S3 后端语义一致。
        self.storage
            .create_dir(&id)
            .await
            .with_context(|| format!("failed to create snapshot dir in backend: {}", id))?;

        for file in files {
            let abs_path = if file.is_absolute() {
                file.clone()
            } else {
                working_dir.join(file)
            };

            // 检查文件存在性 + 是否目录(本地 FS)
            let metadata = match tokio::fs::metadata(&abs_path).await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                continue;
            }

            let rel_path = match pathdiff(&abs_path, working_dir).await {
                Some(rel) => rel,
                None => continue,
            };
            // 统一为正斜杠分隔的存储 key(S3/WebDAV 语义)
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");

            // 读取本地文件(被快照的源文件,在本地 FS)
            let bytes = tokio::fs::read(&abs_path)
                .await
                .with_context(|| format!("failed to read {}", abs_path.display()))?;

            // 写入 backend(存储路径:`{id}/{rel_str}`)
            let backend_path = format!("{}/{}", id, rel_str);
            self.storage
                .write(&backend_path, &bytes)
                .await
                .with_context(|| format!("failed to write to backend: {}", backend_path))?;
        }

        Ok(id)
    }

    async fn rollback(&self, id: &SnapshotId, working_dir: &Path, files: &[PathBuf]) -> Result<()> {
        let exists = self
            .storage
            .exists(id)
            .await
            .with_context(|| format!("failed to check snapshot existence: {}", id))?;
        if !exists {
            return Err(anyhow!("snapshot not found: {}", id));
        }

        for file in files {
            let abs_path = if file.is_absolute() {
                file.clone()
            } else {
                working_dir.join(file)
            };

            let rel_path = match pathdiff(&abs_path, working_dir).await {
                Some(rel) => rel,
                None => continue,
            };
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
            let backend_path = format!("{}/{}", id, rel_str);

            let file_exists = match self.storage.exists(&backend_path).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            if !file_exists {
                continue;
            }

            // 从 backend 读取快照内容
            let bytes = self
                .storage
                .read(&backend_path)
                .await
                .with_context(|| format!("failed to read from backend: {}", backend_path))?;

            // 写回本地 FS(恢复用户文件)
            if let Some(parent) = abs_path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(&abs_path, &bytes)
                .await
                .with_context(|| format!("failed to write {}", abs_path.display()))?;
        }

        Ok(())
    }

    async fn discard(&self, id: &SnapshotId, _working_dir: &Path) -> Result<()> {
        // remove_dir 幂等(NotFound 视为 Ok)
        self.storage
            .remove_dir(id)
            .await
            .with_context(|| format!("failed to remove snapshot dir: {}", id))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SnapshotInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub id: SnapshotId,
    pub backend: BackendKind,
    pub working_dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub created_at: i64,
}

// ---------------------------------------------------------------------------
// SnapshotEngine
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SnapshotEngine {
    inner: Arc<SnapshotEngineInner>,
}

struct SnapshotEngineInner {
    git: GitBackend,
    copy: CopyBackend,
    snapshots: Mutex<HashMap<SnapshotId, SnapshotInfo>>,
}

impl SnapshotEngine {
    pub fn new(storage: DynStorageBackend) -> Result<Self> {
        let copy = CopyBackend::new(storage)?;
        Ok(Self {
            inner: Arc::new(SnapshotEngineInner {
                git: GitBackend::new(),
                copy,
                snapshots: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub async fn create_snapshot(
        &self,
        working_dir: &Path,
        files: &[PathBuf],
    ) -> Result<SnapshotId> {
        let normalized_files: Vec<PathBuf> = files
            .iter()
            .map(|f| {
                if f.is_absolute() {
                    f.clone()
                } else {
                    working_dir.join(f)
                }
            })
            .collect();

        let (backend, id) = if self.inner.git.is_git_repo(working_dir) {
            let id = self
                .inner
                .git
                .create(working_dir, &normalized_files)
                .await?;
            (BackendKind::Git, id)
        } else {
            let id = self
                .inner
                .copy
                .create(working_dir, &normalized_files)
                .await?;
            (BackendKind::Copy, id)
        };

        let info = SnapshotInfo {
            id: id.clone(),
            backend,
            working_dir: working_dir.to_path_buf(),
            files: normalized_files,
            created_at: Utc::now().timestamp_millis(),
        };

        self.inner.snapshots.lock().insert(id.clone(), info);

        Ok(id)
    }

    pub async fn rollback(&self, id: &SnapshotId) -> Result<()> {
        let info = {
            let guard = self.inner.snapshots.lock();
            guard.get(id).cloned()
        };

        let info = info.ok_or_else(|| anyhow!("snapshot not found: {}", id))?;

        let result = match info.backend {
            BackendKind::Git => {
                self.inner
                    .git
                    .rollback(&info.id, &info.working_dir, &info.files)
                    .await
            }
            BackendKind::Copy => {
                self.inner
                    .copy
                    .rollback(&info.id, &info.working_dir, &info.files)
                    .await
            }
        };

        if result.is_ok() {
            self.inner.snapshots.lock().remove(id);
        }

        result
    }

    pub async fn discard(&self, id: &SnapshotId) -> Result<()> {
        let info = {
            let guard = self.inner.snapshots.lock();
            guard.get(id).cloned()
        };

        let info = match info {
            Some(i) => i,
            None => return Ok(()),
        };

        let result = match info.backend {
            BackendKind::Git => self.inner.git.discard(&info.id, &info.working_dir).await,
            BackendKind::Copy => self.inner.copy.discard(&info.id, &info.working_dir).await,
        };

        self.inner.snapshots.lock().remove(id);

        result
    }

    pub fn list_snapshots(&self) -> Vec<SnapshotInfo> {
        let guard = self.inner.snapshots.lock();
        let mut list: Vec<SnapshotInfo> = guard.values().cloned().collect();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        list
    }
}

// ---------------------------------------------------------------------------
// DTOs for Tauri commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfoDto {
    pub id: String,
    pub backend: String,
    pub working_dir: String,
    pub files: Vec<String>,
    pub created_at: i64,
}

impl From<SnapshotInfo> for SnapshotInfoDto {
    fn from(info: SnapshotInfo) -> Self {
        Self {
            id: info.id,
            backend: match info.backend {
                BackendKind::Git => "git".to_string(),
                BackendKind::Copy => "copy".to_string(),
            },
            working_dir: info.working_dir.to_string_lossy().to_string(),
            files: info
                .files
                .iter()
                .map(|f| f.to_string_lossy().to_string())
                .collect(),
            created_at: info.created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalBackend;
    use tempfile::TempDir;

    fn create_temp_dir() -> TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("create should succeed")).expect("create should succeed");
        std::fs::write(path, content).expect("update should succeed");
    }

    fn read_file(path: &Path) -> String {
        std::fs::read_to_string(path).expect("get should succeed")
    }

    /// 构造 LocalBackend + SnapshotEngine,返回 (storage, engine) 供测试使用。
    fn create_engine_with_storage(tmp: &Path) -> (DynStorageBackend, SnapshotEngine) {
        let storage_root = tmp.join("storage");
        let backend = LocalBackend::new(&storage_root).expect("create local backend");
        let storage: DynStorageBackend = Arc::new(backend);
        let engine = SnapshotEngine::new(storage.clone()).expect("create engine");
        (storage, engine)
    }

    /// 构造 LocalBackend + CopyBackend,返回 (storage, copy) 供测试使用。
    fn create_copy_backend(tmp: &Path) -> (DynStorageBackend, CopyBackend) {
        let storage_root = tmp.join("storage");
        let backend = LocalBackend::new(&storage_root).expect("create local backend");
        let storage: DynStorageBackend = Arc::new(backend);
        let copy = CopyBackend::new(storage.clone()).expect("create copy backend");
        (storage, copy)
    }

    // -- GitBackend tests --

    fn git_init(dir: &Path) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .arg("init")
            .output()
            .expect("git init failed");
        assert!(output.status.success(), "git init failed");

        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "user.email", "test@test.com"])
            .status();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "user.name", "Test User"])
            .status();
    }

    fn git_commit_all(dir: &Path, message: &str) {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["add", "."])
            .status();
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-m", message])
            .status();
    }

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn test_git_backend_is_git_repo() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let backend = GitBackend::new();
        assert!(!backend.is_git_repo(tmp.path()));

        git_init(tmp.path());
        assert!(backend.is_git_repo(tmp.path()));
    }

    #[tokio::test]
    async fn test_git_backend_create_rollback_clean_worktree() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let dir = tmp.path();
        git_init(dir);

        let file1 = dir.join("a.txt");
        write_file(&file1, "original content");
        git_commit_all(dir, "initial commit");

        let backend = GitBackend::new();
        let id = backend
            .create(dir, &[file1.clone()])
            .await
            .expect("create snapshot");

        assert!(
            id.starts_with("clean:"),
            "clean worktree should produce clean: prefix id"
        );

        write_file(&file1, "modified content");
        assert_eq!(read_file(&file1), "modified content");

        backend
            .rollback(&id, dir, &[file1.clone()])
            .await
            .expect("rollback snapshot");
        assert_eq!(read_file(&file1), "original content");
    }

    #[tokio::test]
    async fn test_git_backend_create_rollback_dirty_worktree() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let dir = tmp.path();
        git_init(dir);

        let file1 = dir.join("a.txt");
        write_file(&file1, "original content");
        git_commit_all(dir, "initial commit");

        write_file(&file1, "staged content");

        let backend = GitBackend::new();
        let id = backend
            .create(dir, &[file1.clone()])
            .await
            .expect("create snapshot");

        assert!(
            id.starts_with("commit:"),
            "dirty worktree should produce commit: prefix id"
        );

        write_file(&file1, "even more modified");
        assert_eq!(read_file(&file1), "even more modified");

        backend
            .rollback(&id, dir, &[file1.clone()])
            .await
            .expect("rollback snapshot");
        assert_eq!(read_file(&file1), "staged content");
    }

    #[tokio::test]
    async fn test_git_backend_discard_clean() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let dir = tmp.path();
        git_init(dir);

        let file1 = dir.join("a.txt");
        write_file(&file1, "original content");
        git_commit_all(dir, "initial commit");

        let backend = GitBackend::new();
        let id = backend
            .create(dir, &[file1.clone()])
            .await
            .expect("create snapshot");

        write_file(&file1, "modified content");

        backend.discard(&id, dir).await.expect("discard snapshot");

        assert_eq!(read_file(&file1), "modified content");
    }

    #[tokio::test]
    async fn test_git_backend_discard_dirty() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let dir = tmp.path();
        git_init(dir);

        let file1 = dir.join("a.txt");
        write_file(&file1, "original content");
        git_commit_all(dir, "initial commit");

        write_file(&file1, "staged content");

        let backend = GitBackend::new();
        let id = backend
            .create(dir, &[file1.clone()])
            .await
            .expect("create snapshot");

        write_file(&file1, "modified content");

        backend.discard(&id, dir).await.expect("discard snapshot");

        assert_eq!(read_file(&file1), "modified content");
    }

    // -- CopyBackend tests --

    #[tokio::test]
    async fn test_copy_backend_create_rollback_discard() {
        let tmp = create_temp_dir();
        let working_dir = tmp.path().join("work");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");

        let file1 = working_dir.join("a.txt");
        let file2 = working_dir.join("b.txt");
        write_file(&file1, "content a");
        write_file(&file2, "content b");

        let (_storage, backend) = create_copy_backend(tmp.path());
        let id = backend
            .create(&working_dir, &[file1.clone(), file2.clone()])
            .await
            .expect("create snapshot");

        write_file(&file1, "modified a");
        write_file(&file2, "modified b");
        assert_eq!(read_file(&file1), "modified a");
        assert_eq!(read_file(&file2), "modified b");

        backend
            .rollback(&id, &working_dir, &[file1.clone(), file2.clone()])
            .await
            .expect("rollback snapshot");
        assert_eq!(read_file(&file1), "content a");
        assert_eq!(read_file(&file2), "content b");
    }

    #[tokio::test]
    async fn test_copy_backend_discard() {
        let tmp = create_temp_dir();
        let working_dir = tmp.path().join("work");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");

        let file1 = working_dir.join("a.txt");
        write_file(&file1, "original");

        let (storage, backend) = create_copy_backend(tmp.path());
        let id = backend
            .create(&working_dir, &[file1.clone()])
            .await
            .expect("create snapshot");

        write_file(&file1, "modified");
        backend.discard(&id, &working_dir).await.expect("discard");

        assert_eq!(read_file(&file1), "modified");

        // 验证 snapshot 目录已从 backend 中移除
        let exists = storage.exists(&id).await.expect("check exists");
        assert!(!exists, "snapshot dir should be removed from backend");
    }

    #[tokio::test]
    async fn test_copy_backend_preserves_file_content() {
        let tmp = create_temp_dir();
        let working_dir = tmp.path().join("work");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");

        let file = working_dir.join("test.txt");
        let original = "hello world\nline2\nline3";
        write_file(&file, original);

        let (_storage, backend) = create_copy_backend(tmp.path());
        let id = backend.create(&working_dir, &[file.clone()]).await.expect("create should succeed");

        write_file(&file, "overwritten content");
        backend
            .rollback(&id, &working_dir, &[file.clone()])
            .await
            .expect("test op should succeed");

        assert_eq!(read_file(&file), original);
    }

    #[tokio::test]
    async fn test_copy_backend_preserves_relative_paths() {
        let tmp = create_temp_dir();
        let working_dir = tmp.path().join("work");
        let sub_dir = working_dir.join("sub").join("nested");
        std::fs::create_dir_all(&sub_dir).expect("create should succeed");

        let file = sub_dir.join("deep.txt");
        let original = "deep content";
        write_file(&file, original);

        let (storage, backend) = create_copy_backend(tmp.path());
        let id = backend.create(&working_dir, &[file.clone()]).await.expect("create should succeed");

        // 验证 backend 中保存了相对路径结构
        let backend_path = format!("{}/sub/nested/deep.txt", id);
        let exists = storage.exists(&backend_path).await.expect("check exists");
        assert!(
            exists,
            "snapshot file should exist at relative path in backend"
        );

        write_file(&file, "modified deep");
        backend
            .rollback(&id, &working_dir, &[file.clone()])
            .await
            .expect("test op should succeed");

        assert_eq!(read_file(&file), original);
    }

    // -- SnapshotEngine tests --

    #[tokio::test]
    async fn test_snapshot_engine_list() {
        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());

        let working_dir = tmp.path().join("work");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");
        let file1 = working_dir.join("a.txt");
        write_file(&file1, "aaa");

        assert_eq!(engine.list_snapshots().len(), 0);

        let id1 = engine
            .create_snapshot(&working_dir, &[file1.clone()])
            .await
            .expect("test op should succeed");
        assert_eq!(engine.list_snapshots().len(), 1);

        let id2 = engine
            .create_snapshot(&working_dir, &[file1.clone()])
            .await
            .expect("test op should succeed");
        assert_eq!(engine.list_snapshots().len(), 2);

        engine.discard(&id1).await.expect("task should complete");
        assert_eq!(engine.list_snapshots().len(), 1);
        assert_eq!(engine.list_snapshots()[0].id, id2);
    }

    #[tokio::test]
    async fn test_snapshot_engine_auto_select_backend_copy() {
        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());

        let working_dir = tmp.path().join("work_not_git");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");
        let file1 = working_dir.join("a.txt");
        write_file(&file1, "test");

        let id = engine
            .create_snapshot(&working_dir, &[file1.clone()])
            .await
            .expect("test op should succeed");
        let list = engine.list_snapshots();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].backend, BackendKind::Copy);
        assert_eq!(list[0].id, id);

        write_file(&file1, "modified");
        engine.rollback(&id).await.expect("task should complete");
        assert_eq!(read_file(&file1), "test");
    }

    #[tokio::test]
    async fn test_snapshot_engine_auto_select_backend_git_clean() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());

        let working_dir = tmp.path().join("work_git");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");
        git_init(&working_dir);

        let file1 = working_dir.join("a.txt");
        write_file(&file1, "original");
        git_commit_all(&working_dir, "initial");

        let id = engine
            .create_snapshot(&working_dir, &[file1.clone()])
            .await
            .expect("test op should succeed");
        let list = engine.list_snapshots();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].backend, BackendKind::Git);
        assert_eq!(list[0].id, id);
        assert!(id.starts_with("clean:"));

        write_file(&file1, "modified");
        engine.rollback(&id).await.expect("task should complete");
        assert_eq!(read_file(&file1), "original");
    }

    #[tokio::test]
    async fn test_snapshot_engine_auto_select_backend_git_dirty() {
        if !git_available() {
            return;
        }

        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());

        let working_dir = tmp.path().join("work_git_dirty");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");
        git_init(&working_dir);

        let file1 = working_dir.join("a.txt");
        write_file(&file1, "original");
        git_commit_all(&working_dir, "initial");

        write_file(&file1, "staged");

        let id = engine
            .create_snapshot(&working_dir, &[file1.clone()])
            .await
            .expect("test op should succeed");
        let list = engine.list_snapshots();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].backend, BackendKind::Git);
        assert_eq!(list[0].id, id);
        assert!(id.starts_with("commit:"));

        write_file(&file1, "modified more");
        engine.rollback(&id).await.expect("task should complete");
        assert_eq!(read_file(&file1), "staged");
    }

    #[tokio::test]
    async fn test_rollback_restores_file_content() {
        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());

        let working_dir = tmp.path().join("work_rb");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");
        let file = working_dir.join("important.txt");
        let content = "line1\nline2\nline3\n";
        write_file(&file, content);

        let id = engine
            .create_snapshot(&working_dir, &[file.clone()])
            .await
            .expect("test op should succeed");

        write_file(&file, "completely different\ncontent here");
        assert_ne!(read_file(&file), content);

        engine.rollback(&id).await.expect("task should complete");
        assert_eq!(read_file(&file), content);
    }

    #[tokio::test]
    async fn test_discard_removes_from_list() {
        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());

        let working_dir = tmp.path().join("work_dc");
        std::fs::create_dir_all(&working_dir).expect("create should succeed");
        let file = working_dir.join("x.txt");
        write_file(&file, "x");

        let id = engine
            .create_snapshot(&working_dir, &[file.clone()])
            .await
            .expect("test op should succeed");
        assert_eq!(engine.list_snapshots().len(), 1);

        engine.discard(&id).await.expect("task should complete");
        assert_eq!(engine.list_snapshots().len(), 0);
    }

    #[tokio::test]
    async fn test_rollback_unknown_id_returns_error() {
        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());
        let result = engine.rollback(&"nonexistent-id".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_discard_unknown_id_is_noop() {
        let tmp = create_temp_dir();
        let (_storage, engine) = create_engine_with_storage(tmp.path());
        let result = engine.discard(&"nonexistent-id".to_string()).await;
        assert!(result.is_ok());
    }
}
