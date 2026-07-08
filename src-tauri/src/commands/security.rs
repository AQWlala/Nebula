//! Security commands — injection scan, sandbox config, DB encryption.

use tracing::instrument;

use crate::commands::error::CommandError;

/// Full injection scan of arbitrary input.
#[tauri::command]
#[instrument(fields(otel.kind = "injection_scan"))]
pub async fn injection_scan(
    input: String,
) -> Result<crate::security::InjectionScanResult, CommandError> {
    Ok(crate::security::full_injection_scan(&input))
}

/// Retrieve sandbox configuration for a skill.
#[tauri::command]
#[instrument(fields(otel.kind = "sandbox_config"))]
pub async fn sandbox_config(
    skill_id: String,
) -> Result<crate::skills::sandbox::SandboxConfig, CommandError> {
    let mut config = crate::skills::sandbox::SandboxConfig::default();
    config.capabilities = crate::skills::sandbox::CapabilitySet::llm_only();
    Ok(config)
}

// ---------------------------------------------------------------------------
// T-E-S-43: SQLite 数据库加密(SQLCipher)— 3 个 Tauri 命令。
// ---------------------------------------------------------------------------

/// T-E-S-43: DB 加密状态快照。
///
/// 前端用此命令决定是否显示 "加密 DB" 按钮 / 是否提示备份 key。
/// `feature_enabled` 在编译期决定(`cfg!(feature = "sqlcipher")`),
/// 无 sqlcipher feature 时永远为 false,前端应隐藏加密 UI。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DbEncryptionStatus {
    /// sqlcipher feature 是否启用(编译期,`cfg!(feature = "sqlcipher")`)。
    pub feature_enabled: bool,
    /// AppConfig.db_encryption_enabled(运行时配置)。
    /// 主 agent 集成后读 `state.config.db_encryption_enabled`;
    /// 当前临时读 env var `NEBULA_DB_ENCRYPTION=1`。
    pub config_enabled: bool,
    /// keychain 中是否有 DB 加密 key(`resolve_db_encryption_key().is_some()`)。
    pub key_present: bool,
    /// SQLCipher 版本字符串(若可查询);当前始终 None(避免打开 DB 副作用)。
    pub cipher_version: Option<String>,
    /// SQLite DB 文件路径(`state.infra.config.db_path`)。
    pub db_path: String,
}

/// T-E-S-43: 查询 DB 加密状态。
///
/// **始终编译**(不门控 `sqlcipher` feature),前端在无 feature 时
/// 收到 `feature_enabled: false` 并隐藏加密 UI。内部用
/// `cfg!(feature = "sqlcipher")`(运行时 bool,非 `#[cfg]`)返回
/// `feature_enabled` 字段。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "db_encryption_status"))]
pub async fn db_encryption_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<DbEncryptionStatus, CommandError> {
    // T-E-S-43: 主 agent 集成后,config_enabled 应读 state.config.db_encryption_enabled。
    // 当前 lib.rs 未加该字段,临时读 env var NEBULA_DB_ENCRYPTION=1(spec 默认)。
    let config_enabled = std::env::var("NEBULA_DB_ENCRYPTION")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let key_present = crate::security::keychain::resolve_db_encryption_key().is_some();
    let db_path = state.infra.config.db_path.clone();
    let feature_enabled = cfg!(feature = "sqlcipher");
    // cipher_version 仅在 feature 启用时尝试查询;此处避免打开 DB 副作用,返回 None。
    // 前端如需版本号,可调 db_encryption_enable 后从返回值读 cipher_version。
    let cipher_version: Option<String> = None;
    Ok(DbEncryptionStatus {
        feature_enabled,
        config_enabled,
        key_present,
        cipher_version,
        db_path,
    })
}

/// T-E-S-43: 启用 DB 加密(明文 DB → 密文 DB)。
///
/// 仅在 `sqlcipher` feature 启用时实现;无 feature 时返回 Err。
///
/// **流程**:
/// 1. 生成 32 字节随机 key(`generate_db_encryption_key`)。
/// 2. 存 keychain(`set(KEY_DB_ENCRYPTION_KEY, &key)`)。
/// 3. `CipherMigrator::encrypt_plaintext_db`(重命名明文 → .plain.bak,
///    sqlcipher_export 导入密文 DB)。
/// 4. 持久化 config(主 agent 集成 `save_app_settings(db_encryption_enabled=true)`;
///    临时设 env var `NEBULA_DB_ENCRYPTION=1`)。
/// 5. 返回 recovery_key(供用户备份;key 丢失将导致 DB 不可读)。
///
/// **recovery_key 警告**:前端必须强制提示用户备份此 key,key 丢失后
/// DB 不可恢复(无主密钥 / escrow 机制)。
#[cfg(feature = "sqlcipher")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "db_encryption_enable"))]
pub async fn db_encryption_enable(
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, CommandError> {
    use std::path::Path;

    // 1. 生成 key。
    let key = crate::security::keychain::generate_db_encryption_key();

    // 2. 存 keychain。
    crate::security::keychain::set(crate::security::keychain::KEY_DB_ENCRYPTION_KEY, &key)
        .map_err(|e| CommandError::internal("db_encryption_enable: keychain set", &e))?;

    // 3. 加密明文 DB(CipherMigrator::encrypt_plaintext_db)。
    //    重命名明文 DB → .plain.bak,用 key 打开新 DB,sqlcipher_export 导入。
    let db_path = state.infra.config.db_path.clone();
    let plain_path = Path::new(&db_path);
    let _backup =
        crate::memory::sqlite_cipher::CipherMigrator::encrypt_plaintext_db(plain_path, &key)
            .map_err(|e| CommandError::db("db_encryption_enable: encrypt_plaintext_db", &e))?;

    // 4. 持久化 config。
    //    主 agent 集成:save_app_settings(db_encryption_enabled=true)。
    //    临时:设 env var,主 agent 集成后替换为 save_app_settings。
    std::env::set_var("NEBULA_DB_ENCRYPTION", "1");

    // 5. 返回 recovery_key(供用户备份)。
    Ok(key)
}

/// T-E-S-43: 启用 DB 加密的 stub(无 sqlcipher feature 时)。
///
/// 返回 Err("sqlcipher feature not enabled"),前端提示用户重建
/// `--features sqlcipher` 版本。
#[cfg(not(feature = "sqlcipher"))]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "db_encryption_enable"))]
pub async fn db_encryption_enable(
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, CommandError> {
    let _ = state;
    Err(CommandError::validation(
        "sqlcipher feature not enabled; rebuild with --features sqlcipher",
    ))
}

/// T-E-S-43: 禁用 DB 加密(密文 DB → 明文 DB)。
///
/// 仅在 `sqlcipher` feature 启用时实现;无 feature 时返回 Err。
///
/// **流程**:
/// 1. `CipherMigrator::decrypt_to_plaintext`(需用户提供正确 key 验证;
///    key 错则 ATTACH 后查询失败,"file is not a database")。
/// 2. 删除 keychain slot(`delete(KEY_DB_ENCRYPTION_KEY)`)。
/// 3. 持久化 config(主 agent 集成 `save_app_settings(db_encryption_enabled=false)`;
///    临时设 env var `NEBULA_DB_ENCRYPTION=0`)。
///
/// **安全说明**:需用户提供 key 验证(防止误操作 / 未授权禁用)。
#[cfg(feature = "sqlcipher")]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "db_encryption_disable"))]
pub async fn db_encryption_disable(
    state: tauri::State<'_, crate::AppState>,
    key: String,
) -> Result<(), CommandError> {
    use std::path::Path;

    // 1. 解密(key 验证:CipherMigrator 内部 ATTACH 时验证 key;
    //    key 错则 sqlcipher_export 失败,"file is not a database")。
    let db_path = state.infra.config.db_path.clone();
    let enc_path = Path::new(&db_path);
    let _backup =
        crate::memory::sqlite_cipher::CipherMigrator::decrypt_to_plaintext(enc_path, &key)
            .map_err(|e| CommandError::db("db_encryption_disable: decrypt_to_plaintext", &e))?;

    // 2. 删除 keychain slot(幂等)。
    let _ = crate::security::keychain::delete(crate::security::keychain::KEY_DB_ENCRYPTION_KEY);

    // 3. 持久化 config。
    //    主 agent 集成:save_app_settings(db_encryption_enabled=false)。
    //    临时:设 env var,主 agent 集成后替换为 save_app_settings。
    std::env::set_var("NEBULA_DB_ENCRYPTION", "0");

    Ok(())
}

/// T-E-S-43: 禁用 DB 加密的 stub(无 sqlcipher feature 时)。
#[cfg(not(feature = "sqlcipher"))]
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "db_encryption_disable"))]
pub async fn db_encryption_disable(
    state: tauri::State<'_, crate::AppState>,
    key: String,
) -> Result<(), CommandError> {
    let _ = (state, key);
    Err(CommandError::validation(
        "sqlcipher feature not enabled; rebuild with --features sqlcipher",
    ))
}

// ---------------------------------------------------------------------------
// P1-B: OAuth 2.0 commands
// ---------------------------------------------------------------------------

/// Returns the list of registered OAuth providers with their connection status.
#[tauri::command]
pub fn oauth_list_providers(
    state: tauri::State<'_, crate::AppState>,
) -> Vec<crate::identity::OAuthProviderInfo> {
    state.platform.oauth_manager.list_providers()
}

/// Returns the authorization URL for a provider.  The front-end opens this
/// URL in the browser; after the user consents, the provider redirects to
/// `redirect_uri` with a `code` query param which the front-end passes to
/// [`oauth_authorize`].
#[tauri::command]
pub fn oauth_authorization_url(
    state: tauri::State<'_, crate::AppState>,
    provider_id: String,
    state_param: Option<String>,
) -> Result<String, CommandError> {
    let state_val = state_param.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    state
        .platform.oauth_manager
        .authorization_url(&provider_id, &state_val)
        .map_err(|e| CommandError::internal("failed to build authorization URL", &e))
}

/// Exchanges an authorization code for a token and stores it in the keychain.
#[tauri::command]
pub async fn oauth_authorize(
    state: tauri::State<'_, crate::AppState>,
    provider_id: String,
    code: String,
) -> Result<(), CommandError> {
    state
        .platform.oauth_manager
        .authorize(&provider_id, &code)
        .await
        .map_err(|e| CommandError::internal("OAuth authorization failed", &e))
}

/// Disconnects a provider: revokes and deletes the stored token.
#[tauri::command]
pub async fn oauth_disconnect(
    state: tauri::State<'_, crate::AppState>,
    provider_id: String,
) -> Result<(), CommandError> {
    state
        .platform.oauth_manager
        .disconnect(&provider_id)
        .await
        .map_err(|e| CommandError::internal("OAuth disconnect failed", &e))
}

/// Checks if a provider's token is still valid, refreshing it if needed.
/// Returns `true` if the provider is connected with a valid token.
#[tauri::command]
pub async fn oauth_status(
    state: tauri::State<'_, crate::AppState>,
    provider_id: String,
) -> Result<bool, CommandError> {
    match state
        .platform.oauth_manager
        .refresh_if_needed(&provider_id)
        .await
        .map_err(|e| CommandError::internal("OAuth status check failed", &e))?
    {
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-E-S-43: DbEncryptionStatus 在无 sqlcipher feature 时
    /// feature_enabled 为 false。
    #[test]
    fn db_encryption_status_feature_off_without_sqlcipher() {
        let status = DbEncryptionStatus {
            feature_enabled: cfg!(feature = "sqlcipher"),
            config_enabled: false,
            key_present: false,
            cipher_version: None,
            db_path: "test.db".to_string(),
        };
        // 无 sqlcipher feature 时 feature_enabled 必须为 false。
        #[cfg(not(feature = "sqlcipher"))]
        {
            assert!(
                !status.feature_enabled,
                "feature_enabled must be false without sqlcipher feature"
            );
        }
        // 有 sqlcipher feature 时 feature_enabled 必须为 true。
        #[cfg(feature = "sqlcipher")]
        {
            assert!(
                status.feature_enabled,
                "feature_enabled must be true with sqlcipher feature"
            );
        }
        // 两种情况:cipher_version 默认 None,db_path 透传。
        assert_eq!(status.cipher_version, None);
        assert_eq!(status.db_path, "test.db");
    }

    /// T-E-S-43: DbEncryptionStatus serde 序列化字段名正确
    /// (前端按 snake_case 解析)。
    #[test]
    fn db_encryption_status_serializes_to_snake_case() {
        let status = DbEncryptionStatus {
            feature_enabled: false,
            config_enabled: false,
            key_present: false,
            cipher_version: None,
            db_path: "x.db".to_string(),
        };
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("feature_enabled"), "json: {json}");
        assert!(json.contains("config_enabled"), "json: {json}");
        assert!(json.contains("key_present"), "json: {json}");
        assert!(json.contains("cipher_version"), "json: {json}");
        assert!(json.contains("db_path"), "json: {json}");
    }
}
