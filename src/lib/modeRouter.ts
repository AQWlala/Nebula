/**
 * v1.7: 三视角无感切换 — 模式路由器。
 *
 * 设计文档 v7.0 §4 三视角双轨——无感默认 + 显式可选。
 *
 * T-S5-A-02: 升级为 LLM 级路由 — `routeViaLLM(message)` 调用
 * LlmGateway 判断 Chat/Craft/Swarm,保留关键词 `routeMode()` 作为
 * fallback,缓存最近 10 条决策(LRU)。
 *
 * 判断规则(关键词 fallback)：
 * - 命中"写一篇/报告/邮件/文章/翻译/润色"等 → writing
 * - 命中"调试/重构/部署/编译/运行/代码"等 → code
 * - 命中"整理/排期/会议/任务/看板/总结"等 → work
 * - 默认 → code（开发者工具默认）
 */

import { nebulaAPI } from './tauri';

export type WorkMode = 'writing' | 'work' | 'code';

const WRITING_KEYWORDS = [
  '写一篇', '写个', '写一份', '文章', '报告', '邮件', '翻译', '润色', '作文',
  '大纲', '草稿', '散文', '小说', '诗歌', '剧本', '文案', '摘要', '改写',
  'write', 'draft', 'essay', 'article', 'email', 'translate', 'polish', 'outline',
];

const CODE_KEYWORDS = [
  '调试', '重构', '部署', '编译', '运行', '代码', '函数', 'bug', '错误',
  '实现', '开发', '编程', '算法', '数据库', 'api', '接口', '脚本', '命令',
  'debug', 'refactor', 'deploy', 'compile', 'run', 'code', 'function', 'bug',
  'implement', 'develop', 'program', 'algorithm', 'database', 'script',
];

const WORK_KEYWORDS = [
  '整理', '排期', '会议', '任务', '看板', '总结', '计划', '安排', '待办',
  '进度', '回顾', '复盘', '项目管理', '时间管理', '番茄',
  'organize', 'schedule', 'meeting', 'task', 'kanban', 'summary', 'plan',
  'todo', 'progress', 'review', 'retrospective',
];

/**
 * 根据用户输入的任务描述，启发式判断应该切换到哪个工作模式。
 * 返回 null 表示无法判断（保持当前模式）。
 */
export function routeMode(input: string): WorkMode | null {
  if (!input || input.trim().length === 0) return null;
  const lower = input.toLowerCase();

  const writingScore = score(lower, WRITING_KEYWORDS);
  const codeScore = score(lower, CODE_KEYWORDS);
  const workScore = score(lower, WORK_KEYWORDS);

  const max = Math.max(writingScore, codeScore, workScore);
  if (max === 0) return null;

  if (writingScore === max) return 'writing';
  if (codeScore === max) return 'code';
  return 'work';
}

function score(input: string, keywords: string[]): number {
  let s = 0;
  for (const kw of keywords) {
    if (input.includes(kw.toLowerCase())) s += 1;
  }
  return s;
}

// ---------------------------------------------------------------------------
// T-S5-A-02: LLM 级路由 + 缓存
// ---------------------------------------------------------------------------

/** LLM 路由缓存上限(LRU,Map 保持插入顺序)。 */
const LLM_CACHE_MAX = 10;

/** LRU 缓存:message → resolved mode。 */
const llmRouteCache = new Map<string, WorkMode>();

/**
 * 构造 LLM 路由 prompt。要求 LLM 仅回复一个单词:
 * writing / work / code。
 */
function buildRoutePrompt(message: string): string {
  return [
    'Analyze the following user message and determine which work mode it belongs to.',
    'Reply with exactly one word: "writing", "work", or "code".',
    '',
    '- writing: creative writing, drafting, translation, polishing, essays, emails, articles',
    '- work: task management, scheduling, meetings, summaries, planning, kanban',
    '- code: debugging, refactoring, deployment, programming, algorithms, APIs',
    '',
    `Message: "${message}"`,
    '',
    'Mode:',
  ].join('\n');
}

/**
 * 解析 LLM 响应,提取模式。容忍前后空白和标点。
 */
function parseLLMResponse(response: string): WorkMode | null {
  const trimmed = response.trim().toLowerCase();
  // 直接匹配
  if (trimmed === 'writing' || trimmed === 'work' || trimmed === 'code') {
    return trimmed;
  }
  // 容忍额外文本:取第一个出现的模式词
  for (const mode of ['writing', 'work', 'code'] as const) {
    if (trimmed.includes(mode)) return mode;
  }
  return null;
}

/**
 * T-S5-A-02: LLM 级路由 — 调用 LlmGateway 判断工作模式。
 *
 * 流程:
 * 1. 查 LRU 缓存(命中直接返回)
 * 2. 调用 `nebulaAPI.llmComplete` 发送路由 prompt
 * 3. 解析 LLM 回复提取模式词
 * 4. 解析失败或调用异常 → fallback 到关键词 `routeMode()`
 * 5. 成功则写入 LRU 缓存(超过 10 条淘汰最旧)
 *
 * @returns 工作模式,或 null(无法判断,保持当前模式)
 */
export async function routeViaLLM(message: string): Promise<WorkMode | null> {
  if (!message || message.trim().length === 0) return null;

  // 1. 查缓存
  const cached = llmRouteCache.get(message);
  if (cached !== undefined) return cached;

  // 2. 调用 LLM
  let mode: WorkMode | null = null;
  try {
    const response = await nebulaAPI.llmComplete(buildRoutePrompt(message));
    mode = parseLLMResponse(response);
  } catch {
    // LLM 调用失败(Ollama 未启动 / 网络错误等),fallback 到关键词
    mode = null;
  }

  // 3. Fallback 到关键词路由
  if (mode === null) {
    mode = routeMode(message);
  }

  // 4. 写入缓存(仅当有明确结果时)
  if (mode !== null) {
    if (llmRouteCache.size >= LLM_CACHE_MAX) {
      // LRU 淘汰:删除最早的 key(Map 保持插入顺序)
      const oldest = llmRouteCache.keys().next().value;
      if (oldest !== undefined) llmRouteCache.delete(oldest);
    }
    llmRouteCache.set(message, mode);
  }

  return mode;
}

/**
 * 清空 LLM 路由缓存(供测试 / Settings 切换路由模式时调用)。
 */
export function clearRouteCache(): void {
  llmRouteCache.clear();
}

/**
 * 查询当前 LLM 路由缓存大小(供测试 / 指标上报)。
 */
export function routeCacheSize(): number {
  return llmRouteCache.size;
}
