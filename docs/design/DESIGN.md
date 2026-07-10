# Nebula 设计系统声明（DESIGN.md）

> 本文件是 Nebula 项目视觉与交互一致性的唯一事实来源。
> 所有 Token 取自 `src/styles/global.css` 的 `:root` 声明；颜色统一以 OKLCH 表示，便于亮度感知与主题切换。
> Impeccable 前端审计规则 38–41（字体 / 颜色 / 圆角 / 字号一致性）以本文件为对照基线。

---

## Typography（排版）

### 字体族

| Token | 值 | 用途 |
| --- | --- | --- |
| `--font-sans` | `'Geist', 'Outfit', -apple-system, BlinkMacSystemFont, 'Source Han Sans SC', 'PingFang SC', 'Microsoft YaHei', sans-serif` | 正文、UI 文案、按钮、表单 |
| `--font-display` | `'Geist Display', 'Outfit', -apple-system, BlinkMacSystemFont, 'Source Han Sans SC', 'PingFang SC', 'Microsoft YaHei', sans-serif` | 标题、品牌字、英雄区文案 |
| `--font-mono` | `'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace` | 代码块、行内代码、命令行、对齐数值 |

- 西文优先 Geist（P0 修复：从 Inter 切换，避免 Impeccable 审计标记的「Inter 单字族 AI slop」指纹），Outfit 作为几何无衬线回退；中文回退到 Source Han Sans SC / PingFang SC / Microsoft YaHei。
- 标题类（`.brand-text` / `.panel-title` / `.skill-marketplace__header h2` / `.app-loading .title`）强制使用 `--font-display` + `letter-spacing: -0.02em`，与正文形成层级对比。
- 等宽字体优先 JetBrains Mono，保证代码字符等宽与连字（ligature）一致。

### 字号阶梯

基础字号由 `--font-size: 14px` 定义（Settings 面板会在运行时覆写 `<html>` 上的该变量）。围绕 14px 基准约定如下语义阶梯：

| 语义 | 尺寸 | 用途 |
| --- | --- | --- |
| `--font-size`（base） | 14px | 正文、UI 默认字号 |
| sm | 12px | 辅助说明、标签、元信息 |
| md | 14px | 正文段落、列表项 |
| lg | 16px | 卡片标题、强调段落 |
| xl | 20px | 区块标题 |
| 2xl | 24px | 页面标题 |
| 3xl | 32px | 英雄区标题 |

> 任何新增字号必须落入本阶梯；禁止出现 13px、15px 等阶梯外数值。

### 字重

| 字重 | 值 | 用途 | 强制场景 |
| --- | --- | --- | --- |
| Regular | 400 | 正文 | 段落、列表项、说明文案（默认值，无需显式声明） |
| Medium | 500 | 次级强调、按钮、导航 | `.btn` / `.nav-item.active` / `.command-item.active` / `.tag` 强制使用 |
| Semibold | 600 | 卡片标题、表头、激活态 | `.panel-title` / `.skill-card__header h3` / `.nav-item.active` / 表头单元格强制使用 |
| Bold | 700 | 页面标题、英雄区文案 | `.brand-text` / `.app-loading .title` / `.skill-marketplace__header h2` 强制使用 |

**字重一致性约束（Impeccable G38 / Taste-skill DENSITY）：**

- 禁止「全 400」扁平化：同一视图内必须有明确的字重层级（400 → 500/600 → 700），避免 AI 生成的默认扁平外观。
- 禁止「全 700」过度强调：仅在标题与英雄区使用 700，正文过度加粗会破坏可读性层级。
- 500 与 600 不可互换：500 用于交互态（按钮、激活导航），600 用于结构性标题（卡片标题、表头）。混用会模糊「交互强调」与「信息层级」的语义边界。
- 新增组件须在下表登记字重选择，未登记的字重默认回落到 400。

### line-height

| 场景 | 值 |
| --- | --- |
| 正文 / 段落 | 1.6 |
| 标题 | 1.2 |

### letter-spacing

| 场景 | 值 |
| --- | --- |
| 正文 | 0 |
| 标题 | -0.02em |

---

## Color（色彩）

所有颜色以 OKLCH 表示，保证色相（H）一致、亮度（L）可感知均匀。下方同时给出原始 hex 以便检索。

### 背景色阶

| Token | hex | OKLCH | 用途 |
| --- | --- | --- | --- |
| `--bg-primary` | `#0f1923` | `oklch(0.209 0.025 249.1)` | 主背景（Off-black，非纯黑） |
| `--bg-secondary` | `#152233` | `oklch(0.249 0.037 255.5)` | 次级背景、侧边栏、卡片底 |
| `--bg-tertiary` | `#1c3045` | `oklch(0.303 0.046 250.7)` | 三级背景、悬浮卡片、输入框底 |

### 文字色阶

| Token | hex | OKLCH | 用途 |
| --- | --- | --- | --- |
| `--text-primary` | `#e8ecf1` | `oklch(0.941 0.008 253.9)` | 主文字（Off-white，非纯白） |
| `--text-secondary` | `#8a9bb5` | `oklch(0.685 0.043 258.8)` | 次级文字、说明文案 |
| `--text-muted` | `#5a7a9a` | `oklch(0.568 0.062 249.0)` | 弱化文字、占位符、禁用态 |

### 强调色

| Token | hex | OKLCH | 用途 |
| --- | --- | --- | --- |
| `--accent` | — | 取 `--accent-purple`（默认） | 当前激活强调色别名，由 Settings 切换 |
| `--accent-purple` | `#1e3a5f` | `oklch(0.346 0.074 256.0)` | 默认强调色（蓝灰，与中性色同色相） |
| `--accent-neon` | `#ff8c42` | `oklch(0.754 0.164 50.4)` | 霓虹橙，用于关键 CTA / 高亮 |
| `--accent-warning` | `#ef4444` | `oklch(0.637 0.208 25.3)` | 警告 |
| `--accent-error` | `#ef4444` | `oklch(0.637 0.208 25.3)` | 错误 |

> `--accent` 是别名而非固定值：Settings 面板会在 `--accent-purple` / `--accent-neon` / amber 之间切换。组件应引用 `--accent`，禁止硬编码 `--accent-purple`。

### 边框

| Token | hex | OKLCH | 用途 |
| --- | --- | --- | --- |
| `--border` | `#1e3a5f` | `oklch(0.346 0.074 256.0)` | 分割线、卡片描边、输入框边 |

### 主题

| 主题 | 说明 |
| --- | --- |
| `dark` | 默认主题，上述 token 即深空蓝主题 |
| `light` | 浅色主题，覆盖背景 / 文字 / 浮动窗（`--bg-floating*`）token，避免深色硬编码导致主题断裂 |
| `system` | 跟随操作系统 `prefers-color-scheme` 在 dark / light 间切换 |

---

## Spacing（间距）

间距标尺以 4px 为基步长，线性递增。

| Token | 值 | 语义 |
| --- | --- | --- |
| `--spacing-xs` | 4px | 图标与文字间隙、紧凑内边距 |
| `--spacing-sm` | 8px | 小内边距、行间小间隙 |
| `--spacing-md` | 16px | 默认内边距、卡片内边距 |
| `--spacing-lg` | 24px | 区块内边距、分组间距 |
| `--spacing-xl` | 32px | 区块间距 |
| `--spacing-2xl` | 48px | 大区块间距 |
| `--spacing-3xl` | 64px | 页面级垂直留白 |

> 禁止使用 6px、10px、12px 等非标尺数值（4 的倍数除外，须落入上表）。

---

## Radius（圆角）

| Token | 值 | 语义 |
| --- | --- | --- |
| `--radius-sm` | 4px | 标签、徽标、小按钮 |
| `--radius-md` | 8px | 默认圆角（按钮、输入框、卡片） |
| `--radius-lg` | 16px | 大卡片、模态、面板 |

- `--radius` 为 `--radius-md` 的别名，作为默认圆角。
> 圆角只能取以上三档；禁止 6px / 12px / 20px 等中间值。

---

## Shadow（阴影）

| Token | 值 | 语义 |
| --- | --- | --- |
| `--shadow-sm` | `0 2px 4px rgba(0, 0, 0, 0.3)` | 轻微悬浮（标签、小卡片） |
| `--shadow-md` | `0 4px 8px rgba(0, 0, 0, 0.4)` | 默认阴影（卡片、下拉） |
| `--shadow-lg` | `0 8px 32px rgba(0, 0, 0, 0.5)` | 模态、浮层、抽屉 |

- `--shadow` 为 `--shadow-md` 的别名，作为默认阴影。
> 阴影一律使用黑色低透明度，禁止使用彩色发光阴影（见设计原则：无暗色发光）。

---

## Z-Index Scale（层叠标尺）

语义化标尺，禁止使用裸数字。新增层级须落入下列插槽。

| 语义 | 值 | 用途 |
| --- | --- | --- |
| `dropdown` | 100 | 下拉菜单、自动补全 |
| `sticky` | 200 | 吸顶表头、粘性工具栏 |
| `overlay` | 1000 | 全屏遮罩、抽屉 |
| `modal` | 2000 | 模态对话框 |
| `toast` | 3000 | 通知 / Toast |
| `tooltip` | 4000 | 悬浮提示（最高） |

---

## Motion（动效）

### 缓动

| 名称 | 值 | 用途 |
| --- | --- | --- |
| ease-out-quart | `cubic-bezier(0.25, 1, 0.5, 1)` | 默认缓动（出场、过渡） |

### 时长

| 语义 | 值 | 用途 |
| --- | --- | --- |
| 快速 | 150ms | 微交互（hover、激活、选中态） |
| 标准 | 300ms | 展开、折叠、面板切换 |

### reduced-motion 兜底

全局通过 `@media (prefers-reduced-motion: reduce)` 兜底：将所有 transition / animation 时长置为 `0.01ms`，保证无障碍偏好下不产生动效。

---

## 设计原则

1. **Off-black 而非纯黑**：主背景 `#0f1923`（`oklch(0.209 …)`），避免纯黑 `#000` 造成的生硬对比与 OLED 拖影。
2. **Off-white 而非纯白**：主文字 `#e8ecf1`（`oklch(0.941 …)`），避免纯白 `#fff` 在深色背景上的刺眼眩光。
3. **染色中性色（蓝灰色族）**：背景、文字、边框、强调色 `--accent-purple` 共享相近色相（H ≈ 249–256），构成统一的蓝灰中性色族，而非无彩色的灰。
4. **单一强调色策略**：同一视图只允许一个强调色承担「焦点」职责（通过 `--accent` 别名切换），`--accent-neon` 仅用于关键 CTA，避免多色竞争。
5. **无渐变文字**：文字一律使用纯色 token，禁止 `background-clip: text` 渐变文字。
6. **无侧条纹边框**：禁止使用 `border-left` 彩色条纹作为区块装饰；分隔依靠 `--border` 与间距。
7. **无暗色发光**：禁止使用彩色 `box-shadow` / `text-shadow` 发光效果；阴影一律为黑色低透明度（见 Shadow）。

---

## Token 来源与维护

- 所有 token 定义于 `src/styles/global.css` 的 `:root`。
- 浮动窗专用 token（`--bg-floating*`）会在浅色主题中被覆盖，组件须引用变量而非硬编码。
- 对话宽度 token（`--chat-width-*`、`--chat-msg-max-width`、`--sidebar-width`）由 `ChatPanel.tsx` 在运行时通过 `setProperty` 切换。
- 本文件与 `global.css` 保持单一事实来源关系：token 变更须同步更新本文件。
