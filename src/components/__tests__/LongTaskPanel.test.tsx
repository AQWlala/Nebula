/**
 * T-E-C-10: LongTaskPanel 前端测试。
 *
 * 覆盖:渲染/空状态/创建/列表/状态色标/进度条/步骤展开/操作按钮/确认对话框。
 * mock nebulaAPI 的 long_task_* 方法。
 */
import { describe, it, expect, beforeAll, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@testing-library/preact';
import type { LongTask, LongTaskStep } from '../../lib/tauri';

beforeAll(() => {
  if (typeof globalThis.ResizeObserver === 'undefined') {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
});

const { mockList, mockCreate, mockSteps, mockStart, mockPause, mockCancel, mockDelete } =
  vi.hoisted(() => ({
    mockList: vi.fn(),
    mockCreate: vi.fn(),
    mockSteps: vi.fn(),
    mockStart: vi.fn(),
    mockPause: vi.fn(),
    mockCancel: vi.fn(),
    mockDelete: vi.fn(),
  }));

vi.mock('../../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../../lib/tauri')>('../../lib/tauri');
  return {
    ...actual,
    nebulaAPI: {
      ...actual.nebulaAPI,
      longTaskList: mockList,
      longTaskCreate: mockCreate,
      longTaskSteps: mockSteps,
      longTaskStart: mockStart,
      longTaskPause: mockPause,
      longTaskCancel: mockCancel,
      longTaskDelete: mockDelete,
    },
  };
});

function makeTask(overrides: Partial<LongTask> = {}): LongTask {
  return {
    id: overrides.id ?? 'task-0001',
    goal: overrides.goal ?? '测试任务',
    status: overrides.status ?? 'pending',
    workspace_id: overrides.workspace_id ?? null,
    plan_id: overrides.plan_id ?? null,
    progress: overrides.progress ?? 0,
    error: overrides.error ?? null,
    created_at: overrides.created_at ?? Math.floor(Date.now() / 1000),
    updated_at: overrides.updated_at ?? Math.floor(Date.now() / 1000),
    started_at: overrides.started_at ?? null,
    finished_at: overrides.finished_at ?? null,
  };
}

function makeStep(overrides: Partial<LongTaskStep> = {}): LongTaskStep {
  return {
    task_id: overrides.task_id ?? 'task-0001',
    seq: overrides.seq ?? 1,
    description: overrides.description ?? '第一步',
    program: overrides.program ?? 'echo',
    args: overrides.args ?? ['hello'],
    status: overrides.status ?? 'done',
    started_at: overrides.started_at ?? Math.floor(Date.now() / 1000),
    finished_at: overrides.finished_at ?? Math.floor(Date.now() / 1000),
    exit_code: overrides.exit_code ?? 0,
    output: overrides.output ?? 'hello\n',
    error: overrides.error ?? null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
});

describe('LongTaskPanel', () => {
  it('renders_empty_state_when_no_tasks', async () => {
    mockList.mockResolvedValue([]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    expect(await findByTestId('long-task-empty')).toBeTruthy();
  });

  it('lists_tasks_with_status_badge_and_progress', async () => {
    mockList.mockResolvedValue([
      makeTask({ id: 'aaa11111', goal: '任务 A', status: 'running', progress: 50 }),
      makeTask({ id: 'bbb22222', goal: '任务 B', status: 'completed', progress: 100 }),
    ]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, getByText } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-aaa11111');
    await findByTestId('long-task-item-bbb22222');
    expect(getByText('任务 A')).toBeTruthy();
    expect(getByText('任务 B')).toBeTruthy();
    expect(getByText('执行中')).toBeTruthy();
    expect(getByText('已完成')).toBeTruthy();
    // 进度条宽度
    const progressA = await findByTestId('long-task-progress-aaa11111');
    expect((progressA as HTMLElement).style.width).toBe('50%');
  });

  it('create_button_calls_longTaskCreate_with_parsed_steps', async () => {
    mockList.mockResolvedValue([]);
    mockCreate.mockResolvedValue(makeTask({ id: 'new00001', goal: '新任务' }));
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-empty');

    // 填写目标
    const goalInput = (await findByTestId('long-task-goal-input')) as HTMLInputElement;
    fireEvent.input(goalInput, { target: { value: '新任务' } });

    // 填写第一步
    const desc0 = (await findByTestId('long-task-step-desc-0')) as HTMLInputElement;
    const prog0 = (await findByTestId('long-task-step-program-0')) as HTMLInputElement;
    const args0 = (await findByTestId('long-task-step-args-0')) as HTMLInputElement;
    fireEvent.input(desc0, { target: { value: '编译' } });
    fireEvent.input(prog0, { target: { value: 'cargo' } });
    fireEvent.input(args0, { target: { value: 'build --release' } });

    // 点击创建
    const btn = (await findByTestId('long-task-create-btn')) as HTMLButtonElement;
    fireEvent.click(btn);

    await waitFor(() => {
      expect(mockCreate).toHaveBeenCalledWith(
        '新任务',
        [{ description: '编译', program: 'cargo', args: ['build', '--release'] }],
        null,
        null
      );
    });
  });

  it('create_button_disabled_when_goal_empty', async () => {
    mockList.mockResolvedValue([]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-empty');
    const btn = (await findByTestId('long-task-create-btn')) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it('add_step_button_adds_new_step_row', async () => {
    mockList.mockResolvedValue([]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-empty');

    // 初始有 1 行
    expect(await findByTestId('long-task-step-row-0')).toBeTruthy();

    // 点击添加
    fireEvent.click(await findByTestId('long-task-add-step-btn'));
    expect(await findByTestId('long-task-step-row-1')).toBeTruthy();
  });

  it('pending_task_shows_start_button', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'pnd00001', status: 'pending' })]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    expect(await findByTestId('long-task-start-btn-pnd00001')).toBeTruthy();
  });

  it('running_task_shows_pause_button', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'run00001', status: 'running' })]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    expect(await findByTestId('long-task-pause-btn-run00001')).toBeTruthy();
  });

  it('paused_task_shows_resume_button', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'pau00001', status: 'paused' })]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    const btn = await findByTestId('long-task-start-btn-pau00001');
    expect(btn.textContent).toContain('恢复');
  });

  it('clicking_start_calls_longTaskStart', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'sta00001', status: 'pending' })]);
    mockStart.mockResolvedValue(makeTask({ id: 'sta00001', status: 'running' }));
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    fireEvent.click(await findByTestId('long-task-start-btn-sta00001'));
    await waitFor(() => {
      expect(mockStart).toHaveBeenCalledWith('sta00001');
    });
  });

  it('clicking_pause_calls_longTaskPause', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'pse00001', status: 'running' })]);
    mockPause.mockResolvedValue(makeTask({ id: 'pse00001', status: 'paused' }));
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    fireEvent.click(await findByTestId('long-task-pause-btn-pse00001'));
    await waitFor(() => {
      expect(mockPause).toHaveBeenCalledWith('pse00001');
    });
  });

  it('cancel_shows_confirmation_dialog_then_calls_longTaskCancel', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'cnl00001', status: 'running' })]);
    mockCancel.mockResolvedValue(makeTask({ id: 'cnl00001', status: 'cancelled' }));
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, queryByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-cnl00001');

    // 点击取消
    fireEvent.click(await findByTestId('long-task-cancel-btn-cnl00001'));

    // 确认对话框出现
    expect(await findByTestId('long-task-confirm-dialog')).toBeTruthy();

    // 点击确认
    fireEvent.click(await findByTestId('long-task-confirm-ok'));
    await waitFor(() => {
      expect(mockCancel).toHaveBeenCalledWith('cnl00001');
      expect(queryByTestId('long-task-confirm-dialog')).toBeFalsy();
    });
  });

  it('delete_shows_confirmation_dialog_then_calls_longTaskDelete', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'del00001', status: 'completed' })]);
    mockDelete.mockResolvedValue(true);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-del00001');

    fireEvent.click(await findByTestId('long-task-delete-btn-del00001'));
    expect(await findByTestId('long-task-confirm-dialog')).toBeTruthy();

    fireEvent.click(await findByTestId('long-task-confirm-ok'));
    await waitFor(() => {
      expect(mockDelete).toHaveBeenCalledWith('del00001');
    });
  });

  it('expand_button_toggles_steps_view', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'exp00001', status: 'completed' })]);
    mockSteps.mockResolvedValue([makeStep({ task_id: 'exp00001', seq: 1 })]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, queryByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-exp00001');

    // 初始无步骤视图
    expect(queryByTestId('long-task-steps-view-exp00001')).toBeFalsy();

    // 点击展开
    fireEvent.click(await findByTestId('long-task-expand-btn-exp00001'));
    await waitFor(() => {
      expect(mockSteps).toHaveBeenCalledWith('exp00001');
      expect(queryByTestId('long-task-steps-view-exp00001')).toBeTruthy();
    });

    // 再次点击收起
    fireEvent.click(await findByTestId('long-task-expand-btn-exp00001'));
    await waitFor(() => {
      expect(queryByTestId('long-task-steps-view-exp00001')).toBeFalsy();
    });
  });

  it('steps_view_renders_step_timeline', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'tlp00001', status: 'completed' })]);
    mockSteps.mockResolvedValue([
      makeStep({
        task_id: 'tlp00001',
        seq: 1,
        program: 'cargo',
        args: ['build'],
        status: 'done',
        output: 'Compiling...',
      }),
      makeStep({
        task_id: 'tlp00001',
        seq: 2,
        program: 'cargo',
        args: ['test'],
        status: 'failed',
        error: '3 tests failed',
      }),
    ]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, getByText } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-tlp00001');
    fireEvent.click(await findByTestId('long-task-expand-btn-tlp00001'));

    // 两个步骤都渲染
    await findByTestId('long-task-step-tlp00001-1');
    await findByTestId('long-task-step-tlp00001-2');
    // 程序名渲染
    expect(getByText(/cargo build/)).toBeTruthy();
    expect(getByText(/cargo test/)).toBeTruthy();
  });

  it('clicking_step_expands_detail', async () => {
    mockList.mockResolvedValue([makeTask({ id: 'det00001', status: 'completed' })]);
    mockSteps.mockResolvedValue([
      makeStep({
        task_id: 'det00001',
        seq: 1,
        program: 'echo',
        args: ['hi'],
        status: 'done',
        output: 'hi',
      }),
    ]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, queryByTestId } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-det00001');
    fireEvent.click(await findByTestId('long-task-expand-btn-det00001'));
    const stepItem = await findByTestId('long-task-step-det00001-1');

    // 初始无展开
    expect(queryByTestId('long-task-step-detail-det00001-1')).toBeFalsy();

    // 点击展开
    fireEvent.click(stepItem);
    await waitFor(() => {
      expect(queryByTestId('long-task-step-detail-det00001-1')).toBeTruthy();
    });

    // 再次点击收起
    fireEvent.click(stepItem);
    await waitFor(() => {
      expect(queryByTestId('long-task-step-detail-det00001-1')).toBeFalsy();
    });
  });

  it('failed_task_shows_error_message', async () => {
    mockList.mockResolvedValue([
      makeTask({ id: 'err00001', status: 'failed', error: 'compilation failed' }),
    ]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, getByText } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-err00001');
    expect(getByText(/compilation failed/)).toBeTruthy();
  });

  it('workspace_and_plan_ids_displayed_when_present', async () => {
    mockList.mockResolvedValue([
      makeTask({ id: 'ws000001', workspace_id: 'ws-abc123', plan_id: 'plan-xyz' }),
    ]);
    const { LongTaskPanel } = await import('../LongTaskPanel');
    const { findByTestId, getByText } = render(<LongTaskPanel />);
    await findByTestId('long-task-item-ws000001');
    // workspace_id 和 plan_id 都截断到 8 字符显示
    expect(getByText(/ws: ws-abc12/)).toBeTruthy();
    expect(getByText(/plan: plan-xyz/)).toBeTruthy();
  });
});
