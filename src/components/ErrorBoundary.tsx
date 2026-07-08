/**
 * v1.0: top-level error boundary.
 *
 * Preact 10 has no built-in componentDidCatch, so we wrap the
 * app in a thin class component.  Errors here are written to
 * `localStorage` (last 5) so the front-end can attach them to
 * the next crash report.  The recovery button unmounts the
 * failed subtree by bumping a key on `<App />`.
 */
import { Component, type ComponentChildren } from 'preact';
import { t } from '../i18n';

interface State {
  err: Error | null;
  info: string | null;
  fingerprint: string | null;
}

interface Props {
  children: ComponentChildren;
  onReload?: () => void;
}

const STORE_KEY = 'nebula.crashlog';

interface CrashEntry {
  ts: number;
  message: string;
  stack: string | null;
  fingerprint: string;
}

function recordCrash(entry: CrashEntry) {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    const list: CrashEntry[] = raw ? JSON.parse(raw) : [];
    list.unshift(entry);
    while (list.length > 5) list.pop();
    localStorage.setItem(STORE_KEY, JSON.stringify(list));
  } catch {
    /* ignore */
  }
}

export function readCrashLog(): CrashEntry[] {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { err: null, info: null, fingerprint: null };

  static getDerivedStateFromError(err: Error): Partial<State> {
    const fingerprint = `${err.name}|${(err.message || '').slice(0, 64)}`;
    return { err, fingerprint };
  }

  componentDidCatch(err: Error, info: unknown) {
    const infoStr =
      typeof info === 'string'
        ? info
        : (info as { componentStack?: string })?.componentStack || String(info);
    this.setState({ info: infoStr });
    recordCrash({
      ts: Date.now(),
      message: err.message,
      stack: err.stack || null,
      fingerprint: this.state.fingerprint || 'unknown',
    });
    // eslint-disable-next-line no-console
    console.error('[ErrorBoundary]', err, info);
  }

  reload = () => {
    this.setState({ err: null, info: null, fingerprint: null });
    if (this.props.onReload) this.props.onReload();
    else location.reload();
  };

  exportLog = () => {
    const err = this.state.err;
    if (!err) return;
    const ts = new Date().toISOString();
    const userAgent = typeof navigator !== 'undefined' ? navigator.userAgent : 'unknown';
    const platform = typeof navigator !== 'undefined' ? navigator.platform : 'unknown';
    const url = typeof location !== 'undefined' ? location.href : 'unknown';
    const logContent = [
      '# nebula crash log',
      '',
      '## Timestamp',
      ts,
      '',
      '## Error',
      `Name: ${err.name}`,
      `Message: ${err.message}`,
      '',
      '## Error Stack',
      err.stack || '(no stack)',
      '',
      '## Component Stack',
      this.state.info || '(no component stack)',
      '',
      '## Environment',
      `User Agent: ${userAgent}`,
      `Platform: ${platform}`,
      `URL: ${url}`,
      '',
    ].join('\n');
    const blob = new Blob([logContent], { type: 'text/plain' });
    const blobUrl = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = blobUrl;
    a.download = `nebula-crash-${Date.now()}.log`;
    a.click();
    URL.revokeObjectURL(blobUrl);
  };

  render() {
    if (!this.state.err) return this.props.children;
    return (
      <div class="error-boundary" role="alert">
        <h1>{t('errorBoundary.title')}</h1>
        <p>{t('errorBoundary.crash')}</p>
        <pre class="error-message">{this.state.err.message}</pre>
        {this.state.fingerprint && (
          <p class="error-fingerprint">
            {t('errorBoundary.fingerprint')} <code>{this.state.fingerprint}</code>
          </p>
        )}
        <details class="error-details">
          <summary>{t('errorBoundary.stack')}</summary>
          <pre>{this.state.err.stack || t('errorBoundary.noStack')}</pre>
        </details>
        <div class="error-actions">
          <button class="primary" onClick={this.reload}>
            {t('errorBoundary.reload')}
          </button>
          <button class="secondary" onClick={this.exportLog}>
            {t('errorBoundary.exportLog')}
          </button>
        </div>
      </div>
    );
  }
}
