/**
 * T-E-S-10: WorkflowCanvas 前端测试。
 *
 * 覆盖:
 * - 纯函数: validateEdge / wouldCreateCycle / topologicalSort /
 *   workflowToSwarmTask / serialize-parse 往返 / createNode / defaultConfig
 * - 组件渲染: 空状态 / 调色板 / 属性面板
 * - 交互: 点击调色板添加节点 / 选中节点显示属性 / 编辑标题更新节点 /
 *   运行按钮在空画布时禁用 / 运行调用 swarmExecute / 保存降级 localStorage
 */
import { describe, it, expect, beforeAll, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/preact';
import { setLocale } from '../../i18n';
import {
  validateEdge,
  wouldCreateCycle,
  topologicalSort,
  workflowToSwarmTask,
  serializeDocument,
  parseDocument,
  createNode,
  defaultConfig,
  emptyDocument,
  type WorkflowNode,
  type WorkflowEdge,
  type WorkflowDocument,
} from '../workflow/types';

// ---- mock Tauri 桥 ----
const { mockSwarmExecute, mockInvokeTauri, mockSwarmCancel } = vi.hoisted(() => ({
  mockSwarmExecute: vi.fn(),
  mockInvokeTauri: vi.fn(),
  mockSwarmCancel: vi.fn(),
}));

vi.mock('../../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../../lib/tauri')>('../../lib/tauri');
  return {
    ...actual,
    nebulaAPI: {
      ...actual.nebulaAPI,
      swarmExecute: mockSwarmExecute,
    },
    invokeTauri: mockInvokeTauri,
    swarmCancel: mockSwarmCancel,
  };
});

// ---- mock Toast(避免无 DOM 容器时报错) ----
vi.mock('../Toast', () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
  },
  Toasts: () => null,
}));

beforeAll(() => {
  // jsdom 无 ResizeObserver,补一个空实现。
  if (typeof globalThis.ResizeObserver === 'undefined') {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
});

beforeEach(() => {
  vi.clearAllMocks();
  setLocale('en-US');
  // 默认后端命令未注册 → invokeTauri 返回 null(触发 localStorage 降级)。
  mockInvokeTauri.mockResolvedValue(null);
  localStorage.clear();
});

afterEach(() => {
  cleanup();
});

// ==================== 纯函数测试 ====================

describe('workflow/types pure functions', () => {
  describe('defaultConfig', () => {
    it('returns_agent_config_for_agent_type', () => {
      const cfg = defaultConfig('agent');
      expect(cfg.type).toBe('agent');
      if (cfg.type === 'agent') {
        expect(cfg.agent_kind).toBe('generic');
        expect(cfg.max_retries).toBe(1);
      }
    });

    it('returns_io_config_for_io_type', () => {
      const cfg = defaultConfig('io');
      expect(cfg.type).toBe('io');
      if (cfg.type === 'io') {
        expect(cfg.direction).toBe('input');
        expect(cfg.format).toBe('text');
      }
    });

    it('returns_condition_config_with_default_expression', () => {
      const cfg = defaultConfig('condition');
      expect(cfg.type).toBe('condition');
      if (cfg.type === 'condition') {
        expect(cfg.expression).toBe('true');
      }
    });
  });

  describe('createNode', () => {
    it('creates_node_with_unique_id_and_given_position', () => {
      const n = createNode('agent', 100, 200, 1);
      expect(n.id).toMatch(/^n-1-/);
      expect(n.type).toBe('agent');
      expect(n.x).toBe(100);
      expect(n.y).toBe(200);
      expect(n.title).toBe('Agent');
    });
  });

  describe('validateEdge', () => {
    it('rejects_self_loop', () => {
      expect(validateEdge('a', 'a', 'out', [])).toBe('self_loop');
    });

    it('rejects_duplicate_edge', () => {
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' },
      ];
      expect(validateEdge('a', 'b', 'out', edges)).toBe('duplicate');
    });

    it('allows_new_edge', () => {
      expect(validateEdge('a', 'b', 'out', [])).toBeNull();
    });

    it('allows_different_ports_for_same_pair', () => {
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'c', target: 'd', sourcePort: 'true', label: '' },
      ];
      expect(validateEdge('c', 'd', 'false', edges)).toBeNull();
    });

    it('rejects_cycle', () => {
      // a→b 存在,再加 b→a 会成环。
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' },
      ];
      expect(validateEdge('b', 'a', 'out', edges)).toBe('cycle');
    });
  });

  describe('wouldCreateCycle', () => {
    it('detects_indirect_cycle', () => {
      // a→b→c,再加 c→a 会成环。
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' },
        { id: 'e2', source: 'b', target: 'c', sourcePort: 'out', label: '' },
      ];
      expect(wouldCreateCycle('c', 'a', edges)).toBe(true);
    });

    it('allows_acyclic_addition', () => {
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' },
      ];
      expect(wouldCreateCycle('b', 'c', edges)).toBe(false);
    });
  });

  describe('topologicalSort', () => {
    it('returns_nodes_in_topological_order', () => {
      const nodes: WorkflowNode[] = [
        { ...createNode('agent', 0, 0, 1), id: 'a' },
        { ...createNode('task', 0, 0, 2), id: 'b' },
        { ...createNode('io', 0, 0, 3), id: 'c' },
      ];
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' },
        { id: 'e2', source: 'b', target: 'c', sourcePort: 'out', label: '' },
      ];
      const order = topologicalSort(nodes, edges);
      expect(order).toEqual(['a', 'b', 'c']);
    });

    it('returns_null_when_cycle_exists', () => {
      const nodes: WorkflowNode[] = [
        { ...createNode('agent', 0, 0, 1), id: 'a' },
        { ...createNode('agent', 0, 0, 2), id: 'b' },
      ];
      const edges: WorkflowEdge[] = [
        { id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' },
        { id: 'e2', source: 'b', target: 'a', sourcePort: 'out', label: '' },
      ];
      expect(topologicalSort(nodes, edges)).toBeNull();
    });
  });

  describe('workflowToSwarmTask', () => {
    it('collects_agent_kinds_and_builds_description', () => {
      const doc: WorkflowDocument = {
        id: 'wf-1',
        name: 'test',
        nodes: [
          {
            id: 'a',
            type: 'agent',
            title: 'Writer Agent',
            x: 0,
            y: 0,
            config: { type: 'agent', agent_kind: 'writer', prompt: '写一篇文章', max_retries: 2 },
          },
          {
            id: 'b',
            type: 'io',
            title: 'Output',
            x: 0,
            y: 0,
            config: { type: 'io', direction: 'output', format: 'markdown', content: '' },
          },
        ],
        edges: [{ id: 'e1', source: 'a', target: 'b', sourcePort: 'out', label: '' }],
        updated_at: 0,
      };
      const task = workflowToSwarmTask(doc);
      expect(task.agents).toContain('writer');
      expect(task.max_retries).toBe(2);
      expect(task.description).toContain('Writer Agent');
      expect(task.description).toContain('写一篇文章');
      expect(task.description).toContain('[Output]');
    });

    it('defaults_to_generic_agent_when_no_agent_node', () => {
      const doc = emptyDocument();
      doc.nodes = [
        {
          id: 't1',
          type: 'task',
          title: 'Task',
          x: 0,
          y: 0,
          config: { type: 'task', description: 'do something', program: 'echo', args: '' },
        },
      ];
      const task = workflowToSwarmTask(doc);
      expect(task.agents).toEqual(['generic']);
    });
  });

  describe('serializeDocument / parseDocument', () => {
    it('roundtrips_document_through_json', () => {
      const doc = emptyDocument('Roundtrip');
      doc.nodes = [createNode('condition', 50, 60, 1, 'My Cond')];
      doc.edges = [
        { id: 'e1', source: 'n1', target: 'n2', sourcePort: 'true', label: 'yes' },
      ];
      const json = serializeDocument(doc);
      const parsed = parseDocument(json);
      expect(parsed).not.toBeNull();
      expect(parsed!.name).toBe('Roundtrip');
      expect(parsed!.nodes).toHaveLength(1);
      expect(parsed!.edges).toHaveLength(1);
      expect(parsed!.edges[0].label).toBe('yes');
    });

    it('returns_null_for_invalid_json', () => {
      expect(parseDocument('not json')).toBeNull();
    });

    it('returns_null_for_missing_required_fields', () => {
      expect(parseDocument('{"foo":"bar"}')).toBeNull();
    });
  });
});

// ==================== 组件渲染与交互测试 ====================

describe('WorkflowCanvas component', () => {
  it('renders_root_palette_and_empty_panel', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    expect(await findByTestId('workflow-canvas-root')).toBeTruthy();
    expect(await findByTestId('wf-palette')).toBeTruthy();
    expect(await findByTestId('wf-panel')).toBeTruthy();
  });

  it('run_button_disabled_when_canvas_empty', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    const btn = (await findByTestId('wf-btn-run')) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it('clicking_palette_agent_adds_a_node', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId, findAllByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    // 节点出现在画布上(testid 形如 wf-node-n-1-xxxx)。
    const nodes = await findAllByTestId(/^wf-node-/);
    expect(nodes.length).toBe(1);
  });

  it('clicking_node_selects_it_and_shows_property_panel', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId, findAllByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    const nodes = await findAllByTestId(/^wf-node-/);
    fireEvent.mouseDown(nodes[0]);
    // 属性面板应显示标题输入框。
    expect(await findByTestId('wf-prop-title')).toBeTruthy();
    expect(await findByTestId('wf-prop-agent-kind')).toBeTruthy();
  });

  it('editing_title_updates_node_display', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId, findAllByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    const nodes = await findAllByTestId(/^wf-node-/);
    fireEvent.mouseDown(nodes[0]);
    const titleInput = (await findByTestId('wf-prop-title')) as HTMLInputElement;
    fireEvent.input(titleInput, { target: { value: 'My Custom Agent' } });
    // 节点标题应更新。
    await waitFor(() => {
      expect(nodes[0].textContent).toContain('My Custom Agent');
    });
  });

  it('condition_node_shows_true_and_false_ports', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-condition'));
    expect(await findByTestId(/wf-port-true-/)).toBeTruthy();
    expect(await findByTestId(/wf-port-false-/)).toBeTruthy();
  });

  it('save_falls_back_to_localstorage_when_backend_unavailable', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    fireEvent.click(await findByTestId('wf-btn-save'));
    // invokeTauri 返回 null → 降级 localStorage。
    await waitFor(() => {
      expect(mockInvokeTauri).toHaveBeenCalledWith(
        'workflow_save',
        expect.objectContaining({ name: expect.any(String) })
      );
    });
    // localStorage 应有索引写入。
    await waitFor(() => {
      const idx = localStorage.getItem('nebula.workflow.index');
      expect(idx).not.toBeNull();
    });
  });

  it('save_uses_backend_when_available', async () => {
    mockInvokeTauri.mockResolvedValue(true);
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    fireEvent.click(await findByTestId('wf-btn-save'));
    await waitFor(() => {
      expect(mockInvokeTauri).toHaveBeenCalledWith(
        'workflow_save',
        expect.objectContaining({ name: expect.any(String) })
      );
    });
  });

  it('run_calls_swarmExecute_and_shows_result', async () => {
    mockSwarmExecute.mockResolvedValue({
      task_id: 'task-xyz',
      outputs: [{ agent: 'writer', content: '生成的文章内容' }],
      duration_ms: 1200,
      success: true,
    });
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    const runBtn = (await findByTestId('wf-btn-run')) as HTMLButtonElement;
    fireEvent.click(runBtn);
    await waitFor(() => {
      expect(mockSwarmExecute).toHaveBeenCalled();
    });
    // 结果区显示执行结果(findByTestId 会重试直到元素出现)。
    expect(await findByTestId('wf-run-result')).toBeTruthy();
  });

  it('load_button_fetches_list_and_shows_menu', async () => {
    mockInvokeTauri.mockResolvedValue(['workflow-a', 'workflow-b']);
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-btn-load'));
    const menu = await findByTestId('wf-load-menu');
    expect(menu).toBeTruthy();
    expect(menu.textContent).toContain('workflow-a');
    expect(menu.textContent).toContain('workflow-b');
  });

  it('new_button_clears_canvas', async () => {
    const { WorkflowCanvas } = await import('../WorkflowCanvas');
    const { findByTestId, findAllByTestId } = render(<WorkflowCanvas />);
    fireEvent.click(await findByTestId('wf-palette-agent'));
    expect((await findAllByTestId(/^wf-node-/)).length).toBe(1);
    fireEvent.click(await findByTestId('wf-btn-new'));
    // 清空后无节点。
    const nodes = document.querySelectorAll('[data-testid^="wf-node-"]');
    expect(nodes.length).toBe(0);
  });
});
