//! T-S4-B-01: Sidecar Skill 服务 — 单二进制多角色方案。
//!
//! 根据 EXPERT_REVIEW §4.1 决议,采用单二进制多角色方案
//! (`nebula-sidecar --kind=skill`),而非为每种服务编译独立二进制。
//!
//! 本模块定义 Skill sidecar 的服务处理器 [`SkillServiceHandler`],它包装
//! [`SkillEngine`] 并暴露与 gRPC 服务方法对应的 RPC 接口。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  gRPC (tonic)
//!    ▼
//! nebula-sidecar --kind=skill  (监听 127.0.0.1:50054)
//!    │  SkillServiceHandler
//!    ▼
//! SkillEngine (CRUD + 执行 + 审计)
//!    │
//!    ▼
//! SQLite + LLM Gateway
//! ```
//!
//! ## RPC 映射
//!
//! | gRPC RPC          | SkillEngine 方法   |
//! |-------------------|--------------------|
//! | `CreateSkill`     | `create_skill`     |
//! | `ExecuteSkill`    | `use_skill`        |
//! | `ListSkills`      | `list_skills`      |
//! | `SearchSkills`    | `search_skills`    |
//! | `RateSkill`       | `rate_skill`       |
//! | `HealthCheck`     | (always ok)        |

use std::sync::Arc;

use anyhow::Result;
use tracing::{info, instrument};

use crate::skills::engine::SkillEngine;
use crate::skills::types::{
    CreateSkillRequest, ListSkillsRequest, RateSkillRequest, Skill, SkillResult,
    SkillSearchRequest, UseSkillRequest,
};

/// Skill sidecar 服务处理器。
///
/// 包装 [`SkillEngine`],为 gRPC 服务端提供业务逻辑入口。
/// 在进程内模式下也可直接使用(无需 gRPC)。
pub struct SkillServiceHandler {
    engine: Arc<SkillEngine>,
}

impl SkillServiceHandler {
    /// 创建新的 Skill 服务处理器。
    ///
    /// 通常在 sidecar 进程启动时构造,持有与主进程相同的 SkillEngine 实例
    /// (或通过共享 SQLite 数据库重建)。
    pub fn new(engine: Arc<SkillEngine>) -> Self {
        info!(
            target: "nebula.sidecar.skill",
            "SkillServiceHandler initialized"
        );
        Self { engine }
    }

    /// 访问底层 SkillEngine(供 IPC 客户端在进程内模式下直接调用)。
    pub fn engine(&self) -> &Arc<SkillEngine> {
        &self.engine
    }

    /// RPC: HealthCheck — 始终返回 Ok(若 handler 存在则服务可用)。
    pub async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// RPC: CreateSkill — 创建新技能。
    #[instrument(skip(self, req), fields(name = %req.name))]
    pub fn create_skill(&self, req: CreateSkillRequest) -> Result<Skill> {
        self.engine.create_skill(req)
    }

    /// RPC: ExecuteSkill — 执行技能(映射到 SkillEngine::use_skill)。
    #[instrument(skip(self, req), fields(skill_id = %req.id))]
    pub async fn execute_skill(&self, req: UseSkillRequest) -> Result<SkillResult> {
        self.engine.use_skill(req).await
    }

    /// RPC: ListSkills — 列出技能。
    pub fn list_skills(&self, req: ListSkillsRequest) -> Result<Vec<Skill>> {
        self.engine.list_skills(req)
    }

    /// RPC: SearchSkills — 搜索技能。
    pub fn search_skills(&self, req: SkillSearchRequest) -> Result<Vec<Skill>> {
        self.engine.search_skills(req)
    }

    /// RPC: RateSkill — 为技能评分。
    pub fn rate_skill(&self, req: RateSkillRequest) -> Result<Skill> {
        self.engine.rate_skill(req)
    }
}

impl std::fmt::Debug for SkillServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillServiceHandler")
            .field("engine", &"Arc<SkillEngine>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use crate::memory::sqlite_store::SqliteStore;
    use crate::skills::types::{CreateSkillRequest, ListSkillsRequest};

    fn make_handler() -> SkillServiceHandler {
        let tmp = std::env::temp_dir().join(format!(
            "nebula_skill_sidecar_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&tmp);
        let sqlite = Arc::new(SqliteStore::open(&tmp).expect("open temp db"));
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        let gw = std::sync::Arc::new(LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let engine = Arc::new(SkillEngine::new(sqlite, gw));
        SkillServiceHandler::new(engine)
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let h = make_handler();
        assert!(h.health_check().await.unwrap());
    }

    #[test]
    fn create_skill_via_handler() {
        let h = make_handler();
        let req = CreateSkillRequest {
            name: "test-skill".to_string(),
            description: "test".to_string(),
            code: "print('hello')".to_string(),
            language: "python".to_string(),
            tags: vec![],
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: Default::default(),
        };
        let skill = h.create_skill(req).expect("create skill");
        assert_eq!(skill.name, "test-skill");
    }

    #[test]
    fn list_skills_via_handler() {
        let h = make_handler();
        let req = CreateSkillRequest {
            name: "listable-skill".to_string(),
            description: "test".to_string(),
            code: "print('hi')".to_string(),
            language: "python".to_string(),
            tags: vec![],
            source_memory_id: None,
            activation_condition: None,
            platform: None,
            min_confidence: None,
            trust_level: 0,
            permissions: vec![],
            capabilities: Default::default(),
        };
        h.create_skill(req).unwrap();
        let skills = h
            .list_skills(ListSkillsRequest {
                language: None,
                tag: None,
                limit: 10,
                ..Default::default()
            })
            .expect("list skills");
        assert!(!skills.is_empty());
    }

    #[test]
    fn engine_accessor_works() {
        let h = make_handler();
        let _ = h.engine();
    }
}
