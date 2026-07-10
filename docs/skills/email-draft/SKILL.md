---
name: email-draft
version: 1.0.0
description: |
  邮件草稿生成技能——根据主题和要点生成专业邮件草稿。当用户要求"写封邮件"、
  "帮我起草邮件"、"回复这封邮件"时加载此技能。通过 llm:call 能力按主题、
  收件人、语气生成结构化邮件正文，并经 file:write 落盘。对标 Hermes
  邮件撰写能力。
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

# Email Draft 技能（邮件草稿生成）

## 概述

Email Draft 是 Nebula 的邮件撰写辅助技能，帮助用户从一句主题与几个要点
快速生成一封结构完整、语气得体的专业邮件草稿。它理解商务沟通的惯例——
称呼、开场、正文、收尾、署名——并能按收件人身份（客户 / 上级 / 同事 /
合作方）调整正式程度，避免措辞失当。

技能流程：理解主题 → 组织要点 → 匹配语气 → 生成正文 → 润色称呼与署名 →
落盘保存。生成的草稿为初稿，用户可在此基础上微调后发送。支持中英文双语
邮件，以及纯文本与 HTML 两种格式输出。

## 使用场景

- **商务沟通**：向客户发送项目进展、报价、合作邀请
- **内部汇报**：向上级汇报工作进展、申请资源、提交方案
- **会议邀约**：发起会议邀请并附议程说明
- **客户回复**：针对客户咨询、投诉给出得体回复
- **求职申请**：撰写求职信、推荐请求、感谢信
- **跨文化邮件**：生成英文商务邮件，避免中式表达

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `subject` | string | 是 | 邮件主题 |
| `key_points` | string[] | 是 | 邮件要点列表，按出现顺序组织 |
| `recipient_type` | string | 否 | 收件人类型：`client` / `manager` / `colleague` / `partner` / `candidate` |
| `tone` | string | 否 | 语气：`formal`（默认）/ `friendly` / `urgent` / `apologetic` |
| `language` | string | 否 | 邮件语言：`zh`（默认）/ `en` / `ja` |
| `sender_name` | string | 否 | 发件人署名，用于落款 |
| `format` | string | 否 | 输出格式：`plain`（默认）/ `html` / `markdown` |
| `output_path` | string | 否 | 草稿保存路径，省略则仅返回内容不落盘 |
| `context` | string | 否 | 背景上下文，如"上次会议讨论了 X 方案" |

示例输入：
```json
{
  "subject": "关于 Nebula 2.1 发布计划的同步",
  "key_points": [
    "Nebula 2.1 计划于 8 月 15 日发布",
    "本次发布包含技能市场与蜂群编排两大特性",
    "需要市场团队在 8 月 1 日前准备宣传素材",
    "请确认各团队排期无冲突"
  ],
  "recipient_type": "colleague",
  "tone": "formal",
  "language": "zh",
  "sender_name": "张明",
  "format": "plain",
  "output_path": "D:/mails/nebula-2.1-sync.draft.txt"
}
```

## 输出

```json
{
  "output": {
    "subject": "关于 Nebula 2.1 发布计划的同步",
    "greeting": "各位同事，大家好：",
    "body": "Nebula 2.1 版本计划于 8 月 15 日正式发布...\n本次发布的核心特性包括...\n为保证发布顺利，需要市场团队在 8 月 1 日前完成宣传素材准备...\n请各位确认各自团队排期无冲突，如有问题请于本周内回复。",
    "closing": "顺祝工作顺利！",
    "signature": "张明\nNebula 项目组",
    "word_count": 280,
    "format": "plain",
    "path": "D:/mails/nebula-2.1-sync.draft.txt"
  },
  "error": null,
  "latency_ms": 3200
}
```

输出字段说明：
- `greeting`：称呼语，按收件人类型调整
- `body`：邮件正文，按要点顺序组织并润色衔接
- `closing`：收尾语，匹配语气
- `signature`：署名，含发件人姓名
- `path`：草稿落盘路径（若提供 `output_path`）

## 使用示例

### 示例 1：正式商务邮件

用户："帮我写封邮件给客户，说项目要延期两周"

```json
{
  "subject": "关于 XX 项目交付时间的调整说明",
  "key_points": [
    "因第三方接口联调延期，项目交付需顺延两周",
    "新交付时间调整为 8 月 30 日",
    "已安排加急处理，确保不再延期",
    "对由此带来的不便深表歉意"
  ],
  "recipient_type": "client",
  "tone": "apologetic",
  "language": "zh",
  "sender_name": "李华"
}
```

生成得体的致歉与说明邮件，语气诚恳且给出明确新交付时间。

### 示例 2：英文求职信

用户："写封英文求职信，申请高级前端工程师"

```json
{
  "subject": "Application for Senior Frontend Engineer",
  "key_points": [
    "5 年前端开发经验，精通 React 与 TypeScript",
    "主导过大型 ToB 应用的架构设计与性能优化",
    "对 Nebula 的技能系统设计很感兴趣",
    "附上简历与作品集链接"
  ],
  "recipient_type": "candidate",
  "tone": "formal",
  "language": "en",
  "sender_name": "Wang Lei",
  "format": "plain"
}
```

生成符合英文商务邮件规范的求职信，避免中式英语。

### 示例 3：会议邀约

用户："发个邮件约个下周的评审会"

```json
{
  "subject": "Nebula 2.1 技能评审会议邀请",
  "key_points": [
    "时间：下周三 14:00-16:00",
    "地点：3 号会议室 / 线上会议链接",
    "议程：技能市场方案评审、蜂群编排技术对齐",
    "请提前阅读附件中的设计文档"
  ],
  "recipient_type": "colleague",
  "tone": "friendly",
  "language": "zh",
  "context": "上周已初步讨论过技能市场方案，本次为正式评审"
}
```

生成清晰的会议邀请邮件，含议程与准备事项。

## 注意事项

- **草稿性质**：生成的邮件为初稿，建议用户发送前审校内容准确性，尤其是
  日期、数据、人名等关键信息。技能不对邮件内容的事实准确性承担责任。
- **语气把控**：`tone` 参数影响整体措辞，但跨文化沟通中正式程度难以完全
  自动判断。重要商务邮件建议用户二次确认语气是否得体。
- **隐私保护**：邮件内容仅在本地 LLM 调用中处理，草稿默认写入用户指定
  路径，不上传外部服务。涉及敏感信息时建议关闭远程 LLM 端点。
- **落款规范**：`sender_name` 仅用于署名行，技能不会伪造发件人邮箱地址。
  实际发送需用户在邮件客户端中完成。
- **HTML 格式**：`format=html` 时生成简单 HTML 邮件，不含复杂样式。如需
  富文本排版，建议在邮件客户端中二次编辑。
- **多语言支持**：`language=ja` 时生成日文邮件，但日文商务礼仪复杂，建议
  用户审校敬语使用是否恰当。
- **依赖说明**：本技能主要依赖 LLM 能力，Python 仅用于文件落盘封装，
  无额外第三方库依赖。
