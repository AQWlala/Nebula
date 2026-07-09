/**
 * T-E-S-13: 写作模板。
 *
 * 流程: 输入主题 → 大纲 → 草稿 → 审查 → 修改 → 输出文章。
 * 适用于博客、长文、技术文档等创作场景。
 */
import type { WorkflowTemplate } from './types';

const X0 = 80;
const GAP = 280;
const Y_TOP = 220;
const Y_BOT = 320;

export const writingTemplate: WorkflowTemplate = {
  id: 'writing',
  name: 'Writing Workflow',
  description: 'Outline, draft, review and revise a long-form article.',
  category: 'writing',
  i18n: {
    'en-US': {
      name: 'Writing Workflow',
      description: 'Outline, draft, review and revise a long-form article.',
    },
    'zh-CN': {
      name: '写作工作流',
      description: '大纲、草稿、审查并修改长篇文章。',
    },
  },
  nodes: [
    {
      id: 'w-input',
      type: 'io',
      title: 'Topic',
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
      id: 'w-outline',
      type: 'agent',
      title: 'Outline',
      x: X0 + GAP,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'planner',
        prompt: 'Create a structured outline for the article based on the topic.',
        max_retries: 1,
      },
    },
    {
      id: 'w-draft',
      type: 'agent',
      title: 'Draft',
      x: X0 + GAP * 2,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'writer',
        prompt: 'Write the first draft following the outline with full prose.',
        max_retries: 2,
      },
    },
    {
      id: 'w-review',
      type: 'agent',
      title: 'Review',
      x: X0 + GAP * 3,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'reviewer',
        prompt: 'Review the draft for clarity, structure, tone and factual accuracy.',
        max_retries: 1,
      },
    },
    {
      id: 'w-revise',
      type: 'agent',
      title: 'Revise',
      x: X0 + GAP * 4,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'writer',
        prompt: 'Revise the draft based on review feedback to produce the final article.',
        max_retries: 2,
      },
    },
    {
      id: 'w-output',
      type: 'io',
      title: 'Article',
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
    { id: 'w-e1', source: 'w-input', target: 'w-outline', sourcePort: 'out', label: '' },
    { id: 'w-e2', source: 'w-outline', target: 'w-draft', sourcePort: 'out', label: '' },
    { id: 'w-e3', source: 'w-draft', target: 'w-review', sourcePort: 'out', label: '' },
    { id: 'w-e4', source: 'w-review', target: 'w-revise', sourcePort: 'out', label: '' },
    { id: 'w-e5', source: 'w-revise', target: 'w-output', sourcePort: 'out', label: '' },
  ],
  default_values: {
    topic: '',
    tone: 'neutral',
    word_count: 1000,
    language: 'en',
  },
};
