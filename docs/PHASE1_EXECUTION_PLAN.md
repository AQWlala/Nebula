# Phase 1 执行计划：承诺兑现 + 竞品对标

> **For agentic workers:** 本计划基于 IMPROVEMENT_PLAN_v1.0.md 制定，按 Task 顺序执行。每个 Task 产出可独立验证的结果。

**Goal:** 完成 Phase 1 的关键门禁项，将 Stage 7 门禁达标从 5/10 提升至 8/10

**Architecture:** P1-A 已意外完成（tonic_server.rs 1025 行已实现全部 22 RPC），下一步聚焦 CI 稳定化 + OAuth 框架 + Skill 生态补齐

**Tech Stack:** Rust / tonic 0.12 / Tauri 2.x / OAuth 2.0 / reqwest / keyring

---

## 现状修正说明

### P1-A (gRPC wire) — 实际已完成 ✅

改进计划表中 P1-A 标记为 ⏳，但探索发现：

| 计划描述 | 实际状态 | 证据 |
|---------|---------|------|
| "JSON shim，外部客户端无法连接" | ✅ 已修复 | `grpc/mod.rs` 默认路由到 `tonic_server::start_tonic_server` |
| "实现 tonic Service trait" | ✅ 已完成 | `tonic_server.rs` (1025行) 实现 5 个 service trait / 22 个 RPC |
| "替换 accept_loop" | ✅ 已完成 | 使用 `tonic::transport::Server` + `TcpIncoming::from_listener` |
| "stream_events 真实 streaming" | ✅ 已完成 | `async_stream::stream!` + `AgentBus` broadcast channel |
| JSON shim (server.rs) | 保留为 fallback | 仅 `json-framing` feature 开启时使用 |

**结论：** P1-A 无需开发，仅需验证 + 更新计划表状态。

---

## Task 1: P1-A 验证 + 计划表更新

**Files:**
- Modify: `src-tauri/tests/integration/grpc_wire_test.rs` (更新注释)
- Modify: `IMPROVEMENT_PLAN_v1.0.md` (标记 P1-A 完成)

- [ ] **Step 1: 验证 tonic server 编译**

Run: `cd src-tauri ; cargo check --features grpc,channels --lib`
Expected: 编译通过，无 error

- [ ] **Step 2: 运行 grpc_wire_test 验证 tonic 路径**

Run: `cd src-tauri ; cargo nextest run --features grpc,channels -F integration -- grpc_wire`
Expected: 2 个测试通过（server_binds_and_accepts_tcp_connection + service_implements_all_rpcs）

- [ ] **Step 3: 更新 grpc_wire_test.rs 注释**

将文件头注释从 "v0.3 gRPC wire shim" 修正为 "tonic gRPC wire layer"：
```rust
//! P1-A regression tests for the tonic gRPC wire layer.
//!
//! The default build path uses `tonic_server::start_tonic_server`
//! (real tonic::transport::Server with prost-generated types).
//! The JSON shim in `server.rs` is only used when the `json-framing`
//! feature is explicitly enabled.
```

- [ ] **Step 4: 更新 IMPROVEMENT_PLAN_v1.0.md**

将 P1-A 所有任务状态从 ⏳ 改为 ✅，门禁表 gRPC wire 从 ❌ 改为 ✅。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tests/integration/grpc_wire_test.rs IMPROVEMENT_PLAN_v1.0.md
git commit -m "docs: mark P1-A gRPC tonic as complete — verified tonic wire layer"
```

---

## Task 2: CI 稳定化 — macOS 编译修复

**Files:**
- Modify: `.github/workflows/test.yml` (增强 macOS 错误捕获)
- Read: `src-tauri/src/commands/*.rs` (定位编译错误)

- [ ] **Step 1: 本地验证 macOS 相关代码编译**

Run: `cd src-tauri ; cargo check --features grpc,channels --lib 2>&1 | findstr "error"`
Expected: 无 error（本地 Windows 编译通过）

- [ ] **Step 2: 检查 CI 最近一次运行结果**

使用浏览器或 gh CLI 检查 GitHub Actions 最近一次 CI 运行：
```bash
gh run list --limit 3 --json status,conclusion,name,createdAt
```
记录具体失败的 job 和错误信息。

- [ ] **Step 3: 根据 CI 错误修复源码**

常见 macOS 编译问题：
- `commands/*.rs` 和 `channel/router.rs` 中的平台特定代码
- 错误信息被截断 — 增强 test.yml 的 annotation 提取逻辑

如果错误是 feature gate 缺失，添加 `#[cfg(target_os = "...")]` 守卫。
如果错误是类型不匹配，修复具体代码。

- [ ] **Step 4: 本地验证修复**

Run: `cd src-tauri ; cargo check --features grpc,channels --lib`
Expected: 编译通过

- [ ] **Step 5: Commit + Push**

```bash
git add -A
git commit -m "fix: resolve macOS compilation errors in commands/*.rs"
git push
```

---

## Task 3: CI 稳定化 — migration 测试修复

**Files:**
- Read: `src-tauri/src/memory/migration.rs` (验证 BOM 修复)
- Read: `src-tauri/migrations/001_initial.sql` (检查 BOM)

- [ ] **Step 1: 验证 migration BOM 修复**

Run: `cd src-tauri ; python -c "data=open('migrations/001_initial.sql','rb').read(3); print(data == b'\\xef\\xbb\\xbf')"`
Expected: `False`（文件无 BOM）

- [ ] **Step 2: 运行 migration 测试**

Run: `cd src-tauri ; cargo nextest run --features grpc,channels -F integration -- migration`
Expected: 测试通过

- [ ] **Step 3: 如果仍有失败，检查 migration.rs 的 BOM 跳过逻辑**

确认 `statement_is_pragma()` 和 `split_sql()` 正确跳过 U+FEFF。

- [ ] **Step 4: Commit (如有修复)**

```bash
git add -A
git commit -m "fix: migration test BOM handling for CI"
git push
```

---

## Task 4: CI 稳定化 — grpc_wire_test 连接修复

**Files:**
- Read: `src-tauri/tests/integration/grpc_wire_test.rs` (连接重试逻辑)

- [ ] **Step 1: 分析 grpc_wire_test 连接问题**

测试已有 10 次重试 + 500ms 间隔。如果 CI 仍失败，可能是：
- `start_server` 返回前 tonic listener 未就绪
- `TcpIncoming::from_listener` 需要 listener 已 bind

- [ ] **Step 2: 检查 tonic_server.rs 的 start_tonic_server 实现**

确认 `local_addr()` 返回的是已 bind 的地址。
确认 `serve_with_incoming_shutdown` 在返回 handle 前已准备好接受连接。

- [ ] **Step 3: 如果需要，增加 server readiness 等待**

在 `start_test_server()` 中，确认 server handle 的 `local_addr()` 返回有效地址后立即返回。如果 tonic 异步启动慢，可加一个短暂 sleep 或 TCP probe。

- [ ] **Step 4: Commit (如有修复)**

```bash
git add -A
git commit -m "fix: grpc_wire_test server readiness for CI reliability"
git push
```

---

## Task 5: P1-B OAuth 2.0 框架 — 基础设施

**Files:**
- Create: `src-tauri/src/identity/oauth.rs` (OAuthClient + OAuthProvider trait)
- Create: `src-tauri/src/identity/oauth_manager.rs` (OAuthManager)
- Modify: `src-tauri/src/identity/mod.rs` (注册新模块)
- Modify: `src-tauri/src/commands/security.rs` (添加 oauth_* 命令)

- [ ] **Step 1: 创建 OAuthProvider trait**

```rust
// src-tauri/src/identity/oauth.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    pub id: String,
    pub name: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn id(&self) -> &str;
    fn config(&self) -> &OAuthProviderConfig;
    async fn exchange_code(&self, code: &str) -> anyhow::Result<OAuthToken>;
    async fn refresh_token(&self, refresh: &str) -> anyhow::Result<OAuthToken>;
    async fn revoke_token(&self, token: &str) -> anyhow::Result<()>;
}
```

- [ ] **Step 2: 创建 OAuthManager**

```rust
// src-tauri/src/identity/oauth_manager.rs
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use super::oauth::{OAuthProvider, OAuthToken};

pub struct OAuthManager {
    providers: RwLock<HashMap<String, Arc<dyn OAuthProvider>>>,
    tokens: RwLock<HashMap<String, OAuthToken>>,
}

impl OAuthManager {
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
        }
    }

    pub fn register_provider(&self, provider: Arc<dyn OAuthProvider>) {
        let id = provider.id().to_string();
        self.providers.write().insert(id, provider);
    }

    pub fn list_providers(&self) -> Vec<String> {
        self.providers.read().keys().cloned().collect()
    }

    pub async fn authorize(&self, provider_id: &str, code: &str) -> anyhow::Result<()> {
        let provider = self.providers.read().get(provider_id).cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown provider: {provider_id}"))?;
        let token = provider.exchange_code(code).await?;
        // Store in OS keychain, not in memory
        crate::security::keychain::store_oauth_token(provider_id, &token.access_token)?;
        self.tokens.write().insert(provider_id.to_string(), token);
        Ok(())
    }

    pub fn get_token(&self, provider_id: &str) -> Option<OAuthToken> {
        self.tokens.read().get(provider_id).cloned()
    }

    pub async fn disconnect(&self, provider_id: &str) -> anyhow::Result<()> {
        if let Some(token) = self.tokens.write().remove(provider_id) {
            if let Some(provider) = self.providers.read().get(provider_id).cloned() {
                provider.revoke_token(&token.access_token).await.ok();
            }
        }
        crate::security::keychain::delete_oauth_token(provider_id)?;
        Ok(())
    }
}
```

- [ ] **Step 3: 注册模块到 identity/mod.rs**

在 `src-tauri/src/identity/mod.rs` 中添加：
```rust
pub mod oauth;
pub mod oauth_manager;
pub use oauth::{OAuthProvider, OAuthToken, OAuthProviderConfig};
pub use oauth_manager::OAuthManager;
```

- [ ] **Step 4: 添加 keychain 存储函数**

在 `src-tauri/src/security/keychain.rs` 中添加 `store_oauth_token` / `get_oauth_token` / `delete_oauth_token` 函数，使用 OS keychain 存储 token。

- [ ] **Step 5: 添加 Tauri 命令**

在 `src-tauri/src/commands/security.rs` 中添加：
```rust
#[tauri::command]
pub async fn oauth_authorize(state: tauri::State<'_, AppState>, provider_id: String, code: String) -> Result<(), String> {
    // ...
}

#[tauri::command]
pub fn oauth_list_providers(state: tauri::State<'_, AppState>) -> Vec<String> {
    // ...
}

#[tauri::command]
pub async fn oauth_disconnect(state: tauri::State<'_, AppState>, provider_id: String) -> Result<(), String> {
    // ...
}
```

- [ ] **Step 6: 注册命令到 invoke_handler**

在 `src-tauri/src/tauri_setup.rs` 的 `invoke_handler` 中添加：
```rust
crate::commands::security::oauth_authorize,
crate::commands::security::oauth_list_providers,
crate::commands::security::oauth_disconnect,
```

- [ ] **Step 7: 在 AppState 中添加 OAuthManager**

在 `src-tauri/src/app_state.rs` 中添加字段：
```rust
pub oauth_manager: Arc<crate::identity::OAuthManager>,
```

在 `src-tauri/src/bootstrap.rs` 和 `bootstrap_headless.rs` 中初始化：
```rust
let oauth_manager = Arc::new(crate::identity::OAuthManager::new());
```

- [ ] **Step 8: 验证编译**

Run: `cd src-tauri ; cargo check --features grpc,channels --lib`
Expected: 编译通过

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat: P1-B OAuth 2.0 framework — OAuthProvider trait + OAuthManager + keychain storage"
git push
```

---

## Task 6: P1-C Skill 自动发现

**Files:**
- Create: `src-tauri/src/skills/discover.rs` (4 层扫描器)
- Modify: `src-tauri/src/skills/mod.rs` (注册模块)
- Modify: `src-tauri/src/bootstrap.rs` (启动时调用自动发现)

- [ ] **Step 1: 创建 SkillDiscoverer**

```rust
// src-tauri/src/skills/discover.rs
use std::path::{Path, PathBuf};
use crate::skills::store::SkillStore;

pub struct SkillDiscoverer {
    scan_paths: Vec<PathBuf>,
}

impl SkillDiscoverer {
    pub fn new() -> Self {
        let mut paths = vec![
            PathBuf::from(".nebula/skills"),           // 项目级
            dirs::home_dir().map(|h| h.join(".nebula/skills")).unwrap_or_default(), // 用户级
            PathBuf::from("/etc/nebula/skills"),       // 系统级 (Linux)
            PathBuf::from("skills"),                    // 工作区级
        ];
        paths.retain(|p| p.exists());
        Self { scan_paths: paths }
    }

    pub async fn discover(&self, store: &SkillStore) -> anyhow::Result<usize> {
        let mut count = 0;
        for path in &self.scan_paths {
            count += self.scan_directory(path, store).await?;
        }
        Ok(count)
    }

    async fn scan_directory(&self, dir: &Path, store: &SkillStore) -> anyhow::Result<usize> {
        // 扫描 SKILL.md 文件，解析 frontmatter，注册到 SkillStore
        // ...
    }
}
```

- [ ] **Step 2: 注册模块**

在 `src-tauri/src/skills/mod.rs` 中添加 `pub mod discover;`

- [ ] **Step 3: 在 bootstrap 中调用**

在 `src-tauri/src/bootstrap.rs` 的 `bootstrap_skills` 方法末尾添加：
```rust
let discoverer = crate::skills::discover::SkillDiscoverer::new();
match discoverer.discover(ss.as_ref()).await {
    Ok(n) if n > 0 => info!(target: "nebula", count = n, "skills auto-discovered"),
    Ok(_) => {}
    Err(e) => warn!(target: "nebula", error = %e, "skill auto-discovery failed"),
}
```

- [ ] **Step 4: 验证编译**

Run: `cd src-tauri ; cargo check --features grpc,channels --lib`
Expected: 编译通过

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: P1-C skill auto-discovery — 4-layer scanner for SKILL.md files"
git push
```

---

## 验收检查表

### Task 1 完成后
- [ ] P1-A 在 IMPROVEMENT_PLAN_v1.0.md 中标记为 ✅
- [ ] Stage 7 门禁 gRPC wire 达标

### Task 2-4 完成后
- [ ] CI 三平台（ubuntu/windows/macos）全绿
- [ ] Stage 7 门禁 CI 门前达标

### Task 5 完成后
- [ ] `identity/oauth.rs` 编译通过
- [ ] `oauth_authorize` / `oauth_list_providers` / `oauth_disconnect` 命令可调用
- [ ] token 存入 OS keychain

### Task 6 完成后
- [ ] `skills/discover.rs` 编译通过
- [ ] 启动时自动扫描 `~/.nebula/skills/`

### 最终门禁达标目标
- 危险 panic 点 < 50 ✅ (35)
- lib.rs 行数 < 300 ✅ (162)
- 前端测试 ≥ 12 ✅ (12)
- gRPC wire ✅ (Task 1 验证)
- 渠道路由 ✅
- 至少 1 个 OAuth 框架 ✅ (Task 5)
- evolution_run ✅
- **达标项：7/10** (从 5/10 提升)

---

**计划结束。按 Task 顺序执行，每个 Task 完成后 commit + push。**
