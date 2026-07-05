/**
 * EmptyState — 统一所有列表 / 面板的空状态显示。
 *
 * 复用 global.css 中已有的 `.empty-state` 样式(虚线边框 + muted 文本)。
 * 文字由调用方经 i18n 预先翻译后传入,本组件不再二次 t()。
 * 可选 action 按钮用于引导用户进行下一步(如"新建"、"导入")。
 */
import type { ComponentChildren } from 'preact';

export interface EmptyStateProps {
  /** emoji 或 svg 字符,如 '📭' */
  icon?: string;
  /** 主标题(调用方已 i18n) */
  title: string;
  /** 描述文字(调用方已 i18n) */
  description?: string;
  /** action 按钮文字(调用方已 i18n) */
  actionLabel?: string;
  /** action 按钮点击回调 */
  onAction?: () => void;
  /** 直接传入子节点时使用(优先级高于 actionLabel) */
  children?: ComponentChildren;
}

export function EmptyState({
  icon,
  title,
  description,
  actionLabel,
  onAction,
  children,
}: EmptyStateProps) {
  return (
    <div class="empty-state">
      {icon && (
        <div style="font-size: 48px; margin-bottom: 16px; line-height: 1;">
          {icon}
        </div>
      )}
      <div style="font-size: 14px; font-weight: 600; color: var(--text);">
        {title}
      </div>
      {description && (
        <div style="font-size: 12px; margin-top: 8px; color: var(--text-muted);">
          {description}
        </div>
      )}
      {children}
      {actionLabel && onAction && (
        <button
          class="btn"
          onClick={onAction}
          style="margin-top: 16px;"
        >
          {actionLabel}
        </button>
      )}
    </div>
  );
}

export default EmptyState;
