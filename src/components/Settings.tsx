/**
 * v2.2: Settings — macOS 系统偏好设置风格的双栏页面。
 *
 * 重构自原先的 Modal 形态:左侧 `.settings-nav` 分类导航(4 个分组 / 11
 * 个面板),右侧 `.settings-content` 配置面板,通过 `active` class 切换。
 *
 * 既有逻辑全部保留:
 *  - 主题 / 强调色 / 字号经由 `src/theme/index.ts` 信号驱动,保存时
 *    通过 `applyTheme()` 应用到 DOM。
 *  - locale / autosave / ollamaUrl / apiKey / workspace / 预算 /
 *    provider 配置走 localStorage + Tauri keychain。
 *  - 设备管理 / 文件夹监控 / AI 人格 / IM 绑定 / DID 等既有功能原样
 *    迁入对应面板。
 *
 * 新增面板(视觉 / 音频 / 视频 / 心跳 / 浏览器 / 多模态 / 自主性扩展 /
 * 外观扩展)后端尚无对应功能,暂用 localStorage 持久化,UI 正常显示。
 */
import { useEffect, useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
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

/**
 * localStorage 持久化的局部状态钩子,供新增面板(后端暂无对应功能)
 * 使用。`update()` 接受部分补丁,合并后立即写回 localStorage。
 */
function useLocalSettings<T extends object>(key: string, initial: T) {
  const [state, setState] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw ? ({ ...initial, ...(JSON.parse(raw) as Partial<T>) }) : initial;
    } catch {
      return initial;
    }
  });
  const update = (patch: Partial<T>) => {
    setState((prev) => {
      const next = { ...prev, ...patch };
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        /* localStorage 不可用时静默忽略 */
      }
      return next;
    });
  };
  return [state, update] as const;
}

/** 左侧导航分组与面板定义(对齐 preview.html 中的 11 个分类)。 */
const NAV_GROUPS: { group: string; items: { pane: string; icon: string; label: string }[] }[] = [
  {
    group: '基础',
    items: [
      { pane: 'general', icon: '🔧', label: '通用' },
      { pane: 'appearance', icon: '🎨', label: '外观' },
      { pane: 'autonomy', icon: '🎚️', label: '自主性' },
    ],
  },
  {
    group: 'AI 模型',
    items: [
      { pane: 'models', icon: '📦', label: '模型管理' },
      { pane: 'multimodal', icon: '🧩', label: '多模态' },
    ],
  },
  {
    group: '感知',
    items: [
      { pane: 'vision', icon: '👁️', label: '视觉' },
      { pane: 'audio', icon: '🎵', label: '音频' },
      { pane: 'video', icon: '🎬', label: '视频' },
    ],
  },
  {
    group: '连接',
    items: [
      { pane: 'heartbeat', icon: '💓', label: '心跳' },
      { pane: 'browser', icon: '🌐', label: '浏览器' },
    ],
  },
  {
    group: '其他',
    items: [{ pane: 'about', icon: 'ℹ️', label: '关于' }],
  },
];

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

  // 当前激活的设置面板(macOS 风格双栏左侧导航)。
  const [activePane, setActivePane] = useState<string>('general');

  // 外观扩展:毛玻璃 / 圆角 / 减少动效(localStorage,后端暂无对应功能)。
  const [appearanceExt, setAppearanceExt] = useLocalSettings('nebula.appearanceExt', {
    frostedGlass: true,
    borderRadius: 12,
    reduceMotion: false,
  });
  // 通用扩展:托盘最小化 / 自动更新 / 诊断日志(localStorage)。
  const [generalExt, setGeneralExt] = useLocalSettings('nebula.generalExt', {
    trayMinimize: true,
    autoUpdate: false,
    diagnosticLog: false,
  });
  // 自主性:L0-L4 等级 / 自动 Shell / 自动 Git / 沙箱超时(localStorage)。
  const [autonomy, setAutonomy] = useLocalSettings('nebula.autonomy', {
    level: 'L2' as 'L0' | 'L1' | 'L2' | 'L3' | 'L4',
    autoShell: false,
    autoGit: false,
    sandboxTimeout: 300,
  });
  // 多模态开关(localStorage)。
  const [multimodal, setMultimodal] = useLocalSettings('nebula.multimodal', {
    text: true,
    image: true,
    asr: true,
    tts: false,
    video: false,
    ocr: false,
  });
  // 视觉配置(localStorage)。
  const [vision, setVision] = useLocalSettings('nebula.vision', {
    model: 'llama3.1:8b',
    maxResolution: '1024 x 1024',
    ocrEngine: '本地 PP-OCRv5 (NPU)',
    screenshotShortcut: 'Ctrl+Shift+S',
    screenAwareness: false,
    screenshotInterval: 30,
  });
  // 音频配置(localStorage)。
  const [audio, setAudio] = useLocalSettings('nebula.audio', {
    asrModel: 'whisper-large-v3 (本地)',
    inputDevice: '系统默认',
    sampleRate: '16000 Hz',
    vadSensitivity: 50,
    ttsEngine: '本地 MeloTTS',
    voice: '女声 · 柔和',
    ttsSpeed: 100,
    realtimeTranslation: false,
  });
  // 视频配置(localStorage)。
  const [video, setVideo] = useLocalSettings('nebula.video', {
    analysis: true,
    frameInterval: 2,
    maxDuration: 300,
    frameResolution: '512 x 512',
    model: 'llama3.1:8b',
    cameraAnalysis: false,
    cameraInterval: 5,
  });
  // 心跳配置(localStorage,ollamaUrl 复用既有 AppSettings)。
  const [heartbeat, setHeartbeat] = useLocalSettings('nebula.heartbeat', {
    ollamaHealthCheck: true,
    checkInterval: 30,
    connectionTimeout: 5,
    retryCount: 3,
    failoverDegradation: true,
    degradationStrategy: 'DeepSeek → Ollama → 离线',
    swarmHeartbeat: 10,
    memoryGcCycle: 3600,
  });
  // 浏览器配置(localStorage)。
  const [browser, setBrowser] = useLocalSettings('nebula.browser', {
    webSearch: true,
    searchEngine: 'DuckDuckGo (隐私优先)',
    renderMode: '无头浏览器 (Headless)',
    proxyServer: '',
    userAgent: '',
    pageTimeout: 15,
    maxConcurrentPages: 5,
    jsExecution: true,
    autoScroll: false,
    screenshotArchive: false,
  });
  // 模型管理扩展:默认对话 / 编码模型(localStorage,选项来源于 modelsConfig)。
  const [modelsExt, setModelsExt] = useLocalSettings('nebula.modelsExt', {
    defaultChatModel: '',
    defaultCodeModel: '',
  });

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
      setProviderError(t('settings.providers.errNoModels'));
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
      setProviderError(t('settings.providers.errJson') + String(e));
      return;
    }
    // 基本字段完整性检查(与后端 ProviderConfig struct 对齐)。
    if (!parsed.id || !parsed.display_name || !parsed.kind || !Array.isArray(parsed.models)) {
      setProviderError(t('settings.providers.errFields'));
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

  // 恢复默认:仅重置主表单字段为 DEFAULTS(不自动保存,需用户点击「保存」)。
  function resetDefaults() {
    setS({ ...DEFAULTS, apiKey: '', openaiCompatKey: '' });
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

  // ── 行内渲染助手:保持 11 个面板的 JSX 简洁 ──────────────────────
  const renderToggle = (on: boolean, onClick: () => void) => (
    <div
      class={`settings-toggle${on ? ' on' : ''}`}
      role="switch"
      aria-checked={on}
      onClick={onClick}
    />
  );
  const renderRow = (label: string, desc: string, control: ComponentChildren) => (
    <div class="settings-row">
      <div>
        <div class="settings-row-label">{label}</div>
        <div class="settings-row-desc">{desc}</div>
      </div>
      <div class="settings-row-control">{control}</div>
    </div>
  );
  const rangeHint = (text: string) => (
    <span style="font-size:11px;color:rgba(255,255,255,0.4);">{text}</span>
  );
  const unitHint = (text: string) => (
    <span style="font-size:11px;color:rgba(255,255,255,0.4);">{text}</span>
  );

  // 多模态已启用计数(用于面板标题)。
  const multimodalEnabledCount = [
    multimodal.text,
    multimodal.image,
    multimodal.asr,
    multimodal.tts,
    multimodal.video,
    multimodal.ocr,
  ].filter(Boolean).length;

  // 从 modelsConfig 汇总所有模型 id,供「默认对话 / 编码模型」下拉使用。
  const allModelIds: string[] = modelsConfig
    ? modelsConfig.providers.flatMap((p) => p.models.map((m) => m.id))
    : [];

  return (
    <div class="settings-view">
      {/* 页头:标题 + 恢复默认 / 保存 / 关闭 */}
      <div class="page-header">
        <div>
          <div class="page-title">⚙️ {t('settings.title')}</div>
          <div class="page-subtitle">系统配置 · 11 个分类</div>
        </div>
        <div class="page-actions">
          <div class="tool-btn" onClick={resetDefaults}>
            恢复默认
          </div>
          <div class="tool-btn tool-btn-primary" onClick={save}>
            {saved ? t('settings.saved') : t('settings.save')}
          </div>
          <button class="icon-btn" onClick={onClose} aria-label={t('settings.close')}>
            ×
          </button>
        </div>
      </div>

      <div class="settings-layout">
        {/* 左侧分类导航 */}
        <div class="settings-nav">
          {NAV_GROUPS.flatMap((g) => [
            <div key={`${g.group}-g`} class="settings-nav-group">
              {g.group}
            </div>,
            ...g.items.map((it) => (
              <div
                key={it.pane}
                class={`settings-nav-item${activePane === it.pane ? ' active' : ''}`}
                onClick={() => setActivePane(it.pane)}
              >
                <span class="settings-nav-icon">{it.icon}</span>
                {it.label}
              </div>
            )),
          ])}
        </div>

        {/* 右侧配置面板:11 个 pane 同时渲染,通过 active class 切换可见性 */}
        <div class="settings-content">
          {/* ═══ 通用 ═══ */}
          <div class={`settings-pane${activePane === 'general' ? ' active' : ''}`}>
            <div class="settings-pane-title">通用</div>
            {renderRow(
              t('settings.language'),
              '界面显示语言',
              <select
                class="settings-select-sm"
                value={s.locale}
                onChange={(e) => update('locale', e.currentTarget.value as Locale)}
              >
                {LOCALES.map((l) => (
                  <option key={l} value={l}>
                    {l === 'zh-CN' ? '简体中文' : 'English'}
                  </option>
                ))}
              </select>
            )}
            {renderRow(
              t('settings.fontSize'),
              '全局字体缩放 (12-20px)',
              <>
                <input
                  type="range"
                  min={FONT_MIN}
                  max={FONT_MAX}
                  step={FONT_STEP}
                  value={s.fontSize}
                  class="settings-range"
                  style="width:120px;"
                  onInput={(e) =>
                    update('fontSize', Number(e.currentTarget.value) || FONT_DEFAULT)
                  }
                />
                {rangeHint(`${clampFontSize(s.fontSize)}px`)}
              </>
            )}
            {renderRow(
              t('settings.autostart'),
              '系统登录时自动启动 Nebula',
              renderToggle(autostartEnabled, () => toggleAutostart(!autostartEnabled))
            )}
            {renderRow(
              '托盘最小化',
              '关闭窗口时最小化到系统托盘',
              renderToggle(generalExt.trayMinimize, () =>
                setGeneralExt({ trayMinimize: !generalExt.trayMinimize })
              )
            )}
            {renderRow(
              '自动检查更新',
              '每周检查新版本',
              renderToggle(generalExt.autoUpdate, () =>
                setGeneralExt({ autoUpdate: !generalExt.autoUpdate })
              )
            )}
            {renderRow(
              '诊断日志',
              '启用 NEBULA_DIAGNOSTICS 详细日志',
              renderToggle(generalExt.diagnosticLog, () =>
                setGeneralExt({ diagnosticLog: !generalExt.diagnosticLog })
              )
            )}
            {renderRow(
              t('settings.autosave'),
              '会话自动保存间隔',
              <input
                class="settings-input-sm"
                type="number"
                min="1"
                max="60"
                value={s.autosaveSec}
                onInput={(e) => update('autosaveSec', Number(e.currentTarget.value) || 5)}
              />
            )}
            {renderRow(
              t('settings.workspace'),
              'Nebula 工作区根目录',
              <input
                class="settings-input-sm"
                type="text"
                value={s.workspace}
                onInput={(e) => update('workspace', e.currentTarget.value)}
              />
            )}

            {/* 文件夹监控索引 — 监控目录变更自动吸收到 L3 语义记忆 */}
            <div class="settings-section">
              <div class="settings-section-title">{t('settings.watch.title')}</div>
              <div class="settings-row-desc" style="margin-bottom:8px;">
                {t('settings.watch.hint')}
              </div>
              <div
                style="display:flex;align-items:center;justify-content:space-between;margin-bottom:8px;"
              >
                <span
                  style={{
                    fontSize: '12px',
                    color: watchActive ? 'var(--accent)' : 'var(--text-secondary)',
                  }}
                >
                  {watchActive ? t('settings.watch.active') : t('settings.watch.inactive')}
                </span>
                <button
                  type="button"
                  class="tool-btn"
                  disabled={s.watchPaths.length === 0 && !watchActive}
                  onClick={() => toggleWatch(!watchActive)}
                >
                  {watchActive ? t('settings.watch.disable') : t('settings.watch.enable')}
                </button>
              </div>
              {s.watchPaths.length === 0 ? (
                <div style="color:var(--text-secondary);font-size:13px;">
                  {t('settings.watch.empty')}
                </div>
              ) : (
                <div style="display:flex;flex-direction:column;gap:6px;">
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
                        class="tool-btn"
                        onClick={() => removeWatchFolder(p)}
                      >
                        {t('settings.watch.remove')}
                      </button>
                    </div>
                  ))}
                </div>
              )}
              <button type="button" class="tool-btn" onClick={addWatchFolder}>
                {t('settings.watch.add')}
              </button>
            </div>

            {/* 已配对设备 */}
            <div class="settings-section">
              <div class="settings-section-title">{t('settings.devices')}</div>
              {deviceLoading ? (
                <Spinner label={t('common.loading')} />
              ) : devices.length === 0 ? (
                <div style="color:var(--text-secondary);font-size:13px;">
                  {t('settings.devicesHint')}
                </div>
              ) : (
                <div style="display:flex;flex-direction:column;gap:8px;">
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
                      <div style="flex:1;min-width:0;">
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
                        <div style="color:var(--text-secondary);font-size:11px;margin-top:2px;">
                          配对于 {new Date(d.paired_at * 1000).toLocaleString()}
                        </div>
                      </div>
                      {!d.revoked && (
                        <button
                          type="button"
                          class="tool-btn"
                          disabled={deviceRevoking === d.device_id}
                          onClick={() => revokeDevice(d.device_id)}
                        >
                          {deviceRevoking === d.device_id ? '撤销中…' : '撤销'}
                        </button>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>

          {/* ═══ 外观 ═══ */}
          <div class={`settings-pane${activePane === 'appearance' ? ' active' : ''}`}>
            <div class="settings-pane-title">外观</div>
            {renderRow(
              t('settings.theme'),
              '深色 / 浅色 / 跟随系统',
              <div class="settings-segmented" style="margin:0;">
                <div
                  class={`settings-seg-btn${s.theme === 'dark' ? ' active' : ''}`}
                  onClick={() => update('theme', 'dark')}
                >
                  {t('settings.theme.dark')}
                </div>
                <div
                  class={`settings-seg-btn${s.theme === 'light' ? ' active' : ''}`}
                  onClick={() => update('theme', 'light')}
                >
                  {t('settings.theme.light')}
                </div>
                <div
                  class={`settings-seg-btn${s.theme === 'system' ? ' active' : ''}`}
                  onClick={() => update('theme', 'system')}
                >
                  {t('settings.theme.system')}
                </div>
              </div>
            )}
            {renderRow(
              '毛玻璃效果',
              '侧边栏和标题栏使用 backdrop-filter 模糊',
              renderToggle(appearanceExt.frostedGlass, () =>
                setAppearanceExt({ frostedGlass: !appearanceExt.frostedGlass })
              )
            )}
            {renderRow(
              t('settings.accent'),
              '按钮、链接、高亮颜色',
              <div class="color-swatches" role="radiogroup" aria-label={t('settings.accent')}>
                {ACCENT_OPTIONS.map((o) => (
                  <button
                    key={o.value}
                    type="button"
                    role="radio"
                    aria-checked={s.accent === o.value}
                    class={`color-swatch${s.accent === o.value ? ' active' : ''}`}
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
            )}
            {renderRow(
              '圆角半径',
              '卡片和按钮圆角 (4-20px)',
              <>
                <input
                  type="range"
                  min="4"
                  max="20"
                  value={appearanceExt.borderRadius}
                  class="settings-range"
                  style="width:120px;"
                  onInput={(e) =>
                    setAppearanceExt({ borderRadius: Number(e.currentTarget.value) || 12 })
                  }
                />
                {rangeHint(`${appearanceExt.borderRadius}px`)}
              </>
            )}
            {renderRow(
              '减少动效',
              '遵循 prefers-reduced-motion',
              renderToggle(appearanceExt.reduceMotion, () =>
                setAppearanceExt({ reduceMotion: !appearanceExt.reduceMotion })
              )
            )}
          </div>

          {/* ═══ 自主性 ═══ */}
          <div class={`settings-pane${activePane === 'autonomy' ? ' active' : ''}`}>
            <div class="settings-pane-title">自主性等级</div>
            <div class="autonomy-level-card">
              <div class="settings-row-desc" style="margin-bottom:8px;">
                当前等级
              </div>
              <div class="settings-segmented" style="margin:0;display:flex;width:100%;">
                {(['L0', 'L1', 'L2', 'L3', 'L4'] as const).map((lv) => (
                  <div
                    key={lv}
                    class={`settings-seg-btn${autonomy.level === lv ? ' active' : ''}`}
                    style="flex:1;text-align:center;"
                    onClick={() => setAutonomy({ level: lv })}
                  >
                    {lv === 'L0'
                      ? 'L0 静默'
                      : lv === 'L1'
                        ? 'L1 建议'
                        : lv === 'L2'
                          ? 'L2 对话'
                          : lv === 'L3'
                            ? 'L3 半自动'
                            : 'L4 自动'}
                  </div>
                ))}
              </div>
              <div class="settings-row-desc" style="margin-top:10px;">
                {autonomy.level === 'L0'
                  ? 'L0 静默:仅响应显式指令'
                  : autonomy.level === 'L1'
                    ? 'L1 建议:可主动建议,执行需确认'
                    : autonomy.level === 'L2'
                      ? 'L2 对话:执行前需用户确认,可自动搜索和读取文件'
                      : autonomy.level === 'L3'
                        ? 'L3 半自动:白名单内命令可自动执行'
                        : 'L4 自动:全自主,沙箱内可自由执行'}
              </div>
            </div>
            {renderRow(
              '自动执行 Shell 命令',
              'L3+ 时允许自动运行白名单内的命令',
              renderToggle(autonomy.autoShell, () => setAutonomy({ autoShell: !autonomy.autoShell }))
            )}
            {renderRow(
              '自动提交 Git',
              'L3+ 时允许自动 git add/commit',
              renderToggle(autonomy.autoGit, () => setAutonomy({ autoGit: !autonomy.autoGit }))
            )}
            {renderRow(
              '沙箱超时',
              '影子工作区任务最长执行时间',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={autonomy.sandboxTimeout}
                  onInput={(e) =>
                    setAutonomy({ sandboxTimeout: Number(e.currentTarget.value) || 300 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
            {renderRow(
              'LLM 智能路由',
              '关闭后退化为关键词启发式',
              renderToggle(aiAutoMode, () => toggleAiAutoMode(!aiAutoMode))
            )}
            {modeMisclassification > 0 && (
              <div class="settings-row-desc" style="margin:-4px 0 8px;">
                误分类计数: {modeMisclassification}
              </div>
            )}

            {/* AI 人格 — SOUL.md / AGENTS.md / TOOLS.md */}
            <div class="settings-section">
              <div class="settings-section-title">{t('settings.persona.title')}</div>
              <div class="settings-row-desc" style="margin-bottom:8px;">
                {t('settings.persona.hint')}
              </div>
              <div style="display:flex;flex-direction:column;gap:6px;margin-bottom:8px;">
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
                        style={{
                          fontSize: '12px',
                          display: 'flex',
                          alignItems: 'center',
                          gap: '6px',
                        }}
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
                          {isLoaded ? t('settings.persona.loaded') : t('settings.persona.missing')}
                        </span>
                      </span>
                      <button
                        type="button"
                        class="tool-btn"
                        onClick={() => editPersonaFile(filename)}
                      >
                        {t('settings.persona.edit')}
                      </button>
                    </div>
                  );
                })}
              </div>
              {persona && !persona.soul_md && !persona.agents_md && !persona.tools_md && (
                <div style="color:var(--text-secondary);font-size:12px;margin-bottom:8px;">
                  {t('settings.persona.allMissing')}
                </div>
              )}
              <div style="display:flex;gap:8px;flex-wrap:wrap;">
                <button
                  type="button"
                  class="tool-btn"
                  disabled={personaReloading}
                  onClick={reloadPersona}
                >
                  {personaReloading ? <Spinner size={16} showLabel={false} /> : t('settings.persona.reload')}
                </button>
                <button
                  type="button"
                  class="tool-btn"
                  onClick={() => setEvolutionLogOpen(true)}
                  title="查看 EvolutionEngine 4 Phase 进化日志 + 回滚 Soul 反哺"
                >
                  🧬 进化日志
                </button>
              </div>
            </div>
          </div>

          {/* ═══ 模型管理 ═══ */}
          <div class={`settings-pane${activePane === 'models' ? ' active' : ''}`}>
            <div class="settings-pane-head">
              <span class="settings-pane-title">{t('settings.providers.title')}</span>
              <div style="display:flex;gap:6px;flex-wrap:wrap;">
                <button
                  type="button"
                  class="tool-btn"
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
                >
                  {t('settings.providers.reload')}
                </button>
                <button
                  type="button"
                  class="tool-btn"
                  onClick={() => setWorkTypeConfigOpen(true)}
                  title={t('workTypeConfig.openButtonTitle')}
                >
                  {t('workTypeConfig.openButton')}
                </button>
                <button
                  type="button"
                  class="tool-btn"
                  onClick={() => setModelConfigOpen(true)}
                  title="打开模型配置中心(provider 管理 / API Key / 连通性测试 / 模型发现 / WorkType 路由)"
                >
                  模型配置中心
                </button>
              </div>
            </div>
            <div class="settings-row-desc" style="margin-bottom:8px;">
              {t('settings.providers.hint') ||
                '管理 LLM provider 列表;新增/删除重启生效,默认值可热更新'}
            </div>
            {modelsConfig === null ? (
              <Spinner label={t('common.loading')} />
            ) : (
              <div style="display:flex;flex-direction:column;gap:8px;">
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
                                ✓ {t('settings.providers.default')}
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
                                {t('settings.providers.builtin')}
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
                            {t('settings.providers.models')}
                            {p.base_url ? ` · ${p.base_url}` : ''}
                          </div>
                        </div>
                        <div style={{ display: 'flex', gap: '6px', flexShrink: 0 }}>
                          {!isDefault && (
                            <button
                              type="button"
                              class="tool-btn"
                              onClick={() => setDefaultProvider(p.id)}
                            >
                              {t('settings.providers.set_default')}
                            </button>
                          )}
                          {!p.is_builtin && (
                            <button
                              type="button"
                              class="tool-btn"
                              onClick={() => removeProvider(p.id)}
                              disabled={isDefault}
                            >
                              {t('settings.providers.remove')}
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
                              setProviderKeys((prev) => ({
                                ...prev,
                                [p.id]: e.currentTarget.value,
                              }))
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
                            class="tool-btn"
                            onClick={() => saveProviderKey(p.id)}
                          >
                            {t('settings.providers.save_key')}
                          </button>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}

            {/* 添加自定义 provider(JSON 文本框) */}
            <div class="settings-section">
              <div class="settings-row-desc" style="margin-bottom:4px;">
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
                rows={6}
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
              <div style="display:flex;gap:6px;margin-top:6px;">
                <button
                  type="button"
                  class="tool-btn tool-btn-primary"
                  onClick={addProviderFromJson}
                  disabled={newProviderJson.trim().length === 0}
                >
                  {t('settings.providers.add')}
                </button>
              </div>
              {providerError && (
                <div style={{ color: 'var(--danger, #e53935)', fontSize: '11px', marginTop: '6px' }}>
                  {providerError}
                </div>
              )}
            </div>

            {/* API Key(主 provider,经 OS keychain) */}
            {renderRow(
              t('settings.apiKey'),
              keyConfigured ? '✓ ' + t('settings.apiKeyConfigured') : '存储于系统钥匙串',
              <input
                class="settings-input-sm"
                type="password"
                value={s.apiKey}
                onInput={(e) => update('apiKey', e.currentTarget.value)}
                autocomplete="off"
                spellcheck={false}
                placeholder={keyConfigured ? '••••••••' : ''}
              />
            )}
            {/* LLM Provider 选择 */}
            {renderRow(
              t('settings.provider'),
              '主 LLM 提供商',
              <select
                class="settings-select-sm"
                value={s.llmProvider}
                onChange={(e) => update('llmProvider', e.currentTarget.value)}
              >
                <option value="deepseek">{t('settings.provider.deepseek') || 'DeepSeek'}</option>
                <option value="ollama">{t('settings.provider.ollama')}</option>
                <option value="openai-compat">{t('settings.provider.openai-compat')}</option>
                <option value="anthropic">
                  {t('settings.provider.anthropic') || 'Anthropic Claude'}
                </option>
              </select>
            )}
            {/* OpenAI 兼容配置区 — 仅当 provider=openai-compat 时显示 */}
            {s.llmProvider === 'openai-compat' && (
              <div class="settings-section">
                <div class="settings-section-title">{t('settings.openaiCompat')}</div>
                {renderRow(
                  t('settings.openaiCompatUrl') || 'Base URL',
                  t('settings.openaiCompatUrlHint') ||
                    '如 https://openrouter.ai/api/v1 或 http://localhost:1234/v1',
                  <input
                    class="settings-input-sm"
                    type="url"
                    value={s.openaiCompatUrl}
                    onInput={(e) => update('openaiCompatUrl', e.currentTarget.value)}
                    placeholder="https://openrouter.ai/api/v1"
                  />
                )}
                {renderRow(
                  t('settings.openaiCompatModel'),
                  t('settings.openaiCompatModelHint') ||
                    '如 openai/gpt-4o-mini / llama-3.1-8b-instruct',
                  <input
                    class="settings-input-sm"
                    type="text"
                    value={s.openaiCompatModel}
                    onInput={(e) => update('openaiCompatModel', e.currentTarget.value)}
                    placeholder="openai/gpt-4o-mini"
                  />
                )}
                {renderRow(
                  t('settings.openaiCompatKey') || 'API Key',
                  openaiCompatKeyConfigured
                    ? '✓ ' + t('settings.openaiCompatKeyConfigured')
                    : '存储于系统钥匙串',
                  <input
                    class="settings-input-sm"
                    type="password"
                    value={s.openaiCompatKey}
                    onInput={(e) => update('openaiCompatKey', e.currentTarget.value)}
                    autocomplete="off"
                    spellcheck={false}
                    placeholder={openaiCompatKeyConfigured ? '••••••••' : ''}
                  />
                )}
              </div>
            )}
            {/* 预算 */}
            {renderRow(
              '月预算 (USD, 0=不限)',
              '本月 API 花费上限',
              <input
                class="settings-input-sm"
                type="number"
                min="0"
                step="0.50"
                value={s.monthlyBudgetUsd}
                onInput={(e) => update('monthlyBudgetUsd', parseFloat(e.currentTarget.value) || 0)}
              />
            )}
            {renderRow(
              '日预算 (USD, 0=不限, 超限降级 Ollama)',
              '今日 API 花费上限',
              <input
                class="settings-input-sm"
                type="number"
                min="0"
                step="0.10"
                value={s.dailyBudgetUsd}
                onInput={(e) => update('dailyBudgetUsd', parseFloat(e.currentTarget.value) || 0)}
              />
            )}
            {/* 默认对话 / 编码模型 */}
            {renderRow(
              '默认对话模型',
              '全局使用的默认 LLM',
              <select
                class="settings-select-sm"
                value={modelsExt.defaultChatModel}
                onChange={(e) => setModelsExt({ defaultChatModel: e.currentTarget.value })}
              >
                <option value="">
                  {allModelIds.length === 0 ? '（暂无可用模型）' : '选择模型'}
                </option>
                {allModelIds.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            )}
            {renderRow(
              '默认编码模型',
              '代码工作台使用的模型',
              <select
                class="settings-select-sm"
                value={modelsExt.defaultCodeModel}
                onChange={(e) => setModelsExt({ defaultCodeModel: e.currentTarget.value })}
              >
                <option value="">
                  {allModelIds.length === 0 ? '（暂无可用模型）' : '选择模型'}
                </option>
                {allModelIds.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            )}
          </div>

          {/* ═══ 多模态 ═══ */}
          <div class={`settings-pane${activePane === 'multimodal' ? ' active' : ''}`}>
            <div class="settings-pane-head">
              <span class="settings-pane-title">多模态调用</span>
              <span class="settings-row-desc">{multimodalEnabledCount} / 6 已启用</span>
            </div>
            {renderRow(
              '📝 文本对话',
              '纯文本输入输出 · deepseek-chat',
              renderToggle(multimodal.text, () => setMultimodal({ text: !multimodal.text }))
            )}
            {renderRow(
              '🖼️ 图像理解',
              '图片输入 → 文本描述 · llama3.1:8b',
              renderToggle(multimodal.image, () => setMultimodal({ image: !multimodal.image }))
            )}
            {renderRow(
              '🎤 语音输入 (ASR)',
              '音频 → 文本 · whisper-large-v3 本地',
              renderToggle(multimodal.asr, () => setMultimodal({ asr: !multimodal.asr }))
            )}
            {renderRow(
              '🔊 语音输出 (TTS)',
              '文本 → 音频 · 未绑定模型',
              renderToggle(multimodal.tts, () => setMultimodal({ tts: !multimodal.tts }))
            )}
            {renderRow(
              '🎥 视频分析',
              '视频帧抽取 + 图像理解 · 未绑定',
              renderToggle(multimodal.video, () => setMultimodal({ video: !multimodal.video }))
            )}
            {renderRow(
              '📄 文档解析 (OCR)',
              'PDF/图片 → Markdown · 未绑定',
              renderToggle(multimodal.ocr, () => setMultimodal({ ocr: !multimodal.ocr }))
            )}
            <div class="settings-sovereignty">
              🔒 数据主权:所有模态优先使用本地模型,远程模型仅在本地不可用时降级使用
            </div>
          </div>

          {/* ═══ 视觉 ═══ */}
          <div class={`settings-pane${activePane === 'vision' ? ' active' : ''}`}>
            <div class="settings-pane-title">视觉配置</div>
            {renderRow(
              '图像理解模型',
              '用于图片描述、截图分析',
              <select
                class="settings-select-sm"
                value={vision.model}
                onChange={(e) => setVision({ model: e.currentTarget.value })}
              >
                <option>llama3.1:8b</option>
                <option>qwen2-vl:7b</option>
                <option>claude-3-5-sonnet</option>
              </select>
            )}
            {renderRow(
              '最大图片分辨率',
              '超过此尺寸自动缩放',
              <select
                class="settings-select-sm"
                value={vision.maxResolution}
                onChange={(e) => setVision({ maxResolution: e.currentTarget.value })}
              >
                <option>1024 x 1024</option>
                <option>2048 x 2048</option>
                <option>原始尺寸</option>
              </select>
            )}
            {renderRow(
              'OCR 引擎',
              '文档/截图文字识别',
              <select
                class="settings-select-sm"
                value={vision.ocrEngine}
                onChange={(e) => setVision({ ocrEngine: e.currentTarget.value })}
              >
                <option>本地 PP-OCRv5 (NPU)</option>
                <option>本地 Tesseract</option>
                <option>禁用</option>
              </select>
            )}
            {renderRow(
              '截图快捷键',
              '全屏截图发送给 AI 分析',
              <input
                class="settings-input-sm"
                style="width:120px;"
                value={vision.screenshotShortcut}
                onInput={(e) => setVision({ screenshotShortcut: e.currentTarget.value })}
              />
            )}
            {renderRow(
              '屏幕实时感知',
              '定时截屏并喂给视觉模型(高消耗)',
              renderToggle(vision.screenAwareness, () =>
                setVision({ screenAwareness: !vision.screenAwareness })
              )
            )}
            {renderRow(
              '截屏间隔',
              '屏幕感知模式下的截图频率',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={vision.screenshotInterval}
                  onInput={(e) =>
                    setVision({ screenshotInterval: Number(e.currentTarget.value) || 30 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
          </div>

          {/* ═══ 音频 ═══ */}
          <div class={`settings-pane${activePane === 'audio' ? ' active' : ''}`}>
            <div class="settings-pane-title">音频配置</div>
            <div class="settings-subhead">语音输入 (ASR)</div>
            {renderRow(
              'ASR 模型',
              '语音转文字引擎',
              <select
                class="settings-select-sm"
                value={audio.asrModel}
                onChange={(e) => setAudio({ asrModel: e.currentTarget.value })}
              >
                <option>whisper-large-v3 (本地)</option>
                <option>whisper-medium (本地)</option>
                <option>Paraformer (流式)</option>
              </select>
            )}
            {renderRow(
              '输入设备',
              '麦克风来源',
              <select
                class="settings-select-sm"
                value={audio.inputDevice}
                onChange={(e) => setAudio({ inputDevice: e.currentTarget.value })}
              >
                <option>系统默认</option>
                <option>外接麦克风</option>
              </select>
            )}
            {renderRow(
              '采样率',
              '音频采样质量',
              <select
                class="settings-select-sm"
                value={audio.sampleRate}
                onChange={(e) => setAudio({ sampleRate: e.currentTarget.value })}
              >
                <option>16000 Hz</option>
                <option>44100 Hz</option>
              </select>
            )}
            {renderRow(
              'VAD 灵敏度',
              '语音活动检测阈值',
              <input
                type="range"
                min="0"
                max="100"
                value={audio.vadSensitivity}
                class="settings-range"
                style="width:120px;"
                onInput={(e) =>
                  setAudio({ vadSensitivity: Number(e.currentTarget.value) || 50 })
                }
              />
            )}
            <div class="settings-subhead">语音输出 (TTS)</div>
            {renderRow(
              'TTS 引擎',
              '文字转语音引擎',
              <select
                class="settings-select-sm"
                value={audio.ttsEngine}
                onChange={(e) => setAudio({ ttsEngine: e.currentTarget.value })}
              >
                <option>本地 MeloTTS</option>
                <option>本地 Kokoro</option>
                <option>禁用</option>
              </select>
            )}
            {renderRow(
              '语音音色',
              '发音人声线',
              <select
                class="settings-select-sm"
                value={audio.voice}
                onChange={(e) => setAudio({ voice: e.currentTarget.value })}
              >
                <option>女声 · 柔和</option>
                <option>男声 · 沉稳</option>
                <option>女声 · 活泼</option>
              </select>
            )}
            {renderRow(
              '语速',
              'TTS 输出速度倍率',
              <>
                <input
                  type="range"
                  min="50"
                  max="200"
                  value={audio.ttsSpeed}
                  class="settings-range"
                  style="width:120px;"
                  onInput={(e) => setAudio({ ttsSpeed: Number(e.currentTarget.value) || 100 })}
                />
                {rangeHint(`${(audio.ttsSpeed / 100).toFixed(1)}x`)}
              </>
            )}
            {renderRow(
              '实时语音翻译',
              '同声传译模式(ASR → 翻译 → TTS)',
              renderToggle(audio.realtimeTranslation, () =>
                setAudio({ realtimeTranslation: !audio.realtimeTranslation })
              )
            )}
          </div>

          {/* ═══ 视频 ═══ */}
          <div class={`settings-pane${activePane === 'video' ? ' active' : ''}`}>
            <div class="settings-pane-title">视频配置</div>
            {renderRow(
              '视频分析',
              '启用视频帧抽取 + 图像理解',
              renderToggle(video.analysis, () => setVideo({ analysis: !video.analysis }))
            )}
            {renderRow(
              '帧抽取间隔',
              '每 N 秒抽取一帧送入视觉模型',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={video.frameInterval}
                  onInput={(e) =>
                    setVideo({ frameInterval: Number(e.currentTarget.value) || 2 })
                  }
                />
                {unitHint('秒/帧')}
              </>
            )}
            {renderRow(
              '最大视频时长',
              '超过此时长的视频截断处理',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={video.maxDuration}
                  onInput={(e) => setVideo({ maxDuration: Number(e.currentTarget.value) || 300 })}
                />
                {unitHint('秒')}
              </>
            )}
            {renderRow(
              '抽帧分辨率',
              '抽取帧的缩放尺寸',
              <select
                class="settings-select-sm"
                value={video.frameResolution}
                onChange={(e) => setVideo({ frameResolution: e.currentTarget.value })}
              >
                <option>512 x 512</option>
                <option>1024 x 1024</option>
                <option>原始尺寸</option>
              </select>
            )}
            {renderRow(
              '视频理解模型',
              '用于分析抽取帧的视觉模型',
              <select
                class="settings-select-sm"
                value={video.model}
                onChange={(e) => setVideo({ model: e.currentTarget.value })}
              >
                <option>llama3.1:8b</option>
                <option>qwen2-vl:7b</option>
              </select>
            )}
            {renderRow(
              '摄像头实时分析',
              '定时从摄像头取帧分析(高消耗)',
              renderToggle(video.cameraAnalysis, () =>
                setVideo({ cameraAnalysis: !video.cameraAnalysis })
              )
            )}
            {renderRow(
              '摄像头间隔',
              '摄像头分析模式下的取帧频率',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={video.cameraInterval}
                  onInput={(e) =>
                    setVideo({ cameraInterval: Number(e.currentTarget.value) || 5 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
          </div>

          {/* ═══ 心跳 ═══ */}
          <div class={`settings-pane${activePane === 'heartbeat' ? ' active' : ''}`}>
            <div class="settings-pane-title">心跳配置</div>
            {renderRow(
              t('settings.ollamaUrl'),
              '本地 Ollama 守护进程地址',
              <input
                class="settings-input-sm"
                type="url"
                value={s.ollamaUrl}
                onInput={(e) => update('ollamaUrl', e.currentTarget.value)}
              />
            )}
            {renderRow(
              'Ollama 健康检查',
              '定时检测本地 Ollama 守护进程状态',
              renderToggle(heartbeat.ollamaHealthCheck, () =>
                setHeartbeat({ ollamaHealthCheck: !heartbeat.ollamaHealthCheck })
              )
            )}
            {renderRow(
              '检查间隔',
              'Provider 健康检查频率',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={heartbeat.checkInterval}
                  onInput={(e) =>
                    setHeartbeat({ checkInterval: Number(e.currentTarget.value) || 30 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
            {renderRow(
              '连接超时',
              '单次健康检查超时时间',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={heartbeat.connectionTimeout}
                  onInput={(e) =>
                    setHeartbeat({ connectionTimeout: Number(e.currentTarget.value) || 5 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
            {renderRow(
              '重试次数',
              '检查失败后重试次数',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={heartbeat.retryCount}
                  onInput={(e) =>
                    setHeartbeat({ retryCount: Number(e.currentTarget.value) || 3 })
                  }
                />
                {unitHint('次')}
              </>
            )}
            {renderRow(
              '失败降级',
              'Provider 不可用时自动切换到备用模型',
              renderToggle(heartbeat.failoverDegradation, () =>
                setHeartbeat({ failoverDegradation: !heartbeat.failoverDegradation })
              )
            )}
            {renderRow(
              '降级策略',
              'Provider 不可用时的处理方式',
              <select
                class="settings-select-sm"
                value={heartbeat.degradationStrategy}
                onChange={(e) => setHeartbeat({ degradationStrategy: e.currentTarget.value })}
              >
                <option>DeepSeek → Ollama → 离线</option>
                <option>仅本地 Ollama</option>
                <option>提示用户</option>
              </select>
            )}
            {renderRow(
              '蜂群心跳',
              'Worker Agent 活跃检测间隔',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={heartbeat.swarmHeartbeat}
                  onInput={(e) =>
                    setHeartbeat({ swarmHeartbeat: Number(e.currentTarget.value) || 10 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
            {renderRow(
              '记忆 GC 周期',
              'L0 缓存清理 + L2 经验蒸馏频率',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={heartbeat.memoryGcCycle}
                  onInput={(e) =>
                    setHeartbeat({ memoryGcCycle: Number(e.currentTarget.value) || 3600 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
          </div>

          {/* ═══ 浏览器 ═══ */}
          <div class={`settings-pane${activePane === 'browser' ? ' active' : ''}`}>
            <div class="settings-pane-title">浏览器配置</div>
            {renderRow(
              '启用网页搜索',
              '允许 AI 通过浏览器搜索实时信息',
              renderToggle(browser.webSearch, () => setBrowser({ webSearch: !browser.webSearch }))
            )}
            {renderRow(
              '搜索引擎',
              '默认搜索后端',
              <select
                class="settings-select-sm"
                value={browser.searchEngine}
                onChange={(e) => setBrowser({ searchEngine: e.currentTarget.value })}
              >
                <option>DuckDuckGo (隐私优先)</option>
                <option>Google</option>
                <option>Bing</option>
                <option>SearXNG (自建)</option>
              </select>
            )}
            {renderRow(
              '渲染模式',
              '网页内容获取方式',
              <select
                class="settings-select-sm"
                value={browser.renderMode}
                onChange={(e) => setBrowser({ renderMode: e.currentTarget.value })}
              >
                <option>无头浏览器 (Headless)</option>
                <option>HTTP 抓取 (轻量)</option>
                <option>Defuddle 正文提取</option>
              </select>
            )}
            {renderRow(
              '代理服务器',
              'HTTP/SOCKS5 代理地址',
              <input
                class="settings-input-sm"
                style="width:160px;"
                value={browser.proxyServer}
                onInput={(e) => setBrowser({ proxyServer: e.currentTarget.value })}
                placeholder="http://127.0.0.1:7890"
              />
            )}
            {renderRow(
              'User-Agent',
              '自定义浏览器标识',
              <input
                class="settings-input-sm"
                style="width:160px;"
                value={browser.userAgent}
                onInput={(e) => setBrowser({ userAgent: e.currentTarget.value })}
                placeholder="默认"
              />
            )}
            {renderRow(
              '页面超时',
              '单页加载最长等待时间',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={browser.pageTimeout}
                  onInput={(e) =>
                    setBrowser({ pageTimeout: Number(e.currentTarget.value) || 15 })
                  }
                />
                {unitHint('秒')}
              </>
            )}
            {renderRow(
              '最大并发页数',
              '同时打开的标签页数量上限',
              <>
                <input
                  class="settings-input-sm"
                  type="number"
                  value={browser.maxConcurrentPages}
                  onInput={(e) =>
                    setBrowser({ maxConcurrentPages: Number(e.currentTarget.value) || 5 })
                  }
                />
                {unitHint('页')}
              </>
            )}
            {renderRow(
              'JavaScript 执行',
              '无头模式下是否执行 JS(SPA 必需)',
              renderToggle(browser.jsExecution, () =>
                setBrowser({ jsExecution: !browser.jsExecution })
              )
            )}
            {renderRow(
              '自动滚动',
              '滚动加载懒加载内容',
              renderToggle(browser.autoScroll, () => setBrowser({ autoScroll: !browser.autoScroll }))
            )}
            {renderRow(
              '截图存档',
              '搜索结果页面截图存入 L0 缓存',
              renderToggle(browser.screenshotArchive, () =>
                setBrowser({ screenshotArchive: !browser.screenshotArchive })
              )
            )}
          </div>

          {/* ═══ 关于 ═══ */}
          <div class={`settings-pane${activePane === 'about' ? ' active' : ''}`}>
            <div class="about-hero">
              <div class="about-logo">🌌</div>
              <div class="about-name">Nebula</div>
              <div class="about-version">v2.2.0 · 你的思考,不该成为别人的养料</div>
              <div class="about-stack">
                Tauri + Preact + Rust
                <br />
                ~160K 行代码 · 387 个 .rs 文件
                <br />
                131 个功能任务 · 41 个技术债项
              </div>
              <div class="about-actions">
                <div class="tool-btn">检查更新</div>
                <div class="tool-btn">导出诊断</div>
                <div class="tool-btn">开源许可</div>
              </div>
            </div>

            {/* DID 身份 */}
            <div class="settings-section">
              <div class="settings-section-title">{t('settings.identity')}</div>
              <button
                type="button"
                class="tool-btn"
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
                {t('settings.generateDid')}
              </button>
            </div>

            {/* IM 绑定 — Feishu/WeCom/DingTalk webhook 推送 */}
            <div class="settings-section">
              <ImBindingPanel />
            </div>
          </div>
        </div>
      </div>

      {/* M6 #77: Soul 编辑器 Modal — 从自主性面板「编辑 SOUL.md」按钮触发。 */}
      <SoulEditor open={soulEditorOpen} onClose={() => setSoulEditorOpen(false)} />

      {/* M6 #78: 进化日志 Modal — 从自主性面板「🧬 进化日志」按钮触发。 */}
      <EvolutionLogView open={evolutionLogOpen} onClose={() => setEvolutionLogOpen(false)} />

      {/* M6 #83: WorkType 配置 Modal — 从模型管理面板「⚙ WorkType 配置」按钮触发。 */}
      <WorkTypeConfigView open={workTypeConfigOpen} onClose={() => setWorkTypeConfigOpen(false)} />

      {/* P0-1: 模型配置中心 Modal — 从模型管理面板「模型配置中心」按钮触发。 */}
      <ModelConfigPanel open={modelConfigOpen} onClose={() => setModelConfigOpen(false)} />
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
