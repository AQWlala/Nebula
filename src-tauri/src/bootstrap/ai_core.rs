use std::sync::Arc;

use tauri::Emitter;
use tracing::{info, warn};

use crate::app_config::AppConfig;
use crate::app_state::AppState;
use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::OllamaClient;
use crate::llm::openai_compat::OpenAICompatClient;
use crate::llm::semantic_cache::SemanticCache;
use crate::memory::acl::{AclEffect, AclPermission, AclRule, MemoryAcl};
use crate::memory::blackhole::BlackholeEngine;
use crate::memory::embedder::Embedder;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::vector_store::VectorStore;
use crate::perf::StartupTimer;

impl AppState {
    pub(crate) async fn bootstrap_ai_core(
        config: &AppConfig,
        sqlite: &Arc<SqliteStore>,
        lance: &Arc<dyn VectorStore>,
        startup: &StartupTimer,
        app_handle: &tauri::AppHandle,
    ) -> anyhow::Result<(
        Arc<Embedder>,
        Arc<LlmGateway>,
        Arc<SpongeEngine>,
        Arc<BlackholeEngine>,
        Arc<crate::skills::exec_approval::ExecApprovalTracker>,
        Arc<crate::llm::cost_tracker::CostTracker>,
        Arc<crate::editor::InlineCompletionEngine>,
        Arc<parking_lot::RwLock<crate::llm::models_config::ModelsConfig>>,
        Arc<SemanticCache>,
    )> {
        let models_config_value =
            crate::llm::models_config::ModelsConfig::load(&config.models_config_path);
        info!(
            target: "nebula",
            path = %config.models_config_path.display(),
            providers = models_config_value.providers.len(),
            "models.json loaded"
        );
        let embedder = Arc::new(Embedder::new(
            OllamaClient::new(config.ollama_url.clone()),
            config.embed_model.clone(),
            config.embedding_dim,
        ));
        let ollama = Arc::new(OllamaClient::new(config.ollama_url.clone()));
        let inline_model = std::env::var("NEBULA_INLINE_MODEL")
            .unwrap_or_else(|_| "qwen2.5-coder:0.5b".to_string());
        let inline_completion = Arc::new(crate::editor::InlineCompletionEngine::new(
            ollama.clone(),
            inline_model,
        ));
        let ollama_for_compress = ollama.clone();
        let ak = crate::security::keychain::resolve_anthropic_key();
        let am = std::env::var("NEBULA_ANTHROPIC_MODEL").ok();
        let exec_approval = Arc::new(crate::skills::exec_approval::ExecApprovalTracker::new(
            config.exec_approval_timeout_secs,
        ));
        let cost_tracker = {
            use crate::llm::cost_tracker::CostTracker;
            let base = CostTracker::new();
            let base = match config.automation_daily_budget_usd {
                Some(budget) if budget > 0.0 => {
                    let app_for_emit = app_handle.clone();
                    let callback: Arc<dyn Fn(crate::llm::cost_tracker::BudgetAlert) + Send + Sync> =
                        Arc::new(move |alert| {
                            if let Err(e) = app_for_emit.emit("budget_exceeded", &alert) {
                                tracing::warn!(
                                    target: "nebula.cost_tracker",
                                    error = %e,
                                    "failed to emit budget_exceeded event"
                                );
                            } else {
                                info!(
                                    target: "nebula.cost_tracker",
                                    daily_cost = alert.daily_cost_usd,
                                    budget = alert.budget_usd,
                                    "automation daily budget exceeded; emitted budget_exceeded"
                                );
                            }
                        });
                    base.with_budget_alert(Some(budget), callback)
                }
                _ => base,
            };
            // T-E-L-06: Loop 月度预算注入(80% warning / 100% exceeded)。
            // 100% 超限时仅 emit `loop_budget_exceeded` 事件;`pause_all` 由前端
            // 监听该事件后调用 Tauri 命令(Task 8)执行,避免在 bootstrap 中持有
            // LongTaskEngine 引用(此时可能尚未创建)。
            let loop_budget_tokens = config.loop_monthly_budget_tokens;
            let loop_budget_usd = config.loop_monthly_budget_usd;
            let base = if loop_budget_tokens.is_some() || loop_budget_usd.is_some() {
                let app_for_loop_emit = app_handle.clone();
                let callback: Arc<dyn Fn(crate::llm::cost_tracker::LoopBudgetAlert) + Send + Sync> =
                    Arc::new(move |alert| match alert.level.as_str() {
                        "warning" => {
                            if let Err(e) = app_for_loop_emit.emit("loop_budget_warning", &alert) {
                                tracing::warn!(
                                    target: "nebula.cost_tracker",
                                    error = %e,
                                    "failed to emit loop_budget_warning event"
                                );
                            } else {
                                info!(
                                    target: "nebula.cost_tracker",
                                    level = %alert.level,
                                    ratio = alert.ratio,
                                    used_tokens = alert.used_tokens,
                                    used_usd = alert.used_usd,
                                    "loop monthly budget warning; emitted loop_budget_warning"
                                );
                            }
                        }
                        "exceeded" => {
                            if let Err(e) = app_for_loop_emit.emit("loop_budget_exceeded", &alert) {
                                tracing::warn!(
                                    target: "nebula.cost_tracker",
                                    error = %e,
                                    "failed to emit loop_budget_exceeded event"
                                );
                            } else {
                                info!(
                                    target: "nebula.cost_tracker",
                                    level = %alert.level,
                                    ratio = alert.ratio,
                                    used_tokens = alert.used_tokens,
                                    used_usd = alert.used_usd,
                                    "loop monthly budget exceeded; emitted loop_budget_exceeded \
                                     (pause_all triggered by frontend command)"
                                );
                            }
                        }
                        _ => {}
                    });
                base.with_loop_budget(loop_budget_tokens, loop_budget_usd, callback)
            } else {
                base
            };
            let base = base.attach_store(sqlite.as_ref().clone());
            info!(
                target: "nebula.cost_tracker",
                "cost tracker attached to sqlite store; historical records backfilled"
            );
            Arc::new(base)
        };
        let mut llm_builder = LlmGateway::new(
            ollama,
            config.chat_model.clone(),
            config.llm_provider.clone(),
            Some(config.deepseek_api_url.clone()),
            crate::security::keychain::resolve_deepseek_key(),
            config.remote_fallback_url.clone(),
            ak,
            am,
        );
        let semantic_cache: Arc<SemanticCache> = Arc::new(
            crate::llm::semantic_cache::SemanticCache::default_config(
                lance.clone(),
                embedder.clone(),
            )
            .with_sqlite(sqlite.clone()),
        );
        if config.semantic_cache_enabled {
            llm_builder = llm_builder.with_semantic_cache(semantic_cache.clone());
            info!(target: "nebula", "semantic cache wired into LlmGateway");
        }
        if config.cost_tracker_enabled {
            llm_builder = llm_builder.with_cost_tracker(cost_tracker.clone());
            info!(target: "nebula", "cost tracker wired into LlmGateway");
        }
        if config.token_juice_enabled {
            let compressor = Arc::new(crate::llm::token_juice::TokenJuiceCompressor::new(
                ollama_for_compress.clone(),
                config.chat_model.clone(),
                crate::llm::token_juice::TokenJuiceConfig::default(),
            ));
            llm_builder = llm_builder.with_token_juice(compressor);
            info!(target: "nebula", "token juice compressor wired into LlmGateway");
        }
        if config.router_enabled {
            let router = Arc::new(crate::llm::model_router::ModelRouter::new(
                ollama_for_compress.clone(),
                config.router_classifier_model.clone(),
            ));
            llm_builder = llm_builder.with_model_router(router);
            info!(target: "nebula", "model router wired into LlmGateway");
        }
        if config.daily_budget_usd > 0.0 {
            llm_builder = llm_builder.with_daily_budget(config.daily_budget_usd);
            info!(
                target: "nebula",
                budget = config.daily_budget_usd,
                "daily budget wired into LlmGateway"
            );
        }
        if let Some(base_url) = config.openai_compat_base_url.as_ref() {
            let model = config
                .openai_compat_model
                .clone()
                .unwrap_or_else(|| "local-model".to_string());
            let client = OpenAICompatClient::new(
                base_url.clone(),
                crate::security::keychain::resolve_openai_compat_key(),
                model,
            );
            llm_builder = llm_builder.with_openai_compat(client);
            info!(
                target: "nebula",
                base_url = %base_url,
                "openai-compat client wired into LlmGateway"
            );
        }
        let llm = Arc::new(llm_builder);
        startup.mark("bootstrap.llm");
        let sponge = {
            let mut sponge_builder =
                SpongeEngine::new(sqlite.clone(), lance.clone(), embedder.clone());
            sponge_builder = sponge_builder.with_cost_tracker(cost_tracker.clone());
            match Self::load_acl_from_store(&sqlite) {
                Ok(acl) => {
                    sponge_builder = sponge_builder.with_acl(Arc::new(acl));
                    info!(target: "nebula", "ACL loaded into SpongeEngine");
                }
                Err(e) => {
                    warn!(
                        target: "nebula",
                        error = %e,
                        "failed to load ACL rules from SQLite; sponge search will not enforce ACL"
                    );
                }
            }
            Arc::new(sponge_builder)
        };
        let blackhole = Arc::new(BlackholeEngine::new(
            sqlite.clone(),
            lance.clone(),
            config.blackhole_threshold_days,
        ));
        crate::llm::cost_tracker::update_models_config_override(models_config_value.clone());
        let models_config = Arc::new(parking_lot::RwLock::new(models_config_value));
        match crate::security::keychain::migrate_env_to_keychain() {
            Ok(n) if n > 0 => {
                info!(target: "nebula", count = n, "migrated env-var credentials to keychain");
            }
            Ok(_) => {}
            Err(e) => {
                warn!(target: "nebula", error = %e, "credential migration skipped");
            }
        }
        Ok((
            embedder,
            llm,
            sponge,
            blackhole,
            exec_approval,
            cost_tracker,
            inline_completion,
            models_config,
            semantic_cache,
        ))
    }

    /// T-S1-A-04: 从 SQLite `memory_acl` 表加载规则，构建 `MemoryAcl`。
    pub(crate) fn load_acl_from_store(sqlite: &Arc<SqliteStore>) -> anyhow::Result<MemoryAcl> {
        let rows = sqlite.list_acl()?;
        let mut acl = MemoryAcl::new();
        for (id, principal, resource, permission_s, effect_s) in rows {
            let permission = match permission_s.as_str() {
                "read" => AclPermission::Read,
                "write" => AclPermission::Write,
                "delete" => AclPermission::Delete,
                other => {
                    warn!(
                        target: "nebula",
                        acl_id = %id,
                        bad_value = other,
                        "skipping ACL rule with unknown permission"
                    );
                    continue;
                }
            };
            let effect = match effect_s.as_str() {
                "allow" => AclEffect::Allow,
                "deny" => AclEffect::Deny,
                other => {
                    warn!(
                        target: "nebula",
                        acl_id = %id,
                        bad_value = other,
                        "skipping ACL rule with unknown effect"
                    );
                    continue;
                }
            };
            acl.add_rule(AclRule {
                principal,
                resource,
                permission,
                effect,
            });
        }
        Ok(acl)
    }
}
