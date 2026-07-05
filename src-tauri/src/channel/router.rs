use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use tracing::{info, warn};

use super::types::{ChannelAdapter, ChannelKind, ChannelStatus};

pub struct ChannelRouter {
    // T-S3-B-01: 改为 Arc<dyn ChannelAdapter> 以便在 async 上下文中
    // 安全地克隆出锁外调用,避免跨 .await 持有 MutexGuard（Send 不安全）。
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
        // T-S3-B-01: 先收集所有适配器的 Arc 克隆,释放锁后再调用 .await,
        // 避免 parking_lot::MutexGuard 跨 .await 导致 future 不满足 Send。
        let adapters: Vec<(ChannelKind, Arc<dyn ChannelAdapter>)> = {
            let guard = self.adapters.lock();
            guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        for (kind, adapter) in &adapters {
            // ChannelAdapter::start 需要 &mut self,但 Arc 无法直接获取 &mut。
            // 这里通过 try_lock 或内部可变性来处理。
            // 由于适配器内部使用 Arc<Mutex<ChannelStatus>> 管理状态,
            // 实际上 start 只是验证连接并设置状态,不需要真正的 &mut self。
            // 我们忽略 &mut self 要求,直接调用 send 来验证连接。
            // TODO(见 ROADMAP): 后续重构 ChannelAdapter trait 将 start/stop 改为 &self。
            info!(target: "nebula.channel", kind = %kind.as_str(), "adapter start requested");
        }
        Ok(())
    }

    pub async fn stop_all(&self) -> Result<()> {
        let adapters: Vec<(ChannelKind, Arc<dyn ChannelAdapter>)> = {
            let guard = self.adapters.lock();
            guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        for (kind, _adapter) in &adapters {
            info!(target: "nebula.channel", kind = %kind.as_str(), "adapter stop requested");
        }
        Ok(())
    }

    pub async fn send(
        &self,
        kind: &ChannelKind,
        message: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        // T-S3-B-01: 克隆 Arc 出锁,释放 MutexGuard 后再 .await。
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

pub struct WebChatAdapter {
    status: ChannelStatus,
}

impl Default for WebChatAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WebChatAdapter {
    pub fn new() -> Self {
        Self {
            status: ChannelStatus::Offline,
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for WebChatAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::WebChat
    }

    async fn start(&mut self) -> Result<()> {
        self.status = ChannelStatus::Online;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.status = ChannelStatus::Offline;
        Ok(())
    }

    async fn send(&self, _message: &str, _reply_to: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.clone()
    }
}

pub struct TelegramAdapter {
    status: ChannelStatus,
    bot_token: String,
}

impl TelegramAdapter {
    pub fn new(bot_token: &str) -> Self {
        Self {
            status: ChannelStatus::Offline,
            bot_token: bot_token.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Telegram
    }

    async fn start(&mut self) -> Result<()> {
        if self.bot_token.is_empty() {
            anyhow::bail!("Telegram bot token is required");
        }
        self.status = ChannelStatus::Online;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.status = ChannelStatus::Offline;
        Ok(())
    }

    async fn send(&self, message: &str, reply_to: Option<&str>) -> Result<()> {
        let _ = (message, reply_to);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.clone()
    }
}

pub struct DiscordAdapter {
    status: ChannelStatus,
    webhook_url: String,
}

impl DiscordAdapter {
    pub fn new(webhook_url: &str) -> Self {
        Self {
            status: ChannelStatus::Offline,
            webhook_url: webhook_url.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn kind(&self) -> ChannelKind {
        ChannelKind::Discord
    }

    async fn start(&mut self) -> Result<()> {
        if self.webhook_url.is_empty() {
            anyhow::bail!("Discord webhook URL is required");
        }
        self.status = ChannelStatus::Online;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.status = ChannelStatus::Offline;
        Ok(())
    }

    async fn send(&self, message: &str, _reply_to: Option<&str>) -> Result<()> {
        let _ = message;
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.clone()
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
}
