//! T-E-S-33: OpenAPI 工具服务器。
//!
//! 将 OpenAPI spec(3.0/3.1)自动转换为 `Tool` trait 实现,使 LLM 可通过
//! `ToolRegistry` 调用 REST API。每个 `paths.operations` 条目生成一个
//! `OpenApiToolAdapter`,注册到 `ToolRegistry` 后由 Agent 调用。
//!
//! ## 架构
//!
//! - [`OpenApiToolServer`]:持有解析后的 `openapiv3::OpenAPI` + base_url + auth,
//!   提供 `from_spec` / `list_tool_definitions` / `execute` 方法。
//! - [`OpenApiToolAdapter`]:async→sync 桥接(参照 `McpToolAdapter`),实现
//!   `Tool` trait,注册到 `ToolRegistry`。
//! - [`OpenApiAuth`]:Bearer / ApiKey 鉴权注入。
//! - SSRF 防护复用 `crate::security::ssrf_guard::SsrfGuard`(initial URL 验证
//!   + redirect chain 验证)。
//!
//! ## Feature gate
//!
//! 整个模块由 `#[cfg(feature = "openapi")]` 门控,默认不编译。

#![cfg(feature = "openapi")]

use std::sync::Arc;

use anyhow::{anyhow, bail, Result};

use crate::security::ssrf_guard::SsrfGuard;
use crate::tools::{Tool, ToolOutput};

/// 响应体截断阈值(10KB,spec §4 R4)。
const MAX_RESPONSE_BYTES: usize = 10_240;

/// HTTP 请求超时(30s,spec §4 R3)。
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// OpenAPI 鉴权方式。
#[derive(Debug, Clone)]
pub enum OpenApiAuth {
    /// `Authorization: Bearer <token>` 注入。
    Bearer(String),
    /// 自定义 header 注入(如 `X-API-Key: <value>`)。
    ApiKey { header: String, value: String },
}

/// 解析后的工具定义(内部表示,用于生成 ToolDefinition + execute 路由)。
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub method: String,
    pub path: String,
}

/// OpenAPI 工具服务器:解析 spec + 构造 HTTP 请求 + 鉴权注入 + SSRF 防护。
///
/// 用法:
/// ```ignore
/// let server = OpenApiToolServer::from_spec(&yaml)?
///     .with_auth(OpenApiAuth::Bearer("xxx".into()));
/// let defs = server.list_tool_definitions();
/// let body = server.execute("getUser", &json!({"id": 42})).await?;
/// ```
pub struct OpenApiToolServer {
    spec: openapiv3::OpenAPI,
    base_url: String,
    auth: Option<OpenApiAuth>,
    ssrf_guard: SsrfGuard,
}

impl OpenApiToolServer {
    /// 从 JSON 或 YAML 字符串解析 OpenAPI spec。
    ///
    /// 自动判别:字符串以 `{` 开头视为 JSON,否则视为 YAML。
    /// base_url 取 `spec.servers[0].url`;无 servers 时为空字符串
    /// (调用方需通过 `with_base_url` 覆盖)。
    pub fn from_spec(spec_yaml_or_json: &str) -> Result<Self> {
        let spec: openapiv3::OpenAPI = if spec_yaml_or_json.trim_start().starts_with('{') {
            serde_json::from_str(spec_yaml_or_json)
                .map_err(|e| anyhow!("JSON spec parse error: {e}"))?
        } else {
            serde_yaml::from_str(spec_yaml_or_json)
                .map_err(|e| anyhow!("YAML spec parse error: {e}"))?
        };
        let base_url = spec
            .servers
            .first()
            .map(|s| s.url.clone())
            .unwrap_or_default();
        Ok(Self {
            spec,
            base_url,
            auth: None,
            ssrf_guard: SsrfGuard::new(),
        })
    }

    /// Builder:设置鉴权(链式调用)。
    pub fn with_auth(mut self, auth: OpenApiAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Builder:覆盖 base_url(用于测试或运行时重定向)。
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Builder:允许内网 IP(测试用;生产环境应保持 false)。
    pub fn with_allow_private(mut self, allow: bool) -> Self {
        self.ssrf_guard = SsrfGuard::new().with_allow_private(allow);
        self
    }

    /// 返回鉴权 header(name, value),用于测试验证。
    pub fn auth_header(&self) -> Option<(String, String)> {
        match &self.auth {
            Some(OpenApiAuth::Bearer(t)) => {
                Some(("Authorization".to_string(), format!("Bearer {t}")))
            }
            Some(OpenApiAuth::ApiKey { header, value }) => Some((header.clone(), value.clone())),
            None => None,
        }
    }

    /// 遍历 `spec.paths.operations`,每个 operation 生成一个 [`ToolDefinition`]。
    ///
    /// 工具名取 `operation_id`;无 `operation_id` 时生成 `method_path`
    /// (path 中的 `/` 替换为 `_`)。
    pub fn list_tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = Vec::new();
        for (path, item_ref) in &self.spec.paths.paths {
            let item = match item_ref {
                openapiv3::ReferenceOr::Item(i) => i,
                openapiv3::ReferenceOr::Reference { .. } => continue,
            };
            for (method, op) in item.iter() {
                let name = op
                    .operation_id
                    .clone()
                    .unwrap_or_else(|| format!("{}_{}", method, path.replace('/', "_")));
                let description = op
                    .summary
                    .clone()
                    .or_else(|| op.description.clone())
                    .unwrap_or_default();
                let parameters = extract_parameters(op);
                defs.push(ToolDefinition {
                    name,
                    description,
                    parameters,
                    method: method.to_string(),
                    path: path.clone(),
                });
            }
        }
        defs
    }

    /// 查找指定工具名对应的 (method, path, operation)。
    fn find_operation(&self, tool_name: &str) -> Result<(String, String, openapiv3::Operation)> {
        for (path, item_ref) in &self.spec.paths.paths {
            let item = match item_ref {
                openapiv3::ReferenceOr::Item(i) => i,
                openapiv3::ReferenceOr::Reference { .. } => continue,
            };
            for (method, op) in item.iter() {
                let name = op
                    .operation_id
                    .clone()
                    .unwrap_or_else(|| format!("{}_{}", method, path.replace('/', "_")));
                if name == tool_name {
                    return Ok((method.to_string(), path.clone(), op.clone()));
                }
            }
        }
        bail!("tool '{}' not found in OpenAPI spec paths", tool_name)
    }

    /// 执行工具调用:构造 HTTP 请求 + 鉴权注入 + SSRF 验证 + 发送。
    ///
    /// - Path 参数(在 spec 中声明为 `in: path`)从 `args` 取值并替换 URL 占位符。
    /// - Query 参数(`in: query`)追加到 URL query string。
    /// - POST/PUT/PATCH:剩余 args 作为 JSON body。
    /// - GET/DELETE:剩余 args 追加为 query 参数(best-effort)。
    /// - 响应体超过 10KB 时截断。
    pub async fn execute(&self, tool_name: &str, args: &serde_json::Value) -> Result<String> {
        let (method_str, path, op) = self.find_operation(tool_name)?;

        if self.base_url.is_empty() {
            bail!("OpenAPI spec has no server URL; call with_base_url to set one");
        }

        let mut url = format!("{}{}", self.base_url.trim_end_matches('/'), path);

        // 用空 map 作为 fallback(args 不是 object 时)。
        let empty_map = serde_json::Map::new();
        let obj = args.as_object().unwrap_or(&empty_map);

        // 分类参数:path / query / header
        let mut path_params: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut query_params: Vec<(String, String)> = Vec::new();
        let mut consumed_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

        for param_ref in &op.parameters {
            let p = match param_ref {
                openapiv3::ReferenceOr::Item(p) => p,
                openapiv3::ReferenceOr::Reference { .. } => continue,
            };
            let data = match p {
                openapiv3::Parameter::Query { parameter_data, .. } => parameter_data,
                openapiv3::Parameter::Header { parameter_data, .. } => parameter_data,
                openapiv3::Parameter::Path { parameter_data, .. } => parameter_data,
                openapiv3::Parameter::Cookie { parameter_data, .. } => parameter_data,
            };
            match p {
                openapiv3::Parameter::Path { .. } => {
                    path_params.insert(data.name.clone());
                    if let Some(val) = obj.get(&data.name) {
                        let s = value_to_string(val);
                        url = url.replace(&format!("{{{}}}", data.name), &s);
                        consumed_keys.insert(data.name.clone());
                    }
                }
                openapiv3::Parameter::Query { .. } => {
                    if let Some(val) = obj.get(&data.name) {
                        query_params.push((data.name.clone(), value_to_string(val)));
                        consumed_keys.insert(data.name.clone());
                    }
                }
                _ => {}
            }
        }

        // 追加 query string
        if !query_params.is_empty() {
            let qs = query_params
                .iter()
                .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            url.push_str("?");
            url.push_str(&qs);
        }

        // SSRF 验证:拦截 127.0.0.1 / 10.x / 192.168.x / 169.254.x / 100.64.x
        self.ssrf_guard.validate_url(&url)?;

        // 构建 SSRF-safe HTTP client(redirect chain 也验证)
        let client = self.ssrf_guard.build_safe_client()?;

        let method = method_from_str(&method_str);
        let mut req = client.request(method, &url);

        // 鉴权注入
        if let Some((hdr_name, hdr_val)) = self.auth_header() {
            req = req.header(&hdr_name, &hdr_val);
        }

        // POST/PUT/PATCH:剩余 args 作为 JSON body
        let is_body_method = matches!(method_str.as_str(), "post" | "put" | "patch");
        if is_body_method {
            let body: serde_json::Value = obj
                .iter()
                .filter(|(k, _)| !consumed_keys.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<serde_json::Map<_, _>>()
                .into();
            if body.as_object().map(|m| !m.is_empty()).unwrap_or(false) {
                req = req.json(&body);
            }
        } else {
            // GET/DELETE:剩余 args 追加为 query 参数
            for (k, v) in obj.iter() {
                if !consumed_keys.contains(k.as_str()) {
                    req = req.query(&[(k.as_str(), value_to_string(v))]);
                }
            }
        }

        // 发送请求(30s 超时)
        let resp = req
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            bail!("HTTP {}: {}", status, truncate(&body, MAX_RESPONSE_BYTES));
        }

        Ok(truncate(&body, MAX_RESPONSE_BYTES))
    }
}

/// 把 `serde_json::Value` 转换为 URL/path 友好的字符串。
fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// 简单 URL 编码(仅编码特殊字符)。
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 截断字符串到指定字节长度(UTF-8 安全:在字符边界截断)。
fn truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[truncated]", &s[..end])
}

/// 从 method 字符串构造 `reqwest::Method`。
fn method_from_str(s: &str) -> reqwest::Method {
    match s.to_ascii_lowercase().as_str() {
        "get" => reqwest::Method::GET,
        "post" => reqwest::Method::POST,
        "put" => reqwest::Method::PUT,
        "delete" => reqwest::Method::DELETE,
        "patch" => reqwest::Method::PATCH,
        "head" => reqwest::Method::HEAD,
        "options" => reqwest::Method::OPTIONS,
        "trace" => reqwest::Method::TRACE,
        _ => reqwest::Method::GET,
    }
}

/// 从 OpenAPI Operation 提取参数 schema(JSON Schema 格式)。
fn extract_parameters(op: &openapiv3::Operation) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for param_ref in &op.parameters {
        let p = match param_ref {
            openapiv3::ReferenceOr::Item(p) => p,
            openapiv3::ReferenceOr::Reference { .. } => continue,
        };
        let data = match p {
            openapiv3::Parameter::Query { parameter_data, .. } => parameter_data,
            openapiv3::Parameter::Header { parameter_data, .. } => parameter_data,
            openapiv3::Parameter::Path { parameter_data, .. } => parameter_data,
            openapiv3::Parameter::Cookie { parameter_data, .. } => parameter_data,
        };
        let mut schema = serde_json::json!({"type": "string"});
        if let Some(desc) = &data.description {
            schema["description"] = serde_json::Value::String(desc.clone());
        }
        properties.insert(data.name.clone(), schema);
        if data.required {
            required.push(data.name.clone());
        }
    }

    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

// ===========================================================================
// OpenApiToolAdapter: async→sync 桥接(参照 McpToolAdapter L34-118)
// ===========================================================================

/// 把 `OpenApiToolServer` 适配为 `Tool` trait,使其可注册到 `ToolRegistry`。
///
/// 持有 `Arc<OpenApiToolServer>` + 对应的 `ToolDefinition`。
/// `call()` 内部通过 `tokio::task::block_in_place` 把 async `execute` 桥接到
/// 同步 trait(在多线程 runtime 上有效);若当前不在 runtime 中(测试场景),
/// 则临时构建 current-thread runtime 完成 `block_on`。
pub struct OpenApiToolAdapter {
    server: Arc<OpenApiToolServer>,
    def: ToolDefinition,
}

impl OpenApiToolAdapter {
    pub fn new(server: Arc<OpenApiToolServer>, def: ToolDefinition) -> Self {
        Self { server, def }
    }
}

impl Tool for OpenApiToolAdapter {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn description(&self) -> &str {
        &self.def.description
    }

    fn schema(&self) -> serde_json::Value {
        self.def.parameters.clone()
    }

    fn call(&self, arguments: serde_json::Value) -> Result<ToolOutput> {
        let server = self.server.clone();
        let name = self.def.name.clone();
        let fut = async move { server.execute(&name, &arguments).await };

        // 把 async 调用桥接到同步 trait(参照 McpToolAdapter)。
        let result = match tokio::runtime::Handle::try_current() {
            Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(fut)
            }
        };

        match result {
            Ok(body) => Ok(ToolOutput {
                success: true,
                result: body,
                error: None,
            }),
            Err(e) => Ok(ToolOutput {
                success: false,
                result: String::new(),
                error: Some(e.to_string()),
            }),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 最小可用 OpenAPI 3.0 spec(JSON),含 2 个 operation。
    fn sample_spec_json(base_url: &str) -> String {
        format!(
            r#"{{
  "openapi": "3.0.0",
  "info": {{ "title": "Test API", "version": "1.0" }},
  "servers": [{{ "url": "{}" }}],
  "paths": {{
    "/users/{{userId}}": {{
      "get": {{
        "operationId": "getUser",
        "summary": "Get a user by ID",
        "parameters": [
          {{
            "name": "userId",
            "in": "path",
            "required": true,
            "schema": {{ "type": "integer" }}
          }}
        ],
        "responses": {{
          "200": {{ "description": "OK" }}
        }}
      }}
    }},
    "/users": {{
      "post": {{
        "operationId": "createUser",
        "summary": "Create a user",
        "requestBody": {{
          "content": {{
            "application/json": {{
              "schema": {{ "type": "object" }}
            }}
          }}
        }},
        "responses": {{
          "201": {{ "description": "Created" }}
        }}
      }}
    }}
  }}
}}"#,
            base_url
        )
    }

    /// 最小可用 OpenAPI 3.0 spec(YAML)。
    fn sample_spec_yaml(base_url: &str) -> String {
        format!(
            r#"openapi: 3.0.0
info:
  title: Test API
  version: "1.0"
servers:
  - url: {base_url}
paths:
  /users/{{userId}}:
    get:
      operationId: getUser
      summary: Get a user by ID
      parameters:
        - name: userId
          in: path
          required: true
          schema:
            type: integer
      responses:
        "200":
          description: OK
  /users:
    post:
      operationId: createUser
      summary: Create a user
      responses:
        "201":
          description: Created
"#,
            base_url = base_url
        )
    }

    #[test]
    fn test_parse_openapi_json() {
        let spec = sample_spec_json("https://api.example.com");
        let server = OpenApiToolServer::from_spec(&spec);
        assert!(server.is_ok(), "JSON parse should succeed");
        let server = server.expect("test op should succeed");
        assert_eq!(server.base_url, "https://api.example.com");
        assert!(server.auth.is_none());
    }

    #[test]
    fn test_parse_openapi_yaml() {
        let spec = sample_spec_yaml("https://api.example.com");
        let server = OpenApiToolServer::from_spec(&spec);
        assert!(server.is_ok(), "YAML parse should succeed");
        let server = server.expect("test op should succeed");
        assert_eq!(server.base_url, "https://api.example.com");
    }

    #[test]
    fn test_list_tool_definitions() {
        let spec = sample_spec_json("https://api.example.com");
        let server = OpenApiToolServer::from_spec(&spec).expect("create should succeed");
        let defs = server.list_tool_definitions();
        assert_eq!(defs.len(), 2, "should have 2 tool definitions");

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"getUser"));
        assert!(names.contains(&"createUser"));

        let get_user = defs
            .iter()
            .find(|d| d.name == "getUser")
            .expect("query should succeed");
        assert_eq!(get_user.method, "get");
        assert_eq!(get_user.path, "/users/{userId}");
        assert_eq!(get_user.description, "Get a user by ID");
        // parameters schema 应包含 userId
        let props = get_user.parameters["properties"]
            .as_object()
            .expect("get should succeed");
        assert!(props.contains_key("userId"));
        let required = get_user.parameters["required"]
            .as_array()
            .expect("get should succeed");
        assert!(required.iter().any(|r| r == "userId"));
    }

    #[test]
    fn test_bearer_auth_injection() {
        let server = OpenApiToolServer::from_spec(&sample_spec_json("https://api.example.com"))
            .expect("test op should succeed")
            .with_auth(OpenApiAuth::Bearer("my-secret-token".into()));

        let header = server.auth_header().expect("auth header should be set");
        assert_eq!(header.0, "Authorization");
        assert!(header.1.starts_with("Bearer "));
        assert!(header.1.contains("my-secret-token"));
    }

    #[test]
    fn test_apikey_auth_injection() {
        let server = OpenApiToolServer::from_spec(&sample_spec_json("https://api.example.com"))
            .expect("test op should succeed")
            .with_auth(OpenApiAuth::ApiKey {
                header: "X-API-Key".into(),
                value: "abc123".into(),
            });

        let header = server.auth_header().expect("auth header should be set");
        assert_eq!(header.0, "X-API-Key");
        assert_eq!(header.1, "abc123");
    }

    #[test]
    fn test_ssrf_block_private_ip() {
        // spec 指向 127.0.0.1(默认 SSRF 拦截)
        let spec = sample_spec_json("http://127.0.0.1:8080");
        let server = OpenApiToolServer::from_spec(&spec).expect("create should succeed");

        let rt = tokio::runtime::Runtime::new().expect("create should succeed");
        let result = rt.block_on(async { server.execute("getUser", &json!({"userId": 42})).await });

        assert!(result.is_err(), "should block 127.0.0.1");
        let err = result.unwrap_err().to_string();
        assert!(
            err.to_lowercase().contains("ssrf") || err.to_lowercase().contains("loopback"),
            "error should mention SSRF/loopback, got: {err}"
        );
    }

    /// 启动一个简单的 mock HTTP 服务器(单请求),返回固定 JSON 响应。
    fn start_mock_server(
        status_code: u16,
        response_body: String,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test op should succeed");
        let port = listener
            .local_addr()
            .expect("test op should succeed")
            .port();
        let url = format!("http://127.0.0.1:{}", port);
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test op should succeed");
            let mut buf = [0u8; 8192];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                status_code,
                response_body.len(),
                response_body
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        });
        (url, handle)
    }

    /// 启动一个 echo mock HTTP 服务器(单请求),返回收到的请求头。
    ///
    /// 读循环:持续 read 直到看到 `\r\n\r\n`(HTTP 头结束标记)或达到 buffer 上限。
    /// 这样可以可靠捕获 reqwest 发送的完整请求头(包括 Authorization / X-API-Key),
    /// 避免单次 read 只读到部分缓冲区导致 echo 缺失鉴权 header。
    fn start_echo_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test op should succeed");
        let port = listener
            .local_addr()
            .expect("test op should succeed")
            .port();
        let url = format!("http://127.0.0.1:{}", port);
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test op should succeed");
            // 设置 read 超时,避免对端不发完整请求时永久阻塞
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));

            let mut buf = Vec::with_capacity(16 * 1024);
            let mut chunk = [0u8; 4096];

            // 循环读取直到看到 HTTP 头结束标记 `\r\n\r\n`,或对端关闭写,或达到上限。
            loop {
                if buf.len() >= 32 * 1024 {
                    break;
                }
                let n = match std::io::Read::read(&mut stream, &mut chunk) {
                    Ok(0) => break, // EOF
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(_) => break,
                };
                buf.extend_from_slice(&chunk[..n]);
                // 检测 HTTP 头结束标记
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }

            let request = String::from_utf8_lossy(&buf).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                request.len(),
                request
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        });
        (url, handle)
    }

    #[test]
    fn test_execute_tool_call() {
        // 启动 mock 服务器,返回固定 JSON
        let (mock_url, mock_handle) =
            start_mock_server(200, r#"{"id": 42, "name": "Alice"}"#.to_string());

        // spec 指向 mock 服务器,允许内网 IP
        let spec = sample_spec_json(&mock_url);
        let server = OpenApiToolServer::from_spec(&spec)
            .expect("test op should succeed")
            .with_allow_private(true);

        let rt = tokio::runtime::Runtime::new().expect("create should succeed");
        let result = rt.block_on(async { server.execute("getUser", &json!({"userId": 42})).await });

        assert!(result.is_ok(), "execute should succeed");
        let body = result.expect("test op should succeed");
        assert!(body.contains("Alice"), "body should contain Alice");

        // 等待 mock 服务器线程结束
        let _ = mock_handle.join();
    }

    #[test]
    fn test_bearer_auth_injection_via_http() {
        // 启动 echo 服务器,返回收到的请求
        let (mock_url, mock_handle) = start_echo_server();

        let spec = sample_spec_json(&mock_url);
        let server = OpenApiToolServer::from_spec(&spec)
            .expect("test op should succeed")
            .with_allow_private(true)
            .with_auth(OpenApiAuth::Bearer("test-bearer-xyz".into()));

        let rt = tokio::runtime::Runtime::new().expect("create should succeed");
        let result = rt.block_on(async { server.execute("getUser", &json!({"userId": 42})).await });

        assert!(
            result.is_ok(),
            "execute should succeed, err: {:?}",
            result.err()
        );
        let echoed = result.expect("test op should succeed");
        // reqwest/hyper 默认将 header 名小写化(HTTP/2 规范要求,HTTP/1.1 大小写不敏感),
        // 所以这里用大小写不敏感比较验证 Bearer header 注入。
        let echoed_lower = echoed.to_lowercase();
        assert!(
            echoed_lower.contains("authorization: bearer test-bearer-xyz"),
            "echoed request should contain Bearer header, got: {echoed}"
        );

        let _ = mock_handle.join();
    }

    #[test]
    fn test_apikey_auth_injection_via_http() {
        let (mock_url, mock_handle) = start_echo_server();

        let spec = sample_spec_json(&mock_url);
        let server = OpenApiToolServer::from_spec(&spec)
            .expect("test op should succeed")
            .with_allow_private(true)
            .with_auth(OpenApiAuth::ApiKey {
                header: "X-API-Key".into(),
                value: "key-abc-123".into(),
            });

        let rt = tokio::runtime::Runtime::new().expect("create should succeed");
        let result = rt.block_on(async { server.execute("getUser", &json!({"userId": 42})).await });

        assert!(
            result.is_ok(),
            "execute should succeed, err: {:?}",
            result.err()
        );
        let echoed = result.expect("test op should succeed");
        // reqwest 会把 header 名小写化(如 `x-api-key: key-abc-123`),
        // 所以用大小写不敏感比较验证 API key header 注入。
        let echoed_lower = echoed.to_lowercase();
        assert!(
            echoed_lower.contains("x-api-key: key-abc-123"),
            "echoed request should contain API key header, got: {echoed}"
        );

        let _ = mock_handle.join();
    }

    #[test]
    fn test_adapter_tool_metadata() {
        let spec = sample_spec_json("https://api.example.com");
        let server = Arc::new(OpenApiToolServer::from_spec(&spec).expect("create should succeed"));
        let defs = server.list_tool_definitions();
        let def = defs
            .into_iter()
            .find(|d| d.name == "getUser")
            .expect("query should succeed");
        let adapter = OpenApiToolAdapter::new(server, def);

        assert_eq!(adapter.name(), "getUser");
        assert_eq!(adapter.description(), "Get a user by ID");
        let schema = adapter.schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn test_register_openapi_tools() {
        use crate::tools::ToolRegistry;

        let spec = sample_spec_json("https://api.example.com");
        let server = OpenApiToolServer::from_spec(&spec).expect("create should succeed");

        let registry = ToolRegistry::new();
        let count = registry.register_openapi_tools(server);
        assert_eq!(count, 2, "should register 2 tools");

        // 验证工具已注册
        let all = registry.list_all();
        let names: Vec<String> = all.iter().map(|(n, _, _)| n.clone()).collect();
        assert!(names.iter().any(|n| n == "getUser"));
        assert!(names.iter().any(|n| n == "createUser"));
    }
}
