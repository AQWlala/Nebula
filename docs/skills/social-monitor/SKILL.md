---
name: social-monitor
version: 1.0.0
description: |
  社交平台热门内容爬取整理技能——抓取微博/知乎/小红书/X 等平台热门内容并分类整理。
  当用户要求"看看今天微博热搜"、"整理一下 X 上关于 AI 的热门讨论"时加载此技能。
  通过 HTTP 能力抓取内容，再经 LLM 分类摘要。对标 CoPaw social_monitor 能力。
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

# Social Monitor 技能（社交平台热门内容爬取整理）

## 概述

Social Monitor 是 Nebula 的社交内容监控技能，负责从公开网页或平台 API
抓取热门内容（热搜、话题、帖子），按主题/情感/热度分类整理，生成结构化
摘要报告。通过 `net:http` 能力发起请求，结合 `llm:call` 进行内容分类、
去重、摘要。

该技能面向运营、市场、研究者：每天花 5 分钟了解多平台热点，而非逐个 App
刷屏。所有抓取仅针对公开内容，不涉及账号登录与隐私数据。

## 使用场景

- **多平台热点扫描**：一次看遍微博/知乎/X 今日热点
- **话题追踪**：持续追踪"AI Agent"等关键词在各平台的讨论
- **竞品舆情**：监控品牌/产品名在社交平台的提及
- **内容选题**：从热门讨论中提取灵感，生成内容选题清单
- **情感分析**：对某话题下的帖子做正负面情感汇总

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `platforms` | string[] | 是 | 平台列表：`weibo` / `zhihu` / `xiaohongshu` / `x` / `hackernews` |
| `action` | string | 否 | 操作：`hot`（默认，热门）/ `keyword`（关键词搜索）/ `topic`（话题） |
| `keyword` | string | 否 | `action=keyword` 时的搜索词 |
| `topic` | string | 否 | `action=topic` 时的话题标签 |
| `time_range` | string | 否 | 时间范围：`today`（默认）/ `24h` / `7d` |
| `limit_per_platform` | number | 否 | 每平台返回条目数，默认 20 |
| `summarize` | boolean | 否 | 是否生成汇总摘要，默认 `true` |
| `output_path` | string | 否 | 报告保存路径，默认不保存文件 |

示例输入：
```json
{
  "platforms": ["weibo", "zhihu", "x"],
  "action": "keyword",
  "keyword": "AI Agent",
  "time_range": "24h",
  "limit_per_platform": 15,
  "summarize": true
}
```

## 输出

```json
{
  "output": {
    "fetched_at": "2026-07-10T15:00:00+08:00",
    "platforms_covered": ["weibo", "zhihu", "x"],
    "total_items": 45,
    "items": [
      {
        "platform": "weibo",
        "title": "AI Agent 桌面助手会成为下一个入口吗？",
        "url": "https://weibo.com/xxx/123",
        "hot_score": 9821,
        "author": "科技博主老王",
        "published_at": "2026-07-10T12:30:00+08:00",
        "snippet": "最近 Nebula 这类桌面 Agent 火了……",
        "sentiment": "positive"
      }
    ],
    "summary": {
      "key_topics": ["桌面 AI Agent", "Tauri 生态", "本地优先"],
      "sentiment_distribution": {"positive": 60, "neutral": 30, "negative": 10},
      "top_threads": ["微博：AI Agent 桌面助手会成为下一个入口吗？", "X: Nebula vs ChatGPT Desktop comparison"]
    }
  },
  "error": null,
  "latency_ms": 6800
}
```

输出字段说明：
- `items`：抓取到的内容条目，含标题、链接、热度、摘要、情感
- `summary`：跨平台汇总摘要，含关键话题、情感分布、热门讨论线索
- `sentiment`：单条内容的情感标签（positive/neutral/negative）

## 使用示例

### 示例 1：扫描多平台今日热点

用户："看看今天微博、知乎、X 上都热什么"

```json
{
  "platforms": ["weibo", "zhihu", "x"],
  "action": "hot",
  "time_range": "today",
  "limit_per_platform": 20,
  "summarize": true
}
```

抓取三个平台今日热门内容，生成跨平台热点汇总报告。

### 示例 2：追踪关键词讨论

用户："看看这周 X 上关于 AI Agent 的讨论"

```json
{
  "platforms": ["x"],
  "action": "keyword",
  "keyword": "AI Agent",
  "time_range": "7d",
  "limit_per_platform": 30
}
```

按关键词搜索 X 上的相关帖子，按热度排序返回。

### 示例 3：保存报告到文件

用户："把今天的舆情扫描结果存下来"

```json
{
  "platforms": ["weibo", "xiaohongshu"],
  "action": "keyword",
  "keyword": "Nebula",
  "time_range": "today",
  "output_path": "D:/reports/social-2026-07-10.md"
}
```

抓取后生成 Markdown 报告并保存到指定路径。

## 注意事项

- **公开数据**：仅抓取平台公开可见内容，不涉及登录态、私密消息、关注流。
  需登录的平台（如 X 高级搜索）需用户在设置中配置 API Key 或 Cookie。
- **频率限制**：默认对每平台每分钟最多发起 10 次请求，避免触发反爬。
  抓取间隔加入随机延迟（1-3 秒）。
- **合规性**：抓取行为遵循各平台 robots.txt 与 ToS。仅供个人研究使用，
  不可用于商业数据倒卖。生成的摘要须标注原始链接。
- **数据时效**：热门榜单数据可能延迟 5-15 分钟；历史帖子的精确发布时间
  依赖平台 API 返回。
- **情感分析**：情感标签由 LLM 推断，中文内容准确率约 85%。涉及反讽、
  梗文化的帖子可能误判，建议人工复核关键结论。
- **依赖说明**：需要 Python 环境用于 HTTP 请求（httpx）与 HTML 解析
  （selectolax/beautifulsoup4）。部分平台通过官方 API 调用。
