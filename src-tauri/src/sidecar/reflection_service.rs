//! T-S4-B-02: Sidecar Reflection 服务 — 单二进制多角色方案。
//!
//! 延续 T-S4-B-01 的单二进制多角色方案
//! (`nebula-sidecar --kind=reflection`),为自我反思引擎提供
//! 独立进程隔离。
//!
//! 本模块定义 Reflection sidecar 的服务处理器 [`ReflectionServiceHandler`],
//! 它包装 [`SelfReflectionEngine`] 并暴露与 gRPC 服务方法对应的 RPC 接口。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  gRPC (tonic)
//!    ▼
//! nebula-sidecar --kind=reflection  (监听 127.0.0.1:50055)
//!    │  ReflectionServiceHandler
//!    ▼
//! SelfReflectionEngine (L5 真反思 + 持久化)
//!    │
//!    ▼
//! SQLite (self_reflections 表)
//! ```
//!
//! ## RPC 映射
//!
//! | gRPC RPC              | SelfReflectionEngine 方法        |
//! |-----------------------|----------------------------------|
//! | `ReflectAll`          | `reflect_all`                    |
//! | `ListRecent`          | `list_recent_self_reflections`   |
//! | `PersistReflection`   | `persist_reflection`             |
//! | `HealthCheck`         | (always ok)                      |
//!
//! ## 依赖
//!
//! 依赖 T-S1-A-06 已完成的 `self_reflections` 表持久化(护栏状态机)。
//! `ReflectConfig` 控制反思频率/阈值,默认值适用于 sidecar 模式。

use std::sync::Arc;

use anyhow::Result;
use tracing::{info, instrument};

use crate::memory::self_reflection::{SelfReflection, SelfReflectionEngine};

/// Reflection sidecar 服务处理器。
///
/// 包装 [`SelfReflectionEngine`],为 gRPC 服务端提供业务逻辑入口。
/// 在进程内模式下也可直接使用(无需 gRPC)。
pub struct ReflectionServiceHandler {
    engine: Arc<SelfReflectionEngine>,
}

impl ReflectionServiceHandler {
    /// 创建新的 Reflection 服务处理器。
    ///
    /// 通常在 sidecar 进程启动时构造,持有与主进程相同的
    /// `SelfReflectionEngine` 实例(或通过共享 SQLite 数据库重建)。
    pub fn new(engine: Arc<SelfReflectionEngine>) -> Self {
        info!(
            target: "nebula.sidecar.reflection",
            "ReflectionServiceHandler initialized"
        );
        Self { engine }
    }

    /// 访问底层 SelfReflectionEngine(供 IPC 客户端在进程内模式下直接调用)。
    pub fn engine(&self) -> &Arc<SelfReflectionEngine> {
        &self.engine
    }

    /// RPC: HealthCheck — 始终返回 Ok(若 handler 存在则服务可用)。
    pub async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// RPC: ReflectAll — 执行一次完整的自我反思(所有三种类型)。
    ///
    /// 结果会自动持久化到 `self_reflections` 表(T-S1-A-06)。
    #[instrument(skip(self))]
    pub async fn reflect_all(&self) -> Result<Vec<SelfReflection>> {
        self.engine.reflect_all().await
    }

    /// RPC: ListRecent — 查询历史反思记录(按时间倒序)。
    ///
    /// 返回 `(id, kind, title, content, created_at)` 元组列表,
    /// 供 UI 历史回溯面板使用。
    #[instrument(skip(self), fields(limit = limit))]
    pub fn list_recent(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, String, String, String, i64)>> {
        self.engine.list_recent_self_reflections(limit)
    }

    /// RPC: PersistReflection — 持久化单条反思到 `self_reflections` 表。
    ///
    /// 供外部系统(如 PlanEngine 回调)写入反思结论时使用。
    #[instrument(skip(self, reflection), fields(kind = reflection.kind.as_str()))]
    pub fn persist_reflection(&self, reflection: &SelfReflection) -> Result<()> {
        self.engine.persist_reflection(reflection)
    }
}

impl std::fmt::Debug for ReflectionServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflectionServiceHandler")
            .field("engine", &"Arc<SelfReflectionEngine>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::reflect::ReflectConfig;
    use crate::memory::sqlite_store::SqliteStore;
    use crate::memory::values::ValuesLayer;

    fn make_handler() -> ReflectionServiceHandler {
        let tmp = std::env::temp_dir().join(format!(
            "nebula_reflection_sidecar_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&tmp);
        let sqlite = Arc::new(SqliteStore::open(&tmp).expect("open temp db"));
        let engine = Arc::new(SelfReflectionEngine::new(
            sqlite,
            ValuesLayer::with_defaults(),
            ReflectConfig::default(),
        ));
        ReflectionServiceHandler::new(engine)
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let h = make_handler();
        assert!(h.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn reflect_all_returns_empty_when_no_memories() {
        // 无近期记忆时 reflect_all 应返回空 Vec(不报错)。
        let h = make_handler();
        let results = h.reflect_all().await.expect("reflect_all should succeed");
        assert!(results.is_empty(), "expected no reflections without memories");
    }

    #[test]
    fn list_recent_returns_empty_initially() {
        let h = make_handler();
        let rows = h.list_recent(10).expect("list_recent should succeed");
        assert!(rows.is_empty(), "expected no persisted reflections initially");
    }

    #[test]
    fn persist_and_list_reflection() {
        let h = make_handler();
        let reflection = SelfReflection {
            kind: crate::memory::self_reflection::ReflectionKind::ValueAlignment,
            title: "test reflection".to_string(),
            content: "reflecting on test".to_string(),
            insights: vec!["insight one".to_string()],
            action_items: vec!["do better".to_string()],
            confidence: 0.8,
            severity: 0.3,
            related_memory_ids: vec![],
        };
        h.persist_reflection(&reflection).expect("persist should succeed");
        let rows = h.list_recent(10).expect("list_recent should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "value_alignment");
        assert_eq!(rows[0].2, "test reflection");
    }

    #[test]
    fn engine_accessor_works() {
        let h = make_handler();
        let _ = h.engine();
        // 只要能拿到引用且编译通过即可证明访问器工作。
    }
}
