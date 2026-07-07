//! Notion OAuth integration.
//!
//! Implements the OAuth 2.0 public integration flow against Notion's
//! endpoints and adds thin wrappers around the REST API for fetching
//! databases / pages and pushing Nebula knowledge updates back to
//! Notion (bidirectional sync).
//!
//! 对标: OpenHuman — 双向同步 Notion 页面,拉取用户有权限的 databases
//! 和 pages,推送 Nebula L3 compiled knowledge 到 Notion page update。

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::oauth::{parse_token_response, OAuthProvider, OAuthProviderConfig, OAuthToken};

const NOTION_API_BASE: &str = "https://api.notion.com/v1";
const NOTION_AUTH_URL: &str = "https://api.notion.com/v1/oauth/authorize";
const NOTION_TOKEN_URL: &str = "https://api.notion.com/v1/oauth/token";

/// Notion OAuth provider.
pub struct NotionOAuthProvider {
    config: OAuthProviderConfig,
    client_secret: Option<String>,
    http: reqwest::Client,
}

impl NotionOAuthProvider {
    /// Creates a new Notion provider.
    ///
    /// Notion's public integrations require a client secret for the
    /// token exchange.
    pub fn new(
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
    ) -> Self {
        let config = OAuthProviderConfig {
            id: "notion".to_string(),
            name: "Notion".to_string(),
            client_id,
            redirect_uri,
            auth_url: NOTION_AUTH_URL.to_string(),
            token_url: NOTION_TOKEN_URL.to_string(),
            revoke_url: None, // Notion has no per-token revoke; delete integration from workspace.
            // Notion scopes are workspace-granted; the OAuth flow doesn't request
            // granular scopes, but we list them for documentation purposes.
            scopes: vec![],
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

    /// Fetches all databases the integration has access to.
    pub async fn fetch_databases(
        &self,
        token: &OAuthToken,
        page_size: usize,
    ) -> Result<Vec<DatabaseSummary>> {
        let url = format!("{NOTION_API_BASE}/databases");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(format!("secret_{}", token.access_token))
            .header("Notion-Version", "2022-06-28")
            .query(&[("page_size", &page_size.clamp(1, 100).to_string())])
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        let arr = json["results"].as_array();
        Ok(arr
            .map(|a| {
                a.iter()
                    .map(|d| DatabaseSummary {
                        id: d["id"].as_str().unwrap_or("").to_string(),
                        title: extract_plain_text(&d["title"]),
                        url: d["url"].as_str().unwrap_or("").to_string(),
                        created_time: d["created_time"].as_str().unwrap_or("").to_string(),
                        last_edited_time: d["last_edited_time"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Fetches the text content of a single page (block tree, depth-1).
    pub async fn fetch_page_content(
        &self,
        token: &OAuthToken,
        page_id: &str,
    ) -> Result<String> {
        let url = format!("{NOTION_API_BASE}/blocks/{page_id}/children");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(format!("secret_{}", token.access_token))
            .header("Notion-Version", "2022-06-28")
            .query(&[("page_size", &"100".to_string())])
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        let mut text = String::new();
        if let Some(results) = json["results"].as_array() {
            for block in results {
                if let Some(rich_text) = block["paragraph"]["rich_text"].as_array() {
                    for rt in rich_text {
                        if let Some(s) = rt["plain_text"].as_str() {
                            text.push_str(s);
                            text.push('\n');
                        }
                    }
                }
            }
        }
        Ok(text)
    }

    /// Appends a paragraph block to a Notion page (push direction).
    pub async fn append_to_page(
        &self,
        token: &OAuthToken,
        page_id: &str,
        content: &str,
    ) -> Result<()> {
        let url = format!("{NOTION_API_BASE}/blocks/{page_id}/children");
        let body = serde_json::json!({
            "children": [{
                "object": "block",
                "type": "paragraph",
                "paragraph": {
                    "rich_text": [{
                        "type": "text",
                        "text": { "content": content }
                    }]
                }
            }]
        });
        let resp = self
            .http
            .patch(&url)
            .bearer_auth(format!("secret_{}", token.access_token))
            .header("Notion-Version", "2022-06-28")
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let _ = resp.bytes().await?;
        Ok(())
    }

    /// Bidirectional sync: fetch from Notion, push back to Notion.
    pub async fn sync_bidirectional(
        &self,
        token: &OAuthToken,
        push_updates: &[(String, String)],
    ) -> Result<BiSyncResult> {
        let databases = self.fetch_databases(token, 50).await.unwrap_or_default();

        // Push direction: append each update to its target page.
        let mut pushed = 0;
        let mut push_errors = 0;
        for (page_id, content) in push_updates {
            match self.append_to_page(token, page_id, content).await {
                Ok(_) => pushed += 1,
                Err(e) => {
                    tracing::warn!(
                        target: "nebula.oauth.notion",
                        page_id = %page_id,
                        error = %e,
                        "push failed"
                    );
                    push_errors += 1;
                }
            }
        }

        Ok(BiSyncResult {
            databases_fetched: databases.len(),
            pages_pushed: pushed,
            push_errors,
        })
    }
}

#[async_trait]
impl OAuthProvider for NotionOAuthProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn config(&self) -> &OAuthProviderConfig {
        &self.config
    }

    async fn exchange_code(&self, code: &str) -> Result<OAuthToken> {
        // Notion uses HTTP Basic auth with client_id:client_secret for the
        // token exchange, not form params.
        let creds = format!(
            "{}:{}",
            self.config.client_id,
            self.client_secret.as_deref().unwrap_or("")
        );
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(creds);
        let body = vec![(
            "grant_type".to_string(),
            "authorization_code".to_string(),
        ), (
            "code".to_string(),
            code.to_string(),
        ), (
            "redirect_uri".to_string(),
            self.config.redirect_uri.clone(),
        )];
        let resp = self
            .http
            .post(&self.config.token_url)
            .header("Authorization", format!("Basic {b64}"))
            .form(&body)
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        // Notion wraps the token in an "access_token" field that already
        // includes the "secret_" prefix; reuse the standard parser.
        parse_token_response(&json).context("Notion token exchange failed")
    }

    async fn refresh_token(&self, _refresh: &str) -> Result<OAuthToken> {
        // Notion public integrations don't issue refresh tokens; the
        // access token is valid until the user revokes the integration.
        anyhow::bail!("Notion does not support token refresh; re-authorize the integration")
    }

    async fn revoke_token(&self, _token: &str) -> Result<()> {
        // Notion has no per-token revoke endpoint.
        tracing::info!(
            target: "nebula.oauth.notion",
            "Notion tokens cannot be revoked per-token; user must revoke integration in workspace settings"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSummary {
    pub id: String,
    pub title: String,
    pub url: String,
    pub created_time: String,
    pub last_edited_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiSyncResult {
    pub databases_fetched: usize,
    pub pages_pushed: usize,
    pub push_errors: usize,
}

/// Notion's rich text arrays concat multiple `plain_text` fields.
fn extract_plain_text(value: &serde_json::Value) -> String {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|rt| rt["plain_text"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_is_notion() {
        let p = NotionOAuthProvider::new("cid".into(), None, "http://localhost".into());
        assert_eq!(p.id(), "notion");
    }

    #[test]
    fn extract_plain_text_concats() {
        let v = serde_json::json!([
            { "plain_text": "Hello " },
            { "plain_text": "World" }
        ]);
        assert_eq!(extract_plain_text(&v), "Hello World");
    }

    #[test]
    fn extract_plain_text_empty_array() {
        let v = serde_json::json!([]);
        assert_eq!(extract_plain_text(&v), "");
    }
}
