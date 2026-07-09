/**
 * T-E-S-13: 数据分析模板。
 *
 * 流程: 输入数据源 → 采集 → 清洗 → 分析 → 可视化 → 输出报告。
 * 适用于数据探索、报表生成、指标分析等场景。
 */
import type { WorkflowTemplate } from './types';

const X0 = 80;
const GAP = 280;
const Y_TOP = 220;
const Y_BOT = 320;

export const dataAnalysisTemplate: WorkflowTemplate = {
  id: 'data_analysis',
  name: 'Data Analysis Workflow',
  description: 'Collect, clean, analyze and visualize data into a report.',
  category: 'data_analysis',
  i18n: {
    'en-US': {
      name: 'Data Analysis Workflow',
      description: 'Collect, clean, analyze and visualize data into a report.',
    },
    'zh-CN': {
      name: '数据分析工作流',
      description: '采集、清洗、分析数据并可视化为报告。',
    },
  },
  nodes: [
    {
      id: 'd-input',
      type: 'io',
      title: 'Data Source',
      x: X0,
      y: Y_TOP,
      config: {
        type: 'io',
        direction: 'input',
        format: 'json',
        content: '',
      },
    },
    {
      id: 'd-collect',
      type: 'task',
      title: 'Collect',
      x: X0 + GAP,
      y: Y_TOP,
      config: {
        type: 'task',
        description: 'Fetch raw data from the configured source (file/API/database).',
        program: 'python',
        args: '-m collector',
      },
    },
    {
      id: 'd-clean',
      type: 'task',
      title: 'Clean',
      x: X0 + GAP * 2,
      y: Y_BOT,
      config: {
        type: 'task',
        description: 'Clean data: deduplicate, handle missing values, normalize types.',
        program: 'python',
        args: '-m cleaner',
      },
    },
    {
      id: 'd-analyze',
      type: 'agent',
      title: 'Analyze',
      x: X0 + GAP * 3,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'researcher',
        prompt: 'Analyze the cleaned data: compute statistics, trends and notable patterns.',
        max_retries: 2,
      },
    },
    {
      id: 'd-visualize',
      type: 'agent',
      title: 'Visualize',
      x: X0 + GAP * 4,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'generic',
        prompt: 'Generate visualizations and a narrative summary from the analysis.',
        max_retries: 1,
      },
    },
    {
      id: 'd-output',
      type: 'io',
      title: 'Report',
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
    { id: 'd-e1', source: 'd-input', target: 'd-collect', sourcePort: 'out', label: '' },
    { id: 'd-e2', source: 'd-collect', target: 'd-clean', sourcePort: 'out', label: '' },
    { id: 'd-e3', source: 'd-clean', target: 'd-analyze', sourcePort: 'out', label: '' },
    { id: 'd-e4', source: 'd-analyze', target: 'd-visualize', sourcePort: 'out', label: '' },
    { id: 'd-e5', source: 'd-visualize', target: 'd-output', sourcePort: 'out', label: '' },
  ],
  default_values: {
    source_type: 'file',
    source_path: '',
    output_format: 'markdown',
  },
};
