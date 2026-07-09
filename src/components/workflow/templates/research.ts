/**
 * T-E-S-13: 研究模板。
 *
 * 流程: 输入主题 → 搜索 → 分析 → 总结 → 输出报告。
 * 适用于课题调研、竞品分析、知识汇总等场景。
 */
import type { WorkflowTemplate } from './types';

/** 研究模板节点坐标(水平排列,垂直微调)。 */
const X0 = 80;
const GAP = 280;
const Y_TOP = 220;
const Y_BOT = 320;

export const researchTemplate: WorkflowTemplate = {
  id: 'research',
  name: 'Research Workflow',
  description: 'Search, analyze, summarize and output a research report.',
  category: 'research',
  i18n: {
    'en-US': {
      name: 'Research Workflow',
      description: 'Search, analyze, summarize and output a research report.',
    },
    'zh-CN': {
      name: '研究工作流',
      description: '搜索、分析、总结并输出研究报告。',
    },
  },
  nodes: [
    {
      id: 'r-input',
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
      id: 'r-search',
      type: 'agent',
      title: 'Search',
      x: X0 + GAP,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'researcher',
        prompt: 'Search for relevant information about the given topic and collect key findings.',
        max_retries: 2,
      },
    },
    {
      id: 'r-analyze',
      type: 'agent',
      title: 'Analyze',
      x: X0 + GAP * 2,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'researcher',
        prompt: 'Analyze the collected findings: identify themes, gaps and key insights.',
        max_retries: 2,
      },
    },
    {
      id: 'r-summarize',
      type: 'agent',
      title: 'Summarize',
      x: X0 + GAP * 3,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'writer',
        prompt: 'Summarize the analysis into a concise brief with bullet points.',
        max_retries: 1,
      },
    },
    {
      id: 'r-output',
      type: 'io',
      title: 'Report',
      x: X0 + GAP * 4,
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
    { id: 'r-e1', source: 'r-input', target: 'r-search', sourcePort: 'out', label: '' },
    { id: 'r-e2', source: 'r-search', target: 'r-analyze', sourcePort: 'out', label: '' },
    { id: 'r-e3', source: 'r-analyze', target: 'r-summarize', sourcePort: 'out', label: '' },
    { id: 'r-e4', source: 'r-summarize', target: 'r-output', sourcePort: 'out', label: '' },
  ],
  default_values: {
    topic: '',
    max_sources: 5,
    language: 'en',
  },
};
