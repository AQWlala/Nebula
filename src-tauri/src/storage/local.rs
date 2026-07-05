//! T-E-S-44 LocalBackend — 基于 tokio::fs 的本地存储后端。
//!
//! 完整实现所有 StorageBackend 方法。零新依赖(tokio::fs + async_stream 已有)。
//!
//! ## 安全
//! - 根路径沙箱:拒绝 `..` / 绝对路径 / Windows 盘符。
//! - tmp+rename 原子写:写入 `{path}.partial` 后 rename,避免半写状态。
//!
//! ## 幂等
//! - `delete` / `remove_dir`:NotFound 视为 Ok(参考 transport.rs `ack`)。
//! - `create_dir`:`create_dir_all` 已幂等。

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{FileMetadata, StorageBackend, StorageError, StorageResult};

/// 将 `Pin<Box<dyn Stream>>` 包装为 `Unpin` 类型。
/// `Pin<Box<T>>` 本身是 `Unpin`(Box 是指针类型),所以这个包装器自动是 `Unpin`。
/// 用于 `read_stream` 返回值满足 `+ Send + Unpin` 约束。
struct BoxedUnpinStream {
    inner: Pin<Box<dyn Stream<Item = StorageResult<Bytes>> + Send>>,
}

impl Stream for BoxedUnpinStream {
    type Item = StorageResult<Bytes>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// 本地文件系统存储后端。
pub struct LocalBackend {
    /// 根路径,所有存储路径都解析为该路径下的子路径。
    root: PathBuf,
}

impl LocalBackend {
    /// 创建后端,会自动创建根目录。
    pub fn new<P: AsRef<Path>>(root: P) -> StorageResult<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// 将存储路径(正斜杠分隔)解析为根路径下的绝对 FS 路径,执行沙箱检查。
    ///
    /// 拒绝:
    /// - 空路径
    /// - 绝对路径(`/foo`, `\foo`)
    /// - Windows 盘符(`C:\foo`)
    /// - 路径遍历(`../etc/passwd`)
    fn resolve(&self, path: &str) -> StorageResult<PathBuf> {
        if path.is_empty() {
            return Err(StorageError::InvalidPath("empty path".to_string()));
        }
        // 绝对路径检查(正斜杠 / 反斜杠开头)
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(StorageError::InvalidPath(format!(
                "absolute path not allowed: {path}"
            )));
        }
        // Windows 盘符检查(如 C:)
        let bytes = path.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err(StorageError::InvalidPath(format!(
                "drive path not allowed: {path}"
            )));
        }
        // 路径遍历检查 — 同时检查正斜杠和反斜杠分隔的组件
        for component in path.split(|c| c == '/' || c == '\\') {
            if component == ".." {
                return Err(StorageError::InvalidPath(format!(
                    "path traversal not allowed: {path}"
                )));
            }
        }
        // 将正斜杠转为平台分隔符
        let relative = path.replace('/', std::path::MAIN_SEPARATOR_STR);
        Ok(self.root.join(relative))
    }

    /// 构造 tmp 文件路径(同目录,追加 `.partial` 后缀)。
    /// tmp 文件必须与目标在同一文件系统,确保 rename 是原子的。
    fn tmp_path(&self, fs_path: &Path) -> PathBuf {
        let mut name = fs_path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".partial");
        fs_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(name)
    }

    /// 读取文件 mtime(Unix 毫秒)。
    async fn read_mtime(metadata: &std::fs::Metadata) -> Option<i64> {
        metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
    }
}

#[async_trait]
impl StorageBackend for LocalBackend {
    fn kind(&self) -> &'static str {
        "local"
    }

    async fn read(&self, path: &str) -> StorageResult<Vec<u8>> {
        let fs_path = self.resolve(path)?;
        match fs::read(&fs_path).await {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StorageError::NotFound(path.to_string()))
            }
            Err(e) => Err(StorageError::Io { source: e }),
        }
    }

    async fn write(&self, path: &str, bytes: &[u8]) -> StorageResult<()> {
        let fs_path = self.resolve(path)?;
        // 确保父目录存在
        if let Some(parent) = fs_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        // tmp + rename 原子写
        let tmp = self.tmp_path(&fs_path);
        fs::write(&tmp, bytes).await?;
        fs::rename(&tmp, &fs_path)
            .await
            .map_err(|e| StorageError::Io { source: e })?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> StorageResult<()> {
        let fs_path = self.resolve(path)?;
        match fs::remove_file(&fs_path).await {
            Ok(()) => Ok(()),
            // 幂等:NotFound 视为 Ok
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Io { source: e }),
        }
    }

    async fn exists(&self, path: &str) -> StorageResult<bool> {
        let fs_path = self.resolve(path)?;
        match fs::metadata(&fs_path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StorageError::Io { source: e }),
        }
    }

    async fn metadata(&self, path: &str) -> StorageResult<FileMetadata> {
        let fs_path = self.resolve(path)?;
        match fs::metadata(&fs_path).await {
            Ok(m) => Ok(FileMetadata {
                path: path.to_string(),
                size: m.len(),
                modified_at: Self::read_mtime(&m).await,
                is_dir: m.is_dir(),
                etag: None,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StorageError::NotFound(path.to_string()))
            }
            Err(e) => Err(StorageError::Io { source: e }),
        }
    }

    async fn read_stream(
        &self,
        path: &str,
    ) -> StorageResult<Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>> {
        let fs_path = self.resolve(path)?;
        let mut file = match fs::File::open(&fs_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StorageError::NotFound(path.to_string()));
            }
            Err(e) => return Err(StorageError::Io { source: e }),
        };

        // 用 async_stream 生成器构造分块流(64KB 块)。
        let stream = async_stream::stream! {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match file.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => yield Ok(Bytes::copy_from_slice(&buf[..n])),
                    Err(e) => {
                        yield Err(StorageError::Io { source: e });
                        break;
                    }
                }
            }
        };

        Ok(Box::new(BoxedUnpinStream {
            inner: Box::pin(stream),
        }))
    }

    async fn write_stream(
        &self,
        path: &str,
        stream: Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>,
        _expected_size: Option<u64>,
    ) -> StorageResult<()> {
        let fs_path = self.resolve(path)?;
        if let Some(parent) = fs_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let tmp = self.tmp_path(&fs_path);
        let mut file = fs::File::create(&tmp).await?;
        let mut stream = stream;
        while let Some(item) = stream.next().await {
            let bytes = item?;
            file.write_all(&bytes).await?;
        }
        file.flush().await?;
        // 显式 drop 文件句柄,确保 Windows 上 rename 不会因句柄占用失败。
        drop(file);
        fs::rename(&tmp, &fs_path)
            .await
            .map_err(|e| StorageError::Io { source: e })?;
        Ok(())
    }

    async fn create_dir(&self, path: &str) -> StorageResult<()> {
        let fs_path = self.resolve(path)?;
        // create_dir_all 已幂等(目录存在不报错)。
        fs::create_dir_all(&fs_path).await?;
        Ok(())
    }

    async fn remove_dir(&self, path: &str) -> StorageResult<()> {
        let fs_path = self.resolve(path)?;
        match fs::remove_dir_all(&fs_path).await {
            Ok(()) => Ok(()),
            // 幂等:NotFound 视为 Ok
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Io { source: e }),
        }
    }

    async fn list(&self, prefix: &str) -> StorageResult<Vec<FileMetadata>> {
        // 空前缀视为根目录(resolve 会拒绝空路径)。
        let dir = if prefix.is_empty() {
            self.root.clone()
        } else {
            self.resolve(prefix)?
        };
        let mut entries = fs::read_dir(&dir).await?;
        let mut result = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            let name = entry.file_name().to_string_lossy().to_string();
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let modified_at = Self::read_mtime(&metadata).await;
            result.push(FileMetadata {
                path,
                size: metadata.len(),
                modified_at,
                is_dir: metadata.is_dir(),
                etag: None,
            });
        }
        result.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_backend() -> LocalBackend {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.into_path();
        LocalBackend::new(&path).expect("create local backend")
    }

    /// 测试:write + read 往返。
    #[tokio::test]
    async fn write_read_round_trip() {
        let backend = temp_backend();
        let content = b"hello world";
        backend.write("foo.txt", content).await.expect("write");
        let read = backend.read("foo.txt").await.expect("read");
        assert_eq!(read, content);
    }

    /// 测试:流式读写 10MB。
    #[tokio::test]
    async fn stream_read_write_10mb() {
        let backend = temp_backend();
        // 构造 10MB 数据,每块 1MB
        let chunk = vec![0xABu8; 1024 * 1024]; // 1MB
        let total_chunks = 10;

        // 构造输入流
        let chunks: Vec<StorageResult<Bytes>> = (0..total_chunks)
            .map(|_| Ok(Bytes::copy_from_slice(&chunk)))
            .collect();
        let input_stream: Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin> =
            Box::new(futures::stream::iter(chunks));
        backend
            .write_stream("big.bin", input_stream, Some((chunk.len() * total_chunks) as u64))
            .await
            .expect("write_stream");

        // 流式读回并累加
        let read_stream = backend.read_stream("big.bin").await.expect("read_stream");
        let mut total = 0u64;
        futures::pin_mut!(read_stream);
        while let Some(item) = read_stream.next().await {
            let bytes = item.expect("stream item");
            total += bytes.len() as u64;
        }
        assert_eq!(total, (chunk.len() * total_chunks) as u64);
    }

    /// 测试:delete 幂等(删除不存在的文件不报错)。
    #[tokio::test]
    async fn delete_idempotent() {
        let backend = temp_backend();
        backend.write("foo.txt", b"hi").await.expect("write");
        backend.delete("foo.txt").await.expect("delete first");
        // 再次删除不应报错
        backend.delete("foo.txt").await.expect("delete again");
    }

    /// 测试:create_dir 递归 + 幂等。
    #[tokio::test]
    async fn create_dir_recursive_and_idempotent() {
        let backend = temp_backend();
        backend
            .create_dir("a/b/c")
            .await
            .expect("create nested dirs");
        // 再次创建不应报错
        backend.create_dir("a/b/c").await.expect("create again");
        // 验证目录可写
        backend
            .write("a/b/c/file.txt", b"deep")
            .await
            .expect("write into nested dir");
    }

    /// 测试:list 非递归(只列出直接子条目)。
    #[tokio::test]
    async fn list_non_recursive() {
        let backend = temp_backend();
        backend.write("a.txt", b"a").await.expect("write a");
        backend.write("b.txt", b"b").await.expect("write b");
        backend.create_dir("sub").await.expect("create sub");
        backend.write("sub/c.txt", b"c").await.expect("write sub/c");

        let entries = backend.list("").await.expect("list root");
        let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"sub"));
        // 非递归:不应包含 sub/c.txt
        assert!(!names.contains(&"sub/c.txt"));
    }

    /// 测试:路径遍历拒绝(../etc/passwd)。
    #[tokio::test]
    async fn path_traversal_rejected() {
        let backend = temp_backend();
        let result = backend.write("../etc/passwd", b"evil").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            StorageError::InvalidPath(msg) => {
                assert!(msg.contains(".."), "error should mention traversal: {msg}");
            }
            other => panic!("expected InvalidPath, got {other:?}"),
        }

        // 绝对路径也应拒绝
        let abs = if cfg!(windows) {
            "C:/Windows/System32/evil.txt"
        } else {
            "/etc/passwd"
        };
        assert!(backend.write(abs, b"evil").await.is_err());
    }

    /// 测试:tmp+rename 原子写(无半写)。
    /// 写入完成后目标文件存在且内容正确,tmp 文件已被清理。
    #[tokio::test]
    async fn atomic_write_no_partial() {
        let backend = temp_backend();
        let content = b"atomic content";
        backend.write("atom.txt", content).await.expect("write");

        // 目标文件存在
        assert!(backend.exists("atom.txt").await.expect("exists"));
        // 内容正确
        let read = backend.read("atom.txt").await.expect("read");
        assert_eq!(read, content);

        // 验证 .partial 临时文件不存在(已被 rename 清理)
        let fs_path = backend.resolve("atom.txt").expect("resolve");
        let tmp = backend.tmp_path(&fs_path);
        assert!(!tmp.exists(), "tmp file should not exist after write: {}", tmp.display());
    }

    /// 测试:read 不存在的文件返回 NotFound。
    #[tokio::test]
    async fn read_not_found() {
        let backend = temp_backend();
        let result = backend.read("nonexistent.txt").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    /// 测试:remove_dir 幂等。
    #[tokio::test]
    async fn remove_dir_idempotent() {
        let backend = temp_backend();
        backend.create_dir("dir").await.expect("create dir");
        backend.write("dir/file.txt", b"x").await.expect("write file");
        backend.remove_dir("dir").await.expect("remove dir first");
        // 再次删除不应报错
        backend.remove_dir("dir").await.expect("remove dir again");
    }

    /// 测试:metadata 返回正确信息。
    #[tokio::test]
    async fn metadata_correct() {
        let backend = temp_backend();
        backend.write("meta.txt", b"1234567890").await.expect("write");
        let meta = backend.metadata("meta.txt").await.expect("metadata");
        assert_eq!(meta.path, "meta.txt");
        assert_eq!(meta.size, 10);
        assert!(!meta.is_dir);
        assert!(meta.modified_at.is_some());
    }

    /// 测试:exists 对目录返回 true。
    #[tokio::test]
    async fn exists_directory() {
        let backend = temp_backend();
        backend.create_dir("mydir").await.expect("create dir");
        assert!(backend.exists("mydir").await.expect("exists dir"));
    }
}
