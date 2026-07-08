/**
 * nebula · Nebula 主应用
 *
 * v1.0.1 layout additions:
 *  - P0#06: loadTheme() + applyTheme() at boot, and an effect that
 *    re-applies whenever any of the three theme signals change.
 *  - P0#07: kick off the first Ollama health check, then poll every
 *    30s while the app is alive.
 *  - P0#09: read the onboarding flag exactly once after boot
 *    completes.  `shouldShowOnboarding()` now returns synchronously
 *    from a signal, so a re-mount cannot race with the localStorage
 *    write.
 */
import { useEffect, useState } from 'preact/hooks';
import { lazy, Suspense } from 'preact/compat';
import { signal } from '@preact/signals';
import { CodeMode } from './components/CodeMode';
import { ModeSwitcher } from './components/ModeSwitcher';
import { AutonomySlider } from './components/AutonomySlider';
import { Onboarding, shouldShowOnboarding } from './components/Onboarding';
import { StatusBar } from './components/StatusBar';
import { ErrorBoundary } from './components/ErrorBoundary';
import {
  CommandPalette,
  buildDefaultCommands,
  buildMemoryItems,
  useCommandPaletteShortcut,
} from './components/CommandPalette';
import { Toasts, toast } from './components/Toast';
import { nebulaStore, type View } from './stores/nebulaStore';
import { t, currentLocale } from './i18n';
import { loadTheme, applyTheme } from './theme';
// T-E-A-11: Smart Prefetch — 打开文件时预取历史对话预热 SemanticCache。
import { invokeTauri } from './lib/tauri';

// T-S5-B-03: 代码分割懒加载 — 将大组件拆分为独立 chunk,
// 仅在用户切换到对应视图时按需加载。
// 保留 eager 的组件: CodeMode(默认视图,避免首屏闪烁)、
// ModeSwitcher/StatusBar/ErrorBoundary/Toasts(常驻)、
// Onboarding(模块因 shouldShowOnboarding 已加载)、
// CommandPalette(工具函数需 eager)。
const ChatPanel = lazy(() =>
  import('./components/ChatPanel').then((m) => ({ default: m.ChatPanel }))
);
const SwarmView = lazy(() =>
  import('./components/SwarmView').then((m) => ({ default: m.SwarmView }))
);
const MemoryInspector = lazy(() =>
  import('./components/MemoryInspector').then((m) => ({ default: m.MemoryInspector }))
);
const MemoryMap = lazy(() =>
  import('./components/MemoryMap').then((m) => ({ default: m.MemoryMap }))
);
const TimelineView = lazy(() =>
  import('./components/TimelineView').then((m) => ({ default: m.TimelineView }))
);
const SkillPanel = lazy(() => import('./components/SkillPanel'));
const Dashboard = lazy(() =>
  import('./components/Dashboard').then((m) => ({ default: m.Dashboard }))
);
const CreditsDashboard = lazy(() =>
  import('./components/CreditsDashboard').then((m) => ({ default: m.CreditsDashboard }))
);
const Settings = lazy(() => import('./components/Settings').then((m) => ({ default: m.Settings })));
const WritingMode = lazy(() =>
  import('./components/WritingMode').then((m) => ({ default: m.WritingMode }))
);
const WorkMode = lazy(() => import('./components/WorkMode').then((m) => ({ default: m.WorkMode })));
// T-E-S-27: Trusted Diagnostics Channels 前端面板。
const DiagnosticsView = lazy(() =>
  import('./components/DiagnosticsView').then((m) => ({ default: m.DiagnosticsView }))
);
// T-E-C-08: Shadow Workspace 隔离执行环境面板。
const ShadowWorkspacePanel = lazy(() =>
  import('./components/ShadowWorkspacePanel').then((m) => ({ default: m.ShadowWorkspacePanel }))
);
// T-E-C-10: 异步长任务面板。
const LongTaskPanel = lazy(() =>
  import('./components/LongTaskPanel').then((m) => ({ default: m.LongTaskPanel }))
);

/** T-S5-B-03: 懒加载 chunk 下载期间的统一 fallback。 */
function LoadingFallback() {
  return (
    <div
      class="lazy-loading-fallback"
      style="display:flex;align-items:center;justify-content:center;height:100%;color:var(--text-muted);font-size:14px;"
    >
      <span class="lazy-spinner" style="margin-right:8px;">
        ⏳
      </span>
      {t('app.loading')}
    </div>
  );
}

// View 类型从 nebulaStore 导入(T-E-B-02 重构后统一来源)。

// 全局状态：当前模式 + 当前 view
// T-E-B-02: currentMode 移至 nebulaStore 以便 ChatPanel `/journey` 跨组件切换。
const currentMode = nebulaStore.currentMode;
const paletteOpen = signal(false);
const settingsOpen = signal(false);
const appKey = signal(0);

const OLLAMA_POLL_MS = 30_000;

export function App() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // P0#09: hold the onboarding decision in a ref-ish local state
  // so it is computed exactly once after `ready` flips, regardless
  // of any later re-mount of the App tree (e.g. ErrorBoundary
  // onReload → appKey.value++).  Reading the signal is cheap and
  // always returns the same value because the signal only flips
  // when the user explicitly finishes the onboarding.
  const [showOnboarding, setShowOnboarding] = useState(false);

  // P1-6 / T-E-B-02: memory view mode — 'map' / 'list' / 'timeline' 三视图。
  // 改用 nebulaStore.memoryView signal,以便 ChatPanel `/journey` 命令跨组件切换。
  const memoryView = nebulaStore.memoryView;

  // P0#3: read the locale signal once here so the entire App tree
  // re-renders whenever the user changes language.  Every
  // descendant calls `t(...)` which itself reads the signal, so
  // they are individually subscribed too — this top-level read is
  // just belt-and-suspenders.
  void currentLocale.value;

  // 启动时检查后端
  useEffect(() => {
    // P0#06: hydrate the theme signals from localStorage and apply
    // them to the document *before* the rest of the tree renders.
    // This avoids a flash of the default theme on every reload.
    loadTheme();
    applyTheme();

    nebulaStore.bootstrap().then(
      () => {
        setReady(true);
        // P0#09: a single, post-boot read of the onboarding flag.
        // The signal is hydrated synchronously at module load, so
        // this is no longer racing with the async backend bootstrap.
        setShowOnboarding(shouldShowOnboarding());
      },
      (e) => setError(String(e))
    );

    // P0#07: poll the Ollama health endpoint every 30s so the
    // banner appears / disappears live as the user starts / stops
    // the daemon.  Cleanup on unmount.
    const poll = window.setInterval(() => {
      nebulaStore.checkOllama();
    }, OLLAMA_POLL_MS);
    return () => window.clearInterval(poll);
  }, []);

  // v1.7: 监听全局快捷键触发的 view 切换事件（由 Rust 端 emit）。
  useEffect(() => {
    let unlistens: (() => void)[] = [];
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        // view 切换
        const u1 = await listen<string>('nebula://switch-view', (event) => {
          const view = event.payload;
          if (
            view === 'memory' ||
            view === 'swarm' ||
            view === 'chat' ||
            view === 'code' ||
            view === 'skills' ||
            view === 'dashboard' ||
            view === 'shadow' ||
            view === 'longtask'
          ) {
            currentMode.value = view;
          }
        });
        unlistens.push(u1);

        // 文件打开（双击 .md/.txt 等）→ 切到 code 视图并通知 nebulaStore
        const u2 = await listen<string>('nebula://open-file', (event) => {
          const path = event.payload;
          if (path) {
            currentMode.value = 'code';
            nebulaStore.openExternalFile(path);
            // T-E-A-11: 后台预取该文件相关的历史对话到 SemanticCache。
            // fire-and-forget,失败静默(invokeTauri 内部 catch)。
            invokeTauri('prefetch_for_file', { path });
          }
        });
        unlistens.push(u2);

        // 文件拖入窗口
        const u3 = await listen<string[]>('nebula://drag-drop', (event) => {
          const paths = event.payload;
          if (paths && paths.length > 0) {
            currentMode.value = 'code';
            nebulaStore.openExternalFile(paths[0]);
            // T-E-A-11: 拖入文件同样触发预取。
            invokeTauri('prefetch_for_file', { path: paths[0] });
          }
        });
        unlistens.push(u3);

        // T-E-D-06: 右键"问Nebula" → 切到 chat + 预填输入框。
        const u4 = await listen<string>('nebula://ask-file', (event) => {
          const path = event.payload;
          if (path) {
            currentMode.value = 'chat';
            nebulaStore.setChatPrefill(`请帮我分析这个文件:${path}`);
          }
        });
        unlistens.push(u4);
      } catch {
        // Tauri runtime not available; ignore.
      }
    })();
    return () => {
      unlistens.forEach((u) => u());
    };
  }, []);

  useCommandPaletteShortcut(() => {
    paletteOpen.value = true;
  });

  if (error) {
    return (
      <div class="app-error">
        <h1>{t('app.error')}</h1>
        <pre>{error}</pre>
        <button onClick={() => location.reload()}>{t('app.retry')}</button>
      </div>
    );
  }

  if (!ready) {
    return (
      <div class="app-loading">
        <div class="logo">🐍</div>
        <div class="title">{t('app.name')}</div>
        <div class="subtitle">{t('app.tagline')}</div>
        <div class="spinner">{t('app.loading')}</div>
      </div>
    );
  }

  if (showOnboarding) {
    return (
      <ErrorBoundary>
        <Onboarding onDone={() => setShowOnboarding(false)} />
        <Toasts />
      </ErrorBoundary>
    );
  }

  return (
    <ErrorBoundary onReload={() => appKey.value++}>
      <div class="app" key={appKey.value}>
        <Sidebar />
        <main class="main">
          <Suspense fallback={<LoadingFallback />}>
            {currentMode.value === 'code' ? (
              <Workspace />
            ) : (
              <>
                {currentMode.value === 'chat' && <ChatPanel />}
                {currentMode.value === 'swarm' && <SwarmView />}
                {currentMode.value === 'memory' && (
                  <div className="memory-view-container h-full flex flex-col">
                    {/* P1-6 / T-E-B-02: 三视图切换 — 图谱 / Markdown列表 / 时间轴 */}
                    <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-800">
                      <button
                        className={`px-3 py-1 text-xs rounded ${memoryView.value === 'map' ? 'bg-blue-600 text-white' : 'bg-gray-800 text-gray-400'}`}
                        onClick={() => (memoryView.value = 'map')}
                      >
                        {t('memoryMap.title')}
                      </button>
                      <button
                        className={`px-3 py-1 text-xs rounded ${memoryView.value === 'list' ? 'bg-blue-600 text-white' : 'bg-gray-800 text-gray-400'}`}
                        onClick={() => (memoryView.value = 'list')}
                      >
                        {t('memoryView.list')}
                      </button>
                      <button
                        data-testid="view-timeline-btn"
                        className={`px-3 py-1 text-xs rounded ${memoryView.value === 'timeline' ? 'bg-blue-600 text-white' : 'bg-gray-800 text-gray-400'}`}
                        onClick={() => (memoryView.value = 'timeline')}
                      >
                        时间轴
                      </button>
                    </div>
                    {memoryView.value === 'map' && <MemoryMap />}
                    {memoryView.value === 'list' && <MemoryInspector />}
                    {memoryView.value === 'timeline' && <TimelineView />}
                  </div>
                )}
                {currentMode.value === 'skills' && <SkillPanel />}
                {currentMode.value === 'dashboard' && <Dashboard />}
                {currentMode.value === 'credits' && <CreditsDashboard />}
                {currentMode.value === 'diagnostics' && <DiagnosticsView />}
                {currentMode.value === 'shadow' && <ShadowWorkspacePanel />}
                {currentMode.value === 'longtask' && <LongTaskPanel />}
              </>
            )}
          </Suspense>
        </main>
        <StatusBar />
        <Suspense fallback={<LoadingFallback />}>
          {settingsOpen.value && (
            <Settings
              onClose={() => {
                settingsOpen.value = false;
                toast.success(t('settings.saved'));
              }}
            />
          )}
        </Suspense>
        <CommandPalette
          open={paletteOpen.value}
          onClose={() => {
            paletteOpen.value = false;
          }}
          commands={buildDefaultCommands(
            () => {
              paletteOpen.value = false;
            },
            {
              setMode: (m) => {
                currentMode.value = m;
              },
              setSubMode: (m) => {
                nebulaStore.mode.value = m;
              },
              openSettings: () => {
                settingsOpen.value = true;
              },
              triggerReflection: () => {
                nebulaStore.triggerReflection().then(
                  () => toast.success('Reflection complete'),
                  (e) => toast.error('Reflection failed', String(e))
                );
              },
            }
          )}
          extraItems={[]}
        />
        <Toasts />
      </div>
    </ErrorBoundary>
  );
}

/** v0.5: Code 视图内挂载 ModeSwitcher + 三模式视图。
 *  v1.7: 重命名为 Workspace，语义为"统一工作台的三视角"。
 *  T-S5-B-03: WritingMode/WorkMode 懒加载,CodeMode 保持 eager(默认子视图)。 */
function Workspace() {
  const mode = nebulaStore.mode.value;
  return (
    <div class="code-router">
      <ModeSwitcher />
      {/* T-E-S-50: 自主度滑块 L0-L5,与 ModeSwitcher(任务领域)正交。 */}
      <AutonomySlider />
      <div class="code-router-body">
        <Suspense fallback={<LoadingFallback />}>
          {mode === 'writing' && <WritingMode />}
          {mode === 'work' && <WorkMode />}
          {mode === 'code' && <CodeMode />}
        </Suspense>
      </div>
    </div>
  );
}

function Sidebar() {
  const items: { id: View; icon: string; label: string }[] = [
    { id: 'chat', icon: '💬', label: t('nav.chat') },
    { id: 'swarm', icon: '🐝', label: t('nav.swarm') },
    { id: 'memory', icon: '🧠', label: t('nav.memory') },
    { id: 'code', icon: '💻', label: t('nav.code') },
    { id: 'skills', icon: '🔍', label: t('nav.skills') },
    { id: 'dashboard', icon: '📊', label: t('nav.dashboard') },
    { id: 'credits', icon: '💰', label: 'Credits' },
    { id: 'diagnostics', icon: '🩺', label: t('nav.diagnostics') },
    { id: 'shadow', icon: '🌑', label: 'Shadow' },
    { id: 'longtask', icon: '⏳', label: '长任务' },
  ];

  return (
    <nav class="sidebar">
      <div class="brand">
        <span class="brand-icon">🐍</span>
        <span class="brand-text">{t('app.name')}</span>
      </div>
      {items.map((it) => (
        <button
          key={it.id}
          class={`nav-item ${currentMode.value === it.id ? 'active' : ''}`}
          onClick={() => (currentMode.value = it.id)}
        >
          <span class="nav-icon">{it.icon}</span>
          <span class="nav-label">{it.label}</span>
        </button>
      ))}
      <div class="sidebar-footer">
        <button
          class="nav-item settings-btn"
          onClick={() => {
            settingsOpen.value = true;
          }}
          title={t('nav.settings')}
        >
          <span class="nav-icon">⚙️</span>
          <span class="nav-label">{t('nav.settings')}</span>
        </button>
        <span class="version">v{nebulaStore.version}</span>
        <span class="slogan">{t('app.slogan')}</span>
      </div>
    </nav>
  );
}

// Re-export for tests
export { buildMemoryItems };
