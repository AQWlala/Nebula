//! T-E-S-44 StorageBackendFactory — 根据 StorageConfig 构造后端。
//!
//! 工厂模式统一后端创建逻辑:
//! - `local`:LocalBackend(默认,零新依赖)
//! - `s3`:S3Backend(feature gate `storage-s3`,关闭时返回 Err)
//! - `webdav`:WebDavBackend(feature gate `storage-webdav`,关闭时返回 stub)

use std::sync::Arc;

use super::{DynStorageBackend, LocalBackend, StorageConfig, StorageError, StorageResult, WebDavBackend};

/// 存储后端工厂。
pub struct StorageBackendFactory;

impl StorageBackendFactory {
    /// 根据 StorageConfig 构造存储后端。
    ///
    /// - `local`:始终可用(默认)。
    /// - `s3`:需要 `storage-s3` feature,关闭时返回 Err。
    /// - `webdav`:始终构造实例;`storage-webdav` 关闭时为 stub(方法返回 Err)。
    pub fn from_config(config: &StorageConfig) -> StorageResult<DynStorageBackend> {
        match config.kind.as_str() {
            "local" => {
                let backend = LocalBackend::new(&config.root)?;
                Ok(Arc::new(backend))
            }
            "s3" => {
                // S3 必须显式启用 feature;关闭时工厂返回 Err。
                #[cfg(not(feature = "storage-s3"))]
                {
                    Err(StorageError::Unavailable(
                        "S3 backend requires `storage-s3` feature. Rebuild with --features storage-s3."
                            .to_string(),
                    ))
                }
                #[cfg(feature = "storage-s3")]
                {
                    // feature 开启但 aws-sdk-s3 完整实现留 TODO(T-E-S-44)。
                    let _ = (&config.s3_bucket, &config.s3_region, &config.s3_endpoint);
                    Err(StorageError::Unavailable(
                        "S3 backend not yet implemented (feature enabled, TODO)".to_string(),
                    ))
                }
            }
            "webdav" => {
                // WebDAV 始终构造实例(feature 关闭时为 stub,方法返回 Err)。
                let url = config
                    .webdav_url
                    .as_ref()
                    .ok_or_else(|| {
                        StorageError::Unavailable("WebDAV backend requires `webdav_url`".to_string())
                    })?;
                let backend = WebDavBackend::new(
                    url,
                    config.webdav_username.clone(),
                    config.webdav_password.clone(),
                )?;
                Ok(Arc::new(backend))
            }
            other => Err(StorageError::Unavailable(format!(
                "unknown storage backend: {other} (expected local/s3/webdav)"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试:工厂默认构造 local 后端。
    #[test]
    fn factory_local_default() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let config = StorageConfig {
            kind: "local".to_string(),
            root: tmp.path().to_string_lossy().to_string(),
            ..Default::default()
        };
        let backend = StorageBackendFactory::from_config(&config).expect("create local backend");
        assert_eq!(backend.kind(), "local");
    }

    /// 测试:S3 无 feature 时返回 Err。
    #[test]
    fn factory_s3_without_feature_returns_err() {
        let config = StorageConfig {
            kind: "s3".to_string(),
            s3_bucket: Some("test".to_string()),
            ..Default::default()
        };
        let result = StorageBackendFactory::from_config(&config);
        assert!(result.is_err());
        match result {
            Err(StorageError::Unavailable(_)) => {}
            other => panic!("expected Unavailable, got: {:?}", other.as_ref().err()),
        }
    }

    /// 测试:未知后端类型返回 Err。
    #[test]
    fn factory_unknown_backend_returns_err() {
        let config = StorageConfig {
            kind: "ftp".to_string(),
            ..Default::default()
        };
        let result = StorageBackendFactory::from_config(&config);
        assert!(result.is_err());
    }

    /// 测试:WebDAV 构造(无 feature 时为 stub,但构造成功)。
    #[test]
    fn factory_webdav_stub() {
        let config = StorageConfig {
            kind: "webdav".to_string(),
            webdav_url: Some("http://localhost:9999/".to_string()),
            ..Default::default()
        };
        let backend = StorageBackendFactory::from_config(&config).expect("construct webdav backend");
        assert_eq!(backend.kind(), "webdav");
    }

    /// 测试:WebDAV 缺少 URL 返回 Err。
    #[test]
    fn factory_webdav_missing_url() {
        let config = StorageConfig {
            kind: "webdav".to_string(),
            webdav_url: None,
            ..Default::default()
        };
        let result = StorageBackendFactory::from_config(&config);
        assert!(result.is_err());
    }
}
