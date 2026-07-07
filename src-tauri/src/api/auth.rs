//! T-S2-B-03a: REST API 认证模块。
//!
//! 双模式认证：
//! 1. `Authorization: Bearer <token>` — 标准 Bearer token
//! 2. `X-API-Key: <key>` — API key 头
//!
//! 认证策略：
//! - 若 `rest_api_token` 为 `None`（未配置），跳过认证（开发模式）
//! - 若 `rest_api_token` 为 `Some(expected)`，请求必须携带匹配的 token/key
//! - `/api/health` 路径免认证（健康检查端点）

use hyper::Request;

/// 认证结果。`Ok(())` 表示通过或跳过，`Err((status, message))` 表示拒绝。
pub type AuthResult = Result<(), (u16, String)>;

/// 检查请求是否通过认证。
///
/// # 参数
/// - `req`: hyper 请求
/// - `expected_token`: 配置的 token。`None` 表示跳过认证（开发模式）。
///
/// # 返回
/// - `Ok(())`: 认证通过或跳过
/// - `Err((401, message))`: 缺少认证头或 token 不匹配
pub fn check_auth<B>(req: &Request<B>, expected_token: &Option<String>) -> AuthResult {
    // 未配置 token → 开发模式，跳过认证
    let expected = match expected_token {
        None => return Ok(()),
        Some(t) => t,
    };

    // 健康检查端点免认证
    let path = req.uri().path();
    if path == "/api/health" {
        return Ok(());
    }

    // 检查 Authorization: Bearer <token>
    if let Some(auth_header) = req.headers().get(hyper::header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                if token == expected {
                    return Ok(());
                }
                return Err((401, "invalid bearer token".to_string()));
            }
        }
    }

    // 检查 X-API-Key: <key>
    if let Some(api_key_header) = req.headers().get("x-api-key") {
        if let Ok(key_str) = api_key_header.to_str() {
            if key_str == expected {
                return Ok(());
            }
            return Err((401, "invalid api key".to_string()));
        }
    }

    Err((401, "missing or invalid authorization header".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::Request;

    fn build_req(token: Option<&str>, api_key: Option<&str>, path: &str) -> Request<()> {
        let mut builder = Request::builder().uri(path);
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {}", t));
        }
        if let Some(k) = api_key {
            builder = builder.header("x-api-key", k);
        }
        builder.body(()).unwrap()
    }

    #[test]
    fn no_token_configured_bypasses_auth() {
        let req = build_req(None, None, "/api/memories");
        assert!(check_auth(&req, &None).is_ok());
    }

    #[test]
    fn valid_bearer_token_passes() {
        let req = build_req(Some("secret123"), None, "/api/memories");
        assert!(check_auth(&req, &Some("secret123".to_string())).is_ok());
    }

    #[test]
    fn invalid_bearer_token_rejected() {
        let req = build_req(Some("wrong"), None, "/api/memories");
        let result = check_auth(&req, &Some("secret123".to_string()));
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, 401);
    }

    #[test]
    fn valid_api_key_passes() {
        let req = build_req(None, Some("secret123"), "/api/memories");
        assert!(check_auth(&req, &Some("secret123".to_string())).is_ok());
    }

    #[test]
    fn missing_auth_rejected_when_token_configured() {
        let req = build_req(None, None, "/api/memories");
        let result = check_auth(&req, &Some("secret123".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn health_endpoint_bypasses_auth() {
        let req = build_req(None, None, "/api/health");
        assert!(check_auth(&req, &Some("secret123".to_string())).is_ok());
    }

    #[test]
    fn malformed_authorization_header_rejected() {
        let mut builder = Request::builder().uri("/api/memories");
        builder = builder.header("authorization", "NotBearer abc");
        let req = builder.body(()).unwrap();
        let result = check_auth(&req, &Some("abc".to_string()));
        assert!(result.is_err());
    }
}
