// v0.3: Skill Marketplace UI.
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
import { NineSnakeAPI, type Skill } from '../lib/tauri';

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
      const data = await NineSnakeAPI.skillList({
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
      const result = await NineSnakeAPI.skillUse({ id: skill.id, params: {} });
      setUseResult({ skillId: skill.id, output: result.output });
      await reload();
    } catch (e) {
      setError(`use failed: ${e}`);
    }
  }

  async function submitRating(skill: Skill, rating: number) {
    try {
      await NineSnakeAPI.skillRate({ id: skill.id, rating });
      setRateOpen(null);
      await reload();
    } catch (e) {
      setError(`rate failed: ${e}`);
    }
  }

  return (
    <div class="skill-marketplace">
      <header class="skill-marketplace__header">
        <h2>🛒 技能市场 / Skill Marketplace</h2>
        <div class="skill-marketplace__actions">
          <input
            type="search"
            placeholder="search skills…"
            value={search}
            onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
          />
          <button onClick={() => setCreateOpen(true)} class="primary">
            + 新建技能
          </button>
        </div>
      </header>

      {error && <div class="error">⚠ {error}</div>}
      {loading && <div class="muted">loading…</div>}

      <div class="skill-marketplace__filters">
        <label>
          语言：
          <select value={langFilter} onChange={(e) => setLangFilter((e.target as HTMLSelectElement).value)}>
            <option value="">全部</option>
            {allLanguages.map((l) => (
              <option key={l} value={l}>
                {LANG_BADGE[l] ?? l}
              </option>
            ))}
          </select>
        </label>
        <label>
          标签：
          <select value={tagFilter} onChange={(e) => setTagFilter((e.target as HTMLSelectElement).value)}>
            <option value="">全部</option>
            {allTags.map((t) => (
              <option key={t} value={t}>
                #{t}
              </option>
            ))}
          </select>
        </label>
        <span class="muted">
          {visible.length} / {skills.length} skills
        </span>
      </div>

      {visible.length === 0 && !loading ? (
        <div class="empty-state">
          <p>还没有技能。点击 <strong>+ 新建技能</strong> 创建第一个吧。</p>
        </div>
      ) : (
        <ul class="skill-cards">
          {visible.map((s) => (
            <li key={s.id} class="skill-card">
              <div class="skill-card__header">
                <h3>{s.name}</h3>
                <span class="skill-card__lang">{LANG_BADGE[s.language] ?? s.language}</span>
              </div>
              <p class="skill-card__desc">{s.description || <em>no description</em>}</p>
              <div class="skill-card__meta">
                <span>{formatRating(s.avg_rating, s.rating_count)}</span>
                <span>·</span>
                <span>{formatUsage(s.usage_count)}</span>
                {s.source_memory_id && (
                  <>
                    <span>·</span>
                    <span title={s.source_memory_id}>from memory</span>
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
                <button onClick={() => void runSkill(s)}>▶ Use</button>
                <button onClick={() => setRateOpen(s)}>★ Rate</button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {useResult && (
        <Modal title="▶ Skill output" onClose={() => setUseResult(null)}>
          <pre class="skill-output">{useResult.output || '(empty output)'}</pre>
        </Modal>
      )}

      {rateOpen && (
        <Modal title={`★ Rate "${rateOpen.name}"`} onClose={() => setRateOpen(null)}>
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
        <button onClick={onCancel}>取消</button>
        <button class="primary" onClick={() => onSubmit(value)}>
          提交
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
      await NineSnakeAPI.skillCreate({
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
    <Modal title="+ 新建技能" onClose={onClose}>
      <div class="form">
        <label>
          名称
          <input value={name} onInput={(e) => setName((e.target as HTMLInputElement).value)} />
        </label>
        <label>
          描述
          <input
            value={description}
            onInput={(e) => setDescription((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          语言
          <select value={language} onChange={(e) => setLanguage((e.target as HTMLSelectElement).value)}>
            <option value="rust">rust</option>
            <option value="python">python</option>
            <option value="javascript">javascript</option>
            <option value="bash">bash</option>
            <option value="llm">llm (prompt template)</option>
          </select>
        </label>
        <label>
          标签（逗号分隔）
          <input
            value={tags}
            placeholder="e.g. string, utility"
            onInput={(e) => setTags((e.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          代码
          <textarea
            rows={10}
            value={code}
            onInput={(e) => setCode((e.target as HTMLTextAreaElement).value)}
          />
        </label>
        {err && <div class="error">⚠ {err}</div>}
        <div class="modal__actions">
          <button onClick={onClose} disabled={busy}>
            取消
          </button>
          <button class="primary" onClick={submit} disabled={busy || !name.trim() || !code.trim()}>
            {busy ? '提交中…' : '创建'}
          </button>
        </div>
      </div>
    </Modal>
  );
}

function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: JSX.Element | JSX.Element[];
  onClose: () => void;
}): JSX.Element {
  return (
    <div class="modal-backdrop" onClick={onClose}>
      <div class="modal" onClick={(e) => e.stopPropagation()}>
        <header class="modal__header">
          <h3>{title}</h3>
          <button class="modal__close" onClick={onClose} aria-label="close">
            ✕
          </button>
        </header>
        <div class="modal__body">{children}</div>
      </div>
    </div>
  );
}
