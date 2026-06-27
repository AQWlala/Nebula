# 🔧 CI / CD Secrets Setup (v1.0.1)

> **本文件是 nine-snake 项目 GitHub Actions 所需 secret 的**唯一权威**清单。**
> 任何在 `.github/workflows/` 中引用、但本文件未列出的 secret 都是
> 实现错误，请**先**修复 workflow 再继续。

> 配合阅读：[`SECURITY_KEY_ROTATION.md`](SECURITY_KEY_ROTATION.md)
> 了解为什么 v1.0.1 之后必须使用**新**的 `TAURI_SIGNING_PRIVATE_KEY`，
> 而不是 v1.0.0 时期仓库里误提交的那一对。

---

## 1. 配置入口

GitHub → 仓库 → **Settings** → **Secrets and variables** → **Actions**
→ **New repository secret**。

⚠️ **永远不要**把 secret 写在 PR 评论 / commit message / issue / docs
里。本文件只描述 secret 的**名称、用途、生成方式**——不包含任何值。

---

## 2. Secret 清单

### 2.1 Tauri 自动更新签名（必需 · 4 平台通用）

| Secret 名称 | 类型 | 内容 | 生成方式 |
|------------|------|------|----------|
| `TAURI_SIGNING_PRIVATE_KEY` | base64 文本 | v1.0.1 新生成的 32 字节 Ed25519 **私钥 seed**（含末尾换行） | `cat keys/updater_private.b64`（在本地重新跑 `python scripts/generate-updater-key.py` 后） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | base64 文本 | v1.0.1 新生成的 32 字节随机密码 | `cat keys/updater_private_password.b64` |

> **v1.0.0 的旧密钥已永久作废**。如果 GitHub 里还残留着 v1.0.0 时期的值，
> **必须立即删除**——否则旧密钥可被滥用。详见
> [`SECURITY_KEY_ROTATION.md` §4.3](SECURITY_KEY_ROTATION.md#43-ci--部署运维)。

### 2.2 macOS 代码签名与公证（必需 · 仅 macos runner）

| Secret 名称 | 类型 | 内容 | 生成方式 |
|------------|------|------|----------|
| `APPLE_CERTIFICATE` | base64 文本 | Developer ID Application `.p12`（base64 编码，**无**末尾换行） | `base64 -i DeveloperID.p12 \| pbcopy`（macOS）或 `certutil -encode`（Windows） |
| `APPLE_CERTIFICATE_PASSWORD` | 文本 | `.p12` 的导出密码 | 创建 .p12 时设置的密码 |
| `KEYCHAIN_PASSWORD` | 文本 | GitHub runner 上临时 keychain 的解锁密码（任意随机串） | `openssl rand -hex 32` |
| `APPLE_SIGNING_IDENTITY` | 文本 | 完整证书名 | 在 Keychain Access 中右键证书 → "Get Info" → "Common Name"。例：`Developer ID Application: Your Name (TEAMID1234)` |
| `APPLE_ID` | 邮箱 | Apple Developer 账号邮箱 | Apple Developer 注册邮箱 |
| `APPLE_PASSWORD` | 文本 | **App-specific password**（不是账号密码） | <https://appleid.apple.com/account/manage> → App-Specific Passwords |
| `APPLE_TEAM_ID` | 文本 | 10 位 Apple Team ID | <https://developer.apple.com/account/#/membership> |

> macOS 公证（notarization）由 `tauri-action` 内部调用
> `xcrun notarytool` 完成，**不需要**额外的 `--api-key` 文件。

### 2.3 Windows 代码签名（必需 · 仅 windows runner）

| Secret 名称 | 类型 | 内容 | 生成方式 |
|------------|------|------|----------|
| `WINDOWS_CERTIFICATE` | base64 文本 | EV / OV code-signing `.pfx`（base64 编码） | `base64 -w 0 nine-snake.pfx > b64.txt`（Git Bash / WSL）或 `[Convert]::ToBase64String([IO.File]::ReadAllBytes('nine-snake.pfx'))`（PowerShell） |
| `WINDOWS_CERTIFICATE_PASSWORD` | 文本 | `.pfx` 的导入密码 | 创建 .pfx 时设置的密码 |

> 强烈建议使用 **EV (Extended Validation)** 证书——它不需要累计 SmartScreen
> 信誉。否则首次安装时 Windows 会弹出 SmartScreen 警告。

### 2.4 内置 GitHub 提供的 secret

| Secret 名称 | 来源 | 用途 |
|------------|------|------|
| `GITHUB_TOKEN` | GitHub 自动 | `actions/checkout`、`tauri-action`、`softprops/action-gh-release` |
| `RUNNER_TEMP` | GitHub Actions runner | workflow 中用作临时目录（**不是** secret，但常被混用） |

---

## 3. 验证 checklist

在 push 第一个 v1.0.1 tag 之前，逐条确认：

- [ ] 所有 §2.1 / §2.2 / §2.3 中的 secret 都已添加到 GitHub。
- [ ] 仓库**不再**有 v1.0.0 时期的 secret 残留。
- [ ] 至少成功跑过一次 `Actions → release → build` workflow（可以
      `workflow_dispatch` 触发，不打 tag）。
- [ ] 产出的 `linux-x86_64` 产物能正常启动（无签名要求）。
- [ ] 产出的 `macos-aarch64` 产物 `codesign -dv` 显示正确的 Team ID 且
      `xcrun stapler validate` 通过。
- [ ] 产出的 `windows-x86_64` 产物右键 → "属性" → "数字签名" 显示
      有效的证书链。
- [ ] `cargo test --test integration key_rotation` 在 PR 中通过。
- [ ] `cargo test --test integration updater_pubkey` 在 PR 中通过。
- [ ] `cargo audit` 在 PR 中通过（无未处理的高危 advisory）。

---

## 4. Secret 轮换策略

| Secret | 轮换频率 | 触发事件 |
|--------|----------|----------|
| `TAURI_SIGNING_PRIVATE_KEY` | **每 12 个月** 或**任何怀疑泄露** | `SECURITY_KEY_ROTATION.md` 流程 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 同上 | 同上 |
| `APPLE_CERTIFICATE` | 证书到期前 30 天 | Apple Developer 邮件通知 |
| `APPLE_PASSWORD` | **每 6 个月** | 内部安全策略 |
| `KEYCHAIN_PASSWORD` | 每次 CI 跑（不强求持久化） | — |
| `WINDOWS_CERTIFICATE` | 证书到期前 30 天 | CA 邮件 |
| `WINDOWS_CERTIFICATE_PASSWORD` | **每 12 个月** | 内部安全策略 |

---

## 5. 常见错误

| 症状 | 原因 | 修复 |
|------|------|------|
| `codesign` failed: `errSecInternalComponent` | `.p12` 未导入 keychain，或 import 时漏了 `-A` | 检查 `Import Apple Developer Certificate` 步骤 |
| `notarytool` failed: `Authentication failed` | `APPLE_PASSWORD` 是账号密码而非 app-specific | 重新生成 app-specific password |
| `signtool` failed: `No certificates were found` | `.pfx` 密码错误 | 重新导出 `.pfx` 并更新 `WINDOWS_CERTIFICATE_PASSWORD` |
| Workflow 在 macOS runner 上找不到 `tauri-action` | GitHub 缓存的旧 ref | 清空 Actions cache 后重跑 |
| `cargo audit` 报 `RUSTSEC-2024-XXXX` | 依赖里有已知漏洞 | 见 [§6 应急流程](#6-应急流程) |

---

## 6. 应急流程

1. **怀疑 secret 泄露**：
   1. 立即在 GitHub → Settings → Secrets **rotate**（重新生成）所有
      受影响 secret。
   2. 若是 `TAURI_SIGNING_PRIVATE_KEY`：按
      [`SECURITY_KEY_ROTATION.md`](SECURITY_KEY_ROTATION.md) 走**完整**
      轮换流程（发新版 client，强制升级）。
2. **CVE / RustSec advisory**：
   1. 在 `src-tauri/Cargo.toml` 中升级受影响的 crate。
   2. 提交 PR + `cargo audit` 通过。
   3. 必要时 hotfix release（版本号 `+0.0.1`）。

---

## 7. 参考

* Tauri 自动更新：<https://v2.tauri.app/distribute/sign/>
* Tauri-action secrets：<https://github.com/tauri-apps/tauri-action#inputs>
* Apple 公证：<https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution>
* signtool：<https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool>

---

**最后更新**：2026-06-21 · v1.0.1 release.
**负责**：nine-snake DevOps 维护者.
