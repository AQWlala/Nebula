//! T-E-C-18: Slack OAuth provider(pull-only)。
//!
//! Slack OAuth v2 流程:**不支持 PKCE**,token 不过期。scope 为只读
//! (channels / groups / im / mpim 的 :read),遵循 Connectors 只读语义。

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{build_authorization_url, build_code_exchange_body};
use crate::oauth::{OAuthProvider, ProviderConfig, TokenSet};

const SLACK_AUTH_URL: &str = "https://slack.com/oauth/v2/authorize";
const SLACK_TOKEN_URL: &str = "https://slack.com/api/oauth.v2.access";

/// Slack OAuth provider。
pub struct SlackProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl SlackProvider {
    /// 创建 Slack provider。
    pub fn new(client_id: String, client_secret: Option<String>, redirect_uri: String) -> Self {
        let config = ProviderConfig {
            id: "slack".to_string(),
            name: "Slack".to_string(),
            client_id,
            client_secret,
            redirect_uri,
            auth_url: SLACK_AUTH_URL.to_string(),
            token_url: SLACK_TOKEN_URL.to_string(),
            revoke_url: None, // Slack 无公共 per-token 撤销端点(需通过 admin 或 apps.uninstall)。
            // 只读 scope:频道 / 群组 / 私聊 / 多人私聊的读取权限。
            scopes: vec![
                "channels:read".to_string(),
                "groups:read".to_string(),
                "im:read".to_string(),
                "mpim:read".to_string(),
                "users:read".to_string(),
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
        let client_id = std::env::var("NEBULA_SLACK_CLIENT_ID").ok()?;
        let client_secret = std::env::var("NEBULA_SLACK_CLIENT_SECRET").ok();
        Some(Self::new(
            client_id,
            client_secret,
            redirect_uri.to_string(),
        ))
    }
}

#[async_trait]
impl OAuthProvider for SlackProvider {
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
        // Slack oauth.v2.access 需要 client_id + client_secret 作为 form 参数。
        let body = build_code_exchange_body(
            &self.config,
            code,
            None,
            self.config.client_secret.as_deref(),
        );
        let resp = self
            .http
            .post(&self.config.token_url)
            .form(&body)
            .send()
            .await
            .context("Slack token 交换请求失败")?
            .error_for_status()
            .context("Slack token 交换返回非 2xx")?;
        let json: serde_json::Value = resp.json().await.context("解析 Slack 响应失败")?;
        // Slack 响应用 ok=true/false 表示成功,access_token 在 authed_user.access_token。
        if json["ok"].as_bool() != Some(true) {
            let err = json["error"].as_str().unwrap_or("unknown");
            anyhow::bail!("Slack token 交换失败: {err}");
        }
        // 提取 authed_user.access_token 到标准位置。
        let user_token = json["authed_user"]["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Slack 响应缺少 authed_user.access_token"))?;
        let normalized = serde_json::json!({
            "access_token": user_token,
            "token_type": "Bearer",
            "scope": json["authed_user"]["scope"].as_str().unwrap_or(""),
        });
        TokenSet::from_response(&normalized).context("Slack token 交换失败")
    }

    async fn refresh(&self, _refresh_token: &str) -> Result<TokenSet> {
        // Slack bot / user token 不过期,无需刷新。
        anyhow::bail!("Slack token 不过期,无需刷新;如已失效请重新授权")
    }

    async fn revoke(&self, _token: &str) -> Result<()> {
        // Slack 无公共 per-token 撤销端点;可通过 auth.revoke API 或卸载 App。
        tracing::info!(
            target: "nebula.oauth.slack",
            "Slack 无公共 per-token 撤销端点,请通过 Slack App 管理页面卸载应用"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_name_no_pkce() {
        let p = SlackProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        assert_eq!(p.id(), "slack");
        assert_eq!(p.name(), "Slack");
        assert!(!p.supports_pkce());
    }

    #[test]
    fn authorize_url_has_readonly_scopes() {
        let p = SlackProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        let url = p.authorize("st", None);
        assert!(url.contains("scope=channels%3Aread"));
        assert!(url.contains("state=st"));
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn scopes_all_read() {
        let p = SlackProvider::new("cid".into(), None, "http://127.0.0.1:1/callback".into());
        for s in &p.config().scopes {
            assert!(s.ends_with(":read"), "scope {s} 应为只读(:read)");
        }
    }
}
