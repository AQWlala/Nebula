/**
 * v2.2: 状态栏重构。
 *
 * 优化点：
 * - 隐藏 "–" 空值指标（缓存/检索/对话延迟无数据时不渲染，避免占位噪音）
 * - 左侧核心指标（模式/模型/内存）+ 右侧操作按钮（浮动/悬浮球）分区布局
 * - 模型状态用圆点指示器替代文字，更紧凑
 * - 记忆数仅在 > 0 时显示
 */
import { useEffect, useState } from 'preact/hooks';
import { signal } from '@preact/signals';
import { nebulaStore } from '../stores/nebulaStore';
import { t } from '../i18n';
import { invokeTauri, nebulaAPI, type MetricsSnapshot } from '../lib/tauri';
import { toast } from './Toast';

interface PerfSample {
  rss_bytes?: number | null;
  over_budget?: boolean;
  ts_ms?: number;
}

export const rssBudgetBytes = signal<number | null>(null);
export const startupMs = signal<number | null>(null);
export const llmOnline = signal<boolean>(true);

function fmtBytes(n: number | null | undefined): string {
  if (n == null) return '';
  if (n < 1024) return `${n} B`;
  const kb = n / 1024;
  if (kb < 1024) return `${kb.toFixed(0)} KB`;
  const mb = kb / 1024;
  return `${mb.toFixed(0)} MB`;
}

function fmtMs(us: number | null | undefined, count: number | null | undefined): string {
  if (!count || count === 0) return '';
  const avgMs = (us ?? 0) / count / 1000;
  if (avgMs < 1) return `${(avgMs * 1000).toFixed(0)}µs`;
  if (avgMs < 1000) return `${avgMs.toFixed(0)}ms`;
  return `${(avgMs / 1000).toFixed(2)}s`;
}

function fmtRatio(hits: number | null | undefined, misses: number | null | undefined): string {
  const h = hits ?? 0;
  const m = misses ?? 0;
  const total = h + m;
  if (total === 0) return '';
  const r = (h / total) * 100;
  return `${r.toFixed(0)}%`;
}

export function StatusBar() {
  const [perf, setPerf] = useState<PerfSample | null>(null);
  const [metrics, setMetrics] = useState<MetricsSnapshot | null>(null);
  const memCount = nebulaStore.recentMemories.value.length;
  const mode = nebulaStore.mode.value;
  const online = llmOnline.value;
  const rssOver = rssBudgetBytes.value != null && (perf?.rss_bytes ?? 0) > rssBudgetBytes.value;

  useEffect(() => {
    const ac = new AbortController();
    async function tick() {
      try {
        const [perfSample, metricsSnap] = await Promise.all([
          invokeTauri<PerfSample>('perf_sample'),
          invokeTauri<MetricsSnapshot>('metrics'),
        ]);
        if (!ac.signal.aborted) {
          if (perfSample) setPerf(perfSample);
          if (metricsSnap) setMetrics(metricsSnap);
        }
      } catch {
        /* ignore */
      }
    }
    tick();
    const id = setInterval(tick, 2000);
    return () => {
      ac.abort();
      clearInterval(id);
    };
  }, []);

  /** 打开浮动聊天窗口 (PIP)。 */
  async function openFloatingChat() {
    try {
      await nebulaAPI.floatingChatOpen();
    } catch (e) {
      toast.error(t('statusBar.floatingChatFailed'), String(e));
    }
  }

  /** 打开 / toggle 桌面悬浮球。 */
  async function openFloatingBall() {
    try {
      await nebulaAPI.floatingBallOpen();
    } catch (e) {
      toast.error(t('statusBar.floatingBallFailed'), String(e));
    }
  }

  // v2.2: 次要指标仅在有实际数据时才计算，空字符串则不渲染。
  const rssText = fmtBytes(perf?.rss_bytes ?? null);
  const cacheText = fmtRatio(metrics?.embedding_cache_hits, metrics?.embedding_cache_misses);
  const searchText = fmtMs(metrics?.memory_search_latency_us_total, metrics?.memory_search_latency_count);
  const chatText = fmtMs(metrics?.llm_chat_latency_us_total, metrics?.llm_chat_latency_count);

  return (
    <footer class="statusbar" role="status" aria-live="polite">
      {/* 左侧：核心指标 */}
      <span class="sb-item">
        <span class="sb-key">{t('statusbar.mode')}</span>
        <span class="sb-val">{t(`mode.${mode}`)}</span>
      </span>
      <span class="sb-divider" />
      <span class="sb-item">
        <span class={`sb-dot ${online ? 'ok' : 'off'}`} />
        <span class="sb-key">{t('statusbar.llm')}</span>
        <span class={`sb-val ${online ? 'ok' : 'off'}`}>
          {online ? t('statusbar.llm.online') : t('statusbar.llm.offline')}
        </span>
      </span>
      {rssText && (
        <>
          <span class="sb-divider" />
          <span class={`sb-item ${rssOver ? 'warn' : ''}`}>
            <span class="sb-key">{t('statusbar.memory')}</span>
            <span class="sb-val">
              {rssText}
              {rssOver ? ` (${t('statusbar.rss.over')})` : ''}
            </span>
          </span>
        </>
      )}
      {memCount > 0 && (
        <>
          <span class="sb-divider" />
          <span class="sb-item">
            <span class="sb-key">{t('statusbar.memories')}</span>
            <span class="sb-val">{memCount}</span>
          </span>
        </>
      )}
      {startupMs.value != null && (
        <>
          <span class="sb-divider" />
          <span class="sb-item">
            <span class="sb-key">{t('statusbar.startup')}</span>
            <span class="sb-val">{(startupMs.value / 1000).toFixed(1)}s</span>
          </span>
        </>
      )}

      {/* 次要指标：仅有数据时显示 */}
      {cacheText && (
        <>
          <span class="sb-divider" />
          <span class="sb-item sb-secondary">
            <span class="sb-key">{t('statusbar.cache')}</span>
            <span class="sb-val">{cacheText}</span>
          </span>
        </>
      )}
      {searchText && (
        <>
          <span class="sb-divider" />
          <span class="sb-item sb-secondary">
            <span class="sb-key">{t('statusbar.search')}</span>
            <span class="sb-val">{searchText}</span>
          </span>
        </>
      )}
      {chatText && (
        <>
          <span class="sb-divider" />
          <span class="sb-item sb-secondary">
            <span class="sb-key">{t('statusbar.chat')}</span>
            <span class="sb-val">{chatText}</span>
          </span>
        </>
      )}

      {/* 右侧：操作按钮 */}
      <button
        class="sb-item sb-floating-btn"
        title={t('statusBar.openFloatingChat')}
        onClick={() => void openFloatingChat()}
      >
        🪟 {t('statusBar.floating')}
      </button>
      <button
        class="sb-item sb-floating-btn"
        title={t('nav.floatingBall')}
        onClick={() => void openFloatingBall()}
      >
        🌀 {t('nav.floatingBall')}
      </button>
    </footer>
  );
}
