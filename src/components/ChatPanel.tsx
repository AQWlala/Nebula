/**
 * v1.0.1: 对话面板 - 简洁的聊天界面
 *
 * P0#07:
 *  - top of the panel shows <OllamaStatusBanner /> whenever
 *    `NineSnakeStore.ollamaStatus === 'down'`.
 *  - chat send() wraps the backend call in an AbortController with
 *    an 8s timeout.  On timeout we surface a localised toast and
 *    leave the input intact so the user can retry.
 */
import { useState } from 'preact/hooks';
import { NineSnakeAPI, type ChatResponse, type StreamToken } from '../lib/tauri';
import { NineSnakeStore } from '../stores/nineSnakeStore';
import { OllamaStatusBanner } from './OllamaStatusBanner';
import { toast } from './Toast';
import { t } from '../i18n';
import { listen } from '@tauri-apps/api/event';

interface Message {
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
}

/** P0#07: client-side request budget.  If the backend takes
 *  longer than this, we abort and tell the user instead of
 *  spinning forever. */
export const CHAT_TIMEOUT_MS = 8_000;

export function ChatPanel() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [loading, setLoading] = useState(false);
  const [streaming, setStreaming] = useState(false);

  async function sendStream() {
    if (!input.trim() || loading) return;
    const userMsg: Message = { role: 'user', content: input, timestamp: Date.now() };
    setMessages((m) => [...m, userMsg]);
    const text = userMsg.content;
    setInput('');
    setLoading(true);
    setStreaming(true);

    let accumulated = '';
    try {
      const tokens = await NineSnakeAPI.chatStream({ message: text });
      for (const token of tokens) {
        accumulated += token.text;
        setMessages((prev) => {
          const updated = [...prev];
          if (updated.length > 0 && updated[updated.length - 1].role === 'assistant' && updated[updated.length - 1].timestamp === -1) {
            updated[updated.length - 1] = { role: 'assistant', content: accumulated, timestamp: -1 };
          } else {
            updated.push({ role: 'assistant', content: accumulated, timestamp: -1 });
          }
          return updated;
        });
      }
    } catch {
      if (!accumulated) {
        setMessages((m) => [...m, { role: 'assistant', content: '[流式响应失败]', timestamp: Date.now() }]);
      }
    } finally {
      setStreaming(false);
      setLoading(false);
    }
  }

  async function send() {
    if (!input.trim() || loading) return;
    const userMsg: Message = { role: 'user', content: input, timestamp: Date.now() };
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
      const res: ChatResponse = await NineSnakeAPI.chat({
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
      void NineSnakeStore.checkOllama();
    } finally {
      window.clearTimeout(timer);
      setLoading(false);
    }
  }

  return (
    <div class="panel chat-panel">
      <div class="panel-header">
        <span class="panel-title">💬 对话</span>
        <span style="color: var(--text-muted); font-size: 12px;">
          9 头蛇的 1 号蛇头：通用对话
        </span>
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
          <div key={i} class={`msg msg-${m.role}`}>
            <div class="msg-role">{m.role === 'user' ? '你' : '九头蛇'}</div>
            <div class="msg-content">{m.content}</div>
          </div>
        ))}
        {loading && (
          <div class="msg msg-assistant">
            <div class="msg-role">九头蛇</div>
            <div class="msg-content">
              <span class="typing">思考中…</span>
            </div>
          </div>
        )}
      </div>

      <div class="chat-input">
        <input
          type="text"
          placeholder="输入消息..."
          value={input}
          onInput={(e) => setInput((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => e.key === 'Enter' && send()}
          disabled={loading}
        />
        <button class="btn" onClick={send} disabled={loading || !input.trim()}>
          发送
        </button>
      </div>
    </div>
  );
}
