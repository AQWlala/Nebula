---
name: translation
version: 1.0.0
description: |
  多语言翻译技能——中英日韩多语言互译，保持专业术语准确。当用户要求
  "翻译一下"、"翻成英文"、"这段日语什么意思"时加载此技能。通过 llm:call
  能力结合领域术语表完成高质量翻译。对标 OpenAkita translation 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Translation 技能（多语言翻译）

## 概述

Translation 是 Nebula 的多语言翻译技能，支持中文、英文、日文、韩文四语
之间的任意方向互译。不同于通用机器翻译，本技能的核心价值是**术语准确**
与**语境适配**——通过领域参数（技术 / 法律 / 医疗 / 商务）切换术语表，
保证专业词汇翻译一致；通过 `formality` 参数控制敬语与正式程度，适配
不同场合的表达习惯。

技能流程：检测源语言 → 识别领域术语 → 翻译正文 → 校验术语一致性 →
输出译文。支持长文本分段翻译与术语表自定义，适合技术文档、合同、论文等
对术语精度要求高的场景。

## 使用场景

- **技术文档翻译**：将英文 API 文档翻成中文，保持技术术语一致
- **商务合同翻译**：中英合同互译，确保法律术语准确
- **论文摘要翻译**：将中文论文摘要翻成英文用于投稿
- **日常沟通翻译**：理解日文 / 韩文邮件或消息的含义
- **本地化适配**：将产品文案翻成多语言用于国际化发布
- **术语统一**：长文档翻译时保持同一术语前后翻译一致

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `text` | string | 是 | 待翻译文本（与 `source_path` 二选一） |
| `source_path` | string | 否 | 待翻译文件路径，支持 .txt / .md |
| `source_lang` | string | 否 | 源语言：`zh` / `en` / `ja` / `ko` / `auto`（默认自动检测） |
| `target_lang` | string | 是 | 目标语言：`zh` / `en` / `ja` / `ko` |
| `domain` | string | 否 | 领域：`general`（默认）/ `tech` / `legal` / `medical` / `business` |
| `formality` | string | 否 | 正式程度：`formal` / `neutral`（默认）/ `informal` |
| `glossary` | object | 否 | 自定义术语表，如 `{"MCP": "模型上下文协议"}` |
| `preserve_format` | boolean | 否 | 是否保留原文格式（Markdown 标记等），默认 `true` |

示例输入：
```json
{
  "text": "The Model Context Protocol (MCP) enables LLMs to interact with external services through well-designed tools.",
  "source_lang": "en",
  "target_lang": "zh",
  "domain": "tech",
  "formality": "neutral",
  "glossary": {"MCP": "模型上下文协议", "LLM": "大语言模型"}
}
```

## 输出

```json
{
  "output": {
    "source_lang": "en",
    "target_lang": "zh",
    "translated": "模型上下文协议（MCP）使大语言模型能够通过设计良好的工具与外部服务交互。",
    "terms_applied": [
      {"source": "MCP", "translation": "模型上下文协议"},
      {"source": "LLM", "translation": "大语言模型"}
    ],
    "char_count": 38,
    "domain": "tech"
  },
  "error": null,
  "latency_ms": 1800
}
```

输出字段说明：
- `translated`：翻译后的文本
- `terms_applied`：本次翻译中应用的术语映射，便于用户核对
- `char_count`：译文字符数
- `domain`：实际使用的领域

## 使用示例

### 示例 1：技术文档中英翻译

用户："把这段英文技术文档翻成中文"

```json
{
  "text": "Nebula uses a dual-controller architecture with swarm workers to orchestrate skills.",
  "source_lang": "en",
  "target_lang": "zh",
  "domain": "tech",
  "glossary": {"swarm workers": "蜂群 worker", "dual-controller": "双主控"}
}
```

保持技术术语一致，"swarm workers" 按术语表译为"蜂群 worker"。

### 示例 2：日文邮件理解

用户："这段日语邮件是什么意思？"

```json
{
  "text": "お世話になっております。来週の会議の件ですが、日程を変更していただけないでしょうか。",
  "source_lang": "ja",
  "target_lang": "zh",
  "domain": "business",
  "formality": "neutral"
}
```

识别商务日语语境，准确翻译敬语表达，输出自然的中文。

### 示例 3：长文档分段翻译

用户："把这个 md 文档翻成英文"

```json
{
  "source_path": "D:/docs/nebula-design.md",
  "source_lang": "zh",
  "target_lang": "en",
  "domain": "tech",
  "preserve_format": true,
  "glossary": {"技能": "skill", "蜂群": "swarm", "主控": "controller"}
}
```

读取 Markdown 文件，按段落翻译并保留原文格式（标题、列表、代码块），
术语表确保全篇术语一致。

## 注意事项

- **术语一致性**：长文档翻译时，建议提供 `glossary` 锁定关键术语翻译，
  避免同一术语在不同段落出现不同译法。技能会全篇应用术语表。
- **自动检测局限**：`source_lang=auto` 对短文本可能误判，建议短句翻译
  时显式指定源语言。混合语言文本以主要语言为准。
- **领域适配**：`domain` 影响术语选择与表达风格。法律、医疗领域术语
  精度要求高，建议用户提供领域术语表并人工审校。
- **格式保留**：`preserve_format=true` 时保留 Markdown 标记与代码块，
  但代码块内的内容不翻译。如需翻译代码注释，请单独处理。
- **敬语处理**：日文 / 韩文翻译涉及敬语等级，`formality` 参数控制大致
  正式程度，但无法完全替代母语者的语感判断。正式商务场景建议审校。
- **不可译内容**：人名、地名、品牌名等专有名词默认保留原文，可在
  `glossary` 中指定译法。代码标识符（变量名、函数名）不翻译。
- **隐私保护**：翻译内容仅在本地 LLM 调用中处理，不持久化、不上传
  外部服务（除非显式配置远程 LLM 端点）。
- **依赖说明**：本技能仅依赖 LLM 能力，Python 用于封装与分段处理，
  无额外第三方库依赖。
