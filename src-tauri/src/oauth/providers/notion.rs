//! T-E-C-18: Notion OAuth provider(pull-only)。
//!
//! Notion 公共集成流程:**不支持 PKCE**,token 不过期。token 交换使用
//! HTTP Basic auth(`client_id:client_secret`),而非 form 参数传递 secret。

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;

use super::build_authorization_url;
use crate::oauth::{OAuthProvider, ProviderConfig, TokenSet};

const NOTION_AUTH_URL: &str = "https://api.notion.com/v1/oauth/authorize";
const NOTION_TOKEN_URL: &str = "https://api.notion.com/v1/oauth/token";

/// Notion OAuth provider。
pub struct NotionProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl NotionProvider {
    /// 创建 Notion provider。
    ///
    /// Notion token 交换**需要** client_secret(用 HTTP Basic auth 传输)。
    pub fn new(client_id: String, client_secret: Option<String>, redirect_uri: String) -> Self {
        let config = ProviderConfig {
            id: "notion".to_string(),
            name: "Notion".to_string(),
            client_id,
            client_secret,
            redirect_uri,
            auth_url: NOTION_AUTH_URL.to_string(),
            token_url: NOTION_TOKEN_URL.to_string(),
            revoke_url: None, // Notion 无 per-token 撤销端点(在工作区设置移除集成)。
            // Notion scope 由工作区授予,OAuth 流程不请求细粒度 scope。
            scopes: vec![],
        };
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, http }
    }

    /// 从环境变量构造。
    pub fn from_env(redirect_uri: &str) -> Option<Self> {
        let client_id = std::env::var("NEBULA_NOTION_CLIENT_ID").ok()?;
        let client_secret = std::env::var("NEBULA_NOTION_CLIENT_SECRET").ok();
        Some(Self::new(
            client_id,
            client_secret,
            redirect_uri.to_string(),
        ))
    }
}

#[async_trait]
impl OAuthProvider for NotionProvider {
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
        // Notion 要求 owner=user 参数(指定为用户身份授权)。
        let mut url = build_authorization_url(&self.config, state, None);
        url.push_str("&owner=user");
        url
    }

    async fn callback(&self, code: &str, _pkce_verifier: Option<&str>) -> Result<TokenSet> {
        // Notion 用 HTTP Basic auth:Authorization: Basic base64(client_id:client_secret)。
        let creds = format!(
            "{}:{}",
            self.config.client_id,
            self.config.client_secret.as_deref().unwrap_or("")
        );
        let b64 = base64::engine::general_purpose::STANDARD.encode(creds);
        let body = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), self.config.redirect_uri.clone()),
        ];
        let resp = self
            .http
            .post(&self.config.token_url)
            .header("Authorization", format!("Basic {b64}"))
            .form(&body)
            .send()
            .await
            .context("Notion token 交换请求失败")?
            .error_for_status()
            .context("Notion token 交换返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 Notion 响应失败")?;
        // Notion 的 access_token 已含 "secret_" 前缀,直接用标准解析器。
        TokenSet::from_response(&json).context("Notion token 交换失败")
    }

    async fn refresh(&self, _refresh_token: &str) -> Result<TokenSet> {
        // Notion 公共集成不签发 refresh_token,token 不过期。
        anyhow::bail!("Notion token 不过期,无需刷新;如已失效请在工作区重新授权")
    }

    async fn revoke(&self, _token: &str) -> Result<()> {
        // Notion 无 per-token 撤销端点;用户须在工作区设置移除集成。
        tracing::info!(
            target: "nebula.oauth.notion",
            "Notion 无 per-token 撤销端点,请在工作区设置移除集成"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_name_no_pkce() {
        let p = NotionProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert_eq!(p.id(), "notion");
        assert_eq!(p.name(), "Notion");
        assert!(!p.supports_pkce());
    }

    #[test]
    fn authorize_url_has_owner_user() {
        let p = NotionProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let url = p.authorize("st", None);
        assert!(url.contains("owner=user"));
        assert!(url.contains("state=st"));
        assert!(url.contains("client_id=cid"));
    }

    #[test]
    fn scopes_empty_for_notion() {
        let p = NotionProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert!(p.config().scopes.is_empty(), "Notion scope 由工作区授予");
    }
}
