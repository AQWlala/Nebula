---
name: api-tester
version: 1.0.0
description: |
  API 接口测试技能——测试 REST API 接口，生成测试报告。当用户要求"测一下
  这个接口"、"测试 API"、"这个 API 能不能用"时加载此技能。通过 net:http
  发起请求、exec:shell 运行测试脚本、llm:call 生成诊断报告。对标 OpenAkita
  api_tester 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "net:http", "exec:shell"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# API Tester 技能（API 接口测试）

## 概述

API Tester 是 Nebula 的接口测试技能，帮助开发者快速验证 REST API 的
可用性与正确性。它通过 `net:http` 能力发起 HTTP 请求，支持 GET / POST /
PUT / DELETE 等全方法，可批量执行测试用例并断言响应状态码、Header 与
Body；通过 `exec:shell` 运行可选的测试脚本（如 curl / Python 脚本）进行
扩展验证；最后由 `llm:call` 综合分析响应并生成结构化测试报告。

技能流程：解析接口定义 → 构造请求 → 发起 HTTP 调用 → 断言响应 →
（可选）运行扩展脚本 → LLM 诊断异常 → 生成报告。支持鉴权、环境变量
注入与测试用例集合，适合接口联调与回归测试。

## 使用场景

- **接口联调**：开发完成后快速验证接口是否正常返回
- **回归测试**：接口升级后批量跑用例确认无回归
- **异常诊断**：接口报错时自动分析响应并给出修复建议
- **接口文档验证**：对照 OpenAPI 文档验证实际响应是否符合预期
- **环境切换测试**：在 dev / staging / prod 不同环境间对比接口行为
- **性能初探**：测量接口响应时间，定位慢接口

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `url` | string | 是 | 接口地址，如 `https://api.example.com/v1/users` |
| `method` | string | 否 | HTTP 方法：`GET`（默认）/ `POST` / `PUT` / `DELETE` / `PATCH` |
| `headers` | object | 否 | 请求头，如 `{"Authorization": "Bearer xxx"}` |
| `body` | object | 否 | 请求体（POST / PUT / PATCH） |
| `expected_status` | number | 否 | 预期状态码，默认 200 |
| `assertions` | object[] | 否 | 断言列表，如 `{"path": "data.id", "op": "eq", "value": 1}` |
| `test_cases` | object[] | 否 | 批量测试用例，每项含独立 url / method / body |
| `auth` | object | 否 | 鉴权配置：`{"type": "bearer", "token": "xxx"}` |
| `timeout_sec` | number | 否 | 单请求超时秒数，默认 30 |
| `output_path` | string | 否 | 报告保存路径 |
| `diagnose` | boolean | 否 | 是否对失败用例做 LLM 诊断，默认 `true` |

示例输入：
```json
{
  "url": "https://api.example.com/v1/users",
  "method": "GET",
  "headers": {"Accept": "application/json"},
  "auth": {"type": "bearer", "token": "eyJhbGciOi..."},
  "expected_status": 200,
  "assertions": [
    {"path": "data.length", "op": "gte", "value": 1},
    {"path": "data[0].id", "op": "exists"}
  ],
  "timeout_sec": 15,
  "output_path": "D:/reports/api-test-users.md",
  "diagnose": true
}
```

## 输出

```json
{
  "output": {
    "url": "https://api.example.com/v1/users",
    "method": "GET",
    "status": 200,
    "duration_ms": 340,
    "response_size": 2840,
    "assertions": [
      {"path": "data.length", "op": "gte", "value": 1, "passed": true, "actual": 25},
      {"path": "data[0].id", "op": "exists", "passed": true}
    ],
    "all_passed": true,
    "response_preview": "{\"data\":[{\"id\":1,\"name\":\"alice\"},...]}",
    "report_path": "D:/reports/api-test-users.md",
    "diagnosis": null
  },
  "error": null,
  "latency_ms": 1200
}
```

输出字段说明：
- `status`：实际响应状态码
- `duration_ms`：请求耗时
- `assertions`：每个断言的执行结果（含实际值）
- `all_passed`：是否全部断言通过
- `response_preview`：响应体预览（前 500 字符）
- `diagnosis`：失败用例的 LLM 诊断（`diagnose=true` 且有失败时）

## 使用示例

### 示例 1：单接口快速验证

用户："测一下这个 GET 接口能不能用"

```json
{
  "url": "https://api.example.com/v1/health",
  "method": "GET",
  "expected_status": 200,
  "timeout_sec": 10
}
```

发起 GET 请求并验证状态码，快速判断接口可用性。

### 示例 2：带鉴权的 POST 接口测试

用户："测试一下创建用户的接口，带上 token"

```json
{
  "url": "https://api.example.com/v1/users",
  "method": "POST",
  "auth": {"type": "bearer", "token": "eyJhbGciOi..."},
  "body": {"name": "alice", "email": "alice@example.com"},
  "expected_status": 201,
  "assertions": [
    {"path": "data.id", "op": "exists"},
    {"path": "data.name", "op": "eq", "value": "alice"}
  ],
  "diagnose": true
}
```

发起带 Bearer 鉴权的 POST 请求，断言返回的用户对象字段正确。

### 示例 3：批量回归测试

用户："把这几个接口都测一遍，确认升级后没回归"

```json
{
  "test_cases": [
    {"url": "https://api.example.com/v1/users", "method": "GET", "expected_status": 200},
    {"url": "https://api.example.com/v1/orders", "method": "GET", "expected_status": 200},
    {"url": "https://api.example.com/v1/products", "method": "GET", "expected_status": 200}
  ],
  "auth": {"type": "bearer", "token": "eyJhbGciOi..."},
  "diagnose": true,
  "output_path": "D:/reports/api-regression-2026-07-10.md"
}
```

批量执行多个接口测试，汇总为 Markdown 回归报告，对失败用例自动诊断。

## 注意事项

- **网络依赖**：本技能依赖 `net:http` 能力发起网络请求。离线环境下
  技能将拒绝加载并提示用户检查网络连接。
- **SSRF 防护**：HTTP 请求经过 Nebula 的 SSRF 校验层，拒绝访问内网
  地址（127.0.0.1 / 10.x / 192.168.x 等）与非常规端口。需测试内网
  接口时，用户须在配置中显式加入白名单。
- **鉴权安全**：`auth.token` 等敏感信息仅在内存中传递给 HTTP 能力，
  不会写入测试报告明文（报告中以 `***` 脱敏）。建议通过环境变量注入
  token，避免在配置中硬编码。
- **超时保护**：单请求超过 `timeout_sec` 会被强制终止并标记为 `failed`。
  批量测试时建议合理设置超时，避免单接口拖慢整体。
- **断言路径**：`assertions.path` 使用 JSONPath 语法定位响应字段。
  响应为非 JSON 格式时，仅支持状态码断言。
- **扩展脚本**：`exec:shell` 用于运行可选的扩展测试脚本（如复杂断言、
  数据库校验）。脚本在独立 shell 中执行，环境变量不污染主进程。
- **速率限制**：批量测试默认每请求间隔不少于 200ms，避免压垮目标
  服务。压测场景请使用专用工具，本技能不适合高并发场景。
- **依赖说明**：需要 Python 环境用于请求封装与报告生成（requests）。
  实际 HTTP 调用由 Nebula 的 `net:http` 能力原生处理。
