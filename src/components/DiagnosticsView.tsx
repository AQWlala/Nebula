/**
 * T-E-S-27: 诊断面板 — 实时展示可信诊断事件流。
 *
 * 与 SwarmView 结构类似:面板头部 + 事件列表。
 * 通过 `nebulaAPI.diagnosticsSubscribe` 订阅实时事件,
 * 挂载时同时调用 `diagnosticsSnapshot` 拉取最近事件作为初始状态。
 */
import { useEffect, useRef, useState } from 'preact/hooks';
import { nebulaAPI, type DiagnosticEvent } from '../lib/tauri';
import { toast } from './Toast';
import { t } from '../i18n';

/** 每个事件类型对应的图标 + 标签。 */
const KIND_META: Record<DiagnosticEvent['kind'], { icon: string; label: string }> = {
  l4_deny: { icon: '🚫', label: 'L4 Deny' },
  acl_rejected: { icon: '🔒', label: 'ACL Rejected' },
  injection_guard_hit: { icon: '💉', label: 'Injection Guard' },
  sidecar_crash: { icon: '💥', label: 'Sidecar Crash' },
  tracing_warn: { icon: '⚠️', label: 'Tracing Warn' },
  dropped: { icon: '📉', label: 'Dropped' },
};

/** 错误级别的事件类型集合 — 用于日志颜色编码。 */
const ERROR_KINDS: ReadonlySet<DiagnosticEvent['kind']> = new Set([
  'l4_deny',
  'acl_rejected',
  'injection_guard_hit',
  'sidecar_crash',
]);

/** 警告级别的事件类型集合 — 用于日志颜色编码。 */
const WARN_KINDS: ReadonlySet<DiagnosticEvent['kind']> = new Set(['tracing_warn', 'dropped']);

/** 把事件渲染为单行摘要。 */
function eventSummary(evt: DiagnosticEvent): string {
  switch (evt.kind) {
    case 'l4_deny':
      return `memory=${evt.memory_id} reason=${evt.reason}`;
    case 'acl_rejected':
      return `user=${evt.user} resource=${evt.resource}`;
    case 'injection_guard_hit':
      return `pattern=${evt.pattern} input=${evt.input.slice(0, 60)}`;
    case 'sidecar_crash':
      return `name=${evt.name} exit_code=${evt.exit_code}`;
    case 'tracing_warn':
      return `target=${evt.target} msg=${evt.message.slice(0, 80)}`;
    case 'dropped':
      return `count=${evt.count}`;
    default:
      return '';
  }
}

/** 把 DiagnosticEvent seq 转为可读时间(粗略:seq 越大越新)。 */
function eventTime(evt: DiagnosticEvent): string {
  return `#${evt.seq}`;
}

/** 根据事件类型返回日志级别 CSS 类名。 */
function logLevelClass(kind: DiagnosticEvent['kind']): string {
  if (ERROR_KINDS.has(kind)) return 'log-level-error';
  if (WARN_KINDS.has(kind)) return 'log-level-warn';
  return 'log-level-info';
}

export function DiagnosticsView() {
  const [events, setEvents] = useState<DiagnosticEvent[]>([]);
  const [enabled, setEnabled] = useState<boolean>(true);
  const [capacity, setCapacity] = useState<number>(512);
  const [subscribed, setSubscribed] = useState<boolean>(false);
  const eventsEndRef = useRef<HTMLDivElement | null>(null);

  /** 挂载时:拉取快照 + 启动订阅。 */
  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();

    // 1) 拉取最近事件快照作为初始状态。
    nebulaAPI
      .diagnosticsSnapshot(50)
      .then((snap) => {
        if (ac.signal.aborted) return;
        setEvents(snap.events);
        setEnabled(snap.enabled);
        setCapacity(snap.capacity);
      })
      .catch((e) => {
        toast.error(t('diagnostics.title'), String(e));
      });

    // 2) 启动实时订阅。Promise 在后端通道关闭时 resolve(组件卸载时
    //    Tauri runtime 自动关闭 ipc::Channel,后端 recv 循环退出)。
    nebulaAPI
      .diagnosticsSubscribe((evt) => {
        if (ac.signal.aborted) return;
        setEvents((prev) => {
          // 限制本地缓存 200 条,防止内存膨胀。
          const next = [evt, ...prev];
          return next.length > 200 ? next.slice(0, 200) : next;
        });
      })
      .then(() => {
        if (!ac.signal.aborted) setSubscribed(false);
      })
      .catch(() => {
        if (!ac.signal.aborted) setSubscribed(false);
      });
    setSubscribed(true);

    return () => {
      ac.abort();
    };
  }, []);

  /** 打开日志目录。 */
  async function openLogs() {
    try {
      const path = await nebulaAPI.diagnosticsOpenLogs();
      if (path) {
        // 用 shell plugin 打开目录(若 Tauri runtime 不可用则提示路径)。
        try {
          const { open } = await import('@tauri-apps/plugin-shell');
          await open(path);
        } catch {
          toast.info(t('diagnostics.title'), path);
        }
      } else {
        toast.warning(t('diagnostics.title'), t('diagnostics.logDirNotAvailable'));
      }
    } catch (e) {
      toast.error(t('diagnostics.title'), String(e));
    }
  }

  return (
    <div class="diagnostics-view" style="display:flex;flex-direction:column;height:100%;">
      {/* 页面头:标题 + 工具按钮 */}
      <div class="page-header">
        <div>
          <div class="page-title">🩺 {t('diagnostics.title')}</div>
          <div class="page-subtitle">
            {enabled
              ? t('diagnostics.eventsCount', {
                  count: events.length,
                  capacity,
                  live: subscribed ? ' · ' + t('diagnostics.live') : '',
                })
              : t('diagnostics.disabled')}
          </div>
        </div>
        <div class="page-actions">
          <button class="tool-btn" onClick={openLogs} title={t('diagnostics.openLogs')}>
            📂 {t('diagnostics.openLogs')}
          </button>
          <span
            class="tool-btn"
            style={{
              background: subscribed ? 'rgba(40,200,64,0.2)' : 'rgba(255,255,255,0.06)',
              color: subscribed ? '#28c840' : 'rgba(255,255,255,0.4)',
              cursor: 'default',
            }}
          >
            {subscribed ? t('diagnostics.live') : t('diagnostics.idle')}
          </span>
        </div>
      </div>

      <div class="page-body" style="display:flex;flex-direction:column;gap:20px;">
        {/* 健康检查网格(2 列) */}
        <div class="diag-section">
          <div class="diag-section-title">{t('diagnostics.title')}</div>
          <div class="diag-checks">
            <div class="diag-check">
              <span class="diag-icon">{enabled ? '✅' : '⚠️'}</span>
              <span class="diag-name">{t('diagnostics.title')}</span>
              <span class={`diag-status ${enabled ? 'ok' : 'warn'}`}>
                {enabled ? '正常' : '已禁用'}
              </span>
            </div>
            <div class="diag-check">
              <span class="diag-icon">{subscribed ? '✅' : '⏳'}</span>
              <span class="diag-name">{t('diagnostics.live')}</span>
              <span class={`diag-status ${subscribed ? 'ok' : 'warn'}`}>
                {subscribed ? '实时' : '空闲'}
              </span>
            </div>
            <div class="diag-check">
              <span class="diag-icon">📊</span>
              <span class="diag-name">{t('diagnostics.eventsCount', { count: events.length, capacity, live: '' })}</span>
              <span class="diag-status ok">{events.length}</span>
            </div>
            <div class="diag-check">
              <span class="diag-icon">📦</span>
              <span class="diag-name">Buffer</span>
              <span class="diag-status ok">{capacity}</span>
            </div>
          </div>
        </div>

        {/* 实时日志流 */}
        <div class="diag-section">
          <div class="diag-section-title">{t('diagnostics.live')}</div>
          {events.length === 0 ? (
            <div class="stat-card" style="padding:24px;text-align:center;color:rgba(255,255,255,0.4);">
              {enabled ? t('diagnostics.empty') : t('diagnostics.disabled')}
            </div>
          ) : (
            <div class="diag-log">
              {events.map((evt, i) => {
                const meta = KIND_META[evt.kind] || { icon: '❓', label: evt.kind };
                const levelClass = logLevelClass(evt.kind);
                return (
                  <div class="log-line" key={`${evt.seq}-${i}`}>
                    <span class={levelClass}>[{meta.label}]</span>{' '}
                    {eventTime(evt)} — {eventSummary(evt)}
                  </div>
                );
              })}
              <div ref={eventsEndRef} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
