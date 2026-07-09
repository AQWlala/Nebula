/**
 * T-E-S-10: WorkflowCanvas 数据模型与纯函数。
 *
 * 定义工作流编排画布的核心类型(Agent/Task/Condition/IO 四类节点)、
 * 边、文档结构,以及拓扑排序、合法性校验、序列化、向 SwarmTask 转换等纯函数。
 *
 * 设计要点:
 * - 所有函数均为纯函数,便于单元测试与 reducer 复用。
 * - 节点 id / 边 id 使用 `n-<seq>` / `e-<seq>` 短格式,降低序列化体积。
 * - 边通过 sourcePort 区分条件节点的 true/false 分支与普通数据流出口。
 */

/** 工作流节点类型。 */
export type WorkflowNodeType = 'agent' | 'task' | 'condition' | 'io';

/** Agent 节点的可选角色(与后端 AgentKind 对齐)。 */
export type AgentKind =
  | 'generic'
  | 'coder'
  | 'writer'
  | 'reviewer'
  | 'researcher'
  | 'planner';

/** IO 节点的数据方向。 */
export type IoDirection = 'input' | 'output';

/** IO 节点的数据格式。 */
export type IoFormat = 'text' | 'json' | 'markdown';

/** 各节点类型的配置载荷。 */
export interface AgentNodeConfig {
  agent_kind: AgentKind;
  prompt: string;
  max_retries: number;
}

export interface TaskNodeConfig {
  description: string;
  program: string;
  args: string;
}

export interface ConditionNodeConfig {
  expression: string;
}

export interface IoNodeConfig {
  direction: IoDirection;
  format: IoFormat;
  content: string;
}

/** 节点配置的联合类型(按 type 字段判别)。 */
export type WorkflowNodeConfig =
  | ({ type: 'agent' } & AgentNodeConfig)
  | ({ type: 'task' } & TaskNodeConfig)
  | ({ type: 'condition' } & ConditionNodeConfig)
  | ({ type: 'io' } & IoNodeConfig);

/** 画布上的一个节点。 */
export interface WorkflowNode {
  id: string;
  type: WorkflowNodeType;
  title: string;
  x: number;
  y: number;
  config: WorkflowNodeConfig;
}

/**
 * 边的出口端口标识。
 * - 普通节点: 'out'(单一数据流出口)。
 * - 条件节点: 'true' / 'false'(两分支出口)。
 */
export type EdgePort = 'out' | 'true' | 'false';

/** 画布上的一条连线,表示数据流 / 控制流。 */
export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
  sourcePort: EdgePort;
  label: string;
}

/** 完整的工作流文档。 */
export interface WorkflowDocument {
  id: string;
  name: string;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
  updated_at: number;
}

/** 执行状态。 */
export type RunStatus = 'idle' | 'running' | 'success' | 'failed';

/** 节点默认尺寸(像素),用于命中检测与连线锚点计算。 */
export const NODE_WIDTH = 180;
export const NODE_HEIGHT = 84;
/** 输入/输出端口的相对坐标(相对于节点左上角)。 */
export const INPUT_PORT_OFFSET = { x: 0, y: NODE_HEIGHT / 2 };
export const OUTPUT_PORT_OFFSET = { x: NODE_WIDTH, y: NODE_HEIGHT / 2 };
/** 条件节点 true/false 分支端口相对坐标。 */
export const CONDITION_TRUE_PORT_OFFSET = { x: NODE_WIDTH, y: NODE_HEIGHT * 0.3 };
export const CONDITION_FALSE_PORT_OFFSET = { x: NODE_WIDTH, y: NODE_HEIGHT * 0.7 };

/** 各节点类型的默认配置工厂。 */
export function defaultConfig(type: WorkflowNodeType): WorkflowNodeConfig {
  switch (type) {
    case 'agent':
      return {
        type: 'agent',
        agent_kind: 'generic',
        prompt: '',
        max_retries: 1,
      };
    case 'task':
      return { type: 'task', description: '', program: 'echo', args: '' };
    case 'condition':
      return { type: 'condition', expression: 'true' };
    case 'io':
      return { type: 'io', direction: 'input', format: 'text', content: '' };
  }
}

/** 各节点类型的默认标题(英文短名,前端展示时再走 i18n)。 */
export function defaultTitle(type: WorkflowNodeType): string {
  switch (type) {
    case 'agent':
      return 'Agent';
    case 'task':
      return 'Task';
    case 'condition':
      return 'Condition';
    case 'io':
      return 'I/O';
  }
}

/** 生成唯一节点 id(基于自增计数器 + 时间戳低位,降低碰撞)。 */
export function makeNodeId(seq: number): string {
  return `n-${seq}-${(Date.now() & 0xffff).toString(36)}`;
}

/** 生成唯一边 id。 */
export function makeEdgeId(seq: number): string {
  return `e-${seq}-${(Date.now() & 0xffff).toString(36)}`;
}

/** 创建一个新节点,放在画布指定坐标。 */
export function createNode(
  type: WorkflowNodeType,
  x: number,
  y: number,
  seq: number,
  title?: string
): WorkflowNode {
  return {
    id: makeNodeId(seq),
    type,
    title: title ?? defaultTitle(type),
    x,
    y,
    config: defaultConfig(type),
  };
}

/**
 * 校验一条新边是否合法。
 * 返回 null 表示合法,否则返回错误原因(英文短码,供前端展示)。
 */
export function validateEdge(
  source: string,
  target: string,
  sourcePort: EdgePort,
  existing: WorkflowEdge[]
): string | null {
  if (source === target) return 'self_loop';
  // 同方向同端口的重复边(忽略 label 差异)。
  if (
    existing.some(
      (e) => e.source === source && e.target === target && e.sourcePort === sourcePort
    )
  ) {
    return 'duplicate';
  }
  // 简单环检测:若 target 已有路径回到 source,则禁止(防止成环)。
  if (wouldCreateCycle(source, target, existing)) return 'cycle';
  return null;
}

/**
 * 判断新增 source→target 边是否会在图中形成环。
 * 使用 DFS 从 target 出发,看能否到达 source。
 */
export function wouldCreateCycle(
  source: string,
  target: string,
  existing: WorkflowEdge[]
): boolean {
  if (source === target) return true;
  const adj = new Map<string, string[]>();
  for (const e of existing) {
    const arr = adj.get(e.source) ?? [];
    arr.push(e.target);
    adj.set(e.source, arr);
  }
  const visited = new Set<string>();
  const stack = [target];
  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur === source) return true;
    if (visited.has(cur)) continue;
    visited.add(cur);
    const next = adj.get(cur);
    if (next) stack.push(...next);
  }
  return false;
}

/** 拓扑排序(Kahn 算法)。返回节点 id 数组;若存在环返回 null。 */
export function topologicalSort(nodes: WorkflowNode[], edges: WorkflowEdge[]): string[] | null {
  const inDegree = new Map<string, number>();
  const adj = new Map<string, string[]>();
  for (const n of nodes) {
    inDegree.set(n.id, 0);
    adj.set(n.id, []);
  }
  for (const e of edges) {
    if (!inDegree.has(e.source) || !inDegree.has(e.target)) continue;
    adj.get(e.source)!.push(e.target);
    inDegree.set(e.target, (inDegree.get(e.target) ?? 0) + 1);
  }
  const queue: string[] = [];
  for (const [id, deg] of inDegree) {
    if (deg === 0) queue.push(id);
  }
  const result: string[] = [];
  while (queue.length > 0) {
    const cur = queue.shift()!;
    result.push(cur);
    for (const next of adj.get(cur) ?? []) {
      const d = (inDegree.get(next) ?? 0) - 1;
      inDegree.set(next, d);
      if (d === 0) queue.push(next);
    }
  }
  return result.length === nodes.length ? result : null;
}

/**
 * 将工作流文档转换为 SwarmTask 描述(供 swarm_execute 调用)。
 *
 * 转换策略:
 * 1. 拓扑排序节点;若存在环,回退为节点原顺序。
 * 2. agent 节点收集为 agents 数组(去重)。
 * 3. 按拓扑顺序拼接各节点的描述:IO 输入 → task → condition → agent → IO 输出。
 */
export function workflowToSwarmTask(
  doc: WorkflowDocument
): { description: string; agents: string[]; max_retries: number } {
  const order = topologicalSort(doc.nodes, doc.edges) ?? doc.nodes.map((n) => n.id);
  const nodeMap = new Map(doc.nodes.map((n) => [n.id, n]));
  const lines: string[] = [];
  const agents: string[] = [];
  let maxRetries = 1;

  for (const id of order) {
    const node = nodeMap.get(id);
    if (!node) continue;
    switch (node.config.type) {
      case 'io':
        if (node.config.direction === 'input') {
          lines.push(`[Input] ${node.title}: ${node.config.content || '(empty)'}`);
        } else {
          lines.push(`[Output] ${node.title} (${node.config.format})`);
        }
        break;
      case 'task':
        lines.push(
          `[Task] ${node.title}: ${node.config.description} (${node.config.program} ${node.config.args})`
        );
        break;
      case 'condition':
        lines.push(`[Condition] ${node.title}: if (${node.config.expression})`);
        break;
      case 'agent':
        lines.push(`[Agent:${node.config.agent_kind}] ${node.title}: ${node.config.prompt || '(no prompt)'}`);
        if (!agents.includes(node.config.agent_kind)) agents.push(node.config.agent_kind);
        if (node.config.max_retries > maxRetries) maxRetries = node.config.max_retries;
        break;
    }
  }

  if (agents.length === 0) agents.push('generic');
  return {
    description: lines.join('\n'),
    agents,
    max_retries: maxRetries,
  };
}

/** 创建一份空白工作流文档。 */
export function emptyDocument(name = 'Untitled Workflow'): WorkflowDocument {
  return {
    id: `wf-${(Date.now() & 0xffffff).toString(36)}`,
    name,
    nodes: [],
    edges: [],
    updated_at: Date.now(),
  };
}

/** 序列化文档为 JSON 字符串。 */
export function serializeDocument(doc: WorkflowDocument): string {
  return JSON.stringify(doc, null, 2);
}

/** 反序列化 JSON 字符串为文档;失败返回 null。 */
export function parseDocument(json: string): WorkflowDocument | null {
  try {
    const obj = JSON.parse(json) as WorkflowDocument;
    if (
      typeof obj?.id === 'string' &&
      typeof obj?.name === 'string' &&
      Array.isArray(obj?.nodes) &&
      Array.isArray(obj?.edges)
    ) {
      return obj;
    }
    return null;
  } catch {
    return null;
  }
}

/** 计算节点输出端口的绝对坐标(画布坐标系)。 */
export function outputPortPos(node: WorkflowNode, port: EdgePort): { x: number; y: number } {
  if (node.type === 'condition') {
    if (port === 'true') {
      return { x: node.x + CONDITION_TRUE_PORT_OFFSET.x, y: node.y + CONDITION_TRUE_PORT_OFFSET.y };
    }
    if (port === 'false') {
      return { x: node.x + CONDITION_FALSE_PORT_OFFSET.x, y: node.y + CONDITION_FALSE_PORT_OFFSET.y };
    }
  }
  return { x: node.x + OUTPUT_PORT_OFFSET.x, y: node.y + OUTPUT_PORT_OFFSET.y };
}

/** 计算节点输入端口的绝对坐标(画布坐标系)。 */
export function inputPortPos(node: WorkflowNode): { x: number; y: number } {
  return { x: node.x + INPUT_PORT_OFFSET.x, y: node.y + INPUT_PORT_OFFSET.y };
}

/**
 * 计算两点之间的三次贝塞尔路径(垂直方向控制点偏移)。
 * 用于绘制平滑的连线。
 */
export function bezierPath(
  x1: number,
  y1: number,
  x2: number,
  y2: number
): string {
  const dx = Math.abs(x2 - x1);
  const offset = Math.max(40, dx * 0.5);
  return `M ${x1} ${y1} C ${x1 + offset} ${y1}, ${x2 - offset} ${y2}, ${x2} ${y2}`;
}
