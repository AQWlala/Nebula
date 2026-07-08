/**
 * T-E-D-03: 桌面悬浮球组件。
 *
 * 运行在 240x240 的无边框透明置顶窗口中(球本身 80x80 居中),作为系统状态指示器:
 *  - 空闲(绿色稳定)/ 思考(橙色脉冲)/ 执行(霓虹橙快速闪烁)/ 通知(红色)
 *  - 整球 data-tauri-drag-region 可拖动
 *  - 点击展开迷你菜单(打开主窗 / 关闭球)
 *  - 通过 `nebula://ball-state` 事件接收状态推送
 *  - T-E-S-57: 通过 `nebula://floating-ball-state` 事件接收 working 状态 + 任务计数
 *  - T-E-D-06: 监听 `nebula://drag-drop` 事件 → 调用 sponge_absorb_file
 *    吸收到记忆系统;absorb 期间临时切到 executing 状态,结束后 toast 反馈。
 *
 * 复用 FloatingChat 的 transparent 背景覆盖模式 (见 FloatingChat.tsx:42-53),
 * 否则圆角外会是黑色方块。
 */
import { useEffect, useState, useCallback, useRef } from 'preact/hooks';
import { nebulaAPI, type BallState } from '../lib/tauri';
import { toast } from './Toast';
import { t } from '../i18n';

/** 浮动球可见尺寸(直径,与 CSS .floating-ball width/height 一致)。 */
const BALL_SIZE = 80;
/** 球在 240x240 透明窗口内的偏移(=(窗口边长-BALL_SIZE)/2),
 *  用于拖拽边界约束:窗口左上角比球左上角偏左/偏上这么多。 */
const BALL_OFFSET = 80;
/** 距屏幕边缘的最小边距,防止球完全贴边看不见。 */
const EDGE_MARGIN = 4;

/** 显示主窗口 (label="main") — 拉起被隐藏到托盘的主窗。 */
async function showMainWindow() {
  try {
    const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const main = await WebviewWindow.getByLabel('main');
    if (main) {
      await main.show();
      await main.setFocus();
    }
  } catch {
    /* Tauri 运行时不可用 (浏览器预览):忽略 */
  }
}

/** 隐藏悬浮球窗口 (与 StatusBar 按钮 toggle 显隐配合)。 */
async function hideBallWindow() {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().hide();
  } catch {
    /* ignore */
  }
}

/** 浮动球状态文本(根据 BallState 查 i18n)。 */
const stateLabel = (s: BallState): string => t(`floatingBall.state.${s}`);

export function FloatingBall() {
  const [state, setState] = useState<BallState>('idle');
  const [taskCount, setTaskCount] = useState<number>(0);
  const [menuOpen, setMenuOpen] = useState(false);
  // T-E-D-06: 保存最新 state 供 drag-drop 异步回调读取,避免 stale closure。
  const stateRef = useRef<BallState>('idle');
  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  useEffect(() => {
    // 覆盖 global.css 中 html/body/#app 的不透明背景,让窗口 transparent:true 生效。
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';

    let unsubOld: (() => void) | null = null;
    let unsubNew: (() => void) | null = null;
    let unsubDrag: (() => void) | null = null;

    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');

        // 旧事件: nebula://ball-state — 兼容 idle/thinking/executing/notification
        unsubOld = await nebulaAPI.subscribeBallState((s) => {
          if (s !== 'working') {
            setState(s);
          }
        });

        // 新事件: nebula://floating-ball-state — T-E-S-57 新增
        unsubNew = await listen<{ state: BallState; task_count?: number }>(
          'nebula://floating-ball-state',
          (event) => {
            if (event.payload?.state) {
              setState(event.payload.state);
              if (event.payload.state === 'working' && event.payload.task_count !== undefined) {
                setTaskCount(event.payload.task_count);
              }
            }
          }
        );

        // T-E-D-06: 悬浮球窗口专用拖拽事件 → sponge_absorb_file。
        // 主窗口的 drag-drop 监听器语义是"打开为代码文件",与悬浮球
        // "吸收到记忆"语义冲突,故后端按 window label 分流到此事件。
        unsubDrag = await listen<string[]>('nebula://ball-drag-drop', async (event) => {
          const paths = event.payload;
          if (!paths || paths.length === 0) return;

          const prevState = stateRef.current;
          setState('executing');
          let ok = 0;
          let fail = 0;
          for (const p of paths) {
            try {
              await nebulaAPI.spongeAbsorbFile(p);
              ok += 1;
            } catch (err) {
              fail += 1;
              const e = err as { message?: string };
              toast.error(t('floatingBall.absorbFailed'), e?.message ?? String(err));
            }
          }
          if (ok > 0) {
            toast.success(
              t('floatingBall.absorbedMemory', { count: ok }),
              fail > 0 ? t('floatingBall.absorbPartial', { ok, fail }) : undefined
            );
          }
          setState(prevState);
        });
      } catch {
        /* Tauri 运行时不可用:保持 idle 默认状态 */
      }
    })();

    return () => {
      if (unsubOld) unsubOld();
      if (unsubNew) unsubNew();
      if (unsubDrag) unsubDrag();
    };
  }, []);

  const toggleMenu = useCallback(() => {
    // 拖拽后不触发菜单切换(区分拖拽和点击)
    if (didDragRef.current) {
      didDragRef.current = false;
      return;
    }
    setMenuOpen((v) => !v);
  }, []);

  // P2 修复:浮动球拖拽边界约束。
  //
  // 原实现用 data-tauri-drag-region 让 Tauri 原生处理窗口拖拽,
  // 但无法限制窗口不被拖出屏幕外。改用 JS 手动实现 mousedown/mousemove/
  // mouseup 拖拽,mousemove 时先做边界 clamp 再调用 Tauri setPosition。
  //
  // 拖拽状态用 ref 保存(不触发 re-render):
  // - draggingRef:是否正在拖拽
  // - dragOffsetRef:鼠标在窗口内的相对偏移(点击时记录)
  const draggingRef = useRef(false);
  const dragOffsetRef = useRef({ x: 0, y: 0 });
  const didDragRef = useRef(false);

  const handleMouseDown = useCallback((e: MouseEvent) => {
    // 只响应左键
    if (e.button !== 0) return;
    draggingRef.current = true;
    didDragRef.current = false;
    // 记录鼠标在窗口内的偏移(相对于窗口左上角)
    dragOffsetRef.current = { x: e.clientX, y: e.clientY };

    const handleMouseMove = async (ev: MouseEvent) => {
      if (!draggingRef.current) return;
      // 移动距离 > 3px 才认为是拖拽(区别于点击)
      const dx = ev.clientX - dragOffsetRef.current.x;
      const dy = ev.clientY - dragOffsetRef.current.y;
      if (Math.abs(dx) > 3 || Math.abs(dy) > 3) {
        didDragRef.current = true;
      }

      try {
        const { getCurrentWindow, LogicalPosition } = await import('@tauri-apps/api/window');
        const { monitorFromPoint } = await import('@tauri-apps/api/window');
        const win = getCurrentWindow();

        // 获取鼠标所在屏幕的尺寸(用于边界约束)
        const pos = await win.outerPosition();
        const screenX = pos.x + ev.clientX;
        const screenY = pos.y + ev.clientY;
        const monitor = await monitorFromPoint(screenX, screenY);

        let newX = screenX - dragOffsetRef.current.x;
        let newY = screenY - dragOffsetRef.current.y;

        if (monitor) {
          // 球居中在 240x240 透明窗口里,窗口左上角比球左上角偏左/上 BALL_OFFSET。
          // 约束窗口位置使「球」(而非整个透明窗口)留在屏幕内。
          const minX = monitor.position.x + EDGE_MARGIN - BALL_OFFSET;
          const maxX =
            monitor.position.x + monitor.size.width - BALL_OFFSET - BALL_SIZE - EDGE_MARGIN;
          const minY = monitor.position.y + EDGE_MARGIN - BALL_OFFSET;
          const maxY =
            monitor.position.y + monitor.size.height - BALL_OFFSET - BALL_SIZE - EDGE_MARGIN;
          newX = Math.max(minX, Math.min(maxX, newX));
          newY = Math.max(minY, Math.min(maxY, newY));
        }

        await win.setPosition(new LogicalPosition(newX, newY));
      } catch {
        /* Tauri 不可用时忽略(浏览器预览) */
      }
    };

    const handleMouseUp = () => {
      draggingRef.current = false;
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);
  }, []);

  const handleOpenMain = useCallback((e: Event) => {
    e.stopPropagation();
    setMenuOpen(false);
    void showMainWindow();
  }, []);

  const handleCloseBall = useCallback((e: Event) => {
    e.stopPropagation();
    setMenuOpen(false);
    void hideBallWindow();
  }, []);

  return (
    <div
      class={`floating-ball floating-ball--${state}`}
      onMouseDown={handleMouseDown}
      onClick={toggleMenu}
      title={stateLabel(state)}
      style={{ cursor: 'grab' }}
    >
      <div class="floating-ball__core">
        <span class="floating-ball__icon">🐍</span>
      </div>
      {state === 'working' && taskCount > 0 && (
        <div class="floating-ball__badge">{taskCount > 99 ? '99+' : taskCount}</div>
      )}
      {menuOpen && (
        <div class="floating-ball__menu" role="menu">
          <button class="floating-ball__menu-item" onClick={handleOpenMain}>
            🪟 {t('floatingBall.openMain')}
          </button>
          <button class="floating-ball__menu-item" onClick={handleCloseBall}>
            ✕ {t('floatingBall.closeBall')}
          </button>
        </div>
      )}
    </div>
  );
}
