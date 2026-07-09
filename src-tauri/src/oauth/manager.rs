//! T-E-C-18: OAuthManager — 编排完整授权码 + PKCE 流程。
//!
//! [`OAuthManager`] 持有所有已注册的 [`OAuthProvider`](super::OAuthProvider)
//! 实例和一个 [`TokenStore`],对外暴露 4 个核心操作:
//!
//! 1. `begin_flow` — 生成 state(+ PKCE pair)、启动回调服务器、返回授权 URL。
//! 2. `complete_flow` — 收到回调后校验 state、交换 token、加密存储。
//! 3. `get_valid_token` — 读取 token,过期则自动刷新。
//! 4. `disconnect` — 撤销 token + 删除存储。
//!
//! ## 并发安全
//!
//! provider 注册表用 `RwLock<HashMap>`;进行中的 flow(PKCE verifier /
//! state 映射)用 `Mutex<HashMap>`。`complete_flow` 在 await 前释放所有锁。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::{Mutex, RwLock};
use tracing::{info, warn};

use super::callback_server::{start_callback_server, CallbackResult};
use super::pkce::PkcePair;
use super::token_store::TokenStore;
use super::{generate_state, OAuthProvider, TokenSet};

/// 一个进行中的授权流程的临时状态(state → verifier + 接收回调的 channel)。
struct PendingFlow {
    #[allow(dead_code)]
    state: String,
    pkce_verifier: Option<String>,
    rx: tokio::sync::oneshot::Receiver<CallbackResult>,
}

/// 已注册 provider 的轻量摘要(供 UI 列表)。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub supports_pkce: bool,
    pub connected: bool,
}

/// OAuth 编排器:聚合 provider + token 存储 + 进行中的 flow。
pub struct OAuthManager {
    providers: RwLock<HashMap<String, Arc<dyn OAuthProvider>>>,
    store: Mutex<TokenStore>,
    pending: Mutex<HashMap<String, PendingFlow>>,
}

impl OAuthManager {
    /// 创建 manager,内部打开指定路径的加密 token 存储。
    pub fn new(store: TokenStore) -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            store: Mutex::new(store),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// 注册一个 provider(同 id 覆盖)。
    pub fn register_provider(&self, provider: Arc<dyn OAuthProvider>) {
        let id = provider.id().to_string();
        let name = provider.name().to_string();
        self.providers.write().insert(id.clone(), provider);
        info!(target: "nebula.oauth", provider = %id, name = %name, "provider 已注册");
    }

    /// 列出所有已注册 provider 及连接状态。
    pub fn list_providers(&self) -> Vec<ProviderInfo> {
        let connected: Vec<String> = self.store.lock().list_connected().unwrap_or_default();
        self.providers
            .read()
            .iter()
            .map(|(id, p)| ProviderInfo {
                id: id.clone(),
                name: p.name().to_string(),
                supports_pkce: p.supports_pkce(),
                connected: connected.contains(id),
            })
            .collect()
    }

    /// 开始一次授权流程:生成 state + PKCE、启动回调服务器、返回授权 URL。
    ///
    /// 返回值中的 `redirect_uri` 应与 provider 控制台配置一致(动态端口
    /// 场景下,provider 须允许 `http://127.0.0.1` 任意端口)。
    pub async fn begin_flow(&self, provider_id: &str) -> Result<FlowHandle> {
        let provider = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("未知的 OAuth provider: {provider_id}"))?;

        // 1. 启动回调服务器,拿到动态 redirect_uri。
        let (redirect_uri, rx) = start_callback_server().await?;

        // 2. 生成 state(防 CSRF)。
        let state = generate_state();

        // 3. 若 provider 支持 PKCE,生成 verifier / challenge 对。
        let pkce_pair = if provider.supports_pkce() {
            Some(PkcePair::generate()?)
        } else {
            None
        };

        // 4. 构建授权 URL。
        let auth_url = provider.authorize(
            &state,
            pkce_pair.as_ref().map(|p| p.code_challenge.as_str()),
        );

        // 5. 记录 pending flow,等待 complete_flow 消费。
        let flow_id = state.clone();
        self.pending.lock().insert(
            flow_id.clone(),
            PendingFlow {
                state: state.clone(),
                pkce_verifier: pkce_pair.map(|p| p.code_verifier),
                rx,
            },
        );

        info!(target: "nebula.oauth", provider = %provider_id, state = %state, "授权流程已启动");

        Ok(FlowHandle {
            provider_id: provider_id.to_string(),
            state,
            auth_url,
            redirect_uri,
        })
    }

    /// 完成授权流程:等待回调 → 校验 state → 交换 token → 加密存储。
    ///
    /// `timeout_secs` 为等待用户在浏览器完成授权的最长时间。
    pub async fn complete_flow(
        &self,
        provider_id: &str,
        flow_state: &str,
        timeout_secs: u64,
    ) -> Result<TokenSet> {
        // 1. 取出 pending flow(释放锁后再 await)。
        let pending = self
            .pending
            .lock()
            .remove(flow_state)
            .ok_or_else(|| anyhow::anyhow!("无此 state 的 pending flow(可能已超时或已完成)"))?;

        // 2. 等待回调,带超时。
        let callback =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), pending.rx)
                .await
                .context("等待 OAuth 回调超时")?
                .map_err(|_| anyhow::anyhow!("回调 channel 已关闭"))?;

        // 3. 校验 state(防 CSRF)。
        if callback.state != flow_state {
            anyhow::bail!(
                "state 不匹配(预期 {flow_state},收到 {}),可能是 CSRF 攻击",
                callback.state
            );
        }
        if callback.code.is_empty() {
            anyhow::bail!("回调未携带授权码(用户可能拒绝了授权)");
        }

        // 4. 取 provider,交换 token。
        let provider = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("provider {provider_id} 在 flow 期间被注销"))?;

        let token = provider
            .callback(&callback.code, pending.pkce_verifier.as_deref())
            .await
            .context("交换授权码失败")?;

        // 5. 加密存储。
        self.store.lock().save(provider_id, &token)?;

        info!(target: "nebula.oauth", provider = %provider_id, "授权完成,token 已加密存储");
        Ok(token)
    }

    /// 获取一个有效的 token:从存储读取,过期则自动刷新。
    ///
    /// 未连接返回 `None`。刷新失败返回 `Err`(调用方可提示用户重新授权)。
    pub async fn get_valid_token(&self, provider_id: &str) -> Result<Option<TokenSet>> {
        let stored = self.store.lock().load(provider_id)?;
        let token = match stored {
            None => return Ok(None),
            Some(s) => s.token_set,
        };

        if !token.is_expired() {
            return Ok(Some(token));
        }

        // 过期 → 尝试刷新。
        let refresh_token = token
            .refresh_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("token 已过期且无 refresh_token,需重新授权"))?;

        let provider = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("未知的 OAuth provider: {provider_id}"))?;

        match provider.refresh(&refresh_token).await {
            Ok(new_token) => {
                self.store.lock().save(provider_id, &new_token)?;
                info!(target: "nebula.oauth", provider = %provider_id, "token 已自动刷新");
                Ok(Some(new_token))
            }
            Err(e) => {
                warn!(target: "nebula.oauth", provider = %provider_id, error = %e, "token 刷新失败");
                Err(e).context("token 刷新失败,请重新授权")
            }
        }
    }

    /// 断开一个 provider:撤销 token(best-effort)+ 删除存储。
    pub async fn disconnect(&self, provider_id: &str) -> Result<()> {
        // 先读出 token(供撤销),再删除存储。
        let token = self.store.lock().load(provider_id)?;
        self.store.lock().delete(provider_id)?;

        if let Some(stored) = token {
            let provider = self.providers.read().get(provider_id).cloned();
            if let Some(provider) = provider {
                if let Err(e) = provider.revoke(&stored.token_set.access_token).await {
                    warn!(
                        target: "nebula.oauth",
                        provider = %provider_id,
                        error = %e,
                        "撤销 token 失败(非致命)"
                    );
                }
            }
        }

        info!(target: "nebula.oauth", provider = %provider_id, "已断开连接");
        Ok(())
    }
}

/// `begin_flow` 返回的句柄,包含打开浏览器所需的授权 URL。
#[derive(Debug, Clone)]
pub struct FlowHandle {
    /// provider id。
    pub provider_id: String,
    /// state 参数(传给 `complete_flow` 校验)。
    pub state: String,
    /// 打开此 URL 让用户在浏览器授权。
    pub auth_url: String,
    /// 回调服务器监听地址(provider 控制台需允许此前缀)。
    pub redirect_uri: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{OAuthProvider, ProviderConfig, TokenSet};
    use async_trait::async_trait;
    use chrono::{Duration, Utc};

    /// 测试用 stub provider:不真正发 HTTP,callback 直接返回固定 token。
    struct StubProvider {
        config: ProviderConfig,
    }

    impl StubProvider {
        fn new(id: &str) -> Self {
            Self {
                config: ProviderConfig {
                    id: id.to_string(),
                    name: format!("Stub-{id}"),
                    client_id: "cid".into(),
                    client_secret: None,
                    redirect_uri: "http://127.0.0.1/callback".into(),
                    auth_url: "https://example.com/auth".into(),
                    token_url: "https://example.com/token".into(),
                    revoke_url: None,
                    scopes: vec!["read".into()],
                },
            }
        }
    }

    #[async_trait]
    impl OAuthProvider for StubProvider {
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
        fn authorize(&self, state: &str, pkce: Option<&str>) -> String {
            format!(
                "https://example.com/auth?state={state}&pkce={}",
                pkce.unwrap_or("none")
            )
        }
        async fn callback(&self, code: &str, _verifier: Option<&str>) -> Result<TokenSet> {
            Ok(TokenSet {
                access_token: format!("stub_token_{code}"),
                refresh_token: Some("stub_refresh".into()),
                expires_at: Utc::now() + Duration::seconds(3600),
                scope: "read".into(),
                token_type: "Bearer".into(),
            })
        }
        async fn refresh(&self, _refresh: &str) -> Result<TokenSet> {
            Ok(TokenSet {
                access_token: "stub_refreshed".into(),
                refresh_token: Some("stub_refresh".into()),
                expires_at: Utc::now() + Duration::seconds(3600),
                scope: "read".into(),
                token_type: "Bearer".into(),
            })
        }
        async fn revoke(&self, _token: &str) -> Result<()> {
            Ok(())
        }
    }

    fn temp_store() -> TokenStore {
        let mut p = std::env::temp_dir();
        p.push(format!("nebula_oauth_mgr_{}.db", uuid::Uuid::new_v4()));
        TokenStore::open(&p).unwrap()
    }

    #[test]
    fn list_providers_reports_connection_status() {
        let store = temp_store();
        let mgr = OAuthManager::new(store);
        mgr.register_provider(Arc::new(StubProvider::new("stub")));
        let infos = mgr.list_providers();
        assert_eq!(infos.len(), 1);
        assert!(!infos[0].connected);
        assert!(infos[0].supports_pkce);
    }

    #[tokio::test]
    async fn full_flow_completes_and_stores_token() {
        let store = temp_store();
        let mgr = OAuthManager::new(store);
        mgr.register_provider(Arc::new(StubProvider::new("stub")));

        // 开始 flow。
        let handle = mgr.begin_flow("stub").await.unwrap();
        assert!(handle.auth_url.contains("state="));

        // 模拟回调:向回调服务器发请求。
        let port: u16 = handle
            .redirect_uri
            .rsplit(':')
            .next()
            .unwrap()
            .trim_end_matches("/callback")
            .parse()
            .unwrap();
        let state = handle.state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let mut s = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let req = format!(
                "GET /callback?code=abc789&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = [0u8; 128];
            let _ = s.read(&mut buf).await;
        });

        // 完成 flow。
        let token = mgr.complete_flow("stub", &handle.state, 10).await.unwrap();
        assert_eq!(token.access_token, "stub_token_abc789");

        // 验证已存储。
        let loaded = mgr.get_valid_token("stub").await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "stub_token_abc789");
    }

    #[tokio::test]
    async fn disconnect_removes_token() {
        let store = temp_store();
        let mgr = OAuthManager::new(store);
        mgr.register_provider(Arc::new(StubProvider::new("stub")));

        // 手动存一个 token。
        let token = TokenSet {
            access_token: "manual".into(),
            refresh_token: None,
            expires_at: Utc::now() + Duration::days(365),
            scope: "read".into(),
            token_type: "Bearer".into(),
        };
        mgr.store.lock().save("stub", &token).unwrap();
        assert!(mgr.get_valid_token("stub").await.unwrap().is_some());

        mgr.disconnect("stub").await.unwrap();
        assert!(mgr.get_valid_token("stub").await.unwrap().is_none());
    }
}
