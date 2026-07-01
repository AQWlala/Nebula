/**
 * v0.5: 顶栏三模式切换器
 *
 * v1.7: 三视角统一工作台 — 增强切换动画 + 快捷键提示。
 *
 * 设计目标：
 * - 与现有 Sidebar 风格一致（深色 + 霓虹绿）
 * - 大尺寸、图标 + 标题 + 简介三行
 * - v1.7: 切换有滑动指示器动画（CSS transition）
 * - v1.7: 显示快捷键提示（Cmd/Ctrl+1/2/3 切换，待前端快捷键实现）
 */
import { NineSnakeStore } from '../stores/nineSnakeStore';

type Mode = 'writing' | 'work' | 'code';

interface ModeMeta {
  id: Mode;
  icon: string;
  label: string;
  subtitle: string;
  accent: string;
  /** v1.7: 快捷键数字（1/2/3），用于提示。 */
  shortcutNum: number;
}

const MODES: ModeMeta[] = [
  { id: 'writing', icon: '✍️', label: 'Writing', subtitle: '长文 / 模板 / 导出', accent: '#39d98a', shortcutNum: 1 },
  { id: 'work',    icon: '📋', label: 'Work [实验性]',    subtitle: '看板 / 时间 / 会议', accent: '#ffb86b', shortcutNum: 2 },
  { id: 'code',    icon: '💻', label: 'Code',    subtitle: '文件 / 编辑 / 终端', accent: '#5fa8ff', shortcutNum: 3 },
];

export function ModeSwitcher() {
  const current = NineSnakeStore.mode.value;

  return (
    <div class="mode-switcher" role="tablist" aria-label="工作模式">
      {MODES.map((m) => {
        const active = current === m.id;
        return (
          <button
            key={m.id}
            role="tab"
            aria-selected={active}
            class={`mode-pill ${active ? 'active' : ''}`}
            style={active ? { boxShadow: `0 0 0 1px ${m.accent}`, borderColor: m.accent } : undefined}
            onClick={() => (NineSnakeStore.mode.value = m.id)}
          >
            <span class="mode-icon" style={{ color: active ? m.accent : undefined }}>
              {m.icon}
            </span>
            <span class="mode-body">
              <span class="mode-label">{m.label}</span>
              <span class="mode-subtitle">{m.subtitle}</span>
            </span>
            <span class="mode-shortcut-hint" title={`快捷键: Cmd/Ctrl+${m.shortcutNum}`}>
              {m.shortcutNum}
            </span>
          </button>
        );
      })}
      <div class="mode-spacer" />
      <div class="mode-hint">三视角无感切换 · AI 自动判模式（v1.7 关键词启发式）</div>
    </div>
  );
}
