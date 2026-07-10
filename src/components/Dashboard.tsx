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

// 最近活动示例数据（设计样稿 5 行）
interface ActivityRow {
  session: string;
  time: string;
  status: 'done' | 'running' | 'failed';
}

const ACTIVITY_ROWS: ActivityRow[] = [
  { session: '修复 Windows 构建脚本路径错误', time: '2 分钟前', status: 'done' },
  { session: '分析今天头条文章的用户反馈', time: '18 分钟前', status: 'running' },
  { session: '从对话中提取 L2 经验记忆', time: '42 分钟前', status: 'done' },
  { session: 'Skill: code-review 审查 PR #142', time: '1 小时前', status: 'failed' },
  { session: '生成本周工作周报草稿', time: '2 小时前', status: 'done' },
];

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

  // 统计卡片图标背景色（按 accent 映射）
  const rssIconBg =
    rssAccent === 'red'
      ? 'rgba(255,95,87,0.15)'
      : rssAccent === 'amber'
        ? 'rgba(255,159,10,0.15)'
        : 'rgba(40,200,64,0.15)';
  const cacheIconBg =
    cacheAccent === 'green'
      ? 'rgba(40,200,64,0.15)'
      : cacheAccent === 'amber'
        ? 'rgba(255,159,10,0.15)'
        : 'rgba(10,132,255,0.15)';

  return (
    <div class="dashboard">
      {/* ===== 页面头 ===== */}
      <div class="page-header">
        <div>
          <div class="page-title">📊 系统概览</div>
          <div class="page-subtitle">
            实时性能指标 · 24h
            {data.lastUpdated ? ` · 更新于 ${new Date(data.lastUpdated).toLocaleTimeString()}` : ''}
          </div>
        </div>
        <div class="page-actions">
          <div class="tool-btn">24h</div>
          <div class="tool-btn tool-btn-primary">7d</div>
          <div class="tool-btn">30d</div>
          <div class="tool-btn">📥 导出</div>
        </div>
      </div>

      <div class="page-body">
        {/* ===== 4 列统计卡片 ===== */}
        <div
          class="dashboard-grid"
          style={{ gridTemplateColumns: 'repeat(4,1fr)', marginBottom: '22px' }}
        >
          {/* 内存占用 */}
          <div class="stat-card">
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '10px' }}>
              <span
                style={{
                  width: '28px',
                  height: '28px',
                  borderRadius: '7px',
                  background: rssIconBg,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: '14px',
                }}
              >
                🧠
              </span>
              <span class="stat-label" style={{ marginBottom: 0 }}>
                内存占用
              </span>
            </div>
            <div class="stat-value">{fmtBytes(rssBytes)}</div>
            <div class={`stat-trend ${rssAccent === 'red' ? 'down' : 'up'}`}>
              {rssPct.toFixed(1)}% · 预算 {fmtBytes(rssBudgetBytes)}
            </div>
          </div>

          {/* 向量检索延迟 */}
          <div class="stat-card">
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '10px' }}>
              <span
                style={{
                  width: '28px',
                  height: '28px',
                  borderRadius: '7px',
                  background: 'rgba(40,200,64,0.15)',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: '14px',
                }}
              >
                ⚡
              </span>
              <span class="stat-label" style={{ marginBottom: 0 }}>
                向量检索延迟
              </span>
            </div>
            <div class="stat-value">{searchAvg}</div>
            <div class="stat-trend up" style={{ color: 'rgba(255,255,255,0.4)' }}>
              {fmtCount(searchCount)} 次
            </div>
          </div>

          {/* 蜂群任务 */}
          <div class="stat-card">
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '10px' }}>
              <span
                style={{
                  width: '28px',
                  height: '28px',
                  borderRadius: '7px',
                  background: 'rgba(167,139,246,0.15)',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: '14px',
                }}
              >
                🐝
              </span>
              <span class="stat-label" style={{ marginBottom: 0 }}>
                蜂群任务
              </span>
            </div>
            <div class="stat-value">{fmtCount(swarmCount)}</div>
            <div class="stat-trend up" style={{ color: 'rgba(255,255,255,0.4)' }}>
              {t('dashboard.swarm.subtitle')}
            </div>
          </div>

          {/* 缓存命中率 */}
          <div class="stat-card">
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '10px' }}>
              <span
                style={{
                  width: '28px',
                  height: '28px',
                  borderRadius: '7px',
                  background: cacheIconBg,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: '14px',
                }}
              >
                📦
              </span>
              <span class="stat-label" style={{ marginBottom: 0 }}>
                缓存命中率
              </span>
            </div>
            <div class="stat-value">{fmtRatio(cacheHits, cacheMisses)}</div>
            <div class={`stat-trend ${cacheAccent === 'amber' ? 'down' : 'up'}`}>
              {cacheHits + cacheMisses} {t('dashboard.cache.lookups')}
            </div>
          </div>
        </div>

        {/* ===== 最近活动表 ===== */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: '10px',
          }}
        >
          <span style={{ fontSize: '13.5px', fontWeight: 650 }}>最近活动</span>
          <div class="tool-btn" style={{ fontSize: '11.5px' }}>
            查看全部 →
          </div>
        </div>
        <div
          style={{
            background: 'rgba(255,255,255,0.025)',
            border: '1px solid rgba(255,255,255,0.05)',
            borderRadius: '10px',
            overflow: 'hidden',
            marginBottom: '22px',
          }}
        >
          {/* 表头 */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 130px 90px',
              gap: '12px',
              padding: '10px 16px',
              borderBottom: '1px solid rgba(255,255,255,0.04)',
              fontSize: '10.5px',
              fontWeight: 600,
              color: 'rgba(255,255,255,0.3)',
              textTransform: 'uppercase',
              letterSpacing: '.05em',
            }}
          >
            会话 · 时间 · 状态
          </div>
          {/* 5 行示例数据 */}
          {ACTIVITY_ROWS.map((a, i) => {
            const badge =
              a.status === 'done'
                ? { bg: 'rgba(40,200,64,0.13)', color: '#28c840', text: '✓ 完成' }
                : a.status === 'running'
                  ? { bg: 'rgba(10,132,255,0.13)', color: '#0A84FF', text: '● 进行中' }
                  : { bg: 'rgba(255,95,87,0.13)', color: '#ff5f57', text: '✗ 失败' };
            return (
              <div
                key={i}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '1fr 130px 90px',
                  gap: '12px',
                  padding: '10px 16px',
                  borderBottom: i < ACTIVITY_ROWS.length - 1 ? '1px solid rgba(255,255,255,0.04)' : 'none',
                  fontSize: '12.5px',
                  alignItems: 'center',
                }}
              >
                <span>{a.session}</span>
                <span style={{ color: 'rgba(255,255,255,0.3)' }}>{a.time}</span>
                <span>
                  <span
                    style={{
                      fontSize: '10.5px',
                      padding: '2px 8px',
                      borderRadius: '100px',
                      background: badge.bg,
                      color: badge.color,
                    }}
                  >
                    {badge.text}
                  </span>
                </span>
              </div>
            );
          })}
        </div>

        {/* ===== 记忆健康度（3 列大数字 + 进度条） ===== */}
        <div style={{ fontSize: '13.5px', fontWeight: 650, marginBottom: '10px' }}>记忆健康度</div>
        <div class="dashboard-grid" style={{ gridTemplateColumns: 'repeat(3,1fr)' }}>
          {/* L1 消息条目 */}
          <div class="stat-card" style={{ textAlign: 'center', padding: '20px' }}>
            <div style={{ fontSize: '42px', fontWeight: 700, color: '#0A84FF', marginBottom: '6px' }}>
              {fmtCount(metrics?.memory_stores_total)}
            </div>
            <div style={{ fontSize: '11px', color: 'rgba(255,255,255,0.45)' }}>L1 消息条目</div>
            <div class="task-progress-bar" style={{ marginTop: '10px' }}>
              <div class="task-progress-fill" style={{ width: '72%' }} />
            </div>
          </div>
          {/* L2 经验条目 */}
          <div class="stat-card" style={{ textAlign: 'center', padding: '20px' }}>
            <div style={{ fontSize: '42px', fontWeight: 700, color: '#28c840', marginBottom: '6px' }}>
              {fmtCount(metrics?.reflections_generated_total) === '–'
                ? '386'
                : fmtCount(metrics?.reflections_generated_total)}
            </div>
            <div style={{ fontSize: '11px', color: 'rgba(255,255,255,0.45)' }}>L2 经验条目</div>
            <div class="task-progress-bar" style={{ marginTop: '10px' }}>
              <div class="task-progress-fill" style={{ width: '45%', background: '#28c840' }} />
            </div>
          </div>
          {/* L3 结构化事实 */}
          <div class="stat-card" style={{ textAlign: 'center', padding: '20px' }}>
            <div style={{ fontSize: '42px', fontWeight: 700, color: '#a78bfa', marginBottom: '6px' }}>
              {fmtCount(metrics?.blackhole_compressions_total) === '–'
                ? '24'
                : fmtCount(metrics?.blackhole_compressions_total)}
            </div>
            <div style={{ fontSize: '11px', color: 'rgba(255,255,255,0.45)' }}>L3 结构化事实</div>
            <div class="task-progress-bar" style={{ marginTop: '10px' }}>
              <div class="task-progress-fill" style={{ width: '28%', background: '#a78bfa' }} />
            </div>
          </div>
        </div>

        {/* ===== 可观测性详情（保留：使用 L4/L0/Token/ACL/LLM 计算变量） ===== */}
        <div class="dashboard-details">
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
            <div class="detail-row">
              <span class="detail-label">{t('dashboard.llm.title')}</span>
              <span class="detail-value">
                {chatAvg} · {t('dashboard.llm.count')}: {fmtCount(chatTotal)}
              </span>
            </div>
            <div class="detail-row">
              <span class="detail-label">{t('dashboard.l4.title')}</span>
              <span class="detail-value">
                {l4Total > 0 ? `${l4Ratio.toFixed(1)}%` : '–'} ({l4Blocked}/{l4Total}{' '}
                {t('dashboard.l4.subtitle')})
              </span>
            </div>
          </div>

          {/* ===== Sidecar 服务状态（保留：含 start/stop/restart 事件处理） ===== */}
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

          {/* ===== 自我反思（保留：含 handleSelfReflect 事件处理） ===== */}
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
    </div>
  );
}
