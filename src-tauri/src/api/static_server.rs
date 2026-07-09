//! Web frontend static file server.
//!
//! 对标: OpenClaw Web Admin、Hermes Dashboard。
//!
//! 在无头模式下,REST API 服务器额外提供前端静态文件。
//! 前端通过 `pnpm build` → `dist/` 目录,本模块负责:
//! 1. 读取 dist/ 目录(或内嵌的压缩前端资源)
//! 2. 提供 `/` → index.html
//! 3. 提供 `/assets/*` → 静态资源
//! 4. SPA fallback: 所有非 `/api/*` 路径 → index.html
//!
//! 集成方式:在 [`crate::api::rest::RestApiServer`] 的路由表中
//! 调用 [`WebStaticServer::try_serve`] 处理非 API 路径。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bytes::Bytes;
// T-D-B-12: `BodyExt` trait provides `.boxed()` used below to coerce
// `Full<Bytes>` into the erased `BoxBody` return type.  This import was
// missing (PRE-EXISTING bug masked by `rest-api` not being in CI),
// causing `no method named 'boxed' found` when T-D-B-12 first compiled
// the module with `--features rest-api`.
use http_body_util::BodyExt;
use http_body_util::Full;
use std::convert::Infallible;
use tracing::{info, warn};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

/// Web frontend static file server.
///
/// Serves files from a `dist/` directory produced by the frontend build.
/// Falls back to `index.html` for unknown non-API paths (SPA routing).
pub struct WebStaticServer {
    dist_path: PathBuf,
    /// Whether the dist directory was found at construction time.
    enabled: bool,
}

impl WebStaticServer {
    /// Creates a new static server pointing at the given dist path.
    ///
    /// If the path doesn't exist, the server is created in a disabled
    /// state — [`try_serve`] will return `None` for every request,
    /// allowing the caller to fall through to the API router.
    pub fn new(dist_path: PathBuf) -> Self {
        let enabled = dist_path.is_dir();
        if !enabled {
            warn!(
                target: "nebula.web",
                path = %dist_path.display(),
                "dist directory not found — Web UI disabled"
            );
        } else {
            info!(
                target: "nebula.web",
                path = %dist_path.display(),
                "Web UI static server enabled"
            );
        }
        Self { dist_path, enabled }
    }

    /// Returns `true` if the dist directory exists and static serving
    /// is active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Attempts to serve a request as a static file.
    ///
    /// Returns `Ok(Some(response))` if the path matched a static file
    /// (or fell back to index.html for SPA routing).  Returns
    /// `Ok(None)` if the path is an API route or the server is
    /// disabled, allowing the caller to fall through to the API
    /// router.
    pub async fn try_serve(
        &self,
        method: &str,
        path: &str,
    ) -> Result<Option<hyper::Response<BoxBody>>> {
        if !self.enabled || method != "GET" {
            return Ok(None);
        }

        // Never intercept API routes.
        if path.starts_with("/api/") {
            return Ok(None);
        }

        // Normalize the request path into a relative file path.
        // Strip leading `/` and prevent path traversal.
        let rel = path.trim_start_matches('/');
        if rel.is_empty() {
            return self.serve_index().await.map(Some);
        }

        // Reject path traversal attempts.
        if rel.contains("..") || rel.contains('\0') {
            return Ok(Some(self.text_response(400, "Bad Request")));
        }

        let candidate = self.dist_path.join(rel);

        // If the candidate is a file, serve it.
        if candidate.is_file() {
            return self.serve_file(&candidate).await.map(Some);
        }

        // SPA fallback: serve index.html for any non-file, non-API path.
        // This allows client-side routing (e.g. /settings, /memory) to work.
        self.serve_index().await.map(Some)
    }

    /// Serves the `index.html` entry point.
    async fn serve_index(&self) -> Result<hyper::Response<BoxBody>> {
        let index = self.dist_path.join("index.html");
        self.serve_file(&index).await
    }

    /// Serves a single file with the correct Content-Type.
    async fn serve_file(&self, path: &Path) -> Result<hyper::Response<BoxBody>> {
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("failed to read static file: {}", path.display()))?;
        let mime = mime_for(path);
        let body = Full::new(Bytes::from(bytes)).boxed();
        let resp = hyper::Response::builder()
            .status(200)
            .header("content-type", mime)
            .header("cache-control", cache_control_for(path))
            .body(body)
            .expect("must succeed");
        Ok(resp)
    }

    /// Builds a plain-text response.
    fn text_response(&self, status: u16, body: &str) -> hyper::Response<BoxBody> {
        let body = Full::new(Bytes::from(body.to_string())).boxed();
        hyper::Response::builder()
            .status(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(body)
            .expect("must succeed")
    }
}

/// Returns the MIME type for a file based on its extension.
fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("map") => "application/json; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Returns the Cache-Control header value for a static asset.
///
/// Hashed assets (in `/assets/`) get a long max-age since their
/// filename includes a content hash.  Other files (index.html, etc.)
/// get a short cache with must-revalidate to ensure users always get
/// the latest entry point.
fn cache_control_for(path: &Path) -> &'static str {
    if path.starts_with("assets") || path.to_string_lossy().contains("/assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache, must-revalidate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_for_html() {
        assert_eq!(
            mime_for(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
    }

    #[test]
    fn mime_for_js() {
        assert_eq!(
            mime_for(Path::new("app.js")),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn mime_for_unknown() {
        assert_eq!(mime_for(Path::new("file.xyz")), "application/octet-stream");
    }

    #[test]
    fn cache_control_for_assets_is_immutable() {
        let ctrl = cache_control_for(Path::new("assets/app-abc123.js"));
        assert!(ctrl.contains("immutable"));
    }

    #[test]
    fn cache_control_for_index_is_no_cache() {
        let ctrl = cache_control_for(Path::new("index.html"));
        assert!(ctrl.contains("no-cache"));
    }

    #[tokio::test]
    async fn disabled_server_returns_none() {
        let server = WebStaticServer::new(PathBuf::from("/nonexistent/dist"));
        assert!(!server.is_enabled());
        let result = server.try_serve("GET", "/").await.expect("get should succeed");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn api_paths_are_not_intercepted() {
        // Use the real dist path if it exists, otherwise the test just
        // verifies the API-path short-circuit regardless of dist state.
        let temp = std::env::temp_dir().join("nebula_test_dist");
        let _ = tokio::fs::create_dir_all(&temp).await;
        let _ = tokio::fs::write(temp.join("index.html"), "<html></html>").await;
        let server = WebStaticServer::new(temp);
        let result = server.try_serve("GET", "/api/health").await.expect("get should succeed");
        assert!(result.is_none(), "API paths should not be intercepted");
    }
}
