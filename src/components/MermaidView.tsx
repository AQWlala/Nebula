/**
 * T-E-S-38 MermaidView — 动态 import('mermaid') + mermaid.render。
 *
 * 设计要点:
 * - 动态 import('mermaid') 让 Vite 代码分割,首屏不加载 mermaid(~600KB)。
 * - mermaid.render 返回 SVG 字符串,通过 dangerouslySetInnerHTML 注入。
 * - 降级:render 失败显示错误信息 + 原始代码 <pre>。
 * - mindmap 语法在 mermaid 11+ 原生支持,无需特殊处理。
 */

import { useEffect, useRef, useState } from 'preact/hooks';
import { t } from '../i18n';

interface MermaidViewProps {
  /** Mermaid 源代码(不含 ```mermaid fenced 包裹)。 */
  code: string;
  /** 可选 id 前缀,避免多实例 SVG id 冲突。 */
  idPrefix?: string;
}

interface MermaidApi {
  initialize: (config: Record<string, unknown>) => void;
  render: (id: string, code: string) => Promise<{ svg: string }>;
}

/** 模块级缓存:mermaid 仅 import 一次,后续渲染复用。 */
let mermaidApiPromise: Promise<MermaidApi> | null = null;

async function loadMermaid(): Promise<MermaidApi> {
  if (!mermaidApiPromise) {
    // 动态 import — Vite 会自动代码分割出 mermaid chunk。
    // mermaid 11+ 默认导出即为 mermaid API 对象(initialize / render 等方法)。
    mermaidApiPromise = import('mermaid').then((mod) => {
      const mermaid = (mod as { default: MermaidApi }).default;
      mermaid.initialize({
        startOnLoad: false,
        theme: 'dark',
        securityLevel: 'strict',
        fontFamily: 'ui-sans-serif, system-ui, sans-serif',
      });
      return mermaid;
    });
  }
  return mermaidApiPromise;
}

export default function MermaidView({ code, idPrefix = 'mmd' }: MermaidViewProps) {
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const renderSeqRef = useRef(0);

  useEffect(() => {
    const trimmed = code?.trim();
    if (!trimmed) {
      setSvg(null);
      setError(null);
      return;
    }
    const seq = ++renderSeqRef.current;
    setLoading(true);
    setError(null);
    loadMermaid()
      .then((mermaid) => {
        const renderId = `${idPrefix}-${seq}-${Date.now()}`;
        return mermaid.render(renderId, trimmed).then(
          (result: { svg: string }) => {
            // 仅采纳最新一次渲染结果,避免乱序覆盖。
            if (renderSeqRef.current !== seq) return;
            setSvg(result.svg);
            setError(null);
            setLoading(false);
          },
          (err: unknown) => {
            if (renderSeqRef.current !== seq) return;
            const msg = err instanceof Error ? err.message : String(err);
            setError(msg);
            setSvg(null);
            setLoading(false);
          }
        );
      })
      .catch((err: unknown) => {
        if (renderSeqRef.current !== seq) return;
        const msg = err instanceof Error ? err.message : String(err);
        setError(t('mermaidView.loadFailed', { error: msg }));
        setSvg(null);
        setLoading(false);
      });
  }, [code, idPrefix]);

  if (loading) {
    return (
      <div class="flex items-center justify-center h-full text-gray-400 text-sm">
        {t('mermaidView.rendering')}
      </div>
    );
  }

  if (error) {
    return (
      <div class="h-full flex flex-col gap-2">
        <div class="px-3 py-2 bg-red-900/30 border border-red-700 text-red-300 text-xs rounded-md">
          {t('mermaidView.renderFailed', { error })}
        </div>
        <pre class="flex-1 p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-auto">
          <code>{code}</code>
        </pre>
      </div>
    );
  }

  if (!svg) {
    return (
      <div class="flex items-center justify-center h-full text-gray-500 text-sm">
        {t('mermaidView.waitingInput')}
      </div>
    );
  }

  return (
    <div
      class="mermaid-render h-full w-full overflow-auto flex items-center justify-center p-2"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}
