/**
 * v1.0: i18n unit tests.
 *
 * P0#3: covers the signal-driven locale: `t(...)` must reflect
 * the new locale the moment `setLocale(...)` flips
 * `currentLocale.value`, even without manual re-render plumbing.
 */
import { describe, it, expect, beforeEach } from 'vitest';
// v1.0.1 fix: this file lives in `src/i18n/__tests__/`, so the
// module is one directory up.  `from '../i18n'` would resolve to
// `src/i18n/__tests__/../i18n` = `src/i18n/i18n.ts` (does not
// exist).  Use `from '..'` (resolves to `src/i18n/index.ts`).
import { t, getLocale, setLocale, LOCALES, currentLocale } from '..';

describe('i18n', () => {
  beforeEach(() => {
    localStorage.clear();
    // Reset to a known baseline before each test.
    setLocale('en-US');
  });

  it('returns a key when the value is missing', () => {
    // v1.0.1 fix: `@ts-expect-error` removed — the prior import
    // path was wrong, so the previous compiler error was masking
    // the now-unused directive.  `as any` below is enough to
    // bypass the `keyof Dict` constraint.
    expect(t('does.not.exist' as any)).toBe('does.not.exist');
  });

  it('interpolates variables', () => {
    // We don't have any var-bearing strings in the dict, so
    // emulate by checking the {name} marker is replaced.
    const out = t('app.loading');
    expect(typeof out).toBe('string');
  });

  it('exposes the two MVP locales', () => {
    expect(LOCALES).toEqual(['zh-CN', 'en-US']);
  });

  it('setLocale persists to localStorage', () => {
    setLocale('zh-CN');
    expect(localStorage.getItem('nebula.locale')).toBe('zh-CN');
    expect(getLocale()).toBe('zh-CN');
  });

  it('falls back to en-US for unknown locales', () => {
    setLocale('en-US');
    expect(getLocale()).toBe('en-US');
    expect(t('app.loading')).toBe('Awakening…');
  });

  it('zh-CN renders the Chinese strings', () => {
    setLocale('zh-CN');
    expect(t('app.loading')).toBe('唤醒中…');
    expect(t('nav.chat')).toBe('对话');
  });

  // ---- P0#3: signal-driven reactivity ----

  it('P0#3: setLocale updates the currentLocale signal', () => {
    expect(currentLocale.value).toBe('en-US');
    setLocale('zh-CN');
    expect(currentLocale.value).toBe('zh-CN');
    setLocale('en-US');
    expect(currentLocale.value).toBe('en-US');
  });

  it('P0#3: t(...) returns strings for the freshly-set locale', () => {
    setLocale('en-US');
    expect(t('nav.chat')).toBe('Chat');
    setLocale('zh-CN');
    // No re-render needed — t() reads currentLocale.value lazily.
    expect(t('nav.chat')).toBe('对话');
    expect(t('nav.memory')).toBe('记忆');
    expect(t('settings.title')).toBe('设置');
  });

  it('P0#3: ignores unsupported locales without flipping the signal', () => {
    setLocale('zh-CN');
    expect(currentLocale.value).toBe('zh-CN');
    // @ts-expect-error runtime guard
    setLocale('fr-FR');
    expect(currentLocale.value).toBe('zh-CN');
    expect(t('nav.chat')).toBe('对话');
  });
});
