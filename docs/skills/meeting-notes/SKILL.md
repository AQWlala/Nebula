---
name: meeting-notes
version: 1.0.0
description: |
  会议纪要生成技能——自动生成结构化会议纪要。当用户要求"整理会议纪要"、
  "把会议录音转成纪要"、"总结一下刚才的会"时加载此技能。通过 llm:call
  能力将会议转录文本/口述要点整理为含决议、待办、责任人的正式纪要，
  并经 file:write 能力落盘。对标 OpenAkita 会议纪要能力。
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

# Meeting Notes 技能（会议纪要生成）

## 概述

Meeting Notes 是 Nebula 面向职场高频场景的技能，将杂乱的会议转录文本
或口述要点，整理为结构清晰、可执行的会议纪要。它不只是"摘要发言"，
而是提取三类关键信息：**决议**（决定了什么）、**待办**（谁做什么）、
**未决**（下次讨论什么）。

技能理解会议的类型差异——决策会重结论、站会重进度、评审会重反馈、
头脑风暴重创意——并按对应模板组织纪要结构。

## 使用场景

- **录音转纪要**：会议录音先经 ASR 转文本，再用本技能整理为纪要
- **口述整理**：会后口述几个要点，让 Nebula 扩展为正式纪要
- **实时纪要**：会议进行中边听边记，会后一键生成结构化版本
- **周会归档**：每周例会纪要自动归档至团队知识库
- **客户会议**：整理客户沟通记录，提取需求与承诺事项
- **评审记录**：技术评审会的意见汇总与决策追踪

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 输出纪要文件路径（.md / .html） |
| `transcript` | string | 否 | 会议转录文本（与 `points` 二选一） |
| `points` | string[] | 否 | 口述要点列表（与 `transcript` 二选一） |
| `meeting_type` | string | 否 | 会议类型：`decision` / `standup` / `review` / `brainstorm` / `general` |
| `title` | string | 否 | 会议标题（从内容推断若不提供） |
| `date` | string | 否 | 会议日期 YYYY-MM-DD（默认今天） |
| `participants` | string[] | 否 | 参会人列表 |

示例输入：
```json
{
  "path": "D:/meetings/2026-07-10-release-planning.md",
  "transcript": "今天讨论 v2.1 发布计划...产品说本期必须上线技能市场...开发说最快7月25日...决定7月28日发布...",
  "meeting_type": "decision",
  "title": "v2.1 发布计划评审会",
  "date": "2026-07-10",
  "participants": ["张三", "李四", "王五"]
}
```

## 输出

```json
{
  "output": {
    "path": "D:/meetings/2026-07-10-release-planning.md",
    "title": "v2.1 发布计划评审会",
    "date": "2026-07-10",
    "participants": ["张三", "李四", "王五"],
    "summary": "本次会议讨论 v2.1 版本发布计划，确认技能市场为本期核心功能...",
    "decisions": [
      "v2.1 于 2026-07-28 正式发布",
      "技能市场作为本期必上功能",
      "测试环境 7-20 前就绪"
    ],
    "action_items": [
      {"task": "完成技能市场联调", "owner": "李四", "due": "2026-07-20"},
      {"task": "准备发布说明文档", "owner": "王五", "due": "2026-07-26"},
      {"task": "测试环境部署", "owner": "张三", "due": "2026-07-20"}
    ],
    "open_issues": ["技能审核流程尚未敲定，下次会议讨论"],
    "bytes_written": 4200
  },
  "error": null,
  "latency_ms": 6500
}
```

输出字段说明：
- `decisions`：会议决议列表（已达成共识的结论）
- `action_items`：待办事项列表，每条含任务、负责人、截止日期
- `open_issues`：未决问题列表（下次需讨论的）

## 使用示例

### 示例 1：转录文本转纪要

用户："这是刚才发布评审会的录音转写，帮我整理成纪要"

```json
{
  "path": "D:/meetings/release-review.md",
  "transcript": "（粘贴 ASR 转录文本）",
  "meeting_type": "decision"
}
```

自动识别发言人、决议、待办，生成结构化纪要。

### 示例 2：口述要点扩写

用户："开完会了，记几个要点：1. 下周上线 2. 小明负责测试 3. 文档还没定"

```json
{
  "path": "D:/meetings/quick-notes.md",
  "points": ["下周上线", "小明负责测试", "文档还没定"],
  "meeting_type": "general"
}
```

将零散要点扩展为含决议与待办的正式纪要。

### 示例 3：站会纪要

用户："整理一下今天站会的内容"

```json
{
  "path": "D:/meetings/standup-2026-07-10.md",
  "points": ["张三：昨天完成登录模块，今天做权限，无阻塞", "李四：昨天修了3个bug，今天继续，需要测试环境"],
  "meeting_type": "standup"
}
```

按站会模板整理：每人昨日完成、今日计划、阻塞项。

## 注意事项

- **转录质量**：ASR 转录文本的质量直接影响纪要质量。口音重、背景噪音大
  的录音可能产生错误转录，建议关键决议人工校对。
- **发言人识别**：若转录文本未标注发言人，技能尝试根据上下文推断，
  但准确率有限。建议使用支持说话人分离的 ASR 工具。
- **隐私保护**：会议内容可能包含商业敏感信息。转录文本与生成的纪要
  仅在本地 LLM 处理，不上传外部服务（除非配置远程 LLM 端点）。
- **待办追踪**：生成的 `action_items` 不自动同步至任务管理系统。如需
  同步至飞书任务/Jira/Trello，请配合对应的集成技能使用。
- **模板适配**：不同团队有不同纪要模板习惯。`meeting_type` 选择对应
  模板，如需自定义模板，可在 `points` 中传入模板要求。
- **大会议处理**：超过 2 小时的会议转录文本分批处理并合并纪要，避免
  单次 LLM 调用上下文溢出。可能丢失部分细节，建议长会议分段整理。
- **依赖说明**：需要 Python 环境用于文本预处理（分段、发言人识别辅助）。
  纯 LLM 整理不依赖 Python，但准确度略低。
