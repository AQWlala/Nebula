/**
 * P1-7: 技能调试器 — Inspector / TestRunner / Debugger / Profiler。
 *
 * 四个 Tab 为技能开发者提供全链路调试能力:
 * * Inspector  — 检查 manifest + body + 三层校验 + 依赖检查 + 使用统计。
 * * TestRunner — 单次沙箱测试运行,查看输出 / 日志 / 延迟。
 * * Debugger  — 启动调试会话,逐步执行(load / validate / execute),
 *               查看变量状态 + 调用栈。
 * * Profiler  — 性能分析,CPU / 内存 / IO / 子调用 + 时间线。
 *
 * 后端命令: skill_inspect / skill_test_run / skill_debug_start /
 *           skill_debug_step / skill_debug_stop / skill_profile。
 */

import { useState, useCallback, useEffect } from 'preact/hooks';
import {
  nebulaAPI,
  SkillInspection,
  SkillTestResult,
  DebugStepResult,
  SkillProfile,
} from '../lib/tauri';
import { Spinner } from './Spinner';
import { t } from '../i18n';

interface SkillDebuggerProps {
  /** 目标技能 ID。 */
  skillId: string;
  /** 目标技能名称(显示在标题)。 */
  skillName: string;
  /** 关闭弹窗回调。 */
  onClose: () => void;
}

type Tab = 'inspector' | 'testRunner' | 'debugger' | 'profiler';

export default function SkillDebugger({
  skillId,
  skillName,
  onClose,
}: SkillDebuggerProps) {
  const [tab, setTab] = useState<Tab>('inspector');

  // 组件卸载时自动停止调试会话(防止泄漏)。
  const [sessionId, setSessionId] = useState<string | null>(null);
  useEffect(() => {
    return () => {
      if (sessionId) {
        nebulaAPI.skillDebugStop(sessionId).catch(() => {});
      }
    };
  }, [sessionId]);

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div class="w-[90vw] h-[85vh] max-w-5xl bg-[#1E293B] rounded-lg border border-gray-700 flex flex-col overflow-hidden">
        {/* Header */}
        <div class="flex items-center justify-between px-5 py-3 border-b border-gray-700">
          <div class="flex items-center gap-2">
            <h2 class="text-lg font-semibold text-white">
              {t('skillDebugger.title')}
            </h2>
            <span class="text-sm text-gray-400">— {skillName}</span>
            <span class="text-xs text-gray-500">P1-7</span>
          </div>
          <button
            onClick={onClose}
            class="text-gray-400 hover:text-white text-xl leading-none px-2"
            title={t('skillDebugger.close')}
          >
            ×
          </button>
        </div>

        {/* Tabs */}
        <div class="flex gap-1 px-5 py-2 border-b border-gray-700 bg-gray-800/50">
          <DebuggerTabButton
            label={t('skillDebugger.tab.inspector')}
            active={tab === 'inspector'}
            onClick={() => setTab('inspector')}
          />
          <DebuggerTabButton
            label={t('skillDebugger.tab.testRunner')}
            active={tab === 'testRunner'}
            onClick={() => setTab('testRunner')}
          />
          <DebuggerTabButton
            label={t('skillDebugger.tab.debugger')}
            active={tab === 'debugger'}
            onClick={() => setTab('debugger')}
          />
          <DebuggerTabButton
            label={t('skillDebugger.tab.profiler')}
            active={tab === 'profiler'}
            onClick={() => setTab('profiler')}
          />
        </div>

        {/* Tab content */}
        <div class="flex-1 overflow-y-auto p-5">
          {tab === 'inspector' && (
            <InspectorTab skillId={skillId} />
          )}
          {tab === 'testRunner' && (
            <TestRunnerTab skillId={skillId} />
          )}
          {tab === 'debugger' && (
            <DebuggerTab
              skillId={skillId}
              sessionId={sessionId}
              setSessionId={setSessionId}
            />
          )}
          {tab === 'profiler' && (
            <ProfilerTab skillId={skillId} />
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// TabButton
// ---------------------------------------------------------------------------

function DebuggerTabButton({
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
// InspectorTab — 检查 manifest + body + 校验 + 依赖 + 使用统计
// ---------------------------------------------------------------------------

function InspectorTab({ skillId }: { skillId: string }) {
  const [inspection, setInspection] = useState<SkillInspection | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleInspect = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await nebulaAPI.skillInspect(skillId);
      setInspection(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [skillId]);

  // 自动触发首次检查。
  useEffect(() => {
    handleInspect();
  }, [handleInspect]);

  if (loading && !inspection) {
    return <Spinner size={32} />;
  }

  if (error) {
    return (
      <div class="text-red-400 text-sm">
        {t('skillDebugger.error', { error })}
      </div>
    );
  }

  if (!inspection) {
    return null;
  }

  const pct = (v: number) => `${Math.round(v * 100)}%`;
  const ts = (v: number | null) =>
    v ? new Date(v).toLocaleString('zh-CN') : t('skillDebugger.inspection.notUsed');

  return (
    <div class="space-y-4">
      {/* Manifest */}
      <section class="bg-gray-800/50 rounded-md p-4">
        <h3 class="text-sm font-semibold text-gray-300 mb-2">
          {t('skillDebugger.inspection.manifest')}
        </h3>
        <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
          <dt class="text-gray-500">name</dt>
          <dd class="text-gray-300">{inspection.manifest.name}</dd>
          <dt class="text-gray-500">version</dt>
          <dd class="text-gray-300">{inspection.manifest.version}</dd>
          <dt class="text-gray-500">transport</dt>
          <dd class="text-gray-300">{inspection.manifest.transport}</dd>
          <dt class="text-gray-500">status</dt>
          <dd class="text-gray-300">{inspection.manifest.status ?? '—'}</dd>
          <dt class="text-gray-500">capabilities</dt>
          <dd class="text-gray-300">
            {inspection.manifest.capabilities.length > 0
              ? inspection.manifest.capabilities.join(', ')
              : '—'}
          </dd>
        </dl>
      </section>

      {/* Validation */}
      <section class="bg-gray-800/50 rounded-md p-4">
        <h3 class="text-sm font-semibold text-gray-300 mb-2">
          {t('skillDebugger.inspection.validation')}
        </h3>
        <div class="flex gap-4 mb-2 text-sm">
          <span class={inspection.validation.structure_ok ? 'text-green-400' : 'text-red-400'}>
            {inspection.validation.structure_ok ? '✓' : '✗'} {t('skillDebugger.inspection.structure')}
          </span>
          <span class={inspection.validation.spec_ok ? 'text-green-400' : 'text-red-400'}>
            {inspection.validation.spec_ok ? '✓' : '✗'} {t('skillDebugger.inspection.spec')}
          </span>
          <span class={inspection.validation.eligibility_ok ? 'text-green-400' : 'text-red-400'}>
            {inspection.validation.eligibility_ok ? '✓' : '✗'} {t('skillDebugger.inspection.eligibility')}
          </span>
        </div>
        {inspection.validation.errors.length > 0 && (
          <div class="mb-2">
            <p class="text-xs text-red-400 font-semibold mb-1">
              {t('skillDebugger.inspection.errors')}
            </p>
            <ul class="text-xs text-red-300 list-disc list-inside space-y-0.5">
              {inspection.validation.errors.map((e, i) => (
                <li key={i}>{e}</li>
              ))}
            </ul>
          </div>
        )}
        {inspection.validation.warnings.length > 0 && (
          <div>
            <p class="text-xs text-yellow-400 font-semibold mb-1">
              {t('skillDebugger.inspection.warnings')}
            </p>
            <ul class="text-xs text-yellow-300 list-disc list-inside space-y-0.5">
              {inspection.validation.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          </div>
        )}
      </section>

      {/* Dependencies */}
      <section class="bg-gray-800/50 rounded-md p-4">
        <h3 class="text-sm font-semibold text-gray-300 mb-2">
          {t('skillDebugger.inspection.dependencies')}
        </h3>
        <div class="text-sm space-y-1">
          <div>
            <span class="text-gray-500">{t('skillDebugger.inspection.bins')}:</span>
            {Object.entries(inspection.dependency_check.bins_available).length === 0 ? (
              <span class="ml-2 text-gray-400">—</span>
            ) : (
              <span class="ml-2">
                {Object.entries(inspection.dependency_check.bins_available).map(([bin, ok]) => (
                  <span
                    key={bin}
                    class={`ml-1 px-1.5 py-0.5 text-xs rounded ${ok ? 'bg-green-900/40 text-green-300' : 'bg-red-900/40 text-red-300'}`}
                  >
                    {bin} {ok ? '✓' : '✗'}
                  </span>
                ))}
              </span>
            )}
          </div>
          <div>
            <span class="text-gray-500">{t('skillDebugger.inspection.osSupported')}:</span>
            <span class={`ml-2 ${inspection.dependency_check.os_supported ? 'text-green-400' : 'text-red-400'}`}>
              {inspection.dependency_check.os_supported ? '✓' : '✗'}
            </span>
          </div>
        </div>
      </section>

      {/* Usage Stats */}
      <section class="bg-gray-800/50 rounded-md p-4">
        <h3 class="text-sm font-semibold text-gray-300 mb-2">
          {t('skillDebugger.inspection.usageStats')}
        </h3>
        <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
          <div>
            <span class="text-gray-500">{t('skillDebugger.inspection.callCount')}:</span>
            <span class="ml-2 text-gray-300">{inspection.usage_stats.call_count}</span>
          </div>
          <div>
            <span class="text-gray-500">{t('skillDebugger.inspection.successRate')}:</span>
            <span class="ml-2 text-gray-300">{pct(inspection.usage_stats.success_rate)}</span>
          </div>
          <div>
            <span class="text-gray-500">{t('skillDebugger.inspection.avgLatency')}:</span>
            <span class="ml-2 text-gray-300">{inspection.usage_stats.avg_latency_ms}ms</span>
          </div>
          <div>
            <span class="text-gray-500">{t('skillDebugger.inspection.lastUsed')}:</span>
            <span class="ml-2 text-gray-300">{ts(inspection.usage_stats.last_used)}</span>
          </div>
        </div>
      </section>

      {/* Body */}
      <section class="bg-gray-800/50 rounded-md p-4">
        <h3 class="text-sm font-semibold text-gray-300 mb-2">
          {t('skillDebugger.inspection.body')}
        </h3>
        <pre class="p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-x-auto max-h-64">
          <code>{inspection.body}</code>
        </pre>
      </section>

      {/* Refresh */}
      <button
        onClick={handleInspect}
        disabled={loading}
        class="px-3 py-1.5 text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700
               text-white rounded-md transition-colors"
      >
        {loading ? <Spinner size={16} showLabel={false} /> : t('skillDebugger.inspect')}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// TestRunnerTab — 单次沙箱测试运行
// ---------------------------------------------------------------------------

function TestRunnerTab({ skillId }: { skillId: string }) {
  const [testInput, setTestInput] = useState('');
  const [result, setResult] = useState<SkillTestResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleRun = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await nebulaAPI.skillTestRun(skillId, testInput);
      setResult(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [skillId, testInput]);

  return (
    <div class="space-y-4">
      {/* Test input */}
      <div>
        <label class="block text-sm text-gray-400 mb-1">
          {t('skillDebugger.testInput')}
        </label>
        <textarea
          class="w-full h-24 p-3 bg-gray-900 border border-gray-700 rounded-md text-sm
                 text-gray-200 font-mono resize-y focus:border-blue-500 focus:outline-none"
          placeholder={t('skillDebugger.testInputPlaceholder')}
          value={testInput}
          onInput={(e) => setTestInput((e.target as HTMLTextAreaElement).value)}
        />
      </div>

      {/* Run button */}
      <button
        onClick={handleRun}
        disabled={loading}
        class="px-4 py-2 text-sm bg-green-600 hover:bg-green-700 disabled:bg-gray-700
               text-white rounded-md transition-colors"
      >
        {loading ? <Spinner size={16} showLabel={false} /> : `▶ ${t('skillDebugger.run')}`}
      </button>

      {/* Error */}
      {error && (
        <div class="text-red-400 text-sm">
          {t('skillDebugger.error', { error })}
        </div>
      )}

      {/* Result */}
      {result && (
        <div class="space-y-3">
          {/* Status + latency */}
          <div class="flex items-center gap-4 text-sm">
            <span class={result.success ? 'text-green-400' : 'text-red-400'}>
              {result.success ? '✓ Success' : '✗ Failed'}
            </span>
            <span class="text-gray-400">
              {t('skillDebugger.testResult.latency', { ms: result.latency_ms })}
            </span>
          </div>

          {/* Error message */}
          {result.error && (
            <div class="p-3 bg-red-900/30 border border-red-700 rounded-md text-sm text-red-300">
              {result.error}
            </div>
          )}

          {/* Output */}
          {result.output && (
            <div>
              <h4 class="text-sm font-semibold text-gray-300 mb-1">
                {t('skillDebugger.testResult.output')}
              </h4>
              <pre class="p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-x-auto max-h-48">
                <code>{result.output}</code>
              </pre>
            </div>
          )}

          {/* Logs */}
          {result.logs.length > 0 && (
            <div>
              <h4 class="text-sm font-semibold text-gray-300 mb-1">
                {t('skillDebugger.testResult.logs')}
              </h4>
              <pre class="p-3 bg-gray-900 rounded-md text-xs text-gray-500 overflow-x-auto max-h-48">
                <code>{result.logs.join('\n')}</code>
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// DebuggerTab — 逐步调试会话
// ---------------------------------------------------------------------------

function DebuggerTab({
  skillId,
  sessionId,
  setSessionId,
}: {
  skillId: string;
  sessionId: string | null;
  setSessionId: (id: string | null) => void;
}) {
  const [testInput, setTestInput] = useState('');
  const [steps, setSteps] = useState<DebugStepResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleStart = useCallback(async () => {
    setLoading(true);
    setError(null);
    setSteps([]);
    try {
      const id = await nebulaAPI.skillDebugStart(skillId, testInput);
      setSessionId(id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [skillId, testInput, setSessionId]);

  const handleStop = useCallback(async () => {
    if (!sessionId) return;
    setLoading(true);
    setError(null);
    try {
      await nebulaAPI.skillDebugStop(sessionId);
      setSessionId(null);
      setSteps([]);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, setSessionId]);

  const handleStep = useCallback(
    async (step: string) => {
      if (!sessionId) return;
      setLoading(true);
      setError(null);
      try {
        const r = await nebulaAPI.skillDebugStep(sessionId, step);
        setSteps((prev) => [...prev, r]);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setLoading(false);
      }
    },
    [sessionId]
  );

  const lastStep = steps.length > 0 ? steps[steps.length - 1] : null;

  return (
    <div class="space-y-4">
      {/* Test input + session controls */}
      {!sessionId ? (
        <>
          <div>
            <label class="block text-sm text-gray-400 mb-1">
              {t('skillDebugger.testInput')}
            </label>
            <textarea
              class="w-full h-20 p-3 bg-gray-900 border border-gray-700 rounded-md text-sm
                     text-gray-200 font-mono resize-y focus:border-blue-500 focus:outline-none"
              placeholder={t('skillDebugger.testInputPlaceholder')}
              value={testInput}
              onInput={(e) => setTestInput((e.target as HTMLTextAreaElement).value)}
            />
          </div>
          <button
            onClick={handleStart}
            disabled={loading}
            class="px-4 py-2 text-sm bg-green-600 hover:bg-green-700 disabled:bg-gray-700
                   text-white rounded-md transition-colors"
          >
            {loading ? <Spinner size={16} showLabel={false} /> : `▶ ${t('skillDebugger.debugger.start')}`}
          </button>
        </>
      ) : (
        <div class="flex items-center gap-2">
          <span class="text-xs text-gray-500 font-mono">
            session: {sessionId.slice(0, 8)}…
          </span>
          <button
            onClick={handleStop}
            disabled={loading}
            class="px-3 py-1.5 text-sm bg-red-600 hover:bg-red-700 disabled:bg-gray-700
                   text-white rounded-md transition-colors"
          >
            {t('skillDebugger.debugger.stop')}
          </button>
        </div>
      )}

      {/* Error */}
      {error && (
        <div class="text-red-400 text-sm">
          {t('skillDebugger.error', { error })}
        </div>
      )}

      {/* Step buttons */}
      {sessionId && (
        <div class="flex items-center gap-2">
          <span class="text-sm text-gray-400">{t('skillDebugger.debugger.step')}:</span>
          <button
            onClick={() => handleStep('load')}
            disabled={loading}
            class="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700
                   text-white rounded transition-colors"
          >
            {t('skillDebugger.debugger.stepLoad')}
          </button>
          <button
            onClick={() => handleStep('validate')}
            disabled={loading}
            class="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700
                   text-white rounded transition-colors"
          >
            {t('skillDebugger.debugger.stepValidate')}
          </button>
          <button
            onClick={() => handleStep('execute')}
            disabled={loading}
            class="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-700 disabled:bg-gray-700
                   text-white rounded transition-colors"
          >
            {t('skillDebugger.debugger.stepExecute')}
          </button>
          {loading && <Spinner size={16} showLabel={false} />}
        </div>
      )}

      {/* No session hint */}
      {!sessionId && steps.length === 0 && !error && (
        <p class="text-sm text-gray-500">{t('skillDebugger.debugger.noSession')}</p>
      )}

      {/* Steps history */}
      {steps.length > 0 && (
        <div>
          <h4 class="text-sm font-semibold text-gray-300 mb-1">
            {t('skillDebugger.debugger.steps')} ({steps.length})
          </h4>
          <div class="space-y-1 max-h-32 overflow-y-auto">
            {steps.map((s, i) => (
              <div key={i} class="text-xs flex items-center gap-2">
                <span class={s.success ? 'text-green-400' : 'text-red-400'}>
                  {s.success ? '✓' : '✗'}
                </span>
                <span class="text-gray-300 font-mono">{s.step}</span>
                <span class="text-gray-500 truncate">{s.output}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Variables + Call stack (from last step) */}
      {lastStep && (
        <div class="grid grid-cols-2 gap-4">
          {/* Variables */}
          <div>
            <h4 class="text-sm font-semibold text-gray-300 mb-1">
              {t('skillDebugger.debugger.variables')}
            </h4>
            <div class="bg-gray-900 rounded-md p-3 max-h-48 overflow-y-auto">
              {Object.entries(lastStep.variables).length === 0 ? (
                <span class="text-xs text-gray-500">—</span>
              ) : (
                <dl class="text-xs space-y-0.5">
                  {Object.entries(lastStep.variables).map(([k, v]) => (
                    <div key={k} class="flex gap-2">
                      <dt class="text-gray-500 font-mono shrink-0">{k}:</dt>
                      <dd class="text-gray-300 font-mono break-all">{v}</dd>
                    </div>
                  ))}
                </dl>
              )}
            </div>
          </div>

          {/* Call stack */}
          <div>
            <h4 class="text-sm font-semibold text-gray-300 mb-1">
              {t('skillDebugger.debugger.callStack')}
            </h4>
            <div class="bg-gray-900 rounded-md p-3 max-h-48 overflow-y-auto">
              {lastStep.call_stack.length === 0 ? (
                <span class="text-xs text-gray-500">—</span>
              ) : (
                <ul class="text-xs space-y-0.5">
                  {lastStep.call_stack.map((frame, i) => (
                    <li key={i} class="text-gray-300 font-mono">
                      {i === 0 ? '▶ ' : '  '}
                      {'— '.repeat(i)}{frame}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProfilerTab — 性能分析
// ---------------------------------------------------------------------------

function ProfilerTab({ skillId }: { skillId: string }) {
  const [testInput, setTestInput] = useState('');
  const [profile, setProfile] = useState<SkillProfile | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleProfile = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await nebulaAPI.skillProfile(skillId, testInput);
      setProfile(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [skillId, testInput]);

  const formatBytes = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  return (
    <div class="space-y-4">
      {/* Test input */}
      <div>
        <label class="block text-sm text-gray-400 mb-1">
          {t('skillDebugger.testInput')}
        </label>
        <textarea
          class="w-full h-20 p-3 bg-gray-900 border border-gray-700 rounded-md text-sm
                 text-gray-200 font-mono resize-y focus:border-blue-500 focus:outline-none"
          placeholder={t('skillDebugger.testInputPlaceholder')}
          value={testInput}
          onInput={(e) => setTestInput((e.target as HTMLTextAreaElement).value)}
        />
      </div>

      {/* Profile button */}
      <button
        onClick={handleProfile}
        disabled={loading}
        class="px-4 py-2 text-sm bg-purple-600 hover:bg-purple-700 disabled:bg-gray-700
               text-white rounded-md transition-colors"
      >
        {loading ? <Spinner size={16} showLabel={false} /> : `📊 ${t('skillDebugger.profile')}`}
      </button>

      {/* Error */}
      {error && (
        <div class="text-red-400 text-sm">
          {t('skillDebugger.error', { error })}
        </div>
      )}

      {/* Profile result */}
      {profile && (
        <div class="space-y-3">
          {/* Metrics grid */}
          <div class="grid grid-cols-2 gap-3">
            <div class="bg-gray-800/50 rounded-md p-3">
              <p class="text-xs text-gray-500">{t('skillDebugger.profiler.cpuTime')}</p>
              <p class="text-lg text-gray-200 font-mono">{profile.cpu_time_ms} ms</p>
            </div>
            <div class="bg-gray-800/50 rounded-md p-3">
              <p class="text-xs text-gray-500">{t('skillDebugger.profiler.memory')}</p>
              <p class="text-lg text-gray-200 font-mono">{formatBytes(profile.memory_bytes)}</p>
            </div>
            <div class="bg-gray-800/50 rounded-md p-3">
              <p class="text-xs text-gray-500">{t('skillDebugger.profiler.ioOps')}</p>
              <p class="text-lg text-gray-200 font-mono">{profile.io_operations}</p>
            </div>
            <div class="bg-gray-800/50 rounded-md p-3">
              <p class="text-xs text-gray-500">{t('skillDebugger.profiler.subCalls')}</p>
              <p class="text-lg text-gray-200 font-mono">{profile.sub_calls}</p>
            </div>
          </div>

          {/* Timeline */}
          <div>
            <h4 class="text-sm font-semibold text-gray-300 mb-1">
              {t('skillDebugger.profiler.timeline')}
            </h4>
            <div class="bg-gray-900 rounded-md p-3">
              {/* Timeline bar visualization */}
              {profile.timeline.length > 0 && (
                <div class="skill-debugger-timeline">
                  {profile.timeline.map((evt, i) => {
                    const totalMs = profile.timeline[profile.timeline.length - 1].timestamp_ms || 1;
                    const leftPct = (evt.timestamp_ms / totalMs) * 100;
                    const widthPct = evt.duration_ms > 0
                      ? (evt.duration_ms / totalMs) * 100
                      : 2;
                    return (
                      <div key={i} class="skill-debugger-timeline__row">
                        <span class="skill-debugger-timeline__label">{evt.name}</span>
                        <div class="skill-debugger-timeline__bar-wrap">
                          <div
                            class={`skill-debugger-timeline__bar skill-debugger-timeline__bar--${evt.name}`}
                            style={`left: ${leftPct}%; width: ${widthPct}%;`}
                          />
                        </div>
                        <span class="skill-debugger-timeline__time">
                          {evt.timestamp_ms}ms
                          {evt.duration_ms > 0 ? ` (+${evt.duration_ms}ms)` : ''}
                        </span>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
