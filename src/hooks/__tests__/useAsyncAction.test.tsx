/**
 * P0-C: useAsyncAction hook unit tests.
 *
 * Tests the hook through a minimal Preact wrapper. We verify
 * Promise return values (deterministic) rather than chasing
 * Preact re-render timing for loading/data state.
 */
import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/preact';
import { useAsyncAction } from '../useAsyncAction';

function renderHook<A extends any[], T>(action: (...args: A) => Promise<T>) {
  const store: { hook: ReturnType<typeof useAsyncAction<T, A>> | null } = { hook: null };
  function Tester() {
    store.hook = useAsyncAction(action);
    return null;
  }
  const { rerender } = render(<Tester />);
  return { hook: () => store.hook!, rerender };
}

describe('useAsyncAction', () => {
  it('initial loading is false', () => {
    const { hook } = renderHook(async () => 'ok');
    expect(hook().loading).toBe(false);
  });

  it('returns value from action via run()', async () => {
    const fn = vi.fn(async (x: number) => x * 2);
    const { hook } = renderHook(fn);
    const r = await hook().run(21);
    expect(r).toBe(42);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('propagates error', async () => {
    const fn = vi.fn(async () => { throw new Error('boom'); });
    const { hook } = renderHook(fn);
    await expect(hook().run()).rejects.toThrow('boom');
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('does not call action twice when run called concurrently', async () => {
    const fn = vi.fn(
      () => new Promise<string>((r) => setTimeout(() => r('ok'), 10)),
    );
    const { hook } = renderHook(fn);
    const p1 = hook().run();
    hook().run(); // second call: should be skipped
    await p1;
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('reset is callable without error', () => {
    const { hook } = renderHook(async () => 'x');
    expect(() => hook().reset()).not.toThrow();
  });
});
