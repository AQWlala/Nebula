/**
 * T-S5-B-01: 浮动窗 / 画中画聊天组件。
 *
 * 这是一个精简版的 ChatPanel,运行在独立的无边框窗口中:
 *  - 复用 ChatPanel 的核心流式聊天逻辑 (send/stop)
 *  - UI 更紧凑,半透明深色背景 + 圆角,PIP 风格
 *  - 顶部标题栏 "🐍 Nebula · 浮动" + 关闭按钮 (调用 Tauri window.close())
 *  - 内嵌 OllamaStatusBanner
 *  - 不需要 modeRouter / nebulaStore.bootstrap() — 浮动窗只做轻量聊天
 *  - 直接用 nebulaAPI.chatStream,失败时显示错误消息
 *
 * 容错:若 Tauri API 不可用 (浏览器预览),关闭按钮调用 window.close()。
 */
import { useEffect, useState } from 'preact/hooks';
import { nebulaAPI, type StreamToken } from '../lib/tauri';
import { nebulaStore } from '../stores/nebulaStore';
import { OllamaStatusBanner } from './OllamaStatusBanner';
import { t } from '../i18n';

interface Message {
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
}

/** 关闭当前浮动窗口。Tauri 不可用时回退到 window.close()。 */
async function closeFloatingWindow() {
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().close();
  } catch {
    // Tauri 运行时不可用 (浏览器预览):回退到浏览器原生关闭。
    window.close();
  }
}

export function FloatingChat() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [streamController, setStreamController] = useState<AbortController | null>(null);

  useEffect(() => {
    // 浮动窗背景透明 — 让窗口的 transparent:true 生效,实现圆角 + 半透明。
    // global.css 默认给 html/body/#app 设置了不透明背景,这里覆盖。
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';

    // 挂载时触发一次 Ollama 健康检查,让 OllamaStatusBanner 能正确显示。
    // 注意:这里只调用 checkOllama(),不调用 bootstrap() — 浮动窗只做轻量聊天。
    void nebulaStore.checkOllama().catch(() => {
      /* 忽略:浮动窗允许在 Ollama 离线时静默启动 */
    });
  }, []);

  /** 中止正在进行的流式生成,保留已累积内容并追加停止标记。 */
  function stopStreaming() {
    if (streamController) {
      streamController.abort();
      setStreamController(null);
    }
    setStreaming(false);
    setMessages((prev) => {
      if (prev.length === 0) return prev;
      const last = prev[prev.length - 1];
      if (last.role === 'assistant' && last.timestamp === -1) {
        const updated = [...prev];
        updated[updated.length - 1] = {
          role: 'assistant',
          content: last.content + t('floatingChat.streamStopped'),
          timestamp: Date.now(),
        };
        return updated;
      }
      return prev;
    });
  }

  /** 流式发送 — 使用 Tauri ipc::Channel 回调逐字渲染。 */
  async function sendStream() {
    if (!input.trim() || streaming) return;
    const userMsg: Message = { role: 'user', content: input, timestamp: Date.now() };
    setMessages((m) => [...m, userMsg]);
    const text = userMsg.content;
    setInput('');
    setStreaming(true);

    // 插入占位 assistant 消息,timestamp=-1 标识"流式进行中"
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
          setMessages((prev) => {
            const updated = [...prev];
            if (
              updated.length > 0 &&
              updated[updated.length - 1].role === 'assistant' &&
              updated[updated.length - 1].timestamp === -1
            ) {
              updated[updated.length - 1] = {
                role: 'assistant',
                content: accumulated,
                timestamp: -1,
              };
            }
            return updated;
          });
        },
        controller.signal
      );

      // 流结束:用 ChatComplete.content 做最终同步
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
            };
          }
          return updated;
        });
      }
    } catch (e) {
      // abort 不算错误 (由 stopStreaming 触发)
      if (controller.signal.aborted) {
        // stopStreaming 已处理 UI,这里不重复处理
      } else if (!accumulated) {
        setMessages((prev) => {
          const filtered = prev.filter(
            (m) => !(m.role === 'assistant' && m.timestamp === -1 && m.content === '')
          );
          return [
            ...filtered,
            {
              role: 'assistant',
              content: t('floatingChat.streamFailed', { error: String(e) }),
              timestamp: Date.now(),
            },
          ];
        });
      } else {
        // 已有累积内容,保留并标记失败
        setMessages((prev) => {
          const updated = [...prev];
          if (
            updated.length > 0 &&
            updated[updated.length - 1].role === 'assistant' &&
            updated[updated.length - 1].timestamp === -1
          ) {
            updated[updated.length - 1] = {
              role: 'assistant',
              content: accumulated + t('floatingChat.streamInterrupted'),
              timestamp: Date.now(),
            };
          }
          return updated;
        });
      }
    } finally {
      setStreamController(null);
      setStreaming(false);
    }
  }

  return (
    <div class="floating-chat">
      <header class="floating-chat__titlebar" data-tauri-drag-region>
        <span class="floating-chat__title">{t('floatingChat.title')}</span>
        <button
          class="floating-chat__close"
          title={t('floatingChat.close')}
          onClick={() => void closeFloatingWindow()}
        >
          ✕
        </button>
      </header>

      <OllamaStatusBanner />

      <div class="floating-chat__messages">
        {messages.length === 0 && (
          <div class="floating-chat__empty">
            <div class="floating-chat__empty-icon">🐍</div>
            <div>{t('floatingChat.ready')}</div>
          </div>
        )}
        {messages.map((m, i) => (
          <div
            key={`${m.role}-${i}-${m.content.slice(0, 20)}`}
            class={`floating-msg floating-msg-${m.role}`}
          >
            <div class="floating-msg__role">
              {m.role === 'user' ? t('floatingChat.you') : t('floatingChat.assistant')}
            </div>
            <div class="floating-msg__content">{m.content}</div>
          </div>
        ))}
        {streaming && messages[messages.length - 1]?.content === '' && (
          <div class="floating-msg floating-msg-assistant">
            <div class="floating-msg__role">{t('floatingChat.assistant')}</div>
            <div class="floating-msg__content">
              <span class="typing">{t('floatingChat.thinking')}</span>
            </div>
          </div>
        )}
      </div>

      <div class="floating-chat__input">
        <input
          type="text"
          placeholder={t('floatingChat.inputPlaceholder')}
          value={input}
          onInput={(e) => setInput((e.target as HTMLInputElement).value)}
          onKeyDown={(e) => e.key === 'Enter' && sendStream()}
          disabled={streaming}
        />
        {streaming ? (
          <button
            class="floating-chat__btn floating-chat__btn-stop"
            onClick={stopStreaming}
            title={t('floatingChat.stop')}
          >
            ⏹
          </button>
        ) : (
          <button
            class="floating-chat__btn"
            onClick={sendStream}
            disabled={!input.trim()}
            title={t('floatingChat.send')}
          >
            ➤
          </button>
        )}
      </div>
    </div>
  );
}
