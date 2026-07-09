/**
 * T-E-S-10: WorkflowCanvas 本地双语字符串。
 *
 * 不修改全局 src/i18n/*.json(避免并发冲突),改用本模块自带的
 * 中英字典 + currentLocale 信号实现响应式双语切换。
 * 读取 currentLocale.value 使组件在语言切换时自动重渲染。
 */
import { currentLocale } from '../../i18n';
import type { Locale } from '../../i18n';

/** 工作流画布用到的所有可见字符串键。 */
export interface WorkflowStrings {
  title: string;
  toolbar: {
    newDoc: string;
    save: string;
    load: string;
    run: string;
    stop: string;
    exportJson: string;
    importJson: string;
  };
  palette: {
    title: string;
    agent: string;
    task: string;
    condition: string;
    io: string;
    hint: string;
  };
  node: {
    agent: string;
    task: string;
    condition: string;
    io: string;
    input: string;
    output: string;
    prompt: string;
    agentKind: string;
    maxRetries: string;
    description: string;
    program: string;
    args: string;
    expression: string;
    direction: string;
    format: string;
    content: string;
    titleField: string;
  };
  panel: {
    title: string;
    empty: string;
    deleteNode: string;
    deleteEdge: string;
    edgeLabel: string;
  };
  status: {
    idle: string;
    running: string;
    success: string;
    failed: string;
    nodes: string;
    edges: string;
    saved: string;
    loaded: string;
    saveFailed: string;
    loadFailed: string;
  };
  error: {
    selfLoop: string;
    duplicate: string;
    cycle: string;
    empty: string;
    invalidJson: string;
  };
  run: {
    preparing: string;
    result: string;
    empty: string;
  };
}

const EN: WorkflowStrings = {
  title: 'Workflow Canvas',
  toolbar: {
    newDoc: 'New',
    save: 'Save',
    load: 'Load',
    run: 'Run',
    stop: 'Stop',
    exportJson: 'Export',
    importJson: 'Import',
  },
  palette: {
    title: 'Node Palette',
    agent: 'Agent',
    task: 'Task',
    condition: 'Condition',
    io: 'I/O',
    hint: 'Click to add a node to the canvas',
  },
  node: {
    agent: 'Agent',
    task: 'Task',
    condition: 'Condition',
    io: 'I/O',
    input: 'Input',
    output: 'Output',
    prompt: 'Prompt',
    agentKind: 'Agent Kind',
    maxRetries: 'Max Retries',
    description: 'Description',
    program: 'Program',
    args: 'Arguments',
    expression: 'Expression',
    direction: 'Direction',
    format: 'Format',
    content: 'Content',
    titleField: 'Title',
  },
  panel: {
    title: 'Properties',
    empty: 'Select a node or edge to edit its properties',
    deleteNode: 'Delete Node',
    deleteEdge: 'Delete Edge',
    edgeLabel: 'Edge Label',
  },
  status: {
    idle: 'Idle',
    running: 'Running…',
    success: 'Completed',
    failed: 'Failed',
    nodes: 'nodes',
    edges: 'edges',
    saved: 'Workflow saved',
    loaded: 'Workflow loaded',
    saveFailed: 'Save failed',
    loadFailed: 'Load failed',
  },
  error: {
    selfLoop: 'Cannot connect a node to itself',
    duplicate: 'This connection already exists',
    cycle: 'This connection would create a cycle',
    empty: 'Workflow is empty, add nodes first',
    invalidJson: 'Invalid workflow JSON',
  },
  run: {
    preparing: 'Preparing swarm task…',
    result: 'Execution Result',
    empty: 'No result yet',
  },
};

const ZH: WorkflowStrings = {
  title: '工作流画布',
  toolbar: {
    newDoc: '新建',
    save: '保存',
    load: '加载',
    run: '运行',
    stop: '停止',
    exportJson: '导出',
    importJson: '导入',
  },
  palette: {
    title: '节点面板',
    agent: 'Agent 节点',
    task: '任务节点',
    condition: '条件节点',
    io: 'IO 节点',
    hint: '点击向画布添加节点',
  },
  node: {
    agent: 'Agent',
    task: '任务',
    condition: '条件',
    io: '输入输出',
    input: '输入',
    output: '输出',
    prompt: '提示词',
    agentKind: 'Agent 类型',
    maxRetries: '最大重试',
    description: '描述',
    program: '程序',
    args: '参数',
    expression: '表达式',
    direction: '方向',
    format: '格式',
    content: '内容',
    titleField: '标题',
  },
  panel: {
    title: '属性',
    empty: '选中节点或连线以编辑属性',
    deleteNode: '删除节点',
    deleteEdge: '删除连线',
    edgeLabel: '连线标签',
  },
  status: {
    idle: '空闲',
    running: '运行中…',
    success: '已完成',
    failed: '失败',
    nodes: '节点',
    edges: '连线',
    saved: '工作流已保存',
    loaded: '工作流已加载',
    saveFailed: '保存失败',
    loadFailed: '加载失败',
  },
  error: {
    selfLoop: '不能连接到自身',
    duplicate: '该连线已存在',
    cycle: '该连线会形成环',
    empty: '工作流为空,请先添加节点',
    invalidJson: '无效的工作流 JSON',
  },
  run: {
    preparing: '正在准备蜂群任务…',
    result: '执行结果',
    empty: '暂无结果',
  },
};

const DICTS: Record<Locale, WorkflowStrings> = {
  'en-US': EN,
  'zh-CN': ZH,
};

/**
 * 获取当前语言的工作流画布字符串。
 * 内部读取 currentLocale.value 以建立信号订阅,
 * 使组件在语言切换时自动重渲染。
 */
export function workflowStrings(): WorkflowStrings {
  return DICTS[currentLocale.value] ?? EN;
}
