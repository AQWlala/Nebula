/**
 * T-E-S-13: 审查模板。
 *
 * 流程: 输入主题 → 收集 → 分析 → 建议 → 报告 → 输出审查报告。
 * 适用于代码审查、文档评审、方案评估等场景。
 */
import type { WorkflowTemplate } from './types';

const X0 = 80;
const GAP = 280;
const Y_TOP = 220;
const Y_BOT = 320;

export const reviewTemplate: WorkflowTemplate = {
  id: 'review',
  name: 'Review Workflow',
  description: 'Collect, analyze, suggest and report a structured review.',
  category: 'review',
  i18n: {
    'en-US': {
      name: 'Review Workflow',
      description: 'Collect, analyze, suggest and report a structured review.',
    },
    'zh-CN': {
      name: '审查工作流',
      description: '收集、分析、建议并输出结构化审查报告。',
    },
  },
  nodes: [
    {
      id: 'v-input',
      type: 'io',
      title: 'Subject',
      x: X0,
      y: Y_TOP,
      config: {
        type: 'io',
        direction: 'input',
        format: 'text',
        content: '',
      },
    },
    {
      id: 'v-collect',
      type: 'agent',
      title: 'Collect',
      x: X0 + GAP,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'researcher',
        prompt: 'Collect all relevant materials and context about the subject under review.',
        max_retries: 2,
      },
    },
    {
      id: 'v-analyze',
      type: 'agent',
      title: 'Analyze',
      x: X0 + GAP * 2,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'reviewer',
        prompt: 'Analyze the materials for strengths, weaknesses, risks and compliance.',
        max_retries: 2,
      },
    },
    {
      id: 'v-suggest',
      type: 'agent',
      title: 'Suggest',
      x: X0 + GAP * 3,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'reviewer',
        prompt: 'Produce prioritized, actionable suggestions for improvement.',
        max_retries: 1,
      },
    },
    {
      id: 'v-report',
      type: 'agent',
      title: 'Report',
      x: X0 + GAP * 4,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'writer',
        prompt: 'Compose the final review report with summary, findings and recommendations.',
        max_retries: 1,
      },
    },
    {
      id: 'v-output',
      type: 'io',
      title: 'Review Report',
      x: X0 + GAP * 5,
      y: Y_TOP,
      config: {
        type: 'io',
        direction: 'output',
        format: 'markdown',
        content: '',
      },
    },
  ],
  edges: [
    { id: 'v-e1', source: 'v-input', target: 'v-collect', sourcePort: 'out', label: '' },
    { id: 'v-e2', source: 'v-collect', target: 'v-analyze', sourcePort: 'out', label: '' },
    { id: 'v-e3', source: 'v-analyze', target: 'v-suggest', sourcePort: 'out', label: '' },
    { id: 'v-e4', source: 'v-suggest', target: 'v-report', sourcePort: 'out', label: '' },
    { id: 'v-e5', source: 'v-report', target: 'v-output', sourcePort: 'out', label: '' },
  ],
  default_values: {
    subject: '',
    severity_threshold: 'medium',
    include_positives: true,
  },
};
