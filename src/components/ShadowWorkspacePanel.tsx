/**
 * T-E-C-08: Shadow Workspace 面板。
 *
 * 管理 Agent 隔离执行环境:
 * - 创建新 workspace(任务描述 + 可选 base 分支)
 * - 列出所有 workspace(状态/分支/时间戳)
 * - 查看 diff(与 base_branch 对比)
 * - 合并(merge 回 base)或丢弃(abort)
 *
 * 设计要点:
 * - 状态色标:running 蓝/completed 绿/failed 红/merged 灰/aborted 黄
 * - diff 在内联展开(不弹窗,长 diff 用 pre-wrap + max-height 滚动)
 * - 合并/丢弃需二次确认(不可逆操作)
 */
import { useState, useEffect, useCallback } from 'preact/hooks';
import { nebulaAPI, type ShadowWorkspace, type ShadowStatus } from '../lib/tauri';
import { toast } from './Toast';
import { t } from '../i18n';

const STATUS_COLORS: Record<ShadowStatus, string> = {
  creating: '#9CA3AF',
  running: '#3B82F6',
  completed: '#10B981',
  failed: '#EF4444',
  merged: '#6B7280',
  aborted: '#F59E0B',
};

const STATUS_LABELS: Record<ShadowStatus, string> = {
  creating: '创建中',
  running: '运行中',
  completed: '已完成',
  failed: '失败',
  merged: '已合并',
  aborted: '已丢弃',
};

function formatTime(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
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
  const [confirmAction, setConfirmAction] = useState<{ id: string; kind: 'merge' | 'abort' } | null>(null);

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
      toast.warning('请输入任务描述');
      return;
    }
    setCreating(true);
    try {
      const ws = await nebulaAPI.shadowCreate(taskDesc.trim(), baseBranch.trim() || null);
      toast.success(`已创建 Shadow Workspace: ${ws.id}`);
      setTaskDesc('');
      setBaseBranch('');
      await refresh();
    } catch (e) {
      toast.error('创建失败', String(e));
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
      setDiffText(diff || '(无差异)');
    } catch (e) {
      setDiffText(`获取 diff 失败: ${e}`);
    } finally {
      setDiffLoading(false);
    }
  };

  const handleComplete = async (id: string) => {
    try {
      await nebulaAPI.shadowComplete(id);
      toast.success('已标记完成');
      await refresh();
    } catch (e) {
      toast.error('操作失败', String(e));
    }
  };

  const handleConfirmAction = async () => {
    if (!confirmAction) return;
    const { id, kind } = confirmAction;
    try {
      if (kind === 'merge') {
        await nebulaAPI.shadowMerge(id);
        toast.success('已合并回 base 分支');
      } else {
        await nebulaAPI.shadowAbort(id);
        toast.success('已丢弃');
      }
      await refresh();
    } catch (e) {
      toast.error(kind === 'merge' ? '合并失败' : '丢弃失败', String(e));
    } finally {
      setConfirmAction(null);
    }
  };

  return (
    <div className="shadow-workspace-panel h-full flex flex-col bg-gray-950 text-white" data-testid="shadow-workspace-panel">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-800">
        <h2 className="text-sm font-semibold text-gray-300">🌑 Shadow Workspace</h2>
        <div className="flex items-center gap-3">
          {loading && <span className="text-xs text-gray-500">加载中…</span>}
          <span className="text-xs text-gray-500">{workspaces.length} 个工作区</span>
          <button
            onClick={refresh}
            className="text-xs text-gray-400 hover:text-white transition-colors"
            title="刷新"
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
            placeholder="任务描述,如:重构 memory 模块..."
            value={taskDesc}
            onInput={(e) => setTaskDesc((e.target as HTMLInputElement).value)}
            className="flex-1 px-2 py-1 text-sm bg-gray-900 border border-gray-700 rounded text-white placeholder-gray-600 focus:border-blue-600 outline-none"
            data-testid="shadow-task-input"
          />
          <input
            type="text"
            placeholder="base 分支(可选,默认当前)"
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
            {creating ? '创建中…' : '创建'}
          </button>
        </div>
        <p className="text-xs text-gray-600">
          Agent 将在独立 git worktree + 分支 <code className="text-gray-500">agent/&lt;id&gt;</code> 中执行,不影响当前工作区。
        </p>
      </div>

      {/* 列表 */}
      <div className="flex-1 overflow-y-auto">
        {workspaces.length === 0 && !loading && (
          <div className="text-center text-gray-500 py-12" data-testid="shadow-empty">
            暂无 Shadow Workspace
          </div>
        )}
        {workspaces.map((ws) => {
          const isExpanded = diffFor === ws.id;
          const canMerge = ws.status === 'completed' || ws.status === 'running';
          const canAbort = ws.status === 'running' || ws.status === 'completed' || ws.status === 'failed';
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
                  style={{ backgroundColor: `${STATUS_COLORS[ws.status]}33`, color: STATUS_COLORS[ws.status] }}
                >
                  {STATUS_LABELS[ws.status]}
                </span>
                <span className="text-xs text-gray-600">·</span>
                <span className="text-xs text-gray-500 font-mono">{ws.branch}</span>
                {ws.error && (
                  <span className="text-xs text-red-400 truncate" title={ws.error}>⚠ {ws.error}</span>
                )}
              </div>
              {/* 任务描述 */}
              <div className="text-sm text-gray-200 mb-1">{ws.task_description}</div>
              {/* 元数据 */}
              <div className="flex flex-wrap gap-3 text-xs text-gray-500 mb-1">
                <span>base: {ws.base_branch}</span>
                <span>创建: {formatTime(ws.created_at)}</span>
                {ws.finished_at && <span>完成: {formatTime(ws.finished_at)}</span>}
              </div>
              {/* 操作按钮 */}
              <div className="flex flex-wrap gap-2 mt-1">
                <button
                  onClick={() => handleViewDiff(ws.id)}
                  className="text-xs px-2 py-0.5 bg-gray-800 hover:bg-gray-700 rounded text-gray-300 transition-colors"
                  data-testid={`shadow-diff-btn-${ws.id}`}
                >
                  {isExpanded ? '隐藏 diff' : '查看 diff'}
                </button>
                {canComplete && (
                  <button
                    onClick={() => handleComplete(ws.id)}
                    className="text-xs px-2 py-0.5 bg-green-900/60 hover:bg-green-800/60 rounded text-green-300 transition-colors"
                  >
                    标记完成
                  </button>
                )}
                {canMerge && (
                  <button
                    onClick={() => setConfirmAction({ id: ws.id, kind: 'merge' })}
                    className="text-xs px-2 py-0.5 bg-blue-900/60 hover:bg-blue-800/60 rounded text-blue-300 transition-colors"
                  >
                    合并
                  </button>
                )}
                {canAbort && (
                  <button
                    onClick={() => setConfirmAction({ id: ws.id, kind: 'abort' })}
                    className="text-xs px-2 py-0.5 bg-red-900/60 hover:bg-red-800/60 rounded text-red-300 transition-colors"
                  >
                    丢弃
                  </button>
                )}
              </div>
              {/* diff 展开 */}
              {isExpanded && (
                <div className="mt-2" data-testid={`shadow-diff-view-${ws.id}`}>
                  {diffLoading ? (
                    <div className="text-xs text-gray-500">加载 diff…</div>
                  ) : (
                    <pre className="text-xs text-gray-300 bg-gray-900 border border-gray-800 rounded p-2 overflow-auto max-h-80 whitespace-pre-wrap">
                      {diffText}
                    </pre>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {/* 二次确认对话框 */}
      {confirmAction && (
        <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" data-testid="shadow-confirm-dialog">
          <div className="bg-gray-900 border border-gray-700 rounded-lg p-4 max-w-sm">
            <h3 className="text-sm font-semibold text-white mb-2">
              {confirmAction.kind === 'merge' ? '确认合并' : '确认丢弃'}
            </h3>
            <p className="text-xs text-gray-400 mb-4">
              {confirmAction.kind === 'merge'
                ? '将把 Agent 分支的修改合并回 base 分支。合并后 worktree 将被清理。'
                : '将强制清理 worktree 并删除分支,所有未合并的修改将丢失。此操作不可逆。'}
            </p>
            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setConfirmAction(null)}
                className="text-xs px-3 py-1 bg-gray-800 hover:bg-gray-700 rounded text-gray-300"
              >
                取消
              </button>
              <button
                onClick={handleConfirmAction}
                className={`text-xs px-3 py-1 rounded text-white ${confirmAction.kind === 'merge' ? 'bg-blue-600 hover:bg-blue-700' : 'bg-red-600 hover:bg-red-700'}`}
                data-testid="shadow-confirm-btn"
              >
                {confirmAction.kind === 'merge' ? '合并' : '丢弃'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
