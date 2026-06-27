# 🔐 SECURITY: Tauri Updater Signing Key Rotation (v1.0.0 → v1.0.1)

> ⚠️ **本仓库的 updater 私钥已在 v1.0.1 轮换。**
> 旧 v1.0.0 私钥**已被永久废除**。继续信任 v1.0.0 签名的 update manifest
> 等同于接受任意恶意更新。

---

## 1. 事件时间线

| 时间 (Asia/Shanghai) | 事件 |
|----------------------|------|
| 2026-06-21 之前 | v1.0.0 仓库的首次 commit **误提交** 了 `keys/updater_private.b64` 与 `keys/updater_private_password.b64`。 |
| 2026-06-21 (v1.0.1) | 私钥被标记为 **compromised** 并生成新的 Ed25519 密钥对。`tauri.conf.json::plugins.updater.pubkey` 已指向新公钥。 |

旧公钥（**已作废**）：
```
1F44kpaO8aqD+6pQBCUlNhCBuMJ5hnAFEFCf3GFNKJY=
```

新公钥（**当前有效**）：
```
vl2AY5Eme9dkHDZG0e/4e+cFmuk/41zgGH9LCAmflVc=
```

---

## 2. 受影响范围

* **直接受影响**：所有在 2026-06-21 之前从 v1.0.0 commit 直接 / 间接 fork
  的仓库。该 fork 的 git 历史中包含旧私钥。
* **下游用户**：所有运行 v1.0.0 客户端的用户在 OTA 检查时**不会**因此事件
  立即受害（attacker 仍需先攻破 GitHub Releases 的签名），但 v1.0.1 起
  强制升级以彻底切断这条链。

---

## 3. 已采取的修复（P0#01）

1. 用 `scripts/generate-updater-key.py` **生成新密钥对**（Ed25519，32 字节）。
2. `tauri.conf.json::plugins.updater.pubkey` → 新公钥。
3. **仓库内 `keys/updater_private.b64` 与 `keys/updater_private_password.b64`
   已删除**。仅保留 `keys/updater_public.b64`（公钥可公开）。
4. `.gitignore` 中 `keys/` 条目**保留**，避免再次入库。
5. 新增守护测试 `tests/integration/key_rotation_test.rs`：
   * `tauri.conf.json` 的 pubkey 必须与 `keys/updater_public.b64` **byte-equal**。
   * 当前 pubkey **不能**出现在「已知 compromised 列表」中。
6. 旧 `updater_pubkey_test.rs` 也加了一条 compromised 列表断言。

> **注意**：本仓库的 git 历史仍包含 v1.0.0 的私钥（旧 commit）。
> 这是**妥协方案**——理想情况下应使用 `git filter-repo` 或 `BFG Repo-Cleaner`
> 重写整个历史。考虑到重写历史对协作者与 fork 的破坏性远大于直接轮换密钥
> 的安全收益，我们采用「废弃旧公钥 + 强制升级」的策略。

---

## 4. 用户与 Fork 维护者必须采取的行动

### 4.1 普通用户
1. **立即升级到 v1.0.1**。v1.0.0 客户端在下一次 OTA 时**不会被新签名接受**，
   属于"安全失败"——但建议手动下载最新安装包以获得其他修复。
2. 升级后无需任何额外配置（公钥已硬编码在 `tauri.conf.json` 中）。

### 4.2 Fork 维护者
**若你从 v1.0.0 之前 fork 过此仓库，请立即**：

1. **删除 fork**（GitHub → Settings → Danger Zone → Delete this repository）。
2. 重新从 upstream `https://github.com/nine-snake/nine-snake` 拉取最新代码。
3. 若你的 fork 中已经派生出新 commit，请人工逐文件 `diff` 合并，**不要**使用
   `git pull` 自动同步（这会保留旧密钥 commit 的可达性）。
4. **轮换你自己 fork 的所有 deploy key / PAT**——它们若曾在旧 fork 上使用，
   视为 compromised。

### 4.3 CI / 部署运维
1. 在 GitHub repo → Settings → Secrets → Actions 中：
   * `TAURI_SIGNING_PRIVATE_KEY` = 新私钥（base64，**含**末尾换行）
   * `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = 新密码（base64）
   * **删除**任何残留的旧密钥值。
2. 完整 secret 清单见 [`docs/CI_SETUP.md`](CI_SETUP.md)。
3. 任何外部 CI runner / Docker 镜像 / 备份中**搜索**旧私钥
   （`vuLhrQs+LBs+v75Iablb5eVb8G5tvr3hhEWj1iZrpd4=`）
   并清除。`grep -r` 全仓库、CI 缓存、容器镜像层。

---

## 5. 为什么不重写 git 历史？

| 方案 | 收益 | 代价 |
|------|------|------|
| `git filter-repo` 重写 | 旧私钥从历史中消失 | 所有 fork / clone 失效；所有 commit hash 变化；`git log` 不可信 |
| **轮换密钥 + 强制升级**（本方案） | 旧公钥作废，签名验证天然失败 | 历史中仍可访问旧私钥（但已**没有匹配的信任锚**） |

由于 v1.0.0 客户端的**信任锚是 v1.0.0 公钥**——而 v1.0.0 公钥不再被任何
v1.0.1+ 客户端承认——历史中残留的旧私钥**没有可签署的对象**。这等价于
"密钥销毁"。若未来 v1.0.0 公钥以任何形式复活，签名都将被拒绝。

---

## 6. 审计与检测

* 自动化检查：`cargo test --test integration key_rotation` 在 CI 中必须通过。
* 人工 review：每次 release 前确认 `keys/` 目录**仅**含 `updater_public.b64`。
* 监控：GitHub → Security → Secret scanning 警报会标记任何 `keys/*.b64`
  私钥文件再次入库。

---

## 7. 引用

* Tauri 官方 signing 流程：<https://v2.tauri.app/distribute/sign/>
* Ed25519 密钥生成（`cryptography` Python lib）— 与 `ed25519-dalek` byte-兼容。
* NIST SP 800-186：EdDSA 私钥泄露后的轮换建议（短期：弃用 + 重发；长期：重写历史）。

---

**最后更新**：2026-06-21 · v1.0.1 强制升级生效。
**负责**：nine-snake 安全维护者。
