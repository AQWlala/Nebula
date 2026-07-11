# v2.0.0-feature-completion — Phase 0-3 功能补齐

> **状态**: archived
> **领域**: memory, swarm, llm, security, skills, evolution, os, sync, ui, project
> **提出日期**: 2026-07-08
> **归档日期**: 2026-07-10
> **类型**: 大版本 (功能补齐 + 架构升级)
> **基线版本**: v1.1.4

## 为什么

v1.x 系列完成了 Nebula 的 MVP 和安全加固，但 `DEVELOPMENT_PROPOSAL_v2.0.md` 的评估暴露了系统性问题：

1. **功能缺口大** — 四大支柱（更省钱/更智能/更贴合/更快）的完成度分别为 100%/94%/45%/60%，"更贴合"和"更快"两大支柱严重滞后。OS-Controller 双模式、Hybrid Browser Agent、OAuth 集成、语音交互等关键功能未实现，导致 Nebula 无法覆盖完整工作链路。

2. **技术债务积压** — 19 项技术债务全未开始（0%）。`tauri.ts` 单文件 3190 行、`bootstrap.rs` 1113 行单函数、`memory/` 40+ 子文件平铺等问题严重阻碍可维护性，违反"可读"哲学。

3. **构建与质量阻塞** — 3 个 P0 严重缺陷（digest crate 版本冲突、Windows CI 集成测试被跳过、cargo audit 14 个安全建议被忽略）直接阻碍构建和质量保障。

4. **记忆系统需深化** — L0-L5 层级虽已落地，但"信任三原则"（可读/可编辑/可追溯）仍需强化：Memory Inspector 需升级、LLM Wiki 编译需完善、版本控制和 provenance 需补齐。

5. **蜂群协作不完整** — Master-Orchestrator 框架已就位，但 DAG 画布、事件流协议化、Loop 运行时阶段环、CRDT 同步等关键能力未完成，蜂群无法真正高效协作。

## 做什么

按四个阶段系统性补齐功能与清偿技术债务：

### Phase 0: 地基修复（2-3 周）
- 修复 3 个 P0 构建阻塞（digest 冲突 / CI 集成测试 / cargo audit）
- 修复 4 个严重质量问题（tauri.ts 拆分 / bootstrap.rs 拆分 / 前端测试覆盖 / CI 跨平台）
- 修复 10 个可维护性问题（ESLint / Prettier / tsconfig / 死 feature 清理等）

### Phase 1: 质量闭环（4-6 周）
- 技术债务 P0 清算
- 核心文件测试补齐（bootstrap / gateway / dispatcher / app_config）
- CI 跨平台恢复（Windows + Linux）
- 前端质量重构

### Phase 2: 功能补齐（6-8 周）
- 支柱 C（更贴合）: OS-Controller 双模式、Hybrid Browser Agent、OAuth 集成、语音交互
- 支柱 D（更快）: 8 人格系统、Proactive Engine、WebGL 性能优化
- 支柱 S（贯穿）: WorkflowCanvas、蜂群画布、Event Stream 协议化、Cron 定时引擎
- Loop Engineering 内化
- 端到端体验优化

### Phase 3: 创新扩展（长期）
- v3.0 全自主革命目标的初步探索
- L6 蒸馏层可行性研究（实际实现在 v2.4.0）

## 不做什么

- **不做 v3.0 全自主革命** — Phase 3 仅做可行性研究，实际实现留待 v3.x
- **不做 macOS 风格 UI 重设计** — 那是 v2.3.0 的独立 change
- **不引入新的云端强制依赖** — 所有新增功能遵循本地优先原则
- **不破坏 v1.x 的 API 兼容性** — 增量扩展，不重构公共 API
- **不做移动端** — 桌面端功能补齐优先
