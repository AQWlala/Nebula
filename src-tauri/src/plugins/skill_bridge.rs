//! T-E-S-35: Skill 层 bridge — `SkillEngineBridge`。
//!
//! 适配既有 `skills::SkillEngine` 为 `plugins::Skill` trait,不改 SkillEngine
//! 本身。方法对齐 SkillEngine 公共 API(use_skill / list_skills / search_skills)。
//!
//! `use_skill` 是 async(LLM/沙箱调用),`list_skills`/`search_skills` 在
//! SkillEngine 中是同步方法,bridge 用 async trait 包装后仍可直接调用。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::skills::{ListSkillsRequest, SkillEngine, SkillSearchRequest, UseSkillRequest};

use super::traits::{Skill, SkillRecord, SkillResult};

/// 适配 SkillEngine 为 `plugins::Skill` trait 的 bridge。
///
/// 持有 `Arc<SkillEngine>`,所有调用透传给引擎。零状态,可廉价 clone
/// (clone 共享底层 Arc)。
pub struct SkillEngineBridge {
    engine: Arc<SkillEngine>,
}

impl SkillEngineBridge {
    pub fn new(engine: Arc<SkillEngine>) -> Self {
        Self { engine }
    }

    /// 借用底层 SkillEngine(供需要直接访问引擎其他方法的调用方使用)。
    pub fn engine(&self) -> &Arc<SkillEngine> {
        &self.engine
    }
}

#[async_trait]
impl Skill for SkillEngineBridge {
    fn name(&self) -> &str {
        "skill_engine"
    }

    async fn use_skill(&self, id: &str, params: HashMap<String, String>) -> Result<SkillResult> {
        self.engine
            .use_skill(UseSkillRequest {
                id: id.to_string(),
                params,
            })
            .await
    }

    async fn list_skills(
        &self,
        language: Option<String>,
        tag: Option<String>,
        limit: u32,
    ) -> Result<Vec<SkillRecord>> {
        self.engine.list_skills(ListSkillsRequest {
            language,
            tag,
            limit,
            ..Default::default()
        })
    }

    async fn search_skills(&self, query: &str, limit: u32) -> Result<Vec<SkillRecord>> {
        self.engine.search_skills(SkillSearchRequest {
            query: query.to_string(),
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmGateway, OllamaClient};
    use crate::memory::sqlite_store::SqliteStore;
    use std::path::{Path, PathBuf};

    /// 临时 SQLite + 已运行 migrations,供 SkillEngine 构造。
    /// 镜像 `skills/engine.rs` 测试辅助函数(独立复制以避免跨模块依赖)。
    fn temp_db() -> (PathBuf, Arc<SqliteStore>) {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "nebula_plugins_skill_bridge_test_{}.db",
            uuid::Uuid::new_v4()
        ));
        let sqlite = Arc::new(SqliteStore::open(&p).expect("create should succeed"));
        {
            let rc = sqlite.raw_connection();
            let g = rc.lock();
            crate::memory::migration::run_bundled_migrations(&g).expect("test op should succeed");
        }
        (p, sqlite)
    }

    /// 指向不存在的 Ollama 端点(list/search 不会触达 LLM)。
    fn llm() -> Arc<LlmGateway> {
        let client = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        Arc::new(LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ))
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    #[tokio::test]
    async fn bridge_name_is_skill_engine() {
        let (p, sqlite) = temp_db();
        let engine = Arc::new(SkillEngine::new(sqlite, llm()));
        let bridge = SkillEngineBridge::new(engine);
        assert_eq!(bridge.name(), "skill_engine");
        cleanup(&p);
    }

    #[tokio::test]
    async fn bridge_list_skills_on_empty_store() {
        let (p, sqlite) = temp_db();
        let engine = Arc::new(SkillEngine::new(sqlite, llm()));
        let bridge = SkillEngineBridge::new(engine);

        let list = bridge
            .list_skills(None, None, 10)
            .await
            .expect("task should complete");
        assert!(list.is_empty(), "fresh engine should have no skills");

        let found = bridge
            .search_skills("anything", 10)
            .await
            .expect("query should succeed");
        assert!(found.is_empty());

        cleanup(&p);
    }

    #[tokio::test]
    async fn bridge_use_skill_unknown_id_errors() {
        let (p, sqlite) = temp_db();
        let engine = Arc::new(SkillEngine::new(sqlite, llm()));
        let bridge = SkillEngineBridge::new(engine);

        let err = bridge
            .use_skill("nonexistent-id", HashMap::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"), "got: {err}");

        cleanup(&p);
    }
}
