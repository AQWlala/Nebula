---
name: summary-generator
version: 1.0.0
description: |
  长文摘要生成技能——将长篇文章/论文/报告压缩为结构化摘要。当用户要求
  "总结这篇长文"、"压缩这篇论文"、"生成摘要"时加载此技能。通过 file:read
  读取原文、llm:call 生成多层级结构化摘要。对标 Hermes 摘要能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "file:read"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Summary Generator 技能（长文摘要生成）

## 概述

Summary Generator 是 Nebula 的长文压缩技能，专为处理超长文本（论文 /
报告 / 长文章 / 技术规范）设计。它通过 `file:read` 读取原文，经分段
切块后由 `llm:call` 逐段摘要，再综合为多层级结构化摘要——既有 30 字
的一句话总结，也有分章节的详细摘要，满足"快速浏览"与"深入理解"两种
需求。

与 file-reader 技能的区别：file-reader 侧重多格式文档（PDF / docx）
的读取与轻量摘要；本技能专注于**长文本的深度压缩**，支持分层摘要、
关键论点提取、术语高亮，适合万字以上的论文与报告。

## 使用场景

- **论文速读**：万字学术论文 30 秒内把握核心论点与方法
- **报告压缩**：将百页行业报告压缩为一页纸执行摘要
- **技术规范摘要**：从冗长技术规范中提取关键约束与接口定义
- **长文速览**：在收藏的长文章中快速定位值得精读的部分
- **会议材料预处理**：会前将多份长报告压缩为摘要带进会议室
- **文献综述辅助**：批量摘要多篇论文，辅助文献综述写作

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 原文文件路径，支持 .txt / .md / .pdf / .docx |
| `summary_type` | string | 否 | 摘要类型：`abstract`（默认）/ `bullet` / `structured` / `executive` |
| `max_words` | number | 否 | 摘要最大字数，默认 800 |
| `focus` | string | 否 | 摘要关注焦点，如"研究方法"、"结论"、"技术方案" |
| `levels` | boolean | 否 | 是否生成多层级摘要（一句话 + 段落 + 章节），默认 `false` |
| `extract_keywords` | boolean | 否 | 是否提取关键词，默认 `true` |
| `output_format` | string | 否 | 输出格式：`markdown`（默认）/ `plain` / `json` |

示例输入：
```json
{
  "path": "D:/papers/llm-agent-survey-2026.pdf",
  "summary_type": "structured",
  "max_words": 1200,
  "focus": "Agent 架构演进与工具调用机制",
  "levels": true,
  "extract_keywords": true,
  "output_format": "markdown"
}
```

## 输出

```json
{
  "output": {
    "file": "llm-agent-survey-2026.pdf",
    "pages": 38,
    "word_count": 18500,
    "one_line": "本文综述了 2023-2026 年 LLM Agent 架构从单轮调用到多智能体协作的演进路径，指出工具调用规范化是当前规模化落地的关键瓶颈。",
    "summary": "## 研究背景\nLLM Agent 经历了...\n## 核心方法\n本文从架构、工具调用、记忆机制三维度...\n## 主要结论\n多智能体协作架构在复杂任务上表现更优...",
    "section_summaries": [
      {"section": "2. Agent 架构演进", "summary": "从 ReAct 到 Plan-and-Execute，架构逐步分离规划与执行..."},
      {"section": "3. 工具调用机制", "summary": "Function Calling 与 MCP 协议成为主流，标准化程度提升..."}
    ],
    "key_points": [
      "工具调用规范化是规模化落地的关键瓶颈",
      "多智能体协作在复杂任务上优于单 Agent",
      "记忆机制的长程一致性仍是开放问题"
    ],
    "keywords": ["LLM Agent", "工具调用", "MCP", "多智能体", "ReAct", "记忆机制"],
    "reading_time_min": 65,
    "summary_reading_time_min": 3
  },
  "error": null,
  "latency_ms": 12500
}
```

输出字段说明：
- `one_line`：一句话总结（`levels=true` 时输出）
- `summary`：主体摘要，按 `summary_type` 组织结构
- `section_summaries`：分章节摘要（`levels=true` 时输出）
- `key_points`：核心论点列表
- `keywords`：抽取的高频 / 关键术语
- `summary_reading_time_min`：摘要预估阅读时长

## 使用示例

### 示例 1：论文结构化摘要

用户："总结一下这篇 LLM Agent 综述论文"

```json
{
  "path": "D:/papers/llm-agent-survey-2026.pdf",
  "summary_type": "structured",
  "max_words": 1000,
  "levels": true,
  "extract_keywords": true
}
```

生成含一句话总结、分章节摘要、核心论点与关键词的结构化摘要，帮助
用户快速判断论文是否值得精读。

### 示例 2：行业报告执行摘要

用户："把这份 200 页的行业报告压成一页纸"

```json
{
  "path": "D:/reports/ai-industry-2026.pdf",
  "summary_type": "executive",
  "max_words": 600,
  "focus": "市场规模、竞争格局、投资机会",
  "levels": false
}
```

生成面向决策者的执行摘要，突出关键数据与结论，弱化技术细节。

### 示例 3：聚焦特定主题的长文摘要

用户："这篇文章很长，我只关心它的实验方法部分"

```json
{
  "path": "D:/articles/research-methods.md",
  "summary_type": "bullet",
  "focus": "实验设计与数据采集方法",
  "max_words": 500
}
```

以要点列表形式提取与"实验方法"相关的内容，弱化无关章节。

## 注意事项

- **长文本处理**：单文件超过 5 万字时采用分段摘要再综合的策略，可能产生
  轻微的信息损失。超过 20 万字时建议按章节拆分后分别摘要。
- **PDF 扫描件**：纯图片型扫描 PDF（无文本层）无法直接摘要，需先经 OCR
  处理。技能会检测并提示用户使用 pdf-extractor 技能。
- **摘要保真度**：摘要不可避免存在信息压缩损失，关键数据与结论建议用户
  对照原文核实。技能会对不确定的内容标注"[建议核实]"。
- **焦点优先**：指定 `focus` 时会优先提取相关内容，可能弱化其他章节。
  需要全面摘要时请勿指定 `focus`。
- **与 file-reader 的关系**：file-reader 适合短文档快速摘要；本技能适合
  万字以上长文的深度压缩。两者可组合使用——先用本技能压缩，再用
  file-reader 处理重点章节。
- **隐私保护**：原文内容仅在本地 LLM 调用中处理，摘要不持久化到磁盘
  （除非用户显式要求保存），不上传外部服务。
- **依赖说明**：需要 Python 环境用于解析 `.pdf`（pdfplumber）与 `.docx`
  （python-docx）。`.txt` / `.md` 由 Nebula 原生读取。
