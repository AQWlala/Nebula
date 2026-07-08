/**
 * M6 #78: 进化日志 UI — EvolutionLog 查看器 + 回滚操作。
 *
 * ## 功能
 * - 列出全部 EvolutionLogEntry(按写入顺序倒序展示,最新在前)
 * - 显示每个 Phase 的元数据:entry_id / phase / timestamp / master_id /
 *   memory_id / content_bytes / soul_md_path
 * - 按 phase 着色(extract=蓝 / compile=绿 / reflect=紫 / soul=橙)
 * - 运行时开关:查询 + 切换 evolution_enabled,前端实时反映状态
 * - 回滚操作:输入 N,调 `evolutionRollback(n)`,展示结果
 *   (实际回滚条数 + 失败 warnings),成功后刷新列表
 * - 容错:后端未启用 evolution-engine / self-evolution feature 时,
 *   invoke 会 reject,UI 显示 "进化引擎未编译进二进制" 提示
 *
 * ## 集成
 * 从 Settings.tsx 的"进化日志"按钮触发,作为 Modal 弹出(类似 SoulEditor)。
 *
 * ## 不做的事
 * - 不触发 4 Phase 进化(evolution_run 命令未实现,需流式事件推送)
 * - 不编辑 EvolutionEngineConfig(当前硬编码)
 * - 不显示 L2/L3/L5 memory 内容(需调 memory_get,超出本组件范围)
 */
import { useState, useEffect, useCallback } from 'preact/hooks';
import {
  nebulaAPI,
  type EvolutionLogEntry,
  type EvolutionPhase,
  type RollbackResult,
} from '../lib/tauri';
import { Modal } from './Modal';
import { toast, toastFromError } from './Toast';
import { t } from '../i18n';

interface EvolutionLogViewProps {
  open: boolean;
  onClose: () => void;
}

/** Phase 元数据:着色 + 中文标签 + emoji 图标。 */
const PHASE_META: Record<
  EvolutionPhase,
  { icon: string; label: string; color: string; bg: string }
> = {
  extract: {
    icon: '🔍',
    label: t('evolutionLog.phase.extract'),
    color: '#3b82f6',
    bg: 'rgba(59,130,246,0.12)',
  },
  compile: {
    icon: '🔧',
    label: t('evolutionLog.phase.compile'),
    color: '#10b981',
    bg: 'rgba(16,185,129,0.12)',
  },
  reflect: {
    icon: '🧠',
    label: t('evolutionLog.phase.reflect'),
    color: '#8b5cf6',
    bg: 'rgba(139,92,246,0.12)',
  },
  soul: {
    icon: '✨',
    label: t('evolutionLog.phase.soul'),
    color: '#f59e0b',
    bg: 'rgba(245,158,11,0.12)',
  },
};

/** 格式化字节数为人类可读(B / KB / MB)。 */
function formatBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / 1024 / 1024).toFixed(2)} MB`;
}

/** 格式化 RFC3339 时间戳为本地时间(YYYY-MM-DD HH:MM:SS)。 */
function formatTimestamp(rfc3339: string): string {
  try {
    const d = new Date(rfc3339);
    if (Number.isNaN(d.getTime())) return rfc3339;
    const pad = (n: number) => n.toString().padStart(2, '0');
    return (
      `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
      `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
    );
  } catch {
    return rfc3339;
  }
}

/** 截断长字符串(如 memory_id),中间用 … 替换。 */
function truncate(s: string, max = 24): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max / 2)}…${s.slice(-max / 2)}`;
}

export function EvolutionLogView({ open, onClose }: EvolutionLogViewProps) {
  const [loading, setLoading] = useState(false);
  const [entries, setEntries] = useState<EvolutionLogEntry[]>([]);
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [toggling, setToggling] = useState(false);
  const [rollbackN, setRollbackN] = useState(1);
  const [rolling, setRolling] = useState(false);
  const [lastRollback, setLastRollback] = useState<RollbackResult | null>(null);
  /** feature 是否编译进二进制(invoke reject 时设为 false)。 */
  const [featureAvailable, setFeatureAvailable] = useState<boolean>(true);

  /** 加载日志列表 + 运行时开关。 */
  const reload = useCallback(async () => {
    setLoading(true);
    try {
      // 并发拉取(运行时开关 self-evolution feature 即可,日志列表需 evolution-engine)
      const [enabledResult, listResult] = await Promise.allSettled([
        nebulaAPI.evolutionEnabled(),
        nebulaAPI.evolutionLogList(),
      ]);

      // 处理 evolution_enabled
      if (enabledResult.status === 'fulfilled') {
        setEnabled(enabledResult.value);
        setFeatureAvailable(true);
      } else {
        // enabled 命令也 reject → self-evolution feature 未编译
        setEnabled(null);
        setFeatureAvailable(false);
      }

      // 处理 evolution_log_list
      if (listResult.status === 'fulfilled') {
        // 倒序(最新在前)
        setEntries([...listResult.value].reverse());
      } else {
        // 日志命令可能因 evolution-engine feature 未启用而 reject
        // (但 self-evolution 已启用 → enabled 命令成功)
        setEntries([]);
        if (enabledResult.status === 'fulfilled') {
          // self-evolution 启用但 evolution-engine 未启用 → 提示用户
          toast.warning(
            t('evolutionLog.toast.engineNotCompiled.title'),
            t('evolutionLog.toast.engineNotCompiled.body')
          );
        }
      }
    } finally {
      setLoading(false);
    }
  }, []);

  // open 变为 true 时加载
  useEffect(() => {
    if (open) {
      void reload();
      setLastRollback(null);
    }
  }, [open, reload]);

  /** 切换运行时开关。 */
  const handleToggleEnabled = useCallback(async () => {
    if (enabled === null) return;
    setToggling(true);
    try {
      await nebulaAPI.evolutionSetEnabled(!enabled);
      setEnabled(!enabled);
      toast.success(
        !enabled ? t('evolutionLog.toast.enabled.title') : t('evolutionLog.toast.disabled.title'),
        !enabled ? t('evolutionLog.toast.enabled.body') : t('evolutionLog.toast.disabled.body')
      );
    } catch (err) {
      toastFromError(err);
    } finally {
      setToggling(false);
    }
  }, [enabled]);

  /** 执行回滚。 */
  const handleRollback = useCallback(async () => {
    if (rollbackN < 1) {
      toast.warning(
        t('evolutionLog.toast.invalidCount.title'),
        t('evolutionLog.toast.invalidCount.body')
      );
      return;
    }
    setRolling(true);
    try {
      const result = await nebulaAPI.evolutionRollback(rollbackN);
      setLastRollback(result);
      if (result.failed === 0 && result.rolled_back > 0) {
        toast.success(
          t('evolutionLog.toast.rollbackSuccess.title', { rolled_back: result.rolled_back }),
          result.entry_ids.join('\n')
        );
      } else if (result.rolled_back === 0) {
        toast.warning(
          t('evolutionLog.toast.rollbackEmpty.title'),
          result.warnings[0] ?? t('evolutionLog.toast.rollbackEmpty.body')
        );
      } else {
        toast.warning(
          t('evolutionLog.toast.rollbackPartial.title', {
            rolled_back: result.rolled_back,
            failed: result.failed,
          }),
          result.warnings.join('\n')
        );
      }
      // 刷新列表
      await reload();
    } catch (err) {
      toastFromError(err);
    } finally {
      setRolling(false);
    }
  }, [rollbackN, reload]);

  const soulEntriesCount = entries.filter((e) => e.phase === 'soul').length;

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('evolutionLog.title')}
      size="lg"
      footer={
        <>
          <button
            type="button"
            onClick={reload}
            disabled={loading || !featureAvailable}
            style={{
              fontSize: '12px',
              padding: '6px 12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'transparent',
              color: 'var(--text-primary)',
              cursor: loading || !featureAvailable ? 'not-allowed' : 'pointer',
              opacity: loading || !featureAvailable ? 0.5 : 1,
              marginRight: '8px',
            }}
          >
            {loading ? t('evolutionLog.refreshing') : t('evolutionLog.refresh')}
          </button>
          <button
            type="button"
            onClick={onClose}
            style={{
              fontSize: '12px',
              padding: '6px 12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'var(--accent-neon)',
              color: 'var(--bg-primary)',
              cursor: 'pointer',
            }}
          >
            {t('evolutionLog.close')}
          </button>
        </>
      }
    >
      <div class="evolution-log-view">
        {/* Feature 未编译提示 */}
        {!featureAvailable && (
          <div class="evolution-feature-unavailable">
            <strong>{t('evolutionLog.featureUnavailable.title')}</strong>
            <p>{t('evolutionLog.featureUnavailable.body')}</p>
            <p style={{ marginTop: 6, fontSize: 11, opacity: 0.7 }}>
              {t('evolutionLog.featureUnavailable.hint')}
            </p>
          </div>
        )}

        {/* 运行时开关 + 统计概览 */}
        {featureAvailable && (
          <div class="evolution-status-bar">
            <div class="evolution-status-item">
              <span class="evolution-status-label">{t('evolutionLog.runtimeSwitch')}</span>
              <button
                type="button"
                onClick={handleToggleEnabled}
                disabled={toggling || enabled === null}
                class={`evolution-toggle-btn ${enabled ? 'on' : 'off'}`}
                title={
                  enabled === null
                    ? t('evolutionLog.toggle.unknownTitle')
                    : enabled
                      ? t('evolutionLog.toggle.disableTitle')
                      : t('evolutionLog.toggle.enableTitle')
                }
              >
                {toggling
                  ? t('evolutionLog.toggle.toggling')
                  : enabled === null
                    ? t('evolutionLog.toggle.unknown')
                    : enabled
                      ? t('evolutionLog.toggle.enabled')
                      : t('evolutionLog.toggle.disabled')}
              </button>
            </div>
            <div class="evolution-status-item">
              <span class="evolution-status-label">{t('evolutionLog.stat.totalEntries')}</span>
              <span class="evolution-stat-value">{entries.length}</span>
            </div>
            <div class="evolution-status-item">
              <span class="evolution-status-label">{t('evolutionLog.stat.soulEntries')}</span>
              <span class="evolution-stat-value">{soulEntriesCount}</span>
            </div>
          </div>
        )}

        {/* 回滚操作区 */}
        {featureAvailable && soulEntriesCount > 0 && (
          <div class="evolution-rollback-bar">
            <label class="evolution-rollback-label">
              {t('evolutionLog.rollback.labelPrefix')}
              <input
                type="number"
                min={1}
                max={soulEntriesCount}
                value={rollbackN}
                onInput={(e) =>
                  setRollbackN(parseInt((e.target as HTMLInputElement).value, 10) || 1)
                }
                disabled={rolling}
                style={{
                  width: 60,
                  margin: '0 6px',
                  padding: '4px 6px',
                  fontSize: 12,
                  borderRadius: 4,
                  border: '1px solid var(--border)',
                  background: 'var(--bg-secondary)',
                  color: 'var(--text-primary)',
                }}
              />
              {t('evolutionLog.rollback.labelSuffix')}
            </label>
            <button
              type="button"
              onClick={handleRollback}
              disabled={rolling || rollbackN < 1}
              style={{
                fontSize: 12,
                padding: '6px 14px',
                borderRadius: 4,
                border: '1px solid #ef4444',
                background: rolling ? 'rgba(239,68,68,0.3)' : 'rgba(239,68,68,0.1)',
                color: '#ef4444',
                cursor: rolling || rollbackN < 1 ? 'not-allowed' : 'pointer',
                opacity: rolling || rollbackN < 1 ? 0.6 : 1,
              }}
            >
              {rolling
                ? t('evolutionLog.rollback.rolling')
                : t('evolutionLog.rollback.button', { n: rollbackN })}
            </button>
          </div>
        )}

        {/* 最近一次回滚结果 */}
        {lastRollback && (
          <div class="evolution-rollback-result">
            <strong>{t('evolutionLog.rollbackResult.title')}</strong>
            <div style={{ marginTop: 4 }}>
              {t('evolutionLog.rollbackResult.summary', {
                requested: lastRollback.requested_count,
                success: lastRollback.rolled_back,
                failed: lastRollback.failed,
              })}
            </div>
            {lastRollback.entry_ids.length > 0 && (
              <ul style={{ margin: '4px 0 0', paddingLeft: 18, fontSize: 11 }}>
                {lastRollback.entry_ids.map((id) => (
                  <li key={id} style={{ fontFamily: 'monospace' }}>
                    {id}
                  </li>
                ))}
              </ul>
            )}
            {lastRollback.warnings.length > 0 && (
              <div class="evolution-warnings">
                {lastRollback.warnings.map((w, i) => (
                  <div key={i}>⚠️ {w}</div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* 日志列表 */}
        {featureAvailable && (
          <div class="evolution-log-list">
            {loading && entries.length === 0 ? (
              <div class="evolution-empty">{t('evolutionLog.loading')}</div>
            ) : entries.length === 0 ? (
              <div class="evolution-empty">
                {t('evolutionLog.empty.title')}
                <div style={{ marginTop: 6, fontSize: 11, opacity: 0.7 }}>
                  {t('evolutionLog.empty.hint')}
                </div>
              </div>
            ) : (
              <ul class="evolution-entries">
                {entries.map((entry) => {
                  const meta = PHASE_META[entry.phase];
                  return (
                    <li
                      key={entry.entry_id}
                      class="evolution-entry"
                      style={{
                        borderLeft: `3px solid ${meta.color}`,
                        background: meta.bg,
                      }}
                    >
                      <div class="evolution-entry-header">
                        <span class="evolution-entry-phase" style={{ color: meta.color }}>
                          {meta.icon} {meta.label}
                        </span>
                        <span class="evolution-entry-timestamp">
                          {formatTimestamp(entry.timestamp)}
                        </span>
                      </div>
                      <div class="evolution-entry-meta">
                        <div class="evolution-meta-row">
                          <span class="evolution-meta-key">entry_id</span>
                          <code class="evolution-meta-val">{entry.entry_id}</code>
                        </div>
                        <div class="evolution-meta-row">
                          <span class="evolution-meta-key">master_id</span>
                          <code class="evolution-meta-val">{entry.master_id}</code>
                        </div>
                        {entry.memory_id && (
                          <div class="evolution-meta-row">
                            <span class="evolution-meta-key">memory_id</span>
                            <code class="evolution-meta-val" title={entry.memory_id}>
                              {truncate(entry.memory_id)}
                            </code>
                          </div>
                        )}
                        <div class="evolution-meta-row">
                          <span class="evolution-meta-key">content_bytes</span>
                          <span class="evolution-meta-val">{formatBytes(entry.content_bytes)}</span>
                        </div>
                        {entry.soul_md_path && (
                          <div class="evolution-meta-row">
                            <span class="evolution-meta-key">soul_md_path</span>
                            <code class="evolution-meta-val">{entry.soul_md_path}</code>
                          </div>
                        )}
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
