/**
 * v1.7: 三视角无感切换 — 关键词启发式模式路由器。
 *
 * 设计文档 v7.0 §4 三视角双轨——无感默认 + 显式可选。
 *
 * Phase 6 降级版：基于任务关键词的启发式判断（非 LLM）。
 * 真正的"无感"依赖 Team Context Pool（Phase 2/3 的 L4 + Memory
 * Orchestrator），若未做则只能实现关键词启发式。
 *
 * 判断规则：
 * - 命中"写一篇/报告/邮件/文章/翻译/润色"等 → writing
 * - 命中"调试/重构/部署/编译/运行/代码"等 → code
 * - 命中"整理/排期/会议/任务/看板/总结"等 → work
 * - 默认 → code（开发者工具默认）
 */

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
