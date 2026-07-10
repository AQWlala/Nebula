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
import { nebulaAPI, Skill, ListSkillsRequest, ImportResult, TagCount } from '../lib/tauri';
import VisualCreatorDialog from './VisualCreatorDialog';
import SkillDebugger from './SkillDebugger';
import type { VizKind } from './VizRenderer';
import { Spinner } from './Spinner';
import { t } from '../i18n';

type Tab = 'browse' | 'import' | 'detail';

// ---------------------------------------------------------------------------
// P1-6: 来源 badge 辅助 — 从 skill id 前缀解析来源
// ---------------------------------------------------------------------------

/** P1-6: 从 skill id 前缀解析来源标识符。
 *
 * id 格式约定（importer.rs store_skill_with_source）:
 * - `import-openclaw-<name>-<ts>` → "openclaw"
 * - `import-clawhub-<name>-<ts>` → "clawhub"
 * - `import-agentskills-<name>-<ts>` → "agentskills"
 * - `import-teamskillshub-<name>-<ts>` → "teamskillshub"
 * - `import-<name>-<ts>`（旧格式）→ "agentskills"（向后兼容）
 * - 其他 → "local"
 */
function getSourceFromId(id: string): string {
  if (!id.startsWith('import-')) return 'local';
  // 去掉 "import-" 前缀后检查第二段是否为已知 source 标签
  const rest = id.slice(7); // "import-".length === 7
  const knownSources = ['openclaw', 'clawhub', 'agentskills', 'teamskillshub'];
  for (const src of knownSources) {
    if (rest.startsWith(src + '-')) return src;
  }
  // 旧格式 import-<name>-<ts>，无法区分，默认 agentskills
  return 'agentskills';
}

/** P1-6: 来源标识符 → badge CSS class 后缀。 */
function sourceToBadgeClass(source: string): string {
  switch (source) {
    case 'openclaw':
      return 'skill-source-badge--openclaw';
    case 'agentskills':
      return 'skill-source-badge--agentskills';
    case 'clawhub':
      return 'skill-source-badge--clawhub';
    case 'teamskillshub':
      return 'skill-source-badge--teamhub';
    default:
      return 'skill-source-badge--local';
  }
}

/** P1-6: 来源标识符 → i18n badge 文本 key。 */
function sourceToBadgeLabel(source: string): string {
  switch (source) {
    case 'openclaw':
      return t('skillPanel.sourceBadge.openclaw');
    case 'agentskills':
      return t('skillPanel.sourceBadge.agentskills');
    case 'clawhub':
      return t('skillPanel.sourceBadge.clawhub');
    case 'teamskillshub':
      return t('skillPanel.sourceBadge.teamskillshub');
    default:
      return t('skillPanel.sourceBadge.local');
  }
}

/** T-E-S-37: 多 tag 匹配模式(与后端 TagMatch lowercase 序列化对齐)。 */
type TagMatchMode = 'any' | 'all';

export default function SkillPanel() {
  const [tab, setTab] = useState<Tab>('browse');
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');
  // T-E-S-37: 多 tag 选择(数组,空数组 = 不按 tag 过滤)。
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  // T-E-S-37: 多 tag 匹配模式('any' = OR,'all' = AND)。
  const [tagMatch, setTagMatch] = useState<TagMatchMode>('any');
  // T-E-S-37: 热门标签云(从 skill_tags 命令获取,按 count 降序)。
  const [topTags, setTopTags] = useState<TagCount[]>([]);
  const [selectedSkill, setSelectedSkill] = useState<Skill | null>(null);
  // T-E-S-38: 可视化生成弹窗状态。
  const [vizDialogKind, setVizDialogKind] = useState<VizKind | null>(null);
  // P1-7: 技能调试器弹窗状态。
  const [debuggerSkill, setDebuggerSkill] = useState<Skill | null>(null);

  // Import state
  const [importUrl, setImportUrl] = useState('');
  const [importSource, setImportSource] = useState<string>('url');
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);

  // P1-6: 来源筛选器状态（空字符串 = 不按来源过滤，展示全部）。
  const [sourceFilter, setSourceFilter] = useState<string>('');
  // P1-6: 命令行安装状态。
  const [cliInput, setCliInput] = useState('');
  const [cliInstalling, setCliInstalling] = useState(false);
  const [cliResult, setCliResult] = useState<{ kind: 'success' | 'error'; msg: string } | null>(
    null
  );

  // T-E-S-37: 加载热门标签云(只加载一次,在 mount 时)。
  const loadTopTags = useCallback(async () => {
    try {
      const tags = await nebulaAPI.skillTags();
      setTopTags(tags || []);
    } catch {
      // 静默失败:skill_tags 命令不可用时降级到本地派生(下方 allTags 兜底)。
      setTopTags([]);
    }
  }, []);

  useEffect(() => {
    loadTopTags();
  }, [loadTopTags]);

  // Load skills
  const loadSkills = useCallback(async () => {
    setLoading(true);
    try {
      const req: ListSkillsRequest = {};
      // T-E-S-37: 多 tag 优先。selectedTags 非空时走多 tag 路径(tags + tag_match),
      // 否则降级到不按 tag 过滤(展示全部)。
      if (selectedTags.length > 0) {
        req.tags = selectedTags;
        req.tag_match = tagMatch;
      }
      const result = await nebulaAPI.skillList(req);
      setSkills(result || []);
    } finally {
      setLoading(false);
    }
  }, [selectedTags, tagMatch]);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  // Filtered skills — P1-6: 同时支持文本搜索 + 来源筛选。
  const filtered = skills.filter((s) => {
    // 文本搜索过滤
    if (search.trim()) {
      const q = search.toLowerCase();
      const textMatch =
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q));
      if (!textMatch) return false;
    }
    // P1-6: 来源筛选（sourceFilter 非空时按来源 badge 过滤）。
    if (sourceFilter) {
      const skillSource = getSourceFromId(s.id);
      if (skillSource !== sourceFilter) return false;
    }
    return true;
  });

  // T-E-S-37: 标签云源 — 优先用 skill_tags 命令的全局聚合,降级到当前 skills 派生。
  // 显示用:tag + 频次。过滤用:selectedTags 数组。
  const tagCloud: { tag: string; count: number }[] =
    topTags.length > 0
      ? topTags
      : [...new Set(skills.flatMap((s) => s.tags))]
          .map((tag) => ({ tag, count: skills.filter((s) => s.tags.includes(tag)).length }))
          .sort((a, b) => b.count - a.count);

  // T-E-S-37: 切换单个 tag 的选中状态(空 -> 加入 / 已存在 -> 移除)。
  const toggleTag = (tag: string) => {
    setSelectedTags((prev) =>
      prev.includes(tag) ? prev.filter((t) => t !== tag) : [...prev, tag]
    );
  };

  // T-E-S-37: 清空所有 tag 选择(= 不按 tag 过滤,展示全部)。
  const clearTags = () => setSelectedTags([]);

  // Handle import
  const handleImport = async () => {
    if (!importUrl.trim()) return;
    setImporting(true);
    setImportResult(null);
    try {
      const result = await nebulaAPI.skillImport(importUrl.trim(), importSource);
      setImportResult(result);
      if (result.success) {
        await loadSkills();
      }
    } finally {
      setImporting(false);
    }
  };

  // P1-6: 命令行式安装处理器 — 支持 openclaw/<slug> / GitHub URL / 任意 URL 三种格式。
  const handleCliInstall = async () => {
    const input = cliInput.trim();
    if (!input) return;
    setCliInstalling(true);
    setCliResult(null);
    try {
      let skillName = '';
      if (input.startsWith('openclaw/')) {
        // openclaw/<slug> 格式 → 从 OpenClaw 社区安装
        const slug = input.slice('openclaw/'.length);
        const skillId = await nebulaAPI.installSkillFromOpenclaw(slug);
        skillName = skillId;
      } else if (input.startsWith('http://') || input.startsWith('https://')) {
        // URL 格式 → 通用 URL 安装
        const skillId = await nebulaAPI.installSkillFromUrl(input);
        skillName = skillId;
      } else {
        // 无前缀的 slug → 默认按 OpenClaw slug 安装
        const skillId = await nebulaAPI.installSkillFromOpenclaw(input);
        skillName = skillId;
      }
      setCliResult({
        kind: 'success',
        msg: t('skillPanel.cliInstallSuccess', { name: skillName }),
      });
      setCliInput('');
      // 安装成功后刷新技能列表。
      await loadSkills();
    } catch (e) {
      setCliResult({
        kind: 'error',
        msg: t('skillPanel.cliInstallFailed', {
          error: e instanceof Error ? e.message : String(e),
        }),
      });
    } finally {
      setCliInstalling(false);
    }
  };

  // Open detail
  const openDetail = (skill: Skill) => {
    setSelectedSkill(skill);
    setTab('detail');
  };

  // T-E-S-38: 打开可视化生成弹窗。
  const openVizDialog = (kind: VizKind) => {
    setVizDialogKind(kind);
  };

  // T-E-S-38: detail Tab 的"使用 skill"按钮 — 若是 viz creator 则打开弹窗,
  // 否则无操作(其他 skill 类型不在本次范围内)。
  const handleUseSkill = (skill: Skill) => {
    if (skill.name === 'canvas-creator') {
      openVizDialog('canvas');
    } else if (skill.name === 'mermaid-creator') {
      openVizDialog('mermaid');
    } else if (skill.name === 'mindmap-creator') {
      openVizDialog('mindmap');
    }
  };

  return (
    <div class="skill-panel h-full flex flex-col bg-[#1E293B] text-gray-200">
      {/* Header */}
      <div class="flex items-center justify-between px-4 py-3 border-b border-gray-700">
        <div class="flex items-center gap-1">
          <TabButton
            label={t('skillPanel.browse')}
            active={tab === 'browse'}
            onClick={() => setTab('browse')}
          />
          <TabButton
            label={t('skillPanel.import')}
            active={tab === 'import'}
            onClick={() => setTab('import')}
          />
          {selectedSkill && (
            <TabButton
              label={t('skillPanel.detail', { name: selectedSkill.name })}
              active={tab === 'detail'}
              onClick={() => setTab('detail')}
            />
          )}
        </div>
        <span class="text-xs text-gray-500">
          {t('skillPanel.skillCount', { count: skills.length })}
        </span>
      </div>

      {/* Tab content */}
      <div class="flex-1 overflow-y-auto p-4">
        {tab === 'browse' && (
          <BrowseTab
            search={search}
            onSearch={setSearch}
            tagCloud={tagCloud}
            selectedTags={selectedTags}
            onToggleTag={toggleTag}
            onClearTags={clearTags}
            tagMatch={tagMatch}
            onTagMatchChange={setTagMatch}
            skills={filtered}
            loading={loading}
            onSelect={openDetail}
            onRefresh={loadSkills}
            onOpenVizCreator={openVizDialog}
            sourceFilter={sourceFilter}
            onSourceFilterChange={setSourceFilter}
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
            cliInput={cliInput}
            onCliInputChange={setCliInput}
            onCliInstall={handleCliInstall}
            cliInstalling={cliInstalling}
            cliResult={cliResult}
          />
        )}

        {tab === 'detail' && selectedSkill && (
          <DetailTab
            skill={selectedSkill}
            onBack={() => setTab('browse')}
            onUseSkill={handleUseSkill}
            onDebug={() => setDebuggerSkill(selectedSkill)}
          />
        )}
      </div>

      {/* T-E-S-38: 可视化生成弹窗 */}
      {vizDialogKind && (
        <VisualCreatorDialog initialKind={vizDialogKind} onClose={() => setVizDialogKind(null)} />
      )}

      {/* P1-7: 技能调试器弹窗 */}
      {debuggerSkill && (
        <SkillDebugger
          skillId={debuggerSkill.id}
          skillName={debuggerSkill.name}
          onClose={() => setDebuggerSkill(null)}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// TabButton
// ---------------------------------------------------------------------------

function TabButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      class={`px-3 py-1.5 text-sm rounded-md transition-colors ${
        active ? 'bg-blue-600 text-white' : 'text-gray-400 hover:text-white hover:bg-gray-700'
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
  search,
  onSearch,
  tagCloud,
  selectedTags,
  onToggleTag,
  onClearTags,
  tagMatch,
  onTagMatchChange,
  skills,
  loading,
  onSelect,
  onRefresh,
  onOpenVizCreator,
  sourceFilter,
  onSourceFilterChange,
}: {
  search: string;
  onSearch: (v: string) => void;
  tagCloud: { tag: string; count: number }[];
  selectedTags: string[];
  onToggleTag: (t: string) => void;
  onClearTags: () => void;
  tagMatch: 'any' | 'all';
  onTagMatchChange: (m: 'any' | 'all') => void;
  skills: Skill[];
  loading: boolean;
  onSelect: (s: Skill) => void;
  onRefresh: () => void;
  onOpenVizCreator: (kind: VizKind) => void;
  // P1-6: 来源筛选器
  sourceFilter: string;
  onSourceFilterChange: (s: string) => void;
}) {
  return (
    <div>
      {/* T-E-S-38: 三个可视化 creator 快速入口卡片 */}
      <div class="mb-5">
        <h3 class="text-xs text-gray-500 uppercase tracking-wide mb-2">
          {t('skillPanel.vizCreators')}
        </h3>
        <div class="grid grid-cols-3 gap-2">
          <VizQuickEntry
            icon="🎨"
            label={t('skillPanel.canvasLabel')}
            hint={t('skillPanel.canvasHint')}
            onClick={() => onOpenVizCreator('canvas')}
          />
          <VizQuickEntry
            icon="📊"
            label={t('skillPanel.mermaidLabel')}
            hint={t('skillPanel.mermaidHint')}
            onClick={() => onOpenVizCreator('mermaid')}
          />
          <VizQuickEntry
            icon="🧠"
            label={t('skillPanel.mindmapLabel')}
            hint={t('skillPanel.mindmapHint')}
            onClick={() => onOpenVizCreator('mindmap')}
          />
        </div>
      </div>

      {/* Search bar */}
      <div class="flex gap-2 mb-4">
        <input
          type="text"
          placeholder={t('skillPanel.searchPlaceholder')}
          value={search}
          onInput={(e) => onSearch((e.target as HTMLInputElement).value)}
          class="flex-1 px-3 py-2 bg-gray-800 border border-gray-600 rounded-md text-sm
                 placeholder-gray-500 focus:outline-none focus:border-blue-500"
        />
        <button
          onClick={onRefresh}
          class="px-3 py-2 text-sm bg-gray-700 hover:bg-gray-600 rounded-md transition-colors"
          title={t('skillPanel.refresh')}
        >
          ↻
        </button>
      </div>

      {/* P1-6: 来源筛选器 — 按来源 badge 过滤技能列表。 */}
      <div class="mb-4">
        <h3 class="text-xs text-gray-500 uppercase tracking-wide mb-2">
          {t('skillPanel.sourceFilter')}
        </h3>
        <div class="skill-source-filter">
          <button
            onClick={() => onSourceFilterChange('')}
            class={`source-filter-chip ${!sourceFilter ? 'active' : ''}`}
          >
            {t('skillPanel.sourceFilterAll')}
          </button>
          {(['openclaw', 'agentskills', 'clawhub', 'teamskillshub', 'local'] as const).map(
            (src) => (
              <button
                key={src}
                onClick={() => onSourceFilterChange(sourceFilter === src ? '' : src)}
                class={`source-filter-chip ${sourceFilter === src ? 'active' : ''}`}
              >
                {sourceToBadgeLabel(src)}
              </button>
            )
          )}
        </div>
      </div>

      {/* T-E-S-37: 标签云 — 显示热门 tag(最多前 10)+ 频次 + 多选 chip。 */}
      {tagCloud.length > 0 && (
        <div class="mb-4">
          <div class="flex items-center justify-between mb-2">
            <h3 class="text-xs text-gray-500 uppercase tracking-wide">
              {t('skillPanel.popularTags')}
              {selectedTags.length > 0
                ? ` · ${t('skillPanel.selectedTags', { count: selectedTags.length })}`
                : ''}
            </h3>
            {/* T-E-S-37: 多 tag 匹配模式切换(仅当选中 ≥ 2 个 tag 时显示)。 */}
            {selectedTags.length >= 2 && (
              <div class="flex gap-1 text-[10px]">
                <MatchModeButton
                  label={t('skillPanel.matchAny')}
                  active={tagMatch === 'any'}
                  onClick={() => onTagMatchChange('any')}
                />
                <MatchModeButton
                  label={t('skillPanel.matchAll')}
                  active={tagMatch === 'all'}
                  onClick={() => onTagMatchChange('all')}
                />
              </div>
            )}
          </div>
          <div class="flex flex-wrap gap-1.5">
            <TagChip
              label={t('skillPanel.allTags')}
              active={selectedTags.length === 0}
              onClick={onClearTags}
            />
            {tagCloud.slice(0, 10).map(({ tag, count }) => (
              <TagChip
                key={tag}
                label={`${tag} (${count})`}
                active={selectedTags.includes(tag)}
                onClick={() => onToggleTag(tag)}
              />
            ))}
          </div>
        </div>
      )}

      {/* Skill cards */}
      {loading ? (
        <div class="text-center py-8">
          <Spinner label={t('common.loading')} />
        </div>
      ) : skills.length === 0 ? (
        <div class="text-center text-gray-500 py-8">
          {search || selectedTags.length > 0 ? t('skillPanel.noMatch') : t('skillPanel.empty')}
        </div>
      ) : (
        <div class="grid gap-3 grid-cols-1">
          {skills.map((skill) => (
            <SkillCard key={skill.id} skill={skill} onClick={() => onSelect(skill)} />
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// T-E-S-37: MatchModeButton — Any / All 切换按钮
// ---------------------------------------------------------------------------

function MatchModeButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      class={`px-2 py-0.5 rounded transition-colors ${
        active ? 'bg-blue-600 text-white' : 'bg-gray-700 text-gray-400 hover:bg-gray-600'
      }`}
    >
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------
// T-E-S-38: VizQuickEntry — 可视化 creator 快速入口卡片
// ---------------------------------------------------------------------------

function VizQuickEntry({
  icon,
  label,
  hint,
  onClick,
}: {
  icon: string;
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      class="flex flex-col items-start gap-1 p-3 bg-gray-800 border border-gray-700 rounded-lg
             hover:border-blue-500 hover:bg-gray-750 transition-colors text-left"
    >
      <span class="text-xl">{icon}</span>
      <span class="text-sm font-semibold text-white">{label}</span>
      <span class="text-[10px] text-gray-400">{hint}</span>
    </button>
  );
}

// ---------------------------------------------------------------------------
// P1-6: SourceBadge — 来源 badge 组件
// ---------------------------------------------------------------------------

function SourceBadge({ source }: { source: string }) {
  return (
    <span class={`skill-source-badge ${sourceToBadgeClass(source)}`}>
      {sourceToBadgeLabel(source)}
    </span>
  );
}

// ---------------------------------------------------------------------------
// SkillCard
// ---------------------------------------------------------------------------

function SkillCard({ skill, onClick }: { skill: Skill; onClick: () => void }) {
  // P1-6: 从 skill id 解析来源，用于显示 badge。
  const source = getSourceFromId(skill.id);
  return (
    <div
      onClick={onClick}
      class="skill-card p-4 bg-gray-800 border border-gray-700 rounded-lg cursor-pointer
             hover:border-blue-500 transition-colors"
    >
      <div class="flex items-start justify-between">
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 mb-1">
            <h3 class="text-sm font-semibold text-white truncate">{skill.name}</h3>
            {/* P1-6: 来源 badge */}
            <SourceBadge source={source} />
          </div>
          <p class="text-xs text-gray-400 mt-1 line-clamp-2">{skill.description}</p>
        </div>
        <div class="flex items-center gap-2 ml-3 shrink-0">
          {skill.avg_rating > 0 && (
            <span
              class="text-xs text-yellow-500"
              title={t('skillPanel.rating', { value: skill.avg_rating.toFixed(1) })}
            >
              {'★'.repeat(Math.round(skill.avg_rating))}
            </span>
          )}
          <span class="text-xs text-gray-500">
            {t('skillPanel.usageCount', { count: skill.usage_count })}
          </span>
        </div>
      </div>
      <div class="flex flex-wrap gap-1 mt-2">
        {skill.tags.map((tag) => (
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

function TagChip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      class={`px-2 py-0.5 text-xs rounded-full transition-colors ${
        active ? 'bg-blue-600 text-white' : 'bg-gray-700 text-gray-400 hover:bg-gray-600'
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
  url,
  onUrlChange,
  source,
  onSourceChange,
  importing,
  result,
  onImport,
  // P1-6: 命令行安装相关 props
  cliInput,
  onCliInputChange,
  onCliInstall,
  cliInstalling,
  cliResult,
}: {
  url: string;
  onUrlChange: (v: string) => void;
  source: string;
  onSourceChange: (s: string) => void;
  importing: boolean;
  result: ImportResult | null;
  onImport: () => void;
  // P1-6: 命令行安装 props
  cliInput: string;
  onCliInputChange: (v: string) => void;
  onCliInstall: () => void;
  cliInstalling: boolean;
  cliResult: { kind: 'success' | 'error'; msg: string } | null;
}) {
  return (
    <div class="max-w-lg">
      <h2 class="text-lg font-semibold text-white mb-4">{t('skillPanel.importTitle')}</h2>

      {/* Source selector */}
      <div class="flex gap-2 mb-4">
        {[
          { value: 'url', label: t('skillPanel.urlLabel') },
          { value: 'clawhub', label: t('skillPanel.clawhubLabel') },
          { value: 'teamskillshub', label: t('skillPanel.teamSkillHubLabel') },
        ].map((opt) => (
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
          {source === 'clawhub'
            ? t('skillPanel.clawhubSlugHint')
            : source === 'teamskillshub'
              ? t('skillPanel.assetIdHint')
              : t('skillPanel.skillUrlHint')}
        </label>
        <input
          type="text"
          value={url}
          onInput={(e) => onUrlChange((e.target as HTMLInputElement).value)}
          placeholder={
            source === 'clawhub'
              ? 'clawd/text-summarizer'
              : source === 'teamskillshub'
                ? 'asset-12345'
                : 'https://raw.githubusercontent.com/.../SKILL.md'
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
        {importing ? t('skillPanel.importing') : t('skillPanel.importButton')}
      </button>

      {/* Result */}
      {result && (
        <div
          class={`mt-4 p-3 rounded-md text-sm ${
            result.success
              ? 'bg-green-900/30 border border-green-700 text-green-300'
              : 'bg-red-900/30 border border-red-700 text-red-300'
          }`}
        >
          {result.success ? (
            <div>
              <p class="font-semibold">{t('skillPanel.importSuccess')}</p>
              <p class="mt-1">
                {t('skillPanel.skillLabel')}
                <strong>{result.skill?.name}</strong>
              </p>
              <p class="text-xs text-green-400 mt-1">
                {t('skillPanel.sourceLabel')}
                {result.source}
              </p>
            </div>
          ) : (
            <div>
              <p class="font-semibold">{t('skillPanel.importFailed')}</p>
              <p class="mt-1">{result.error || t('skillPanel.unknownError')}</p>
            </div>
          )}
        </div>
      )}

      {/* P1-6: 命令行安装区域 — 支持 openclaw/<slug> / GitHub URL / 任意 URL 三种格式。 */}
      <div class="skill-cli-install">
        <div class="cli-install-header">
          <span class="cli-icon">⌘</span>
          <h3 class="cli-title">{t('skillPanel.cliInstallTitle')}</h3>
        </div>
        <p class="cli-install-hint">{t('skillPanel.cliInstallHint')}</p>
        <div class="cli-install-input-row">
          <input
            type="text"
            value={cliInput}
            onInput={(e) => onCliInputChange((e.target as HTMLInputElement).value)}
            placeholder={t('skillPanel.cliInstallPlaceholder')}
          />
          <button
            onClick={onCliInstall}
            disabled={cliInstalling || !cliInput.trim()}
          >
            {cliInstalling ? t('skillPanel.cliInstalling') : t('skillPanel.cliInstallButton')}
          </button>
        </div>
        {/* 支持的安装格式示例 */}
        <div class="cli-install-formats">
          <p class="formats-title">{t('skillPanel.cliFormatTitle')}</p>
          <ul>
            <li class="format-item">{t('skillPanel.cliFormatOpenclaw')}</li>
            <li class="format-item">{t('skillPanel.cliFormatGithub')}</li>
            <li class="format-item">{t('skillPanel.cliFormatUrl')}</li>
          </ul>
        </div>
        {/* 安装结果反馈 */}
        {cliResult && (
          <div class={`cli-install-result ${cliResult.kind}`}>
            {cliResult.msg}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// DetailTab
// ---------------------------------------------------------------------------

function DetailTab({
  skill,
  onBack,
  onUseSkill,
  onDebug,
}: {
  skill: Skill;
  onBack: () => void;
  onUseSkill: (skill: Skill) => void;
  onDebug: () => void;
}) {
  const [exporting, setExporting] = useState(false);
  const [exportToast, setExportToast] = useState<{ kind: 'success' | 'error'; msg: string } | null>(
    null
  );

  // T-E-S-38: 判断当前 skill 是否为可视化 creator(显示"使用 skill"按钮)。
  const isVizCreator =
    skill.name === 'canvas-creator' ||
    skill.name === 'mermaid-creator' ||
    skill.name === 'mindmap-creator';

  const handleExport = async () => {
    setExporting(true);
    setExportToast(null);
    try {
      const result = await nebulaAPI.skillExportClawhub(skill.id);
      const chars = result.content?.length ?? 0;
      setExportToast({
        kind: 'success',
        msg: t('skillPanel.exported', { chars }),
      });
    } catch (e) {
      setExportToast({
        kind: 'error',
        msg: t('skillPanel.exportFailed', { error: e instanceof Error ? e.message : String(e) }),
      });
    } finally {
      setExporting(false);
    }
  };

  return (
    <div class="max-w-2xl">
      <div class="flex items-center justify-between mb-4">
        <button onClick={onBack} class="text-sm text-blue-400 hover:text-blue-300 inline-block">
          {t('skillPanel.backToList')}
        </button>
        <div class="flex items-center gap-2">
          {/* T-E-S-38: "使用 skill" 按钮 — 仅对可视化 creator 显示。 */}
          {isVizCreator && (
            <button
              onClick={() => onUseSkill(skill)}
              class="px-3 py-1.5 text-sm bg-green-600 hover:bg-green-700
                     text-white rounded-md transition-colors"
              title={t('skillPanel.openVizCreator')}
            >
              {t('skillPanel.useSkill')}
            </button>
          )}
          {/* P1-7: "调试" 按钮 — 打开技能调试器(Inspector/TestRunner/Debugger/Profiler)。 */}
          <button
            onClick={onDebug}
            class="px-3 py-1.5 text-sm bg-purple-600 hover:bg-purple-700
                   text-white rounded-md transition-colors"
            title={t('skillPanel.debug')}
          >
            {t('skillPanel.debug')}
          </button>
          <button
            onClick={handleExport}
            disabled={exporting}
            class="px-3 py-1.5 text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700
                   disabled:text-gray-500 text-white rounded-md transition-colors"
            title={t('skillPanel.exportAsSkill')}
          >
            {exporting ? <Spinner size={16} showLabel={false} /> : t('skillPanel.exportAsSkill')}
          </button>
        </div>
      </div>

      {exportToast && (
        <div
          class={`mb-4 px-3 py-2 rounded-md text-sm ${
            exportToast.kind === 'success'
              ? 'bg-green-900/30 border border-green-700 text-green-300'
              : 'bg-red-900/30 border border-red-700 text-red-300'
          }`}
        >
          {exportToast.kind === 'success' ? '✅ ' : '❌ '}
          {exportToast.msg}
        </div>
      )}

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
            <span class="text-gray-500">{t('skillPanel.languageLabel')}</span>
            <span class="ml-2 text-gray-300">{skill.language}</span>
          </div>
          <div>
            <span class="text-gray-500">{t('skillPanel.usageCountLabel')}</span>
            <span class="ml-2 text-gray-300">{skill.usage_count}</span>
          </div>
          <div>
            <span class="text-gray-500">{t('skillPanel.ratingCountLabel')}</span>
            <span class="ml-2 text-gray-300">{skill.rating_count}</span>
          </div>
          <div>
            <span class="text-gray-500">{t('skillPanel.createdAtLabel')}</span>
            <span class="ml-2 text-gray-300">
              {new Date(skill.created_at).toLocaleDateString('zh-CN')}
            </span>
          </div>
        </div>

        {/* Tags */}
        <div class="flex flex-wrap gap-1 mb-4">
          {skill.tags.map((tag) => (
            <span key={tag} class="px-2 py-0.5 text-xs bg-gray-700 text-gray-400 rounded-full">
              {tag}
            </span>
          ))}
        </div>

        {/* Code preview */}
        {skill.code && (
          <div class="mb-4">
            <h3 class="text-sm font-semibold text-gray-300 mb-2">{t('skillPanel.code')}</h3>
            <pre class="p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-x-auto max-h-64">
              <code>{skill.code}</code>
            </pre>
          </div>
        )}

        {/* Source */}
        {skill.source_memory_id && (
          <div class="text-xs text-gray-500 mt-2">
            {t('skillPanel.sourceMemory', { id: skill.source_memory_id })}
          </div>
        )}

        {/* P1-6: 兼容协议信息 — 展示 skill 来源 badge 与 OpenClaw 兼容性说明。 */}
        <div class="skill-compatibility-info">
          <div class="compat-title flex items-center gap-2">
            <SourceBadge source={getSourceFromId(skill.id)} />
            <span>{t('skillPanel.compatibilityTitle')}</span>
          </div>
          <p class="compat-desc">{t('skillPanel.compatibilityOpenclaw')}</p>
        </div>
      </div>
    </div>
  );
}
