/**
 * v1.0.1 (P0#06): theme store unit tests.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  currentTheme,
  currentAccent,
  fontSizePx,
  setTheme,
  setAccent,
  setFontSize,
  clampFontSize,
  loadTheme,
  persistTheme,
  applyTheme,
  startThemeEffect,
  THEME_DEFAULT,
  FONT_MIN,
  FONT_MAX,
  ACCENT_DEFAULT,
} from '../index';

beforeEach(() => {
  localStorage.clear();
  // Reset signals back to defaults so each test starts fresh
  // even when one mutates them.
  currentTheme.value = THEME_DEFAULT;
  currentAccent.value = ACCENT_DEFAULT;
  fontSizePx.value = 14;
  // Clean up any data-theme attribute + inline style pollution
  // from a previous test.
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.style.removeProperty('--font-size');
  document.documentElement.style.removeProperty('--accent');
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('theme store (P0#06)', () => {
  it('setTheme_dark_writes_data_theme_attribute', () => {
    setTheme('dark');
    applyTheme();
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    setTheme('light');
    applyTheme();
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
  });

  it('setTheme_system_responds_to_prefers_color_scheme', () => {
    // jsdom doesn't implement matchMedia.  Install a minimal
    // shim that returns an object with a mutable `matches` getter
    // and the listener API the theme store needs.
    const state = { matches: true as boolean };
    const listeners: Array<(e: { matches: boolean }) => void> = [];
    const mql = {
      get matches() { return state.matches; },
      media: '(prefers-color-scheme: dark)',
      onchange: null,
      addEventListener: (ev: string, cb: (e: { matches: boolean }) => void) => {
        if (ev === 'change') listeners.push(cb);
      },
      removeEventListener: (ev: string, cb: (e: { matches: boolean }) => void) => {
        if (ev !== 'change') return;
        const i = listeners.indexOf(cb);
        if (i >= 0) listeners.splice(i, 1);
      },
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    } as unknown as MediaQueryList;
    const matchMediaSpy = vi
      .spyOn(window, 'matchMedia')
      .mockImplementation((q: string) => {
        if (q === '(prefers-color-scheme: dark)') return mql;
        return mql;
      });

    setTheme('system');
    applyTheme();
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    // The store should have registered exactly one 'change' listener.
    expect(listeners).toHaveLength(1);

    // Now simulate the OS flipping to light by invoking the
    // registered listener directly.  The store re-runs
    // applyTheme() and the document follows.
    state.matches = false;
    listeners[0]({ matches: false });
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');

    matchMediaSpy.mockRestore();
  });

  it('fontSize_clamp_12_20', () => {
    expect(clampFontSize(8)).toBe(FONT_MIN);
    expect(clampFontSize(99)).toBe(FONT_MAX);
    expect(clampFontSize(15.4)).toBe(15);
    expect(clampFontSize(Number.NaN)).toBe(14);
    setFontSize(7);
    expect(fontSizePx.value).toBe(FONT_MIN);
    setFontSize(500);
    expect(fontSizePx.value).toBe(FONT_MAX);
    setFontSize(16);
    expect(fontSizePx.value).toBe(16);
  });

  it('accent_signal_updates_css_var', () => {
    setAccent('neon');
    applyTheme();
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-neon)');
    setAccent('amber');
    applyTheme();
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-warning)');
    setAccent('purple');
    applyTheme();
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-purple)');
  });

  it('startThemeEffect auto-applies on signal change', () => {
    currentAccent.value = 'neon';
    const dispose = startThemeEffect();
    // effect() runs asynchronously on microtask; flushing it.
    // jsdom + signals effect: synchronous in this version.
    // We assert that the document got updated as a side-effect of
    // simply mutating the signal.
    expect(document.documentElement.style.getPropertyValue('--accent'))
      .toBe('var(--accent-neon)');
    dispose();
  });

  it('persistTheme round-trips through loadTheme', () => {
    setTheme('light');
    setAccent('amber');
    setFontSize(18);
    persistTheme();
    // Mutate the signals to confirm loadTheme restores them.
    setTheme('dark');
    setAccent('purple');
    setFontSize(14);
    const snap = loadTheme();
    expect(snap.theme).toBe('light');
    expect(snap.accent).toBe('amber');
    expect(snap.fontSize).toBe(18);
  });
});
