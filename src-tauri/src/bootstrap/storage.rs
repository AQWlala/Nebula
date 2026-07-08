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
