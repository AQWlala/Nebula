//! T-E-C-18: GitHub OAuth provider(pull-only,为 T-E-L-04 准备)。
//!
//! GitHub OAuth App 流程:**不支持 PKCE**(仅 GitHub App 支持),token 不过期。
//! scope 为 `repo user read:org`,遵循 Loop Engineering Connectors 的
//! 只读拉取语义——拉取用户 repos / issues / events,不回写。

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{build_authorization_url, build_code_exchange_body};
use crate::oauth::{OAuthProvider, ProviderConfig, TokenSet};

const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// GitHub OAuth provider。
pub struct GitHubProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl GitHubProvider {
    /// 创建 GitHub provider。
    ///
    /// `client_id` / `client_secret` 从环境变量 `NEBULA_GITHUB_CLIENT_ID` /
    /// `NEBULA_GITHUB_CLIENT_SECRET` 读取(或由调用方显式传入)。GitHub OAuth App
    /// 的 token 交换需要 client_secret。
    pub fn new(client_id: String, client_secret: Option<String>, redirect_uri: String) -> Self {
        let config = ProviderConfig {
            id: "github".to_string(),
            name: "GitHub".to_string(),
            client_id,
            client_secret,
            redirect_uri,
            auth_url: GITHUB_AUTH_URL.to_string(),
            token_url: GITHUB_TOKEN_URL.to_string(),
            revoke_url: None, // GitHub 无 per-token 撤销端点。
            scopes: vec![
                "repo".to_string(),     // 拉取仓库(repo + issues + PRs)
                "user".to_string(),     // 用户信息
                "read:org".to_string(), // 组织信息(只读)
            ],
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("nebula/2.0 oauth-github")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http }
    }

    /// 从环境变量构造(便捷方法)。
    pub fn from_env(redirect_uri: &str) -> Option<Self> {
        let client_id = std::env::var("NEBULA_GITHUB_CLIENT_ID").ok()?;
        let client_secret = std::env::var("NEBULA_GITHUB_CLIENT_SECRET").ok();
        Some(Self::new(
            client_id,
            client_secret,
            redirect_uri.to_string(),
        ))
    }
}

#[async_trait]
impl OAuthProvider for GitHubProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_pkce(&self) -> bool {
        false
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn authorize(&self, state: &str, _pkce_challenge: Option<&str>) -> String {
        build_authorization_url(&self.config, state, None)
    }

    async fn callback(&self, code: &str, _pkce_verifier: Option<&str>) -> Result<TokenSet> {
        let body = build_code_exchange_body(
            &self.config,
            code,
            None,
            self.config.client_secret.as_deref(),
        );
        let resp = self
            .http
            .post(&self.config.token_url)
            .header("Accept", "application/json")
            .form(&body)
            .send()
            .await
            .context("GitHub token 交换请求失败")?
            .error_for_status()
            .context("GitHub token 交换返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 GitHub 响应失败")?;
        TokenSet::from_response(&json).context("GitHub token 交换失败")
    }

    async fn refresh(&self, _refresh_token: &str) -> Result<TokenSet> {
        // GitHub OAuth token 不过期,无需刷新。过期 / 被撤销需重新授权。
        anyhow::bail!("GitHub token 不过期,无需刷新;如已失效请重新授权")
    }

    async fn revoke(&self, _token: &str) -> Result<()> {
        // GitHub 无 per-token 撤销端点;用户须在 GitHub Settings 撤销 App 访问。
        tracing::info!(
            target: "nebula.oauth.github",
            "GitHub 无 per-token 撤销端点,请在 GitHub Settings 撤销 App 访问"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_name() {
        let p = GitHubProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert_eq!(p.id(), "github");
        assert_eq!(p.name(), "GitHub");
        assert!(!p.supports_pkce());
    }

    #[test]
    fn authorize_url_has_scopes_and_state() {
        let p = GitHubProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let url = p.authorize("stateXYZ", None);
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("state=stateXYZ"));
        assert!(url.contains("scope=repo%20user%20read%3Aorg"));
        // GitHub 不支持 PKCE,URL 中不应出现 code_challenge。
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn config_has_three_scopes() {
        let p = GitHubProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert_eq!(p.config().scopes, vec!["repo", "user", "read:org"]);
    }
}
