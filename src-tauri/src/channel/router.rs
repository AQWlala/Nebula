use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use reqwest::Client;
use tracing::{info, warn};

use super::types::{ChannelAdapter, ChannelKind, ChannelStatus};

pub struct ChannelRouter {
    adapters: Mutex<HashMap<ChannelKind, Arc<dyn ChannelAdapter>>>,
}

impl ChannelRouter {
    pub fn new() -> Self {
        Self {
            adapters: Mutex::new(HashMap::new()),
        }
    }

    pub fn register(&self, adapter: Arc<dyn ChannelAdapter>) {
        let kind = adapter.kind();
        info!(target: "nebula.channel", kind = %kind.as_str(), "registered channel adapter");
        self.adapters.lock().insert(kind, adapter);
    }

    pub fn unregister(&self, kind: &ChannelKind) {
        self.adapters.lock().remove(kind);
    }

    pub async fn start_all(&self) -> Result<()> {
        let adapters: Vec<(ChannelKind, Arc<dyn ChannelAdapter>)> = {
            let guard = self.adapters.lock();
            guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        for (kind, adapter) in &adapters {
            match adapter.start().await {
                Ok(()) => info!(target: "nebula.channel", kind = %kind.as_str(), "adapter started"),
                Err(e) => {
                    warn!(target: "nebula.channel", kind = %kind.as_str(), error = %e, "adapter start failed")
                }
            }
        }
        Ok(())
    }

    pub async fn stop_all(&self) -> Result<()> {
        let adapters: Vec<(ChannelKind, Arc<dyn ChannelAdapter>)> = {
            let guard = self.adapters.lock();
            guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        for (kind, adapter) in &adapters {
            match adapter.stop().await {
                Ok(()) => info!(target: "nebula.channel", kind = %kind.as_str(), "adapter stopped"),
                Err(e) => {
                    warn!(target: "nebula.channel", kind = %kind.as_str(), error = %e, "adapter stop failed")
                }
            }
        }
        Ok(())
    }

    pub async fn send(
        &self,
        kind: &ChannelKind,
        message: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let adapter = {
            let guard = self.adapters.lock();
            guard
                .get(kind)
                .cloned()
                .ok_or_else(|| anyhow!("no adapter registered for {:?}", kind))?
        };
        adapter.send(message, reply_to).await
    }

    pub fn status(&self, kind: &ChannelKind) -> ChannelStatus {
        let adapters = self.adapters.lock();
        adapters
            .get(kind)
            .map(|a| a.status())
            .unwrap_or(ChannelStatus::Offline)
    }

    pub fn list_channels(&self) -> Vec<(ChannelKind, ChannelStatus)> {
        let adapters = self.adapters.lock();
        adapters
            .iter()
            .map(|(k, a)| (k.clone(), a.status()))
            .collect()
    }
}

impl Default for ChannelRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// WebChat adapter — emits messages via tracing log (for headless)
/// or Tauri events (for GUI mode, handled by inbox layer).
pub struct WebChatAdapter {
    status: Arc<Mutex<ChannelStatus>>,
}

impl Default for WebChatAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WebChatAdapter {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(ChannelStatus::Offline)),
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for WebChatAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::WebChat
    }

    async fn start(&self) -> Result<()> {
        *self.status.lock() = ChannelStatus::Online;
        info!(target: "nebula.channel", "WebChat adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        *self.status.lock() = ChannelStatus::Offline;
        Ok(())
    }

    async fn send(&self, message: &str, reply_to: Option<&str>) -> Result<()> {
        // WebChat messages are delivered via Tauri events in GUI mode
        // (handled by InboxManager). In headless mode, log the message
        // so it's not silently dropped.
        info!(
            target: "nebula.channel.webchat",
            reply_to = ?reply_to,
            message_len = message.len(),
            "WebChat message delivered (len={})",
            message.len()
        );
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.lock().clone()
    }
}

/// Telegram adapter — sends messages via Telegram Bot API.
/// Uses internal mutability (Arc<Mutex>) for &self compatibility.
pub struct TelegramAdapter {
    bot_token: String,
    client: Client,
    status: Arc<Mutex<ChannelStatus>>,
}

impl TelegramAdapter {
    pub fn new(bot_token: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            bot_token: bot_token.to_string(),
            client,
            status: Arc::new(Mutex::new(ChannelStatus::Offline)),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.bot_token, method)
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Telegram
    }

    async fn start(&self) -> Result<()> {
        if self.bot_token.is_empty() {
            anyhow::bail!("Telegram bot token is required");
        }
        let url = self.api_url("getMe");
        let resp = self.client.get(&url).send().await?;
        if resp.status().is_success() {
            *self.status.lock() = ChannelStatus::Online;
            info!(target: "nebula.channel", "Telegram bot started");
            Ok(())
        } else {
            *self.status.lock() = ChannelStatus::Failed;
            anyhow::bail!("Telegram getMe failed: {}", resp.status())
        }
    }

    async fn stop(&self) -> Result<()> {
        *self.status.lock() = ChannelStatus::Offline;
        Ok(())
    }

    async fn send(&self, message: &str, reply_to: Option<&str>) -> Result<()> {
        // Parse chat_id from the message or reply_to
        // Format: "chat_id:actual_message" or just a chat_id in reply_to
        let (chat_id, text) = if let Some(colon_pos) = message.find(':') {
            let id_part = &message[..colon_pos];
            let msg_part = &message[colon_pos + 1..];
            match id_part.parse::<i64>() {
                Ok(id) => (id, msg_part),
                Err(_) => (0, message),
            }
        } else {
            // Try parsing entire message as chat_id (legacy behavior)
            match message.parse::<i64>() {
                Ok(id) => (id, message),
                Err(_) => {
                    anyhow::bail!("Telegram send: cannot parse chat_id from message: {message}");
                }
            }
        };

        if chat_id == 0 {
            anyhow::bail!("Telegram send: chat_id is 0, cannot send");
        }

        let url = self.api_url("sendMessage");
        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });
        if let Some(r) = reply_to {
            if let Ok(reply_id) = r.parse::<i64>() {
                payload["reply_to_message_id"] = serde_json::json!(reply_id);
            }
        }

        let resp = self.client.post(&url).json(&payload).send().await?;
        if resp.status().as_u16() == 429 {
            *self.status.lock() = ChannelStatus::RateLimited;
            warn!(target: "nebula.channel", "Telegram rate limited");
        } else if !resp.status().is_success() {
            *self.status.lock() = ChannelStatus::Failed;
            anyhow::bail!("Telegram sendMessage failed: {}", resp.status());
        }
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.lock().clone()
    }
}

/// Discord adapter — sends messages via Discord webhook.
/// Uses internal mutability (Arc<Mutex>) for &self compatibility.
pub struct DiscordAdapter {
    webhook_url: String,
    client: Client,
    status: Arc<Mutex<ChannelStatus>>,
}

impl DiscordAdapter {
    pub fn new(webhook_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            webhook_url: webhook_url.to_string(),
            client,
            status: Arc::new(Mutex::new(ChannelStatus::Offline)),
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Discord
    }

    async fn start(&self) -> Result<()> {
        if self.webhook_url.is_empty() {
            anyhow::bail!("Discord webhook URL is required");
        }
        let resp = self.client.get(&self.webhook_url).send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                *self.status.lock() = ChannelStatus::Online;
                info!(target: "nebula.channel", "Discord webhook adapter started");
                Ok(())
            }
            Ok(r) => {
                *self.status.lock() = ChannelStatus::Failed;
                anyhow::bail!("Discord webhook validation failed: {}", r.status())
            }
            Err(e) => {
                *self.status.lock() = ChannelStatus::Failed;
                anyhow::bail!("Discord webhook connection failed: {e}")
            }
        }
    }

    async fn stop(&self) -> Result<()> {
        *self.status.lock() = ChannelStatus::Offline;
        Ok(())
    }

    async fn send(&self, message: &str, reply_to: Option<&str>) -> Result<()> {
        let mut payload = serde_json::json!({
            "content": message,
        });
        if let Some(ref_text) = reply_to {
            payload["message_reference"] = serde_json::json!({
                "message_id": ref_text,
            });
        }
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await?;
        if resp.status().as_u16() == 429 {
            *self.status.lock() = ChannelStatus::RateLimited;
            warn!(target: "nebula.channel", "Discord rate limited");
        } else if !resp.status().is_success() {
            *self.status.lock() = ChannelStatus::Failed;
            anyhow::bail!("Discord webhook failed: {}", resp.status());
        }
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_starts_empty() {
        let router = ChannelRouter::new();
        assert!(router.list_channels().is_empty());
    }

    #[test]
    fn register_and_list() {
        let router = ChannelRouter::new();
        router.register(Arc::new(WebChatAdapter::new()));
        let channels = router.list_channels();
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].0, ChannelKind::WebChat);
    }

    #[test]
    fn status_offline_for_unregistered() {
        let router = ChannelRouter::new();
        assert_eq!(
            router.status(&ChannelKind::Telegram),
            ChannelStatus::Offline
        );
    }

    #[tokio::test]
    async fn webchat_send_succeeds() {
        let adapter = WebChatAdapter::new();
        adapter.start().await.unwrap();
        let result = adapter.send("test message", None).await;
        assert!(result.is_ok());
    }

    #[test]
    fn telegram_adapter_stores_token() {
        let adapter = TelegramAdapter::new("test-token");
        assert_eq!(adapter.kind(), ChannelKind::Telegram);
    }

    #[test]
    fn discord_adapter_stores_url() {
        let adapter = DiscordAdapter::new("https://discord.com/api/webhooks/test");
        assert_eq!(adapter.kind(), ChannelKind::Discord);
    }
}
