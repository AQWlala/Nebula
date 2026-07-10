/**
 * P0-4: 首次使用向导 — 4 步配置流程。
 *
 * 对标 OpenAkita 的"5分钟上手"向导式体验,在 App 首次启动时引导用户:
 *  1. 欢迎(Nebula logo + slogan + 三大核心特性卡片)
 *  2. 配置模型(选 provider / 填 base_url / 填 API Key / 测试连接 / 保存)
 *  3. 选择技能(勾选推荐内置技能,记录偏好)
 *  4. 完成(配置总结 + 进入 Nebula)
 *
 * 完成状态用 localStorage key `nebula-onboarding-completed` 标记,
 * "稍后" / "跳过" 按钮也会标记完成并关闭向导。
 *
 * 后端命令复用 nebulaAPI:
 *  - testProviderConnection / setProviderKey / setDefaultProvider
 *  - modelsConfigLoad(用于查询所选 provider 的首个模型 id)
 */
import { useState, useEffect } from 'preact/hooks';
import {
  nebulaAPI,
  type ConnectionTestResult,
  type ModelsConfig,
} from '../lib/tauri';

/** localStorage key:是否已完成首次向导。 */
export const ONBOARDING_STORAGE_KEY = 'nebula-onboarding-completed';

/** Provider 选项元数据。id 对齐后端 models.json 内置 provider 标识。 */
interface ProviderOption {
  /** 后端 provider id(ollama / deepseek / anthropic / openai-compat)。 */
  id: string;
  /** 展示名。 */
  label: string;
  /** 简短描述。 */
  desc: string;
  /** 默认 base_url。 */
  defaultBaseUrl: string;
  /** 是否需要 API Key(Ollama 本地无需)。 */
  needsKey: boolean;
}

/** 向导第 2 步可选 provider 列表(默认选中 DeepSeek)。 */
const PROVIDER_OPTIONS: ProviderOption[] = [
  {
    id: 'ollama',
    label: '本地 Ollama',
    desc: '免费,需已安装',
    defaultBaseUrl: 'http://localhost:11434',
    needsKey: false,
  },
  {
    id: 'deepseek',
    label: 'DeepSeek',
    desc: '云端,性价比高',
    defaultBaseUrl: 'https://api.deepseek.com/v1',
    needsKey: true,
  },
  {
    id: 'anthropic',
    label: 'Anthropic Claude',
    desc: '云端,高质量',
    defaultBaseUrl: 'https://api.anthropic.com',
    needsKey: true,
  },
  {
    id: 'openai-compat',
    label: '自定义 API',
    desc: 'OpenAI 兼容端点',
    defaultBaseUrl: '',
    needsKey: true,
  },
];

/** 推荐内置技能(名称对齐 docs/skills/ 目录)。 */
interface SkillOption {
  id: string;
  name: string;
  desc: string;
}

/** 第 3 步展示的 10 个推荐技能。 */
const SKILL_OPTIONS: SkillOption[] = [
  { id: 'file-reader', name: '文件读取', desc: '读取并摘要 .txt/.md/.pdf/.docx 等文档' },
  { id: 'doc-writer', name: '文档撰写', desc: '按主题生成结构化文档并落盘' },
  { id: 'web-search', name: '网页搜索', desc: '联网检索并整合实时信息' },
  { id: 'code-review', name: '代码审查', desc: '从可读性/正确性/安全性等维度评审代码' },
  { id: 'article-writer', name: '文章撰写', desc: '撰写自媒体/博客/技术博客文章' },
  { id: 'code-refactor', name: '代码重构', desc: '给出重构建议并安全改写代码' },
  { id: 'git-helper', name: 'Git 助手', desc: '生成 commit 信息、解读 diff、辅助提交流程' },
  { id: 'meeting-notes', name: '会议纪要', desc: '从转录文本提炼会议纪要与待办' },
  { id: 'pdf-extractor', name: 'PDF 提取', desc: '从 PDF 中抽取文本与结构化要点' },
  { id: 'news-digest', name: '新闻摘要', desc: '聚合多源资讯生成简报' },
];

/** 默认勾选的 5 个最常用技能。 */
const DEFAULT_SKILL_IDS = [
  'file-reader',
  'doc-writer',
  'web-search',
  'code-review',
  'article-writer',
];

/** 三大核心特性卡片内容。 */
const FEATURE_CARDS = [
  {
    icon: '📖',
    title: '可读记忆',
    desc: '8 层记忆系统全部可阅读、可编辑,你看得见 Nebula 记住了什么。',
  },
  {
    icon: '💰',
    title: '成本控制',
    desc: '日/月预算硬限制,超限自动降级到本地 Ollama,账单不再失控。',
  },
  {
    icon: '🤖',
    title: '自主操作',
    desc: 'Swarm 多智能体 + Master 编排,可自主完成长任务并留痕可回滚。',
  },
];

export interface OnboardingWizardProps {
  /** 向导完成(包括"稍后"/"跳过")时回调,App 据此隐藏向导。 */
  onDone: () => void;
}

export function OnboardingWizard({ onDone }: OnboardingWizardProps) {
  const [step, setStep] = useState(1);
  const TOTAL_STEPS = 4;

  // ---- 第 2 步:模型配置 ----
  const [providerId, setProviderId] = useState('deepseek');
  const [baseUrl, setBaseUrl] = useState('https://api.deepseek.com/v1');
  const [apiKey, setApiKey] = useState('');
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);
  // 测试成功后才允许进入下一步。
  const [testPassed, setTestPassed] = useState(false);
  // 后端 ModelsConfig(用于查询所选 provider 的首个模型 id,设默认时用)。
  const [modelsConfig, setModelsConfig] = useState<ModelsConfig | null>(null);

  // ---- 第 3 步:技能选择 ----
  const [selectedSkills, setSelectedSkills] = useState<Set<string>>(
    () => new Set(DEFAULT_SKILL_IDS)
  );

  // 切换 provider 时重置 base_url / 测试状态(API Key 保留,方便用户切换试错)。
  function selectProvider(id: string) {
    const opt = PROVIDER_OPTIONS.find((p) => p.id === id);
    setProviderId(id);
    setBaseUrl(opt?.defaultBaseUrl ?? '');
    setTestResult(null);
    setTestPassed(false);
  }

  // 挂载时加载 ModelsConfig(用于第 2 步保存默认 provider 时取首个模型 id)。
  useEffect(() => {
    const ac = new AbortController();
    (async () => {
      try {
        const cfg = await nebulaAPI.modelsConfigLoad();
        if (!ac.signal.aborted) setModelsConfig(cfg);
      } catch {
        // Tauri 运行时不可用时静默;保存默认 provider 会优雅降级。
      }
    })();
    return () => {
      ac.abort();
    };
  }, []);

  // 测试 provider 连通性,成功后解锁"下一步"。
  async function testConnection() {
    setTesting(true);
    setTestResult(null);
    try {
      const result = await nebulaAPI.testProviderConnection(
        providerId,
        baseUrl,
        apiKey.trim().length > 0 ? apiKey.trim() : null
      );
      setTestResult(result);
      setTestPassed(result.success === true);
    } catch (e) {
      setTestResult({
        success: false,
        latency_ms: 0,
        error: String(e),
      });
      setTestPassed(false);
    } finally {
      setTesting(false);
    }
  }

  // 保存 API Key + 设置默认 provider(各调用独立 try/catch,失败不阻塞向导)。
  async function persistProviderConfig() {
    const opt = PROVIDER_OPTIONS.find((p) => p.id === providerId);
    // 1. 保存 API Key(Ollama 无需保存,跳过)。
    if (opt?.needsKey && apiKey.trim().length > 0) {
      try {
        await nebulaAPI.setProviderKey(providerId, apiKey.trim());
      } catch (e) {
        console.error('onboarding: setProviderKey failed', e);
      }
    }
    // 2. 设置默认 provider(需要 modelId,取该 provider 的首个模型)。
    try {
      const cfg = modelsConfig ?? (await nebulaAPI.modelsConfigLoad());
      const provider = cfg.providers.find((p) => p.id === providerId);
      const firstModel = provider?.models?.[0]?.id;
      if (provider && firstModel) {
        await nebulaAPI.setDefaultProvider(providerId, firstModel);
      }
    } catch (e) {
      console.error('onboarding: setDefaultProvider failed', e);
    }
  }

  // 第 2 步"下一步":先持久化配置再前进。
  async function goNextFromStep2() {
    await persistProviderConfig();
    setStep(3);
  }

  // 第 3 步技能勾选切换。
  function toggleSkill(id: string) {
    setSelectedSkills((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function selectAllSkills() {
    setSelectedSkills(new Set(SKILL_OPTIONS.map((s) => s.id)));
  }

  function clearAllSkills() {
    setSelectedSkills(new Set());
  }

  // 完成向导(包括"稍后"/"跳过"/"进入 Nebula")。
  function finish() {
    onDone();
  }

  // 进度指示器:●●○○
  function renderProgress() {
    return (
      <div class="onboarding-progress" aria-label={`步骤 ${step} / ${TOTAL_STEPS}`}>
        {Array.from({ length: TOTAL_STEPS }, (_, i) => {
          const n = i + 1;
          const cls = n === step ? 'active' : n < step ? 'done' : '';
          return (
            <span key={n} class={`step-dot ${cls}`} aria-label={`Step ${n}`} />
          );
        })}
        <span class="step-counter">Step {step}/{TOTAL_STEPS}</span>
      </div>
    );
  }

  return (
    <div class="onboarding-overlay" role="dialog" aria-modal="true" aria-labelledby="onboarding-wizard-title">
      <div class="onboarding-card">
        {renderProgress()}

        {/* ---- Step 1/4: 欢迎 ---- */}
        {step === 1 && (
          <div class="onboarding-step">
            <div class="onboarding-welcome">
              <div class="onboarding-logo">🐍</div>
              <h2 id="onboarding-wizard-title">Nebula</h2>
              <p class="onboarding-slogan">「你无法信任一段你无法阅读的记忆」</p>
              <p class="onboarding-intro">
                Nebula 是本地优先的自主式知识型桌面 AI 伙伴。
                你的数据留在本机,记忆可读可改,成本可控,操作可回滚。
              </p>
            </div>
            <div class="onboarding-features">
              {FEATURE_CARDS.map((f) => (
                <div key={f.title} class="onboarding-feature-card">
                  <div class="feature-icon">{f.icon}</div>
                  <div class="feature-text">
                    <div class="feature-title">{f.title}</div>
                    <div class="feature-desc">{f.desc}</div>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* ---- Step 2/4: 配置模型 ---- */}
        {step === 2 && (
          <div class="onboarding-step">
            <h3 class="onboarding-step-title">配置你的模型</h3>
            <p class="onboarding-step-hint">选择首选 LLM 提供商,测试连接通过后即可进入下一步。</p>

            <div class="onboarding-provider-list">
              {PROVIDER_OPTIONS.map((p) => (
                <label
                  key={p.id}
                  class={`onboarding-provider-option ${providerId === p.id ? 'selected' : ''}`}
                >
                  <input
                    type="radio"
                    name="onboarding-provider"
                    value={p.id}
                    checked={providerId === p.id}
                    onChange={() => selectProvider(p.id)}
                  />
                  <span class="provider-radio-mark" />
                  <span class="provider-info">
                    <span class="provider-name">{p.label}</span>
                    <span class="provider-desc">{p.desc}</span>
                  </span>
                </label>
              ))}
            </div>

            <label class="onboarding-field">
              <span class="field-label">API 地址</span>
              <input
                type="url"
                value={baseUrl}
                onInput={(e) => {
                  setBaseUrl(e.currentTarget.value);
                  setTestPassed(false);
                }}
                placeholder="https://api.example.com/v1"
              />
            </label>

            {PROVIDER_OPTIONS.find((p) => p.id === providerId)?.needsKey && (
              <label class="onboarding-field">
                <span class="field-label">API Key</span>
                <input
                  type="password"
                  value={apiKey}
                  onInput={(e) => {
                    setApiKey(e.currentTarget.value);
                    setTestPassed(false);
                  }}
                  autocomplete="off"
                  spellcheck={false}
                  placeholder="sk-..."
                />
              </label>
            )}

            <div class="onboarding-test-row">
              <button
                type="button"
                class="btn"
                onClick={testConnection}
                disabled={testing}
              >
                {testing ? '测试中…' : '测试连接'}
              </button>
              {testResult && (
                <span
                  class={`onboarding-test-result ${testResult.success ? 'ok' : 'fail'}`}
                >
                  {testResult.success
                    ? `🟢 连通成功,延迟 ${testResult.latency_ms}ms`
                    : `🔴 连通失败${testResult.error ? ': ' + testResult.error : ''}`}
                </span>
              )}
            </div>
          </div>
        )}

        {/* ---- Step 3/4: 选择技能 ---- */}
        {step === 3 && (
          <div class="onboarding-step">
            <h3 class="onboarding-step-title">选择你感兴趣的技能</h3>
            <p class="onboarding-step-hint">
              勾选推荐技能,记录你的偏好(无需等待安装,随时可在设置中调整)。
            </p>

            <div class="onboarding-skill-actions">
              <button type="button" class="btn-link" onClick={selectAllSkills}>
                全选
              </button>
              <button type="button" class="btn-link" onClick={clearAllSkills}>
                全不选
              </button>
              <span class="skill-count">已选 {selectedSkills.size} / {SKILL_OPTIONS.length}</span>
            </div>

            <div class="onboarding-skill-list">
              {SKILL_OPTIONS.map((s) => (
                <label
                  key={s.id}
                  class={`onboarding-skill-item ${selectedSkills.has(s.id) ? 'checked' : ''}`}
                >
                  <input
                    type="checkbox"
                    checked={selectedSkills.has(s.id)}
                    onChange={() => toggleSkill(s.id)}
                  />
                  <span class="skill-info">
                    <span class="skill-name">{s.name}</span>
                    <span class="skill-desc">{s.desc}</span>
                  </span>
                </label>
              ))}
            </div>
          </div>
        )}

        {/* ---- Step 4/4: 完成 ---- */}
        {step === 4 && (
          <div class="onboarding-step">
            <h3 class="onboarding-step-title">配置完成 🎉</h3>
            <div class="onboarding-summary">
              <div class="summary-row">
                <span class="summary-label">已配置模型</span>
                <span class="summary-value">
                  {PROVIDER_OPTIONS.find((p) => p.id === providerId)?.label ?? providerId}
                </span>
              </div>
              <div class="summary-row">
                <span class="summary-label">已选技能</span>
                <span class="summary-value">{selectedSkills.size} 个</span>
              </div>
            </div>
            <p class="onboarding-tip">你可以随时在设置中修改配置。</p>
          </div>
        )}

        {/* ---- 底部按钮区 ---- */}
        <div class="onboarding-buttons">
          {step === 1 ? (
            <button type="button" class="btn-ghost" onClick={finish}>
              稍后
            </button>
          ) : (
            <button
              type="button"
              class="btn-ghost"
              onClick={() => setStep(step - 1)}
              disabled={step === 1}
            >
              上一步
            </button>
          )}

          <div class="onboarding-buttons-right">
            <button type="button" class="btn-ghost" onClick={finish}>
              跳过
            </button>
            {step < TOTAL_STEPS ? (
              <button
                type="button"
                class="btn-primary"
                onClick={step === 2 ? goNextFromStep2 : () => setStep(step + 1)}
                disabled={step === 2 && !testPassed}
                title={step === 2 && !testPassed ? '请先测试连接并通过' : ''}
              >
                下一步
              </button>
            ) : (
              <button type="button" class="btn-primary" onClick={finish}>
                进入 Nebula
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
