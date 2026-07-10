---
name: calendar-assist
version: 1.0.0
description: |
  日程管理辅助技能——日程创建、查询、提醒，与 Nebula 日历组件集成。
  当用户要求"帮我建个日程"、"今天有什么安排"、"提醒我下午三点开会"时加载此技能。
  通过日历读写能力管理本地或对接的系统日历。对标 OpenAkita calendar_assist 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "calendar:read", "calendar:write"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Calendar Assist 技能（日程管理辅助）

## 概述

Calendar Assist 是 Nebula 的日程管理技能，负责日程的创建、查询、修改、
提醒，并与 Nebula 桌面端的日历组件深度集成。通过 `calendar:read` 读取
日程列表、`calendar:write` 创建或更新日程项，结合 `llm:call` 解析自然
语言时间表达（如"下周三下午三点"、"明天上午"）。

该技能让用户用一句话管理日程，无需在日历 UI 中手动填写表单。支持与系统
日历（macOS Calendar / Outlook / Google Calendar）双向同步。

## 使用场景

- **自然语言建日程**："下周三下午 3 点和产品团队开个会，1 小时"
- **日程查询**："今天还有什么安排"、"这周空余时间多吗"
- **智能提醒**：基于日程时间自动设置提前提醒（会议前 15 分钟）
- **冲突检测**：新建日程与已有安排冲突时主动提示并建议替代时段
- **日程摘要**：每天早晨生成今日日程简报
- **会议准备**：会议前自动拉取相关文档与上次纪要

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | string | 是 | 操作：`create` / `query` / `update` / `delete` / `summary` |
| `title` | string | 否 | `action=create` 时的日程标题 |
| `time_expr` | string | 否 | 自然语言时间，如"明天下午3点"、"2026-07-15 14:00" |
| `duration` | number | 否 | 持续时间（分钟），默认 60 |
| `location` | string | 否 | 地点 |
| `attendees` | string[] | 否 | 参会人邮箱或姓名 |
| `reminder` | number | 否 | 提前提醒分钟数，默认 15 |
| `query_date` | string | 否 | `action=query` 时的查询日期，默认今天 |
| `calendar_id` | string | 否 | 指定日历源，默认主日历 |

示例输入：
```json
{
  "action": "create",
  "title": "产品周会",
  "time_expr": "下周三下午3点",
  "duration": 60,
  "attendees": ["alice@nebula.app", "bob@nebula.app"],
  "reminder": 15
}
```

## 输出

```json
{
  "output": {
    "action": "create",
    "event": {
      "id": "evt-2026-0715-001",
      "title": "产品周会",
      "start": "2026-07-15T15:00:00+08:00",
      "end": "2026-07-15T16:00:00+08:00",
      "location": "",
      "attendees": ["alice@nebula.app", "bob@nebula.app"],
      "reminder_minutes": 15,
      "calendar_id": "primary"
    },
    "conflict_check": {
      "has_conflict": false,
      "nearby_events": [
        {"title": "晨会", "start": "2026-07-15T10:00:00+08:00", "end": "2026-07-15T10:30:00+08:00"}
      ]
    }
  },
  "error": null,
  "latency_ms": 800
}
```

输出字段说明：
- `event`：创建/查询到的日程对象
- `conflict_check`：冲突检测结果，含相邻日程信息
- `reminder_minutes`：提前提醒时间（分钟）

## 使用示例

### 示例 1：自然语言创建日程

用户："下周三下午 3 点和产品团队开个会，1 小时"

```json
{
  "action": "create",
  "title": "产品团队会议",
  "time_expr": "下周三下午3点",
  "duration": 60
}
```

技能解析"下周三下午3点"为具体时间戳，创建日程并自动设置 15 分钟提前提醒。

### 示例 2：查询今日安排

用户："今天有什么安排"

```json
{
  "action": "query",
  "query_date": "today"
}
```

返回今日所有日程列表，按时间顺序排列，含每个日程的标题、时间、地点、参会人。

### 示例 3：生成每日日程简报

用户："给我看看今天的日程简报"

```json
{
  "action": "summary",
  "query_date": "today"
}
```

生成结构化简报：今日共 X 个会议、下一个会议在 Y 分钟后、今日空余时段分布等。

## 注意事项

- **时区处理**：所有时间存储为本地时区（Asia/Shanghai），与外部日历同步时
  自动转换为 UTC。查询时统一返回本地时间。
- **冲突检测**：创建日程时自动检查时间重叠，若有冲突会在响应中标记但不
  阻止创建，由用户决定是否调整。
- **日历源**：默认使用 Nebula 内置日历；如需同步系统日历需在设置中授权
  对应的日历账户（macOS Calendar / Outlook / Google）。
- **提醒机制**：提醒通过 Nebula 桌面通知推送，需确保应用在后台运行。系统
  级提醒（如 macOS 通知中心）可作为补充。
- **隐私保护**：日程数据仅存储在本地与用户授权的日历服务，不上传至 LLM
  服务。仅日程标题与时间参与 LLM 解析，参会人邮箱等敏感信息本地处理。
- **依赖说明**：需要 Python 环境用于自然语言时间解析（dateparser）与
  iCalendar 解析（icalendar）。系统日历同步通过各平台原生 API。
