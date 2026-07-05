/**
 * 共享 Modal 组件 — 统一所有 Dialog 的 backdrop / Esc / a11y / focus 行为。
 *
 * 替代 4+ 个 Modal(KnowledgeCardDialog / ExportDialog / TemplatesDialog /
 * VisualCreatorDialog / SkillMarketplace)的重复实现。
 *
 * 功能:
 * - `role="dialog" aria-modal="true" aria-labelledby`
 * - Esc 键关闭
 * - 点击 backdrop 关闭
 * - 自动 focus 第一个可交互元素(简化: autofocus 标记)
 * - 禁用背景滚动(body.overflow = hidden)
 */
import { useEffect, useRef, type ReactNode } from 'preact/compat';
import { t } from '../i18n';

export interface ModalProps {
  /** 是否显示 */
  open: boolean;
  /** 标题(走 i18n) */
  title: string;
  /** 关闭回调 */
  onClose: () => void;
  /** 内容 */
  children: ReactNode;
  /** 底部操作按钮区(可选) */
  footer?: ReactNode;
  /** 宽度大小:sm=480 / md=640 / lg=900 / xl=1200,默认 md */
  size?: 'sm' | 'md' | 'lg' | 'xl';
  /** 是否允许点击 backdrop 关闭,默认 true */
  closeOnBackdrop?: boolean;
}

const SIZE_WIDTH: Record<NonNullable<ModalProps['size']>, string> = {
  sm: '480px',
  md: '640px',
  lg: '900px',
  xl: '1200px',
};

export function Modal({
  open,
  title,
  onClose,
  children,
  footer,
  size = 'md',
  closeOnBackdrop = true,
}: ModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);

  // Esc 关闭 + 禁用背景滚动
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener('keydown', onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    // autofocus 第一个可交互元素
    const t = window.setTimeout(() => {
      const node = dialogRef.current;
      if (!node) return;
      const focusable = node.querySelector<HTMLElement>(
        'button, [href], input, textarea, select, [tabindex]:not([tabindex="-1"])'
      );
      if (focusable) focusable.focus();
    }, 0);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.body.style.overflow = prevOverflow;
      window.clearTimeout(t);
    };
  }, [open, onClose]);

  if (!open) return null;

  const titleId = 'modal-title';

  return (
    <div
      class="modal-backdrop"
      onClick={(e) => {
        if (closeOnBackdrop && e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        class="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        style={`max-width: ${SIZE_WIDTH[size]}; width: 92vw;`}
      >
        <div class="modal__header">
          <h3 id={titleId}>{title}</h3>
          <button
            class="modal__close"
            onClick={onClose}
            aria-label={t('common.close') || '关闭'}
            title={t('common.close') || '关闭'}
          >
            ×
          </button>
        </div>
        <div class="modal__body">{children}</div>
        {footer && <div class="modal__actions">{footer}</div>}
      </div>
    </div>
  );
}
