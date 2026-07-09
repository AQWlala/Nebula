/**
 * T-E-S-59 统一收件箱视图 — 跨渠道消息列表 + 筛选 + 回复入口。
 *
 * 该组件是纯展示 + 回调组件,不直接调用 Tauri 命令;
 * 数据获取与命令调用由父组件(主 agent 集成时接入 ChatPanel
 * 或新建 InboxPanel)负责。
 *
 * 设计参考:ChatPanel.tsx 的 .msg / .msg-content 样式。
 * 渠道徽标颜色:telegram=蓝、discord=紫、web=绿、其他=灰。
 * 未读消息加粗 + 左侧 4px 色条(对应渠道色)。
 */
import { useMemo, useState } from 'preact/hooks';
import type { UnifiedMessage } from '../../lib/tauri';
import { t } from '../../i18n';

export interface InboxViewProps {
  messages: UnifiedMessage[];
  /** 点击"回复"按钮时触发,参数为原消息 id。 */
  onReply?: (id: string) => void;
  /** 点击"标记已读"按钮时触发,参数为消息 id。 */
  onMarkRead?: (id: string) => void;
}

/** 支持的渠道筛选值。 */
type ChannelFilter = 'all' | 'telegram' | 'discord' | 'web';

/** 渠道 → 显示名 + 主题色映射。
 *  web 包含 webchat / web 两种 source_channel 字符串。 */
const CHANNEL_META: Record<string, { label: string; color: string }> = {
  telegram: { label: 'Telegram', color: '#2AABEE' },
  discord: { label: 'Discord', color: '#5865F2' },
  webchat: { label: 'Web', color: '#22C55E' },
  web: { label: 'Web', color: '#22C55E' },
  jiuwenswarm: { label: 'JiuwenSwarm', color: '#6B7280' },
};

const FALLBACK_META = { label: 'Unknown', color: '#6B7280' };

/** 根据 source_channel 字符串取渠道元信息。 */
function channelMeta(channel: string): { label: string; color: string } {
  return CHANNEL_META[channel.toLowerCase()] ?? FALLBACK_META;
}

/** 将毫秒时间戳格式化为本地时间字符串。 */
function formatTime(ts: number): string {
  if (!ts || ts < 0) return '--';
  try {
    const d = new Date(ts);
    const pad = (n: number) => String(n).padStart(2, '0');
    return (
      `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
      `${pad(d.getHours())}:${pad(d.getMinutes())}`
    );
  } catch {
    return String(ts);
  }
}

/** 判断消息是否属于指定渠道筛选。
 *  'web' 筛选同时匹配 'webchat' 和 'web'。 */
function matchesFilter(msg: UnifiedMessage, filter: ChannelFilter): boolean {
  if (filter === 'all') return true;
  const ch = msg.source_channel.toLowerCase();
  if (filter === 'web') return ch === 'web' || ch === 'webchat';
  return ch === filter;
}

export function InboxView({ messages, onReply, onMarkRead }: InboxViewProps) {
  const [filter, setFilter] = useState<ChannelFilter>('all');

  const filtered = useMemo(
    () => messages.filter((m) => matchesFilter(m, filter)),
    [messages, filter]
  );

  const unreadCount = useMemo(
    () => messages.filter((m) => m.inbound && !m.read).length,
    [messages]
  );

  return (
    <div class="panel inbox-panel" style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
      <div
        class="panel-header"
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: '12px',
        }}
      >
        <span class="panel-title">📥 收件箱</span>
        <span style={{ color: 'var(--text-muted)', fontSize: '12px' }}>
          {filtered.length} / {messages.length} 条
          {unreadCount > 0 && (
            <span style={{ marginLeft: '8px', color: 'var(--accent)', fontWeight: 600 }}>
              {unreadCount} 未读
            </span>
          )}
        </span>
      </div>

      {/* 渠道筛选下拉 */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
        <label
          htmlFor="inbox-channel-filter"
          style={{ fontSize: '12px', color: 'var(--text-muted)' }}
        >
          渠道
        </label>
        <select
          id="inbox-channel-filter"
          value={filter}
          onChange={(e) => setFilter((e.target as HTMLSelectElement).value as ChannelFilter)}
          style={{
            padding: '4px 8px',
            borderRadius: '6px',
            border: '1px solid var(--border)',
            background: 'var(--bg-secondary)',
            color: 'var(--text)',
            fontSize: '13px',
          }}
        >
          <option value="all">{t('inbox.filterAll')}</option>
          <option value="telegram">Telegram</option>
          <option value="discord">Discord</option>
          <option value="web">Web</option>
        </select>
      </div>

      {/* 消息列表 */}
      <div class="chat-messages" style={{ flexDirection: 'column', gap: '6px' }}>
        {filtered.length === 0 && (
          <div style={{ textAlign: 'center', color: 'var(--text-muted)', padding: '32px' }}>
            <div style={{ fontSize: '36px', marginBottom: '8px' }}>📭</div>
            <div>{t('inbox.empty')}</div>
          </div>
        )}
        {filtered.map((m) => {
          const meta = channelMeta(m.source_channel);
          const isUnread = m.inbound && !m.read;
          return (
            <div
              key={m.id}
              class="msg"
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: '4px',
                width: '100%',
                maxWidth: '100%',
                borderLeft: isUnread ? `4px solid ${meta.color}` : '4px solid transparent',
                fontWeight: isUnread ? 600 : 400,
                opacity: m.inbound ? 1 : 0.7,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' }}>
                {/* 渠道徽标 */}
                <span
                  style={{
                    display: 'inline-block',
                    padding: '1px 8px',
                    borderRadius: '10px',
                    fontSize: '11px',
                    color: '#fff',
                    background: meta.color,
                    fontWeight: 600,
                    lineHeight: '1.4',
                  }}
                >
                  {meta.label}
                </span>
                {/* 发送者 */}
                <span class="msg-role" style={{ margin: 0, fontWeight: 500 }}>
                  {m.sender || '(unknown)'}
                </span>
                {/* 时间 */}
                <span style={{ fontSize: '11px', color: 'var(--text-muted)', marginLeft: 'auto' }}>
                  {formatTime(m.timestamp_ms)}
                </span>
                {/* 入站/出站标识 */}
                {!m.inbound && (
                  <span
                    style={{ fontSize: '10px', color: 'var(--text-muted)', fontStyle: 'italic' }}
                  >
                    出站
                  </span>
                )}
              </div>
              <div class="msg-content">{m.content}</div>
              {/* 操作按钮 */}
              {(onReply || onMarkRead) && m.inbound && (
                <div style={{ display: 'flex', gap: '8px', marginTop: '4px' }}>
                  {onReply && (
                    <button
                      class="btn btn-secondary"
                      style={{ fontSize: '11px', padding: '2px 8px' }}
                      onClick={() => onReply(m.id)}
                    >
                      ↩ 回复
                    </button>
                  )}
                  {onMarkRead && isUnread && (
                    <button
                      class="btn btn-secondary"
                      style={{ fontSize: '11px', padding: '2px 8px' }}
                      onClick={() => onMarkRead(m.id)}
                    >
                      ✓ 标记已读
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
