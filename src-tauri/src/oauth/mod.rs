//! T-E-C-18: OAuth 集成层（5 服务）。
//!
//! 独立于 `identity::oauth` 的全新集成层,补充了后者缺失的三个能力:
//!
//! * **PKCE** — RFC 7636 授权码扩展,避免机密 client_secret 泄露(见 [`pkce`] 模块)。
//! * **本地回调服务器** — [`callback_server`] 在 `127.0.0.1` 临时端口上接收
//!   provider 重定向,通过 oneshot channel 把授权码送回等待中的 [`OAuthManager`]。
//! * **加密 SQLite token 存储** — [`token_store`] 用 AES-256-GCM 加密 token JSON
//!   后写入独立 SQLite 文件,密钥从 OS keychain 派生(见 [`token_store`] 文档)。
//!
//! ## 5 个服务
//!
//! | Provider   | PKCE | 刷新 | 备注 |
//! |------------|------|------|------|
//! | GitHub     | ✗    | ✗    | OAuth App 不支持 PKCE;token 不过期(pull-only,为 T-E-L-04 准备) |
//! | Google     | ✓    | ✓    | 标准 OAuth 2.0 + PKCE |
//! | Microsoft  | ✓    | ✓    | Azure AD v2.0 endpoint + PKCE |
//! | Slack      | ✗    | ✗    | Slack OAuth v2 不支持 PKCE;token 不过期 |
//! | Notion     | ✗    | ✗    | 公共集成不支持 PKCE;token 不过期 |
//!
//! ## 设计原则
//!
//! * **零新依赖** — 全部使用 Cargo.toml 已有声明的 crate(reqwest / tokio / rusqlite /
//!   aes-gcm / sha2 / base64 / rand 等)。
//! * **不修改** Cargo.toml / tauri_setup.rs / commands/mod.rs / ROADMAP_v3.1.md。
//! * **pull-only** — Loop Engineering Connectors 语义:只读拉取用户数据,不回写。

pub mod callback_server;
pub mod manager;
pub mod pkce;
pub mod providers;
pub mod token_store;

pub use manager::{OAuthManager, ProviderInfo};
pub use pkce::{PkceChallenge, PkcePair};
pub use token_store::{StoredToken, TokenStore};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// OAuth 授权码流程拿回的 token 集合。
///
/// 对应 RFC 6749 §4.1.4 的 Access Token Response,额外保留 `token_type`
/// 供 bearer 以外的方案使用(目前所有 provider 均为 `"Bearer"`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    /// 访问令牌,调用 provider API 时放入 `Authorization: Bearer <access_token>`。
    pub access_token: String,
    /// 刷新令牌(GitHub / Slack / Notion 不过期 token 无此字段)。
    pub refresh_token: Option<String>,
    /// 绝对过期时间(UTC)。GitHub / Slack 等不过期 token 会被 provider 设为
    /// 一个足够远的未来时间(如 100 年后),`is_expired` 恒返回 false。
    pub expires_at: DateTime<Utc>,
    /// 服务端实际授予的 scope(空格分隔)。可能与请求 scope 不同(服务端可缩减)。
    pub scope: String,
    /// token 类型,目前所有 provider 均为 `"Bearer"`。
    pub token_type: String,
}

impl TokenSet {
    /// 判断 token 是否已过期(留 60 秒安全余量,避免边界竞态)。
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at - chrono::Duration::seconds(60)
    }

    /// 从标准 RFC 6749 token 响应 JSON 解析。
    ///
    /// `expires_in`(秒)被换算成绝对时间 `expires_at`。无 `expires_in` 时
    /// 视为不过期,设为 100 年后(GitHub / Slack 语义)。
    pub fn from_response(json: &serde_json::Value) -> anyhow::Result<Self> {
        let access_token = json["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("token 响应缺少 access_token 字段"))?
            .to_string();
        let refresh_token = json["refresh_token"].as_str().map(|s| s.to_string());
        let scope = json["scope"].as_str().unwrap_or("").to_string();
        let token_type = json["token_type"].as_str().unwrap_or("Bearer").to_string();
        let expires_at = match json["expires_in"].as_u64() {
            Some(secs) => Utc::now() + chrono::Duration::seconds(secs as i64),
            None => Utc::now() + chrono::Duration::days(365 * 100), // 不过期
        };
        Ok(Self {
            access_token,
            refresh_token,
            expires_at,
            scope,
            token_type,
        })
    }
}

/// 提供者静态配置(端点 URL、scope、client 凭据)。
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// 提供者唯一标识(如 `"github"`)。
    pub id: String,
    /// 人类可读名称(如 `"GitHub"`)。
    pub name: String,
    /// OAuth client id。
    pub client_id: String,
    /// OAuth client secret(PKCE 流程可留 None)。
    pub client_secret: Option<String>,
    /// 重定向 URI,必须与 provider 控制台配置一致(通常为 `http://127.0.0.1:<port>/callback`)。
    pub redirect_uri: String,
    /// 授权端点 URL。
    pub auth_url: String,
    /// token 交换端点 URL。
    pub token_url: String,
    /// 撤销端点 URL(部分 provider 无)。
    pub revoke_url: Option<String>,
    /// 请求的 scope 列表。
    pub scopes: Vec<String>,
}

/// T-E-C-18: OAuth 提供者 trait。
///
/// 每个具体服务(GitHub / Google / Microsoft / Slack / Notion)实现此 trait。
/// 方法命名与任务要求一致:**authorize / callback / refresh / revoke**。
///
/// * `authorize` — 构建授权 URL(含 state + 可选 PKCE challenge),由 manager 打开浏览器。
/// * `callback` — 处理回调,用授权码交换 token(可带 PKCE verifier)。
/// * `refresh` — 用 refresh_token 换取新 token。
/// * `revoke` — 撤销 token(best-effort,失败仅记录日志)。
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    /// 提供者唯一标识。
    fn id(&self) -> &str;
    /// 人类可读名称。
    fn name(&self) -> &str;
    /// 是否支持 PKCE(GitHub / Slack / Notion 返回 false)。
    fn supports_pkce(&self) -> bool {
        false
    }
    /// 静态配置引用。
    fn config(&self) -> &ProviderConfig;
    /// 构建授权 URL(含 state + 可选 PKCE code_challenge)。
    fn authorize(&self, state: &str, pkce_challenge: Option<&str>) -> String;
    /// 处理回调:用授权码交换 token。
    async fn callback(&self, code: &str, pkce_verifier: Option<&str>) -> anyhow::Result<TokenSet>;
    /// 用 refresh_token 刷新过期 token。
    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet>;
    /// 撤销 token(best-effort)。
    async fn revoke(&self, token: &str) -> anyhow::Result<()>;
}

/// 生成随机 state 参数(防 CSRF),32 字节 hex 编码。
pub fn generate_state() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(&buf)
}

/// 极简 hex 编码器(避免引入额外 crate)。
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_set_from_response_parses_expires_in() {
        let json = serde_json::json!({
            "access_token": "abc123",
            "refresh_token": "rf456",
            "scope": "repo user",
            "token_type": "Bearer",
            "expires_in": 3600
        });
        let t = TokenSet::from_response(&json).unwrap();
        assert_eq!(t.access_token, "abc123");
        assert_eq!(t.refresh_token.as_deref(), Some("rf456"));
        assert_eq!(t.scope, "repo user");
        assert_eq!(t.token_type, "Bearer");
        assert!(!t.is_expired(), "刚签发的 token 不应过期");
    }

    #[test]
    fn token_set_from_response_no_expires_in_means_non_expiring() {
        let json = serde_json::json!({ "access_token": "x", "token_type": "bearer" });
        let t = TokenSet::from_response(&json).unwrap();
        assert!(!t.is_expired());
        assert!(t.refresh_token.is_none());
    }

    #[test]
    fn token_set_from_response_missing_access_token_errors() {
        let json = serde_json::json!({ "scope": "x" });
        assert!(TokenSet::from_response(&json).is_err());
    }

    #[test]
    fn generate_state_is_unique_and_hex() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2, "两次生成的 state 必须不同");
        assert_eq!(s1.len(), 64, "32 字节 hex = 64 字符");
        assert!(s1.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
