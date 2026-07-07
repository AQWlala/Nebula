//! Gmail OAuth integration.
//!
//! Implements the RFC 6749 authorization-code flow against Google's
//! OAuth 2.0 endpoints and adds a thin wrapper around the Gmail REST
//! API for incremental email fetch.  Fetched emails are returned as
//! [`EmailDelta`] for the caller (typically `OAuthManager`'s sync loop)
//! to feed into the memory pipeline after Injection Guard review.
//!
//!对标: OpenHuman — 每 20 分钟拉取邮件增量,每封邮件 ≤ 3k token。

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::oauth::{
    build_refresh_request_body, build_token_request_body, parse_token_response, OAuthProvider,
    OAuthProviderConfig, OAuthToken,
};

/// Gmail API base URL.
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// Default Gmail OAuth endpoints (Google Identity Platform).
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Gmail OAuth provider.
pub struct GmailOAuthProvider {
    config: OAuthProviderConfig,
    client_secret: Option<String>,
    http: reqwest::Client,
}

impl GmailOAuthProvider {
    /// Creates a new Gmail provider with the given client credentials.
    ///
    /// `client_secret` is `None` for PKCE-only flows (Google now supports
    /// confidential-client flows where the secret is required).
    pub fn new(client_id: String, client_secret: Option<String>, redirect_uri: String) -> Self {
        let config = OAuthProviderConfig {
            id: "gmail".to_string(),
            name: "Gmail".to_string(),
            client_id,
            redirect_uri,
            auth_url: GOOGLE_AUTH_URL.to_string(),
            token_url: GOOGLE_TOKEN_URL.to_string(),
            revoke_url: Some(GOOGLE_REVOKE_URL.to_string()),
            // Gmail readonly scope — read-only access to mailbox contents.
            scopes: vec![
                "https://www.googleapis.com/auth/gmail.readonly".to_string(),
                "https://www.googleapis.com/auth/userinfo.email".to_string(),
            ],
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            client_secret,
            http,
        }
    }

    /// Fetches incremental emails since the given timestamp.
    ///
    /// Uses Gmail's `users.messages.list` with a `q=after:{unix_ts}` filter
    /// for incremental sync, then fetches each message's metadata + body.
    ///
    /// Returns at most `limit` emails (default 50) to bound token usage.
    /// Each email body is truncated to ~3k tokens (roughly 12k chars) to
    /// match OpenHuman's per-message budget.
    pub async fn fetch_emails(
        &self,
        token: &OAuthToken,
        since_unix_ts: i64,
        limit: usize,
    ) -> Result<Vec<EmailDelta>> {
        let max_body_chars = 12_000;

        // 1. List message ids since `since_unix_ts`.
        let list_url = format!("{GMAIL_API_BASE}/messages");
        let q = format!("after:{since_unix_ts}");
        let list_resp = self
            .http
            .get(&list_url)
            .bearer_auth(&token.access_token)
            .query(&[("q", &q), ("maxResults", &limit.to_string())])
            .send()
            .await?
            .error_for_status()?;
        let list_json: serde_json::Value = list_resp.json().await?;
        let message_ids: Vec<String> = list_json["messages"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // 2. Fetch each message's full content.
        let mut deltas = Vec::with_capacity(message_ids.len());
        for id in message_ids {
            let msg_url = format!("{GMAIL_API_BASE}/messages/{id}");
            let msg_resp = match self
                .http
                .get(&msg_url)
                .bearer_auth(&token.access_token)
                .query(&[("format", &"full".to_string())])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(target: "nebula.oauth.gmail", msg_id = %id, error = %e, "fetch failed");
                    continue;
                }
            };
            if !msg_resp.status().is_success() {
                tracing::warn!(
                    target: "nebula.oauth.gmail",
                    msg_id = %id,
                    status = %msg_resp.status(),
                    "fetch non-2xx"
                );
                continue;
            }
            let msg_json: serde_json::Value = match msg_resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(target: "nebula.oauth.gmail", msg_id = %id, error = %e, "decode failed");
                    continue;
                }
            };

            let subject = extract_header(&msg_json, "Subject").unwrap_or_default();
            let from = extract_header(&msg_json, "From").unwrap_or_default();
            let date = extract_header(&msg_json, "Date").unwrap_or_default();
            let body = extract_body(&msg_json, max_body_chars);
            let internal_ts = msg_json["internalDate"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);

            deltas.push(EmailDelta {
                message_id: id,
                subject,
                from,
                date,
                body,
                timestamp_ms: internal_ts,
            });
        }
        Ok(deltas)
    }
}

#[async_trait]
impl OAuthProvider for GmailOAuthProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn config(&self) -> &OAuthProviderConfig {
        &self.config
    }

    async fn exchange_code(&self, code: &str) -> Result<OAuthToken> {
        let body = build_token_request_body(&self.config, code, self.client_secret.as_deref());
        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&body)
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        parse_token_response(&json).context("Gmail token exchange failed")
    }

    async fn refresh_token(&self, refresh: &str) -> Result<OAuthToken> {
        let body = build_refresh_request_body(refresh, &self.config, self.client_secret.as_deref());
        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&body)
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        parse_token_response(&json).context("Gmail token refresh failed")
    }

    async fn revoke_token(&self, token: &str) -> Result<()> {
        // Google's revoke endpoint takes the token as a query param.
        let revoke_url = self
            .config
            .revoke_url
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no revoke url configured"))?;
        let _ = self
            .http
            .post(revoke_url)
            .query(&[("token", token)])
            .send()
            .await?;
        Ok(())
    }
}

/// A single fetched email, ready for memory ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailDelta {
    pub message_id: String,
    pub subject: String,
    pub from: String,
    pub date: String,
    /// Truncated to ~3k tokens (12k chars).
    pub body: String,
    pub timestamp_ms: i64,
}

/// Extracts a header value from a Gmail message payload by name
/// (case-insensitive).
fn extract_header(msg: &serde_json::Value, name: &str) -> Option<String> {
    let headers = msg["payload"]["headers"].as_array()?;
    for h in headers {
        if h["name"]
            .as_str()
            .map(|s| s.eq_ignore_ascii_case(name))
            .unwrap_or(false)
        {
            return h["value"].as_str().map(|s| s.to_string());
        }
    }
    None
}

/// Extracts the plain-text body from a Gmail message payload,
/// truncating to `max_chars`.  Walks multipart payloads looking for
/// the first `text/plain` part.
fn extract_body(msg: &serde_json::Value, max_chars: usize) -> String {
    if let Some(body) = msg["payload"]["body"]["data"].as_str() {
        let decoded = decode_base64url(body);
        return truncate(&decoded, max_chars);
    }
    if let Some(parts) = msg["payload"]["parts"].as_array() {
        for part in parts {
            if part["mimeType"].as_str() == Some("text/plain") {
                if let Some(data) = part["body"]["data"].as_str() {
                    let decoded = decode_base64url(data);
                    return truncate(&decoded, max_chars);
                }
            }
        }
        // Fall back to first part with body data.
        for part in parts {
            if let Some(data) = part["body"]["data"].as_str() {
                let decoded = decode_base64url(data);
                return truncate(&decoded, max_chars);
            }
        }
    }
    String::new()
}

/// Gmail uses URL-safe base64 without padding.
fn decode_base64url(input: &str) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    match engine.decode(input) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => input.to_string(),
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundary() {
        // "héllo" = 6 bytes (h=1, é=2, l=1, l=1, o=1).
        // 100 字节边界恰好在 'l' 上(is_char_boundary(100) == true),
        // 所以截断后 = 100 bytes + "…"(3 bytes UTF-8) = 103 bytes。
        let s = "héllo".repeat(10000);
        let t = truncate(&s, 100);
        // max_chars + "…" (U+2026 = 3 bytes in UTF-8)
        assert!(t.len() <= 103, "got {} bytes", t.len());
        assert!(t.ends_with('…'));
    }

    #[test]
    fn provider_id_is_gmail() {
        let p = GmailOAuthProvider::new("cid".into(), None, "http://localhost".into());
        assert_eq!(p.id(), "gmail");
        assert_eq!(p.config().scopes.len(), 2);
    }

    #[test]
    fn authorization_url_includes_gmail_scope() {
        let p = GmailOAuthProvider::new("cid".into(), None, "http://localhost".into());
        let url = p.config().authorization_url("state123");
        assert!(url.contains("gmail.readonly"));
        assert!(url.contains("state=state123"));
    }
}
