---
name: data-analyzer
version: 1.0.0
description: |
  数据分析可视化技能——分析 CSV/JSON/Excel 数据，生成统计摘要与可视化图表。
  当用户要求"分析这份数据"、"生成数据报表"、"画个图表看看趋势"时加载此技能。
  通过 file:read 读取数据、llm:call 生成分析洞察与统计摘要、file:write 落盘
  报表与图表。对标 OpenAkita data_analyzer 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "file:read", "file:write"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Data Analyzer 技能（数据分析与可视化）

## 概述

Data Analyzer 是 Nebula 的数据分析技能，面向有表格数据但不想写代码的用户。
它读取本地 CSV / JSON / Excel 文件，自动完成数据探查、统计摘要、趋势识别
与异常检测，并生成可视化图表（柱状图 / 折线图 / 饼图 / 散点图）与分析报告，
最终通过 `file:write` 落盘为可分享的 Markdown 报告与 PNG 图表。

技能流程：读取数据 → 推断字段类型 → 生成描述性统计 → 识别趋势与异常 →
由 `llm:call` 生成自然语言洞察 → 渲染图表 → 写入报告。用户只需给一个文件
路径与一句关注焦点，即可得到一份可读的数据分析结论。

## 使用场景

- **销售数据复盘**：拿到一份月度销售 CSV，快速了解哪些品类在涨、哪些在跌
- **用户行为分析**：从 Excel 导出的用户行为日志中找出留存与流失特征
- **财务报表生成**：将原始流水数据加工为带图表的月度财务摘要
- **实验结果对比**：A/B 测试结果数据自动算显著性并出对比图
- **异常排查**：在时序数据中定位异常波动点并给出可能原因
- **数据质量体检**：导入新数据集时快速了解缺失率、字段分布、唯一值

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 数据文件路径，支持 .csv / .json / .xlsx / .xls |
| `analysis_type` | string | 否 | 分析类型：`summary`（默认）/ `trend` / `correlation` / `outlier` |
| `focus` | string | 否 | 分析关注焦点，如"营收增长"、"用户留存"、"成本结构" |
| `chart_type` | string | 否 | 图表类型：`bar` / `line` / `pie` / `scatter` / `auto`（默认） |
| `x_field` | string | 否 | 指定 X 轴字段，省略则自动推断 |
| `y_field` | string | 否 | 指定 Y 轴字段，省略则自动推断 |
| `output_path` | string | 否 | 报告输出路径，默认为数据文件同目录下的 `.report.md` |
| `max_rows` | number | 否 | 读取最大行数，默认 100000，超出将抽样 |

示例输入：
```json
{
  "path": "D:/data/sales-2026-q2.csv",
  "analysis_type": "trend",
  "focus": "各品类销售额环比变化",
  "chart_type": "line",
  "x_field": "month",
  "y_field": "revenue",
  "output_path": "D:/reports/sales-q2-analysis.md"
}
```

## 输出

```json
{
  "output": {
    "file": "sales-2026-q2.csv",
    "rows": 12500,
    "columns": 8,
    "stats": {
      "revenue": {"mean": 48200, "median": 36500, "std": 21300, "min": 1200, "max": 198000},
      "quantity": {"mean": 156, "median": 120, "std": 89}
    },
    "insights": [
      "Q2 整体营收环比增长 18%，主要由 3C 品类贡献",
      "服装品类 6 月出现 12% 的下滑，疑似受促销节奏影响",
      "客单价中位数 365 元，长尾用户贡献约 40% 营收"
    ],
    "chart_path": "D:/reports/sales-q2-trend.png",
    "report_path": "D:/reports/sales-q2-analysis.md",
    "missing_rate": {"revenue": 0, "category": 0.02}
  },
  "error": null,
  "latency_ms": 8600
}
```

输出字段说明：
- `stats`：各数值字段的描述性统计（均值 / 中位数 / 标准差 / 极值）
- `insights`：LLM 生成的自然语言洞察，结合关注焦点给出 3-5 条结论
- `chart_path`：生成的图表 PNG 路径
- `report_path`：完整 Markdown 报告路径，含统计表、图表与洞察
- `missing_rate`：各字段缺失率，用于评估数据质量

## 使用示例

### 示例 1：销售数据趋势分析

用户："分析一下这份 Q2 销售数据，看看各品类趋势"

```json
{
  "path": "D:/data/sales-2026-q2.csv",
  "analysis_type": "trend",
  "focus": "各品类销售额环比变化",
  "chart_type": "line"
}
```

技能将按月份聚合各品类销售额，生成折线趋势图，并给出增长最快与下滑最明显的
品类洞察，报告落盘为 Markdown。

### 示例 2：用户行为相关性分析

用户："看看这份数据里用户活跃度和留存有没有关系"

```json
{
  "path": "D:/data/user-behavior.xlsx",
  "analysis_type": "correlation",
  "focus": "登录频次与 7 日留存的相关性",
  "chart_type": "scatter",
  "x_field": "login_count",
  "y_field": "retention_7d"
}
```

生成散点图与相关系数矩阵，标注显著相关字段对，给出运营建议。

### 示例 3：异常值检测

用户："这份数据里有没有异常值？"

```json
{
  "path": "D:/data/transactions.json",
  "analysis_type": "outlier",
  "chart_type": "auto"
}
```

使用箱线图与 Z-score 识别异常值，列出 Top-N 异常记录及其可能成因。

## 注意事项

- **数据规模**：单文件超过 100 万行时将抽样分析并提示用户结论基于样本。
  超过 500 MB 时拒绝处理，建议先按时间或维度拆分。
- **隐私保护**：数据内容仅在本地 LLM 调用中处理，分析报告与图表默认写入
  用户指定路径，不上传外部服务（除非显式配置远程 LLM 端点）。
- **字段推断**：技能会自动推断字段类型（数值 / 类别 / 时间 / 文本），推断
  失败时按文本处理。建议用户在 `x_field` / `y_field` 中显式指定关键字段。
- **图表渲染**：依赖 Python 的 matplotlib / pandas 生成图表，需 Python 环境。
  无 Python 时退化为纯文本统计表，不生成 PNG 图表。
- **Excel 兼容**：`.xlsx` / `.xls` 支持多工作表，默认分析第一个工作表。
  需分析指定工作表时，在 `focus` 中注明工作表名。
- **数值精度**：统计计算使用双精度浮点，极大数值可能存在精度损失。金融
  场景建议用户对关键结果二次校验。
- **依赖说明**：需要 Python 环境用于数据解析（pandas）与图表渲染（matplotlib）。
  JSON / 简单 CSV 的读取可由 Nebula 原生处理。
