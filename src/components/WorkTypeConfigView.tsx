/**
 * M6 #83: WorkType 配置 UI — models.json work_type_overrides 可视化编辑。
 *
 * ## 功能
 * - 7 个 WorkType 行(chat / swarm_worker / swarm_synthesize / master_task /
 *   evolution / soul_compile / classifier)
 * - 每行:provider 下拉 + model 下拉 + temperature 输入 + max_tokens 输入
 * - local_only 的 3 行(evolution/soul_compile/classifier):
 *   provider 锁定为 local_provider,显示 local-only 徽章
 * - provider 测试按钮:调用 modelsConfigTestProvider,显示 ok/延迟/错误
 * - 保存按钮:调用 modelsConfigSave 发送整个 ModelsConfig(持久化 + 热更新)
 *
 * ## 集成
 * 从 Settings.tsx 的"WorkType 配置"按钮触发,作为 Modal 弹出(类似 SoulEditor)。
 *
 * ## 不做的事
 * - 不编辑 local_provider / local_*_model(走 Settings.tsx 旧 UI 或独立组件)
 * - 不编辑 providers 列表(走 Settings.tsx 旧 UI)
 * - 不测试模型推理(仅测试 provider TCP/HTTP 可达性)
 */
import { useState, useEffect, useMemo, useCallback } from 'preact/hooks';
import {
  nebulaAPI,
  type ModelsConfig,
  type WorkType,
  type WorkTypeOverrideEntry,
  type ProviderTestResult,
} from '../lib/tauri';
import { Modal } from './Modal';
import { toast, toastFromError } from './Toast';
import { t } from '../i18n';

interface WorkTypeConfigViewProps {
  open: boolean;
  onClose: () => void;
}

/** 7 个 WorkType 按�固定顺序展示。 */
const WORK_TYPES: WorkType[] = [
  'chat',
  'swarm_worker',
  'swarm_synthesize',
  'master_task',
  'evolution',
  'soul_compile',
  'classifier',
];

/** local_only 的 WorkType(Evolution / SoulCompile / Classifier)。 */
const LOCAL_ONLY_WORK_TYPES: ReadonlySet<WorkType> = new Set([
  'evolution',
  'soul_compile',
  'classifier',
]);

/** WorkType 元数据:label + 描述 + 是否 local_only。 */
const WORK_TYPE_META: Record<WorkType, { label: string; desc: string }> = {
  chat: {
    label: t('workTypeConfig.workType.chat.label'),
    desc: t('workTypeConfig.workType.chat.desc'),
  },
  swarm_worker: {
    label: t('workTypeConfig.workType.swarm_worker.label'),
    desc: t('workTypeConfig.workType.swarm_worker.desc'),
  },
  swarm_synthesize: {
    label: t('workTypeConfig.workType.swarm_synthesize.label'),
    desc: t('workTypeConfig.workType.swarm_synthesize.desc'),
  },
  master_task: {
    label: t('workTypeConfig.workType.master_task.label'),
    desc: t('workTypeConfig.workType.master_task.desc'),
  },
  evolution: {
    label: t('workTypeConfig.workType.evolution.label'),
    desc: t('workTypeConfig.workType.evolution.desc'),
  },
  soul_compile: {
    label: t('workTypeConfig.workType.soul_compile.label'),
    desc: t('workTypeConfig.workType.soul_compile.desc'),
  },
  classifier: {
    label: t('workTypeConfig.workType.classifier.label'),
    desc: t('workTypeConfig.workType.classifier.desc'),
  },
};

/** 单个 WorkType 行的本地编辑状态(深拷贝自 ModelsConfig.work_type_overrides)。 */
interface WorkTypeRowState {
  provider: string;
  model: string;
  temperature: string; // 字符串输入,保存时 parseFloat
  max_tokens: string; // 字符串输入,保存时 parseInt
}

/** Provider 测试状态(按 provider id 索引)。 */
interface ProviderTestState {
  loading: boolean;
  result: ProviderTestResult | null;
}

/** 默认行状态(provider=空字符串,保存时若仍空则跳过该 override)。 */
function defaultRow(): WorkTypeRowState {
  return { provider: '', model: '', temperature: '', max_tokens: '' };
}

/** 从 ModelsConfig 提取初始行状态。 */
function rowsFromConfig(config: ModelsConfig): Record<WorkType, WorkTypeRowState> {
  const rows: Record<string, WorkTypeRowState> = {};
  for (const wt of WORK_TYPES) {
    const ov = config.work_type_overrides?.[wt];
    if (ov) {
      rows[wt] = {
        provider: ov.provider ?? '',
        model: ov.model ?? '',
        temperature: ov.temperature != null ? String(ov.temperature) : '',
        max_tokens: ov.max_tokens != null ? String(ov.max_tokens) : '',
      };
    } else {
      rows[wt] = defaultRow();
    }
  }
  return rows as Record<WorkType, WorkTypeRowState>;
}

/** 从行状态构建 override map(空 provider 的行跳过)。 */
function overridesFromRows(
  rows: Record<WorkType, WorkTypeRowState>
): Record<string, WorkTypeOverrideEntry> {
  const out: Record<string, WorkTypeOverrideEntry> = {};
  for (const wt of WORK_TYPES) {
    const row = rows[wt];
    if (!row.provider.trim()) continue;
    const temp = row.temperature.trim();
    const maxTok = row.max_tokens.trim();
    out[wt] = {
      provider: row.provider.trim(),
      model: row.model.trim(),
      temperature: temp ? parseFloat(temp) : null,
      max_tokens: maxTok ? parseInt(maxTok, 10) : null,
    };
  }
  return out;
}

export function WorkTypeConfigView({ open, onClose }: WorkTypeConfigViewProps) {
  const [config, setConfig] = useState<ModelsConfig | null>(null);
  const [rows, setRows] = useState<Record<WorkType, WorkTypeRowState> | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [tests, setTests] = useState<Record<string, ProviderTestState>>({});
  /** 行展开状态(默认全部展开)。 */
  const [expanded, setExpanded] = useState<Record<WorkType, boolean>>(() => {
    const e: Record<string, boolean> = {};
    for (const wt of WORK_TYPES) e[wt] = true;
    return e as Record<WorkType, boolean>;
  });

  /** 加载 ModelsConfig。 */
  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const cfg = await nebulaAPI.modelsConfigLoad();
      setConfig(cfg);
      setRows(rowsFromConfig(cfg));
      setTests({});
    } catch (err) {
      toastFromError(err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) {
      void reload();
    }
  }, [open, reload]);

  /** 单行字段更新。 */
  const updateRow = useCallback((wt: WorkType, patch: Partial<WorkTypeRowState>) => {
    setRows((prev) => {
      if (!prev) return prev;
      return { ...prev, [wt]: { ...prev[wt], ...patch } };
    });
  }, []);

  /** local_only WorkType 的 provider 被用户改动时,回滚到 local_provider。 */
  const handleProviderChange = useCallback(
    (wt: WorkType, newProvider: string) => {
      if (LOCAL_ONLY_WORK_TYPES.has(wt)) {
        // 不允许改 local_only 行的 provider
        return;
      }
      updateRow(wt, { provider: newProvider });
    },
    [updateRow]
  );

  /** 测试某个 provider。 */
  const handleTestProvider = useCallback(async (providerId: string) => {
    if (!providerId) return;
    setTests((prev) => ({
      ...prev,
      [providerId]: { loading: true, result: null },
    }));
    try {
      const result = await nebulaAPI.modelsConfigTestProvider(providerId);
      setTests((prev) => ({
        ...prev,
        [providerId]: { loading: false, result },
      }));
      if (result.ok) {
        toast.success(
          t('workTypeConfig.toast.testOk.title'),
          t('workTypeConfig.toast.testOk.body', {
            latency: result.latency_ms,
            status: result.status_code ?? 'N/A',
          })
        );
      } else {
        toast.warning(
          t('workTypeConfig.toast.testFail.title'),
          result.error ?? t('workTypeConfig.toast.testFail.body')
        );
      }
    } catch (err) {
      setTests((prev) => ({
        ...prev,
        [providerId]: { loading: false, result: null },
      }));
      toastFromError(err);
    }
  }, []);

  /** 保存。 */
  const handleSave = useCallback(async () => {
    if (!config || !rows) return;
    setSaving(true);
    try {
      const updated: ModelsConfig = {
        ...config,
        work_type_overrides: overridesFromRows(rows),
      };
      const result = await nebulaAPI.modelsConfigSave(updated);
      setConfig(result);
      setRows(rowsFromConfig(result));
      toast.success(t('workTypeConfig.toast.saved.title'), t('workTypeConfig.toast.saved.body'));
    } catch (err) {
      toastFromError(err);
    } finally {
      setSaving(false);
    }
  }, [config, rows]);

  /** 重置为加载时的状态。 */
  const handleReset = useCallback(() => {
    if (config) {
      setRows(rowsFromConfig(config));
      setTests({});
    }
  }, [config]);

  /** dirty 检测:rows 与 config 的 overrides 是否一致。 */
  const dirty = useMemo(() => {
    if (!config || !rows) return false;
    const current = JSON.stringify(overridesFromRows(rows));
    const saved = JSON.stringify(config.work_type_overrides ?? {});
    return current !== saved;
  }, [config, rows]);

  /** 展开状态切换。 */
  const toggleExpand = useCallback((wt: WorkType) => {
    setExpanded((prev) => ({ ...prev, [wt]: !prev[wt] }));
  }, []);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('workTypeConfig.title')}
      size="xl"
      footer={
        <>
          <button
            type="button"
            onClick={handleReset}
            disabled={!dirty || saving || loading}
            style={{
              fontSize: '12px',
              padding: '4px 12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'transparent',
              color: 'var(--text-primary)',
              cursor: !dirty || saving || loading ? 'not-allowed' : 'pointer',
              opacity: !dirty || saving || loading ? 0.5 : 1,
              marginRight: '8px',
            }}
          >
            {t('workTypeConfig.reset')}
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={!dirty || saving || loading}
            style={{
              fontSize: '12px',
              padding: '4px 12px',
              borderRadius: '4px',
              border: '1px solid var(--accent)',
              background: 'var(--accent)',
              color: 'var(--bg-primary)',
              cursor: !dirty || saving || loading ? 'not-allowed' : 'pointer',
              opacity: !dirty || saving || loading ? 0.5 : 1,
              fontWeight: 500,
            }}
          >
            {saving ? t('workTypeConfig.saving') : t('workTypeConfig.save')}
          </button>
        </>
      }
    >
      <div style={{ marginBottom: '12px', color: 'var(--text-secondary)', fontSize: '12px' }}>
        {t('workTypeConfig.hint')}
      </div>

      {loading || !config || !rows ? (
        <div style={{ padding: '24px', textAlign: 'center', color: 'var(--text-secondary)' }}>
          {t('workTypeConfig.loading')}
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
          {/* 全局 local_provider 提示 */}
          <div
            style={{
              padding: '8px 12px',
              borderRadius: '6px',
              background: 'var(--bg-secondary)',
              border: '1px solid var(--border)',
              fontSize: '12px',
              color: 'var(--text-secondary)',
            }}
          >
            <strong style={{ color: 'var(--text-primary)' }}>
              {t('workTypeConfig.localProviderLabel')}:
            </strong>{' '}
            <code style={{ color: 'var(--accent)' }}>{config.local_provider}</code>
            {' · '}
            <strong style={{ color: 'var(--text-primary)' }}>
              {t('workTypeConfig.localClassifierModelLabel')}:
            </strong>{' '}
            <code>{config.local_classifier_model}</code>
            {' · '}
            <strong style={{ color: 'var(--text-primary)' }}>
              {t('workTypeConfig.localEvolutionModelLabel')}:
            </strong>{' '}
            <code>{config.local_evolution_model}</code>
            {' · '}
            <strong style={{ color: 'var(--text-primary)' }}>
              {t('workTypeConfig.localSoulModelLabel')}:
            </strong>{' '}
            <code>{config.local_soul_model}</code>
            {' · '}
            <strong style={{ color: 'var(--text-primary)' }}>
              {t('workTypeConfig.workerLocalModelLabel')}:
            </strong>{' '}
            <code>{config.worker_local_model}</code>
          </div>

          {WORK_TYPES.map((wt) => {
            const row = rows[wt];
            const meta = WORK_TYPE_META[wt];
            const isLocalOnly = LOCAL_ONLY_WORK_TYPES.has(wt);
            // local_only 行的 provider 锁定为 local_provider
            const effectiveProvider = isLocalOnly ? config.local_provider : row.provider;
            // 选中 provider 的对象(用于下拉 model 列表)
            const providerObj = config.providers.find((p) => p.id === effectiveProvider);
            const modelOptions = providerObj?.models ?? [];
            const testState = tests[effectiveProvider];
            const isExpanded = expanded[wt];

            return (
              <div
                key={wt}
                style={{
                  border: '1px solid var(--border)',
                  borderRadius: '6px',
                  background: isLocalOnly ? 'rgba(245,158,11,0.04)' : 'transparent',
                }}
              >
                {/* 行头部 — WorkType 标题 + local-only 徽章 + 展开/折叠 */}
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '8px 12px',
                    cursor: 'pointer',
                    userSelect: 'none',
                  }}
                  onClick={() => toggleExpand(wt)}
                >
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <span style={{ fontWeight: 500, fontSize: '13px' }}>
                      <code style={{ color: 'var(--accent)' }}>{wt}</code>
                      {' · '}
                      {meta.label}
                    </span>
                    {isLocalOnly && (
                      <span
                        style={{
                          marginLeft: '8px',
                          fontSize: '10px',
                          padding: '2px 6px',
                          borderRadius: '3px',
                          background: 'rgba(245,158,11,0.2)',
                          color: '#f59e0b',
                          border: '1px solid rgba(245,158,11,0.4)',
                          fontWeight: 500,
                        }}
                      >
                        {t('workTypeConfig.localOnlyBadge')}
                      </span>
                    )}
                    <div
                      style={{ color: 'var(--text-secondary)', fontSize: '11px', marginTop: '2px' }}
                    >
                      {meta.desc}
                    </div>
                  </div>
                  <span style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>
                    {isExpanded ? '▼' : '▶'}
                  </span>
                </div>

                {/* 行展开内容 — provider / model / temperature / max_tokens + 测试按钮 */}
                {isExpanded && (
                  <div
                    style={{
                      padding: '8px 12px 12px',
                      borderTop: '1px dashed var(--border)',
                    }}
                  >
                    <div
                      style={{
                        display: 'grid',
                        gridTemplateColumns: '1fr 1fr 100px 110px auto',
                        gap: '8px',
                        alignItems: 'end',
                      }}
                      class="work-type-row-grid"
                    >
                      {/* Provider 下拉 */}
                      <div>
                        <label
                          style={{
                            display: 'block',
                            fontSize: '11px',
                            color: 'var(--text-secondary)',
                            marginBottom: '2px',
                          }}
                        >
                          {t('workTypeConfig.field.provider')}
                        </label>
                        <select
                          value={effectiveProvider}
                          disabled={isLocalOnly}
                          onChange={(e) => handleProviderChange(wt, e.currentTarget.value)}
                          style={{
                            width: '100%',
                            padding: '4px 6px',
                            fontSize: '12px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: isLocalOnly ? 'var(--bg-secondary)' : 'var(--bg-primary)',
                            color: 'var(--text-primary)',
                            cursor: isLocalOnly ? 'not-allowed' : 'pointer',
                          }}
                        >
                          <option value="">({t('workTypeConfig.field.providerDefault')})</option>
                          {config.providers.map((p) => (
                            <option key={p.id} value={p.id}>
                              {p.display_name} ({p.id})
                            </option>
                          ))}
                        </select>
                      </div>

                      {/* Model 下拉 */}
                      <div>
                        <label
                          style={{
                            display: 'block',
                            fontSize: '11px',
                            color: 'var(--text-secondary)',
                            marginBottom: '2px',
                          }}
                        >
                          {t('workTypeConfig.field.model')}
                        </label>
                        <select
                          value={row.model}
                          onChange={(e) => updateRow(wt, { model: e.currentTarget.value })}
                          style={{
                            width: '100%',
                            padding: '4px 6px',
                            fontSize: '12px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'var(--bg-primary)',
                            color: 'var(--text-primary)',
                          }}
                        >
                          <option value="">({t('workTypeConfig.field.modelDefault')})</option>
                          {modelOptions.map((m) => (
                            <option key={m.id} value={m.id}>
                              {m.display_name} ({m.id})
                            </option>
                          ))}
                          {/* model 不在列表中(可能 provider 切换后遗留) */}
                          {row.model && !modelOptions.some((m) => m.id === row.model) && (
                            <option value={row.model}>
                              {row.model} {t('workTypeConfig.field.modelStale')}
                            </option>
                          )}
                        </select>
                      </div>

                      {/* Temperature 输入 */}
                      <div>
                        <label
                          style={{
                            display: 'block',
                            fontSize: '11px',
                            color: 'var(--text-secondary)',
                            marginBottom: '2px',
                          }}
                        >
                          {t('workTypeConfig.field.temperature')}
                        </label>
                        <input
                          type="number"
                          step="0.1"
                          min="0"
                          max="2"
                          value={row.temperature}
                          onInput={(e) => updateRow(wt, { temperature: e.currentTarget.value })}
                          placeholder={t('workTypeConfig.field.temperaturePlaceholder')}
                          style={{
                            width: '100%',
                            padding: '4px 6px',
                            fontSize: '12px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'var(--bg-primary)',
                            color: 'var(--text-primary)',
                          }}
                        />
                      </div>

                      {/* Max tokens 输入 */}
                      <div>
                        <label
                          style={{
                            display: 'block',
                            fontSize: '11px',
                            color: 'var(--text-secondary)',
                            marginBottom: '2px',
                          }}
                        >
                          {t('workTypeConfig.field.maxTokens')}
                        </label>
                        <input
                          type="number"
                          step="1"
                          min="1"
                          value={row.max_tokens}
                          onInput={(e) => updateRow(wt, { max_tokens: e.currentTarget.value })}
                          placeholder={t('workTypeConfig.field.maxTokensPlaceholder')}
                          style={{
                            width: '100%',
                            padding: '4px 6px',
                            fontSize: '12px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'var(--bg-primary)',
                            color: 'var(--text-primary)',
                          }}
                        />
                      </div>

                      {/* Provider 测试按钮 + 结果 */}
                      <div>
                        <button
                          type="button"
                          onClick={() => effectiveProvider && handleTestProvider(effectiveProvider)}
                          disabled={!effectiveProvider || testState?.loading}
                          style={{
                            fontSize: '11px',
                            padding: '4px 10px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'transparent',
                            color: 'var(--text-primary)',
                            cursor:
                              !effectiveProvider || testState?.loading ? 'not-allowed' : 'pointer',
                            opacity: !effectiveProvider || testState?.loading ? 0.5 : 1,
                            whiteSpace: 'nowrap',
                          }}
                        >
                          {testState?.loading
                            ? t('workTypeConfig.testing')
                            : t('workTypeConfig.testButton')}
                        </button>
                        {testState?.result && (
                          <div
                            style={{
                              fontSize: '10px',
                              marginTop: '2px',
                              color: testState.result.ok ? '#10b981' : '#ef4444',
                            }}
                          >
                            {testState.result.ok ? '✓' : '✗'}{' '}
                            {testState.result.ok
                              ? t('workTypeConfig.testResult.ok', {
                                  latency: testState.result.latency_ms,
                                  status: testState.result.status_code ?? 'N/A',
                                })
                              : (testState.result.error ?? t('workTypeConfig.testResult.fail'))}
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
