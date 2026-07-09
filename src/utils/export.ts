import type { Message } from '../components/ChatPanel';
import { t } from '../i18n';

export interface ExportOptions {
  format: 'markdown' | 'docx' | 'pdf';
  includeToolCalls: boolean;
  includeTimestamps: boolean;
  title?: string;
}

function formatTimestamp(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => n.toString().padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function exportToMarkdown(messages: Message[], options: ExportOptions): string {
  const title = options.title || t('export.defaultTitle');
  const now = new Date();
  const lines: string[] = [];

  lines.push(`# ${title}`);
  lines.push('');
  lines.push(`${t('export.exportTime')}: ${formatTimestamp(now.getTime())}`);
  lines.push(`${t('export.messageCount')}: ${messages.length}`);
  lines.push('');
  lines.push('---');
  lines.push('');

  for (const msg of messages) {
    const roleLabel = msg.role === 'user' ? 'user' : 'assistant';
    lines.push(`## ${roleLabel}`);
    if (options.includeTimestamps && msg.timestamp > 0) {
      lines.push('');
      lines.push(`*${formatTimestamp(msg.timestamp)}*`);
    }
    lines.push('');
    lines.push(msg.content);
    lines.push('');
    lines.push('---');
    lines.push('');
  }

  return lines.join('\n');
}

export function downloadMarkdown(content: string, filename?: string): void {
  const blob = new Blob([content], { type: 'text/markdown;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename || `chat-export-${Date.now()}.md`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

export function printToPdf(messages: Message[], options: ExportOptions): void {
  const title = options.title || t('export.defaultTitle');
  const now = new Date();

  const msgHtml = messages
    .map((msg) => {
      const roleLabel = msg.role === 'user' ? t('export.roleUser') : t('export.roleAssistant');
      const roleColor = msg.role === 'user' ? '#2196f3' : '#4caf50';
      const ts =
        options.includeTimestamps && msg.timestamp > 0
          ? `<div style="font-size:12px;color:#888;margin-bottom:4px;">${formatTimestamp(msg.timestamp)}</div>`
          : '';
      return `
        <div style="margin-bottom:20px;padding:12px;border-radius:8px;background:${msg.role === 'user' ? 'rgba(33,150,243,0.05)' : 'rgba(76,175,80,0.05)'};border-left:4px solid ${roleColor};">
          <div style="font-weight:bold;color:${roleColor};margin-bottom:4px;">${roleLabel}</div>
          ${ts}
          <div style="white-space:pre-wrap;line-height:1.6;">${escapeHtml(msg.content)}</div>
        </div>
      `;
    })
    .join('');

  const html = `
    <!DOCTYPE html>
    <html>
    <head>
      <meta charset="UTF-8">
      <title>${title}</title>
      <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; padding: 40px; max-width: 800px; margin: 0 auto; color: #333; }
        h1 { color: #222; border-bottom: 2px solid #eee; padding-bottom: 12px; }
        .meta { color: #666; margin-bottom: 30px; font-size: 14px; }
      </style>
    </head>
    <body>
      <h1>${title}</h1>
      <div class="meta">
        <div>${t('export.exportTime')}: ${formatTimestamp(now.getTime())}</div>
        <div>${t('export.messageCount')}: ${messages.length}</div>
      </div>
      ${msgHtml}
    </body>
    </html>
  `;

  const printWindow = window.open('', '_blank');
  if (printWindow) {
    printWindow.document.write(html);
    printWindow.document.close();
    printWindow.focus();
    setTimeout(() => {
      printWindow.print();
    }, 200);
  }
}

function escapeHtml(text: string): string {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}
