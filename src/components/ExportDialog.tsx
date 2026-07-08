import { useState } from 'preact/hooks';
import type { Message } from './ChatPanel';
import {
  exportToMarkdown,
  downloadMarkdown,
  printToPdf,
  type ExportOptions,
} from '../utils/export';
import { nebulaAPI } from '../lib/tauri';
import { toast } from './Toast';
import { Modal } from './Modal';
import { Spinner } from './Spinner';
import { t } from '../i18n';

interface ExportDialogProps {
  messages: Message[];
  onClose: () => void;
}

export function ExportDialog({ messages, onClose }: ExportDialogProps) {
  const [format, setFormat] = useState<'markdown' | 'docx' | 'pdf'>('markdown');
  const [includeTimestamps, setIncludeTimestamps] = useState(false);
  const [includeToolCalls, setIncludeToolCalls] = useState(true);
  const [title, setTitle] = useState(t('exportDialog.defaultTitle'));
  const [exporting, setExporting] = useState(false);

  async function handleExport() {
    if (messages.length === 0) {
      toast.warning(t('exportDialog.noMessages'));
      return;
    }

    const options: ExportOptions = {
      format,
      includeToolCalls,
      includeTimestamps,
      title,
    };

    setExporting(true);
    try {
      if (format === 'markdown') {
        const md = exportToMarkdown(messages, options);
        downloadMarkdown(md, `${title}-${Date.now()}.md`);
        toast.success(t('exportDialog.exportSuccess'), t('exportDialog.markdownExported'));
        onClose();
      } else if (format === 'pdf') {
        printToPdf(messages, options);
        toast.info(t('exportDialog.printDialogOpened'), t('exportDialog.printHint'));
        onClose();
      } else if (format === 'docx') {
        try {
          const result = await nebulaAPI.exportChatDocx({
            messages: messages.map((m) => ({
              role: m.role,
              content: m.content,
              timestamp: m.timestamp,
            })),
            options: {
              title,
              include_timestamps: includeTimestamps,
            },
          });
          if (result?.file_path) {
            toast.success(
              t('exportDialog.exportSuccess'),
              t('exportDialog.docxSaved', { path: result.file_path })
            );
            onClose();
          } else {
            throw new Error(t('exportDialog.noFilePath'));
          }
        } catch (e) {
          toast.error(t('exportDialog.docxFailed'), String(e));
        }
      }
    } finally {
      setExporting(false);
    }
  }

  const footer = (
    <>
      <button class="btn btn-secondary" onClick={onClose} disabled={exporting}>
        {t('exportDialog.cancel')}
      </button>
      <button class="btn" onClick={handleExport} disabled={exporting || messages.length === 0}>
        {exporting ? <Spinner size={16} showLabel={false} /> : t('exportDialog.export')}
      </button>
    </>
  );

  return (
    <Modal open={true} title={t('exportDialog.title')} onClose={onClose} size="sm" footer={footer}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
        <div>
          <label
            style={{
              display: 'block',
              marginBottom: '6px',
              fontWeight: 500,
              color: 'var(--text-primary)',
            }}
          >
            {t('exportDialog.format')}
          </label>
          <div style={{ display: 'flex', gap: '8px' }}>
            {(['markdown', 'docx', 'pdf'] as const).map((f) => (
              <button
                key={f}
                onClick={() => setFormat(f)}
                style={{
                  flex: 1,
                  padding: '10px 12px',
                  borderRadius: '6px',
                  border: '1px solid',
                  borderColor: format === f ? 'var(--accent-neon)' : 'var(--border)',
                  background: format === f ? 'rgba(var(--accent-rgb), 0.1)' : 'transparent',
                  color: format === f ? 'var(--accent-neon)' : 'var(--text-secondary)',
                  cursor: 'pointer',
                  fontSize: '13px',
                  fontWeight: format === f ? 600 : 400,
                  transition: 'all 0.15s',
                }}
              >
                {f === 'markdown' ? 'Markdown' : f === 'docx' ? 'DOCX' : 'PDF'}
              </button>
            ))}
          </div>
        </div>

        <div>
          <label
            style={{
              display: 'block',
              marginBottom: '6px',
              fontWeight: 500,
              color: 'var(--text-primary)',
            }}
          >
            {t('exportDialog.titleLabel')}
          </label>
          <input
            type="text"
            value={title}
            onInput={(e) => setTitle((e.target as HTMLInputElement).value)}
            style={{
              width: '100%',
              padding: '8px 10px',
              borderRadius: '6px',
              border: '1px solid var(--border)',
              background: 'var(--bg-primary)',
              color: 'var(--text-primary)',
              fontSize: '13px',
              boxSizing: 'border-box',
            }}
          />
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
          <label
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '8px',
              cursor: 'pointer',
              color: 'var(--text-secondary)',
              fontSize: '13px',
            }}
          >
            <input
              type="checkbox"
              checked={includeTimestamps}
              onChange={(e) => setIncludeTimestamps((e.target as HTMLInputElement).checked)}
              style={{ cursor: 'pointer' }}
            />
            {t('exportDialog.includeTimestamps')}
          </label>
          <label
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '8px',
              cursor: 'pointer',
              color: 'var(--text-secondary)',
              fontSize: '13px',
            }}
          >
            <input
              type="checkbox"
              checked={includeToolCalls}
              onChange={(e) => setIncludeToolCalls((e.target as HTMLInputElement).checked)}
              style={{ cursor: 'pointer' }}
            />
            {t('exportDialog.includeToolCalls')}
          </label>
        </div>

        <div
          style={{
            fontSize: '12px',
            color: 'var(--text-muted)',
            background: 'var(--bg-tertiary)',
            padding: '8px 12px',
            borderRadius: '6px',
          }}
        >
          {t('exportDialog.messageCount', { count: messages.length })}
        </div>
      </div>
    </Modal>
  );
}
