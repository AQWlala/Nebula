# 推迟的大版本依赖升级

> **创建日期**: 2026-07-11
> **状态**: 跟踪中
> **类型**: 技术债务

## 背景

以下依赖的大版本升级有 breaking changes，已在 `dependabot.yml` 中忽略 major 版本。
patch/minor 版本仍自动更新。每个依赖需要在大版本发布时手动评估迁移。

## 跟踪列表

### Rust 依赖

| 依赖 | 当前版本 | 目标版本 | 忽略原因 | 迁移工作量 | 优先级 |
|------|---------|---------|---------|-----------|--------|
| rand | 0.8.6 | 0.10.2 | API 完全重构,24个编译错误 | 中(需要重写所有 RNG 调用) | 低 |
| aes-gcm | 0.10.3 | 0.11.0 | `from_slice` 被弃用,需改 `TryFrom` | 小(4个文件,6处调用) | 中 |
| lru | 0.16.4 | 0.18.1 | Dependabot 解析失败(PR #6) | 未知(需手动测试) | 低 |
| reqwest | 0.12.28 | 0.13.4 | major 升级编译失败(PR #31) | 中(需检查 API 变更) | 中 |
| opentelemetry | 0.24.0 | 0.32.0 | major 跨 8 个版本,API 大改(PR #32) | 大(需全面迁移) | 低 |
| pdf-extract | 0.7.12 | 0.12.0 | major 跨 5 个版本,API 大改(PR #28) | 中(需检查文本提取 API) | 低 |
| prometheus | 0.13.4 | 0.14.0 | major 升级,API 大改(PR #25) | 小(需检查指标 API) | 中 |
| axum | 0.7.9 | 0.8.9 | major 升级,API 大改(PR #38) | 中(需检查路由 API) | 中 |
| arrow-array | 58.3.0 | 59.1.0 | major 升级,API 大改(PR #37) | 中(需检查数组 API) | 低 |
| prost | 0.13.5 | 0.14.4 | major 升级,API 大改(PR #35) | 中(需检查 proto 编译) | 中 |
| getrandom | 0.2.17 | 0.4.3 | major 升级,API 大改(PR #41) | 中(需检查 RNG 调用) | 中 |
| notify | 6.1.1 | 8.2.0 | major 跨 2 个版本,API 大改(PR #40) | 中(需检查通知 API) | 中 |
| petgraph | 0.6.5 | 0.8.3 | major 升级,API 大改(PR #39) | 中(需检查图算法 API) | 低 |

### npm 依赖

| 依赖 | 当前版本 | 目标版本 | 忽略原因 | 迁移工作量 | 优先级 |
|------|---------|---------|---------|-----------|--------|
| typescript | 6.0.3 | 7.0.2 | 大版本升级 | 未知(需检查类型推断变化) | 中 |
| eslint | 8.57.1 | 10.7.0 | 大版本升级,配置格式变化 | 中(扁平化配置迁移) | 低 |
| @typescript-eslint/eslint-plugin | 6.21.0 | 8.63.0 | 大版本升级 | 中(规则变更) | 低 |
| @typescript-eslint/parser | 6.21.0 | 8.63.0 | 大版本升级 | 中(配合 eslint 升级) | 低 |
| tailwindcss | 3.4.19 | 4.3.2 | major 升级,配置系统完全重写(PR #30) | 大(CSS-in-JS 配置迁移) | 低 |
| vitest | 1.6.1 | 4.1.10 | major 跨 3 个版本,API 大改(PR #26) | 中(需检查测试 API) | 低 |
| @types/node | 20.19.43 | 26.1.1 | major 跨 6 个版本,API 大改(PR #34) | 中(需检查类型定义) | 低 |
| @vitest/coverage-v8 | 1.6.1 | 4.1.10 | major 跨 3 个版本,配合 vitest 忽略(PR #42) | 中(需检查覆盖率 API) | 低 |
| vite | 5.4.21 | 8.1.4 | major 跨 3 个版本,构建系统大改(PR #46) | 大(需检查 Vite 插件 API) | 低 |

## 迁移检查清单

每次大版本发布时,按以下步骤评估:

### 1. 评估阶段
- [ ] 查看依赖的 CHANGELOG/MIGRATION guide
- [ ] 在分支上尝试升级:`cargo update -p <dep>` 或 `npm install <dep>@latest`
- [ ] 运行 `cargo check` / `tsc --noEmit` 检查 breaking changes
- [ ] 运行完整测试套件

### 2. 迁移阶段
- [ ] 修复所有编译错误
- [ ] 修复所有 clippy warning
- [ ] 修复所有测试失败
- [ ] 更新相关文档

### 3. 合并阶段
- [ ] 从 `dependabot.yml` 中移除该依赖的 ignore 规则
- [ ] 提交 PR,确认 CI 全绿
- [ ] 合并后在此文件中标记完成

## 完成记录

(暂无)
