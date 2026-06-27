/**
 * v1.0.1 (P0#07): ChatPanel timeout + Ollama banner tests.
 */
import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/preact';
import { ChatPanel, CHAT_TIMEOUT_MS } from '../ChatPanel';
import { NineSnakeAPI } from '../../lib/tauri';
import { NineSnakeStore } from '../../stores/nineSnakeStore';
import { setLocale } from '../../i18n';
import { toasts } from '../Toast';

beforeEach(() => {
  cleanup();
  localStorage.clear();
  setLocale('en-US');
  NineSnakeStore.ollamaStatus.value = 'unknown';
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
    const slowChat = vi
      .spyOn(NineSnakeAPI, 'chat')
      .mockImplementation((req: ChatArgs) => {
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
        }) as unknown as ReturnType<typeof NineSnakeAPI.chat>;
      });

    const { getByPlaceholderText, getByText } = render(<ChatPanel />);
    const input = getByPlaceholderText('输入消息...') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'hi' } });
    fireEvent.click(getByText('发送'));

    // Wait a hair past the 8s budget so the abort lands.
    await new Promise((r) => setTimeout(r, CHAT_TIMEOUT_MS + 1_000));

    // The localised timeout toast must be present.
    const titles = toasts.value.map((t0) => t0.title);
    expect(titles).toContain('Request timed out');
    expect(slowChat).toHaveBeenCalled();
  }, 20_000);

  it('renders the Ollama banner when status is down', () => {
    NineSnakeStore.ollamaStatus.value = 'down';
    const { getByTestId } = render(<ChatPanel />);
    expect(getByTestId('ollama-banner')).toBeTruthy();
    expect(getByTestId('ollama-banner-retry')).toBeTruthy();
  });

  it('hides the Ollama banner when status is ok', () => {
    NineSnakeStore.ollamaStatus.value = 'ok';
    const { queryByTestId } = render(<ChatPanel />);
    expect(queryByTestId('ollama-banner')).toBeNull();
  });
});
