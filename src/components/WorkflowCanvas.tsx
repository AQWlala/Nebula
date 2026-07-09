/**
 * T-E-S-10: WorkflowCanvas — 可编排工作流画布。
 *
 * ## 功能
 * - 4 类节点:Agent / Task / Condition / IO(从节点面板点击添加)
 * - 拖拽节点移动位置(鼠标按下节点体 → 拖动 → 释放)
 * - 拖拽连线建立数据流 / 控制流(按下输出端口 → 拖到目标输入端口)
 * - 条件节点支持 true / false 双分支出口
 * - 属性面板:选中节点 / 连线后编辑配置
 * - 工具栏:新建 / 保存 / 加载 / 运行 / 停止 / 导入 / 导出
 * - 纯 Preact + SVG 实现,不引入 reactflow / dagre 等新依赖
 *
 * ## 持久化策略
 * - 优先调用后端 Tauri 命令(workflow_save / workflow_load / workflow_list)
 *   通过 invokeTauri(失败静默返回 null),后端命令未注册时自动降级到
 *   localStorage,保证前端在无后端支持时也可独立使用。
 *
 * ## 执行策略
 * - 运行时将工作流文档转换为 SwarmTask(workflowToSwarmTask),
 *   调用已注册的 nebulaAPI.swarmExecute() 触发蜂群执行。
 * - 停止时调用 swarmCancel(taskId) 取消正在执行的任务。
 */
import { useState, useRef, useCallback, useMemo, useEffect } from 'preact/hooks';
import { nebulaAPI, invokeTauri, swarmCancel } from '../lib/tauri';
import { toast } from './Toast';
import {
  type WorkflowDocument,
  type WorkflowNode,
  type WorkflowEdge,
  type WorkflowNodeType,
  type EdgePort,
  type AgentKind,
  type IoDirection,
  type IoFormat,
  type RunStatus,
  NODE_WIDTH,
  NODE_HEIGHT,
  createNode,
  validateEdge,
  makeEdgeId,
  workflowToSwarmTask,
  emptyDocument,
  serializeDocument,
  parseDocument,
  outputPortPos,
  inputPortPos,
  bezierPath,
} from './workflow/types';
import { workflowStrings } from './workflow/i18n';
import './workflow/workflow.css';

/** 画布可视区域固定尺寸(可滚动)。 */
const CANVAS_W = 2400;
const CANVAS_H = 1600;

/** 节点类型 → 主题色 + 图标。 */
const NODE_THEME: Record<
  WorkflowNodeType,
  { color: string; bg: string; icon: string }
> = {
  agent: { color: '#3b82f6', bg: 'rgba(59,130,246,0.12)', icon: '🐝' },
  task: { color: '#10b981', bg: 'rgba(16,185,129,0.12)', icon: '⚙️' },
  condition: { color: '#f59e0b', bg: 'rgba(245,158,11,0.12)', icon: '🔀' },
  io: { color: '#8b5cf6', bg: 'rgba(139,92,246,0.12)', icon: '📥' },
};

/** 拖拽中的节点信息。 */
interface DraggingNode {
  id: string;
  offsetX: number;
  offsetY: number;
}

/** 正在建立的连线起点。 */
interface Connecting {
  sourceId: string;
  sourcePort: EdgePort;
  x: number;
  y: number;
}

/** localStorage 工作流索引键。 */
const LS_INDEX_KEY = 'nebula.workflow.index';
/** localStorage 单个工作流键前缀。 */
const LS_DOC_PREFIX = 'nebula.workflow.doc.';

/** 读取 localStorage 工作流名称列表。 */
function lsList(): string[] {
  try {
    const raw = localStorage.getItem(LS_INDEX_KEY);
    return raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    return [];
  }
}

/** 写入 localStorage 工作流索引。 */
function lsWriteIndex(names: string[]): void {
  try {
    localStorage.setItem(LS_INDEX_KEY, JSON.stringify(names));
  } catch {
    /* 忽略配额 / 隐私模式错误 */
  }
}

/** 保存单个工作流到 localStorage。 */
function lsSave(doc: WorkflowDocument): void {
  const names = lsList();
  if (!names.includes(doc.name)) {
    names.push(doc.name);
    lsWriteIndex(names);
  }
  try {
    localStorage.setItem(LS_DOC_PREFIX + doc.name, serializeDocument(doc));
  } catch {
    /* 忽略 */
  }
}

/** 从 localStorage 加载单个工作流。 */
function lsLoad(name: string): WorkflowDocument | null {
  try {
    const raw = localStorage.getItem(LS_DOC_PREFIX + name);
    return raw ? parseDocument(raw) : null;
  } catch {
    return null;
  }
}

export function WorkflowCanvas() {
  const s = workflowStrings();
  const [doc, setDoc] = useState<WorkflowDocument>(() => emptyDocument());
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [dragging, setDragging] = useState<DraggingNode | null>(null);
  const [connecting, setConnecting] = useState<Connecting | null>(null);
  const [runStatus, setRunStatus] = useState<RunStatus>('idle');
  const [runResult, setRunResult] = useState<string | null>(null);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [loadList, setLoadList] = useState<string[]>([]);
  const [showLoadMenu, setShowLoadMenu] = useState(false);

  /** 序号计数器,用于生成节点 / 边 id。 */
  const seqRef = useRef(1);
  const canvasRef = useRef<HTMLDivElement | null>(null);

  /** 当前选中的节点对象。 */
  const selectedNode = useMemo(
    () => doc.nodes.find((n) => n.id === selectedNodeId) ?? null,
    [doc.nodes, selectedNodeId]
  );
  /** 当前选中的边对象。 */
  const selectedEdge = useMemo(
    () => doc.edges.find((e) => e.id === selectedEdgeId) ?? null,
    [doc.edges, selectedEdgeId]
  );

  /** 将浏览器坐标转换为画布内坐标(考虑滚动与缩放)。 */
  const toCanvasCoord = useCallback((clientX: number, clientY: number) => {
    const el = canvasRef.current;
    if (!el) return { x: 0, y: 0 };
    const rect = el.getBoundingClientRect();
    return {
      x: clientX - rect.left + el.scrollLeft,
      y: clientY - rect.top + el.scrollTop,
    };
  }, []);

  // ---- 节点操作 ----

  /** 从面板添加新节点(放在画布可见区中心)。 */
  const addNode = useCallback(
    (type: WorkflowNodeType) => {
      const el = canvasRef.current;
      const cx = el ? el.scrollLeft + el.clientWidth / 2 - NODE_WIDTH / 2 : 200;
      const cy = el ? el.scrollTop + el.clientHeight / 2 - NODE_HEIGHT / 2 : 200;
      // 加少量随机偏移避免堆叠。
      const jitter = (seqRef.current % 5) * 24;
      const node = createNode(type, cx + jitter, cy + jitter, seqRef.current++);
      setDoc((d) => ({
        ...d,
        nodes: [...d.nodes, node],
        updated_at: Date.now(),
      }));
      setSelectedNodeId(node.id);
      setSelectedEdgeId(null);
    },
    []
  );

  /** 更新节点位置(拖拽时调用)。 */
  const moveNode = useCallback((id: string, x: number, y: number) => {
    setDoc((d) => ({
      ...d,
      nodes: d.nodes.map((n) => (n.id === id ? { ...n, x, y } : n)),
      updated_at: Date.now(),
    }));
  }, []);

  /** 更新节点标题。 */
  const updateNodeTitle = useCallback((id: string, title: string) => {
    setDoc((d) => ({
      ...d,
      nodes: d.nodes.map((n) => (n.id === id ? { ...n, title } : n)),
      updated_at: Date.now(),
    }));
  }, []);

  /** 更新节点配置(局部合并)。 */
  const updateNodeConfig = useCallback(
    (id: string, patch: Record<string, unknown>) => {
      setDoc((d) => ({
        ...d,
        nodes: d.nodes.map((n) =>
          n.id === id
            ? { ...n, config: { ...n.config, ...patch } as WorkflowNode['config'] }
            : n
        ),
        updated_at: Date.now(),
      }));
    },
    []
  );

  /** 删除节点(同时删除关联的边)。 */
  const deleteNode = useCallback((id: string) => {
    setDoc((d) => ({
      ...d,
      nodes: d.nodes.filter((n) => n.id !== id),
      edges: d.edges.filter((e) => e.source !== id && e.target !== id),
      updated_at: Date.now(),
    }));
    setSelectedNodeId((cur) => (cur === id ? null : cur));
  }, []);

  // ---- 连线操作 ----

  /** 在输出端口按下,开始拖拽连线。 */
  const startConnect = useCallback(
    (sourceId: string, sourcePort: EdgePort, clientX: number, clientY: number) => {
      const pos = toCanvasCoord(clientX, clientY);
      setConnecting({ sourceId, sourcePort, x: pos.x, y: pos.y });
      setSelectedNodeId(null);
      setSelectedEdgeId(null);
    },
    [toCanvasCoord]
  );

  /** 在目标节点上释放,尝试建立连线。 */
  const finishConnect = useCallback(
    (targetId: string) => {
      setConnecting((cur) => {
        if (!cur) return null;
        const err = validateEdge(cur.sourceId, targetId, cur.sourcePort, doc.edges);
        if (err) {
          const msgMap: Record<string, string> = {
            self_loop: s.error.selfLoop,
            duplicate: s.error.duplicate,
            cycle: s.error.cycle,
          };
          toast.error(msgMap[err] ?? err);
          return null;
        }
        const edge: WorkflowEdge = {
          id: makeEdgeId(seqRef.current++),
          source: cur.sourceId,
          target: targetId,
          sourcePort: cur.sourcePort,
          label: '',
        };
        setDoc((d) => ({
          ...d,
          edges: [...d.edges, edge],
          updated_at: Date.now(),
        }));
        return null;
      });
    },
    [doc.edges, s]
  );

  /** 更新边标签。 */
  const updateEdgeLabel = useCallback((id: string, label: string) => {
    setDoc((d) => ({
      ...d,
      edges: d.edges.map((e) => (e.id === id ? { ...e, label } : e)),
      updated_at: Date.now(),
    }));
  }, []);

  /** 删除边。 */
  const deleteEdge = useCallback((id: string) => {
    setDoc((d) => ({
      ...d,
      edges: d.edges.filter((e) => e.id !== id),
      updated_at: Date.now(),
    }));
    setSelectedEdgeId((cur) => (cur === id ? null : cur));
  }, []);

  // ---- 全局鼠标事件(拖拽节点 / 连线) ----

  useEffect(() => {
    if (!dragging && !connecting) return;

    const onMove = (ev: MouseEvent) => {
      if (dragging) {
        const pos = toCanvasCoord(ev.clientX, ev.clientY);
        moveNode(dragging.id, pos.x - dragging.offsetX, pos.y - dragging.offsetY);
      } else if (connecting) {
        const pos = toCanvasCoord(ev.clientX, ev.clientY);
        setConnecting({ ...connecting, x: pos.x, y: pos.y });
      }
    };
    const onUp = () => {
      setDragging(null);
      setConnecting(null);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, [dragging, connecting, moveNode, toCanvasCoord]);

  // ---- 工具栏操作 ----

  /** 新建空白文档。 */
  const newDoc = useCallback(() => {
    setDoc(emptyDocument());
    setSelectedNodeId(null);
    setSelectedEdgeId(null);
    setRunStatus('idle');
    setRunResult(null);
    seqRef.current = 1;
  }, []);

  /** 保存文档(后端优先,降级 localStorage)。 */
  const saveDoc = useCallback(async () => {
    const updated = { ...doc, updated_at: Date.now() };
    // 优先尝试后端 Tauri 命令(未注册时 invokeTauri 返回 null,静默降级)。
    const ok = await invokeTauri<boolean>('workflow_save', {
      name: updated.name,
      documentJson: serializeDocument(updated),
    });
    if (ok === true) {
      toast.success(s.status.saved);
      setDoc(updated);
      return;
    }
    // 降级 localStorage。
    lsSave(updated);
    toast.success(s.status.saved);
    setDoc(updated);
  }, [doc, s]);

  /** 加载文档列表(后端优先,降级 localStorage)。 */
  const loadListFn = useCallback(async () => {
    const remote = await invokeTauri<string[]>('workflow_list');
    if (remote && remote.length >= 0) {
      setLoadList(remote);
    } else {
      setLoadList(lsList());
    }
    setShowLoadMenu(true);
  }, []);

  /** 加载指定名称的文档。 */
  const loadDoc = useCallback(
    async (name: string) => {
      const remote = await invokeTauri<string>('workflow_load', { name });
      if (remote) {
        const parsed = parseDocument(remote);
        if (parsed) {
          setDoc(parsed);
          setSelectedNodeId(null);
          setSelectedEdgeId(null);
          setShowLoadMenu(false);
          toast.success(s.status.loaded);
          return;
        }
      }
      const local = lsLoad(name);
      if (local) {
        setDoc(local);
        setSelectedNodeId(null);
        setSelectedEdgeId(null);
        setShowLoadMenu(false);
        toast.success(s.status.loaded);
      } else {
        toast.error(s.status.loadFailed);
      }
    },
    [s]
  );

  /** 运行工作流(转换为 SwarmTask 调用 swarm_execute)。 */
  const runDoc = useCallback(async () => {
    if (doc.nodes.length === 0) {
      toast.error(s.error.empty);
      return;
    }
    setRunStatus('running');
    setRunResult(s.run.preparing);
    try {
      const task = workflowToSwarmTask(doc);
      const result = await nebulaAPI.swarmExecute({
        description: task.description,
        agents: task.agents,
        max_retries: task.max_retries,
      });
      setRunningTaskId(result.task_id);
      setRunStatus(result.success ? 'success' : 'failed');
      const outputs = result.outputs
        .map((o) => `### ${o.agent}\n${o.content}${o.error ? `\n(Error: ${o.error})` : ''}`)
        .join('\n\n');
      setRunResult(outputs || s.run.empty);
      if (result.success) toast.success(s.status.success);
      else toast.error(s.status.failed);
    } catch (e) {
      setRunStatus('failed');
      setRunResult(String(e));
      toast.error(s.status.failed, String(e));
    }
  }, [doc, s]);

  /** 停止正在运行的任务。 */
  const stopDoc = useCallback(async () => {
    if (runningTaskId) {
      try {
        await swarmCancel(runningTaskId);
      } catch {
        /* 忽略 */
      }
      setRunningTaskId(null);
    }
    setRunStatus('idle');
  }, [runningTaskId]);

  /** 导出为 JSON 文件下载。 */
  const exportJson = useCallback(() => {
    const json = serializeDocument(doc);
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${doc.name || 'workflow'}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }, [doc]);

  /** 从 JSON 文件导入。 */
  const importJson = useCallback(() => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = 'application/json,.json';
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        const parsed = parseDocument(String(reader.result));
        if (parsed) {
          setDoc(parsed);
          setSelectedNodeId(null);
          setSelectedEdgeId(null);
          toast.success(s.status.loaded);
        } else {
          toast.error(s.error.invalidJson);
        }
      };
      reader.readAsText(file);
    };
    input.click();
  }, [s]);

  /** 画布空白处点击:取消选中。 */
  const onCanvasMouseDown = useCallback((ev: MouseEvent) => {
    if (ev.target === ev.currentTarget) {
      setSelectedNodeId(null);
      setSelectedEdgeId(null);
    }
  }, []);

  // ---- 渲染 ----

  const nodeMap = useMemo(() => new Map(doc.nodes.map((n) => [n.id, n])), [doc.nodes]);

  return (
    <div class="wf-root" data-testid="workflow-canvas-root">
      {/* 工具栏 */}
      <div class="wf-toolbar">
        <span class="wf-toolbar-title">{s.title}</span>
        <span class="wf-doc-name">{doc.name}</span>
        <div class="wf-toolbar-spacer" />
        <button
          class="wf-btn"
          data-testid="wf-btn-new"
          onClick={newDoc}
          title={s.toolbar.newDoc}
        >
          📄 {s.toolbar.newDoc}
        </button>
        <button
          class="wf-btn"
          data-testid="wf-btn-save"
          onClick={saveDoc}
          title={s.toolbar.save}
        >
          💾 {s.toolbar.save}
        </button>
        <button
          class="wf-btn"
          data-testid="wf-btn-load"
          onClick={loadListFn}
          title={s.toolbar.load}
        >
          📂 {s.toolbar.load}
        </button>
        <button
          class="wf-btn wf-btn-run"
          data-testid="wf-btn-run"
          onClick={runDoc}
          disabled={runStatus === 'running' || doc.nodes.length === 0}
          title={s.toolbar.run}
        >
          ▶ {s.toolbar.run}
        </button>
        <button
          class="wf-btn wf-btn-stop"
          data-testid="wf-btn-stop"
          onClick={stopDoc}
          disabled={runStatus !== 'running'}
          title={s.toolbar.stop}
        >
          ⏹ {s.toolbar.stop}
        </button>
        <button class="wf-btn" onClick={exportJson} title={s.toolbar.exportJson}>
          ⬆ {s.toolbar.exportJson}
        </button>
        <button class="wf-btn" onClick={importJson} title={s.toolbar.importJson}>
          ⬇ {s.toolbar.importJson}
        </button>
        {/* 状态指示 */}
        <span class={`wf-status wf-status-${runStatus}`} data-testid="wf-status">
          {runStatus === 'idle'
            ? s.status.idle
            : runStatus === 'running'
              ? s.status.running
              : runStatus === 'success'
                ? s.status.success
                : s.status.failed}
        </span>
        <span class="wf-stat">
          {doc.nodes.length} {s.status.nodes} · {doc.edges.length} {s.status.edges}
        </span>
      </div>

      {/* 加载菜单 */}
      {showLoadMenu && (
        <div class="wf-load-menu" data-testid="wf-load-menu">
          <div class="wf-load-menu-header">
            <span>{s.toolbar.load}</span>
            <button class="wf-btn-close" onClick={() => setShowLoadMenu(false)}>
              ✕
            </button>
          </div>
          {loadList.length === 0 ? (
            <div class="wf-load-empty">—</div>
          ) : (
            <ul class="wf-load-list">
              {loadList.map((name) => (
                <li key={name}>
                  <button class="wf-load-item" onClick={() => loadDoc(name)}>
                    {name}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div class="wf-body">
        {/* 节点面板 */}
        <div class="wf-palette" data-testid="wf-palette">
          <div class="wf-palette-title">{s.palette.title}</div>
          <button
            class="wf-palette-item"
            data-testid="wf-palette-agent"
            onClick={() => addNode('agent')}
          >
            <span class="wf-palette-icon">{NODE_THEME.agent.icon}</span>
            <span>{s.palette.agent}</span>
          </button>
          <button
            class="wf-palette-item"
            data-testid="wf-palette-task"
            onClick={() => addNode('task')}
          >
            <span class="wf-palette-icon">{NODE_THEME.task.icon}</span>
            <span>{s.palette.task}</span>
          </button>
          <button
            class="wf-palette-item"
            data-testid="wf-palette-condition"
            onClick={() => addNode('condition')}
          >
            <span class="wf-palette-icon">{NODE_THEME.condition.icon}</span>
            <span>{s.palette.condition}</span>
          </button>
          <button
            class="wf-palette-item"
            data-testid="wf-palette-io"
            onClick={() => addNode('io')}
          >
            <span class="wf-palette-icon">{NODE_THEME.io.icon}</span>
            <span>{s.palette.io}</span>
          </button>
          <div class="wf-palette-hint">{s.palette.hint}</div>
        </div>

        {/* 画布 */}
        <div
          class="wf-canvas"
          ref={canvasRef}
          onMouseDown={onCanvasMouseDown}
          data-testid="wf-canvas"
        >
          <div class="wf-canvas-inner" style={{ width: CANVAS_W, height: CANVAS_H }}>
            {/* SVG 连线层 */}
            <svg
              class="wf-edge-layer"
              width={CANVAS_W}
              height={CANVAS_H}
              data-testid="wf-edge-layer"
            >
              {doc.edges.map((edge) => {
                const src = nodeMap.get(edge.source);
                const dst = nodeMap.get(edge.target);
                if (!src || !dst) return null;
                const sp = outputPortPos(src, edge.sourcePort);
                const tp = inputPortPos(dst);
                const isSel = edge.id === selectedEdgeId;
                return (
                  <g key={edge.id} data-testid={`wf-edge-${edge.id}`}>
                    <path
                      d={bezierPath(sp.x, sp.y, tp.x, tp.y)}
                      fill="none"
                      stroke={isSel ? '#ff8c42' : 'var(--border)'}
                      strokeWidth={isSel ? 2.5 : 1.8}
                      strokeOpacity={isSel ? 1 : 0.6}
                      style={{ cursor: 'pointer' }}
                      onMouseDown={(e) => {
                        e.stopPropagation();
                        setSelectedEdgeId(edge.id);
                        setSelectedNodeId(null);
                      }}
                    />
                    {/* 透明粗命中区 */}
                    <path
                      d={bezierPath(sp.x, sp.y, tp.x, tp.y)}
                      fill="none"
                      stroke="transparent"
                      strokeWidth={14}
                      style={{ cursor: 'pointer' }}
                      onMouseDown={(e) => {
                        e.stopPropagation();
                        setSelectedEdgeId(edge.id);
                        setSelectedNodeId(null);
                      }}
                    />
                    {edge.label && (
                      <text
                        x={(sp.x + tp.x) / 2}
                        y={(sp.y + tp.y) / 2 - 6}
                        textAnchor="middle"
                        fontSize={11}
                        fill="var(--text-secondary)"
                      >
                        {edge.label}
                      </text>
                    )}
                  </g>
                );
              })}
              {/* 临时连线(拖拽中) */}
              {connecting &&
                (() => {
                  const src = nodeMap.get(connecting.sourceId);
                  if (!src) return null;
                  const sp = outputPortPos(src, connecting.sourcePort);
                  return (
                    <path
                      d={bezierPath(sp.x, sp.y, connecting.x, connecting.y)}
                      fill="none"
                      stroke="#ff8c42"
                      strokeWidth={2}
                      strokeDasharray="6 4"
                      strokeOpacity={0.8}
                    />
                  );
                })()}
            </svg>

            {/* 节点层(HTML div) */}
            {doc.nodes.map((node) => {
              const theme = NODE_THEME[node.type];
              const isSel = node.id === selectedNodeId;
              return (
                <div
                  key={node.id}
                  class={`wf-node ${isSel ? 'wf-node-selected' : ''}`}
                  data-testid={`wf-node-${node.id}`}
                  style={{
                    left: node.x,
                    top: node.y,
                    width: NODE_WIDTH,
                    height: NODE_HEIGHT,
                    borderColor: theme.color,
                    background: theme.bg,
                  }}
                  onMouseDown={(ev) => {
                    ev.stopPropagation();
                    setSelectedNodeId(node.id);
                    setSelectedEdgeId(null);
                    const pos = toCanvasCoord(ev.clientX, ev.clientY);
                    setDragging({
                      id: node.id,
                      offsetX: pos.x - node.x,
                      offsetY: pos.y - node.y,
                    });
                  }}
                  onMouseUp={(ev) => {
                    // 连线拖到目标节点上释放。
                    ev.stopPropagation();
                    if (connecting) {
                      finishConnect(node.id);
                    }
                  }}
                >
                  <div class="wf-node-header" style={{ color: theme.color }}>
                    <span class="wf-node-icon">{theme.icon}</span>
                    <span class="wf-node-title">{node.title || s.node[node.type]}</span>
                  </div>
                  <div class="wf-node-type">{s.node[node.type]}</div>

                  {/* 输入端口(左侧) */}
                  <div
                    class="wf-port wf-port-in"
                    data-testid={`wf-port-in-${node.id}`}
                    title="input"
                  />
                  {/* 输出端口(右侧) */}
                  {node.type === 'condition' ? (
                    <>
                      <div
                        class="wf-port wf-port-out wf-port-true"
                        data-testid={`wf-port-true-${node.id}`}
                        title="true"
                        onMouseDown={(ev) => {
                          ev.stopPropagation();
                          startConnect(node.id, 'true', ev.clientX, ev.clientY);
                        }}
                      >
                        <span class="wf-port-label">T</span>
                      </div>
                      <div
                        class="wf-port wf-port-out wf-port-false"
                        data-testid={`wf-port-false-${node.id}`}
                        title="false"
                        onMouseDown={(ev) => {
                          ev.stopPropagation();
                          startConnect(node.id, 'false', ev.clientX, ev.clientY);
                        }}
                      >
                        <span class="wf-port-label">F</span>
                      </div>
                    </>
                  ) : (
                    <div
                      class="wf-port wf-port-out"
                      data-testid={`wf-port-out-${node.id}`}
                      title="output"
                      onMouseDown={(ev) => {
                        ev.stopPropagation();
                        startConnect(node.id, 'out', ev.clientX, ev.clientY);
                      }}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>

        {/* 属性面板 */}
        <div class="wf-panel" data-testid="wf-panel">
          <div class="wf-panel-title">{s.panel.title}</div>
          {!selectedNode && !selectedEdge ? (
            <div class="wf-panel-empty">{s.panel.empty}</div>
          ) : selectedNode ? (
            <NodePropertyEditor
              node={selectedNode}
              s={s}
              onTitle={(v) => updateNodeTitle(selectedNode.id, v)}
              onConfig={(patch) => updateNodeConfig(selectedNode.id, patch)}
              onDelete={() => deleteNode(selectedNode.id)}
            />
          ) : selectedEdge ? (
            <EdgePropertyEditor
              edge={selectedEdge}
              s={s}
              onLabel={(v) => updateEdgeLabel(selectedEdge.id, v)}
              onDelete={() => deleteEdge(selectedEdge.id)}
            />
          ) : null}

          {/* 运行结果 */}
          {runResult !== null && (
            <div class="wf-run-result" data-testid="wf-run-result">
              <div class="wf-run-result-title">{s.run.result}</div>
              <pre class="wf-run-result-body">{runResult}</pre>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ---- 属性面板子组件 ----

interface NodePropertyEditorProps {
  node: WorkflowNode;
  s: ReturnType<typeof workflowStrings>;
  onTitle: (v: string) => void;
  onConfig: (patch: Record<string, unknown>) => void;
  onDelete: () => void;
}

function NodePropertyEditor({ node, s, onTitle, onConfig, onDelete }: NodePropertyEditorProps) {
  return (
    <div class="wf-prop-editor" data-testid={`wf-prop-${node.id}`}>
      <label class="wf-field">
        <span class="wf-field-label">{s.node.titleField}</span>
        <input
          class="wf-input"
          data-testid="wf-prop-title"
          value={node.title}
          onInput={(e) => onTitle((e.target as HTMLInputElement).value)}
        />
      </label>

      {node.config.type === 'agent' && (
        <>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.agentKind}</span>
            <select
              class="wf-input"
              data-testid="wf-prop-agent-kind"
              value={node.config.agent_kind}
              onInput={(e) =>
                onConfig({
                  agent_kind: (e.target as HTMLSelectElement).value as AgentKind,
                })
              }
            >
              <option value="generic">generic</option>
              <option value="coder">coder</option>
              <option value="writer">writer</option>
              <option value="reviewer">reviewer</option>
              <option value="researcher">researcher</option>
              <option value="planner">planner</option>
            </select>
          </label>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.prompt}</span>
            <textarea
              class="wf-input wf-textarea"
              data-testid="wf-prop-prompt"
              rows={3}
              value={node.config.prompt}
              onInput={(e) => onConfig({ prompt: (e.target as HTMLTextAreaElement).value })}
            />
          </label>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.maxRetries}</span>
            <input
              class="wf-input"
              type="number"
              min={0}
              max={5}
              data-testid="wf-prop-max-retries"
              value={node.config.max_retries}
              onInput={(e) =>
                onConfig({
                  max_retries: Math.max(
                    0,
                    Math.min(5, Number((e.target as HTMLInputElement).value) || 0)
                  ),
                })
              }
            />
          </label>
        </>
      )}

      {node.config.type === 'task' && (
        <>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.description}</span>
            <input
              class="wf-input"
              data-testid="wf-prop-task-desc"
              value={node.config.description}
              onInput={(e) =>
                onConfig({ description: (e.target as HTMLInputElement).value })
              }
            />
          </label>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.program}</span>
            <input
              class="wf-input"
              data-testid="wf-prop-program"
              value={node.config.program}
              onInput={(e) => onConfig({ program: (e.target as HTMLInputElement).value })}
            />
          </label>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.args}</span>
            <input
              class="wf-input"
              data-testid="wf-prop-args"
              value={node.config.args}
              onInput={(e) => onConfig({ args: (e.target as HTMLInputElement).value })}
            />
          </label>
        </>
      )}

      {node.config.type === 'condition' && (
        <label class="wf-field">
          <span class="wf-field-label">{s.node.expression}</span>
          <input
            class="wf-input"
            data-testid="wf-prop-expression"
            value={node.config.expression}
            onInput={(e) =>
              onConfig({ expression: (e.target as HTMLInputElement).value })
            }
          />
        </label>
      )}

      {node.config.type === 'io' && (
        <>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.direction}</span>
            <select
              class="wf-input"
              data-testid="wf-prop-direction"
              value={node.config.direction}
              onInput={(e) =>
                onConfig({
                  direction: (e.target as HTMLSelectElement).value as IoDirection,
                })
              }
            >
              <option value="input">{s.node.input}</option>
              <option value="output">{s.node.output}</option>
            </select>
          </label>
          <label class="wf-field">
            <span class="wf-field-label">{s.node.format}</span>
            <select
              class="wf-input"
              data-testid="wf-prop-format"
              value={node.config.format}
              onInput={(e) =>
                onConfig({ format: (e.target as HTMLSelectElement).value as IoFormat })
              }
            >
              <option value="text">text</option>
              <option value="json">json</option>
              <option value="markdown">markdown</option>
            </select>
          </label>
          {node.config.direction === 'input' && (
            <label class="wf-field">
              <span class="wf-field-label">{s.node.content}</span>
              <textarea
                class="wf-input wf-textarea"
                data-testid="wf-prop-content"
                rows={4}
                value={node.config.content}
                onInput={(e) =>
                  onConfig({ content: (e.target as HTMLTextAreaElement).value })
                }
              />
            </label>
          )}
        </>
      )}

      <button
        class="wf-btn wf-btn-danger"
        data-testid="wf-btn-delete-node"
        onClick={onDelete}
      >
        🗑 {s.panel.deleteNode}
      </button>
    </div>
  );
}

interface EdgePropertyEditorProps {
  edge: WorkflowEdge;
  s: ReturnType<typeof workflowStrings>;
  onLabel: (v: string) => void;
  onDelete: () => void;
}

function EdgePropertyEditor({ edge, s, onLabel, onDelete }: EdgePropertyEditorProps) {
  return (
    <div class="wf-prop-editor" data-testid={`wf-prop-edge-${edge.id}`}>
      <div class="wf-field-readonly">
        <strong>{edge.source}</strong>
        <span class="wf-arrow">→</span>
        <strong>{edge.target}</strong>
        <span class="wf-port-tag">{edge.sourcePort}</span>
      </div>
      <label class="wf-field">
        <span class="wf-field-label">{s.panel.edgeLabel}</span>
        <input
          class="wf-input"
          data-testid="wf-prop-edge-label"
          value={edge.label}
          onInput={(e) => onLabel((e.target as HTMLInputElement).value)}
        />
      </label>
      <button
        class="wf-btn wf-btn-danger"
        data-testid="wf-btn-delete-edge"
        onClick={onDelete}
      >
        🗑 {s.panel.deleteEdge}
      </button>
    </div>
  );
}
