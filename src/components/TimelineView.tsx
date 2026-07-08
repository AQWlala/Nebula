/**
 * T-E-B-02: 记忆时间轴视图 — `/journey` 回放记忆演化。
 *
 * 三视图之一(图谱/Markdown/时间轴)。按 created_at 倒序展示记忆,
 * 按日期分组,支持点击展开详情、层筛选、重要性筛选。
 *
 * 设计要点:
 * - 虚拟滚动不必要(memoryListRecent 上限 200,DOM 节点可控)
 * - 按日期分组(YYYY-MM-DD),每组显示计数 + 层分布迷你条
 * - 每条记忆:时间 + 层色点 + 摘要 + importance 条 + source/压缩 badge
 * - 点击展开全文 + 元数据(provenance/access_count/ingest_cost)
 * - 顶部统计栏:总数 / 日期跨度 / 层分布
 */
import { useState, useEffect, useCallback, useMemo } from 'preact/hooks';
import { nebulaAPI, type Memory, type Layer } from '../lib/tauri';
import { t } from '../i18n';

const LAYER_COLORS: Record<Layer, string> = {
  L0: '#9CA3AF',
  L1: '#6EE7B7',
  L2: '#93C5FD',
  L3: '#A78BFA',
  L4: '#F472B6',
  L5: '#F59E0B',
  L6: '#EF4444',
  L7: '#FFD700',
};

const LAYER_ORDER: Layer[] = ['L0', 'L1', 'L2', 'L3', 'L4', 'L5', 'L6', 'L7'];

const layerLabel = (l: Layer): string => t(`memoryMap.layer.${l}`);

interface DayGroup {
  date: string; // YYYY-MM-DD
  label: string; // 本地化日期标签
  memories: Memory[];
  layerCounts: Partial<Record<Layer, number>>;
}

/** 把时间戳(秒)按日期分组。 */
function groupByDay(memories: Memory[]): DayGroup[] {
  const map = new Map<string, DayGroup>();
  // 按创建时间降序(最新在前)
  const sorted = [...memories].sort((a, b) => b.created_at - a.created_at);
  for (const m of sorted) {
    const d = new Date(m.created_at * 1000);
    const dateKey = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    let group = map.get(dateKey);
    if (!group) {
      group = {
        date: dateKey,
        label: d.toLocaleDateString('zh-CN', {
          year: 'numeric',
          month: 'long',
          day: 'numeric',
          weekday: 'long',
        }),
        memories: [],
        layerCounts: {},
      };
      map.set(dateKey, group);
    }
    group.memories.push(m);
    group.layerCounts[m.layer] = (group.layerCounts[m.layer] ?? 0) + 1;
  }
  return Array.from(map.values());
}

export function TimelineView() {
  const [memories, setMemories] = useState<Memory[]>([]);
  const [loading, setLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [layerFilter, setLayerFilter] = useState<Set<Layer>>(new Set());
  const [minImportance, setMinImportance] = useState(0);

  const loadMemories = useCallback(async () => {
    setLoading(true);
    try {
      // 取 200 条用于时间轴回放
      const list = await nebulaAPI.memoryListRecent(200);
      setMemories(list);
    } catch (e) {
      console.error('TimelineView loadMemories failed:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadMemories();
  }, [loadMemories]);

  // 按层 + 重要性筛选
  const filtered = useMemo(() => {
    return memories.filter((m) => {
      if (layerFilter.size > 0 && !layerFilter.has(m.layer)) return false;
      if (m.importance < minImportance) return false;
      return true;
    });
  }, [memories, layerFilter, minImportance]);

  const dayGroups = useMemo(() => groupByDay(filtered), [filtered]);

  // 统计栏
  const stats = useMemo(() => {
    if (filtered.length === 0) return null;
    const layerCounts: Partial<Record<Layer, number>> = {};
    let minTs = Infinity;
    let maxTs = -Infinity;
    for (const m of filtered) {
      layerCounts[m.layer] = (layerCounts[m.layer] ?? 0) + 1;
      if (m.created_at < minTs) minTs = m.created_at;
      if (m.created_at > maxTs) maxTs = m.created_at;
    }
    return {
      total: filtered.length,
      layerCounts,
      span: `${new Date(minTs * 1000).toLocaleDateString('zh-CN')} → ${new Date(maxTs * 1000).toLocaleDateString('zh-CN')}`,
    };
  }, [filtered]);

  const toggleLayer = (layer: Layer) => {
    setLayerFilter((prev) => {
      const next = new Set(prev);
      if (next.has(layer)) next.delete(layer);
      else next.add(layer);
      return next;
    });
  };

  const _expandedMemory = memories.find((m) => m.id === expandedId);
  void _expandedMemory; // used for future rendering expansion

  return (
    <div
      className="timeline-view h-full flex flex-col bg-gray-950 text-white"
      data-testid="timeline-view"
    >
      {/* Header + 统计 */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800">
        <h2 className="text-sm font-semibold text-gray-300">记忆时间轴 · Journey</h2>
        <div className="flex items-center gap-3">
          {loading && <span className="text-xs text-gray-500">加载中…</span>}
          <span className="text-xs text-gray-500">{filtered.length} 条记忆</span>
          <button
            onClick={loadMemories}
            className="text-xs text-gray-400 hover:text-white transition-colors"
            title="刷新"
          >
            ↻
          </button>
        </div>
      </div>

      {/* 统计栏 */}
      {stats && (
        <div className="flex flex-wrap items-center gap-4 px-4 py-1.5 border-b border-gray-800 text-xs text-gray-400">
          <span>共 {stats.total} 条</span>
          <span>跨度: {stats.span}</span>
          <div className="flex items-center gap-1">
            {LAYER_ORDER.map((l) => {
              const count = stats.layerCounts[l] ?? 0;
              if (count === 0) return null;
              return (
                <span key={l} className="flex items-center gap-0.5" title={`${l}: ${count} 条`}>
                  <span
                    className="w-2 h-2 rounded-full"
                    style={{ backgroundColor: LAYER_COLORS[l] }}
                  />
                  {count}
                </span>
              );
            })}
          </div>
        </div>
      )}

      {/* 筛选栏 */}
      <div className="flex flex-wrap items-center gap-3 px-4 py-1.5 border-b border-gray-800 text-xs">
        <span className="text-gray-500">层筛选:</span>
        {LAYER_ORDER.map((layer) => (
          <label
            key={layer}
            data-testid={`timeline-filter-${layer}`}
            className="flex items-center gap-1 cursor-pointer select-none"
          >
            <input
              type="checkbox"
              checked={layerFilter.has(layer)}
              onChange={() => toggleLayer(layer)}
              className="w-3 h-3"
            />
            <span
              className="w-2 h-2 rounded-full inline-block"
              style={{ backgroundColor: LAYER_COLORS[layer] }}
            />
            <span className="text-gray-400">{layer}</span>
          </label>
        ))}
        <span className="text-gray-500 ml-4">最小重要性:</span>
        <input
          type="range"
          min={0}
          max={1}
          step={0.1}
          value={minImportance}
          onChange={(e) => setMinImportance(parseFloat((e.target as HTMLInputElement).value))}
          className="w-24"
          data-testid="timeline-importance-slider"
        />
        <span className="text-gray-400">{minImportance.toFixed(1)}</span>
      </div>

      {/* 时间轴主体 */}
      <div className="flex-1 overflow-y-auto">
        {dayGroups.length === 0 && !loading && (
          <div className="text-center text-gray-500 py-12" data-testid="timeline-empty">
            暂无符合条件的记忆
          </div>
        )}
        {dayGroups.map((group) => (
          <div key={group.date} className="timeline-day-group">
            {/* 日期标题 */}
            <div className="sticky top-0 bg-gray-900/95 backdrop-blur px-4 py-1.5 border-b border-gray-800 flex items-center justify-between">
              <span className="text-xs font-medium text-gray-300">{group.label}</span>
              <span className="text-xs text-gray-600">{group.memories.length} 条</span>
            </div>
            {/* 当日记忆列表 */}
            <div className="relative pl-8 pr-4 py-2">
              {/* 竖线 */}
              <div className="absolute left-4 top-0 bottom-0 w-px bg-gray-800" />
              {group.memories.map((m) => {
                const isExpanded = expandedId === m.id;
                const summary = m.summary.s150 || m.summary.s50 || m.content.slice(0, 150);
                const time = new Date(m.created_at * 1000).toLocaleTimeString('zh-CN', {
                  hour: '2-digit',
                  minute: '2-digit',
                });
                return (
                  <div key={m.id} className="relative mb-3">
                    {/* 节点圆点 */}
                    <span
                      className="absolute -left-4 top-2 w-3 h-3 rounded-full border-2 border-gray-950"
                      style={{ backgroundColor: LAYER_COLORS[m.layer] }}
                    />
                    <div
                      data-testid={`timeline-item-${m.id}`}
                      className={`bg-gray-900 rounded p-2 cursor-pointer border transition-colors ${isExpanded ? 'border-blue-600' : 'border-gray-800 hover:border-gray-700'}`}
                      onClick={() => setExpandedId(isExpanded ? null : m.id)}
                    >
                      <div className="flex items-center gap-2 mb-1">
                        <span className="text-xs text-gray-500">{time}</span>
                        <span className="text-xs text-gray-600">·</span>
                        <span className="text-xs" style={{ color: LAYER_COLORS[m.layer] }}>
                          {m.layer}
                        </span>
                        <span className="text-xs text-gray-600">{layerLabel(m.layer)}</span>
                        {m.compressed_from && (
                          <span className="text-xs px-1.5 py-0.5 rounded bg-red-900/60 text-red-300">
                            压缩
                          </span>
                        )}
                        {m.pinned && (
                          <span className="text-xs px-1.5 py-0.5 rounded bg-yellow-900/60 text-yellow-300">
                            置顶
                          </span>
                        )}
                        {/* importance 条 */}
                        <span className="flex items-center gap-1 ml-auto">
                          <span className="text-xs text-gray-600">重要性</span>
                          <span className="w-12 h-1.5 bg-gray-700 rounded-full overflow-hidden">
                            <span
                              className="block h-full bg-blue-500 rounded-full"
                              style={{ width: `${m.importance * 100}%` }}
                            />
                          </span>
                        </span>
                      </div>
                      <div className={`text-sm text-gray-200 ${isExpanded ? '' : 'line-clamp-2'}`}>
                        {summary}
                      </div>
                      {isExpanded && (
                        <div className="mt-2 pt-2 border-t border-gray-800 text-xs text-gray-400 space-y-1">
                          <div className="text-gray-300 whitespace-pre-wrap">{m.content}</div>
                          <div className="flex flex-wrap gap-3 pt-1">
                            <span>来源: {m.source}</span>
                            <span>类型: {m.memory_type}</span>
                            <span>访问: {m.access_count}次</span>
                            {m.ingest_cost != null && m.ingest_cost > 0 && (
                              <span>💰 ${m.ingest_cost.toFixed(4)}</span>
                            )}
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        ))}
      </div>

      {/* 层图例 */}
      <div className="flex flex-wrap gap-3 px-4 py-2 border-t border-gray-800 text-xs">
        {LAYER_ORDER.map((layer) => (
          <div key={layer} className="flex items-center gap-1">
            <div
              className="w-3 h-3 rounded-full"
              style={{ backgroundColor: LAYER_COLORS[layer] }}
            />
            <span className="text-gray-400">{layer}</span>
            <span className="text-gray-600">{layerLabel(layer)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
