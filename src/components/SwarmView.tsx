/**
 * 蜂群视图 - 实时显示多 Agent 协作进度
 *
 * v1.0.1 (P0#08):
 *  - Each agent card is now an expandable <details> block.  When
 *    the agent's row has status 'failed' or 'error', the summary
 *    shows a "View error" affordance and the body shows the error
 *    message, the last 20 lines of stdout / stderr, and the
 *    elapsed_ms.
 *  - Failed cards get a "Retry this agent" button that re-runs the
 *    swarm scoped to that one agent via
 *    `NineSnakeStore.runSwarmSingle(description, agent)`.
 */
import { useState } from 'preact/hooks';
import { NineSnakeStore } from '../stores/nineSnakeStore';
import type { SwarmAgentResult } from '../lib/tauri';

const AGENT_META: Record<string, { icon: string; color: string; desc: string }> = {
  coder: { icon: '💻', color: 'var(--accent-purple)', desc: '写代码' },
  writer: { icon: '✍️', color: 'var(--accent-neon)', desc: '写文档' },
  reviewer: { icon: '🔍', color: 'var(--accent-warning)', desc: '审代码' },
};

const TAIL_LINES = 20;

function tail(text: string | null | undefined, n: number): string {
  if (!text) return '';
  const lines = text.split(/\r?\n/);
  if (lines.length <= n) return text;
  return lines.slice(-n).join('\n');
}

function isFailure(row: SwarmAgentResult): boolean {
  return row.status === 'failed' || row.status === 'error';
}

export function SwarmView() {
  const [task, setTask] = useState('');
  const [agents, setAgents] = useState(['coder', 'writer', 'reviewer']);
  const taskState = NineSnakeStore.currentTask.value;
  const outputs = NineSnakeStore.swarmOutputs.value;
  const running = taskState?.status === 'running';

  async function run() {
    if (!task.trim()) return;
    await NineSnakeStore.runSwarm(task, agents);
  }

  function toggleAgent(name: string) {
    setAgents((a) => (a.includes(name) ? a.filter((x) => x !== name) : [...a, name]));
  }

  /** P0#08: re-run the swarm with only the named agent. */
  async function retrySingle(agent: string) {
    if (!task.trim()) return;
    await NineSnakeStore.runSwarmSingle(task, agent);
  }

  return (
    <div class="panel">
      <div class="panel-header">
        <span class="panel-title">🐝 蜂群</span>
        <span style="color: var(--text-muted); font-size: 12px;">
          ≥ 2 Agent 协同（v0.1 强制三件套起步）
        </span>
      </div>

      <div class="swarm-config card">
        <label style="display: block; margin-bottom: 8px; color: var(--text-secondary);">
          任务描述
        </label>
        <textarea
          rows={3}
          style="width: 100%; margin-bottom: 12px; font-family: inherit;"
          placeholder="例如：写一个 Rust 函数，验证一个字符串是否是回文"
          value={task}
          onInput={(e) => setTask((e.target as HTMLTextAreaElement).value)}
          disabled={running}
        />

        <label style="display: block; margin-bottom: 8px; color: var(--text-secondary);">
          参与 Agent（必须 ≥ 2）
        </label>
        <div style="display: flex; gap: 8px; margin-bottom: 12px;">
          {Object.entries(AGENT_META).map(([name, meta]) => (
            <button
              key={name}
              class={`agent-chip ${agents.includes(name) ? 'active' : ''}`}
              onClick={() => toggleAgent(name)}
              disabled={running}
              style={{
                padding: '8px 16px',
                borderRadius: 20,
                background: agents.includes(name) ? meta.color : 'var(--bg-tertiary)',
                color: agents.includes(name) ? 'var(--bg-primary)' : 'var(--text-secondary)',
                fontWeight: 500,
              }}
            >
              {meta.icon} {name}
            </button>
          ))}
        </div>

        <button class="btn btn-neon" onClick={run} disabled={running || !task.trim() || agents.length < 2}>
          {running ? '运行中…' : '🐝 启动蜂群'}
        </button>
      </div>

      {taskState && (
        <div class="card" style="margin-top: 16px;">
          <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 12px;">
            <span>任务 ID：{taskState.id}</span>
            <span
              class="badge"
              style={{
                background:
                  taskState.status === 'success'
                    ? '#1e5f3a'
                    : taskState.status === 'failed' || taskState.status === 'error'
                    ? '#5f1e1e'
                    : '#3a3a5f',
                color: 'var(--text-primary)',
              }}
            >
              {taskState.status}
            </span>
          </div>

          {outputs.length > 0 && (
            <div class="swarm-outputs">
              {outputs.map((o, i) => {
                const meta = AGENT_META[o.agent] || { icon: '🐍', color: 'var(--text-muted)', desc: '' };
                const failed = isFailure(o);
                return (
                  <details
                    key={i}
                    class={`card swarm-output ${failed ? 'swarm-output--failed' : ''}`}
                    style={`margin-bottom: 8px; border-left: 3px solid ${meta.color};`}
                    data-agent={o.agent}
                    data-testid={`swarm-output-${o.agent}`}
                  >
                    <summary class="swarm-output__summary">
                      <span style="font-size: 20px;">{meta.icon}</span>
                      <strong>{o.agent}</strong>
                      <span style="color: var(--text-muted); font-size: 12px;">{meta.desc}</span>
                      {failed && (
                        <span
                          class="badge"
                          data-testid={`swarm-output-${o.agent}-status`}
                          style="background: #5f1e1e; color: var(--text-primary); margin-left: auto;"
                        >
                          {o.status ?? 'failed'}
                        </span>
                      )}
                      {!failed && (
                        <span
                          class="badge"
                          style="background: #1e5f3a; color: var(--text-primary); margin-left: auto;"
                        >
                          {o.status ?? 'ok'}
                        </span>
                      )}
                    </summary>
                    <pre
                      style="white-space: pre-wrap; font-family: 'Menlo', 'Consolas', monospace; font-size: 12px; margin-top: 8px;"
                      data-testid={`swarm-output-${o.agent}-content`}
                    >
                      {o.content}
                    </pre>
                    {failed && (
                      <div class="swarm-output__failure" data-testid={`swarm-output-${o.agent}-failure`}>
                        {o.error && (
                          <div class="swarm-output__error">
                            <strong>error:</strong> <code>{o.error}</code>
                          </div>
                        )}
                        {o.stdout && (
                          <details>
                            <summary>stdout (last {TAIL_LINES} lines)</summary>
                            <pre data-testid={`swarm-output-${o.agent}-stdout`}>{tail(o.stdout, TAIL_LINES)}</pre>
                          </details>
                        )}
                        {o.stderr && (
                          <details open>
                            <summary>stderr (last {TAIL_LINES} lines)</summary>
                            <pre data-testid={`swarm-output-${o.agent}-stderr`}>{tail(o.stderr, TAIL_LINES)}</pre>
                          </details>
                        )}
                        {o.elapsed_ms != null && (
                          <div class="swarm-output__meta">
                            elapsed: <code>{o.elapsed_ms} ms</code>
                          </div>
                        )}
                        <button
                          type="button"
                          class="btn"
                          style="margin-top: 8px;"
                          data-testid={`swarm-output-${o.agent}-retry`}
                          disabled={running || !task.trim()}
                          onClick={() => retrySingle(o.agent)}
                        >
                          ↻ 重试此 agent
                        </button>
                      </div>
                    )}
                  </details>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
