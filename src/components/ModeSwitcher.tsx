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
import { nebulaStore } from '../stores/nebulaStore';
import { t } from '../i18n';

type Mode = 'writing' | 'work' | 'code';

interface ModeMeta {
  id: Mode;
  icon: string;
  accent: string;
  /** v1.7: 快捷键数字（1/2/3），用于提示。 */
  shortcutNum: number;
}

const MODES: ModeMeta[] = [
  { id: 'writing', icon: '✍️', accent: '#39d98a', shortcutNum: 1 },
  { id: 'work', icon: '📋', accent: '#ffb86b', shortcutNum: 2 },
  { id: 'code', icon: '💻', accent: '#5fa8ff', shortcutNum: 3 },
];

function modeLabel(m: Mode): string {
  return t(`modeSwitcher.${m}.label`);
}

function modeSubtitle(m: Mode): string {
  return t(`modeSwitcher.${m}.subtitle`);
}

export function ModeSwitcher() {
  const current = nebulaStore.mode.value;

  /**
   * T-S5-A-03: 手动切换模式时,若与最近一次自动路由结果不同,
   * 递增 `modeMisclassification` 计数(供指标上报)。
   */
  function handleManualSwitch(mode: Mode): void {
    const lastAuto = nebulaStore.lastAutoRoutedMode.value;
    if (lastAuto !== null && lastAuto !== mode) {
      nebulaStore.modeMisclassification.value += 1;
    }
    nebulaStore.mode.value = mode;
  }

  return (
    <div class="mode-switcher" role="tablist" aria-label={t('modeSwitcher.ariaLabel')}>
      {MODES.map((m) => {
        const active = current === m.id;
        return (
          <button
            key={m.id}
            role="tab"
            aria-selected={active}
            class={`mode-pill ${active ? 'active' : ''}`}
            style={
              active ? { boxShadow: `0 0 0 1px ${m.accent}`, borderColor: m.accent } : undefined
            }
            onClick={() => handleManualSwitch(m.id)}
          >
            <span class="mode-icon" style={{ color: active ? m.accent : undefined }}>
              {m.icon}
            </span>
            <span class="mode-body">
              <span class="mode-label">{modeLabel(m.id)}</span>
              <span class="mode-subtitle">{modeSubtitle(m.id)}</span>
            </span>
            <span
              class="mode-shortcut-hint"
              title={t('modeSwitcher.shortcutHint', { num: m.shortcutNum })}
            >
              {m.shortcutNum}
            </span>
          </button>
        );
      })}
      <div class="mode-spacer" />
      <div class="mode-hint">
        {t('modeSwitcher.hint', {
          mode: nebulaStore.aiAutoMode.value
            ? t('modeSwitcher.llmRoute')
            : t('modeSwitcher.keywordHeuristic'),
        })}
      </div>
    </div>
  );
}
