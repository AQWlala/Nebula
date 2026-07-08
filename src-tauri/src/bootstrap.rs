//! Bootstrap phase — constructs all subsystems and assembles [`AppState`].
//!
//! T-D-B-02: 原单文件 1073 行拆分为 5 个子模块:
//! - [`core`]: `bootstrap()` 主入口装配
//! - [`storage`]: SQLite + LanceDB 初始化
//! - [`ai_core`]: embedder/LlmGateway/sponge/blackhole/cost_tracker 装配 + ACL 加载
//! - [`swarm`]: SwarmOrchestrator + ReflectionEngine + Skills 生态
//! - [`platform`]: editor/clipboard/sync/channels 辅助构造
//!
//! 本文件仅保留 `shutdown()` 和 `try_compile_soul()` 两个 AppState 方法。

use std::time::Duration;

use tracing::{info, warn};

use crate::app_state::AppState;

mod ai_core;
mod core;
mod platform;
mod storage;
mod swarm;

impl AppState {
    /// Wakes the background reflection worker, signals the gRPC
    /// server to stop, and awaits both joins with a brief grace
    /// period. Idempotent and safe to call from Tauri shutdown.
    pub async fn shutdown(&self) {
        let notify = self.memory.reflection.shutdown_handle();
        notify.notify_waiters();

        let worker = { self.memory.reflect_worker.lock().take() };
        if let Some(h) = worker {
            match tokio::time::timeout(Duration::from_millis(250), h).await {
                Ok(_) => info!(target: "nebula", "reflection worker stopped"),
                Err(_) => warn!(target: "nebula", "reflection worker did not stop in time"),
            }
        }

        #[cfg(feature = "grpc")]
        {
            let grpc = { self.platform.grpc_server.lock().take() };
            if let Some(h) = grpc {
                h.shutdown().await;
            }
        }

        {
            self.memory.file_watcher_worker.lock().take();
        }
        self.memory.file_watcher.stop().await;

        {
            let mut watcher = self.platform.clipboard_watcher.lock().await;
            watcher.stop();
        }

        self.swarm.trigger_engine.stop();
    }

    /// M1 任务 #23: 尝试 Soul 编译（cfg-gated）。
    #[cfg(feature = "soul-system")]
    pub(crate) async fn try_compile_soul(&self) -> Option<String> {
        use crate::soul::soul_system_enabled;

        if !soul_system_enabled() {
            return None;
        }

        let compiler = self.infra.config.soul_compiler.as_ref()?;

        let soul_md_text = self.infra.config.persona.as_ref().and_then(|pc| {
            let guard = pc.read();
            guard.soul_md.clone()
        })?;

        if soul_md_text.trim().is_empty() {
            return None;
        }

        match compiler.compile(&soul_md_text).await {
            Ok(compiled) => {
                if compiled.degraded {
                    tracing::warn!(
                        target: "nebula.soul",
                        warnings = compiled.warnings.len(),
                        "Soul compiled in degraded mode (text-only, no LLM)"
                    );
                } else {
                    tracing::info!(
                        target: "nebula.soul",
                        warnings = compiled.warnings.len(),
                        "Soul compiled successfully"
                    );
                }
                if compiled.system_prompt.is_empty() {
                    None
                } else {
                    Some(compiled.system_prompt)
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "nebula.soul",
                    error = %e,
                    "Soul compile failed; falling back to PersonaConfig"
                );
                None
            }
        }
    }
}
