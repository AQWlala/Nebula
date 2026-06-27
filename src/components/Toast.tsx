/**
 * v1.0: toast / notification stack.
 *
 * The store lives in module scope so any component can call
 * `toast.success('Saved!')` without prop-drilling.  The
 * `<Toasts />` renderer is mounted once at the top of the App.
 */
import { signal } from '@preact/signals';
import { useEffect } from 'preact/hooks';
import { t } from '../i18n';

export type ToastLevel = 'info' | 'success' | 'warning' | 'error';

export interface Toast {
  id: number;
  level: ToastLevel;
  title: string;
  body?: string;
  /** ms until auto-dismiss.  0 = sticky. */
  ttlMs: number;
}

export const toasts = signal<Toast[]>([]);

let nextId = 1;

export function showToast(level: ToastLevel, title: string, body?: string, ttlMs = 4000): number {
  const id = nextId++;
  toasts.value = [...toasts.value, { id, level, title, body, ttlMs }];
  if (ttlMs > 0) {
    setTimeout(() => dismissToast(id), ttlMs);
  }
  return id;
}

export function dismissToast(id: number) {
  toasts.value = toasts.value.filter((t0) => t0.id !== id);
}

export const toast = {
  info: (title: string, body?: string) => showToast('info', title, body),
  success: (title: string, body?: string) => showToast('success', title, body),
  warning: (title: string, body?: string) => showToast('warning', title, body),
  error: (title: string, body?: string) => showToast('error', title, body, 8000),
};

export function Toasts() {
  const list = toasts.value;
  // No-op effect: re-render on signal change is automatic in
  // Preact, but a small useEffect keeps the array pinned to the
  // signal lifecycle (helps tests).
  useEffect(() => { /* */ }, [list]);
  return (
    <div class="toast-stack" role="region" aria-label="Notifications">
      {list.map((tt) => (
        <div key={tt.id} class={`toast toast-${tt.level}`} role="alert">
          <div class="toast-title">{tt.title}</div>
          {tt.body && <div class="toast-body">{tt.body}</div>}
          <button class="toast-close" onClick={() => dismissToast(tt.id)} aria-label="Dismiss">×</button>
        </div>
      ))}
    </div>
  );
}

/** Translate a `CommandError`-shaped object into a toast. */
export function toastFromError(err: unknown) {
  const e = err as { code?: string; message?: string };
  const code = e?.code ?? 'internal';
  const msg = e?.message ?? String(err);
  toast.error(`${t('toast.error')} · ${code}`, msg);
}
