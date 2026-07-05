/**
 * v0.5: Writing 模式
 *
 * 左侧：模板库 + 文档列表
 * 右侧：编辑器（粗略字数 / 阅读时间 / 自动保存状态）
 * 顶部工具栏：保存 / 导出 Markdown / 导出 HTML / 删除
 *
 * 模板选择 → 占位符填表 → 一键创建文档
 * 编辑过程中自动保存到后端（每 1.5s 防抖），
 * 后端同时把内容镜像到 L3 记忆（见 writing/mod.rs）。
 */
import { useEffect, useMemo, useState } from 'preact/hooks';
import { marked } from 'marked';
import {
  nebulaAPI,
  type Document,
  type WritingTemplate,
} from '../lib/tauri';
import { t } from '../i18n';

const SAVE_DEBOUNCE_MS = 1500;

export function WritingMode() {
  const [templates, setTemplates] = useState<WritingTemplate[]>([]);
  const [documents, setDocuments] = useState<Document[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [templateId, setTemplateId] = useState<string>('tech-blog');
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle');
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<'edit' | 'preview'>('edit');

  // 初始加载
  useEffect(() => {
    nebulaAPI.writingListTemplates().then(setTemplates).catch((e) => setError(String(e)));
    refreshDocuments();
  }, []);

  // 自动保存（防抖）
  useEffect(() => {
    if (!currentId) return;
    if (!content && !title) return;
    setSaveState('saving');
    const t = setTimeout(async () => {
      try {
        await nebulaAPI.writingUpdateDocument(currentId, content);
        setSaveState('saved');
        // 后端 word_count 已经更新，刷新列表里那一行
        refreshDocuments();
      } catch (e) {
        setSaveState('error');
        setError(String(e));
      }
    }, SAVE_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [currentId, content]);

  const refreshDocuments = async () => {
    try {
      const docs = await nebulaAPI.writingListDocuments(50);
      setDocuments(docs);
    } catch (e) {
      console.error('refreshDocuments failed:', e);
    }
  };

  const currentTemplate = useMemo(
    () => templates.find((t) => t.id === templateId) ?? null,
    [templates, templateId],
  );

  const wordCount = useMemo(() => countWords(content), [content]);
  const readMinutes = Math.max(1, Math.round(wordCount / 300));

  const onNewFromTemplate = async () => {
    if (!currentTemplate) return;
    const values: Record<string, string> = {};
    for (const p of currentTemplate.placeholders) {
      const v = prompt(`${p.hint}${p.multiline ? t('writingMode.multilineHint') : ''}`);
      if (v === null) return;
      values[p.name] = v;
    }
    let body = currentTemplate.body;
    for (const [k, v] of Object.entries(values)) {
      body = body.split(`{{${k}}}`).join(v);
    }
    const finalTitle = values.title || currentTemplate.label;
    try {
      const doc = await nebulaAPI.writingCreateDocument({
        title: finalTitle,
        template_id: currentTemplate.id,
        content: body,
        metadata: { from_template: currentTemplate.id, created_via: 'ui' },
      });
      await refreshDocuments();
      setCurrentId(doc.id);
      setTitle(doc.title);
      setContent(doc.content);
      setTemplateId(doc.template_id);
      setSaveState('saved');
    } catch (e) {
      setError(String(e));
    }
  };

  const onNewBlank = async () => {
    try {
      const doc = await nebulaAPI.writingCreateDocument({
        title: t('writingMode.untitled'),
        template_id: 'blank',
        content: t('writingMode.defaultContent'),
        metadata: null,
      });
      await refreshDocuments();
      setCurrentId(doc.id);
      setTitle(doc.title);
      setContent(doc.content);
      setTemplateId(doc.template_id);
    } catch (e) {
      setError(String(e));
    }
  };

  const onOpen = (doc: Document) => {
    setCurrentId(doc.id);
    setTitle(doc.title);
    setContent(doc.content);
    setTemplateId(doc.template_id);
    setSaveState('saved');
  };

  const onDelete = async (id: string) => {
    if (!confirm(t('writingMode.confirmDelete'))) return;
    try {
      await nebulaAPI.writingDeleteDocument(id);
      if (currentId === id) {
        setCurrentId(null);
        setTitle('');
        setContent('');
      }
      await refreshDocuments();
    } catch (e) {
      setError(String(e));
    }
  };

  const onExport = async (format: 'markdown' | 'html') => {
    if (!currentId) return;
    try {
      const exp = await nebulaAPI.writingExport(currentId, format);
      // 复制到剪贴板（如果可用）+ 浏览器下载
      try {
        await nebulaAPI.osClipboardWrite(exp.body);
      } catch {
        // 剪贴板不可用时不阻塞
      }
      const blob = new Blob([exp.body], {
        type: format === 'html' ? 'text/html' : 'text/markdown',
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${sanitizeFilename(title || 'document')}.${format === 'html' ? 'html' : 'md'}`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div class="writing-mode">
      <aside class="writing-side">
        <section class="writing-templates">
          <h3>{t('writingMode.templates')}</h3>
          <div class="template-grid">
            {templates.map((tpl) => (
              <button
                key={tpl.id}
                class={`template-card ${templateId === tpl.id ? 'active' : ''}`}
                onClick={() => setTemplateId(tpl.id)}
                title={tpl.description}
              >
                <div class="tpl-icon">{tpl.icon}</div>
                <div class="tpl-label">{tpl.label}</div>
                <div class="tpl-desc">{tpl.description}</div>
              </button>
            ))}
          </div>
          <button class="primary" disabled={!currentTemplate} onClick={onNewFromTemplate}>
            {t('writingMode.newFromTemplate')}
          </button>
          <button class="ghost" onClick={onNewBlank}>
            {t('writingMode.newBlank')}
          </button>
        </section>

        <section class="writing-docs">
          <h3>{t('writingMode.myDocs')}</h3>
          {documents.length === 0 ? (
            <p class="empty">{t('writingMode.noDocs')}</p>
          ) : (
            <ul>
              {documents.map((d) => (
                <li
                  key={d.id}
                  class={currentId === d.id ? 'active' : ''}
                  onClick={() => onOpen(d)}
                >
                  <div class="doc-title">{d.title}</div>
                  <div class="doc-meta">
                    {d.word_count} {t('writingMode.words')} · {tplLabel(d.template_id, templates)}
                  </div>
                  <button class="doc-del" onClick={(e) => { e.stopPropagation(); onDelete(d.id); }}>
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </aside>

      <main class="writing-main">
        {currentId ? (
          <>
            <header class="writing-toolbar">
              <input
                class="doc-title-input"
                value={title}
                onInput={(e) => setTitle((e.target as HTMLInputElement).value)}
                placeholder={t('writingMode.titlePlaceholder')}
              />
              <span class={`save-state save-${saveState}`}>
                {saveState === 'idle' && '—'}
                {saveState === 'saving' && t('writingMode.saving')}
                {saveState === 'saved' && t('writingMode.saved')}
                {saveState === 'error' && t('writingMode.saveError')}
              </span>
              <div class="spacer" />
              <span class="stats">{t('writingMode.wordCount', { count: wordCount, minutes: readMinutes })}</span>
              <button onClick={() => onExport('markdown')}>{t('writingMode.exportMd')}</button>
              <button onClick={() => onExport('html')}>{t('writingMode.exportHtml')}</button>
            </header>
            <div class="writing-tabs">
              <button class={tab === 'edit' ? 'active' : ''} onClick={() => setTab('edit')}>
                {t('writingMode.edit')}
              </button>
              <button class={tab === 'preview' ? 'active' : ''} onClick={() => setTab('preview')}>
                {t('writingMode.preview')}
              </button>
            </div>
            {tab === 'edit' ? (
              <textarea
                class="doc-editor"
                value={content}
                onInput={(e) => setContent((e.target as HTMLTextAreaElement).value)}
                placeholder={t('writingMode.editorPlaceholder')}
              />
            ) : (
              <article
                class="doc-preview"
                // marked is XSS-safe for our use; the user owns the content.
                // eslint-disable-next-line react/no-danger
                dangerouslySetInnerHTML={{ __html: marked.parse(content) as string }}
              />
            )}
          </>
        ) : (
          <div class="writing-empty">
            <h2>{t('writingMode.emptyTitle')}</h2>
            <p>{t('writingMode.emptySubtitle1')}</p>
            <p>{t('writingMode.emptySubtitle2')}</p>
            {error && <p class="error">{error}</p>}
          </div>
        )}
      </main>
    </div>
  );
}

function countWords(s: string): number {
  let n = 0;
  let inLatin = false;
  for (const ch of s) {
    if (/\s/.test(ch)) inLatin = false;
    else if (/[a-zA-Z0-9]/.test(ch)) {
      if (!inLatin) {
        n++;
        inLatin = true;
      }
    } else {
      n++;
      inLatin = false;
    }
  }
  return n;
}

function sanitizeFilename(s: string): string {
  return s.replace(/[\\/:*?"<>|]/g, '_').slice(0, 80) || 'document';
}

function tplLabel(id: string, all: WritingTemplate[]): string {
  if (id === 'blank') return t('writingMode.blank');
  return all.find((tpl) => tpl.id === id)?.label ?? id;
}
