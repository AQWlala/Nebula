/**
 * T-E-C-10: 异步长任务面板。
 *
 * 用户描述目标 + 步骤序列,后台分步执行(跨小时/跨天)。
 * - 创建任务(目标 + 步骤[描述/程序/参数] + 可选 workspace_id + 可选 plan_id)
 * - 列出所有任务(状态色标 + 进度条)
 * - 展开:步骤时间线(每步状态/输出/错误)
 * - 操作:启动/暂停/恢复/取消/删除
 *
 * 设计要点:
 * - 状态色标:pending 灰/running 蓝/paused 黄/completed 绿/failed 红/cancelled 深灰
 * - 进度条 0-100%
 * - 步骤时间线内联展开(不弹窗)
 * - 取消/删除需二次确认(不可逆操作)
 * - 创建时可添加任意多步骤(动态行)
 */
import { useState, useEffect, useCallback } from 'preact/hooks';
import {
  nebulaAPI,
  type LongTask,
  type LongTaskStatus,
  type LongTaskStep,
  type StepInput,
  type StepStatus,
} from '../lib/tauri';
import { toast } from './Toast';
import { t, type Dict } from '../i18n';

const STATUS_COLORS: Record<LongTaskStatus, string> = {
  pending: '#9CA3AF',
  running: '#3B82F6',
  paused: '#F59E0B',
  completed: '#10B981',
  failed: '#EF4444',
  cancelled: '#6B7280',
};

const STATUS_LABELS: Record<LongTaskStatus, keyof Dict> = {
  pending: 'longTask.status.pending',
  running: 'longTask.status.running',
  paused: 'longTask.status.paused',
  completed: 'longTask.status.completed',
  failed: 'longTask.status.failed',
  cancelled: 'longTask.status.cancelled',
};

const STEP_STATUS_COLORS: Record<StepStatus, string> = {
  pending: '#6B7280',
  running: '#3B82F6',
  done: '#10B981',
  failed: '#EF4444',
  skipped: '#9CA3AF',
};

const STEP_STATUS_LABELS: Record<StepStatus, keyof Dict> = {
  pending: 'longTask.stepStatus.pending',
  running: 'longTask.stepStatus.running',
  done: 'longTask.stepStatus.done',
  failed: 'longTask.stepStatus.failed',
  skipped: 'longTask.stepStatus.skipped',
};

interface StepDraft {
  description: string;
  program: string;
  args: string;
}

function makeStepDraft(): StepDraft {
  return { description: '', program: '', args: '' };
}

function formatTime(unixSec: number | null): string {
  if (!unixSec) return '-';
  return new Date(unixSec * 1000).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

export function LongTaskPanel() {
  const [tasks, setTasks] = useState<LongTask[]>([]);
  const [loading, setLoading] = useState(false);
  // 创建表单
  const [goal, setGoal] = useState('');
  const [workspaceId, setWorkspaceId] = useState('');
  const [planId, setPlanId] = useState('');
  const [steps, setSteps] = useState<StepDraft[]>([makeStepDraft()]);
  const [creating, setCreating] = useState(false);
  // 展开的任务(查看步骤)
  const [expandedTask, setExpandedTask] = useState<string | null>(null);
  const [stepsData, setStepsData] = useState<LongTaskStep[]>([]);
  const [stepsLoading, setStepsLoading] = useState(false);
  const [expandedStep, setExpandedStep] = useState<number | null>(null);
  // 确认对话框
  const [confirmAction, setConfirmAction] = useState<{
    id: string;
    kind: 'cancel' | 'delete';
  } | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await nebulaAPI.longTaskList();
      setTasks(list);
    } catch (e) {
      console.error('long task list failed:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleAddStep = () => {
    setSteps([...steps, makeStepDraft()]);
  };

  const handleRemoveStep = (idx: number) => {
    if (steps.length === 1) {
      toast.warning(t('longTask.toast.needStep'));
      return;
    }
    setSteps(steps.filter((_, i) => i !== idx));
  };

  const handleStepChange = (idx: number, field: keyof StepDraft, value: string) => {
    setSteps(steps.map((s, i) => (i === idx ? { ...s, [field]: value } : s)));
  };

  const handleCreate = async () => {
    if (!goal.trim()) {
      toast.warning(t('longTask.toast.goalRequired'));
      return;
    }
    // 解析步骤
    const stepInputs: StepInput[] = steps
      .filter((s) => s.program.trim())
      .map((s) => ({
        description: s.description.trim(),
        program: s.program.trim(),
        args: s.args.trim() ? s.args.split(/\s+/).filter(Boolean) : [],
      }));
    if (stepInputs.length === 0) {
      toast.warning(t('longTask.toast.needValidStep'));
      return;
    }
    setCreating(true);
    try {
      const task = await nebulaAPI.longTaskCreate(
        goal.trim(),
        stepInputs,
        workspaceId.trim() || null,
        planId.trim() || null
      );
      toast.success(t('longTask.toast.created', { id: task.id.slice(0, 8) }));
      setGoal('');
      setWorkspaceId('');
      setPlanId('');
      setSteps([makeStepDraft()]);
      await refresh();
    } catch (e) {
      toast.error(t('longTask.toast.createFailed'), String(e));
    } finally {
      setCreating(false);
    }
  };

  const handleExpand = async (id: string) => {
    if (expandedTask === id) {
      setExpandedTask(null);
      setStepsData([]);
      setExpandedStep(null);
      return;
    }
    setExpandedTask(id);
    setStepsLoading(true);
    setStepsData([]);
    setExpandedStep(null);
    try {
      const data = await nebulaAPI.longTaskSteps(id);
      setStepsData(data);
    } catch (e) {
      console.error('long task steps failed:', e);
      setStepsData([]);
    } finally {
      setStepsLoading(false);
    }
  };

  const handleStart = async (id: string) => {
    try {
      await nebulaAPI.longTaskStart(id);
      toast.success(t('longTask.toast.started'));
      await refresh();
    } catch (e) {
      toast.error(t('longTask.toast.startFailed'), String(e));
    }
  };

  const handlePause = async (id: string) => {
    try {
      await nebulaAPI.longTaskPause(id);
      toast.success(t('longTask.toast.paused'));
      await refresh();
    } catch (e) {
      toast.error(t('longTask.toast.pauseFailed'), String(e));
    }
  };

  // T-E-C-10: Resume handler — will be wired to UI in future iteration.
  const _handleResume = async (id: string) => {
    try {
      await nebulaAPI.longTaskResume(id);
      toast.success(t('longTask.toast.resumed'));
      await refresh();
    } catch (e) {
      toast.error(t('longTask.toast.resumeFailed'), String(e));
    }
  };
  void _handleResume;

  const handleConfirmAction = async () => {
    if (!confirmAction) return;
    const { id, kind } = confirmAction;
    try {
      if (kind === 'cancel') {
        await nebulaAPI.longTaskCancel(id);
        toast.success(t('longTask.toast.cancelled'));
      } else {
        await nebulaAPI.longTaskDelete(id);
        toast.success(t('longTask.toast.deleted'));
        setExpandedTask(null);
      }
      await refresh();
    } catch (e) {
      toast.error(
        kind === 'cancel' ? t('longTask.toast.cancelFailed') : t('longTask.toast.deleteFailed'),
        String(e)
      );
    } finally {
      setConfirmAction(null);
    }
  };

  return (
    <div
      className="long-task-panel h-full flex flex-col bg-gray-950 text-white"
      data-testid="long-task-panel"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800">
        <h2 className="text-sm font-semibold text-gray-300">{t('longTask.title')}</h2>
        <div className="flex items-center gap-3">
          {loading && <span className="text-xs text-gray-500">{t('longTask.loading')}</span>}
          <span className="text-xs text-gray-500">{t('longTask.taskCount', { n: tasks.length })}</span>
          <button
            onClick={refresh}
            className="text-xs text-gray-400 hover:text-white transition-colors"
            title={t('longTask.refresh')}
          >
            ↻
          </button>
        </div>
      </div>

      {/* 创建表单 */}
      <div className="px-4 py-2 border-b border-gray-800 space-y-2">
        <input
          type="text"
          placeholder={t('longTask.goalPlaceholder')}
          value={goal}
          onInput={(e) => setGoal((e.target as HTMLInputElement).value)}
          className="w-full px-2 py-1 text-sm bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
          data-testid="long-task-goal-input"
        />
        <div className="flex gap-2">
          <input
            type="text"
            placeholder="workspace_id(可选,Shadow Workspace 隔离执行)"
            value={workspaceId}
            onInput={(e) => setWorkspaceId((e.target as HTMLInputElement).value)}
            className="flex-1 px-2 py-1 text-xs bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
            data-testid="long-task-workspace-input"
          />
          <input
            type="text"
            placeholder="plan_id(可选,关联 PlanEngine)"
            value={planId}
            onInput={(e) => setPlanId((e.target as HTMLInputElement).value)}
            className="flex-1 px-2 py-1 text-xs bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
            data-testid="long-task-plan-input"
          />
        </div>
        {/* 步骤编辑器 */}
        <div className="space-y-1">
          {steps.map((step, idx) => (
            <div
              key={idx}
              className="flex gap-1 items-center"
              data-testid={`long-task-step-row-${idx}`}
            >
              <span className="text-xs text-gray-500 w-6">#{idx + 1}</span>
              <input
                type="text"
                placeholder={t('longTask.stepDescPlaceholder')}
                value={step.description}
                onInput={(e) =>
                  handleStepChange(idx, 'description', (e.target as HTMLInputElement).value)
                }
                className="flex-1 px-2 py-1 text-xs bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
                data-testid={`long-task-step-desc-${idx}`}
              />
              <input
                type="text"
                placeholder={t('longTask.programPlaceholder')}
                value={step.program}
                onInput={(e) =>
                  handleStepChange(idx, 'program', (e.target as HTMLInputElement).value)
                }
                className="w-32 px-2 py-1 text-xs bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
                data-testid={`long-task-step-program-${idx}`}
              />
              <input
                type="text"
                placeholder={t('longTask.argsPlaceholder')}
                value={step.args}
                onInput={(e) => handleStepChange(idx, 'args', (e.target as HTMLInputElement).value)}
                className="w-40 px-2 py-1 text-xs bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
                data-testid={`long-task-step-args-${idx}`}
              />
              <button
                onClick={() => handleRemoveStep(idx)}
                className="px-2 py-1 text-xs text-red-400 hover:text-red-300"
                title={t('longTask.removeStep')}
                data-testid={`long-task-step-remove-${idx}`}
              >
                ✕
              </button>
            </div>
          ))}
          <button
            onClick={handleAddStep}
            className="text-xs text-blue-400 hover:text-blue-300"
            data-testid="long-task-add-step-btn"
          >
            {t('longTask.addStep')}
          </button>
        </div>
        <button
          onClick={handleCreate}
          disabled={creating || !goal.trim()}
          className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed rounded text-white transition-colors"
          data-testid="long-task-create-btn"
        >
          {creating ? t('longTask.creating') : t('longTask.create')}
        </button>
      </div>

      {/* 任务列表 */}
      <div className="flex-1 overflow-y-auto">
        {tasks.length === 0 && !loading && (
          <div className="text-center text-gray-500 py-12" data-testid="long-task-empty">
            {t('longTask.empty')}
          </div>
        )}
        {tasks.map((task) => (
          <div
            key={task.id}
            className="border-b border-gray-800 px-4 py-3 hover:bg-gray-900/50"
            data-testid={`long-task-item-${task.id}`}
          >
            {/* 任务头部 */}
            <div className="flex items-start justify-between">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <span
                    className="inline-block w-2 h-2 rounded-full"
                    style={{ backgroundColor: STATUS_COLORS[task.status] }}
                  />
                  <span
                    className="text-xs font-medium"
                    style={{ color: STATUS_COLORS[task.status] }}
                    data-testid={`long-task-status-${task.id}`}
                  >
                    {t(STATUS_LABELS[task.status])}
                  </span>
                  <span className="text-xs text-gray-600">·</span>
                  <span className="text-xs text-gray-500 font-mono">{task.id.slice(0, 8)}</span>
                </div>
                <div
                  className="text-sm text-white truncate"
                  data-testid={`long-task-goal-${task.id}`}
                >
                  {task.goal}
                </div>
                {/* 进度条 */}
                <div className="mt-2 flex items-center gap-2">
                  <div className="flex-1 h-1.5 bg-gray-800 rounded-full overflow-hidden">
                    <div
                      className="h-full transition-all"
                      style={{
                        width: `${task.progress}%`,
                        backgroundColor: STATUS_COLORS[task.status],
                      }}
                      data-testid={`long-task-progress-${task.id}`}
                    />
                  </div>
                  <span className="text-xs text-gray-500 w-10 text-right">{task.progress}%</span>
                </div>
                {/* 元信息 */}
                <div className="mt-1 flex items-center gap-3 text-xs text-gray-600">
                  <span>{t('longTask.createdAt', { time: formatTime(task.created_at) })}</span>
                  {task.started_at && (
                    <span>{t('longTask.startedAt', { time: formatTime(task.started_at) })}</span>
                  )}
                  {task.finished_at && (
                    <span>{t('longTask.finishedAt', { time: formatTime(task.finished_at) })}</span>
                  )}
                  {task.workspace_id && (
                    <span className="text-blue-500/70">ws: {task.workspace_id.slice(0, 8)}</span>
                  )}
                  {task.plan_id && (
                    <span className="text-purple-500/70">plan: {task.plan_id.slice(0, 8)}</span>
                  )}
                </div>
                {task.error && (
                  <div
                    className="mt-1 text-xs text-red-400 truncate"
                    data-testid={`long-task-error-${task.id}`}
                  >
                    ⚠ {task.error}
                  </div>
                )}
              </div>
            </div>

            {/* 操作按钮 */}
            <div className="mt-2 flex items-center gap-2 flex-wrap">
              {(task.status === 'pending' || task.status === 'paused') && (
                <button
                  onClick={() => handleStart(task.id)}
                  className="px-2 py-0.5 text-xs bg-green-600 hover:bg-green-700 rounded text-white"
                  data-testid={`long-task-start-btn-${task.id}`}
                >
                  {task.status === 'paused' ? t('longTask.resume') : t('longTask.start')}
                </button>
              )}
              {task.status === 'running' && (
                <button
                  onClick={() => handlePause(task.id)}
                  className="px-2 py-0.5 text-xs bg-yellow-600 hover:bg-yellow-700 rounded text-white"
                  data-testid={`long-task-pause-btn-${task.id}`}
                >
                  {t('longTask.pause')}
                </button>
              )}
              {!['completed', 'cancelled'].includes(task.status) && (
                <button
                  onClick={() => setConfirmAction({ id: task.id, kind: 'cancel' })}
                  className="px-2 py-0.5 text-xs bg-orange-600 hover:bg-orange-700 rounded text-white"
                  data-testid={`long-task-cancel-btn-${task.id}`}
                >
                  {t('longTask.cancel')}
                </button>
              )}
              <button
                onClick={() => setConfirmAction({ id: task.id, kind: 'delete' })}
                className="px-2 py-0.5 text-xs bg-red-900 hover:bg-red-800 rounded text-white"
                data-testid={`long-task-delete-btn-${task.id}`}
              >
                {t('longTask.delete')}
              </button>
              <button
                onClick={() => handleExpand(task.id)}
                className="px-2 py-0.5 text-xs bg-gray-700 hover:bg-gray-600 rounded text-white"
                data-testid={`long-task-expand-btn-${task.id}`}
              >
                {expandedTask === task.id ? t('longTask.collapse') : t('longTask.steps')}
              </button>
            </div>

            {/* 步骤时间线(展开) */}
            {expandedTask === task.id && (
              <div
                className="mt-3 pl-4 border-l border-gray-800"
                data-testid={`long-task-steps-view-${task.id}`}
              >
                {stepsLoading && <div className="text-xs text-gray-500">{t('longTask.loadingSteps')}</div>}
                {!stepsLoading && stepsData.length === 0 && (
                  <div
                    className="text-xs text-gray-600"
                    data-testid={`long-task-steps-empty-${task.id}`}
                  >
                    {t('longTask.noStepData')}
                  </div>
                )}
                {stepsData.map((step) => (
                  <div
                    key={step.seq}
                    className="mb-2 cursor-pointer"
                    onClick={() => setExpandedStep(expandedStep === step.seq ? null : step.seq)}
                    data-testid={`long-task-step-${task.id}-${step.seq}`}
                  >
                    <div className="flex items-center gap-2">
                      <span
                        className="inline-block w-1.5 h-1.5 rounded-full"
                        style={{ backgroundColor: STEP_STATUS_COLORS[step.status] }}
                      />
                      <span className="text-xs text-gray-500">#{step.seq}</span>
                      <span
                        className="text-xs font-medium"
                        style={{ color: STEP_STATUS_COLORS[step.status] }}
                      >
                        {t(STEP_STATUS_LABELS[step.status])}
                      </span>
                      <code className="text-xs text-gray-300">
                        {step.program} {step.args.join(' ')}
                      </code>
                    </div>
                    {step.description && (
                      <div className="ml-4 text-xs text-gray-500">{step.description}</div>
                    )}
                    {expandedStep === step.seq && (
                      <div
                        className="ml-4 mt-1 p-2 bg-gray-900 rounded text-xs font-mono text-gray-400 max-h-40 overflow-y-auto whitespace-pre-wrap"
                        data-testid={`long-task-step-detail-${task.id}-${step.seq}`}
                      >
                        {step.output && <div className="text-green-400/70">{step.output}</div>}
                        {step.error && <div className="text-red-400/70">⚠ {step.error}</div>}
                        {step.exit_code !== null && (
                          <div className="text-gray-600">exit: {step.exit_code}</div>
                        )}
                        {!step.output && !step.error && step.exit_code === null && (
                          <div className="text-gray-600">{t('longTask.noOutput')}</div>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>

      {/* 确认对话框 */}
      {confirmAction && (
        <div
          className="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
          data-testid="long-task-confirm-dialog"
        >
          <div className="bg-gray-900 border border-gray-700 rounded p-4 max-w-sm">
            <div className="text-sm text-white mb-3">
              {confirmAction.kind === 'cancel'
                ? t('longTask.confirmCancel')
                : t('longTask.confirmDelete')}
            </div>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConfirmAction(null)}
                className="px-3 py-1 text-xs bg-gray-700 hover:bg-gray-600 rounded text-white"
                data-testid="long-task-confirm-cancel"
              >
                {t('longTask.dialogCancel')}
              </button>
              <button
                onClick={handleConfirmAction}
                className="px-3 py-1 text-xs bg-red-600 hover:bg-red-700 rounded text-white"
                data-testid="long-task-confirm-ok"
              >
                {t('longTask.dialogConfirm')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
