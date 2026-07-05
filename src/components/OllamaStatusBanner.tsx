/**
 * v1.0.1 (P0#07): friendly "Ollama is down" banner.
 *
 * Rendered at the top of ChatPanel whenever
 * `nebulaStore.ollamaStatus.value === 'down'`.  Offers:
 *  - a link to the upstream project (https://ollama.com)
 *  - a "Retry" button that calls `checkOllama()` again
 *
 * Rendered as a normal DOM subtree, no portal — the panel already
 * stacks the banner above the message list.
 */
import { nebulaStore } from '../stores/nebulaStore';
import { t } from '../i18n';

const OLLAMA_URL = 'https://ollama.com';

export function OllamaStatusBanner() {
  const status = nebulaStore.ollamaStatus.value;
  if (status !== 'down') return null;

  function retry() {
    void nebulaStore.checkOllama();
  }

  return (
    <div
      class="ollama-banner"
      role="alert"
      aria-live="polite"
      data-testid="ollama-banner"
    >
      <div class="ollama-banner__body">
        <strong>{t('ollama.banner.title')}</strong>
        <span>{t('ollama.banner.body')}</span>
      </div>
      <div class="ollama-banner__actions">
        <a
          class="ollama-banner__link"
          href={OLLAMA_URL}
          target="_blank"
          rel="noreferrer noopener"
        >
          {t('ollama.banner.howto')} ↗
        </a>
        <button
          type="button"
          class="ollama-banner__retry"
          onClick={retry}
          data-testid="ollama-banner-retry"
        >
          {t('ollama.banner.retry')}
        </button>
      </div>
    </div>
  );
}
