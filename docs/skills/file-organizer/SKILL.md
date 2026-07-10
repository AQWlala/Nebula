---
name: file-organizer
version: 1.0.0
description: |
  桌面/文件夹自动整理技能——按类型、日期或项目分类整理散乱文件。
  当用户要求"整理一下桌面"、"把这个文件夹的文件归类"、"按日期分组文件"时加载此技能。
  通过文件读取 + 移动能力，将杂乱目录重组为清晰的层级结构。对标 CoPaw file_organizer 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "file:read", "file:write", "file:move"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# File Organizer 技能（桌面/文件夹自动整理）

## 概述

File Organizer 是 Nebula 的桌面整理技能，负责将散乱的文件夹（尤其是桌面、
下载目录）按规则重组为清晰的层级结构。支持按文件类型、修改日期、项目关键词
三种主要分类策略，通过 `file:read` 扫描目录、`file:move` 安全移动文件、
`file:write` 生成整理报告。

该技能解决知识工作者"桌面一团乱"的高频痛点：截图、文档、压缩包、安装包
混杂在一起。技能默认采用保守策略——先列计划再执行，避免误移动用户重要文件。

## 使用场景

- **桌面清理**：桌面堆满了几十个文件，按类型分到 Documents/Images/Archives 等子目录
- **下载目录归档**：Downloads 目录按月份分组，旧文件归档到 `2025-archives/`
- **项目素材归集**：把分散的某个项目相关文件按关键词聚拢到统一目录
- **重复文件识别**：扫描同名/同尺寸文件并提示用户处理
- **批量重命名**：按统一命名规范（如 `2026-07-10-report.pdf`）批量重命名

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 待整理目录绝对路径，如 `D:/Desktop` |
| `strategy` | string | 否 | 分类策略：`type`（默认）/ `date` / `project` |
| `dry_run` | boolean | 否 | 是否仅预览不执行，默认 `true` |
| `project_keywords` | string[] | 否 | `strategy=project` 时的项目关键词列表 |
| `date_format` | string | 否 | `strategy=date` 时的分组粒度：`month`（默认）/ `week` / `year` |
| `exclude` | string[] | 否 | 排除的文件名/扩展名，如 `["*.tmp", "desktop.ini"]` |

示例输入：
```json
{
  "path": "C:/Users/nebula/Desktop",
  "strategy": "type",
  "dry_run": true,
  "exclude": ["*.lnk", "desktop.ini"]
}
```

## 输出

```json
{
  "output": {
    "scanned": 87,
    "moved": 0,
    "skipped": 87,
    "plan": [
      {"file": "report.pdf", "from": "Desktop/", "to": "Desktop/Documents/"},
      {"file": "screenshot-01.png", "from": "Desktop/", "to": "Desktop/Images/"}
    ],
    "categories": ["Documents", "Images", "Archives", "Installers", "Others"],
    "report_path": "C:/Users/nebula/Desktop/_organize-report.md"
  },
  "error": null,
  "latency_ms": 1500
}
```

输出字段说明：
- `scanned`：扫描到的文件总数
- `moved`：实际移动的文件数（dry_run 模式下为 0）
- `plan`：整理计划，每条记录含源路径与目标路径
- `categories`：本次生成的分类目录列表
- `report_path`：整理报告 Markdown 文件路径

## 使用示例

### 示例 1：预览桌面整理方案

用户："帮我看看桌面该怎么整理"

```json
{
  "path": "C:/Users/nebula/Desktop",
  "strategy": "type",
  "dry_run": true
}
```

技能扫描后返回整理计划，列出每个文件将归入哪个分类目录，不实际移动。
用户确认后再以 `dry_run: false` 执行。

### 示例 2：按项目关键词归集素材

用户："把所有跟 Nebula 项目相关的文件归到一起"

```json
{
  "path": "D:/workspace",
  "strategy": "project",
  "project_keywords": ["nebula", "skill", "tauri"],
  "dry_run": false
}
```

技能遍历目录树，匹配文件名/内容含关键词的文件，移动到 `D:/workspace/nebula-project/`。

### 示例 3：按月份归档下载目录

用户："Downloads 按月份归档一下"

```json
{
  "path": "C:/Users/nebula/Downloads",
  "strategy": "date",
  "date_format": "month",
  "dry_run": false
}
```

按文件修改时间分组到 `2026-07/`、`2026-06/` 等子目录。

## 注意事项

- **默认预览**：`dry_run` 默认为 `true`，必须显式设为 `false` 才会实际移动文件，
  避免误操作。即便执行后也可通过报告文件回溯。
- **系统文件保护**：自动排除 `desktop.ini`、`.DS_Store`、`Thumbs.db` 等系统文件，
  以及符号链接与隐藏文件，避免破坏系统结构。
- **冲突处理**：目标位置已存在同名文件时，自动追加 `_1`、`_2` 后缀而非覆盖。
- **跨盘移动**：跨磁盘移动会触发复制+删除，耗时较长；同盘移动为原子操作。
- **路径白名单**：拒绝整理系统关键目录（`C:/Windows`、`C:/Program Files` 等），
  仅允许用户目录与显式授权的工作区。
- **依赖说明**：需要 Python 环境用于目录遍历与文件元信息读取（os/pathlib/shutil）。
