/**
 * T-E-S-50: 自主度滑块 L0-L5 — 前端类型 + API 封装。
 *
 * 与 modeRouter(任务领域 writing/work/code)正交:
 * `(WorkMode, AutonomyLevel)` 组合决定最终行为。
 *
 * 默认 L2(对话),行为与当前 ChatPanel 一致。
 *
 * Wire 格式 "L0".."L5" 与后端 `AutonomyLevel` 对齐(serde rename = "L0".."L5")。
 */
import { invoke } from '@tauri-apps/api/core';

/** 6 档自主度等级(wire 格式,与后端 `AutonomyLevel` 对齐)。 */
export type AutonomyLevel = 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5';

/** 每级行为参数(与后端 `AutonomyConfig` / `AutonomyConfigDto` 字段一致)。 */
export interface AutonomyConfig {
  requires_approval: boolean;
  runs_in_background: boolean;
  auto_execute: boolean;
  allows_inline_ui: boolean;
  routes_to_swarm: boolean;
  routes_to_plan: boolean;
}

/** 单个等级的元信息(供滑块渲染,与后端 `AutonomyLevelInfo` 对齐)。 */
export interface AutonomyLevelInfo {
  /** Wire 字符串("L0".."L5")。 */
  level: AutonomyLevel;
  /** 数值索引(0..=5)。 */
  index: number;
  /** 英文标签。 */
  label: string;
  /** 中文标签。 */
  label_zh: string;
  /** 英文描述。 */
  description: string;
  /** 中文描述。 */
  description_zh: string;
  /** 该等级的行为参数。 */
  config: AutonomyConfig;
}

/** 等级列表(L0→L5)。 */
export const AUTONOMY_LEVELS: AutonomyLevel[] = ['L0', 'L1', 'L2', 'L3', 'L4', 'L5'];

/** 默认等级(最低风险,行为与当前 ChatPanel 一致)。 */
export const DEFAULT_AUTONOMY_LEVEL: AutonomyLevel = 'L2';

/**
 * 静态等级元信息(与后端 `autonomy_list_levels` 输出对齐)。
 * 用于滑块立即渲染,避免阻塞在后端调用;运行时可通过 `listLevels()` 刷新。
 */
export const AUTONOMY_LEVEL_INFOS: AutonomyLevelInfo[] = [
  {
    level: 'L0',
    index: 0,
    label: 'Inline Completion',
    label_zh: '内联补全',
    description: 'Inline AI suggestions as you type',
    description_zh: '输入时内联 AI 补全建议',
    config: {
      requires_approval: false,
      runs_in_background: false,
      auto_execute: true,
      allows_inline_ui: true,
      routes_to_swarm: false,
      routes_to_plan: false,
    },
  },
  {
    level: 'L1',
    index: 1,
    label: 'Directed Edit',
    label_zh: '定向编辑',
    description: 'Rewrite the selected text on shortcut',
    description_zh: '选中文字 + 快捷键 → AI 局部改写',
    config: {
      requires_approval: false,
      runs_in_background: false,
      auto_execute: true,
      allows_inline_ui: false,
      routes_to_swarm: false,
      routes_to_plan: false,
    },
  },
  {
    level: 'L2',
    index: 2,
    label: 'Chat',
    label_zh: '对话',
    description: 'Conversational replies, no auto-execution',
    description_zh: '对话回复,不自动执行',
    config: {
      requires_approval: false,
      runs_in_background: false,
      auto_execute: false,
      allows_inline_ui: false,
      routes_to_swarm: false,
      routes_to_plan: false,
    },
  },
  {
    level: 'L3',
    index: 3,
    label: 'Plan',
    label_zh: '计划',
    description: 'High-risk actions require approval',
    description_zh: '高风险操作需审批',
    config: {
      requires_approval: true,
      runs_in_background: false,
      auto_execute: false,
      allows_inline_ui: false,
      routes_to_swarm: false,
      routes_to_plan: true,
    },
  },
  {
    level: 'L4',
    index: 4,
    label: 'Swarm',
    label_zh: '蜂群',
    description: 'Fully autonomous multi-agent swarm',
    description_zh: '全自主多智能体蜂群',
    config: {
      requires_approval: false,
      runs_in_background: false,
      auto_execute: false,
      allows_inline_ui: false,
      routes_to_swarm: true,
      routes_to_plan: false,
    },
  },
  {
    level: 'L5',
    index: 5,
    label: 'Background',
    label_zh: '后台',
    description: 'Cron/trigger-driven background automation',
    description_zh: 'Cron/触发器驱动后台自动化',
    config: {
      requires_approval: false,
      runs_in_background: true,
      auto_execute: false,
      allows_inline_ui: false,
      routes_to_swarm: false,
      routes_to_plan: false,
    },
  },
];

/**
 * 读取当前自主度等级。
 * Tauri 不可用时(浏览器/单测)返回默认 'L2'。
 */
export async function getLevel(): Promise<AutonomyLevel> {
  try {
    const lvl = await invoke<string>('autonomy_get_level');
    return (lvl as AutonomyLevel) ?? DEFAULT_AUTONOMY_LEVEL;
  } catch {
    return DEFAULT_AUTONOMY_LEVEL;
  }
}

/**
 * 设置自主度等级。
 * Tauri 不可用时静默忽略(浏览器/单测)。
 */
export async function setLevel(level: AutonomyLevel): Promise<void> {
  try {
    await invoke('autonomy_set_level', { level });
  } catch {
    /* Tauri runtime unavailable; ignore */
  }
}

/**
 * 枚举全部 6 档等级(含 label/description/config)。
 * Tauri 不可用时返回静态 `AUTONOMY_LEVEL_INFOS`。
 */
export async function listLevels(): Promise<AutonomyLevelInfo[]> {
  try {
    return await invoke<AutonomyLevelInfo[]>('autonomy_list_levels');
  } catch {
    return AUTONOMY_LEVEL_INFOS;
  }
}

/**
 * 调试用:查询指定等级 + 任务的路由决策(Debug 字符串)。
 * Tauri 不可用时返回 'Unknown'。
 */
export async function route(level: AutonomyLevel, task: string): Promise<string> {
  try {
    return await invoke<string>('autonomy_route', { level, task });
  } catch {
    return 'Unknown';
  }
}

/** 等级转数值索引(0..=5)。未知等级返回 -1。 */
export function levelToIndex(level: AutonomyLevel): number {
  return AUTONOMY_LEVELS.indexOf(level);
}

/** 数值索引转等级。越界回退到 L2。 */
export function indexToLevel(index: number): AutonomyLevel {
  const i = Math.max(0, Math.min(5, Math.floor(index)));
  return AUTONOMY_LEVELS[i] ?? DEFAULT_AUTONOMY_LEVEL;
}
