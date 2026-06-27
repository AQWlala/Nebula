/**
 * v1.0: Toast store tests.
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { toasts, showToast, dismissToast, toast } from '../Toast';

describe('Toast store', () => {
  beforeEach(() => {
    toasts.value = [];
    vi.useFakeTimers();
  });

  it('pushes a toast and assigns a unique id', () => {
    const id1 = showToast('info', 'one');
    const id2 = showToast('info', 'two');
    expect(id1).not.toBe(id2);
    expect(toasts.value).toHaveLength(2);
  });

  it('dismisses by id', () => {
    const id = showToast('info', 'x');
    expect(toasts.value).toHaveLength(1);
    dismissToast(id);
    expect(toasts.value).toHaveLength(0);
  });

  it('auto-dismisses after ttl', () => {
    showToast('info', 'a', undefined, 1000);
    expect(toasts.value).toHaveLength(1);
    vi.advanceTimersByTime(1100);
    expect(toasts.value).toHaveLength(0);
  });

  it('exposes the four sugar helpers', () => {
    toast.info('i');
    toast.success('s');
    toast.warning('w');
    toast.error('e');
    expect(toasts.value).toHaveLength(4);
    expect(toasts.value.map((t) => t.level)).toEqual(['info', 'success', 'warning', 'error']);
  });
});
