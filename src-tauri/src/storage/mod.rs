//! T-E-S-44 StorageBackend trait — 统一存储后端抽象。
//!
//! 支持 Local / S3 / WebDAV 三种后端,通过 trait 对象在运行时切换。
//! - `LocalBackend`:tokio::fs 异步,根路径沙箱,tmp+rename 原子写。
//! - `S3Backend`:feature gate `storage-s3`,默认编译 stub 返回 Err。
//! - `WebDavBackend`:feature gate `storage-webdav`,默认编译 stub;
//!   feature 开启时用 reqwest 手写 PUT/GET/DELETE/MKCOL/PROPFIND。
//!
//! ## 设计约束
//! - **Path 抽象为 `&str`**:S3 key / WebDAV href 非 FS Path,统一字符串(正斜杠分隔)。
//! - **tmp+rename 原子写**:LocalBackend 写入 tmp 后 rename,避免半写状态。
//! - **路径沙箱**:LocalBackend 拒绝 `..` / 绝对路径。
//! - **幂等 delete/remove_dir**:NotFound 视为 Ok。
//! - **不暴露 rename**:跨后端语义不一;调用方 read→write→delete 模拟。
//! - **StorageError enum**:thiserror(已有),封装 io/http/协议错误。

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use serde::Serialize;
use thiserror::Error;

// ---------------------------------------------------------------------------
// StorageError
// ---------------------------------------------------------------------------

/// 存储后端错误。
#[derive(Debug, Error)]
pub enum StorageError {
    /// 路径不存在(对 delete/remove_dir 幂等场景调用方自行处理)。
    #[error("not found: {0}")]
    NotFound(String),
    /// 底层 IO 错误(tokio::fs / 网络 read)。
    #[error("io: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },
    /// HTTP 层错误(reqwest 状态码非 2xx)。
    #[error("http: {status} {body}")]
    Http { status: u16, body: String },
    /// reqwest 网络错误(连接失败 / 超时 / DNS)。
    #[error("request: {source}")]
    Request {
        #[from]
        source: reqwest::Error,
    },
    /// 路径不合法(包含 `..` / 绝对路径 / 空路径)。
    #[error("invalid path: {0}")]
    InvalidPath(String),
    /// 后端不可用(feature 未启用或配置缺失)。
    #[error("backend unavailable: {0}")]
    Unavailable(String),
}

pub type StorageResult<T> = Result<T, StorageError>;

// ---------------------------------------------------------------------------
// FileMetadata
// ---------------------------------------------------------------------------

/// 文件元数据。`list` / `metadata` 返回。
#[derive(Debug, Clone, Serialize)]
pub struct FileMetadata {
    /// 存储路径(正斜杠分隔相对路径)。
    pub path: String,
    /// 字节数。
    pub size: u64,
    /// Unix 毫秒时间戳(若后端支持)。
    pub modified_at: Option<i64>,
    /// 是否为目录(WebDAV collection / S3 prefix)。
    pub is_dir: bool,
    /// ETag(若后端返回)。
    pub etag: Option<String>,
}

// ---------------------------------------------------------------------------
// StorageBackend trait
// ---------------------------------------------------------------------------

/// 存储后端 trait:统一 Local / S3 / WebDAV 接口。
///
/// 路径统一用 `&str`(正斜杠分隔相对路径)。S3 key / WebDAV href 非 FS Path,
/// 统一字符串抽象避免 `Path` 跨平台问题。
///
/// 不暴露 `rename`:S3 无原子 rename,跨后端语义不一;调用方 read→write→delete 模拟。
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// 后端类型标识(`"local"` / `"s3"` / `"webdav"`)。
    fn kind(&self) -> &'static str;

    /// 读取整个文件为 `Vec<u8>`。小文件用;大文件用 `read_stream`。
    async fn read(&self, path: &str) -> StorageResult<Vec<u8>>;

    /// 写入字节数组(原子写:Local 用 tmp+rename;WebDAV/S3 用 PUT)。
    async fn write(&self, path: &str, bytes: &[u8]) -> StorageResult<()>;

    /// 删除文件。幂等:NotFound 视为 Ok。
    async fn delete(&self, path: &str) -> StorageResult<()>;

    /// 检查文件是否存在。
    async fn exists(&self, path: &str) -> StorageResult<bool>;

    /// 返回文件元数据。
    async fn metadata(&self, path: &str) -> StorageResult<FileMetadata>;

    /// 流式读取:返回 `Box<dyn Stream<Item = StorageResult<Bytes>>>`。
    /// 大文件(snapshot 备份)用,避免内存撑爆。
    async fn read_stream(
        &self,
        path: &str,
    ) -> StorageResult<Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>>;

    /// 流式写入:消费 `Box<dyn Stream<Item = StorageResult<Bytes>>>`。
    /// `expected_size` 用于 S3 multipart 的 Content-Length 提示(可空)。
    async fn write_stream(
        &self,
        path: &str,
        stream: Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>,
        expected_size: Option<u64>,
    ) -> StorageResult<()>;

    /// 创建目录(Local 用 create_dir_all;WebDAV 用 MKCOL)。幂等。
    async fn create_dir(&self, path: &str) -> StorageResult<()>;

    /// 递归删除目录。幂等:NotFound 视为 Ok。
    async fn remove_dir(&self, path: &str) -> StorageResult<()>;

    /// 列出 prefix 下的直接条目(非递归)。
    async fn list(&self, prefix: &str) -> StorageResult<Vec<FileMetadata>>;
}

/// 动态分发后端类型别名(`Arc<dyn StorageBackend>`)。
pub type DynStorageBackend = Arc<dyn StorageBackend>;

// ---------------------------------------------------------------------------
// 子模块
// ---------------------------------------------------------------------------

pub mod local;
pub mod registry;
pub mod s3;
pub mod webdav;

pub use local::LocalBackend;
pub use registry::StorageBackendFactory;
pub use s3::S3Backend;
pub use webdav::WebDavBackend;

// ---------------------------------------------------------------------------
// StorageConfig
// ---------------------------------------------------------------------------

/// 存储后端配置(从 AppConfig 派生,供工厂构造后端)。
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// 后端类型:`"local"` / `"s3"` / `"webdav"`。
    pub kind: String,
    /// Local 根路径(仅 `kind == "local"` 时使用)。
    pub root: String,
    /// WebDAV base URL(仅 `kind == "webdav"` 时使用)。
    pub webdav_url: Option<String>,
    /// WebDAV 用户名(Basic Auth)。
    pub webdav_username: Option<String>,
    /// WebDAV 密码(Basic Auth)。
    pub webdav_password: Option<String>,
    /// S3 bucket(仅 `kind == "s3"` 时使用)。
    pub s3_bucket: Option<String>,
    /// S3 region。
    pub s3_region: Option<String>,
    /// S3 endpoint(自定义 MinIO 等)。
    pub s3_endpoint: Option<String>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            kind: "local".to_string(),
            root: std::env::temp_dir()
                .join("nebula-storage")
                .to_string_lossy()
                .to_string(),
            webdav_url: None,
            webdav_username: None,
            webdav_password: None,
            s3_bucket: None,
            s3_region: None,
            s3_endpoint: None,
        }
    }
}
