/**
 * T-E-S-26: EventStreamViewer — 实时事件流查看器
 *
 * 功能:
 * - 时间线列表(按 timestamp 降序)
 * - event_type 多选过滤(checkbox)
 * - 展开详情(payload JSON + trace_id)
 * - 自动滚动到底部(可暂停)
 * - 虚拟化列表(只渲染可见项 + 缓冲，支持 1000+ 事件)
 */
import { useState, useEffect, useRef, useCallback, useMemo } from 'preact/hooks';
import { subscribeEvents, type EventEnvelope, type SwarmEvent } from '../lib/tauri';
import { t } from '../i18n';

/** 所有已知事件类型 */
const ALL_EVENT_TYPES = [
  'AgentStarted',
  'AgentCompleted',
  'NegotiationStarted',
  'ArbitrationResolved',
  'AgentToolCall',
  'AgentOutputChunk',
  'SwarmCompleted',
  'DeadlockDetected',
  'TreeOfThoughtsStarted',
  'PathCompleted',
];

/** 事件类型 → 图标映射 */
const EVENT_ICONS: Record<string, string> = {
  AgentStarted: '🚀',
  AgentCompleted: '✅',
  NegotiationStarted: '🤝',
  ArbitrationResolved: '⚖️',
  AgentToolCall: '🔧',
  AgentOutputChunk: '📝',
  SwarmCompleted: '🏁',
  DeadlockDetected: '💀',
  TreeOfThoughtsStarted: '🌳',
  PathCompleted: '🌿',
};

/** 事件类型 → 颜色映射 */
const EVENT_COLORS: Record<string, string> = {
  AgentStarted: '#4a9eff',
  AgentCompleted: '#4caf50',
  NegotiationStarted: '#ff9800',
  ArbitrationResolved: '#9c27b0',
  AgentToolCall: '#00bcd4',
  AgentOutputChunk: '#795548',
  SwarmCompleted: '#8bc34a',
  DeadlockDetected: '#f44336',
  TreeOfThoughtsStarted: '#3f51b5',
  PathCompleted: '#009688',
};

function formatTime(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function formatMs(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3 });
}

/** 提取事件摘要 */
function eventSummary(event: SwarmEvent): string {
  switch (event.kind) {
    case 'agent_started':
      return `${event.agent_kind} started (${event.task_id})`;
    case 'agent_completed':
      return `${event.agent_kind} ${event.success ? 'ok' : 'FAIL'} (${event.task_id})`;
    case 'negotiation_started':
      return `${event.candidate_count} candidates (${event.task_id})`;
    case 'arbitration_resolved':
      return `${event.chosen_kind} wins (${event.task_id})`;
    case 'agent_tool_call':
      return `${event.tool_name} ${event.success ? 'ok' : 'FAIL'} (${event.agent_id})`;
    case 'agent_output_chunk':
      return `chunk: ${event.delta.slice(0, 30)}${event.delta.length > 30 ? '…' : ''}`;
    case 'swarm_completed':
      return `${event.success_count}ok/${event.failure_count}fail (${event.task_id})`;
    case 'deadlock_detected':
      return `cycle: ${event.cycle.join(' → ')}`;
    case 'tree_of_thoughts_started':
      return `${event.branches} branches (${event.task_id})`;
    case 'path_completed':
      return `${event.strategy} (${event.path_id})`;
    default:
      return '';
  }
}

const MAX_EVENTS = 500;

/** 虚拟化参数 */
const COLLAPSED_HEIGHT_ESTIMATE = 32;
const EXPANDED_HEIGHT_ESTIMATE = 350;
const VIRTUAL_BUFFER = 5;

export function EventStreamViewer() {
  const [events, setEvents] = useState<EventEnvelope[]>([]);
  const [selectedTypes, setSelectedTypes] = useState<Set<string>>(new Set(ALL_EVENT_TYPES));
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [filterOpen, setFilterOpen] = useState(false);
  // 虚拟化状态
  const [scrollTop, setScrollTop] = useState(0);
  const [containerHeight, setContainerHeight] = useState(0);
  const [, setMeasureTick] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);
  // 测量缓存: key = `${trace_id}:${isExpanded}` -> height (事件移位时保持稳定)
  const measuredRef = useRef<Map<string, number>>(new Map());

  // 订阅事件流
  useEffect(() => {
    let unsubscribe: (() => void) | null = null;
    subscribeEvents((envelope) => {
      setEvents((prev) => {
        const next = [...prev, envelope];
        // 限制最大事件数
        return next.length > MAX_EVENTS ? next.slice(next.length - MAX_EVENTS) : next;
      });
    }).then((fn) => {
      unsubscribe = fn;
    }).catch(() => {
      // 非 Tauri 环境静默忽略
    });
    return () => {
      if (unsubscribe) unsubscribe();
    };
  }, []);

  // 自动滚动到底部
  useEffect(() => {
    if (autoScroll && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [events, autoScroll]);

  // 测量容器尺寸 (ResizeObserver)
  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    setContainerHeight(el.clientHeight);
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerHeight(entry.contentRect.height);
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // 清理 rAF
  useEffect(() => {
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
      }
    };
  }, []);

  // 测量已渲染项的高度 (预估 + 测量修正)
  // 每次渲染后查询 DOM，将实际高度写回 measuredRef，触发重算
  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const nodes = el.querySelectorAll('[data-item-key]');
    let changed = false;
    nodes.forEach((node) => {
      const key = node.getAttribute('data-item-key');
      if (!key) return;
      const height = (node as HTMLElement).getBoundingClientRect().height;
      const existing = measuredRef.current.get(key);
      if (existing === undefined || Math.abs(existing - height) > 1) {
        measuredRef.current.set(key, height);
        changed = true;
      }
    });
    if (changed) {
      // 触发重算 (spacer 高度基于实测值)
      setMeasureTick((t) => t + 1);
    }
  });

  const toggleType = useCallback((type: string) => {
    setSelectedTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) {
        next.delete(type);
      } else {
        next.add(type);
      }
      return next;
    });
  }, []);

  const selectAll = useCallback(() => setSelectedTypes(new Set(ALL_EVENT_TYPES)), []);
  const selectNone = useCallback(() => setSelectedTypes(new Set()), []);

  // 过滤 & 排序 (memoize 以稳定后续虚拟化计算)
  const filtered = useMemo(
    () => events.filter((e) => selectedTypes.has(e.event_type)),
    [events, selectedTypes]
  );
  // 按时间戳降序(最新在上)
  const sorted = useMemo(() => [...filtered].reverse(), [filtered]);

  // 估算项高度 (未测量时使用)
  const estimateHeight = useCallback(
    (index: number) =>
      expandedIdx === index ? EXPANDED_HEIGHT_ESTIMATE : COLLAPSED_HEIGHT_ESTIMATE,
    [expandedIdx]
  );

  // 获取项高度: 测量值优先，否则估算
  const getItemHeight = useCallback(
    (index: number) => {
      const envelope = sorted[index];
      if (!envelope) return estimateHeight(index);
      const key = `${envelope.trace_id}:${expandedIdx === index}`;
      const measured = measuredRef.current.get(key);
      return measured !== undefined ? measured : estimateHeight(index);
    },
    [sorted, expandedIdx, estimateHeight]
  );

  // 根据 offset 找到对应索引 (累加高度)
  const findIndexAtOffset = useCallback(
    (offset: number) => {
      if (offset <= 0) return 0;
      let acc = 0;
      for (let i = 0; i < sorted.length; i++) {
        acc += getItemHeight(i);
        if (acc > offset) return i;
      }
      return sorted.length;
    },
    [sorted.length, getItemHeight]
  );

  // 计算可见范围 (含缓冲)
  const startIndex = Math.max(0, findIndexAtOffset(scrollTop) - VIRTUAL_BUFFER);
  const endIndex = Math.min(
    sorted.length,
    findIndexAtOffset(scrollTop + containerHeight) + VIRTUAL_BUFFER
  );

  // 上方占位高度
  const topSpacer = useMemo(() => {
    let h = 0;
    for (let i = 0; i < startIndex; i++) h += getItemHeight(i);
    return h;
  }, [startIndex, getItemHeight]);

  // 下方占位高度
  const bottomSpacer = useMemo(() => {
    let h = 0;
    for (let i = endIndex; i < sorted.length; i++) h += getItemHeight(i);
    return h;
  }, [endIndex, sorted.length, getItemHeight]);

  const visibleItems = sorted.slice(startIndex, endIndex);

  // rAF 节流 scroll 回调
  const handleScroll = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      if (listRef.current) {
        setScrollTop(listRef.current.scrollTop);
      }
    });
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* 工具栏 */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '6px 10px',
          borderBottom: '1px solid var(--border)',
          background: 'var(--bg-secondary)',
          fontSize: 12,
        }}
      >
        <span style={{ fontWeight: 600, color: 'var(--text-primary)' }}>
          {t('eventStream.title')}
        </span>
        <span style={{ color: 'var(--text-muted)', fontSize: 11 }}>
          {t('eventStream.eventCount', { count: events.length })}
        </span>
        <div style={{ flex: 1 }} />
        <button
          onClick={() => setFilterOpen(!filterOpen)}
          style={{
            padding: '2px 8px',
            fontSize: 11,
            cursor: 'pointer',
            border: '1px solid var(--border)',
            borderRadius: 4,
            background: filterOpen ? 'var(--accent-neon)' : 'transparent',
            color: filterOpen ? 'var(--bg-primary)' : 'var(--text-secondary)',
          }}
        >
          {t('eventStream.filter')}
        </button>
        <button
          onClick={() => setAutoScroll(!autoScroll)}
          style={{
            padding: '2px 8px',
            fontSize: 11,
            cursor: 'pointer',
            border: '1px solid var(--border)',
            borderRadius: 4,
            background: autoScroll ? 'var(--accent-neon)' : 'transparent',
            color: autoScroll ? 'var(--bg-primary)' : 'var(--text-secondary)',
          }}
        >
          {autoScroll ? t('eventStream.autoScrollOn') : t('eventStream.autoScrollOff')}
        </button>
        <button
          onClick={() => setEvents([])}
          style={{
            padding: '2px 8px',
            fontSize: 11,
            cursor: 'pointer',
            border: '1px solid var(--border)',
            borderRadius: 4,
            background: 'transparent',
            color: 'var(--text-secondary)',
          }}
        >
          {t('eventStream.clear')}
        </button>
      </div>

      {/* 过滤面板 */}
      {filterOpen && (
        <div
          style={{
            padding: '8px 10px',
            borderBottom: '1px solid var(--border)',
            background: 'var(--bg-tertiary)',
            display: 'flex',
            flexWrap: 'wrap',
            gap: 4,
            alignItems: 'center',
          }}
        >
          <button
            onClick={selectAll}
            style={{ fontSize: 10, padding: '2px 6px', cursor: 'pointer', border: '1px solid var(--border)', borderRadius: 3, background: 'transparent', color: 'var(--text-secondary)' }}
          >
            {t('eventStream.selectAll')}
          </button>
          <button
            onClick={selectNone}
            style={{ fontSize: 10, padding: '2px 6px', cursor: 'pointer', border: '1px solid var(--border)', borderRadius: 3, background: 'transparent', color: 'var(--text-secondary)' }}
          >
            {t('eventStream.selectNone')}
          </button>
          {ALL_EVENT_TYPES.map((type) => (
            <label
              key={type}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 3,
                fontSize: 11,
                cursor: 'pointer',
                padding: '1px 6px',
                borderRadius: 3,
                background: selectedTypes.has(type) ? 'var(--bg-primary)' : 'transparent',
                border: `1px solid ${selectedTypes.has(type) ? EVENT_COLORS[type] || 'var(--border)' : 'var(--border)'}`,
              }}
            >
              <input
                type="checkbox"
                checked={selectedTypes.has(type)}
                onChange={() => toggleType(type)}
                style={{ margin: 0, accentColor: EVENT_COLORS[type] || 'var(--accent-neon)' }}
              />
              <span style={{ color: EVENT_COLORS[type] || 'var(--text-primary)' }}>
                {EVENT_ICONS[type] || '•'} {type}
              </span>
            </label>
          ))}
        </div>
      )}

      {/* 事件列表 (虚拟化) */}
      <div
        ref={listRef}
        onScroll={handleScroll}
        style={{
          flex: 1,
          overflowY: 'auto',
          fontFamily: "'Menlo', 'Consolas', monospace",
          fontSize: 12,
        }}
      >
        {sorted.length === 0 && (
          <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-muted)' }}>
            {t('eventStream.empty')}
          </div>
        )}
        {/* 上方占位: 撑起未渲染的顶部高度 */}
        {topSpacer > 0 && <div style={{ height: topSpacer }} />}
        {/* 可见项 + 缓冲 */}
        {visibleItems.map((envelope, i) => {
          const actualIndex = startIndex + i;
          const isExpanded = expandedIdx === actualIndex;
          const color = EVENT_COLORS[envelope.event_type] || 'var(--text-primary)';
          const icon = EVENT_ICONS[envelope.event_type] || '•';
          // 测量缓存键: trace_id + 展开状态 (事件移位时仍稳定)
          const itemKey = `${envelope.trace_id}:${isExpanded}`;
          return (
            <div
              key={`${envelope.timestamp}-${actualIndex}`}
              data-item-key={itemKey}
              style={{
                borderBottom: '1px solid var(--border-subtle)',
                cursor: 'pointer',
              }}
              onClick={() => setExpandedIdx(isExpanded ? null : actualIndex)}
            >
              {/* 摘要行 */}
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '4px 10px',
                  borderLeft: `3px solid ${color}`,
                }}
              >
                <span style={{ fontSize: 14 }}>{icon}</span>
                <span style={{ color, fontWeight: 600, minWidth: 140 }}>
                  {envelope.event_type}
                </span>
                <span style={{ color: 'var(--text-muted)', fontSize: 11, flex: 1 }}>
                  {eventSummary(envelope.payload)}
                </span>
                <span style={{ color: 'var(--text-muted)', fontSize: 10 }}>
                  {formatTime(envelope.timestamp)}
                </span>
              </div>

              {/* 展开详情 */}
              {isExpanded && (
                <div
                  style={{
                    padding: '8px 10px 8px 38px',
                    background: 'var(--bg-tertiary)',
                    borderTop: '1px solid var(--border-subtle)',
                  }}
                >
                  <div style={{ marginBottom: 6 }}>
                    <span style={{ color: 'var(--text-muted)', fontSize: 10, marginRight: 8 }}>
                      {t('eventStream.traceId')}
                    </span>
                    <code style={{ fontSize: 11, color: 'var(--accent-neon)', wordBreak: 'break-all' }}>
                      {envelope.trace_id}
                    </code>
                  </div>
                  <div style={{ marginBottom: 6 }}>
                    <span style={{ color: 'var(--text-muted)', fontSize: 10, marginRight: 8 }}>
                      {t('eventStream.timestamp')}
                    </span>
                    <code style={{ fontSize: 11 }}>{formatMs(envelope.timestamp)}</code>
                  </div>
                  <div>
                    <span style={{ color: 'var(--text-muted)', fontSize: 10, display: 'block', marginBottom: 4 }}>
                      {t('eventStream.payload')}
                    </span>
                    <pre
                      style={{
                        margin: 0,
                        padding: 8,
                        background: 'var(--bg-primary)',
                        borderRadius: 4,
                        fontSize: 11,
                        whiteSpace: 'pre-wrap',
                        wordBreak: 'break-all',
                        maxHeight: 200,
                        overflowY: 'auto',
                      }}
                    >
                      {JSON.stringify(envelope.payload, null, 2)}
                    </pre>
                  </div>
                </div>
              )}
            </div>
          );
        })}
        {/* 下方占位: 撑起未渲染的底部高度 */}
        {bottomSpacer > 0 && <div style={{ height: bottomSpacer }} />}
      </div>
    </div>
  );
}
