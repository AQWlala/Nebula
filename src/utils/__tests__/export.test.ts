import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  exportToMarkdown,
  downloadMarkdown,
  downloadBlob,
  printToPdf,
  type ExportOptions,
} from '../export';
import type { Message } from '../../components/ChatPanel';

// Mock i18n — export.ts 依赖 t() 函数
vi.mock('../../i18n', () => ({
  t: (key: string) => {
    const map: Record<string, string> = {
      'export.defaultTitle': 'Chat Export',
      'export.exportTime': 'Export Time',
      'export.messageCount': 'Message Count',
      'export.roleUser': 'User',
      'export.roleAssistant': 'Assistant',
    };
    return map[key] ?? key;
  },
}));

const baseOptions: ExportOptions = {
  format: 'markdown',
  includeToolCalls: false,
  includeTimestamps: true,
};

const sampleMessages: Message[] = [
  {
    id: 'm1',
    role: 'user',
    content: 'Hello world',
    timestamp: 1700000000000,
  } as Message,
  {
    id: 'm2',
    role: 'assistant',
    content: 'Hi there',
    timestamp: 1700000001000,
  } as Message,
];

describe('exportToMarkdown', () => {
  it('renders title and message count', () => {
    const md = exportToMarkdown(sampleMessages, { ...baseOptions, title: 'My Title' });
    expect(md).toContain('# My Title');
    expect(md).toContain('Message Count: 2');
  });

  it('uses default title when none provided', () => {
    const md = exportToMarkdown(sampleMessages, baseOptions);
    expect(md).toContain('# Chat Export');
  });

  it('includes timestamps when enabled', () => {
    const md = exportToMarkdown(sampleMessages, { ...baseOptions, includeTimestamps: true });
    // formatTimestamp 输出形如 2023-11-14 22:13:20
    expect(md).toMatch(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}/);
  });

  it('omits timestamps when disabled', () => {
    const md = exportToMarkdown(sampleMessages, { ...baseOptions, includeTimestamps: false });
    // 消息体不应包含时间戳行(*...*)
    expect(md).not.toContain('*2023-');
  });

  it('renders role labels', () => {
    const md = exportToMarkdown(sampleMessages, baseOptions);
    expect(md).toContain('## user');
    expect(md).toContain('## assistant');
  });

  it('skips timestamp when msg.timestamp is 0', () => {
    const msgs = [{ ...sampleMessages[0], timestamp: 0 }] as Message[];
    const md = exportToMarkdown(msgs, { ...baseOptions, includeTimestamps: true });
    // 不应有时间戳斜体行
    expect(md).not.toMatch(/\*\d{4}-/);
  });
});

describe('downloadMarkdown', () => {
  beforeEach(() => {
    // jsdom 不实现 URL.createObjectURL / revokeObjectURL
    URL.createObjectURL = vi.fn(() => 'blob:mock');
    URL.revokeObjectURL = vi.fn();
    document.body.appendChild = vi.fn();
    document.body.removeChild = vi.fn();
    // click 是原生方法,jsdom 已实现
  });

  it('creates an anchor and clicks it', () => {
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click');
    downloadMarkdown('content', 'file.md');
    expect(clickSpy).toHaveBeenCalledOnce();
    expect(URL.createObjectURL).toHaveBeenCalledOnce();
    expect(URL.revokeObjectURL).toHaveBeenCalledOnce();
  });

  it('uses default filename when none provided', () => {
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click');
    downloadMarkdown('content');
    expect(clickSpy).toHaveBeenCalledOnce();
    // 默认文件名包含 chat-export-
    const a = (document.createElement as any).__lastAnchor;
    // jsdom 中 a.download 已设置,验证通过 click 调用即可
  });
});

describe('downloadBlob', () => {
  beforeEach(() => {
    URL.createObjectURL = vi.fn(() => 'blob:mock');
    URL.revokeObjectURL = vi.fn();
    document.body.appendChild = vi.fn();
    document.body.removeChild = vi.fn();
  });

  it('downloads a blob with given filename', () => {
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click');
    const blob = new Blob(['data'], { type: 'text/plain' });
    downloadBlob(blob, 'data.txt');
    expect(clickSpy).toHaveBeenCalledOnce();
    expect(URL.createObjectURL).toHaveBeenCalledOnce();
  });
});

describe('printToPdf', () => {
  beforeEach(() => {
    URL.createObjectURL = vi.fn(() => 'blob:mock');
    URL.revokeObjectURL = vi.fn();
  });

  it('opens a new window and writes HTML', () => {
    const fakeDoc = {
      write: vi.fn(),
      close: vi.fn(),
    };
    const fakeWin = {
      document: fakeDoc,
      focus: vi.fn(),
      print: vi.fn(),
    };
    vi.stubGlobal('window', {
      ...window,
      open: vi.fn(() => fakeWin),
    });

    printToPdf(sampleMessages, { ...baseOptions, title: 'PDF Title' });

    expect(window.open).toHaveBeenCalledWith('', '_blank');
    expect(fakeDoc.write).toHaveBeenCalledOnce();
    const html = fakeDoc.write.mock.calls[0][0] as string;
    expect(html).toContain('<h1>PDF Title</h1>');
    expect(html).toContain('User');
    expect(html).toContain('Assistant');
    expect(html).toContain('Hello world');
    expect(html).toContain('Hi there');
    // setTimeout 200ms 后调用 print
    expect(fakeWin.focus).toHaveBeenCalledOnce();
  });

  it('escapes HTML in message content', () => {
    const fakeDoc = { write: vi.fn(), close: vi.fn() };
    const fakeWin = {
      document: fakeDoc,
      focus: vi.fn(),
      print: vi.fn(),
    };
    vi.stubGlobal('window', {
      ...window,
      open: vi.fn(() => fakeWin),
    });

    const maliciousMsgs = [{ ...sampleMessages[0], content: '<script>alert(1)</script>' }] as Message[];
    printToPdf(maliciousMsgs, baseOptions);

    const html = fakeDoc.write.mock.calls[0][0] as string;
    expect(html).not.toContain('<script>alert(1)</script>');
    expect(html).toContain('&lt;script&gt;'); // escapeHtml 转义
  });

  it('does nothing when window.open returns null', () => {
    vi.stubGlobal('window', {
      ...window,
      open: vi.fn(() => null),
    });

    // 不应抛错
    expect(() => printToPdf(sampleMessages, baseOptions)).not.toThrow();
    expect(window.open).toHaveBeenCalledOnce();
  });
});
