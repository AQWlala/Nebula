/**
 * T-E-S-13: 翻译模板。
 *
 * 流程: 输入源文 → 分段 → 翻译 → 校对 → 合并 → 输出译文。
 * 适用于长文翻译、多语言本地化等场景。
 */
import type { WorkflowTemplate } from './types';

const X0 = 80;
const GAP = 280;
const Y_TOP = 220;
const Y_BOT = 320;

export const translationTemplate: WorkflowTemplate = {
  id: 'translation',
  name: 'Translation Workflow',
  description: 'Segment, translate, proofread and merge a long document.',
  category: 'translation',
  i18n: {
    'en-US': {
      name: 'Translation Workflow',
      description: 'Segment, translate, proofread and merge a long document.',
    },
    'zh-CN': {
      name: '翻译工作流',
      description: '分段、翻译、校对并合并长文档。',
    },
  },
  nodes: [
    {
      id: 't-input',
      type: 'io',
      title: 'Source Text',
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
      id: 't-segment',
      type: 'task',
      title: 'Segment',
      x: X0 + GAP,
      y: Y_TOP,
      config: {
        type: 'task',
        description: 'Split the source text into translatable segments by paragraph/sentence.',
        program: 'python',
        args: '-m segmenter',
      },
    },
    {
      id: 't-translate',
      type: 'agent',
      title: 'Translate',
      x: X0 + GAP * 2,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'generic',
        prompt: 'Translate each segment into the target language preserving meaning and tone.',
        max_retries: 2,
      },
    },
    {
      id: 't-proofread',
      type: 'agent',
      title: 'Proofread',
      x: X0 + GAP * 3,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'reviewer',
        prompt: 'Proofread the translation for accuracy, fluency and terminology consistency.',
        max_retries: 2,
      },
    },
    {
      id: 't-merge',
      type: 'agent',
      title: 'Merge',
      x: X0 + GAP * 4,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'writer',
        prompt: 'Merge the proofread segments back into a coherent full document.',
        max_retries: 1,
      },
    },
    {
      id: 't-output',
      type: 'io',
      title: 'Translation',
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
    { id: 't-e1', source: 't-input', target: 't-segment', sourcePort: 'out', label: '' },
    { id: 't-e2', source: 't-segment', target: 't-translate', sourcePort: 'out', label: '' },
    { id: 't-e3', source: 't-translate', target: 't-proofread', sourcePort: 'out', label: '' },
    { id: 't-e4', source: 't-proofread', target: 't-merge', sourcePort: 'out', label: '' },
    { id: 't-e5', source: 't-merge', target: 't-output', sourcePort: 'out', label: '' },
  ],
  default_values: {
    source_language: 'auto',
    target_language: 'zh-CN',
    preserve_formatting: true,
  },
};
