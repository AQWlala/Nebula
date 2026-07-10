/**
 * nebula · Nebula 主应用
 *
 * v1.0.1 layout additions:
 *  - P0#06: loadTheme() + applyTheme() at boot, and an effect that
 *    re-applies whenever any of the three theme signals change.
 *  - P0#07: kick off the first Ollama health check, then poll every
 *    30s while the app is alive.
 *  - P0#09 / P0-4: read the onboarding flag exactly once after boot
 *    completes.  We check `localStorage.getItem('nebula-onboarding-completed')`
 *    directly so a re-mount cannot race with the localStorage write.
 *    P0-4 replaces the old 3-step Onboarding with a 4-step
 *    OnboardingWizard (欢迎 / 配置模型 / 选择技能 / 完成)。
 */
import { useEffect, useRef, useState } from 'preact/hooks';
import { lazy, Suspense } from 'preact/compat';
import { signal } from '@preact/signals';
import { CodeMode } from './components/CodeMode';
import { ModeSwitcher } from './components/ModeSwitcher';
import { AutonomySlider } from './components/AutonomySlider';
import { OnboardingWizard, ONBOARDING_STORAGE_KEY } from './components/OnboardingWizard';
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
// OnboardingWizard(首次启动需立即可见,且需读取 localStorage 判定是否渲染)、
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
const appKey = signal(0);


export function App() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // P0-4: hold the onboarding decision in local state so it is
  // computed exactly once after `ready` flips, regardless of any
  // later re-mount of the App tree (e.g. ErrorBoundary
  // onReload → appKey.value++).  The decision is read directly from
  // localStorage key `nebula-onboarding-completed`.
  const [showOnboarding, setShowOnboarding] = useState(false);

  // P1-4: 侧边栏宽度（可拖拽调整 180-320px）+ 折叠状态，均持久化到 localStorage。
  // --sidebar-width CSS 变量驱动 .sidebar 的 width；is-collapsed class 触发 48px 折叠态。
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const saved = Number(localStorage.getItem('nebula-sidebar-width'));
    return saved >= 180 && saved <= 320 ? saved : 240;
  });
  const [isResizing, setIsResizing] = useState(false);
  const [isCollapsed, setCollapsed] = useState(() => {
    return localStorage.getItem('nebula-sidebar-collapsed') === 'true';
  });
  // ref 镜像 sidebarWidth，供 mouseup 回调读取最新值而无需将 sidebarWidth 纳入 effect 依赖。
  const sidebarWidthRef = useRef(sidebarWidth);
  sidebarWidthRef.current = sidebarWidth;

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
        // P0-4: a single, post-boot read of the onboarding flag from
        // localStorage.  If the user has never finished the wizard
        // (key missing or not 'true'), we render the 4-step
        // OnboardingWizard over the whole app.
        try {
          setShowOnboarding(localStorage.getItem(ONBOARDING_STORAGE_KEY) !== 'true');
        } catch {
          // localStorage 不可用时(隐私模式等)默认不显示向导,避免阻塞。
          setShowOnboarding(false);
        }
      },
      (e) => setError(String(e))
    );

    // v2.2: Ollama 健康轮询已移除——后端多 provider 架构不强制本地 Ollama。

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
            nebulaStore.setChatPrefill(t('chat.askFilePrefill', { path }));
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

  // P1-4: 把 sidebarWidth 同步到 --sidebar-width CSS 变量（驱动 .sidebar width）。
  useEffect(() => {
    document.documentElement.style.setProperty('--sidebar-width', `${sidebarWidth}px`);
  }, [sidebarWidth]);

  // P1-4: 拖拽中给 body 添加 is-resizing class（CSS 禁用文本选择 + 统一 col-resize 指针）。
  useEffect(() => {
    if (isResizing) {
      document.body.classList.add('is-resizing');
    } else {
      document.body.classList.remove('is-resizing');
    }
  }, [isResizing]);

  // P1-4: 鼠标按下分隔条后监听 mousemove/mouseup，实时调整侧边栏宽度。
  // 范围 180-320px，松开时持久化到 localStorage。
  useEffect(() => {
    if (!isResizing) return;
    const handleMouseMove = (e: MouseEvent) => {
      const newWidth = Math.min(320, Math.max(180, e.clientX));
      setSidebarWidth(newWidth);
    };
    const handleMouseUp = () => {
      setIsResizing(false);
      try {
        localStorage.setItem('nebula-sidebar-width', String(sidebarWidthRef.current));
      } catch {
        /* localStorage 不可用时忽略 */
      }
    };
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizing]);

  // P1-4: 折叠/展开侧边栏，状态持久化到 localStorage。
  const handleToggleCollapse = () => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem('nebula-sidebar-collapsed', String(next));
      } catch {
        /* localStorage 不可用时忽略 */
      }
      return next;
    });
  };

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
        <OnboardingWizard
          onDone={() => {
            // P0-4: 向导完成(包括"稍后"/"跳过"/"进入 Nebula")后,
            // 标记 localStorage 并隐藏向导。
            try {
              localStorage.setItem(ONBOARDING_STORAGE_KEY, 'true');
            } catch {
              /* localStorage 不可用时静默;下次启动仍会显示向导 */
            }
            setShowOnboarding(false);
          }}
        />
        <Toasts />
      </ErrorBoundary>
    );
  }

  return (
    <ErrorBoundary onReload={() => appKey.value++}>
      <div class="app" key={appKey.value}>
        <Titlebar />
        <div class="main-layout">
        <Sidebar isCollapsed={isCollapsed} onToggleCollapse={handleToggleCollapse} />
        <div
          class={`sidebar-resizer${isResizing ? ' is-active' : ''}`}
          onMouseDown={(e) => {
            e.preventDefault();
            setIsResizing(true);
          }}
        />
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
                        {t('memoryView.timeline')}
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
                {/* v2.2: settings 内联视图路由。Settings 当前为 Modal 形态，
                    后续 agent 会改造 Settings.tsx 为纯内联；此处保留 onClose 以满足类型。 */}
                {(currentMode.value as string) === 'settings' && (
                  <Settings
                    onClose={() => {
                      currentMode.value = 'chat';
                    }}
                  />
                )}
              </>
            )}
          </Suspense>
        </main>
        </div>
        <StatusBar />
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
                // v2.2: 设置改为内联视图，切换到 settings view（View 类型暂未包含 'settings'，用断言）。
                currentMode.value = 'settings' as View;
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

/** v2.2: macOS 风格 Titlebar —— 红绿灯占位 + Spotlight 搜索框 + 浮动窗/悬浮球/设置按钮。
 *  搜索框点击触发 CommandPalette；设置按钮切换到 settings 视图。 */
function Titlebar() {
  return (
    <div class="titlebar">
      {/* 红绿灯占位（仅视觉，窗口控制由 Tauri 处理） */}
      <div class="traffic-lights">
        <div class="traffic-light tl-close" />
        <div class="traffic-light tl-min" />
        <div class="traffic-light tl-max" />
      </div>
      <div class="titlebar-title">{t('app.name')}</div>
      {/* Spotlight 风格搜索框：点击打开命令面板 */}
      <button
        type="button"
        class="titlebar-search"
        onClick={() => {
          paletteOpen.value = true;
        }}
        title="搜索或输入命令"
      >
        <span>🔍</span>
        <span>搜索或输入命令...</span>
        <span style="margin-left:auto;font-size:11px;opacity:0.5;">⌘K</span>
      </button>
      <div class="titlebar-actions">
        <button type="button" class="titlebar-btn" title="浮动窗">🪟</button>
        <button type="button" class="titlebar-btn" title="悬浮球">🌀</button>
        <button
          type="button"
          class="titlebar-btn"
          title={t('nav.settings')}
          onClick={() => {
            currentMode.value = 'settings' as View;
          }}
        >
          ⚙️
        </button>
      </div>
    </div>
  );
}

/** v2.2: macOS 风格分组导航侧边栏 —— 品牌 + 4 组导航(收藏/工作/监控/高级)
 *  + 系统组(设置) + 底部状态区。保留 P1-4 的折叠/展开功能。 */
function Sidebar({
  isCollapsed,
  onToggleCollapse,
}: {
  isCollapsed: boolean;
  onToggleCollapse: () => void;
}) {
  // 导航分组定义（顺序对应设计样稿：收藏 / 工作 / 监控 / 高级）。
  const groups: { label: string; items: { id: View; icon: string; label: string }[] }[] = [
    {
      label: '收藏',
      items: [
        { id: 'chat', icon: '💬', label: t('nav.chat') },
        { id: 'swarm', icon: '🐝', label: t('nav.swarm') },
      ],
    },
    {
      label: '工作',
      items: [
        { id: 'memory', icon: '🧠', label: t('nav.memory') },
        { id: 'code', icon: '💻', label: t('nav.code') },
        { id: 'skills', icon: '🔍', label: t('nav.skills') },
      ],
    },
    {
      label: '监控',
      items: [
        { id: 'dashboard', icon: '📊', label: t('nav.dashboard') },
        { id: 'credits', icon: '💰', label: t('nav.credits') },
        { id: 'diagnostics', icon: '🩺', label: t('nav.diagnostics') },
      ],
    },
    {
      label: '高级',
      items: [
        { id: 'shadow', icon: '🌑', label: t('nav.shadow') },
        { id: 'longtask', icon: '⏳', label: t('nav.longtask') },
      ],
    },
  ];

  // 以字符串读取当前模式，兼容尚未加入 View 联合类型的 'settings' 视图。
  const activeMode = currentMode.value as string;

  return (
    <nav class={`sidebar${isCollapsed ? ' is-collapsed' : ''}`}>
      {/* 品牌区 */}
      <div class="sidebar-brand">
        <span class="sidebar-brand-icon">🌌</span>
        <span class="sidebar-brand-text">{t('app.name')}</span>
        <button
          class="sidebar-collapse-btn"
          onClick={onToggleCollapse}
          title={isCollapsed ? '展开侧边栏' : '折叠侧边栏'}
          aria-label={isCollapsed ? '展开侧边栏' : '折叠侧边栏'}
        >
          {isCollapsed ? '▶' : '◀'}
        </button>
      </div>

      {/* 导航分组 */}
      {groups.map((g) => (
        <div class="nav-group" key={g.label}>
          {!isCollapsed && <div class="nav-group-label">{g.label}</div>}
          {g.items.map((it) => (
            <button
              key={it.id}
              class={`nav-item${activeMode === it.id ? ' active' : ''}`}
              onClick={() => {
                currentMode.value = it.id;
              }}
              title={it.label}
            >
              <span class="nav-item-icon">{it.icon}</span>
              <span class="nav-label">{it.label}</span>
            </button>
          ))}
        </div>
      ))}

      {/* 系统组：设置 */}
      <div class="nav-group">
        {!isCollapsed && <div class="nav-group-label">系统</div>}
        <button
          class={`nav-item${activeMode === 'settings' ? ' active' : ''}`}
          onClick={() => {
            currentMode.value = 'settings' as View;
          }}
          title={t('nav.settings')}
        >
          <span class="nav-item-icon">⚙️</span>
          <span class="nav-label">{t('nav.settings')}</span>
        </button>
      </div>

      {/* 底部状态区：模型状态 + 内存 + 版本 */}
      <div class="sidebar-status">
        <div class="status-row">
          <span class="status-dot ok" />
          <span>模型在线 · deepseek-chat</span>
        </div>
        <div class="status-row">
          <span>内存 247MB</span>
        </div>
        <div class="status-row" style="opacity:0.6;">
          <span>v{nebulaStore.version}</span>
        </div>
      </div>
    </nav>
  );
}

// Re-export for tests
export { buildMemoryItems };
