---
name: clipboard-manager
version: 1.0.0
description: |
  剪贴板智能监听与历史管理技能——监听剪贴板变化、记录历史、智能分类与搜索。
  当用户要求"我刚才复制了什么"、"找一下之前复制的那段代码"、"清空剪贴板历史"时加载此技能。
  通过剪贴板读写能力，构建可检索的剪贴板历史库。对标 CoPaw clipboard_manager 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "clipboard:read", "clipboard:write"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Clipboard Manager 技能（剪贴板智能监听+历史）

## 概述

Clipboard Manager 是 Nebula 的剪贴板增强技能，负责监听系统剪贴板变化、
记录历史条目、按内容类型智能分类、并支持全文检索。通过 `clipboard:read`
轮询剪贴板内容、`clipboard:write` 回写指定条目，结合 `llm:call` 对文本
进行语义分类与摘要。

该技能突破系统剪贴板"只记一条"的限制，让用户随时回溯之前复制过的代码片段、
链接、文字、图片路径。所有历史存储在本地 SQLite 数据库，不上传任何内容。

## 使用场景

- **找回历史复制内容**：刚才复制了一段代码然后被覆盖了，想找回来
- **代码片段管理**：复制过多段代码片段，按语言/项目分类管理
- **链接收藏**：浏览时复制的链接自动归类到"链接"分组，便于后续查阅
- **快速粘贴模板**：把常用回复/签名存为模板，一键写入剪贴板
- **敏感内容识别**：复制密码/令牌时自动标记并提示及时清空

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | string | 是 | 操作类型：`search` / `list` / `write` / `clear` / `start_listen` / `stop_listen` |
| `query` | string | 否 | `action=search` 时的搜索关键词 |
| `category` | string | 否 | 按分类筛选：`code` / `url` / `text` / `image` / `all` |
| `limit` | number | 否 | `action=list` 时返回的条目数，默认 20 |
| `content` | string | 否 | `action=write` 时要写入剪贴板的内容 |
| `entry_id` | string | 否 | 写入/删除特定历史条目的 ID |
| `time_range` | string | 否 | 时间范围筛选，如 `1h` / `today` / `7d` |

示例输入：
```json
{
  "action": "search",
  "query": "async function",
  "category": "code",
  "time_range": "today"
}
```

## 输出

```json
{
  "output": {
    "action": "search",
    "results": [
      {
        "id": "clip-2026-0710-001",
        "content": "async function fetchData(url) {\n  return await fetch(url);\n}",
        "category": "code",
        "language": "javascript",
        "source_app": "VS Code",
        "copied_at": "2026-07-10T14:23:11+08:00",
        "char_count": 56
      }
    ],
    "total": 1
  },
  "error": null,
  "latency_ms": 120
}
```

输出字段说明：
- `results`：匹配的历史条目列表，按时间倒序
- `category`：自动识别的内容分类（code/url/text/image）
- `source_app`：复制来源应用（如可识别）
- `copied_at`：复制时间戳（ISO 8601）

## 使用示例

### 示例 1：搜索今天复制的代码片段

用户："我之前复制过一个 async 函数，找一下"

```json
{
  "action": "search",
  "query": "async function",
  "category": "code",
  "time_range": "today"
}
```

技能在历史库中检索含关键词的代码条目并返回匹配结果。

### 示例 2：列出最近剪贴板历史

用户："看看我最近都复制了什么"

```json
{
  "action": "list",
  "limit": 10
}
```

返回最近 10 条剪贴板记录，按时间倒序排列，包含内容预览与分类标签。

### 示例 3：把历史条目写回剪贴板

用户："把刚才那段代码再放回剪贴板"

```json
{
  "action": "write",
  "entry_id": "clip-2026-0710-001"
}
```

根据 ID 从历史库取出内容，通过 `clipboard:write` 写入系统剪贴板。

## 注意事项

- **隐私优先**：所有历史仅存储在本地 SQLite，进程退出后保留；不上传任何内容
  至云端或外部 LLM。语义分类在本地完成或仅传递摘要。
- **敏感内容检测**：识别到密码模式（如 `password=`、`token=`、长随机串）
  时自动标记 `sensitive`，并在 60 秒后建议清空。
- **图片剪贴板**：图片以文件路径形式存储（`%TEMP%/nebula-clip/<hash>.png`），
  不直接入库二进制，避免数据库膨胀。原图保留 24 小时后自动清理。
- **监听频率**：默认每 500ms 轮询一次剪贴板，CPU 占用 < 1%。可配置更高频率
  但不建议低于 200ms。
- **去重策略**：连续复制相同内容不重复入库；仅前后空白差异视为同一条。
- **跨平台**：Windows 使用 `win32clipboard`、macOS 使用 `pbpaste`、
  Linux 使用 `xclip`/`xsel`，需 Python 环境统一封装。
- **存储上限**：默认保留最近 1000 条，超出后按 FIFO 淘汰最旧条目。
