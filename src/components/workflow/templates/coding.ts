/**
 * T-E-S-13: 编码模板。
 *
 * 流程: 输入需求 → 设计 → 实现 → 测试 → 输出代码。
 * 适用于功能开发、原型实现、重构等编码场景。
 */
import type { WorkflowTemplate } from './types';

const X0 = 80;
const GAP = 280;
const Y_TOP = 220;
const Y_BOT = 320;

export const codingTemplate: WorkflowTemplate = {
  id: 'coding',
  name: 'Coding Workflow',
  description: 'Design, implement and test a feature from requirements.',
  category: 'coding',
  i18n: {
    'en-US': {
      name: 'Coding Workflow',
      description: 'Design, implement and test a feature from requirements.',
    },
    'zh-CN': {
      name: '编码工作流',
      description: '根据需求设计、实现并测试功能。',
    },
  },
  nodes: [
    {
      id: 'c-input',
      type: 'io',
      title: 'Requirements',
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
      id: 'c-design',
      type: 'agent',
      title: 'Design',
      x: X0 + GAP,
      y: Y_TOP,
      config: {
        type: 'agent',
        agent_kind: 'planner',
        prompt: 'Design the solution architecture, data structures and interfaces for the requirements.',
        max_retries: 1,
      },
    },
    {
      id: 'c-implement',
      type: 'agent',
      title: 'Implement',
      x: X0 + GAP * 2,
      y: Y_BOT,
      config: {
        type: 'agent',
        agent_kind: 'coder',
        prompt: 'Implement the code following the design, with clear naming and edge-case handling.',
        max_retries: 2,
      },
    },
    {
      id: 'c-test',
      type: 'task',
      title: 'Test',
      x: X0 + GAP * 3,
      y: Y_TOP,
      config: {
        type: 'task',
        description: 'Run the test suite and report failures.',
        program: 'npm',
        args: 'test',
      },
    },
    {
      id: 'c-output',
      type: 'io',
      title: 'Code',
      x: X0 + GAP * 4,
      y: Y_TOP,
      config: {
        type: 'io',
        direction: 'output',
        format: 'text',
        content: '',
      },
    },
  ],
  edges: [
    { id: 'c-e1', source: 'c-input', target: 'c-design', sourcePort: 'out', label: '' },
    { id: 'c-e2', source: 'c-design', target: 'c-implement', sourcePort: 'out', label: '' },
    { id: 'c-e3', source: 'c-implement', target: 'c-test', sourcePort: 'out', label: '' },
    { id: 'c-e4', source: 'c-test', target: 'c-output', sourcePort: 'out', label: '' },
  ],
  default_values: {
    requirements: '',
    language: 'typescript',
    test_command: 'npm test',
  },
};
