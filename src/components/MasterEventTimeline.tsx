/**
 * M6 #82: Master 编排事件时间线 + L4 审批交互组件。
 *
 * ## 功能
 * - 启动 MasterOrchestrator 编排(输入框 + 模式选择 standard/bypass/plan)
 * - 实时展示 11 个 MasterEvent 变体形成时间线
 * - 接收 `user_confirmation_required` 事件时弹审批 modal:
 *   - 显示 prompt + diff(若有)
 *   - "确认"/"拒绝" 按钮调用 `masterConfirm(confirmation_id)`
 *   - 5 分钟超时倒计时(前端按 `created_at` + CONFIRMATION_TIMEOUT_MS 计算)
 * - 流结束后展示 MasterReport(综合输出 + 子任务统计)
 *
 * ## 集成位置
 * 嵌入 SwarmView 顶部 tab(与 AgentColumn / EventStreamViewer 并列)。
 *
 * ## 容错
 * - 后端未启用 master-orchestrator feature 时,masterRun invoke 会 reject,
 *   显示 "Master 编排功能未启用" 提示。
 */
import { useState, useEffect, useRef, useMemo } from 'preact/hooks';
import {
  nebulaAPI,
  type MasterEvent,
  type MasterReport,
  type ExecuteMode,
  type PendingConfirmation,
  type ConfirmationStatus,
  CONFIRMATION_TIMEOUT_MS,
} from '../lib/tauri';
import { t } from '../i18n';
import { toast } from './Toast';
import { DagCanvas } from './DagCanvas';

/** 时间线单条记录(便于 undo/重放)。 */
interface TimelineEntry {
  id: number;
  event: MasterEvent;
}

/** 当前 pending 的审批请求(由 user_confirmation_required 事件触发)。 */
interface PendingApproval {
  confirmation_id: string;
  task_id: string;
  sub_task_id: string;
  prompt: string;
  created_at: number;
  /** 状态:pending / confirming / confirmed / expired / error */
  status: 'pending' | 'confirming' | 'confirmed' | 'expired' | 'error';
  status_message?: string;
}

/** 把 MasterEvent.kind 转为中文标签 + emoji 图标。 */
function eventLabel(kind: MasterEvent['kind']): { icon: string; label: string; tone: 'info' | 'success' | 'warning' | 'error' } {
  switch (kind) {
    case 'decompose_started': return { icon: '🧩', label: t('masterTimeline.event.decomposeStarted'), tone: 'info' };
    case 'decompose_completed': return { icon: '✅', label: t('masterTimeline.event.decomposeCompleted'), tone: 'success' };
    case 'decompose_failed': return { icon: '❌', label: t('masterTimeline.event.decomposeFailed'), tone: 'error' };
    case 'layer_started': return { icon: '▶️', label: t('masterTimeline.event.layerStarted'), tone: 'info' };
    case 'layer_completed': return { icon: '✅', label: t('masterTimeline.event.layerCompleted'), tone: 'success' };
    case 'sub_task_started': return { icon: '🚀', label: t('masterTimeline.event.subTaskStarted'), tone: 'info' };
    case 'sub_task_completed': return { icon: '✅', label: t('masterTimeline.event.subTaskCompleted'), tone: 'success' };
    case 'synthesize_started': return { icon: '🧪', label: t('masterTimeline.event.synthesizeStarted'), tone: 'info' };
    case 'synthesize_completed': return { icon: '✅', label: t('masterTimeline.event.synthesizeCompleted'), tone: 'success' };
    case 'dag_failed': return { icon: '❌', label: t('masterTimeline.event.dagFailed'), tone: 'error' };
    case 'user_confirmation_required': return { icon: '⚠️', label: t('masterTimeline.event.userConfirmationRequired'), tone: 'warning' };
    case 'master_completed': return { icon: '🎉', label: t('masterTimeline.event.masterCompleted'), tone: 'success' };
  }
}

/** 格式化时间戳为 HH:MM:SS.mmm。 */
function formatTime(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number, l = 2) => n.toString().padStart(l, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

/** 计算剩余时间(秒),<0 返回 0。 */
function remainingSeconds(createdAt: number, now: number): number {
  const elapsed = now - createdAt;
  const remaining = CONFIRMATION_TIMEOUT_MS - elapsed;
  return Math.max(0, Math.floor(remaining / 1000));
}

export function MasterEventTimeline() {
  const [input, setInput] = useState('');
  const [mode, setMode] = useState<ExecuteMode>('standard');
  const [running, setRunning] = useState(false);
  const [timeline, setTimeline] = useState<TimelineEntry[]>([]);
  const [report, setReport] = useState<MasterReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pendingApprovals, setPendingApprovals] = useState<PendingApproval[]>([]);
  // 用于触发倒计时刷新(每秒更新一次 pendingApprovals 的剩余时间显示)
  const [nowTick, setNowTick] = useState(Date.now());
  // AbortController ref
  const controllerRef = useRef<AbortController | null>(null);
  // event id 自增计数器
  const eventIdRef = useRef(0);
  // M6 #79: 视图切换 — 时间线 / DAG 画布
  const [viewMode, setViewMode] = useState<'timeline' | 'dag'>('timeline');
  // M6 #79: 被点击展开的 sub_task_id(用于高亮)
  const [selectedSubTask, setSelectedSubTask] = useState<string | null>(null);

  // 倒计时 tick:每秒更新一次,刷新 pendingApprovals 的剩余时间显示。
  // 仅在有 pending 审批时启动 interval,避免空转。
  useEffect(() => {
    if (pendingApprovals.length === 0) return;
    const interval = setInterval(() => setNowTick(Date.now()), 1000);
    return () => clearInterval(interval);
  }, [pendingApprovals.length]);

  // 检查 pendingApprovals 是否有过期的(剩余 0 秒)。
  useEffect(() => {
    setPendingApprovals((prev) =>
      prev.map((p) => {
        if (p.status !== 'pending') return p;
        if (remainingSeconds(p.created_at, nowTick) === 0) {
          return { ...p, status: 'expired', status_message: t('masterTimeline.approval.expired') };
        }
        return p;
      }),
    );
  }, [nowTick]);

  // 卸载时取消正在进行的 master_run
  useEffect(() => {
    return () => {
      controllerRef.current?.abort();
    };
  }, []);

  /** 处理 MasterEvent:追加到时间线 + 触发审批 modal。 */
  function handleEvent(event: MasterEvent) {
    setTimeline((prev) => [...prev, { id: eventIdRef.current++, event }]);
    if (event.kind === 'user_confirmation_required') {
      setPendingApprovals((prev) => [
        ...prev,
        {
          confirmation_id: event.confirmation_id,
          task_id: event.task_id,
          sub_task_id: event.sub_task_id,
          prompt: event.prompt,
          created_at: event.created_at,
          status: 'pending',
        },
      ]);
    }
  }

  /** 启动 master_run。 */
  async function startMaster() {
    if (!input.trim() || running) return;
    setRunning(true);
    setTimeline([]);
    setReport(null);
    setError(null);
    setPendingApprovals([]);

    const controller = new AbortController();
    controllerRef.current = controller;

    try {
      const result = await nebulaAPI.masterRun(
        { input: input.trim(), mode },
        handleEvent,
        controller.signal,
      );
      if (!controller.signal.aborted) {
        setReport(result);
        toast.success(t('masterTimeline.toast.completed.title'), t('masterTimeline.toast.completed.body', { taskId: result.task_id, success: result.successful_sub_tasks, total: result.total_sub_tasks }));
      }
    } catch (e) {
      if (!controller.signal.aborted) {
        const msg = String(e);
        setError(msg);
        // 检测 feature 未启用的错误
        if (msg.includes('not found') || msg.includes('command')) {
          toast.error(t('masterTimeline.toast.unavailable.title'), t('masterTimeline.toast.unavailable.body'));
        } else {
          toast.error(t('masterTimeline.toast.failed.title'), msg);
        }
      }
    } finally {
      setRunning(false);
      controllerRef.current = null;
    }
  }

  /** 中止 master_run。 */
  function stopMaster() {
    controllerRef.current?.abort();
    setRunning(false);
  }

  /** 用户确认审批请求。 */
  async function confirmApproval(confirmationId: string) {
    setPendingApprovals((prev) =>
      prev.map((p) =>
        p.confirmation_id === confirmationId
          ? { ...p, status: 'confirming' }
          : p,
      ),
    );
    try {
      const status: ConfirmationStatus = await nebulaAPI.masterConfirm(confirmationId);
      setPendingApprovals((prev) =>
        prev.map((p) => {
          if (p.confirmation_id !== confirmationId) return p;
          if (status === 'confirmed') {
            return { ...p, status: 'confirmed', status_message: t('masterTimeline.confirmStatus.confirmed') };
          }
          const label = {
            already_used: t('masterTimeline.confirmStatus.alreadyUsed'),
            expired: t('masterTimeline.confirmStatus.expired'),
            not_found: t('masterTimeline.confirmStatus.notFound'),
            confirmed: t('masterTimeline.confirmStatus.confirmed'),
          }[status];
          return { ...p, status: 'error', status_message: label };
        }),
      );
      if (status === 'confirmed') {
        toast.success(t('masterTimeline.toast.approvalConfirmed.title'), confirmationId.slice(0, 8));
      } else {
        toast.warning(t('masterTimeline.toast.approvalAbnormal.title'), t('masterTimeline.toast.approvalAbnormal.body', { status, id: confirmationId.slice(0, 8) }));
      }
    } catch (e) {
      setPendingApprovals((prev) =>
        prev.map((p) =>
          p.confirmation_id === confirmationId
            ? { ...p, status: 'error', status_message: String(e) }
            : p,
        ),
      );
      toast.error(t('masterTimeline.toast.approvalConfirmFailed.title'), String(e));
    }
  }

  /** 用户拒绝审批请求(目前后端无 reject API,前端仅标记本地状态)。 */
  function rejectApproval(confirmationId: string) {
    setPendingApprovals((prev) =>
      prev.map((p) =>
        p.confirmation_id === confirmationId
          ? { ...p, status: 'error', status_message: t('masterTimeline.approval.rejected') }
          : p,
      ),
    );
    toast.info(t('masterTimeline.toast.approvalRejected.title'), t('masterTimeline.toast.approvalRejected.body', { id: confirmationId.slice(0, 8) }));
  }

  return (
    <div class="master-timeline">
      {/* 输入区 */}
      <div class="master-input-bar">
        <input
          type="text"
          value={input}
          onInput={(e) => setInput((e.target as HTMLInputElement).value)}
          placeholder={t('masterTimeline.input.placeholder')}
          disabled={running}
          class="master-input"
        />
        <select
          value={mode}
          onChange={(e) => setMode((e.target as HTMLSelectElement).value as ExecuteMode)}
          disabled={running}
          class="master-mode-select"
          title={t('masterTimeline.mode.title')}
        >
          <option value="standard">Standard</option>
          <option value="bypass">Bypass</option>
          <option value="plan">Plan</option>
        </select>
        {running ? (
          <button class="btn btn-stop" onClick={stopMaster}>{t('masterTimeline.button.stop')}</button>
        ) : (
          <button class="btn btn-neon" onClick={startMaster} disabled={!input.trim()}>{t('masterTimeline.button.start')}</button>
        )}
      </div>

      {/* 错误提示 */}
      {error && (
        <div class="error master-error">
          {error}
        </div>
      )}

      {/* M6 #79: 视图切换 — 时间线 / DAG 画布(仅在有事件时显示) */}
      {timeline.length > 0 && (
        <div class="dag-view-toggle">
          <button
            type="button"
            class={`dag-toggle-btn ${viewMode === 'timeline' ? 'active' : ''}`}
            onClick={() => setViewMode('timeline')}
          >
            {t('masterTimeline.view.timeline')}
          </button>
          <button
            type="button"
            class={`dag-toggle-btn ${viewMode === 'dag' ? 'active' : ''}`}
            onClick={() => setViewMode('dag')}
          >
            {t('masterTimeline.view.dag')}
          </button>
          {selectedSubTask && (
            <span
              class="dag-selected-badge"
              title={selectedSubTask}
              onClick={() => setSelectedSubTask(null)}
              style={{ cursor: 'pointer' }}
            >
              {t('masterTimeline.selectedBadge', { subTaskId: selectedSubTask.slice(0, 16) })}
            </span>
          )}
        </div>
      )}

      {/* M6 #79: DAG 画布视图 */}
      {viewMode === 'dag' && timeline.length > 0 && (
        <DagCanvas
          events={timeline.map((e) => e.event)}
          canvasWidth={860}
          onNodeClick={(subTaskId) => {
            setSelectedSubTask(subTaskId);
            toast.info(t('masterTimeline.toast.nodeSelected.title'), subTaskId);
          }}
        />
      )}

      {/* pending 审批 modals */}
      {pendingApprovals.map((p) => {
        if (p.status === 'confirmed') return null;
        const remaining = p.status === 'pending' ? remainingSeconds(p.created_at, nowTick) : 0;
        return (
          <div key={p.confirmation_id} class="modal-backdrop master-approval-modal">
            <div class="modal">
              <div class="modal__header">
                <h3>{t('masterTimeline.approvalModal.title')}</h3>
                {p.status === 'pending' && (
                  <span class={`approval-countdown ${remaining < 60 ? 'urgent' : ''}`}>
                    {t('masterTimeline.approvalModal.countdown', { minutes: Math.floor(remaining / 60), seconds: (remaining % 60).toString().padStart(2, '0') })}
                  </span>
                )}
              </div>
              <div class="modal__body">
                <div class="approval-meta">
                  <span class="badge">{t('masterTimeline.approvalModal.taskBadge', { taskId: p.task_id.slice(0, 12) })}</span>
                  <span class="badge">{t('masterTimeline.approvalModal.subTaskBadge', { subTaskId: p.sub_task_id.slice(0, 12) })}</span>
                  <span class="badge">ID {p.confirmation_id.slice(0, 8)}</span>
                </div>
                <pre class="approval-prompt">{p.prompt}</pre>
              </div>
              <div class="modal__actions">
                {p.status === 'pending' && (
                  <>
                    <button
                      class="btn btn-neon"
                      onClick={() => confirmApproval(p.confirmation_id)}
                    >
                      {t('masterTimeline.approvalModal.confirm')}
                    </button>
                    <button
                      class="btn"
                      onClick={() => rejectApproval(p.confirmation_id)}
                    >
                      {t('masterTimeline.approvalModal.reject')}
                    </button>
                  </>
                )}
                {p.status === 'confirming' && <span>{t('masterTimeline.approvalModal.confirming')}</span>}
                {p.status === 'expired' && (
                  <span class="approval-status approval-status-error">
                    {p.status_message}
                  </span>
                )}
                {p.status === 'error' && (
                  <span class="approval-status approval-status-error">
                    {p.status_message}
                  </span>
                )}
              </div>
            </div>
          </div>
        );
      })}

      {/* 时间线(仅在时间线视图显示) */}
      {viewMode === 'timeline' && timeline.length > 0 && (
        <div class="master-timeline-list">
          <h4>{t('masterTimeline.timelineHeader', { count: timeline.length })}</h4>
          <ul>
            {timeline.map((entry) => {
              const { icon, label, tone } = eventLabel(entry.event.kind);
              const isSelected =
                selectedSubTask !== null &&
                ('sub_task_id' in entry.event) &&
                entry.event.sub_task_id === selectedSubTask;
              return (
                <li
                  key={entry.id}
                  class={`timeline-entry timeline-${tone} ${isSelected ? 'timeline-selected' : ''}`}
                >
                  <span class="timeline-time">{formatTime(entry.event.timestamp)}</span>
                  <span class="timeline-icon">{icon}</span>
                  <span class="timeline-label">{label}</span>
                  <span class="timeline-detail">
                    {eventDetail(entry.event)}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {/* 最终报告 */}
      {report && (
        <div class="card master-report">
          <h4>{t('masterTimeline.report.title')}</h4>
          <div class="master-report-meta">
            <span class="badge">{t('masterTimeline.report.taskId', { taskId: report.task_id })}</span>
            <span class="badge">{t('masterTimeline.report.elapsed', { seconds: (report.elapsed_ms / 1000).toFixed(2) })}</span>
            <span class="badge">{t('masterTimeline.report.subTasks', { success: report.successful_sub_tasks, total: report.total_sub_tasks })}</span>
            {report.bypassed && <span class="badge badge-l4">{t('masterTimeline.report.bypassed')}</span>}
          </div>
          <pre class="master-output">{report.output}</pre>
        </div>
      )}
    </div>
  );
}

/** 从 MasterEvent 提取简短详情用于时间线展示。 */
function eventDetail(event: MasterEvent): string {
  switch (event.kind) {
    case 'decompose_started':
      return t('masterTimeline.detail.decomposeStarted', { summary: event.input_summary.slice(0, 80) + (event.input_summary.length > 80 ? '...' : '') });
    case 'decompose_completed':
      return t('masterTimeline.detail.decomposeCompleted', { nodes: event.node_count, edges: event.edge_count });
    case 'decompose_failed':
      return event.error.slice(0, 120);
    case 'layer_started':
      return t('masterTimeline.detail.layerStarted', { index: event.layer_index, nodes: event.node_count });
    case 'layer_completed':
      return t('masterTimeline.detail.layerCompleted', { index: event.layer_index, success: event.success_count, failure: event.failure_count });
    case 'sub_task_started':
      return `${event.sub_task_id.slice(0, 12)}, ${event.worker_count} workers`;
    case 'sub_task_completed':
      return `${event.sub_task_id.slice(0, 12)} ${event.success ? '✓' : '✗'}${event.error ? ` ${event.error.slice(0, 80)}` : ''} (${event.elapsed_ms}ms)`;
    case 'synthesize_started':
      return t('masterTimeline.detail.synthesizeStarted', { count: event.result_count });
    case 'synthesize_completed':
      return t('masterTimeline.detail.synthesizeCompleted', { count: event.output_length });
    case 'dag_failed':
      return `${event.failed_sub_task_id.slice(0, 12)}: ${event.reason.slice(0, 100)}`;
    case 'user_confirmation_required':
      return `${event.confirmation_id.slice(0, 8)} — ${event.prompt.slice(0, 80)}`;
    case 'master_completed':
      return t('masterTimeline.detail.masterCompleted', { success: event.successful_sub_tasks, total: event.total_sub_tasks, ms: event.elapsed_ms });
  }
}
