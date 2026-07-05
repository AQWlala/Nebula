//! T-E-S-43: SQLite 明文 ↔ 密文迁移(`CipherMigrator`)。
//!
//! 用 SQLCipher 的 `sqlcipher_export()` 模式实现整库加密 / 解密:
//!
//! - **加密**(`encrypt_plaintext_db`):重命名明文 DB → `.plain.bak`,
//!   用 key 打开新 DB(空),`ATTACH` 旧 DB 作为 `plain KEY ''`,
//!   `SELECT sqlcipher_export('main', 'plain')` 把数据从明文导入密文,
//!   `DETACH`。
//! - **解密**(`decrypt_to_plaintext`):反向,`ATTACH` 加密 DB 作为
//!   `enc KEY '<key>'`,`SELECT sqlcipher_export('main', 'enc')` 把数据
//!   从密文导出为明文。
//!
//! `sqlcipher_export` 是 SQLCipher 提供的内置函数,等价于
//! `INSERT INTO main.<table> SELECT * FROM plain.<table>` 的批量化版本,
//! 自动处理所有表 + 索引 + 触发器 + 视图。
//!
//! **feature gate**:整个文件仅在 `sqlcipher` feature 启用时编译。
//! 无 feature 时 `CipherMigrator` 不存在,`db_encryption_enable` /
//! `db_encryption_disable` 命令返回 "sqlcipher feature not enabled" 错误。

#![cfg(feature = "sqlcipher")]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// T-E-S-43: SQLite 明文 ↔ 密文迁移器。
///
/// 所有方法仅在 `sqlcipher` feature 启用时编译(`#![cfg(feature = "sqlcipher")]`
/// 守卫整个文件)。无 feature 时整个文件不参与编译。
///
/// 迁移是幂等的:加密后明文 DB 被重命名为 `.plain.bak`(不删除,供备份);
/// 解密后加密 DB 被重命名为 `.enc.bak`(不删除,供备份)。
pub struct CipherMigrator;

impl CipherMigrator {
    /// 将明文 DB 加密为密文 DB(用 `key` 加密)。
    ///
    /// # 流程
    ///
    /// 1. 重命名 `plain_path` 为 `{plain_path}.plain.bak`(备份原明文 DB)。
    /// 2. 用 `key` 打开新 DB(空,位于 `plain_path` 原位置)。
    /// 3. `ATTACH DATABASE '{plain_path}.plain.bak' AS plain KEY '';`(明文 DB 无 key)。
    /// 4. `SELECT sqlcipher_export('main', 'plain');` — 把数据从 `plain` 导入 `main`。
    /// 5. `DETACH DATABASE plain;`
    /// 6. 返回 `.plain.bak` 路径(供调用方备份 / 清理)。
    ///
    /// # 安全说明
    ///
    /// `ATTACH` 的路径来自调用方(内部构造,非用户输入),key 通过参数转义
    /// 单引号(`'` → `''`)防止 SQL 注入。生产环境应优先用参数绑定,但
    /// `ATTACH ... KEY` 的参数绑定在部分 rusqlite 版本不稳定,故用转义。
    pub fn encrypt_plaintext_db(plain_path: &Path, key: &str) -> Result<PathBuf> {
        let backup_path = PathBuf::from(format!("{}.plain.bak", plain_path.display()));

        // 1. 重命名明文 DB 为 .plain.bak。
        std::fs::rename(plain_path, &backup_path).with_context(|| {
            format!(
                "renaming plaintext db to backup: {}",
                backup_path.display()
            )
        })?;

        // 2. 用 key 打开新 DB(空,位于原位置)。
        let conn = Connection::open(plain_path)
            .with_context(|| format!("opening new encrypted db at {}", plain_path.display()))?;
        conn.pragma_update(None, "key", key)
            .context("setting PRAGMA key (sqlcipher not compiled?)")?;

        // 3. ATTACH 明文 DB 作为 plain(KEY '')。
        //    路径转义:替换单引号为两个单引号(SQL 字符串字面量标准转义)。
        let escaped_backup = backup_path.display().to_string().replace('\'', "''");
        let attach_sql = format!("ATTACH DATABASE '{}' AS plain KEY ''", escaped_backup);
        conn.execute_batch(&attach_sql)
            .context("attaching plaintext db as plain (KEY '')")?;

        // 4. sqlcipher_export('main', 'plain') — 从 plain 导入到 main。
        //    sqlcipher_export 是 SQLCipher 内置函数,批量复制所有表 + 索引 + 触发器。
        conn.execute_batch("SELECT sqlcipher_export('main', 'plain');")
            .context("sqlcipher_export('main', 'plain') failed")?;

        // 5. DETACH plain。
        conn.execute_batch("DETACH DATABASE plain;")
            .context("detaching plain")?;

        Ok(backup_path)
    }

    /// 将加密 DB 解密为明文 DB(反向)。
    ///
    /// # 流程
    ///
    /// 1. 重命名 `enc_path` 为 `{enc_path}.enc.bak`(备份原加密 DB)。
    /// 2. 打开新明文 DB(空,位于 `enc_path` 原位置,无 key)。
    /// 3. `ATTACH DATABASE '{enc_path}.enc.bak' AS enc KEY '<key>';`。
    /// 4. `SELECT sqlcipher_export('main', 'enc');` — 把数据从 `enc` 导出为 `main`(明文)。
    /// 5. `DETACH DATABASE enc;`
    /// 6. 返回 `.enc.bak` 路径(供调用方备份 / 清理)。
    ///
    /// # 安全说明
    ///
    /// 调用方需提供正确的 `key`(错误的 key 会导致 ATTACH 后查询失败,
    /// "file is not a database")。
    pub fn decrypt_to_plaintext(enc_path: &Path, key: &str) -> Result<PathBuf> {
        let backup_path = PathBuf::from(format!("{}.enc.bak", enc_path.display()));

        // 1. 重命名加密 DB 为 .enc.bak。
        std::fs::rename(enc_path, &backup_path).with_context(|| {
            format!(
                "renaming encrypted db to backup: {}",
                backup_path.display()
            )
        })?;

        // 2. 打开新明文 DB(空,位于原位置,无 key)。
        let conn = Connection::open(enc_path)
            .with_context(|| format!("opening new plaintext db at {}", enc_path.display()))?;

        // 3. ATTACH 加密 DB 作为 enc(KEY '<key>')。
        //    路径 + key 转义:替换单引号为两个单引号。
        let escaped_backup = backup_path.display().to_string().replace('\'', "''");
        let escaped_key = key.replace('\'', "''");
        let attach_sql = format!(
            "ATTACH DATABASE '{}' AS enc KEY '{}'",
            escaped_backup, escaped_key
        );
        conn.execute_batch(&attach_sql)
            .context("attaching encrypted db as enc (KEY <key>)")?;

        // 4. sqlcipher_export('main', 'enc') — 从 enc 导出为 main(明文)。
        conn.execute_batch("SELECT sqlcipher_export('main', 'enc');")
            .context("sqlcipher_export('main', 'enc') failed")?;

        // 5. DETACH enc。
        conn.execute_batch("DETACH DATABASE enc;")
            .context("detaching enc")?;

        Ok(backup_path)
    }

    /// 查询 SQLCipher 版本字符串。
    ///
    /// 返回类似 `"4.5.5 community"` 的版本字符串。若 sqlcipher 未编译,
    /// `PRAGMA cipher_version` 不存在,返回 `Err`。
    pub fn cipher_version(conn: &Connection) -> Result<String> {
        let v: String = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .context("querying PRAGMA cipher_version (sqlcipher not compiled?)")?;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sqlite_store::SqliteStore;
    use crate::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};

    fn temp_db_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "nebula_test_cipher_{}.db",
            uuid::Uuid::new_v4()
        ));
        p
    }

    /// T-E-S-43: 明文→密文迁移(sqlcipher_export)。
    /// 预置明文 DB with 数据 → encrypt → open_encrypted → 验证数据完整。
    #[tokio::test]
    async fn encrypt_plaintext_db_migrates_data() {
        let path = temp_db_path();
        // 1. 预置明文 DB with 数据。
        let store = SqliteStore::open(&path).expect("open plaintext");
        let mut m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "encrypt test content",
            SourceKind::UserInput,
        );
        m.id = "cipher-test-001".to_string();
        m.summary = crate::memory::types::MultiGranularity::new(
            "enc",
            "encrypt test",
            "encrypt test content",
            "encrypt test content for cipher migration",
        );
        store.insert_guarded(&m).expect("insert");
        drop(store);

        // 2. encrypt:明文 DB → 加密 DB。
        let key = crate::security::keychain::generate_db_encryption_key();
        let backup = CipherMigrator::encrypt_plaintext_db(&path, &key).expect("encrypt");
        assert!(backup.exists(), "backup .plain.bak file should exist");

        // 3. open_encrypted 验证数据完整。
        let enc_store = SqliteStore::open_encrypted(&path, &key).expect("open_encrypted");
        let got = enc_store.get("cipher-test-001").await.expect("get");
        assert!(got.is_some(), "migrated memory must exist in encrypted db");
        let got = got.unwrap();
        assert_eq!(got.content, "encrypt test content");
        assert_eq!(got.layer, MemoryLayer::L3);

        // 清理。
        drop(enc_store);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(backup);
    }

    /// T-E-S-43: cipher_version 通过 Connection 查询非空。
    #[tokio::test]
    async fn cipher_version_returns_nonempty() {
        let path = temp_db_path();
        let key = crate::security::keychain::generate_db_encryption_key();
        let store = SqliteStore::open_encrypted(&path, &key).expect("open_encrypted");
        let conn = store.raw_connection();
        let lock = conn.lock();
        let v = CipherMigrator::cipher_version(&lock).expect("cipher_version");
        assert!(!v.is_empty(), "cipher_version must be non-empty");
        drop(lock);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
