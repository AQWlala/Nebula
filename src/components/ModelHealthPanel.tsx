/**
 * P1-1: 模型健康面板 — 展示所有 provider 的延迟/成本/命中率/断路器状态。
 *
 * ## 功能
 * - 表格展示所有 provider 的健康指标(延迟 / 今日费用 / 当月费用 / 缓存命中率 /
 *   今日请求次数 / 断路器状态 / 最近错误 / 最近请求时间)
 * - 状态灯:🟢 已配置 Key / 🔴 未配置 Key
 * - 延迟颜色:绿 <500ms / 黄 <2000ms / 红 ≥2000ms
 * - 断路器:🟢 Closed(正常) / 🔴 Open(熔断) / 🟡 HalfOpen(半开试探)
 * - 刷新按钮 + 30 秒自动刷新开关
 * - 费用预警:今日费用超过阈值时整行高亮黄色背景
 *
 * ## 数据来源
 * 调用 `nebulaAPI.getModelHealth()` → 后端 `get_model_health` Tauri 命令,
 * 聚合 ModelHealthTracker / CostTracker / metrics snapshot 数据。
 *
 * ## 集成
 * 在 ModelConfigPanel 底部渲染,独立管理刷新状态,不依赖父组件。
 */
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { nebulaAPI, type ModelHealthInfo } from '../lib/tauri';

/** 自动刷新间隔(毫秒)。 */
const AUTO_REFRESH_INTERVAL_MS = 30_000;

/**
 * 费用预警阈值(USD)。
 *
 * 后端 ModelHealthInfo 尚未携带 per-provider 日预算字段,此处用固定阈值
 * 作为简化预警。当今日费用达到此值时整行高亮黄色。
 * 后续若后端新增 daily_budget 字段,可改为按预算 80% 判断。
 */
const COST_WARN_USD = 1.0;

/** 延迟颜色档位阈值(毫秒)。 */
const LATENCY_GOOD_MS = 500;
const LATENCY_WARN_MS = 2000;

/** 延迟文字颜色。 */
function latencyColor(ms: number | null): string {
  if (ms === null) return 'var(--text-muted)';
  if (ms < LATENCY_GOOD_MS) return '#22c55e';
  if (ms < LATENCY_WARN_MS) return '#ffc107';
  return 'var(--accent-error)';
}

/** 断路器状态对应的 emoji + 颜色。 */
function circuitBreakerDisplay(status: string): { emoji: string; color: string } {
  switch (status) {
    case 'Closed':
      return { emoji: '🟢', color: '#22c55e' };
    case 'Open':
      return { emoji: '🔴', color: 'var(--accent-error)' };
    case 'HalfOpen':
      return { emoji: '🟡', color: '#ffc107' };
    default:
      return { emoji: '⚪', color: 'var(--text-muted)' };
  }
}

/** 格式化费用(USD),保留 4 位小数,不足时显示更少。 */
function formatCost(usd: number): string {
  if (usd === 0) return '$0';
  if (usd < 0.01) return '<$0.01';
  return `$${usd.toFixed(4)}`;
}

/** 格式化缓存命中率为百分比。 */
function formatRate(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}

/** 将 Unix 时间戳(秒)格式化为简短的可读时间。 */
function formatTimestamp(ts: number | null): string {
  if (ts === null) return '—';
  const date = new Date(ts * 1000);
  const now = Date.now();
  const diffSec = Math.floor((now - date.getTime()) / 1000);
  if (diffSec < 60) return `${diffSec}秒前`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}分钟前`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}小时前`;
  return date.toLocaleString();
}

export function ModelHealthPanel() {
  const [healthData, setHealthData] = useState<ModelHealthInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [autoRefresh, setAutoRefresh] = useState(false);
  /** 防止并发刷新(手动 + 自动同时触发)。 */
  const refreshingRef = useRef(false);

  /** 拉取健康数据。 */
  const refresh = useCallback(async () => {
    if (refreshingRef.current) return;
    refreshingRef.current = true;
    setLoading(true);
    setError(null);
    try {
      const data = await nebulaAPI.getModelHealth();
      setHealthData(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
      refreshingRef.current = false;
    }
  }, []);

  /** 首次挂载时拉取一次。 */
  useEffect(() => {
    void refresh();
  }, [refresh]);

  /** 自动刷新:每 30 秒拉取一次(仅在开关开启时)。 */
  useEffect(() => {
    if (!autoRefresh) return;
    const timer = setInterval(() => {
      void refresh();
    }, AUTO_REFRESH_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [autoRefresh, refresh]);

  return (
    <div
      style={{
        marginTop: '16px',
        paddingTop: '16px',
        borderTop: '1px solid var(--border)',
      }}
    >
      {/* 标题栏 + 操作按钮 */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: '8px',
        }}
      >
        <div style={{ fontSize: '12px', color: 'var(--text-secondary)', textTransform: 'uppercase' }}>
          模型健康面板
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <label
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '4px',
              fontSize: '11px',
              color: 'var(--text-secondary)',
              cursor: 'pointer',
            }}
          >
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.currentTarget.checked)}
              style={{ cursor: 'pointer' }}
            />
            自动刷新(30s)
          </label>
          <button
            type="button"
            class="btn btn-small"
            onClick={() => void refresh()}
            disabled={loading}
            style={{ fontSize: '11px', padding: '4px 10px' }}
          >
            {loading ? '刷新中…' : '刷新'}
          </button>
        </div>
      </div>

      {/* 错误提示 */}
      {error && (
        <div
          style={{
            padding: '8px 12px',
            borderRadius: '6px',
            border: '1px solid var(--accent-error)',
            background: 'rgba(239, 68, 68, 0.1)',
            color: 'var(--accent-error)',
            fontSize: '12px',
            marginBottom: '8px',
          }}
        >
          加载失败:{error}
        </div>
      )}

      {/* 健康指标表格 */}
      {healthData.length === 0 && !loading && !error ? (
        <div
          style={{
            padding: '16px',
            textAlign: 'center',
            color: 'var(--text-muted)',
            fontSize: '12px',
            border: '1px dashed var(--border)',
            borderRadius: '6px',
          }}
        >
          暂无 provider 健康数据
        </div>
      ) : (
        <div
          style={{
            borderRadius: '6px',
            border: '1px solid var(--border)',
            background: 'var(--bg-primary)',
            overflowX: 'auto',
          }}
        >
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: '12px',
            }}
          >
            <thead>
              <tr
                style={{
                  borderBottom: '1px solid var(--border)',
                  color: 'var(--text-secondary)',
                  textAlign: 'left',
                }}
              >
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  Provider
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  状态
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  延迟
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  今日费用
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  当月费用
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  缓存命中率
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  今日请求
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  断路器
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  最近错误
                </th>
                <th style={{ padding: '6px 10px', fontWeight: 500, whiteSpace: 'nowrap' }}>
                  最近请求
                </th>
              </tr>
            </thead>
            <tbody>
              {healthData.map((h) => {
                const cb = circuitBreakerDisplay(h.circuit_breaker_status);
                const costWarn = h.cost_today_usd >= COST_WARN_USD;
                return (
                  <tr
                    key={h.provider_id}
                    style={{
                      borderBottom: '1px solid var(--border)',
                      background: costWarn
                        ? 'rgba(255, 193, 7, 0.12)'
                        : 'transparent',
                    }}
                  >
                    {/* Provider 名称 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: 'var(--text-primary)',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {h.provider_name}
                      <span style={{ fontSize: '10px', color: 'var(--text-muted)', marginLeft: '4px' }}>
                        {h.provider_kind}
                      </span>
                    </td>
                    {/* 配置状态灯 */}
                    <td style={{ padding: '6px 10px', whiteSpace: 'nowrap' }}>
                      <span title={h.is_configured ? '已配置' : '未配置'}>
                        {h.is_configured ? '🟢' : '🔴'}
                      </span>
                    </td>
                    {/* 延迟 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: latencyColor(h.latency_ms),
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {h.latency_ms !== null ? `${h.latency_ms}ms` : '—'}
                    </td>
                    {/* 今日费用 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: costWarn ? '#ffc107' : 'var(--text-primary)',
                        fontWeight: costWarn ? 600 : 400,
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {formatCost(h.cost_today_usd)}
                      {costWarn && ' ⚠'}
                    </td>
                    {/* 当月费用 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: 'var(--text-secondary)',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {formatCost(h.cost_month_usd)}
                    </td>
                    {/* 缓存命中率 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: 'var(--text-secondary)',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {formatRate(h.cache_hit_rate)}
                    </td>
                    {/* 今日请求次数 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: 'var(--text-secondary)',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {h.request_count_today}
                    </td>
                    {/* 断路器状态 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: cb.color,
                        whiteSpace: 'nowrap',
                      }}
                    >
                      <span title={h.circuit_breaker_status}>
                        {cb.emoji} {h.circuit_breaker_status}
                      </span>
                    </td>
                    {/* 最近错误 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: h.last_error ? 'var(--accent-error)' : 'var(--text-muted)',
                        maxWidth: '200px',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                      title={h.last_error ?? undefined}
                    >
                      {h.last_error ?? '—'}
                    </td>
                    {/* 最近请求时间 */}
                    <td
                      style={{
                        padding: '6px 10px',
                        color: 'var(--text-muted)',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {formatTimestamp(h.last_request_at)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
