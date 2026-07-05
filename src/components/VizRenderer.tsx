/**
 * T-E-S-38 VizRenderer — 渲染分发器。
 *
 * 根据 kind 选择渲染路径:
 * - canvas: 从 LLM 输出中提取 <<<HTML>>>...<<<END>>> 包裹的 HTML,
 *   用 iframe srcdoc 渲染(sandbox="allow-scripts" 隔离 DOM)。
 *   LLM 输出不符约定时降级为 <pre> 显示原始输出。
 * - mermaid / mindmap: 从 LLM 输出中提取 ```mermaid ... ``` fenced 代码块,
 *   交给 MermaidView 渲染。提取失败降级为 <pre> 显示原始输出。
 */

import MermaidView from './MermaidView';
import { t } from '../i18n';

export type VizKind = 'canvas' | 'mermaid' | 'mindmap';

interface VizRendererProps {
  /** 渲染类型。 */
  kind: VizKind;
  /** LLM 原始输出(skillUse 的 output 字段)。 */
  output: string;
}

/**
 * 从 LLM 输出中提取 <<<HTML>>>...<<<END>>> 包裹的 HTML 内容。
 * 提取失败返回 null(调用方降级为 <pre> 显示)。
 */
export function extractCanvasHtml(output: string): string | null {
  if (!output) return null;
  const re = /<<<HTML>>>\s*([\s\S]*?)\s*<<<END>>>/i;
  const m = output.match(re);
  return m ? m[1].trim() : null;
}

/**
 * 从 LLM 输出中提取 ```mermaid ... ``` fenced 代码块内容。
 * 支持多个 fenced block,取第一个非空的。
 * 提取失败返回 null(调用方降级为 <pre> 显示)。
 */
export function extractMermaidCode(output: string): string | null {
  if (!output) return null;
  // 匹配 ```mermaid ... ``` (允许 ``` 后跟语言标识)。
  const re = /```(?:mermaid|mindmap)\s*\n([\s\S]*?)```/i;
  const m = output.match(re);
  if (m) return m[1].trim();
  // 退化:如果输出本身看起来就是 mermaid 源(以 flowchart/sequenceDiagram/
  // gantt/stateDiagram/classDiagram/mindmap 开头),直接当作源码。
  const trimmed = output.trim();
  if (/^(flowchart|graph|sequenceDiagram|gantt|stateDiagram|classDiagram|mindmap|erDiagram|pie|gitGraph|journey)\b/i.test(trimmed)) {
    return trimmed;
  }
  return null;
}

export default function VizRenderer({ kind, output }: VizRendererProps) {
  if (!output?.trim()) {
    return (
      <div class="flex items-center justify-center h-full text-gray-500 text-sm">
        {t('vizRenderer.waiting')}
      </div>
    );
  }

  if (kind === 'canvas') {
    const html = extractCanvasHtml(output);
    if (!html) {
      // 降级:LLM 输出不符约定,显示原始输出。
      return (
        <div class="h-full flex flex-col gap-2">
          <div class="px-3 py-2 bg-yellow-900/30 border border-yellow-700 text-yellow-300 text-xs rounded-md">
            {t('vizRenderer.canvasFallback')}
          </div>
          <pre class="flex-1 p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-auto whitespace-pre-wrap">
            <code>{output}</code>
          </pre>
        </div>
      );
    }
    // iframe srcdoc + sandbox="allow-scripts"(无 allow-same-origin)隔离 DOM。
    return (
      <iframe
        title="canvas-viz"
        class="w-full h-full border-0 bg-[#1E293B] rounded-md"
        sandbox="allow-scripts"
        srcdoc={html}
      />
    );
  }

  // mermaid / mindmap 共用 MermaidView。
  const code = extractMermaidCode(output);
  if (!code) {
    return (
      <div class="h-full flex flex-col gap-2">
        <div class="px-3 py-2 bg-yellow-900/30 border border-yellow-700 text-yellow-300 text-xs rounded-md">
          {t('vizRenderer.mermaidFallback')}
        </div>
        <pre class="flex-1 p-3 bg-gray-900 rounded-md text-xs text-gray-300 overflow-auto whitespace-pre-wrap">
          <code>{output}</code>
        </pre>
      </div>
    );
  }

  return (
    <div class="h-full w-full bg-gray-900 rounded-md p-2">
      <MermaidView code={code} idPrefix={kind} />
    </div>
  );
}
