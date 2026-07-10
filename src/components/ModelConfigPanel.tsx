/**
 * P0-1: 模型配置中心 — provider 列表 + 配置表单 + WorkType 路由 + 模型健康面板。
 *
 * ## 功能
 * - 左侧 Provider 列表(状态灯 🟢已配置Key / 🔴未配置Key / 🟡测试中)
 * - 右侧配置表单(名称 / base_url / API Key 显示切换 / 模型列表 / 测试连接 / 保存)
 * - WorkType 路由区(Chat / SwarmWorker / SwarmSynthesize / MasterTask)
 * - 模型健康面板(最近一次测试结果:延迟 / 状态)
 * - 添加 Provider 按钮(名称 + base_url + kind 选择)
 *
 * ## 集成
 * 从 Settings.tsx 的"模型配置中心"按钮触发,作为 Modal 弹出。
 *
 * ## 后端命令
 * - getProviderKeyStatus / setProviderKey(复用既有)
 * - testProviderConnection / discoverModels
 * - addCustomProvider / removeProvider
 * - setDefaultProvider / setWorktypeRouting
 * - modelsConfigLoad / modelsConfigReload(读取最新配置)
 */
import { useState, useEffect, useCallback } from 'preact/hooks';
import {
  nebulaAPI,
  type ModelsConfig,
  type ProviderConfig,
  type ConnectionTestResult,
  type ModelInfo,
  type WorkType,
} from '../lib/tauri';
import { Modal } from './Modal';
import { toast, toastFromError } from './Toast';

interface ModelConfigPanelProps {
  open: boolean;
  onClose: () => void;
}

/** Provider 健康状态(按 provider id 索引)。 */
interface HealthState {
  /** 是否正在测试连通性。 */
  testing: boolean;
  /** 最近一次连通性测试结果。 */
  result: ConnectionTestResult | null;
  /** Keychain 中是否存在该 provider 的 key。 */
  keyConfigured: boolean | null;
}

/** 可路由的 WorkType 列表(仅非 local_only 的 4 个)。 */
const ROUTABLE_WORK_TYPES: WorkType[] = [
  'chat',
  'swarm_worker',
  'swarm_synthesize',
  'master_task',
];

/** WorkType 中文标签。 */
const WORK_TYPE_LABELS: Record<string, string> = {
  chat: 'Chat 对话',
  swarm_worker: 'Swarm Worker',
  swarm_synthesize: 'Swarm Synthesize',
  master_task: 'Master Task',
};

/** 新增 provider 表单状态。 */
interface AddProviderForm {
  name: string;
  baseUrl: string;
  kind: string;
}

const DEFAULT_ADD_FORM: AddProviderForm = {
  name: '',
  baseUrl: '',
  kind: 'openai-compat',
};

/** Provider kind 选项。 */
const KIND_OPTIONS: { value: string; label: string }[] = [
  { value: 'openai-compat', label: 'OpenAI 兼容' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'ollama', label: 'Ollama(本地)' },
  { value: 'custom', label: '自定义' },
];

export function ModelConfigPanel({ open, onClose }: ModelConfigPanelProps) {
  const [config, setConfig] = useState<ModelsConfig | null>(null);
  const [loading, setLoading] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [healthMap, setHealthMap] = useState<Record<string, HealthState>>({});
  /** API Key 输入值(按 provider id 索引)。 */
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  /** 是否显示明文 Key(按 provider id 索引)。 */
  const [showKey, setShowKey] = useState<Record<string, boolean>>({});
  /** discover_models 返回的模型列表(按 provider id 索引)。 */
  const [discoveredModels, setDiscoveredModels] = useState<Record<string, ModelInfo[]>>({});
  /** 新增 provider 表单。 */
  const [addForm, setAddForm] = useState<AddProviderForm>(DEFAULT_ADD_FORM);
  const [adding, setAdding] = useState(false);
  const [savingKey, setSavingKey] = useState<string | null>(null);

  /** 加载 ModelsConfig + 各 provider 的 key 状态。 */
  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const cfg = await nebulaAPI.modelsConfigLoad();
      setConfig(cfg);
      // 默认选中第一个 provider。
      if (cfg.providers.length > 0 && !selectedId) {
        setSelectedId(cfg.providers[0].id);
      }
      // 并发查询各 provider 的 key 状态。
      const entries: Record<string, HealthState> = {};
      await Promise.all(
        cfg.providers.map(async (p) => {
          try {
            const ok = await nebulaAPI.getProviderKeyStatus(p.id);
            entries[p.id] = {
              testing: false,
              result: null,
              keyConfigured: ok,
            };
          } catch {
            entries[p.id] = { testing: false, result: null, keyConfigured: null };
          }
        })
      );
      setHealthMap(entries);
    } catch (err) {
      toastFromError(err);
    } finally {
      setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (open) {
      void reload();
    }
  }, [open, reload]);

  /** 当前选中的 provider 对象。 */
  const selectedProvider: ProviderConfig | null = (() => {
    if (!config || !selectedId) return null;
    return config.providers.find((p) => p.id === selectedId) ?? null;
  })();

  /** 测试 provider 连通性。 */
  const testConnection = useCallback(
    async (provider: ProviderConfig) => {
      setHealthMap((prev) => ({
        ...prev,
        [provider.id]: { ...prev[provider.id], testing: true },
      }));
      try {
        const baseUrl = provider.base_url ?? '';
        const apiKey = keyInputs[provider.id] ?? null;
        const result = await nebulaAPI.testProviderConnection(
          provider.id,
          baseUrl,
          apiKey
        );
        setHealthMap((prev) => ({
          ...prev,
          [provider.id]: { ...prev[provider.id], testing: false, result },
        }));
        if (result.success) {
          toast.success(`连通成功,延迟 ${result.latency_ms}ms`);
        } else {
          toast.error(`连通失败: ${result.error ?? '未知错误'}`);
        }
      } catch (err) {
        setHealthMap((prev) => ({
          ...prev,
          [provider.id]: { ...prev[provider.id], testing: false },
        }));
        toastFromError(err);
      }
    },
    [keyInputs]
  );

  /** 自动发现模型。 */
  const discoverModels = useCallback(
    async (provider: ProviderConfig) => {
      try {
        const baseUrl = provider.base_url ?? '';
        const apiKey = keyInputs[provider.id] ?? null;
        const models = await nebulaAPI.discoverModels(provider.id, baseUrl, apiKey);
        setDiscoveredModels((prev) => ({ ...prev, [provider.id]: models }));
        toast.success(`发现 ${models.length} 个模型`);
      } catch (err) {
        toastFromError(err);
      }
    },
    [keyInputs]
  );

  /** 保存 API Key 到 keychain。 */
  const saveApiKey = useCallback(
    async (providerId: string) => {
      const value = keyInputs[providerId]?.trim();
      if (!value) {
        toast.error('API Key 不能为空');
        return;
      }
      setSavingKey(providerId);
      try {
        await nebulaAPI.setProviderKey(providerId, value);
        setHealthMap((prev) => ({
          ...prev,
          [providerId]: { ...prev[providerId], keyConfigured: true },
        }));
        toast.success('API Key 已保存到 keychain');
      } catch (err) {
        toastFromError(err);
      } finally {
        setSavingKey(null);
      }
    },
    [keyInputs]
  );

  /** 设置默认 provider(使用该 provider 的第一个模型)。 */
  const setDefault = useCallback(
    async (provider: ProviderConfig) => {
      if (provider.models.length === 0) {
        toast.error('该 provider 没有模型,请先添加或发现模型');
        return;
      }
      try {
        await nebulaAPI.setDefaultProvider(provider.id, provider.models[0].id);
        await reload();
        toast.success(`已设置 ${provider.display_name} 为默认 provider`);
      } catch (err) {
        toastFromError(err);
      }
    },
    [reload]
  );

  /** 删除 provider。 */
  const removeProvider = useCallback(
    async (providerId: string) => {
      try {
        await nebulaAPI.removeProvider(providerId);
        if (selectedId === providerId) {
          setSelectedId(null);
        }
        await reload();
        toast.success('provider 已删除');
      } catch (err) {
        toastFromError(err);
      }
    },
    [reload, selectedId]
  );

  /** 设置 WorkType 路由。 */
  const setRouting = useCallback(
    async (workType: string, providerId: string) => {
      try {
        await nebulaAPI.setWorktypeRouting(workType, providerId);
        await reload();
        toast.success(`${WORK_TYPE_LABELS[workType] ?? workType} 路由已更新`);
      } catch (err) {
        toastFromError(err);
      }
    },
    [reload]
  );

  /** 添加自定义 provider。 */
  const addProvider = useCallback(async () => {
    const name = addForm.name.trim();
    if (!name) {
      toast.error('provider 名称不能为空');
      return;
    }
    setAdding(true);
    try {
      const id = await nebulaAPI.addCustomProvider(name, addForm.baseUrl.trim(), addForm.kind);
      setAddForm(DEFAULT_ADD_FORM);
      await reload();
      setSelectedId(id);
      toast.success(`provider ${name} 已添加`);
    } catch (err) {
      toastFromError(err);
    } finally {
      setAdding(false);
    }
  }, [addForm, reload]);

  /** 状态灯 emoji。 */
  function statusEmoji(providerId: string): string {
    const h = healthMap[providerId];
    if (!h) return '⚪';
    if (h.testing) return '🟡';
    if (h.keyConfigured === true) return '🟢';
    if (h.keyConfigured === false) return '🔴';
    return '⚪';
  }

  /** 切换 Key 显示/隐藏。 */
  const toggleShowKey = useCallback((providerId: string) => {
    setShowKey((prev) => ({ ...prev, [providerId]: !prev[providerId] }));
  }, []);

  if (!open) return null;

  return (
    <Modal open={open} title="模型配置中心" onClose={onClose} size="xl">
      {loading && !config ? (
        <div style={{ padding: '24px', textAlign: 'center', color: 'var(--text-secondary)' }}>
          加载中…
        </div>
      ) : !config ? (
        <div style={{ padding: '24px', textAlign: 'center', color: 'var(--accent-error)' }}>
          配置加载失败,请关闭后重试
        </div>
      ) : (
        <div style={{ display: 'flex', gap: '16px', minHeight: '480px' }}>
          {/* 左侧:Provider 列表 */}
          <div
            style={{
              width: '260px',
              flexShrink: 0,
              borderRight: '1px solid var(--border)',
              paddingRight: '12px',
              overflowY: 'auto',
            }}
          >
            <div
              style={{
                fontSize: '12px',
                color: 'var(--text-secondary)',
                marginBottom: '8px',
                textTransform: 'uppercase',
              }}
            >
              Provider 列表
            </div>
            {config.providers.map((p) => {
              const isDefault = config.default_provider === p.id;
              const isSelected = selectedId === p.id;
              return (
                <button
                  key={p.id}
                  type="button"
                  onClick={() => setSelectedId(p.id)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '8px',
                    width: '100%',
                    padding: '8px 10px',
                    marginBottom: '4px',
                    borderRadius: '6px',
                    border: `1px solid ${isSelected ? 'var(--accent)' : 'var(--border)'}`,
                    background: isSelected ? 'var(--bg-tertiary)' : 'transparent',
                    color: 'var(--text-primary)',
                    cursor: 'pointer',
                    textAlign: 'left',
                    fontSize: '13px',
                  }}
                >
                  <span style={{ fontSize: '14px' }}>{statusEmoji(p.id)}</span>
                  <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {p.display_name}
                    {isDefault && (
                      <span style={{ fontSize: '10px', marginLeft: '4px', color: 'var(--accent)' }}>
                        ✓
                      </span>
                    )}
                  </span>
                  {p.is_builtin && (
                    <span
                      style={{
                        fontSize: '9px',
                        padding: '1px 4px',
                        borderRadius: '3px',
                        background: 'var(--bg-tertiary)',
                        color: 'var(--text-secondary)',
                      }}
                    >
                      内置
                    </span>
                  )}
                </button>
              );
            })}

            {/* 添加 Provider 区 */}
            <div
              style={{
                marginTop: '12px',
                paddingTop: '12px',
                borderTop: '1px solid var(--border)',
              }}
            >
              <div style={{ fontSize: '12px', color: 'var(--text-secondary)', marginBottom: '6px' }}>
                添加 Provider
              </div>
              <input
                type="text"
                value={addForm.name}
                onInput={(e) => setAddForm((f) => ({ ...f, name: e.currentTarget.value }))}
                placeholder="名称(如 My vLLM)"
                style={{
                  width: '100%',
                  fontSize: '12px',
                  padding: '4px 8px',
                  marginBottom: '4px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'var(--bg-primary)',
                  color: 'var(--text-primary)',
                  boxSizing: 'border-box',
                }}
              />
              <input
                type="text"
                value={addForm.baseUrl}
                onInput={(e) => setAddForm((f) => ({ ...f, baseUrl: e.currentTarget.value }))}
                placeholder="base_url(如 http://127.0.0.1:8000/v1)"
                style={{
                  width: '100%',
                  fontSize: '12px',
                  padding: '4px 8px',
                  marginBottom: '4px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'var(--bg-primary)',
                  color: 'var(--text-primary)',
                  boxSizing: 'border-box',
                }}
              />
              <select
                value={addForm.kind}
                onChange={(e) => setAddForm((f) => ({ ...f, kind: e.currentTarget.value }))}
                style={{
                  width: '100%',
                  fontSize: '12px',
                  padding: '4px 8px',
                  marginBottom: '6px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'var(--bg-primary)',
                  color: 'var(--text-primary)',
                  boxSizing: 'border-box',
                }}
              >
                {KIND_OPTIONS.map((o) => (
                  <option value={o.value}>{o.label}</option>
                ))}
              </select>
              <button
                type="button"
                class="btn btn-small"
                onClick={addProvider}
                disabled={adding || !addForm.name.trim()}
                style={{ width: '100%', fontSize: '12px', padding: '4px 8px' }}
              >
                {adding ? '添加中…' : '+ 添加'}
              </button>
            </div>
          </div>

          {/* 右侧:配置表单 + 健康面板 + WorkType 路由 */}
          <div style={{ flex: 1, overflowY: 'auto', minWidth: 0 }}>
            {!selectedProvider ? (
              <div
                style={{
                  padding: '48px',
                  textAlign: 'center',
                  color: 'var(--text-secondary)',
                }}
              >
                ← 请在左侧选择一个 provider
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
                {/* 配置表单 */}
                <div>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'space-between',
                      marginBottom: '8px',
                    }}
                  >
                    <h4 style={{ margin: 0, fontSize: '15px' }}>
                      {selectedProvider.display_name}
                      <code
                        style={{
                          marginLeft: '8px',
                          fontSize: '11px',
                          color: 'var(--text-secondary)',
                        }}
                      >
                        {selectedProvider.id}
                      </code>
                    </h4>
                    <div style={{ display: 'flex', gap: '6px' }}>
                      {!selectedProvider.is_builtin && (
                        <button
                          type="button"
                          class="btn btn-danger btn-small"
                          onClick={() => removeProvider(selectedProvider.id)}
                          disabled={config.default_provider === selectedProvider.id}
                          style={{ fontSize: '11px', padding: '3px 10px' }}
                        >
                          删除
                        </button>
                      )}
                    </div>
                  </div>

                  {/* 基本信息 */}
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                    <label style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>
                      类型
                      <input
                        type="text"
                        value={selectedProvider.kind}
                        disabled
                        style={{
                          width: '100%',
                          fontSize: '12px',
                          padding: '4px 8px',
                          marginTop: '2px',
                          borderRadius: '4px',
                          border: '1px solid var(--border)',
                          background: 'var(--bg-primary)',
                          color: 'var(--text-secondary)',
                          boxSizing: 'border-box',
                        }}
                      />
                    </label>
                    <label style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>
                      Base URL
                      <input
                        type="text"
                        value={selectedProvider.base_url ?? ''}
                        disabled
                        style={{
                          width: '100%',
                          fontSize: '12px',
                          padding: '4px 8px',
                          marginTop: '2px',
                          borderRadius: '4px',
                          border: '1px solid var(--border)',
                          background: 'var(--bg-primary)',
                          color: 'var(--text-secondary)',
                          boxSizing: 'border-box',
                        }}
                      />
                    </label>
                  </div>

                  {/* API Key 输入 */}
                  <div style={{ marginTop: '10px' }}>
                    <label
                      style={{
                        display: 'block',
                        fontSize: '12px',
                        color: 'var(--text-secondary)',
                        marginBottom: '4px',
                      }}
                    >
                      API Key
                      {healthMap[selectedProvider.id]?.keyConfigured === true && (
                        <span
                          style={{
                            marginLeft: '6px',
                            fontSize: '10px',
                            color: 'var(--accent)',
                          }}
                        >
                          ✓ 已配置
                        </span>
                      )}
                    </label>
                    <div style={{ display: 'flex', gap: '6px' }}>
                      <input
                        type={showKey[selectedProvider.id] ? 'text' : 'password'}
                        value={keyInputs[selectedProvider.id] ?? ''}
                        onInput={(e) =>
                          setKeyInputs((prev) => ({
                            ...prev,
                            [selectedProvider.id]: e.currentTarget.value,
                          }))
                        }
                        autocomplete="off"
                        spellcheck={false}
                        placeholder={
                          healthMap[selectedProvider.id]?.keyConfigured
                            ? '已保存,输入新值覆盖'
                            : '输入 API Key'
                        }
                        style={{
                          flex: 1,
                          fontSize: '12px',
                          padding: '4px 8px',
                          borderRadius: '4px',
                          border: '1px solid var(--border)',
                          background: 'var(--bg-primary)',
                          color: 'var(--text-primary)',
                        }}
                      />
                      <button
                        type="button"
                        onClick={() => toggleShowKey(selectedProvider.id)}
                        style={{
                          fontSize: '11px',
                          padding: '4px 8px',
                          borderRadius: '4px',
                          border: '1px solid var(--border)',
                          background: 'transparent',
                          color: 'var(--text-secondary)',
                          cursor: 'pointer',
                          flexShrink: 0,
                        }}
                      >
                        {showKey[selectedProvider.id] ? '隐藏' : '显示'}
                      </button>
                      <button
                        type="button"
                        class="btn btn-small"
                        onClick={() => saveApiKey(selectedProvider.id)}
                        disabled={savingKey === selectedProvider.id}
                        style={{ fontSize: '11px', padding: '4px 10px', flexShrink: 0 }}
                      >
                        {savingKey === selectedProvider.id ? '保存中…' : '保存 Key'}
                      </button>
                    </div>
                  </div>

                  {/* 操作按钮 */}
                  <div
                    style={{
                      display: 'flex',
                      gap: '6px',
                      marginTop: '10px',
                      flexWrap: 'wrap',
                    }}
                  >
                    <button
                      type="button"
                      class="btn btn-small"
                      onClick={() => testConnection(selectedProvider)}
                      disabled={healthMap[selectedProvider.id]?.testing}
                      style={{ fontSize: '11px', padding: '4px 10px' }}
                    >
                      {healthMap[selectedProvider.id]?.testing ? '测试中…' : '测试连接'}
                    </button>
                    <button
                      type="button"
                      class="btn btn-small"
                      onClick={() => discoverModels(selectedProvider)}
                      style={{ fontSize: '11px', padding: '4px 10px' }}
                    >
                      发现模型
                    </button>
                    {config.default_provider !== selectedProvider.id && (
                      <button
                        type="button"
                        class="btn btn-small"
                        onClick={() => setDefault(selectedProvider)}
                        style={{ fontSize: '11px', padding: '4px 10px' }}
                      >
                        设为默认
                      </button>
                    )}
                  </div>

                  {/* 健康面板 */}
                  {(() => {
                    const result = healthMap[selectedProvider.id]?.result;
                    if (!result) return null;
                    return (
                      <div
                        style={{
                          marginTop: '10px',
                          padding: '8px 12px',
                          borderRadius: '6px',
                          border: '1px solid var(--border)',
                          background: 'var(--bg-primary)',
                          fontSize: '12px',
                        }}
                      >
                        <div style={{ fontWeight: 500, marginBottom: '4px' }}>
                          模型健康面板
                        </div>
                        <div style={{ color: 'var(--text-secondary)' }}>
                          状态:{' '}
                          <span
                            style={{
                              color: result.success
                                ? 'var(--accent)'
                                : 'var(--accent-error)',
                            }}
                          >
                            {result.success ? '✓ 可达' : '✗ 不可达'}
                          </span>
                          {' · 延迟 '}
                          {result.latency_ms}ms
                          {result.error && (
                            <>
                              {' · 错误: '}
                              {result.error}
                            </>
                          )}
                        </div>
                      </div>
                    );
                  })()}
                </div>

                {/* 模型列表 */}
                <div>
                  <div
                    style={{
                      fontSize: '12px',
                      color: 'var(--text-secondary)',
                      marginBottom: '6px',
                      textTransform: 'uppercase',
                    }}
                  >
                    模型列表({selectedProvider.models.length})
                  </div>
                  {selectedProvider.models.length === 0 ? (
                    <div
                      style={{
                        padding: '12px',
                        textAlign: 'center',
                        color: 'var(--text-muted)',
                        fontSize: '12px',
                        border: '1px dashed var(--border)',
                        borderRadius: '6px',
                      }}
                    >
                      暂无模型,点击"发现模型"自动拉取
                    </div>
                  ) : (
                    <div
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '4px',
                      }}
                    >
                      {selectedProvider.models.map((m) => (
                        <div
                          key={m.id}
                          style={{
                            display: 'flex',
                            justifyContent: 'space-between',
                            alignItems: 'center',
                            padding: '6px 10px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'var(--bg-primary)',
                            fontSize: '12px',
                          }}
                        >
                          <span>
                            {m.display_name}
                            <code
                              style={{
                                marginLeft: '6px',
                                fontSize: '10px',
                                color: 'var(--text-secondary)',
                              }}
                            >
                              {m.id}
                            </code>
                          </span>
                          <span style={{ color: 'var(--text-muted)', fontSize: '11px' }}>
                            {m.context_window ? `${m.context_window} ctx` : ''}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                  {/* 发现的模型(未添加到配置的) */}
                  {discoveredModels[selectedProvider.id]?.length ? (
                    <div style={{ marginTop: '8px' }}>
                      <div
                        style={{
                          fontSize: '11px',
                          color: 'var(--text-muted)',
                          marginBottom: '4px',
                        }}
                      >
                        发现的模型({discoveredModels[selectedProvider.id]!.length}):
                      </div>
                      <div
                        style={{
                          display: 'flex',
                          flexWrap: 'wrap',
                          gap: '4px',
                        }}
                      >
                        {discoveredModels[selectedProvider.id]!.map((m) => (
                          <span
                            key={m.id}
                            style={{
                              fontSize: '11px',
                              padding: '2px 6px',
                              borderRadius: '3px',
                              background: 'var(--bg-tertiary)',
                              color: 'var(--text-secondary)',
                            }}
                          >
                            {m.id}
                            {m.context_length ? ` (${m.context_length})` : ''}
                          </span>
                        ))}
                      </div>
                    </div>
                  ) : null}
                </div>

                {/* WorkType 路由区 */}
                <div>
                  <div
                    style={{
                      fontSize: '12px',
                      color: 'var(--text-secondary)',
                      marginBottom: '6px',
                      textTransform: 'uppercase',
                    }}
                  >
                    WorkType 路由
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                    {ROUTABLE_WORK_TYPES.map((wt) => {
                      const override = config.work_type_overrides?.[wt];
                      const currentProvider = override?.provider ?? config.default_provider;
                      const currentModel = override?.model ?? config.default_model;
                      return (
                        <div
                          key={wt}
                          style={{
                            display: 'flex',
                            alignItems: 'center',
                            gap: '8px',
                            padding: '6px 10px',
                            borderRadius: '4px',
                            border: '1px solid var(--border)',
                            background: 'var(--bg-primary)',
                            fontSize: '12px',
                          }}
                        >
                          <span style={{ minWidth: '140px', color: 'var(--text-secondary)' }}>
                            {WORK_TYPE_LABELS[wt] ?? wt}
                          </span>
                          <span style={{ flex: 1, color: 'var(--text-muted)' }}>
                            {currentProvider} / {currentModel}
                          </span>
                          <button
                            type="button"
                            class="btn btn-small"
                            onClick={() => setRouting(wt, selectedProvider.id)}
                            disabled={currentProvider === selectedProvider.id}
                            style={{ fontSize: '11px', padding: '2px 8px' }}
                          >
                            {currentProvider === selectedProvider.id ? '当前' : '路由到此'}
                          </button>
                        </div>
                      );
                    })}
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}
