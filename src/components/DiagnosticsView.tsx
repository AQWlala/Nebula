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

export function DiagnosticsView() {
  const [events, setEvents] = useState<DiagnosticEvent[]>([]);
  const [enabled, setEnabled] = useState<boolean>(true);
  const [capacity, setCapacity] = useState<number>(512);
  const [subscribed, setSubscribed] = useState<boolean>(false);
  const eventsEndRef = useRef<HTMLDivElement | null>(null);

  /** 挂载时:拉取快照 + 启动订阅。 */
  useEffect(() => {
    let cancelled = false;

    // 1) 拉取最近事件快照作为初始状态。
    nebulaAPI.diagnosticsSnapshot(50)
      .then((snap) => {
        if (cancelled) return;
        setEvents(snap.events);
        setEnabled(snap.enabled);
        setCapacity(snap.capacity);
      })
      .catch((e) => {
        toast.error(t('diagnostics.title'), String(e));
      });

    // 2) 启动实时订阅。Promise 在后端通道关闭时 resolve(组件卸载时
    //    Tauri runtime 自动关闭 ipc::Channel,后端 recv 循环退出)。
    nebulaAPI.diagnosticsSubscribe((evt) => {
      if (cancelled) return;
      setEvents((prev) => {
        // 限制本地缓存 200 条,防止内存膨胀。
        const next = [evt, ...prev];
        return next.length > 200 ? next.slice(0, 200) : next;
      });
    })
      .then(() => {
        if (!cancelled) setSubscribed(false);
      })
      .catch(() => {
        if (!cancelled) setSubscribed(false);
      });
    setSubscribed(true);

    return () => {
      cancelled = true;
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
    <div class="panel">
      <div class="panel-header">
        <span class="panel-title">🩺 {t('diagnostics.title')}</span>
        <span style="color: var(--text-muted); font-size: 12px;">
          {enabled
            ? t('diagnostics.eventsCount', {
                count: events.length,
                capacity,
                live: subscribed ? ' · ' + t('diagnostics.live') : '',
              })
            : t('diagnostics.disabled')}
        </span>
      </div>

      <div class="card" style="margin-bottom: 12px; display: flex; gap: 8px; align-items: center;">
        <button
          class="btn"
          onClick={openLogs}
          title={t('diagnostics.openLogs')}
        >
          📂 {t('diagnostics.openLogs')}
        </button>
        <span style="flex: 1;" />
        <span
          class="badge"
          style={{
            background: subscribed ? '#1e5f3a' : '#3a3a5f',
            color: 'var(--text-primary)',
          }}
        >
          {subscribed ? t('diagnostics.live') : t('diagnostics.idle')}
        </span>
      </div>

      {events.length === 0 ? (
        <div class="card" style="padding: 24px; text-align: center; color: var(--text-muted);">
          {enabled ? t('diagnostics.empty') : t('diagnostics.disabled')}
        </div>
      ) : (
        <div class="diagnostics-list" style="display: flex; flex-direction: column; gap: 4px;">
          {events.map((evt, i) => {
            const meta = KIND_META[evt.kind] || { icon: '❓', label: evt.kind };
            return (
              <div
                key={`${evt.seq}-${i}`}
                class="card"
                style={{
                  padding: '8px 12px',
                  display: 'flex',
                  alignItems: 'flex-start',
                  gap: '8px',
                  fontSize: '12px',
                  borderLeft: `3px solid ${evt.kind === 'dropped' ? '#5f1e1e' : 'var(--accent-purple)'}`,
                }}
              >
                <span style="font-size: 16px; flex-shrink: 0;">{meta.icon}</span>
                <div style="flex: 1; min-width: 0;">
                  <div style="display: flex; align-items: center; gap: 6px; margin-bottom: 2px;">
                    <strong style="font-size: 12px;">{meta.label}</strong>
                    <span style="color: var(--text-muted); font-size: 11px;">{eventTime(evt)}</span>
                  </div>
                  <code
                    style={{
                      display: 'block',
                      whiteSpace: 'pre-wrap',
                      wordBreak: 'break-all',
                      fontFamily: 'Menlo, Consolas, monospace',
                      fontSize: '11px',
                      color: 'var(--text-secondary)',
                    }}
                  >
                    {eventSummary(evt)}
                  </code>
                </div>
              </div>
            );
          })}
          <div ref={eventsEndRef} />
        </div>
      )}
    </div>
  );
}
