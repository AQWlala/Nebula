import { render } from 'preact';
import { lazy, Suspense } from 'preact/compat';
import { App } from './App';
import './styles/global.css';

// T-E-D-01: 顶层视图 lazy 化 — 浮动聊天/悬浮球/进度窗各自独立 chunk,
// 主入口只加载 App。每个视图通过 URL ?view=... 路由分发,首次访问时
// 才下载对应 chunk(Suspense fallback 期间显示 Loading)。
//
// 注意:Preact 用 `preact/compat` 提供 lazy + Suspense(不是 react)。
// 组件用 named export,需 `.then((m) => ({ default: m.X }))` 转换。
const FloatingChat = lazy(() =>
  import('./components/FloatingChat').then((m) => ({ default: m.FloatingChat }))
);
const FloatingBall = lazy(() =>
  import('./components/FloatingBall').then((m) => ({ default: m.FloatingBall }))
);
const FloatingProgress = lazy(() =>
  import('./components/FloatingProgress').then((m) => ({ default: m.FloatingProgress }))
);

/** T-E-D-01: lazy chunk 下载期间的统一 fallback。 */
function Loading() {
  return (
    <div
      class="lazy-loading-fallback"
      style="display:flex;align-items:center;justify-content:center;height:100%;color:var(--text-muted);font-size:14px;"
    >
      <span class="lazy-spinner" style="margin-right:8px;">
        ⏳
      </span>
      Loading...
    </div>
  );
}

const root = document.getElementById('app');
if (root) {
  // T-S5-B-01: 多入口路由 — 通过 URL 查询参数 ?view=floating
  // 决定渲染主应用还是浮动聊天窗口。浮动窗由 Rust 端
  // WebviewWindowBuilder 运行时创建,加载同一前端 dist/devUrl,
  // 仅通过 query 参数区分视图。
  // T-E-D-03: ?view=ball 渲染桌面悬浮球。
  // T-E-D-07: ?view=progress 渲染浮动进度窗。
  // T-E-D-01: 4 个顶层视图改 lazy + Suspense(原 eager import 已移除)。
  const params = new URLSearchParams(window.location.search);
  const view = params.get('view');
  if (view === 'floating') {
    render(
      <Suspense fallback={<Loading />}>
        <FloatingChat />
      </Suspense>,
      root
    );
  } else if (view === 'ball') {
    render(
      <Suspense fallback={<Loading />}>
        <FloatingBall />
      </Suspense>,
      root
    );
  } else if (view === 'progress') {
    render(
      <Suspense fallback={<Loading />}>
        <FloatingProgress />
      </Suspense>,
      root
    );
  } else {
    render(<App />, root);
  }
}
