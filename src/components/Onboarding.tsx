/**
 * v1.0.1: P1-7 Onboarding 引导增强
 *
 * 3 步引导流程：
 * 1. 欢迎 + 确认安装路径
 * 2. 配置 Ollama 端点（自动检测 localhost:11434）
 * 3. 开始使用（创建第一个 Skill 或导入示例数据）
 *
 * 进度指示器 + 自动 Ollama 连接检测
 */
import { useState, useEffect } from 'preact/hooks';
import { signal } from '@preact/signals';
import { invoke } from '@tauri-apps/api/core';
import { t } from '../i18n';

const STORAGE_KEY = 'nebula.onboarding.completed';

interface Step {
  title: () => string;
  body: () => string;
}

const STEPS: Step[] = [
  { title: () => t('onboarding.step1.title'), body: () => t('onboarding.step1.body') },
  { title: () => t('onboarding.step2.title'), body: () => t('onboarding.step2.body') },
  { title: () => t('onboarding.step3.title'), body: () => t('onboarding.step3.body') },
];

/** 进度指示器配置 */
const STEP_META = [
  { labelKey: 'onboarding.stepWelcome', descKey: 'onboarding.stepWelcomeDesc' },
  { labelKey: 'onboarding.stepLlm', descKey: 'onboarding.stepLlmDesc' },
  { labelKey: 'onboarding.stepStart', descKey: 'onboarding.stepStartDesc' },
];

/** Synchronously seeded from localStorage. */
function readInitialOnboarded(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

export const onboarded = signal<boolean>(readInitialOnboarded());

export function markOnboarded(): void {
  onboarded.value = true;
  try {
    localStorage.setItem(STORAGE_KEY, '1');
  } catch {
    /* ignore quota / private-mode errors */
  }
}

export function Onboarding({ onDone }: { onDone: () => void }) {
  const [idx, setIdx] = useState(0);
  const last = idx === STEPS.length - 1;
  const step = STEPS[idx];

  // P1-7: Ollama 连接状态
  const [ollamaStatus, setOllamaStatus] = useState<'checking' | 'ok' | 'down'>('checking');
  const [ollamaUrl, setOllamaUrl] = useState('http://localhost:11434');

  // P1-7: Step 2 时自动检测 Ollama
  useEffect(() => {
    if (idx === 1) {
      checkOllama();
    }
  }, [idx]);

  async function checkOllama() {
    setOllamaStatus('checking');
    try {
      const health = await invoke<{ status: string; version: string; ollama?: 'ok' | 'down' }>('health');
      setOllamaStatus(health?.ollama === 'ok' ? 'ok' : 'down');
    } catch {
      setOllamaStatus('down');
    }
  }

  function finish() {
    markOnboarded();
    onDone();
  }

  return (
    <div class="onboarding" role="dialog" aria-labelledby="onboarding-title">
      <div class="onboarding-card">
        {/* P1-7: 3 步进度指示器 */}
        <div class="onboarding-progress">
          {STEP_META.map((meta, i) => (
            <div key={`step-${i}`} class={`step-indicator ${i === idx ? 'active' : i < idx ? 'done' : ''}`}>
              <div class="step-dot">
                {i < idx ? '✓' : i + 1}
              </div>
              <div class="step-label">
                <span class="step-title">{t(meta.labelKey as any)}</span>
                <span class="step-desc">{t(meta.descKey as any)}</span>
              </div>
              {i < STEP_META.length - 1 && <div class="step-line" />}
            </div>
          ))}
        </div>

        <h2 id="onboarding-title">{step.title()}</h2>
        <p>{step.body()}</p>

        {/* P1-7: Step 2 - Ollama 配置界面 */}
        {idx === 1 && (
          <div class="ollama-config">
            <div class="ollama-status-row">
              <span class="ollama-status-label">{t('onboarding.ollamaEndpoint')}:</span>
              <input
                type="text"
                value={ollamaUrl}
                onInput={(e) => setOllamaUrl((e.target as HTMLInputElement).value)}
                class="ollama-url-input"
                placeholder="http://localhost:11434"
              />
            </div>
            <div class="ollama-status-row">
              <span class="ollama-status-label">{t('onboarding.connectionStatus')}:</span>
              <span class={`ollama-status-badge ${ollamaStatus}`}>
                {ollamaStatus === 'checking' && t('onboarding.checking')}
                {ollamaStatus === 'ok' && t('onboarding.ollamaOk')}
                {ollamaStatus === 'down' && t('onboarding.ollamaDown')}
              </span>
              <button onClick={checkOllama} class="ghost" disabled={ollamaStatus === 'checking'}>
                {t('onboarding.retry')}
              </button>
            </div>
            {ollamaStatus === 'down' && (
              <div class="ollama-hint">
                {t('onboarding.ollamaHint')}
              </div>
            )}
          </div>
        )}

        <footer class="onboarding-footer">
          <button class="ghost" onClick={finish}>{t('onboarding.skip')}</button>
          <div class="spacer" />
          {idx > 0 && (
            <button onClick={() => setIdx(idx - 1)}>{t('onboarding.back')}</button>
          )}
          {last ? (
            <button class="primary" onClick={finish}>{t('onboarding.finish')}</button>
          ) : (
            <button class="primary" onClick={() => setIdx(idx + 1)}>
              {t('onboarding.next')}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

export function shouldShowOnboarding(): boolean {
  return !onboarded.value;
}

export function __resetOnboardingForTests(): void {
  onboarded.value = false;
  try { localStorage.removeItem(STORAGE_KEY); } catch { /* ignore */ }
}
