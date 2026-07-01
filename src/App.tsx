/**
 * nine-snake · 九头蛇 主应用
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
import { signal } from '@preact/signals';
import { ChatPanel } from './components/ChatPanel';
import { SwarmView } from './components/SwarmView';
import { MemoryInspector } from './components/MemoryInspector';
import { MemoryMap } from './components/MemoryMap';
import { CodeMode } from './components/CodeMode';
import SkillPanel from './components/SkillPanel';
import { WritingMode } from './components/WritingMode';
import { WorkMode } from './components/WorkMode';
import { ModeSwitcher } from './components/ModeSwitcher';
import { Settings } from './components/Settings';
import { Onboarding, shouldShowOnboarding } from './components/Onboarding';
import { StatusBar } from './components/StatusBar';
import { ErrorBoundary } from './components/ErrorBoundary';
import { CommandPalette, buildDefaultCommands, buildMemoryItems, useCommandPaletteShortcut } from './components/CommandPalette';
import { Toasts, toast } from './components/Toast';
import { Dashboard } from './components/Dashboard';
import { NineSnakeStore } from './stores/nineSnakeStore';
import { t, currentLocale } from './i18n';
import { loadTheme, applyTheme } from './theme';

type View = 'chat' | 'swarm' | 'memory' | 'code' | 'skills' | 'dashboard';

// 全局状态：当前模式 + 当前 view
const currentMode = signal<View>('code');
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

  // P1-6: memory view mode - 'list' or 'map'
  const [memoryView, setMemoryView] = useState<'list' | 'map'>('map');

  // P0#3: read the locale signal once here so the entire App tree
  // re-renders whenever the user changes language.  Every
  // descendant calls `t(...)` which itself reads the signal, so
  // they are individually subscribed too — this top-level read is
  // just belt-and-suspenders.
  const _localeTick = currentLocale.value;

  // 启动时检查后端
  useEffect(() => {
    // P0#06: hydrate the theme signals from localStorage and apply
    // them to the document *before* the rest of the tree renders.
    // This avoids a flash of the default theme on every reload.
    loadTheme();
    applyTheme();

    NineSnakeStore.bootstrap().then(
      () => {
        setReady(true);
        // P0#09: a single, post-boot read of the onboarding flag.
        // The signal is hydrated synchronously at module load, so
        // this is no longer racing with the async backend bootstrap.
        setShowOnboarding(shouldShowOnboarding());
      },
      (e) => setError(String(e)),
    );

    // P0#07: poll the Ollama health endpoint every 30s so the
    // banner appears / disappears live as the user starts / stops
    // the daemon.  Cleanup on unmount.
    const poll = window.setInterval(() => {
      NineSnakeStore.checkOllama();
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
        const u1 = await listen<string>('nine-snake://switch-view', (event) => {
          const view = event.payload;
          if (view === 'memory' || view === 'swarm' || view === 'chat' || view === 'code' || view === 'skills' || view === 'dashboard') {
            currentMode.value = view;
          }
        });
        unlistens.push(u1);

        // 文件打开（双击 .md/.txt 等）→ 切到 code 视图并通知 NineSnakeStore
        const u2 = await listen<string>('nine-snake://open-file', (event) => {
          const path = event.payload;
          if (path) {
            currentMode.value = 'code';
            NineSnakeStore.openExternalFile(path);
          }
        });
        unlistens.push(u2);

        // 文件拖入窗口
        const u3 = await listen<string[]>('nine-snake://drag-drop', (event) => {
          const paths = event.payload;
          if (paths && paths.length > 0) {
            currentMode.value = 'code';
            NineSnakeStore.openExternalFile(paths[0]);
          }
        });
        unlistens.push(u3);
      } catch {
        // Tauri runtime not available; ignore.
      }
    })();
    return () => {
      unlistens.forEach((u) => u());
    };
  }, []);

  useCommandPaletteShortcut(() => { paletteOpen.value = true; });

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
          {currentMode.value === 'code' ? (
            <Workspace />
          ) : (
            <>
              {currentMode.value === 'chat' && <ChatPanel />}
              {currentMode.value === 'swarm' && <SwarmView />}
              {currentMode.value === 'memory' && (
                <div className="memory-view-container h-full flex flex-col">
                  {/* P1-6: View mode toggle */}
                  <div className="flex items-center gap-2 px-4 py-2 border-b border-gray-800">
                    <button
                      className={`px-3 py-1 text-xs rounded ${memoryView === 'map' ? 'bg-blue-600 text-white' : 'bg-gray-800 text-gray-400'}`}
                      onClick={() => setMemoryView('map')}
                    >
                      {t('memoryMap.title')}
                    </button>
                    <button
                      className={`px-3 py-1 text-xs rounded ${memoryView === 'list' ? 'bg-blue-600 text-white' : 'bg-gray-800 text-gray-400'}`}
                      onClick={() => setMemoryView('list')}
                    >
                      {t('memoryView.list')}
                    </button>
                  </div>
                  {memoryView === 'map' ? <MemoryMap /> : <MemoryInspector />}
                </div>
              )}
              {currentMode.value === 'skills' && <SkillPanel />}
              {currentMode.value === 'dashboard' && <Dashboard />}
            </>
          )}
        </main>
        <StatusBar />
        {settingsOpen.value && (
          <Settings onClose={() => { settingsOpen.value = false; toast.success(t('settings.saved')); }} />
        )}
        <CommandPalette
          open={paletteOpen.value}
          onClose={() => { paletteOpen.value = false; }}
          commands={buildDefaultCommands(
            () => { paletteOpen.value = false; },
            {
              setMode: (m) => { currentMode.value = m; },
              setSubMode: (m) => { NineSnakeStore.mode.value = m; },
              openSettings: () => { settingsOpen.value = true; },
              triggerReflection: () => {
                NineSnakeStore.triggerReflection().then(
                  () => toast.success('Reflection complete'),
                  (e) => toast.error('Reflection failed', String(e)),
                );
              },
            },
          )}
          extraItems={[]}
        />
        <Toasts />
      </div>
    </ErrorBoundary>
  );
}

/** v0.5: Code 视图内挂载 ModeSwitcher + 三模式视图。
 *  v1.7: 重命名为 Workspace，语义为"统一工作台的三视角"。 */
function Workspace() {
  const mode = NineSnakeStore.mode.value;
  return (
    <div class="code-router">
      <ModeSwitcher />
      <div class="code-router-body">
        {mode === 'writing' && <WritingMode />}
        {mode === 'work' && <WorkMode />}
        {mode === 'code' && <CodeMode />}
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
          onClick={() => { settingsOpen.value = true; }}
          title={t('nav.settings')}
        >
          <span class="nav-icon">⚙️</span>
          <span class="nav-label">{t('nav.settings')}</span>
        </button>
        <span class="version">v{NineSnakeStore.version}</span>
        <span class="slogan">{t('app.slogan')}</span>
      </div>
    </nav>
  );
}

// Re-export for tests
export { buildMemoryItems };
