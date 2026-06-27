# 九头蛇 · 用户指南 (User Guide)

> 适用于 v1.0。面向最终用户。

---

## 1. 快速开始

### 1.1 安装

* **macOS** — 双击 `.dmg`，把 *九头蛇* 拖到 *Applications*。
* **Windows** — 双击 `.msi`，按向导完成安装。
* **Linux** — `.AppImage` 直接运行；`.deb` 用 `sudo dpkg -i ...`；`.rpm` 用 `sudo rpm -i ...`。

### 1.2 启动

打开 *九头蛇*，会看到引导页（首次使用）。完成 4 步介绍后即可使用。

### 1.3 配置 Ollama

九头蛇默认调用本地 Ollama：

```bash
# 安装 Ollama
curl -fsSL https://ollama.com/install.sh | sh
ollama serve           # 启动服务
ollama pull qwen2.5:3b # 拉一个对话模型
ollama pull bge-small-zh-v1.5 # 拉一个嵌入模型
```

*九头蛇 → 设置 → Ollama URL* 可改成远程地址。

---

## 2. 界面速览

| 区域 | 作用 |
| ---- | ---- |
| **左栏** | 5 个主视图：对话、蜂群、记忆、代码、技能 |
| **顶栏 (Code 视图)** | 三模式切换：写作 / 工作 / 代码 |
| **主面板** | 当前视图的内容 |
| **底栏** | 模式、记忆数、内存占用、LLM 状态 |
| **命令面板 (⌘K / Ctrl+K)** | 模糊搜索所有命令 |

---

## 3. 三种工作模式

### 3.1 写作模式 (Writing)

* 选择 **模板**（日记、技术博客、报告、邮件…）
* 填写占位符，开始写
* 每隔 N 秒自动保存到 SQLite
* 一键导出 Markdown / HTML

### 3.2 工作模式 (Work)

* 创建任务：标题、描述、优先级、截止日期
* 拖动到 *Doing* → 启动计时器
* 拖动到 *Done* → 停止计时
* 会议纪要：粘贴转写稿，自动提取决议与 Action Items

### 3.3 代码模式 (Code)

* 左侧文件树、右侧 Monaco 编辑器、底部 xterm
* Git 状态、Log、Diff、Commit 一体化
* Shell 可执行白名单内的命令（`ls`、`git`、`cargo`…）

---

## 4. 8 层记忆

| 层 | 名称 | 用途 |
| -- | ---- | ---- |
| L0 | 原始 | 字节级原始事实 |
| L1 | 情景 | 一次对话、一条消息 |
| L2 | 语义 | 提炼的实体 / 概念 |
| L3 | 程序 | "如何做" 的步骤 |
| L4 | 情感 | 用户偏好 / 情绪 |
| L5 | 元认知 | 反思 (Reflection) |
| L6 | 概念 | 跨任务抽象 |
| L7 | 自传 | 长期主线 |

* **手动反思**：⌘K → "Trigger reflection now"
* **自动反思**：后台 worker 每 N 秒跑一次

---

## 5. 蜂群 (Swarm)

蜂群由 3 个 agent 协作：

| Agent | 强项 |
| ----- | ---- |
| coder | 写代码 |
| writer | 写文档 |
| reviewer | 审查 |

触发方式：进入 *蜂群* 视图 → 输入任务描述 → 运行。

---

## 6. 命令面板

按 **⌘K** (macOS) / **Ctrl+K** (Windows/Linux) 唤起。输入关键词搜索：

* 视图跳转
* 子模式切换
* 触发反思
* 打开设置
* 最近的记忆

键盘导航：↑/↓ 选择，Enter 执行，Esc 关闭。

---

## 7. 设置

点击左栏底部的 ⚙️：

* 主题（深 / 浅 / 跟随系统）
* 主色（颜色选择器）
* 字体大小、自动保存间隔
* Ollama URL
* API Key（仅本机，不上传）
* 工作区
* 语言

---

## 8. E2EE 同步

九头蛇之间的同步使用 X25519 + AES-256-GCM：

1. 双方各生成一对密钥
2. 交换公钥（通过任意通道）
3. 发送方用对方公钥加密，落地到本地 inbox
4. 接收方用自己的私钥 + 对方公钥解密

详见 [docs/ARCHITECTURE.md §6](ARCHITECTURE.md)。

---

## 9. 常见问题

### 9.1 启动慢

* 检查是否首次启动（首次会跑迁移）
* 关掉杀毒软件对 `nine-snake.exe` 的实时扫描
* 把 `NINE_SNAKE_DB` 放到 SSD 上

### 9.2 Ollama 连不上

* `ollama serve` 启动了吗？
* 防火墙是否放行 `127.0.0.1:11434`？
* 远程 URL 是否需要 token？

### 9.3 内存占用高

* 默认预算 500 MB，状态栏会标红
* 关闭编辑器中的大文件
* 减少 `recentMemories` 数量

详见 [docs/TROUBLESHOOTING.md](TROUBLESHOOTING.md)。

---

## 10. 反馈

* [GitHub Issues](https://github.com/nine-snake/nine-snake/issues)
* 邮件：hello@nine-snake.app
