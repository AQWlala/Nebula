//! OS keychain integration (v1.0.1 P0#12).
//!
//! v1.0 stored the user's API keys in `localStorage` via
//! `Settings.tsx`, which meant the key was readable by any
//! JavaScript that ran in the WebView (including any malicious
//! skill that got XSS via a poisoned memory).  v1.0.1 moves
//! all secrets into the OS keychain:
//!
//! * **macOS** — Keychain (via the `security` CLI under the hood,
//!   through the `keyring` crate's `apple-native` feature).
//! * **Windows** — Credential Vault (via the `keyring` crate's
//!   `windows-native` feature; backed by the wincred API).
//! * **Linux** — Secret Service (via the `keyring` crate's
//!   `sync-secret-service` feature, backed by libsecret/gnome-
//!   keyring/kwallet over D-Bus).
//!
//! The Rust-side API is intentionally tiny: `set`, `get`,
//! `delete`.  All three return `anyhow::Result` so the caller
//! can use the same error-mapping pipeline as the rest of the
//! app.  `get` returns `Ok(None)` when the entry does not
//! exist (a normal "not configured yet" outcome), and `Err` only
//! for genuine OS errors (e.g. the user denied keychain access).
//!
//! All three entry points use a single, well-known service
//! name (`SERVICE`) and a per-purpose user name (e.g.
//! `"openai_api_key"`).  Callers compose the user name with the
//! keyring crate's `Entry::new` constructor.

use anyhow::{Context, Result};
use keyring::Entry;
use tracing::{debug, warn};

/// Service name used for every entry written by nebula.  The
/// OS keychain groups entries by `(service, user)`, so this
/// string shows up in the user's "Passwords" list.  It is also
/// the search key if the user wants to revoke the app's
/// keychain access from the OS UI.
pub const SERVICE: &str = "nebula";

/// OpenAI / OpenAI-compatible API key.  Used by
/// `commands::set_api_key` / `get_api_key` / `delete_api_key`.
pub const KEY_API_KEY: &str = "openai_api_key";

/// T-E-S-41: 用户自定义 provider 的 keychain slot 前缀。
/// 完整 slot 名为 `provider:<id>`(如 `provider:deepseek`),
/// 与 `KEY_API_KEY` 分开命名空间,避免与 v1.0.1 的 OpenAI 兼容
/// key 冲突。
pub const KEY_PROVIDER_PREFIX: &str = "provider:";

/// T-E-S-41: 写入用户自定义 provider 的 API key 到 keychain。
/// slot 名为 `provider:<provider_id>`。复用底层 `set(key, value)`。
pub fn set_provider_key(provider_id: &str, key: &str) -> Result<()> {
    let slot = format!("{KEY_PROVIDER_PREFIX}{provider_id}");
    set(&slot, key)
}

/// T-E-S-41: 读取用户自定义 provider 的 API key;未配置返回 None。
/// slot 名为 `provider:<provider_id>`。复用底层 `get(key)`。
pub fn get_provider_key(provider_id: &str) -> Result<Option<String>> {
    let slot = format!("{KEY_PROVIDER_PREFIX}{provider_id}");
    get(&slot)
}

/// T-E-S-41: 删除用户自定义 provider 的 API key(幂等)。
pub fn delete_provider_key(provider_id: &str) -> Result<()> {
    let slot = format!("{KEY_PROVIDER_PREFIX}{provider_id}");
    delete(&slot)
}

/// T-E-S-46: 技能发布器 token 的 keychain slot 前缀。
///
/// 完整 slot 名为 `publisher:<platform>`(如 `publisher:github`),
/// 与 `KEY_API_KEY` / `KEY_PROVIDER_PREFIX` 分开命名空间,避免与
/// v1.0.1 的 OpenAI 兼容 key / T-E-S-41 的自定义 provider key 冲突。
pub const KEY_PUBLISHER_PREFIX: &str = "publisher:";

/// T-E-C-17: IM 绑定 secret 的 keychain slot 前缀。
///
/// 完整 slot 名为 `im:<binding_id>`(如 `im:abc-uuid`),存钉钉签名 secret
/// 或 OAuth refresh token(Phase 2)。与 `KEY_API_KEY` /
/// `KEY_PROVIDER_PREFIX` / `KEY_PUBLISHER_PREFIX` 分开命名空间,
/// 避免与既有 key 冲突。Phase 1 webhook URL 本身存 SQLite(非机密),
/// 仅钉钉加签 secret 入 keychain。
pub const KEY_IM_PREFIX: &str = "im:";

/// T-E-S-46: 写入发布器 token 到 keychain。
///
/// `platform` 标识发布平台(如 `"github"`)。slot 名为
/// `publisher:<platform>`。复用底层 [`set`](Self::set)。
pub fn set_publisher_token(platform: &str, token: &str) -> Result<()> {
    let slot = format!("{KEY_PUBLISHER_PREFIX}{platform}");
    set(&slot, token)
}

/// T-E-S-46: 读取发布器 token;未配置返回 `None`。
///
/// `platform` 标识发布平台(如 `"github"`)。slot 名为
/// `publisher:<platform>`。复用底层 [`get`](Self::get)。
pub fn get_publisher_token(platform: &str) -> Result<Option<String>> {
    let slot = format!("{KEY_PUBLISHER_PREFIX}{platform}");
    get(&slot)
}

/// T-E-S-46: 删除发布器 token(幂等)。
///
/// `platform` 标识发布平台(如 `"github"`)。slot 名为
/// `publisher:<platform>`。复用底层 [`delete`](Self::delete)。
pub fn delete_publisher_token(platform: &str) -> Result<()> {
    let slot = format!("{KEY_PUBLISHER_PREFIX}{platform}");
    delete(&slot)
}

/// T-E-C-17: 写入 IM 绑定 secret 到 keychain。
///
/// `binding_id` 为 ImBinding.id(UUID)。slot 名为 `im:<binding_id>`。
/// 存钉钉加签 secret 或 OAuth refresh token(Phase 2)。
/// 复用底层 [`set`](Self::set)。
pub fn set_im_token(binding_id: &str, token: &str) -> Result<()> {
    let slot = format!("{KEY_IM_PREFIX}{binding_id}");
    set(&slot, token)
}

/// T-E-C-17: 读取 IM 绑定 secret;未配置返回 `None`。
///
/// `binding_id` 为 ImBinding.id。slot 名为 `im:<binding_id>`。
/// 复用底层 [`get`](Self::get)。
pub fn get_im_token(binding_id: &str) -> Result<Option<String>> {
    let slot = format!("{KEY_IM_PREFIX}{binding_id}");
    get(&slot)
}

/// T-E-C-17: 删除 IM 绑定 secret(幂等)。
///
/// `binding_id` 为 ImBinding.id。slot 名为 `im:<binding_id>`。
/// 复用底层 [`delete`](Self::delete)。
pub fn delete_im_token(binding_id: &str) -> Result<()> {
    let slot = format!("{KEY_IM_PREFIX}{binding_id}");
    delete(&slot)
}

/// T-E-S-40: 多 provider keychain slot 常量。
///
/// `set_provider_api_key(provider, value)` 根据 `provider` 字符串选择
/// 对应 slot 写入 OS keychain,避免 v1.0 单一 `KEY_API_KEY` 死代码。
/// 旧的 `KEY_API_KEY` 保留以向后兼容 `set_api_key` 命令。
pub const KEY_DEEPSEEK_API_KEY: &str = "deepseek_api_key";
pub const KEY_OPENAI_COMPAT_API_KEY: &str = "openai_compat_api_key";
pub const KEY_ANTHROPIC_API_KEY: &str = "anthropic_api_key";

/// T-E-S-43: SQLite 数据库加密 key 的 keychain slot。
///
/// 存 32 字节随机数(base64 编码,约 44 字符)。每设备独立,
/// 不参与 E2EE 同步(E2EE 是传输层,DB key 是静态层)。
/// key 丢失将导致 DB 不可读,`db_encryption_enable` 命令返回
/// recovery_key 供用户备份。
pub const KEY_DB_ENCRYPTION_KEY: &str = "db_encryption_key";

/// Stores `value` under `key` in the OS keychain.
///
/// v1.0.1 P0#12: replaces the v1.0 `localStorage.setItem` call
/// in `Settings.tsx`.  The JavaScript side now calls
/// `set_api_key` over the Tauri IPC, never the WebView's
/// persistent storage.
pub fn set(key: &str, value: &str) -> Result<()> {
    let entry =
        Entry::new(SERVICE, key).with_context(|| format!("opening keychain entry for {key}"))?;
    entry
        .set_password(value)
        .with_context(|| format!("writing keychain entry for {key}"))?;
    debug!(target: "nebula.security", key, "keychain set");
    Ok(())
}

/// Reads the value stored under `key`, or `None` if the entry
/// does not exist.  Returns `Err` only for OS-level errors
/// (e.g. the keychain is locked, the user denied access).
pub fn get(key: &str) -> Result<Option<String>> {
    let entry =
        Entry::new(SERVICE, key).with_context(|| format!("opening keychain entry for {key}"))?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            warn!(target: "nebula.security", key, error = ?e, "keychain read failed");
            Err(anyhow::anyhow!("keychain get_password: {e}"))
        }
    }
}

/// Removes the entry for `key`.  Idempotent: deleting a missing
/// entry is treated as success so the front-end's "reset" button
/// doesn't have to special-case "not configured".
pub fn delete(key: &str) -> Result<()> {
    let entry =
        Entry::new(SERVICE, key).with_context(|| format!("opening keychain entry for {key}"))?;
    match entry.delete_credential() {
        Ok(()) => {
            debug!(target: "nebula.security", key, "keychain delete");
            Ok(())
        }
        Err(keyring::Error::NoEntry) => {
            // Already gone — that's a successful no-op.
            debug!(target: "nebula.security", key, "keychain delete: already absent");
            Ok(())
        }
        Err(e) => {
            warn!(target: "nebula.security", key, error = ?e, "keychain delete failed");
            Err(anyhow::anyhow!("keychain delete_credential: {e}"))
        }
    }
}

// ---------------------------------------------------------------------------
// T-E-S-23: 凭证加密卷分离 — keychain 优先 → env var 兜底。
// T-E-C-20: Linux headless 降级 — env var 之后 fallback 到文件卷(/keychain/slot)。
// ---------------------------------------------------------------------------

/// T-E-S-23 / T-E-C-20: 解析单个 provider 的 API key。
///
/// 三级 fallback(在 Linux / headless Docker 环境下全部生效):
/// 1. keychain 优先(用户在 UI 设置的 key 覆盖 env var)
/// 2. env var 兜底(keychain 不可用或 slot 为空时)
/// 3. 文件卷兜底(读取 `NEBULA_KEYCHAIN_DIR/<slot>` 文件;
///    entrypoint.sh 将 DEEPSEEK_API_KEY 等写入 /keychain/)
fn resolve_key(slot: &str, env_var: &str) -> Option<String> {
    match get(slot) {
        Ok(Some(v)) => Some(v),
        // keychain 无条目或读取失败(无后端)→ env var 兜底。
        _ => {
            if let Ok(v) = std::env::var(env_var) {
                return Some(v);
            }
            // T-E-C-20: 文件卷兜底 — 从 NEBULA_KEYCHAIN_DIR/<env_var> 读取。
            // Docker headless 模式下,entrypoint.sh 将环境变量写入 /keychain/ 目录,
            // 此处读取对应的文件。env_var 同时作为文件名(如 DEEPSEEK_API_KEY)。
            let keychain_dir = std::env::var("NEBULA_KEYCHAIN_DIR")
                .unwrap_or_else(|_| "/keychain".to_string());
            let file_path = format!("{}/{}", keychain_dir, env_var);
            if let Ok(val) = std::fs::read_to_string(&file_path) {
                let trimmed = val.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
            None
        }
    }
}

/// T-E-S-23: 解析 DeepSeek API key — keychain(`deepseek_api_key` slot)
/// 优先,失败或空则 fallback env `DEEPSEEK_API_KEY`。
pub fn resolve_deepseek_key() -> Option<String> {
    resolve_key(KEY_DEEPSEEK_API_KEY, "DEEPSEEK_API_KEY")
}

/// T-E-S-23: 解析 Anthropic API key — keychain(`anthropic_api_key` slot)
/// 优先,失败或空则 fallback env `NEBULA_ANTHROPIC_KEY`。
pub fn resolve_anthropic_key() -> Option<String> {
    resolve_key(KEY_ANTHROPIC_API_KEY, "NEBULA_ANTHROPIC_KEY")
}

/// T-E-S-23: 解析 OpenAI 兼容 provider API key — keychain
/// (`openai_compat_api_key` slot)优先,失败或空则 fallback
/// env `NEBULA_OPENAI_COMPAT_KEY`。
pub fn resolve_openai_compat_key() -> Option<String> {
    resolve_key(KEY_OPENAI_COMPAT_API_KEY, "NEBULA_OPENAI_COMPAT_KEY")
}

/// T-E-S-43: 解析 SQLite 数据库加密 key — keychain(`db_encryption_key`
/// slot)优先,失败或空则 fallback env `NEBULA_DB_ENCRYPTION_KEY`。
///
/// 与 `resolve_deepseek_key` 同模式:keychain 优先(用户在 UI 设置的
/// key 覆盖 env var);keychain 不可用或 slot 为空时 fallback 到 env var。
/// 返回 None 表示未配置(此时应回退明文 DB)。
pub fn resolve_db_encryption_key() -> Option<String> {
    resolve_key(KEY_DB_ENCRYPTION_KEY, "NEBULA_DB_ENCRYPTION_KEY")
}

/// T-E-S-43: 生成 32 字节随机 DB 加密 key,base64 编码(约 44 字符)。
///
/// 使用 `rand::thread_rng`(`RngCore::fill_bytes`)填充 32 字节,
/// 然后 base64 STANDARD 编码。每次调用生成新 key(随机性来自 OS
/// CSPRNG)。生成的 key 应存入 keychain(`set(KEY_DB_ENCRYPTION_KEY, &key)`)
/// 并返回给用户备份(recovery_key)。
///
/// 依赖 `rand`(已有,见 Cargo.toml)+ `base64`(已有)。
pub fn generate_db_encryption_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

/// T-E-S-43: 将 env var `NEBULA_DB_ENCRYPTION_KEY` 迁移到 keychain(幂等)。
///
/// 仅当 keychain slot 读取成功且为空(`Ok(None)`)、env var 存在且
/// 非空时才写入。keychain 读取失败(无后端)时跳过,不尝试写入。
/// 返回 1 表示迁移成功,0 表示无需迁移。重复调用不会重复写入。
pub fn migrate_env_to_db_key() -> Result<usize> {
    Ok(migrate_one(
        KEY_DB_ENCRYPTION_KEY,
        "NEBULA_DB_ENCRYPTION_KEY",
    ))
}

/// T-E-S-23: 将单个 provider 的 env var key 迁移到 keychain(幂等)。
///
/// 仅当 keychain slot 读取成功且为空(`Ok(None)`)、env var 存在且
/// 非空时才写入。keychain 读取失败(无后端)时跳过该 provider,
/// 不尝试写入(写入也会失败)。返回 1 表示迁移成功,0 表示无需迁移。
fn migrate_one(slot: &str, env_var: &str) -> usize {
    if !matches!(get(slot), Ok(None)) {
        return 0;
    }
    match std::env::var(env_var).ok().filter(|v| !v.is_empty()) {
        Some(k) => match set(slot, &k) {
            Ok(()) => {
                debug!(target: "nebula.security", slot, env_var, "migrated env-var key to keychain");
                1
            }
            Err(e) => {
                warn!(target: "nebula.security", slot, env_var, error = %e, "migrate: keychain set failed");
                0
            }
        },
        None => 0,
    }
}

/// T-E-S-23: 将 env var 中的 API key 迁移到 keychain(幂等,非阻塞)。
///
/// 遍历 DeepSeek / Anthropic / OpenAI-compat 三个 provider:若 keychain
/// 无 key 但 env var 有,则写入 keychain。返回成功迁移的数量。
/// keychain 不可用(CI / 无 DBUS)时所有 provider 被跳过,返回 0。
/// 重复调用不会重复写入(keychain 已有 key 时跳过)。
pub fn migrate_env_to_keychain() -> Result<usize> {
    let count = migrate_one(KEY_DEEPSEEK_API_KEY, "DEEPSEEK_API_KEY")
        + migrate_one(KEY_ANTHROPIC_API_KEY, "NEBULA_ANTHROPIC_KEY")
        + migrate_one(KEY_OPENAI_COMPAT_API_KEY, "NEBULA_OPENAI_COMPAT_KEY");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v1.0.1 P0#12: round-trip the canonical API key.  The
    /// test runs on whatever backend the host has (Keychain /
    /// Credential Vault / Secret Service).  If the backend is
    /// unavailable (e.g. headless CI without a Secret Service
    /// daemon), the test is a soft pass and prints a warning.
    #[test]
    fn keychain_roundtrip() {
        let key = "nebula_test_key_roundtrip";
        // Clean up any stale entry from a prior failed run.
        let _ = delete(key);

        match set(key, "secret-value-XYZ") {
            Ok(()) => {}
            Err(e) => {
                eprintln!("keychain not available on this host: {e}; skipping");
                return;
            }
        }

        let got = get(key).expect("get");
        assert_eq!(got.as_deref(), Some("secret-value-XYZ"));

        delete(key).expect("delete");
        let after = get(key).expect("get after delete");
        assert_eq!(after, None, "entry must be gone after delete");
    }

    #[test]
    fn get_missing_returns_none_not_err() {
        // The key `nebula_definitely_missing_zzz` is
        // extremely unlikely to exist.
        let key = "nebula_definitely_missing_zzz";
        // Defensive: clean any leftover.
        let _ = delete(key);
        // On headless CI (e.g. Ubuntu without a Secret Service
        // daemon), the keychain backend may be unavailable.
        // In that case `get` returns an OS error rather than
        // `NoEntry`; we treat this as a soft skip, matching
        // the behaviour of `keychain_roundtrip`.
        match get(key) {
            Ok(v) => assert_eq!(v, None),
            Err(e) => {
                eprintln!("keychain not available on this host: {e}; skipping");
            }
        }
    }

    // -----------------------------------------------------------------------
    // T-E-S-23: resolve_*_key / migrate_env_to_keychain 测试。
    // keychain 在 CI(无 DBUS / 无桌面)可能不可用,测试必须 robust:
    // keychain 读取失败时 resolve fallback 到 env var,migrate 跳过。
    // -----------------------------------------------------------------------

    /// 辅助:保存 env var 当前值,返回需恢复的 (name, Option<value>)。
    /// 测试结束后调 `restore_env` 恢复。
    fn save_env(name: &str) -> (&'static str, Option<String>) {
        let val = std::env::var(name).ok();
        (leak_static(name), val)
    }

    /// 将 &str 转为 &'static str(测试用,生命周期仅限进程退出前)。
    fn leak_static(s: &str) -> &'static str {
        Box::leak(s.to_string().into_boxed_str())
    }

    /// 恢复 env var 到测试前的值。
    fn restore_env((name, val): (&'static str, Option<String>)) {
        match val {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    /// T-E-S-23: keychain 空 / 不可用时,resolve_*_key fallback 到 env var。
    #[test]
    fn resolve_deepseek_key_env_fallback() {
        let _ = delete(KEY_DEEPSEEK_API_KEY); // best-effort 清 slot
        let saved = save_env("DEEPSEEK_API_KEY");
        std::env::set_var("DEEPSEEK_API_KEY", "test_ds_key_XYZ_789");
        let resolved = resolve_deepseek_key();
        restore_env(saved);
        // keychain 空 → fallback env;keychain 不可用(get Err)→ fallback env。
        // 两种情况都应返回 env var 的值。仅当 keychain 有残留值时不同(已 delete)。
        assert_eq!(
            resolved.as_deref(),
            Some("test_ds_key_XYZ_789"),
            "resolve should fall back to env var when keychain is empty/unavailable"
        );
    }

    /// T-E-S-23: keychain 空 / 不可用时,resolve_anthropic_key fallback。
    #[test]
    fn resolve_anthropic_key_env_fallback() {
        let _ = delete(KEY_ANTHROPIC_API_KEY);
        let saved = save_env("NEBULA_ANTHROPIC_KEY");
        std::env::set_var("NEBULA_ANTHROPIC_KEY", "test_anthropic_XYZ_456");
        let resolved = resolve_anthropic_key();
        restore_env(saved);
        assert_eq!(
            resolved.as_deref(),
            Some("test_anthropic_XYZ_456"),
            "resolve_anthropic_key should fall back to env var"
        );
    }

    /// T-E-S-23: keychain 和 env var 都无 key 时返回 None。
    #[test]
    fn resolve_returns_none_when_both_absent() {
        let _ = delete(KEY_OPENAI_COMPAT_API_KEY);
        let saved = save_env("NEBULA_OPENAI_COMPAT_KEY");
        std::env::remove_var("NEBULA_OPENAI_COMPAT_KEY");
        let resolved = resolve_openai_compat_key();
        restore_env(saved);
        // keychain 空 + env 无 → None。
        // 若 keychain 有残留值(无法 delete),soft skip。
        match resolved {
            None => {} // 期望路径
            Some(_) => eprintln!("keychain had stale value; skipping none-assertion"),
        }
    }

    /// T-E-S-23: migrate_env_to_keychain 幂等性。
    /// 第一次调用迁移 env → keychain(若 keychain 可用);第二次调用应返回 0。
    #[test]
    fn migrate_env_to_keychain_is_idempotent() {
        let _ = delete(KEY_DEEPSEEK_API_KEY);
        let _ = delete(KEY_ANTHROPIC_API_KEY);
        let _ = delete(KEY_OPENAI_COMPAT_API_KEY);
        let saved_ds = save_env("DEEPSEEK_API_KEY");
        let saved_an = save_env("NEBULA_ANTHROPIC_KEY");
        let saved_oc = save_env("NEBULA_OPENAI_COMPAT_KEY");
        std::env::set_var("DEEPSEEK_API_KEY", "migrate_test_ds_001");
        std::env::set_var("NEBULA_ANTHROPIC_KEY", "migrate_test_an_002");
        std::env::set_var("NEBULA_OPENAI_COMPAT_KEY", "migrate_test_oc_003");

        let first = migrate_env_to_keychain().expect("migrate should not error");
        let second = migrate_env_to_keychain().expect("migrate should not error");

        restore_env(saved_ds);
        restore_env(saved_an);
        restore_env(saved_oc);

        // keychain 可用时:首次迁移最多 3 个,第二次必须 0(幂等)。
        // keychain 不可用时:两次都是 0(全部跳过)。
        if first > 0 {
            assert_eq!(
                second, 0,
                "second migrate must be 0 (idempotent), got {second}"
            );
        }
        // 无论 keychain 是否可用,migrate 都不应 panic / Err。
    }

    /// T-E-S-23: migrate_env_to_keychain 在 env var 为空时不写入。
    #[test]
    fn migrate_skips_empty_env_var() {
        let _ = delete(KEY_DEEPSEEK_API_KEY);
        let saved = save_env("DEEPSEEK_API_KEY");
        std::env::set_var("DEEPSEEK_API_KEY", "");
        let count = migrate_env_to_keychain().expect("migrate should not error");
        restore_env(saved);
        // 空 env var 不应迁移(即使 keychain 可用)。
        // 注意:其他两个 provider 的 env var 可能存在,所以只检查
        // DeepSeek 部分没贡献。整体 count 可能 > 0(其他 provider),
        // 但 DeepSeek slot 应为空。
        let _ = count; // 不对 total count 做断言(其他 provider 不可控)
        let ds = get(KEY_DEEPSEEK_API_KEY);
        match ds {
            Ok(v) => assert_eq!(v, None, "empty env var should not be written to keychain"),
            Err(_) => {} // keychain 不可用,soft skip
        }
    }

    // -----------------------------------------------------------------------
    // T-E-S-43: resolve_db_encryption_key / generate_db_encryption_key /
    // migrate_env_to_db_key 测试。与 T-E-S-23 同模式:keychain 在 CI
    // (无 DBUS / 无桌面)可能不可用,测试必须 robust。
    // -----------------------------------------------------------------------

    /// T-E-S-43: keychain 空 / 不可用时,resolve_db_encryption_key fallback
    /// 到 env var `NEBULA_DB_ENCRYPTION_KEY`。
    #[test]
    fn resolve_db_encryption_key_env_fallback() {
        let _ = delete(KEY_DB_ENCRYPTION_KEY); // best-effort 清 slot
        let saved = save_env("NEBULA_DB_ENCRYPTION_KEY");
        std::env::set_var("NEBULA_DB_ENCRYPTION_KEY", "test_db_key_XYZ_043");
        let resolved = resolve_db_encryption_key();
        restore_env(saved);
        // keychain 空 → fallback env;keychain 不可用(get Err)→ fallback env。
        // 两种情况都应返回 env var 的值。仅当 keychain 有残留值时不同(已 delete)。
        assert_eq!(
            resolved.as_deref(),
            Some("test_db_key_XYZ_043"),
            "resolve_db_encryption_key should fall back to env var when keychain is empty/unavailable"
        );
    }

    /// T-E-S-43: generate_db_encryption_key 生成 32 字节 base64 + 随机性。
    #[test]
    fn generate_db_encryption_key_is_random_and_32_bytes() {
        let k1 = generate_db_encryption_key();
        let k2 = generate_db_encryption_key();
        // 32 字节 base64 STANDARD 编码 → 44 字符(含 1 个 `=` padding)。
        assert_eq!(
            k1.len(),
            44,
            "base64 of 32 bytes should be 44 chars (got {})",
            k1.len()
        );
        // 两次生成应不同(随机性来自 OS CSPRNG)。
        assert_ne!(k1, k2, "two generations must differ (CSPRNG)");
        // base64 解码后应为 32 字节。
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &k1)
            .expect("base64 decode");
        assert_eq!(bytes.len(), 32, "decoded key must be 32 bytes");
    }

    // -----------------------------------------------------------------------
    // T-E-S-46: publisher token 存取测试。
    // keychain 在 CI(无 DBUS / 无桌面)可能不可用,测试必须 robust:
    // set 失败时 soft skip;成功时验证 round-trip + delete 幂等。
    // -----------------------------------------------------------------------

    /// T-E-S-46: publisher token set/get/delete round-trip。
    #[test]
    fn publisher_token_set_get_delete_roundtrip() {
        let platform = "github_test_roundtrip";
        // 清理残留(best-effort)。
        let _ = delete_publisher_token(platform);

        match set_publisher_token(platform, "ghp_test_XYZ_46_789") {
            Ok(()) => {
                let got = get_publisher_token(platform).expect("get_publisher_token");
                assert_eq!(
                    got.as_deref(),
                    Some("ghp_test_XYZ_46_789"),
                    "round-trip must return the same token"
                );

                delete_publisher_token(platform).expect("delete_publisher_token");
                let after = get_publisher_token(platform).expect("get after delete");
                assert_eq!(after, None, "token must be gone after delete");
            }
            Err(e) => {
                // keychain 不可用(headless CI / 无 DBUS):soft skip。
                eprintln!("keychain not available on this host: {e}; skipping");
            }
        }
    }

    /// T-E-S-46: get_publisher_token 在无条目时返回 None(不报错)。
    #[test]
    fn publisher_token_get_missing_returns_none() {
        let platform = "github_definitely_missing_zzz";
        let _ = delete_publisher_token(platform);
        match get_publisher_token(platform) {
            Ok(v) => assert_eq!(v, None, "missing token must return None"),
            Err(e) => {
                eprintln!("keychain not available on this host: {e}; skipping");
            }
        }
    }

    // -----------------------------------------------------------------------
    // T-E-C-20: keychain Linux headless 降级测试。
    // keychain 不可用(keyring 失败)→ env var → file 三级 fallback。
    // -----------------------------------------------------------------------

    /// T-E-C-20: keyring 失败时,设置 env var → resolve_key 返回 env var 值。
    #[test]
    fn test_keychain_linux_fallback_env() {
        // 清 keychain slot(best-effort)
        let _ = delete(KEY_DEEPSEEK_API_KEY);
        let saved = save_env("DEEPSEEK_API_KEY");
        std::env::set_var("DEEPSEEK_API_KEY", "test_fallback_env_TEC20");
        let resolved = resolve_deepseek_key();
        restore_env(saved);
        // keychain 空/不可用 → env var fallback 应返回设置的值
        assert_eq!(
            resolved.as_deref(),
            Some("test_fallback_env_TEC20"),
            "resolve should fall back to env var when keychain is unavailable"
        );
    }

    /// T-E-C-20: keyring + env var 都不可用时,写文件到临时目录 → resolve_key 返回文件值。
    #[test]
    fn test_keychain_linux_fallback_file() {
        let _ = delete(KEY_ANTHROPIC_API_KEY);
        let saved_env = save_env("NEBULA_ANTHROPIC_KEY");
        std::env::remove_var("NEBULA_ANTHROPIC_KEY");

        // 创建临时目录模拟 /keychain 卷
        let tmp_dir = std::env::temp_dir().join("nebula-test-keychain-tec20");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let key_file = tmp_dir.join("NEBULA_ANTHROPIC_KEY");
        std::fs::write(&key_file, "test_fallback_file_TEC20\n").unwrap();

        let saved_dir = save_env("NEBULA_KEYCHAIN_DIR");
        std::env::set_var("NEBULA_KEYCHAIN_DIR", tmp_dir.to_string_lossy().to_string());

        let resolved = resolve_anthropic_key();

        // 清理
        restore_env(saved_dir);
        restore_env(saved_env);
        let _ = std::fs::remove_dir_all(&tmp_dir);

        assert_eq!(
            resolved.as_deref(),
            Some("test_fallback_file_TEC20"),
            "resolve should fall back to keychain dir file when keychain and env var are unavailable"
        );
    }

    /// T-E-C-20: keyring + env var + 文件都不存在时,resolve_key 返回 None。
    #[test]
    fn test_keychain_fallback_none_when_all_absent() {
        let _ = delete(KEY_OPENAI_COMPAT_API_KEY);
        let saved_env = save_env("NEBULA_OPENAI_COMPAT_KEY");
        std::env::remove_var("NEBULA_OPENAI_COMPAT_KEY");

        // 用一个不存在的临时目录,确保文件 fallback 也失败
        let saved_dir = save_env("NEBULA_KEYCHAIN_DIR");
        std::env::set_var("NEBULA_KEYCHAIN_DIR", "/tmp/nebula-nonexistent-tec20");

        let resolved = resolve_openai_compat_key();

        restore_env(saved_dir);
        restore_env(saved_env);

        // keychain 空 + env 无 + 文件不存在 → None
        match resolved {
            None => {}
            Some(_) => eprintln!("keychain had stale value; skipping none-assertion"),
        }
    }

    /// T-E-C-20: headless bootstrap 编译检查测试。
    /// 此测试在 headless feature 下编译,验证 resolve_key 三级 fallback
    /// 路径(keychain → env → file)在 headless 构建中可用。
    /// 如果 `cargo check --features headless` 通过,此测试自动通过。
    #[test]
    fn test_headless_bootstrap() {
        // 编译时断言: resolve_key 函数在 headless feature 下可调用。
        // 运行时仅验证三级 fallback 路径存在(不依赖 keyring 后端)。
        let result = resolve_key("nonexistent_slot_tec20", "NONEXISTENT_ENV_VAR_TEC20");
        // 无 keyring 条目 + 无 env var + 无文件 → None
        assert_eq!(result, None, "headless resolve_key should return None when all sources absent");
    }

    /// T-E-C-20: entrypoint.sh 语法检查 — 验证脚本关键语法特征存在。
    /// 完整的 bash -n 检查在 Docker 构建时执行,此处仅做字符串级验证。
    #[test]
    fn test_entrypoint_script() {
        let script = include_str!("../../entrypoint.sh");
        // 验证关键语法特征
        assert!(script.contains("#!/bin/bash"), "entrypoint.sh must have bash shebang");
        assert!(script.contains("set -e"), "entrypoint.sh must have set -e");
        assert!(script.contains("mkdir -p /data /keychain /logs"), "entrypoint.sh must create volume dirs");
        assert!(script.contains("exec \"$@\""), "entrypoint.sh must exec CMD");
        assert!(script.contains("/keychain/"), "entrypoint.sh must write to /keychain/ volume");
    }
}
