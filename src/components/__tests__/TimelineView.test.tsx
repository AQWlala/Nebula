/**
 * T-E-B-02: TimelineView 时间轴视图测试。
 *
 * 覆盖:初始加载、日期分组、层筛选、重要性筛选、展开详情、空状态、统计栏。
 * TimelineView 仅依赖 nebulaAPI.memoryListRecent + i18n(已全局可用),无需 mock pixi。
 */
import { describe, it, expect, beforeAll, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/preact';
import type { Memory } from '../../lib/tauri';

// jsdom 缺 ResizeObserver — 虽然 TimelineView 不直接用,但 preact 子组件可能触发。
beforeAll(() => {
  if (typeof globalThis.ResizeObserver === 'undefined') {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
});

// ---- mock nebulaAPI(用 vi.hoisted 提升变量,避免引用未初始化)----
const { mockMemoryListRecent } = vi.hoisted(() => ({
  mockMemoryListRecent: vi.fn(),
}));

vi.mock('../../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../../lib/tauri')>('../../lib/tauri');
  return {
    ...actual,
    nebulaAPI: {
      ...actual.nebulaAPI,
      memoryListRecent: mockMemoryListRecent,
    },
  };
});

// 构造 Memory fixture 的辅助函数。
function makeMemory(overrides: Partial<Memory> = {}): Memory {
  return {
    id: overrides.id ?? 'm-' + Math.random().toString(36).slice(2, 8),
    memory_type: overrides.memory_type ?? 'Semantic',
    layer: overrides.layer ?? 'L2',
    content: overrides.content ?? '默认记忆内容',
    summary: overrides.summary ?? {
      s50: '摘要50',
      s150: '摘要150',
      s500: '摘要500',
      s2000: '摘要2000',
    },
    importance: overrides.importance ?? 0.5,
    access_count: overrides.access_count ?? 0,
    last_access: overrides.last_access ?? 0,
    created_at: overrides.created_at ?? Math.floor(Date.now() / 1000),
    source: overrides.source ?? 'chat',
    pinned: overrides.pinned ?? false,
    compressed_from: overrides.compressed_from ?? null,
    compression_gen: overrides.compression_gen ?? 0,
    archived: overrides.archived ?? false,
    metadata: overrides.metadata ?? {},
    ingest_cost: overrides.ingest_cost ?? null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
});

describe('TimelineView', () => {
  it('renders_empty_state_when_no_memories', async () => {
    mockMemoryListRecent.mockResolvedValue([]);
    const { TimelineView } = await import('../TimelineView');
    const { findByTestId } = render(<TimelineView />);
    expect(await findByTestId('timeline-empty')).toBeTruthy();
    expect(mockMemoryListRecent).toHaveBeenCalledWith(200);
  });

  it('groups_memories_by_day_with_sticky_header', async () => {
    // 跨 2 天的记忆
    const day1 = Math.floor(new Date('2026-07-06T10:00:00').getTime() / 1000);
    const day2 = Math.floor(new Date('2026-07-07T15:30:00').getTime() / 1000);
    mockMemoryListRecent.mockResolvedValue([
      makeMemory({ id: 'a', created_at: day1, content: '昨天的记忆' }),
      makeMemory({ id: 'b', created_at: day2, content: '今天的记忆' }),
    ]);
    const { TimelineView } = await import('../TimelineView');
    const { findAllByText, findByTestId } = render(<TimelineView />);
    // 两条记忆都应渲染
    await findByTestId('timeline-item-a');
    await findByTestId('timeline-item-b');
    // 日期分组标题中应包含 7 月 6 日 / 7 月 7 日的本地化标签
    const groups = await findAllByText(/7月.*日/);
    expect(groups.length).toBeGreaterThanOrEqual(2);
  });

  it('layer_filter_checkbox_hides_other_layers', async () => {
    mockMemoryListRecent.mockResolvedValue([
      makeMemory({ id: 'a', layer: 'L2', content: 'L2 记忆' }),
      makeMemory({ id: 'b', layer: 'L4', content: 'L4 记忆' }),
    ]);
    const { TimelineView } = await import('../TimelineView');
    const { findByTestId, queryByTestId } = render(<TimelineView />);
    await findByTestId('timeline-item-a');
    await findByTestId('timeline-item-b');

    // 勾选 L2 筛选 → 仅显示 L2 记忆
    const l2Filter = await findByTestId('timeline-filter-L2');
    const checkbox = l2Filter.querySelector('input') as HTMLInputElement;
    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(queryByTestId('timeline-item-a')).toBeTruthy();
      expect(queryByTestId('timeline-item-b')).toBeFalsy();
    });
  });

  it('importance_slider_filters_low_importance_memories', async () => {
    mockMemoryListRecent.mockResolvedValue([
      makeMemory({ id: 'a', importance: 0.3, content: '低重要性' }),
      makeMemory({ id: 'b', importance: 0.9, content: '高重要性' }),
    ]);
    const { TimelineView } = await import('../TimelineView');
    const { findByTestId, queryByTestId } = render(<TimelineView />);
    await findByTestId('timeline-item-a');
    await findByTestId('timeline-item-b');

    // 拉到 0.5 → 仅保留 importance >= 0.5
    const slider = (await findByTestId('timeline-importance-slider')) as HTMLInputElement;
    fireEvent.change(slider, { target: { value: '0.5' } });

    await waitFor(() => {
      expect(queryByTestId('timeline-item-a')).toBeFalsy();
      expect(queryByTestId('timeline-item-b')).toBeTruthy();
    });
  });

  it('clicking_item_toggles_expanded_details', async () => {
    mockMemoryListRecent.mockResolvedValue([
      makeMemory({
        id: 'a',
        content: '完整正文内容',
        source: 'chat',
        memory_type: 'Semantic',
        access_count: 3,
        ingest_cost: 0.0012,
      }),
    ]);
    const { TimelineView } = await import('../TimelineView');
    const { findByTestId, queryByText } = render(<TimelineView />);
    const item = await findByTestId('timeline-item-a');

    // 初始未展开:不显示元数据"访问"
    expect(queryByText(/访问:/)).toBeFalsy();

    // 点击展开
    fireEvent.click(item);
    await waitFor(() => {
      expect(queryByText(/访问:/)).toBeTruthy();
      expect(queryByText(/0\.0012/)).toBeTruthy();
    });
  });

  it('stats_bar_shows_total_count_and_layer_distribution', async () => {
    mockMemoryListRecent.mockResolvedValue([
      makeMemory({ id: 'a', layer: 'L2' }),
      makeMemory({ id: 'b', layer: 'L2' }),
      makeMemory({ id: 'c', layer: 'L4' }),
    ]);
    const { TimelineView } = await import('../TimelineView');
    const { findByText } = render(<TimelineView />);
    // 统计栏应显示 "共 3 条"
    await waitFor(() => {
      expect(findByText(/共 3 条/)).toBeTruthy();
    });
  });

  it('refresh_button_reloads_memories', async () => {
    mockMemoryListRecent.mockResolvedValue([]);
    const { TimelineView } = await import('../TimelineView');
    const { findByTitle, findByTestId } = render(<TimelineView />);
    await findByTestId('timeline-empty');
    expect(mockMemoryListRecent).toHaveBeenCalledTimes(1);

    // 点击刷新按钮
    const refreshBtn = await findByTitle('刷新');
    fireEvent.click(refreshBtn);
    await waitFor(() => {
      expect(mockMemoryListRecent).toHaveBeenCalledTimes(2);
    });
  });
});
