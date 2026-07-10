/**
 * T-E-S-50: 自主度滑块 L0-L5。
 *
 * 6 档水平滑块,与 ModeSwitcher(任务领域 writing/work/code)正交:
 * - L0 内联补全 / L1 定向编辑 / L2 对话(默认) /
 *   L3 Plan / L4 蜂群 / L5 后台自动化
 *
 * 样式参考 ModeSwitcher.tsx(深色 + 霓虹绿 #39d98a)。
 * 切换时调用 `invoke('autonomy_set_level', { level })`。
 *
 * 样式自包含(通过 useEffect 注入 `<style>`,不修改 global.css),
 * 避免与并行 P0 任务在 global.css 上产生合并冲突。
 */
import { useEffect, useState } from 'preact/hooks';
import { signal } from '@preact/signals';
import {
  AUTONOMY_LEVEL_INFOS,
  DEFAULT_AUTONOMY_LEVEL,
  getLevel,
  setLevel,
  type AutonomyLevel,
  type AutonomyLevelInfo,
} from '../lib/autonomy';
import { t } from '../i18n';
import { nebulaStore } from '../stores/nebulaStore';

/** 全局当前自主度等级 signal(供其他组件订阅,主 agent 集成时使用)。 */
export const currentAutonomyLevel = signal<AutonomyLevel>(DEFAULT_AUTONOMY_LEVEL);

const NEON_GREEN = '#39d98a';
const STYLE_ID = 'autonomy-slider-styles';

/**
 * T-D-F-04: 自主度等级 label/desc 的 i18n key 映射。
 *
 * 用 `as const` 推导字面量联合类型,这些字面量都是 en-US.json 的实际
 * key,因此可直接传给 `t(key: keyof Dict)` 而无需 `as keyof Dict` 断言
 * (编译期类型安全,缺 key 会在 `t()` 调用处报错)。
 *
 * 保留 `AUTONOMY_LEVEL_INFOS` 里的 `label`/`label_zh`/`description`/
 * `description_zh` 字段以兼容后端 wire 格式 (`autonomy_list_levels`
 * 返回结构),仅渲染层改走 i18n。
 */
const AUTONOMY_LABEL_KEYS = {
  L0: 'autonomy.level.l0.label',
  L1: 'autonomy.level.l1.label',
  L2: 'autonomy.level.l2.label',
  L3: 'autonomy.level.l3.label',
  L4: 'autonomy.level.l4.label',
  L5: 'autonomy.level.l5.label',
} as const;

const AUTONOMY_DESC_KEYS = {
  L0: 'autonomy.level.l0.desc',
  L1: 'autonomy.level.l1.desc',
  L2: 'autonomy.level.l2.desc',
  L3: 'autonomy.level.l3.desc',
  L4: 'autonomy.level.l4.desc',
  L5: 'autonomy.level.l5.desc',
} as const;

/** 注入组件样式一次(幂等)。 */
function injectStyles(): void {
  if (typeof document === 'undefined') return;
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = `
.autonomy-slider {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 16px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.autonomy-slider__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.autonomy-slider__title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 1px;
}
.autonomy-slider__current {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
}
.autonomy-slider__range {
  -webkit-appearance: none;
  appearance: none;
  width: 100%;
  height: 6px;
  background: var(--bg-tertiary);
  border-radius: 3px;
  outline: none;
  cursor: pointer;
  padding: 0;
}
.autonomy-slider__range::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: ${NEON_GREEN};
  border: 2px solid var(--bg-secondary);
  cursor: pointer;
  transition: transform 0.1s ease;
}
.autonomy-slider__range::-webkit-slider-thumb:hover {
  transform: scale(1.15);
}
.autonomy-slider__range::-moz-range-thumb {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: ${NEON_GREEN};
  border: 2px solid var(--bg-secondary);
  cursor: pointer;
}
.autonomy-slider__range:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.autonomy-slider__ticks {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 4px;
}
.autonomy-slider__tick {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 4px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text-secondary);
  cursor: pointer;
  transition: all 0.15s ease;
  font: inherit;
}
.autonomy-slider__tick:hover {
  color: var(--text-primary);
  border-color: ${NEON_GREEN};
}
.autonomy-slider__tick.active {
  color: ${NEON_GREEN};
  border-color: ${NEON_GREEN};
  background: rgba(57, 217, 138, 0.08);
}
.autonomy-slider__tick-level {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.5px;
}
.autonomy-slider__tick-label {
  font-size: 10px;
  color: var(--text-muted);
}
.autonomy-slider__tick.active .autonomy-slider__tick-label {
  color: inherit;
}
.autonomy-slider__desc {
  font-size: 11px;
  color: var(--text-muted);
  min-height: 14px;
}
`;
  document.head.appendChild(style);
}

export function AutonomySlider() {
  const [level, setLevelState] = useState<AutonomyLevel>(DEFAULT_AUTONOMY_LEVEL);
  const [loading, setLoading] = useState(true);

  // 注入样式(仅一次,幂等)。
  useEffect(() => {
    injectStyles();
  }, []);

  // 启动时从后端读取当前等级。
  useEffect(() => {
    let mounted = true;
    getLevel().then((lvl) => {
      if (!mounted) return;
      setLevelState(lvl);
      currentAutonomyLevel.value = lvl;
      // T-E-S-50: 同步到全局 store,供 InlineSuggestion 等组件读取。
      nebulaStore.autonomyLevel.value = lvl;
      setLoading(false);
    });
    return () => {
      mounted = false;
    };
  }, []);

  // T-D-F-04: 语言切换重渲染由 `t()` 内部读取 `currentLocale.value` 触发,
  // 无需手动订阅 locale signal。

  const infos: AutonomyLevelInfo[] = AUTONOMY_LEVEL_INFOS;
  const currentIndex = Math.max(
    0,
    infos.findIndex((i) => i.level === level)
  );
  const current = infos[currentIndex];

  function apply(next: AutonomyLevel): void {
    const prev = level;
    if (next === prev) return;
    // 乐观更新 + 失败回滚。
    setLevelState(next);
    currentAutonomyLevel.value = next;
    // T-E-S-50: 同步到全局 store,供 InlineSuggestion 等组件读取。
    nebulaStore.autonomyLevel.value = next;
    setLevel(next).catch(() => {
      setLevelState(prev);
      currentAutonomyLevel.value = prev;
      nebulaStore.autonomyLevel.value = prev;
    });
  }

  function handleRange(e: Event): void {
    const target = e.target as HTMLInputElement;
    const idx = parseInt(target.value, 10);
    const next = infos[idx]?.level ?? DEFAULT_AUTONOMY_LEVEL;
    apply(next);
  }

  return (
    <div class="autonomy-slider" role="group" aria-label={t('autonomySlider.ariaLabel')}>
      <div class="autonomy-slider__header">
        <span class="autonomy-slider__title">{t('autonomySlider.title')}</span>
        <span
          class="autonomy-slider__current"
          title={current ? t(AUTONOMY_DESC_KEYS[current.level]) : ''}
        >
          {current ? `${current.level} · ${t(AUTONOMY_LABEL_KEYS[current.level])}` : '—'}
        </span>
      </div>
      <input
        class="autonomy-slider__range"
        type="range"
        min="0"
        max="5"
        step="1"
        value={currentIndex}
        onInput={handleRange}
        disabled={loading}
        aria-label={t('autonomySlider.rangeAriaLabel')}
      />
      <div class="autonomy-slider__ticks">
        {infos.map((info) => {
          const active = info.level === level;
          return (
            <button
              key={info.level}
              type="button"
              class={`autonomy-slider__tick ${active ? 'active' : ''}`}
              onClick={() => apply(info.level)}
              title={t(AUTONOMY_DESC_KEYS[info.level])}
              aria-pressed={active}
              aria-label={`${info.level} ${t(AUTONOMY_LABEL_KEYS[info.level])}`}
            >
              <span class="autonomy-slider__tick-level">{info.level}</span>
              <span class="autonomy-slider__tick-label">{t(AUTONOMY_LABEL_KEYS[info.level])}</span>
            </button>
          );
        })}
      </div>
      <div class="autonomy-slider__desc">
        {current ? t(AUTONOMY_DESC_KEYS[current.level]) : ''}
      </div>
    </div>
  );
}
