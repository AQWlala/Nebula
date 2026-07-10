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
 * form values.  The old `nebula.settings` localStorage key is
 * preserved for backward compatibility.
 */
import { useEffect, useState } from 'preact/hooks';
import { t, LOCALES, type Locale, getLocale, setLocale } from '../i18n';
import {
  nebulaAPI,
  type DeviceInfo,
  type ModelsConfig,
  type PersonaConfig,
  type ProviderConfig,
} from '../lib/tauri';
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
import { nebulaStore } from '../stores/nebulaStore';
// T-E-C-17: IM 绑定面板(Feishu/WeCom/DingTalk webhook)。
import { ImBindingPanel } from './ImBindingPanel';
import { toast } from './Toast';
import { Spinner } from './Spinner';
import { SoulEditor } from './SoulEditor';
import { EvolutionLogView } from './EvolutionLogView';
import { WorkTypeConfigView } from './WorkTypeConfigView';
// P0-1: 模型配置中心(provider 列表 + 配置表单 + WorkType 路由 + 模型健康面板)。
import { ModelConfigPanel } from './ModelConfigPanel';
// T-E-B-09: 文件夹选择对话框(tauri-plugin-dialog)。
import { open as openDialog } from '@tauri-apps/plugin-dialog';

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
  /** T-E-A-07: 月预算(USD),0 = 不限制。 */
  monthlyBudgetUsd: number;
  /** T-E-A-05: 日预算(USD),0 = 不限制。超限自动降级到 Ollama。 */
  dailyBudgetUsd: number;
  /** T-E-S-40: 主 LLM provider(deepseek/ollama/openai-compat/anthropic)。 */
  llmProvider: string;
  /** T-E-S-40: OpenAI 兼容 base URL。 */
  openaiCompatUrl: string;
  /** T-E-S-40: OpenAI 兼容默认模型名。 */
  openaiCompatModel: string;
  /** T-E-S-40: OpenAI 兼容 API key(同 apiKey,从不持久化到 localStorage)。 */
  openaiCompatKey: string;
  /** T-E-B-09: 文件夹监控路径列表(权威源在后端 settings.json)。 */
  watchPaths: string[];
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
  monthlyBudgetUsd: 0,
  dailyBudgetUsd: 0,
  // T-E-S-40: 默认 deepseek(对齐后端 AppConfig 默认值)。
  llmProvider: 'deepseek',
  openaiCompatUrl: '',
  openaiCompatModel: '',
  openaiCompatKey: '',
  // T-E-B-09: 默认无监控目录。
  watchPaths: [],
};

const STORAGE_KEY = 'nebula.settings';

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
      // T-E-S-40: openaiCompatKey 同样从不在前端持久化,表单永远空白。
      openaiCompatKey: '',
    };
  });
  const [saved, setSaved] = useState(false);
  // v1.0.1 P0#12: separate state for the "keychain already has
  // a key" indicator.  We use a separate piece of state (not
  // a field on `AppSettings`) so it's never persisted to
  // `localStorage`.
  const [keyConfigured, setKeyConfigured] = useState(false);
  // T-E-S-40: OpenAI 兼容 provider key 是否已配置(掩码查询,从不持有明文)。
  const [openaiCompatKeyConfigured, setOpenaiCompatKeyConfigured] = useState(false);
  // v1.7: 开机自启动开关状态。
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  // T-S5-A-01: 已配对设备列表 + 加载/撤销状态。
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [deviceLoading, setDeviceLoading] = useState(false);
  const [deviceRevoking, setDeviceRevoking] = useState<string | null>(null);
  // T-S5-A-03: AI 自动模式开关 + 误分类计数(从 store 信号同步)。
  const [aiAutoMode, setAiAutoMode] = useState(nebulaStore.aiAutoMode.value);
  const [modeMisclassification] = useState(nebulaStore.modeMisclassification.value);
  // T-E-B-09: 文件夹监控当前是否在运行(由后端 watch_status 查询)。
  const [watchActive, setWatchActive] = useState(false);
  // T-E-S-41: models.json 动态配置(provider 列表 + 默认值)。
  const [modelsConfig, setModelsConfig] = useState<ModelsConfig | null>(null);
  // T-E-S-41: 添加新 provider 的 JSON 文本框内容。
  const [newProviderJson, setNewProviderJson] = useState('');
  // T-E-S-41: 添加 provider 时的错误提示(校验失败 / id 冲突等)。
  const [providerError, setProviderError] = useState('');
  // T-E-S-41: 各 provider 的 API key 输入框值(按 provider id 索引,从不持久化)。
  const [providerKeys, setProviderKeys] = useState<Record<string, string>>({});
  // T-E-S-41: 各 provider 的 API key 是否已配置(掩码查询结果)。
  const [providerKeyConfigured, setProviderKeyConfigured] = useState<Record<string, boolean>>({});
  // T-E-S-39: SOUL.md/AGENTS.md/TOOLS.md persona 配置快照。
  const [persona, setPersona] = useState<PersonaConfig | null>(null);
  const [personaReloading, setPersonaReloading] = useState(false);
  // M6 #77: Soul 编辑器 Modal 开关。
  const [soulEditorOpen, setSoulEditorOpen] = useState(false);
  // M6 #78: 进化日志 Modal 开关。
  const [evolutionLogOpen, setEvolutionLogOpen] = useState(false);
  // M6 #83: WorkType 配置 Modal 开关。
  const [workTypeConfigOpen, setWorkTypeConfigOpen] = useState(false);
  // P0-1: 模型配置中心 Modal 开关。
  const [modelConfigOpen, setModelConfigOpen] = useState(false);

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
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const v = await invokeTauri<string | null>('get_api_key');
        if (ac.signal.aborted) return;
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
      ac.abort();
    };
  }, []);

  // T-E-S-40: 查询 openai-compat provider key 是否已配置(掩码查询)。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const v = await nebulaAPI.getProviderApiKey('openai-compat');
        if (ac.signal.aborted) return;
        if (v && v.length > 0) {
          setOpenaiCompatKeyConfigured(true);
        }
      } catch {
        // Tauri runtime not available; keep default false.
      }
    })();
    return () => {
      ac.abort();
    };
  }, []);

  // T-E-S-40: 从后端 settings.json 加载 llm_provider / openai_compat_url /
  // openai_compat_model(后端为权威源,localStorage 仅作离线回退)。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const dto = await invokeTauri<Record<string, unknown>>('load_app_settings');
        if (ac.signal.aborted || !dto) return;
        setS((prev) => ({
          ...prev,
          llmProvider: typeof dto.llm_provider === 'string' ? dto.llm_provider : prev.llmProvider,
          openaiCompatUrl:
            typeof dto.openai_compat_url === 'string'
              ? dto.openai_compat_url
              : prev.openaiCompatUrl,
          openaiCompatModel:
            typeof dto.openai_compat_model === 'string'
              ? dto.openai_compat_model
              : prev.openaiCompatModel,
          dailyBudgetUsd:
            typeof dto.daily_budget_usd === 'number' ? dto.daily_budget_usd : prev.dailyBudgetUsd,
          // T-E-B-09: 加载文件夹监控路径(后端 settings.json 为权威源)。
          watchPaths: Array.isArray(dto.watch_paths)
            ? (dto.watch_paths as string[]).filter((p) => typeof p === 'string')
            : prev.watchPaths,
        }));
      } catch {
        // Tauri runtime not available; keep localStorage values.
      }
    })();
    return () => {
      ac.abort();
    };
  }, []);

  // v1.7: 查询当前开机自启动状态。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const enabled = await invokeTauri<boolean>('os_autostart_is_enabled');
        if (ac.signal.aborted) return;
        setAutostartEnabled(enabled === true);
      } catch {
        // Tauri runtime not available; keep default false.
      }
    })();
    return () => {
      ac.abort();
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

  // T-S5-A-01: 加载已配对设备列表。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      setDeviceLoading(true);
      try {
        const list = await nebulaAPI.deviceList();
        if (ac.signal.aborted) return;
        setDevices(list);
      } catch {
        // Tauri runtime not available or device manager not ready;
        // keep empty list.
      } finally {
        if (!ac.signal.aborted) setDeviceLoading(false);
      }
    })();
    return () => {
      ac.abort();
    };
  }, []);

  // T-S5-A-01: 撤销设备配对。
  async function revokeDevice(deviceId: string) {
    setDeviceRevoking(deviceId);
    try {
      const ok = await nebulaAPI.deviceRevoke(deviceId);
      if (ok) {
        // 刷新列表以反映 revoked 状态。
        const list = await nebulaAPI.deviceList();
        setDevices(list);
      }
    } catch (e) {
      console.error('revoke device failed', e);
    } finally {
      setDeviceRevoking(null);
    }
  }

  // T-S5-A-03: 切换 AI 自动模式(LLM 路由 vs 关键词启发式)。
  function toggleAiAutoMode(next: boolean) {
    setAiAutoMode(next);
    nebulaStore.aiAutoMode.value = next;
  }

  // T-E-B-09: 查询文件夹监控运行状态(挂载时 + watchPaths 变化后)。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const status = await nebulaAPI.watchStatus();
        if (ac.signal.aborted) return;
        setWatchActive(status?.active === true);
      } catch {
        // Tauri runtime not available; keep default false.
      }
    })();
    return () => {
      ac.abort();
    };
  }, [s.watchPaths]);

  // T-E-B-09: 添加监控文件夹(原生目录选择对话框)。
  async function addWatchFolder() {
    try {
      const selected = await openDialog({ directory: true, multiple: true });
      if (!selected) return;
      const picked = Array.isArray(selected) ? selected : [selected];
      if (picked.length === 0) return;
      setS((prev) => {
        const existing = new Set(prev.watchPaths);
        const merged = [...prev.watchPaths];
        for (const p of picked) {
          if (!existing.has(p)) {
            merged.push(p);
            existing.add(p);
          }
        }
        return { ...prev, watchPaths: merged };
      });
    } catch (e) {
      console.error('addWatchFolder dialog failed', e);
    }
  }

  // T-E-B-09: 移除单个监控文件夹。
  function removeWatchFolder(path: string) {
    setS((prev) => ({
      ...prev,
      watchPaths: prev.watchPaths.filter((p) => p !== path),
    }));
  }

  // T-E-B-09: 启用/停用文件夹监控(立即生效,不等保存)。
  async function toggleWatch(next: boolean) {
    try {
      if (next) {
        if (s.watchPaths.length === 0) return;
        await nebulaAPI.watchStart(s.watchPaths);
        setWatchActive(true);
      } else {
        await nebulaAPI.watchStop();
        setWatchActive(false);
      }
    } catch (e) {
      console.error('toggleWatch failed', e);
    }
  }

  // T-E-S-41: 加载 models.json(provider 列表 + 默认值)。挂载时执行一次。
  // 同时为每个非内置 provider 查询 keychain 是否已有 key(掩码查询)。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const cfg = await nebulaAPI.modelsConfigLoad();
        if (ac.signal.aborted) return;
        setModelsConfig(cfg);
        // 并行查询所有自定义 provider 的 keychain 状态。
        const entries = await Promise.all(
          cfg.providers
            .filter((p) => !p.is_builtin)
            .map(async (p) => {
              try {
                const masked = await nebulaAPI.getProviderKey(p.id);
                return [p.id, masked !== null] as [string, boolean];
              } catch {
                return [p.id, false] as [string, boolean];
              }
            })
        );
        if (ac.signal.aborted) return;
        setProviderKeyConfigured(Object.fromEntries(entries));
      } catch {
        // Tauri runtime not available; keep null.
      }
    })();
    return () => {
      ac.abort();
    };
  }, []);

  // T-E-S-41: 设默认 provider + model(用该 provider 的第一个 model 作为默认 model)。
  async function setDefaultProvider(providerId: string) {
    if (!modelsConfig) return;
    const provider = modelsConfig.providers.find((p) => p.id === providerId);
    if (!provider || provider.models.length === 0) {
      setProviderError(t('settings.providers.errNoModels') );
      return;
    }
    try {
      const updated = await nebulaAPI.modelsConfigSetDefault(providerId, provider.models[0].id);
      setModelsConfig(updated);
      setProviderError('');
    } catch (e) {
      setProviderError(String(e));
    }
  }

  // T-E-S-39: 加载 persona 快照(SOUL.md/AGENTS.md/TOOLS.md 状态)。
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();
    (async () => {
      try {
        const pc = await nebulaAPI.personaGet();
        if (!ac.signal.aborted) setPersona(pc);
      } catch {
        // Tauri runtime not available; keep null.
      }
    })();
    return () => {
      ac.abort();
    };
  }, []);

  // T-E-S-39: 重新加载 persona(从工作区根目录读盘)。
  async function reloadPersona() {
    setPersonaReloading(true);
    try {
      const pc = await nebulaAPI.personaReload();
      setPersona(pc);
    } catch {
      // ignore; keep stale state
    } finally {
      setPersonaReloading(false);
    }
  }

  // T-E-S-39 + M6 #77: 编辑 persona 文件。SOUL.md 走 SoulEditor 双分区可视化,
  // AGENTS.md / TOOLS.md 走 editor_read/write 简单编辑。
  async function editPersonaFile(filename: string) {
    if (filename === 'SOUL.md') {
      setSoulEditorOpen(true);
      return;
    }
    // 其他文件:读取 → 创建空文件(若不存在)→ 留给 Monaco 编辑器处理
    try {
      await nebulaAPI.editorRead(filename);
    } catch {
      try {
        await nebulaAPI.editorWrite(filename, '');
      } catch {
        // ignore
      }
    }
  }

  // T-E-S-41: 删除 provider(内置不可删,默认不可删)。
  async function removeProvider(providerId: string) {
    try {
      const updated = await nebulaAPI.modelsConfigRemoveProvider(providerId);
      setModelsConfig(updated);
      setProviderError('');
    } catch (e) {
      setProviderError(String(e));
    }
  }

  // T-E-S-41: 添加 provider(JSON 文本框解析 + 校验 + 落盘)。
  async function addProviderFromJson() {
    setProviderError('');
    let parsed: ProviderConfig;
    try {
      parsed = JSON.parse(newProviderJson) as ProviderConfig;
    } catch (e) {
      setProviderError(t('settings.providers.errJson')  + String(e));
      return;
    }
    // 基本字段完整性检查(与后端 ProviderConfig struct 对齐)。
    if (!parsed.id || !parsed.display_name || !parsed.kind || !Array.isArray(parsed.models)) {
      setProviderError(
        t('settings.providers.errFields') 
      );
      return;
    }
    try {
      const updated = await nebulaAPI.modelsConfigAddProvider(parsed);
      setModelsConfig(updated);
      setNewProviderJson('');
    } catch (e) {
      setProviderError(String(e));
    }
  }

  // T-E-S-41: 写入自定义 provider 的 API key 到 keychain(空串=删除)。
  async function saveProviderKey(providerId: string) {
    const value = providerKeys[providerId] ?? '';
    try {
      await nebulaAPI.setProviderKey(providerId, value);
      setProviderKeyConfigured((prev) => ({
        ...prev,
        [providerId]: value.trim().length > 0,
      }));
      setProviderKeys((prev) => {
        const next = { ...prev };
        delete next[providerId];
        return next;
      });
    } catch (e) {
      setProviderError(String(e));
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
      // T-E-S-40: 同样剥离 openaiCompatKey(走多 provider keychain 命令)。
      const { apiKey, openaiCompatKey, ...persistable } = normalized;
      localStorage.setItem(STORAGE_KEY, JSON.stringify(persistable));
      setLocale(normalized.locale);
      // P0#06: push the new theme values into the signal store
      // and re-apply them to the document.  `persistTheme()`
      // mirrors them under `nebula.theme` for the boot path.
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
      // T-E-S-40: 写入 openai-compat provider key 到 keychain(空串=删除)。
      try {
        await nebulaAPI.setProviderApiKey('openai-compat', openaiCompatKey);
        if (openaiCompatKey.trim().length > 0) {
          setOpenaiCompatKeyConfigured(true);
        } else {
          setOpenaiCompatKeyConfigured(false);
        }
      } catch (e) {
        console.error('failed to persist openai-compat api key to keychain', e);
      }
      // T-E-A-05: 持久化日预算到 settings.json(后端可读,热更新 LlmGateway)。
      // 先 load 已有 settings 再合并,避免覆盖其他字段。
      // T-E-S-40: 同时持久化 llm_provider / openai_compat_url / openai_compat_model。
      try {
        const existing = await invokeTauri<Record<string, unknown>>('load_app_settings');
        const dto = {
          ...(existing ?? {}),
          daily_budget_usd: normalized.dailyBudgetUsd,
          llm_provider: normalized.llmProvider,
          openai_compat_url: normalized.openaiCompatUrl,
          openai_compat_model: normalized.openaiCompatModel,
          // T-E-B-09: 持久化监控路径(后端 save_app_settings 会热更新 watcher)。
          watch_paths: normalized.watchPaths,
        };
        await invokeTauri('save_app_settings', { settings: dto });
      } catch (e) {
        console.error('failed to persist daily budget to settings.json', e);
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
          <button class="icon-btn" onClick={onClose} aria-label={t('settings.close')}>
            ×
          </button>
        </header>
        <div class="settings-body">
          <label class="row">
            <span>{t('settings.language')}</span>
            <select
              value={s.locale}
              onChange={(e) => update('locale', e.currentTarget.value as Locale)}
            >
              {LOCALES.map((l) => (
                <option key={l} value={l}>
                  {l}
                </option>
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
                  {o.value === 'purple'
                    ? 'Deep purple'
                    : o.value === 'neon'
                      ? 'Neon green'
                      : 'Amber gold'}
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
                  {' '}
                  ✓ {t('settings.apiKeyConfigured')}
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
          <label class="row">
            <span>月预算 (USD, 0=不限)</span>
            <input
              type="number"
              min="0"
              step="0.50"
              value={s.monthlyBudgetUsd}
              onInput={(e) => update('monthlyBudgetUsd', parseFloat(e.currentTarget.value) || 0)}
            />
          </label>
          <label class="row">
            <span>日预算 (USD, 0=不限, 超限降级 Ollama)</span>
            <input
              type="number"
              min="0"
              step="0.10"
              value={s.dailyBudgetUsd}
              onInput={(e) => update('dailyBudgetUsd', parseFloat(e.currentTarget.value) || 0)}
            />
          </label>
          <label class="row">
            <span>{t('settings.provider') }</span>
            <select
              value={s.llmProvider}
              onChange={(e) => update('llmProvider', e.currentTarget.value)}
            >
              <option value="deepseek">{t('settings.provider.deepseek') || 'DeepSeek'}</option>
              <option value="ollama">{t('settings.provider.ollama') }</option>
              <option value="openai-compat">
                {t('settings.provider.openai-compat') }
              </option>
              <option value="anthropic">
                {t('settings.provider.anthropic') || 'Anthropic Claude'}
              </option>
            </select>
          </label>
        </div>

        {/* T-E-S-40: OpenAI 兼容配置区 — 仅当 provider=openai-compat 时显示 */}
        {s.llmProvider === 'openai-compat' && (
          <div class="card" style="margin-top: 16px;">
            <h3 style="margin-bottom: 8px;">{t('settings.openaiCompat') }</h3>
            <label class="row">
              <span>{t('settings.openaiCompatUrl') || 'Base URL'}</span>
              <input
                type="url"
                value={s.openaiCompatUrl}
                onInput={(e) => update('openaiCompatUrl', e.currentTarget.value)}
                placeholder="https://openrouter.ai/api/v1"
              />
            </label>
            <div style="color: var(--text-secondary); font-size: 11px; margin: -4px 0 8px 0;">
              {t('settings.openaiCompatUrlHint') ||
                '如 https://openrouter.ai/api/v1 或 http://localhost:1234/v1'}
            </div>
            <label class="row">
              <span>{t('settings.openaiCompatModel') }</span>
              <input
                type="text"
                value={s.openaiCompatModel}
                onInput={(e) => update('openaiCompatModel', e.currentTarget.value)}
                placeholder="openai/gpt-4o-mini"
              />
            </label>
            <div style="color: var(--text-secondary); font-size: 11px; margin: -4px 0 8px 0;">
              {t('settings.openaiCompatModelHint') ||
                '如 openai/gpt-4o-mini / llama-3.1-8b-instruct'}
            </div>
            <label class="row">
              <span>
                {t('settings.openaiCompatKey') || 'API Key'}
                {openaiCompatKeyConfigured && (
                  <span class="api-key-configured" data-testid="openai-compat-key-configured">
                    {' '}
                    ✓ {t('settings.openaiCompatKeyConfigured') }
                  </span>
                )}
              </span>
              <input
                type="password"
                value={s.openaiCompatKey}
                onInput={(e) => update('openaiCompatKey', e.currentTarget.value)}
                autocomplete="off"
                spellcheck={false}
                placeholder={openaiCompatKeyConfigured ? '••••••••' : ''}
              />
            </label>
          </div>
        )}

        {/* T-S5-A-01: Device Management — 已配对设备列表 + 撤销按钮 */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">{t('settings.devices') }</h3>
          {deviceLoading ? (
            <Spinner label={t('common.loading')} />
          ) : devices.length === 0 ? (
            <div id="device-list" style="color: var(--text-secondary); font-size: 13px;">
              {t('settings.devicesHint') }
            </div>
          ) : (
            <div id="device-list" style="display: flex; flex-direction: column; gap: 8px;">
              {devices.map((d) => (
                <div
                  key={d.device_id}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '8px 12px',
                    borderRadius: '6px',
                    border: '1px solid var(--border)',
                    opacity: d.revoked ? 0.5 : 1,
                  }}
                >
                  <div style="flex: 1; min-width: 0;">
                    <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                      <span style={{ fontWeight: 500, fontSize: '13px' }}>
                        {d.device_id.slice(0, 12)}…
                      </span>
                      {d.revoked && (
                        <span
                          style={{
                            fontSize: '11px',
                            padding: '2px 6px',
                            borderRadius: '4px',
                            background: 'var(--bg-secondary)',
                            color: 'var(--text-secondary)',
                          }}
                        >
                          已撤销
                        </span>
                      )}
                    </div>
                    <div style="color: var(--text-secondary); font-size: 11px; margin-top: 2px;">
                      配对于 {new Date(d.paired_at * 1000).toLocaleString()}
                    </div>
                  </div>
                  {!d.revoked && (
                    <button
                      type="button"
                      disabled={deviceRevoking === d.device_id}
                      onClick={() => revokeDevice(d.device_id)}
                      style={{
                        fontSize: '12px',
                        padding: '4px 12px',
                        borderRadius: '4px',
                        border: '1px solid var(--border)',
                        background: 'transparent',
                        color: 'var(--text-primary)',
                        cursor: deviceRevoking === d.device_id ? 'not-allowed' : 'pointer',
                        opacity: deviceRevoking === d.device_id ? 0.5 : 1,
                      }}
                    >
                      {deviceRevoking === d.device_id ? '撤销中…' : '撤销'}
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>

        {/* v1.7: OS 集成（开机自启动） */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">{t('settings.os') }</h3>
          <label
            class="row"
            style="display: flex; align-items: center; justify-content: space-between;"
          >
            <span>{t('settings.autostart') }</span>
            <input
              type="checkbox"
              checked={autostartEnabled}
              onChange={(e) => toggleAutostart(e.currentTarget.checked)}
              style={{ width: 'auto' }}
            />
          </label>
        </div>

        {/* T-S5-A-03: AI 自动模式 — 启用 LLM 路由 vs 关键词启发式 */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">AI 自动模式</h3>
          <label
            class="row"
            style="display: flex; align-items: center; justify-content: space-between;"
          >
            <span>LLM 智能路由（关闭后退化为关键词启发式）</span>
            <input
              type="checkbox"
              checked={aiAutoMode}
              onChange={(e) => toggleAiAutoMode(e.currentTarget.checked)}
              style={{ width: 'auto' }}
            />
          </label>
          {modeMisclassification > 0 && (
            <div style="color: var(--text-secondary); font-size: 11px; margin-top: 4px;">
              误分类计数: {modeMisclassification}
            </div>
          )}
        </div>

        {/* v1.3: DID Identity */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 8px;">{t('settings.identity') }</h3>
          <button
            class="btn"
            onClick={async () => {
              try {
                const result = await nebulaAPI.generateDid();
                // 复制到剪贴板而非阻塞 alert,用户可粘贴
                try {
                  await navigator.clipboard.writeText(result.did);
                  toast.success('DID 已生成并复制到剪贴板');
                } catch {
                  // 剪贴板不可用时回退到 toast 显示完整 DID
                  toast.success('DID 已生成', result.did);
                }
              } catch {
                /* noop */
              }
            }}
          >
            {t('settings.generateDid') }
          </button>
        </div>

        {/* T-E-S-41: LLM 提供商 — models.json 动态配置(provider 列表 + 添加 + 删除 + 设默认)。 */}
        <div class="card" style="margin-top: 16px;">
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              marginBottom: '4px',
            }}
          >
            <h3 style="margin: 0;">{t('settings.providers.title') }</h3>
            <div style={{ display: 'flex', gap: '8px' }}>
              <button
                type="button"
                onClick={async () => {
                  try {
                    const cfg = await nebulaAPI.modelsConfigReload();
                    setModelsConfig(cfg);
                  } catch (e) {
                    console.error('reload models.json failed', e);
                  }
                }}
                title={
                  t('settings.providers.reloadTitle') ||
                  '从磁盘重新加载 models.json(手动编辑文件后使用)'
                }
                style={{
                  fontSize: '12px',
                  padding: '4px 12px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'transparent',
                  color: 'var(--text-primary)',
                  cursor: 'pointer',
                }}
              >
                {t('settings.providers.reload') }
              </button>
              <button
                type="button"
                onClick={() => setWorkTypeConfigOpen(true)}
                title={t('workTypeConfig.openButtonTitle') }
                style={{
                  fontSize: '12px',
                  padding: '4px 12px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'transparent',
                  color: 'var(--text-primary)',
                  cursor: 'pointer',
                }}
              >
                {t('workTypeConfig.openButton') }
              </button>
              <button
                type="button"
                onClick={() => setModelConfigOpen(true)}
                title="打开模型配置中心(provider 管理 / API Key / 连通性测试 / 模型发现 / WorkType 路由)"
                style={{
                  fontSize: '12px',
                  padding: '4px 12px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'transparent',
                  color: 'var(--text-primary)',
                  cursor: 'pointer',
                }}
              >
                模型配置中心
              </button>
            </div>
          </div>
          <div style="color: var(--text-secondary); font-size: 11px; margin-bottom: 8px;">
            {t('settings.providers.hint') ||
              '管理 LLM provider 列表;新增/删除重启生效,默认值可热更新'}
          </div>
          {modelsConfig === null ? (
            <Spinner label={t('common.loading')} />
          ) : (
            <div style="display: flex; flex-direction: column; gap: 8px;">
              {modelsConfig.providers.map((p) => {
                const isDefault = modelsConfig.default_provider === p.id;
                return (
                  <div
                    key={p.id}
                    style={{
                      padding: '8px 12px',
                      borderRadius: '6px',
                      border: `1px solid ${isDefault ? 'var(--accent)' : 'var(--border)'}`,
                      background: isDefault ? 'var(--bg-secondary)' : 'transparent',
                    }}
                  >
                    <div
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                      }}
                    >
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <span style={{ fontWeight: 500, fontSize: '13px' }}>
                          {p.display_name}
                          {isDefault && (
                            <span
                              style={{
                                fontSize: '11px',
                                marginLeft: '6px',
                                color: 'var(--accent)',
                              }}
                            >
                              {' '}
                              ✓ {t('settings.providers.default') }
                            </span>
                          )}
                          {p.is_builtin && (
                            <span
                              style={{
                                fontSize: '10px',
                                marginLeft: '6px',
                                color: 'var(--text-secondary)',
                              }}
                            >
                              {t('settings.providers.builtin') }
                            </span>
                          )}
                        </span>
                        <div
                          style={{
                            color: 'var(--text-secondary)',
                            fontSize: '11px',
                            marginTop: '2px',
                          }}
                        >
                          <code>{p.id}</code> · {p.kind} · {p.models.length}{' '}
                          {t('settings.providers.models') }
                          {p.base_url ? ` · ${p.base_url}` : ''}
                        </div>
                      </div>
                      <div style={{ display: 'flex', gap: '6px', flexShrink: 0 }}>
                        {!isDefault && (
                          <button
                            type="button"
                            onClick={() => setDefaultProvider(p.id)}
                            style={{
                              fontSize: '11px',
                              padding: '3px 10px',
                              borderRadius: '4px',
                              border: '1px solid var(--border)',
                              background: 'transparent',
                              color: 'var(--text-primary)',
                              cursor: 'pointer',
                            }}
                          >
                            {t('settings.providers.set_default') }
                          </button>
                        )}
                        {!p.is_builtin && (
                          <button
                            type="button"
                            onClick={() => removeProvider(p.id)}
                            disabled={isDefault}
                            style={{
                              fontSize: '11px',
                              padding: '3px 10px',
                              borderRadius: '4px',
                              border: '1px solid var(--border)',
                              background: 'transparent',
                              color: 'var(--text-primary)',
                              cursor: isDefault ? 'not-allowed' : 'pointer',
                              opacity: isDefault ? 0.5 : 1,
                            }}
                          >
                            {t('settings.providers.remove') }
                          </button>
                        )}
                      </div>
                    </div>
                    {/* 自定义 provider 的 API key 输入(内置 provider 走 T-E-S-40 的旧 keychain 命令)。 */}
                    {!p.is_builtin && (
                      <div
                        style={{
                          display: 'flex',
                          gap: '6px',
                          marginTop: '6px',
                          alignItems: 'center',
                        }}
                      >
                        <input
                          type="password"
                          value={providerKeys[p.id] ?? ''}
                          onInput={(e) =>
                            setProviderKeys((prev) => ({ ...prev, [p.id]: e.currentTarget.value }))
                          }
                          autocomplete="off"
                          spellcheck={false}
                          placeholder={
                            providerKeyConfigured[p.id]
                              ? t('settings.providers.keyConfigured') 
                              : t('settings.providers.keyPlaceholder') 
                          }
                          style={{
                            flex: 1,
                            fontSize: '12px',
                            padding: '4px 8px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'var(--bg-primary)',
                            color: 'var(--text-primary)',
                          }}
                        />
                        <button
                          type="button"
                          onClick={() => saveProviderKey(p.id)}
                          style={{
                            fontSize: '11px',
                            padding: '4px 10px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'transparent',
                            color: 'var(--text-primary)',
                            cursor: 'pointer',
                            flexShrink: 0,
                          }}
                        >
                          {t('settings.providers.save_key') }
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
          {/* 添加自定义 provider(JSON 文本框) */}
          <div style={{ marginTop: '12px' }}>
            <div style={{ fontSize: '12px', marginBottom: '4px', color: 'var(--text-secondary)' }}>
              {t('settings.providers.add_hint') ||
                '粘贴 provider JSON 添加自定义 provider(kind: openai-compat/anthropic/ollama/custom)'}
            </div>
            <textarea
              value={newProviderJson}
              onInput={(e) => setNewProviderJson(e.currentTarget.value)}
              placeholder={JSON.stringify(
                {
                  id: 'my-provider',
                  kind: 'openai-compat',
                  display_name: 'My Provider',
                  base_url: 'https://api.example.com/v1',
                  supports_tools: true,
                  supports_streaming: true,
                  is_builtin: false,
                  models: [
                    {
                      id: 'my-model',
                      display_name: 'My Model',
                      context_window: 32000,
                      pricing: { input_usd_per_1m: 0.5, output_usd_per_1m: 1.5 },
                    },
                  ],
                },
                null,
                2
              )}
              rows={8}
              style={{
                width: '100%',
                fontSize: '11px',
                fontFamily: 'monospace',
                padding: '8px',
                borderRadius: '4px',
                border: '1px solid var(--border)',
                background: 'var(--bg-primary)',
                color: 'var(--text-primary)',
                boxSizing: 'border-box',
                resize: 'vertical',
              }}
            />
            <button
              type="button"
              class="btn"
              onClick={addProviderFromJson}
              disabled={newProviderJson.trim().length === 0}
              style={{ marginTop: '6px', fontSize: '12px' }}
            >
              {t('settings.providers.add') }
            </button>
            {providerError && (
              <div style={{ color: 'var(--danger, #e53935)', fontSize: '11px', marginTop: '6px' }}>
                {providerError}
              </div>
            )}
          </div>
        </div>

        {/* T-E-B-09: 文件夹监控索引 — 监控目录变更自动吸收到 L3 语义记忆。 */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 4px;">{t('settings.watch.title') }</h3>
          <div style="color: var(--text-secondary); font-size: 11px; margin-bottom: 8px;">
            {t('settings.watch.hint') }
          </div>
          <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px;">
            <span
              style={{
                fontSize: '12px',
                color: watchActive ? 'var(--accent)' : 'var(--text-secondary)',
              }}
            >
              {watchActive
                ? t('settings.watch.active') 
                : t('settings.watch.inactive') }
            </span>
            <button
              type="button"
              disabled={s.watchPaths.length === 0 && !watchActive}
              onClick={() => toggleWatch(!watchActive)}
              style={{
                fontSize: '12px',
                padding: '4px 12px',
                borderRadius: '4px',
                border: '1px solid var(--border)',
                background: 'transparent',
                color: 'var(--text-primary)',
                cursor: s.watchPaths.length === 0 && !watchActive ? 'not-allowed' : 'pointer',
                opacity: s.watchPaths.length === 0 && !watchActive ? 0.5 : 1,
              }}
            >
              {watchActive
                ? t('settings.watch.disable') 
                : t('settings.watch.enable') }
            </button>
          </div>
          {s.watchPaths.length === 0 ? (
            <div style="color: var(--text-secondary); font-size: 13px;">
              {t('settings.watch.empty') }
            </div>
          ) : (
            <div style="display: flex; flex-direction: column; gap: 6px;">
              {s.watchPaths.map((p) => (
                <div
                  key={p}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '6px 10px',
                    borderRadius: '6px',
                    border: '1px solid var(--border)',
                  }}
                >
                  <span
                    style={{
                      flex: 1,
                      minWidth: 0,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                      fontSize: '12px',
                    }}
                    title={p}
                  >
                    {p}
                  </span>
                  <button
                    type="button"
                    onClick={() => removeWatchFolder(p)}
                    style={{
                      fontSize: '11px',
                      padding: '2px 8px',
                      marginLeft: '8px',
                      borderRadius: '4px',
                      border: '1px solid var(--border)',
                      background: 'transparent',
                      color: 'var(--text-primary)',
                      cursor: 'pointer',
                      flexShrink: 0,
                    }}
                  >
                    {t('settings.watch.remove') }
                  </button>
                </div>
              ))}
            </div>
          )}
          <button
            type="button"
            class="btn"
            onClick={addWatchFolder}
            style={{ marginTop: '8px', fontSize: '12px' }}
          >
            {t('settings.watch.add') }
          </button>
        </div>

        {/* T-E-S-39: AI 人格 — SOUL.md/AGENTS.md/TOOLS.md 注入到 LLM system prompt 前缀。 */}
        <div class="card" style="margin-top: 16px;">
          <h3 style="margin-bottom: 4px;">{t('settings.persona.title') }</h3>
          <div style="color: var(--text-secondary); font-size: 11px; margin-bottom: 8px;">
            {t('settings.persona.hint') ||
              '从工作区根目录读取 SOUL.md/AGENTS.md/TOOLS.md,注入到 LLM system prompt 前缀'}
          </div>
          <div style="display: flex; flex-direction: column; gap: 6px; margin-bottom: 8px;">
            {(
              [
                ['SOUL.md', 'soul_md', t('settings.persona.soul') || 'SOUL.md'],
                ['AGENTS.md', 'agents_md', t('settings.persona.agents') || 'AGENTS.md'],
                ['TOOLS.md', 'tools_md', t('settings.persona.tools') || 'TOOLS.md'],
              ] as const
            ).map(([filename, field, label]) => {
              const isLoaded = persona ? persona[field] != null : false;
              return (
                <div
                  key={filename}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '6px 10px',
                    borderRadius: '6px',
                    border: '1px solid var(--border)',
                  }}
                >
                  <span
                    style={{ fontSize: '12px', display: 'flex', alignItems: 'center', gap: '6px' }}
                  >
                    <span
                      style={{
                        display: 'inline-block',
                        width: '8px',
                        height: '8px',
                        borderRadius: '50%',
                        background: isLoaded ? 'var(--accent)' : 'var(--text-secondary)',
                        opacity: isLoaded ? 1 : 0.4,
                      }}
                    />
                    {label}
                    <span style={{ color: 'var(--text-secondary)', fontSize: '11px' }}>
                      {isLoaded
                        ? t('settings.persona.loaded') 
                        : t('settings.persona.missing') }
                    </span>
                  </span>
                  <button
                    type="button"
                    onClick={() => editPersonaFile(filename)}
                    style={{
                      fontSize: '11px',
                      padding: '2px 8px',
                      borderRadius: '4px',
                      border: '1px solid var(--border)',
                      background: 'transparent',
                      color: 'var(--text-primary)',
                      cursor: 'pointer',
                      flexShrink: 0,
                    }}
                  >
                    {t('settings.persona.edit') }
                  </button>
                </div>
              );
            })}
          </div>
          {persona && !persona.soul_md && !persona.agents_md && !persona.tools_md && (
            <div style="color: var(--text-secondary); font-size: 12px; margin-bottom: 8px;">
              {t('settings.persona.allMissing') }
            </div>
          )}
          <button
            type="button"
            disabled={personaReloading}
            onClick={reloadPersona}
            style={{
              fontSize: '12px',
              padding: '4px 12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'transparent',
              color: 'var(--text-primary)',
              cursor: personaReloading ? 'wait' : 'pointer',
              opacity: personaReloading ? 0.6 : 1,
            }}
          >
            {personaReloading ? (
              <Spinner size={16} showLabel={false} />
            ) : (
              t('settings.persona.reload') 
            )}
          </button>
          <button
            type="button"
            onClick={() => setEvolutionLogOpen(true)}
            title="查看 EvolutionEngine 4 Phase 进化日志 + 回滚 Soul 反哺"
            style={{
              fontSize: '12px',
              padding: '4px 12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'transparent',
              color: 'var(--text-primary)',
              cursor: 'pointer',
              marginLeft: '8px',
            }}
          >
            🧬 进化日志
          </button>
        </div>

        {/* T-E-C-17: IM 绑定 — Feishu/WeCom/DingTalk webhook 推送。 */}
        <ImBindingPanel />

        {/* M6 #77: Soul 编辑器 Modal — 从 persona 卡片"编辑 SOUL.md"按钮触发。 */}
        <SoulEditor open={soulEditorOpen} onClose={() => setSoulEditorOpen(false)} />

        {/* M6 #78: 进化日志 Modal — 从 persona 卡片"🧬 进化日志"按钮触发。 */}
        <EvolutionLogView open={evolutionLogOpen} onClose={() => setEvolutionLogOpen(false)} />

        {/* M6 #83: WorkType 配置 Modal — 从 LLM 提供商卡片"⚙ WorkType 配置"按钮触发。 */}
        <WorkTypeConfigView
          open={workTypeConfigOpen}
          onClose={() => setWorkTypeConfigOpen(false)}
        />

        {/* P0-1: 模型配置中心 Modal — 从 LLM 提供商卡片"模型配置中心"按钮触发。 */}
        <ModelConfigPanel
          open={modelConfigOpen}
          onClose={() => setModelConfigOpen(false)}
        />

        <footer class="settings-footer">
          <span class="settings-status">{saved ? t('settings.saved') : ''}</span>
          <button class="primary" onClick={save}>
            {t('settings.save')}
          </button>
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
