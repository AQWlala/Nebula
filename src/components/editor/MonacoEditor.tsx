/**
 * v0.5: Monaco Editor 集成
 *
 * 包装 @monaco-editor/react，提供：
 * - 自动按文件扩展名推断语言
 * - 暗色主题（vs-dark）
 * - 只读模式
 * - 失焦自动保存（由父组件传入 onChange）
 *
 * 在 Vite + Tauri 下，Monaco 通过 CDN 加载，避免 worker
 * 配置；如果在生产模式下发现 worker 404，可以加
 * `vite-plugin-monaco-editor`。
 */
import Editor, { type OnMount } from '@monaco-editor/react';
import { useEffect, useRef } from 'preact/hooks';

interface MonacoEditorProps {
  value: string;
  language: string;
  path: string;
  readOnly?: boolean;
  onChange?: (value: string) => void;
}

export function MonacoEditor({ value, language, path, readOnly, onChange }: MonacoEditorProps) {
  const handleMount: OnMount = (_editor, monaco) => {
    monaco.editor.defineTheme('nine-snake-dark', {
      base: 'vs-dark',
      inherit: true,
      rules: [],
      colors: {
        'editor.background': '#0d1117',
        'editor.foreground': '#c9d1d9',
        'editorCursor.foreground': '#39d98a',
        'editor.lineHighlightBackground': '#161b22',
        'editorLineNumber.foreground': '#3d4451',
        'editor.selectionBackground': '#264f78',
      },
    });
  };

  // 编辑器实例引用，留给父组件（不常用）
  const editorRef = useRef<unknown>(null);
  useEffect(() => () => { editorRef.current = null; }, []);

  return (
    <Editor
      height="100%"
      width="100%"
      path={path}
      language={language}
      value={value}
      theme="nine-snake-dark"
      onMount={handleMount}
      onChange={(v) => onChange?.(v ?? '')}
      options={{
        readOnly,
        fontSize: 13,
        minimap: { enabled: true, scale: 1 },
        scrollBeyondLastLine: false,
        automaticLayout: true,
        wordWrap: 'on',
        renderWhitespace: 'selection',
        tabSize: 2,
        fontFamily: 'Menlo, Consolas, "Courier New", monospace',
      }}
    />
  );
}

const LANG_BY_EXT: Record<string, string> = {
  rs: 'rust',
  ts: 'typescript',
  tsx: 'typescript',
  js: 'javascript',
  jsx: 'javascript',
  json: 'json',
  toml: 'ini',
  yaml: 'yaml',
  yml: 'yaml',
  md: 'markdown',
  py: 'python',
  go: 'go',
  html: 'html',
  css: 'css',
  scss: 'scss',
  sh: 'shell',
  bash: 'shell',
  sql: 'sql',
  c: 'c',
  cpp: 'cpp',
  h: 'cpp',
  java: 'java',
  kt: 'kotlin',
  swift: 'swift',
};

export function detectLanguage(path: string): string {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  return LANG_BY_EXT[ext] ?? 'plaintext';
}
