//! T-E-C-18: Microsoft OAuth provider(Azure AD v2.0 + PKCE)。
//!
//! 使用 Microsoft identity platform 的 `/common` 多租户端点,支持任意 Azure AD
//! 租户 + 个人 Microsoft 账户。PKCE 为公开客户端推荐做法。scope 使用
//! Microsoft Graph 的只读委派权限。

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{build_authorization_url, build_code_exchange_body, build_refresh_body};
use crate::oauth::{OAuthProvider, ProviderConfig, TokenSet};

const MS_AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MS_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

/// Microsoft OAuth provider。
pub struct MicrosoftProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl MicrosoftProvider {
    /// 创建 Microsoft provider。
    ///
    /// Microsoft 公开客户端(桌面应用)推荐用 PKCE 而非 client_secret。
    pub fn new(client_id: String, client_secret: Option<String>, redirect_uri: String) -> Self {
        let config = ProviderConfig {
            id: "microsoft".to_string(),
            name: "Microsoft".to_string(),
            client_id,
            client_secret,
            redirect_uri,
            auth_url: MS_AUTH_URL.to_string(),
            token_url: MS_TOKEN_URL.to_string(),
            revoke_url: None, // Microsoft 无公共 per-token 撤销端点(需通过企业管理)。
            // Microsoft Graph 只读委派权限。
            scopes: vec![
                "User.Read".to_string(),
                "Mail.Read".to_string(),
                "Calendars.Read".to_string(),
                "Files.Read".to_string(),
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
        let client_id = std::env::var("NEBULA_MICROSOFT_CLIENT_ID").ok()?;
        let client_secret = std::env::var("NEBULA_MICROSOFT_CLIENT_SECRET").ok();
        Some(Self::new(
            client_id,
            client_secret,
            redirect_uri.to_string(),
        ))
    }
}

#[async_trait]
impl OAuthProvider for MicrosoftProvider {
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
        build_authorization_url(&self.config, state, pkce_challenge)
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
            .context("Microsoft token 交换请求失败")?
            .error_for_status()
            .context("Microsoft token 交换返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 Microsoft 响应失败")?;
        TokenSet::from_response(&json).context("Microsoft token 交换失败")
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
            .context("Microsoft token 刷新请求失败")?
            .error_for_status()
            .context("Microsoft token 刷新返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 Microsoft 刷新响应失败")?;
        let mut token = TokenSet::from_response(&json).context("Microsoft token 刷新失败")?;
        if token.refresh_token.is_none() {
            token.refresh_token = Some(refresh_token.to_string());
        }
        Ok(token)
    }

    async fn revoke(&self, _token: &str) -> Result<()> {
        // Microsoft identity platform 不提供公共 per-token 民销端点;
        // 撤销需通过 Azure 门户或 PowerShell(End-AzureADSession 等)。
        tracing::info!(
            target: "nebula.oauth.microsoft",
            "Microsoft 无公共 per-token 撤销端点,请在 Azure 门户管理应用权限"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_name_pkce() {
        let p = MicrosoftProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert_eq!(p.id(), "microsoft");
        assert_eq!(p.name(), "Microsoft");
        assert!(p.supports_pkce());
    }

    #[test]
    fn authorize_url_with_pkce() {
        let p = MicrosoftProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let url = p.authorize("st99", Some("ch777"));
        assert!(url.contains("code_challenge=ch777"));
        assert!(url.contains("state=st99"));
        assert!(url.contains("client_id=cid"));
    }

    #[test]
    fn scopes_are_read_delegated() {
        let p = MicrosoftProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let joined = p.config().scopes.join(" ");
        assert!(joined.contains("Mail.Read"));
        assert!(joined.contains("Calendars.Read"));
        // 不含 ReadWrite 权限。
        assert!(!joined.contains("ReadWrite"));
    }
}
