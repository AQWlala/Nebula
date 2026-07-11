/**
 * T-E-B-07: MemoryMap 力导向图视图测试。
 *
 * PixiJS 需要 WebGL,jsdom 环境下无法初始化,因此整体 mock `pixi.js`。
 * 重点测试:视图切换、维度筛选、MDRM 图谱加载、空边/截断提示。
 */
import { describe, it, expect, beforeAll, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/preact';

// jsdom 缺 ResizeObserver,提供 no-op polyfill(MemoryMap 初始化时需要)
beforeAll(() => {
  if (typeof globalThis.ResizeObserver === 'undefined') {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
});

// ---- mock pixi.js(jsdom 无 WebGL)----
// 提供 MemoryMap 用到的全部 API 的 no-op stub。
vi.mock('pixi.js', () => {
  class ContainerMock {
    x = 0;
    y = 0;
    scale = { set: vi.fn() };
    addChild = vi.fn();
    removeChildren = vi.fn();
  }
  class GraphicsMock {
    x = 0;
    y = 0;
    alpha = 1;
    visible = true;
    eventMode = '';
    cursor = '';
    circle = vi.fn().mockReturnThis();
    fill = vi.fn().mockReturnThis();
    stroke = vi.fn().mockReturnThis();
    clear = vi.fn().mockReturnThis();
    moveTo = vi.fn().mockReturnThis();
    lineTo = vi.fn().mockReturnThis();
    on = vi.fn();
    getGlobalPosition = vi.fn(() => ({ x: 0, y: 0 }));
    destroy = vi.fn();
  }
  class TextMock {
    anchor = { set: vi.fn() };
    x = 0;
    y = 0;
    constructor(_opts: unknown) {}
  }
  class ApplicationMock {
    screen = { width: 800, height: 600 };
    stage = { addChild: vi.fn() };
    renderer = { resize: vi.fn() };
    ticker = { add: vi.fn() };
    init = vi.fn().mockResolvedValue(undefined);
    destroy = vi.fn();
  }
  return {
    Application: ApplicationMock,
    Container: ContainerMock,
    Graphics: GraphicsMock,
    Text: TextMock,
  };
});

// ---- mock nebulaAPI(用 vi.hoisted 提升变量,避免引用未初始化)----
const { mockMemoryListRecent, mockMdrmGetGraph } = vi.hoisted(() => ({
  mockMemoryListRecent: vi.fn(),
  mockMdrmGetGraph: vi.fn(),
}));

vi.mock('../../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../../lib/tauri')>('../../lib/tauri');
  return {
    ...actual,
    nebulaAPI: {
      ...actual.nebulaAPI,
      memoryListRecent: mockMemoryListRecent,
      mdrmGetGraph: mockMdrmGetGraph,
    },
  };
});

import { MemoryMap } from '../MemoryMap';

function makeMemory(id: string, layer = 'L2' as const) {
  return {
    id,
    memory_type: 'Episodic' as const,
    layer,
    content: `content-${id}`,
    summary: { s50: `s50-${id}`, s150: `s150-${id}`, s500: '', s2000: '' },
    importance: 0.5,
    access_count: 1,
    last_access: 0,
    created_at: 1000,
    source: 'test',
    pinned: false,
    compressed_from: null,
    compression_gen: 0,
    archived: false,
    metadata: {},
  };
}

function makeSnapshot(
  rootId: string,
  opts?: { edges?: number; truncated?: boolean; nodeCount?: number }
) {
  const nodeCount = opts?.nodeCount ?? 3;
  const nodes = Array.from({ length: nodeCount }, (_, i) => ({
    id: i === 0 ? rootId : `node-${i}`,
    depth: i,
    role: (i === 0 ? 'root' : 'inner') as 'root' | 'inner' | 'leaf',
    layer: 'L2' as const,
    summary: `summary-${i}`,
    importance: 0.5,
  }));
  const edges = Array.from({ length: opts?.edges ?? 2 }, (_, i) => ({
    src_id: rootId,
    dst_id: `node-${i + 1}`,
    kind: 'causes',
    dimension: 'causal' as const,
    weight: 0.8,
  }));
  return {
    root_id: rootId,
    dimensions: ['causal' as const],
    nodes,
    edges,
    truncated: opts?.truncated ?? false,
  };
}

beforeEach(() => {
  cleanup();
  vi.clearAllMocks();
  mockMemoryListRecent.mockResolvedValue([makeMemory('m1'), makeMemory('m2')]);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('MemoryMap (T-E-B-07)', () => {
  it('renders_layer_view_by_default', async () => {
    const { getByTestId, queryByTestId } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());
    expect(getByTestId('view-layer')).toBeTruthy();
    expect(getByTestId('view-graph')).toBeTruthy();
    // 默认 layer 模式,不显示维度筛选
    expect(queryByTestId('dim-causal')).toBeNull();
  });

  it('switching_to_graph_view_loads_mdrm_graph', async () => {
    mockMdrmGetGraph.mockResolvedValue(makeSnapshot('m1'));
    const { getByTestId } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());

    fireEvent.click(getByTestId('view-graph'));
    await waitFor(() => expect(mockMdrmGetGraph).toHaveBeenCalled());
    // 以第一个记忆为根
    expect(mockMdrmGetGraph).toHaveBeenCalledWith('m1', expect.anything());
  });

  it('dimension_filter_toggle_changes_active_dims', async () => {
    mockMdrmGetGraph.mockResolvedValue(makeSnapshot('m1'));
    const { getByTestId } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());

    fireEvent.click(getByTestId('view-graph'));
    await waitFor(() => expect(mockMdrmGetGraph).toHaveBeenCalled());
    vi.clearAllMocks();

    // 取消勾选 causal
    const causalLabel = getByTestId('dim-causal');
    const checkbox = causalLabel.querySelector('input')!;
    fireEvent.click(checkbox);
    // 轮询直到最新调用的 dims 不再包含 causal(处理异步 state 传播)
    await waitFor(() => {
      expect(mockMdrmGetGraph).toHaveBeenCalled();
      const lastCall = mockMdrmGetGraph.mock.calls[mockMdrmGetGraph.mock.calls.length - 1];
      const dims = lastCall?.[1] as string[] | null;
      expect(dims).not.toContain('causal');
    });
    const lastCall = mockMdrmGetGraph.mock.calls[mockMdrmGetGraph.mock.calls.length - 1];
    const dims = lastCall?.[1] as string[] | null;
    expect(dims).toContain('temporal');
  });

  it('empty_edges_shows_hint', async () => {
    mockMdrmGetGraph.mockResolvedValue(makeSnapshot('m1', { edges: 0 }));
    const { getByTestId } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());

    fireEvent.click(getByTestId('view-graph'));
    await waitFor(() => expect(getByTestId('empty-edges')).toBeTruthy());
  });

  it('truncated_snapshot_shows_warning', async () => {
    mockMdrmGetGraph.mockResolvedValue(makeSnapshot('m1', { truncated: true }));
    const { getByTestId } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());

    fireEvent.click(getByTestId('view-graph'));
    await waitFor(() => expect(getByTestId('truncated-warning')).toBeTruthy());
  });

  it('switching_back_to_layer_hides_dim_filter', async () => {
    mockMdrmGetGraph.mockResolvedValue(makeSnapshot('m1'));
    const { getByTestId, queryByTestId } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());

    fireEvent.click(getByTestId('view-graph'));
    await waitFor(() => expect(getByTestId('dim-causal')).toBeTruthy());

    fireEvent.click(getByTestId('view-layer'));
    expect(queryByTestId('dim-causal')).toBeNull();
  });

  it('header_shows_node_and_edge_count_in_graph_view', async () => {
    mockMdrmGetGraph.mockResolvedValue(makeSnapshot('m1', { edges: 4, nodeCount: 5 }));
    const { getByTestId, container } = render(<MemoryMap />);
    await waitFor(() => expect(mockMemoryListRecent).toHaveBeenCalled());

    fireEvent.click(getByTestId('view-graph'));
    await waitFor(() => expect(mockMdrmGetGraph).toHaveBeenCalled());
    // v2.3: header 改为 .page-header 类名,节点/边计数在视图状态栏
    const headerText = container.querySelector('.page-header')?.textContent ?? '';
    const statusText = container.textContent ?? '';
    // 计数可能在 page-header 或状态栏,两处都检查
    const combinedText = headerText + ' ' + statusText;
    expect(combinedText).toMatch(/5 (nodes|节点) \/ 4 (edges|边)/);
  });
});
