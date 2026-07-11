# 前端 UI 行为契约

> **领域**: ui
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

前端 UI 采用 macOS 风格三栏布局,毛玻璃侧边栏(vibrancy)+ Spotlight 风格工具栏,提供 11 个核心视图,支持中英文 i18n,使用 Preact signals 进行状态管理,WebGL 驱动 3D 可视化。基于 Tauri 2.0 + Preact + TypeScript + Tailwind CSS 构建。

## Requirements

### Requirement: 三栏布局
The system SHALL render a macOS-style three-pane layout with a vibrancy sidebar and Spotlight-style toolbar.
- 三栏布局:侧边栏(导航)/ 主内容区 / 可选右栏(上下文)
- 侧边栏宽度可拖拽调整(180-320px),持久化到 localStorage
- 侧边栏支持折叠(48px 折叠态)
- macOS vibrancy:`tauri.conf.json` 中 `macOSPrivateApi: true` 启用毛玻璃效果
- 44px Spotlight 风格工具栏:全局搜索 / 命令面板入口
- 窗口默认尺寸:1200×800,最小 800×600,可调整大小

#### Scenario: 侧边栏拖拽调整
- **WHEN** 用户拖拽侧边栏边缘
- **THEN** 侧边栏宽度在 180-320px 范围内调整
- **AND** 宽度持久化到 localStorage(`nebula-sidebar-width`)
- **AND** CSS 变量 `--sidebar-width` 驱动布局

#### Scenario: 侧边栏折叠
- **WHEN** 用户点击折叠按钮
- **THEN** 侧边栏收缩为 48px 折叠态(`is-collapsed` class)
- **AND** 仅显示图标,隐藏文字标签

### Requirement: 11 个核心视图
The system SHALL provide 11 core views accessible via sidebar navigation.
- 视图列表(由 `nebulaStore.currentMode` 路由):
  1. `chat` — 对话(ChatPanel)
  2. `swarm` — 蜂群(SwarmView)
  3. `memory` — 记忆(MemoryInspector / MemoryMap / TimelineView 三视图切换)
  4. `code` — 代码工作台(CodeMode)
  5. `skills` — 技能市场(SkillPanel / SkillMarketplace)
  6. `dashboard` — 仪表盘(Dashboard)
  7. `credits` — 积分费用(CreditsDashboard)
  8. `diagnostics` — 诊断(DiagnosticsView)
  9. `shadow` — 影子工作区(ShadowWorkspacePanel)
  10. `longtask` — 长任务(LongTaskPanel)
  11. `settings` — 设置(Settings)
- 代码分割懒加载:除 CodeMode / ModeSwitcher / StatusBar / ErrorBoundary / Toasts / OnboardingWizard / CommandPalette 外,其余视图懒加载(`lazy(() => import(...))`)
- 顶部模式切换:Writing / Work / Code(`ModeSwitcher`)

#### Scenario: 视图切换
- **WHEN** 用户点击侧边栏"蜂群"图标
- **THEN** `nebulaStore.currentMode` 切换为 `swarm`
- **AND** SwarmView chunk 懒加载并渲染
- **AND** 加载期间显示 `LoadingFallback`(⏳ + "加载中")

#### Scenario: 记忆三视图切换
- **WHEN** 用户在记忆视图中切换到"时间轴"
- **THEN** `nebulaStore.memoryView` 切换为 `timeline`
- **AND** TimelineView 渲染记忆时间轴

### Requirement: 国际化
The system SHALL support Chinese (zh-CN) and English (en-US) internationalization.
- i18n 模块:`src/i18n/`,提供 `t()` 翻译函数与 `currentLocale` 信号
- 语言包:`zh-CN.json`(中文)/ `en-US.json`(英文)
- 语言切换:Settings 中切换,实时生效无需重启
- `View` 类型注释与 UI 文本均经 i18n 覆盖

#### Scenario: 语言切换
- **WHEN** 用户在设置中将语言从"中文"切换为"English"
- **THEN** `currentLocale` 切换为 `en-US`
- **AND** 所有 UI 文本实时切换为英文
- **AND** 无需重启应用

### Requirement: 状态管理
The system SHALL manage global state using Preact signals (nebulaStore).
- `nebulaStore`(`src/stores/nebulaStore.ts`):全局状态单例
- 状态信号(reactive signals):
  - `ready` / `version` / `recentMemories` / `currentTask` / `swarmOutputs`
  - `metrics` / `migrationStatus` / `reflections` / `mode` / `ollamaStatus`
  - `currentMode`(视图路由)/ `memoryView`(记忆子视图)/ `autonomyLevel`
  - `externalFilePath` / `chatPrefill` / `aiAutoMode` / `modeMisclassification`
- Bootstrap 流程:bootstrap → health → refreshMemories → refreshMetrics → refreshReflections → checkOllama → getLevel
- Ollama 健康检查:每 30 秒轮询(`checkOllama`)

#### Scenario: Bootstrap 初始化
- **WHEN** 应用启动
- **THEN** nebulaStore.bootstrap() 依次执行:bootstrap → health → refreshMemories → refreshMetrics
- **AND** 完成后 `ready` 信号置为 true,UI 渲染主视图
- **AND** Ollama 健康检查每 30 秒轮询

#### Scenario: 响应式状态更新
- **WHEN** 后端推送新的 swarm 输出
- **THEN** `nebulaStore.swarmOutputs` 信号更新
- **AND** SwarmView 自动重新渲染(信号驱动)

### Requirement: WebGL 3D 可视化
The system SHALL render 3D visualizations (memory graph, swarm DAG) using WebGL via Pixi.js.
- WebGL 模块:`src/lib/webgl/`(`WebGLRenderer` / `Scene` / `Camera` / `shaders`)
- Pixi.js(v8.x)作为 2D/WebGL 渲染引擎
- 记忆图谱(MemoryMap):3D 节点-边图谱可视化
- 蜂群 DAG(DagCanvas):任务依赖图可视化
- 性能:WebGL 加速保证大规模图谱流畅渲染

#### Scenario: 记忆图谱 3D 渲染
- **WHEN** 用户切换到记忆"图谱"视图
- **THEN** MemoryMap 使用 WebGLRenderer 渲染 3D 节点-边图谱
- **AND** 节点代表记忆条目,边代表关系
- **AND** 支持缩放、旋转、点击节点查看详情

### Requirement: 命令面板
The system SHALL provide a Spotlight-style command palette (Cmd/Ctrl+P) for quick navigation and actions.
- `CommandPalette` 组件:全局快捷键唤起(`useCommandPaletteShortcut`)
- 默认命令:`buildDefaultCommands` 提供视图切换、设置、导出等
- 记忆搜索:`buildMemoryItems` 提供记忆条目快速跳转
- Fuse.js 模糊搜索:输入即搜
- 命令分类:导航 / 操作 / 记忆 / 技能

#### Scenario: 命令面板快速跳转
- **WHEN** 用户按下 Cmd/Ctrl+P 并输入"记忆"
- **THEN** CommandPalette 弹出,Fuse.js 模糊搜索匹配"记忆"相关命令
- **AND** 用户选择后直接跳转到记忆视图

### Requirement: 主题与外观
The system SHALL support theme switching (light/dark/system) with CSS variables.
- 主题模块:`src/theme/`(`loadTheme` / `applyTheme`)
- 三种主题:Light / Dark / System(跟随系统)
- CSS 变量驱动:颜色、字体、间距统一通过 CSS 变量定义
- 主题选择持久化到 localStorage
- Boot 时 `loadTheme()` + `applyTheme()`,主题信号变更时重新应用

#### Scenario: 深色模式切换
- **WHEN** 用户在设置中切换为"深色模式"
- **THEN** `applyTheme` 更新 CSS 变量
- **AND** 全局 UI 即时切换为深色配色
- **AND** 选择持久化,下次启动保持

### Requirement: 自主度滑块
The system SHALL provide an autonomy slider (L0-L5) controlling the system's autonomy level.
- `AutonomySlider` 组件:L0(完全手动)至 L5(完全自主)
- 默认等级:L2(对话模式)
- `nebulaStore.autonomyLevel` 信号持久化
- 与 `modeRouter` 正交:自主度控制操作审批阈值,模式控制工作场景
- 从后端同步当前等级(`getLevel`)

#### Scenario: 自主度等级调整
- **WHEN** 用户将自主度滑块从 L2 调整到 L4
- **THEN** `autonomyLevel` 信号更新为 L4
- **AND` 更多操作自动执行无需逐一审批
- **AND** 等级同步到后端持久化
