//! OAuth manager — aggregates providers, manages token lifecycle.
//!
//! Tokens are stored in the OS keychain (via `security::keychain`) keyed
//! by `oauth:{provider_id}`.  The in-memory cache is only a read-through
//! cache; the keychain is the source of truth.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{info, warn};

use super::oauth::{OAuthProvider, OAuthToken};

/// Keychain key prefix for OAuth tokens.
const KEYCHAIN_PREFIX: &str = "oauth:";

/// Aggregates all registered OAuth providers and manages token storage.
pub struct OAuthManager {
    providers: RwLock<HashMap<String, Arc<dyn OAuthProvider>>>,
    /// In-memory token cache (keychain is the source of truth).
    tokens: RwLock<HashMap<String, OAuthToken>>,
}

impl OAuthManager {
    /// Creates an empty manager with no providers registered.
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
        }
    }

    /// Registers a provider.  If a provider with the same id already
    /// exists it is replaced.
    pub fn register_provider(&self, provider: Arc<dyn OAuthProvider>) {
        let id = provider.id().to_string();
        let name = provider.config().name.clone();
        self.providers.write().insert(id.clone(), provider);
        info!(target: "nebula.oauth", provider = %id, name = %name, "provider registered");
    }

    /// Returns the ids of all registered providers.
    pub fn list_providers(&self) -> Vec<OAuthProviderInfo> {
        self.providers
            .read()
            .iter()
            .map(|(id, p)| OAuthProviderInfo {
                id: id.clone(),
                name: p.config().name.clone(),
                connected: self.tokens.read().contains_key(id),
            })
            .collect()
    }

    /// Returns the authorization URL for a provider.
    pub fn authorization_url(&self, provider_id: &str, state: &str) -> anyhow::Result<String> {
        let provider = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown OAuth provider: {provider_id}"))?;
        Ok(provider.config().authorization_url(state))
    }

    /// Exchanges an authorization code for a token, stores it in the
    /// keychain, and caches it in memory.
    pub async fn authorize(&self, provider_id: &str, code: &str) -> anyhow::Result<()> {
        let provider = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown OAuth provider: {provider_id}"))?;

        let token = provider.exchange_code(code).await?;

        // Persist to OS keychain.
        let key = format!("{KEYCHAIN_PREFIX}{provider_id}");
        let token_json = serde_json::to_string(&token)?;
        crate::security::keychain::set(&key, &token_json)?;

        // Cache in memory.
        self.tokens.write().insert(provider_id.to_string(), token);
        info!(target: "nebula.oauth", provider = %provider_id, "authorization successful");
        Ok(())
    }

    /// Returns the cached token for a provider, or loads it from the
    /// keychain if not cached.  Returns `None` if the provider is not
    /// connected.
    pub fn get_token(&self, provider_id: &str) -> anyhow::Result<Option<OAuthToken>> {
        // Check memory cache first.
        if let Some(token) = self.tokens.read().get(provider_id) {
            return Ok(Some(token.clone()));
        }

        // Fall back to keychain.
        let key = format!("{KEYCHAIN_PREFIX}{provider_id}");
        match crate::security::keychain::get(&key)? {
            Some(json) => {
                let token: OAuthToken = serde_json::from_str(&json)?;
                // Cache for future lookups.
                self.tokens
                    .write()
                    .insert(provider_id.to_string(), token.clone());
                Ok(Some(token))
            }
            None => Ok(None),
        }
    }

    /// Refreshes an expired token using the stored refresh token.
    pub async fn refresh_if_needed(&self, provider_id: &str) -> anyhow::Result<Option<OAuthToken>> {
        let token = match self.get_token(provider_id)? {
            Some(t) if t.is_expired() => t,
            Some(t) => return Ok(Some(t)), // still valid
            None => return Ok(None),       // not connected
        };

        let refresh_token = token
            .refresh_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("token expired but no refresh_token available"))?;

        let provider = self
            .providers
            .read()
            .get(provider_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown OAuth provider: {provider_id}"))?;

        let new_token = provider.refresh_token(refresh_token).await?;

        // Persist refreshed token.
        let key = format!("{KEYCHAIN_PREFIX}{provider_id}");
        let token_json = serde_json::to_string(&new_token)?;
        crate::security::keychain::set(&key, &token_json)?;

        self.tokens
            .write()
            .insert(provider_id.to_string(), new_token.clone());
        info!(target: "nebula.oauth", provider = %provider_id, "token refreshed");
        Ok(Some(new_token))
    }

    /// Revokes and removes a provider's token.
    pub async fn disconnect(&self, provider_id: &str) -> anyhow::Result<()> {
        // Remove from memory cache.
        let token = self.tokens.write().remove(provider_id);

        // Best-effort revocation — clone the Arc out of the lock before
        // awaiting so the RwLockReadGuard is not held across the await.
        let provider_arc = self.providers.read().get(provider_id).cloned();

        if let Some(token) = token {
            if let Some(provider) = provider_arc {
                if let Err(e) = provider.revoke_token(&token.access_token).await {
                    warn!(
                        target: "nebula.oauth",
                        provider = %provider_id,
                        error = %e,
                        "token revocation failed (non-fatal)"
                    );
                }
            }
        }

        // Remove from keychain.
        let key = format!("{KEYCHAIN_PREFIX}{provider_id}");
        crate::security::keychain::delete(&key)?;
        info!(target: "nebula.oauth", provider = %provider_id, "disconnected");
        Ok(())
    }
}

impl Default for OAuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Lightweight provider info returned by [`OAuthManager::list_providers`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct OAuthProviderInfo {
    pub id: String,
    pub name: String,
    pub connected: bool,
}
