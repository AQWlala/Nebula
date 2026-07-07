/**
 * T-E-C-08: ShadowWorkspacePanel 前端测试。
 *
 * 覆盖:渲染/空状态/创建/列表/diff 展开/合并确认/丢弃确认/状态色标。
 * mock nebulaAPI 的 shadow_* 方法。
 */
import { describe, it, expect, beforeAll, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/preact';
import type { ShadowWorkspace } from '../../lib/tauri';

beforeAll(() => {
  if (typeof globalThis.ResizeObserver === 'undefined') {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
});

const { mockShadowList, mockShadowCreate, mockShadowDiff, mockShadowMerge, mockShadowAbort } = vi.hoisted(() => ({
  mockShadowList: vi.fn(),
  mockShadowCreate: vi.fn(),
  mockShadowDiff: vi.fn(),
  mockShadowMerge: vi.fn(),
  mockShadowAbort: vi.fn(),
}));

vi.mock('../../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../../lib/tauri')>('../../lib/tauri');
  return {
    ...actual,
    nebulaAPI: {
      ...actual.nebulaAPI,
      shadowList: mockShadowList,
      shadowCreate: mockShadowCreate,
      shadowDiff: mockShadowDiff,
      shadowMerge: mockShadowMerge,
      shadowAbort: mockShadowAbort,
    },
  };
});

function makeWs(overrides: Partial<ShadowWorkspace> = {}): ShadowWorkspace {
  return {
    id: overrides.id ?? 'abc12345',
    branch: overrides.branch ?? 'agent/abc12345',
    path: overrides.path ?? '/tmp/nebula-shadow-ws/abc12345',
    task_description: overrides.task_description ?? '测试任务',
    status: overrides.status ?? 'running',
    created_at: overrides.created_at ?? Math.floor(Date.now() / 1000),
    finished_at: overrides.finished_at ?? null,
    base_branch: overrides.base_branch ?? 'main',
    error: overrides.error ?? null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
});

describe('ShadowWorkspacePanel', () => {
  it('renders_empty_state_when_no_workspaces', async () => {
    mockShadowList.mockResolvedValue([]);
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByTestId } = render(<ShadowWorkspacePanel />);
    expect(await findByTestId('shadow-empty')).toBeTruthy();
  });

  it('lists_workspaces_with_status_badge', async () => {
    mockShadowList.mockResolvedValue([
      makeWs({ id: 'aaa11111', status: 'running', task_description: '运行中任务' }),
      makeWs({ id: 'bbb22222', status: 'completed', task_description: '已完成任务' }),
    ]);
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByTestId, getByText } = render(<ShadowWorkspacePanel />);
    await findByTestId('shadow-item-aaa11111');
    await findByTestId('shadow-item-bbb22222');
    expect(getByText('运行中任务')).toBeTruthy();
    expect(getByText('已完成任务')).toBeTruthy();
    expect(getByText('运行中')).toBeTruthy();
    expect(getByText('已完成')).toBeTruthy();
  });

  it('create_button_calls_shadowCreate_and_refreshes', async () => {
    mockShadowList.mockResolvedValue([]);
    mockShadowCreate.mockResolvedValue(makeWs({ id: 'new12345', task_description: '新任务' }));
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByTestId } = render(<ShadowWorkspacePanel />);
    await findByTestId('shadow-empty');

    const input = await findByTestId('shadow-task-input') as HTMLInputElement;
    const btn = await findByTestId('shadow-create-btn') as HTMLButtonElement;
    fireEvent.input(input, { target: { value: '新任务' } });
    fireEvent.click(btn);

    await waitFor(() => {
      expect(mockShadowCreate).toHaveBeenCalledWith('新任务', null);
    });
  });

  it('diff_button_toggles_diff_view', async () => {
    mockShadowList.mockResolvedValue([makeWs({ id: 'diff0001', status: 'completed' })]);
    mockShadowDiff.mockResolvedValue('--- a/file\n+++ b/file\n@@ -1 +1 @@\n-hello\n+world\n');
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByTestId, queryByTestId } = render(<ShadowWorkspacePanel />);
    const diffBtn = await findByTestId('shadow-diff-btn-diff0001');

    // 初始无 diff 视图
    expect(queryByTestId('shadow-diff-view-diff0001')).toBeFalsy();

    // 点击展开
    fireEvent.click(diffBtn);
    await waitFor(() => {
      expect(mockShadowDiff).toHaveBeenCalledWith('diff0001');
      expect(queryByTestId('shadow-diff-view-diff0001')).toBeTruthy();
    });

    // 再次点击关闭
    fireEvent.click(diffBtn);
    await waitFor(() => {
      expect(queryByTestId('shadow-diff-view-diff0001')).toBeFalsy();
    });
  });

  it('merge_shows_confirmation_dialog_then_calls_shadowMerge', async () => {
    mockShadowList.mockResolvedValue([makeWs({ id: 'mrg00001', status: 'completed' })]);
    mockShadowMerge.mockResolvedValue(makeWs({ id: 'mrg00001', status: 'merged' }));
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByText, findByTestId, queryByTestId } = render(<ShadowWorkspacePanel />);
    await findByTestId('shadow-item-mrg00001');

    // 点击合并按钮
    fireEvent.click(await findByText('合并'));

    // 确认对话框出现
    const dialog = await findByTestId('shadow-confirm-dialog');
    expect(dialog).toBeTruthy();

    // 点击确认
    const confirmBtn = await findByTestId('shadow-confirm-btn');
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(mockShadowMerge).toHaveBeenCalledWith('mrg00001');
      expect(queryByTestId('shadow-confirm-dialog')).toBeFalsy();
    });
  });

  it('abort_shows_confirmation_dialog_then_calls_shadowAbort', async () => {
    mockShadowList.mockResolvedValue([makeWs({ id: 'abt00001', status: 'running' })]);
    mockShadowAbort.mockResolvedValue(makeWs({ id: 'abt00001', status: 'aborted' }));
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByText, findByTestId } = render(<ShadowWorkspacePanel />);
    await findByTestId('shadow-item-abt00001');

    fireEvent.click(await findByText('丢弃'));
    await findByTestId('shadow-confirm-dialog');

    const confirmBtn = await findByTestId('shadow-confirm-btn');
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(mockShadowAbort).toHaveBeenCalledWith('abt00001');
    });
  });

  it('failed_status_shows_error_message', async () => {
    mockShadowList.mockResolvedValue([
      makeWs({ id: 'err00001', status: 'failed', error: 'compilation error' }),
    ]);
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByTestId, getByText } = render(<ShadowWorkspacePanel />);
    await findByTestId('shadow-item-err00001');
    expect(getByText(/compilation error/)).toBeTruthy();
  });

  it('create_button_disabled_when_task_empty', async () => {
    mockShadowList.mockResolvedValue([]);
    const { ShadowWorkspacePanel } = await import('../ShadowWorkspacePanel');
    const { findByTestId } = render(<ShadowWorkspacePanel />);
    await findByTestId('shadow-empty');
    const btn = await findByTestId('shadow-create-btn') as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });
});
