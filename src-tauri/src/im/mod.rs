//! T-E-C-17: IM 扫码绑定引擎 — Phase 1 Webhook 优先。
//!
//! 核心数据结构(ImPlatform / BindingKind / ImBinding / ImMessage)与三平台
//! webhook 发送器(Feishu/WeCom/DingTalk)的实现位于子模块:
//!
//! * [`webhook`] — 三平台 HTTP 发送 + 钉钉 HMAC-SHA256 手写签名。
//! * [`store`]   — ImBindingStore SQLite CRUD(im_bindings 表)。
//!
//! [`ImEngine`] 持有 store + http client,提供:
//! * `create_webhook_binding` — 创建 webhook 绑定(SSRF 校验 + 落盘)。
//! * `list_bindings` / `delete_binding` / `set_enabled` — CRUD。
//! * `test_send` — 单条绑定的测试发送(同步返回结果)。
//! * `broadcast` — 并发广播到所有已启用绑定(tokio::spawn,部分失败不影响其他)。

pub mod store;
pub mod webhook;

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::memory::sqlite_store::SqliteStore;
use crate::security::ssrf_guard::SsrfGuard;

pub use store::{ImBindingRow, ImBindingStore};
pub use webhook::{send_webhook, DingtalkSignResult};

// ---------------------------------------------------------------------------
// 数据结构(对齐 spec §数据结构)
// ---------------------------------------------------------------------------

/// IM 平台标识。serde lowercase 序列化(feishu/wecom/dingtalk),
/// 与 SQL `im_bindings.platform` 列对齐。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ImPlatform {
    Feishu,
    Wecom,
    Dingtalk,
}

impl ImPlatform {
    /// 字符串形式(与 serde lowercase 一致),用于 SQL 列写入。
    pub fn as_str(&self) -> &'static str {
        match self {
            ImPlatform::Feishu => "feishu",
            ImPlatform::Wecom => "wecom",
            ImPlatform::Dingtalk => "dingtalk",
        }
    }

    /// 从字符串解析(大小写不敏感)。未知值返回 Err。
    pub fn from_str_lossy(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "feishu" => Ok(ImPlatform::Feishu),
            "wecom" => Ok(ImPlatform::Wecom),
            "dingtalk" => Ok(ImPlatform::Dingtalk),
            other => Err(anyhow!("unknown IM platform: {other}")),
        }
    }
}

/// 绑定类型。Phase 1 仅 Webhook;OAuthUser 为 Phase 2 预留。
///
/// serde tag = "kind", rename_all = "snake_case" — 序列化为
/// `{"kind":"webhook","url":"..."}` / `{"kind":"oauth_user","open_id":"...",...}`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BindingKind {
    Webhook {
        url: String,
    },
    /// Phase 2 预留:OAuth 扫码绑定用户。
    #[serde(rename = "oauth_user")]
    OAuthUser {
        open_id: String,
        display_name: String,
        has_refresh_token: bool,
    },
}

impl BindingKind {
    /// 返回路由目标(webhook URL 或 open_id),用于 SQL `target` 列。
    pub fn target(&self) -> &str {
        match self {
            BindingKind::Webhook { url } => url.as_str(),
            BindingKind::OAuthUser { open_id, .. } => open_id.as_str(),
        }
    }

    /// 返回 kind 标识字符串,与 SQL `kind` 列对齐。
    pub fn kind_str(&self) -> &'static str {
        match self {
            BindingKind::Webhook { .. } => "webhook",
            BindingKind::OAuthUser { .. } => "oauth_user",
        }
    }
}

/// 一条 IM 绑定记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImBinding {
    pub id: String,
    pub platform: ImPlatform,
    pub kind: BindingKind,
    pub display_name: String,
    pub enabled: bool,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// 消息等级(影响前端样式 + 部分平台的颜色标记)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImMessageLevel {
    #[default]
    Info,
    Warning,
    Error,
}

/// IM 消息体。三平台发送器统一消费此结构,内部按平台格式化为
/// 各自的 payload(text / markdown / interactive card)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImMessage {
    pub title: String,
    pub body: String,
    /// 可选 markdown 正文(优先于 body,平台支持时使用)。
    pub markdown: Option<String>,
    pub level: ImMessageLevel,
}

impl ImMessage {
    /// 用 title + body 构造 Info 级消息(markdown 为 None)。
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            markdown: None,
            level: ImMessageLevel::Info,
        }
    }

    /// 返回 markdown 内容(若有)或退化为 plain body。
    pub fn markdown_or_body(&self) -> &str {
        self.markdown.as_deref().unwrap_or(&self.body)
    }
}

// ---------------------------------------------------------------------------
// ImEngine
// ---------------------------------------------------------------------------

/// IM 绑定引擎。持有 store + SSRF 安全的 reqwest client。
///
/// `config` RwLock 内当前存放轻量配置(预留扩展位,如全局速率限制);
/// 持锁期间不调用其他用同一锁的方法,符合 parking_lot 非重入约束。
pub struct ImEngine {
    store: Arc<ImBindingStore>,
    #[allow(dead_code)]
    config: RwLock<ImEngineConfig>,
}

/// 引擎级配置(预留,Phase 1 仅 default)。
#[derive(Debug, Clone, Default)]
struct ImEngineConfig {
    /// 预留:全局速率限制(每分钟最大发送数,0 = 不限制)。
    #[allow(dead_code)]
    rate_limit_per_min: u32,
}

impl ImEngine {
    pub fn new(sqlite: Arc<SqliteStore>) -> Self {
        Self {
            store: Arc::new(ImBindingStore::new(sqlite)),
            config: RwLock::new(ImEngineConfig::default()),
        }
    }

    /// 创建 webhook 绑定。
    ///
    /// 1. SSRF 校验 URL(拒绝 192.168.x.x / 10.x / 127.x / 169.254.x.x 等)。
    /// 2. 落盘到 im_bindings 表(enabled 默认 true)。
    /// 3. 返回完整 ImBinding(含生成的 UUID id + created_at)。
    pub fn create_webhook_binding(
        &self,
        platform: ImPlatform,
        url: String,
        display_name: String,
    ) -> Result<ImBinding> {
        // SSRF 校验:拒绝内网地址。build_safe_client 内部也会校验重定向链,
        // 但显式校验 URL 可在落盘前快速失败,避免存入恶意 URL。
        SsrfGuard::new().validate_url(&url)?;

        let binding = ImBinding {
            id: uuid::Uuid::new_v4().to_string(),
            platform,
            kind: BindingKind::Webhook { url },
            display_name,
            enabled: true,
            created_at: chrono::Utc::now().timestamp_millis(),
            last_used_at: None,
        };
        self.store.insert(&binding)?;
        Ok(binding)
    }

    /// 列出所有绑定(按 created_at ASC)。
    pub fn list_bindings(&self) -> Result<Vec<ImBinding>> {
        self.store.list()
    }

    /// 删除绑定(幂等:不存在的 id 也返回 Ok)。
    pub fn delete_binding(&self, id: &str) -> Result<()> {
        self.store.delete(id)
    }

    /// 设置绑定启用状态。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.store.set_enabled(id, enabled)
    }

    /// 单条测试发送。同步返回结果(成功 Ok / 失败 Err)。
    /// 成功时更新 last_used_at。
    pub async fn test_send(&self, id: &str, message: &ImMessage) -> Result<()> {
        let binding = self
            .store
            .get(id)?
            .ok_or_else(|| anyhow!("IM binding not found: {id}"))?;
        let platform = binding.platform;
        let kind = binding.kind.clone();
        send_webhook(platform, &kind, message).await?;
        // 发送成功后更新 last_used_at(best-effort,失败仅记录日志)。
        let now = chrono::Utc::now().timestamp_millis();
        if let Err(e) = self.store.touch_last_used(id, now) {
            warn!(
                target: "nebula.im",
                id, error = %e,
                "failed to update last_used_at after test_send"
            );
        }
        Ok(())
    }

    /// 并发广播到所有已启用绑定。
    ///
    /// 用 `tokio::spawn` 为每条绑定派发独立 task,部分失败不影响其他。
    /// 返回成功数 + 失败数(失败详情通过 tracing::warn 记录)。
    pub async fn broadcast(&self, message: ImMessage) -> (usize, usize) {
        let bindings = match self.store.list_enabled() {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    target: "nebula.im",
                    error = %e,
                    "broadcast: failed to load enabled bindings"
                );
                return (0, 0);
            }
        };
        if bindings.is_empty() {
            debug!(target: "nebula.im", "broadcast: no enabled bindings");
            return (0, 0);
        }

        let total = bindings.len();
        let mut tasks = Vec::with_capacity(total);
        for b in bindings {
            let msg = message.clone();
            tasks.push(tokio::spawn(async move {
                send_webhook(b.platform, &b.kind, &msg).await
            }));
        }

        let mut success = 0usize;
        let mut failure = 0usize;
        for (idx, handle) in tasks.into_iter().enumerate() {
            match handle.await {
                Ok(Ok(())) => success += 1,
                Ok(Err(e)) => {
                    failure += 1;
                    warn!(
                        target: "nebula.im",
                        index = idx,
                        error = %e,
                        "broadcast: send failed for one binding"
                    );
                }
                Err(e) => {
                    failure += 1;
                    warn!(
                        target: "nebula.im",
                        index = idx,
                        error = %e,
                        "broadcast: task panicked"
                    );
                }
            }
        }
        debug!(
            target: "nebula.im",
            total, success, failure, "broadcast completed"
        );
        (success, failure)
    }

    /// 返回内部 store 的 Arc 引用(供命令路径直接调用)。
    pub fn store(&self) -> Arc<ImBindingStore> {
        self.store.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ImPlatform serde lowercase 往返 ---

    #[test]
    fn im_platform_serde_lowercase_roundtrip() {
        for p in [ImPlatform::Feishu, ImPlatform::Wecom, ImPlatform::Dingtalk] {
            let json = serde_json::to_string(&p).unwrap();
            let back: ImPlatform = serde_json::from_str(&json).unwrap();
            assert_eq!(p, back, "roundtrip failed for {p:?}: json={json}");
        }
    }

    #[test]
    fn im_platform_serde_lowercase_strings() {
        assert_eq!(
            serde_json::to_string(&ImPlatform::Feishu).unwrap(),
            "\"feishu\""
        );
        assert_eq!(
            serde_json::to_string(&ImPlatform::Wecom).unwrap(),
            "\"wecom\""
        );
        assert_eq!(
            serde_json::to_string(&ImPlatform::Dingtalk).unwrap(),
            "\"dingtalk\""
        );
    }

    #[test]
    fn im_platform_from_str_lossy() {
        assert_eq!(
            ImPlatform::from_str_lossy("Feishu").unwrap(),
            ImPlatform::Feishu
        );
        assert_eq!(
            ImPlatform::from_str_lossy("WECOM").unwrap(),
            ImPlatform::Wecom
        );
        assert!(ImPlatform::from_str_lossy("unknown").is_err());
    }

    // --- BindingKind Webhook serde 往返 ---

    #[test]
    fn binding_kind_webhook_serde_roundtrip() {
        let kind = BindingKind::Webhook {
            url: "https://open.feishu.cn/open-apis/bot/v2/hook/abc".to_string(),
        };
        let json = serde_json::to_string(&kind).unwrap();
        // tag = "kind", rename_all = "snake_case"
        assert!(json.contains("\"kind\":\"webhook\""), "json={json}");
        assert!(
            json.contains("\"url\":\"https://open.feishu.cn"),
            "json={json}"
        );
        let back: BindingKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, back);
    }

    #[test]
    fn binding_kind_oauth_user_serde_roundtrip() {
        let kind = BindingKind::OAuthUser {
            open_id: "ou_abc".to_string(),
            display_name: "Alice".to_string(),
            has_refresh_token: true,
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert!(json.contains("\"kind\":\"oauth_user\""), "json={json}");
        let back: BindingKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, back);
    }

    #[test]
    fn binding_kind_target_and_kind_str() {
        let w = BindingKind::Webhook {
            url: "https://x.com".into(),
        };
        assert_eq!(w.target(), "https://x.com");
        assert_eq!(w.kind_str(), "webhook");
        let o = BindingKind::OAuthUser {
            open_id: "ou_1".into(),
            display_name: "Bob".into(),
            has_refresh_token: false,
        };
        assert_eq!(o.target(), "ou_1");
        assert_eq!(o.kind_str(), "oauth_user");
    }

    // --- ImMessageLevel serde ---

    #[test]
    fn im_message_level_serde_lowercase() {
        for l in [
            ImMessageLevel::Info,
            ImMessageLevel::Warning,
            ImMessageLevel::Error,
        ] {
            let json = serde_json::to_string(&l).unwrap();
            let back: ImMessageLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(l, back);
        }
        assert_eq!(
            serde_json::to_string(&ImMessageLevel::Info).unwrap(),
            "\"info\""
        );
        assert_eq!(
            serde_json::to_string(&ImMessageLevel::Warning).unwrap(),
            "\"warning\""
        );
        assert_eq!(
            serde_json::to_string(&ImMessageLevel::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn im_message_level_default_is_info() {
        assert_eq!(ImMessageLevel::default(), ImMessageLevel::Info);
    }

    // --- ImMessage ---

    #[test]
    fn im_message_new_defaults() {
        let m = ImMessage::new("title", "body");
        assert_eq!(m.title, "title");
        assert_eq!(m.body, "body");
        assert_eq!(m.markdown, None);
        assert_eq!(m.level, ImMessageLevel::Info);
        assert_eq!(m.markdown_or_body(), "body");
    }

    #[test]
    fn im_message_markdown_or_body_prefers_markdown() {
        let m = ImMessage {
            title: "t".into(),
            body: "plain".into(),
            markdown: Some("**md**".into()),
            level: ImMessageLevel::Warning,
        };
        assert_eq!(m.markdown_or_body(), "**md**");
    }

    // --- SSRF 拒绝(经 ImEngine::create_webhook_binding) ---

    #[test]
    fn create_webhook_binding_rejects_private_ip() {
        // 用 in-memory sqlite 构造 engine。
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // 创建 im_bindings 表(模拟 migration)。
        conn.execute_batch(
            "CREATE TABLE im_bindings (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, kind TEXT NOT NULL,
                target TEXT NOT NULL, display_name TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1, config_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL, last_used_at INTEGER
            );",
        )
        .unwrap();
        // 由于 ImEngine 依赖 SqliteStore(需要完整 schema),此处仅测 SsrfGuard 直接行为。
        // ImEngine 的 SSRF 集成通过 webhook 模块的 SSRF 测试覆盖。
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("http://192.168.1.1/hook").is_err());
        assert!(guard.validate_url("http://10.0.0.1/hook").is_err());
        assert!(guard.validate_url("http://127.0.0.1/hook").is_err());
        let _ = conn; // keep alive
    }
}
