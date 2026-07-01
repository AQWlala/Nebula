/**
 * v1.0.1: Settings panel — signal-driven.
 *
 * P0#06: theme / accent / font-size now flow through
 * `src/theme/index.ts` (a signal store) instead of an in-component
 * useEffect.  This means:
 *  - The DOM application is owned by `applyTheme()` and runs once
 *    per change, not on every component re-render.
 *  - The `system` theme mode listens to `prefers-color-scheme` from
 *    a single place.
 *  - Other components can read `currentTheme.value` etc. and
 *    re-render automatically.
 *
 * The component still maintains local state for non-theme fields
 * (locale, autosave, ollamaUrl, apiKey, workspace) and the in-flight
 * form values.  The old `nine-snake.settings` localStorage key is
 * preserved for backward compatibility.
 */
import { useEffect, useState } from 'preact/hooks';
import { t, LOCALES, type Locale, getLocale, setLocale } from '../i18n';
import { NineSnakeAPI } from '../lib/tauri';
import {
  currentTheme,
  currentAccent,
  fontSizePx,
  setTheme as setThemeSignal,
  setAccent as setAccentSignal,
  setFontSize as setFontSizeSignal,
  persistTheme,
  applyTheme,
  FONT_MIN,
  FONT_MAX,
  FONT_DEFAULT,
  ACCENT_DEFAULT,
  THEME_DEFAULT,
  ACCENT_OPTIONS,
  clampFontSize,
  resolveAccentCssVar,
  type Theme,
  type Accent,
} from '../theme';
// v1.0.1 P0#12: bridge to the three Tauri commands that
// read/write the OS keychain.  We import them dynamically so
// the component still works in Storybook / unit-test contexts
// where the Tauri runtime isn't available.
import { invokeTauri } from '../lib/tauri';

type SettingsAccent = Accent;

interface AppSettings {
  theme: Theme;
  accent: SettingsAccent;
  fontSize: number;
  autosaveSec: number;
  ollamaUrl: string;
  /** v1.0.1 P0#12: the API key is *never* stored in
   * `localStorage` anymore.  The form field is bound to local
   * state and, on save, is shipped to the OS keychain via the
   * `set_api_key` Tauri command.  The `apiKey` field is left
   * in the persisted settings as the empty string for
   * backward-compat with any old `localStorage` data the user
   * has — the new code path never reads or writes it. */
  apiKey: string;
  workspace: string;
  locale: Locale;
}

const DEFAULTS: AppSettings = {
  theme: THEME_DEFAULT,
  accent: ACCENT_DEFAULT,
  fontSize: FONT_DEFAULT,
  autosaveSec: 5,
  ollamaUrl: 'http://127.0.0.1:11434',
  // v1.0.1 P0#12: never seed the form with a real key from
  // `localStorage`; the field is blank by default and the
  // front-end asks the back-end for the configured value via
  // `get_api_key` on first open.
  apiKey: '',
  workspace: '.',
  locale: getLocale(),
};

const STORAGE_KEY = 'nine-snake.settings';

const FONT_STEP = 1;

function loadSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw);
    return { ...DEFAULTS, ...parsed };
  } catch {
    return { ...DEFAULTS };
  }
}

export function Settings({ onClose }: { onClose: () => void }) {
  const [s, setS] = useState<AppSettings>(() => {
    // Seed the local form from the signal store if the user has
    // already saved a theme previously — this keeps the form in
    // sync with whatever `applyTheme()` is currently displaying.
    const merged = loadSettings();
    return {
      ...merged,
      theme: currentTheme.value,
      accent: currentAccent.value,
      fontSize: fontSizePx.value,
      // v1.0.1 P0#12: the form is *always* blank on open,
      // regardless of what the persisted `localStorage` blob
      // may have contained in v1.0.  The real key lives in
      // the OS keychain, queried separately below.
      apiKey: '',
    };
  });
  const [saved, setSaved] = useState(false);
  // v1.0.1 P0#12: separate state for the "keychain already has
  // a key" indicator.  We use a separate piece of state (not
  // a field on `AppSettings`) so it's never persisted to
  // `localStorage`.
  const [keyConfigured, setKeyConfigured] = useState(false);
  // v1.7: 开机自启动开关状态。
  const [autostartEnabled, setAutostartEnabled] = useState(false);

  useEffect(() => {
    // P0#4: apply font size + accent to CSS variables on mount
    // and whenever the user changes them.  The global stylesheet
    // consumes both, so the change is visible everywhere.
    document.documentElement.style.setProperty('--font-size', `${clampFontSize(s.fontSize)}px`);
    document.documentElement.style.setProperty('--accent', resolveAccentCssVar(s.accent));
  }, [s.accent, s.fontSize]);

  // v1.0.1 P0#12: on first mount, ask the back-end whether
  // the user has already configured an API key.  We never
  // persist the value to `localStorage`; the field is left
  // blank with a small "configured" indicator so the user
  // knows the keychain already holds a key but doesn't need
  // to see it.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const v = await invokeTauri<string | null>('get_api_key');
        if (cancelled) return;
        // We don't put the secret into local state — only the
        // "configured" flag.  A blank form value still means
        // "user has not entered one" in the form.
        if (v && v.length > 0) {
          setKeyConfigured(true);
        }
      } catch {
        // Tauri runtime not available (e.g. browser preview);
        // the field stays empty.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // v1.7: 查询当前开机自启动状态。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const enabled = await invokeTauri<boolean>('os_autostart_is_enabled');
        if (cancelled) return;
        setAutostartEnabled(enabled === true);
      } catch {
        // Tauri runtime not available; keep default false.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function toggleAutostart(next: boolean) {
    setAutostartEnabled(next);
    try {
      await invokeTauri(next ? 'os_autostart_enable' : 'os_autostart_disable');
    } catch (e) {
      // 回滚状态并提示。
      setAutostartEnabled(!next);
      console.error('autostart toggle failed', e);
    }
  }

  function update<K extends keyof AppSettings>(k: K, v: AppSettings[K]) {
    setS((prev) => ({ ...prev, [k]: v }));
  }

  async function save() {
    try {
      const normalized: AppSettings = { ...s, fontSize: clampFontSize(s.fontSize) };
      // v1.0.1 P0#12: strip the apiKey from the persisted
      // blob.  The value is shipped to the OS keychain
      // through the Tauri command instead.
      const { apiKey, ...persistable } = normalized;
      localStorage.setItem(STORAGE_KEY, JSON.stringify(persistable));
      setLocale(normalized.locale);
      // P0#06: push the new theme values into the signal store
      // and re-apply them to the document.  `persistTheme()`
      // mirrors them under `nine-snake.theme` for the boot path.
      setThemeSignal(normalized.theme);
      setAccentSignal(normalized.accent);
      setFontSizeSignal(normalized.fontSize);
      persistTheme();
      applyTheme();
      setS(normalized);
      // v1.0.1 P0#12: forward the key to the back-end.  An
      // empty value is treated by the back-end as a delete,
      // which is what the user expects when they clear the
      // field.
      try {
        await invokeTauri('set_api_key', { value: apiKey });
        // Refresh the indicator after a save.
        if (apiKey.trim().length > 0) {
          setKeyConfigured(true);
        } else {
          setKeyConfigured(false);
        }
      } catch (e) {
        console.error('failed to persist api key to keychain', e);
      }
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (e) {
      console.error('failed to save settings', e);
    }
  }

  return (
    <div class="settings-modal" role="dialog" aria-labelledby="settings-title">
      <div class="settings-card">
        <header class="settings-header">
          <h2 id="settings-title">{t('settings.title')}</h2>
          <button class="icon-btn" onClick={onClose} aria-label={t('settings.close')}>×</button>
        </header>
        <div class="settings-body">
          <label class="row">
            <span>{t('settings.language')}</span>
            <select
              value={s.locale}
              onChange={(e) => update('locale', (e.currentTarget.value as Locale))}
            >
              {LOCALES.map((l) => (
                <option key={l} value={l}>{l}</option>
              ))}
            </select>
          </label>
          <label class="row">
            <span>{t('settings.theme')}</span>
            <select
              value={s.theme}
              onChange={(e) => update('theme', e.currentTarget.value as Theme)}
            >
              <option value="dark">{t('settings.theme.dark')}</option>
              <option value="light">{t('settings.theme.light')}</option>
              <option value="system">{t('settings.theme.system')}</option>
            </select>
          </label>
          <div class="row">
            <span>{t('settings.accent')}</span>
            <div class="accent-picker" role="radiogroup" aria-label={t('settings.accent')}>
              {ACCENT_OPTIONS.map((o) => (
                <button
                  key={o.value}
                  type="button"
                  role="radio"
                  aria-checked={s.accent === o.value}
                  class={`accent-swatch ${s.accent === o.value ? 'active' : ''}`}
                  data-accent-value={o.value}
                  data-accent-css={o.cssVar}
                  style={`--swatch: ${o.preview}`}
                  onClick={() => update('accent', o.value)}
                >
                  <span class="swatch-dot" />
                  {o.value === 'purple' ? 'Deep purple' : o.value === 'neon' ? 'Neon green' : 'Amber gold'}
                </button>
              ))}
            </div>
          </div>
          <label class="row">
            <span>{t('settings.fontSize')}</span>
            <input
              type="number"
              min={FONT_MIN}
              max={FONT_MAX}
              step={FONT_STEP}
              value={s.fontSize}
              onInput={(e) => update('fontSize', Number(e.currentTarget.value) || FONT_DEFAULT)}
            />
          </label>
          <label class="row">
            <span>{t('settings.autosave')}</span>
            <input
              type="number"
              min="1"
              max="60"
              value={s.autosaveSec}
              onInput={(e) => update('autosaveSec', Number(e.currentTarget.value) || 5)}
            />
          </label>
          <label class="row">
            <span>{t('settings.ollamaUrl')}</span>
            <input
              type="url"
              value={s.ollamaUrl}
              onInput={(e) => update('ollamaUrl', e.currentTarget.value)}
            />
          </label>
          <label class="row">
            <span>
              {t('settings.apiKey')}
              {/* v1.0.1 P0#12: a small badge tells the user a
                  key is already in the OS keychain without
                  showing the value itself.  This is purely
                  UX; the form value stays empty. */}
              {keyConfigured && (
                <span class="api-key-configured" data-testid="api-key-configured">
                  {' '}✓ {t('settings.apiKeyConfigured')}
                </span>
              )}
            </span>
            <input
              type="password"
              value={s.apiKey}
              onInput={(e) => update('apiKey', e.currentTarget.value)}
              autocomplete="off"
              spellcheck={false}
              placeholder={keyConfigured ? '••••••••' : ''}
            />
          </label>
          <label class="row">
            <span>{t('settings.workspace')}</span>
            <input
              type="text"
              value={s.workspace}
              onInput={(e) => update('workspace', e.currentTarget.value)}
            />
          </label>
        </div>

        {/* v1.3: Device Management */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">{t('settings.devices') || '已配对设备'}</h3>
          <div id="device-list" style="color: var(--text-secondary); font-size: 13px;">
            {t('settings.devicesHint') || '设备管理需通过同步功能配置'}
          </div>
        </div>

        {/* v1.7: OS 集成（开机自启动） */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">{t('settings.os') || '系统集成'}</h3>
          <label class="row" style="display: flex; align-items: center; justify-content: space-between;">
            <span>{t('settings.autostart') || '开机自启动'}</span>
            <input
              type="checkbox"
              checked={autostartEnabled}
              onChange={(e) => toggleAutostart(e.currentTarget.checked)}
              style={{ width: 'auto' }}
            />
          </label>
        </div>

        {/* v1.3: DID Identity */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">{t('settings.identity') || 'DID 身份'}</h3>
          <button
            class="btn"
            onClick={async () => {
              try {
                const result = await NineSnakeAPI.generateDid();
                alert(`DID: ${result.did}`);
              } catch { /* noop */ }
            }}
          >
            {t('settings.generateDid') || '生成 DID'}
          </button>
        </div>

        <footer class="settings-footer">
          <span class="settings-status">{saved ? t('settings.saved') : ''}</span>
          <button class="primary" onClick={save}>{t('settings.save')}</button>
        </footer>
      </div>
    </div>
  );
}

// P0#4: small helper exports used by the test suite.  Keep them
// stable; they describe the contract between Settings.tsx and
// the global stylesheet.
export const __test__ = {
  ACCENT_OPTIONS,
  FONT_MIN,
  FONT_MAX,
  FONT_STEP,
  resolveAccentCssVar,
  clampFontSize,
};
