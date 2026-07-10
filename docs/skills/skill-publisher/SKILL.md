---
name: skill-publisher
version: 1.0.0
description: |
  技能发布技能——将本地技能发布到 GitHub Gist 或 Nebula 技能市场。
  当用户要求"把这个技能发到 Gist"、"发布我的技能到市场"、"分享这个 skill"时加载此技能。
  通过文件读取 + HTTP 能力完成发布，需 GITHUB_TOKEN 环境变量。对标现有 publisher.rs 模块。
author: Nebula Project
status: stable
capabilities: ["llm:call", "net:http", "file:read"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: ["GITHUB_TOKEN"]
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Skill Publisher 技能（技能发布到 Gist/市场）

## 概述

Skill Publisher 是 Nebula 的技能分发技能，负责将本地开发完成的技能
（`SKILL.md` + 附属资源）发布到 GitHub Gist 作为可分享的公开链接，
或提交到 Nebula 官方技能市场供其他用户安装。通过 `file:read` 读取技能
内容、`net:http` 调用 GitHub API 或市场 API 完成上传。

该技能面向技能开发者：开发完成后一键发布，无需手动打包上传。发布前会
自动校验 frontmatter 格式、必填字段、目录结构，避免发布残缺技能。

## 使用场景

- **Gist 分享**：把技能发到 Gist，得到一个链接分享给同事
- **市场发布**：提交技能到 Nebula 市场，供全球用户安装
- **版本更新**：已发布技能的新版本推送
- **私有分发**：发布到私有 Gist，仅指定人可见
- **批量发布**：一次性发布多个技能

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `skill_path` | string | 是 | 技能目录绝对路径，含 SKILL.md |
| `target` | string | 是 | 发布目标：`gist` / `marketplace` |
| `visibility` | string | 否 | Gist 可见性：`public`（默认）/ `secret`，仅 `target=gist` |
| `update` | boolean | 否 | 是否更新已存在的发布，默认 `false` |
| `gist_id` | string | 否 | `update=true` 且 `target=gist` 时的目标 Gist ID |
| `marketplace_category` | string | 否 | 市场分类，如 `productivity` / `devtools` / `research` |
| `dry_run` | boolean | 否 | 仅校验不发布，默认 `false` |
| `notes` | string | 否 | 发布说明（changelog） |

示例输入：
```json
{
  "skill_path": "D:/nebula/docs/skills/file-organizer",
  "target": "gist",
  "visibility": "public",
  "dry_run": false,
  "notes": "v1.0.0 首次发布"
}
```

## 输出

```json
{
  "output": {
    "skill_name": "file-organizer",
    "version": "1.0.0",
    "target": "gist",
    "validation": {
      "valid": true,
      "errors": [],
      "warnings": []
    },
    "published": true,
    "gist": {
      "id": "abc123def456",
      "url": "https://gist.github.com/nebula-user/abc123def456",
      "raw_url": "https://gist.githubusercontent.com/nebula-user/abc123def456/raw/SKILL.md",
      "visibility": "public",
      "files_published": ["SKILL.md"]
    },
    "install_command": "nebula skill install gist:abc123def456"
  },
  "error": null,
  "latency_ms": 3200
}
```

输出字段说明：
- `validation`：发布前校验结果，含错误与警告
- `gist` / `marketplace`：发布目标的具体信息（URL、ID 等）
- `install_command`：其他用户安装此技能的命令

## 使用示例

### 示例 1：发布技能到 Gist

用户："把 file-organizer 技能发到 Gist 上"

```json
{
  "skill_path": "D:/nebula/docs/skills/file-organizer",
  "target": "gist",
  "visibility": "public"
}
```

读取技能目录，校验 frontmatter，调用 GitHub API 创建 Gist，返回可分享链接。

### 示例 2：发布到 Nebula 市场

用户："把这个技能发布到官方市场"

```json
{
  "skill_path": "D:/nebula/docs/skills/clipboard-manager",
  "target": "marketplace",
  "marketplace_category": "productivity",
  "notes": "剪贴板增强 v1.0.0"
}
```

提交技能到 Nebula 市场审核流程，附带分类与发布说明。

### 示例 3：更新已发布技能

用户："file-reader 有新版本，更新一下 Gist"

```json
{
  "skill_path": "D:/nebula/docs/skills/file-reader",
  "target": "gist",
  "update": true,
  "gist_id": "abc123def456",
  "notes": "v1.1.0 修复 PDF 编码问题"
}
```

根据 gist_id 更新已有 Gist 内容，保留原 URL。

## 注意事项

- **Token 必需**：发布到 Gist 需要 `GITHUB_TOKEN` 环境变量（具 `gist` scope）。
  发布到市场需要 Nebula 账户 token（在应用设置中配置）。缺失 token 时拒绝执行。
- **发布前校验**：技能会校验 frontmatter 必填字段（name/version/description/
  capabilities/transport/eligibility）、SKILL.md body 完整性、目录结构合规性。
  校验失败时不发布，返回详细错误列表。
- **文件打包**：Gist 模式仅发布 `SKILL.md` 单文件；市场模式支持发布完整
  目录（含 `templates/`、`tests/` 等附属资源，打包为 tar.gz）。
- **Gist 大小限制**：单个 Gist 文件不超过 1 MB。超过时技能会拆分为多个
  文件或提示用户精简。
- **市场审核**：发布到市场后需通过审核才会公开可见。审核结果通过 Nebula
  通知推送给作者。
- **版本管理**：建议每次发布递增 `version` 字段。相同版本号重复发布会
  被市场拒绝，但 Gist 允许覆盖。
- **撤销发布**：可通过 `update=false` + 删除 Gist / 市场下架流程撤销。
  Gist 删除调用 GitHub API，市场下架需在作者后台操作。
- **依赖说明**：需要 Python 环境用于目录打包与 HTTP 请求（httpx）。
  GitHub API 与市场 API 调用通过 `net:http` 能力统一处理。
