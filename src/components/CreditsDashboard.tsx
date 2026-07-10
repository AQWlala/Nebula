/** T-E-A-07: Credits Dashboard — 日/周/月趋势图 + provider/agent 分桶 + 预算预警 + 缓存命中率。 */

import { useEffect, useRef, useState } from 'preact/hooks';
import { invoke } from '@tauri-apps/api/core';
import { Sparkline } from './charts/Sparkline';
import { BarChart } from './charts/BarChart';
import { toast } from './Toast';
import { t, type Dict } from '../i18n';

interface DailyAggregate {
  date: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}
interface WeeklyAggregate {
  week_start: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}
interface MonthlyAggregate {
  year_month: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}
interface ProviderBucket {
  provider: string;
  calls: number;
  cost_usd: number;
}
// T-E-A-12: 按来源(source)分桶的费用聚合。
interface SourceBucket {
  source: string;
  calls: number;
  cost_usd: number;
}
interface CreditsOverview {
  daily: DailyAggregate[];
  weekly: WeeklyAggregate[];
  monthly: MonthlyAggregate[];
  by_provider: ProviderBucket[];
  by_agent: ProviderBucket[];
  // T-E-A-12: 按来源分桶(chat / automation / cron / background)。
  by_source: SourceBucket[];
  // M6 #81: 按 WorkType 分桶(chat / swarm_worker / swarm_synthesize / master_task /
  // evolution / soul_compile / classifier / unknown)。
  // Evolution / SoulCompile / Classifier 为 local_only(零远端成本)。
  by_work_type: SourceBucket[];
  total_cost_usd: number;
  semantic_cache_hits: number;
  semantic_cache_misses: number;
  // T-E-A-10: 省 Token / 省钱指标。
  cost_saved_usd: number;
  prefix_cache_cached_tokens: number;
}

type TrendTab = 'daily' | 'weekly' | 'monthly' | 'source' | 'work_type';

/**
 * M6 #81: WorkType 元数据 — 中文名 + 是否 local_only(零远端成本)。
 * 与后端 `WorkType::is_local_only()` 对齐:
 * Evolution / SoulCompile / Classifier 为 true(强制本地路由)。
 */
const WORK_TYPE_META: Record<string, { label: keyof Dict; localOnly: boolean; color: string }> = {
  chat: { label: 'creditsDashboard.workType.chat', localOnly: false, color: '#3b82f6' },
  swarm_worker: { label: 'creditsDashboard.workType.swarmWorker', localOnly: false, color: '#10b981' },
  swarm_synthesize: { label: 'creditsDashboard.workType.swarmSynthesize', localOnly: false, color: '#06b6d4' },
  master_task: { label: 'creditsDashboard.workType.masterTask', localOnly: false, color: '#8b5cf6' },
  evolution: { label: 'creditsDashboard.workType.evolution', localOnly: true, color: '#f59e0b' },
  soul_compile: { label: 'creditsDashboard.workType.soulCompile', localOnly: true, color: '#ec4899' },
  classifier: { label: 'creditsDashboard.workType.classifier', localOnly: true, color: '#84cc16' },
  unknown: { label: 'creditsDashboard.workType.unknown', localOnly: false, color: '#9ca3af' },
};

function loadBudget(): number {
  try {
    const raw = localStorage.getItem('nebula.settings');
    if (raw) {
      const parsed = JSON.parse(raw);
      return parsed.monthlyBudgetUsd ?? 0;
    }
  } catch {
    /* noop */
  }
  return 0;
}

function loadDailyBudget(): number {
  try {
    const raw = localStorage.getItem('nebula.settings');
    if (raw) {
      const parsed = JSON.parse(raw);
      return parsed.dailyBudgetUsd ?? 0;
    }
  } catch {
    /* noop */
  }
  return 0;
}

export function CreditsDashboard() {
  const [overview, setOverview] = useState<CreditsOverview | null>(null);
  const [tab, setTab] = useState<TrendTab>('daily');
  const [budget] = useState(loadBudget);
  const [dailyBudget] = useState(loadDailyBudget);
  const [error, setError] = useState<string | null>(null);
  // T-E-A-10: 命中率报警去重守卫，<30% 触发一次,恢复 ≥30% 才允许重报。
  const alarmedRef = useRef(false);

  useEffect(() => {
    let mounted = true;
    const load = () => {
      invoke<CreditsOverview>('credits_overview')
        .then((data: CreditsOverview) => {
          if (mounted) setOverview(data);
        })
        .catch((e: unknown) => {
          if (mounted) setError(String(e));
        });
    };
    load();
    // T-E-A-10: 2s 轮询实时刷新省 Token / 省钱 / 命中率。
    const timer = setInterval(load, 2000);
    return () => {
      mounted = false;
      clearInterval(timer);
    };
  }, []);

  if (error) {
    return (
      <div class="credits-dashboard">
        <p style="color:var(--text-secondary)">{error}</p>
      </div>
    );
  }
  if (!overview) {
    return (
      <div class="credits-dashboard">
        <p>{t('common.loading')}</p>
      </div>
    );
  }

  const trendData =
    tab === 'daily'
      ? overview.daily.map((d) => d.cost_usd)
      : tab === 'weekly'
        ? overview.weekly.map((w) => w.cost_usd)
        : tab === 'monthly'
          ? overview.monthly.map((m) => m.cost_usd)
          : tab === 'source'
            ? overview.by_source.map((s) => s.cost_usd)
            : overview.by_work_type.map((w) => w.cost_usd);

  // T-E-A-12: source 分组 — Chat vs Automation 两栏汇总。
  const chatBucket = overview.by_source.find((s) => s.source === 'chat');
  const automationBucket = overview.by_source.find((s) => s.source === 'automation');
  const chatCost = chatBucket ? chatBucket.cost_usd : 0;
  const automationCost = automationBucket ? automationBucket.cost_usd : 0;
  const chatCalls = chatBucket ? chatBucket.calls : 0;
  const automationCalls = automationBucket ? automationBucket.calls : 0;
  const sourceTotal = chatCost + automationCost;
  const automationRatio = sourceTotal > 0 ? (automationCost / sourceTotal) * 100 : 0;

  // M6 #81: WorkType 分域 — local_only(Evolution/SoulCompile/Classifier) vs remote-allowed。
  // local_only 工作类型强制本地路由,零远端成本;其余走 ModelRouter 可能命中远端。
  const workTypeBuckets = overview.by_work_type;
  const workTypeTotal = workTypeBuckets.reduce((sum, w) => sum + w.cost_usd, 0);
  const localBuckets = workTypeBuckets.filter((w) => {
    const meta = WORK_TYPE_META[w.source];
    return meta ? meta.localOnly : false;
  });
  const remoteBuckets = workTypeBuckets.filter((w) => {
    const meta = WORK_TYPE_META[w.source];
    return meta ? !meta.localOnly : true;
  });
  const localTotal = localBuckets.reduce((sum, w) => sum + w.cost_usd, 0);
  const remoteTotal = remoteBuckets.reduce((sum, w) => sum + w.cost_usd, 0);
  const localRatio = workTypeTotal > 0 ? (localTotal / workTypeTotal) * 100 : 0;
  const localCalls = localBuckets.reduce((sum, w) => sum + w.calls, 0);
  const remoteCalls = remoteBuckets.reduce((sum, w) => sum + w.calls, 0);
  // 柱状图归一化:取最大桶费用作为标尺。
  const maxWorkTypeCost = Math.max(...workTypeBuckets.map((w) => w.cost_usd), 0.01);

  const cacheTotal = overview.semantic_cache_hits + overview.semantic_cache_misses;
  const cacheHitRate = cacheTotal > 0 ? (overview.semantic_cache_hits / cacheTotal) * 100 : 0;
  const monthlyBudgetThreshold = budget > 0 ? budget : undefined;
  const overBudget = budget > 0 && overview.total_cost_usd > budget;

  // T-E-A-05: 当日已用费用(UTC 当天)。
  const todayUtc = new Date().toISOString().slice(0, 10);
  const todayAgg = overview.daily.find((d) => d.date === todayUtc);
  const todayCost = todayAgg ? todayAgg.cost_usd : 0;
  const overDailyBudget = dailyBudget > 0 && todayCost >= dailyBudget;

  // T-E-A-10: 命中率 <30% 报警(样本数 >10 才触发,避免冷启动误报),去重不刷屏。
  if (cacheHitRate < 30 && cacheTotal > 10 && !alarmedRef.current) {
    toast.warning(
      t('creditsDashboard.cacheRateLow'),
      t('creditsDashboard.cacheRateLowBody', { rate: cacheHitRate.toFixed(1) })
    );
    alarmedRef.current = true;
  } else if (cacheHitRate >= 30) {
    alarmedRef.current = false;
  }

  // Provider 分布归一化:取最大桶费用作为标尺。
  const maxProviderCost = Math.max(...overview.by_provider.map((p) => p.cost_usd), 0.01);

  // 预算进度条颜色:<60% 绿 / <90% 橙 / ≥90% 红。
  const monthlyPct = budget > 0 ? (overview.total_cost_usd / budget) * 100 : 0;
  const monthlyFillClass = overBudget || monthlyPct >= 90 ? 'red' : monthlyPct >= 60 ? 'orange' : 'green';
  const dailyPct = dailyBudget > 0 ? (todayCost / dailyBudget) * 100 : 0;
  const dailyFillClass = overDailyBudget || dailyPct >= 90 ? 'red' : dailyPct >= 60 ? 'orange' : 'green';
  const cacheFillClass = cacheHitRate >= 70 ? 'green' : cacheHitRate >= 30 ? 'orange' : 'red';

  return (
    <div class="credits-dashboard" style="display:flex;flex-direction:column;height:100%;">
      {/* 页面头:标题 + 工具按钮 */}
      <div class="page-header">
        <div>
          <div class="page-title">💰 {t('creditsDashboard.totalCost')}</div>
          <div class="page-subtitle">
            {budget > 0
              ? t('creditsDashboard.monthlyBudget', { budget: budget.toFixed(2) })
              : t('creditsDashboard.costTrend')}
          </div>
        </div>
        <div class="page-actions">
          <div class="tool-btn">📊 {t('creditsDashboard.costTrend')}</div>
        </div>
      </div>

      <div class="page-body" style="display:flex;flex-direction:column;gap:16px;">
        {/* 预算卡片:月度预算 / 今日花费 / 缓存命中 / 省钱 */}
        <div class="credits-overview">
          {/* 月度预算 */}
          <div class="credit-card">
            <div class="credit-label">{t('creditsDashboard.totalCost')}</div>
            <div class="credit-value" style={{ color: overBudget ? '#ff5f57' : undefined }}>
              ${overview.total_cost_usd.toFixed(4)}
              {budget > 0 && (
                <span style="font-size:14px;color:rgba(255,255,255,0.3);"> / ${budget.toFixed(2)}</span>
              )}
            </div>
            {budget > 0 && (
              <div class="credit-bar">
                <div
                  class={`credit-fill ${monthlyFillClass}`}
                  style={`width:${Math.min(monthlyPct, 100)}%`}
                />
              </div>
            )}
            {budget > 0 && (
              <div style="font-size:11px;color:rgba(255,255,255,0.4);margin-top:6px;">
                {overBudget
                  ? t('creditsDashboard.overBudget')
                  : t('creditsDashboard.budgetRemaining', {
                      amount: (budget - overview.total_cost_usd).toFixed(2),
                    })}
              </div>
            )}
          </div>

          {/* 今日花费 */}
          <div class="credit-card">
            <div class="credit-label">{t('creditsDashboard.dailyCost')}</div>
            <div class="credit-value" style={{ color: overDailyBudget ? '#ff5f57' : undefined }}>
              ${todayCost.toFixed(4)}
              {dailyBudget > 0 && (
                <span style="font-size:14px;color:rgba(255,255,255,0.3);"> / ${dailyBudget.toFixed(2)}</span>
              )}
            </div>
            {dailyBudget > 0 && (
              <div class="credit-bar">
                <div
                  class={`credit-fill ${dailyFillClass}`}
                  style={`width:${Math.min(dailyPct, 100)}%`}
                />
              </div>
            )}
            <div style="font-size:11px;color:rgba(255,255,255,0.4);margin-top:6px;">
              {dailyBudget > 0
                ? overDailyBudget
                  ? t('creditsDashboard.overDailyBudget')
                  : t('creditsDashboard.dailyBudgetRemaining', {
                      amount: (dailyBudget - todayCost).toFixed(2),
                    })
                : t('creditsDashboard.dailyBudgetNotSet')}
            </div>
          </div>

          {/* 缓存命中率 */}
          <div class="credit-card">
            <div class="credit-label">{t('creditsDashboard.cacheHitRate')}</div>
            <div class="credit-value">{cacheHitRate.toFixed(1)}%</div>
            <div class="credit-bar">
              <div
                class={`credit-fill ${cacheFillClass}`}
                style={`width:${Math.min(cacheHitRate, 100)}%`}
              />
            </div>
            <div style="font-size:11px;color:rgba(255,255,255,0.4);margin-top:6px;">
              {overview.semantic_cache_hits} / {cacheTotal}
            </div>
          </div>

          {/* 省钱 */}
          <div class="credit-card">
            <div class="credit-label">{t('creditsDashboard.saved')}</div>
            <div class="credit-value">${overview.cost_saved_usd.toFixed(4)}</div>
            <div class="credit-bar">
              <div class="credit-fill green" style="width:100%" />
            </div>
            <div style="font-size:11px;color:rgba(255,255,255,0.4);margin-top:6px;">
              {t('creditsDashboard.prefixCacheTokens', {
                count: overview.prefix_cache_cached_tokens,
              })}
            </div>
          </div>
        </div>

        {/* 趋势图 */}
        <div class="stat-card" style="padding:16px;">
          <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:8px;">
            <div class="stat-label" style="margin-bottom:0;">
              {tab === 'source'
                ? t('creditsDashboard.chatVsAutomation')
                : tab === 'work_type'
                  ? t('creditsDashboard.workTypeTitle')
                  : t('creditsDashboard.costTrend')}
            </div>
            <div class="page-actions">
              {(['daily', 'weekly', 'monthly', 'source', 'work_type'] as TrendTab[]).map((tb) => (
                <button
                  key={tb}
                  onClick={() => setTab(tb)}
                  class={`tool-btn ${tab === tb ? 'tool-btn-primary' : ''}`}
                  style={{ cursor: 'pointer', border: 'none' }}
                >
                  {t(`creditsDashboard.tab.${tb}`)}
                </button>
              ))}
            </div>
          </div>
          {tab === 'source' ? (
            /* T-E-A-12: Chat vs Automation 两栏对比 — 当月费用 + 调用次数 + 自动化占比。 */
            <div style="display:flex;gap:12px;">
              <div
                class="stat-card"
                style={{ flex: 1, padding: '12px', borderLeft: '3px solid #3b82f6' }}
              >
                <div class="stat-label" style="margin-bottom:0;">{t('creditsDashboard.chatManual')}</div>
                <div class="stat-value" style={{ color: '#3b82f6', fontSize: '20px' }}>
                  ${chatCost.toFixed(4)}
                </div>
                <div style="font-size:11px;color:rgba(255,255,255,0.4);">
                  {t('creditsDashboard.callsCount', { count: chatCalls })}
                </div>
              </div>
              <div
                class="stat-card"
                style={{ flex: 1, padding: '12px', borderLeft: '3px solid #f59e0b' }}
              >
                <div class="stat-label" style="margin-bottom:0;">{t('creditsDashboard.automation')}</div>
                <div class="stat-value" style={{ color: '#f59e0b', fontSize: '20px' }}>
                  ${automationCost.toFixed(4)}
                </div>
                <div style="font-size:11px;color:rgba(255,255,255,0.4);">
                  {t('creditsDashboard.callsCount', { count: automationCalls })}
                </div>
              </div>
              <div
                class="stat-card"
                style={{
                  flex: 1,
                  padding: '12px',
                  borderLeft: `3px solid ${automationRatio > 50 ? '#ff5f57' : '#0A84FF'}`,
                }}
              >
                <div class="stat-label" style="margin-bottom:0;">{t('creditsDashboard.automationRatio')}</div>
                <div
                  class="stat-value"
                  style={{ color: automationRatio > 50 ? '#ff5f57' : undefined, fontSize: '20px' }}
                >
                  {automationRatio.toFixed(1)}%
                </div>
                <div style="font-size:11px;color:rgba(255,255,255,0.4);">
                  {t('creditsDashboard.sourceTotal', { amount: sourceTotal.toFixed(4) })}
                </div>
              </div>
            </div>
          ) : tab === 'work_type' ? (
            /* M6 #81: WorkType 分域 — local_only vs remote-allowed + 7 桶柱状图(每桶独立着色)。 */
            <div class="work-type-view">
              {/* 顶部:三栏汇总卡片 */}
              <div style="display:flex;gap:12px;margin-bottom:12px;">
                <div
                  class="stat-card"
                  style={{ flex: 1, padding: '12px', borderLeft: '3px solid #10b981' }}
                >
                  <div class="stat-label" style="margin-bottom:0;">{t('creditsDashboard.workTypeLocal')}</div>
                  <div class="stat-value" style={{ color: '#10b981', fontSize: '20px' }}>
                    ${localTotal.toFixed(4)}
                  </div>
                  <div style="font-size:11px;color:rgba(255,255,255,0.4);">
                    {t('creditsDashboard.workTypeCalls', { count: localCalls })}
                  </div>
                </div>
                <div
                  class="stat-card"
                  style={{ flex: 1, padding: '12px', borderLeft: '3px solid #3b82f6' }}
                >
                  <div class="stat-label" style="margin-bottom:0;">{t('creditsDashboard.workTypeRemote')}</div>
                  <div class="stat-value" style={{ color: '#3b82f6', fontSize: '20px' }}>
                    ${remoteTotal.toFixed(4)}
                  </div>
                  <div style="font-size:11px;color:rgba(255,255,255,0.4);">
                    {t('creditsDashboard.workTypeCalls', { count: remoteCalls })}
                  </div>
                </div>
                <div
                  class="stat-card"
                  style={{
                    flex: 1,
                    padding: '12px',
                    borderLeft: `3px solid ${localRatio >= 50 ? '#10b981' : '#f59e0b'}`,
                  }}
                >
                  <div class="stat-label" style="margin-bottom:0;">{t('creditsDashboard.automationRatio')}</div>
                  <div
                    class="stat-value"
                    style={{ color: localRatio >= 50 ? '#10b981' : '#f59e0b', fontSize: '20px' }}
                  >
                    {localRatio.toFixed(1)}%
                  </div>
                  <div style="font-size:11px;color:rgba(255,255,255,0.4);">
                    {t('creditsDashboard.workTypeLocalRatio', { ratio: localRatio.toFixed(1) })}
                  </div>
                </div>
              </div>

              {/* 中部:7 桶柱状图(每桶独立着色,local_only 半透明虚线边框) */}
              {workTypeBuckets.length === 0 ? (
                <div class="work-type-empty">{t('common.loading')}</div>
              ) : (
                <svg
                  class="work-type-bar-chart"
                  width={560}
                  height={140}
                  style={{ display: 'block' }}
                >
                  {workTypeBuckets.map((w, i) => {
                    const meta = WORK_TYPE_META[w.source] ?? WORK_TYPE_META.unknown;
                    const barWidth = 560 / workTypeBuckets.length;
                    const barAreaHeight = 100;
                    const barHeight = (w.cost_usd / maxWorkTypeCost) * barAreaHeight;
                    const x = i * barWidth + 4;
                    const y = barAreaHeight - barHeight + 4;
                    const labelY = barAreaHeight + 16;
                    const valueY = y - 4;
                    const truncatedLabel =
                      meta.label.length > 8 ? meta.label.slice(0, 7) + '..' : meta.label;
                    return (
                      <g key={w.source}>
                        <rect
                          x={x}
                          y={y}
                          width={barWidth - 8}
                          height={barHeight}
                          fill={meta.color}
                          fillOpacity={meta.localOnly ? 0.55 : 0.85}
                          stroke={meta.localOnly ? meta.color : 'none'}
                          strokeDasharray={meta.localOnly ? '3,2' : 'none'}
                          rx={2}
                        />
                        <text
                          x={x + (barWidth - 8) / 2}
                          y={valueY}
                          fill="var(--text-secondary, #888)"
                          fontSize={9}
                          textAnchor="middle"
                        >
                          ${w.cost_usd.toFixed(3)}
                        </text>
                        <text
                          x={x + (barWidth - 8) / 2}
                          y={labelY}
                          fill="var(--text-secondary, #888)"
                          fontSize={9}
                          textAnchor="middle"
                        >
                          {truncatedLabel}
                        </text>
                        <text
                          x={x + (barWidth - 8) / 2}
                          y={labelY + 12}
                          fill="var(--text-secondary, #888)"
                          fontSize={8}
                          textAnchor="middle"
                        >
                          {w.calls}
                        </text>
                      </g>
                    );
                  })}
                </svg>
              )}

              {/* 底部:桶明细列表(按费用降序) */}
              <div class="work-type-list">
                {workTypeBuckets.map((w) => {
                  const meta = WORK_TYPE_META[w.source] ?? WORK_TYPE_META.unknown;
                  return (
                    <div
                      key={w.source}
                      class="work-type-row"
                      style={{ borderLeft: `3px solid ${meta.color}` }}
                    >
                      <span class="work-type-label" title={w.source}>
                        <span
                          class="work-type-dot"
                          style={{
                            backgroundColor: meta.color,
                            opacity: meta.localOnly ? 0.55 : 1,
                          }}
                        />
                        {t(meta.label)}
                      </span>
                      <span class="work-type-cost">${w.cost_usd.toFixed(4)}</span>
                      <span class="work-type-calls">
                        {t('creditsDashboard.workTypeCalls', { count: w.calls })}
                      </span>
                      {meta.localOnly && (
                        <span
                          class="work-type-badge"
                          title={t('creditsDashboard.workTypeLocalOnlyHint')}
                        >
                          local-only
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          ) : (
            <Sparkline data={trendData} width={560} height={80} threshold={monthlyBudgetThreshold} />
          )}
        </div>

        {/* Provider 分布条形图 */}
        <div class="stat-card" style="padding:16px;">
          <div class="stat-label" style="margin-bottom:10px;">
            {t('creditsDashboard.byProvider')}
          </div>
          <div class="provider-list">
            {overview.by_provider.map((p) => {
              const pct = (p.cost_usd / maxProviderCost) * 100;
              return (
                <div class="provider-row" key={p.provider}>
                  <span class="provider-name">{p.provider}</span>
                  <div class="provider-bar">
                    <div class="provider-fill" style={`width:${pct}%`} />
                  </div>
                  <span class="provider-cost">${p.cost_usd.toFixed(2)}</span>
                </div>
              );
            })}
          </div>
        </div>

        {/* Agent 分布 */}
        <div class="stat-card" style="padding:16px;">
          <div class="stat-label" style="margin-bottom:10px;">
            {t('creditsDashboard.byAgent')}
          </div>
          <BarChart
            data={overview.by_agent.map((a) => ({ label: a.provider, value: a.cost_usd }))}
            width={560}
            height={120}
            color="#8b5cf6"
            valueFormatter={(v) => `$${v.toFixed(3)}`}
          />
        </div>
      </div>
    </div>
  );
}
