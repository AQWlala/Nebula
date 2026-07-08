/**
 * P0#06: theme + accent + font-size signal store.
 *
 * Replaces the Settings.tsx useEffect + style.setProperty chain with
 * a signal-driven store that any component can subscribe to.  The
 * store is also the single source of truth for persisting the user's
 * preferences in localStorage and for applying them to the document.
 *
 * The "system" mode follows the OS `prefers-color-scheme` media
 * query and re-applies whenever that changes.  Switching between
 * dark / light / system is instant: read `currentTheme.value` in
 * your JSX (or call `applyTheme()`) and the CSS variables flip.
 */
import { signal, effect, type Signal } from '@preact/signals';

export type Theme = 'dark' | 'light' | 'system';
export type Accent = 'purple' | 'neon' | 'amber';

export interface ThemeSnapshot {
  theme: Theme;
  accent: Accent;
  fontSize: number;
}

const STORAGE_KEY = 'nebula.theme';
export const FONT_MIN = 12;
export const FONT_MAX = 20;
export const FONT_DEFAULT = 14;
export const ACCENT_DEFAULT: Accent = 'purple';
export const THEME_DEFAULT: Theme = 'dark';

/** Same preset list as Settings.tsx; keep them in sync. */
export const ACCENT_OPTIONS: { value: Accent; cssVar: string; preview: string }[] = [
  { value: 'purple', cssVar: '--accent-purple', preview: '#9d4edd' },
  { value: 'neon', cssVar: '--accent-neon', preview: '#00ff9d' },
  { value: 'amber', cssVar: '--accent-warning', preview: '#ffb86b' },
];

const VALID_THEMES: Theme[] = ['dark', 'light', 'system'];
const VALID_ACCENTS: Accent[] = ['purple', 'neon', 'amber'];

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

export const currentTheme: Signal<Theme> = signal<Theme>(THEME_DEFAULT);
export const currentAccent: Signal<Accent> = signal<Accent>(ACCENT_DEFAULT);
export const fontSizePx: Signal<number> = signal<number>(FONT_DEFAULT);

/** Resolved theme after `system` has been mapped to dark/light.
 *  Driven by `applyTheme()`; components should normally read
 *  `currentTheme.value` and let the data-theme attribute carry
 *  the actual state.  This is exposed for tests. */
export const resolvedTheme: Signal<'dark' | 'light'> = signal<'dark' | 'light'>('dark');

// ---------------------------------------------------------------------------
// Pure helpers (exported for tests)
// ---------------------------------------------------------------------------

export function clampFontSize(n: number): number {
  if (!Number.isFinite(n)) return FONT_DEFAULT;
  return Math.max(FONT_MIN, Math.min(FONT_MAX, Math.round(n)));
}

export function isValidTheme(v: unknown): v is Theme {
  return typeof v === 'string' && (VALID_THEMES as string[]).includes(v);
}

export function isValidAccent(v: unknown): v is Accent {
  return typeof v === 'string' && (VALID_ACCENTS as string[]).includes(v);
}

export function resolveSystemTheme(): 'dark' | 'light' {
  if (typeof window === 'undefined' || !window.matchMedia) return 'dark';
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function resolveAccentCssVar(accent: Accent): string {
  const found = ACCENT_OPTIONS.find((o) => o.value === accent);
  return found ? `var(${found.cssVar})` : 'var(--accent-purple)';
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

function readStored(): ThemeSnapshot {
  if (typeof localStorage === 'undefined') {
    return { theme: THEME_DEFAULT, accent: ACCENT_DEFAULT, fontSize: FONT_DEFAULT };
  }
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { theme: THEME_DEFAULT, accent: ACCENT_DEFAULT, fontSize: FONT_DEFAULT };
    const parsed = JSON.parse(raw) as Partial<ThemeSnapshot>;
    return {
      theme: isValidTheme(parsed.theme) ? parsed.theme : THEME_DEFAULT,
      accent: isValidAccent(parsed.accent) ? parsed.accent : ACCENT_DEFAULT,
      fontSize: clampFontSize(typeof parsed.fontSize === 'number' ? parsed.fontSize : FONT_DEFAULT),
    };
  } catch {
    return { theme: THEME_DEFAULT, accent: ACCENT_DEFAULT, fontSize: FONT_DEFAULT };
  }
}

export function loadTheme(): ThemeSnapshot {
  const snap = readStored();
  currentTheme.value = snap.theme;
  currentAccent.value = snap.accent;
  fontSizePx.value = snap.fontSize;
  return snap;
}

export function persistTheme(): void {
  if (typeof localStorage === 'undefined') return;
  try {
    const snap: ThemeSnapshot = {
      theme: currentTheme.value,
      accent: currentAccent.value,
      fontSize: fontSizePx.value,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(snap));
  } catch {
    /* ignore quota / private-mode errors */
  }
}

// ---------------------------------------------------------------------------
// Mutators
// ---------------------------------------------------------------------------

export function setTheme(t: Theme): void {
  if (!isValidTheme(t)) return;
  currentTheme.value = t;
}

export function setAccent(a: Accent): void {
  if (!isValidAccent(a)) return;
  currentAccent.value = a;
}

export function setFontSize(n: number): void {
  fontSizePx.value = clampFontSize(n);
}

// ---------------------------------------------------------------------------
// DOM application
// ---------------------------------------------------------------------------

/** `MediaQueryList` listener handle for the "system" mode. */
let mql: MediaQueryList | null = null;
let mqlHandler: ((e: MediaQueryListEvent) => void) | null = null;

/** Apply all three preferences to the document.  Idempotent. */
export function applyTheme(): 'dark' | 'light' {
  if (typeof document === 'undefined') return 'dark';
  const theme = currentTheme.value;
  const accent = currentAccent.value;
  const font = fontSizePx.value;

  const resolved: 'dark' | 'light' = theme === 'system' ? resolveSystemTheme() : theme;
  document.documentElement.setAttribute('data-theme', resolved);
  document.documentElement.style.setProperty('--font-size', `${clampFontSize(font)}px`);
  document.documentElement.style.setProperty('--accent', resolveAccentCssVar(accent));
  resolvedTheme.value = resolved;

  // Re-install the media-query listener when (and only when) we
  // actually need it.  Tearing it down on dark/light avoids leaks
  // and prevents the listener from racing with an explicit pick.
  if (theme === 'system' && typeof window !== 'undefined' && window.matchMedia) {
    if (!mql) {
      mql = window.matchMedia('(prefers-color-scheme: dark)');
      mqlHandler = () => applyTheme();
      mql.addEventListener('change', mqlHandler);
    }
  } else if (mql && mqlHandler) {
    mql.removeEventListener('change', mqlHandler);
    mql = null;
    mqlHandler = null;
  }

  return resolved;
}

/**
 * Drive the DOM application automatically when any of the three
 * signals change.  Call once at app boot; the returned disposer
 * tears down the subscription.  Safe to call multiple times: the
 * most recent disposer wins.
 */
let effectDisposer: (() => void) | null = null;
export function startThemeEffect(): () => void {
  if (effectDisposer) return effectDisposer;
  effectDisposer = effect(() => {
    // Read all three so the effect re-runs on any change.
    void currentTheme.value;
    void currentAccent.value;
    void fontSizePx.value;
    applyTheme();
  });
  return effectDisposer;
}
