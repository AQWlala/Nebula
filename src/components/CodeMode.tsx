/**
 * v0.5: Code 模式（增强）
 *
 * 三栏布局：
 * - 左：FileTree（项目文件浏览）
 * - 中：MonacoEditor（当前打开文件的编辑器，含保存按钮）
 * - 右：Terminal（xterm 集成终端）
 *
 * 顶部工具栏：Git status / refresh / 提交 / 显示 diff
 * 底部：AI 代码生成（保留 v0.1 体验）+ 存为 Skill
 *
 * P1-5: Code Diff 预览
 * - Agent 修改文件后，使用 Monaco DiffEditor 预览变更
 * - 支持"应用修改"和"撤销"操作
 */
import { useEffect, useState } from 'preact/hooks';
import Editor, { DiffEditor } from '@monaco-editor/react';
import { NineSnakeAPI, type FileEntry, type FileContent, type GitStatus, type GitLogEntry } from '../lib/tauri';
import { MonacoEditor, detectLanguage } from './editor/MonacoEditor';
import { FileTree, refreshFileTree } from './editor/FileTree';
import { Terminal } from './editor/Terminal';
import { t } from '../i18n';

export function CodeMode() {
  const [workspace, setWorkspace] = useState<string>('');
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [openPath, setOpenPath] = useState<string | null>(null);
  const [openContent, setOpenContent] = useState<string>('');
  const [dirty, setDirty] = useState(false);
  const [gitStatus, setGitStatus] = useState<GitStatus | null>(null);
  const [gitLog, setGitLog] = useState<GitLogEntry[]>([]);
  const [diff, setDiff] = useState<string>('');
  const [showGit, setShowGit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // P1-5: Agent 文件修改 Diff 预览状态
  const [agentDiff, setAgentDiff] = useState<{ original: string; modified: string; path: string } | null>(null);
  const [showAgentDiff, setShowAgentDiff] = useState(false);
  const [aiPrompt, setAiPrompt] = useState('');
  const [aiLang, setAiLang] = useState('rust');
  const [aiResult, setAiResult] = useState('');
  const [aiLoading, setAiLoading] = useState(false);

  useEffect(() => {
    NineSnakeAPI.editorWorkspaceRoot().then(setWorkspace).catch((e) => setError(String(e)));
    refreshFileTree(setEntries);
    refreshGit();
  }, []);

  const refreshGit = async () => {
    try {
      const [s, l] = await Promise.all([
        NineSnakeAPI.gitStatus(),
        NineSnakeAPI.gitLog(10),
      ]);
      setGitStatus(s);
      setGitLog(l);
    } catch (e) {
      // 仓库不存在时 git 命令会失败
      setGitStatus({ branch: '(no repo)', entries: [], clean: true });
      setGitLog([]);
    }
  };

  const onOpen = async (path: string) => {
    if (dirty && !confirm('当前文件有未保存的改动，放弃吗？')) return;
    try {
      const c = await NineSnakeAPI.editorRead(path);
      setOpenPath(c.path);
      setOpenContent(c.content);
      setDirty(false);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const onSave = async () => {
    if (!openPath) return;
    try {
      const c = await NineSnakeAPI.editorWrite(openPath, openContent);
      setOpenContent(c.content);
      setDirty(false);
      // 写文件后刷新文件树以更新 mtime
      refreshFileTree(setEntries);
    } catch (e) {
      setError(String(e));
    }
  };

  const onCommit = async () => {
    if (!gitStatus) return;
    const msg = prompt('提交信息：');
    if (!msg) return;
    try {
      await NineSnakeAPI.gitCommit(msg);
      await refreshGit();
    } catch (e) {
      setError(String(e));
    }
  };

  const onShowDiff = async () => {
    try {
      const d = await NineSnakeAPI.gitDiff('');
      setDiff(d.body || '(无差异)');
      setShowGit(true);
    } catch (e) {
      setError(String(e));
    }
  };

  // P1-5: 显示 Agent 修改的 Diff 预览
  const onShowAgentDiff = async (path: string, modifiedContent: string) => {
    try {
      const originalContent = await NineSnakeAPI.editorRead(path);
      setAgentDiff({
        original: originalContent.content,
        modified: modifiedContent,
        path,
      });
      setShowAgentDiff(true);
    } catch (e) {
      setError(String(e));
    }
  };

  // P1-5: 应用 Agent 修改
  const onApplyAgentDiff = async () => {
    if (!agentDiff) return;
    try {
      const result = await NineSnakeAPI.editorWrite(agentDiff.path, agentDiff.modified);
      setOpenContent(result.content);
      setDirty(false);
      setShowAgentDiff(false);
      setAgentDiff(null);
      refreshFileTree(setEntries);
    } catch (e) {
      setError(String(e));
    }
  };

  // P1-5: 撤销 Agent 修改
  const onRevertAgentDiff = () => {
    if (!agentDiff) return;
    setShowAgentDiff(false);
    setAgentDiff(null);
  };

  const onAiGenerate = async () => {
    if (!aiPrompt.trim()) return;
    setAiLoading(true);
    setAiResult('');
    try {
      const text = await NineSnakeAPI.llmComplete(
        `用 ${aiLang} 实现：\n${aiPrompt}\n\n只返回代码，不要解释。`,
      );
      setAiResult(text);
    } catch (e) {
      setError(String(e));
    } finally {
      setAiLoading(false);
    }
  };

  const onStoreAsSkill = async () => {
    if (!aiResult) return;
    try {
      await NineSnakeAPI.skillCreate({
        name: `snippet-${Date.now()}`,
        description: `Code snippet in ${aiLang}`,
        code: aiResult,
        language: aiLang,
        tags: [aiLang, 'snippet'],
      });
      alert('已存入 L4');
    } catch (e) {
      setError(String(e));
    }
  };

  const onApplySnippet = async () => {
    if (!openPath || !aiResult) return;
    setOpenContent((cur) => cur + '\n\n' + aiResult);
    setDirty(true);
  };

  // P1-5: 暴露 onShowAgentDiff 到 window，供外部组件（如 Agent）调用
  useEffect(() => {
    (window as unknown as Record<string, unknown>).nineSnakeShowAgentDiff = onShowAgentDiff;
    return () => {
      delete (window as unknown as Record<string, unknown>).nineSnakeShowAgentDiff;
    };
  }, []);

  return (
    <div class="code-mode">
      <div class="code-toolbar">
        <span class="code-title">💻 Code</span>
        <span class="code-workspace" title={workspace}>{workspace}</span>
        {gitStatus && (
          <span class={`git-pill ${gitStatus.clean ? 'clean' : 'dirty'}`}>
            {gitStatus.branch} · {gitStatus.clean ? '✓' : `${gitStatus.entries.length} 变更`}
          </span>
        )}
        <div class="spacer" />
        <button onClick={onShowDiff}>diff</button>
        <button onClick={refreshGit} title="刷新 Git">↻</button>
        <button onClick={onCommit} disabled={!gitStatus || gitStatus.clean}>commit</button>
        <button onClick={() => setShowGit((v) => !v)}>{showGit ? '关闭' : 'Git'}</button>
      </div>

      {error && <div class="code-error">{error}</div>}

      <div class="code-grid">
        <aside class="code-sidebar">
          <FileTree
            entries={entries}
            workspace={workspace}
            currentPath={openPath}
            onOpen={onOpen}
            onRefresh={() => refreshFileTree(setEntries)}
          />
        </aside>

        <main class="code-editor">
          {showAgentDiff && agentDiff ? (
            <>
              <header class="editor-header">
                <span class="editor-path">{t('codeMode.diffPreview')}: {agentDiff.path}</span>
                <div class="spacer" />
                <button onClick={onApplyAgentDiff} class="primary">{t('codeMode.applyChanges')}</button>
                <button onClick={onRevertAgentDiff} class="ghost">{t('codeMode.revert')}</button>
              </header>
              <div class="editor-host">
                <DiffEditor
                  original={agentDiff.original}
                  modified={agentDiff.modified}
                  language={detectLanguage(agentDiff.path)}
                  theme="vs-dark"
                  options={{
                    readOnly: true,
                    renderSideBySide: true,
                    minimap: { enabled: false },
                    scrollBeyondLastLine: false,
                    fontSize: 13,
                    fontFamily: 'Menlo, Consolas, "Courier New", monospace',
                  }}
                />
              </div>
            </>
          ) : openPath ? (
            <>
              <header class="editor-header">
                <span class="editor-path">{openPath}</span>
                {dirty && <span class="editor-dirty">●</span>}
                <div class="spacer" />
                <span class="editor-lang">{detectLanguage(openPath)}</span>
                <button onClick={onSave} disabled={!dirty} class="primary">保存</button>
              </header>
              <div class="editor-host">
                <MonacoEditor
                  value={openContent}
                  language={detectLanguage(openPath)}
                  path={openPath}
                  onChange={(v) => { setOpenContent(v); setDirty(true); }}
                />
              </div>
            </>
          ) : (
            <div class="editor-empty">
              <h2>💻 Code 模式</h2>
              <p>从左侧选择一个文件开始编辑</p>
            </div>
          )}
        </main>

        <aside class="code-right">
          <div class="code-ai">
            <h3>AI 代码生成</h3>
            <div class="row">
              <select
                value={aiLang}
                onChange={(e) => setAiLang((e.target as HTMLSelectElement).value)}
              >
                <option value="rust">Rust</option>
                <option value="python">Python</option>
                <option value="typescript">TypeScript</option>
                <option value="go">Go</option>
                <option value="javascript">JavaScript</option>
              </select>
            </div>
            <textarea
              rows={3}
              placeholder="描述要写的代码…"
              value={aiPrompt}
              onInput={(e) => setAiPrompt((e.target as HTMLTextAreaElement).value)}
            />
            <div class="row">
              <button onClick={onAiGenerate} disabled={aiLoading || !aiPrompt.trim()}>
                {aiLoading ? '生成中…' : '✨ 生成'}
              </button>
              {aiResult && openPath && (
                <button onClick={onApplySnippet}>追加到编辑器</button>
              )}
              {aiResult && <button onClick={onStoreAsSkill}>存为 Skill</button>}
            </div>
            {aiResult && (
              <pre class="ai-out"><code>{aiResult}</code></pre>
            )}
          </div>
          <div class="code-terminal">
            <h3>终端</h3>
            <Terminal />
          </div>
        </aside>
      </div>

      {showGit && (
        <div class="git-modal" onClick={() => setShowGit(false)}>
          <div class="git-card" onClick={(e) => e.stopPropagation()}>
            <header>
              <h3>Git 视图</h3>
              <button onClick={() => setShowGit(false)}>×</button>
            </header>
            <section>
              <h4>状态</h4>
              {gitStatus && (gitStatus.clean ? (
                <p class="muted">✓ 工作区干净</p>
              ) : (
                <ul class="git-status-list">
                  {gitStatus.entries.map((e, i) => (
                    <li key={i}><code>{e.code}</code> {e.path}</li>
                  ))}
                </ul>
              ))}
            </section>
            <section>
              <h4>未提交 diff</h4>
              <pre class="git-diff"><code>{diff || '(点击 diff 按钮加载)'}</code></pre>
            </section>
            <section>
              <h4>最近提交</h4>
              <ul class="git-log">
                {gitLog.map((e) => (
                  <li key={e.hash}>
                    <code>{e.short}</code> {e.subject} <span class="muted">— {e.author}</span>
                  </li>
                ))}
              </ul>
            </section>
          </div>
        </div>
      )}
    </div>
  );
}
