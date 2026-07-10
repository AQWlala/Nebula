---
name: video-script
version: 1.0.0
description: |
  视频脚本草稿生成技能——根据主题生成视频脚本，含分镜、旁白、字幕。
  当用户要求"帮我写个视频脚本"、"生成一个 3 分钟的产品介绍分镜"时加载此技能。
  通过 LLM 调用生成结构化脚本，并写入文件保存。对标 CoPaw video_script 能力。
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

# Video Script 技能（视频脚本草稿生成）

## 概述

Video Script 是 Nebula 的视频脚本创作技能，根据用户提供的主题、时长、
风格等参数，自动生成包含分镜描述、旁白文案、字幕文本、镜头建议的结构化
脚本。通过 `llm:call` 完成创意生成，再用 `file:write` 输出为 Markdown
或 JSON 文件供剪辑工具导入。

该技能面向短视频创作者、产品演示制作、教学视频作者。生成的脚本是"草稿"，
强调可编辑性——用户可在此基础上调整镜头顺序、替换文案、补充细节，
而非直接交付成片。

## 使用场景

- **短视频脚本**：为抖音/B站/YouTube 生成 1-3 分钟的脚本草稿
- **产品演示**：新功能发布前生成产品介绍视频脚本
- **教学内容**：把一篇技术文章转为教学视频脚本
- **口播文案**：纯口播视频的文案与停顿标注
- **广告脚本**：信息流广告的镜头脚本与旁白

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `topic` | string | 是 | 视频主题，如"AI 桌面助手 Nebula 介绍" |
| `duration_sec` | number | 否 | 目标时长（秒），默认 60 |
| `style` | string | 否 | 风格：`tutorial` / `promo` / `vlog` / `explainer` / `ad` |
| `platform` | string | 否 | 目标平台：`bilibili` / `douyin` / `youtube` / `general` |
| `tone` | string | 否 | 文案语气：`professional`（默认）/ `casual` / `humorous` |
| `language` | string | 否 | 字幕语言：`zh`（默认）/ `en` / `bilingual` |
| `output_path` | string | 否 | 脚本文件保存路径，默认 `<工作区>/video-scripts/` |
| `reference` | string | 否 | 参考文案或既有素材的路径/文本 |

示例输入：
```json
{
  "topic": "Nebula 桌面 AI Agent 功能介绍",
  "duration_sec": 90,
  "style": "promo",
  "platform": "bilibili",
  "tone": "professional",
  "language": "zh"
}
```

## 输出

```json
{
  "output": {
    "script_path": "D:/workspace/video-scripts/nebula-intro-2026-07-10.md",
    "title": "Nebula：你的桌面 AI Agent",
    "total_duration_sec": 92,
    "scene_count": 6,
    "scenes": [
      {
        "index": 1,
        "duration_sec": 12,
        "shot": "开场：桌面图标特写，淡入 Nebula logo",
        "narration": "每天打开电脑，重复的操作占据了大量时间……",
        "subtitle": "每天打开电脑，重复的操作占据了大量时间",
        "b_roll": "屏幕录制：手动整理桌面文件"
      }
    ],
    "word_count": 320
  },
  "error": null,
  "latency_ms": 4500
}
```

输出字段说明：
- `script_path`：完整脚本的 Markdown 文件路径
- `scenes`：分镜列表，每镜含镜头描述、旁白、字幕、B-roll 建议
- `total_duration_sec`：脚本估算总时长
- `word_count`：旁白总字数

## 使用示例

### 示例 1：生成 90 秒产品介绍脚本

用户："帮我写个 90 秒的 Nebula 介绍视频脚本，B 站风格"

```json
{
  "topic": "Nebula 桌面 AI Agent 功能介绍",
  "duration_sec": 90,
  "style": "promo",
  "platform": "bilibili"
}
```

生成 6-8 个分镜的脚本，每镜含镜头描述、旁白、字幕，整体节奏适配 B 站观众偏好。

### 示例 2：技术教程脚本

用户："做一个 Tauri 入门教学视频脚本，5 分钟"

```json
{
  "topic": "Tauri 桌面应用开发入门",
  "duration_sec": 300,
  "style": "tutorial",
  "tone": "professional",
  "language": "zh"
}
```

生成结构化教学脚本：知识点拆分为多个分镜，每个分镜标注演示操作与解说文案。

### 示例 3：基于参考素材改写

用户："根据这篇博客帮我做一期视频脚本"

```json
{
  "topic": "AI Agent 架构解析",
  "reference": "D:/blog/ai-agent-architecture.md",
  "duration_sec": 180,
  "style": "explainer"
}
```

读取参考文章，提取核心观点，重组为视频分镜脚本。

## 注意事项

- **草稿定位**：输出是"可编辑草稿"，用户应在此基础上调整。技能不会生成
  最终成片或剪辑工程文件，仅产出文字脚本。
- **时长估算**：旁白字数按中文 4 字/秒、英文 2.5 词/秒估算，实际录制可能
  偏差 ±15%。建议用户录制后回填实际时长。
- **版权规避**：脚本内容为 LLM 原创生成，不直接复制已有作品文案。如用户提供
  参考素材，仅提取观点结构，不照搬原文。
- **平台适配**：不同平台的脚本节奏不同（抖音前 3 秒钩子、B 站铺垫较长、
  YouTube 重视标题与封面建议），技能会按 `platform` 调整结构。
- **字幕规范**：字幕文本每行不超过 20 字（中文）/ 42 字符（英文），
  便于剪辑软件直接套用。
- **依赖说明**：需要 Python 环境用于文件写入与模板渲染（Jinja2）。
  LLM 调用由 Nebula 统一能力处理。
