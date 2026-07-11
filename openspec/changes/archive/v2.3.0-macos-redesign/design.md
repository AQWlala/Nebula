# 技术方案 — v2.3.0-macos-redesign

> **领域**: ui
> **设计日期**: 2026-07-09
> **状态**: 已实现并归档

## 现状

v2.2.0 的 UI 行为契约（`specs/ui/spec.md`）中定义了以下相关 Requirement：

- **Requirement: 侧边栏布局** — 240px 宽，纯色背景 `#152233`，10 个导航项平铺无分组
- **Requirement: 状态栏** — 底部独立一行，24px 高，显示模型/内存/版本
- **Requirement: 默认视图** — 应用启动后进入 `code`（代码工作台）
- **Requirement: 命令面板** — Cmd+K 触发，浮层形式，无常驻搜索框

上述设计在 v2.2.0 周期内暴露出导航效率低、视觉沉重、空间浪费三个问题（详见 proposal.md）。

## 方案

### 1. 三栏布局重构

采用 macOS 原生应用经典三栏结构：

```
┌─────────────────────────────────────────────────────────┐
│  Titlebar (44px)  搜索框(Cmd+K)        🪟 🌀 ⚙️          │
├──────────────┬──────────────────────────────────────────┤
│  Sidebar     │          主内容区                         │
│  (毛玻璃)     │     (大圆角半透明容器)                     │
│  220px       │                                          │
│  ─────────  │                                          │
│  ● 模型在线  │                                          │
│  内存 42MB   │                                          │
│  v2.3.0     │                                          │
├──────────────┴──────────────────────────────────────────┤
│           (状态栏已融入侧边栏底部)                         │
└─────────────────────────────────────────────────────────┘
```

| 维度 | 现状 | 重设计 |
|------|------|--------|
| 侧边栏宽度 | 240px（可拖拽 180-320） | 220px（可拖拽 200-280） |
| 侧边栏背景 | 纯色 `#152233` | 毛玻璃 `backdrop-filter: blur(20px) saturate(180%)` |
| 导航分组 | 10 项平铺 | 4 组：收藏/工作/监控/高级 |
| 状态栏 | 底部独立一行 | 融入侧边栏底部 |
| 工具栏 | 无 | 顶部 44px macOS titlebar |
| 内容区圆角 | 8px | 16px |

### 2. 导航分组方案

借鉴 macOS Finder 侧边栏，10 个导航项按职能分 4 组：

```typescript
const NAV_GROUPS = [
  { label: '收藏', items: ['chat', 'swarm'] },
  { label: '工作', items: ['memory', 'code', 'skills'] },
  { label: '监控', items: ['dashboard', 'credits', 'diagnostics'] },
  { label: '高级', items: ['shadow', 'longtask'] },
];
```

分组标题样式：12px、uppercase、letter-spacing 0.5px、`--text-muted`、padding-left 20px。
导航项 active 态：圆角 8px 半透明强调色背景块（macOS 风格），非 font-weight 加粗。

### 3. 毛玻璃效果

```css
.sidebar {
  background: rgba(30, 30, 30, 0.6);
  backdrop-filter: saturate(180%) blur(20px);
}

.content-card {
  background: rgba(40, 40, 40, 0.5);
  backdrop-filter: blur(10px);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 16px;
}
```

使用纯 CSS `backdrop-filter`，不引入额外依赖。`-webkit-` 前缀确保 Tauri WebView 兼容。

### 4. Spotlight 风格工具栏

顶部 44px titlebar：
- 左侧：当前视图标题（16px font-display）
- 中间：Spotlight 风格搜索框（点击触发命令面板，复用现有 CommandPalette 组件）
- 右侧：浮动窗 / 悬浮球 / 设置 快捷按钮

### 5. 色彩系统调整

| Token | 现值 | 调整后 | 说明 |
|-------|------|--------|------|
| `--bg-primary` | `#0f1923` | `#1a1a1a` | 更中性，接近 macOS 暗色 |
| `--bg-secondary` | `#152233` | `rgba(255,255,255,0.03)` | 半透明分层 |
| `--bg-tertiary` | `#1c3045` | `rgba(255,255,255,0.06)` | 悬浮层半透明 |
| `--border` | `#1e3a5f` | `rgba(255,255,255,0.08)` | 细微白色边框 |
| `--accent` | `#1e3a5f` | `#0A84FF` | macOS Blue |
| `--accent-neon` | `#ff8c42` | `#FF9F0A` | macOS Orange |

### 6. 默认视图切换

`ModeSwitcher` 的默认值从 `code` 改为 `chat`。模式判断逻辑（`modeRouter.ts`）不变，仅改初始值。这降低新用户上手门槛——对话是最直觉的交互方式。

## 关键决策

| 决策 | 选项 | 选择 | 理由 |
|------|------|------|------|
| 毛玻璃实现 | A: CSS backdrop-filter / B: 引入 blur.js 库 | A | 原生 CSS 零依赖，Tauri WebView 支持良好 |
| 导航分组数量 | A: 3 组 / B: 4 组 / C: 不分组 | B | 4 组（收藏/工作/监控/高级）符合 macOS Finder 习惯，每组 2-3 项不超载 |
| 状态栏位置 | A: 保留底部 / B: 融入侧边栏 / C: 融入 titlebar | B | 释放底部空间，模型/内存信息与导航同栏更紧凑 |
| 默认视图 | A: 保持 code / B: 改为 chat | B | 对话是最直觉入口，code 工作台对新手门槛高 |
| 搜索框 | A: 仅 Cmd+K 浮层 / B: titlebar 常驻框 | B | 常驻框提升搜索可发现性，点击仍触发命令面板 |

## 风险与缓解

- **风险1**: `backdrop-filter` 在低端 GPU 上可能掉帧 → 缓解: 添加 `will-change: backdrop-filter`，并在 Settings 提供"关闭毛玻璃"开关（降级为纯色）
- **风险2**: 默认视图改为 chat 后，重度代码用户不适应 → 缓解: 首次启动时 Onboarding 引导提示"可在设置中改回 code 默认"
- **风险3**: 色彩系统大改导致现有组件视觉不一致 → 缓解: 全局替换 CSS token，所有组件引用 token 而非硬编码色值，一次性生效
