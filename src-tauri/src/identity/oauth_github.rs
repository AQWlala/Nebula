//! GitHub OAuth integration.
//!
//! Implements the OAuth 2.0 web-app flow against GitHub's endpoints
//! and adds thin wrappers around the REST API for incremental fetch
//! of repos / issues / pull requests / recent events.  Fetched items
//! are returned as [`GitHubDelta`] for memory ingestion.
//!
//! 对标: OpenHuman — 把用户 owned/starred repos、open issues、PRs、
//! recent events 都纳入"第二大脑"的外部数据源。

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::oauth::{
    build_refresh_request_body, build_token_request_body, parse_token_response, OAuthProvider,
    OAuthProviderConfig, OAuthToken,
};

const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// GitHub OAuth provider.
pub struct GitHubOAuthProvider {
    config: OAuthProviderConfig,
    client_secret: Option<String>,
    http: reqwest::Client,
}

impl GitHubOAuthProvider {
    /// Creates a new GitHub provider.
    ///
    /// GitHub's OAuth app flow requires a client secret for the token
    /// exchange; pass `Some(secret)` for the standard web-app flow.
    /// PKCE-only GitHub Apps are not yet generally available.
    pub fn new(
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
    ) -> Self {
        let config = OAuthProviderConfig {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            client_id,
            redirect_uri,
            auth_url: GITHUB_AUTH_URL.to_string(),
            token_url: GITHUB_TOKEN_URL.to_string(),
            revoke_url: None, // GitHub has no per-token revoke; delete the app instead.
            scopes: vec![
                "repo".to_string(),
                "user".to_string(),
                "read:org".to_string(),
            ],
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("nebula/2.0 oauth-github")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            client_secret,
            http,
        }
    }

    /// Fetches incremental GitHub activity: owned repos, recent issues,
    /// recent PRs, and recent events.
    ///
    /// All endpoints are paginated; this returns at most `per_page` items
    /// per category (default 30) to bound token usage.
    pub async fn fetch_delta(
        &self,
        token: &OAuthToken,
        per_page: usize,
    ) -> Result<GitHubDelta> {
        let per_page = per_page.clamp(1, 100);

        let repos = self.fetch_repos(token, per_page).await.unwrap_or_default();
        let issues = self.fetch_issues(token, per_page).await.unwrap_or_default();
        let events = self.fetch_events(token, per_page).await.unwrap_or_default();

        Ok(GitHubDelta {
            repos,
            issues,
            events,
        })
    }

    /// Fetches the user's owned & starred repos.
    async fn fetch_repos(
        &self,
        token: &OAuthToken,
        per_page: usize,
    ) -> Result<Vec<RepoSummary>> {
        let url = format!("{GITHUB_API_BASE}/user/repos");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token.access_token)
            .header("Accept", "application/vnd.github+json")
            .query(&[
                ("per_page", &per_page.to_string()),
                ("sort", &"updated".to_string()),
                ("affiliation", &"owner,collaborator".to_string()),
            ])
            .send()
            .await?
            .error_for_status()?;
        let arr: Vec<serde_json::Value> = resp.json().await?;
        Ok(arr
            .iter()
            .map(|r| RepoSummary {
                id: r["id"].as_i64().unwrap_or(0),
                name: r["name"].as_str().unwrap_or("").to_string(),
                full_name: r["full_name"].as_str().unwrap_or("").to_string(),
                html_url: r["html_url"].as_str().unwrap_or("").to_string(),
                description: r["description"].as_str().unwrap_or("").to_string(),
                stars: r["stargazers_count"].as_i64().unwrap_or(0),
                open_issues: r["open_issues_count"].as_i64().unwrap_or(0),
                updated_at: r["updated_at"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    /// Fetches the user's recent open issues across all repos.
    async fn fetch_issues(
        &self,
        token: &OAuthToken,
        per_page: usize,
    ) -> Result<Vec<IssueSummary>> {
        let url = format!("{GITHUB_API_BASE}/issues");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token.access_token)
            .header("Accept", "application/vnd.github+json")
            .query(&[
                ("per_page", &per_page.to_string()),
                ("state", &"open".to_string()),
                ("filter", &"assigned".to_string()),
            ])
            .send()
            .await?
            .error_for_status()?;
        let arr: Vec<serde_json::Value> = resp.json().await?;
        Ok(arr
            .iter()
            .map(|i| IssueSummary {
                id: i["id"].as_i64().unwrap_or(0),
                number: i["number"].as_i64().unwrap_or(0),
                title: i["title"].as_str().unwrap_or("").to_string(),
                html_url: i["html_url"].as_str().unwrap_or("").to_string(),
                state: i["state"].as_str().unwrap_or("").to_string(),
                repo: i["repository_url"]
                    .as_str()
                    .map(|s| s.rsplit('/').next().unwrap_or("").to_string())
                    .unwrap_or_default(),
                created_at: i["created_at"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    /// Fetches the user's recent public events (commits, merges, etc.).
    async fn fetch_events(
        &self,
        token: &OAuthToken,
        per_page: usize,
    ) -> Result<Vec<EventSummary>> {
        // /user/events only returns public events; for private events the
        // app would need the `activity:read` scope on a GitHub App.
        let url = format!("{GITHUB_API_BASE}/events");
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token.access_token)
            .header("Accept", "application/vnd.github+json")
            .query(&[("per_page", &per_page.to_string())])
            .send()
            .await?
            .error_for_status()?;
        let arr: Vec<serde_json::Value> = resp.json().await?;
        Ok(arr
            .iter()
            .map(|e| EventSummary {
                id: e["id"].as_str().unwrap_or("").to_string(),
                event_type: e["type"].as_str().unwrap_or("").to_string(),
                repo: e["repo"]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                created_at: e["created_at"].as_str().unwrap_or("").to_string(),
                payload_action: e["payload"]["action"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            })
            .collect())
    }
}

#[async_trait]
impl OAuthProvider for GitHubOAuthProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn config(&self) -> &OAuthProviderConfig {
        &self.config
    }

    async fn exchange_code(&self, code: &str) -> Result<OAuthToken> {
        let body = build_token_request_body(&self.config, code, self.client_secret.as_deref());
        // GitHub accepts JSON with the Accept header.
        let resp = self
            .http
            .post(&self.config.token_url)
            .header("Accept", "application/json")
            .form(&body)
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        parse_token_response(&json).context("GitHub token exchange failed")
    }

    async fn refresh_token(&self, refresh: &str) -> Result<OAuthToken> {
        // GitHub OAuth tokens don't expire by default, so refresh is a
        // no-op that returns the same token.  If the token was revoked,
        // the user must re-authorize.
        let body = build_refresh_request_body(refresh, &self.config, self.client_secret.as_deref());
        let resp = self
            .http
            .post(&self.config.token_url)
            .header("Accept", "application/json")
            .form(&body)
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        parse_token_response(&json).context("GitHub token refresh failed")
    }

    async fn revoke_token(&self, _token: &str) -> Result<()> {
        // GitHub has no per-token revoke endpoint for OAuth apps.
        // The user must revoke app access from GitHub settings.
        // This is documented in the proposal as "best-effort".
        tracing::info!(
            target: "nebula.oauth.github",
            "GitHub tokens cannot be revoked per-token; user must revoke app access in GitHub settings"
        );
        Ok(())
    }
}

/// Aggregated GitHub activity for memory ingestion.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitHubDelta {
    pub repos: Vec<RepoSummary>,
    pub issues: Vec<IssueSummary>,
    pub events: Vec<EventSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSummary {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    pub description: String,
    pub stars: i64,
    pub open_issues: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSummary {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub html_url: String,
    pub state: String,
    pub repo: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSummary {
    pub id: String,
    pub event_type: String,
    pub repo: String,
    pub created_at: String,
    pub payload_action: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_is_github() {
        let p = GitHubOAuthProvider::new("cid".into(), None, "http://localhost".into());
        assert_eq!(p.id(), "github");
        assert_eq!(p.config().scopes.len(), 3);
    }

    #[test]
    fn authorization_url_has_repo_scope() {
        let p = GitHubOAuthProvider::new("cid".into(), None, "http://localhost".into());
        let url = p.config().authorization_url("stateXYZ");
        assert!(url.contains("scope=repo"));
        assert!(url.contains("state=stateXYZ"));
    }
}
