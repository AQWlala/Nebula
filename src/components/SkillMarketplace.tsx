// v0.3: Skill Browser (技能浏览器).
//
// The marketplace shows every skill the local engine knows about,
// grouped by language and tag. Each card exposes three actions:
//
//   * Use     — invokes the skill with the user's parameters
//   * Rate    — opens a 1-5 star rating input
//   * Inspect — opens a modal with the full code + metadata
//
// A "Create new skill" button at the top opens a modal with a small
// form: name, description, code, language, tags. The form is
// intentionally tiny — the marketplace is a launching pad, not an
// IDE. The CodeMode tab remains the right place for bigger edits.

import { useEffect, useMemo, useState } from 'preact/hooks';
import type { JSX } from 'preact';
import { nebulaAPI, type Skill } from '../lib/tauri';
import { Modal } from './Modal';
import { t } from '../i18n';

const LANG_BADGE: Record<string, string> = {
  rust: '🦀 rust',
  python: '🐍 python',
  javascript: '⚡ js',
  typescript: '⚡ ts',
  bash: '🐚 bash',
  llm: '🧠 llm',
};

function formatRating(avg: number, count: number): string {
  if (count === 0) return '— unrated —';
  return `★ ${avg.toFixed(2)} (${count})`;
}

function formatUsage(n: number): string {
  if (n === 0) return 'unused';
  if (n === 1) return 'used once';
  return `used ${n}×`;
}

export function SkillMarketplace(): JSX.Element {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [langFilter, setLangFilter] = useState<string>('');
  const [tagFilter, setTagFilter] = useState<string>('');
  const [search, setSearch] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState<boolean>(false);
  const [rateOpen, setRateOpen] = useState<Skill | null>(null);
  const [useResult, setUseResult] = useState<{ skillId: string; output: string } | null>(null);

  const reload = async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await nebulaAPI.skillList({
        language: langFilter || undefined,
        tag: tagFilter || undefined,
        limit: 100,
      });
      setSkills(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [langFilter, tagFilter]);

  const allLanguages = useMemo(() => {
    const s = new Set<string>();
    skills.forEach((sk) => s.add(sk.language));
    return Array.from(s).sort();
  }, [skills]);

  const allTags = useMemo(() => {
    const s = new Set<string>();
    skills.forEach((sk) => sk.tags.forEach((t) => s.add(t)));
    return Array.from(s).sort();
  }, [skills]);

  const visible = useMemo(() => {
    if (!search.trim()) return skills;
    const q = search.toLowerCase();
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q))
    );
  }, [skills, search]);

  async function runSkill(skill: Skill) {
    setUseResult(null);
    try {
      const result = await nebulaAPI.skillUse({ id: skill.id, params: {} });
      setUseResult({ skillId: skill.id, output: result.output });
      await reload();
    } catch (e) {
      setError(`use failed: ${e}`);
    }
  }

  async function submitRating(skill: Skill, rating: number) {
    try {
      await nebulaAPI.skillRate({ id: skill.id, rating });
      setRateOpen(null);
      await reload();
    } catch (e) {
      setError(`rate failed: ${e}`);
    }
  }

  return (
    <div class="skill-browser">
      <header class="skill-browser__header">
        <h2>🔍 {t('skillMarketplace.title')}</h2>
        <div class="skill-browser__actions">
          <input
            type="search"
            placeholder={t('skillMarketplace.searchPlaceholder')}
            value={search}
            onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
          />
          <button onClick={() => setCreateOpen(true)} class="primary">
            {t('skillMarketplace.newSkill')}
          </button>
        </div>
      </header>

      {error && <div class="error">⚠ {error}</div>}
      {loading && <div class="muted">{t('skillMarketplace.loading')}</div>}

      <div class="skill-browser__filters">
        <label>
          {t('skillMarketplace.language')}
          <select value={langFilter} onChange={(e) => setLangFilter((e.target as HTMLSelectElement).value)}>
            <option value="">{t('skillMarketplace.all')}</option>
            {allLanguages.map((l) => (
              <option key={l} value={l}>
                {LANG_BADGE[l] ?? l}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t('skillMarketplace.tag')}
          <select value={tagFilter} onChange={(e) => setTagFilter((e.target as HTMLSelectElement).value)}>
            <option value="">{t('skillMarketplace.all')}</option>
            {allTags.map((t) => (
              <option key={t} value={t}>
                #{t}
              </option>
            ))}
          </select>
        </label>
        <span class="muted">
          {t('skillMarketplace.skillCount', { visible: visible.length, total: skills.length })}
        </span>
      </div>

      {visible.length === 0 && !loading ? (
        <div class="empty-state">
          <p dangerouslySetInnerHTML={{ __html: t('skillMarketplace.empty') }} />
        </div>
      ) : (
        <ul class="skill-cards">
          {visible.map((s) => (
            <li key={s.id} class="skill-card">
              <div class="skill-card__header">
                <h3>{s.name}</h3>
                <span class="skill-card__lang">{LANG_BADGE[s.language] ?? s.language}</span>
              </div>
              <p class="skill-card__desc">{s.description || <em>{t('skillMarketplace.noDescription')}</em>}</p>
              <div class="skill-card__meta">
                <span>{formatRating(s.avg_rating, s.rating_count)}</span>
                <span>·</span>
                <span>{formatUsage(s.usage_count)}</span>
                {s.source_memory_id && (
                  <>
                    <span>·</span>
                    <span title={s.source_memory_id}>{t('skillMarketplace.fromMemory')}</span>
                  </>
                )}
              </div>
              <div class="skill-card__tags">
                {s.tags.map((t) => (
                  <span key={t} class="tag">
                    #{t}
                  </span>
                ))}
              </div>
              <div class="skill-card__actions">
                <button onClick={() => void runSkill(s)}>{t('skillMarketplace.use')}</button>
                <button onClick={() => setRateOpen(s)}>{t('skillMarketplace.rate')}</button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {useResult && (
        <Modal open={true} title={t('skillMarketplace.skillOutput')} onClose={() => setUseResult(null)}>
          <pre class="skill-output">{useResult.output || t('skillMarketplace.emptyOutput')}</pre>
        </Modal>
      )}

      {rateOpen && (
        <Modal open={true} title={t('skillMarketplace.rateTitle', { name: rateOpen.name })} onClose={() => setRateOpen(null)}>
          <RatingPicker
            onSubmit={(r) => void submitRating(rateOpen, r)}
            onCancel={() => setRateOpen(null)}
          />
        </Modal>
      )}

      {createOpen && (
        <CreateSkillModal
          onClose={() => setCreateOpen(false)}
          onCreated={async () => {
            setCreateOpen(false);
            await reload();
          }}
        />
      )}
    </div>
  );
}

function RatingPicker({
  onSubmit,
  onCancel,
}: {
  onSubmit: (rating: number) => void;
  onCancel: () => void;
}): JSX.Element {
  const [value, setValue] = useState(4);
  return (
    <div>
      <div class="rating-picker">
        {[1, 2, 3, 4, 5].map((n) => (
          <button
            key={n}
            class={n <= value ? 'star star--on' : 'star'}
            onClick={() => setValue(n)}
          >
            ★
          </button>
        ))}
        <span class="muted">{value}/5</span>
      </div>
      <div class="modal__actions">
        <button onClick={onCancel}>{t('skillMarketplace.cancel')}</button>
        <button class="primary" onClick={() => onSubmit(value)}>
          {t('skillMarketplace.submit')}
        </button>
      </div>
    </div>
  );
}

function CreateSkillModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: () => Promise<void> | void;
}): JSX.Element {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [code, setCode] = useState('');
  const [language, setLanguage] = useState('rust');
  const [tags, setTags] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setErr(null);
    try {
      await nebulaAPI.skillCreate({
        name: name.trim(),
        description: description.trim(),
        code,
        language,
        tags: tags
          .split(/[,\s]+/)
          .map((t) => t.trim().replace(/^#/, ''))
          .filter(Boolean),
      });
      await onCreated();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={true} title={t('skillMarketplace.createTitle')} onClose={onClose}>
      <div class="form">
        <label>
          {t('skillMarketplace.name')}
          <input value={name} onInput={(e) => setName((e.target as HTMLInputElement).value)} />
        </label>
        <label>
          {t('skillMarketplace.description')}
          <input
            value={description}
            onInput={(e) => setDescription((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          {t('skillMarketplace.languageLabel')}
          <select value={language} onChange={(e) => setLanguage((e.target as HTMLSelectElement).value)}>
            <option value="rust">rust</option>
            <option value="python">python</option>
            <option value="javascript">javascript</option>
            <option value="bash">bash</option>
            <option value="llm">llm (prompt template)</option>
          </select>
        </label>
        <label>
          {t('skillMarketplace.tags')}
          <input
            value={tags}
            placeholder="e.g. string, utility"
            onInput={(e) => setTags((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          {t('skillMarketplace.code')}
          <textarea
            rows={10}
            value={code}
            onInput={(e) => setCode((e.target as HTMLTextAreaElement).value)}
          />
        </label>
        {err && <div class="error">⚠ {err}</div>}
        <div class="modal__actions">
          <button onClick={onClose} disabled={busy}>
            {t('skillMarketplace.cancel')}
          </button>
          <button class="primary" onClick={submit} disabled={busy || !name.trim() || !code.trim()}>
            {busy ? t('skillMarketplace.creating') : t('skillMarketplace.create')}
          </button>
        </div>
      </div>
    </Modal>
  );
}


