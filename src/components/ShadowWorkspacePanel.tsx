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

const STATUS_LABELS: Record<ShadowStatus, keyof Dict> = {
  creating: 'shadowWorkspace.status.creating',
  running: 'shadowWorkspace.status.running',
  completed: 'shadowWorkspace.status.completed',
  failed: 'shadowWorkspace.status.failed',
  merged: 'shadowWorkspace.status.merged',
  aborted: 'shadowWorkspace.status.aborted',
};

/** ShadowStatus → task-status CSS 类名映射(running/done/queued/failed)。 */
const STATUS_TASK_CLASS: Record<ShadowStatus, string> = {
  creating: 'queued',
  running: 'running',
  completed: 'done',
  failed: 'failed',
  merged: 'done',
  aborted: 'queued',
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
      class="shadow-workspace-panel"
      style="display:flex;flex-direction:column;height:100%;"
      data-testid="shadow-workspace-panel"
    >
      {/* 页面头:标题 + 工具按钮 */}
      <div class="page-header">
        <div>
          <div class="page-title">🌑 {t('shadowWorkspace.title')}</div>
          <div class="page-subtitle">
            {loading && <span>{t('shadowWorkspace.loading')} · </span>}
            {t('shadowWorkspace.wsCount', { n: workspaces.length })}
          </div>
        </div>
        <div class="page-actions">
          <button class="tool-btn" onClick={refresh} title={t('shadowWorkspace.refresh')}>
            ↻ {t('shadowWorkspace.refresh')}
          </button>
        </div>
      </div>

      <div class="page-body" style="display:flex;flex-direction:column;gap:12px;">
        {/* 创建表单 */}
        <div class="stat-card" style="padding:14px 16px;">
          <div style="display:flex;gap:8px;margin-bottom:8px;">
            <input
              type="text"
              placeholder={t('shadowWorkspace.descPlaceholder')}
              value={taskDesc}
              onInput={(e) => setTaskDesc((e.target as HTMLInputElement).value)}
              style="flex:1;padding:6px 10px;font-size:13px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:7px;color:inherit;outline:none;"
              data-testid="shadow-task-input"
            />
            <input
              type="text"
              placeholder={t('shadowWorkspace.baseBranchPlaceholder')}
              value={baseBranch}
              onInput={(e) => setBaseBranch((e.target as HTMLInputElement).value)}
              style="width:160px;padding:6px 10px;font-size:13px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);border-radius:7px;color:inherit;outline:none;"
              data-testid="shadow-base-input"
            />
            <button
              onClick={handleCreate}
              disabled={creating || !taskDesc.trim()}
              class="tool-btn tool-btn-primary"
              style={{ cursor: 'pointer', opacity: creating || !taskDesc.trim() ? 0.4 : 1 }}
              data-testid="shadow-create-btn"
            >
              {creating ? t('shadowWorkspace.creating') : t('shadowWorkspace.create')}
            </button>
          </div>
          <p style="font-size:11px;color:rgba(255,255,255,0.3);">{t('shadowWorkspace.hint')}</p>
        </div>

        {/* Workspace 列表 */}
        {workspaces.length === 0 && !loading && (
          <div
            class="stat-card"
            style="padding:48px;text-align:center;color:rgba(255,255,255,0.4);"
            data-testid="shadow-empty"
          >
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
              class="task-card"
              data-testid={`shadow-item-${ws.id}`}
            >
              {/* 任务头部:状态 + ID + 分支 */}
              <div class="task-header">
                <span
                  class={`task-status ${STATUS_TASK_CLASS[ws.status]}`}
                  style={{ padding: '2px 8px', borderRadius: '4px', fontSize: '10px', fontWeight: 600 }}
                  data-testid={`shadow-status-${ws.id}`}
                >
                  {t(STATUS_LABELS[ws.status])}
                </span>
                <span style="font-size:11px;color:rgba(255,255,255,0.35);font-family:monospace;">{ws.id}</span>
                <span style="font-size:11px;color:rgba(255,255,255,0.3);">·</span>
                <span style="font-size:11px;color:rgba(255,255,255,0.35);font-family:monospace;">{ws.branch}</span>
                {ws.error && (
                  <span style="font-size:11px;color:#ff5f57;overflow:hidden;text-overflow:ellipsis;" title={ws.error}>
                    ⚠ {ws.error}
                  </span>
                )}
              </div>

              {/* 任务描述 */}
              <div style="font-size:13px;color:rgba(255,255,255,0.8);margin-bottom:6px;">
                {ws.task_description}
              </div>

              {/* 元数据 */}
              <div style="display:flex;flex-wrap:wrap;gap:12px;font-size:11px;color:rgba(255,255,255,0.3);margin-bottom:8px;">
                <span>base: {ws.base_branch}</span>
                <span>{t('shadowWorkspace.createdAt', { time: formatTime(ws.created_at) })}</span>
                {ws.finished_at && (
                  <span>{t('shadowWorkspace.finishedAt', { time: formatTime(ws.finished_at) })}</span>
                )}
              </div>

              {/* 操作按钮 */}
              <div class="shadow-actions" style="padding:0;border-top:none;">
                <button
                  onClick={() => handleViewDiff(ws.id)}
                  class="tool-btn"
                  style={{ cursor: 'pointer' }}
                  data-testid={`shadow-diff-btn-${ws.id}`}
                >
                  {isExpanded ? t('shadowWorkspace.hideDiff') : t('shadowWorkspace.viewDiff')}
                </button>
                <button
                  onClick={() => handleViewRecording(ws.id)}
                  class="tool-btn"
                  style={{ cursor: 'pointer' }}
                  data-testid={`shadow-replay-btn-${ws.id}`}
                  title={t('shadowWorkspace.replayTitle')}
                >
                  {isRecExpanded ? t('shadowWorkspace.hideReplay') : t('shadowWorkspace.replay')}
                </button>
                {canComplete && (
                  <button
                    onClick={() => handleComplete(ws.id)}
                    class="tool-btn"
                    style={{ cursor: 'pointer', color: '#28c840' }}
                  >
                    {t('shadowWorkspace.markComplete')}
                  </button>
                )}
                {canMerge && (
                  <button
                    onClick={() => setConfirmAction({ id: ws.id, kind: 'merge' })}
                    class="tool-btn tool-btn-primary"
                    style={{ cursor: 'pointer' }}
                  >
                    ▶ {t('shadowWorkspace.merge')}
                  </button>
                )}
                {canAbort && (
                  <button
                    onClick={() => setConfirmAction({ id: ws.id, kind: 'abort' })}
                    class="tool-btn"
                    style={{ cursor: 'pointer', color: '#ff5f57' }}
                  >
                    🗑 {t('shadowWorkspace.abort')}
                  </button>
                )}
              </div>

              {/* Diff 展开:左右分栏对比(原始 vs 修改) */}
              {isExpanded && (
                <div style="margin-top:10px;" data-testid={`shadow-diff-view-${ws.id}`}>
                  {diffLoading ? (
                    <div style="font-size:12px;color:rgba(255,255,255,0.4);">{t('shadowWorkspace.loadingDiff')}</div>
                  ) : (
                    <div class="shadow-split">
                      {/* 原始文件(删除行高亮) */}
                      <div class="shadow-pane">
                        <div class="shadow-pane-header">📄 原始文件</div>
                        <div class="shadow-pane-body">
                          {diffText.split('\n').map((line, i) => {
                            if (line.startsWith('+++') || line.startsWith('---')) return null;
                            if (line.startsWith('+')) return <div key={i}>&nbsp;</div>;
                            const isDel = line.startsWith('-');
                            return (
                              <div key={i} class={isDel ? 'diff-del' : ''}>
                                {line || '\u00A0'}
                              </div>
                            );
                          })}
                        </div>
                      </div>
                      {/* 影子副本(添加行高亮) */}
                      <div class="shadow-pane">
                        <div class="shadow-pane-header">📝 影子副本 (已修改)</div>
                        <div class="shadow-pane-body">
                          {diffText.split('\n').map((line, i) => {
                            if (line.startsWith('+++') || line.startsWith('---')) return null;
                            if (line.startsWith('-')) return <div key={i}>&nbsp;</div>;
                            const isAdd = line.startsWith('+');
                            return (
                              <div key={i} class={isAdd ? 'diff-add' : ''}>
                                {line || '\u00A0'}
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* T-E-C-09: 录屏回放时间线 */}
              {isRecExpanded && (
                <div style="margin-top:10px;" data-testid={`shadow-recording-view-${ws.id}`}>
                  {recLoading ? (
                    <div style="font-size:12px;color:rgba(255,255,255,0.4);">{t('shadowWorkspace.loadingRecording')}</div>
                  ) : recOps.length === 0 ? (
                    <div
                      style="font-size:12px;color:rgba(255,255,255,0.3);"
                      data-testid={`shadow-recording-empty-${ws.id}`}
                    >
                      {t('shadowWorkspace.recordingEmpty')}
                    </div>
                  ) : (
                    <div class="stat-card" style="padding:0;overflow:hidden;">
                      <div style="font-size:12px;color:rgba(255,255,255,0.4);padding:8px 12px;border-bottom:1px solid rgba(255,255,255,0.04);">
                        {t('shadowWorkspace.recordingHeader', { n: recOps.length })}
                      </div>
                      <ul style="list-style:none;max-height:240px;overflow-y:auto;margin:0;padding:0;">
                        {recOps.map((op) => (
                          <li
                            key={op.seq}
                            style="padding:6px 12px;cursor:pointer;border-bottom:1px solid rgba(255,255,255,0.03);"
                            data-testid={`shadow-recording-op-${ws.id}-${op.seq}`}
                            onClick={() => setRecExpanded(recExpanded === op.seq ? null : op.seq)}
                          >
                            <div style="display:flex;align-items:center;gap:8px;">
                              <span style="font-size:11px;color:rgba(255,255,255,0.3);font-family:monospace;width:24px;flex-shrink:0;">
                                #{op.seq}
                              </span>
                              <span style="font-size:13px;flex-shrink:0;">{OP_ICONS[op.kind]}</span>
                              <span style="font-size:11px;color:rgba(255,255,255,0.4);flex-shrink:0;">
                                {t(OP_LABELS[op.kind])}
                              </span>
                              {op.target && (
                                <span style="font-size:11px;color:rgba(255,255,255,0.3);font-family:monospace;overflow:hidden;text-overflow:ellipsis;flex:1;">
                                  {op.target}
                                </span>
                              )}
                              <span style={{ fontSize: '11px', flexShrink: 0, color: op.success ? '#28c840' : '#ff5f57' }}>
                                {op.success ? '✓' : '✗'}
                              </span>
                              <span style="font-size:11px;color:rgba(255,255,255,0.3);flex-shrink:0;">
                                {formatRecTime(op.ts_ms)}
                              </span>
                            </div>
                            {op.detail && recExpanded !== op.seq && (
                              <div style="font-size:11px;color:rgba(255,255,255,0.3);margin-top:2px;margin-left:32px;overflow:hidden;text-overflow:ellipsis;">
                                {op.detail}
                              </div>
                            )}
                            {recExpanded === op.seq && (
                              <div
                                style="margin-top:4px;margin-left:32px;"
                                data-testid={`shadow-recording-detail-${ws.id}-${op.seq}`}
                              >
                                {op.detail && (
                                  <div>
                                    <span style="font-size:11px;color:rgba(255,255,255,0.3);">{t('shadowWorkspace.detailLabel')}</span>
                                    <code style="font-size:11px;color:rgba(255,255,255,0.3);">{op.detail}</code>
                                  </div>
                                )}
                                {op.message && (
                                  <div>
                                    <span style="font-size:11px;color:rgba(255,255,255,0.3);">{t('shadowWorkspace.messageLabel')}</span>
                                    <pre style="font-size:11px;color:rgba(255,255,255,0.4);background:rgba(0,0,0,0.3);border:1px solid rgba(255,255,255,0.04);border-radius:4px;padding:4px;margin-top:2px;overflow:auto;max-height:160px;white-space:pre-wrap;">
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
          style="position:fixed;inset:0;background:rgba(0,0,0,0.6);display:flex;align-items:center;justify-content:center;z-index:50;"
          data-testid="shadow-confirm-dialog"
        >
          <div class="stat-card" style="padding:16px;max-width:320px;">
            <h3 style="font-size:14px;font-weight:600;color:rgba(255,255,255,0.9);margin-bottom:8px;">
              {confirmAction.kind === 'merge'
                ? t('shadowWorkspace.confirmMergeTitle')
                : t('shadowWorkspace.confirmAbortTitle')}
            </h3>
            <p style="font-size:12px;color:rgba(255,255,255,0.4);margin-bottom:16px;">
              {confirmAction.kind === 'merge'
                ? t('shadowWorkspace.confirmMergeBody')
                : t('shadowWorkspace.confirmAbortBody')}
            </p>
            <div style="display:flex;gap:8px;justify-content:flex-end;">
              <button
                onClick={() => setConfirmAction(null)}
                class="tool-btn"
                style={{ cursor: 'pointer' }}
              >
                {t('shadowWorkspace.dialogCancel')}
              </button>
              <button
                onClick={handleConfirmAction}
                class={`tool-btn ${confirmAction.kind === 'merge' ? 'tool-btn-primary' : ''}`}
                style={{ cursor: 'pointer', color: confirmAction.kind === 'merge' ? '#fff' : '#ff5f57' }}
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
