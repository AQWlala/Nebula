-- T-E-C-17: IM 扫码绑定 — im_bindings 表。
--
-- Phase 1 Webhook 优先:持久化 Feishu/WeCom/DingTalk 三平台 webhook 绑定。
-- Phase 2 OAuth 扫码绑定复用本表,kind 字段区分(webhook vs oauth_user)。
--
-- 字段说明:
--   id           — UUID v4,主键
--   platform     — 平台标识(feishu/wecom/dingtalk),与 ImPlatform serde lowercase 对齐
--   kind         — 绑定类型(webhook/oauth_user),与 BindingKind serde tag 对齐
--   target       — 路由目标(webhook URL 或 open_id),供快速查找
--   display_name — 用户可读名称(如 "团队群")
--   enabled      — 是否启用(0/1),broadcast 仅发送 enabled=1 的绑定
--   config_json  — 完整 BindingKind JSON(Webhook.url / OAuthUser.*),与 kind 配套
--   created_at   — 创建时间(Unix 毫秒)
--   last_used_at — 上次成功发送时间(Unix 毫秒,可空)
CREATE TABLE IF NOT EXISTS im_bindings (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);

-- 按平台 + 启用状态查询(供 broadcast 仅取已启用绑定)
CREATE INDEX IF NOT EXISTS idx_im_bindings_platform_enabled
    ON im_bindings(platform, enabled);

-- 按 target 查询(去重 / 查找现有 webhook)
CREATE INDEX IF NOT EXISTS idx_im_bindings_target
    ON im_bindings(target);
