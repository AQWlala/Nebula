/**
 * SkillPanel — v1.2 enhanced skill management
 *
 * Replaces the basic SkillMarketplace with:
 * - Full-text search + tag filtering
 * - Import skills from agentskills.io / ClawHub
 * - Skill detail inspection
 * - Quick-use with parameter input
 * - Visual status indicators
 */

import { useState, useEffect, useCallback } from 'preact/hooks';
import { NineSnakeAPI, Skill, ListSkillsRequest, ImportResult } from '../lib/tauri';

type Tab = 'browse' | 'import' | 'detail';

export default function SkillPanel() {
  const [tab, setTab] = useState<Tab>('browse');
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');
  const [tagFilter, setTagFilter] = useState<string | null>(null);
  const [selectedSkill, setSelectedSkill] = useState<Skill | null>(null);

  // Import state
  const [importUrl, setImportUrl] = useState('');
  const [importSource, setImportSource] = useState<string>('url');
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);

  // Load skills
  const loadSkills = useCallback(async () => {
    setLoading(true);
    try {
      const req: ListSkillsRequest = {};
      if (tagFilter) req.tag = tagFilter;
      const result = await NineSnakeAPI.skillList(req);
      setSkills(result || []);
    } finally {
      setLoading(false);
    }
  }, [tagFilter]);

  useEffect(() => { loadSkills(); }, [loadSkills]);

  // Filtered skills
  const filtered = skills.filter(s => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return (
      s.name.toLowerCase().includes(q) ||
      s.description.toLowerCase().includes(q) ||
      s.tags.some(t => t.toLowerCase().includes(q))
    );
  });

  // All unique tags
  const allTags = [...new Set(skills.flatMap(s => s.tags))].sort();

  // Handle import
  const handleImport = async () => {
    if (!importUrl.trim()) return;
    setImporting(true);
    setImportResult(null);
    try {
      const result = await NineSnakeAPI.skillImport(importUrl.trim(), importSource);
      setImportResult(result);
      if (result.success) {
        await loadSkills();
      }
    } finally {
      setImporting(false);
    }
  };

  // Open detail
  const openDetail = (skill: Skill) => {
    setSelectedSkill(skill);
    setTab('detail');
  };

  return (
    <div class="skill-panel h-full flex flex-col bg-[#1E293B] text-gray-200">
      {/* Header */}
      <div class="flex items-center justify-between px-4 py-3 border-b border-gray-700">
        <div class="flex items-center gap-1">
          <TabButton label="浏览" active={tab === 'browse'} onClick={() => setTab('browse')} />
          <TabButton label="导入" active={tab === 'import'} onClick={() => setTab('import')} />
          {selectedSkill && (
            <TabButton
              label={`详情: ${selectedSkill.name}`}
              active={tab === 'detail'}
              onClick={() => setTab('detail')}
            />
          )}
        </div>
        <span class="text-xs text-gray-500">{skills.length} skills</span>
      </div>

      {/* Tab content */}
      <div class="flex-1 overflow-y-auto p-4">
        {tab === 'browse' && (
          <BrowseTab
            search={search}
            onSearch={setSearch}
            tags={allTags}
            tagFilter={tagFilter}
            onTagFilter={setTagFilter}
            skills={filtered}
            loading={loading}
            onSelect={openDetail}
            onRefresh={loadSkills}
          />
        )}

        {tab === 'import' && (
          <ImportTab
            url={importUrl}
            onUrlChange={setImportUrl}
            source={importSource}
            onSourceChange={setImportSource}
            importing={importing}
            result={importResult}
            onImport={handleImport}
          />
        )}

        {tab === 'detail' && selectedSkill && (
          <DetailTab skill={selectedSkill} onBack={() => setTab('browse')} />
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// TabButton
// ---------------------------------------------------------------------------

function TabButton({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      class={`px-3 py-1.5 text-sm rounded-md transition-colors ${
        active
          ? 'bg-blue-600 text-white'
          : 'text-gray-400 hover:text-white hover:bg-gray-700'
      }`}
    >
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------
// BrowseTab
// ---------------------------------------------------------------------------

function BrowseTab({
  search, onSearch, tags, tagFilter, onTagFilter,
  skills, loading, onSelect, onRefresh,
}: {
  search: string;
  onSearch: (v: string) => void;
  tags: string[];
  tagFilter: string | null;
  onTagFilter: (t: string | null) => void;
  skills: Skill[];
  loading: boolean;
  onSelect: (s: Skill) => void;
  onRefresh: () => void;
}) {
  return (
    <div>
      {/* Search bar */}
      <div class="flex gap-2 mb-4">
        <input
          type="text"
          placeholder="搜索技能名称、描述、标签..."
          value={search}
          onInput={(e) => onSearch((e.target as HTMLInputElement).value)}
          class="flex-1 px-3 py-2 bg-gray-800 border border-gray-600 rounded-md text-sm 
                 placeholder-gray-500 focus:outline-none focus:border-blue-500"
        />
        <button
          onClick={onRefresh}
          class="px-3 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded-md transition-colors"
          title="刷新"
        >
          ↻
        </button>
      </div>

      {/* Tag chips */}
      {tags.length > 0 && (
        <div class="flex flex-wrap gap-1.5 mb-4">
          <TagChip label="全部" active={tagFilter === null} onClick={() => onTagFilter(null)} />
          {tags.map(tag => (
            <TagChip key={tag} label={tag} active={tagFilter === tag} onClick={() => onTagFilter(tag)} />
          ))}
        </div>
      )}

      {/* Skill cards */}
      {loading ? (
        <div class="text-center text-gray-500 py-8">加载中...</div>
      ) : skills.length === 0 ? (
        <div class="text-center text-gray-500 py-8">
          {search || tagFilter ? '没有匹配的技能' : '技能库为空，尝试导入或创建一个技能'}
        </div>
      ) : (
        <div class="grid gap-3 grid-cols-1">
          {skills.map(skill => (
            <SkillCard key={skill.id} skill={skill} onClick={() => onSelect(skill)} />
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// SkillCard
// ---------------------------------------------------------------------------

function SkillCard({ skill, onClick }: { skill: Skill; onClick: () => void }) {
  return (
    <div
      onClick={onClick}
      class="skill-card p-4 bg-gray-800 border border-gray-700 rounded-lg cursor-pointer
             hover:border-blue-500 transition-colors"
    >
      <div class="flex items-start justify-between">
        <div class="flex-1 min-w-0">
          <h3 class="text-sm font-semibold text-white truncate">{skill.name}</h3>
          <p class="text-xs text-gray-400 mt-1 line-clamp-2">{skill.description}</p>
        </div>
        <div class="flex items-center gap-2 ml-3 shrink-0">
          {skill.avg_rating > 0 && (
            <span class="text-xs text-yellow-500" title={`评分 ${skill.avg_rating.toFixed(1)}`}>
              {'★'.repeat(Math.round(skill.avg_rating))}
            </span>
          )}
          <span class="text-xs text-gray-500">{skill.usage_count}次</span>
        </div>
      </div>
      <div class="flex flex-wrap gap-1 mt-2">
        {skill.tags.map(tag => (
          <span key={tag} class="px-1.5 py-0.5 text-[10px] bg-gray-700 text-gray-400 rounded">
            {tag}
          </span>
        ))}
        <span class="px-1.5 py-0.5 text-[10px] bg-blue-900/50 text-blue-400 rounded">
          {skill.language}
        </span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// TagChip
// ---------------------------------------------------------------------------

function TagChip({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      class={`px-2 py-0.5 text-xs rounded-full transition-colors ${
        active
          ? 'bg-blue-600 text-white'
          : 'bg-gray-700 text-gray-400 hover:bg-gray-600'
      }`}
    >
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------
// ImportTab
// ---------------------------------------------------------------------------

function ImportTab({
  url, onUrlChange, source, onSourceChange,
  importing, result, onImport,
}: {
  url: string;
  onUrlChange: (v: string) => void;
  source: string;
  onSourceChange: (s: string) => void;
  importing: boolean;
  result: ImportResult | null;
  onImport: () => void;
}) {
  return (
    <div class="max-w-lg">
      <h2 class="text-lg font-semibold text-white mb-4">导入外部技能</h2>

      {/* Source selector */}
      <div class="flex gap-2 mb-4">
        {[
          { value: 'url', label: 'URL (agentskills.io)' },
          { value: 'clawhub', label: 'ClawHub' },
          { value: 'teamskillshub', label: 'TeamSkillHub' },
        ].map(opt => (
          <button
            key={opt.value}
            onClick={() => onSourceChange(opt.value)}
            class={`px-3 py-1.5 text-sm rounded-md transition-colors ${
              source === opt.value
                ? 'bg-blue-600 text-white'
                : 'bg-gray-700 text-gray-400 hover:bg-gray-600'
            }`}
          >
            {opt.label}
          </button>
        ))}
      </div>

      {/* URL / slug input */}
      <div class="mb-4">
        <label class="block text-xs text-gray-400 mb-1">
          {source === 'clawhub' ? 'ClawHub Slug (e.g. org/skill-name)' :
           source === 'teamskillshub' ? 'Asset ID' :
           'Skill URL (raw SKILL.md)'}
        </label>
        <input
          type="text"
          value={url}
          onInput={(e) => onUrlChange((e.target as HTMLInputElement).value)}
          placeholder={
            source === 'clawhub' ? 'clawd/text-summarizer' :
            source === 'teamskillshub' ? 'asset-12345' :
            'https://raw.githubusercontent.com/.../SKILL.md'
          }
          class="w-full px-3 py-2 bg-gray-800 border border-gray-600 rounded-md text-sm
                 placeholder-gray-500 focus:outline-none focus:border-blue-500"
        />
      </div>

      <button
        onClick={onImport}
        disabled={importing || !url.trim()}
        class="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700 disabled:text-gray-500
               text-white text-sm rounded-md transition-colors"
      >
        {importing ? '导入中...' : '导入技能'}
      </button>

      {/* Result */}
      {result && (
        <div class={`mt-4 p-3 rounded-md text-sm ${
          result.success ? 'bg-green-900/30 border border-green-700 text-green-300' :
                          'bg-red-900/30 border border-red-700 text-red-300'
        }`}>
          {result.success ? (
            <div>
              <p class="font-semibold">✅ 导入成功</p>
              <p class="mt-1">技能: <strong>{result.skill?.name}</strong></p>
              <p class="text-xs text-green-400 mt-1">来源: {result.source}</p>
            </div>
          ) : (
            <div>
              <p class="font-semibold">❌ 导入失败</p>
              <p class="mt-1">{result.error || '未知错误'}</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// DetailTab
// ---------------------------------------------------------------------------

function DetailTab({ skill, onBack }: { skill: Skill; onBack: () => void }) {
  return (
    <div class="max-w-2xl">
      <button
        onClick={onBack}
        class="text-sm text-blue-400 hover:text-blue-300 mb-4 inline-block"
      >
        ← 返回列表
      </button>

      <div class="bg-gray-800 border border-gray-700 rounded-lg p-5">
        <div class="flex items-start justify-between mb-3">
          <div>
            <h2 class="text-xl font-bold text-white">{skill.name}</h2>
            <p class="text-sm text-gray-400 mt-1">{skill.description}</p>
          </div>
          {skill.avg_rating > 0 && (
            <span class="text-yellow-500 text-lg" title={`${skill.avg_rating.toFixed(1)} / 5`}>
              {'★'.repeat(Math.round(skill.avg_rating))}
            </span>
          )}
        </div>

        {/* Meta */}
        <div class="grid grid-cols-2 gap-3 mb-4 text-sm">
          <div>
            <span class="text-gray-500">语言:</span>
            <span class="ml-2 text-gray-300">{skill.language}</span>
          </div>
          <div>
            <span class="text-gray-500">使用次数:</span>
            <span class="ml-2 text-gray-300">{skill.usage_count}</span>
          </div>
          <div>
            <span class="text-gray-500">评分次数:</span>
            <span class="ml-2 text-gray-300">{skill.rating_count}</span>
          </div>
          <div>
            <span class="text-gray-500">创建时间:</span>
            <span class="ml-2 text-gray-300">{new Date(skill.created_at).toLocaleDateString('zh-CN')}</span>
          </div>
        </div>

        {/* Tags */}
        <div class="flex flex-wrap gap-1 mb-4">
          {skill.tags.map(tag => (
            <span key={tag} class="px-2 py-0.5 text-xs bg-gray-700 text-gray-400 rounded-full">
              {tag}
            </span>
          ))}
        </div>

        {/* Code preview */}
        {skill.code && (
          <div class="mb-4">
            <h3 class="text-sm font-semibold text-gray-300 mb-2">代码</h3>
            <pre class="p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-x-auto max-h-64">
              <code>{skill.code}</code>
            </pre>
          </div>
        )}

        {/* Source */}
        {skill.source_memory_id && (
          <div class="text-xs text-gray-500 mt-2">
            来源记忆: {skill.source_memory_id}
          </div>
        )}
      </div>
    </div>
  );
}
