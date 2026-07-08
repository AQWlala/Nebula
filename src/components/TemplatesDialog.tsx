/**
 * T-E-C-13: 工作场景模板选择器对话框。
 *
 * 左侧分类树(写作 / 编码 / 管理)+ 右侧模板卡片网格。
 * 用户点击卡片上的"使用"按钮后,弹出用户输入框,确认后:
 *   1. 调用 `scenarioInstantiate({ id, user_input })` 拿到 `SwarmTask`
 *   2. 调用 `swarmExecute(task)` 启动蜂群
 *   3. 关闭对话框
 *
 * 模板数据由后端 `scenarios.json` 静态嵌入,通过 `scenario_list` 命令一次性拉取。
 */
import { useState, useEffect, useMemo } from 'preact/hooks';
import {
  nebulaAPI,
  type ScenarioTemplate,
  type ScenarioCategory,
  type SwarmTask,
  type LoopTemplateSummary,
} from '../lib/tauri';
import { toast } from './Toast';
import { Spinner } from './Spinner';
import { t } from '../i18n';

interface TemplatesDialogProps {
  onClose: () => void;
  /** 蜂群启动后的回调(供 ChatPanel 注入消息/滚动等)。 */
  onSwarmStarted?: (task: SwarmTask) => void;
}

/**
 * T-E-L-05: 左侧分类树条目类型。
 * - scenario: 传统工作场景模板(writing/coding/management)
 * - automation: Loop 自动化模板(T-E-L-05 新增,默认只露 2 个入口)
 */
type CategoryKey = ScenarioCategory | 'automation';

/** 分类元信息:icon 图标。 */
const CATEGORY_META: Record<CategoryKey, { icon: string }> = {
  writing: { icon: '✍️' },
  coding: { icon: '💻' },
  management: { icon: '📋' },
  automation: { icon: '🔄' },
};

/** 分类顺序(左侧树从上到下)。automation 放最后(高级功能)。 */
const CATEGORY_ORDER: CategoryKey[] = ['writing', 'coding', 'management', 'automation'];

/** T-E-L-05: automation 类别默认展示的 Loop 模板数量(收起状态)。 */
const AUTOMATION_DEFAULT_COUNT = 2;

/** 角色 badge 颜色映射 + i18n label。 */
const ROLE_BADGE: Record<string, { color: string; bg: string }> = {
  writer: { color: '#2196f3', bg: 'rgba(33,150,243,0.12)' },
  coder: { color: '#4caf50', bg: 'rgba(76,175,80,0.12)' },
  manager: { color: '#ff9800', bg: 'rgba(255,152,0,0.12)' },
};

/** i18n label lookups: convert const Record maps to function form. */
const categoryLabel = (c: CategoryKey): string => t(`templatesDialog.category.${c}`);
const roleLabel = (r: string): string => {
  // only known roles have translation keys; unknown roles fall back to raw text.
  if (r === 'writer' || r === 'coder' || r === 'manager') {
    return t(`templatesDialog.role.${r}`);
  }
  return r;
};

export function TemplatesDialog({ onClose, onSwarmStarted }: TemplatesDialogProps) {
  const [templates, setTemplates] = useState<ScenarioTemplate[]>([]);
  // T-E-L-05: Loop 自动化模板(独立拉取,master-orchestrator feature 门控)。
  const [loopTemplates, setLoopTemplates] = useState<LoopTemplateSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedCategory, setSelectedCategory] = useState<CategoryKey>('writing');
  // T-E-L-05: automation 类别展开状态(false=只露 2 个,true=全部)。
  const [automationExpanded, setAutomationExpanded] = useState(false);
  // 当前选中要实例化的模板(id)。非 null 时显示用户输入弹层。
  const [pendingTemplate, setPendingTemplate] = useState<ScenarioTemplate | null>(null);
  const [userInput, setUserInput] = useState('');
  const [submitting, setSubmitting] = useState(false);

  // 拉取全部场景模板(一次性)。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await nebulaAPI.scenarioList(null);
        if (!cancelled) {
          setTemplates(list);
        }
      } catch (e) {
        toast.error(t('templatesDialog.loadFailed'), String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // T-E-L-05: 拉取 Loop 模板(独立请求,master-orchestrator 未启用时降级为空)。
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await nebulaAPI.loopTemplatesList();
        if (!cancelled) {
          setLoopTemplates(list);
        }
      } catch (e) {
        // master-orchestrator feature 未启用时后端返回 command not found,
        // 降级为空列表(automation 类别显示"暂无模板"),不弹 toast 避免噪音。
        if (!cancelled) {
          setLoopTemplates([]);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // 按分类分组(场景模板)。
  const grouped = useMemo(() => {
    const map: Record<ScenarioCategory, ScenarioTemplate[]> = {
      writing: [],
      coding: [],
      management: [],
    };
    for (const tpl of templates) {
      map[tpl.category].push(tpl);
    }
    return map;
  }, [templates]);

  // T-E-L-05: automation 类别当前展示的 Loop 模板列表。
  // 收起时只露 AUTOMATION_DEFAULT_COUNT 个,展开时全部。
  const automationList = useMemo(() => {
    if (automationExpanded) return loopTemplates;
    return loopTemplates.slice(0, AUTOMATION_DEFAULT_COUNT);
  }, [loopTemplates, automationExpanded]);

  // 当前选中类别的模板列表(scenario 类别用 grouped,automation 用 automationList)。
  const isAutomation = selectedCategory === 'automation';
  const currentList = isAutomation ? [] : grouped[selectedCategory as ScenarioCategory] ?? [];

  /** 提交用户输入,实例化模板并启动蜂群。 */
  async function handleSubmit() {
    if (!pendingTemplate) return;
    if (!userInput.trim()) {
      toast.warning(t('templatesDialog.inputRequired'));
      return;
    }
    setSubmitting(true);
    try {
      const task = await nebulaAPI.scenarioInstantiate({
        id: pendingTemplate.id,
        user_input: userInput.trim(),
      });
      if (!task) {
        toast.error(t('templatesDialog.templateNotFound'), t('templatesDialog.templateNotFoundBody', { id: pendingTemplate.id }));
        return;
      }
      // 启动蜂群。
      await nebulaAPI.swarmExecute(task);
      toast.success(t('templatesDialog.swarmStarted'), t('templatesDialog.swarmStartedBody', { name: pendingTemplate.name }));
      onSwarmStarted?.(task);
      // 关闭整个对话框。
      onClose();
    } catch (e) {
      toast.error(t('templatesDialog.swarmStartFailed'), String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      class="modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget && !submitting) onClose();
      }}
    >
      <div class="modal" style={{ minWidth: '720px', maxWidth: '900px', maxHeight: '80vh' }}>
        <div class="modal__header">
          <h3>{t('templatesDialog.title')}</h3>
          <button
            class="modal__close"
            onClick={onClose}
            aria-label={t('templatesDialog.close')}
            disabled={submitting}
          >
            ×
          </button>
        </div>

        <div class="modal__body" style={{ padding: 0, overflow: 'hidden' }}>
          {loading ? (
            <div
              style={{
                padding: '40px',
                textAlign: 'center',
              }}
            >
              <Spinner label={t('common.loading')} />
            </div>
          ) : (
            <div style={{ display: 'flex', height: '480px' }}>
              {/* 左侧分类树 */}
              <div
                style={{
                  width: '160px',
                  borderRight: '1px solid var(--border)',
                  padding: '8px 0',
                  overflowY: 'auto',
                  background: 'var(--bg-secondary, transparent)',
                }}
              >
                {CATEGORY_ORDER.map((cat) => {
                  const meta = CATEGORY_META[cat];
                  // T-E-L-05: automation 类别计数来自 loopTemplates,非 grouped。
                  const count =
                    cat === 'automation' ? loopTemplates.length : grouped[cat].length;
                  const active = selectedCategory === cat;
                  return (
                    <button
                      key={cat}
                      onClick={() => setSelectedCategory(cat)}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        width: '100%',
                        padding: '10px 14px',
                        cursor: 'pointer',
                        border: 'none',
                        borderLeft: active
                          ? '3px solid var(--accent-neon)'
                          : '3px solid transparent',
                        background: active
                          ? 'rgba(var(--accent-rgb), 0.08)'
                          : 'transparent',
                        color: active ? 'var(--accent-neon)' : 'var(--text-primary)',
                        fontSize: '13px',
                        fontWeight: active ? 600 : 400,
                        textAlign: 'left',
                        transition: 'all 0.15s',
                      }}
                    >
                      <span>
                        {meta.icon} {categoryLabel(cat)}
                      </span>
                      <span style={{ fontSize: '11px', opacity: 0.7 }}>{count}</span>
                    </button>
                  );
                })}
              </div>

              {/* 右侧模板卡片网格 */}
              <div
                style={{
                  flex: 1,
                  padding: '12px',
                  overflowY: 'auto',
                }}
              >
                {isAutomation ? (
                  // T-E-L-05: automation 类别 — 渲染 Loop 模板卡片
                  // 默认只露 AUTOMATION_DEFAULT_COUNT 个,展开按钮切换全部/收起
                  automationList.length === 0 ? (
                    <div
                      style={{
                        padding: '40px',
                        textAlign: 'center',
                        color: 'var(--text-muted)',
                      }}
                    >
                      {t('templatesDialog.categoryEmpty')}
                    </div>
                  ) : (
                    <div>
                      <div
                        style={{
                          display: 'grid',
                          gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))',
                          gap: '10px',
                        }}
                      >
                        {automationList.map((tpl) => (
                          <LoopTemplateCard key={tpl.name} template={tpl} />
                        ))}
                      </div>
                      {/* 展开/收起按钮:Loop 模板超过默认数量时显示 */}
                      {loopTemplates.length > AUTOMATION_DEFAULT_COUNT && (
                        <button
                          onClick={() => setAutomationExpanded(!automationExpanded)}
                          style={{
                            display: 'block',
                            margin: '12px auto 0',
                            padding: '6px 16px',
                            cursor: 'pointer',
                            border: '1px solid var(--border)',
                            borderRadius: '4px',
                            background: 'transparent',
                            color: 'var(--accent-neon)',
                            fontSize: '12px',
                            fontWeight: 600,
                            transition: 'all 0.15s',
                          }}
                        >
                          {automationExpanded
                            ? t('templatesDialog.automationCollapse')
                            : t('templatesDialog.automationExpand', {
                                count: loopTemplates.length - AUTOMATION_DEFAULT_COUNT,
                              })}
                        </button>
                      )}
                    </div>
                  )
                ) : currentList.length === 0 ? (
                  <div
                    style={{
                      padding: '40px',
                      textAlign: 'center',
                      color: 'var(--text-muted)',
                    }}
                  >
                    {t('templatesDialog.categoryEmpty')}
                  </div>
                ) : (
                  <div
                    style={{
                      display: 'grid',
                      gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))',
                      gap: '10px',
                    }}
                  >
                    {currentList.map((tpl) => (
                      <TemplateCard
                        key={tpl.id}
                        template={tpl}
                        onUse={() => {
                          setPendingTemplate(tpl);
                          setUserInput('');
                        }}
                      />
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>

        {/* 用户输入弹层(选择模板后显示) */}
        {pendingTemplate && (
          <div
            style={{
              position: 'absolute',
              inset: 0,
              background: 'rgba(0,0,0,0.5)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              zIndex: 10,
            }}
            onClick={(e) => {
              if (e.target === e.currentTarget && !submitting) {
                setPendingTemplate(null);
              }
            }}
          >
            <div
              class="modal"
              style={{ minWidth: '480px', maxWidth: '600px' }}
            >
              <div class="modal__header">
                <h3>{t('templatesDialog.useTemplate', { name: pendingTemplate.name })}</h3>
                <button
                  class="modal__close"
                  onClick={() => !submitting && setPendingTemplate(null)}
                  aria-label={t('templatesDialog.close')}
                  disabled={submitting}
                >
                  ×
                </button>
              </div>
              <div class="modal__body">
                <div style={{ marginBottom: '8px', fontSize: '12px', color: 'var(--text-muted)' }}>
                  {pendingTemplate.description}
                </div>
                <div
                  style={{
                    marginBottom: '12px',
                    padding: '8px',
                    background: 'var(--bg-secondary, transparent)',
                    border: '1px solid var(--border)',
                    borderRadius: '4px',
                    fontSize: '11px',
                    color: 'var(--text-secondary)',
                    maxHeight: '120px',
                    overflowY: 'auto',
                    whiteSpace: 'pre-wrap',
                  }}
                >
                  <strong>{t('templatesDialog.systemPrompt')}</strong> {pendingTemplate.system_prompt}
                </div>
                <label
                  style={{
                    display: 'block',
                    marginBottom: '6px',
                    fontSize: '12px',
                    color: 'var(--text-primary)',
                  }}
                >
                  {t('templatesDialog.taskContentLabel')}
                </label>
                <textarea
                  value={userInput}
                  onInput={(e) =>
                    setUserInput((e.target as HTMLTextAreaElement).value)
                  }
                  placeholder={t('templatesDialog.taskContentPlaceholder')}
                  rows={4}
                  autoFocus
                  style={{
                    width: '100%',
                    padding: '8px 10px',
                    fontSize: '13px',
                    border: '1px solid var(--border)',
                    borderRadius: '4px',
                    background: 'transparent',
                    color: 'var(--text-primary)',
                    resize: 'vertical',
                    fontFamily: 'inherit',
                  }}
                />
              </div>
              <div class="modal__actions">
                <button
                  onClick={() => setPendingTemplate(null)}
                  disabled={submitting}
                  style={{
                    padding: '6px 14px',
                    cursor: submitting ? 'not-allowed' : 'pointer',
                    border: '1px solid var(--border)',
                    borderRadius: '4px',
                    background: 'transparent',
                    color: 'var(--text-muted)',
                    fontSize: '12px',
                  }}
                >
                  {t('templatesDialog.cancel')}
                </button>
                <button
                  onClick={handleSubmit}
                  disabled={submitting || !userInput.trim()}
                  style={{
                    padding: '6px 14px',
                    cursor:
                      submitting || !userInput.trim() ? 'not-allowed' : 'pointer',
                    border: '1px solid var(--accent-neon)',
                    borderRadius: '4px',
                    background: 'var(--accent-neon)',
                    color: '#000',
                    fontSize: '12px',
                    fontWeight: 600,
                    opacity: submitting || !userInput.trim() ? 0.5 : 1,
                  }}
                >
                  {submitting ? t('templatesDialog.starting') : t('templatesDialog.startSwarm')}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 子组件:模板卡片
// ---------------------------------------------------------------------------

interface TemplateCardProps {
  template: ScenarioTemplate;
  onUse: () => void;
}

function TemplateCard({ template, onUse }: TemplateCardProps) {
  const roleBadge = ROLE_BADGE[template.role] ?? {
    color: 'var(--text-muted)',
    bg: 'transparent',
  };
  const isBase = template.id.endsWith('-base');

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        padding: '10px',
        border: '1px solid var(--border)',
        borderRadius: '6px',
        background: isBase ? 'rgba(var(--accent-rgb), 0.04)' : 'transparent',
        transition: 'all 0.15s',
        minHeight: '140px',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.borderColor = 'var(--accent-neon)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.borderColor = 'var(--border)';
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: '6px',
        }}
      >
        <strong style={{ fontSize: '13px', color: 'var(--text-primary)' }}>
          {isBase ? '⭐ ' : ''}
          {template.name}
        </strong>
        <span
          style={{
            fontSize: '10px',
            padding: '1px 6px',
            borderRadius: '3px',
            color: roleBadge.color,
            background: roleBadge.bg,
            border: `1px solid ${roleBadge.color}`,
          }}
        >
          {roleLabel(template.role)}
        </span>
      </div>
      <div
        style={{
          fontSize: '11px',
          color: 'var(--text-secondary)',
          flex: 1,
          marginBottom: '8px',
          overflow: 'hidden',
          display: '-webkit-box',
          WebkitLineClamp: 3,
          WebkitBoxOrient: 'vertical',
        }}
      >
        {template.description}
      </div>
      <div
        style={{
          display: 'flex',
          flexWrap: 'wrap',
          gap: '4px',
          marginBottom: '8px',
        }}
      >
        {template.agents.slice(0, 4).map((a) => (
          <span
            key={`${a.kind}-${a.role}`}
            style={{
              fontSize: '10px',
              padding: '1px 5px',
              borderRadius: '3px',
              background: 'var(--bg-secondary, rgba(255,255,255,0.05))',
              color: 'var(--text-muted)',
              border: '1px solid var(--border)',
            }}
          >
            {a.kind}
          </span>
        ))}
        {template.agents.length > 4 && (
          <span
            style={{
              fontSize: '10px',
              padding: '1px 5px',
              color: 'var(--text-muted)',
            }}
          >
            +{template.agents.length - 4}
          </span>
        )}
      </div>
      <button
        onClick={onUse}
        style={{
          padding: '4px 10px',
          cursor: 'pointer',
          border: '1px solid var(--accent-neon)',
          borderRadius: '4px',
          background: 'transparent',
          color: 'var(--accent-neon)',
          fontSize: '11px',
          fontWeight: 600,
          transition: 'all 0.15s',
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLButtonElement).style.background =
            'var(--accent-neon)';
          (e.currentTarget as HTMLButtonElement).style.color = '#000';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLButtonElement).style.background = 'transparent';
          (e.currentTarget as HTMLButtonElement).style.color =
            'var(--accent-neon)';
        }}
      >
        {t('templatesDialog.use')}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// T-E-L-05: Loop 模板卡片子组件
// ---------------------------------------------------------------------------

/** Loop 自主度等级 badge 颜色映射。 */
const AUTONOMY_BADGE: Record<string, { color: string; bg: string }> = {
  L0: { color: '#9e9e9e', bg: 'rgba(158,158,158,0.12)' },
  L1: { color: '#2196f3', bg: 'rgba(33,150,243,0.12)' },
  L2: { color: '#4caf50', bg: 'rgba(76,175,80,0.12)' },
  L3: { color: '#ff9800', bg: 'rgba(255,152,0,0.12)' },
  L4: { color: '#e91e63', bg: 'rgba(233,30,99,0.12)' },
  L5: { color: '#9c27b0', bg: 'rgba(156,39,176,0.12)' },
};

interface LoopTemplateCardProps {
  template: LoopTemplateSummary;
}

/**
 * T-E-L-05: Loop 模板卡片 — 展示 Loop 名称、描述、自主度 badge、
 * cron 表达式和预算信息。
 *
 * 与场景模板 [`TemplateCard`] 不同,Loop 模板暂不支持直接"使用"启动
 * (需先经过 `loop_run` 命令,涉及 LoopEngine + 预算审批流程),
 * 当前只做展示 + 预览。后续 T-E-L-06(预算管理)完成后可加"创建 Loop"按钮。
 */
function LoopTemplateCard({ template }: LoopTemplateCardProps) {
  const badge = AUTONOMY_BADGE[template.autonomy] ?? {
    color: 'var(--text-muted)',
    bg: 'transparent',
  };

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        padding: '10px',
        border: '1px solid var(--border)',
        borderRadius: '6px',
        background: 'transparent',
        transition: 'all 0.15s',
        minHeight: '140px',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.borderColor = 'var(--accent-neon)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.borderColor = 'var(--border)';
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: '6px',
        }}
      >
        <strong style={{ fontSize: '13px', color: 'var(--text-primary)' }}>
          {template.name}
        </strong>
        <span
          style={{
            fontSize: '10px',
            padding: '1px 6px',
            borderRadius: '3px',
            color: badge.color,
            background: badge.bg,
            border: `1px solid ${badge.color}`,
          }}
        >
          {template.autonomy}
        </span>
      </div>
      <div
        style={{
          fontSize: '11px',
          color: 'var(--text-secondary)',
          flex: 1,
          marginBottom: '8px',
          overflow: 'hidden',
          display: '-webkit-box',
          WebkitLineClamp: 3,
          WebkitBoxOrient: 'vertical',
        }}
      >
        {template.description}
      </div>
      <div
        style={{
          display: 'flex',
          flexWrap: 'wrap',
          gap: '4px',
          marginBottom: '8px',
        }}
      >
        <span
          style={{
            fontSize: '10px',
            padding: '1px 5px',
            borderRadius: '3px',
            background: 'var(--bg-secondary, rgba(255,255,255,0.05))',
            color: 'var(--text-muted)',
            border: '1px solid var(--border)',
          }}
        >
          ⏰ {template.cadence}
        </span>
        {template.budget_tokens > 0 && (
          <span
            style={{
              fontSize: '10px',
              padding: '1px 5px',
              borderRadius: '3px',
              background: 'var(--bg-secondary, rgba(255,255,255,0.05))',
              color: 'var(--text-muted)',
              border: '1px solid var(--border)',
            }}
          >
            {(template.budget_tokens / 1000).toFixed(0)}k tok
          </span>
        )}
        {template.budget_minutes > 0 && (
          <span
            style={{
              fontSize: '10px',
              padding: '1px 5px',
              borderRadius: '3px',
              background: 'var(--bg-secondary, rgba(255,255,255,0.05))',
              color: 'var(--text-muted)',
              border: '1px solid var(--border)',
            }}
          >
            {template.budget_minutes}min
          </span>
        )}
      </div>
      <div
        style={{
          fontSize: '10px',
          color: 'var(--text-muted)',
          fontStyle: 'italic',
        }}
      >
        {t('templatesDialog.automationPreview')}
      </div>
    </div>
  );
}
