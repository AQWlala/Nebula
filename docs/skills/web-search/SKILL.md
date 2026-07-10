---
name: web-search
version: 1.0.0
description: |
  网页搜索摘要技能——执行网页搜索并对结果进行摘要。当用户要求"搜一下"、
  "查查这个"、"网上有没有关于..."时加载此技能。通过 net:http 能力发起
  搜索请求，再用 llm:call 能力对结果网页摘要，返回结构化答案。对标
  OpenAkita 搜索能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "net:http"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Web Search 技能（网页搜索摘要）

## 概述

Web Search 是 Nebula 的信息检索技能，将"搜索 → 抓取 → 摘要"三步合一。
用户用自然语言提问，技能自动构造搜索 query、发起 HTTP 请求获取搜索结果、
抓取 Top-N 结果网页正文、最后由 LLM 综合摘要为带引用的结构化答案。

不同于浏览器手动搜索，本技能的核心价值是**信息综合**——不是返回一堆链接，
而是阅读多个来源后给出整合答案，并标注每条结论的来源 URL，便于用户溯源
验证。

## 使用场景

- **事实查询**："Rust 2.0 发布了吗？" "Tauri 最新版本是多少？"
- **技术调研**："对比一下 React 和 Vue 2026 年的生态差异"
- **新闻追踪**："今天 AI 领域有什么大事？"
- **概念解释**："什么是 MCP 协议？有什么用？"
- **购物决策**："2026 年适合程序员用的机械键盘推荐"
- **问题排查**："这个报错 'E0693' 是什么意思怎么解决？"

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `query` | string | 是 | 自然语言搜索查询 |
| `max_results` | number | 否 | 抓取的结果网页数量，默认 5，最大 10 |
| `summary_style` | string | 否 | 摘要风格：`concise`（简洁）/ `detailed`（详细）/ `qa`（问答式） |
| `lang` | string | 否 | 搜索语言偏好：`zh` / `en` / `auto`（默认） |
| `time_range` | string | 否 | 时间范围：`day` / `week` / `month` / `year` / `all`（默认） |

示例输入：
```json
{
  "query": "Tauri 2.0 与 Electron 2026 年性能对比",
  "max_results": 5,
  "summary_style": "detailed",
  "lang": "zh",
  "time_range": "year"
}
```

## 输出

```json
{
  "output": {
    "query": "Tauri 2.0 与 Electron 2026 年性能对比",
    "answer": "## 综合结论\n根据多个来源对比，Tauri 2.0 在内存占用上...\n## 性能数据\n- Tauri: 内存 45MB...\n- Electron: 内存 180MB...",
    "sources": [
      {"title": "Tauri vs Electron Benchmark 2026", "url": "https://example.com/1", "snippet": "..."},
      {"title": "桌面框架性能横评", "url": "https://example.com/2", "snippet": "..."}
    ],
    "result_count": 5,
    "search_engine": "bing"
  },
  "error": null,
  "latency_ms": 8200
}
```

输出字段说明：
- `answer`：综合摘要答案（Markdown 格式，含结论与数据）
- `sources`：引用来源列表，每条含标题、URL、摘要片段
- `result_count`：实际抓取并阅读的网页数量

## 使用示例

### 示例 1：技术概念查询

用户："MCP 协议是什么？"

```json
{
  "query": "Model Context Protocol MCP 协议 介绍",
  "summary_style": "qa",
  "lang": "zh"
}
```

返回问答式摘要："MCP 是什么？有什么用？谁在用？"三段式结构。

### 示例 2：最新新闻追踪

用户："这周 AI Agent 领域有什么新动态？"

```json
{
  "query": "AI Agent 最新动态 本周",
  "max_results": 8,
  "summary_style": "concise",
  "time_range": "week"
}
```

返回本周 5-8 条重要动态的精简摘要，每条一句话 + 来源链接。

### 示例 3：报错排查

用户："Rust 编译报 'cannot borrow as mutable' 怎么解决？"

```json
{
  "query": "Rust cannot borrow as mutable 错误 解决方案",
  "summary_style": "detailed",
  "lang": "auto"
}
```

返回错误原因解释 + 常见修复方案 + 代码示例，附 Stack Overflow 等来源。

## 注意事项

- **网络依赖**：本技能依赖 `net:http` 能力发起网络请求。离线环境下
  技能将拒绝加载并提示用户检查网络连接。
- **SSRF 防护**：HTTP 请求经过 Nebula 的 SSRF 校验层，拒绝访问内网
  地址（127.0.0.1 / 10.x / 192.168.x 等）与非常规端口，防止服务端
  请求伪造攻击。
- **内容时效性**：搜索结果反映抓取时刻的网页状态，可能滞后于实时
  信息。对时效敏感的查询（股价、赛事比分）建议标注"截至 YYYY-MM-DD"。
- **来源可信度**：技能不对来源真伪做担保，摘要中会标注来源域名。
  涉及医疗、法律、金融等专业建议时，提示用户咨询专业人士。
- **隐私保护**：搜索 query 与抓取的网页内容不持久化，仅用于本次
  LLM 摘要。查询日志默认不记录，可在设置中开启用于调试。
- **速率限制**：为避免被搜索引擎封禁，默认每次搜索间隔不少于 1 秒，
  单次会话搜索次数上限 50 次。
- **反爬应对**：部分网站设有反爬机制，抓取可能失败。技能会跳过
  失败来源并在 `sources` 中标注"抓取失败"，继续处理其他结果。
