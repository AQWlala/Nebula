---
name: skill-test-runner
version: 1.0.0
description: |
  技能测试运行器技能——运行和验证 Nebula 技能的测试用例，输出结构化测试报告。
  当用户要求"跑一下这个技能的测试"、"验证 skill 是否正常工作"时加载此技能。
  通过 shell 执行能力运行测试套件，并经 LLM 调用生成诊断报告。对标 OpenAkita skill_test_runner 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "exec:shell"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Skill Test Runner 技能（技能测试运行器）

## 概述

Skill Test Runner 是 Nebula 的技能质量保障技能，负责运行技能自带的
测试用例（位于技能目录的 `tests/` 子目录或 `SKILL.md` 中声明的测试
脚本），收集执行结果，生成结构化的测试报告。通过 `exec:shell` 执行
测试命令、`llm:call` 分析失败原因并给出修复建议。

该技能面向技能开发者与质检流程：在技能发布前运行测试套件验证功能正确性，
在技能升级后回归测试确保兼容性。支持多语言测试（pytest / jest / cargo test）。

## 使用场景

- **发布前验证**：技能发布到市场前运行完整测试套件
- **升级回归**：Nebula 升级后批量运行所有技能测试，确认无回归
- **失败诊断**：测试失败时自动分析原因，给出修复建议
- **兼容性检查**：在不同 OS / Python 版本下运行测试
- **CI 集成**：作为 CI 流水线的一环，自动运行并产出报告

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `skill_id` | string | 否 | 单个技能 ID，如 `file-reader`；省略则运行全部 |
| `test_filter` | string | 否 | 测试用例过滤，如 `test_read_*` |
| `parallel` | boolean | 否 | 是否并行运行多个技能的测试，默认 `false` |
| `timeout_sec` | number | 否 | 单测试超时秒数，默认 60 |
| `env` | object | 否 | 测试时注入的环境变量 |
| `report_format` | string | 否 | 报告格式：`json`（默认）/ `markdown` / `junit` |
| `output_path` | string | 否 | 报告保存路径 |
| `diagnose_failures` | boolean | 否 | 是否对失败用例做 LLM 诊断，默认 `true` |

示例输入：
```json
{
  "skill_id": "file-reader",
  "test_filter": "test_read_pdf*",
  "timeout_sec": 120,
  "report_format": "markdown",
  "output_path": "D:/reports/skill-test-2026-07-10.md",
  "diagnose_failures": true
}
```

## 输出

```json
{
  "output": {
    "skill_id": "file-reader",
    "total": 18,
    "passed": 16,
    "failed": 2,
    "skipped": 0,
    "duration_sec": 23.4,
    "results": [
      {
        "name": "test_read_pdf_with_text_layer",
        "status": "passed",
        "duration_ms": 320
      },
      {
        "name": "test_read_pdf_scanned_image",
        "status": "failed",
        "duration_ms": 5100,
        "error": "AssertionError: expected summary but got empty string",
        "diagnosis": "扫描型 PDF 无文本层，OCR 模块未触发。建议在 file:read 中检测 PDF 类型并调用 pdf-extractor。",
        "fix_suggestion": "在 src/skills/file_reader/reader.rs:42 添加 is_scanned_pdf() 检测"
      }
    ],
    "coverage": 78,
    "report_path": "D:/reports/skill-test-2026-07-10.md"
  },
  "error": null,
  "latency_ms": 23800
}
```

输出字段说明：
- `total/passed/failed/skipped`：测试用例统计
- `results`：每个测试用例的执行结果
- `diagnosis`：失败用例的 LLM 诊断（`diagnose_failures=true` 时）
- `fix_suggestion`：修复建议，含文件路径与行号
- `coverage`：测试覆盖率（如可测量）

## 使用示例

### 示例 1：运行单个技能的全部测试

用户："跑一下 file-reader 技能的测试"

```json
{
  "skill_id": "file-reader",
  "report_format": "markdown"
}
```

运行 `docs/skills/file-reader/tests/` 下的所有测试，生成 Markdown 报告。

### 示例 2：运行全部技能测试

用户："把所有技能的测试都跑一遍"

```json
{
  "parallel": true,
  "timeout_sec": 90,
  "report_format": "json"
}
```

遍历 `docs/skills/` 下所有技能目录，并行运行各自的测试套件，汇总为 JSON 报告。

### 示例 3：失败诊断

用户："file-reader 的测试失败了，帮我看看为什么"

```json
{
  "skill_id": "file-reader",
  "diagnose_failures": true,
  "report_format": "markdown"
}
```

运行测试，对失败用例调用 LLM 分析错误日志与相关源码，给出根因诊断与修复建议。

## 注意事项

- **测试约定**：技能测试应位于 `docs/skills/<skill>/tests/` 目录，且包含
  `run.sh`（或 `run.py`）作为入口。无测试目录的技能会被标记为 `no_tests`。
- **环境隔离**：测试在独立 shell 中执行，环境变量不污染主进程。`env` 参数
  显式注入的变量才会传递给测试进程。
- **超时保护**：单个测试用例超过 `timeout_sec` 会被强制终止并标记为 `failed`，
  错误信息含 `TimeoutExceeded`。
- **并行安全**：`parallel=true` 时各技能测试独立进程运行，但共享文件系统。
  涉及同一文件的测试可能产生竞争，建议技能测试使用临时目录隔离。
- **诊断成本**：`diagnose_failures=true` 会为每个失败用例调用一次 LLM，
  失败用例较多时耗时与成本上升。可先关闭诊断批量跑，再针对关键失败开启。
- **依赖说明**：需要 Python 环境用于测试运行器封装与报告生成（pytest）。
  实际测试执行可能调用各语言原生测试命令（cargo / npm / go）。
