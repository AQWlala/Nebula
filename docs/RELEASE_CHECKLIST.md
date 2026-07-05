# 发布准备检查清单

> **关联**: M7b #98 发布准备
> **日期**: 2026-07-05
> **版本**: v2.0.0(蜂群进化 v2.0 + ADR-003)

---

## 1. 发布前检查

### 1.1 编译验证

- [x] `cargo check`(default features)exit 0
- [x] `cargo check --features soul-system` exit 0
- [x] `cargo check --features unified-dispatcher` exit 0
- [x] `npm run typecheck` exit 0
- [ ] `cargo build --release` exit 0(运行中)
- [ ] `cargo build --release --features soul-system` exit 0(待验证)

### 1.2 测试验证

- [x] `cargo test --lib`:1340+ passed(含 22 migration + 18 sqlite_store + 39 cost_tracker + 19 dispatcher + 26 sponge + 19 injection_guard + 10 ssrf_guard)
- [x] `cargo test --test integration`:114 passed, 0 failed, 4 ignored
- [x] `cargo test --test m5_test`:16 passed
- [x] `cargo bench --bench dispatcher --features unified-dispatcher -- --test`:3 个基准 Success

### 1.3 安全验证

- [x] injection_guard 全路径覆盖(13 处)
- [x] SSRF 校验(13 处)
- [x] is_local_only 强制执行(审计结论:严密)
- [x] `docs/SECURITY_AUDIT_REPORT.md` 完整

### 1.4 文档验证

- [x] `docs/ADR-003-unified-model-dispatcher.md` v2.1(已实施状态)
- [x] `docs/CHANGELOG.md`(M0a-M7b 全里程碑)
- [x] `docs/MIGRATION_ROLLBACK.md`(回滚策略)
- [x] `docs/FEATURE_FLAG_AUDIT.md`(feature flag 审计)
- [x] `docs/SECURITY_AUDIT_REPORT.md`(安全审计报告)
- [x] `docs/PRODUCTION_TASK_TRACKER.md`(进度更新)

---

## 2. Feature Flag 配置

### 2.1 默认构建(推荐用户使用)

```bash
cargo build --release
```

- 默认 features: `vector-store`
- 所有 v2.0 feature flag(soul-system / master-orchestrator / evolution-engine / unified-dispatcher)均 off
- 用户可通过环境变量 + Settings UI 在运行时启用部分功能

### 2.2 完整功能构建(开发/测试)

```bash
cargo build --release --features "soul-system,master-orchestrator,evolution-engine,unified-dispatcher"
```

- 启用所有 v2.0 功能
- soul-system 隐含 unified-dispatcher
- master-orchestrator 隐含 unified-dispatcher
- evolution-engine 隐含 self-evolution + unified-dispatcher

### 2.3 运行时启用

用户通过环境变量启用功能(编译期 feature 必须已开启):

```powershell
# Soul 系统
$env:SOUL_SYSTEM_ENABLED = "1"

# 进化引擎
$env:EVOLUTION_ENABLED = "1"
```

或通过 Settings UI 的运行时开关(soul_system_set_enabled / evolution_set_enabled)。

---

## 3. 数据库迁移

### 3.1 迁移版本

当前 bundled_migrations() 包含 001-036 共 36 个迁移:
- 001-022: v0.x 基础 schema
- 023-029: v1.x 扩展(inbox/cost/skill/CRDT)
- 030-036: v2.0 新增(domain/arena/cost_work_type 等)

### 3.2 迁移前备份

`run_bundled_migrations()` 在应用 pending migrations 前自动用 `VACUUM INTO` 创建备份:
- 备份位置:`<db_dir>/<db_name>.migrate_v<from>_to_v<to>.bak`
- 失败仅 warn,不阻塞迁移
- `:memory:` 数据库跳过备份

### 3.3 回滚策略

参见 `docs/MIGRATION_ROLLBACK.md`:
- 标准回滚:用备份覆盖当前数据库
- 部分回滚:降级 user_version + 手动执行回滚 SQL
- 紧急恢复:`.recover` 命令或 BackupScheduler 定期备份

---

## 4. 发布包验证

### 4.1 Tauri 打包

```bash
cd src-tauri
cargo tauri build
```

生成:
- `src-tauri/target/release/bundle/msi/*.msi`(Windows 安装包)
- `src-tauri/target/release/bundle/nsis/*.exe`(NSIS 安装包)

### 4.2 安装包测试清单

- [ ] 在干净 Windows 11 上安装
- [ ] 首次启动无崩溃
- [ ] 数据库自动创建(路径:`%LOCALAPPDATA%\com.nebula.desktop\`)
- [ ] 日志文件创建(路径:`%LOCALAPPDATA%\nebula\logs\`)
- [ ] 基础聊天功能可用
- [ ] Settings 面板可访问
- [ ] 升级测试(从 v1.x 数据库升级,验证迁移成功)

### 4.3 签名

- [ ] 代码签名证书已配置(`src-tauri/tauri.conf.json` 的 `signingIdentity`)
- [ ] 安装包签名验证通过

---

## 5. 已知限制

### 5.1 Feature Flag 设计偏差

`master-orchestrator` 和 `unified-dispatcher` 采用"编译期 gate + Option<Arc> 软回退"模式,而非 ADR-004 原设计的 AtomicBool 运行时开关。详见 `docs/FEATURE_FLAG_AUDIT.md`。

### 5.2 旧路径代码

M7a 完成后,chat 命令已迁移到 UnifiedModelDispatcher,但旧路径(LlmGateway::chat_stream)作为回退保留。feature off 时使用旧路径。建议在 ADR-004 清理阶段删除。

### 5.3 Flaky 测试

以下测试可能因环境因素 flaky:
- `ollama` concurrency 测试(并发竞争)
- `keychain` env fallback 测试(env var 污染)

单独运行均通过,不影响 release。

---

## 6. 发布后监控

- [ ] 应用启动日志无 error
- [ ] 数据库迁移成功(检查 `PRAGMA user_version` = 36)
- [ ] 用户反馈渠道畅通
- [ ] 准备 hotfix 流程

---

## 7. 发布批准

- [ ] 所有 M7b 任务(#90-#98)完成
- [ ] 编译 + 测试 + 安全验证通过
- [ ] 文档完备
- [ ] 安装包测试通过
- [ ] 架构组批准发布
