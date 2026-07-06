//! T-E-S-44 WebDavBackend — feature gate `storage-webdav`。
//!
//! 默认编译 stub:所有方法返回 `StorageError::Unavailable`。
//! 启用 `storage-webdav` feature 后用 reqwest 手写 WebDAV REST。
//!
//! ## WebDAV 方法映射
//! | StorageBackend | WebDAV 方法 | 说明 |
//! |----------------|-------------|------|
//! | `read`         | GET         | 返回文件内容 |
//! | `write`        | PUT         | 原子覆盖(服务器端保证) |
//! | `delete`       | DELETE      | 幂等(404 视为 Ok) |
//! | `exists`       | HEAD        | 200=存在, 404=不存在 |
//! | `metadata`     | PROPFIND    | Depth: 0,解析 XML |
//! | `read_stream`  | GET         | 流式响应 |
//! | `write_stream` | PUT         | 流式请求体 |
//! | `create_dir`   | MKCOL       | 幂等(405=已存在视为 Ok) |
//! | `remove_dir`   | DELETE      | 递归删除 collection |
//! | `list`         | PROPFIND    | Depth: 1,解析 XML |
//!
//! 零新依赖:reqwest 已有。XML 解析用简单字符串匹配(不引入 xml-rs)。

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
// Method / StatusCode 在 storage-webdav feature 实现中使用,默认编译会报 unused。
// 加 allow 避免 warning 噪音,feature 启用时自然消除。
#[allow(unused_imports)]
use reqwest::{Method, StatusCode};

use crate::security::SsrfGuard;

use super::{FileMetadata, StorageBackend, StorageError, StorageResult};

/// WebDAV 存储后端。
pub struct WebDavBackend {
    /// feature 开启时持有 reqwest::Client;关闭时为 None(stub)。
    #[cfg(feature = "storage-webdav")]
    client: reqwest::Client,
    /// WebDAV base URL(如 `https://example.com/dav/`)。
    base_url: String,
    /// Basic Auth 用户名。
    username: Option<String>,
    /// Basic Auth 密码。
    password: Option<String>,
}

impl WebDavBackend {
    /// 构造后端。
    ///
    /// feature 关闭时返回 stub 实例(不持有 client),方法层面返回 Err。
    #[allow(unused_variables)]
    pub fn new(
        base_url: impl Into<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> StorageResult<Self> {
        let base_url = base_url.into();
        // M7b #94: SSRF 校验 — WebDAV 通常是远端服务,不需要 allow_loopback。
        SsrfGuard::new().validate_url(&base_url).map_err(|e| {
            StorageError::Unavailable(format!("SSRF validation failed for WebDAV URL: {e}"))
        })?;
        #[cfg(feature = "storage-webdav")]
        {
            let client = reqwest::Client::builder()
                .build()
                .map_err(|e| StorageError::Unavailable(format!("reqwest client build failed: {e}")))?;
            Ok(Self {
                client,
                base_url,
                username,
                password,
            })
        }
        #[cfg(not(feature = "storage-webdav"))]
        {
            Ok(Self {
                base_url,
                username,
                password,
            })
        }
    }

    /// feature 关闭时的统一错误。
    #[cfg(not(feature = "storage-webdav"))]
    fn unavailable() -> StorageError {
        StorageError::Unavailable(
            "WebDAV backend requires `storage-webdav` feature. Rebuild with --features storage-webdav."
                .to_string(),
        )
    }

    /// 构造完整 URL:base_url + path(正斜杠拼接)。
    fn url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{base}/{path}")
    }

    /// 给 reqwest 请求添加 Basic Auth(若配置了凭证)。
    #[cfg(feature = "storage-webdav")]
    fn with_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match (&self.username, &self.password) {
            (Some(u), Some(p)) => req.basic_auth(u, Some(p)),
            _ => req,
        }
    }

    /// 检查 HTTP 状态码,2xx 返回 Ok,404 返回 NotFound,其他返回 Http 错误。
    #[cfg(feature = "storage-webdav")]
    async fn check_status(response: reqwest::Response, path: &str) -> StorageResult<reqwest::Response> {
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else if status == StatusCode::NOT_FOUND {
            Err(StorageError::NotFound(path.to_string()))
        } else {
            let body = response
                .text()
                .await
                .unwrap_or_default();
            Err(StorageError::Http {
                status: status.as_u16(),
                body,
            })
        }
    }

    /// 从 PROPFIND XML 响应中解析文件列表。
    /// 简单字符串匹配,不引入 xml-rs。
    #[cfg(feature = "storage-webdav")]
    fn parse_propfind_response(body: &str, prefix: &str) -> Vec<FileMetadata> {
        let mut results = Vec::new();
        // WebDAV PROPFIND 响应格式:
        // <D:response>
        //   <D:href>/dav/path/to/file</D:href>
        //   <D:propstat>
        //     <D:prop>
        //       <D:getcontentlength>1234</D:getcontentlength>
        //       <D:getlastmodified>Mon, 01 Jan 2024 00:00:00 GMT</D:getlastmodified>
        //       <D:resourcetype><D:collection/></D:resourcetype>
        //       <D:getetag>"abc"</D:getetag>
        //     </D:prop>
        //   </D:propstat>
        // </D:response>

        // 大小写不敏感地按 <d:response> 分割(常见 D: / d: 前缀)
        let body_lower = body.to_lowercase();
        let open_tag = "<d:response>";
        let close_tag = "</d:response>";
        let mut segments: Vec<&str> = Vec::new();
        let mut search_start = 0;
        while let Some(rel_start) = body_lower[search_start..].find(open_tag) {
            let abs_start = search_start + rel_start + open_tag.len();
            if let Some(rel_end) = body_lower[abs_start..].find(close_tag) {
                let abs_end = abs_start + rel_end;
                segments.push(&body[abs_start..abs_end]);
                search_start = abs_end + close_tag.len();
            } else {
                // 没有闭合标签,取剩余部分
                segments.push(&body[abs_start..]);
                break;
            }
        }

        for resp in segments {
            // 提取 href(大小写不敏感)
            let href = extract_xml_tag_ci(resp, "d:href")
                .or_else(|| extract_xml_tag_ci(resp, "href"))
                .unwrap_or_default();
            let decoded_href = html_decode(&href);

            // 提取 size
            let size: u64 = extract_xml_tag_ci(resp, "d:getcontentlength")
                .or_else(|| extract_xml_tag_ci(resp, "getcontentlength"))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);

            // 提取 is_dir(有 <collection/> 子标签 = 目录)
            let resp_lower = resp.to_lowercase();
            let is_dir = resp_lower.contains("<d:collection/>")
                || resp_lower.contains("<d:collection />")
                || resp_lower.contains("<d:collection></d:collection>");

            // 提取 etag
            let etag = extract_xml_tag_ci(resp, "d:getetag")
                .or_else(|| extract_xml_tag_ci(resp, "getetag"))
                .map(|s| s.trim_matches('"').to_string());

            // 提取 modified_at(RFC 1123 → Unix 毫秒)
            let modified_at = extract_xml_tag_ci(resp, "d:getlastmodified")
                .or_else(|| extract_xml_tag_ci(resp, "getlastmodified"))
                .and_then(|s| parse_http_date_to_millis(s.trim()));

            // 构造存储路径:从 href 中提取相对 prefix 的路径
            let path = href_to_storage_path(&decoded_href, &prefix);
            if !path.is_empty() {
                results.push(FileMetadata {
                    path,
                    size,
                    modified_at,
                    is_dir,
                    etag,
                });
            }
        }
        results.sort_by(|a, b| a.path.cmp(&b.path));
        results
    }
}

/// 从 XML 文本中大小写不敏感地提取第一个 `<tag>...</tag>` 的内容。
/// tag 参数应小写(如 "d:href")。
#[cfg(feature = "storage-webdav")]
fn extract_xml_tag_ci(xml: &str, tag: &str) -> Option<String> {
    let xml_lower = xml.to_lowercase();
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml_lower.find(&open)? + open.len();
    let end = xml_lower[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

/// HTML 实体解码(&amp; &lt; &gt; &quot; &apos;)。
#[cfg(feature = "storage-webdav")]
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// 将 WebDAV href 转换为存储路径(相对 prefix)。
#[cfg(feature = "storage-webdav")]
fn href_to_storage_path(href: &str, prefix: &str) -> String {
    // href 可能是绝对路径(/dav/path/to/file)或完整 URL
    // 去掉 URL scheme + host
    let path = if let Some(pos) = href.find("://") {
        let after_scheme = &href[pos + 3..];
        if let Some(slash) = after_scheme.find('/') {
            &after_scheme[slash..]
        } else {
            ""
        }
    } else {
        href
    };
    // 去掉前导斜杠
    let path = path.trim_start_matches('/');
    // 去掉 URL 编码的空格等(%20)
    let path = path.replace("%20", " ");
    // 如果 prefix 非空,去掉 prefix 前缀
    if !prefix.is_empty() {
        let prefix = prefix.trim_start_matches('/');
        if let Some(rest) = path.strip_prefix(prefix) {
            rest.trim_start_matches('/').to_string()
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    }
}

/// 解析 HTTP 日期(RFC 1123)为 Unix 毫秒。
#[cfg(feature = "storage-webdav")]
fn parse_http_date_to_millis(s: &str) -> Option<i64> {
    // 简单解析:用 chrono 解析 RFC 2822
    use chrono::DateTime;
    use chrono::NaiveDateTime;
    // 尝试 RFC 2822
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.timestamp_millis());
    }
    // 尝试 RFC 3339
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // 尝试常见格式 "Mon, 01 Jan 2024 00:00:00 GMT"
    let _ = NaiveDateTime::parse_from_str;
    None
}

#[async_trait]
impl StorageBackend for WebDavBackend {
    fn kind(&self) -> &'static str {
        "webdav"
    }

    async fn read(&self, path: &str) -> StorageResult<Vec<u8>> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(self.client.get(&url))
                .send()
                .await?;
            let resp = Self::check_status(resp, path).await?;
            let bytes = resp.bytes().await?;
            Ok(bytes.to_vec())
        }
    }

    async fn write(&self, path: &str, bytes: &[u8]) -> StorageResult<()> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = (path, bytes);
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(self.client.put(&url))
                .body(bytes.to_vec())
                .send()
                .await?;
            Self::check_status(resp, path).await?;
            Ok(())
        }
    }

    async fn delete(&self, path: &str) -> StorageResult<()> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(self.client.delete(&url))
                .send()
                .await?;
            // 幂等:404 视为 Ok
            match resp.status() {
                s if s.is_success() => Ok(()),
                StatusCode::NOT_FOUND => Ok(()),
                s => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(StorageError::Http {
                        status: s.as_u16(),
                        body,
                    })
                }
            }
        }
    }

    async fn exists(&self, path: &str) -> StorageResult<bool> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(self.client.head(&url))
                .send()
                .await?;
            match resp.status() {
                StatusCode::OK => Ok(true),
                StatusCode::NOT_FOUND => Ok(false),
                s => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(StorageError::Http {
                        status: s.as_u16(),
                        body,
                    })
                }
            }
        }
    }

    async fn metadata(&self, path: &str) -> StorageResult<FileMetadata> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            // PROPFIND with Depth: 0
            let resp = self
                .with_auth(self.client.request(Method::from_bytes(b"PROPFIND").unwrap(), &url))
                .header("Depth", "0")
                .header(
                    "Content-Type",
                    "application/xml; charset=utf-8",
                )
                .body(r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:getcontentlength/><D:getlastmodified/><D:resourcetype/><D:getetag/></D:prop></D:propfind>"#)
                .send()
                .await?;
            let resp = Self::check_status(resp, path).await?;
            let body = resp.text().await?;
            let entries = Self::parse_propfind_response(&body, path);
            // 取匹配 path 的条目,或第一个条目
            entries
                .iter()
                .find(|e| e.path == path)
                .cloned()
                .or_else(|| entries.first().cloned())
                .ok_or_else(|| {
                    StorageError::Unavailable(format!(
                        "PROPFIND returned no entries for {path}"
                    ))
                })
        }
    }

    async fn read_stream(
        &self,
        path: &str,
    ) -> StorageResult<Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(self.client.get(&url))
                .send()
                .await?;
            let resp = Self::check_status(resp, path).await?;
            // 将 reqwest 的 bytes_stream 转换为 StorageResult<Bytes> 流
            let stream = resp
                .bytes_stream()
                .map(|result| result.map_err(|e| StorageError::Request { source: e }));
            Ok(Box::new(stream))
        }
    }

    async fn write_stream(
        &self,
        path: &str,
        stream: Box<dyn Stream<Item = StorageResult<Bytes>> + Send + Unpin>,
        _expected_size: Option<u64>,
    ) -> StorageResult<()> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = (path, stream, _expected_size);
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            // 将 StorageResult<Bytes> 流转换为 reqwest::Body
            let byte_stream = stream.map(|result| result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string())));
            let body = reqwest::Body::wrap_stream(byte_stream);
            let resp = self
                .with_auth(self.client.put(&url))
                .body(body)
                .send()
                .await?;
            Self::check_status(resp, path).await?;
            Ok(())
        }
    }

    async fn create_dir(&self, path: &str) -> StorageResult<()> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(
                    self.client
                        .request(Method::from_bytes(b"MKCOL").unwrap(), &url),
                )
                .send()
                .await?;
            // 幂等:405 Method Not Allowed 通常表示 collection 已存在
            match resp.status() {
                s if s.is_success() => Ok(()),
                StatusCode::METHOD_NOT_ALLOWED => Ok(()),
                StatusCode::CONFLICT => {
                    // 父目录不存在,递归创建
                    if let Some(parent) = path.rfind('/') {
                        let parent_path = &path[..parent];
                        self.create_dir(parent_path).await?;
                        // 重试
                        let resp = self
                            .with_auth(
                                self.client
                                    .request(Method::from_bytes(b"MKCOL").unwrap(), &url),
                            )
                            .send()
                            .await?;
                        Self::check_status(resp, path).await?;
                    }
                    Ok(())
                }
                s => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(StorageError::Http {
                        status: s.as_u16(),
                        body,
                    })
                }
            }
        }
    }

    async fn remove_dir(&self, path: &str) -> StorageResult<()> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = path;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(path);
            let resp = self
                .with_auth(self.client.delete(&url))
                .send()
                .await?;
            // 幂等:404 视为 Ok
            match resp.status() {
                s if s.is_success() => Ok(()),
                StatusCode::NOT_FOUND => Ok(()),
                s => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(StorageError::Http {
                        status: s.as_u16(),
                        body,
                    })
                }
            }
        }
    }

    async fn list(&self, prefix: &str) -> StorageResult<Vec<FileMetadata>> {
        #[cfg(not(feature = "storage-webdav"))]
        {
            let _ = prefix;
            return Err(Self::unavailable());
        }
        #[cfg(feature = "storage-webdav")]
        {
            let url = self.url(prefix);
            // PROPFIND with Depth: 1
            let resp = self
                .with_auth(self.client.request(Method::from_bytes(b"PROPFIND").unwrap(), &url))
                .header("Depth", "1")
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:getcontentlength/><D:getlastmodified/><D:resourcetype/><D:getetag/></D:prop></D:propfind>"#)
                .send()
                .await?;
            let resp = Self::check_status(resp, prefix).await?;
            let body = resp.text().await?;
            let mut entries = Self::parse_propfind_response(&body, prefix);
            // 过滤掉 prefix 自身(PROPFIND Depth:1 会返回 collection 自身)
            entries.retain(|e| !e.path.is_empty() && e.path != prefix);
            Ok(entries)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试:WebDavBackend stub(feature 关闭时返回 Err)。
    #[tokio::test]
    async fn webdav_stub_returns_err() {
        // 使用非 loopback 地址避免 SSRF 校验拒绝 (M7b #94)。
        let backend = WebDavBackend::new("https://example.com/dav/", None, None)
            .expect("construct stub backend");
        assert_eq!(backend.kind(), "webdav");

        let result = backend.read("foo.txt").await;
        assert!(result.is_err());
        match result {
            Err(StorageError::Unavailable(_)) => {}
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    /// 测试:URL 构造。
    #[test]
    fn url_construction() {
        let backend = WebDavBackend::new("https://example.com/dav/", None, None).unwrap();
        assert_eq!(backend.url("foo.txt"), "https://example.com/dav/foo.txt");
        assert_eq!(backend.url("/bar/baz"), "https://example.com/dav/bar/baz");
    }

    /// 测试:WebDAV PUT/GET/DELETE 往返(用内置 mock HTTP 服务器)。
    ///
    /// 启动一个简单的 HTTP/1.1 服务器,处理 PUT/GET/DELETE 三个方法,
    /// 验证 WebDavBackend 的往返语义。仅 storage-webdav feature 开启时运行。
    #[cfg(feature = "storage-webdav")]
    #[tokio::test]
    async fn webdav_put_get_delete_round_trip() {
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::sync::Mutex;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let base_url = format!("http://127.0.0.1:{port}/");
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let store_clone = store.clone();

        // 启动 mock 服务器
        let server_handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let store = store_clone.clone();
                tokio::spawn(async move {
                    handle_webdav_request(&mut socket, &store).await;
                });
            }
        });

        // 用 WebDavBackend 做往返测试
        let backend = WebDavBackend::new(&base_url, None, None).expect("construct backend");

        // PUT
        backend
            .write("test.txt", b"hello webdav")
            .await
            .expect("PUT");

        // 验证服务器侧存储了内容
        {
            let store_guard = store.lock().await;
            assert_eq!(
                store_guard.get("test.txt"),
                Some(&b"hello webdav".to_vec())
            );
        }

        // GET
        let read = backend.read("test.txt").await.expect("GET");
        assert_eq!(read, b"hello webdav");

        // DELETE
        backend.delete("test.txt").await.expect("DELETE");

        // 验证已删除
        {
            let store_guard = store.lock().await;
            assert!(!store_guard.contains_key("test.txt"));
        }

        // DELETE 幂等(已删除再删不报错)
        backend.delete("test.txt").await.expect("DELETE idempotent");

        server_handle.abort();
    }

    /// 处理单个 WebDAV 请求(PUT/GET/DELETE/HEAD)。
    #[cfg(feature = "storage-webdav")]
    async fn handle_webdav_request(
        socket: &mut tokio::net::TcpStream,
        store: &Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // 读取请求行 + headers
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let mut lines = request.lines();
        let request_line = lines.next().unwrap_or("");
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await;
            return;
        }
        let method = parts[0];
        let path = parts[1].trim_start_matches('/');

        // 解析 Content-Length(可能有 body)
        let mut content_length = 0usize;
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some(val) = line.strip_prefix("Content-Length:").or_else(|| line.strip_prefix("content-length:")) {
                content_length = val.trim().parse().unwrap_or(0);
            }
        }

        // 读取剩余 body(如果已经在 buf 中部分读取)
        let body_start = request.find("\r\n\r\n").map(|p| p + 4).unwrap_or(n);
        let mut body = buf[body_start..n].to_vec();
        if body.len() < content_length {
            let remaining = content_length - body.len();
            let mut rest = vec![0u8; remaining];
            let _ = socket.read_exact(&mut rest).await;
            body.extend_from_slice(&rest);
        }

        match method {
            "PUT" => {
                let mut store_guard = store.lock().await;
                store_guard.insert(path.to_string(), body.clone());
                let response = "HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
            }
            "GET" => {
                let store_guard = store.lock().await;
                if let Some(data) = store_guard.get(path) {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                        data.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(data).await;
                } else {
                    let _ = socket
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                        .await;
                }
            }
            "DELETE" => {
                let mut store_guard = store.lock().await;
                store_guard.remove(path);
                let _ = socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
            "HEAD" => {
                let store_guard = store.lock().await;
                if store_guard.contains_key(path) {
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                        .await;
                } else {
                    let _ = socket
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                        .await;
                }
            }
            _ => {
                let _ = socket
                    .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        }
    }
}
