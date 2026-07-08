/**
 * T-E-S-38 VisualCreatorDialog — 可视化生成弹窗。
 *
 * 布局:左侧描述输入 + 右侧预览,Tab 切换三种 creator
 * (canvas-creator / mermaid-creator / mindmap-creator)。
 *
 * 调用 nebulaAPI.skillUse 复用既有 skillUse API,不新增 Tauri 命令。
 * 渲染分发由 VizRenderer 处理(canvas iframe / mermaid MermaidView)。
 */

import { useState, useCallback } from 'preact/hooks';
import { nebulaAPI, SkillResult } from '../lib/tauri';
import VizRenderer, { VizKind } from './VizRenderer';
import { Spinner } from './Spinner';
import { t } from '../i18n';

interface VisualCreatorDialogProps {
  /** 关闭弹窗回调。 */
  onClose: () => void;
  /** 初始 creator(kind),默认 'canvas'。 */
  initialKind?: VizKind;
}

interface CreatorConfig {
  kind: VizKind;
  /** 对应的内置 skill name(seeder.rs 注册)。 */
  skillName: string;
}

/** i18n label/description/placeholder lookups: convert const Record fields to function form. */
const creatorLabel = (k: VizKind): string => t(`visualCreatorDialog.${k}.label`);
const creatorDescription = (k: VizKind): string => t(`visualCreatorDialog.${k}.description`);
const creatorPlaceholder = (k: VizKind): string => t(`visualCreatorDialog.${k}.placeholder`);

const CREATORS: CreatorConfig[] = [
  { kind: 'canvas', skillName: 'canvas-creator' },
  { kind: 'mermaid', skillName: 'mermaid-creator' },
  { kind: 'mindmap', skillName: 'mindmap-creator' },
];

export default function VisualCreatorDialog({
  onClose,
  initialKind = 'canvas',
}: VisualCreatorDialogProps) {
  const [kind, setKind] = useState<VizKind>(initialKind);
  const [description, setDescription] = useState('');
  const [output, setOutput] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // skillId 缓存:首次生成时通过 skillList 查找,避免重复查询。
  const [skillIdCache, setSkillIdCache] = useState<Record<string, string>>({});

  const currentCreator = CREATORS.find((c) => c.kind === kind) ?? CREATORS[0];

  const handleGenerate = useCallback(async () => {
    const desc = description.trim();
    if (!desc) return;
    setLoading(true);
    setError(null);
    setOutput('');
    try {
      // 查找 skill id(缓存命中则直接用)。
      let skillId = skillIdCache[currentCreator.skillName];
      if (!skillId) {
        const skills = await nebulaAPI.skillList({ limit: 200 });
        const found = skills?.find((s) => s.name === currentCreator.skillName);
        if (!found) {
          setError(t('visualCreatorDialog.skillNotFound', { name: currentCreator.skillName }));
          setLoading(false);
          return;
        }
        skillId = found.id;
        setSkillIdCache((prev) => ({
          ...prev,
          [currentCreator.skillName]: skillId as string,
        }));
      }

      // 调用 skillUse,params 含 description 字段。
      // 后端 execute_llm 会把 params 序列化为 JSON 注入 prompt。
      const result: SkillResult = await nebulaAPI.skillUse({
        id: skillId,
        params: { description: desc, INPUT: desc },
      });
      setOutput(result?.output ?? '');
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(t('visualCreatorDialog.generateFailed', { error: msg }));
    } finally {
      setLoading(false);
    }
  }, [description, currentCreator, skillIdCache]);

  const handleTabSwitch = (newKind: VizKind) => {
    setKind(newKind);
    setOutput('');
    setError(null);
  };

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div class="w-[90vw] h-[85vh] max-w-6xl bg-[#1E293B] rounded-lg border border-gray-700 flex flex-col overflow-hidden">
        {/* Header */}
        <div class="flex items-center justify-between px-5 py-3 border-b border-gray-700">
          <div class="flex items-center gap-2">
            <h2 class="text-lg font-semibold text-white">{t('visualCreatorDialog.title')}</h2>
            <span class="text-xs text-gray-500">T-E-S-38</span>
          </div>
          <button
            onClick={onClose}
            class="text-gray-400 hover:text-white text-xl leading-none px-2"
            title={t('visualCreatorDialog.close')}
          >
            ×
          </button>
        </div>

        {/* Tabs */}
        <div class="flex gap-1 px-5 py-2 border-b border-gray-700 bg-gray-800/50">
          {CREATORS.map((c) => (
            <button
              key={c.kind}
              onClick={() => handleTabSwitch(c.kind)}
              class={`px-3 py-1.5 text-sm rounded-md transition-colors ${
                kind === c.kind
                  ? 'bg-blue-600 text-white'
                  : 'text-gray-400 hover:text-white hover:bg-gray-700'
              }`}
            >
              {creatorLabel(c.kind)}
            </button>
          ))}
        </div>

        {/* Body: 左侧输入 + 右侧预览 */}
        <div class="flex-1 flex overflow-hidden">
          {/* Left: description input */}
          <div class="w-2/5 border-r border-gray-700 flex flex-col p-4 gap-3">
            <div>
              <label class="block text-xs text-gray-400 mb-1">
                {creatorLabel(currentCreator.kind)}
              </label>
              <p class="text-xs text-gray-500">{creatorDescription(currentCreator.kind)}</p>
            </div>
            <textarea
              value={description}
              onInput={(e) => setDescription((e.target as HTMLTextAreaElement).value)}
              placeholder={creatorPlaceholder(currentCreator.kind)}
              class="flex-1 w-full p-3 bg-gray-800 border border-gray-600 rounded-md text-sm
                     text-gray-200 placeholder-gray-500 focus:outline-none focus:border-blue-500
                     resize-none font-mono"
            />
            <div class="flex items-center justify-between gap-2">
              <span class="text-xs text-gray-500">
                {t('visualCreatorDialog.charCount', { count: description.length })}
              </span>
              <button
                onClick={handleGenerate}
                disabled={loading || !description.trim()}
                class="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700
                       disabled:text-gray-500 text-white text-sm rounded-md transition-colors"
              >
                {loading ? (
                  <Spinner size={16} showLabel={false} />
                ) : (
                  t('visualCreatorDialog.generate')
                )}
              </button>
            </div>
          </div>

          {/* Right: preview */}
          <div class="flex-1 flex flex-col p-4 gap-2 min-w-0">
            <div class="flex items-center justify-between">
              <span class="text-xs text-gray-400">{t('visualCreatorDialog.preview')}</span>
              {output && (
                <span class="text-xs text-gray-500">
                  {t('visualCreatorDialog.charCount', { count: output.length })}
                </span>
              )}
            </div>
            {error && (
              <div class="px-3 py-2 bg-red-900/30 border border-red-700 text-red-300 text-xs rounded-md">
                ❌ {error}
              </div>
            )}
            <div class="flex-1 min-h-0 bg-gray-900 rounded-md overflow-hidden">
              {loading && !output ? (
                <div class="flex items-center justify-center h-full text-gray-400 text-sm">
                  <div class="flex items-center gap-2">
                    <span class="inline-block w-3 h-3 border-2 border-blue-500 border-t-transparent rounded-full animate-spin" />
                    {t('visualCreatorDialog.callingLLM')}
                  </div>
                </div>
              ) : (
                <VizRenderer kind={kind} output={output} />
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
