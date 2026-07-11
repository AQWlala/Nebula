# 技能系统 行为契约

> **领域**: skills
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

技能系统(Skills)是 Nebula 的程序性记忆子系统,负责技能的创建、发现、执行、评分与市场分发。技能以 SKILL.md 格式定义(YAML frontmatter + Markdown 正文),存放于 `docs/skills/<name>/SKILL.md`,在能力沙箱中执行,支持热更新与自动发明。

## Requirements

### Requirement: SKILL.md 格式
The system SHALL define skills using the SKILL.md format: YAML frontmatter + Markdown body.
- frontmatter 必填字段:`name`, `version`, `description`, `author`, `status`, `capabilities`, `transport`, `dependencies`, `eligibility`, `min_nebula_version`
- `capabilities` 声明技能所需能力(如 `llm:call`, `net:http`, `exec:shell`)
- `eligibility` 声明运行资格(bins / env / os)
- Markdown 正文包含:概述、使用场景、输入参数表、输出格式、使用示例、注意事项
- 内置技能存放于 `docs/skills/<name>/SKILL.md`(如 `api-tester`, `code-review`, `web-search` 等 27 个技能)

#### Scenario: 技能加载与解析
- **WHEN** SkillEngine 加载一个 SKILL.md 文件
- **THEN** 解析 YAML frontmatter 为 `SkillManifest`,解析 Markdown 正文为说明文档
- **AND** 校验必填字段缺失时返回 `SkillSpecReport` 错误

#### Scenario: 能力声明与资格检查
- **WHEN** 技能声明 `capabilities: ["exec:shell"]` 且 `eligibility.bins: ["python"]`
- **THEN** 执行前检查系统是否具备 `python` 可执行文件
- **AND** 缺失时拒绝执行并提示用户安装依赖

### Requirement: 技能目录
The system SHALL discover skills from the `docs/skills/<name>/SKILL.md` directory structure.
- 技能目录:`docs/skills/`,每个子目录为一个技能包
- 发现器 `SkillDiscoverer` 执行 4 层扫描:内置目录 / 用户目录 / 团队 Hub / 市场
- 发现结果 `DiscoveryResult` 包含 `DiscoveryStatus`(Found / NotFound / Error)
- 技能可从 GitHub Gist 或本地文件发布

#### Scenario: 内置技能发现
- **WHEN** SkillDiscoverer 扫描 `docs/skills/` 目录
- **THEN** 发现 27 个内置技能(如 `api-tester`, `code-review`, `web-search` 等)
- **AND** 每个技能解析为 `SkillManifest` 并注册到 `CapabilityRegistry`

### Requirement: 沙箱执行
The system SHALL execute skills in a capability-based sandbox with risk-level isolation.
- 能力模型 `CapabilitySet`:声明技能可执行的操作(net:http / exec:shell / llm:call / file:read / file:write)
- 风险等级 `RiskLevel`:Low / Medium / High / Critical
- 沙箱策略 `SandboxPolicy`:按风险等级限制路径访问、网络访问、环境变量、资源上限
- 执行器类型:`LocalExecutor`(本地 python/node/bash/powershell)、`McpExecutor`(MCP 协议)、`RemoteExecutor`(远端)
- exec 类操作需经 `ExecApprovalTracker` 审批门禁(fail-closed 超时拒绝)
- 默认审批超时:`DEFAULT_EXEC_APPROVAL_TIMEOUT_SECS`
- 可选 WASM 沙箱(`wasm-sandbox` feature,wasmtime 24.x)

#### Scenario: 高风险操作审批
- **WHEN** 技能执行 `exec:shell` 类操作(RiskLevel = High)
- **THEN** 触发 `ExecApprovalTracker` 审批门禁
- **AND** 用户未在超时时间内批准时,fail-closed 拒绝执行
- **AND** 返回 `TIMEOUT_FAIL_CLOSED_REASON`

#### Scenario: 沙箱路径限制
- **WHEN** 技能尝试访问沙箱策略外的路径
- **THEN** 访问被拒绝,返回路径越权错误
- **AND** 沙箱仅允许 `SandboxConfig` 中声明的路径

### Requirement: 技能市场
The system SHALL provide a skill marketplace with search, one-click install, update checking, and publishing.
- `SkillMarketplace` 支持搜索(`MarketplaceQuery`)、排序(`SortBy`)、统计(`MarketplaceStats`)
- 一键安装:从市场拉取技能包并导入到本地 `docs/skills/`
- 更新检查:`UpdateInfo` 检测已安装技能是否有新版本
- 发布:`PublishManifest` 支持发布到 GitHub Gist 或本地文件
- `SkillPublisher` trait 提供 `FilePublisher` 与 `GistPublisher` 实现
- 团队 Hub:`TeamSkillsHubClient` 从团队技能中心拉取

#### Scenario: 市场搜索与安装
- **WHEN** 用户在技能市场搜索 "api 测试"
- **THEN** `SkillMarketplace` 返回匹配的 `SearchHit` 列表
- **AND** 用户点击"安装"后,技能包下载并导入到本地 `docs/skills/`
- **AND** 安装后技能立即可用(热更新)

### Requirement: 自动发明器
The system SHALL automatically invent skill drafts by detecting repeated operation sequences.
- `SkillAutoInventor` 监听用户操作序列(`OperationRecord`)
- `RingBuffer` 缓存最近操作,检测重复模式(`DetectedPattern`)
- 检测到重复模式后生成 SKILL.md 草稿
- 草稿经用户审核后正式注册
- 配置 `AutoInventorConfig` 控制检测灵敏度与草稿生成阈值

#### Scenario: 重复操作检测与草稿生成
- **WHEN** 用户连续 3 次执行相同的"搜索 → 摘要 → 翻译"操作序列
- **THEN** `SkillAutoInventor` 检测到 `DetectedPattern`
- **AND** 生成 SKILL.md 草稿并提示用户审核
- **AND** 用户确认后技能注册到本地目录

### Requirement: 热更新
The system SHALL support hot-reloading of skills without restarting the application.
- 技能文件变更后(新增/修改/删除),SkillEngine 重新扫描并更新注册表
- `file_watcher` 监听 `docs/skills/` 目录变更
- 变更后触发 `CapabilityRegistry` 刷新
- 正在执行的技能不受影响(完成后下次调用使用新版本)

#### Scenario: 技能文件修改后热更新
- **WHEN** 用户修改 `docs/skills/api-tester/SKILL.md` 的 frontmatter
- **THEN** file_watcher 检测到变更,触发 SkillEngine 重新加载该技能
- **AND** 注册表更新,下次调用使用新版本
- **AND** 正在执行的技能实例不受影响
