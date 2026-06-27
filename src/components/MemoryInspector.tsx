/**
 * 记忆检视器 - 查看 / 搜索记忆
 */
import { useEffect, useState } from 'preact/hooks';
import { NineSnakeAPI, type Memory, type Layer } from '../lib/tauri';

export function MemoryInspector() {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Memory[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    loadRecent();
  }, []);

  async function loadRecent() {
    setLoading(true);
    try {
      setResults(await NineSnakeAPI.memoryListRecent(30));
    } finally {
      setLoading(false);
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
      const resp = await NineSnakeAPI.memorySearch({ query, limit: 30 });
      setResults(resp.hits.map((h) => h.memory));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div class="panel">
      <div class="panel-header">
        <span class="panel-title">🧠 记忆系统</span>
        <span style="color: var(--text-muted); font-size: 12px;">
          v7.0 8 层架构 · 黑洞压缩 · 海绵吸收
        </span>
      </div>

      <div class="memory-search" style="display: flex; gap: 8px; margin-bottom: 16px;">
        <input
          type="text"
          placeholder="搜索记忆（向量检索）..."
          value={query}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => e.key === 'Enter' && search()}
          style="flex: 1;"
        />
        <button class="btn" onClick={search} disabled={loading}>
          {loading ? '搜索中…' : '🔍 搜索'}
        </button>
        <button class="btn" onClick={loadRecent}>最近</button>
      </div>

      <div class="memory-list">
        {results.length === 0 && (
          <div style="text-align: center; color: var(--text-muted); padding: 40px;">
            <div style="font-size: 48px; margin-bottom: 16px;">🧠</div>
            <div>暂无记忆</div>
            <div style="font-size: 12px; margin-top: 8px;">开始对话后，记忆会被自动写入</div>
          </div>
        )}

        {results.map((m) => (
          <MemoryCard key={m.id} memory={m} />
        ))}
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
        {memory.compressed_from && <span class="badge" style="background: #5f3a3a; color: #ff9c9c;">已压缩</span>}
        <span style="margin-left: auto; color: var(--text-muted); font-size: 11px;">{date}</span>
      </div>
      <div style="font-size: 13px; margin-bottom: 8px;">{memory.content}</div>
      <div style="display: flex; gap: 12px; color: var(--text-muted); font-size: 11px;">
        <span>重要性：{memory.importance.toFixed(2)}</span>
        <span>访问：{memory.access_count} 次</span>
      </div>
    </div>
  );
}
