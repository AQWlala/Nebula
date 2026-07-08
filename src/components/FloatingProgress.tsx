/**
 * T-E-D-07: 浮动进度窗组件。
 *
 * 运行在 360x180 的无边框透明置顶窗口中,显示长任务执行进度:
 *  - 顶部:`data-tauri-drag-region` 拖拽条 + 任务标题
 *  - 中部:进度条(width %)+ 当前 Agent 名称 + completed/total 文本
 *  - 底部:中断按钮(红色)→ 调用 swarmCancel(taskId) → 完成后关闭窗口
 *
 * 事件流:通过 `subscribeEvents` 订阅 SwarmEvent(Channel 模式):
 *  - agent_started → 累计 total,记录当前 agent
 *  - agent_completed → 累计 completed
 *  - swarm_completed → 终态(approved=true→完成 / false→失败),3 秒后自动关闭
 *
 * 从 URL query 解析 taskId 和 title(由 open_floating_progress 命令注入)。
 * 完成态:绿色进度条 + "✓ 完成" / 红色 + "✗ 失败"。
 * 透明背景:覆盖 global.css 的不透明背景,让窗口 transparent:true 生效。
 */
import { useEffect, useState, useCallback, useRef } from 'preact/hooks';
import { subscribeEvents, swarmCancel, type SwarmEvent } from '../lib/tauri';
import { t } from '../i18n';

type ProgressStatus = 'running' | 'completed' | 'failed';

/** 关闭当前浮动进度窗口。Tauri 不可用时回退到 window.close()。 */
async function closeProgressWindow() {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().close();
  } catch {
    window.close();
  }
}

export function FloatingProgress() {
  const [taskId, setTaskId] = useState<string>('');
  const [title, setTitle] = useState<string>(t('floatingProgress.defaultTitle'));
  const [total, setTotal] = useState<number>(0);
  const [completed, setCompleted] = useState<number>(0);
  const [currentAgent, setCurrentAgent] = useState<string>('');
  const [status, setStatus] = useState<ProgressStatus>('running');

  // 用 ref 保存最新 taskId,避免闭包陈旧引用(cancel 回调使用)。
  const taskIdRef = useRef<string>('');
  taskIdRef.current = taskId;
  // 防止终态后重复触发自动关闭。
  const closingRef = useRef<boolean>(false);

  // 解析 URL query:taskId + title。
  useEffect(() => {
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';

    const params = new URLSearchParams(window.location.search);
    const id = params.get('taskId') ?? '';
    const t = params.get('title');
    setTaskId(id);
    taskIdRef.current = id;
    if (t) {
      setTitle(decodeURIComponent(t.replace(/\+/g, ' ')));
    }
  }, []);

  // 订阅 SwarmEvent 流。
  useEffect(() => {
    let unsub: (() => void) | null = null;

    (async () => {
      try {
        unsub = await subscribeEvents((envelope) => {
          handleEvent(envelope.payload);
        });
      } catch {
        /* Tauri 运行时不可用:保持初始状态 */
      }
    })();

    function handleEvent(event: SwarmEvent) {
      // 只处理与当前 taskId 匹配的事件(若有 taskId)。
      const currentId = taskIdRef.current;
      const evtTaskId = 'task_id' in event ? event.task_id : null;
      if (currentId && evtTaskId && evtTaskId !== currentId) return;

      switch (event.kind) {
        case 'agent_started':
          setTotal((n) => n + 1);
          setCurrentAgent(event.agent_kind);
          break;
        case 'agent_completed':
          setCompleted((n) => n + 1);
          break;
        case 'negotiation_started':
          // 进入协商阶段:当前 agent 显示为 "协商中"。
          setCurrentAgent(t('floatingProgress.negotiating'));
          break;
        case 'arbitration_resolved':
          setCurrentAgent(event.chosen_kind);
          break;
        case 'swarm_completed':
          setStatus(event.approved ? 'completed' : 'failed');
          // 终态 3 秒后自动关闭窗口。
          if (!closingRef.current) {
            closingRef.current = true;
            setTimeout(() => {
              void closeProgressWindow();
            }, 3000);
          }
          break;
      }
    }

    return () => {
      if (unsub) unsub();
    };
  }, []);

  /** 中断按钮:调用 swarmCancel → 关闭窗口。 */
  const handleCancel = useCallback(async () => {
    const id = taskIdRef.current;
    if (id) {
      try {
        await swarmCancel(id);
      } catch {
        /* 取消失败也关闭窗口 */
      }
    }
    await closeProgressWindow();
  }, []);

  const pct = total > 0 ? Math.min(100, Math.round((completed / total) * 100)) : 0;
  const statusText =
    status === 'completed'
      ? t('floatingProgress.completed')
      : status === 'failed'
        ? t('floatingProgress.failed')
        : `${completed} / ${total}`;

  return (
    <div class={`floating-progress floating-progress--${status}`}>
      <div class="floating-progress__dragbar" data-tauri-drag-region>
        <span class="floating-progress__title" data-tauri-drag-region>
          {title}
        </span>
      </div>

      <div class="floating-progress__body">
        <div class="floating-progress__bar-track">
          <div
            class={`floating-progress__bar-fill floating-progress__bar-fill--${status}`}
            style={`width: ${pct}%`}
          />
        </div>
        <div class="floating-progress__meta">
          <span class="floating-progress__agent">
            {status === 'running' && currentAgent ? `▶ ${currentAgent}` : statusText}
          </span>
          {status === 'running' && (
            <span class="floating-progress__count">
              {completed} / {total}
            </span>
          )}
        </div>
      </div>

      <div class="floating-progress__footer">
        {status === 'running' ? (
          <button class="floating-progress__cancel" onClick={handleCancel}>
            {t('floatingProgress.cancel')}
          </button>
        ) : (
          <span class={`floating-progress__status floating-progress__status--${status}`}>
            {statusText}
          </span>
        )}
      </div>
    </div>
  );
}
