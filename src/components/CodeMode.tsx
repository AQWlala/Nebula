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
import { useEffect, useRef, useState } from 'preact/hooks';
import * as monaco from 'monaco-editor';
import { DiffEditor } from '@monaco-editor/react';
import { nebulaAPI, type FileEntry, type GitStatus, type GitLogEntry } from '../lib/tauri';
import { nebulaStore } from '../stores/nebulaStore';
import { MonacoEditor, detectLanguage } from './editor/MonacoEditor';
import { FileTree, refreshFileTree } from './editor/FileTree';
import { Terminal } from './editor/Terminal';
import { toast } from './Toast';
import { Spinner } from './Spinner';
import { t } from '../i18n';

export function CodeMode() {
  const [workspace, setWorkspace] = useState<string>('');
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [openPath, setOpenPath] = useState<string | null>(null);
  const [openContent, setOpenContent] = useState<string>('');
  const [dirty, setDirty] = useState(false);
  // T-E-S-52: Monaco editor 实例(L1 定向编辑用)。
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const [gitStatus, setGitStatus] = useState<GitStatus | null>(null);
  const [gitLog, setGitLog] = useState<GitLogEntry[]>([]);
  const [diff, setDiff] = useState<string>('');
  const [showGit, setShowGit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // P1-5: Agent 文件修改 Diff 预览状态
  const [agentDiff, setAgentDiff] = useState<{
    original: string;
    modified: string;
    path: string;
  } | null>(null);
  const [showAgentDiff, setShowAgentDiff] = useState(false);
  const [aiPrompt, setAiPrompt] = useState('');
  const [aiLang, setAiLang] = useState('rust');
  const [aiResult, setAiResult] = useState('');
  const [aiLoading, setAiLoading] = useState(false);

  useEffect(() => {
    nebulaAPI
      .editorWorkspaceRoot()
      .then(setWorkspace)
      .catch((e) => setError(String(e)));
    refreshFileTree(setEntries);
    refreshGit();
  }, []);

  const refreshGit = async () => {
    try {
      const [s, l] = await Promise.all([nebulaAPI.gitStatus(), nebulaAPI.gitLog(10)]);
      setGitStatus(s);
      setGitLog(l);
    } catch (e) {
      // 仓库不存在时 git 命令会失败
      setGitStatus({ branch: t('codeMode.noRepo'), entries: [], clean: true });
      setGitLog([]);
    }
  };

  const onOpen = async (path: string) => {
    if (dirty && !confirm(t('codeMode.confirmDiscard'))) return;
    try {
      const c = await nebulaAPI.editorRead(path);
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
      const c = await nebulaAPI.editorWrite(openPath, openContent);
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
    const msg = prompt(t('codeMode.commitMessage'));
    if (!msg) return;
    try {
      await nebulaAPI.gitCommit(msg);
      await refreshGit();
    } catch (e) {
      setError(String(e));
    }
  };

  const onShowDiff = async () => {
    try {
      const d = await nebulaAPI.gitDiff('');
      setDiff(d.body || t('codeMode.noDiff'));
      setShowGit(true);
    } catch (e) {
      setError(String(e));
    }
  };

  // P1-5: 显示 Agent 修改的 Diff 预览
  const onShowAgentDiff = async (path: string, modifiedContent: string) => {
    try {
      const originalContent = await nebulaAPI.editorRead(path);
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
      const result = await nebulaAPI.editorWrite(agentDiff.path, agentDiff.modified);
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
      const text = await nebulaAPI.llmComplete(
        `用 ${aiLang} 实现：\n${aiPrompt}\n\n只返回代码，不要解释。`
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
      await nebulaAPI.skillCreate({
        name: `snippet-${Date.now()}`,
        description: `Code snippet in ${aiLang}`,
        code: aiResult,
        language: aiLang,
        tags: [aiLang, 'snippet'],
      });
      toast.success(t('codeMode.savedToL4'));
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
    (window as unknown as Record<string, unknown>).nebulaShowAgentDiff = onShowAgentDiff;
    return () => {
      delete (window as unknown as Record<string, unknown>).nebulaShowAgentDiff;
    };
  }, []);

  return (
    <div class="code-mode">
      <div class="code-toolbar">
        <span class="code-title">{t('codeMode.title')}</span>
        <span class="code-workspace" title={workspace}>
          {workspace}
        </span>
        {gitStatus && (
          <span class={`git-pill ${gitStatus.clean ? 'clean' : 'dirty'}`}>
            {gitStatus.branch} ·{' '}
            {gitStatus.clean ? '✓' : `${gitStatus.entries.length} ${t('codeMode.changes')}`}
          </span>
        )}
        <div class="spacer" />
        <button onClick={onShowDiff}>{t('codeMode.diff')}</button>
        <button onClick={refreshGit} title={t('codeMode.refreshGit')}>
          ↻
        </button>
        <button onClick={onCommit} disabled={!gitStatus || gitStatus.clean}>
          {t('codeMode.commit')}
        </button>
        <button onClick={() => setShowGit((v) => !v)}>
          {showGit ? t('codeMode.closeGit') : t('codeMode.git')}
        </button>
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
                <span class="editor-path">
                  {t('codeMode.diffPreview')}: {agentDiff.path}
                </span>
                <div class="spacer" />
                <button onClick={onApplyAgentDiff} class="primary">
                  {t('codeMode.applyChanges')}
                </button>
                <button onClick={onRevertAgentDiff} class="ghost">
                  {t('codeMode.revert')}
                </button>
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
                <button onClick={onSave} disabled={!dirty} class="primary">
                  {t('codeMode.save')}
                </button>
              </header>
              <div class="editor-host">
                <MonacoEditor
                  value={openContent}
                  language={detectLanguage(openPath)}
                  path={openPath}
                  onEditorMount={(editor) => {
                    editorRef.current = editor;
                    // T-E-S-52: L1 定向编辑 — Ctrl/Cmd+R 重写选中文本。
                    editor.onKeyDown((e) => {
                      if (
                        nebulaStore.autonomyLevel.value === 'L1' &&
                        (e.ctrlKey || e.metaKey) &&
                        e.keyCode === monaco.KeyCode.KeyR
                      ) {
                        e.preventDefault();
                        e.stopPropagation();
                        const sel = editor.getSelection();
                        if (!sel || sel.isEmpty()) {
                          toast.warning(t('codeMode.selectText'));
                          return;
                        }
                        const selected = editor.getModel()?.getValueInRange(sel) ?? '';
                        if (!selected) return;
                        nebulaAPI
                          .directedEdit(selected)
                          .then((rewritten) => {
                            editor.executeEdits('l1-directed-edit', [
                              {
                                range: sel,
                                text: rewritten,
                              },
                            ]);
                            editor.pushUndoStop();
                            setOpenContent(editor.getValue());
                            setDirty(true);
                          })
                          .catch((err) => toast.error(t('codeMode.directEditFailed'), String(err)));
                      }
                    });
                  }}
                  onChange={(v) => {
                    setOpenContent(v);
                    setDirty(true);
                  }}
                />
              </div>
            </>
          ) : (
            <div class="editor-empty">
              <h2>{t('codeMode.emptyTitle')}</h2>
              <p>{t('codeMode.emptySubtitle')}</p>
            </div>
          )}
        </main>

        <aside class="code-right">
          <div class="code-ai">
            <h3>{t('codeMode.aiCodeGen')}</h3>
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
              placeholder={t('codeMode.aiPlaceholder')}
              value={aiPrompt}
              onInput={(e) => setAiPrompt((e.target as HTMLTextAreaElement).value)}
            />
            <div class="row">
              <button onClick={onAiGenerate} disabled={aiLoading || !aiPrompt.trim()}>
                {aiLoading ? <Spinner size={16} showLabel={false} /> : t('codeMode.generate')}
              </button>
              {aiResult && openPath && (
                <button onClick={onApplySnippet}>{t('codeMode.appendToEditor')}</button>
              )}
              {aiResult && <button onClick={onStoreAsSkill}>{t('codeMode.saveAsSkill')}</button>}
            </div>
            {aiResult && (
              <pre class="ai-out">
                <code>{aiResult}</code>
              </pre>
            )}
          </div>
          <div class="code-terminal">
            <h3>{t('codeMode.terminal')}</h3>
            <Terminal />
          </div>
        </aside>
      </div>

      {showGit && (
        <div class="git-modal" onClick={() => setShowGit(false)}>
          <div class="git-card" onClick={(e) => e.stopPropagation()}>
            <header>
              <h3>{t('codeMode.gitView')}</h3>
              <button onClick={() => setShowGit(false)}>×</button>
            </header>
            <section>
              <h4>{t('codeMode.status')}</h4>
              {gitStatus &&
                (gitStatus.clean ? (
                  <p class="muted">{t('codeMode.clean')}</p>
                ) : (
                  <ul class="git-status-list">
                    {gitStatus.entries.map((e) => (
                      <li key={`${e.code}-${e.path}`}>
                        <code>{e.code}</code> {e.path}
                      </li>
                    ))}
                  </ul>
                ))}
            </section>
            <section>
              <h4>{t('codeMode.uncommittedDiff')}</h4>
              <pre class="git-diff">
                <code>{diff || t('codeMode.clickDiffHint')}</code>
              </pre>
            </section>
            <section>
              <h4>{t('codeMode.recentCommits')}</h4>
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
