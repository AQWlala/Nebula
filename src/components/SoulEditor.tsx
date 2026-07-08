/**
 * M6 #77: Soul 编辑器 UI — SOUL.md 双分区可视化编辑。
 *
 * ## 功能
 * - 加载当前 SOUL.md 内容(通过 `editor_read("SOUL.md")`,不存在时从
 *   `persona_get().soul_md` 取内存缓存)
 * - 解析双分区结构:
 *   - preamble: Section 之外自由文本(可编辑)
 *   - immutable_from_ai: AI 不可改区(只读,灰色展示)
 *   - evolution-append: 进化追加区(可编辑,EvolutionEngine Phase 4 写入)
 * - 实时校验 Section 标签配对(BEGIN/END 数量一致 + name 闭集)
 * - 保存:
 *   - 序列化为 SOUL.md 文本(保留 preamble + 两个 Section)
 *   - 调 `editor_write("SOUL.md", text)` 落盘
 *   - 调 `persona_set_file("soul", text)` 更新内存缓存
 *   - 调 `persona_reload()` 刷新 persona 状态
 *   - toast 提示成功/失败
 *
 * ## 不做的事
 * - 不调用 SoulCompiler 预览编译结果(需新增 Tauri 命令,M6 #77 范围外)
 * - 不直接修改 `immutable_from_ai` 区(强制只读,防用户破坏人格底色)
 * - 不支持添加 Section 之外的 Section name(闭集约束)
 *
 * ## 集成
 * 从 Settings.tsx 的 persona 卡片"编辑"按钮触发,作为 Modal 弹出。
 */
import { useState, useEffect, useMemo } from 'preact/hooks';
import { nebulaAPI } from '../lib/tauri';
import { Modal } from './Modal';
import { toast } from './Toast';
import { t } from '../i18n';

/** Section 标签语法(镜像 src-tauri/src/soul/structure.rs)。 */
const SECTION_BEGIN_PREFIX = '<!-- BEGIN SECTION: ';
const SECTION_END_PREFIX = '<!-- END SECTION: ';
const SECTION_SUFFIX = ' -->';

/** 已知 Section 名称闭集(镜像后端 KNOWN_SECTIONS)。 */
const KNOWN_SECTIONS = ['immutable_from_ai', 'evolution-append'] as const;
type SectionName = (typeof KNOWN_SECTIONS)[number];

/** 解析后的 SOUL.md 结构。 */
interface ParsedSoul {
  /** Section 之前的自由文本(可编辑)。 */
  preamble: string;
  /** immutable_from_ai section 内容(只读)。 */
  immutable_from_ai: string;
  /** evolution-append section 内容(可编辑)。 */
  evolution_append: string;
  /** Section 之间的自由文本(若有,合并到对应 section 后)。 */
  between: string;
  /** 末尾自由文本。 */
  trailing: string;
  /** 解析错误(标签不配对等)。 */
  errors: string[];
}

/** 空结构。 */
const EMPTY_SOUL: ParsedSoul = {
  preamble: '',
  immutable_from_ai: '',
  evolution_append: '',
  between: '',
  trailing: '',
  errors: [],
};

/**
 * 解析 SOUL.md 文本为分区结构。
 *
 * 简化版解析:仅识别 immutable_from_ai 和 evolution-append 两个 Section,
 * 不支持嵌套,Section 之外的所有文本合并到 preamble/between/trailing。
 *
 * 完整校验逻辑在后端 `parse_soul_md`,此处仅做前端预校验。
 */
function parseSoulMd(text: string): ParsedSoul {
  if (!text) return { ...EMPTY_SOUL };

  const lines = text.split('\n');
  const result: ParsedSoul = { ...EMPTY_SOUL };

  // 收集所有 BEGIN/END 标签的行号 + name
  const tags: { line: number; kind: 'begin' | 'end'; name: string }[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line.startsWith(SECTION_BEGIN_PREFIX) && line.endsWith(SECTION_SUFFIX)) {
      const name = line.slice(SECTION_BEGIN_PREFIX.length, -SECTION_SUFFIX.length).trim();
      tags.push({ line: i, kind: 'begin', name });
    } else if (line.startsWith(SECTION_END_PREFIX) && line.endsWith(SECTION_SUFFIX)) {
      const name = line.slice(SECTION_END_PREFIX.length, -SECTION_SUFFIX.length).trim();
      tags.push({ line: i, kind: 'end', name });
    }
  }

  // 校验:每个 begin 必须有同名 end,且不嵌套
  const errors: string[] = [];
  const stack: { name: string; beginLine: number }[] = [];
  for (const tag of tags) {
    if (tag.kind === 'begin') {
      if (!KNOWN_SECTIONS.includes(tag.name as SectionName)) {
        errors.push(
          t('soulEditor.parseError.unknownSection', { line: tag.line + 1, name: tag.name })
        );
      }
      if (stack.length > 0) {
        errors.push(
          t('soulEditor.parseError.nestedSection', {
            line: tag.line + 1,
            name: stack[stack.length - 1].name,
          })
        );
      }
      stack.push({ name: tag.name, beginLine: tag.line });
    } else {
      // end
      if (stack.length === 0) {
        errors.push(t('soulEditor.parseError.orphanEnd', { line: tag.line + 1 }));
        continue;
      }
      const top = stack.pop()!;
      if (top.name !== tag.name) {
        errors.push(
          t('soulEditor.parseError.mismatch', {
            line: tag.line + 1,
            endName: tag.name,
            beginName: top.name,
            beginLine: top.beginLine + 1,
          })
        );
      }
    }
  }
  for (const unmatched of stack) {
    errors.push(
      t('soulEditor.parseError.missingEnd', { line: unmatched.beginLine + 1, name: unmatched.name })
    );
  }
  result.errors = errors;

  // 提取 Section 内容(即使有错误也尽量提取)
  // 简化:遍历 tags 配对,取 begin+1 ~ end-1 行
  const pairs: { name: string; beginLine: number; endLine: number }[] = [];
  const beginStack: { name: string; line: number }[] = [];
  for (const tag of tags) {
    if (tag.kind === 'begin') {
      beginStack.push({ name: tag.name, line: tag.line });
    } else {
      const top = beginStack.pop();
      if (top && top.name === tag.name) {
        pairs.push({ name: top.name, beginLine: top.line, endLine: tag.line });
      }
    }
  }

  // 提取 preamble(第一个 begin 之前)
  const firstBegin = pairs.length > 0 ? Math.min(...pairs.map((p) => p.beginLine)) : lines.length;
  result.preamble = lines.slice(0, firstBegin).join('\n').replace(/\n$/, '');

  // 提取每个 section 内容
  for (const pair of pairs) {
    const content = lines.slice(pair.beginLine + 1, pair.endLine).join('\n');
    if (pair.name === 'immutable_from_ai') {
      result.immutable_from_ai = content;
    } else if (pair.name === 'evolution-append') {
      result.evolution_append = content;
    }
  }

  // 提取 between 和 trailing(简化:between = 两个 section 之间,
  // trailing = 最后 section 之后。仅当两个 section 都存在时才计算 between)
  if (pairs.length === 2) {
    const sorted = [...pairs].sort((a, b) => a.beginLine - b.beginLine);
    const first = sorted[0];
    const second = sorted[1];
    result.between = lines
      .slice(first.endLine + 1, second.beginLine)
      .join('\n')
      .replace(/^\n+/, '')
      .replace(/\n+$/, '');
    result.trailing = lines
      .slice(second.endLine + 1)
      .join('\n')
      .replace(/^\n+/, '')
      .replace(/\n+$/, '');
  } else if (pairs.length === 1) {
    result.trailing = lines
      .slice(pairs[0].endLine + 1)
      .join('\n')
      .replace(/^\n+/, '')
      .replace(/\n+$/, '');
  }

  return result;
}

/** 序列化回 SOUL.md 文本。 */
function serializeSoulMd(parsed: ParsedSoul): string {
  const parts: string[] = [];

  if (parsed.preamble.trim()) {
    parts.push(parsed.preamble);
  }

  // immutable_from_ai section(只读,但序列化时保留原内容)
  parts.push(`${SECTION_BEGIN_PREFIX}immutable_from_ai${SECTION_SUFFIX}`);
  if (parsed.immutable_from_ai) {
    parts.push(parsed.immutable_from_ai);
  }
  parts.push(`${SECTION_END_PREFIX}immutable_from_ai${SECTION_SUFFIX}`);

  if (parsed.between.trim()) {
    parts.push(parsed.between);
  }

  // evolution-append section
  parts.push(`${SECTION_BEGIN_PREFIX}evolution-append${SECTION_SUFFIX}`);
  if (parsed.evolution_append) {
    parts.push(parsed.evolution_append);
  }
  parts.push(`${SECTION_END_PREFIX}evolution-append${SECTION_SUFFIX}`);

  if (parsed.trailing.trim()) {
    parts.push(parsed.trailing);
  }

  return parts.join('\n');
}

interface SoulEditorProps {
  open: boolean;
  onClose: () => void;
}

export function SoulEditor({ open, onClose }: SoulEditorProps) {
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [rawText, setRawText] = useState('');
  // 可编辑部分:preamble / evolution_append / between / trailing
  const [preamble, setPreamble] = useState('');
  const [evolutionAppend, setEvolutionAppend] = useState('');
  const [between, setBetween] = useState('');
  const [trailing, setTrailing] = useState('');
  // 只读部分
  const [immutableContent, setImmutableContent] = useState('');
  // 解析错误
  const [parseErrors, setParseErrors] = useState<string[]>([]);
  // 是否有 immutable_from_ai section(用户初次创建时可能没有)
  const [hasImmutable, setHasImmutable] = useState(false);

  // 加载 SOUL.md
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    (async () => {
      try {
        let text = '';
        // 先尝试从磁盘读取
        try {
          const file = await nebulaAPI.editorRead('SOUL.md');
          text = file.content;
        } catch {
          // 磁盘没有,从内存 persona 缓存读
          try {
            const persona = await nebulaAPI.personaGet();
            text = persona.soul_md ?? '';
          } catch {
            text = '';
          }
        }
        if (cancelled) return;
        setRawText(text);
        const parsed = parseSoulMd(text);
        setPreamble(parsed.preamble);
        setImmutableContent(parsed.immutable_from_ai);
        setEvolutionAppend(parsed.evolution_append);
        setBetween(parsed.between);
        setTrailing(parsed.trailing);
        setParseErrors(parsed.errors);
        setHasImmutable(parsed.immutable_from_ai.length > 0 || text.includes('immutable_from_ai'));
      } catch (e) {
        toast.error(t('soulEditor.toast.loadFailed.title'), String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  // 当前序列化文本(用于 dirty 检测)
  const currentText = useMemo(() => {
    return serializeSoulMd({
      preamble,
      immutable_from_ai: immutableContent,
      evolution_append: evolutionAppend,
      between,
      trailing,
      errors: [],
    });
  }, [preamble, immutableContent, evolutionAppend, between, trailing]);

  const isDirty = currentText !== rawText;

  // 保存
  async function handleSave() {
    setSaving(true);
    try {
      // 落盘
      await nebulaAPI.editorWrite('SOUL.md', currentText);
      // 更新内存缓存
      await nebulaAPI.personaSetFile('soul', currentText);
      // 刷新 persona
      await nebulaAPI.personaReload();
      setRawText(currentText);
      toast.success(t('soulEditor.toast.saved.title'), t('soulEditor.toast.saved.body'));
    } catch (e) {
      toast.error(t('soulEditor.toast.saveFailed.title'), String(e));
    } finally {
      setSaving(false);
    }
  }

  // 重置为原始内容
  function handleReset() {
    const parsed = parseSoulMd(rawText);
    setPreamble(parsed.preamble);
    setImmutableContent(parsed.immutable_from_ai);
    setEvolutionAppend(parsed.evolution_append);
    setBetween(parsed.between);
    setTrailing(parsed.trailing);
    setParseErrors(parsed.errors);
    setHasImmutable(parsed.immutable_from_ai.length > 0 || rawText.includes('immutable_from_ai'));
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('soulEditor.title')}
      size="lg"
      footer={
        <>
          <button
            type="button"
            onClick={handleReset}
            disabled={!isDirty || saving || loading}
            style={{
              fontSize: '12px',
              padding: '6px 12px',
              borderRadius: '4px',
              border: '1px solid var(--border)',
              background: 'transparent',
              color: 'var(--text-primary)',
              cursor: !isDirty || saving || loading ? 'not-allowed' : 'pointer',
              opacity: !isDirty || saving || loading ? 0.5 : 1,
            }}
          >
            {t('soulEditor.reset')}
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={!isDirty || saving || loading || parseErrors.length > 0}
            style={{
              fontSize: '12px',
              padding: '6px 14px',
              borderRadius: '4px',
              border: 'none',
              background: parseErrors.length > 0 ? 'var(--text-muted)' : 'var(--accent-neon)',
              color: parseErrors.length > 0 ? 'var(--text-secondary)' : 'var(--bg-primary)',
              cursor:
                !isDirty || saving || loading || parseErrors.length > 0 ? 'not-allowed' : 'pointer',
              opacity: !isDirty || saving || loading ? 0.5 : 1,
            }}
          >
            {saving ? t('soulEditor.saving') : t('soulEditor.save')}
          </button>
        </>
      }
    >
      {loading ? (
        <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-muted)' }}>
          {t('soulEditor.loading')}
        </div>
      ) : (
        <div class="soul-editor">
          {/* 解析错误提示 */}
          {parseErrors.length > 0 && (
            <div class="soul-editor-errors">
              <strong>{t('soulEditor.parseError.title', { count: parseErrors.length })}</strong>
              <ul>
                {parseErrors.map((err, i) => (
                  <li key={i}>{err}</li>
                ))}
              </ul>
              <div style="font-size: 11px; color: var(--text-muted); margin-top: 4px;">
                {t('soulEditor.parseError.hint')}
              </div>
            </div>
          )}

          {/* 说明 */}
          <div class="soul-editor-hint">{t('soulEditor.hint.structure')}</div>

          {/* preamble(可编辑) */}
          <div class="soul-section soul-section-editable">
            <div class="soul-section-header">
              <span class="soul-section-label">{t('soulEditor.section.preamble.title')}</span>
              <span class="soul-section-tag">{t('soulEditor.tag.editable')}</span>
            </div>
            <div class="soul-section-desc">{t('soulEditor.section.preamble.desc')}</div>
            <textarea
              class="soul-textarea"
              value={preamble}
              onInput={(e) => setPreamble((e.target as HTMLTextAreaElement).value)}
              placeholder={t('soulEditor.section.preamble.placeholder')}
              rows={3}
            />
          </div>

          {/* immutable_from_ai(只读) */}
          <div class="soul-section soul-section-readonly">
            <div class="soul-section-header">
              <span class="soul-section-label">🔒 immutable_from_ai</span>
              <span class="soul-section-tag soul-tag-readonly">{t('soulEditor.tag.readonly')}</span>
            </div>
            <div class="soul-section-desc">{t('soulEditor.section.immutable.desc')}</div>
            {hasImmutable ? (
              <pre class="soul-readonly-content">
                {immutableContent || t('soulEditor.section.immutable.empty')}
              </pre>
            ) : (
              <div class="soul-section-empty">{t('soulEditor.section.immutable.missing')}</div>
            )}
          </div>

          {/* between(可编辑,仅当两个 section 都存在时显示) */}
          {hasImmutable && between.trim() && (
            <div class="soul-section soul-section-editable">
              <div class="soul-section-header">
                <span class="soul-section-label">{t('soulEditor.section.between.title')}</span>
                <span class="soul-section-tag">{t('soulEditor.tag.editable')}</span>
              </div>
              <div class="soul-section-desc">{t('soulEditor.section.between.desc')}</div>
              <textarea
                class="soul-textarea"
                value={between}
                onInput={(e) => setBetween((e.target as HTMLTextAreaElement).value)}
                rows={2}
              />
            </div>
          )}

          {/* evolution-append(可编辑) */}
          <div class="soul-section soul-section-editable">
            <div class="soul-section-header">
              <span class="soul-section-label">
                {t('soulEditor.section.evolutionAppend.title')}
              </span>
              <span class="soul-section-tag">{t('soulEditor.tag.editable')}</span>
            </div>
            <div class="soul-section-desc">{t('soulEditor.section.evolutionAppend.desc')}</div>
            <textarea
              class="soul-textarea soul-textarea-large"
              value={evolutionAppend}
              onInput={(e) => setEvolutionAppend((e.target as HTMLTextAreaElement).value)}
              placeholder={t('soulEditor.section.evolutionAppend.placeholder')}
              rows={8}
            />
          </div>

          {/* trailing(可编辑) */}
          {trailing.trim() && (
            <div class="soul-section soul-section-editable">
              <div class="soul-section-header">
                <span class="soul-section-label">{t('soulEditor.section.trailing.title')}</span>
                <span class="soul-section-tag">{t('soulEditor.tag.editable')}</span>
              </div>
              <textarea
                class="soul-textarea"
                value={trailing}
                onInput={(e) => setTrailing((e.target as HTMLTextAreaElement).value)}
                rows={2}
              />
            </div>
          )}

          {/* dirty 指示器 */}
          {isDirty && <div class="soul-dirty-indicator">{t('soulEditor.dirtyIndicator')}</div>}
        </div>
      )}
    </Modal>
  );
}
