---
name: doc-writer
version: 1.0.0
description: |
  文档创建编辑技能——创建和编辑 Markdown / HTML 文档。当用户要求"写一份
  文档"、"生成 README"、"整理成 md"、"做个 HTML 页面"时加载此技能。通过
  LLM 生成初稿 + 文件写入能力落盘，支持模板套用、章节续写、格式美化。
  对标 CoPaw doc_writer 能力。
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

# Doc Writer 技能（文档创建编辑）

## 概述

Doc Writer 是 Nebula 的文档生产力技能，负责将用户的自然语言需求转化为
结构化的 Markdown 或 HTML 文档。它不只是"生成文字"，而是完整地完成
"构思大纲 → 撰写正文 → 格式化 → 落盘保存"的端到端流程。

通过 `llm:call` 能力生成符合文体规范的内容，再经 `file:write` 能力安全
写入指定路径。支持技术文档、产品 PRD、README、API 文档、HTML 报告等
多种文档类型，每种类型有对应的结构模板与语气规范。

## 使用场景

- **项目文档**：为新项目生成 README.md，包含介绍、安装、使用、贡献指南
- **技术规格**：将口述需求整理为结构化 PRD 或技术设计文档
- **API 文档**：根据代码注释或接口定义生成 API 参考文档
- **HTML 报告**：将数据分析结果输出为可分享的 HTML 报告页面
- **文档续写**：在已有文档基础上补充章节或扩写内容
- **格式转换**：把散乱的纯文本笔记整理为规范的 Markdown 结构

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 输出文件路径，扩展名决定格式（.md / .html） |
| `topic` | string | 是 | 文档主题或需求描述 |
| `doc_type` | string | 否 | 文档类型：`readme` / `prd` / `api` / `report` / `generic` |
| `outline` | string[] | 否 | 自定义大纲（章节标题列表），不传则自动生成 |
| `tone` | string | 否 | 语气：`formal`（正式）/ `casual`（随和）/ `technical`（技术） |
| `overwrite` | boolean | 否 | 已存在文件是否覆盖，默认 false（追加/续写模式） |

示例输入：
```json
{
  "path": "D:/projects/nebula/README.md",
  "topic": "Nebula 桌面 AI Agent 项目介绍",
  "doc_type": "readme",
  "tone": "technical"
}
```

## 输出

```json
{
  "output": {
    "path": "D:/projects/nebula/README.md",
    "format": "markdown",
    "sections": ["项目简介", "核心特性", "快速开始", "架构概览", "贡献指南"],
    "word_count": 1850,
    "bytes_written": 12480
  },
  "error": null,
  "latency_ms": 5400
}
```

输出字段说明：
- `sections`：生成的文档章节列表
- `word_count`：正文字数统计
- `bytes_written`：实际写入磁盘的字节数

## 使用示例

### 示例 1：生成项目 README

用户："给 nebula 项目写个 README"

```json
{
  "path": "D:/projects/nebula/README.md",
  "topic": "Nebula——Tauri 桌面 AI Agent，双主控 + 蜂群 worker 架构",
  "doc_type": "readme",
  "tone": "technical"
}
```

将生成包含项目徽章、简介、特性列表、安装步骤、使用示例、架构图说明、
许可证的标准 README。

### 示例 2：创建 HTML 数据报告

用户："把这次调研结果整理成一个 HTML 报告"

```json
{
  "path": "D:/reports/market-research-2026.html",
  "topic": "2026 上半年 AI 桌面应用市场调研报告",
  "doc_type": "report",
  "outline": ["摘要", "市场规模", "竞争格局", "用户画像", "趋势预测"],
  "tone": "formal"
}
```

将生成带样式的独立 HTML 文件，含标题层级、表格、引用块，可直接用浏览器打开。

### 示例 3：续写已有文档

用户："在现有 PRD 里补一章关于权限设计的内容"

```json
{
  "path": "D:/docs/auth-system-prd.md",
  "topic": "RBAC 权限模型设计：角色定义、权限粒度、继承关系",
  "doc_type": "prd",
  "overwrite": false
}
```

`overwrite: false` 时在文档末尾追加新章节，保留原有内容。

## 注意事项

- **写入安全**：`file:write` 能力强制路径校验，仅允许写入用户工作区与
  明确授权目录。拒绝覆盖系统文件或未授权路径下的文件。
- **覆盖保护**：默认 `overwrite: false`，已存在文件将进入续写模式。
  若需覆盖，必须显式传 `overwrite: true`，且技能会在写入前备份原文件
  至 `.nebula/backup/` 目录。
- **内容合规**：生成的文档内容遵循 Nebula ValuesLayer 价值层约束，
  拒绝生成违法、欺诈或侵犯版权的内容。
- **HTML 安全**：生成的 HTML 文档不内联外部脚本，不包含 `javascript:`
  协议链接，避免 XSS 风险。样式使用内联 CSS，确保文件可独立分发。
- **大文档分章**：当目标文档超过 5000 字时，技能自动分章节生成并
  逐章写入，避免单次 LLM 调用上下文溢出。
- **编码规范**：所有文件以 UTF-8 无 BOM 编码写入，换行符遵循目标平台
  惯例（Windows 为 CRLF，Unix 为 LF）。
