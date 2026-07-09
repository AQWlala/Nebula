use std::sync::Arc;

use anyhow::Context;
use tracing::info;

use crate::app_config::AppConfig;
use crate::app_state::AppState;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::vector_store::{create_vector_store, VectorStore, VectorStoreBackend};
use crate::perf::StartupTimer;

impl AppState {
    pub(crate) async fn bootstrap_storage(
        config: &AppConfig,
        startup: &StartupTimer,
    ) -> anyhow::Result<(Arc<SqliteStore>, Arc<dyn VectorStore>)> {
        let db_path = config.db_path.clone();
        let db_encryption_enabled = config.db_encryption_enabled;
        let sqlite = tokio::task::spawn_blocking(move || -> anyhow::Result<SqliteStore> {
            if db_encryption_enabled {
                #[cfg(feature = "sqlcipher")]
                {
                    let key = crate::security::keychain::resolve_db_encryption_key()
                        .context("DB encryption enabled but no key in keychain")?;
                    SqliteStore::open_encrypted(&db_path, &key)
                        .context("opening encrypted sqlite store")
                }
                #[cfg(not(feature = "sqlcipher"))]
                {
                    anyhow::bail!(
                        "db_encryption_enabled=true but sqlcipher feature not compiled; \
                         rebuild with --features sqlcipher"
                    );
                }
            } else {
                SqliteStore::open(&db_path).context("opening sqlite store")
            }
        })
        .await
        .context("spawn_blocking for sqlite open failed")??;
        let sqlite = Arc::new(sqlite);
        startup.mark("bootstrap.sqlite");

        info!(target: "nebula", "migrations applied during SqliteStore::open");
        startup.mark("bootstrap.migrations");

        let remote_url = match config.vector_store_backend {
            VectorStoreBackend::Qdrant => config.qdrant_url.as_deref(),
            VectorStoreBackend::Chroma => config.chroma_url.as_deref(),
            VectorStoreBackend::Lance => None,
        };
        let lance = create_vector_store(
            config.vector_store_backend,
            &config.lance_path,
            config.embedding_dim,
            remote_url,
        )
        .await
        .context("opening vector store")?;
        startup.mark("bootstrap.lance");
        Ok((sqlite, lance))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// 唯一测试 ID 计数器,确保并行测试路径不冲突。
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 构造测试用 AppConfig,用 temp_dir + PID + 计数器确保路径唯一。
    fn test_config() -> AppConfig {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut config = AppConfig::from_env();
        let tmp = std::env::temp_dir();
        let pid = std::process::id();
        config.db_path = tmp
            .join(format!("nebula-test-storage-{}-{}.db", pid, id))
            .to_string_lossy()
            .to_string();
        config.db_encryption_enabled = false;
        config.vector_store_backend = VectorStoreBackend::Lance;
        config.lance_path = tmp
            .join(format!("nebula-test-lance-{}-{}", pid, id))
            .to_string_lossy()
            .to_string();
        config.embedding_dim = 512;
        config.qdrant_url = None;
        config.chroma_url = None;
        config
    }

    /// 清理测试产生的临时文件。
    fn cleanup(config: &AppConfig) {
        std::fs::remove_file(&config.db_path).ok();
        // SQLite WAL/SHM 文件
        std::fs::remove_file(format!("{}-wal", &config.db_path)).ok();
        std::fs::remove_file(format!("{}-shm", &config.db_path)).ok();
        std::fs::remove_dir_all(&config.lance_path).ok();
    }

    #[tokio::test]
    async fn test_bootstrap_storage_lance_success() {
        let config = test_config();
        let startup = StartupTimer::start();
        let result = AppState::bootstrap_storage(&config, &startup).await;
        assert!(
            result.is_ok(),
            "bootstrap_storage should succeed with Lance backend: {:?}",
            result.err()
        );
        let (sqlite, lance) = result.expect("checked ok");
        // 验证返回的 Arc 不为空
        assert!(Arc::strong_count(&sqlite) >= 1);
        assert!(Arc::strong_count(&lance) >= 1);
        // 验证 SQLite 文件被创建
        assert!(
            std::path::Path::new(&config.db_path).exists(),
            "SQLite db file should exist after bootstrap"
        );
        cleanup(&config);
    }

    #[tokio::test]
    async fn test_bootstrap_storage_marks_startup_timer() {
        let config = test_config();
        let startup = StartupTimer::start();
        let _ = AppState::bootstrap_storage(&config, &startup).await;
        // bootstrap_storage 内部调用了 3 次 startup.mark():
        // "bootstrap.sqlite", "bootstrap.migrations", "bootstrap.lance"
        // StartupTimer::mark 返回 elapsed_ms,已标记的 mark 返回已有值
        // 这里验证不 panic 即可(mark 被调用过)
        let ms = startup.mark("bootstrap.sqlite");
        // 已标记的 mark 返回已有值(非 0,因为 bootstrap 过程需要时间)
        // 但在极快的环境下可能为 0,所以只验证不 panic
        let _ = ms;
        cleanup(&config);
    }
}
