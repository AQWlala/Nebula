# Nebula 前端重设计方案 v3.0

> 借鉴 jiuwenswarm（模型管理）、openakita（面板化+5分钟上手）、hermes（技能生态）的设计优势，
> 结合 macOS 设计语言，对 Nebula 前端进行系统性重设计。

---

## 一、设计理念

### 核心原则
1. **macOS 原生质感**：毛玻璃侧边栏（vibrancy）、大圆角卡片、半透明分层、细腻阴影
2. **导航分组**：借鉴 macOS Finder 侧边栏，10 个导航项按职能分组，不再平铺
3. **对话优先**：默认进入对话视图（而非代码工作台），降低上手门槛
4. **状态栏融入侧边栏**：底部状态栏不再独占一行，模型/内存信息显示在侧边栏底部
5. **工具栏统一**：顶部 44px macOS 风格工具栏，搜索居中（Spotlight 风格）

### 竞品借鉴点
| 竞品 | 借鉴内容 |
|------|---------|
| jiuwenswarm | 模型配置"配置→测试→关联"三步流程、agent-studio 可视化管理 |
| openakita | 面板化设计、向导式首次配置、布局清晰、5分钟上手 |
| hermes | 技能市场卡片设计、技能来源 badge 体系 |

---

## 二、布局结构

### 2.1 整体布局（macOS 风格三栏）

```
┌─────────────────────────────────────────────────────────┐
│  Titlebar (44px)  搜索框(Cmd+K)        🪟 🌀 ⚙️          │
├──────────────┬──────────────────────────────────────────┤
│              │                                          │
│  Sidebar     │          主内容区                         │
│  (毛玻璃)     │     (大圆角半透明容器)                     │
│  220px       │                                          │
│              │                                          │
│  ── 收藏 ──  │                                          │
│  💬 对话     │                                          │
│  🐝 蜂群     │                                          │
│              │                                          │
│  ── 工作 ──  │                                          │
│  🧠 记忆     │                                          │
│  💻 代码     │                                          │
│  🔍 技能     │                                          │
│              │                                          │
│  ── 监控 ──  │                                          │
│  📊 仪表盘   │                                          │
│  💰 积分     │                                          │
│  🩺 诊断     │                                          │
│              │                                          │
│  ── 高级 ──  │                                          │
│  🌑 影子     │                                          │
│  ⏳ 长任务   │                                          │
│              │                                          │
│  ─────────  │                                          │
│  ● 模型在线  │                                          │
│  内存 42MB   │                                          │
│  v2.2.0     │                                          │
├──────────────┴──────────────────────────────────────────┤
│           (状态栏已融入侧边栏底部)                         │
└─────────────────────────────────────────────────────────┘
```

### 2.2 关键变化

| 项目 | 现状 | 重设计 |
|------|------|--------|
| 侧边栏宽度 | 240px（可拖拽 180-320） | 220px（可拖拽 200-280） |
| 侧边栏背景 | 纯色 `#152233` | 毛玻璃 `backdrop-filter: blur(20px) saturate(180%)` |
| 导航分组 | 10 项平铺无分组 | 4 组：收藏/工作/监控/高级 |
| 导航项 active | 背景色 + font-weight | 圆角半透明背景块（macOS 风格） |
| 状态栏 | 底部独立一行 | 融入侧边栏底部 |
| 工具栏 | 无统一工具栏 | 顶部 44px macOS titlebar |
| 搜索 | Cmd+K 命令面板 | 顶部 Spotlight 风格搜索框 |
| 默认视图 | code（代码工作台） | chat（对话） |
| 内容区圆角 | 8px | 16px（大圆角容器） |
| 卡片边框 | 1px solid border | 1px solid rgba(255,255,255,0.08) |

---

## 三、导航分组方案

### 3.1 分组详情

```typescript
const NAV_GROUPS = [
  {
    label: '收藏',  // i18n: nav.group.favorites
    items: [
      { id: 'chat',   icon: '💬', label: '对话' },
      { id: 'swarm',  icon: '🐝', label: '蜂群' },
    ]
  },
  {
    label: '工作',  // i18n: nav.group.workspace
    items: [
      { id: 'memory', icon: '🧠', label: '记忆' },
      { id: 'code',   icon: '💻', label: '代码' },
      { id: 'skills', icon: '🔍', label: '技能' },
    ]
  },
  {
    label: '监控',  // i18n: nav.group.monitor
    items: [
      { id: 'dashboard',   icon: '📊', label: '仪表盘' },
      { id: 'credits',     icon: '💰', label: '积分' },
      { id: 'diagnostics', icon: '🩺', label: '诊断' },
    ]
  },
  {
    label: '高级',  // i18n: nav.group.advanced
    items: [
      { id: 'shadow',   icon: '🌑', label: '影子' },
      { id: 'longtask', icon: '⏳', label: '长任务' },
    ]
  },
];
```

### 3.2 分组样式（macOS Finder 风格）

- 分组标题：12px、uppercase、letter-spacing 0.5px、color `--text-muted`、padding-left 20px
- 分组间距：16px
- 导航项：圆角 8px、hover 半透明白色背景、active 半透明强调色背景

---

## 四、对话界面重设计

### 4.1 布局

```
┌──────────────────────────────────────────────┐
│  💬 对话          📐中  🕐  📤  📋          │  ← 精简工具栏
├──────────────────────────────────────────────┤
│                                              │
│              ┌─────────────────┐             │
│              │ 用户消息（右）    │             │  ← 大圆角气泡
│              └─────────────────┘             │
│                                              │
│  ┌─────────────────────────────┐             │
│  │ Nebula 回复（左）            │             │
│  │ 含推理链 / 一致性 badge      │             │
│  └─────────────────────────────┘             │
│                                              │
├──────────────────────────────────────────────┤
│  ┌──────────────────────────────────────┐   │
│  │  输入消息...                    发送  │   │  ← macOS 风格输入框
│  └──────────────────────────────────────┘   │  ← 大圆角 + 毛玻璃
└──────────────────────────────────────────────┘
```

### 4.2 消息气泡

- 用户消息：靠右、半透明蓝色背景 `rgba(0,122,255,0.15)`、圆角 16px（右下角 4px）
- 助手消息：靠左、半透明卡片背景 `rgba(255,255,255,0.05)`、圆角 16px（左下角 4px）
- 最大宽度：70%（从 75% 微调）
- 间距：12px

### 4.3 输入框

- 大圆角容器 `border-radius: 12px`
- 毛玻璃背景 `backdrop-filter: blur(8px)` + `rgba(255,255,255,0.05)`
- 内边距：12px 16px
- 发送按钮：圆形、强调色背景、内嵌右侧

---

## 五、色彩系统调整

### 5.1 macOS 暗色模式启发

| Token | 现值 | 调整后 | 说明 |
|-------|------|--------|------|
| `--bg-primary` | `#0f1923` | `#1a1a1a` | 更中性，接近 macOS 暗色 |
| `--bg-secondary` | `#152233` | `rgba(255,255,255,0.03)` | 半透明分层 |
| `--bg-tertiary` | `#1c3045` | `rgba(255,255,255,0.06)` | 悬浮层半透明 |
| `--border` | `#1e3a5f` | `rgba(255,255,255,0.08)` | 细微白色边框 |
| `--accent` | `#1e3a5f` | `#0A84FF` | macOS Blue |
| `--accent-neon` | `#ff8c42` | `#FF9F0A` | macOS Orange |
| 圆角 md | `8px` | `12px` | 更大圆角 |
| 圆角 lg | `16px` | `16px` | 保持 |

### 5.2 毛玻璃效果

```css
.sidebar {
  background: rgba(30, 30, 30, 0.6);
  backdrop-filter: saturate(180%) blur(20px);
  -webkit-backdrop-filter: saturate(180%) blur(20px);
}

.content-card {
  background: rgba(40, 40, 40, 0.5);
  backdrop-filter: blur(10px);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 16px;
}
```

---

## 六、工具栏设计（新增）

### 6.1 macOS Titlebar 风格

```
┌──────────────────────────────────────────────────────┐
│  Nebula    [🔍 搜索或输入命令... Cmd+K]    🪟 🌀 ⚙️  │
└──────────────────────────────────────────────────────┘
  44px 高度
```

- 左侧：当前视图标题（16px font-display）
- 中间：Spotlight 风格搜索框（点击触发命令面板）
- 右侧：浮动窗 / 悬浮球 / 设置 快捷按钮

### 6.2 样式

```css
.titlebar {
  height: 44px;
  display: flex;
  align-items: center;
  padding: 0 16px;
  background: rgba(30, 30, 30, 0.6);
  backdrop-filter: saturate(180%) blur(20px);
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.titlebar-search {
  flex: 1;
  max-width: 400px;
  margin: 0 auto;
  background: rgba(255, 255, 255, 0.06);
  border-radius: 8px;
  padding: 6px 12px;
  color: var(--text-secondary);
}
```

---

## 七、侧边栏底部状态区

### 7.1 融入侧边栏

状态栏不再独占底部一行，模型/内存信息显示在侧边栏底部：

```
│  ─────────  │
│  ● 模型在线  │  ← 圆点 + 文字
│  内存 42MB   │  ← 仅有数据时显示
│  v2.2.0     │  ← 版本号
```

### 7.2 样式

```css
.sidebar-status {
  padding: 12px 16px;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  font-size: 11px;
  color: var(--text-muted);
}
```

---

## 八、技能市场重设计（借鉴 hermes）

### 8.1 卡片网格

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│ 🔍 技能名     │  │ 🔍 技能名     │  │ 🔍 技能名     │
│              │  │              │  │              │
│ 描述文字...   │  │ 描述文字...   │  │ 描述文字...   │
│              │  │              │  │              │
│ 🦀 rust ★4.5 │  │ 🐍 py  ★4.2 │  │ ⚡ js  ★4.8 │
│ [使用] [详情] │  │ [使用] [详情] │  │ [使用] [详情] │
└──────────────┘  └──────────────┘  └──────────────┘
```

- 网格：`repeat(auto-fill, minmax(240px, 1fr))`
- 卡片：16px 圆角、半透明背景、hover 上浮 + 阴影
- 来源 badge：右上角彩色标签

---

## 九、实施路线图

| 阶段 | 内容 | 影响范围 |
|------|------|---------|
| Phase 1 | 色彩系统 + 毛玻璃 + 圆角调整 | global.css :root |
| Phase 2 | 侧边栏分组 + 底部状态区 | App.tsx Sidebar + StatusBar |
| Phase 3 | 工具栏新增 | App.tsx + global.css |
| Phase 4 | 对话界面气泡 + 输入框 | ChatPanel.tsx + global.css |
| Phase 5 | 默认视图 chat + 技能市场卡片 | nebulaStore.ts + SkillPanel.tsx |

---

## 十、与现有设计系统的兼容

- 保留 `DESIGN.md` 中的字号阶梯、字重层级、间距标尺、z-index 语义标尺
- 保留 Impeccable / Taste-skill 审计规则（无侧条纹、无暗色发光、无渐变文字）
- 保留 `prefers-reduced-motion` 无障碍兜底
- 保留 `focus-visible` 全局样式
- 色彩从 OKLCH 表示法切换到 rgba 半透明分层（macOS 风格核心特征）
