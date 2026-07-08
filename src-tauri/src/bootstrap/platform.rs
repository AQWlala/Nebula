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
        EditorState::new(&config.editor_workspace).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                workspace = %config.editor_workspace,
                "editor workspace unavailable; falling back to current dir");
            EditorState::new(".").expect("current dir is always a directory")
        })
    }

    pub(crate) fn bootstrap_clipboard() -> ClipboardService {
        ClipboardService::new().unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                "clipboard unavailable; using noop fallback");
            ClipboardService::noop()
        })
    }

    pub(crate) fn bootstrap_sync(config: &AppConfig) -> Arc<LocalTransport> {
        Arc::new(LocalTransport::new(&config.sync_inbox).unwrap_or_else(|e| {
            tracing::warn!(target: "nebula", error = ?e,
                inbox = %config.sync_inbox,
                "sync inbox unavailable; using temp dir");
            let tmp = std::env::temp_dir().join("nebula-sync-inbox");
            LocalTransport::new(&tmp).expect("temp dir always works")
        }))
    }
}
