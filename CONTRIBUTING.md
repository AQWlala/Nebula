# 贡献指南 (Contributing to Nebula)

感谢你有兴趣让 Nebula 变得更好！本文档说明如何贡献。

## 行为准则

* 友善、包容、专业
* 假设对方出于好意
* 接受建设性批评
* 关注对社区最有利的事

## 报告 Bug

提交 [GitHub Issue](https://github.com/AQWlala/nebula/issues/new?template=bug.md) 并附：

1. 复现步骤
2. 期望行为
3. 实际行为
4. 截图（如有）
5. 操作系统 + 版本
6. Nebula 版本（`health` command 输出）
7. 日志（`NEBULA_LOG_DIR` 下）

## 提议新特性

提交 Feature Request issue 描述：

1. 问题 / 痛点
2. 建议的解决方案
3. 替代方案
4. 影响的子系统

## 提交 PR

### 流程

1. Fork → 创建特性分支
2. 写代码 + 测试
3. 跑测试：`cargo test && npm test`
4. 跑 lint：`cargo clippy && npm run lint`
5. 跑 typecheck：`cargo check && npm run typecheck`
6. 提交 PR，关联 issue

### 提交信息格式

```
<type>(<scope>): <subject>

<body>

<footer>
```

type: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`

例：

```
feat(memory): add L6 abstraction layer

L6 abstracts patterns across tasks; L7 weaves them into a
long-term narrative.  This commit adds the storage + the
reflection hook.

Refs: #123
```

## 代码风格

* **Rust** — `cargo fmt` + `cargo clippy -D warnings`
* **TypeScript** — Prettier (项目根 `prettier` 配置) + ESLint
* **提交前** — `cargo test` + `npm test` 必须全绿

## 添加新依赖

在 PR 描述里说明：

1. 为什么需要
2. 是否活跃维护
3. license 兼容 MIT
4. 引入后的体积影响（前端要看 `vite build` 输出）

## 架构决策

重大变更（新子系统、breaking API）需要先开一个 *Architecture Decision Record* (ADR) issue 让维护者 review。

## 发布周期

* **Patch** — 随时，bug fix
* **Minor** — 每月 1 号
* **Major** — 不定（breaking change）

## 联系方式

* GitHub Issues — bug / feature
* Discussions — Q&A / 想法
* Discord — `https://discord.gg/nebula-ai`（即将开放）
* Email — `hello@nebula-ai.app`

## 许可证

贡献即同意 [MIT License](LICENSE)。