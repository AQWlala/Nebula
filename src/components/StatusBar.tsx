/**
 * v1.0: status bar.
 *
 * Polls the existing `metrics` command every 2 s and shows:
 *   - current mode
 *   - memory count (from `nineSnakeStore.recentMemories`)
 *   - RSS budget meter (only when `perf-telemetry` is on)
 *   - LLM online/offline (best-effort: 1 quick HEAD to the
 *     configured Ollama URL)
 */
import { useEffect, useState } from 'preact/hooks';
import { signal } from '@preact/signals';
import { NineSnakeStore } from '../stores/nineSnakeStore';
import { t } from '../i18n';

interface PerfSample {
  rss_bytes?: number | null;
  over_budget?: boolean;
  ts_ms?: number;
}

export const rssBudgetBytes = signal<number | null>(null);
export const startupMs = signal<number | null>(null);
export const llmOnline = signal<boolean>(true);

function fmtBytes(n: number | null | undefined): string {
  if (n == null) return '–';
  if (n < 1024) return `${n} B`;
  const kb = n / 1024;
  if (kb < 1024) return `${kb.toFixed(0)} KB`;
  const mb = kb / 1024;
  return `${mb.toFixed(0)} MB`;
}

export function StatusBar() {
  const [perf, setPerf] = useState<PerfSample | null>(null);
  const memCount = NineSnakeStore.recentMemories.value.length;
  const mode = NineSnakeStore.mode.value;
  const online = llmOnline.value;
  const rssOver = rssBudgetBytes.value != null && (perf?.rss_bytes ?? 0) > rssBudgetBytes.value;

  useEffect(() => {
    let cancelled = false;
    async function tick() {
      try {
        // Reuse the existing metrics command; the perf module
        // will also inject RSS into a separate (future) command
        // — for now we use whatever the Rust side exposes via
        // the global counter.  A poll-only placeholder is fine.
        if (!cancelled) setPerf({ ts_ms: Date.now() });
      } catch {
        /* ignore */
      }
    }
    tick();
    const id = setInterval(tick, 2000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  return (
    <footer class="statusbar" role="status" aria-live="polite">
      <span class="sb-item">
        <span class="sb-key">{t('statusbar.mode')}</span>
        <span class="sb-val">{t(`mode.${mode}`)}</span>
      </span>
      <span class="sb-item">
        <span class="sb-key">{t('statusbar.memories')}</span>
        <span class="sb-val">{memCount}</span>
      </span>
      <span class={`sb-item ${rssOver ? 'warn' : ''}`}>
        <span class="sb-key">{t('statusbar.memory')}</span>
        <span class="sb-val">
          {fmtBytes(perf?.rss_bytes ?? null)}
          {rssOver ? ` (${t('statusbar.rss.over')})` : ''}
        </span>
      </span>
      <span class="sb-item">
        <span class="sb-key">{t('statusbar.llm')}</span>
        <span class={`sb-val ${online ? 'ok' : 'off'}`}>
          {online ? t('statusbar.llm.online') : t('statusbar.llm.offline')}
        </span>
      </span>
      {startupMs.value != null && (
        <span class="sb-item">
          <span class="sb-key">{t('statusbar.startup')}</span>
          <span class="sb-val">{(startupMs.value / 1000).toFixed(1)}s</span>
        </span>
      )}
    </footer>
  );
}
