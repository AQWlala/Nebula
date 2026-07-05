/**
 * 记忆检视器 - 查看 / 搜索记忆
 */
import { useEffect, useState } from 'preact/hooks';
import { nebulaAPI, type Memory, type Layer, type WikiNote } from '../lib/tauri';
import { Spinner } from './Spinner';
import { t } from '../i18n';

export function MemoryInspector() {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Memory[]>([]);
  const [loading, setLoading] = useState(false);

  // T-E-B-03: Wiki 笔记双向同步状态。
  // wikiNotes — 顶部笔记列表(展示标题 + 标签 + 编辑入口)
  // editingNoteId — 当前正在编辑的笔记 UUID(null = 非编辑态)
  // editText — textarea 当前文本(编辑态初始为笔记当前 body)
  // editSaving — 保存按钮 loading 状态(防止重复点击)
  // editError — 保存失败时的错误信息(供 UI 显示)
  const [wikiNotes, setWikiNotes] = useState<WikiNote[]>([]);
  const [wikiLoading, setWikiLoading] = useState(false);
  const [editingNoteId, setEditingNoteId] = useState<string | null>(null);
  const [editText, setEditText] = useState('');
  const [editSaving, setEditSaving] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);

  useEffect(() => {
    loadRecent();
    loadWikiNotes();
  }, []);

  async function loadRecent() {
    setLoading(true);
    try {
      setResults(await nebulaAPI.memoryListRecent(30));
    } finally {
      setLoading(false);
    }
  }

  async function loadWikiNotes() {
    setWikiLoading(true);
    try {
      setWikiNotes(await nebulaAPI.wikiList(20, 0));
    } catch (e) {
      console.error('load wiki notes failed', e);
    } finally {
      setWikiLoading(false);
    }
  }

  async function search() {
    if (!query.trim()) {
      await loadRecent();
      return;
    }
    setLoading(true);
    try {
      // v1.0.1 fix: `memorySearch` returns `SearchResponse` (hits
      // wrapper), not a flat `Memory[]`.  Flatten via `.hits[*].memory`.
      const resp = await nebulaAPI.memorySearch({ query, k: 30 });
      setResults(resp.hits.map((h) => h.memory));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }

  /**
   * T-E-B-03: 进入编辑态。
   *
   * 通过 `wikiRead(id)` 拉取笔记当前 body(markdown 正文)作为
   * textarea 初始值。失败则用空字符串占位,不阻断进入编辑态
   * (用户仍可输入新内容覆盖)。
   */
  async function startEditNote(note: WikiNote) {
    setEditingNoteId(note.id);
    setEditError(null);
    try {
      const resp = await nebulaAPI.wikiRead(note.id);
      setEditText(resp.markdown);
    } catch (e) {
      console.error('wikiRead failed, fallback to empty', e);
      setEditText('');
    }
  }

  function cancelEditNote() {
    setEditingNoteId(null);
    setEditText('');
    setEditError(null);
  }

  /**
   * T-E-B-03: 保存编辑。
   *
   * 调用 `wikiUpdateFromUser(noteId, newBody)`,后端执行:
   * SQLite UPDATE → sponge.absorb_text → 文件重写 → vc.commit → LogEvent::Updated。
   * 成功后退出编辑态并刷新笔记列表(显示新 updated_at)。
   * 失败不退出编辑态(保留用户输入),仅显示错误供修正后重试。
   */
  async function saveEditNote() {
    if (editingNoteId === null) return;
    setEditSaving(true);
    setEditError(null);
    try {
      await nebulaAPI.wikiUpdateFromUser(editingNoteId, editText);
      setEditingNoteId(null);
      setEditText('');
      await loadWikiNotes();
    } catch (e) {
      console.error('wikiUpdateFromUser failed', e);
      setEditError(e instanceof Error ? e.message : String(e));
    } finally {
      setEditSaving(false);
    }
  }

  return (
    <div class="panel">
      <div class="panel-header">
        <span class="panel-title">{t('memoryInspector.title')}</span>
        <span style="color: var(--text-muted); font-size: 12px;">
          {t('memoryInspector.subtitle')}
        </span>
      </div>

      <div class="memory-search" style="display: flex; gap: 8px; margin-bottom: 16px;">
        <input
          type="text"
          placeholder={t('memoryInspector.searchPlaceholder')}
          value={query}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => e.key === 'Enter' && search()}
          style="flex: 1;"
        />
        <button class="btn" onClick={search} disabled={loading}>
          {loading ? <Spinner size={16} showLabel={false} /> : t('memoryInspector.search')}
        </button>
        <button class="btn" onClick={loadRecent}>{t('memoryInspector.recent')}</button>
      </div>

      <div class="memory-list">
        {results.length === 0 && (
          <div style="text-align: center; color: var(--text-muted); padding: 40px;">
            <div style="font-size: 48px; margin-bottom: 16px;">🧠</div>
            <div>{t('memoryInspector.empty')}</div>
            <div style="font-size: 12px; margin-top: 8px;">{t('memoryInspector.emptyHint')}</div>
          </div>
        )}

        {results.map((m) => (
          <MemoryCard key={m.id} memory={m} />
        ))}
      </div>

      {/* T-E-B-03: Wiki 笔记双向同步 — 编辑入口 */}
      <div class="wiki-section" style="margin-top: 24px; border-top: 1px solid var(--border-color, #333); padding-top: 16px;">
        <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 12px;">
          <span style="font-size: 14px; font-weight: 600;">{t('memoryInspector.wikiTitle')}</span>
          <span style="color: var(--text-muted); font-size: 11px;">
            {t('memoryInspector.wikiHint')}
          </span>
          <button
            class="btn"
            onClick={loadWikiNotes}
            disabled={wikiLoading}
            style="margin-left: auto;"
          >
            {wikiLoading ? t('memoryInspector.refreshing') : t('memoryInspector.refresh')}
          </button>
        </div>

        {wikiNotes.length === 0 && !wikiLoading && (
          <div style="text-align: center; color: var(--text-muted); padding: 20px; font-size: 12px;">
            {t('memoryInspector.wikiEmpty')}
          </div>
        )}

        {wikiNotes.map((note) => {
          const isEditing = editingNoteId === note.id;
          return (
            <div class="card memory-card" key={note.id} style="margin-bottom: 8px;">
              <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 8px;">
                <span style="font-weight: 600; font-size: 13px;">{note.title}</span>
                {note.tags.length > 0 && (
                  <span style="color: var(--text-muted); font-size: 11px;">
                    {note.tags.map((t2) => `#${t2}`).join(' ')}
                  </span>
                )}
                <span style="margin-left: auto; color: var(--text-muted); font-size: 11px;">
                  {new Date(note.updated_at).toLocaleString('zh-CN')}
                </span>
              </div>

              {isEditing ? (
                <div style="display: flex; flex-direction: column; gap: 8px;">
                  <textarea
                    value={editText}
                    onInput={(e) => setEditText((e.target as HTMLTextAreaElement).value)}
                    rows={8}
                    placeholder={t('memoryInspector.editPlaceholder')}
                    style="width: 100%; box-sizing: border-box; padding: 8px; font-family: monospace; font-size: 12px; resize: vertical; min-height: 120px;"
                  />
                  {editError && (
                    <div style="color: #ff6b6b; font-size: 11px;">{t('memoryInspector.saveFailed', { error: editError })}</div>
                  )}
                  <div style="display: flex; gap: 8px;">
                    <button
                      class="btn"
                      onClick={saveEditNote}
                      disabled={editSaving}
                    >
                      {editSaving ? <Spinner size={16} showLabel={false} /> : t('memoryInspector.save')}
                    </button>
                    <button
                      class="btn"
                      onClick={cancelEditNote}
                      disabled={editSaving}
                    >
                      {t('memoryInspector.cancel')}
                    </button>
                  </div>
                </div>
              ) : (
                <div style="display: flex; gap: 8px;">
                  <button
                    class="btn"
                    onClick={() => startEditNote(note)}
                    style="font-size: 11px;"
                  >
                    {t('memoryInspector.edit')}
                  </button>
                  <a
                    href={`#wiki/${note.slug}`}
                    style="font-size: 11px; color: var(--text-muted); align-self: center;"
                  >
                    {note.slug}.md
                  </a>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function MemoryCard({ memory }: { memory: Memory }) {
  const date = new Date(memory.created_at * 1000).toLocaleString('zh-CN');
  return (
    <div class="card memory-card">
      <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 8px;">
        <span class={`badge badge-${memory.layer.toLowerCase()}`}>{memory.layer}</span>
        <span class={`badge badge-${memory.memory_type.toLowerCase()}`}>{memory.memory_type}</span>
        {memory.pinned && <span class="badge" style="background: #ffd66d; color: #000;">📌 L7</span>}
        {memory.compressed_from && <span class="badge" style="background: #5f3a3a; color: #ff9c9c;">{t('memoryInspector.compressed')}</span>}
        {/* T-E-B-04: 记忆溯源 [来源:工具] badge */}
        {memory.metadata?.provenance && (
          <span class="badge" style="background: #3b5998; color: #fff;">
            {t('memoryInspector.provenance', {
              source: (() => {
                const p = memory.metadata.provenance as { tool?: string; source?: string };
                return p.tool ?? p.source ?? '';
              })(),
            })}
          </span>
        )}
        {/* T-E-A-09: 吸收成本 badge(仅非零时显示) */}
        {memory.ingest_cost != null && memory.ingest_cost > 0 && (
          <span class="badge" style="background: #2e7d32; color: #fff;" title={t('memoryInspector.ingestCost')}>
            💰 ${memory.ingest_cost.toFixed(4)}
          </span>
        )}
        <span style="margin-left: auto; color: var(--text-muted); font-size: 11px;">{date}</span>
      </div>
      <div style="font-size: 13px; margin-bottom: 8px;">{memory.content}</div>
      <div style="display: flex; gap: 12px; color: var(--text-muted); font-size: 11px;">
        <span>{t('memoryInspector.importance', { value: memory.importance.toFixed(2) })}</span>
        <span>{t('memoryInspector.access', { count: memory.access_count })}</span>
      </div>
    </div>
  );
}
