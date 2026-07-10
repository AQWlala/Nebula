---
name: test-generator
version: 1.0.0
description: |
  自动生成测试用例技能——分析源码自动生成单元测试，覆盖边界条件与异常路径。
  当用户要求"给这个函数生成测试"、"补一下测试用例"、"提高这个模块的覆盖率"时加载此技能。
  通过文件读取 + LLM 调用分析源码，再写入测试文件。对标 Hermes test_generator 能力。
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

# Test Generator 技能（自动生成测试用例）

## 概述

Test Generator 是 Nebula 的代码测试自动化技能，负责读取源码文件、分析
函数签名与逻辑分支、自动生成覆盖正常路径、边界条件、异常场景的单元测试
用例。通过 `file:read` 读取源码、`llm:call` 推理测试逻辑、`file:write`
输出测试文件。

该技能面向开发者：在新功能开发完成后快速补齐测试，或在遗留代码上补测试
以建立重构安全网。生成的测试遵循项目既有测试框架与命名规范。

## 使用场景

- **新功能补测试**：刚写完一个函数，快速生成对应单元测试
- **覆盖率提升**：针对未覆盖的代码路径补充测试用例
- **遗留代码重构前**：先补测试建立安全网，再进行重构
- **边界用例挖掘**：自动识别边界条件（空值、极值、溢出）并生成测试
- **测试框架迁移**：把已有测试迁移到新框架（如 Jest → Vitest）

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `source_path` | string | 是 | 源码文件绝对路径 |
| `framework` | string | 否 | 测试框架：`auto`（默认，自动识别）/ `pytest` / `jest` / `vitest` / `go test` |
| `output_path` | string | 否 | 测试文件保存路径，默认同目录 `__tests__/` 或 `_test.go` |
| `coverage_target` | number | 否 | 目标覆盖率（0-100），默认 80 |
| `include_edge_cases` | boolean | 否 | 是否生成边界用例，默认 `true` |
| `include_mocks` | boolean | 否 | 是否自动生成 mock，默认 `true` |
| `style` | string | 否 | 测试风格：`bdd`（默认）/ `tdd` / `simple` |
| `existing_tests` | string | 否 | 已有测试文件路径，用于风格学习与去重 |

示例输入：
```json
{
  "source_path": "D:/nebula/src/skills/registry.rs",
  "framework": "auto",
  "coverage_target": 85,
  "include_edge_cases": true,
  "include_mocks": true,
  "style": "bdd"
}
```

## 输出

```json
{
  "output": {
    "test_file_path": "D:/nebula/src/skills/registry_test.rs",
    "framework_detected": "cargo test",
    "test_count": 12,
    "tests": [
      {
        "name": "should_register_skill_with_valid_metadata",
        "type": "happy_path",
        "covers": "Registry::register",
        "description": "验证有效元数据时可成功注册技能"
      },
      {
        "name": "should_reject_duplicate_skill_id",
        "type": "edge_case",
        "covers": "Registry::register",
        "description": "重复 ID 时返回错误而非覆盖"
      },
      {
        "name": "should_handle_empty_capabilities_list",
        "type": "edge_case",
        "covers": "Registry::register",
        "description": "空 capabilities 列表应被允许"
      }
    ],
    "estimated_coverage": 87,
    "mocks_generated": ["MockFileSystem", "MockLLMClient"]
  },
  "error": null,
  "latency_ms": 5400
}
```

输出字段说明：
- `test_file_path`：生成的测试文件路径
- `framework_detected`：自动识别的测试框架
- `tests`：测试用例列表，含名称、类型、覆盖目标、描述
- `estimated_coverage`：估算覆盖率（百分比）
- `mocks_generated`：自动生成的 mock 列表

## 使用示例

### 示例 1：为 Rust 模块生成测试

用户："给 registry.rs 补一下测试"

```json
{
  "source_path": "D:/nebula/src/skills/registry.rs",
  "framework": "auto",
  "coverage_target": 80
}
```

技能识别为 Rust 项目，使用 `cargo test` 框架，生成 `registry_test.rs`。

### 示例 2：为 TypeScript 函数生成测试

用户："给这个 util 函数生成 jest 测试"

```json
{
  "source_path": "D:/nebula/src/utils/format.ts",
  "framework": "jest",
  "include_edge_cases": true,
  "style": "bdd"
}
```

生成 `format.test.ts`，使用 BDD 风格（describe/it），含边界用例。

### 示例 3：基于已有测试风格扩展

用户："按现有测试的风格补一下 service 的测试"

```json
{
  "source_path": "D:/nebula/src/services/auth.ts",
  "existing_tests": "D:/nebula/src/services/user.test.ts",
  "framework": "auto"
}
```

技能学习已有测试文件的命名规范、mock 用法、断言风格，生成风格一致的测试。

## 注意事项

- **草稿属性**：生成的测试是"高质量草稿"，可能需要开发者调整断言精度、
  补充业务特定的场景。技能建议运行 `cargo test` / `npm test` 验证通过。
- **框架识别**：通过项目配置文件（Cargo.toml / package.json / go.mod）
  自动识别测试框架。识别失败时需用户显式指定 `framework`。
- **覆盖率估算**：估算值基于代码分支数与生成用例数的启发式计算，
  与实际覆盖率可能有 5-15% 偏差。准确覆盖率需运行 `cargo tarpaulin`
  / `jest --coverage` 等工具。
- **Mock 生成**：自动生成的 mock 基于接口/ trait 定义，复杂依赖可能需要
  手动调整。技能会标注需要人工确认的 mock 点。
- **测试隔离**：生成的测试默认不依赖外部服务（数据库、网络），所有外部
  依赖通过 mock 隔离。集成测试需单独编写。
- **依赖说明**：需要 Python 环境用于源码解析（tree-sitter）与 AST 分析。
  LLM 调用由 Nebula 统一能力处理。
