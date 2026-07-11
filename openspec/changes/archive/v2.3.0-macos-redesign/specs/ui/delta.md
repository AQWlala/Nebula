# Delta for UI

> **变更**: v2.3.0-macos-redesign
> **领域**: ui

## ADDED Requirements

### Requirement: 导航分组
The system SHALL organize sidebar navigation items into 4 functional groups: Favorites (收藏), Workspace (工作), Monitor (监控), Advanced (高级).
- 分组标题样式: 12px、uppercase、letter-spacing 0.5px、`--text-muted`
- 每组包含 2-3 个导航项
- 分组间距: 16px

#### Scenario: 用户查看侧边栏分组
- **WHEN** 应用启动并显示侧边栏
- **THEN** 导航项按 4 组显示：收藏(chat, swarm)、工作(memory, code, skills)、监控(dashboard, credits, diagnostics)、高级(shadow, longtask)
- **AND** 每组上方显示分组标题

### Requirement: 顶部 Titlebar
The system SHALL render a 44px macOS-style titlebar at the top of the application window.
- 左侧: 当前视图标题 (16px font-display)
- 中间: Spotlight 风格搜索框 (点击触发命令面板)
- 右侧: 浮动窗 / 悬浮球 / 设置 快捷按钮

#### Scenario: 用户点击 Spotlight 搜索框
- **WHEN** 用户点击 titlebar 中间的搜索框
- **THEN** 系统打开 CommandPalette 命令面板
- **AND** 搜索框获得视觉聚焦反馈

#### Scenario: 用户使用 Cmd+K 快捷键
- **WHEN** 用户按下 Cmd+K (macOS) 或 Ctrl+K (Windows)
- **THEN** 系统打开 CommandPalette 命令面板
- **AND** 行为与点击搜索框一致

### Requirement: 毛玻璃降级开关
The system SHALL provide a user-configurable toggle to disable backdrop-filter effects for low-end GPUs.
- 开关位置: Settings 面板
- 默认: 开启 (毛玻璃生效)
- 关闭时: 侧边栏和内容卡片降级为纯色背景
- 设置持久化到 localStorage

#### Scenario: 用户关闭毛玻璃
- **WHEN** 用户在 Settings 中关闭"毛玻璃效果"开关
- **THEN** 侧边栏背景立即变为纯色 `#1a1a1a`
- **AND** 内容卡片背景变为纯色 `#242424`
- **AND** 设置保存到 localStorage，重启后生效

## MODIFIED Requirements

### Requirement: 侧边栏布局
The system SHALL render a 220px-wide sidebar with macOS vibrancy (frosted glass) background. (Previously: 240px-wide sidebar with solid color `#152233` background, no grouping)
- 宽度: 220px (可拖拽 200-280)
- 背景: `rgba(30, 30, 30, 0.6)` + `backdrop-filter: saturate(180%) blur(20px)`
- 导航项 active 态: 圆角 8px 半透明强调色背景块 (非 font-weight 加粗)
- 状态栏内容(模型/内存/版本) 显示在侧边栏底部

#### Scenario: 用户拖拽侧边栏宽度
- **WHEN** 用户拖拽侧边栏右边缘
- **THEN** 宽度在 200px 到 280px 之间调整
- **AND** 毛玻璃效果在调整过程中保持流畅

#### Scenario: 导航项 active 状态显示
- **WHEN** 用户切换到某个导航项
- **THEN** 该项显示圆角 8px 半透明强调色背景块
- **AND** 其他导航项不显示背景块

### Requirement: 状态栏
The system SHALL display model status, memory usage, and version info at the bottom of the sidebar (not as a separate bottom bar). (Previously: 24px-high separate bottom bar)
- 位置: 侧边栏底部
- 内容: 模型在线状态 / 内存占用 / 版本号
- 不再独占底部一行

#### Scenario: 模型状态变化
- **WHEN** 模型从在线变为离线
- **THEN** 侧边栏底部的模型状态指示器更新为离线样式
- **AND** 不影响主内容区布局

### Requirement: 默认视图
The system SHALL default to the chat (conversation) view on application startup. (Previously: default to code workspace)
- 默认视图: chat
- ModeSwitcher 的模式判断逻辑不变
- 用户可在 Settings 中改回 code 默认

#### Scenario: 新用户首次启动
- **WHEN** 新用户首次启动应用
- **THEN** 默认进入对话视图
- **AND** Onboarding 引导提示"默认视图已改为对话，可在设置中改回"

### Requirement: 内容区卡片样式
The system SHALL render content cards with 16px border-radius and semi-transparent frosted glass background. (Previously: 8px border-radius with solid color background)
- 圆角: 16px
- 背景: `rgba(40, 40, 40, 0.5)` + `backdrop-filter: blur(10px)`
- 边框: `1px solid rgba(255,255,255,0.08)`

#### Scenario: 内容卡片在浅色/暗色模式下的表现
- **WHEN** 用户切换浅色/暗色模式
- **THEN** 内容卡片的半透明背景自适应调整
- **AND** 边框透明度保持 0.08

### Requirement: 色彩系统
The system SHALL use a macOS dark-mode-inspired neutral gray color palette with semi-transparent layering. (Previously: deep blue solid colors `#0f1923` / `#152233` / `#1c3045`)
- `--bg-primary`: `#1a1a1a` (中性灰)
- `--bg-secondary`: `rgba(255,255,255,0.03)` (半透明分层)
- `--bg-tertiary`: `rgba(255,255,255,0.06)` (悬浮层半透明)
- `--border`: `rgba(255,255,255,0.08)` (细微白色边框)
- `--accent`: `#0A84FF` (macOS Blue)
- `--accent-neon`: `#FF9F0A` (macOS Orange)

#### Scenario: 组件引用色彩 token
- **WHEN** 任何组件需要背景色
- **THEN** 必须引用 `--bg-primary` / `--bg-secondary` / `--bg-tertiary` token
- **AND** 不得硬编码色值

## REMOVED Requirements

(none)
