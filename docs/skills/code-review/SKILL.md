---
name: code-review
version: 1.0.0
description: |
  代码审查技能——对代码进行审查并给出改进建议。当用户要求"审查这段代码"、
  "review 一下"、"看看有什么问题"、"代码质量怎么样"时加载此技能。通过
  file:read 能力读取代码文件，再用 llm:call 能力从可读性、正确性、安全性、
  性能、可维护性五个维度评审。对标 Hermes 代码审查能力。
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

# Code Review 技能（代码审查）

## 概述

Code Review 是 Nebula 面向开发者的核心技能，模拟资深工程师的代码审查
流程。它读取指定代码文件（或代码片段），从五个维度进行系统性评审，
输出结构化审查报告：每条问题标注严重等级、所在位置、修复建议与示例代码。

五个审查维度：
1. **正确性**：逻辑错误、边界条件、空指针、资源泄漏
2. **可读性**：命名规范、注释完整性、代码结构、函数长度
3. **安全性**：注入风险、硬编码密钥、不安全 API、权限越界
4. **性能**：不必要的分配、N+1 查询、算法复杂度、缓存机会
5. **可维护性**：耦合度、扩展性、测试覆盖、设计模式适用性

## 使用场景

- **提交前自审**：写完一段代码，提交前让 Nebula 帮忙过一遍
- **PR 审查**：收到 Pull Request，快速获取审查意见再人工复核
- **遗留代码评估**：接手老项目，评估代码质量与重构优先级
- **学习改进**：通过审查反馈学习最佳实践与常见陷阱
- **安全审计**：对涉及认证/支付/数据库的代码做安全专项审查
- **团队规范对齐**：检查代码是否符合团队的编码规范

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 代码文件绝对路径（或目录，将审查目录下所有源文件） |
| `language` | string | 否 | 编程语言（自动检测，可手动指定确保准确） |
| `focus` | string | 否 | 审查重点：`security` / `performance` / `readability` / `all`（默认） |
| `severity_filter` | string | 否 | 最低报告严重级：`info` / `warning` / `error` / `critical`（默认 info） |
| `context` | string | 否 | 额外上下文，如"这是支付模块"、"即将上线" |

示例输入：
```json
{
  "path": "D:/projects/nebula/src-tauri/src/skills/executor.rs",
  "language": "rust",
  "focus": "security",
  "severity_filter": "warning",
  "context": "此模块执行外部 skill 代码，安全敏感"
}
```

## 输出

```json
{
  "output": {
    "file": "executor.rs",
    "language": "rust",
    "lines_reviewed": 342,
    "issues": [
      {
        "severity": "critical",
        "line": 87,
        "dimension": "security",
        "title": "命令注入风险",
        "description": "exec_command 将用户输入直接拼入 shell 命令字符串...",
        "suggestion": "使用 Command::new 分离参数，避免 shell 解释",
        "example": "let mut cmd = Command::new(\"git\");\ncmd.arg(\"commit\").arg(message);"
      },
      {
        "severity": "warning",
        "line": 156,
        "dimension": "performance",
        "title": "循环内重复分配",
        "description": "...",
        "suggestion": "..."
      }
    ],
    "summary": {
      "critical": 1,
      "error": 0,
      "warning": 3,
      "info": 5,
      "overall_score": 72
    }
  },
  "error": null,
  "latency_ms": 6500
}
```

输出字段说明：
- `issues`：问题列表，每条含严重级、行号、维度、标题、描述、建议、示例
- `summary`：按严重级统计 + 整体质量评分（0-100）
- `overall_score`：综合质量分，90+ 优秀 / 70-89 良好 / 50-69 合格 / <50 待改进

## 使用示例

### 示例 1：审查单个 Rust 文件

用户："帮我 review 一下 executor.rs，重点看安全"

```json
{
  "path": "D:/projects/nebula/src-tauri/src/skills/executor.rs",
  "focus": "security",
  "severity_filter": "warning"
}
```

输出将聚焦安全问题，报告注入风险、不安全 API 使用等，每条附修复示例。

### 示例 2：审查整个目录

用户："看看 src/utils 这个目录的代码质量"

```json
{
  "path": "D:/projects/nebula/src-tauri/src/utils/",
  "focus": "all",
  "severity_filter": "info"
}
```

将遍历目录下所有源文件，汇总审查报告，适合评估模块整体质量。

### 示例 3：上线前安全专项

用户："这个支付模块明天上线，帮我做安全审计"

```json
{
  "path": "D:/projects/payment/src/",
  "focus": "security",
  "severity_filter": "error",
  "context": "支付模块，即将上线，安全敏感度最高"
}
```

仅报告 error 及以上安全问题，确保上线前阻断高危漏洞。

## 注意事项

- **只读审查**：本技能仅读取代码并给建议，不修改任何文件。修复由
  用户或 `code-refactor` 技能执行。
- **上下文限制**：单文件超过 2000 行时，技能分批审查并合并报告。
  超大文件（>10000 行）建议按模块拆分后分别审查。
- **语言支持**：支持 Rust / TypeScript / JavaScript / Python / Go /
  Java / C# / C++ 等主流语言。不识别的语言将退化为通用文本审查。
- **误报可能**：LLM 审查可能产生误报（将安全写法误判为风险）。每条
  建议需用户结合上下文判断，critical 级问题建议人工复核确认。
- **不替代人工**：本技能是辅助工具，不替代正式的 Code Review 流程。
  安全关键代码必须由人工审查者签字确认。
- **隐私保护**：代码内容仅在本地 LLM 调用中处理，不上传外部服务
  （除非配置远程 LLM 端点）。审查报告不持久化，除非用户显式保存。
- **依赖说明**：需要 Python 环境用于部分语言的语法解析（AST 分析）。
  纯文本审查不依赖 Python，但准确度略低。
