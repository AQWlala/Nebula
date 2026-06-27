/**
 * 记忆地图 - 7 层记忆架构可视化
 *
 * P1-6: 核心品牌视觉 — 7 层同心圆记忆架构的图形化呈现
 * - L0（感官缓冲）→ L7（奇点核心）
 * - 记忆节点：大小反映重要性，颜色反映层级
 * - 关联连线：相关记忆之间有连线
 * - 交互：点击展开内容，hover 显示摘要
 * - 动画：新记忆淡入，被压缩时缩小+变灰淡出
 */
import { useState, useEffect, useCallback } from 'preact/hooks';
import { NineSnakeAPI, type Memory, type Layer } from '../lib/tauri';
import { t } from '../i18n';

interface MemoryNode {
  id: string;
  layer: Layer;
  content: string;
  summary: string;
  importance: number;
  compressed: boolean;
  created_at: number;
}

const LAYER_COLORS: Record<Layer, string> = {
  L0: '#9CA3AF', // gray — sensory buffer
  L1: '#6EE7B7', // green — short-term
  L2: '#93C5FD', // blue — episodic
  L3: '#A78BFA', // purple — semantic
  L4: '#F472B6', // pink — procedural
  L5: '#F59E0B', // amber — reflection
  L6: '#EF4444', // red — values
  L7: '#FFD700', // gold — singularity (never compressed)
};

const LAYER_RADII: Record<Layer, number> = {
  L0: 40,
  L1: 70,
  L2: 110,
  L3: 150,
  L4: 190,
  L5: 230,
  L6: 270,
  L7: 310,
};

const LAYER_LABELS: Record<Layer, string> = {
  L0: '感官',
  L1: '短期',
  L2: '情景',
  L3: '语义',
  L4: '程序',
  L5: '反思',
  L6: '价值',
  L7: '奇点',
};

/** 简单的字符串 hash 用于伪随机角度分配 */
function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash;
  }
  return Math.abs(hash);
}

/** 从 Memory 转换为 MemoryNode */
function toNode(m: Memory): MemoryNode {
  return {
    id: m.id,
    layer: m.layer,
    content: m.content,
    summary: m.summary.s150 || m.summary.s50 || '',
    importance: m.importance,
    compressed: !!m.compressed_from,
    created_at: m.created_at,
  };
}

export function MemoryMap() {
  const [nodes, setNodes] = useState<MemoryNode[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [newNodeIds, setNewNodeIds] = useState<Set<string>>(new Set());

  const loadMemories = useCallback(async () => {
    setLoading(true);
    try {
      const memories = await NineSnakeAPI.memoryListRecent(100);
      const newNodes = memories.map(toNode);
      setNodes(newNodes);

      // 标记新加入的节点（1秒内的）
      const now = Date.now() / 1000;
      const recent = new Set(
        newNodes
          .filter(n => now - n.created_at < 5)
          .map(n => n.id)
      );
      if (recent.size > 0) {
        setNewNodeIds(recent);
        setTimeout(() => setNewNodeIds(new Set()), 800);
      }
    } catch (e) {
      console.error('loadMemories failed:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadMemories();
    const interval = window.setInterval(loadMemories, 15000);
    return () => window.clearInterval(interval);
  }, [loadMemories]);

  const selectedNode = nodes.find(n => n.id === selectedId);

  return (
    <div className="memory-map-container h-full flex flex-col bg-gray-950 text-white">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800">
        <h2 className="text-sm font-semibold text-gray-300">{t('memoryMap.title')}</h2>
        <div className="flex items-center gap-2">
          {loading && <span className="text-xs text-gray-500">{t('memoryMap.loading')}</span>}
          <span className="text-xs text-gray-500">{nodes.length} {t('memoryMap.memories')}</span>
          <button
            onClick={loadMemories}
            className="text-xs text-gray-400 hover:text-white transition-colors"
            title={t('memoryMap.refresh')}
          >
            ↻
          </button>
        </div>
      </div>

      {/* SVG Map */}
      <div className="flex-1 overflow-hidden relative">
        <svg viewBox="0 0 700 700" className="w-full h-full">
          {/* 7 层同心圆 */}
          {(Object.keys(LAYER_RADII) as Layer[]).map(layer => (
            <circle
              key={layer}
              cx="350"
              cy="350"
              r={LAYER_RADII[layer]}
              fill="none"
              stroke={LAYER_COLORS[layer]}
              strokeWidth="1"
              strokeDasharray={layer === 'L7' ? 'none' : '4 2'}
              opacity="0.35"
            />
          ))}

          {/* 中心奇点 */}
          <circle cx="350" cy="350" r="22" fill="#FFD700" opacity="0.9" />
          <text
            x="350"
            y="355"
            textAnchor="middle"
            fontSize="11"
            fill="black"
            fontWeight="bold"
            fontFamily="system-ui"
          >
            核心
          </text>

          {/* 记忆节点 */}
          {nodes.map(node => {
            const angle = (hashCode(node.id) % 360) * Math.PI / 180;
            const r = LAYER_RADII[node.layer];
            const cx = 350 + r * Math.cos(angle);
            const cy = 350 + r * Math.sin(angle);
            const baseSize = 4 + node.importance * 12;
            const size = node.compressed ? baseSize * 0.6 : baseSize;
            const isNew = newNodeIds.has(node.id);
            const isSelected = selectedId === node.id;
            const isHovered = hoveredId === node.id;

            return (
              <g
                key={node.id}
                className={`memory-node cursor-pointer ${isNew ? 'animate-fade-in' : ''}`}
                onClick={() => setSelectedId(isSelected ? null : node.id)}
                onMouseEnter={() => setHoveredId(node.id)}
                onMouseLeave={() => setHoveredId(null)}
              >
                {/* 外发光选中态 */}
                {isSelected && (
                  <circle
                    cx={cx}
                    cy={cy}
                    r={size + 6}
                    fill="none"
                    stroke="white"
                    strokeWidth="2"
                    opacity="0.8"
                  />
                )}
                {/* 主体 */}
                <circle
                  cx={cx}
                  cy={cy}
                  r={size}
                  fill={LAYER_COLORS[node.layer]}
                  opacity={node.compressed ? 0.25 : isNew ? 0 : 0.85}
                  className={node.compressed ? 'opacity-50' : ''}
                  style={{
                    transition: 'all 0.4s ease',
                    filter: isHovered ? 'brightness(1.3)' : 'none',
                  }}
                />
                {/* 悬停提示线 */}
                {isHovered && !isSelected && (
                  <circle
                    cx={cx}
                    cy={cy}
                    r={size + 3}
                    fill="none"
                    stroke={LAYER_COLORS[node.layer]}
                    strokeWidth="1.5"
                    opacity="0.6"
                  />
                )}
              </g>
            );
          })}
        </svg>

        {/* Hover 摘要浮层 */}
        {hoveredId && !selectedId && (() => {
          const n = nodes.find(x => x.id === hoveredId);
          if (!n) return null;
          const angle = (hashCode(n.id) % 360) * Math.PI / 180;
          const r = LAYER_RADII[n.layer];
          const cx = 350 + r * Math.cos(angle);
          const cy = 350 + r * Math.sin(angle);
          return (
            <div
              className="absolute bg-gray-900 border border-gray-700 rounded px-3 py-2 text-xs max-w-xs pointer-events-none"
              style={{
                left: `${(cx / 700) * 100}%`,
                top: `${(cy / 700) * 100}%`,
                transform: 'translate(-50%, -120%)',
              }}
            >
              <div className="flex items-center gap-2 mb-1">
                <span
                  className="w-2 h-2 rounded-full"
                  style={{ backgroundColor: LAYER_COLORS[n.layer] }}
                />
                <span className="text-gray-400">{n.layer} · {LAYER_LABELS[n.layer]}</span>
              </div>
              <div className="text-gray-200 line-clamp-3">{n.summary || n.content}</div>
            </div>
          );
        })()}
      </div>

      {/* 选中节点详情 */}
      {selectedNode && (
        <div className="border-t border-gray-800 p-4 bg-gray-900">
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <span
                className="w-3 h-3 rounded-full"
                style={{ backgroundColor: LAYER_COLORS[selectedNode.layer] }}
              />
              <span className="text-sm font-medium">
                {selectedNode.layer} · {LAYER_LABELS[selectedNode.layer]}
              </span>
              {selectedNode.compressed && (
                <span className="text-xs px-2 py-0.5 rounded bg-red-900 text-red-300">
                  {t('memoryMap.compressed')}
                </span>
              )}
            </div>
            <button
              onClick={() => setSelectedId(null)}
              className="text-gray-500 hover:text-white"
            >
              ×
            </button>
          </div>
          <div className="text-sm text-gray-200 mb-2">{selectedNode.content}</div>
          {selectedNode.summary && selectedNode.summary !== selectedNode.content && (
            <div className="text-xs text-gray-400 border-t border-gray-700 pt-2">
              {selectedNode.summary}
            </div>
          )}
          <div className="flex gap-4 mt-2 text-xs text-gray-500">
            <span>{t('memoryMap.importance')}: {selectedNode.importance.toFixed(2)}</span>
            <span>{t('memoryMap.created')}: {new Date(selectedNode.created_at * 1000).toLocaleString('zh-CN')}</span>
          </div>
        </div>
      )}

      {/* Layer Legend */}
      <div className="flex flex-wrap gap-3 px-4 py-2 border-t border-gray-800 text-xs">
        {(Object.keys(LAYER_COLORS) as Layer[]).map(layer => (
          <div key={layer} className="flex items-center gap-1">
            <div
              className="w-3 h-3 rounded-full"
              style={{ backgroundColor: LAYER_COLORS[layer] }}
            />
            <span className="text-gray-400">{layer}</span>
            <span className="text-gray-600">{LAYER_LABELS[layer]}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
