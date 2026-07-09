//! T-E-C-18: Google OAuth provider(支持 PKCE + 刷新 + 撤销)。
//!
//! 使用 Google Identity Services 的 OAuth 2.0 v2 端点。PKCE 适用于公开客户端
//! (桌面应用无法安全保管 client_secret)。scope 为只读(userInfo + readonly API)。

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{build_authorization_url, build_code_exchange_body, build_refresh_body};
use crate::oauth::{OAuthProvider, ProviderConfig, TokenSet};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Google OAuth provider。
pub struct GoogleProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl GoogleProvider {
    /// 创建 Google provider。
    ///
    /// `client_secret` 在纯 PKCE 流程下可为 `None`(公开客户端)。
    /// 若使用机密客户端则需提供。
    pub fn new(client_id: String, client_secret: Option<String>, redirect_uri: String) -> Self {
        let config = ProviderConfig {
            id: "google".to_string(),
            name: "Google".to_string(),
            client_id,
            client_secret,
            redirect_uri,
            auth_url: GOOGLE_AUTH_URL.to_string(),
            token_url: GOOGLE_TOKEN_URL.to_string(),
            revoke_url: Some(GOOGLE_REVOKE_URL.to_string()),
            // 只读 scope:用户信息 + Gmail 只读 + Calendar 只读 + Drive 元数据只读。
            scopes: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
                "https://www.googleapis.com/auth/gmail.readonly".to_string(),
                "https://www.googleapis.com/auth/calendar.readonly".to_string(),
                "https://www.googleapis.com/auth/drive.metadata.readonly".to_string(),
            ],
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http }
    }

    /// 从环境变量构造。
    pub fn from_env(redirect_uri: &str) -> Option<Self> {
        let client_id = std::env::var("NEBULA_GOOGLE_CLIENT_ID").ok()?;
        let client_secret = std::env::var("NEBULA_GOOGLE_CLIENT_SECRET").ok();
        Some(Self::new(
            client_id,
            client_secret,
            redirect_uri.to_string(),
        ))
    }
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn supports_pkce(&self) -> bool {
        true
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn authorize(&self, state: &str, pkce_challenge: Option<&str>) -> String {
        // Google 要求 access_type=offline 才返回 refresh_token,prompt=consent 强制每次授权。
        let mut url = build_authorization_url(&self.config, state, pkce_challenge);
        url.push_str("&access_type=offline&prompt=consent");
        url
    }

    async fn callback(&self, code: &str, pkce_verifier: Option<&str>) -> Result<TokenSet> {
        let body = build_code_exchange_body(
            &self.config,
            code,
            pkce_verifier,
            self.config.client_secret.as_deref(),
        );
        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&body)
            .send()
            .await
            .context("Google token 交换请求失败")?
            .error_for_status()
            .context("Google token 交换返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 Google 响应失败")?;
        TokenSet::from_response(&json).context("Google token 交换失败")
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenSet> {
        let body = build_refresh_body(
            refresh_token,
            &self.config,
            self.config.client_secret.as_deref(),
        );
        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&body)
            .send()
            .await
            .context("Google token 刷新请求失败")?
            .error_for_status()
            .context("Google token 刷新返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 Google 刷新响应失败")?;
        // Google 刷新响应可能不含 refresh_token(复用原 token)。
        let mut token = TokenSet::from_response(&json).context("Google token 刷新失败")?;
        if token.refresh_token.is_none() {
            token.refresh_token = Some(refresh_token.to_string());
        }
        Ok(token)
    }

    async fn revoke(&self, token: &str) -> Result<()> {
        // Google 撤销端点:POST https://oauth2.googleapis.com/revoke?token=...
        let url = format!("{}?token={}", GOOGLE_REVOKE_URL, token);
        let resp = self
            .http
            .post(&url)
            .send()
            .await
            .context("Google 撤销请求失败")?;
        if !resp.status().is_success() {
            tracing::warn!(
                target: "nebula.oauth.google",
                status = %resp.status(),
                "Google 撤销返回非 2xx(非致命)"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_name_pkce() {
        let p = GoogleProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert_eq!(p.id(), "google");
        assert_eq!(p.name(), "Google");
        assert!(p.supports_pkce());
    }

    #[test]
    fn authorize_url_includes_pkce_and_offline() {
        let p = GoogleProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let url = p.authorize("st", Some("challengeABC"));
        assert!(url.contains("code_challenge=challengeABC"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
    }

    #[test]
    fn scopes_are_readonly() {
        let p = GoogleProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let joined = p.config().scopes.join(" ");
        assert!(joined.contains("gmail.readonly"));
        assert!(joined.contains("calendar.readonly"));
        // 不含写入 scope。
        assert!(!joined.contains(".modify"));
        assert!(!joined.contains(".full"));
    }
}
