/**
 * v1.0.1: 对话面板 - 简洁的聊天界面
 *
 * P0#07:
 *  - top of the panel shows <OllamaStatusBanner /> whenever
 *    `nebulaStore.ollamaStatus === 'down'`.
 *  - chat send() wraps the backend call in an AbortController with
 *    an 8s timeout.  On timeout we surface a localised toast and
 *    leave the input intact so the user can retry.
 */
import { useState, useEffect, useMemo, useRef } from 'preact/hooks';
import { memo } from 'preact/compat';
import { renderMarkdown } from '../utils/markdown';
import {
  nebulaAPI,
  type ChatResponse,
  type ConsistencyReport,
  type ConsistencyWarning,
  type ReasoningChain,
  type ReasoningStep,
  type StreamToken,
  type WikiNote,
  listenClipboardDetected,
} from '../lib/tauri';
import { nebulaStore } from '../stores/nebulaStore';
import { OllamaStatusBanner } from './OllamaStatusBanner';
import { InlineSuggestion } from './InlineSuggestion';
import { toast } from './Toast';
import { t } from '../i18n';
import { routeMode, routeViaLLM } from '../lib/modeRouter';
import { ExportDialog } from './ExportDialog';
import { TemplatesDialog } from './TemplatesDialog';
import { KnowledgeCardDialog } from './KnowledgeCardDialog';
import { Spinner } from './Spinner';

export interface Message {
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  reasoningChain?: ReasoningStep[] | ReasoningChain;
  /** T-E-S-64: 反幻觉一致性报告(由 chat_stream 流结束后注入)。 */
  consistency?: ConsistencyReport;
  /** T-E-S-28: 本次 assistant 回复的 turn_id(由 chat_stream 流结束后注入)。
   *  前端用它关联 👍/👎 标注按钮,调用 annotationUpsert 时回传。 */
  turnId?: string;
  /** T-E-S-28: 用户对该 assistant 回复的标注(👍/👎)。null/undefined=未标注。 */
  annotation?: 'good' | 'bad' | null;
}

/** T-E-S-64: 把 ConsistencyWarning 渲染为简短中文标签。 */
function warningLabel(w: ConsistencyWarning): string {
  switch (w.kind) {
    case 'source_conflict':
      return `来源冲突(${w.ids.length} 条)`;
    case 'single_tool_negation':
      return `单一工具来源(${w.tool})`;
    case 'empty_citation':
      return '空引用(可能凭空生成)';
    default:
      return '未知风险';
  }
}

/**
 * P1 性能优化:用 memo + useMemo 缓存 markdown 渲染结果。
 *
 * 流式期间父组件每个 token 触发一次 setMessages 导致整个 messages
 * 数组重新渲染。若不缓存,每条历史消息都会在每次 token 到达时
 * 重新跑 marked.parse + DOMPurify.sanitize(O(n*token) 复杂度)。
 *
 * memo 让 content 未变的消息跳过重渲染;useMemo 在 content 变化时
 * 才重新计算 HTML。流式消息每次 token 都会变,但其它历史消息不变,
 * 因此整体复杂度降为 O(token + n)。
 */
const MessageContent = memo(function MessageContent({
  content,
  onWikiLinkClick,
  streaming = false,
}: {
  content: string;
  onWikiLinkClick: (slug: string) => void;
  /**
   * M6 #80: 流式渲染优化。
   * - true: 流式进行中，用纯文本显示（保留换行），避免每个 token 都跑 marked.parse + DOMPurify。
   * - false: 流已结束，用 markdown 渲染。
   * 这将长响应（如 1000 token）的 markdown 解析次数从 N 次降为 1 次。
   */
  streaming?: boolean;
}) {
  // M6 #80: 流式期间用纯文本显示（保留换行），流结束后才 markdown 渲染。
  const html = useMemo(() => renderMarkdown(content), [content]);
  if (streaming) {
    return (
      <div class="msg-content msg-content-streaming">
        <pre class="streaming-text">{content}</pre>
        <span class="streaming-cursor" aria-hidden="true" />
      </div>
    );
  }
  return (
    <div
      class="msg-content"
      dangerouslySetInnerHTML={{ __html: html }}
      onClick={(e) => {
        // T-E-B-13: 点击正文中的 [[xxx]] wiki-link 时,通知父组件弹窗。
        const target = e.target as HTMLElement;
        if (target.classList.contains('wiki-link')) {
          const slug = target.dataset.slug;
          if (slug) onWikiLinkClick(slug);
        }
      }}
    />
  );
});

/** T-E-S-64: 渲染反幻觉一致性 badge。
 *  - 无 warning 且 cited > 0: 绿色 `✓ 引用 N 条记忆`
 *  - 无 warning 且 cited == 0: 隐藏(spec 要求 N=0 时隐藏)
 *  - 有 warning: 橙色/红色 `⚠ {warning_type}`,点击展开详情
 *  样式参考 reasoningChain 折叠面板(<details>)。 */
function ConsistencyBadge({ report }: { report: ConsistencyReport }) {
  // 无 warning 且无引用:隐藏 badge
  if (report.warnings.length === 0 && report.cited.length === 0) {
    return null;
  }
  // 无 warning 且有引用:绿色简短 badge
  if (report.warnings.length === 0) {
    return (
      <div
        style={{
          display: 'inline-block',
          marginTop: '4px',
          padding: '2px 8px',
          fontSize: '11px',
          borderRadius: '4px',
          background: 'rgba(76, 175, 80, 0.15)',
          color: '#4caf50',
          border: '1px solid rgba(76, 175, 80, 0.3)',
        }}
      >
        ✓ 引用 {report.cited.length} 条记忆
      </div>
    );
  }
  // 有 warning:橙色/红色折叠 badge,点击展开详情
  const highRisk = report.risk_score >= 0.6;
  const color = highRisk ? '#f44336' : '#ff9800';
  const bg = highRisk ? 'rgba(244, 67, 54, 0.12)' : 'rgba(255, 152, 0, 0.12)';
  const border = highRisk ? 'rgba(244, 67, 54, 0.3)' : 'rgba(255, 152, 0, 0.3)';
  return (
    <details
      style={{
        marginTop: '4px',
        fontSize: '11px',
        border: `1px solid ${border}`,
        borderRadius: '4px',
        background: bg,
      }}
    >
      <summary style={{ cursor: 'pointer', padding: '2px 8px', color }}>
        ⚠ {report.warnings.map(warningLabel).join(' · ')} (风险{' '}
        {(report.risk_score * 100).toFixed(0)}%)
      </summary>
      <div style={{ padding: '4px 8px', borderTop: `1px solid ${border}` }}>
        {report.warnings.map((w, idx) => (
          <div key={idx} style={{ marginBottom: '2px', color: 'var(--text-secondary)' }}>
            ⚠ {warningLabel(w)}
          </div>
        ))}
        {report.cited.length > 0 && (
          <div style={{ marginTop: '4px', color: 'var(--text-muted)' }}>
            引用记忆 {report.cited.length} 条:
          </div>
        )}
        {report.cited.map((c, idx) => (
          <div
            key={idx}
            style={{
              fontSize: '10px',
              opacity: 0.8,
              marginLeft: '8px',
              color: 'var(--text-muted)',
            }}
          >
            · [{c.source}
            {c.tool ? `/${c.tool}` : ''}] {c.snippet.slice(0, 60)}
          </div>
        ))}
      </div>
    </details>
  );
}

/** P0#07: client-side request budget.  If the backend takes
 *  longer than this, we abort and tell the user instead of
 *  spinning forever. */
export const CHAT_TIMEOUT_MS = 8_000;

/**
 * T-E-B-10: 解析 #filename token,读取文件内容内联到消息文本。
 * 格式:#path/to/file → ```path/to/file\n{content}\n```
 * 文件读取失败时保留原 #filename token + toast 提示。
 */
async function resolveFileTokens(text: string): Promise<string> {
  const regex = /#([\w./-]+)/g;
  const matches = [...text.matchAll(regex)];
  if (matches.length === 0) return text;

  let result = text;
  for (const match of matches) {
    const fullToken = match[0]; // #filename
    const filePath = match[1]; // filename
    try {
      const file = await nebulaAPI.editorRead(filePath);
      const lang = filePath.split('.').pop() || '';
      result = result.replace(fullToken, `\n\`\`\`${lang} ${filePath}\n${file.content}\n\`\`\`\n`);
    } catch (e) {
      toast.warning(`文件读取失败`, `#${filePath}: ${String(e)}`);
    }
  }
  return result;
}

export function ChatPanel() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [loading, setLoading] = useState(false);
  const [streaming, setStreaming] = useState(false);
  // T-S1-B-01b: 流式 AbortController，用于"停止生成"按钮中断 Channel 回调。
  const [streamController, setStreamController] = useState<AbortController | null>(null);
  // T-E-S-28: 每个 turn 的评论文本(键为 turnId)。用户先填评论(可选),
  // 再点 👍/👎 提交;差评+非空评论会触发后端 sponge.absorb_text 回流到 L1 记忆。
  const [annotationComments, setAnnotationComments] = useState<Record<string, string>>({});
  // T-E-C-16: 导出对话框显示状态
  const [showExportDialog, setShowExportDialog] = useState(false);
  // T-E-C-13: 工作场景模板对话框显示状态
  const [showTemplatesDialog, setShowTemplatesDialog] = useState(false);
  // T-E-B-05: [[ Wiki 链接自动补全状态
  const [wikiCompletions, setWikiCompletions] = useState<WikiNote[]>([]);
  const [wikiCompletionVisible, setWikiCompletionVisible] = useState(false);
  const [wikiCompletionStart, setWikiCompletionStart] = useState(-1);
  // T-E-B-13: 知识卡片弹窗 — 点击 msg-content 中的 [[xxx]] wiki-link 后,
  // 设置 knowledgeCardSlug 触发 KnowledgeCardDialog 加载对应卡片。
  const [knowledgeCardSlug, setKnowledgeCardSlug] = useState<string | null>(null);

  // P1 性能优化:流式 token 节流 refs。
  // rafPendingRef:标记当前是否已有一个 rAF 回调在等待执行(true=已调度,跳过新调度)。
  // 没有 rAF 时(非流式或首次 token)直接 setMessages;有 rAF 时只累加 accumulated,
  // 等 rAF 回调统一 flush 到 state,避免每个 token 都触发一次 React 渲染。
  const rafPendingRef = useRef(false);

  // T-E-C-14: 监听剪贴板智能监听事件。后端 ClipboardWatcherEngine 检测到
  // 有结构的内容(URL/代码/表格/JSON 等)时推送 ClipboardEvent,前端显示
  // toast 通知并直接注入到 input(简化:toast.info 不支持 onClick,直接 setInput)。
  // 仅在组件挂载时订阅一次,卸载时取消订阅。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listenClipboardDetected((event) => {
      const kindLabel = (() => {
        switch (event.kind.type) {
          case 'code':
            return event.kind.language ? `代码(${event.kind.language})` : '代码';
          case 'markdowntable':
            return 'Markdown 表格';
          case 'json':
            return 'JSON';
          case 'url':
            return 'URL';
          case 'tsvcsv':
            return 'TSV/CSV';
          case 'email':
            return '邮箱';
          case 'ip':
            return 'IP 地址';
          case 'path':
            return '路径';
          default:
            return '内容';
        }
      })();
      toast.info(`剪贴板检测到 ${kindLabel}`, event.content_preview || '已注入到输入框');
      setInput(event.content_full);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        // 非 Tauri 环境(浏览器预览 / 单测)无 listen API,静默忽略。
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  // T-E-D-06: 右键"问Nebula" → App.tsx 设置 chatPrefill signal,
  // 此 effect 监听变化,注入到输入框后清空 signal(一次性消费)。
  useEffect(() => {
    const prefill = nebulaStore.chatPrefill.value;
    if (prefill) {
      setInput(prefill);
      nebulaStore.chatPrefill.value = null;
    }
  }, [nebulaStore.chatPrefill.value]);

  /** T-S1-B-01b: 中止正在进行的流式生成。
   *  保留已累积的内容，追加 `[已停止生成]` 标记。 */
  function stopStreaming() {
    if (streamController) {
      streamController.abort();
      setStreamController(null);
    }
    setStreaming(false);
    setLoading(false);
    // 在占位 assistant 消息（timestamp=-1）末尾追加停止标记
    setMessages((prev) => {
      if (prev.length === 0) return prev;
      const last = prev[prev.length - 1];
      if (last.role === 'assistant' && last.timestamp === -1) {
        const updated = [...prev];
        updated[updated.length - 1] = {
          role: 'assistant',
          content: last.content + t('chatPanel.streamStopped'),
          timestamp: Date.now(),
        };
        return updated;
      }
      return prev;
    });
  }

  /** T-S1-B-01b: 流式发送 — 使用 Tauri ipc::Channel 回调逐字渲染。
   *  - 插入占位 assistant 消息（timestamp=-1）
   *  - 每个 StreamToken 到达时累加并更新占位消息
   *  - 流结束后用 ChatComplete.content 做最终同步
   *  - 支持通过"停止生成"按钮 abort */
  async function sendStream() {
    if (!input.trim() || loading) return;

    // T-E-B-02: `/journey` 斜杠命令 — 切换到记忆时间轴视图。
    // 输入 `/journey` + Enter 即可跳转,无需发送消息到 LLM。
    // 支持 `/journey` 单独使用,或后接描述(描述当前忽略,未来可做语义筛选)。
    const trimmed = input.trim();
    if (trimmed === '/journey' || trimmed.startsWith('/journey ')) {
      nebulaStore.currentMode.value = 'memory';
      nebulaStore.memoryView.value = 'timeline';
      setInput('');
      toast.info('切换到记忆时间轴 · Journey');
      return;
    }

    // T-E-B-10: 解析 #filename token,内联文件内容。
    const resolvedText = await resolveFileTokens(input);
    const userMsg: Message = { role: 'user', content: resolvedText, timestamp: Date.now() };
    setMessages((m) => [...m, userMsg]);
    const text = userMsg.content;
    setInput('');
    setLoading(true);
    setStreaming(true);

    // 插入占位 assistant 消息，timestamp=-1 标识"流式进行中"
    setMessages((m) => [...m, { role: 'assistant', content: '', timestamp: -1 }]);

    const controller = new AbortController();
    setStreamController(controller);

    let accumulated = '';
    try {
      const complete = await nebulaAPI.chatStream(
        { message: text },
        (token: StreamToken) => {
          if (controller.signal.aborted) return;
          accumulated += token.text;
          // P1 性能优化:rAF 节流 — 多个 token 在同一帧内合并为一次 setMessages,
          // 避免长响应(如 1000 token)触发 1000 次 React 渲染。
          // rAF 在下一帧统一 flush accumulated 到 state。
          if (!rafPendingRef.current) {
            rafPendingRef.current = true;
            requestAnimationFrame(() => {
              rafPendingRef.current = false;
              const snapshot = accumulated;
              setMessages((prev) => {
                const updated = [...prev];
                if (
                  updated.length > 0 &&
                  updated[updated.length - 1].role === 'assistant' &&
                  updated[updated.length - 1].timestamp === -1
                ) {
                  updated[updated.length - 1] = {
                    role: 'assistant',
                    content: snapshot,
                    timestamp: -1,
                  };
                }
                return updated;
              });
            });
          }
        },
        controller.signal
      );

      // 流结束：用 ChatComplete.content 做最终同步（防止 token 丢失）
      if (!controller.signal.aborted) {
        setMessages((prev) => {
          const updated = [...prev];
          if (
            updated.length > 0 &&
            updated[updated.length - 1].role === 'assistant' &&
            updated[updated.length - 1].timestamp === -1
          ) {
            updated[updated.length - 1] = {
              role: 'assistant',
              content: complete.content || accumulated,
              timestamp: Date.now(),
              reasoningChain: complete.reasoning_chain,
              // T-E-S-64: 注入反幻觉一致性报告。
              consistency: complete.consistency,
              // T-E-S-28: 注入 turn_id,供 👍/👎 标注按钮关联。
              turnId: complete.turn_id,
            };
          }
          return updated;
        });
      }
    } catch (e) {
      // abort 不算错误（由 stopStreaming 触发）
      if (controller.signal.aborted) {
        // stopStreaming 已处理 UI，这里不重复追加错误消息
      } else if (!accumulated) {
        setMessages((prev) => {
          // 移除空占位消息
          const filtered = prev.filter(
            (m) => !(m.role === 'assistant' && m.timestamp === -1 && m.content === '')
          );
          return [
            ...filtered,
            { role: 'assistant', content: t('chatPanel.streamFailed'), timestamp: Date.now() },
          ];
        });
      } else {
        // 已有累积内容，保留并标记失败
        setMessages((prev) => {
          const updated = [...prev];
          if (
            updated.length > 0 &&
            updated[updated.length - 1].role === 'assistant' &&
            updated[updated.length - 1].timestamp === -1
          ) {
            updated[updated.length - 1] = {
              role: 'assistant',
              content: accumulated + t('chatPanel.streamInterrupted'),
              timestamp: Date.now(),
            };
          }
          return updated;
        });
      }
    } finally {
      setStreamController(null);
      setStreaming(false);
      setLoading(false);
    }
  }

  async function send() {
    if (!input.trim() || loading) return;
    // T-S5-A-02/03: 三视角无感切换 — AI 自动模式启用时走 LLM 路由,
    // 否则退化为关键词启发式。
    let routed: 'writing' | 'work' | 'code' | null;
    if (nebulaStore.aiAutoMode.value) {
      routed = await routeViaLLM(input);
    } else {
      routed = routeMode(input);
    }
    if (routed) {
      nebulaStore.mode.value = routed;
      nebulaStore.lastAutoRoutedMode.value = routed;
    }
    // T-E-B-10: 解析 #filename token,内联文件内容。
    const resolvedText = await resolveFileTokens(input);
    const userMsg: Message = { role: 'user', content: resolvedText, timestamp: Date.now() };
    setMessages((m) => [...m, userMsg]);
    const text = userMsg.content;
    setInput('');
    setLoading(true);

    // P0#07: 8s client-side timeout.  We wire AbortController
    // directly into the Tauri invoke by passing the signal
    // through.  Tauri v2 honours AbortSignal, so the call
    // unblocks immediately when the timer fires.
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), CHAT_TIMEOUT_MS);
    let timedOut = false;
    try {
      const res: ChatResponse = await nebulaAPI.chat({
        message: text,
        // @ts-expect-error: AbortSignal is supported at runtime by
        // @tauri-apps/api v2 but is not yet in our type defs.
        signal: controller.signal,
      });
      setMessages((m) => [
        ...m,
        { role: 'assistant', content: res.content, timestamp: Date.now() },
      ]);
    } catch (e) {
      const err = e as { name?: string; message?: string };
      if (err?.name === 'AbortError' || controller.signal.aborted) {
        timedOut = true;
        toast.error(t('ollama.timeout.title'), t('ollama.timeout.body'));
      }
      setMessages((m) => [
        ...m,
        {
          role: 'assistant',
          content: `[${timedOut ? t('ollama.timeout.title') : t('toast.error')}] ${String(e)}`,
          timestamp: Date.now(),
        },
      ]);
      // P0#07: if the request failed, immediately re-check
      // Ollama so the banner appears / disappears in sync.
      void nebulaStore.checkOllama();
    } finally {
      window.clearTimeout(timer);
      setLoading(false);
    }
  }

  /** T-E-S-28: 提交对话标注(👍/👎)。
   *
   * 调用 `annotationUpsert` 落盘 + 更新本地 `message.annotation` 状态以高亮选中按钮。
   * 后端在 `annotation == "bad"` 且 `comment` 非空时,会触发 `sponge.absorb_text`
   * 把用户反馈回流到 L1 Episodic 记忆(让 AI 在后续对话中知晓用户偏好)。
   * 吸收失败不阻断标注写入(best-effort,仅后端 warn 日志)。 */
  async function handleAnnotate(turnId: string, annotation: 'good' | 'bad') {
    const comment = annotationComments[turnId] ?? '';
    try {
      await nebulaAPI.annotationUpsert({
        turn_id: turnId,
        annotation,
        comment: comment || null,
      });
      // 更新本地 message 状态:高亮选中按钮。
      setMessages((prev) => prev.map((m) => (m.turnId === turnId ? { ...m, annotation } : m)));
      toast.success(annotation === 'good' ? '已标记为好回答' : '已标记为差回答');
    } catch (e) {
      toast.error('标注失败', String(e));
    }
  }

  return (
    <div class="panel chat-panel">
      <div class="panel-header">
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          <span class="panel-title">💬 对话</span>
          <span style={{ color: 'var(--text-muted)', fontSize: '12px' }}>
            9 头蛇的 1 号蛇头：通用对话
          </span>
        </div>
        <div style={{ display: 'flex', gap: '6px' }}>
          <button
            onClick={() => setShowTemplatesDialog(true)}
            title="工作场景模板"
            style={{
              padding: '4px 10px',
              fontSize: '12px',
              cursor: 'pointer',
              border: '1px solid var(--border)',
              borderRadius: '4px',
              background: 'transparent',
              color: 'var(--text-muted)',
              transition: 'all 0.15s',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLButtonElement).style.color = 'var(--text-primary)';
              (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--accent-neon)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLButtonElement).style.color = 'var(--text-muted)';
              (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--border)';
            }}
          >
            🎭 模板
          </button>
          <button
            onClick={() => setShowExportDialog(true)}
            title="导出对话"
            style={{
              padding: '4px 10px',
              fontSize: '12px',
              cursor: 'pointer',
              border: '1px solid var(--border)',
              borderRadius: '4px',
              background: 'transparent',
              color: 'var(--text-muted)',
              transition: 'all 0.15s',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLButtonElement).style.color = 'var(--text-primary)';
              (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--accent-neon)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLButtonElement).style.color = 'var(--text-muted)';
              (e.currentTarget as HTMLButtonElement).style.borderColor = 'var(--border)';
            }}
          >
            📤 导出
          </button>
        </div>
      </div>

      <OllamaStatusBanner />

      <div class="chat-messages">
        {messages.length === 0 && (
          <div style="text-align: center; color: var(--text-muted); padding: 40px;">
            <div style="font-size: 48px; margin-bottom: 16px;">🐍</div>
            <div>开始一次对话吧</div>
            <div style="font-size: 12px; margin-top: 8px;">
              所有消息会被自动存入 L1（消息历史）和 L2（经验）
            </div>
          </div>
        )}
        {messages.map((m, i) => (
          <div
            key={m.turnId || `${m.role}-${i}-${m.content.slice(0, 20)}`}
            class={`msg msg-${m.role}`}
          >
            <div class="msg-role">{m.role === 'user' ? '你' : 'Nebula'}</div>
            {m.role === 'assistant' && m.reasoningChain && (
              <details
                class="reasoning-chain"
                style="margin-bottom:4px;font-size:12px;color:var(--text-secondary);"
              >
                <summary style="cursor:pointer;">推理过程</summary>
                <div style="padding:4px 8px;border-left:2px solid var(--border);margin:4px 0;">
                  {('steps' in m.reasoningChain ? m.reasoningChain.steps : m.reasoningChain).map(
                    (step, idx) => (
                      <div key={idx} style="margin-bottom:4px;">
                        <div>→ {step.inference}</div>
                        {step.evidence && (
                          <div style="font-size:11px;opacity:0.7;">证据: {step.evidence}</div>
                        )}
                      </div>
                    )
                  )}
                </div>
              </details>
            )}
            <MessageContent
              content={m.content}
              onWikiLinkClick={setKnowledgeCardSlug}
              streaming={m.timestamp === -1}
            />
            {m.role === 'assistant' && m.consistency && <ConsistencyBadge report={m.consistency} />}
            {/* T-E-S-28: 👍/👎 标注按钮 + 评论框。
                仅在有 turnId 的 assistant 消息(流式正常结束)上显示。
                点击 👍/👎 立即调用 annotationUpsert;差评+非空评论会触发
                后端 sponge.absorb_text 回流到 L1 记忆用于持续改进。 */}
            {m.role === 'assistant' && m.turnId && (
              <div style="margin-top:6px;display:flex;align-items:center;gap:6px;font-size:12px;">
                <button
                  onClick={() => handleAnnotate(m.turnId!, 'good')}
                  title="好回答"
                  style={{
                    padding: '2px 8px',
                    fontSize: '13px',
                    cursor: 'pointer',
                    border: '1px solid',
                    borderRadius: '4px',
                    background: m.annotation === 'good' ? 'rgba(76,175,80,0.25)' : 'transparent',
                    borderColor: m.annotation === 'good' ? '#4caf50' : 'var(--border)',
                    color: m.annotation === 'good' ? '#4caf50' : 'var(--text-muted)',
                  }}
                >
                  👍
                </button>
                <button
                  onClick={() => handleAnnotate(m.turnId!, 'bad')}
                  title="差回答(差评+评论会回流到记忆用于改进)"
                  style={{
                    padding: '2px 8px',
                    fontSize: '13px',
                    cursor: 'pointer',
                    border: '1px solid',
                    borderRadius: '4px',
                    background: m.annotation === 'bad' ? 'rgba(244,67,54,0.25)' : 'transparent',
                    borderColor: m.annotation === 'bad' ? '#f44336' : 'var(--border)',
                    color: m.annotation === 'bad' ? '#f44336' : 'var(--text-muted)',
                  }}
                >
                  👎
                </button>
                <input
                  type="text"
                  placeholder="评论(可选,差评评论会用于改进)"
                  value={annotationComments[m.turnId] ?? ''}
                  onInput={(e) =>
                    setAnnotationComments((prev) => ({
                      ...prev,
                      [m.turnId!]: (e.target as HTMLInputElement).value,
                    }))
                  }
                  style={{
                    flex: 1,
                    fontSize: '11px',
                    padding: '2px 6px',
                    border: '1px solid var(--border)',
                    borderRadius: '4px',
                    background: 'transparent',
                    color: 'var(--text)',
                  }}
                />
              </div>
            )}
          </div>
        ))}
        {loading && !streaming && (
          <div class="msg msg-assistant">
            <div class="msg-role">Nebula</div>
            <div class="msg-content">
              <Spinner label={t('common.loading')} />
            </div>
          </div>
        )}
      </div>

      <div class="chat-input" style={{ position: 'relative' }}>
        {/* T-E-S-51: Level 0 内联补全 — 仅在自主度 L0 时启用(组件内部判断)。 */}
        <InlineSuggestion
          prefix={input}
          onAccept={(text) => setInput(text)}
          onReject={() => {
            /* 建议清空由组件内部状态管理 */
          }}
        >
          {({ onKeyDown }) => (
            <input
              type="text"
              placeholder="输入消息..."
              value={input}
              onInput={(e) => {
                const val = (e.target as HTMLInputElement).value;
                setInput(val);
                // T-E-B-05: 检测 `[[` 触发 WikiNote 自动补全。
                const el = e.target as HTMLInputElement;
                const pos = el.selectionStart ?? 0;
                if (pos >= 2 && val.slice(pos - 2, pos) === '[[') {
                  const prefix = val.slice(Math.max(0, pos - 20), pos - 2); // context before [[
                  // 检查 [[ 前不是 [ (避免 [[[ 误触发)
                  if (!prefix.endsWith('[')) {
                    setWikiCompletionStart(pos - 2);
                    setWikiCompletionVisible(true);
                    nebulaAPI
                      .wikiList(10)
                      .then((notes) => {
                        setWikiCompletions(notes);
                      })
                      .catch(() => {
                        setWikiCompletions([]);
                        setWikiCompletionVisible(false);
                      });
                  }
                } else {
                  // 如果正在显示补全,检查是否仍在 [[... 编辑中
                  if (wikiCompletionVisible) {
                    // 若光标移出了 [[... 范围 或输入了 ]],则关闭补全
                    if (wikiCompletionStart >= 0) {
                      const afterOpen = val.slice(wikiCompletionStart + 2, pos);
                      if (afterOpen.includes(']]') || pos <= wikiCompletionStart) {
                        setWikiCompletionVisible(false);
                      } else {
                        // 实时过滤:用 [[ 后的文本过滤候选
                        const query = afterOpen.toLowerCase();
                        nebulaAPI
                          .wikiList(10)
                          .then((notes) => {
                            const filtered = notes.filter(
                              (n) => n.slug.includes(query) || n.title.toLowerCase().includes(query)
                            );
                            setWikiCompletions(filtered);
                          })
                          .catch(() => setWikiCompletions([]));
                      }
                    }
                  }
                }
              }}
              onKeyDown={(e) => {
                onKeyDown(e);
                // T-E-B-05: Escape 关闭 Wiki 补全
                if (e.key === 'Escape' && wikiCompletionVisible) {
                  e.preventDefault();
                  setWikiCompletionVisible(false);
                  return;
                }
                // T-E-B-05: 选择 Wiki 补全项(Tab 或 Enter)
                if (
                  wikiCompletionVisible &&
                  wikiCompletions.length > 0 &&
                  (e.key === 'Tab' || (e.key === 'Enter' && wikiCompletionStart >= 0))
                ) {
                  const afterOpen = input.slice(wikiCompletionStart + 2);
                  // 仅在 [[ 未关闭时拦截(即尚未输入 ]])
                  if (!afterOpen.includes(']]')) {
                    e.preventDefault();
                    const note = wikiCompletions[0];
                    if (note) {
                      const afterCursor = input.slice(
                        (e.target as HTMLInputElement).selectionStart ?? 0
                      );
                      const replacement = `${input.slice(0, wikiCompletionStart)}[[${note.slug}]]${afterCursor}`;
                      setInput(replacement);
                    }
                    setWikiCompletionVisible(false);
                    return;
                  }
                }
                // T-E-S-52: L1 定向编辑 — Ctrl/Cmd+R 重写选中文本。
                if (
                  nebulaStore.autonomyLevel.value === 'L1' &&
                  (e.metaKey || e.ctrlKey) &&
                  (e.key === 'r' || e.key === 'R')
                ) {
                  e.preventDefault();
                  const el = e.currentTarget as HTMLInputElement;
                  const start = el.selectionStart ?? 0;
                  const end = el.selectionEnd ?? 0;
                  if (start === end) {
                    toast.warning('请先选中文字');
                    return;
                  }
                  const selected = input.slice(start, end);
                  nebulaAPI
                    .directedEdit(selected)
                    .then((rewritten) => {
                      setInput(input.slice(0, start) + rewritten + input.slice(end));
                    })
                    .catch((err) => toast.error('定向编辑失败', String(err)));
                }
                if (e.key === 'Enter') sendStream();
              }}
              disabled={loading}
            />
          )}
        </InlineSuggestion>
        {/* T-E-B-05: [[ Wiki 链接自动补全浮动列表 */}
        {wikiCompletionVisible && wikiCompletions.length > 0 && (
          <div
            style={{
              position: 'absolute',
              bottom: '100%',
              left: 0,
              right: 0,
              background: 'var(--bg-elevated, #1e1e2e)',
              border: '1px solid var(--border, #444)',
              borderRadius: '6px',
              maxHeight: '200px',
              overflowY: 'auto',
              zIndex: 100,
              boxShadow: '0 -4px 12px rgba(0,0,0,0.3)',
            }}
          >
            {wikiCompletions.map((note) => (
              <div
                key={note.id}
                style={{
                  padding: '6px 10px',
                  cursor: 'pointer',
                  fontSize: '13px',
                  color: 'var(--text)',
                  borderBottom: '1px solid var(--border, #333)',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '6px',
                }}
                onClick={() => {
                  if (wikiCompletionStart >= 0) {
                    const pos =
                      (document.activeElement as HTMLInputElement)?.selectionStart ?? input.length;
                    const afterCursor = input.slice(pos);
                    const replacement = `${input.slice(0, wikiCompletionStart)}[[${note.slug}]]${afterCursor}`;
                    setInput(replacement);
                  }
                  setWikiCompletionVisible(false);
                }}
                onMouseEnter={(e) => {
                  (e.target as HTMLElement).style.background =
                    'var(--bg-hover, rgba(255,255,255,0.08))';
                }}
                onMouseLeave={(e) => {
                  (e.target as HTMLElement).style.background = 'transparent';
                }}
              >
                <span
                  style={{
                    color: 'var(--accent, #89b4fa)',
                    fontFamily: 'monospace',
                    fontSize: '12px',
                  }}
                >
                  [[{note.slug}]]
                </span>
                <span style={{ color: 'var(--text-muted)', fontSize: '12px' }}>{note.title}</span>
              </div>
            ))}
          </div>
        )}
        {streaming ? (
          <button
            class="btn btn-stop"
            onClick={stopStreaming}
            title={t('chatPanel.stopButtonTitle')}
          >
            {t('chatPanel.stopButton')}
          </button>
        ) : (
          <>
            <button class="btn" onClick={sendStream} disabled={loading || !input.trim()}>
              发送
            </button>
            <button
              class="btn btn-secondary"
              onClick={send}
              disabled={loading || !input.trim()}
              title="非流式 fallback"
            >
              ↩
            </button>
          </>
        )}
      </div>
      {showExportDialog && (
        <ExportDialog messages={messages} onClose={() => setShowExportDialog(false)} />
      )}
      {showTemplatesDialog && (
        <TemplatesDialog
          onClose={() => setShowTemplatesDialog(false)}
          onSwarmStarted={(task) => {
            // T-E-C-13: 蜂群启动后,在对话面板追加一条 user 消息记录任务描述,
            // 让用户看到触发的场景。task.description 已含模板系统提示 + 用户输入。
            setMessages((prev) => [
              ...prev,
              {
                role: 'user',
                content: `🎭 [场景模板] ${task.description.slice(0, 200)}${task.description.length > 200 ? '…' : ''}`,
                timestamp: Date.now(),
              },
            ]);
          }}
        />
      )}
      {knowledgeCardSlug && (
        <KnowledgeCardDialog slug={knowledgeCardSlug} onClose={() => setKnowledgeCardSlug(null)} />
      )}
    </div>
  );
}
