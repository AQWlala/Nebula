//! T-E-C-18: 加密 SQLite token 存储。
//!
//! 用 AES-256-GCM 在**应用层**加密 token JSON,再写入独立 SQLite 文件。
//! 这样无论是否启用 `sqlcipher` feature,token 都在磁盘上加密。
//!
//! ## 密钥管理
//!
//! 32 字节加密密钥存 OS keychain(slot `oauth_token_encryption_key`)。
//! 首次启动时自动生成并写入;后续读取复用。keychain 不可用时
//! (headless CI / 无 DBUS)fallback 到 env `NEBULA_OAUTH_TOKEN_KEY`。
//!
//! ## 表结构
//!
//! ```sql
//! CREATE TABLE oauth_tokens (
//!     provider_id    TEXT PRIMARY KEY,
//!     encrypted_blob TEXT NOT NULL,   -- base64(nonce || ciphertext)
//!     scope          TEXT NOT NULL DEFAULT '',
//!     expires_at     INTEGER NOT NULL, -- Unix 秒
//!     created_at     INTEGER NOT NULL,
//!     updated_at     INTEGER NOT NULL
//! );
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use parking_lot::Mutex;
use rusqlite::Connection;
use tracing::{debug, info, warn};

use super::TokenSet;

/// keychain slot:OAuth token 加密密钥。
const KEYCHAIN_SLOT: &str = "oauth_token_encryption_key";
/// env var 兜底:keychain 不可用时从此读取密钥。
const ENV_KEY: &str = "NEBULA_OAUTH_TOKEN_KEY";
/// AES-256-GCM nonce 长度(12 字节,标准)。
const NONCE_LEN: usize = 12;

/// 持久化的 token(含元数据),从 `TokenSet` 转换而来。
#[derive(Debug, Clone)]
pub struct StoredToken {
    pub provider_id: String,
    pub token_set: TokenSet,
}

/// 加密 SQLite token 存储器。
pub struct TokenStore {
    /// SQLite 连接(内部加锁,线程安全)。
    conn: Arc<Mutex<Connection>>,
    /// AES-256-GCM 加密器(密钥已固定)。
    cipher: Aes256Gcm,
}

impl TokenStore {
    /// 打开(或创建)指定路径的 token 存储。
    ///
    /// 自动建表 + 解析 / 生成加密密钥。
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("创建 token 存储目录失败: {}", parent.display()))?;
            }
        }

        let conn = Connection::open(path)
            .with_context(|| format!("打开 token 存储 SQLite 失败: {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // 建表(幂等)。
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS oauth_tokens (
                provider_id    TEXT PRIMARY KEY,
                encrypted_blob TEXT NOT NULL,
                scope          TEXT NOT NULL DEFAULT '',
                expires_at     INTEGER NOT NULL,
                created_at     INTEGER NOT NULL,
                updated_at     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_oauth_tokens_expires
                ON oauth_tokens(expires_at);",
        )
        .context("创建 oauth_tokens 表失败")?;

        let key_bytes = resolve_encryption_key()?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));

        info!(target: "nebula.oauth.store", path = %path.display(), "token 存储已就绪");
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            cipher,
        })
    }

    /// 加密并持久化一个 provider 的 token(upsert)。
    pub fn save(&self, provider_id: &str, token: &TokenSet) -> Result<()> {
        let json = serde_json::to_string(token).context("序列化 token 失败")?;
        let blob = self.encrypt(json.as_bytes())?;
        let now = Utc::now().timestamp();
        let expires_at = token.expires_at.timestamp();
        let scope = &token.scope;

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO oauth_tokens (provider_id, encrypted_blob, scope, expires_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(provider_id) DO UPDATE SET
                encrypted_blob = excluded.encrypted_blob,
                scope = excluded.scope,
                expires_at = excluded.expires_at,
                updated_at = excluded.updated_at",
            rusqlite::params![provider_id, blob, scope, expires_at, now],
        )
        .context("写入 oauth_tokens 失败")?;
        debug!(target: "nebula.oauth.store", provider = provider_id, "token 已保存");
        Ok(())
    }

    /// 读取并解密一个 provider 的 token;未连接返回 `None`。
    pub fn load(&self, provider_id: &str) -> Result<Option<StoredToken>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT encrypted_blob, scope FROM oauth_tokens WHERE provider_id = ?1")?;
        let row = stmt
            .query_row(rusqlite::params![provider_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .ok();
        drop(stmt);
        drop(conn);

        match row {
            None => Ok(None),
            Some((blob, _scope)) => {
                let plaintext = self.decrypt(&blob)?;
                let token_set: TokenSet =
                    serde_json::from_slice(&plaintext).context("反序列化 token 失败")?;
                Ok(Some(StoredToken {
                    provider_id: provider_id.to_string(),
                    token_set,
                }))
            }
        }
    }

    /// 删除一个 provider 的 token(幂等)。
    pub fn delete(&self, provider_id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM oauth_tokens WHERE provider_id = ?1",
            rusqlite::params![provider_id],
        )?;
        debug!(target: "nebula.oauth.store", provider = provider_id, "token 已删除");
        Ok(())
    }

    /// 列出所有已连接的 provider id。
    pub fn list_connected(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT provider_id FROM oauth_tokens")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    // -- 加密 / 解密 ----------------------------------------------------------

    /// AES-256-GCM 加密,返回 `base64(nonce || ciphertext)`。
    fn encrypt(&self, plaintext: &[u8]) -> Result<String> {
        use rand::RngCore;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("AES-GCM 加密失败: {e}"))?;
        // 拼接 nonce + ciphertext,base64 编码。
        let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);
        Ok(base64::engine::general_purpose::STANDARD.encode(&combined))
    }

    /// 解密 `base64(nonce || ciphertext)`,返回明文。
    fn decrypt(&self, blob: &str) -> Result<Vec<u8>> {
        let combined = base64::engine::general_purpose::STANDARD
            .decode(blob)
            .context("base64 解码失败")?;
        if combined.len() < NONCE_LEN {
            anyhow::bail!("加密 blob 过短(少于 nonce 长度)");
        }
        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("AES-GCM 解密失败(密钥不匹配? {e})"))?;
        Ok(plaintext)
    }

    /// 返回存储文件路径(供测试 / 备份使用)。
    pub fn db_path(&self) -> Option<PathBuf> {
        let conn = self.conn.lock();
        conn.query_row("PRAGMA database_list", [], |r| {
            let s: String = r.get(2)?;
            Ok(PathBuf::from(s))
        })
        .ok()
    }
}

/// 解析 32 字节加密密钥:keychain 优先 → env 兜底 → 自动生成。
fn resolve_encryption_key() -> Result<[u8; 32]> {
    // 1. 尝试 keychain。
    match crate::security::keychain::get(KEYCHAIN_SLOT) {
        Ok(Some(b64)) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .context("keychain 中的 OAuth 密钥 base64 解码失败")?;
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Ok(arr);
            }
            warn!(target: "nebula.oauth.store", "keychain 密钥长度异常({}),重新生成", bytes.len());
        }
        Ok(None) => debug!(target: "nebula.oauth.store", "keychain 无 OAuth 密钥,尝试 env / 生成"),
        Err(e) => warn!(target: "nebula.oauth.store", error = %e, "keychain 不可用,fallback env"),
    }

    // 2. 尝试 env var。
    if let Ok(val) = std::env::var(ENV_KEY) {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&val) {
            if bytes.len() == 32 {
                debug!(target: "nebula.oauth.store", "从 env {ENV_KEY} 读取密钥");
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Ok(arr);
            }
        }
    }

    // 3. 生成新密钥并尝试写入 keychain(失败不致命)。
    use rand::RngCore;
    let mut key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key_bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(key_bytes);
    if let Err(e) = crate::security::keychain::set(KEYCHAIN_SLOT, &b64) {
        warn!(target: "nebula.oauth.store", error = %e, "无法写入 keychain,密钥仅存活于内存(重启后需重新授权)");
    }
    info!(target: "nebula.oauth.store", "已生成新的 OAuth token 加密密钥");
    Ok(key_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_token() -> TokenSet {
        TokenSet {
            access_token: "ghp_test_abc123".to_string(),
            refresh_token: Some("rfr_test_456".to_string()),
            expires_at: Utc::now() + Duration::seconds(3600),
            scope: "repo user".to_string(),
            token_type: "Bearer".to_string(),
        }
    }

    fn temp_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("nebula_oauth_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    #[test]
    fn save_load_roundtrip() {
        let path = temp_path();
        let store = TokenStore::open(&path).unwrap();
        let token = sample_token();
        store.save("github", &token).unwrap();

        let loaded = store.load("github").unwrap().expect("token 应存在");
        assert_eq!(loaded.provider_id, "github");
        assert_eq!(loaded.token_set.access_token, "ghp_test_abc123");
        assert_eq!(
            loaded.token_set.refresh_token.as_deref(),
            Some("rfr_test_456")
        );
        assert_eq!(loaded.token_set.scope, "repo user");
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_missing_returns_none() {
        let path = temp_path();
        let store = TokenStore::open(&path).unwrap();
        assert!(store.load("nonexistent").unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn delete_is_idempotent() {
        let path = temp_path();
        let store = TokenStore::open(&path).unwrap();
        store.save("slack", &sample_token()).unwrap();
        store.delete("slack").unwrap();
        assert!(store.load("slack").unwrap().is_none());
        // 再次删除不报错。
        store.delete("slack").unwrap();
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn upsert_overwrites() {
        let path = temp_path();
        let store = TokenStore::open(&path).unwrap();
        let mut token = sample_token();
        store.save("google", &token).unwrap();
        token.access_token = "ya29_new_value".to_string();
        store.save("google", &token).unwrap();

        let loaded = store.load("google").unwrap().unwrap();
        assert_eq!(loaded.token_set.access_token, "ya29_new_value");
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn list_connected_providers() {
        let path = temp_path();
        let store = TokenStore::open(&path).unwrap();
        store.save("github", &sample_token()).unwrap();
        store.save("notion", &sample_token()).unwrap();
        let mut ids = store.list_connected().unwrap();
        ids.sort();
        assert_eq!(ids, vec!["github".to_string(), "notion".to_string()]);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn encrypted_blob_is_not_plaintext() {
        // 确保磁盘上的密文不含明文 access_token。
        let path = temp_path();
        let store = TokenStore::open(&path).unwrap();
        let token = sample_token();
        store.save("github", &token).unwrap();
        drop(store);

        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !raw.contains("ghp_test_abc123"),
            "明文 token 不应出现在 SQLite 文件中"
        );
        let _ = std::fs::remove_file(path);
    }
}
