# Nebula · 故障排查 (Troubleshooting)

> v1.0. 按问题类别组织。

---

## 1. 安装

### 1.1 macOS: "Nebula.app 已损坏，无法打开"

* **原因** — 没有 Apple Developer ID 签名。
* **解决** — `xattr -dr com.apple.quarantine /Applications/Nebula.app`。
  或者右键 → *打开* → *打开*。
* **长期** — v1.1 我们会上 Apple Developer 账号签名 + notarization。

### 1.2 Windows: SmartScreen 阻止运行

* **解决** — *更多信息* → *仍要运行*。
* **长期** — 提交给 Microsoft 做 EV 签名。

### 1.3 Linux: AppImage 无法执行

```bash
chmod +x nebula-v1.0.0-x86_64.AppImage
./nebula-v1.0.0-x86_64.AppImage
```

缺少 fuse：

```bash
sudo apt install libfuse2
```

---

## 2. 启动

### 2.1 启动崩溃 / 白屏

1. 看终端 / 控制台：
   * macOS / Linux: `tail -F ~/Library/Logs/nebula/*.log` 或 `~/.local/share/nebula/logs/`
   * Windows: 事件查看器 → Applications
2. 启用 JSON 日志：`NEBULA_LOG_FORMAT=json NEBULA_LOG_DIR=./logs ./nebula`。
3. 提交 GitHub issue，附上日志。

### 2.2 启动慢

* **首次** — 跑迁移 + 写 schema，正常。
* **后续** — 检查杀毒软件是否在扫描 `nebula.exe`。
* **建议** — 把 `NEBULA_DB` 放到 SSD。

### 2.3 "Nebula启动失败" 对话框

* 看对话框里的错误码 + 消息。
* 常见：
  * `sqlite` — 权限不足 / 磁盘满
  * `lance` — 写入权限
  * `llm` — Ollama 不可达
  * `internal` — 见日志

---

## 3. Ollama

### 3.1 "无法连接模型"

```bash
curl http://127.0.0.1:11434/api/tags
# 期待：JSON 列表
```

* 报错 `connection refused` → `ollama serve` 没跑。
* 报错 `timeout` → 防火墙 / VPN / Ollama 端口被改。
* 远程 URL 需要 token → 在 `NEBULA_REMOTE_URL` 设置 Bearer。

### 3.2 模型下载慢

Ollama 镜像在 `~/.ollama/models/`。改 `OLLAMA_MODELS` 环境变量。

### 3.3 中文乱码

* 嵌入模型需要支持中文：`bge-small-zh-v1.5`、`bge-large-zh-v1.5`。
* 对话模型用 `qwen2.5:3b` 中文效果不错。

---

## 4. 记忆 / 搜索

### 4.1 搜索结果为空

* 确认 LanceDB 路径存在：`ls -la ~/.local/share/nebula/lance/`。
* 检查嵌入模型是否一致 — 换模型后旧向量会失效。

### 4.2 内存增长失控

* L0 太多没被压缩 — 调小 `NEBULA_BH_DAYS`（默认 30）。
* 反思频率过低 — 调小 `NEBULA_REFLECT_INTERVAL`（默认 600s）。

### 4.3 SQLite locked

另一个进程占用了 DB。检查：

```bash
# macOS / Linux
fuser nebula.db
```

---

## 5. 编辑器

### 5.1 文件读取失败

* **路径越界** — 工作区根以外的路径会被拒。
* **文件过大** — 默认 8 MB 上限。
* **编码** — 假设 UTF-8；GBK 等需手动转。

### 5.2 Git 命令失败

* 工作区不是 git 仓库 → `git init`。
* 没装 `git` → 装 git。

### 5.3 Shell 命令被拒

* 只有 24 个白名单二进制可执行。
* 想加新的？v1.0 不支持运行时加，需 v1.1。开发可以改 `os/shell.rs::default_whitelist`。

---

## 6. E2EE 同步

### 6.1 解密失败

* 公钥 / 私钥不匹配。
* Envelope 损坏 — 检查传输。
* 协议版本不匹配（v0.5 与 v1.0 的 envelope 不兼容）—— 双方需同版本。

### 6.2 fingerprint 校验

`fingerprint` 是 16 字节短哈希。匹配 → 密钥正确；不匹配 → 公钥错了。

---

## 7. UI

### 7.1 字体大小不生效

改完设置后，CSS 变量在 `document.documentElement` 上更新。如果还不对，刷新一下。

### 7.2 ⌘K 不工作

* 焦点不在 input 里。
* 检查 `useCommandPaletteShortcut` 是否被调用（看 DevTools）。

### 7.3 Toast 不显示

* `Toasts` 组件没挂载在 App 树里。
* CSS 没加载 — 检查 `global.css` 是否包含 `.toast-stack`。

### 7.4 状态栏一直 "offline"

* Ollama 不在跑。
* 设置里的 `OLLAMA_URL` 配错。

---

## 8. 性能

### 8.1 RSS 超过 500MB

* 状态栏会标红。
* 关掉大文件、关掉不用的视图。
* 提交 issue，附 `startup_report` 输出。

### 8.2 操作响应 > 200ms

* 看 `metrics` command — 哪个计数器暴涨？
* LLM 慢 → 换小模型。
* LanceDB 慢 → 检查磁盘 IO。

### 8.3 反思太频繁

调高 `NEBULA_REFLECT_INTERVAL`。

---

## 9. 自动更新

### 9.1 检测不到更新

* endpoint 配错 — 默认 `https://github.com/...`。
* 没有网络。

### 9.2 签名验证失败

* `pubkey` 没配或配错 — 在 `tauri.conf.json::plugins.updater.pubkey`。
* 解决：用 `tauri signer sign` 重新签名 release。

### 9.3 不想自动更新

`tauri.conf.json` 里设 `"updater": { "active": false }`。

---

## 10. 调试清单

提交 issue 时附：

1. 操作系统 + 版本
2. Nebula版本 (`health` command)
3. `startup_report` 输出
4. `metrics` 输出
5. 复现步骤
6. 日志 (`NEBULA_LOG_DIR` 下)

---

## 11. 已知问题 (v1.0)

| Issue | 说明 | 计划版本 |
| ----- | ---- | -------- |
| E2EE 是单棘轮 | 密钥泄露后历史消息不安全 | v1.1 双棘轮 |
| API key 明文存 | `settings.json` 没加密 | v1.1 OS keychain |
| shell 白名单写死 | 用户不能运行时加 | v1.1 |
| 没有 iOS / Android | 当前仅 desktop | v2.0 |
| 没有官方插件 SDK | 第三方集成需手动改代码 | v1.1 |
| 多用户 | 不支持 | v2.0 |
