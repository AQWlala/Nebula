---
name: article-writer
version: 1.0.0
description: |
  文章撰写技能——撰写自媒体 / 博客文章。当用户要求"写一篇文章"、
  "帮我写个推文"、"出一篇技术博客"时加载此技能。通过 llm:call 能力
  按指定主题、平台风格、目标读者生成完整文章，并经 file:write 能力
  落盘。对标 Hermes 内容创作能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "file:write"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Article Writer 技能（文章撰写）

## 概述

Article Writer 是 Nebula 的内容创作技能，帮助用户从一句主题描述生成
一篇完整的可发布文章。它理解不同平台的风格差异——技术博客重深度与代码，
公众号重标题与节奏，知乎重论证与引用，小红书重情绪与 emoji——并按
对应风格输出。

技能流程：理解主题 → 确定角度 → 生成大纲 → 撰写正文 → 润色标题 →
落盘保存。每一步都可通过参数调控，既支持"一键生成"也支持"逐节精修"。

## 使用场景

- **技术博客**：把刚解决的一个技术问题写成可发布的博客文章
- **公众号推文**：将产品更新整理成适合微信阅读的推文
- **知乎回答**：针对某个问题撰写有论据支撑的长回答
- **小红书笔记**：生成带 emoji 与话题标签的短笔记
- **Newsletter**：定期撰写给订阅者的周报/月报
- **SEO 文章**：围绕关键词生成搜索引擎友好的内容
- **转载改写**：将英文文章改写为中文，或调整风格适配不同平台

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 输出文件路径（.md / .html / .txt） |
| `topic` | string | 是 | 文章主题或核心观点 |
| `platform` | string | 否 | 目标平台：`blog` / `wechat` / `zhihu` / `xiaohongshu` / `newsletter` |
| `audience` | string | 否 | 目标读者，如"前端开发者"、"产品经理"、"普通用户" |
| `word_count` | number | 否 | 目标字数，默认 1500 |
| `tone` | string | 否 | 语气：`professional` / `friendly` / `passionate` / `objective` |
| `outline` | string[] | 否 | 自定义大纲，不传则自动生成 |
| `keywords` | string[] | 否 | SEO 关键词列表 |

示例输入：
```json
{
  "path": "D:/blog/tauri-vs-electron-2026.md",
  "topic": "2026 年 Tauri 与 Electron 的性能与生态对比",
  "platform": "blog",
  "audience": "桌面应用开发者",
  "word_count": 3000,
  "tone": "professional",
  "keywords": ["Tauri", "Electron", "桌面开发", "性能对比"]
}
```

## 输出

```json
{
  "output": {
    "path": "D:/blog/tauri-vs-electron-2026.md",
    "title": "Tauri vs Electron：2026 年了，谁才是桌面应用的最优解？",
    "word_count": 2980,
    "sections": ["引言", "架构差异", "性能实测", "生态对比", "选型建议"],
    "platform": "blog",
    "reading_time_min": 12,
    "bytes_written": 18900
  },
  "error": null,
  "latency_ms": 12000
}
```

输出字段说明：
- `title`：生成的文章标题（已润色，含平台适配）
- `sections`：文章章节列表
- `reading_time_min`：预估阅读时长
- `bytes_written`：写入磁盘的字节数

## 使用示例

### 示例 1：技术博客

用户："写一篇关于 Nebula 架构设计的技术博客"

```json
{
  "path": "D:/blog/nebula-architecture.md",
  "topic": "Nebula 双主控 + 蜂群 worker 架构设计思路与实践",
  "platform": "blog",
  "audience": "AI Agent 架构师与 Rust 开发者",
  "word_count": 4000,
  "tone": "professional"
}
```

生成深度技术文章，含架构图说明、设计权衡、代码片段。

### 示例 2：公众号推文

用户："把这次产品更新写成公众号推文"

```json
{
  "path": "D:/wechat/update-2026-07.md",
  "topic": "Nebula 2.1 新增技能市场与蜂群编排",
  "platform": "wechat",
  "audience": "Nebula 用户与 AI 爱好者",
  "word_count": 2000,
  "tone": "friendly"
}
```

生成适合微信阅读的推文，短段落、小标题、引导关注结尾。

### 示例 3：小红书笔记

用户："写个小红书笔记推荐这个 AI 工具"

```json
{
  "path": "D:/xiaohongshu/ai-tool-recommend.md",
  "topic": "推荐 Nebula 这个免费的桌面 AI Agent",
  "platform": "xiaohongshu",
  "audience": "效率工具爱好者",
  "word_count": 500,
  "tone": "passionate"
}
```

生成短笔记，含 emoji、话题标签、个人体验口吻。

## 注意事项

- **原创性声明**：生成的文章为 AI 辅助创作，建议用户发布时标注"AI 辅助
  创作"并做人工审校。技能不对内容的版权合规性承担责任。
- **事实核查**：LLM 生成的内容可能包含不准确的数据或过时信息。涉及具体
  数据、引用、案例时，技能会标注"[需核实]"提示用户验证。
- **平台规范**：各平台有内容审核规范，技能遵循 ValuesLayer 约束不生成
  违规内容。但平台规则可能变化，发布前请用户确认合规。
- **写作风格**：技能尽量匹配目标平台风格，但无法完全模仿个人写作风格。
  建议将生成的文章作为初稿，再由用户润色加入个人特色。
- **SEO 合规**：`keywords` 参数用于自然融入关键词，不堆砌。过度优化
  可能被搜索引擎惩罚，技能遵循合理的关键词密度（1%-3%）。
- **大文章分章**：目标字数超过 3000 字时，技能自动分章节生成并逐节
  写入，避免单次 LLM 调用质量下降。
- **版权尊重**：`转载改写`场景下，技能要求用户提供原文来源 URL，并在
  生成文章中标注原作者与原文链接。拒绝洗稿请求。
