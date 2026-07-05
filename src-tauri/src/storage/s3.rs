//! T-E-S-44 S3Backend — feature gate `storage-s3`。
//!
//! 默认编译 stub:所有方法返回 `StorageError::Unavailable`。
//! 启用 `storage-s3` feature 后链接 `aws-sdk-s3`(完整实现留 TODO)。
//!
//! ## 设计
//! S3 key 是扁平字符串(正斜杠分隔),无真实目录语义:
//! - `create_dir` / `remove_dir`:S3 无目录,视为 no-op 或对 prefix 批量 delete。
//! - `list`:用 ListObjectsV2 + prefix/delimiter 模拟目录层级。
//! - `rename`:S3 无原子 rename,trait 不暴露;调用方 read→write→delete 模拟。

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

use super::{FileMetadata, StorageBackend, StorageError, StorageResult};

/// S3 存储后端。
///
/// `storage-s3` feature 关闭时为 stub,所有方法返回 Err。
/// feature 开启后应注入 `aws_sdk_s3::Client`(TODO,T-E-S-44)。
pub struct S3Backend {
    /// 占位字段,feature 开启后替换为 aws_sdk_s3::Client。
    _private: (),
}

impl S3Backend {
    /// 构造 stub 实例。
    ///
    /// feature 关闭时也返回 Ok(stub),让工厂能构造实例;
    /// 真正的 IO 操作在方法层面返回 Err。
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for S3Backend {
    fn default() -> Self {
        Self::new()
    }
}

/// feature 关闭时的统一错误。
#[cfg(not(feature = "storage-s3"))]
fn unavailable() -> StorageError {
    StorageError::Unavailable(
        "S3 backend requires `storage-s3` feature (aws-sdk-s3). Rebuild with --features storage-s3."
            .to_string(),
    )
}

#[async_trait]
impl StorageBackend for S3Backend {
    fn kind(&self) -> &'static str {
        "s3"
    }

    async fn read(&self, _path: &str) -> StorageResult<Vec<u8>> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            // TODO(T-E-S-44): 用 aws_sdk_s3::Client::get_object 实现。
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn write(&self, _path: &str, _bytes: &[u8]) -> StorageResult<()> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn delete(&self, _path: &str) -> StorageResult<()> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn exists(&self, _path: &str) -> StorageResult<bool> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn metadata(&self, _path: &str) -> StorageResult<FileMetadata> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn read_stream(
        &self,
        _path: &str,
    ) -> StorageResult<Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn write_stream(
        &self,
        _path: &str,
        _stream: Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>,
        _expected_size: Option<u64>,
    ) -> StorageResult<()> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn create_dir(&self, _path: &str) -> StorageResult<()> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn remove_dir(&self, _path: &str) -> StorageResult<()> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }

    async fn list(&self, _prefix: &str) -> StorageResult<Vec<FileMetadata>> {
        #[cfg(not(feature = "storage-s3"))]
        {
            Err(unavailable())
        }
        #[cfg(feature = "storage-s3")]
        {
            Err(StorageError::Unavailable(
                "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试:S3Backend stub(feature 关闭时返回 Err)。
    #[tokio::test]
    async fn s3_stub_returns_err() {
        let backend = S3Backend::new();
        assert_eq!(backend.kind(), "s3");

        // feature 关闭时所有方法应返回 Err
        let result = backend.read("foo.txt").await;
        assert!(result.is_err());
        match result {
            Err(StorageError::Unavailable(_)) => {}
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }
}
