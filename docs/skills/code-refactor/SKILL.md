---
name: code-refactor
version: 1.0.0
description: |
  代码重构建议技能——分析代码并给出重构建议。当用户要求"重构这段代码"、
  "这段代码怎么优化"、"改善代码结构"时加载此技能。通过 file:read 能力
  读取代码，再用 llm:call 能力从设计模式、复杂度、重复代码、耦合度等
  维度给出重构方案与示例代码。对标 Hermes 代码重构能力。
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

# Code Refactor 技能（代码重构建议）

## 概述

Code Refactor 是 Nebula 面向技术债务治理的技能，在不改变代码外部行为的
前提下，改善其内部结构。与 `code-review` 的区别：`code-review` 找问题
（发现问题），`code-refactor` 给方案（解决问题）。

技能遵循 Martin Fowler 的重构哲学：**小步重构、保持测试通过、行为不变**。
每个建议都包含"为什么改"、"怎么改"、"改成什么样"三部分，并评估重构的
风险与收益，帮助用户判断是否值得动刀。

## 使用场景

- **遗留代码改造**：接手老项目，逐步改善代码可维护性
- **技术债务清理**：识别高债务区域，制定重构优先级
- **设计模式应用**：识别可用模式改善的代码结构
- **复杂度降低**：拆分超长函数、简化嵌套条件、消除重复
- **性能优化前置**：重构为更易优化的结构，再做性能调优
- **代码评审跟进**：review 发现的问题，用本技能出具体重构方案

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 代码文件绝对路径 |
| `language` | string | 否 | 编程语言（自动检测） |
| `goal` | string | 否 | 重构目标：`readability` / `performance` / `maintainability` / `all`（默认） |
| `aggressive` | boolean | 否 | 是否激进重构（大改），默认 false（保守小步） |
| `max_suggestions` | number | 否 | 最大建议数，默认 10 |
| `include_examples` | boolean | 否 | 是否含示例代码，默认 true |

示例输入：
```json
{
  "path": "D:/projects/app/src/services/payment.ts",
  "language": "typescript",
  "goal": "maintainability",
  "aggressive": false,
  "max_suggestions": 5
}
```

## 输出

```json
{
  "output": {
    "file": "payment.ts",
    "language": "typescript",
    "lines_analyzed": 320,
    "suggestions": [
      {
        "id": "R001",
        "title": "提取 processPayment 为策略模式",
        "type": "extract_pattern",
        "priority": "high",
        "benefit": "新增支付方式时无需修改 processPayment，符合开闭原则",
        "risk": "medium",
        "effort": "2-4 小时",
        "before": "function processPayment(type, amount) {\n  if (type === 'alipay') {...}\n  else if (type === 'wechat') {...}\n  else if (type === 'card') {...}\n}",
        "after": "interface PaymentStrategy {\n  pay(amount: number): Promise<Result>;\n}\nclass AlipayStrategy implements PaymentStrategy {...}",
        "tests_impact": "现有测试需调整 mock 方式，行为不变"
      }
    ],
    "summary": {
      "total": 5,
      "high_priority": 2,
      "medium_priority": 2,
      "low_priority": 1,
      "estimated_effort": "1-2 天"
    }
  },
  "error": null,
  "latency_ms": 8200
}
```

输出字段说明：
- `suggestions`：重构建议列表，每条含标题、类型、优先级、收益、风险、前后对比
- `suggestions[].before` / `after`：重构前后的代码对比
- `summary.estimated_effort`：预估总工作量

## 使用示例

### 示例 1：可维护性重构

用户："这个 payment.ts 越来越难维护了，帮我看看怎么重构"

```json
{
  "path": "D:/projects/app/src/services/payment.ts",
  "goal": "maintainability",
  "aggressive": false
}
```

输出保守的小步重构建议：提取函数、消除重复、简化条件。

### 示例 2：性能导向重构

用户："这个数据处理模块太慢了，先重构再优化"

```json
{
  "path": "D:/projects/app/src/utils/data-processor.py",
  "goal": "performance",
  "aggressive": true
}
```

输出面向性能的重构：减少分配、向量化、惰性求值、缓存机会。

### 示例 3：长函数拆分

用户："这个函数 300 行了，帮我拆一下"

```json
{
  "path": "D:/projects/app/src/handlers/order.rs",
  "goal": "readability",
  "max_suggestions": 3
}
```

聚焦拆分超长函数，给出提取边界与命名建议。

## 注意事项

- **建议非自动执行**：本技能只给建议与示例代码，不自动修改源文件。
  用户需手动应用或配合编辑器操作。未来版本可能支持自动应用并跑测试。
- **行为保持**：重构的核心原则是"行为不变"。每条建议都标注 `tests_impact`，
  提醒用户在应用前确保有足够测试覆盖，应用后跑全量测试验证。
- **风险评估**：`risk` 字段标注每条建议的风险等级（low/medium/high）。
  high 风险建议（如跨模块重构）建议分多次小步完成，每步独立测试。
- **激进模式**：`aggressive: true` 会给出大刀阔斧的建议（如整体架构调整），
  可能涉及多文件改动。除非有完善测试，否则不建议在上线前使用。
- **语言特性**：建议会利用目标语言的特定特性（如 Rust 的 trait /
  Python 的 dataclass / TS 的泛型）。跨语言迁移场景需人工调整。
- **依赖关系**：重构建议可能涉及多个文件。技能分析当前文件时无法感知
  全项目依赖，建议结合 `code-review` 全目录审查后制定重构计划。
- **隐私保护**：代码内容仅在本地 LLM 调用中处理，不上传外部服务。
  重构建议不持久化，除非用户显式保存。
- **依赖说明**：需要 Python 环境用于 AST 分析与圈复杂度计算。纯文本
  分析不依赖 Python，但建议精度略低。
