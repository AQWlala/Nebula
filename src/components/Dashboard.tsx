/**
 * v2.0: 可观测性仪表盘 + sidecar 状态 + 自我反思。
 *
 * 展示 6 项核心指标（设计文档 §7）：
 *   - 内存占用 (RSS + Virtual + Budget 状态)
 *   - 向量检索延迟 (平均 + P95 估算 + 总调用次数)
 *   - 蜂群状态 (执行次数 + 成功率估算 + 活跃 agent 数)
 *   - 缓存命中率 (embedding cache hit ratio + 趋势)
 *   - L4 拦截率 (安全拦截统计 — L4 未实现前展示 0 + 占位)
 *   - LLM 调用延迟/成本 (平均延迟 + 总调用次数 + token 估算)
 *
 * v2.0 新增：
 *   - Sidecar 服务状态（Memory/LLM/Swarm 三个进程状态）
 *   - 自我反思（价值对齐 + 结局复盘 + 自我改进）
 *
 * 数据每 2 秒刷新一次，来源于 `metrics` 和 `perf_sample` 命令。
 */
import { useEffect, useState } from 'preact/hooks';
import {
  invokeTauri,
  nebulaAPI,
  type MetricsSnapshot,
  type PerfSample,
  type SidecarStatusInfo,
  type SelfReflection,
} from '../lib/tauri';
import { t } from '../i18n';

interface DashboardData {
  metrics: MetricsSnapshot | null;
  perf: PerfSample | null;
  sidecars: SidecarStatusInfo[];
  lastUpdated: number;
}

function fmtBytes(n: number | null | undefined): string {
  if (n == null) return '–';
  if (n < 1024) return `${n} B`;
  const kb = n / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(2)} GB`;
}

function fmtMs(usTotal: number | null | undefined, count: number | null | undefined): string {
  if (!count || count === 0) return '–';
  const avgMs = (usTotal ?? 0) / count / 1000;
  if (avgMs < 1) return `${(avgMs * 1000).toFixed(0)}µs`;
  if (avgMs < 1000) return `${avgMs.toFixed(0)}ms`;
  return `${(avgMs / 1000).toFixed(2)}s`;
}

function fmtRatio(hits: number, misses: number): string {
  const total = hits + misses;
  if (total === 0) return '–';
  return `${((hits / total) * 100).toFixed(1)}%`;
}

function fmtCount(n: number | null | undefined): string {
  if (n == null) return '–';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

interface MetricCardProps {
  title: string;
  value: string;
  subtitle?: string;
  icon: string;
  accent?: 'blue' | 'green' | 'amber' | 'red' | 'purple' | 'cyan';
  progress?: number; // 0-100
}

function MetricCard({ title, value, subtitle, icon, accent = 'blue', progress }: MetricCardProps) {
  const accentClass = `card-accent-${accent}`;
  return (
    <div class={`metric-card ${accentClass}`}>
      <div class="metric-card-header">
        <span class="metric-icon">{icon}</span>
        <span class="metric-title">{title}</span>
      </div>
      <div class="metric-value">{value}</div>
      {subtitle && <div class="metric-subtitle">{subtitle}</div>}
      {progress != null && (
        <div class="metric-progress-bar">
          <div
            class="metric-progress-fill"
            style={{ width: `${Math.min(100, Math.max(0, progress))}%` }}
          />
        </div>
      )}
    </div>
  );
}

export function Dashboard() {
  const [data, setData] = useState<DashboardData>({
    metrics: null,
    perf: null,
    sidecars: [],
    lastUpdated: 0,
  });
  const [reflections, setReflections] = useState<SelfReflection[]>([]);
  const [reflecting, setReflecting] = useState(false);

  useEffect(() => {
    // T-D-F-06: AbortController 替代 cancelled 布尔反模式。
    const ac = new AbortController();

    async function tick() {
      try {
        const [perf, metrics, sidecars] = await Promise.all([
          invokeTauri<PerfSample>('perf_sample'),
          invokeTauri<MetricsSnapshot>('metrics'),
          invokeTauri<SidecarStatusInfo[]>('sidecar_list_status'),
        ]);
        if (!ac.signal.aborted) {
          setData({
            perf: perf ?? null,
            metrics: metrics ?? null,
            sidecars: sidecars ?? [],
            lastUpdated: Date.now(),
          });
        }
      } catch {
        /* ignore — next tick will retry */
      }
    }

    tick();
    const id = setInterval(tick, 2000);
    return () => {
      ac.abort();
      clearInterval(id);
    };
  }, []);

  const { metrics, perf, sidecars } = data;

  // 内存占用卡片数据
  const rssBytes = perf?.rss_bytes ?? null;
  const rssBudgetBytes = 500 * 1024 * 1024; // 500MB
  const rssPct = rssBytes ? (rssBytes / rssBudgetBytes) * 100 : 0;
  const rssAccent = rssPct > 90 ? 'red' : rssPct > 70 ? 'amber' : 'green';

  // 向量检索延迟卡片数据
  const searchAvg = fmtMs(
    metrics?.memory_search_latency_us_total,
    metrics?.memory_search_latency_count
  );
  const searchCount = metrics?.memory_search_latency_count ?? 0;

  // 蜂群状态卡片数据
  const swarmCount = metrics?.swarm_executions_total ?? 0;

  // 缓存命中率卡片数据
  const cacheHits = metrics?.embedding_cache_hits ?? 0;
  const cacheMisses = metrics?.embedding_cache_misses ?? 0;
  const cacheRatio =
    cacheHits + cacheMisses > 0 ? (cacheHits / (cacheHits + cacheMisses)) * 100 : 0;
  const cacheAccent = cacheRatio > 80 ? 'green' : cacheRatio > 50 ? 'blue' : 'amber';

  // L4 拦截率 — T-S1-B-03: 接入真实 L4 裁定计数。
  // "拦截"= 非直接放行(Confirm + Plan + Deny);总数 = Allow + Confirm + Plan + Deny。
  const l4Allow = metrics?.l4_allow_total ?? 0;
  const l4Confirm = metrics?.l4_confirm_total ?? 0;
  const l4Plan = metrics?.l4_plan_total ?? 0;
  const l4Deny = metrics?.l4_deny_total ?? 0;
  const l4Blocked = l4Confirm + l4Plan + l4Deny;
  const l4Total = l4Allow + l4Confirm + l4Plan + l4Deny;
  const l4Ratio = l4Total > 0 ? (l4Blocked / l4Total) * 100 : 0;

  // T-S1-B-03: L0 热缓存命中率(独立于 embedder 缓存)。
  const l0Hits = metrics?.l0_hits ?? 0;
  const l0Misses = metrics?.l0_misses ?? 0;
  const l0Ratio = l0Hits + l0Misses > 0 ? (l0Hits / (l0Hits + l0Misses)) * 100 : 0;

  // T-S1-B-03: Token 用量(累计)。
  const tokenPrompt = metrics?.token_prompt_total ?? 0;
  const tokenCompletion = metrics?.token_completion_total ?? 0;
  const tokenTotal = tokenPrompt + tokenCompletion;

  // T-S1-B-03: ACL 拒绝率。
  const aclAllow = metrics?.acl_allow_total ?? 0;
  const aclDeny = metrics?.acl_deny_total ?? 0;
  const aclTotal = aclAllow + aclDeny;
  const aclDenyRatio = aclTotal > 0 ? (aclDeny / aclTotal) * 100 : 0;

  // T-S1-B-03: 反思被 RoundGuard skip 次数。
  const reflectionsSkipped = metrics?.reflections_skipped_total ?? 0;

  // LLM 调用延迟/成本
  const chatAvg = fmtMs(metrics?.llm_chat_latency_us_total, metrics?.llm_chat_latency_count);
  const chatTotal = metrics?.chat_total ?? 0;

  // 自我反思
  async function handleSelfReflect() {
    if (reflecting) return;
    setReflecting(true);
    try {
      const result = await nebulaAPI.selfReflectNow();
      if (result) setReflections(result);
    } catch {
      /* ignore */
    } finally {
      setReflecting(false);
    }
  }

  // sidecar 操作
  async function handleSidecarAction(kind: string, action: 'start' | 'stop' | 'restart') {
    try {
      if (action === 'start') await nebulaAPI.sidecarStart(kind);
      else if (action === 'stop') await nebulaAPI.sidecarStop(kind);
      else await nebulaAPI.sidecarRestart(kind);
      // 刷新状态
      const updated = await nebulaAPI.sidecarListStatus();
      if (updated) {
        setData((d) => ({ ...d, sidecars: updated }));
      }
    } catch {
      /* ignore */
    }
  }

  function sidecarStatusLabel(status: string): string {
    const key = `dashboard.sidecar.status.${status}`;
    const translated = t(key as any);
    return translated === key ? status : translated;
  }

  function sidecarStatusColor(status: string): string {
    switch (status) {
      case 'running':
        return 'status-green';
      case 'starting':
        return 'status-amber';
      case 'restarting':
        return 'status-amber';
      case 'crashed':
        return 'status-red';
      default:
        return 'status-gray';
    }
  }

  function reflectionKindLabel(kind: string): string {
    const map: Record<string, string> = {
      value_alignment: t('dashboard.selfReflect.kind.valueAlignment' as any),
      outcome_review: t('dashboard.selfReflect.kind.outcomeReview' as any),
      self_improvement: t('dashboard.selfReflect.kind.selfImprovement' as any),
    };
    return map[kind] || kind;
  }

  function severityColor(severity: number): string {
    if (severity >= 0.7) return 'status-red';
    if (severity >= 0.4) return 'status-amber';
    return 'status-green';
  }

  return (
    <div class="dashboard">
      <div class="dashboard-header">
        <h2 class="dashboard-title">📊 {t('dashboard.title')}</h2>
        <span class="dashboard-update">
          {t('dashboard.lastUpdated')} {new Date(data.lastUpdated).toLocaleTimeString()}
        </span>
      </div>

      <div class="dashboard-grid">
        <MetricCard
          title={t('dashboard.memory.title')}
          value={fmtBytes(rssBytes)}
          subtitle={`${t('dashboard.memory.budget')}: ${fmtBytes(rssBudgetBytes)}`}
          icon="💾"
          accent={rssAccent as 'green' | 'amber' | 'red'}
          progress={rssPct}
        />

        <MetricCard
          title={t('dashboard.search.title')}
          value={searchAvg}
          subtitle={`${t('dashboard.search.count')}: ${fmtCount(searchCount)}`}
          icon="🔍"
          accent="cyan"
        />

        <MetricCard
          title={t('dashboard.swarm.title')}
          value={fmtCount(swarmCount)}
          subtitle={t('dashboard.swarm.subtitle')}
          icon="🐝"
          accent="amber"
        />

        <MetricCard
          title={t('dashboard.cache.title')}
          value={fmtRatio(cacheHits, cacheMisses)}
          subtitle={`${cacheHits + cacheMisses} ${t('dashboard.cache.lookups')}`}
          icon="⚡"
          accent={cacheAccent as 'blue' | 'green' | 'amber'}
          progress={cacheRatio}
        />

        <MetricCard
          title={t('dashboard.l4.title')}
          value={l4Total > 0 ? `${l4Ratio.toFixed(1)}%` : '–'}
          subtitle={`${l4Blocked}/${l4Total} ${t('dashboard.l4.subtitle')}`}
          icon="🛡️"
          accent="purple"
          progress={l4Ratio}
        />

        <MetricCard
          title={t('dashboard.llm.title')}
          value={chatAvg}
          subtitle={`${t('dashboard.llm.count')}: ${fmtCount(chatTotal)}`}
          icon="🤖"
          accent="blue"
        />
      </div>

      <div class="dashboard-details">
        <div class="detail-section">
          <h3>{t('dashboard.perf.title')}</h3>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.perf.rss')}</span>
            <span class="detail-value">{fmtBytes(perf?.rss_bytes)}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.perf.virtual')}</span>
            <span class="detail-value">{fmtBytes(perf?.virt_bytes)}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.perf.cpu')}</span>
            <span class="detail-value">
              {perf?.cpu_pct != null ? `${perf.cpu_pct.toFixed(1)}%` : '–'}
            </span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.perf.overBudget')}</span>
            <span class={`detail-value ${perf?.over_budget ? 'text-red' : 'text-green'}`}>
              {perf?.over_budget ? t('common.yes') : t('common.no')}
            </span>
          </div>
        </div>

        <div class="detail-section">
          <h3>{t('dashboard.counters.title')}</h3>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.counters.stores')}</span>
            <span class="detail-value">{fmtCount(metrics?.memory_stores_total)}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.counters.searches')}</span>
            <span class="detail-value">{fmtCount(metrics?.memory_searches_total)}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.counters.blackhole')}</span>
            <span class="detail-value">{fmtCount(metrics?.blackhole_compressions_total)}</span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.counters.reflections')}</span>
            <span class="detail-value">{fmtCount(metrics?.reflections_generated_total)}</span>
          </div>
        </div>

        <div class="detail-section">
          <h3>{t('dashboard.observability.title')}</h3>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.observability.l0Hits')}</span>
            <span class="detail-value">
              {l0Ratio.toFixed(1)}% ({fmtCount(l0Hits)}/{fmtCount(l0Hits + l0Misses)})
            </span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.observability.tokenTotal')}</span>
            <span class="detail-value">
              {fmtCount(tokenTotal)} (P:{fmtCount(tokenPrompt)} / C:{fmtCount(tokenCompletion)})
            </span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.observability.l4Breakdown')}</span>
            <span class="detail-value">
              ✓{fmtCount(l4Allow)} / ?{fmtCount(l4Confirm)} / P{fmtCount(l4Plan)} / ✗
              {fmtCount(l4Deny)}
            </span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.observability.aclDeny')}</span>
            <span class="detail-value">
              {aclDenyRatio.toFixed(1)}% ({fmtCount(aclDeny)}/{fmtCount(aclTotal)})
            </span>
          </div>
          <div class="detail-row">
            <span class="detail-label">{t('dashboard.observability.reflectionsSkipped')}</span>
            <span class={`detail-value ${reflectionsSkipped > 0 ? 'text-amber' : ''}`}>
              {fmtCount(reflectionsSkipped)}
            </span>
          </div>
        </div>

        <div class="detail-section">
          <h3>{t('dashboard.sidecar.title')}</h3>
          {sidecars.length === 0 ? (
            <div class="detail-empty">–</div>
          ) : (
            sidecars.map((sc) => (
              <div key={sc.kind} class="sidecar-row">
                <div class="sidecar-info">
                  <span class={`status-dot ${sidecarStatusColor(sc.status)}`}></span>
                  <span class="sidecar-name">
                    {sc.kind === 'memory' && t('dashboard.sidecar.memory')}
                    {sc.kind === 'llm' && t('dashboard.sidecar.llm')}
                    {sc.kind === 'swarm' && t('dashboard.sidecar.swarm')}
                    {sc.kind !== 'memory' && sc.kind !== 'llm' && sc.kind !== 'swarm' && sc.kind}
                  </span>
                  <span class="sidecar-status">{sidecarStatusLabel(sc.status)}</span>
                </div>
                <div class="sidecar-actions">
                  {!sc.running ? (
                    <button
                      class="btn btn-small btn-primary"
                      onClick={() => handleSidecarAction(sc.kind, 'start')}
                    >
                      {t('dashboard.sidecar.actions.start')}
                    </button>
                  ) : (
                    <>
                      <button
                        class="btn btn-small"
                        onClick={() => handleSidecarAction(sc.kind, 'restart')}
                      >
                        {t('dashboard.sidecar.actions.restart')}
                      </button>
                      <button
                        class="btn btn-small btn-danger"
                        onClick={() => handleSidecarAction(sc.kind, 'stop')}
                      >
                        {t('dashboard.sidecar.actions.stop')}
                      </button>
                    </>
                  )}
                </div>
              </div>
            ))
          )}
        </div>

        <div class="detail-section">
          <div class="section-header">
            <h3>{t('dashboard.selfReflect.title')}</h3>
            <button
              class={`btn btn-small btn-primary ${reflecting ? 'btn-loading' : ''}`}
              onClick={handleSelfReflect}
              disabled={reflecting}
            >
              {reflecting
                ? t('dashboard.selfReflect.reflecting')
                : t('dashboard.selfReflect.button')}
            </button>
          </div>
          {reflections.length === 0 ? (
            <div class="detail-empty">{t('dashboard.selfReflect.button')} →</div>
          ) : (
            reflections.map((r) => (
              <div key={`${r.kind}-${r.title}`} class="reflection-card">
                <div class="reflection-header">
                  <span class="reflection-kind">{reflectionKindLabel(r.kind)}</span>
                  <span class={`reflection-severity ${severityColor(r.severity)}`}>
                    {Math.round(r.severity * 100)}%
                  </span>
                </div>
                <h4 class="reflection-title">{r.title}</h4>
                <div
                  class="reflection-body"
                  dangerouslySetInnerHTML={{ __html: r.content.replace(/\n/g, '<br/>') }}
                />
                {r.insights.length > 0 && (
                  <div class="reflection-insights">
                    <strong>{t('dashboard.selfReflect.insights')}:</strong>
                    <ul>
                      {r.insights.map((ins, j) => (
                        <li key={j}>{ins}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {r.actionItems.length > 0 && (
                  <div class="reflection-actions">
                    <strong>{t('dashboard.selfReflect.actionItems')}:</strong>
                    <ol>
                      {r.actionItems.map((act, j) => (
                        <li key={j}>{act}</li>
                      ))}
                    </ol>
                  </div>
                )}
                <div class="reflection-meta">
                  <span>
                    {t('dashboard.selfReflect.confidence')}: {Math.round(r.confidence * 100)}%
                  </span>
                  <span>
                    {t('dashboard.selfReflect.severity')}: {Math.round(r.severity * 100)}%
                  </span>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
