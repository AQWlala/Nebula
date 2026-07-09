//! T-E-C-18: 5 个 OAuth provider 实现。
//!
//! | 模块        | Provider  | PKCE | 刷新 | 撤销 |
//! |-------------|-----------|------|------|------|
//! | [`github`]  | GitHub    | ✗    | ✗    | ✗(无端点) |
//! | [`google`]  | Google    | ✓    | ✓    | ✓    |
//! | [`microsoft`]| Microsoft | ✓    | ✓    | ✗(无公共端点) |
//! | [`slack`]   | Slack     | ✗    | ✗    | ✗(无端点) |
//! | [`notion`]  | Notion    | ✗    | ✗    | ✗(无端点) |
//!
//! GitHub 为 **pull-only**(为 T-E-L-04 GitHub MCP 连接器准备),
//! 其余 provider 同样遵循 Loop Engineering Connectors 的只读语义。

pub mod github;
pub mod google;
pub mod microsoft;
pub mod notion;
pub mod slack;

pub use github::GitHubProvider;
pub use google::GoogleProvider;
pub use microsoft::MicrosoftProvider;
pub use notion::NotionProvider;
pub use slack::SlackProvider;

use crate::oauth::ProviderConfig;

/// 构建授权 URL 的通用 helper:把 query 参数拼接到 `auth_url` 后。
///
/// PKCE challenge 由调用方传入(`Some` 时附加 `code_challenge` +
/// `code_challenge_method=S256`)。
pub(crate) fn build_authorization_url(
    config: &ProviderConfig,
    state: &str,
    pkce_challenge: Option<&str>,
) -> String {
    let mut params: Vec<(String, String)> = vec![
        ("client_id".into(), config.client_id.clone()),
        ("redirect_uri".into(), config.redirect_uri.clone()),
        ("response_type".into(), "code".into()),
        ("state".into(), state.to_string()),
    ];
    if !config.scopes.is_empty() {
        params.push(("scope".into(), config.scopes.join(" ")));
    }
    if let Some(challenge) = pkce_challenge {
        params.push(("code_challenge".into(), challenge.to_string()));
        params.push(("code_challenge_method".into(), "S256".into()));
    }
    let query = encode_query(&params);
    format!("{}?{}", config.auth_url, query)
}

/// 构建标准 `grant_type=authorization_code` 请求体(含可选 PKCE verifier)。
pub(crate) fn build_code_exchange_body(
    config: &ProviderConfig,
    code: &str,
    pkce_verifier: Option<&str>,
    client_secret: Option<&str>,
) -> Vec<(String, String)> {
    let mut body: Vec<(String, String)> = vec![
        ("grant_type".into(), "authorization_code".into()),
        ("code".into(), code.to_string()),
        ("redirect_uri".into(), config.redirect_uri.clone()),
        ("client_id".into(), config.client_id.clone()),
    ];
    if let Some(verifier) = pkce_verifier {
        body.push(("code_verifier".into(), verifier.to_string()));
    }
    if let Some(secret) = client_secret {
        body.push(("client_secret".into(), secret.to_string()));
    }
    body
}

/// 构建标准 `grant_type=refresh_token` 请求体。
pub(crate) fn build_refresh_body(
    refresh_token: &str,
    config: &ProviderConfig,
    client_secret: Option<&str>,
) -> Vec<(String, String)> {
    let mut body: Vec<(String, String)> = vec![
        ("grant_type".into(), "refresh_token".into()),
        ("refresh_token".into(), refresh_token.to_string()),
        ("client_id".into(), config.client_id.clone()),
    ];
    if let Some(secret) = client_secret {
        body.push(("client_secret".into(), secret.to_string()));
    }
    body
}

/// 极简 query string 编码(RFC 3986 unreserved 字符不转义)。
pub(crate) fn encode_query(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", pct_encode(k), pct_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// percent-encode(RFC 3986 unreserved: A-Za-z0-9-._~)。
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ProviderConfig {
        ProviderConfig {
            id: "test".into(),
            name: "Test".into(),
            client_id: "cid".into(),
            client_secret: None,
            redirect_uri: "http://127.0.0.1:1/callback".into(),
            auth_url: "https://example.com/auth".into(),
            token_url: "https://example.com/token".into(),
            revoke_url: None,
            scopes: vec!["read".into(), "write".into()],
        }
    }

    #[test]
    fn build_auth_url_includes_params() {
        let url = build_authorization_url(&cfg(), "st123", None);
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("state=st123"));
        assert!(url.contains("scope=read%20write"));
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn build_auth_url_with_pkce() {
        let url = build_authorization_url(&cfg(), "st", Some("ch456"));
        assert!(url.contains("code_challenge=ch456"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn encode_query_escents_spaces() {
        let q = encode_query(&[("a".into(), "b c".into())]);
        assert_eq!(q, "a=b%20c");
    }
}
