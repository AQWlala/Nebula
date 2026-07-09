use std::sync::Arc;

use tracing::info;

use crate::app_config::AppConfig;
use crate::app_state::AppState;
#[cfg(feature = "channels")]
use crate::channel::bridge::MessageBridge;
use crate::editor::EditorState;
use crate::os::ClipboardService;
use crate::sync::LocalTransport;

impl AppState {
    #[cfg(feature = "channels")]
    pub(crate) fn bootstrap_message_bridge() -> Option<Arc<MessageBridge>> {
        let url = std::env::var("NEBULA_BRIDGE_URL").unwrap_or_default();
        let b = MessageBridge::new(&url).map(Arc::new);
        if b.is_some() {
            info!(target: "nebula", bridge_url = %url, "message bridge initialised");
        }
        b
    }

    /// T-S3-B-01: 初始化原生渠道路由器。
    #[cfg(feature = "channels")]
    pub(crate) fn bootstrap_channel_router() -> Arc<crate::channel::ChannelRouter> {
        use crate::channel::{ChannelRouter, DiscordBotAdapter, TelegramBotAdapter};

        let router = Arc::new(ChannelRouter::new());

        let tg_token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        if !tg_token.is_empty() {
            let adapter = TelegramBotAdapter::new(&tg_token);
            router.register(Arc::new(adapter) as Arc<dyn crate::channel::ChannelAdapter>);
            info!(target: "nebula.channel", "Telegram adapter registered");
        }

        let dc_webhook = std::env::var("DISCORD_WEBHOOK_URL").unwrap_or_default();
        if !dc_webhook.is_empty() {
            let adapter = DiscordBotAdapter::new(&dc_webhook);
            router.register(Arc::new(adapter) as Arc<dyn crate::channel::ChannelAdapter>);
            info!(target: "nebula.channel", "Discord adapter registered");
        }

        router
    }

    pub(crate) fn bootstrap_editor(config: &AppConfig) -> EditorState {
        EditorState::new(&config.editor_workspace)
            .or_else(|e| {
                tracing::warn!(target: "nebula", error = ?e,
                    workspace = %config.editor_workspace,
                    "editor workspace unavailable; falling back to current dir");
                EditorState::new(".")
            })
            .or_else(|e| {
                tracing::warn!(target: "nebula", error = ?e,
                    "current dir unavailable; falling back to temp dir");
                EditorState::new(std::env::temp_dir())
            })
            // T-D-B-07: 字面量保证有效,保留 expect — temp_dir 一定存在且可规范化
            .expect("temp_dir is always a valid directory")
    }

    pub(crate) fn bootstrap_clipboard() -> ClipboardService {
        ClipboardService::new().unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                "clipboard unavailable; using noop fallback");
            ClipboardService::noop()
        })
    }

    pub(crate) fn bootstrap_sync(config: &AppConfig) -> Arc<LocalTransport> {
        Arc::new(
            LocalTransport::new(&config.sync_inbox)
                .or_else(|e| {
                    tracing::warn!(target: "nebula", error = ?e,
                        inbox = %config.sync_inbox,
                        "sync inbox unavailable; using temp dir");
                    let tmp = std::env::temp_dir().join("nebula-sync-inbox");
                    LocalTransport::new(&tmp)
                })
                .or_else(|e| {
                    tracing::warn!(target: "nebula", error = ?e,
                        "nebula-sync-inbox creation failed; using temp dir directly");
                    LocalTransport::new(std::env::temp_dir())
                })
                // T-D-B-07: 字面量保证有效,保留 expect — temp_dir 一定可写
                .expect("temp_dir is always writable"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 串行化环境变量测试,避免并行污染。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 构造测试用 AppConfig,用 temp_dir 作为路径避免污染当前目录。
    fn test_config() -> AppConfig {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // 清理可能影响 from_env 的环境变量,确保默认值
        for k in ["NEBULA_WORKSPACE", "NEBULA_SYNC_INBOX"] {
            std::env::remove_var(k);
        }
        let mut config = AppConfig::from_env();
        let tmp = std::env::temp_dir();
        config.editor_workspace = tmp.to_string_lossy().to_string();
        config.sync_inbox = tmp
            .join(format!("nebula-test-sync-{}", std::process::id()))
            .to_string_lossy()
            .to_string();
        config
    }

    #[test]
    fn test_bootstrap_editor_valid_path() {
        let config = test_config();
        let editor = AppState::bootstrap_editor(&config);
        // 不 panic 即成功
        let _ = editor;
    }

    #[test]
    fn test_bootstrap_editor_path_is_file_fallback() {
        let config = test_config();
        let mut cfg = config;
        // 创建一个临时文件用作 workspace(不是目录),触发 fallback
        let file_path =
            std::env::temp_dir().join(format!("nebula-test-file-{}.txt", std::process::id()));
        std::fs::write(&file_path, "test").expect("write test file");
        cfg.editor_workspace = file_path.to_string_lossy().to_string();
        // 应该 fallback 到 "." 而不是 panic
        let editor = AppState::bootstrap_editor(&cfg);
        let _ = editor;
        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn test_bootstrap_clipboard_returns_service() {
        let _clipboard = AppState::bootstrap_clipboard();
        // 不 panic 即成功(可能真实 clipboard 或 noop fallback)
    }

    #[test]
    fn test_bootstrap_sync_valid_path() {
        let config = test_config();
        let transport = AppState::bootstrap_sync(&config);
        let _ = transport;
    }

    #[test]
    fn test_bootstrap_sync_unwritable_path_fallback() {
        let mut config = test_config();
        // 用一个文件路径作为 sync_inbox(不是目录),触发 fallback
        let file_path =
            std::env::temp_dir().join(format!("nebula-test-sync-file-{}.txt", std::process::id()));
        std::fs::write(&file_path, "test").expect("write test file");
        config.sync_inbox = file_path.to_string_lossy().to_string();
        // 应该 fallback 到 temp_dir/nebula-sync-inbox 而不是 panic
        let transport = AppState::bootstrap_sync(&config);
        let _ = transport;
        std::fs::remove_file(&file_path).ok();
    }

    #[cfg(feature = "channels")]
    #[test]
    fn test_bootstrap_message_bridge_no_url_returns_none() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        std::env::remove_var("NEBULA_BRIDGE_URL");
        let bridge = AppState::bootstrap_message_bridge();
        assert!(bridge.is_none(), "bridge should be None when no URL is set");
    }

    #[cfg(feature = "channels")]
    #[test]
    fn test_bootstrap_channel_router_no_tokens_returns_empty() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("DISCORD_WEBHOOK_URL");
        let router = AppState::bootstrap_channel_router();
        assert!(
            router.list_channels().is_empty(),
            "router should have no adapters when no tokens are set"
        );
    }
}
