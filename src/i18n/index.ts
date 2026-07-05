/**
 * v1.0: lightweight i18n.
 *
 * Two-locale MVP (zh-CN, en-US).  Locale is read from
 * `localStorage` first, then `navigator.language`, then defaults
 * to `en-US`.  The set of supported locales is fixed; we do not
 * fall back across files — an unknown locale falls back to en-US.
 *
 * P0#3 fix: the current locale is now exposed as a
 * `@preact/signals` signal so any component that reads
 * `currentLocale.value` inside JSX (or `t(...)`, which reads the
 * signal internally) automatically re-renders when the user
 * switches language.  The old `onLocaleChange` callback API had
 * zero subscribers and was therefore dead code.
 */
import { signal } from '@preact/signals';
import enUS from './en-US.json';
import zhCN from './zh-CN.json';

export type Locale = 'zh-CN' | 'en-US';

export type Dict = typeof enUS;

const DICTS: Record<Locale, Dict> = {
  'en-US': enUS as Dict,
  'zh-CN': zhCN as unknown as Dict,
};

const STORAGE_KEY = 'nebula.locale';

function detectLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY) as Locale | null;
    if (saved && saved in DICTS) return saved;
  } catch {
    /* localStorage may be unavailable; fall through */
  }
  const nav = (navigator?.language || 'en-US').toLowerCase();
  if (nav.startsWith('zh')) return 'zh-CN';
  return 'en-US';
}

/** P0#3: signal-driven current locale.  Reading `.value` inside
 *  a component's render automatically subscribes to changes. */
export const currentLocale = signal<Locale>(detectLocale());

export function getLocale(): Locale {
  return currentLocale.value;
}

export function setLocale(l: Locale): void {
  if (!(l in DICTS)) return;
  currentLocale.value = l;
  try {
    localStorage.setItem(STORAGE_KEY, l);
  } catch {
    /* ignore */
  }
}

/**
 * Look up `key` in the *current* locale's dictionary.  Falls
 * back to en-US, then to the raw key.  Reading
 * `currentLocale.value` is what wires this into the signal so the
 * whole tree re-renders on locale change.
 */
export function t(key: keyof Dict, vars?: Record<string, string | number>): string {
  const locale = currentLocale.value;
  const dict = DICTS[locale] || DICTS['en-US'];
  const raw = (dict as Record<string, string>)[key as string]
    ?? (DICTS['en-US'] as Record<string, string>)[key as string]
    ?? key;
  if (!vars) return raw;
  return Object.entries(vars).reduce(
    (acc, [k, v]) => acc.replace(new RegExp(`\\{${k}\\}`, 'g'), String(v)),
    raw,
  );
}

export const LOCALES: Locale[] = ['zh-CN', 'en-US'];

export const dict = DICTS;
