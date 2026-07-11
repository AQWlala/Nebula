# 归档记录 — v2.3.0-macos-redesign

> **归档时间**: 2026-07-11
> **归档人**: nebula:archive
> **合并到的版本**: v2.3.0 (大版本)
> **版本文件**: `versions/major/v2.3.0.md`

## 归档摘要

本次归档将 v2.3.0 macOS 风格前端重设计从进行中状态转为已完成状态。

## 合并的 Delta

- `specs/ui/delta.md` → 合并到 `specs/ui/spec.md`
  - ADDED: 导航分组、顶部 Titlebar、毛玻璃降级开关 (3 个新 Requirement)
  - MODIFIED: 侧边栏布局、状态栏、默认视图、内容区卡片样式、色彩系统 (5 个 Requirement 修改)
  - REMOVED: (无)

## specs/ 更新

`specs/ui/spec.md` 的以下字段已更新：
- `> **最后更新**:` 改为 `2026-07-11 (merged from change: v2.3.0-macos-redesign)`

## 验证结果

- [x] cargo test --lib 全通过 (本次纯前端变更,未触及 Rust)
- [x] npm test 全通过 (15 个前端测试通过,含 5 个修复后的测试)
- [x] tsc --noEmit 无错误
- [x] CI 全绿 (Run: 29132407863)

## 后续

- v2.3.1 (小版本) 修复了本次重设计导致的 5 个前端测试失败,详见 `versions/minor/v2.3.1.md`
- 毛玻璃降级开关的默认值在 v2.3.1 中根据用户反馈保持"开启"
