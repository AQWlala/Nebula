/**
 * T-E-C-08 / T-E-C-09: Shadow Workspace 面板。
 *
 * 管理 Agent 隔离执行环境:
 * - 创建新 workspace(任务描述 + 可选 base 分支)
 * - 列出所有 workspace(状态/分支/时间戳)
 * - 查看 diff(与 base_branch 对比)
 * - ▶ 回放操作录屏(T-E-C-09):查看 Agent 执行的每步操作时间线
 * - 合并(merge 回 base)或丢弃(abort)
 *
 * 设计要点:
 * - 状态色标:running 蓝/completed 绿/failed 红/merged 灰/aborted 黄
 * - diff / 录屏均在内联展开(不弹窗,长内容用 pre-wrap + max-height 滚动)
 * - 合并/丢弃需二次确认(不可逆操作)
 * - 录屏在合并/丢弃后仍可查看(引擎保留录屏供事后审查)
 */
import { useState, useEffect, useCallback } from 'preact/hooks';
import {
  nebulaAPI,
  type ShadowWorkspace,
  type ShadowStatus,
  type OperationKind,
  type OperationRecord,
} from '../lib/tauri';
import { toast } from './Toast';
import { t, type Dict } from '../i18n';

const STATUS_COLORS: Record<ShadowStatus, string> = {
  creating: '#9CA3AF',
  running: '#3B82F6',
  completed: '#10B981',
  failed: '#EF4444',
  merged: '#6B7280',
  aborted: '#F59E0B',
};

const STATUS_LABELS: Record<ShadowStatus, keyof Dict> = {
  creating: 'shadowWorkspace.status.creating',
  running: 'shadowWorkspace.status.running',
  completed: 'shadowWorkspace.status.completed',
  failed: 'shadowWorkspace.status.failed',
  merged: 'shadowWorkspace.status.merged',
  aborted: 'shadowWorkspace.status.aborted',
};

// T-E-C-09: 操作种类图标 + 标签(供录屏时间线渲染)
const OP_ICONS: Record<OperationKind, string> = {
  file_create: '📄✚',
  file_write: '✏️',
  file_delete: '🗑️',
  command: '⌘',
  note: '📝',
};

const OP_LABELS: Record<OperationKind, keyof Dict> = {
  file_create: 'shadowWorkspace.op.file_create',
  file_write: 'shadowWorkspace.op.file_write',
  file_delete: 'shadowWorkspace.op.file_delete',
  command: 'shadowWorkspace.op.command',
  note: 'shadowWorkspace.op.note',
};

function formatTime(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** 录屏时间戳格式化(毫秒 → 时:分:秒.毫秒)。 */
function formatRecTime(tsMs: number): string {
  return new Date(tsMs).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

export function ShadowWorkspacePanel() {
  const [workspaces, setWorkspaces] = useState<ShadowWorkspace[]>([]);
  const [loading, setLoading] = useState(false);
  const [taskDesc, setTaskDesc] = useState('');
  const [baseBranch, setBaseBranch] = useState('');
  const [creating, setCreating] = useState(false);
  const [diffFor, setDiffFor] = useState<string | null>(null);
  const [diffText, setDiffText] = useState('');
  const [diffLoading, setDiffLoading] = useState(false);
  // T-E-C-09: 录屏回放状态
  const [recFor, setRecFor] = useState<string | null>(null);
  const [recOps, setRecOps] = useState<OperationRecord[]>([]);
  const [recLoading, setRecLoading] = useState(false);
  const [recExpanded, setRecExpanded] = useState<number | null>(null);
  const [confirmAction, setConfirmAction] = useState<{
    id: string;
    kind: 'merge' | 'abort';
  } | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await nebulaAPI.shadowList();
      setWorkspaces(list);
    } catch (e) {
      console.error('shadow list failed:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleCreate = async () => {
    if (!taskDesc.trim()) {
      toast.warning(t('shadowWorkspace.toast.descRequired'));
      return;
    }
    setCreating(true);
    try {
      const ws = await nebulaAPI.shadowCreate(taskDesc.trim(), baseBranch.trim() || null);
      toast.success(t('shadowWorkspace.toast.created', { id: ws.id }));
      setTaskDesc('');
      setBaseBranch('');
      await refresh();
    } catch (e) {
      toast.error(t('shadowWorkspace.toast.createFailed'), String(e));
    } finally {
      setCreating(false);
    }
  };

  const handleViewDiff = async (id: string) => {
    if (diffFor === id) {
      // 切换关闭
      setDiffFor(null);
      setDiffText('');
      return;
    }
    setDiffFor(id);
    setDiffLoading(true);
    setDiffText('');
    try {
      const diff = await nebulaAPI.shadowDiff(id);
      setDiffText(diff || t('shadowWorkspace.noDiff'));
    } catch (e) {
      setDiffText(t('shadowWorkspace.diffFailed', { error: String(e) }));
    } finally {
      setDiffLoading(false);
    }
  };

  // T-E-C-09: 加载/切换录屏时间线
  const handleViewRecording = async (id: string) => {
    if (recFor === id) {
      // 切换关闭
      setRecFor(null);
      setRecOps([]);
      setRecExpanded(null);
      return;
    }
    setRecFor(id);
    setRecLoading(true);
    setRecOps([]);
    setRecExpanded(null);
    try {
      const ops = await nebulaAPI.shadowRecordingList(id);
      setRecOps(ops);
    } catch (e) {
      console.error('shadow recording list failed:', e);
      setRecOps([]);
    } finally {
      setRecLoading(false);
    }
  };

  const handleComplete = async (id: string) => {
    try {
      await nebulaAPI.shadowComplete(id);
      toast.success(t('shadowWorkspace.toast.markedComplete'));
      await refresh();
    } catch (e) {
      toast.error(t('shadowWorkspace.toast.opFailed'), String(e));
    }
  };

  const handleConfirmAction = async () => {
    if (!confirmAction) return;
    const { id, kind } = confirmAction;
    try {
      if (kind === 'merge') {
        await nebulaAPI.shadowMerge(id);
        toast.success(t('shadowWorkspace.toast.merged'));
      } else {
        await nebulaAPI.shadowAbort(id);
        toast.success(t('shadowWorkspace.toast.aborted'));
      }
      await refresh();
    } catch (e) {
      toast.error(
        kind === 'merge'
          ? t('shadowWorkspace.toast.mergeFailed')
          : t('shadowWorkspace.toast.abortFailed'),
        String(e)
      );
    } finally {
      setConfirmAction(null);
    }
  };

  return (
    <div
      className="shadow-workspace-panel h-full flex flex-col bg-gray-950 text-white"
      data-testid="shadow-workspace-panel"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800">
        <h2 className="text-sm font-semibold text-gray-300">{t('shadowWorkspace.title')}</h2>
        <div className="flex items-center gap-3">
          {loading && <span className="text-xs text-gray-500">{t('shadowWorkspace.loading')}</span>}
          <span className="text-xs text-gray-500">
            {t('shadowWorkspace.wsCount', { n: workspaces.length })}
          </span>
          <button
            onClick={refresh}
            className="text-xs text-gray-400 hover:text-white transition-colors"
            title={t('shadowWorkspace.refresh')}
          >
            ↻
          </button>
        </div>
      </div>

      {/* 创建表单 */}
      <div className="px-4 py-2 border-b border-gray-800 space-y-2">
        <div className="flex gap-2">
          <input
            type="text"
            placeholder={t('shadowWorkspace.descPlaceholder')}
            value={taskDesc}
            onInput={(e) => setTaskDesc((e.target as HTMLInputElement).value)}
            className="flex-1 px-2 py-1 text-sm bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
            data-testid="shadow-task-input"
          />
          <input
            type="text"
            placeholder={t('shadowWorkspace.baseBranchPlaceholder')}
            value={baseBranch}
            onInput={(e) => setBaseBranch((e.target as HTMLInputElement).value)}
            className="w-40 px-2 py-1 text-sm bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
            data-testid="shadow-base-input"
          />
          <button
            onClick={handleCreate}
            disabled={creating || !taskDesc.trim()}
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed rounded text-white transition-colors"
            data-testid="shadow-create-btn"
          >
            {creating ? t('shadowWorkspace.creating') : t('shadowWorkspace.create')}
          </button>
        </div>
        <p className="text-xs text-gray-600">{t('shadowWorkspace.hint')}</p>
      </div>

      {/* 列表 */}
      <div className="flex-1 overflow-y-auto">
        {workspaces.length === 0 && !loading && (
          <div className="text-center text-gray-500 py-12" data-testid="shadow-empty">
            {t('shadowWorkspace.empty')}
          </div>
        )}
        {workspaces.map((ws) => {
          const isExpanded = diffFor === ws.id;
          const isRecExpanded = recFor === ws.id;
          const canMerge = ws.status === 'completed' || ws.status === 'running';
          const canAbort =
            ws.status === 'running' || ws.status === 'completed' || ws.status === 'failed';
          const canComplete = ws.status === 'running';
          return (
            <div
              key={ws.id}
              data-testid={`shadow-item-${ws.id}`}
              className="border-b border-gray-800 px-4 py-2"
            >
              {/* 行头 */}
              <div className="flex items-center gap-2 mb-1">
                <span
                  className="w-2 h-2 rounded-full flex-shrink-0"
                  style={{ backgroundColor: STATUS_COLORS[ws.status] }}
                />
                <span className="text-xs text-gray-500 font-mono">{ws.id}</span>
                <span
                  className="text-xs px-1.5 py-0.5 rounded"
                  style={{
                    backgroundColor: `${STATUS_COLORS[ws.status]}33`,
                    color: STATUS_COLORS[ws.status],
                  }}
                >
                  {t(STATUS_LABELS[ws.status])}
                </span>
                <span className="text-xs text-gray-600">·</span>
                <span className="text-xs text-gray-500 font-mono">{ws.branch}</span>
                {ws.error && (
                  <span className="text-xs text-red-400 truncate" title={ws.error}>
                    ⚠ {ws.error}
                  </span>
                )}
              </div>
              {/* 任务描述 */}
              <div className="text-sm text-gray-200 mb-1">{ws.task_description}</div>
              {/* 元数据 */}
              <div className="flex flex-wrap gap-3 text-xs text-gray-500 mb-1">
                <span>base: {ws.base_branch}</span>
                <span>{t('shadowWorkspace.createdAt', { time: formatTime(ws.created_at) })}</span>
                {ws.finished_at && (
                  <span>{t('shadowWorkspace.finishedAt', { time: formatTime(ws.finished_at) })}</span>
                )}
              </div>
              {/* 操作按钮 */}
              <div className="flex flex-wrap gap-2 mt-1">
                <button
                  onClick={() => handleViewDiff(ws.id)}
                  className="text-xs px-2 py-0.5 bg-gray-800 hover:bg-gray-700 rounded text-gray-300 transition-colors"
                  data-testid={`shadow-diff-btn-${ws.id}`}
                >
                  {isExpanded ? t('shadowWorkspace.hideDiff') : t('shadowWorkspace.viewDiff')}
                </button>
                <button
                  onClick={() => handleViewRecording(ws.id)}
                  className="text-xs px-2 py-0.5 bg-gray-800 hover:bg-gray-700 rounded text-gray-300 transition-colors"
                  data-testid={`shadow-replay-btn-${ws.id}`}
                  title={t('shadowWorkspace.replayTitle')}
                >
                  {isRecExpanded ? t('shadowWorkspace.hideReplay') : t('shadowWorkspace.replay')}
                </button>
                {canComplete && (
                  <button
                    onClick={() => handleComplete(ws.id)}
                    className="text-xs px-2 py-0.5 bg-green-900/60 hover:bg-green-800/60 rounded text-green-300 transition-colors"
                  >
                    {t('shadowWorkspace.markComplete')}
                  </button>
                )}
                {canMerge && (
                  <button
                    onClick={() => setConfirmAction({ id: ws.id, kind: 'merge' })}
                    className="text-xs px-2 py-0.5 bg-blue-900/60 hover:bg-blue-800/60 rounded text-blue-300 transition-colors"
                  >
                    {t('shadowWorkspace.merge')}
                  </button>
                )}
                {canAbort && (
                  <button
                    onClick={() => setConfirmAction({ id: ws.id, kind: 'abort' })}
                    className="text-xs px-2 py-0.5 bg-red-900/60 hover:bg-red-800/60 rounded text-red-300 transition-colors"
                  >
                    {t('shadowWorkspace.abort')}
                  </button>
                )}
              </div>
              {/* diff 展开 */}
              {isExpanded && (
                <div className="mt-2" data-testid={`shadow-diff-view-${ws.id}`}>
                  {diffLoading ? (
                    <div className="text-xs text-gray-500">{t('shadowWorkspace.loadingDiff')}</div>
                  ) : (
                    <pre className="text-xs text-gray-300 bg-gray-900 border border-gray-800 rounded p-2 overflow-auto max-h-80 whitespace-pre-wrap">
                      {diffText}
                    </pre>
                  )}
                </div>
              )}
              {/* T-E-C-09: 录屏回放时间线 */}
              {isRecExpanded && (
                <div className="mt-2" data-testid={`shadow-recording-view-${ws.id}`}>
                  {recLoading ? (
                    <div className="text-xs text-gray-500">{t('shadowWorkspace.loadingRecording')}</div>
                  ) : recOps.length === 0 ? (
                    <div
                      className="text-xs text-gray-600"
                      data-testid={`shadow-recording-empty-${ws.id}`}
                    >
                      {t('shadowWorkspace.recordingEmpty')}
                    </div>
                  ) : (
                    <div className="border border-gray-800 rounded">
                      <div className="text-xs text-gray-500 px-2 py-1 border-b border-gray-800 bg-gray-900/50">
                        {t('shadowWorkspace.recordingHeader', { n: recOps.length })}
                      </div>
                      <ul className="divide-y divide-gray-800 max-h-96 overflow-y-auto">
                        {recOps.map((op) => (
                          <li
                            key={op.seq}
                            className="px-2 py-1.5 hover:bg-gray-900/50 cursor-pointer"
                            data-testid={`shadow-recording-op-${ws.id}-${op.seq}`}
                            onClick={() => setRecExpanded(recExpanded === op.seq ? null : op.seq)}
                          >
                            <div className="flex items-center gap-2">
                              <span className="text-xs text-gray-600 font-mono w-6 flex-shrink-0">
                                #{op.seq}
                              </span>
                              <span className="text-sm flex-shrink-0">{OP_ICONS[op.kind]}</span>
                              <span className="text-xs text-gray-400 flex-shrink-0">
                                {t(OP_LABELS[op.kind])}
                              </span>
                              {op.target && (
                                <span className="text-xs text-gray-300 font-mono truncate flex-1">
                                  {op.target}
                                </span>
                              )}
                              <span
                                className={`text-xs flex-shrink-0 ${op.success ? 'text-green-400' : 'text-red-400'}`}
                              >
                                {op.success ? '✓' : '✗'}
                              </span>
                              <span className="text-xs text-gray-600 flex-shrink-0">
                                {formatRecTime(op.ts_ms)}
                              </span>
                            </div>
                            {op.detail && recExpanded !== op.seq && (
                              <div className="text-xs text-gray-600 mt-0.5 ml-8 truncate">
                                {op.detail}
                              </div>
                            )}
                            {recExpanded === op.seq && (
                              <div
                                className="mt-1 ml-8 space-y-1"
                                data-testid={`shadow-recording-detail-${ws.id}-${op.seq}`}
                              >
                                {op.detail && (
                                  <div>
                                    <span className="text-xs text-gray-500">{t('shadowWorkspace.detailLabel')}</span>
                                    <code className="text-xs text-gray-300">{op.detail}</code>
                                  </div>
                                )}
                                {op.message && (
                                  <div>
                                    <span className="text-xs text-gray-500">{t('shadowWorkspace.messageLabel')}</span>
                                    <pre className="text-xs text-gray-400 bg-gray-900 border border-gray-800 rounded p-1 mt-0.5 overflow-auto max-h-40 whitespace-pre-wrap inline-block w-full">
                                      {op.message}
                                    </pre>
                                  </div>
                                )}
                              </div>
                            )}
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {/* 二次确认对话框 */}
      {confirmAction && (
        <div
          className="fixed inset-0 bg-black/60 flex items-center justify-center z-50"
          data-testid="shadow-confirm-dialog"
        >
          <div className="bg-gray-900 border border-gray-700 rounded-lg p-4 max-w-sm">
            <h3 className="text-sm font-semibold text-white mb-2">
              {confirmAction.kind === 'merge'
                ? t('shadowWorkspace.confirmMergeTitle')
                : t('shadowWorkspace.confirmAbortTitle')}
            </h3>
            <p className="text-xs text-gray-400 mb-4">
              {confirmAction.kind === 'merge'
                ? t('shadowWorkspace.confirmMergeBody')
                : t('shadowWorkspace.confirmAbortBody')}
            </p>
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setConfirmAction(null)}
                className="text-xs px-3 py-1 bg-gray-800 hover:bg-gray-700 rounded text-gray-300"
              >
                {t('shadowWorkspace.dialogCancel')}
              </button>
              <button
                onClick={handleConfirmAction}
                className={`text-xs px-3 py-1 rounded text-white ${confirmAction.kind === 'merge' ? 'bg-blue-600 hover:bg-blue-700' : 'bg-red-600 hover:bg-red-700'}`}
                data-testid="shadow-confirm-btn"
              >
                {confirmAction.kind === 'merge'
                  ? t('shadowWorkspace.merge')
                  : t('shadowWorkspace.abort')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
