# OpenSpec — Nebula 的规范驱动开发系统

> **版本**: v1.0
> **基于**: [OpenSpec](https://github.com/Fission-AI/OpenSpec) by Fission-AI
> **定制**: 加入 Nebula 专用工件(记忆影响评估 + 数据主权检查)

## 目录结构

- `specs/` — 系统当前行为的"真实来源"(source of truth)
- `changes/` — 提议中的修改(每个修改一个文件夹)
- `changes/archive/` — 已完成的修改归档
- `versions/major/` — 大版本更迭 (1.0→1.1, 2.0, 2.1)
- `versions/minor/` — 小版本更迭 (1.0.1, 2.0.1)
- `schema/` — Nebula 专用工作流 schema

## 快速开始

1. 读取 `specs/<domain>/spec.md` 了解当前行为
2. 在 `changes/` 创建变更提案
3. 实现并验证
4. 归档并创建版本文件

详见 `OPENSPEC.md`。
