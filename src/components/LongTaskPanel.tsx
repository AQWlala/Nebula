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

/** LongTaskStatus → task-status CSS 类名映射(running/done/queued/failed)。 */
const STATUS_TASK_CLASS: Record<LongTaskStatus, string> = {
  pending: 'queued',
  running: 'running',
  paused: 'queued',
  completed: 'done',
  failed: 'failed',
  cancelled: 'queued',
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
      class="long-task-panel"
      style="display:flex;flex-direction:column;height:100%;"
      data-testid="long-task-panel"
    >
      {/* 页面头:标题 + 工具按钮 */}
      <div class="page-header">
        <div>
          <div class="page-title">⏳ {t('longTask.title')}</div>
          <div class="page-subtitle">
            {loading && <span>{t('longTask.loading')} · </span>}
            {t('longTask.taskCount', { n: tasks.length })}
          </div>
        </div>
        <div class="page-actions">
          <button class="tool-btn" onClick={refresh} title={t('longTask.refresh')}>
            ↻ {t('longTask.refresh')}
          </button>
        </div>
      </div>

      <div class="page-body" style="display:flex;flex-direction:column;gap:12px;">
        {/* 创建表单 */}
        <div class="stat-card" style="padding:14px 16px;">
          <input
            type="text"
            placeholder={t('longTask.goalPlaceholder')}
            value={goal}
            onInput={(e) => setGoal((e.target as HTMLInputElement).value)}
            style="width:100%;padding:7px 12px;font-size:13px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:8px;color:inherit;outline:none;margin-bottom:8px;"
            data-testid="long-task-goal-input"
          />
          <div style="display:flex;gap:8px;margin-bottom:8px;">
            <input
              type="text"
              placeholder="workspace_id(可选,Shadow Workspace 隔离执行)"
              value={workspaceId}
              onInput={(e) => setWorkspaceId((e.target as HTMLInputElement).value)}
              style="flex:1;padding:6px 10px;font-size:12px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:7px;color:inherit;outline:none;"
              data-testid="long-task-workspace-input"
            />
            <input
              type="text"
              placeholder="plan_id(可选,关联 PlanEngine)"
              value={planId}
              onInput={(e) => setPlanId((e.target as HTMLInputElement).value)}
              style="flex:1;padding:6px 10px;font-size:12px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:7px;color:inherit;outline:none;"
              data-testid="long-task-plan-input"
            />
          </div>
          {/* 步骤编辑器 */}
          <div style="display:flex;flex-direction:column;gap:6px;margin-bottom:10px;">
            {steps.map((step, idx) => (
              <div
                key={idx}
                style="display:flex;gap:6px;align-items:center;"
                data-testid={`long-task-step-row-${idx}`}
              >
                <span style="font-size:12px;color:rgba(255,255,255,0.4);width:24px;">#{idx + 1}</span>
                <input
                  type="text"
                  placeholder={t('longTask.stepDescPlaceholder')}
                  value={step.description}
                  onInput={(e) =>
                    handleStepChange(idx, 'description', (e.target as HTMLInputElement).value)
                  }
                  style="flex:1;padding:5px 8px;font-size:12px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:6px;color:inherit;outline:none;"
                  data-testid={`long-task-step-desc-${idx}`}
                />
                <input
                  type="text"
                  placeholder={t('longTask.programPlaceholder')}
                  value={step.program}
                  onInput={(e) =>
                    handleStepChange(idx, 'program', (e.target as HTMLInputElement).value)
                  }
                  style="width:128px;padding:5px 8px;font-size:12px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:6px;color:inherit;outline:none;"
                  data-testid={`long-task-step-program-${idx}`}
                />
                <input
                  type="text"
                  placeholder={t('longTask.argsPlaceholder')}
                  value={step.args}
                  onInput={(e) => handleStepChange(idx, 'args', (e.target as HTMLInputElement).value)}
                  style="width:160px;padding:5px 8px;font-size:12px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:6px;color:inherit;outline:none;"
                  data-testid={`long-task-step-args-${idx}`}
                />
                <button
                  onClick={() => handleRemoveStep(idx)}
                  class="tool-btn"
                  style="padding:4px 8px;font-size:12px;color:#ff5f57;cursor:pointer;border:none;background:none;"
                  title={t('longTask.removeStep')}
                  data-testid={`long-task-step-remove-${idx}`}
                >
                  ✕
                </button>
              </div>
            ))}
            <button
              onClick={handleAddStep}
              class="tool-btn"
              style="padding:4px 8px;font-size:12px;color:#0A84FF;cursor:pointer;border:none;background:none;"
              data-testid="long-task-add-step-btn"
            >
              + {t('longTask.addStep')}
            </button>
          </div>
          <button
            onClick={handleCreate}
            disabled={creating || !goal.trim()}
            class="tool-btn tool-btn-primary"
            style={{ cursor: 'pointer', opacity: creating || !goal.trim() ? 0.4 : 1 }}
            data-testid="long-task-create-btn"
          >
            {creating ? t('longTask.creating') : t('longTask.create')}
          </button>
        </div>

        {/* 任务列表 */}
        <div class="task-list" style="flex:1;overflow-y:auto;">
          {tasks.length === 0 && !loading && (
            <div
              style="text-align:center;color:rgba(255,255,255,0.3);padding:48px 0;"
              data-testid="long-task-empty"
            >
              {t('longTask.empty')}
            </div>
          )}
          {tasks.map((task) => (
            <div
              key={task.id}
              class="task-card"
              data-testid={`long-task-item-${task.id}`}
            >
              {/* 任务头部:状态徽章 + 目标 */}
              <div class="task-header">
                <div style="flex:1;min-width:0;">
                  <div style="display:flex;align-items:center;gap:8px;margin-bottom:4px;">
                    <span
                      class={`task-status ${STATUS_TASK_CLASS[task.status]}`}
                      data-testid={`long-task-status-${task.id}`}
                    >
                      {t(STATUS_LABELS[task.status])}
                    </span>
                    <span style="font-size:12px;color:rgba(255,255,255,0.3);font-family:monospace;">
                      {task.id.slice(0, 8)}
                    </span>
                  </div>
                  <div
                    style="font-size:14px;color:rgba(255,255,255,0.95);"
                    data-testid={`long-task-goal-${task.id}`}
                  >
                    {task.goal}
                  </div>
                </div>
              </div>

              {/* 进度条 */}
              <div class="task-progress-bar">
                <div
                  class="task-progress-fill"
                  style={`width:${task.progress}%;background:${STATUS_COLORS[task.status]};`}
                  data-testid={`long-task-progress-${task.id}`}
                />
              </div>
              <div
                style="display:flex;justify-content:space-between;font-size:12px;color:rgba(255,255,255,0.4);margin-top:6px;"
              >
                <span>{task.progress}%</span>
                {/* 耗时 + 预计剩余时间(用现有时间戳显示) */}
                {task.started_at && !task.finished_at && (
                  <span>{t('longTask.startedAt', { time: formatTime(task.started_at) })}</span>
                )}
                {task.finished_at && (
                  <span>{t('longTask.finishedAt', { time: formatTime(task.finished_at) })}</span>
                )}
              </div>

              {/* 元信息 */}
              <div
                style="display:flex;align-items:center;gap:12px;font-size:12px;color:rgba(255,255,255,0.3);margin-top:6px;flex-wrap:wrap;"
              >
                <span>{t('longTask.createdAt', { time: formatTime(task.created_at) })}</span>
                {task.workspace_id && (
                  <span style="color:rgba(59,130,246,0.6);">ws: {task.workspace_id.slice(0, 8)}</span>
                )}
                {task.plan_id && (
                  <span style="color:rgba(139,92,246,0.6);">plan: {task.plan_id.slice(0, 8)}</span>
                )}
              </div>
              {task.error && (
                <div
                  style="font-size:12px;color:#ff5f57;margin-top:4px;"
                  data-testid={`long-task-error-${task.id}`}
                >
                  ⚠ {task.error}
                </div>
              )}

              {/* 操作按钮 */}
              <div
                style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:10px;"
              >
                {(task.status === 'pending' || task.status === 'paused') && (
                  <button
                    onClick={() => handleStart(task.id)}
                    class="tool-btn tool-btn-primary"
                    style="padding:4px 12px;font-size:12px;cursor:pointer;border:none;"
                    data-testid={`long-task-start-btn-${task.id}`}
                  >
                    {task.status === 'paused' ? t('longTask.resume') : t('longTask.start')}
                  </button>
                )}
                {task.status === 'running' && (
                  <button
                    onClick={() => handlePause(task.id)}
                    class="tool-btn"
                    style="padding:4px 12px;font-size:12px;cursor:pointer;border:none;background:rgba(245,158,11,0.18);color:#f59e0b;"
                    data-testid={`long-task-pause-btn-${task.id}`}
                  >
                    {t('longTask.pause')}
                  </button>
                )}
                {!['completed', 'cancelled'].includes(task.status) && (
                  <button
                    onClick={() => setConfirmAction({ id: task.id, kind: 'cancel' })}
                    class="tool-btn"
                    style="padding:4px 12px;font-size:12px;cursor:pointer;border:none;background:rgba(255,159,10,0.15);color:#ff9f0a;"
                    data-testid={`long-task-cancel-btn-${task.id}`}
                  >
                    {t('longTask.cancel')}
                  </button>
                )}
                <button
                  onClick={() => setConfirmAction({ id: task.id, kind: 'delete' })}
                  class="tool-btn"
                  style="padding:4px 12px;font-size:12px;cursor:pointer;border:none;background:rgba(255,95,87,0.15);color:#ff5f57;"
                  data-testid={`long-task-delete-btn-${task.id}`}
                >
                  {t('longTask.delete')}
                </button>
                <button
                  onClick={() => handleExpand(task.id)}
                  class="tool-btn"
                  style="padding:4px 12px;font-size:12px;cursor:pointer;border:none;"
                  data-testid={`long-task-expand-btn-${task.id}`}
                >
                  {expandedTask === task.id ? t('longTask.collapse') : t('longTask.steps')}
                </button>
              </div>

              {/* 步骤时间线(展开) */}
              {expandedTask === task.id && (
                <div
                  style="margin-top:12px;padding-left:16px;border-left:1px solid rgba(255,255,255,0.08);"
                  data-testid={`long-task-steps-view-${task.id}`}
                >
                  {stepsLoading && (
                    <div style="font-size:12px;color:rgba(255,255,255,0.3);">
                      {t('longTask.loadingSteps')}
                    </div>
                  )}
                  {!stepsLoading && stepsData.length === 0 && (
                    <div
                      style="font-size:12px;color:rgba(255,255,255,0.25);"
                      data-testid={`long-task-steps-empty-${task.id}`}
                    >
                      {t('longTask.noStepData')}
                    </div>
                  )}
                  {stepsData.map((step) => (
                    <div
                      key={step.seq}
                      style="margin-bottom:8px;cursor:pointer;"
                      onClick={() => setExpandedStep(expandedStep === step.seq ? null : step.seq)}
                      data-testid={`long-task-step-${task.id}-${step.seq}`}
                    >
                      <div style="display:flex;align-items:center;gap:8px;">
                        <span
                          style={`display:inline-block;width:6px;height:6px;border-radius:50%;background:${STEP_STATUS_COLORS[step.status]};`}
                        />
                        <span style="font-size:12px;color:rgba(255,255,255,0.3);">#{step.seq}</span>
                        <span
                          style={`font-size:12px;font-weight:500;color:${STEP_STATUS_COLORS[step.status]};`}
                        >
                          {t(STEP_STATUS_LABELS[step.status])}
                        </span>
                        <code style="font-size:12px;color:rgba(255,255,255,0.6);">
                          {step.program} {step.args.join(' ')}
                        </code>
                      </div>
                      {step.description && (
                        <div style="margin-left:16px;font-size:12px;color:rgba(255,255,255,0.4);">
                          {step.description}
                        </div>
                      )}
                      {expandedStep === step.seq && (
                        <div
                          style="margin-left:16px;margin-top:4px;padding:8px;background:rgba(0,0,0,0.25);border-radius:6px;font-size:12px;font-family:monospace;color:rgba(255,255,255,0.5);max-height:160px;overflow-y:auto;white-space:pre-wrap;"
                          data-testid={`long-task-step-detail-${task.id}-${step.seq}`}
                        >
                          {step.output && (
                            <div style="color:rgba(40,200,64,0.7);">{step.output}</div>
                          )}
                          {step.error && (
                            <div style="color:rgba(255,95,87,0.7);">⚠ {step.error}</div>
                          )}
                          {step.exit_code !== null && (
                            <div style="color:rgba(255,255,255,0.3);">exit: {step.exit_code}</div>
                          )}
                          {!step.output && !step.error && step.exit_code === null && (
                            <div style="color:rgba(255,255,255,0.3);">{t('longTask.noOutput')}</div>
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
      </div>

      {/* 确认对话框 */}
      {confirmAction && (
        <div
          style="position:fixed;inset:0;background:rgba(0,0,0,0.5);display:flex;align-items:center;justify-content:center;z-index:50;"
          data-testid="long-task-confirm-dialog"
        >
          <div class="stat-card" style="padding:16px;max-width:360px;">
            <div style="font-size:14px;color:rgba(255,255,255,0.9);margin-bottom:12px;">
              {confirmAction.kind === 'cancel'
                ? t('longTask.confirmCancel')
                : t('longTask.confirmDelete')}
            </div>
            <div style="display:flex;justify-content:flex-end;gap:8px;">
              <button
                onClick={() => setConfirmAction(null)}
                class="tool-btn"
                style="padding:6px 12px;font-size:12px;cursor:pointer;border:none;"
                data-testid="long-task-confirm-cancel"
              >
                {t('longTask.dialogCancel')}
              </button>
              <button
                onClick={handleConfirmAction}
                class="tool-btn tool-btn-primary"
                style="padding:6px 12px;font-size:12px;cursor:pointer;border:none;background:#ff5f57;"
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
