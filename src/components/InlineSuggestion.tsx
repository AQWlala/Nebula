/**
 * T-E-S-51: Level 0 内联补全 UI 组件。
 *
 * 在 ChatPanel 的单行输入框上叠加灰色 ghost text 建议文本。
 * 用户 Tab 接受、Esc 拒绝;300ms 防抖;AbortController 取消过时请求。
 *
 * ## 集成方式(主 agent 负责)
 *
 * 在 ChatPanel.tsx 中替换原 `<input ...>`:
 * ```tsx
 * <InlineSuggestion
 *   prefix={input}
 *   onAccept={(text) => setInput(text)}
 *   onReject={() => { /* 清空建议 *\/ }}
 * >
 *   {({ onKeyDown }) => (
 *     <input
 *       type="text"
 *       placeholder="输入消息..."
 *       value={input}
 *       onInput={(e) => setInput((e.target as HTMLInputElement).value)}
 *       onKeyDown={(e) => {
 *         onKeyDown(e);
 *         if (e.key === 'Enter') sendStream();
 *       }}
 *       disabled={loading}
 *     />
 *   )}
 * </InlineSuggestion>
 * ```
 *
 * ## 设计约束(spec)
 * - **零成本**:直接 invoke `inline_complete` Tauri 命令,不走远程 API
 * - **失败静默**:任何错误 / null → 不显示建议,不弹 toast
 * - **Ollama 离线降级**:读 `nebulaStore.ollamaStatus`,down 时不发请求
 * - **autonomyLevel 防御性读取**:T-E-S-50 未集成前默认 L2(不启用),
 *   仅 L0 时启用补全
 * - **300ms 防抖**:setTimeout + clearTimeout(useEffect cleanup)
 * - **AbortController**:prefix 变化时取消过时请求
 */
import { useEffect, useRef, useState } from 'preact/hooks';
import { invoke } from '@tauri-apps/api/core';
import { nebulaStore } from '../stores/nebulaStore';

/** 防抖窗口 — spec §设计约束 第 4 条:前端 300ms。 */
const DEBOUNCE_MS = 300;
/** prefix 最小长度(trim 后)— 与后端 MIN_PREFIX_LEN 对齐。 */
const MIN_PREFIX_LEN = 3;
/** input 的左 padding(global.css `input { padding: 8px 12px }`)。 */
const INPUT_PADDING_LEFT = 12;

export interface InlineSuggestionRenderProps {
  /** 传给被包裹 input 的 onKeyDown — 拦截 Tab / Escape。 */
  onKeyDown: (e: any) => void;
}

export interface InlineSuggestionProps {
  /** 当前输入框文本(前缀)。 */
  prefix: string;
  /** Tab 接受时回调,参数为 `prefix + suggestion`(完整文本)。 */
  onAccept: (text: string) => void;
  /** Escape 拒绝时回调。 */
  onReject: () => void;
  /**
   * Render prop — 接收 `{ onKeyDown }` 并返回 input 元素。
   * 调用方应把 `onKeyDown` 合并到 input 的 onKeyDown 中
   * (先调 `onKeyDown(e)` 再处理自己的逻辑,如 Enter 发送)。
   */
  children: (renderProps: InlineSuggestionRenderProps) => any;
}

export function InlineSuggestion({ prefix, onAccept, onReject, children }: InlineSuggestionProps) {
  const [suggestion, setSuggestion] = useState('');
  const [loading, setLoading] = useState(false);
  /** Mirror span ref — 用于测量 prefix 文本宽度,从而定位 ghost text。 */
  const mirrorRef = useRef<HTMLSpanElement>(null);
  const [prefixWidth, setPrefixWidth] = useState(0);

  // T-E-S-50: 读取自主度等级,仅 L0 时启用内联补全(spec §设计约束 第 7 条)。
  const autonomyLevel = nebulaStore.autonomyLevel.value;
  const enabled = autonomyLevel === 'L0';

  // 300ms 防抖 + AbortController 取消过时请求。
  useEffect(() => {
    // prefix 变化时立即清空旧建议(避免显示过时 ghost text)。
    setSuggestion('');

    // 未启用(L2/L3/L4) / prefix 太短 → 不请求。
    // v2.2: 移除 Ollama 离线阻断——后端多 provider 架构不强制本地 Ollama。
    if (!enabled) return;
    if (prefix.trim().length < MIN_PREFIX_LEN) return;

    const controller = new AbortController();
    const timer = setTimeout(async () => {
      setLoading(true);
      try {
        // 直接 invoke Tauri 命令 — 主 agent 集成后可改为 nebulaAPI.inlineComplete。
        // 失败静默:invoke 抛错 → catch → setSuggestion('')。
        const result = await invoke<string | null>('inline_complete', { prefix });
        if (!controller.signal.aborted) {
          setSuggestion(result ?? '');
        }
      } catch {
        // 失败静默(spec §设计约束 第 5 条)— 不弹 toast,不显示错误。
        if (!controller.signal.aborted) {
          setSuggestion('');
        }
      } finally {
        if (!controller.signal.aborted) {
          setLoading(false);
        }
      }
    }, DEBOUNCE_MS);

    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [prefix, enabled]);

  // 测量 prefix 文本宽度(用于定位 ghost text 叠加)。
  // Mirror span 用 visibility:hidden + 相同字体,offsetWidth 即文本像素宽度。
  useEffect(() => {
    if (mirrorRef.current) {
      setPrefixWidth(mirrorRef.current.offsetWidth);
    }
  }, [prefix]);

  /** 拦截 Tab(接受)/ Escape(拒绝)。 */
  const handleKeyDown = (e: any) => {
    if (!suggestion) return;
    const key: string = e.key;
    if (key === 'Tab') {
      e.preventDefault();
      e.stopPropagation();
      onAccept(prefix + suggestion);
      setSuggestion('');
    } else if (key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onReject();
      setSuggestion('');
    }
  };

  return (
    <div
      class="inline-suggestion-wrapper"
      style={{
        position: 'relative',
        flex: 1,
        display: 'flex',
        alignItems: 'stretch',
      }}
    >
      {children({ onKeyDown: handleKeyDown })}

      {/* Mirror span — 不可见,仅用于测量 prefix 文本宽度。
          位置/字体与 input 内容对齐:左 padding 12px + 垂直居中。 */}
      <span
        ref={mirrorRef}
        aria-hidden="true"
        style={{
          position: 'absolute',
          left: `${INPUT_PADDING_LEFT}px`,
          top: '50%',
          transform: 'translateY(-50%)',
          visibility: 'hidden',
          whiteSpace: 'pre',
          pointerEvents: 'none',
          font: 'inherit',
          fontSize: 'inherit',
          letterSpacing: 'inherit',
        }}
      >
        {prefix}
      </span>

      {/* Ghost text 叠加 — 灰色,定位在 prefix 文本之后。 */}
      {suggestion && !loading && (
        <span
          class="inline-suggestion-overlay"
          aria-hidden="true"
          style={{
            position: 'absolute',
            left: `calc(${INPUT_PADDING_LEFT}px + ${prefixWidth}px)`,
            top: '50%',
            transform: 'translateY(-50%)',
            color: 'var(--text-muted)',
            pointerEvents: 'none',
            whiteSpace: 'pre',
            opacity: 0.6,
            font: 'inherit',
            fontSize: 'inherit',
            letterSpacing: 'inherit',
          }}
        >
          {suggestion}
        </span>
      )}
    </div>
  );
}
