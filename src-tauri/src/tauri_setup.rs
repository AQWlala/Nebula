//! Tauri application entry — builds the runtime, wires commands, runs the app.

use tauri::Manager;
use tracing::{error, info, warn};

use crate::app_config::AppConfig;
use crate::app_state::AppState;

/// Tauri application entry — builds the runtime, wires commands, runs the app.
pub fn run() {
    crate::tracing_setup::init_tracing();
    info!(target: "nebula", version = env!("CARGO_PKG_VERSION"), "starting nebula");

    let config = AppConfig::from_env();
    info!(target: "nebula", db_path = ?config.db_path, "loaded configuration");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .on_window_event(move |window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let app = window.app_handle();
                    if window.label() != "floating-chat"
                        && window.label() != "floating-ball"
                        && window.label() != "floating-progress"
                        && app.tray_by_id("nebula-tray").is_some()
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                tauri::WindowEvent::DragDrop(drag_drop) => {
                    #[allow(clippy::collapsible_match)]
                    if let tauri::DragDropEvent::Drop { paths, .. } = drag_drop {
                        let app = window.app_handle();
                        if window.label() == crate::commands::window::FLOATING_BALL_LABEL {
                            crate::os::file_handler::emit_ball_drag_drop(app, paths);
                        } else {
                            crate::os::file_handler::emit_drag_drop(app, paths);
                        }
                    }
                }
                _ => {}
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let mut config = config.clone();
            if let Ok(data_dir) = app.path().app_data_dir() {
                std::fs::create_dir_all(&data_dir).ok();
                if !std::path::Path::new(&config.db_path).is_absolute() {
                    config.db_path = data_dir.join(&config.db_path)
                        .to_string_lossy().to_string();
                }
                if !std::path::Path::new(&config.lance_path).is_absolute() {
                    config.lance_path = data_dir.join(&config.lance_path)
                        .to_string_lossy().to_string();
                }
                info!(target: "nebula", data_dir = ?data_dir, db_path = ?config.db_path, "resolved data paths");
            }

            crate::os::tray::setup(app.handle());
            crate::os::shortcut::setup(app.handle());

            let power_mgr = std::sync::Arc::new(crate::os::PowerManager::new(app.handle().clone()));
            power_mgr.clone().start();
            app.manage(power_mgr);

            crate::os::file_handler::handle_argv_files(app.handle());
            crate::os::file_handler::handle_ask_argv(app.handle());

            tauri::async_runtime::spawn(async move {
                match AppState::bootstrap(config.clone(), handle.clone()).await {
                    Ok(state) => {
                        if let Some(h) = state.memory.reflection.clone().spawn_worker() {
                            *state.memory.reflect_worker.lock() = Some(h);
                            info!(target: "nebula", "reflection worker started");
                        }

                        #[cfg(feature = "grpc")]
                        if state.infra.config.grpc_enabled {
                            match crate::grpc::start_server(
                                state.infra.config.grpc_bind_addr.clone(),
                                state.clone(),
                            )
                            .await
                            {
                                Ok(handle) => {
                                    info!(
                                        target: "nebula",
                                        addr = %state.infra.config.grpc_bind_addr,
                                        "gRPC server started"
                                    );
                                    *state.platform.grpc_server.lock() = Some(handle);
                                }
                                Err(e) => {
                                    error!(
                                        target: "nebula",
                                        error = ?e,
                                        "gRPC server failed to start; continuing without it"
                                    );
                                }
                            }
                        } else {
                            info!(target: "nebula", "gRPC server disabled by config");
                        }

                        if let Some(addr) = crate::metrics::exporter::bind_addr_from_env() {
                            let h = crate::metrics::exporter::start(
                                addr.clone(),
                                state.infra.perf_monitor.clone(),
                            );
                            // T-D-B-03: 不再使用 std::mem::forget 泄露 JoinHandle。
                            // tokio::task::JoinHandle 在 drop 时 detach(任务继续
                            // 运行至进程结束),效果与 forget 等价,但 JoinHandle
                            // 本身的内存被正确释放,不构成泄露。
                            // 若需 shutdown 时 abort,后续可存入
                            // InfraSubsystem::metrics_handle(TODO)。
                            drop(h);
                            info!(
                                target: "nebula.metrics",
                                addr = %addr,
                                "prometheus exporter started"
                            );
                        }

                        #[cfg(feature = "mcp")]
                        {
                            state.platform.mcp_manager.connect_all().await;
                            let mcp_tools = state.platform.mcp_manager.list_all_tools().await;
                            if !mcp_tools.is_empty() {
                                info!(target: "nebula", count = mcp_tools.len(), "MCP tools discovered");
                            }
                            let tool_groups = state.platform.mcp_manager.as_tool_implementations().await;
                            for (server_name, tools) in tool_groups {
                                let count = tools.len();
                                state.infra.tool_registry.register_mcp_tools(&server_name, tools);
                                info!(
                                    target: "nebula",
                                    server = %server_name,
                                    count,
                                    "MCP tools registered into ToolRegistry"
                                );
                            }
                        }

                        {
                            let app_handle = handle.clone();
                            let db_path = std::path::PathBuf::from(&state.infra.config.db_path);
                            let lance_dir = std::path::PathBuf::from(&state.infra.config.lance_path);
                            let scheduler = crate::backup::BackupScheduler::new(app_handle, db_path, lance_dir);
                            if let Err(e) = scheduler.start() {
                                info!(target: "nebula.backup", error = ?e, "failed to start backup scheduler");
                            }
                        }

                        let clipboard_auto_started = state.infra.config.clipboard_watch_enabled;
                        if clipboard_auto_started {
                            let app_handle = handle.clone();
                            let sponge = state.memory.sponge.clone();
                            let mut watcher = state.platform.clipboard_watcher.lock().await;
                            match watcher.start(sponge, app_handle) {
                                Ok(_) => info!(target: "nebula", "clipboard watcher auto-started (T-E-C-14)"),
                                Err(e) => warn!(target: "nebula", error = %e, "clipboard watcher auto-start failed"),
                            }
                        }

                        let notification_svc = state.platform.notification_service.clone();
                        let swarm_bus = state.swarm.swarm.bus().clone();

                        handle.manage(state);
                        info!(target: "nebula", "app state ready");

                        {
                            let sidecar_manager = handle.state::<crate::AppState>().platform.sidecar_manager.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = sidecar_manager.bootstrap().await {
                                    warn!(target: "nebula.sidecar", error = ?e,
                                        "sidecar bootstrap failed (non-blocking, T-E-S-61)");
                                } else {
                                    info!(target: "nebula.sidecar",
                                        "sidecar bootstrap completed (T-E-S-61)");
                                }
                            });
                        }

                        {
                            let state = handle.state::<crate::AppState>();
                            let l0_cache = state.memory.l0.clone();
                            let sqlite_for_l0 = state.memory.sqlite.clone();
                            tauri::async_runtime::spawn(async move {
                                l0_cache.prewarm_from_store(&sqlite_for_l0, 64).await;
                                info!(target: "nebula.l0_cache",
                                    "L0Cache prewarmed with recent memories (T-E-D-01)");
                            });
                        }

                        {
                            let state = handle.state::<crate::AppState>();
                            let ollama = state.llm.llm.ollama_client().clone();
                            let chat_model = state.infra.config.chat_model.clone();
                            let cfg_provider = state.infra.config.llm_provider.clone();
                            tauri::async_runtime::spawn(async move {
                                if cfg_provider == "ollama" && !chat_model.is_empty() {
                                    if let Err(e) = ollama.warmup_model(&chat_model).await {
                                        warn!(
                                            target: "nebula.ollama",
                                            error = %e,
                                            "warmup_model failed (non-blocking, T-E-D-02)"
                                        );
                                    } else {
                                        info!(
                                            target: "nebula.ollama",
                                            model = %chat_model,
                                            "warmup_model completed (T-E-D-02)"
                                        );
                                    }
                                }
                            });
                        }

                        {
                            let state = handle.state::<crate::AppState>();
                            let ollama = state.llm.llm.ollama_client().clone();
                            let vision_model = state.infra.config.vision_model.clone();
                            tauri::async_runtime::spawn(async move {
                                if !vision_model.is_empty() {
                                    if let Err(e) = ollama.warmup_model(&vision_model).await {
                                        warn!(
                                            target: "nebula.ollama",
                                            error = %e,
                                            "vision_model warmup failed (non-blocking, T-E-C-02)"
                                        );
                                    } else {
                                        info!(
                                            target: "nebula.ollama",
                                            model = %vision_model,
                                            "vision_model warmup completed (T-E-C-02)"
                                        );
                                    }
                                }
                            });
                        }

                        tauri::async_runtime::spawn(async move {
                            let mut rx = swarm_bus.subscribe_events();
                            info!(target: "nebula.notify", "notification event subscriber started");
                            loop {
                                match rx.recv().await {
                                    Ok(event) => {
                                        notification_svc.handle_swarm_event(&event);
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                        warn!(
                                            target: "nebula.notify",
                                            lagged = n,
                                            "notification subscriber lagged, skipping stale events"
                                        );
                                        continue;
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                        info!(target: "nebula.notify", "notification subscriber channel closed, exiting");
                                        break;
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!(target: "nebula", error = ?e, "failed to bootstrap app state");
                        use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                        let _ = handle
                            .dialog()
                            .message(format!("Nebula启动失败：{e:#}\n\n将退出应用。"))
                            .title("nebula bootstrap error")
                            .kind(MessageDialogKind::Error)
                            .blocking_show();
                        std::process::exit(1);
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::bootstrap,
            crate::commands::health,
            crate::commands::health_full,
            crate::commands::doctor_run,
            crate::commands::chat,
            crate::commands::memory_store,
            crate::commands::memory_search,
            crate::commands::memory_get,
            crate::commands::memory_list_recent,
            crate::commands::memory_update_importance,
            crate::commands::memory_delete,
            crate::commands::memory_get_many,
            crate::commands::memory_query_dsl,
            crate::commands::memory_stats,
            crate::commands::memory_orchestrator_run,
            crate::commands::causal_trace_root_causes,
            crate::commands::causal_find_effects,
            crate::commands::causal_explain,
            crate::commands::summary_generate,
            crate::commands::memory_branch_list,
            crate::commands::memory_branch_create,
            crate::commands::memory_branch_checkout,
            crate::commands::memory_branch_delete,
            crate::commands::memory_commit,
            crate::commands::memory_log,
            crate::commands::memory_diff,
            crate::commands::memory_revert,
            crate::commands::memory_merge,
            crate::commands::swarm_execute,
            crate::commands::swarm_list_agents,
            crate::commands::swarm_get_agent,
            crate::commands::subscribe_events,
            crate::commands::deadlock_status,
            crate::commands::llm_complete,
            crate::commands::llm_chat,
            crate::commands::llm_embed,
            crate::commands::describe_screenshot,
            crate::commands::reflect_now,
            crate::commands::list_reflections,
            crate::commands::get_reflection,
            crate::commands::self_reflect_now,
            crate::commands::metrics,
            crate::commands::metrics_ttft,
            crate::commands::migration_status,
            crate::commands::startup_report,
            crate::commands::perf_sample,
            crate::commands::load_app_settings,
            crate::commands::save_app_settings,
            crate::commands::skill_create,
            crate::commands::skill_use,
            crate::commands::skill_rate,
            crate::commands::skill_list,
            crate::commands::skill_search,
            crate::commands::skill_import,
            crate::commands::skill_tags,
            crate::commands::skill_export_clawhub,
            crate::commands::writing_list_templates,
            crate::commands::writing_get_template,
            crate::commands::writing_create_document,
            crate::commands::writing_update_document,
            crate::commands::writing_get_document,
            crate::commands::writing_list_documents,
            crate::commands::writing_delete_document,
            crate::commands::writing_export,
            crate::commands::work_create_task,
            crate::commands::work_get_task,
            crate::commands::work_list_tasks,
            crate::commands::work_set_status,
            crate::commands::work_update_task,
            crate::commands::work_delete_task,
            crate::commands::work_recommend_priority,
            crate::commands::work_summarise_meeting,
            crate::commands::work_start_timer,
            crate::commands::work_stop_timer,
            crate::commands::work_add_time,
            crate::commands::work_active_timer,
            crate::commands::editor_read,
            crate::commands::editor_write,
            crate::commands::editor_list,
            crate::commands::editor_workspace_root,
            crate::commands::git_status,
            crate::commands::git_log,
            crate::commands::git_diff,
            crate::commands::git_commit,
            crate::commands::os_clipboard_read,
            crate::commands::os_clipboard_write,
            crate::commands::os_shell_exec,
            crate::commands::os_notify,
            crate::commands::screenshot,
            crate::commands::os_autostart_enable,
            crate::commands::os_autostart_disable,
            crate::commands::os_autostart_is_enabled,
            crate::commands::sync_encrypt,
            crate::commands::sync_decrypt,
            crate::commands::sync_send,
            crate::commands::sync_recv,
            crate::commands::sync_ack,
            crate::commands::sync_make_identity,
            crate::commands::generate_did,
            crate::commands::resolve_did,
            crate::commands::skill_audit_list,
            crate::commands::skill_audit_list_for_skill,
            crate::commands::chat_stream,
            crate::commands::export_memories,
            crate::commands::import_memories,
            crate::commands::export_chat_docx,
            crate::commands::list_devices,
            crate::commands::revoke_device,
            #[cfg(feature = "channels")]
            crate::commands::share_chat,
            crate::commands::acl_set,
            crate::commands::acl_list,
            crate::commands::acl_remove,
            crate::commands::set_api_key,
            crate::commands::get_api_key,
            crate::commands::delete_api_key,
            crate::commands::set_provider_api_key,
            crate::commands::get_provider_api_key,
            #[cfg(feature = "channels")]
            crate::commands::channel_status,
            #[cfg(feature = "channels")]
            crate::commands::channel_send,
            #[cfg(feature = "channels")]
            crate::commands::channel_poll,
            #[cfg(feature = "channels")]
            crate::commands::channel_ping,
            #[cfg(feature = "channels")]
            crate::commands::channel_list_adapters,
            #[cfg(feature = "channels")]
            crate::commands::channel_send_native,
            #[cfg(feature = "channels")]
            crate::commands::channel_start_all,
            crate::commands::injection_scan,
            crate::commands::sandbox_config,
            crate::commands::tool_list,
            crate::commands::tool_invoke,
            crate::commands::marketplace_search,
            crate::commands::marketplace_quick_search,
            crate::commands::marketplace_install,
            crate::commands::marketplace_check_updates,
            crate::commands::marketplace_refresh,
            crate::commands::marketplace_stats,
            crate::commands::marketplace_tags,
            crate::commands::marketplace_generate_manifest,
            crate::commands::skill_publish,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_list_servers,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_add_server,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_remove_server,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_list_tools,
            crate::commands::plan_pre_check,
            crate::commands::plan_approve_confirmation,
            crate::commands::plan_deny_confirmation,
            crate::commands::plan_approve_plan,
            crate::commands::plan_reject_plan,
            crate::commands::plan_get_plan,
            crate::commands::plan_get_confirmation,
            crate::commands::values_redact,
            crate::commands::sidecar_list_status,
            crate::commands::sidecar_start,
            crate::commands::sidecar_stop,
            crate::commands::sidecar_restart,
            crate::commands::open_floating_chat,
            crate::commands::open_floating_ball,
            crate::commands::open_floating_progress,
            crate::commands::swarm_cancel,
            // T-D-C-08: master-orchestrator 运行时开关命令
            #[cfg(feature = "master-orchestrator")]
            crate::commands::master_orchestrator_enabled,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::master_orchestrator_set_enabled,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::master_run,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_run,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_state,
            // T-E-L-05: Loop 模板库命令(编译时内嵌,无运行时文件 I/O)。
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_templates_list,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_template_get,
            // T-E-L-06: Loop 月度预算命令(status / reset / pause_all)。
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_budget_status,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_budget_reset,
            #[cfg(feature = "master-orchestrator")]
            crate::commands::loop_budget_pause_all,
            crate::commands::master_confirm,
            crate::commands::master_confirmation_status,
            crate::commands::master_pending_confirmations,
            #[cfg(feature = "evolution-engine")]
            crate::commands::evolution_log_list,
            #[cfg(feature = "evolution-engine")]
            crate::commands::evolution_log_get,
            #[cfg(feature = "evolution-engine")]
            crate::commands::evolution_rollback,
            #[cfg(feature = "self-evolution")]
            crate::commands::evolution_enabled,
            #[cfg(feature = "self-evolution")]
            crate::commands::evolution_set_enabled,
            #[cfg(feature = "evolution-engine")]
            crate::commands::evolution_run,
            #[cfg(feature = "evolution-engine")]
            crate::commands::evolution_engine_ready,
            #[cfg(feature = "soul-system")]
            crate::commands::soul_system_enabled,
            #[cfg(feature = "soul-system")]
            crate::commands::soul_system_set_enabled,
            crate::commands::moa_execute,
            crate::os::power::power_state,
            crate::os::power::power_pause,
            crate::os::power::power_resume,
            crate::backup::commands::backup_now,
            crate::backup::commands::backup_list,
            crate::backup::commands::backup_restore,
            crate::sync::crdt_op_log::crdt_op_stats,
            crate::sync::crdt_op_log::crdt_op_pending,
            crate::os::controller::os_get_foreground_window,
            crate::os::controller::os_list_windows,
            crate::commands::memory::embed_image,
            crate::commands::memory::sponge_absorb_file,
            crate::os::context_menu::context_menu_install,
            crate::os::context_menu::context_menu_uninstall,
            crate::os::context_menu::context_menu_status,
            crate::commands::cost_summary,
            crate::commands::exec_approval_list,
            crate::commands::autonomy_get_level,
            crate::commands::autonomy_set_level,
            crate::commands::autonomy_list_levels,
            crate::commands::autonomy_route,
            crate::commands::inline_complete,
            #[cfg(feature = "channels")]
            crate::commands::inbox_list,
            #[cfg(feature = "channels")]
            crate::commands::inbox_send,
            #[cfg(feature = "channels")]
            crate::commands::inbox_reply,
            #[cfg(feature = "channels")]
            crate::commands::inbox_mark_read,
            #[cfg(feature = "channels")]
            crate::commands::inbox_unread_count,
            crate::commands::credits_overview,
            crate::commands::cost_report,
            crate::commands::directed_edit,
            crate::commands::watch_start,
            crate::commands::watch_stop,
            crate::commands::watch_status,
            crate::commands::watch_list_paths,
            crate::commands::clipboard_watch_start,
            crate::commands::clipboard_watch_stop,
            crate::commands::clipboard_watch_status,
            crate::commands::subscribe_diagnostics,
            crate::commands::diagnostics_snapshot,
            crate::commands::diagnostics_open_logs,
            crate::commands::models_config_load,
            crate::commands::models_config_save,
            crate::commands::models_config_set_default,
            crate::commands::models_config_reload,
            crate::commands::models_config_add_provider,
            crate::commands::models_config_remove_provider,
            crate::commands::set_provider_key,
            crate::commands::get_provider_key,
            crate::commands::models_config_test_provider,
            crate::commands::persona_reload,
            crate::commands::persona_get,
            crate::commands::persona_set_file,
            crate::commands::annotation_upsert,
            crate::commands::annotation_list,
            crate::commands::annotation_stats,
            crate::commands::annotation_export,
            crate::commands::snapshot_create,
            crate::commands::snapshot_rollback,
            crate::commands::snapshot_discard,
            crate::commands::snapshot_list,
            crate::commands::trigger_create,
            crate::commands::trigger_list,
            crate::commands::trigger_delete,
            crate::commands::trigger_enable,
            crate::commands::trigger_fire_log,
            crate::commands::watch_test,
            crate::commands::scenario_list,
            crate::commands::scenario_get,
            crate::commands::scenario_instantiate,
            crate::commands::im_create_webhook_binding,
            crate::commands::im_list_bindings,
            crate::commands::im_delete_binding,
            crate::commands::im_set_enabled,
            crate::commands::im_test_send,
            crate::commands::im_broadcast,
            crate::commands::prefetch_for_file,
            crate::commands::wiki_compile,
            crate::commands::wiki_list,
            crate::commands::wiki_read,
            crate::commands::wiki_search,
            crate::commands::wiki_delete,
            crate::commands::wiki_regen_index,
            crate::commands::wiki_backlinks,
            crate::commands::wiki_get_card,
            crate::commands::wiki_update_from_user,
            // T-E-B-08: Obsidian vault 兼容命令
            crate::commands::obsidian_detect_vault,
            crate::commands::obsidian_read_config,
            crate::commands::obsidian_scan_vault,
            crate::commands::obsidian_import_note,
            crate::commands::obsidian_export_note,
            // T-E-B-16: MDRM 5 维关系图谱命令
            crate::commands::mdrm::mdrm_trace_temporal,
            crate::commands::mdrm::mdrm_find_entities,
            crate::commands::mdrm::mdrm_trace_hierarchy,
            crate::commands::mdrm::mdrm_find_similar,
            crate::commands::mdrm::mdrm_get_graph,
            crate::commands::otel_status,
            crate::commands::db_encryption_status,
            crate::commands::db_encryption_enable,
            crate::commands::db_encryption_disable,
            crate::commands::security::oauth_list_providers,
            crate::commands::security::oauth_authorization_url,
            crate::commands::security::oauth_authorize,
            crate::commands::security::oauth_disconnect,
            crate::commands::security::oauth_status,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_server_list,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_server_start,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_server_stop,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_server_status,
            #[cfg(feature = "mcp")]
            crate::commands::mcp_server_logs,
            crate::commands::arena_create_match,
            crate::commands::arena_vote,
            crate::commands::arena_leaderboard,
            // T-E-C-08: Shadow Workspace 隔离执行环境命令
            crate::shadow_workspace::commands::shadow_create,
            crate::shadow_workspace::commands::shadow_list,
            crate::shadow_workspace::commands::shadow_status,
            crate::shadow_workspace::commands::shadow_diff,
            crate::shadow_workspace::commands::shadow_run_command,
            crate::shadow_workspace::commands::shadow_complete,
            crate::shadow_workspace::commands::shadow_fail,
            crate::shadow_workspace::commands::shadow_merge,
            crate::shadow_workspace::commands::shadow_abort,
            crate::shadow_workspace::commands::shadow_cleanup,
            // T-E-C-09: 任务录屏回放
            crate::shadow_workspace::commands::shadow_record,
            crate::shadow_workspace::commands::shadow_recording_list,
            crate::shadow_workspace::commands::shadow_recording_clear,
            // T-E-C-10: 异步长任务
            crate::long_task::commands::long_task_create,
            crate::long_task::commands::long_task_get,
            crate::long_task::commands::long_task_list,
            crate::long_task::commands::long_task_steps,
            crate::long_task::commands::long_task_start,
            crate::long_task::commands::long_task_pause,
            crate::long_task::commands::long_task_resume,
            crate::long_task::commands::long_task_cancel,
            crate::long_task::commands::long_task_delete,
            #[cfg(feature = "openapi")]
            crate::commands::openapi_register_tools,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Convenience wrapper for non-Tauri contexts (tests, CLI) that want the
/// same [`AppState`] wiring without spawning a window.
pub async fn build_state_for_tests(
    config: AppConfig,
    app_handle: tauri::AppHandle,
) -> anyhow::Result<AppState> {
    AppState::bootstrap(config, app_handle).await
}
