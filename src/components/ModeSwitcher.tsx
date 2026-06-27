/**
 * v0.5: 顶栏三模式切换器
 *
 * 在主面板顶部放三个大按钮：Writing / Work / Code。
 * 当前模式高亮；点击切换 mode signal。下面挂载对应视图。
 *
 * 设计目标：
 * - 与现有 Sidebar 风格一致（深色 + 霓虹绿）
 * - 大尺寸、图标 + 标题 + 简介三行
 * - 模式可拖拽内容（占位，v1.0 完善）
 */
import { NineSnakeStore } from '../stores/nineSnakeStore';

type Mode = 'writing' | 'work' | 'code';

interface ModeMeta {
  id: Mode;
  icon: string;
  label: string;
  subtitle: string;
  accent: string;
}

const MODES: ModeMeta[] = [
  { id: 'writing', icon: '✍️', label: 'Writing', subtitle: '长文 / 模板 / 导出', accent: '#39d98a' },
  { id: 'work',    icon: '📋', label: 'Work',    subtitle: '看板 / 时间 / 会议', accent: '#ffb86b' },
  { id: 'code',    icon: '💻', label: 'Code',    subtitle: '文件 / 编辑 / 终端', accent: '#5fa8ff' },
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
            style={active ? { boxShadow: `0 0 0 1px ${m.accent}` } : undefined}
            onClick={() => (NineSnakeStore.mode.value = m.id)}
          >
            <span class="mode-icon" style={{ color: active ? m.accent : undefined }}>
              {m.icon}
            </span>
            <span class="mode-body">
              <span class="mode-label">{m.label}</span>
              <span class="mode-subtitle">{m.subtitle}</span>
            </span>
          </button>
        );
      })}
      <div class="mode-spacer" />
      <div class="mode-hint">提示：模式间可拖拽内容（v1.0）</div>
    </div>
  );
}
