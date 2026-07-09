/**
 * v1.0.1 (P0#07): ChatPanel timeout + Ollama banner tests.
 */
import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/preact';
import { ChatPanel, CHAT_TIMEOUT_MS } from '../ChatPanel';
import { nebulaAPI } from '../../lib/tauri';
import { nebulaStore } from '../../stores/nebulaStore';
import { setLocale } from '../../i18n';
import { toasts } from '../Toast';

beforeEach(() => {
  cleanup();
  localStorage.clear();
  setLocale('en-US');
  nebulaStore.ollamaStatus.value = 'unknown';
  toasts.value = [];
  vi.useRealTimers();
});

afterEach(() => {
  vi.restoreAllMocks();
  cleanup();
  toasts.value = [];
});

describe('ChatPanel (P0#07)', () => {
  it('mock_command_returns_after_10s_shows_timeout_toast', async () => {
    expect(CHAT_TIMEOUT_MS).toBe(8_000);

    // The mock honours the AbortSignal the component passes in:
    // when the 8s timer fires the controller.abort() reaches us
    // and we reject with an AbortError, which the component
    // translates into a "Request timed out" toast.  We cast to
    // `any` because the underlying mock has to satisfy the
    // real `Promise<ChatResponse>` signature even though we
    // never resolve it.
    type ChatArgs = { message: string; signal?: AbortSignal };
    const slowChat = vi.spyOn(nebulaAPI, 'chat').mockImplementation((req: ChatArgs) => {
      return new Promise<unknown>((_resolve, reject) => {
        if (req?.signal) {
          if (req.signal.aborted) {
            const e = new Error('aborted');
            e.name = 'AbortError';
            reject(e);
            return;
          }
          req.signal.addEventListener('abort', () => {
            const e = new Error('aborted');
            e.name = 'AbortError';
            reject(e);
          });
        }
      }) as unknown as ReturnType<typeof nebulaAPI.chat>;
    });

    const { getByPlaceholderText, getByText } = render(<ChatPanel />);
    const input = getByPlaceholderText(/Type a message\.\.\.|输入消息\.\.\./) as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'hi' } });
    // T-S1-B-01b: "发送"按钮现在触发流式 sendStream()，超时测试
    // 需要点击非流式 fallback 按钮"↩"来测试 send() 的 8s 超时逻辑。
    fireEvent.click(getByText('↩'));

    // Wait a hair past the 8s budget so the abort lands.
    await new Promise((r) => setTimeout(r, CHAT_TIMEOUT_MS + 1_000));

    // The localised timeout toast must be present.
    const titles = toasts.value.map((t0) => t0.title);
    expect(titles).toContain('Request timed out');
    expect(slowChat).toHaveBeenCalled();
  }, 20_000);

  it('renders the Ollama banner when status is down', () => {
    nebulaStore.ollamaStatus.value = 'down';
    const { getByTestId } = render(<ChatPanel />);
    expect(getByTestId('ollama-banner')).toBeTruthy();
    expect(getByTestId('ollama-banner-retry')).toBeTruthy();
  });

  it('hides the Ollama banner when status is ok', () => {
    nebulaStore.ollamaStatus.value = 'ok';
    const { queryByTestId } = render(<ChatPanel />);
    expect(queryByTestId('ollama-banner')).toBeNull();
  });

  // T-S1-B-01b: 流式 IPC 前端 listen 测试
  it('streaming_renders_tokens_incrementally', async () => {
    // Mock chatStream: 立即调用 onToken 回调推送 3 个 token，然后 resolve
    const streamSpy = vi
      .spyOn(nebulaAPI, 'chatStream')
      .mockImplementation((_req, onToken, _signal) => {
        return new Promise((resolve) => {
          onToken({ text: 'Hello', done: false, incomplete: false });
          onToken({ text: ', ', done: false, incomplete: false });
          onToken({ text: 'world!', done: true, incomplete: false });
          resolve({ model: 'test-model', content: 'Hello, world!', role: 'assistant' });
        });
      });

    const { getByPlaceholderText, getByText, container } = render(<ChatPanel />);
    const input = getByPlaceholderText(/Type a message\.\.\.|输入消息\.\.\./) as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'hi' } });
    fireEvent.click(getByText(/^Send$|^发送$/));

    // 等待 Promise 微任务队列刷新
    await new Promise((r) => setTimeout(r, 50));

    // 验证最终渲染的 assistant 消息包含完整内容
    const msgContents = container.querySelectorAll('.msg-content');
    const lastContent = msgContents[msgContents.length - 1]?.textContent;
    expect(lastContent).toContain('Hello, world!');
    expect(streamSpy).toHaveBeenCalled();
  });
});
