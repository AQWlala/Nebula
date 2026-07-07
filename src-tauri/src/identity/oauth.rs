//! OAuth 2.0 provider abstraction.
//!
//! Each external service (Gmail, GitHub, Google Calendar, etc.) implements
//! [`OAuthProvider`].  The [`crate::identity::OAuthManager`] aggregates all
//! registered providers and stores tokens in the OS keychain.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A successfully exchanged OAuth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Absolute expiry timestamp (UTC).
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// Space-separated scope string granted by the server.
    pub scope: String,
}

impl OAuthToken {
    /// Returns `true` if the token has expired (with a 30s safety margin).
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now() > self.expires_at - chrono::Duration::seconds(30)
    }
}

/// Static configuration for an OAuth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    /// Unique provider id, e.g. `"gmail"`, `"github"`.
    pub id: String,
    /// Human-readable name for UI display.
    pub name: String,
    /// OAuth client id registered with the provider.
    pub client_id: String,
    /// Redirect URI (must match the provider's console config).
    pub redirect_uri: String,
    /// Authorization endpoint URL.
    pub auth_url: String,
    /// Token exchange endpoint URL.
    pub token_url: String,
    /// Optional revocation endpoint URL.
    pub revoke_url: Option<String>,
    /// Scopes to request.
    pub scopes: Vec<String>,
}

impl OAuthProviderConfig {
    /// Builds the full authorization URL for the browser redirect.
    pub fn authorization_url(&self, state: &str) -> String {
        let params: Vec<(String, String)> = vec![
            ("client_id".into(), self.client_id.clone()),
            ("redirect_uri".into(), self.redirect_uri.clone()),
            ("response_type".into(), "code".into()),
            ("scope".into(), self.scopes.join(" ")),
            ("state".into(), state.to_string()),
        ];
        let query = urlencoding::encode_pairs(&params);
        format!("{}?{}", self.auth_url, query)
    }
}

/// Trait that each concrete OAuth provider implements.
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Unique identifier (matches `OAuthProviderConfig.id`).
    fn id(&self) -> &str;
    /// Static configuration reference.
    fn config(&self) -> &OAuthProviderConfig;
    /// Exchange an authorization code for an [`OAuthToken`].
    async fn exchange_code(&self, code: &str) -> anyhow::Result<OAuthToken>;
    /// Refresh an expired token using a refresh token.
    async fn refresh_token(&self, refresh: &str) -> anyhow::Result<OAuthToken>;
    /// Revoke a token (best-effort; errors are logged, not fatal).
    async fn revoke_token(&self, token: &str) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// Generic authorization-code flow helper (works for any RFC 6749 compliant
// provider).  Concrete providers only need to supply `config()` and the
// token endpoint's extra form params (if any).
// ---------------------------------------------------------------------------

/// Builds the POST body for a standard `grant_type=authorization_code` request.
pub fn build_token_request_body(
    config: &OAuthProviderConfig,
    code: &str,
    client_secret: Option<&str>,
) -> Vec<(String, String)> {
    let mut body = vec![
        ("grant_type".into(), "authorization_code".into()),
        ("code".into(), code.to_string()),
        ("redirect_uri".into(), config.redirect_uri.clone()),
        ("client_id".into(), config.client_id.clone()),
    ];
    if let Some(secret) = client_secret {
        body.push(("client_secret".into(), secret.to_string()));
    }
    body
}

/// Builds the POST body for a standard `grant_type=refresh_token` request.
pub fn build_refresh_request_body(
    refresh_token: &str,
    config: &OAuthProviderConfig,
    client_secret: Option<&str>,
) -> Vec<(String, String)> {
    let mut body = vec![
        ("grant_type".into(), "refresh_token".into()),
        ("refresh_token".into(), refresh_token.to_string()),
        ("client_id".into(), config.client_id.clone()),
    ];
    if let Some(secret) = client_secret {
        body.push(("client_secret".into(), secret.to_string()));
    }
    body
}

/// Parses a standard RFC 6749 token response JSON.
pub fn parse_token_response(json: &serde_json::Value) -> anyhow::Result<OAuthToken> {
    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("token response missing access_token"))?
        .to_string();
    let refresh_token = json["refresh_token"].as_str().map(|s| s.to_string());
    let scope = json["scope"].as_str().unwrap_or("").to_string();
    let expires_in = json["expires_in"].as_u64().unwrap_or(3600);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in as i64);
    Ok(OAuthToken {
        access_token,
        refresh_token,
        expires_at,
        scope,
    })
}

// ---------------------------------------------------------------------------
// Minimal URL-encoding helper (avoids pulling in the `url` crate just for
// building a query string).
// ---------------------------------------------------------------------------

mod urlencoding {
    pub fn encode_pairs(pairs: &[(String, String)]) -> String {
        pairs
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }

    fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char);
                }
                _ => out.push_str(&format!("%{:02X}", byte)),
            }
        }
        out
    }
}
