/**
 * M6 #79: 蜂群画布 — DAG 可视化组件。
 *
 * ## 功能
 * - 从 MasterEvent 流重构分层 DAG 视图(无需后端新增命令)
 * - 按 topological layer 横向排列节点(每层一行)
 * - 节点状态实时更新(pending / running / success / failed)
 * - 节点间用 SVG 折线连接(简化:每层节点连到下一层所有节点,
 *   实际边数由 decompose_completed.edge_count 标注)
 * - 节点卡片显示:sub_task_id / worker_count / elapsed_ms / status
 * - 点击节点展开详情(关联的时间线条目)
 *
 * ## 重建逻辑(基于事件流)
 *
 * 由于 MasterEvent 不包含完整 DAG 拓扑(仅 layer_index + node_count +
 * sub_task_id),DagCanvas 采用以下推断规则:
 * 1. `decompose_completed`:获取总 node_count + edge_count(展示在标题)
 * 2. `layer_started`:记录每层(layer_index → node_count)的预期节点数
 * 3. `sub_task_started`:按事件顺序将 sub_task_id 分配到当前 layer
 *    (layer_started 后的 sub_task_started 属于该层,直到 layer_completed
 *    或下一个 layer_started)
 * 4. `sub_task_completed`:更新对应 sub_task 的 status + elapsed_ms
 *
 * 这不是真正的 DAG 拓扑(无法显示具体依赖边),而是"分层进度视图"。
 * 真正的 DAG 拓扑可视化需要后端新增 `master_dag_snapshot` 命令,
 * 在 M6 范围外(后端 TaskDag 未派生 Serialize)。
 *
 * ## 集成
 * 嵌入 MasterEventTimeline 的 view toggle:"时间线" / "DAG 画布"。
 */
import { useMemo } from 'preact/hooks';
import type { MasterEvent } from '../lib/tauri';
import { t } from '../i18n';

/** 单个 DAG 节点的渲染状态。 */
type NodeStatus = 'pending' | 'running' | 'success' | 'failed';

/** 重建后的 DAG 节点。 */
interface DagNode {
  subTaskId: string;
  layerIndex: number;
  workerCount: number;
  status: NodeStatus;
  elapsedMs: number | null;
  error: string | null;
  /** 关联的时间线条目 id(用于点击展开)。 */
  timelineIds: number[];
}

/** 层信息。 */
interface DagLayer {
  index: number;
  expectedNodeCount: number;
  nodes: DagNode[];
  started: boolean;
  completed: boolean;
  successCount: number;
  failureCount: number;
}

/** 从 MasterEvent 流重建分层 DAG。 */
function reconstructDag(events: MasterEvent[]): {
  layers: DagLayer[];
  totalNodes: number;
  totalEdges: number;
  decomposeFailed: string | null;
  dagFailedReason: string | null;
} {
  const layers: DagLayer[] = [];
  let totalNodes = 0;
  let totalEdges = 0;
  let decomposeFailed: string | null = null;
  let dagFailedReason: string | null = null;
  /** sub_task_id → DagNode 引用(用于快速更新状态)。 */
  const nodeMap = new Map<string, DagNode>();
  /** 当前正在接收 sub_task 的层 index(由 layer_started 设置)。 */
  let currentLayerIndex = -1;

  for (let i = 0; i < events.length; i++) {
    const ev = events[i];
    switch (ev.kind) {
      case 'decompose_completed': {
        totalNodes = ev.node_count;
        totalEdges = ev.edge_count;
        break;
      }
      case 'decompose_failed': {
        decomposeFailed = ev.error;
        break;
      }
      case 'layer_started': {
        currentLayerIndex = ev.layer_index;
        // 确保层存在
        while (layers.length <= ev.layer_index) {
          layers.push({
            index: layers.length,
            expectedNodeCount: 0,
            nodes: [],
            started: false,
            completed: false,
            successCount: 0,
            failureCount: 0,
          });
        }
        layers[ev.layer_index].started = true;
        layers[ev.layer_index].expectedNodeCount = ev.node_count;
        break;
      }
      case 'layer_completed': {
        if (layers[ev.layer_index]) {
          layers[ev.layer_index].completed = true;
          layers[ev.layer_index].successCount = ev.success_count;
          layers[ev.layer_index].failureCount = ev.failure_count;
        }
        currentLayerIndex = -1;
        break;
      }
      case 'sub_task_started': {
        // 分配到当前层;若 layer_started 未到达,fallback 到 0 层
        const layerIdx = currentLayerIndex >= 0 ? currentLayerIndex : 0;
        while (layers.length <= layerIdx) {
          layers.push({
            index: layers.length,
            expectedNodeCount: 0,
            nodes: [],
            started: false,
            completed: false,
            successCount: 0,
            failureCount: 0,
          });
        }
        const node: DagNode = {
          subTaskId: ev.sub_task_id,
          layerIndex: layerIdx,
          workerCount: ev.worker_count,
          status: 'running',
          elapsedMs: null,
          error: null,
          timelineIds: [i],
        };
        layers[layerIdx].nodes.push(node);
        nodeMap.set(ev.sub_task_id, node);
        break;
      }
      case 'sub_task_completed': {
        const node = nodeMap.get(ev.sub_task_id);
        if (node) {
          node.status = ev.success ? 'success' : 'failed';
          node.elapsedMs = ev.elapsed_ms;
          node.error = ev.error;
          node.timelineIds.push(i);
        }
        break;
      }
      case 'dag_failed': {
        dagFailedReason = `${ev.failed_sub_task_id}: ${ev.reason}`;
        break;
      }
      default:
        // 其他事件不影响 DAG 结构
        break;
    }
  }

  return { layers, totalNodes, totalEdges, decomposeFailed, dagFailedReason };
}

/** 节点状态 → 颜色 + emoji。 */
const STATUS_STYLE: Record<NodeStatus, { color: string; bg: string; border: string; icon: string; label: string }> = {
  pending: { color: '#9ca3af', bg: 'rgba(156,163,175,0.08)', border: '#9ca3af', icon: '⏳', label: t('dagCanvas.status.pending') },
  running: { color: '#3b82f6', bg: 'rgba(59,130,246,0.12)', border: '#3b82f6', icon: '🔄', label: t('dagCanvas.status.running') },
  success: { color: '#10b981', bg: 'rgba(16,185,129,0.12)', border: '#10b981', icon: '✅', label: t('dagCanvas.status.success') },
  failed: { color: '#ef4444', bg: 'rgba(239,68,68,0.12)', border: '#ef4444', icon: '❌', label: t('dagCanvas.status.failed') },
};

/** 层状态 → 颜色。 */
function layerBadge(layer: DagLayer): { text: string; color: string } {
  if (!layer.started) return { text: t('dagCanvas.layerStatus.notStarted'), color: '#9ca3af' };
  if (!layer.completed) return { text: t('dagCanvas.layerStatus.running'), color: '#3b82f6' };
  if (layer.failureCount > 0) return { text: t('dagCanvas.layerStatus.completedWithFailures', { count: layer.failureCount }), color: '#f59e0b' };
  return { text: t('dagCanvas.layerStatus.completed'), color: '#10b981' };
}

/** 计算节点在画布中的坐标。 */
function layoutNodes(layers: DagLayer[], canvasWidth: number): {
  nodes: Array<{ node: DagNode; x: number; y: number; width: number; height: number }>;
  layerYs: number[];
} {
  const layerHeight = 130;
  const layerGap = 50;
  const nodeWidth = 160;
  const nodeHeight = 80;
  const horizontalPadding = 20;

  const nodes: Array<{ node: DagNode; x: number; y: number; width: number; height: number }> = [];
  const layerYs: number[] = [];

  layers.forEach((layer, layerIdx) => {
    const y = layerIdx * (layerHeight + layerGap) + 20;
    layerYs.push(y);

    const nodeCount = Math.max(layer.nodes.length, layer.expectedNodeCount, 1);
    const totalWidth = canvasWidth - horizontalPadding * 2;
    const stepX = totalWidth / nodeCount;
    const offsetX = stepX / 2 - nodeWidth / 2;

    // 已有节点按顺序放置
    layer.nodes.forEach((node, nodeIdx) => {
      const x = horizontalPadding + nodeIdx * stepX + offsetX;
      nodes.push({ node, x, y, width: nodeWidth, height: nodeHeight });
    });

    // 未到达的预期节点(占位)
    for (let i = layer.nodes.length; i < layer.expectedNodeCount; i++) {
      const x = horizontalPadding + i * stepX + offsetX;
      nodes.push({
        node: {
          subTaskId: `(pending #${i + 1})`,
          layerIndex: layerIdx,
          workerCount: 0,
          status: 'pending',
          elapsedMs: null,
          error: null,
          timelineIds: [],
        },
        x,
        y,
        width: nodeWidth,
        height: nodeHeight,
      });
    }
  });

  return { nodes, layerYs };
}

interface DagCanvasProps {
  /** MasterEvent 流(按时间顺序)。 */
  events: MasterEvent[];
  /** 画布宽度(像素),默认 800。 */
  canvasWidth?: number;
  /** 节点点击回调(传入 sub_task_id)。 */
  onNodeClick?: (subTaskId: string) => void;
}

export function DagCanvas({ events, canvasWidth = 800, onNodeClick }: DagCanvasProps) {
  const dag = useMemo(() => reconstructDag(events), [events]);
  const layout = useMemo(
    () => layoutNodes(dag.layers, canvasWidth),
    [dag.layers, canvasWidth],
  );

  const canvasHeight = dag.layers.length * 180 + 40;

  // 空状态
  if (events.length === 0) {
    return (
      <div class="dag-canvas-empty">
        <div style={{ fontSize: 32, marginBottom: 8 }}>📊</div>
        <div>{t('dagCanvas.empty.hint')}</div>
      </div>
    );
  }

  // decompose 失败
  if (dag.decomposeFailed) {
    return (
      <div class="dag-canvas-error">
        <div style={{ fontSize: 32, marginBottom: 8 }}>💥</div>
        <strong>{t('dagCanvas.error.decomposeFailed')}</strong>
        <pre class="dag-error-detail">{dag.decomposeFailed}</pre>
      </div>
    );
  }

  return (
    <div class="dag-canvas-container">
      {/* 头部统计 */}
      <div class="dag-canvas-header">
        <span class="dag-stat">
          <strong>{dag.totalNodes}</strong> {t('dagCanvas.stat.nodes')}
        </span>
        <span class="dag-stat">
          <strong>{dag.totalEdges}</strong> {t('dagCanvas.stat.edges')}
        </span>
        <span class="dag-stat">
          <strong>{dag.layers.length}</strong> {t('dagCanvas.stat.layers')}
        </span>
        {dag.dagFailedReason && (
          <span class="dag-stat dag-stat-error" title={dag.dagFailedReason}>
            {t('dagCanvas.stat.dagFailed')}
          </span>
        )}
      </div>

      {/* SVG 画布 */}
      <div class="dag-canvas-scroll" style={{ overflowX: 'auto' }}>
        <svg
          width={canvasWidth}
          height={canvasHeight}
          class="dag-canvas-svg"
          style={{ display: 'block' }}
        >
          {/* 层标签 + 分隔线 */}
          {dag.layers.map((layer, idx) => {
            const y = layout.layerYs[idx] ?? 0;
            const badge = layerBadge(layer);
            return (
              <g key={`layer-${idx}`}>
                <line
                  x1={0}
                  y1={y - 20}
                  x2={canvasWidth}
                  y2={y - 20}
                  stroke="var(--border)"
                  strokeWidth={1}
                  strokeDasharray="4 4"
                />
                <text
                  x={8}
                  y={y - 26}
                  fill={badge.color}
                  fontSize={11}
                  fontWeight={600}
                >
                  {t('dagCanvas.layer.label', { idx, status: badge.text, actual: layer.nodes.length, expected: layer.expectedNodeCount })}
                </text>
              </g>
            );
          })}

          {/* 层间连接线(简化:每层节点连到下一层所有节点) */}
          {dag.layers.slice(0, -1).map((layer, layerIdx) => {
            const nextLayer = dag.layers[layerIdx + 1];
            if (!nextLayer) return null;
            const currentNodes = layout.nodes.filter((n) => n.node.layerIndex === layerIdx);
            const nextNodes = layout.nodes.filter((n) => n.node.layerIndex === layerIdx + 1);
            return currentNodes.flatMap((src, srcIdx) =>
              nextNodes.map((dst, dstIdx) => {
                const x1 = src.x + src.width / 2;
                const y1 = src.y + src.height;
                const x2 = dst.x + dst.width / 2;
                const y2 = dst.y;
                const midY = (y1 + y2) / 2;
                // 贝塞尔曲线
                const path = `M ${x1} ${y1} C ${x1} ${midY}, ${x2} ${midY}, ${x2} ${y2}`;
                const isFailed =
                  src.node.status === 'failed' || dst.node.status === 'failed';
                return (
                  <path
                    key={`edge-${layerIdx}-${srcIdx}-${dstIdx}`}
                    d={path}
                    fill="none"
                    stroke={isFailed ? '#ef4444' : 'var(--border)'}
                    strokeWidth={isFailed ? 1.5 : 1}
                    strokeOpacity={isFailed ? 0.7 : 0.4}
                  />
                );
              }),
            );
          })}

          {/* 节点 */}
          {layout.nodes.map((item, idx) => {
            const { node } = item;
            const style = STATUS_STYLE[node.status];
            const isPlaceholder = node.subTaskId.startsWith('(pending');
            return (
              <g
                key={`node-${idx}`}
                transform={`translate(${item.x}, ${item.y})`}
                class="dag-node"
                onClick={() => !isPlaceholder && onNodeClick?.(node.subTaskId)}
                style={{ cursor: isPlaceholder ? 'default' : 'pointer' }}
              >
                <rect
                  width={item.width}
                  height={item.height}
                  rx={6}
                  fill={style.bg}
                  stroke={style.border}
                  strokeWidth={1.5}
                  strokeDasharray={isPlaceholder ? '3 3' : undefined}
                  strokeOpacity={isPlaceholder ? 0.4 : 1}
                />
                <text
                  x={item.width / 2}
                  y={18}
                  textAnchor="middle"
                  fontSize={11}
                  fontWeight={600}
                  fill={style.color}
                >
                  {style.icon} {isPlaceholder ? '⏳' : node.subTaskId.slice(0, 14)}
                </text>
                <text
                  x={item.width / 2}
                  y={36}
                  textAnchor="middle"
                  fontSize={10}
                  fill="var(--text-secondary)"
                >
                  {node.workerCount > 0 ? `${node.workerCount} workers` : '—'}
                </text>
                <text
                  x={item.width / 2}
                  y={52}
                  textAnchor="middle"
                  fontSize={10}
                  fill={style.color}
                >
                  {node.elapsedMs !== null ? `${(node.elapsedMs / 1000).toFixed(2)}s` : style.label}
                </text>
                {node.error && (
                  <title>{node.error}</title>
                )}
              </g>
            );
          })}
        </svg>
      </div>

      {/* 图例 */}
      <div class="dag-legend">
        {(['pending', 'running', 'success', 'failed'] as NodeStatus[]).map((s) => {
          const style = STATUS_STYLE[s];
          return (
            <span key={s} class="dag-legend-item">
              <span
                class="dag-legend-dot"
                style={{ background: style.bg, border: `1.5px solid ${style.border}` }}
              />
              {style.icon} {style.label}
            </span>
          );
        })}
      </div>
    </div>
  );
}
