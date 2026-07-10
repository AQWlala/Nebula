---
name: file-reader
version: 1.0.0
description: |
  文档读取摘要技能——读取并摘要 .txt / .md / .pdf / .docx 等常见文档格式。
  当用户要求"读一下这个文件"、"总结这份文档"、"提取文档要点"时加载此技能。
  通过文件读取 + LLM 摘要能力，将长文档压缩为结构化要点，支持指定摘要长度
  与关注焦点。对标 CoPaw file_reader 能力。
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

# File Reader 技能（文档读取摘要）

## 概述

File Reader 是 Nebula 的基础文档处理技能，负责读取本地磁盘上的文本类文档
并生成结构化摘要。支持 `.txt`、`.md`、`.pdf`、`.docx` 四种主流格式，通过
`file:read` 能力安全读取文件内容，再经 `llm:call` 能力调用大模型生成摘要。

该技能是知识工作者日常高频场景的入口：快速理解一份未读文档的核心内容，
而不必逐页通读。摘要输出遵循"要点 + 细节"两层结构，便于用户决定是否深入。

## 使用场景

- **快速预览**：收到一份长 PDF 报告，需要 30 秒内了解它讲什么
- **会议准备**：会前读取相关文档，生成一页纸摘要带进会议室
- **资料筛选**：批量面对多份文档时，先摘要再决定哪些精读
- **知识归档**：将散落的文档提炼为可检索的要点笔记
- **跨格式统一**：无论原文是 txt / md / pdf / docx，输出格式一致

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 文档绝对路径，支持 .txt / .md / .pdf / .docx |
| `focus` | string | 否 | 摘要关注焦点，如"财务数据"、"技术方案"、"风险点" |
| `max_words` | number | 否 | 摘要最大字数，默认 500 |
| `format` | string | 否 | 输出格式：`markdown`（默认）/ `plain` / `json` |

示例输入：
```json
{
  "path": "D:/docs/report-2026-q2.pdf",
  "focus": "营收增长与成本结构",
  "max_words": 800,
  "format": "markdown"
}
```

## 输出

```json
{
  "output": {
    "file": "report-2026-q2.pdf",
    "format": "pdf",
    "pages": 42,
    "summary": "## 核心要点\n- Q2 营收同比增长 18%...\n## 关键数据\n...",
    "keywords": ["营收", "成本", "Q2", "增长"],
    "reading_time_min": 15
  },
  "error": null,
  "latency_ms": 3200
}
```

输出字段说明：
- `summary`：结构化摘要（Markdown 格式，含"核心要点"与"关键数据"小节）
- `keywords`：从文档中抽取的高频关键词
- `reading_time_min`：原文预估阅读时长（分钟）

## 使用示例

### 示例 1：读取 PDF 季度报告

用户："帮我看看这份 Q2 报告主要讲了什么"

```json
{
  "path": "D:/docs/report-2026-q2.pdf",
  "max_words": 500
}
```

输出摘要将包含报告主题、核心结论、关键数据三部分，帮助用户快速把握全貌。

### 示例 2：聚焦特定主题读取 docx

用户："读一下这份合同，重点看违约条款"

```json
{
  "path": "D:/contracts/service-agreement.docx",
  "focus": "违约责任与赔偿条款",
  "format": "plain"
}
```

技能将优先提取与"违约"相关段落并摘要，弱化无关章节。

### 示例 3：批量摘要 Markdown 笔记

用户："把这个文件夹里的 md 笔记都总结一下"

可循环调用本技能，每次传入一个 `.md` 文件路径，汇总各份摘要。

## 注意事项

- **路径安全**：仅允许读取用户工作区与明确授权目录下的文件，拒绝读取系统
  敏感路径（如 `C:/Windows/System32`）。路径校验由 `file:read` 能力强制。
- **大文件限制**：单文件超过 50 MB 时将分块读取并提示用户可能耗时较长；
  超过 200 MB 时拒绝处理，建议先拆分。
- **PDF 扫描件**：纯图片型扫描 PDF（无文本层）无法直接摘要，需先经 OCR
  处理。技能会检测并提示用户使用 `pdf-extractor` 或外部 OCR 工具。
- **编码识别**：`.txt` 文件自动检测 UTF-8 / GBK / GB2312 编码，避免乱码。
- **隐私保护**：文档内容仅在本地 LLM 调用中处理，不会被持久化到磁盘或
  上传至外部服务（除非用户显式配置远程 LLM 端点）。
- **依赖说明**：需要 Python 环境用于解析 `.pdf`（pdfplumber）与 `.docx`
  （python-docx）格式。`.txt` / `.md` 由 Nebula 原生读取，不依赖 Python。
