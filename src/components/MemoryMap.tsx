/**
 * 记忆地图 - 7 层记忆架构可视化 (PixiJS WebGL 渲染)
 *
 * P1-6: 核心品牌视觉 — 7 层同心圆记忆架构的图形化呈现
 * - L0（感官缓冲）→ L7（奇点核心）
 * - 记忆节点：大小反映重要性，颜色反映层级
 * - 交互：点击展开内容，hover 显示摘要
 * - 动画：新记忆淡入，被压缩时缩小+变灰淡出
 * - T-S5-B-02: SVG → PixiJS 迁移，支持 1000+ 节点流畅渲染、缩放/拖拽
 *
 * T-E-B-07: 力导向图视图 — 消费 T-E-B-16 MDRM 5 维关系图谱数据
 * - 视图切换：Layer View(同心圆) / Graph View(力导向图)
 * - Graph View 渲染节点 + 边，边按维度着色
 * - 5 维筛选：causal/temporal/entity/hierarchical/similarity
 * - 点击节点 → 以该节点为根重新查询 MDRM 图谱
 * - 力学模拟：节点斥力 + 边弹簧 + 向心力 + 阻尼
 */
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { Application, Container, Graphics, Text } from 'pixi.js';
import {
  nebulaAPI,
  type Memory,
  type Layer,
  type GraphSnapshot,
  type RelationDimension,
} from '../lib/tauri';
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

type ViewMode = 'layer' | 'graph';

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

/** T-E-B-07: 5 维关系边配色(与 LAYER_COLORS 解耦,便于辨识关系类型)。 */
const DIM_COLORS: Record<RelationDimension, string> = {
  causal: '#EF4444', // red — 因果
  temporal: '#3B82F6', // blue — 时序
  entity: '#10B981', // green — 实体
  hierarchical: '#A78BFA', // purple — 层级
  similarity: '#F59E0B', // amber — 相似度
};

const ALL_DIMS: RelationDimension[] = [
  'causal',
  'temporal',
  'entity',
  'hierarchical',
  'similarity',
];

/** v1.0 i18n: dimension label via t() — re-renders on locale switch. */
const dimLabel = (d: RelationDimension): string => t(`memoryMap.dim.${d}`);

/** v1.0 i18n: convert const Record map to function form so labels re-render on locale switch. */
const layerLabel = (l: Layer): string => t(`memoryMap.layer.${l}`);

/** 设计稿六层记忆（L0-L5）的图例与卡片标签及配色 */
const VIZ_LAYERS: {
  layer: 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5';
  legend: string;
  card: string;
  color: string;
  dot: string;
}[] = [
  { layer: 'L0', legend: '缓存', card: '原始', color: 'rgba(255,255,255,0.5)', dot: 'rgba(160,160,160,0.65)' },
  { layer: 'L1', legend: '消息', card: '消息摘要', color: '#0A84FF', dot: 'rgba(10,132,255,0.65)' },
  { layer: 'L2', legend: '经验', card: '经验', color: '#28c840', dot: 'rgba(48,209,88,0.65)' },
  { layer: 'L3', legend: '事实', card: '事实', color: '#FF9F0A', dot: 'rgba(255,159,10,0.65)' },
  { layer: 'L4', legend: '知识', card: '知识', color: '#a78bfa', dot: 'rgba(167,139,246,0.7)' },
  { layer: 'L5', legend: '教训', card: '教训', color: '#ff5f57', dot: 'rgba(255,95,87,0.7)' },
];

/** 默认记忆卡片（设计稿示例，真实数据为空时兜底） */
const DEFAULT_CARDS = [
  { layer: 'L5', label: '教训', color: '#ff5f57', dot: 'rgba(255,95,87,0.7)', title: '不要在生产环境热更新模型', desc: '上次热更新导致 3 分钟服务中断。必须在离线测试环境验证后再切换生产模型。' },
  { layer: 'L4', label: '知识', color: '#a78bfa', dot: 'rgba(167,139,246,0.7)', title: '项目名必须统一为 Nebula', desc: '不得出现 nine_snake / 九头蛇 / 蛇头 等旧名残留。CI 配置须用 stable 工具链。' },
  { layer: 'L3', label: '事实', color: '#FF9F0A', dot: 'rgba(255,159,10,0.65)', title: '用户使用 Tauri + Preact + Rust 技术栈', desc: '熟悉 SQLite、LanceDB、gRPC、E2EE 加密（X25519, HKDF, AES-256-GCM）。' },
  { layer: 'L2', label: '经验', color: '#28c840', dot: 'rgba(48,209,88,0.65)', title: 'Windows PowerShell 路径处理', desc: '在 Windows 上用 PathBuf 替代字符串拼接处理路径，避免反斜杠转义问题。来源：Coder agent 任务 #47。' },
  { layer: 'L1', label: '消息摘要', color: '#0A84FF', dot: 'rgba(10,132,255,0.65)', title: '用户偏好苹果系统风格设计', desc: '侧边栏毛玻璃、大圆角卡片、半透明分层、Spotlight 搜索。来源：对话 2026-07-10。' },
  { layer: 'L0', label: '原始', color: 'rgba(255,255,255,0.5)', dot: 'rgba(160,160,160,0.65)', title: '原始输入: 按竞品形式重新布局', desc: '"按照jiuwenswarm和openakita的形式重新布局，喜欢苹果系统风格" — 2026-07-10 09:30' },
];

/** 简单的字符串 hash 用于伪随机角度分配 */
function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = (hash << 5) - hash + char;
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

/** 将 #RRGGBB 转换为 0xRRGGBB 数字 */
function hexToNumber(hex: string): number {
  return parseInt(hex.replace('#', ''), 16);
}

/** 单个节点的 PixiJS 资源(支持 layer + graph 双模式)。 */
interface NodeGraphic {
  graphics: Graphics;
  halo: Graphics;
  /** Layer 模式目标位置。 */
  targetX: number;
  targetY: number;
  baseSize: number;
  alpha: number;
  targetAlpha: number;
  /** Graph 模式力学速度。 */
  vx: number;
  vy: number;
  /** Graph 模式是否为根节点(锁定位置)。 */
  pinned: boolean;
}

/** 拖拽状态 */
interface DragState {
  isDragging: boolean;
  startX: number;
  startY: number;
  lastX: number;
  lastY: number;
  moved: boolean;
}

export function MemoryMap() {
  const [nodes, setNodes] = useState<MemoryNode[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [newNodeIds, setNewNodeIds] = useState<Set<string>>(new Set());
  const [appReady, setAppReady] = useState(false);

  // T-E-B-07: Graph View 状态
  const [viewMode, setViewMode] = useState<ViewMode>('layer');
  const [graphSnapshot, setGraphSnapshot] = useState<GraphSnapshot | null>(null);
  const [graphRootId, setGraphRootId] = useState<string | null>(null);
  const [graphLoading, setGraphLoading] = useState(false);
  const [activeDims, setActiveDims] = useState<Set<RelationDimension>>(new Set(ALL_DIMS));

  // PixiJS 资源引用
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  // 六层记忆图谱 HTML 可视化容器（参考设计稿 preview.html）
  const vizRef = useRef<HTMLDivElement>(null);
  const hoverDivRef = useRef<HTMLDivElement>(null);
  const appRef = useRef<Application | null>(null);
  const worldRef = useRef<Container | null>(null);
  const nodesContainerRef = useRef<Container | null>(null);
  const edgesRef = useRef<Graphics | null>(null);
  const nodeGraphicsRef = useRef<Map<string, NodeGraphic>>(new Map());

  // 同步 state 到 ref（供 PixiJS 事件回调读取最新值，避免闭包陈旧）
  const selectedIdRef = useRef<string | null>(null);
  const hoveredIdRef = useRef<string | null>(null);
  const nodesCountRef = useRef(0);
  const viewModeRef = useRef<ViewMode>('layer');
  const graphSnapshotRef = useRef<GraphSnapshot | null>(null);
  useEffect(() => {
    selectedIdRef.current = selectedId;
  }, [selectedId]);
  useEffect(() => {
    hoveredIdRef.current = hoveredId;
  }, [hoveredId]);
  useEffect(() => {
    nodesCountRef.current = nodes.length;
  }, [nodes]);
  useEffect(() => {
    viewModeRef.current = viewMode;
  }, [viewMode]);
  useEffect(() => {
    graphSnapshotRef.current = graphSnapshot;
  }, [graphSnapshot]);

  // 拖拽状态 + hover 节流时间戳
  const dragRef = useRef<DragState>({
    isDragging: false,
    startX: 0,
    startY: 0,
    lastX: 0,
    lastY: 0,
    moved: false,
  });
  const lastHoverUpdateRef = useRef(0);

  const loadMemories = useCallback(async () => {
    setLoading(true);
    try {
      const memories = await nebulaAPI.memoryListRecent(100);
      const newNodes = memories.map(toNode);
      setNodes(newNodes);

      // 标记新加入的节点（5秒内的）
      const now = Date.now() / 1000;
      const recent = new Set(newNodes.filter((n) => now - n.created_at < 5).map((n) => n.id));
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

  // T-E-B-07: 加载 MDRM 图谱(以指定记忆为根)。
  const loadGraph = useCallback(
    async (rootId: string) => {
      setGraphLoading(true);
      try {
        const dims = activeDims.size === 0 ? null : Array.from(activeDims);
        const snapshot = await nebulaAPI.mdrmGetGraph(rootId, dims);
        setGraphSnapshot(snapshot);
        setGraphRootId(rootId);
      } catch (e) {
        console.error('loadGraph failed:', e);
        setGraphSnapshot(null);
      } finally {
        setGraphLoading(false);
      }
    },
    [activeDims]
  );

  // 切换到 Graph View 时,若未指定根,选第一个节点
  useEffect(() => {
    if (viewMode === 'graph' && !graphRootId && nodes.length > 0) {
      loadGraph(nodes[0].id);
    }
  }, [viewMode, graphRootId, nodes, loadGraph]);

  // activeDims 变化时重新查询(仅 Graph View)
  useEffect(() => {
    if (viewMode === 'graph' && graphRootId) {
      loadGraph(graphRootId);
    }
  }, [activeDims, viewMode, graphRootId, loadGraph]);

  // 初始化 PixiJS 应用（只跑一次）
  useEffect(() => {
    let destroyed = false;
    let resizeObserver: ResizeObserver | null = null;
    let wheelHandler: ((e: WheelEvent) => void) | null = null;
    let pointerDownHandler: ((e: PointerEvent) => void) | null = null;
    let pointerMoveHandler: ((e: PointerEvent) => void) | null = null;
    let pointerUpHandler: (() => void) | null = null;

    const init = async () => {
      if (!canvasRef.current || !containerRef.current) return;

      const app = new Application();
      await app.init({
        view: canvasRef.current,
        background: 0x0a0a0a,
        antialias: true,
        resizeTo: containerRef.current,
      });

      if (destroyed) {
        app.destroy(true, { children: true });
        return;
      }

      appRef.current = app;

      // world 容器：所有可缩放/平移内容的父容器，原点设在屏幕中心
      const world = new Container();
      world.x = app.screen.width / 2;
      world.y = app.screen.height / 2;
      app.stage.addChild(world);
      worldRef.current = world;

      // 7 层同心圆(Layer View 跟 Graph View 都显示,作为背景参考)
      const rings = new Graphics();
      (Object.keys(LAYER_RADII) as Layer[]).forEach((layer) => {
        const r = LAYER_RADII[layer];
        const color = hexToNumber(LAYER_COLORS[layer]);
        rings.circle(0, 0, r);
        rings.stroke({ width: 1, color, alpha: 0.35 });
      });
      world.addChild(rings);

      // 中心奇点
      const centerGfx = new Graphics();
      centerGfx.circle(0, 0, 22);
      centerGfx.fill({ color: 0xffd700, alpha: 0.9 });
      world.addChild(centerGfx);

      // "核心" 文字
      const centerText = new Text({
        text: t('memoryMap.center'),
        style: {
          fontFamily: 'system-ui',
          fontSize: 11,
          fill: 0x000000,
          fontWeight: 'bold',
        },
      });
      centerText.anchor.set(0.5);
      centerText.x = 0;
      centerText.y = 0;
      world.addChild(centerText);

      // 边图层(在节点之下,z-index 更低)
      const edges = new Graphics();
      world.addChild(edges);
      edgesRef.current = edges;

      // 节点容器（统一管理，数据刷新时 clear + 重建）
      const nodesContainer = new Container();
      world.addChild(nodesContainer);
      nodesContainerRef.current = nodesContainer;

      // 滚轮缩放：修改 world.scale，限制 0.3~3.0
      wheelHandler = (e: WheelEvent) => {
        e.preventDefault();
        if (!worldRef.current) return;
        const delta = e.deltaY > 0 ? 0.9 : 1.1;
        const newScale = Math.max(0.3, Math.min(3.0, worldRef.current.scale.x * delta));
        worldRef.current.scale.set(newScale);
      };
      canvasRef.current.addEventListener('wheel', wheelHandler, { passive: false });

      // 拖拽平移：canvas 记录起点，window 监听 move/up 以便拖出 canvas 边界
      // 节点点击时会 stopPropagation PixiJS 事件，但 DOM 事件仍会触发，统一由 dragRef 跟踪
      pointerDownHandler = (e: PointerEvent) => {
        dragRef.current = {
          isDragging: true,
          startX: e.clientX,
          startY: e.clientY,
          lastX: e.clientX,
          lastY: e.clientY,
          moved: false,
        };
      };
      pointerMoveHandler = (e: PointerEvent) => {
        if (!dragRef.current.isDragging || !worldRef.current) return;
        const dx = e.clientX - dragRef.current.lastX;
        const dy = e.clientY - dragRef.current.lastY;
        if (
          Math.abs(e.clientX - dragRef.current.startX) > 3 ||
          Math.abs(e.clientY - dragRef.current.startY) > 3
        ) {
          dragRef.current.moved = true;
        }
        worldRef.current.x += dx;
        worldRef.current.y += dy;
        dragRef.current.lastX = e.clientX;
        dragRef.current.lastY = e.clientY;
      };
      pointerUpHandler = () => {
        dragRef.current.isDragging = false;
      };
      canvasRef.current.addEventListener('pointerdown', pointerDownHandler);
      window.addEventListener('pointermove', pointerMoveHandler);
      window.addEventListener('pointerup', pointerUpHandler);

      // 主循环：Layer 模式 lerp + Graph 模式力学模拟 + hover 浮层跟随
      app.ticker.add(() => {
        const mode = viewModeRef.current;
        const snapshot = graphSnapshotRef.current;

        if (mode === 'graph' && snapshot) {
          // ---- 力导向模拟 ----
          // 1. 节点间斥力(O(n²),n<=200 可接受)
          const entries = Array.from(nodeGraphicsRef.current.entries());
          for (let i = 0; i < entries.length; i++) {
            const [, a] = entries[i];
            if (a.pinned) continue;
            for (let j = i + 1; j < entries.length; j++) {
              const [, b] = entries[j];
              const dx = a.graphics.x - b.graphics.x;
              const dy = a.graphics.y - b.graphics.y;
              const distSq = dx * dx + dy * dy + 0.01;
              const dist = Math.sqrt(distSq);
              // 斥力 ~ 1/dist,clamp 防止过近时爆炸
              const force = Math.min(800 / distSq, 4);
              const fx = (dx / dist) * force;
              const fy = (dy / dist) * force;
              a.vx += fx;
              a.vy += fy;
              if (!b.pinned) {
                b.vx -= fx;
                b.vy -= fy;
              }
            }
          }
          // 2. 边弹簧力(Hooke: 拉向理想长度 80)
          for (const edge of snapshot.edges) {
            const a = nodeGraphicsRef.current.get(edge.src_id);
            const b = nodeGraphicsRef.current.get(edge.dst_id);
            if (!a || !b) continue;
            const dx = b.graphics.x - a.graphics.x;
            const dy = b.graphics.y - a.graphics.y;
            const dist = Math.sqrt(dx * dx + dy * dy) + 0.01;
            const ideal = 80;
            const k = 0.02 * edge.weight;
            const fx = (dx / dist) * (dist - ideal) * k;
            const fy = (dy / dist) * (dist - ideal) * k;
            if (!a.pinned) {
              a.vx += fx;
              a.vy += fy;
            }
            if (!b.pinned) {
              b.vx -= fx;
              b.vy -= fy;
            }
          }
          // 3. 向心力(拉回原点,防止图飘走) + 阻尼 + 位置更新
          for (const [, ng] of nodeGraphicsRef.current) {
            if (ng.pinned) continue;
            ng.vx -= ng.graphics.x * 0.001;
            ng.vy -= ng.graphics.y * 0.001;
            ng.vx *= 0.85; // 阻尼
            ng.vy *= 0.85;
            ng.graphics.x += ng.vx;
            ng.graphics.y += ng.vy;
            ng.halo.x = ng.graphics.x;
            ng.halo.y = ng.graphics.y;
          }
          // 4. 重绘边
          const eg = edgesRef.current;
          if (eg) {
            eg.clear();
            for (const edge of snapshot.edges) {
              const a = nodeGraphicsRef.current.get(edge.src_id);
              const b = nodeGraphicsRef.current.get(edge.dst_id);
              if (!a || !b) continue;
              const color = hexToNumber(DIM_COLORS[edge.dimension] ?? '#666666');
              eg.moveTo(a.graphics.x, a.graphics.y);
              eg.lineTo(b.graphics.x, b.graphics.y);
              eg.stroke({ width: 1.2, color, alpha: 0.4 + edge.weight * 0.4 });
            }
          }
        } else {
          // ---- Layer 模式: lerp 到目标位置 ----
          nodeGraphicsRef.current.forEach((ng) => {
            ng.graphics.x += (ng.targetX - ng.graphics.x) * 0.1;
            ng.graphics.y += (ng.targetY - ng.graphics.y) * 0.1;
            ng.alpha += (ng.targetAlpha - ng.alpha) * 0.15;
            ng.graphics.alpha = ng.alpha;
            ng.halo.x = ng.graphics.x;
            ng.halo.y = ng.graphics.y;
          });
        }

        // hover 浮层位置跟随节点（>200 节点时节流 100ms）
        if (hoveredIdRef.current && hoverDivRef.current) {
          const ng = nodeGraphicsRef.current.get(hoveredIdRef.current);
          if (ng) {
            const now = performance.now();
            const throttle = nodesCountRef.current > 200 ? 100 : 0;
            if (now - lastHoverUpdateRef.current >= throttle) {
              lastHoverUpdateRef.current = now;
              const pos = ng.graphics.getGlobalPosition();
              hoverDivRef.current.style.left = `${pos.x}px`;
              hoverDivRef.current.style.top = `${pos.y}px`;
            }
          }
        }
      });

      // 响应式：容器尺寸变化时重设 renderer 与 world 中心
      resizeObserver = new ResizeObserver(() => {
        if (!appRef.current || !containerRef.current) return;
        const w = containerRef.current.clientWidth;
        const h = containerRef.current.clientHeight;
        appRef.current.renderer.resize(w, h);
        if (worldRef.current) {
          worldRef.current.x = w / 2;
          worldRef.current.y = h / 2;
        }
      });
      resizeObserver.observe(containerRef.current);

      setAppReady(true);
    };

    init();

    return () => {
      destroyed = true;
      if (wheelHandler && canvasRef.current) {
        canvasRef.current.removeEventListener('wheel', wheelHandler);
      }
      if (pointerDownHandler && canvasRef.current) {
        canvasRef.current.removeEventListener('pointerdown', pointerDownHandler);
      }
      if (pointerMoveHandler) {
        window.removeEventListener('pointermove', pointerMoveHandler);
      }
      if (pointerUpHandler) {
        window.removeEventListener('pointerup', pointerUpHandler);
      }
      if (resizeObserver) {
        resizeObserver.disconnect();
      }
      if (appRef.current) {
        appRef.current.destroy(true, { children: true });
        appRef.current = null;
      }
      worldRef.current = null;
      nodesContainerRef.current = null;
      edgesRef.current = null;
      nodeGraphicsRef.current.clear();
      setAppReady(false);
    };
  }, []);

  // 数据/视图模式/快照变化时重建节点
  useEffect(() => {
    const container = nodesContainerRef.current;
    if (!container) return;

    // clear + 重建
    container.removeChildren();
    nodeGraphicsRef.current.clear();
    if (edgesRef.current) edgesRef.current.clear();

    if (viewMode === 'graph' && graphSnapshot) {
      // ---- Graph View: 用 GraphSnapshot 的 nodes ----
      graphSnapshot.nodes.forEach((gn, idx) => {
        const isRoot = gn.id === graphSnapshot.root_id;
        const angle = (idx * 2.4) % (Math.PI * 2); // 黄金角分散初始位置
        const initR = isRoot ? 0 : 60 + gn.depth * 40;
        const targetX = isRoot ? 0 : initR * Math.cos(angle);
        const targetY = isRoot ? 0 : initR * Math.sin(angle);

        const baseSize = 4 + gn.importance * 12;
        const color = hexToNumber(LAYER_COLORS[gn.layer] ?? '#9CA3AF');

        const halo = new Graphics();
        halo.circle(0, 0, baseSize + 6);
        halo.stroke({ width: 2, color: isRoot ? 0xffd700 : 0xffffff, alpha: 0.8 });
        halo.visible = false;

        const g = new Graphics();
        g.circle(0, 0, baseSize);
        g.fill({ color, alpha: 0.85 });
        g.eventMode = 'static';
        g.cursor = 'pointer';

        const ng: NodeGraphic = {
          graphics: g,
          halo,
          targetX,
          targetY,
          baseSize,
          alpha: 1,
          targetAlpha: 1,
          vx: 0,
          vy: 0,
          pinned: isRoot, // 根节点钉在原点
        };

        g.x = targetX;
        g.y = targetY;
        halo.x = targetX;
        halo.y = targetY;
        g.alpha = 1;

        g.on('pointerdown', (e) => {
          e.stopPropagation();
        });
        g.on('pointertap', (e) => {
          e.stopPropagation();
          if (dragRef.current.moved) return;
          // Graph View 点击 → 以该节点为根重新查询
          if (viewModeRef.current === 'graph') {
            loadGraph(gn.id);
          }
          setSelectedId(selectedIdRef.current === gn.id ? null : gn.id);
        });
        g.on('pointerover', () => {
          if (hoveredIdRef.current !== gn.id) {
            setHoveredId(gn.id);
            lastHoverUpdateRef.current = 0;
          }
        });
        g.on('pointerout', () => {
          if (hoveredIdRef.current === gn.id) {
            setHoveredId(null);
          }
        });

        container.addChild(halo);
        container.addChild(g);
        nodeGraphicsRef.current.set(gn.id, ng);
      });
      return;
    }

    // ---- Layer View: 原有同心圆布局 ----
    nodes.forEach((node) => {
      const angle = ((hashCode(node.id) % 360) * Math.PI) / 180;
      const r = LAYER_RADII[node.layer];
      // 层内分散扰动：基于 id hash 的 -10..10 偏移，避免节点重叠
      const scatter = (hashCode(node.id + 'scatter') % 21) - 10;
      const targetX = (r + scatter) * Math.cos(angle);
      const targetY = (r + scatter) * Math.sin(angle);

      const baseSize = 4 + node.importance * 12;
      const size = node.compressed ? baseSize * 0.6 : baseSize;
      const color = hexToNumber(LAYER_COLORS[node.layer]);

      // 选中态外发光圆环
      const halo = new Graphics();
      halo.circle(0, 0, size + 6);
      halo.stroke({ width: 2, color: 0xffffff, alpha: 0.8 });
      halo.visible = false;

      // 主体节点
      const g = new Graphics();
      g.circle(0, 0, size);
      g.fill({ color, alpha: node.compressed ? 0.25 : 0.85 });
      g.eventMode = 'static';
      g.cursor = 'pointer';

      const isNew = newNodeIds.has(node.id);
      const ng: NodeGraphic = {
        graphics: g,
        halo,
        targetX,
        targetY,
        baseSize: size,
        alpha: isNew ? 0 : 1,
        targetAlpha: 1,
        vx: 0,
        vy: 0,
        pinned: false,
      };

      // 初始位置直接放在目标点（非新节点避免动画跳变）
      g.x = targetX;
      g.y = targetY;
      halo.x = targetX;
      halo.y = targetY;
      g.alpha = ng.alpha;

      // 节点事件：点击选中、hover 显示浮层
      g.on('pointerdown', (e) => {
        e.stopPropagation();
      });
      g.on('pointertap', (e) => {
        e.stopPropagation();
        // 拖拽发生过则不视为点击
        if (dragRef.current.moved) return;
        setSelectedId(selectedIdRef.current === node.id ? null : node.id);
      });
      g.on('pointerover', () => {
        if (hoveredIdRef.current !== node.id) {
          setHoveredId(node.id);
          lastHoverUpdateRef.current = 0; // 立即更新一次位置
        }
      });
      g.on('pointerout', () => {
        if (hoveredIdRef.current === node.id) {
          setHoveredId(null);
        }
      });

      container.addChild(halo);
      container.addChild(g);
      nodeGraphicsRef.current.set(node.id, ng);
    });
  }, [nodes, newNodeIds, appReady, viewMode, graphSnapshot, loadGraph]);

  // 选中态变化时切换 halo 可见性
  useEffect(() => {
    nodeGraphicsRef.current.forEach((ng, id) => {
      ng.halo.visible = id === selectedId;
    });
  }, [selectedId]);

  // 六层记忆图谱 HTML 渲染（参考设计稿 preview.html：节点 + 连线动态渲染）
  useEffect(() => {
    const container = vizRef.current;
    if (!container) return;

    const render = () => {
      const W = container.clientWidth || 600;
      const H = 340;
      container.innerHTML = '';

      // 默认节点布局（参考 preview.html）
      const defaultVizNodes = [
        { x: W * 0.22, y: H * 0.55, s: 24, l: 'l0', t: 'L0' },
        { x: W * 0.4, y: H * 0.28, s: 28, l: 'l1', t: 'L1' },
        { x: W * 0.38, y: H * 0.72, s: 32, l: 'l2', t: 'L2' },
        { x: W * 0.58, y: H * 0.42, s: 36, l: 'l3', t: 'L3' },
        { x: W * 0.75, y: H * 0.25, s: 40, l: 'l4', t: 'L4' },
        { x: W * 0.73, y: H * 0.65, s: 44, l: 'l5', t: 'L5' },
        { x: W * 0.12, y: H * 0.22, s: 22, l: 'l0', t: 'L0' },
        { x: W * 0.3, y: H * 0.5, s: 26, l: 'l1', t: 'L1' },
        { x: W * 0.5, y: H * 0.78, s: 30, l: 'l2', t: 'L2' },
        { x: W * 0.65, y: H * 0.15, s: 24, l: 'l1', t: 'L1' },
        { x: W * 0.88, y: H * 0.55, s: 34, l: 'l3', t: 'L3' },
      ];

      // 若有真实记忆数据，按层级映射到 l0-l5；否则用默认节点
      let vizNodes: { x: number; y: number; s: number; l: string; t: string }[];
      if (nodes.length > 0) {
        vizNodes = nodes.slice(0, 12).map((n) => {
          const li = Math.min(parseInt(n.layer.slice(1), 10), 5);
          const angle = ((hashCode(n.id) % 360) * Math.PI) / 180;
          const r = 40 + li * 30;
          return {
            x: W / 2 + r * Math.cos(angle),
            y: H / 2 + r * Math.sin(angle),
            s: 24 + Math.min(Math.round(n.importance * 20), 20),
            l: 'l' + li,
            t: n.layer,
          };
        });
      } else {
        vizNodes = defaultVizNodes;
      }

      // 连线（随机连接，参考 preview.html）
      for (let i = 0; i < vizNodes.length; i++) {
        for (let j = i + 1; j < vizNodes.length; j++) {
          if (Math.random() > 0.55) continue;
          const a = vizNodes[i];
          const b = vizNodes[j];
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const dist = Math.sqrt(dx * dx + dy * dy);
          const angle = (Math.atan2(dy, dx) * 180) / Math.PI;
          const edge = document.createElement('div');
          edge.className = 'memory-edge';
          edge.style.left = a.x + 'px';
          edge.style.top = a.y + 'px';
          edge.style.width = dist + 'px';
          edge.style.transform = 'rotate(' + angle + 'deg)';
          container.appendChild(edge);
        }
      }

      // 节点
      vizNodes.forEach((n) => {
        const el = document.createElement('div');
        el.className = 'memory-node ' + n.l;
        el.style.left = n.x - n.s + 'px';
        el.style.top = n.y - n.s + 'px';
        el.style.width = n.s * 2 + 'px';
        el.style.height = n.s * 2 + 'px';
        el.textContent = n.t;
        container.appendChild(el);
      });
    };

    render();
    const ro = new ResizeObserver(render);
    ro.observe(container);
    return () => ro.disconnect();
  }, [nodes]);

  const selectedNode = nodes.find((n) => n.id === selectedId);
  const hoveredNode = nodes.find((n) => n.id === hoveredId);
  const hoveredGraphNode = graphSnapshot?.nodes.find((n) => n.id === hoveredId);

  // 维度筛选切换
  const toggleDim = (dim: RelationDimension) => {
    setActiveDims((prev) => {
      const next = new Set(prev);
      if (next.has(dim)) {
        next.delete(dim);
      } else {
        next.add(dim);
      }
      return next;
    });
  };

  // 最近提取的记忆卡片：优先用真实数据，为空时用设计稿默认卡片
  const memoryCards =
    nodes.length === 0
      ? DEFAULT_CARDS
      : nodes.slice(0, 6).map((n) => {
          const li = Math.min(parseInt(n.layer.slice(1), 10), 5);
          const meta = VIZ_LAYERS[li];
          return {
            layer: String(n.layer),
            label: meta.card,
            color: meta.color,
            dot: meta.dot,
            title: n.content.length > 40 ? n.content.slice(0, 40) + '…' : n.content,
            desc: n.summary || n.content,
          };
        });

  return (
    <div className="memory-map-container h-full flex flex-col bg-gray-950 text-white">
      {/* 页面头 */}
      <div className="page-header">
        <div>
          <div className="page-title">🧠 记忆系统</div>
          <div className="page-subtitle">
            L0-L5 六层记忆 · {nodes.length} {t('memoryMap.memories')}
          </div>
        </div>
        <div className="page-actions">
          <button className="tool-btn">🔄 反思</button>
          <button className="tool-btn" onClick={loadMemories}>
            📥 导入
          </button>
        </div>
      </div>

      {/* 视图切换 + 状态指示（保留现有功能） */}
      <div className="flex items-center gap-3 px-4 py-1.5 border-b border-gray-800 text-xs">
        <div className="flex bg-gray-800 rounded">
          <button
            data-testid="view-layer"
            onClick={() => setViewMode('layer')}
            className={`px-2 py-1 rounded transition-colors ${viewMode === 'layer' ? 'bg-blue-600 text-white' : 'text-gray-400 hover:text-white'}`}
          >
            {t('memoryMap.viewLayer')}
          </button>
          <button
            data-testid="view-graph"
            onClick={() => setViewMode('graph')}
            className={`px-2 py-1 rounded transition-colors ${viewMode === 'graph' ? 'bg-blue-600 text-white' : 'text-gray-400 hover:text-white'}`}
          >
            {t('memoryMap.viewGraph')}
          </button>
        </div>
        {loading && <span className="text-gray-500">{t('memoryMap.loading')}</span>}
        {graphLoading && <span className="text-gray-500">{t('memoryMap.graphLoading')}</span>}
        <span className="text-gray-500">
          {viewMode === 'graph' && graphSnapshot
            ? t('memoryMap.nodeEdgeCount', { nodes: graphSnapshot.nodes.length, edges: graphSnapshot.edges.length })
            : `${nodes.length} ${t('memoryMap.memories')}`}
        </span>
        <button
          onClick={loadMemories}
          className="text-gray-400 hover:text-white transition-colors"
          title={t('memoryMap.refresh')}
        >
          ↻
        </button>
      </div>

      {/* 维度筛选(仅 Graph View，保留现有功能) */}
      {viewMode === 'graph' && (
        <div className="flex flex-wrap items-center gap-3 px-4 py-1.5 border-b border-gray-800 text-xs">
          <span className="text-gray-500">{t('memoryMap.dimensionLabel')}</span>
          {ALL_DIMS.map((dim) => (
            <label
              key={dim}
              data-testid={`dim-${dim}`}
              className="flex items-center gap-1 cursor-pointer select-none"
            >
              <input
                type="checkbox"
                checked={activeDims.has(dim)}
                onChange={() => toggleDim(dim)}
                className="w-3 h-3"
              />
              <span
                className="w-2 h-2 rounded-full inline-block"
                style={{ backgroundColor: DIM_COLORS[dim] }}
              />
              <span className="text-gray-300">{dimLabel(dim)}</span>
            </label>
          ))}
          {graphSnapshot?.truncated && (
            <span className="text-yellow-600" data-testid="truncated-warning">
              {t('memoryMap.truncated')}
            </span>
          )}
          {graphSnapshot && graphSnapshot.edges.length === 0 && (
            <span className="text-gray-500" data-testid="empty-edges">
              {t('memoryMap.noEdges')}
            </span>
          )}
        </div>
      )}

      <div className="page-body">
        {/* 六层记忆图谱（HTML 节点可视化，参考设计稿 preview.html） */}
        <div style={{ fontSize: '13.5px', fontWeight: 650, marginBottom: 10 }}>六层记忆图谱</div>
        <div className="memory-viz" ref={vizRef} style={{ height: 340 }} />
        <div className="memory-legend">
          {VIZ_LAYERS.map((v) => (
            <span key={v.layer}>
              <span className="layer-dot" style={{ background: v.dot }}></span>
              {v.layer} {v.legend}
            </span>
          ))}
        </div>

        {/* 交互式图谱（PixiJS WebGL，保留现有图谱交互功能） */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            margin: '22px 0 10px',
          }}
        >
          <span style={{ fontSize: '13.5px', fontWeight: 650 }}>交互式图谱</span>
        </div>
        <div
          ref={containerRef}
          className="overflow-hidden relative"
          style={{ height: 320, background: '#0a0a0a', borderRadius: 12 }}
        >
          <canvas ref={canvasRef} className="block w-full h-full" style={{ touchAction: 'none' }} />

          {/* Hover 摘要浮层（HTML absolute，位置由 ticker 直接更新 style）*/}
          {hoveredNode && !selectedId && viewMode === 'layer' && (
            <div
              ref={hoverDivRef}
              className="absolute bg-gray-900 border border-gray-700 rounded px-3 py-2 text-xs max-w-xs pointer-events-none"
              style={{ left: '50%', top: '50%', transform: 'translate(-50%, -120%)' }}
            >
              <div className="flex items-center gap-2 mb-1">
                <span
                  className="w-2 h-2 rounded-full"
                  style={{ backgroundColor: LAYER_COLORS[hoveredNode.layer] }}
                />
                <span className="text-gray-400">
                  {hoveredNode.layer} · {layerLabel(hoveredNode.layer)}
                </span>
              </div>
              <div className="text-gray-200 line-clamp-3">
                {hoveredNode.summary || hoveredNode.content}
              </div>
            </div>
          )}
          {hoveredGraphNode && !selectedId && viewMode === 'graph' && (
            <div
              ref={hoverDivRef}
              className="absolute bg-gray-900 border border-gray-700 rounded px-3 py-2 text-xs max-w-xs pointer-events-none"
              style={{ left: '50%', top: '50%', transform: 'translate(-50%, -120%)' }}
            >
              <div className="flex items-center gap-2 mb-1">
                <span
                  className="w-2 h-2 rounded-full"
                  style={{ backgroundColor: LAYER_COLORS[hoveredGraphNode.layer] }}
                />
                <span className="text-gray-400">
                  {hoveredGraphNode.layer} · depth {hoveredGraphNode.depth}
                  {hoveredGraphNode.id === graphSnapshot?.root_id && t('memoryMap.rootNode')}
                </span>
              </div>
              <div className="text-gray-200 line-clamp-3">{hoveredGraphNode.summary}</div>
            </div>
          )}
        </div>

        {/* 选中节点详情（保留现有功能） */}
        {selectedNode && viewMode === 'layer' && (
          <div className="border-t border-gray-800 p-4 bg-gray-900 mt-3" style={{ borderRadius: 8 }}>
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2">
                <span
                  className="w-3 h-3 rounded-full"
                  style={{ backgroundColor: LAYER_COLORS[selectedNode.layer] }}
                />
                <span className="text-sm font-medium">
                  {selectedNode.layer} · {layerLabel(selectedNode.layer)}
                </span>
                {selectedNode.compressed && (
                  <span className="text-xs px-2 py-0.5 rounded bg-red-900 text-red-300">
                    {t('memoryMap.compressed')}
                  </span>
                )}
              </div>
              <button onClick={() => setSelectedId(null)} className="text-gray-500 hover:text-white">
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
              <span>
                {t('memoryMap.importance')}: {selectedNode.importance.toFixed(2)}
              </span>
              <span>
                {t('memoryMap.created')}:{' '}
                {new Date(selectedNode.created_at * 1000).toLocaleString('zh-CN')}
              </span>
            </div>
          </div>
        )}

        {/* 最近提取的记忆卡片 */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            margin: '22px 0 10px',
          }}
        >
          <span style={{ fontSize: '13.5px', fontWeight: 650 }}>最近提取的记忆</span>
        </div>
        <div className="memory-card-grid">
          {memoryCards.map((card, i) => (
            <div className="memory-card" key={i}>
              <div className="memory-card-layer" style={{ color: card.color }}>
                <span className="layer-dot" style={{ background: card.dot }}></span>
                {card.layer} · {card.label}
              </div>
              <div className="memory-card-title">{card.title}</div>
              <div className="memory-card-desc">{card.desc}</div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
