---
name: git-helper
version: 1.0.0
description: |
  Git 操作辅助技能——辅助 Git 操作（commit / branch / merge 建议）。当用户
  要求"帮我写 commit message"、"该怎么分支"、"这个 merge 冲突怎么解"
  时加载此技能。通过 exec:shell 能力执行 git 命令（只读查询为主），
  再用 llm:call 能力生成规范 commit message / 分支策略 / 冲突解决方案。
  对标 OpenClaw Git 辅助能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "exec:shell"]
transport: local
dependencies: []
eligibility:
  bins: ["git"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Git Helper 技能（Git 操作辅助）

## 概述

Git Helper 是 Nebula 面向开发者的版本控制辅助技能，降低 Git 的使用门槛。
它不是替代 Git，而是在 Git 之上加一层"智能"：分析 diff 生成规范 commit
message、根据变更范围建议分支策略、解读冲突给出合并方案、整理 commit
历史生成 changelog 草稿。

技能以**只读查询为主**（status / diff / log），写操作（commit / merge /
push）需用户确认后执行，且遵循 ValuesLayer 约束：push force / reset hard
等危险操作需显式确认，main 分支硬编码保护。

## 使用场景

- **commit message 生成**：stage 了改动，不知道怎么写规范的 message
- **分支策略建议**：不确定该从哪里切分支、怎么命名
- **冲突解决**：merge / rebase 遇到冲突，不知道保留哪边
- **PR 描述生成**：提交 PR 前根据 diff 生成描述模板
- **历史整理**：把零散的 commit 整理成可读的 changelog
- **回退建议**：改错了，想知道用 revert 还是 reset
- **Git 教学**：不熟悉某个 Git 操作，让 Nebula 解释并指导

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `repo` | string | 是 | Git 仓库根目录绝对路径 |
| `action` | string | 是 | 操作类型：`commit_msg` / `branch_advice` / `conflict_help` / `pr_desc` / `changelog` / `undo_advice` |
| `scope` | string | 否 | 操作范围，如分支名、commit 范围、文件路径 |
| `convention` | string | 否 | commit 规范：`conventional` / `gitmoji` / `custom`（默认 conventional） |

示例输入：
```json
{
  "repo": "D:/projects/nebula",
  "action": "commit_msg",
  "convention": "conventional"
}
```

## 输出

```json
{
  "output": {
    "action": "commit_msg",
    "staged_files": ["src/skills/executor.rs", "src/skills/mod.rs"],
    "suggestions": [
      {
        "message": "feat(skills): add local executor with timeout support",
        "body": "- Add LocalExecutor struct implementing SkillExecutor trait\n- Support configurable timeout via timeout_ms field\n- Add path validation before skill execution",
        "breaking": false
      },
      {
        "message": "feat(skills): implement skill execution timeout and path validation",
        "body": "...",
        "breaking": false
      }
    ],
    "diff_summary": "新增 LocalExecutor，支持超时与路径校验"
  },
  "error": null,
  "latency_ms": 3400
}
```

输出字段说明：
- `suggestions`：建议列表（commit message / 分支名 / 冲突方案等）
- `diff_summary`：变更内容的一句话摘要

## 使用示例

### 示例 1：生成 commit message

用户："我 stage 了改动，帮我写个 commit message"

```json
{
  "repo": "D:/projects/nebula",
  "action": "commit_msg",
  "convention": "conventional"
}
```

分析 staged diff，生成符合 Conventional Commits 规范的 message 选项。

### 示例 2：冲突解决建议

用户："merge 的时候冲突了，帮我看看怎么解"

```json
{
  "repo": "D:/projects/nebula",
  "action": "conflict_help",
  "scope": "src/skills/protocol.rs"
}
```

读取冲突文件的 conflict markers，分析双方意图，给出保留建议。

### 示例 3：生成 PR 描述

用户："准备提 PR 了，帮我写个描述"

```json
{
  "repo": "D:/projects/nebula",
  "action": "pr_desc",
  "scope": "feature/skill-market..main"
}
```

分析分支与 main 的 diff，生成含"改了什么/为什么改/怎么测试"的 PR 模板。

## 注意事项

- **写操作确认**：涉及写操作（commit / merge / push / reset）时，技能
  仅生成建议，不自动执行。用户确认后通过 `exec:shell` 执行，执行前
  再次显示命令全文供用户确认。
- **危险操作保护**：`git push --force` / `git reset --hard` /
  `git checkout -- .` / `git branch -D` 等危险操作受 ValuesLayer.Deny
  名单约束，必须用户显式确认且不在 main/master 分支。
- **main 分支保护**：禁止直接向 main/master 分支 commit 或 push。
  建议通过 feature 分支 + PR 流程合并。
- **大仓库性能**：超大仓库（>10万 commit）的 log 查询可能较慢，建议
  限定 `scope` 范围（如 `--since="1 week ago"`）。
- **LFS 支持**：Git LFS 文件的 diff 无法正常解析，技能会跳过 LFS
  指针文件并提示用户。
- **子模块**：含子模块的仓库，技能默认只分析主仓库。如需分析子模块，
  请在 `repo` 参数中指定子模块路径。
- **隐私保护**：仓库内容与 git 历史仅在本地处理。不访问远程仓库
  （不执行 fetch / pull），除非用户显式要求。
- **平台兼容**：Windows 上 git 命令通过 `git.exe` 执行，路径分隔符
  自动转换为正斜杠。SSH 配置遵循 Nebula 约定（port 443）。
- **依赖说明**：仅需 git 可执行文件在 PATH 中。不依赖 Python 或其他
  运行时。
