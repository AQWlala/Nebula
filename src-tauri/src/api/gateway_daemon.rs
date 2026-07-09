//! T-E-S-60 Gateway 守护进程模块。
//!
//! 提供 HTTP 网关代理能力,支持:
//! - 路由转发(前缀 + 通配符匹配,如 `/api/*`)
//! - 认证(Bearer token)
//! - 限流(令牌桶,每分钟补充令牌)
//! - 热重载配置(`reload_config`)
//! - 运行统计(`stats`)
//! - 优雅关闭(`shutdown`)
//!
//! 对标 OpenClaw Gateway 守护进程,作为 Nebula 的 HTTP 入口代理。
//!
//! 设计约束:本模块不依赖 hyper(避免引入 feature gate),
//! 直接使用 `tokio::net::TcpListener` + 极简 HTTP/1.1 解析;
//! 上游转发使用项目已有的 `reqwest`。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{info, warn};

// =========================================================================
// 配置
// =========================================================================

/// Gateway 守护进程配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// 监听地址,默认 "127.0.0.1:7878"。
    pub listen_addr: String,
    /// 最大并发连接数,默认 100。
    pub max_connections: usize,
    /// 单个上游请求超时(秒),默认 30。
    pub request_timeout_secs: u64,
    /// 是否启用 Bearer token 认证。
    pub enable_auth: bool,
    /// 认证 token(配合 `enable_auth`),期望请求头 `Authorization: Bearer <token>`。
    pub auth_token: Option<String>,
    /// 每分钟令牌补充速率;`None` 表示不限流。
    pub rate_limit_per_min: Option<u32>,
    /// 路由表(按顺序匹配,首个命中胜出)。
    pub routes: Vec<GatewayRoute>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:7878".to_string(),
            max_connections: 100,
            request_timeout_secs: 30,
            enable_auth: false,
            auth_token: None,
            rate_limit_per_min: None,
            routes: Vec::new(),
        }
    }
}

impl GatewayConfig {
    /// 创建一个链式 builder。
    pub fn builder() -> GatewayConfigBuilder {
        GatewayConfigBuilder {
            inner: GatewayConfig::default(),
        }
    }
}

/// `GatewayConfig` 的链式 builder。
pub struct GatewayConfigBuilder {
    inner: GatewayConfig,
}

impl GatewayConfigBuilder {
    pub fn listen_addr(mut self, v: impl Into<String>) -> Self {
        self.inner.listen_addr = v.into();
        self
    }
    pub fn max_connections(mut self, v: usize) -> Self {
        self.inner.max_connections = v;
        self
    }
    pub fn request_timeout_secs(mut self, v: u64) -> Self {
        self.inner.request_timeout_secs = v;
        self
    }
    pub fn enable_auth(mut self, v: bool) -> Self {
        self.inner.enable_auth = v;
        self
    }
    pub fn auth_token(mut self, v: impl Into<String>) -> Self {
        self.inner.auth_token = Some(v.into());
        self
    }
    pub fn rate_limit_per_min(mut self, v: u32) -> Self {
        self.inner.rate_limit_per_min = Some(v);
        self
    }
    pub fn routes(mut self, v: Vec<GatewayRoute>) -> Self {
        self.inner.routes = v;
        self
    }
    /// 追加一条路由(便捷方法)。
    pub fn route(mut self, r: GatewayRoute) -> Self {
        self.inner.routes.push(r);
        self
    }
    pub fn build(self) -> GatewayConfig {
        self.inner
    }
}

/// 单条网关路由。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayRoute {
    /// 路径匹配模式,支持 `/api/*` 通配符(`*` 仅在末尾生效)。
    pub path_pattern: String,
    /// 上游服务基础 URL,例如 `http://localhost:3000`。
    pub upstream_url: String,
    /// 允许的 HTTP 方法;空列表表示允许全部方法。
    pub methods: Vec<String>,
    /// 是否在转发前剥离匹配前缀。
    pub strip_prefix: bool,
    /// 转发时注入/覆盖的请求头。
    pub inject_headers: HashMap<String, String>,
}

impl GatewayRoute {
    /// 判断 `path` 是否命中本路由。
    ///
    /// 匹配规则:
    /// - 模式以 `/*` 结尾:前缀(去 `/*` 后的部分)及其子路径全部命中。
    ///   例如 `/api/*` 命中 `/api`、`/api/`、`/api/users`、`/api/v1/x`。
    /// - 否则:精确匹配。
    /// - `/*` 命中一切。
    pub fn matches(&self, path: &str) -> bool {
        if self.path_pattern.ends_with("/*") {
            let prefix = &self.path_pattern[..self.path_pattern.len() - 2];
            if prefix.is_empty() {
                // "/*" 命中所有路径
                return true;
            }
            path == prefix || path.starts_with(&format!("{}/", prefix))
        } else {
            path == self.path_pattern
        }
    }

    /// 根据 `strip_prefix` 计算转发给上游的路径。
    pub fn build_upstream_path(&self, request_path: &str) -> String {
        if self.strip_prefix {
            if self.path_pattern.ends_with("/*") {
                let prefix = &self.path_pattern[..self.path_pattern.len() - 2];
                if !prefix.is_empty() {
                    let with_slash = format!("{}/", prefix);
                    if let Some(rest) = request_path.strip_prefix(&with_slash) {
                        return format!("/{}", rest);
                    }
                    if request_path == prefix {
                        return "/".to_string();
                    }
                }
                // prefix 为空("/*")时无前缀可剥离
                return request_path.to_string();
            } else if request_path == self.path_pattern {
                return "/".to_string();
            }
        }
        request_path.to_string()
    }

    /// 合并请求头与注入头(注入头覆盖同名请求头)。
    pub fn build_forward_headers(
        &self,
        req_headers: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut out = req_headers.clone();
        for (k, v) in &self.inject_headers {
            out.insert(k.clone(), v.clone());
        }
        out
    }
}

// =========================================================================
// 运行统计
// =========================================================================

/// Gateway 运行统计快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayStats {
    pub total_requests: u64,
    pub active_connections: u64,
    pub requests_per_route: HashMap<String, u64>,
    pub last_request_at: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
}

// =========================================================================
// 请求 / 响应结构
// =========================================================================

/// 极简 HTTP 请求结构(已解析)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

/// 极简 HTTP 响应结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body,
        }
    }

    /// 构造 JSON 响应。
    pub fn json(status: u16, value: serde_json::Value) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_default();
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn not_found() -> Self {
        Self::json(404, serde_json::json!({ "error": "not found" }))
    }

    pub fn method_not_allowed() -> Self {
        Self::json(405, serde_json::json!({ "error": "method not allowed" }))
    }

    pub fn unauthorized() -> Self {
        Self::json(401, serde_json::json!({ "error": "unauthorized" }))
    }

    pub fn too_many_requests() -> Self {
        Self::json(429, serde_json::json!({ "error": "rate limit exceeded" }))
    }

    pub fn bad_gateway(msg: &str) -> Self {
        Self::json(
            502,
            serde_json::json!({ "error": format!("bad gateway: {}", msg) }),
        )
    }
}

// =========================================================================
// 令牌桶限流器
// =========================================================================

/// 令牌桶限流器。
///
/// - `capacity`:桶容量(初始满载)。
/// - `refill_per_min`:每分钟补充的令牌数。
/// - 每次请求消耗 1 个令牌;不足时 `try_acquire` 返回 `false`。
pub struct RateLimiter {
    capacity: u32,
    refill_per_min: u32,
    state: Mutex<LimiterState>,
}

struct LimiterState {
    tokens: f64,
    last_refill: std::time::Instant,
}

impl RateLimiter {
    pub fn new(capacity: u32, refill_per_min: u32) -> Self {
        Self {
            capacity,
            refill_per_min,
            state: Mutex::new(LimiterState {
                tokens: capacity as f64,
                last_refill: std::time::Instant::now(),
            }),
        }
    }

    /// 尝试获取一个令牌;成功返回 `true`,桶空返回 `false`。
    pub fn try_acquire(&self) -> bool {
        let mut state = self.state.lock();
        self.refill_locked(&mut state);
        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 按经过的真实时间补充令牌(上限为容量)。
    fn refill_locked(&self, state: &mut LimiterState) {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(state.last_refill);
        let refill = elapsed.as_secs_f64() * (self.refill_per_min as f64) / 60.0;
        if refill > 0.0 {
            state.tokens = (state.tokens + refill).min(self.capacity as f64);
            state.last_refill = now;
        }
    }
}

// =========================================================================
// Gateway 守护进程
// =========================================================================

/// Gateway 守护进程。
///
/// 持有可热重载的配置、令牌桶限流器、运行统计与关闭标志。
/// 通过 `Arc<Self>` 共享,`start` 在独立 tokio 任务中运行 accept 循环。
pub struct GatewayDaemon {
    config: Arc<RwLock<GatewayConfig>>,
    /// 限流器;`None` 表示不限流。热重载时按新配置重建。
    rate_limiter: Arc<RwLock<Option<RateLimiter>>>,
    /// 复用的上游 HTTP 客户端(关闭重定向,透传 3xx)。
    http_client: reqwest::Client,
    stats_total: AtomicU64,
    stats_active: AtomicU64,
    stats_per_route: RwLock<HashMap<String, u64>>,
    stats_last_request: RwLock<Option<DateTime<Utc>>>,
    started_at: DateTime<Utc>,
    shutdown_flag: Arc<AtomicBool>,
}

impl GatewayDaemon {
    /// 构造守护进程(不启动监听)。
    pub fn new(config: GatewayConfig) -> Self {
        let rate_limiter = config
            .rate_limit_per_min
            .map(|rpm| RateLimiter::new(rpm, rpm));
        // 关闭重定向,让网关透传上游的 3xx 响应。
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build upstream http client");
        Self {
            config: Arc::new(RwLock::new(config)),
            rate_limiter: Arc::new(RwLock::new(rate_limiter)),
            http_client,
            stats_total: AtomicU64::new(0),
            stats_active: AtomicU64::new(0),
            stats_per_route: RwLock::new(HashMap::new()),
            stats_last_request: RwLock::new(None),
            started_at: Utc::now(),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 启动 HTTP 服务器,返回任务句柄。
    ///
    /// accept 循环每 500ms 检查一次关闭标志,实现"准优雅"关闭。
    pub fn start(self: Arc<Self>) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            let listen_addr = self.config.read().listen_addr.clone();
            let listener = TcpListener::bind(&listen_addr)
                .await
                .with_context(|| format!("failed to bind gateway listener on {}", listen_addr))?;
            info!(target: "nebula.gateway", addr = %listen_addr, "gateway daemon listening");

            loop {
                // 周期性检查关闭标志
                if self.shutdown_flag.load(Ordering::SeqCst) {
                    info!(target: "nebula.gateway", "gateway daemon shutting down");
                    break;
                }

                let accept = timeout(Duration::from_millis(500), listener.accept()).await;
                let (stream, peer) = match accept {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        warn!(target: "nebula.gateway", error = %e, "accept failed");
                        continue;
                    }
                    Err(_) => continue, // 超时,回到循环顶部检查关闭标志
                };

                // 最大连接数保护
                let active = self.stats_active.load(Ordering::SeqCst);
                let max = self.config.read().max_connections;
                if active >= max as u64 {
                    warn!(target: "nebula.gateway", peer = %peer, "max_connections exceeded, dropping connection");
                    continue;
                }

                let this = self.clone();
                tokio::spawn(this.handle_connection(stream));
            }
            Ok(())
        })
    }

    /// 处理单个请求:路由匹配 → 方法检查 → 认证 → 限流 → 代理转发 → 响应。
    pub async fn handle_request(&self, req: HttpRequest) -> Result<HttpResponse> {
        self.record_total_request();

        // 1. 路由匹配(按顺序,首个命中)
        let route = match self.match_route(&req.path) {
            Some(r) => r,
            None => return Ok(HttpResponse::not_found()),
        };
        self.record_route_request(&route.path_pattern);

        // 2. 方法检查(空列表表示放行全部)
        if !route.methods.is_empty()
            && !route
                .methods
                .iter()
                .any(|m| m.eq_ignore_ascii_case(&req.method))
        {
            return Ok(HttpResponse::method_not_allowed());
        }

        // 3. 认证检查(读出设置后立即释放锁,避免跨 await 持锁)
        let (enable_auth, auth_token) = {
            let cfg = self.config.read();
            (cfg.enable_auth, cfg.auth_token.clone())
        };
        if enable_auth {
            if let Some(token) = &auth_token {
                if !check_auth_header(&req, token) {
                    return Ok(HttpResponse::unauthorized());
                }
            }
        }

        // 4. 限流检查
        {
            let limiter_guard = self.rate_limiter.read();
            if let Some(limiter) = limiter_guard.as_ref() {
                if !limiter.try_acquire() {
                    return Ok(HttpResponse::too_many_requests());
                }
            }
        }

        // 5. 构造上游请求
        let upstream_path = route.build_upstream_path(&req.path);
        let upstream_url = format!("{}{}", route.upstream_url, upstream_path);
        let forward_headers = route.build_forward_headers(&req.headers);
        let timeout_secs = self.config.read().request_timeout_secs;

        // 6. 代理转发(失败时返回 502,而非向上层抛 Err)
        match self
            .proxy_forward(
                &upstream_url,
                &req.method,
                forward_headers,
                &req.body,
                timeout_secs,
            )
            .await
        {
            Ok(resp) => Ok(resp),
            Err(e) => {
                warn!(target: "nebula.gateway", url = %upstream_url, error = %e, "upstream forward failed");
                Ok(HttpResponse::bad_gateway(&e.to_string()))
            }
        }
    }

    /// 路由匹配;按 `routes` 顺序返回首个命中路由。
    pub fn match_route(&self, path: &str) -> Option<GatewayRoute> {
        let cfg = self.config.read();
        cfg.routes.iter().find(|r| r.matches(path)).cloned()
    }

    /// 校验请求的 `Authorization: Bearer <token>` 头。
    pub fn check_auth(&self, req: &HttpRequest, expected_token: &str) -> bool {
        check_auth_header(req, expected_token)
    }

    /// 返回当前统计快照。
    pub fn stats(&self) -> GatewayStats {
        let requests_per_route = self.stats_per_route.read().clone();
        let last_request_at = *self.stats_last_request.read();
        GatewayStats {
            total_requests: self.stats_total.load(Ordering::SeqCst),
            active_connections: self.stats_active.load(Ordering::SeqCst),
            requests_per_route,
            last_request_at,
            started_at: self.started_at,
        }
    }

    /// 请求优雅关闭;accept 循环将在下一个轮询周期退出。
    pub fn shutdown(&self) -> Result<()> {
        self.shutdown_flag.store(true, Ordering::SeqCst);
        info!(target: "nebula.gateway", "shutdown requested");
        Ok(())
    }

    /// 是否已请求关闭。
    pub fn is_shutdown(&self) -> bool {
        self.shutdown_flag.load(Ordering::SeqCst)
    }

    /// 热重载配置(路由、认证、限流、超时等)。
    ///
    /// 限流器按新的 `rate_limit_per_min` 重建(令牌计数归零)。
    pub fn reload_config(&self, config: GatewayConfig) {
        let new_limiter = config
            .rate_limit_per_min
            .map(|rpm| RateLimiter::new(rpm, rpm));
        *self.rate_limiter.write() = new_limiter;
        *self.config.write() = config;
        info!(target: "nebula.gateway", "config reloaded");
    }

    // --- 内部辅助 ---

    fn record_total_request(&self) {
        self.stats_total.fetch_add(1, Ordering::SeqCst);
        *self.stats_last_request.write() = Some(Utc::now());
    }

    fn record_route_request(&self, pattern: &str) {
        *self
            .stats_per_route
            .write()
            .entry(pattern.to_string())
            .or_insert(0) += 1;
    }

    /// 转发请求到上游并返回响应。
    async fn proxy_forward(
        &self,
        url: &str,
        method: &str,
        headers: HashMap<String, String>,
        body: &[u8],
        timeout_secs: u64,
    ) -> Result<HttpResponse> {
        let m = reqwest::Method::from_bytes(method.as_bytes())
            .with_context(|| format!("invalid http method: {}", method))?;
        let mut builder = self
            .http_client
            .request(m, url)
            .timeout(Duration::from_secs(timeout_secs))
            .body(body.to_vec());
        for (k, v) in &headers {
            builder = builder.header(k, v);
        }
        let resp = builder
            .send()
            .await
            .with_context(|| format!("upstream request failed: {}", url))?;
        let status = resp.status().as_u16();
        let mut resp_headers = HashMap::new();
        for (k, v) in resp.headers().iter() {
            if let Ok(s) = v.to_str() {
                resp_headers.insert(k.as_str().to_string(), s.to_string());
            }
        }
        let resp_body = resp
            .bytes()
            .await
            .context("failed to read upstream response body")?
            .to_vec();
        Ok(HttpResponse {
            status,
            headers: resp_headers,
            body: resp_body,
        })
    }

    /// 处理单条 TCP 连接:解析请求 → handle_request → 写回响应。
    async fn handle_connection(self: Arc<Self>, stream: TcpStream) {
        self.stats_active.fetch_add(1, Ordering::SeqCst);
        let _guard = ActiveGuard {
            counter: &self.stats_active,
        };
        if let Err(e) = self.handle_connection_inner(stream).await {
            warn!(target: "nebula.gateway", error = %e, "connection handler error");
        }
    }

    async fn handle_connection_inner(&self, stream: TcpStream) -> Result<()> {
        let mut reader = BufReader::new(stream);
        let req = parse_http_request(&mut reader).await?;
        let resp = self.handle_request(req).await?;
        let out = serialize_response(&resp);
        reader.get_mut().write_all(&out).await?;
        Ok(())
    }
}

/// 活跃连接计数守卫:drop 时递减。
struct ActiveGuard<'a> {
    counter: &'a AtomicU64,
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

// =========================================================================
// HTTP/1.1 极简解析 / 序列化
// =========================================================================

/// 从 `reader` 解析一个 HTTP/1.1 请求(仅支持 Content-Length 定长 body)。
async fn parse_http_request(reader: &mut BufReader<TcpStream>) -> Result<HttpRequest> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("failed to read request line")?;
    let line = line.trim_end();
    let mut parts = line.splitn(3, ' ');
    let method = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("invalid request line: missing method"))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("invalid request line: missing path"))?
        .to_string();
    // 第 3 段为 HTTP 版本,忽略

    let mut headers = HashMap::new();
    loop {
        let mut hline = String::new();
        let n = reader
            .read_line(&mut hline)
            .await
            .context("failed to read header line")?;
        if n == 0 {
            break;
        }
        let hline = hline.trim_end();
        if hline.is_empty() {
            break;
        }
        if let Some((k, v)) = hline.split_once(':') {
            headers.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    let body = if let Some(clen) = get_header(&headers, "content-length") {
        let n: usize = clen.parse().unwrap_or(0);
        if n == 0 {
            Vec::new()
        } else {
            let mut buf = vec![0u8; n];
            reader
                .read_exact(&mut buf)
                .await
                .context("failed to read request body")?;
            buf
        }
    } else {
        Vec::new()
    };

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

/// 序列化 `HttpResponse` 为 HTTP/1.1 字节流。
fn serialize_response(resp: &HttpResponse) -> Vec<u8> {
    let reason = status_reason(resp.status);
    let mut out = Vec::new();
    out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", resp.status, reason).as_bytes());
    for (k, v) in &resp.headers {
        // 避免与下方显式写入的 content-length 重复
        if k.eq_ignore_ascii_case("content-length") {
            continue;
        }
        out.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
    }
    out.extend_from_slice(format!("content-length: {}\r\n", resp.body.len()).as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&resp.body);
    out
}

/// 状态码 → 原因短语(覆盖常用码,其余回落 "OK")。
fn status_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "OK",
    }
}

/// 大小写不敏感地获取请求头值。
fn get_header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a String> {
    headers.iter().find_map(|(k, v)| {
        if k.eq_ignore_ascii_case(name) {
            Some(v)
        } else {
            None
        }
    })
}

/// 校验 `Authorization: Bearer <expected_token>`。
fn check_auth_header(req: &HttpRequest, expected_token: &str) -> bool {
    match get_header(&req.headers, "authorization") {
        Some(val) => val == &format!("Bearer {}", expected_token),
        None => false,
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn route(pattern: &str, upstream: &str) -> GatewayRoute {
        GatewayRoute {
            path_pattern: pattern.to_string(),
            upstream_url: upstream.to_string(),
            methods: vec![],
            strip_prefix: false,
            inject_headers: HashMap::new(),
        }
    }

    // --- 配置默认值 / builder ---

    #[test]
    fn config_default_values() {
        let c = GatewayConfig::default();
        assert_eq!(c.listen_addr, "127.0.0.1:7878");
        assert_eq!(c.max_connections, 100);
        assert_eq!(c.request_timeout_secs, 30);
        assert!(!c.enable_auth);
        assert!(c.auth_token.is_none());
        assert!(c.rate_limit_per_min.is_none());
        assert!(c.routes.is_empty());
    }

    #[test]
    fn config_builder_sets_fields() {
        let c = GatewayConfig::builder()
            .listen_addr("0.0.0.0:8080")
            .max_connections(200)
            .request_timeout_secs(60)
            .enable_auth(true)
            .auth_token("tok")
            .rate_limit_per_min(1000)
            .route(route("/x", "http://x"))
            .build();
        assert_eq!(c.listen_addr, "0.0.0.0:8080");
        assert_eq!(c.max_connections, 200);
        assert_eq!(c.request_timeout_secs, 60);
        assert!(c.enable_auth);
        assert_eq!(c.auth_token.as_deref(), Some("tok"));
        assert_eq!(c.rate_limit_per_min, Some(1000));
        assert_eq!(c.routes.len(), 1);
    }

    // --- 路由匹配 ---

    #[test]
    fn route_match_exact() {
        let r = route("/health", "http://h");
        assert!(r.matches("/health"));
        assert!(!r.matches("/healthz"));
        assert!(!r.matches("/health/x"));
    }

    #[test]
    fn route_match_wildcard() {
        let r = route("/api/*", "http://a");
        assert!(r.matches("/api"));
        assert!(r.matches("/api/"));
        assert!(r.matches("/api/users"));
        assert!(r.matches("/api/v1/items"));
        assert!(!r.matches("/web"));
        assert!(!r.matches("/apiv2")); // 不应误匹配
    }

    #[test]
    fn route_wildcard_root_matches_all() {
        let r = route("/*", "http://all");
        assert!(r.matches("/"));
        assert!(r.matches("/anything"));
        assert!(r.matches("/api/v1/x"));
    }

    #[test]
    fn route_wildcard_deep() {
        let r = route("/api/v1/*", "http://v1");
        assert!(r.matches("/api/v1/users"));
        assert!(r.matches("/api/v1"));
        assert!(!r.matches("/api/v2/users"));
        assert!(!r.matches("/api/users"));
    }

    // --- strip_prefix / header 注入 ---

    #[test]
    fn strip_prefix_removes_prefix() {
        let mut r = route("/api/*", "http://a");
        r.strip_prefix = true;
        assert_eq!(r.build_upstream_path("/api/users"), "/users");
        assert_eq!(r.build_upstream_path("/api/v1/items"), "/v1/items");
        assert_eq!(r.build_upstream_path("/api"), "/");
    }

    #[test]
    fn no_strip_prefix_keeps_path() {
        let r = route("/api/*", "http://a");
        assert_eq!(r.build_upstream_path("/api/users"), "/api/users");
    }

    #[test]
    fn header_injection_merges_and_overrides() {
        let mut r = route("/api/*", "http://a");
        let mut req_headers = HashMap::new();
        req_headers.insert("X-Request-Id".to_string(), "abc".to_string());
        let mut inject = HashMap::new();
        inject.insert("X-Gateway".to_string(), "nebula".to_string());
        inject.insert("X-Request-Id".to_string(), "overridden".to_string());
        r.inject_headers = inject;
        let merged = r.build_forward_headers(&req_headers);
        assert_eq!(merged.get("X-Gateway").unwrap(), "nebula");
        assert_eq!(merged.get("X-Request-Id").unwrap(), "overridden"); // 注入覆盖原值
    }

    // --- 令牌桶 ---

    #[test]
    fn rate_limiter_allows_capacity() {
        let limiter = RateLimiter::new(3, 60);
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
    }

    #[test]
    fn rate_limiter_blocks_when_empty() {
        let limiter = RateLimiter::new(2, 60);
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire()); // 桶空
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn rate_limiter_refills_over_time() {
        // 容量 1,每分钟 600000 令牌(10000/秒),50ms 内可补充大量令牌(上限 1)
        let limiter = RateLimiter::new(1, 600000);
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire()); // 耗尽
        std::thread::sleep(Duration::from_millis(50));
        assert!(limiter.try_acquire()); // 已补充
    }

    // --- 统计 ---

    #[test]
    fn stats_initial_state() {
        let daemon = GatewayDaemon::new(GatewayConfig::default());
        let stats = daemon.stats();
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.active_connections, 0);
        assert!(stats.last_request_at.is_none());
        assert!(stats.requests_per_route.is_empty());
    }

    #[tokio::test]
    async fn stats_increments_on_request() {
        let daemon = GatewayDaemon::new(GatewayConfig::default()); // 无路由 → 404
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/nothing".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = daemon.handle_request(req).await.unwrap();
        assert_eq!(resp.status, 404);
        let stats = daemon.stats();
        assert_eq!(stats.total_requests, 1);
        assert!(stats.last_request_at.is_some());
    }

    #[tokio::test]
    async fn stats_per_route_recorded() {
        let cfg = GatewayConfig::builder()
            .route(route("/api/*", "http://u"))
            .build();
        let daemon = GatewayDaemon::new(cfg);
        for _ in 0..3 {
            let req = HttpRequest {
                method: "GET".to_string(),
                path: "/api/x".to_string(),
                headers: HashMap::new(),
                body: Vec::new(),
            };
            // 上游不可达 → 502,但统计仍记录
            let _ = daemon.handle_request(req).await.unwrap();
        }
        let stats = daemon.stats();
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.requests_per_route.get("/api/*"), Some(&3));
    }

    // --- 认证 ---

    #[test]
    fn auth_check_helper() {
        let daemon = GatewayDaemon::new(GatewayConfig::default());
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer s3cr3t".to_string());
        let req_ok = HttpRequest {
            method: "GET".to_string(),
            path: "/".to_string(),
            headers,
            body: Vec::new(),
        };
        assert!(daemon.check_auth(&req_ok, "s3cr3t"));
        assert!(!daemon.check_auth(&req_ok, "wrong"));

        let req_no_auth = HttpRequest {
            method: "GET".to_string(),
            path: "/".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        assert!(!daemon.check_auth(&req_no_auth, "s3cr3t"));
    }

    #[tokio::test]
    async fn auth_missing_token_rejected() {
        let cfg = GatewayConfig::builder()
            .enable_auth(true)
            .auth_token("secret")
            .route(route("/api/*", "http://127.0.0.1:1"))
            .build();
        let daemon = GatewayDaemon::new(cfg);
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/api/x".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = daemon.handle_request(req).await.unwrap();
        assert_eq!(resp.status, 401);
    }

    #[tokio::test]
    async fn auth_correct_token_passes_auth() {
        // 认证通过后会尝试转发到不可达上游 → 502,据此判定认证已通过
        let cfg = GatewayConfig::builder()
            .enable_auth(true)
            .auth_token("secret")
            .route(route("/api/*", "http://127.0.0.1:1"))
            .build();
        let daemon = GatewayDaemon::new(cfg);
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer secret".to_string());
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/api/x".to_string(),
            headers,
            body: Vec::new(),
        };
        let resp = daemon.handle_request(req).await.unwrap();
        assert_ne!(resp.status, 401); // 认证通过
        assert_eq!(resp.status, 502); // 上游不可达
    }

    // --- 方法检查 ---

    #[tokio::test]
    async fn method_not_allowed_returns_405() {
        let mut r = route("/api/*", "http://127.0.0.1:1");
        r.methods = vec!["GET".to_string()];
        let cfg = GatewayConfig::builder().route(r).build();
        let daemon = GatewayDaemon::new(cfg);
        let req = HttpRequest {
            method: "POST".to_string(),
            path: "/api/x".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp = daemon.handle_request(req).await.unwrap();
        assert_eq!(resp.status, 405);
    }

    // --- 限流(端到端)---

    #[tokio::test]
    async fn rate_limit_returns_429_when_exhausted() {
        let mut r = route("/api/*", "http://127.0.0.1:1");
        r.methods = vec!["GET".to_string()];
        let cfg = GatewayConfig::builder()
            .rate_limit_per_min(1) // 容量 1
            .route(r)
            .build();
        let daemon = GatewayDaemon::new(cfg);

        // 第 1 次消耗令牌(认证未开 → 直接到限流 → 消耗令牌 → 转发 502)
        let req1 = HttpRequest {
            method: "GET".to_string(),
            path: "/api/x".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp1 = daemon.handle_request(req1).await.unwrap();
        assert_eq!(resp1.status, 502);

        // 第 2 次令牌已空 → 429
        let req2 = HttpRequest {
            method: "GET".to_string(),
            path: "/api/x".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let resp2 = daemon.handle_request(req2).await.unwrap();
        assert_eq!(resp2.status, 429);
    }

    // --- 多路由优先级 ---

    #[tokio::test]
    async fn multi_route_priority_first_match() {
        let cfg = GatewayConfig::builder()
            .route(route("/api/*", "http://first"))
            .route(route("/api/v1/*", "http://second"))
            .build();
        let daemon = GatewayDaemon::new(cfg);
        // /api/v1/users 同时命中两条,但第一条(/api/*)在先
        let matched = daemon.match_route("/api/v1/users").unwrap();
        assert_eq!(matched.upstream_url, "http://first");
    }

    // --- 热重载 ---

    #[test]
    fn hot_reload_replaces_config() {
        let daemon = GatewayDaemon::new(GatewayConfig::default());
        assert!(daemon.match_route("/api/x").is_none()); // 默认无路由
        let new_cfg = GatewayConfig::builder()
            .route(route("/api/*", "http://upstream"))
            .build();
        daemon.reload_config(new_cfg);
        assert!(daemon.match_route("/api/x").is_some()); // 重载后有路由
    }

    #[test]
    fn hot_reload_recreates_rate_limiter() {
        // 默认无限流;重载后启用限流
        let daemon = GatewayDaemon::new(GatewayConfig::default());
        assert!(daemon.rate_limiter.read().is_none());
        let new_cfg = GatewayConfig::builder().rate_limit_per_min(100).build();
        daemon.reload_config(new_cfg);
        assert!(daemon.rate_limiter.read().is_some());
    }

    // --- 关闭标志 ---

    #[test]
    fn shutdown_sets_flag() {
        let daemon = GatewayDaemon::new(GatewayConfig::default());
        assert!(!daemon.is_shutdown());
        daemon.shutdown().unwrap();
        assert!(daemon.is_shutdown());
    }

    // --- HTTP 序列化 ---

    #[test]
    fn serialize_response_sets_content_length() {
        let resp = HttpResponse::json(200, serde_json::json!({"ok": true}));
        let bytes = serialize_response(&resp);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("content-length:"));
        assert!(text.contains("content-type: application/json"));
    }
}
