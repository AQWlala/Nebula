/**
 * 记忆地图 - 7 层记忆架构可视化 (PixiJS WebGL 渲染)
 *
 * P1-6: 核心品牌视觉 — 7 层同心圆记忆架构的图形化呈现
 * - L0（感官缓冲）→ L7（奇点核心）
 * - 记忆节点：大小反映重要性，颜色反映层级
 * - 交互：点击展开内容，hover 显示摘要
 * - 动画：新记忆淡入，被压缩时缩小+变灰淡出
 * - T-S5-B-02: SVG → PixiJS 迁移，支持 1000+ 节点流畅渲染、缩放/拖拽
 */
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { Application, Container, Graphics, Text } from 'pixi.js';
import { nebulaAPI, type Memory, type Layer } from '../lib/tauri';
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

/** v1.0 i18n: convert const Record map to function form so labels re-render on locale switch. */
const layerLabel = (l: Layer): string => t(`memoryMap.layer.${l}`);

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

/** 将 #RRGGBB 转换为 0xRRGGBB 数字 */
function hexToNumber(hex: string): number {
  return parseInt(hex.replace('#', ''), 16);
}

/** 单个节点的 PixiJS 资源 */
interface NodeGraphic {
  graphics: Graphics;
  halo: Graphics;
  targetX: number;
  targetY: number;
  baseSize: number;
  alpha: number;
  targetAlpha: number;
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

  // PixiJS 资源引用
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const hoverDivRef = useRef<HTMLDivElement>(null);
  const appRef = useRef<Application | null>(null);
  const worldRef = useRef<Container | null>(null);
  const nodesContainerRef = useRef<Container | null>(null);
  const nodeGraphicsRef = useRef<Map<string, NodeGraphic>>(new Map());

  // 同步 state 到 ref（供 PixiJS 事件回调读取最新值，避免闭包陈旧）
  const selectedIdRef = useRef<string | null>(null);
  const hoveredIdRef = useRef<string | null>(null);
  const nodesCountRef = useRef(0);
  useEffect(() => { selectedIdRef.current = selectedId; }, [selectedId]);
  useEffect(() => { hoveredIdRef.current = hoveredId; }, [hoveredId]);
  useEffect(() => { nodesCountRef.current = nodes.length; }, [nodes]);

  // 拖拽状态 + hover 节流时间戳
  const dragRef = useRef<DragState>({
    isDragging: false, startX: 0, startY: 0, lastX: 0, lastY: 0, moved: false,
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

      // 7 层同心圆
      const rings = new Graphics();
      (Object.keys(LAYER_RADII) as Layer[]).forEach(layer => {
        const r = LAYER_RADII[layer];
        const color = hexToNumber(LAYER_COLORS[layer]);
        rings.circle(0, 0, r);
        rings.stroke({ width: 1, color, alpha: 0.35 });
      });
      world.addChild(rings);

      // 中心奇点
      const centerGfx = new Graphics();
      centerGfx.circle(0, 0, 22);
      centerGfx.fill({ color: 0xFFD700, alpha: 0.9 });
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
        if (Math.abs(e.clientX - dragRef.current.startX) > 3 ||
            Math.abs(e.clientY - dragRef.current.startY) > 3) {
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

      // 主循环：节点位置/alpha lerp 平滑过渡 + hover 浮层位置跟随
      app.ticker.add(() => {
        nodeGraphicsRef.current.forEach((ng) => {
          // 力导向简化：目标位置 = 层半径 + 层内分散扰动；lerp 0.1 平滑过渡
          ng.graphics.x += (ng.targetX - ng.graphics.x) * 0.1;
          ng.graphics.y += (ng.targetY - ng.graphics.y) * 0.1;
          // 新节点淡入：alpha 0→1 lerp (约 300ms)
          ng.alpha += (ng.targetAlpha - ng.alpha) * 0.15;
          ng.graphics.alpha = ng.alpha;
          // 外发光圆环跟随主体
          ng.halo.x = ng.graphics.x;
          ng.halo.y = ng.graphics.y;
        });

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
      nodeGraphicsRef.current.clear();
      setAppReady(false);
    };
  }, []);

  // 数据/应用就绪变化时重建节点
  useEffect(() => {
    const container = nodesContainerRef.current;
    if (!container) return;

    // clear + 重建
    container.removeChildren();
    nodeGraphicsRef.current.clear();

    nodes.forEach(node => {
      const angle = (hashCode(node.id) % 360) * Math.PI / 180;
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
      halo.stroke({ width: 2, color: 0xFFFFFF, alpha: 0.8 });
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
      };

      // 初始位置直接放在目标点（非新节点避免动画跳变）
      g.x = targetX;
      g.y = targetY;
      halo.x = targetX;
      halo.y = targetY;
      g.alpha = ng.alpha;

      // 节点事件：点击选中、hover 显示浮层
      g.on('pointerdown', (e) => { e.stopPropagation(); });
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
  }, [nodes, newNodeIds, appReady]);

  // 选中态变化时切换 halo 可见性
  useEffect(() => {
    nodeGraphicsRef.current.forEach((ng, id) => {
      ng.halo.visible = id === selectedId;
    });
  }, [selectedId]);

  const selectedNode = nodes.find(n => n.id === selectedId);
  const hoveredNode = nodes.find(n => n.id === hoveredId);

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

      {/* WebGL 画布区 */}
      <div ref={containerRef} className="flex-1 overflow-hidden relative">
        <canvas
          ref={canvasRef}
          className="block w-full h-full"
          style={{ touchAction: 'none' }}
        />

        {/* Hover 摘要浮层（HTML absolute，位置由 ticker 直接更新 style）*/}
        {hoveredNode && !selectedId && (
          <div
            ref={hoverDivRef}
            className="absolute bg-gray-900 border border-gray-700 rounded px-3 py-2 text-xs max-w-xs pointer-events-none"
            style={{
              left: '50%',
              top: '50%',
              transform: 'translate(-50%, -120%)',
            }}
          >
            <div className="flex items-center gap-2 mb-1">
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: LAYER_COLORS[hoveredNode.layer] }}
              />
              <span className="text-gray-400">{hoveredNode.layer} · {layerLabel(hoveredNode.layer)}</span>
            </div>
            <div className="text-gray-200 line-clamp-3">{hoveredNode.summary || hoveredNode.content}</div>
          </div>
        )}
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
                {selectedNode.layer} · {layerLabel(selectedNode.layer)}
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
            <span className="text-gray-600">{layerLabel(layer)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
