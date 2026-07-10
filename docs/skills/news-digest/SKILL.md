---
name: news-digest
version: 1.0.0
description: |
  新闻阅读总结技能——阅读并总结新闻资讯。当用户要求"今天有什么新闻"、
  "总结一下本周科技大事"、"看看 AI 领域最近发生了什么"时加载此技能。
  通过 net:http 能力获取新闻源，再用 llm:call 能力摘要综合，
  返回按重要性排序的新闻摘要。对标 CoPaw 新闻阅读能力。
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

# News Digest 技能（新闻阅读总结）

## 概述

News Digest 是 Nebula 的资讯获取技能，帮用户在信息过载时代高效获取
有价值的内容。它从多个新闻源抓取最新资讯，按主题聚类、按重要性排序、
按用户兴趣过滤，最终输出一份"5 分钟读完"的新闻摘要。

与 `web-search` 的区别：`web-search` 是"用户问一个问题，搜答案"，
`news-digest` 是"主动推送，按领域汇总最新动态"。前者是被动检索，
后者是主动聚合。

## 使用场景

- **晨间简报**：起床后快速了解昨夜今晨发生了什么
- **领域追踪**：持续关注 AI / 区块链 / 新能源等特定领域动态
- **竞品监控**：追踪竞争对手的产品发布与融资新闻
- **周报素材**：为周报/月报收集行业动态素材
- **热点跟进**：某个话题突然火了，快速了解来龙去脉
- **多源对比**：同一事件看多个媒体的报道角度

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `topic` | string | 否 | 新闻主题/领域，如"AI"、"科技"、"财经"（默认综合） |
| `time_range` | string | 否 | 时间范围：`today` / `week` / `month`（默认 today） |
| `max_items` | number | 否 | 最大新闻条数，默认 10 |
| `sources` | string[] | 否 | 指定新闻源（RSS URL 或源名称），不传则用默认源 |
| `language` | string | 否 | 语言偏好：`zh` / `en` / `both`（默认 zh） |
| `summary_style` | string | 否 | 摘要风格：`brief`（一句话）/ `standard`（一段话）/ `detailed`（多段） |

示例输入：
```json
{
  "topic": "AI Agent",
  "time_range": "week",
  "max_items": 8,
  "language": "zh",
  "summary_style": "standard"
}
```

## 输出

```json
{
  "output": {
    "topic": "AI Agent",
    "time_range": "week",
    "digest": "## 本周 AI Agent 领域动态\n\n### 1. OpenAI 发布 GPT-5 Agent 模式\nOpenAI 本周宣布...\n\n### 2. Anthropic Claude 支持工具调用\n...",
    "items": [
      {
        "rank": 1,
        "title": "OpenAI 发布 GPT-5 Agent 模式",
        "summary": "OpenAI 于 7 月 8 日宣布 GPT-5 新增 Agent 模式，支持自主规划与执行多步任务...",
        "source": "36Kr",
        "url": "https://example.com/news/1",
        "published": "2026-07-08",
        "importance": "high"
      },
      {
        "rank": 2,
        "title": "Anthropic Claude 支持工具调用",
        "summary": "...",
        "source": "TechCrunch",
        "url": "https://example.com/news/2",
        "published": "2026-07-09",
        "importance": "medium"
      }
    ],
    "sources_checked": 12,
    "items_returned": 8
  },
  "error": null,
  "latency_ms": 9500
}
```

输出字段说明：
- `digest`：综合摘要（Markdown 格式，按重要性排序）
- `items`：新闻条目列表，每条含排名、标题、摘要、来源、URL、重要级
- `importance`：重要级别 high / medium / low

## 使用示例

### 示例 1：今日综合新闻

用户："今天有什么新闻？"

```json
{
  "topic": "综合",
  "time_range": "today",
  "max_items": 10,
  "summary_style": "brief"
}
```

返回今日 10 条重要新闻，每条一句话摘要。

### 示例 2：本周 AI 领域

用户："这周 AI 领域有什么大事？"

```json
{
  "topic": "AI",
  "time_range": "week",
  "max_items": 8,
  "summary_style": "standard"
}
```

返回本周 AI 领域 8 条动态，每条一段话摘要，附来源链接。

### 示例 3：竞品监控

用户："看看这周竞品有什么动态"

```json
{
  "topic": "竞争对手公司名",
  "time_range": "week",
  "max_items": 5,
  "sources": ["36kr.com", "techcrunch.com"],
  "language": "both"
}
```

从指定源抓取竞品相关新闻，中英文双语汇总。

## 注意事项

- **新闻源依赖**：默认新闻源包含主流科技媒体 RSS（36Kr / 虎嗅 / 少数派 /
  TechCrunch / The Verge 等）。用户可在设置中自定义源列表。
- **网络依赖**：本技能依赖 `net:http` 能力发起网络请求。离线环境下
  技能拒绝加载并提示用户检查网络。
- **SSRF 防护**：HTTP 请求经过 SSRF 校验层，拒绝访问内网地址与非常规端口。
- **时效性说明**：新闻摘要反映抓取时刻的状态，可能与实时有延迟。
  `published` 字段为新闻原始发布时间，供用户判断时效。
- **立场中立**：技能对新闻内容做客观摘要，不添加主观评价。涉及争议性
  话题时，会标注"此事件存在不同观点"并引用多方来源。
- **版权尊重**：摘要为合理使用范围内的简短引用，不复制全文。每条
  附原文链接，鼓励用户访问原站阅读完整报道。
- **重要性排序**：`importance` 基于来源权威度、提及频次、时效性综合
  判定，仅供参考。用户可根据自身兴趣调整。
- **隐私保护**：用户的阅读偏好与查询记录不持久化，不上传至外部服务。
  除非用户显式开启"个性化推荐"并授权数据使用。
- **速率限制**：为避免被新闻源封禁，默认每次抓取间隔不少于 2 秒，
  单次会话抓取次数上限 30 次。
- **依赖说明**：需要 Python 环境用于 RSS 解析（feedparser）与网页正文
  提取（readability-lxml）。
