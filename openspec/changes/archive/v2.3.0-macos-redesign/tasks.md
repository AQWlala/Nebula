# 实现清单 — v2.3.0-macos-redesign

> **总任务数**: 18
> **已完成**: 18
> **状态**: 全部完成

## T-01: 色彩系统 token 替换
- [x] 在 `src/theme/index.ts` 更新 `--bg-primary` 为 `#1a1a1a`
- [x] 更新 `--bg-secondary` 为 `rgba(255,255,255,0.03)`
- [x] 更新 `--bg-tertiary` 为 `rgba(255,255,255,0.06)`
- [x] 更新 `--border` 为 `rgba(255,255,255,0.08)`
- [x] 更新 `--accent` 为 `#0A84FF` (macOS Blue)
- [x] 更新 `--accent-neon` 为 `#FF9F0A` (macOS Orange)
- [x] 更新圆角 md 从 8px 到 12px

## T-02: 侧边栏毛玻璃化
- [x] 在 `src/styles/global.css` 为 `.sidebar` 添加 `backdrop-filter: saturate(180%) blur(20px)`
- [x] 添加 `-webkit-backdrop-filter` 前缀
- [x] 背景改为 `rgba(30, 30, 30, 0.6)`
- [x] 添加 `will-change: backdrop-filter`

## T-03: 侧边栏宽度调整
- [x] 默认宽度从 240px 改为 220px
- [x] 拖拽范围从 180-320 改为 200-280

## T-04: 导航分组实现
- [x] 在 `src/App.tsx` 定义 `NAV_GROUPS` 常量（4 组：收藏/工作/监控/高级）
- [x] 实现分组标题样式（12px uppercase letter-spacing 0.5px）
- [x] 实现导航项 active 态（圆角 8px 半透明强调色背景）
- [x] 添加 i18n key: `nav.group.favorites` / `nav.group.workspace` / `nav.group.monitor` / `nav.group.advanced`

## T-05: 状态栏融入侧边栏
- [x] 从 `src/App.tsx` 移除底部独立 StatusBar 行
- [x] 将 StatusBar 内容（模型/内存/版本）移入 Sidebar 底部
- [x] 调整 StatusBar 组件样式适配侧边栏宽度

## T-06: 顶部 Titlebar 实现
- [x] 新增 44px 高度 titlebar 容器
- [x] 左侧显示当前视图标题（16px font-display）
- [x] 中间放置 Spotlight 风格搜索框
- [x] 右侧放置浮动窗/悬浮球/设置快捷按钮

## T-07: Spotlight 搜索框
- [x] 搜索框样式（圆角、半透明背景、搜索图标）
- [x] 点击触发现有 CommandPalette 组件
- [x] 保留 Cmd+K 快捷键绑定

## T-08: 内容区卡片大圆角化
- [x] `.content-card` 圆角从 8px 改为 16px
- [x] 背景改为 `rgba(40, 40, 40, 0.5)` + `backdrop-filter: blur(10px)`
- [x] 边框改为 `1px solid rgba(255,255,255,0.08)`

## T-09: 默认视图切换
- [x] `ModeSwitcher` 默认值从 `code` 改为 `chat`
- [x] `src/lib/modeRouter.ts` 初始模式判断适配

## T-10: 对话界面消息气泡调整
- [x] 用户消息圆角改为 16px（右下角 4px）
- [x] 助手消息圆角改为 16px（左下角 4px）
- [x] 最大宽度从 75% 微调为 70%

## T-11: 输入框 macOS 风格化
- [x] 圆角容器 `border-radius: 12px`
- [x] 毛玻璃背景 `backdrop-filter: blur(8px)`
- [x] 发送按钮改为圆形强调色

## T-12: 毛玻璃降级开关
- [x] 在 `src/components/Settings.tsx` 添加"关闭毛玻璃"开关
- [x] 开关关闭时 `.sidebar` / `.content-card` 降级为纯色背景
- [x] 设置持久化到 localStorage

## T-13: Onboarding 引导提示
- [x] 首次启动 v2.3.0 时提示"默认视图已改为对话"
- [x] 提示"可在设置中改回代码默认"
- [x] 引导只显示一次（localStorage 标记）

## T-14: i18n 更新
- [x] `src/i18n/zh-CN.json` 添加导航分组翻译
- [x] `src/i18n/en-US.json` 添加导航分组翻译
- [x] 添加 Onboarding 提示文案

## T-15: 测试更新
- [x] 更新 `ChatPanel.test.tsx` 适配新选择器
- [x] 更新 `MemoryMap.test.tsx` header 选择器
- [x] 为导航分组添加测试
- [x] 为 Spotlight 搜索框添加测试

## T-16: 主题测试
- [x] 更新 `src/theme/__tests__/theme.test.ts` 验证新 token 值
- [x] 验证毛玻璃降级逻辑

## T-17: 视觉回归验证
- [x] 手动核对所有 10 个视图在新主题下无视觉破损
- [x] 核对浅色/暗色模式切换正常
- [x] 核对毛玻璃开关开/关两种状态

## T-18: 文档更新
- [x] 更新 `docs/design/REDESIGN_PROPOSAL_v3.0.md` 标注已实现
- [x] 更新 `docs/CHANGELOG.md` 添加 v2.3.0 条目
- [x] 更新 `docs/ROADMAP_v2.3.md` 标记完成
