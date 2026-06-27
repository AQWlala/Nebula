/**
 * v1.0: Settings unit tests.
 *
 * P0#4 covers:
 *  - changing font size sets --font-size on <html>
 *  - changing accent sets --accent on <html> to one of the
 *    named CSS variable references
 *  - clampFontSize keeps the value inside [12, 20]
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/preact';
import { Settings, __test__ } from '../Settings';
import { setLocale } from '../../i18n';

afterEach(() => {
  cleanup();
  localStorage.clear();
  document.documentElement.style.removeProperty('--font-size');
  document.documentElement.style.removeProperty('--accent');
  vi.restoreAllMocks();
});

describe('Settings (P0#4)', () => {
  beforeEach(() => {
    localStorage.clear();
    // Make sure the UI labels are deterministic regardless of
    // whatever locale the previous test file left behind.
    setLocale('en-US');
  });

  it('exposes the three accent presets', () => {
    const values = __test__.ACCENT_OPTIONS.map((o) => o.value);
    expect(values).toEqual(['purple', 'neon', 'amber']);
    // Each preset must reference a real CSS token in the design
    // system so the global stylesheet can resolve it.
    for (const o of __test__.ACCENT_OPTIONS) {
      expect(o.cssVar.startsWith('--accent')).toBe(true);
    }
  });

  it('clamps font size to 12..20 with step 1', () => {
    expect(__test__.clampFontSize(8)).toBe(__test__.FONT_MIN);
    expect(__test__.clampFontSize(99)).toBe(__test__.FONT_MAX);
    expect(__test__.clampFontSize(15.4)).toBe(15);
    expect(__test__.FONT_STEP).toBe(1);
  });

  it('applies --font-size to <html> on mount', () => {
    render(<Settings onClose={() => {}} />);
    // Default font size is 14 (set by the input's `value`).
    expect(document.documentElement.style.getPropertyValue('--font-size'))
      .toBe('14px');
  });

  it('updates --font-size when the user changes the input', () => {
    const { getByDisplayValue } = render(<Settings onClose={() => {}} />);
    const input = getByDisplayValue('14') as HTMLInputElement;
    fireEvent.input(input, { target: { value: '18' } });
    // The component also commits on save() — but the useEffect
    // that mirrors state → CSS variable runs synchronously on
    // re-render, so the side-effect must already be visible.
    expect(document.documentElement.style.getPropertyValue('--font-size'))
      .toBe('18px');
  });

  it('clamps an out-of-range value before applying it', () => {
    const { getByDisplayValue } = render(<Settings onClose={() => {}} />);
    const input = getByDisplayValue('14') as HTMLInputElement;
    fireEvent.input(input, { target: { value: '999' } });
    expect(document.documentElement.style.getPropertyValue('--font-size'))
      .toBe(`${__test__.FONT_MAX}px`);
  });

  it('switching accent updates --accent on <html>', () => {
    const { getByText } = render(<Settings onClose={() => {}} />);
    // Initial accent is "purple".
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-purple)');
    fireEvent.click(getByText('Neon green'));
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-neon)');
    fireEvent.click(getByText('Amber gold'));
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-warning)');
  });

  it('save() persists the clamped values to localStorage', () => {
    const { getByDisplayValue, getByText } = render(<Settings onClose={() => {}} />);
    const input = getByDisplayValue('14') as HTMLInputElement;
    fireEvent.input(input, { target: { value: '17' } });
    fireEvent.click(getByText('Save'));
    const raw = localStorage.getItem('nine-snake.settings');
    expect(raw).toBeTruthy();
    const parsed = JSON.parse(raw!);
    expect(parsed.fontSize).toBe(17);
    expect(parsed.accent).toBe('purple');
  });
});
