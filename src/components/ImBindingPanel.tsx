/**
 * T-E-C-17: IM 绑定面板 — Settings 内嵌 section。
 *
 * Phase 1 Webhook 优先:支持 Feishu/WeCom/DingTalk 三平台 webhook 绑定的
 * - 列表展示(平台 / 类型 / 显示名 / 启用状态 / 上次使用时间)
 * - 创建(粘贴 webhook URL + 选平台 + 输入显示名 + 创建按钮)
 * - 启用/禁用切换
 * - 单条测试发送(输入 title + body,调用 im_test_send)
 * - 删除(幂等)
 * - 全量广播(输入 title + body,调用 im_broadcast,展示 success/failure 计数)
 *
 * 后端命令经 `nebulaAPI` 静态方法调用,失败时 throw CommandError,
 * 本组件 catch 后展示在 `errorText` 区域。
 */
import { useEffect, useState } from 'preact/hooks';
import { nebulaAPI, type ImBinding, type ImPlatform, type ImBroadcastResult } from '../lib/tauri';
import { t } from '../i18n';

/** 平台选项(供 <select> 渲染)。 */
const PLATFORM_OPTIONS: { value: ImPlatform }[] = [
  { value: 'feishu' },
  { value: 'wecom' },
  { value: 'dingtalk' },
];

/** i18n platform label lookup: convert const Record field to function form. */
function platformLabel(p: ImPlatform): string {
  return t(`imBinding.platform.${p}`);
}

/** 格式化 Unix 毫秒时间戳为本地可读字符串。 */
function formatTs(ts: number | null): string {
  if (ts == null) return '—';
  return new Date(ts).toLocaleString();
}

/** 截断长 URL 用于列表展示(避免溢出)。 */
function truncateUrl(url: string, max = 60): string {
  if (url.length <= max) return url;
  return url.slice(0, max - 3) + '...';
}

/**
 * T-E-C-17: IM 绑定面板。Settings 在 persona section 之后渲染此组件。
 */
export function ImBindingPanel() {
  // 绑定列表。
  const [bindings, setBindings] = useState<ImBinding[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorText, setErrorText] = useState('');
  // 创建表单。
  const [newPlatform, setNewPlatform] = useState<ImPlatform>('feishu');
  const [newUrl, setNewUrl] = useState('');
  const [newDisplayName, setNewDisplayName] = useState('');
  const [creating, setCreating] = useState(false);
  // 测试发送表单(按 binding id 索引)。
  const [testTitle, setTestTitle] = useState('');
  const [testBody, setTestBody] = useState('');
  const [testingId, setTestingId] = useState<string | null>(null);
  // 广播表单。
  const [broadcastTitle, setBroadcastTitle] = useState('');
  const [broadcastBody, setBroadcastBody] = useState('');
  const [broadcasting, setBroadcasting] = useState(false);
  const [broadcastResult, setBroadcastResult] = useState<ImBroadcastResult | null>(null);

  /** 拉取绑定列表。 */
  async function refresh() {
    setLoading(true);
    setErrorText('');
    try {
      const list = await nebulaAPI.imListBindings();
      setBindings(list);
    } catch (e) {
      setErrorText(
        t('imBinding.loadFailed', { error: e instanceof Error ? e.message : String(e) })
      );
    } finally {
      setLoading(false);
    }
  }

  // 首次挂载时拉取一次。
  useEffect(() => {
    refresh();
  }, []);

  /** 创建 webhook 绑定。 */
  async function handleCreate(e: Event) {
    e.preventDefault();
    if (!newUrl.trim()) {
      setErrorText(t('imBinding.urlRequired'));
      return;
    }
    setCreating(true);
    setErrorText('');
    try {
      await nebulaAPI.imCreateWebhookBinding({
        platform: newPlatform,
        url: newUrl.trim(),
        display_name: newDisplayName.trim(),
      });
      setNewUrl('');
      setNewDisplayName('');
      await refresh();
    } catch (e) {
      setErrorText(
        t('imBinding.createFailed', { error: e instanceof Error ? e.message : String(e) })
      );
    } finally {
      setCreating(false);
    }
  }

  /** 切换启用状态。 */
  async function handleToggle(id: string, currentEnabled: boolean) {
    setErrorText('');
    try {
      await nebulaAPI.imSetEnabled(id, !currentEnabled);
      // 本地更新,避免重新拉取全表。
      setBindings((prev) =>
        prev.map((b) => (b.id === id ? { ...b, enabled: !currentEnabled } : b))
      );
    } catch (e) {
      setErrorText(
        t('imBinding.toggleFailed', { error: e instanceof Error ? e.message : String(e) })
      );
    }
  }

  /** 删除绑定。 */
  async function handleDelete(id: string) {
    if (!confirm(t('imBinding.confirmDelete'))) return;
    setErrorText('');
    try {
      await nebulaAPI.imDeleteBinding(id);
      setBindings((prev) => prev.filter((b) => b.id !== id));
    } catch (e) {
      setErrorText(
        t('imBinding.deleteFailed', { error: e instanceof Error ? e.message : String(e) })
      );
    }
  }

  /** 单条测试发送。 */
  async function handleTestSend(id: string) {
    if (!testTitle.trim() || !testBody.trim()) {
      setErrorText(t('imBinding.testRequired'));
      return;
    }
    setTestingId(id);
    setErrorText('');
    try {
      await nebulaAPI.imTestSend(id, testTitle.trim(), testBody.trim());
      // 测试发送成功后刷新列表(更新 last_used_at)。
      await refresh();
    } catch (e) {
      setErrorText(
        t('imBinding.testFailed', { error: e instanceof Error ? e.message : String(e) })
      );
    } finally {
      setTestingId(null);
    }
  }

  /** 全量广播。 */
  async function handleBroadcast(e: Event) {
    e.preventDefault();
    if (!broadcastTitle.trim() || !broadcastBody.trim()) {
      setErrorText(t('imBinding.broadcastRequired'));
      return;
    }
    setBroadcasting(true);
    setErrorText('');
    setBroadcastResult(null);
    try {
      const result = await nebulaAPI.imBroadcast({
        title: broadcastTitle.trim(),
        body: broadcastBody.trim(),
      });
      setBroadcastResult(result);
    } catch (e) {
      setErrorText(
        t('imBinding.broadcastFailed', { error: e instanceof Error ? e.message : String(e) })
      );
    } finally {
      setBroadcasting(false);
    }
  }

  return (
    <div class="card" style="margin-top: 16px;">
      <h3 style="margin-bottom: 4px;">{t('imBinding.title')}</h3>
      <div style="color: var(--text-secondary); font-size: 11px; margin-bottom: 12px;">
        {t('imBinding.hint')}
      </div>

      {errorText && (
        <div
          style={{
            color: 'var(--danger, #e53935)',
            fontSize: '12px',
            marginBottom: '8px',
            padding: '4px 8px',
            background: 'rgba(229, 57, 53, 0.08)',
            borderRadius: '4px',
          }}
        >
          {errorText}
        </div>
      )}

      {/* 创建表单 */}
      <form
        onSubmit={handleCreate}
        style={{ display: 'flex', flexDirection: 'column', gap: '6px', marginBottom: '12px' }}
      >
        <div style={{ display: 'flex', gap: '6px', flexWrap: 'wrap' }}>
          <select
            value={newPlatform}
            onChange={(e) =>
              setNewPlatform((e.currentTarget as HTMLSelectElement).value as ImPlatform)
            }
            style={{ flex: '0 0 140px', padding: '4px 6px', fontSize: '12px' }}
          >
            {PLATFORM_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {platformLabel(o.value)}
              </option>
            ))}
          </select>
          <input
            type="text"
            placeholder={t('imBinding.displayNamePlaceholder')}
            value={newDisplayName}
            onInput={(e) => setNewDisplayName((e.currentTarget as HTMLInputElement).value)}
            style={{ flex: '1 1 160px', padding: '4px 6px', fontSize: '12px' }}
          />
        </div>
        <div style={{ display: 'flex', gap: '6px' }}>
          <input
            type="url"
            placeholder={t('imBinding.urlPlaceholder')}
            value={newUrl}
            onInput={(e) => setNewUrl((e.currentTarget as HTMLInputElement).value)}
            style={{ flex: '1 1 auto', padding: '4px 6px', fontSize: '12px' }}
          />
          <button
            type="submit"
            disabled={creating}
            style={{
              flex: '0 0 auto',
              padding: '4px 12px',
              fontSize: '12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'var(--accent)',
              color: '#fff',
              cursor: creating ? 'wait' : 'pointer',
              opacity: creating ? 0.6 : 1,
            }}
          >
            {creating ? t('imBinding.creating') : t('imBinding.createButton')}
          </button>
        </div>
      </form>

      {/* 绑定列表 */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: '6px', marginBottom: '12px' }}>
        {loading && bindings.length === 0 && (
          <div style={{ fontSize: '12px', color: 'var(--text-secondary)', padding: '8px' }}>
            {t('imBinding.loading')}
          </div>
        )}
        {!loading && bindings.length === 0 && (
          <div style={{ fontSize: '12px', color: 'var(--text-secondary)', padding: '8px' }}>
            {t('imBinding.empty')}
          </div>
        )}
        {bindings.map((b) => (
          <div
            key={b.id}
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: '4px',
              padding: '8px',
              borderRadius: '6px',
              border: '1px solid var(--border)',
              background: 'var(--bg-secondary, rgba(255,255,255,0.02))',
            }}
          >
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: '8px',
              }}
            >
              <span style={{ fontSize: '12px', fontWeight: 600 }}>
                {platformLabel(b.platform)}
                {b.display_name && (
                  <span
                    style={{ color: 'var(--text-secondary)', fontWeight: 400, marginLeft: '6px' }}
                  >
                    {b.display_name}
                  </span>
                )}
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                <button
                  type="button"
                  onClick={() => handleToggle(b.id, b.enabled)}
                  style={{
                    fontSize: '11px',
                    padding: '2px 8px',
                    borderRadius: '4px',
                    border: '1px solid var(--border)',
                    background: b.enabled ? 'var(--accent)' : 'transparent',
                    color: b.enabled ? '#fff' : 'var(--text-secondary)',
                    cursor: 'pointer',
                  }}
                >
                  {b.enabled ? t('imBinding.enabled') : t('imBinding.disabled')}
                </button>
                <button
                  type="button"
                  onClick={() => handleDelete(b.id)}
                  style={{
                    fontSize: '11px',
                    padding: '2px 8px',
                    borderRadius: '4px',
                    border: '1px solid var(--border)',
                    background: 'transparent',
                    color: 'var(--danger, #e53935)',
                    cursor: 'pointer',
                  }}
                >
                  {t('imBinding.delete')}
                </button>
              </span>
            </div>
            {b.kind.kind === 'webhook' && (
              <div
                style={{ fontSize: '11px', color: 'var(--text-secondary)', wordBreak: 'break-all' }}
              >
                {truncateUrl(b.kind.url)}
              </div>
            )}
            <div style={{ fontSize: '11px', color: 'var(--text-secondary)' }}>
              {t('imBinding.lastUsed', { time: formatTs(b.last_used_at) })} ·{' '}
              {t('imBinding.created', { time: formatTs(b.created_at) })}
            </div>
          </div>
        ))}
      </div>

      {/* 单条测试发送 */}
      <div
        style={{ borderTop: '1px solid var(--border)', paddingTop: '10px', marginBottom: '12px' }}
      >
        <div style={{ fontSize: '12px', fontWeight: 600, marginBottom: '6px' }}>
          {t('imBinding.testSendTitle')}
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
          <input
            type="text"
            placeholder={t('imBinding.testTitlePlaceholder')}
            value={testTitle}
            onInput={(e) => setTestTitle((e.currentTarget as HTMLInputElement).value)}
            style={{ padding: '4px 6px', fontSize: '12px' }}
          />
          <input
            type="text"
            placeholder={t('imBinding.testBodyPlaceholder')}
            value={testBody}
            onInput={(e) => setTestBody((e.currentTarget as HTMLInputElement).value)}
            style={{ padding: '4px 6px', fontSize: '12px' }}
          />
          <div style={{ display: 'flex', gap: '6px', flexWrap: 'wrap' }}>
            {bindings.length === 0 && (
              <span style={{ fontSize: '11px', color: 'var(--text-secondary)' }}>
                {t('imBinding.noBindingsToTest')}
              </span>
            )}
            {bindings.map((b) => (
              <button
                key={b.id}
                type="button"
                disabled={testingId !== null}
                onClick={() => handleTestSend(b.id)}
                style={{
                  fontSize: '11px',
                  padding: '2px 10px',
                  borderRadius: '4px',
                  border: '1px solid var(--border)',
                  background: 'transparent',
                  color: 'var(--text-primary)',
                  cursor: testingId === b.id ? 'wait' : 'pointer',
                  opacity: testingId !== null && testingId !== b.id ? 0.4 : 1,
                }}
              >
                {testingId === b.id
                  ? t('imBinding.sending')
                  : t('imBinding.testButton', { platform: platformLabel(b.platform) })}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* 全量广播 */}
      <div style={{ borderTop: '1px solid var(--border)', paddingTop: '10px' }}>
        <div style={{ fontSize: '12px', fontWeight: 600, marginBottom: '6px' }}>
          {t('imBinding.broadcastTitle')}
        </div>
        <form
          onSubmit={handleBroadcast}
          style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}
        >
          <input
            type="text"
            placeholder={t('imBinding.broadcastTitlePlaceholder')}
            value={broadcastTitle}
            onInput={(e) => setBroadcastTitle((e.currentTarget as HTMLInputElement).value)}
            style={{ padding: '4px 6px', fontSize: '12px' }}
          />
          <input
            type="text"
            placeholder={t('imBinding.broadcastBodyPlaceholder')}
            value={broadcastBody}
            onInput={(e) => setBroadcastBody((e.currentTarget as HTMLInputElement).value)}
            style={{ padding: '4px 6px', fontSize: '12px' }}
          />
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <button
              type="submit"
              disabled={broadcasting}
              style={{
                padding: '4px 12px',
                fontSize: '12px',
                borderRadius: '4px',
                border: '1px solid var(--border)',
                background: 'var(--accent)',
                color: '#fff',
                cursor: broadcasting ? 'wait' : 'pointer',
                opacity: broadcasting ? 0.6 : 1,
              }}
            >
              {broadcasting ? t('imBinding.broadcasting') : t('imBinding.broadcastButton')}
            </button>
            {broadcastResult && (
              <span style={{ fontSize: '11px', color: 'var(--text-secondary)' }}>
                {t('imBinding.broadcastResult', {
                  success: broadcastResult.success,
                  failure: broadcastResult.failure,
                })}
              </span>
            )}
          </div>
        </form>
      </div>
    </div>
  );
}
