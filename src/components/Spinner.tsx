/**
 * 共享 Spinner 组件 — 统一所有加载态。
 *
 * 替代项目中的 emoji ⏳ / 文字 "...中" / Tailwind animate-spin 三种并存实现。
 * 引用 var(--accent-neon) 保证主题一致。
 */
import { t } from '../i18n';

export interface SpinnerProps {
  /** 大小 px,默认 24 */
  size?: number;
  /** 显示文字(走 i18n),默认 "加载中…" */
  label?: string;
  /** 是否显示文字,默认 true */
  showLabel?: boolean;
}

export function Spinner({ size = 24, label, showLabel = true }: SpinnerProps) {
  const text = label ?? (t('common.loading') || '加载中…');
  return (
    <span class="spinner" role="status" aria-live="polite">
      <span
        class="spinner__circle"
        style={`width: ${size}px; height: ${size}px;`}
        aria-hidden="true"
      />
      {showLabel && <span class="spinner__label">{text}</span>}
    </span>
  );
}
