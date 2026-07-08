/**
 * v1.0: ErrorBoundary tests.
 */
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/preact';
import type { ComponentChildren } from 'preact';
import { ErrorBoundary, readCrashLog } from '../ErrorBoundary';

afterEach(() => cleanup());

// v1.0.1 fix: 显式标注 `ComponentChildren` 返回类型。`throw` 让
// TypeScript 推断返回 `never`，与 `ComponentChildren` 不兼容。
// 这里我们返回 `null` —— 运行时永远到达不到，但类型层面合法。
function Boom(): ComponentChildren {
  throw new Error('boom-test');
}

describe('ErrorBoundary', () => {
  it('renders children when no error', () => {
    const { getByText } = render(
      <ErrorBoundary>
        <div>ok</div>
      </ErrorBoundary>
    );
    expect(getByText('ok')).toBeTruthy();
  });

  it('catches and renders an error card', () => {
    // Suppress the React/Preact error log for this test.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { getByText } = render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>
    );
    expect(getByText('boom-test')).toBeTruthy();
    expect(spy).toHaveBeenCalled();
    spy.mockRestore();
  });

  it('records the crash to localStorage', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    localStorage.clear();
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>
    );
    const log = readCrashLog();
    expect(log).toHaveLength(1);
    expect(log[0].message).toBe('boom-test');
    spy.mockRestore();
  });
});
