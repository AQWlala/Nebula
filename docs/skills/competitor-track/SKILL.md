---
name: competitor-track
version: 1.0.0
description: |
  竞品动态追踪技能——监控竞品官网、博客、GitHub 仓库更新，定期生成竞品动态报告。
  当用户要求"看看竞品最近有什么更新"、"追踪一下 XX 的新版本"时加载此技能。
  通过 HTTP 抓取 + 文件写入能力，构建可追踪的竞品情报库。对标 OpenAkita competitor_track 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "net:http", "file:write"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Competitor Track 技能（竞品动态追踪）

## 概述

Competitor Track 是 Nebula 的竞品情报追踪技能，定期监控竞品的官网、博客、
GitHub Release、Changelog 等渠道，发现新版本发布、功能更新、定价调整等
动态，并生成结构化的竞品情报报告。通过 `net:http` 抓取目标页面、
`file:write` 持久化报告、`llm:call` 进行变更识别与摘要。

该技能面向产品经理、市场分析师、创业者：用自动化代替手动巡查，每周自动
汇总竞品动态，避免遗漏关键信号。

## 使用场景

- **新版本监控**：竞品发布新版本时第一时间获知功能变化
- **定价策略追踪**：监控竞品定价页面的调整
- **GitHub 活跃度**：跟踪竞品开源仓库的 Release、Issue、Star 增长
- **博客内容**：竞品发布技术博客时的要点提取
- **对比报告**：把多家竞品动态汇总为一份周报/月报

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `competitors` | object[] | 是 | 竞品列表，每项含 `name` 与 `sources` |
| `sources` | string[] | 否 | 监控源 URL 列表（官网/博客/GitHub Release） |
| `since` | string | 否 | 起始日期，默认 `7d`（最近 7 天） |
| `report_format` | string | 否 | 报告格式：`markdown`（默认）/ `json` / `html` |
| `output_path` | string | 否 | 报告保存路径 |
| `diff_mode` | boolean | 否 | 是否与上次抓取内容做 diff，默认 `true` |
| `notify` | boolean | 否 | 有新动态时是否推送通知，默认 `false` |

示例输入：
```json
{
  "competitors": [
    {
      "name": "Cursor",
      "sources": [
        "https://cursor.com/changelog",
        "https://github.com/getcursor/cursor/releases"
      ]
    },
    {
      "name": "Windsurf",
      "sources": ["https://windsurf.com/blog"]
    }
  ],
  "since": "7d",
  "report_format": "markdown",
  "output_path": "D:/reports/competitor-2026-07-10.md",
  "diff_mode": true
}
```

## 输出

```json
{
  "output": {
    "report_path": "D:/reports/competitor-2026-07-10.md",
    "tracked_at": "2026-07-10T15:00:00+08:00",
    "competitors_count": 2,
    "changes_detected": 3,
    "competitors": [
      {
        "name": "Cursor",
        "changes": [
          {
            "source": "https://cursor.com/changelog",
            "type": "release",
            "title": "v0.42 - Background Agents",
            "summary": "新增后台 Agent 能力，支持长任务异步执行",
            "detected_at": "2026-07-09T08:00:00+08:00",
            "url": "https://cursor.com/changelog#v0-42"
          }
        ]
      }
    ],
    "highlights": [
      "Cursor 发布 Background Agents 功能，与本品路线图冲突，建议优先评估",
      "Windsurf 博客发布企业版定价调整"
    ]
  },
  "error": null,
  "latency_ms": 9200
}
```

输出字段说明：
- `changes_detected`：本次发现的总变更数
- `competitors[].changes`：每个竞品的变更列表
- `highlights`：LLM 提炼的关键动态要点，按重要性排序

## 使用示例

### 示例 1：追踪竞品周报

用户："帮我看看这周 Cursor 和 Windsurf 都更新了什么"

```json
{
  "competitors": [
    {"name": "Cursor", "sources": ["https://cursor.com/changelog"]},
    {"name": "Windsurf", "sources": ["https://windsurf.com/blog"]}
  ],
  "since": "7d",
  "report_format": "markdown"
}
```

抓取各竞品源页面，与上次缓存内容做 diff，识别出的变更生成 Markdown 周报。

### 示例 2：监控 GitHub Release

用户："追踪 LangChain 的 Release 动态"

```json
{
  "competitors": [
    {
      "name": "LangChain",
      "sources": ["https://github.com/langchain-ai/langchain/releases"]
    }
  ],
  "since": "14d",
  "diff_mode": true
}
```

抓取 GitHub Release 页面，提取版本号、Release Note、发布时间。

### 示例 3：定时任务生成月报

配合 Nebula 定时任务，每月 1 日自动运行：

```json
{
  "competitors": [...],
  "since": "30d",
  "report_format": "markdown",
  "output_path": "D:/reports/competitor-monthly-{date}.md",
  "notify": true
}
```

生成月度竞品情报报告并保存，有重大变更时推送桌面通知。

## 注意事项

- **抓取频率**：建议每周一次，避免高频请求被目标站点封禁。技能内置
  请求间隔（2-5 秒）与 User-Agent 轮换。
- **Diff 缓存**：上次抓取的页面内容缓存在 `.nebula/competitor-cache/`，
  用于变更比对。缓存保留 90 天后自动清理。
- **页面解析**：依赖页面结构稳定性，若竞品改版可能导致解析失败。
  技能会在解析失败时返回原始 HTML 片段供人工排查。
- **GitHub API 限制**：未认证的 GitHub API 每小时 60 次请求。
  建议在配置中设置 `GITHUB_TOKEN` 提升至 5000 次/小时。
- **报告归档**：每次生成的报告独立保存，文件名含日期戳，便于历史回溯。
- **依赖说明**：需要 Python 环境用于 HTTP 请求（httpx）、HTML 解析
  （selectolax）与 Markdown 渲染（Jinja2）。
