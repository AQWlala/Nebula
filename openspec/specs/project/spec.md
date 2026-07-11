# 项目级约束 行为契约

> **领域**: project
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

本契约定义 Nebula 项目级别的不可逾越约束,涵盖项目身份、数据主权红线、技术栈选型、CI/CD 策略与版本基线。所有领域的子契约必须遵守本文件中的红线约束;任何与本契约冲突的子契约条款无效。

## Requirements

### Requirement: 项目身份
The system SHALL identify itself as "Nebula" across all user-facing surfaces, package metadata, and build artifacts.
- Cargo 包名 `nebula`,版本 `2.3.0`(见 `src-tauri/Cargo.toml`)
- npm 包名 `nebula`,版本 `2.3.0`(见 `package.json`)
- Tauri `productName` 为 `nebula`,`identifier` 为 `com.nebula.desktop`
- 应用描述:"Nebula — A local-first AI assistant that evolves with your knowledge"
- 项目代码库名称不得出现 `nine_snake` 或任何非 Nebula 别名

#### Scenario: 用户查询应用身份
- **WHEN** 用户在设置页面或命令面板中查看"关于"
- **THEN** 显示 "Nebula v2.3.0"
- **AND** 不出现任何其他项目名或别名

#### Scenario: 构建产物命名
- **WHEN** CI 执行 `tauri build` 生成安装包
- **THEN** 产物文件名以 `nebula_` 前缀命名(如 `nebula_2.3.0_x64-setup.exe`)
- **AND** 可执行文件名为 `nebula.exe`

### Requirement: 数据主权红线
The system MUST NOT transmit user data to third-party servers, MUST NOT introduce closed-source correctness checkers, and MUST prioritize local execution.
- 用户记忆、对话、知识图谱存储在本地 SQLite + LanceDB
- LLM 调用优先走本地 Ollama;远端调用(Anthropic/OpenAI 兼容)仅在用户显式配置 API Key 后启用
- 进化引擎(Evolution/SoulCompile/Classifier)强制走本地 Ollama,忽略非本地 override
- 不引入任何闭源 Checker 模块;所有校验逻辑开源可审计
- E2EE 加密保证多设备同步时数据对中继服务器不可见

#### Scenario: 本地优先 LLM 调用
- **WHEN** 进化引擎触发 LLM 调用(WorkType = Evolution/SoulCompile/Classifier)
- **THEN** UnifiedModelDispatcher 强制路由到本地 Ollama
- **AND** 即使用户在 `models.json` 中配置了非本地 override,该 override 被忽略

#### Scenario: 用户数据不出域
- **WHEN** 用户与助手对话产生记忆条目
- **THEN** 记忆写入本地 SQLite,不发送到任何第三方服务器
- **AND** 仅当用户显式启用多设备同步时,数据经 E2EE 加密后通过中继传输

### Requirement: 技术栈
The system SHALL be built on Tauri 2.0 + Rust + Preact + TypeScript.
- 后端:Rust 2021 edition,MSRV 1.75,Tauri 2.0 框架
- 前端:Preact 10.x + TypeScript + Vite 5.x + Tailwind CSS 3.x
- 存储:rusqlite 0.31(bundled) + LanceDB 0.31(optional,feature `vector-store`)
- 加密:aes-gcm 0.10 + x25519-dalek 2.0 + hkdf 0.13 + sha2 0.11
- DAG:petgraph 0.6(master-orchestrator feature)
- 不使用 Electron 或任何基于 Chromium 的重运行时

#### Scenario: 技术栈一致性校验
- **WHEN** 开发者提交 PR 修改 `Cargo.toml` 或 `package.json`
- **THEN** CI 校验核心依赖版本未被降级
- **AND** 不得引入 Electron / NW.js 等替代运行时

### Requirement: CI 构建策略
The system SHALL only build for windows-x86_64 target using NSIS bundling in CI.
- GitHub Actions `release.yml` 矩阵仅包含 `windows-x86_64`
- 目标三元组:`x86_64-pc-windows-msvc`
- 打包格式:`nsis`(不生成 msi / app / dmg / appimage)
- macOS / Linux 构建矩阵已注释,恢复时需手动启用
- 最小化构建(`--no-default-features`)必须编译通过,作为安全网

#### Scenario: CI 触发 release 构建
- **WHEN** 推送 `v*.*.*` 格式的 tag 或手动触发 workflow
- **THEN** 仅在 `windows-latest` runner 上构建 `x86_64-pc-windows-msvc` 目标
- **AND** 生成 NSIS 安装包并上传为 artifact
- **AND** 生成 `latest.json` 供 Tauri updater 使用,仅包含 `windows-x86_64` 平台条目

#### Scenario: 最小化构建安全网
- **WHEN** 执行 `cargo build --no-default-features`
- **THEN** 编译成功,剥离 LanceDB 和 gRPC 依赖
- **AND** 向量搜索降级为内存线性余弦扫描,gRPC 服务器不编译

### Requirement: 版本基线
The system SHALL track version 2.3.0 as the current release baseline across all manifests.
- `src-tauri/Cargo.toml`: `version = "2.3.0"`
- `package.json`: `"version": "2.3.0"`
- `src-tauri/tauri.conf.json`: `"version": "2.3.0"`
- 三个文件版本必须一致;版本不一致视为发布阻断缺陷

#### Scenario: 版本一致性校验
- **WHEN** 执行发布前检查
- **THEN** Cargo.toml / package.json / tauri.conf.json 三处版本号均为 `2.3.0`
- **AND** 不一致时 RELEASE_CHECKLIST 标记为阻断
