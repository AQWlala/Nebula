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
 *    `nebulaStore.runSwarmSingle(description, agent)`.
 *
 * T-E-D-10 (MVP):
 *  - 新增分栏视图模式，每个 Agent 一列
 *  - 显示工具调用列表、耗时、状态
 *  - 订阅 SwarmEvent 实时更新
 */
import { useState, useEffect } from 'preact/hooks';
import { nebulaStore } from '../stores/nebulaStore';
import { subscribeEvents, type SwarmAgentResult, type AgentToolCall } from '../lib/tauri';
import { AgentColumn } from './AgentColumn';
import { EventStreamViewer } from './EventStreamViewer';
import { MasterEventTimeline } from './MasterEventTimeline';
import { t } from '../i18n';

const AGENT_META: Record<string, { icon: string; color: string; descKey: string }> = {
  coder: { icon: '💻', color: 'var(--accent-purple)', descKey: 'swarm.coderDesc' },
  writer: { icon: '✍️', color: 'var(--accent-neon)', descKey: 'swarm.writerDesc' },
  reviewer: { icon: '🔍', color: 'var(--accent-warning)', descKey: 'swarm.reviewerDesc' },
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
  const [viewMode, setViewMode] = useState<'list' | 'columns' | 'events' | 'master'>('list');
  const [agentToolCalls, setAgentToolCalls] = useState<Record<string, AgentToolCall[]>>({});
  const [agentStatuses, setAgentStatuses] = useState<
    Record<string, 'running' | 'completed' | 'failed'>
  >({});
  const taskState = nebulaStore.currentTask.value;
  const outputs = nebulaStore.swarmOutputs.value;
  const running = taskState?.status === 'running';

  useEffect(() => {
    let unsubscribe: (() => void) | null = null;
    subscribeEvents((envelope) => {
      const event = envelope.payload;
      if (event.kind === 'agent_tool_call') {
        setAgentToolCalls((prev) => {
          const key = event.agent_id;
          const existing = prev[key] || [];
          return { ...prev, [key]: [...existing, event] };
        });
      }
      if (event.kind === 'agent_started') {
        setAgentStatuses((prev) => ({
          ...prev,
          [event.agent_kind]: 'running',
        }));
      }
      if (event.kind === 'agent_completed') {
        setAgentStatuses((prev) => ({
          ...prev,
          [event.agent_kind]: event.success ? 'completed' : 'failed',
        }));
      }
    })
      .then((fn) => {
        unsubscribe = fn;
      })
      .catch(() => {
        // 非 Tauri 环境静默忽略
      });
    return () => {
      if (unsubscribe) unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (!running) {
      setAgentToolCalls({});
      setAgentStatuses({});
    }
  }, [running]);

  async function run() {
    if (!task.trim()) return;
    setAgentToolCalls({});
    setAgentStatuses({});
    await nebulaStore.runSwarm(task, agents);
  }

  function toggleAgent(name: string) {
    setAgents((a) => (a.includes(name) ? a.filter((x) => x !== name) : [...a, name]));
  }

  /** P0#08: re-run the swarm with only the named agent. */
  async function retrySingle(agent: string) {
    if (!task.trim()) return;
    setAgentToolCalls({});
    setAgentStatuses({});
    await nebulaStore.runSwarmSingle(task, agent);
  }

  const totalToolCalls = Object.values(agentToolCalls).reduce(
    (sum, calls) => sum + calls.length,
    0
  );
  const totalToolDuration = Object.values(agentToolCalls).reduce(
    (sum, calls) => sum + calls.reduce((s, c) => s + c.duration_ms, 0),
    0
  );

  return (
    <div class="panel">
      <div class="panel-header">
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          <span class="panel-title">{t('swarm.title')}</span>
          <span style="color: var(--text-muted); font-size: 12px;">{t('swarm.subtitle')}</span>
        </div>
        <div style={{ display: 'flex', gap: 4 }}>
          {taskState && (
            <>
              <button
                onClick={() => setViewMode('list')}
                style={{
                  padding: '4px 10px',
                  fontSize: '12px',
                  cursor: 'pointer',
                  border: '1px solid var(--border)',
                  borderRadius: '4px',
                  background: viewMode === 'list' ? 'var(--accent-neon)' : 'transparent',
                  color: viewMode === 'list' ? 'var(--bg-primary)' : 'var(--text-muted)',
                  transition: 'all 0.15s',
                }}
              >
                {t('swarm.listView')}
              </button>
              <button
                onClick={() => setViewMode('columns')}
                style={{
                  padding: '4px 10px',
                  fontSize: '12px',
                  cursor: 'pointer',
                  border: '1px solid var(--border)',
                  borderRadius: '4px',
                  background: viewMode === 'columns' ? 'var(--accent-neon)' : 'transparent',
                  color: viewMode === 'columns' ? 'var(--bg-primary)' : 'var(--text-muted)',
                  transition: 'all 0.15s',
                }}
              >
                {t('swarm.columnsView')}
              </button>
              <button
                onClick={() => setViewMode('events')}
                style={{
                  padding: '4px 10px',
                  fontSize: '12px',
                  cursor: 'pointer',
                  border: '1px solid var(--border)',
                  borderRadius: '4px',
                  background: viewMode === 'events' ? 'var(--accent-neon)' : 'transparent',
                  color: viewMode === 'events' ? 'var(--bg-primary)' : 'var(--text-muted)',
                  transition: 'all 0.15s',
                }}
              >
                {t('swarm.eventsView')}
              </button>
            </>
          )}
          <button
            onClick={() => setViewMode('master')}
            title="Master 编排模式：拆解复杂任务为 DAG 并行执行"
            style={{
              padding: '4px 10px',
              fontSize: '12px',
              cursor: 'pointer',
              border: '1px solid var(--border)',
              borderRadius: '4px',
              background: viewMode === 'master' ? 'var(--accent-neon)' : 'transparent',
              color: viewMode === 'master' ? 'var(--bg-primary)' : 'var(--text-muted)',
              transition: 'all 0.15s',
            }}
          >
            🎯 Master
          </button>
        </div>
      </div>

      <div class="swarm-config card">
        <label style="display: block; margin-bottom: 8px; color: var(--text-secondary);">
          {t('swarm.taskDesc')}
        </label>
        <textarea
          rows={3}
          style="width: 100%; margin-bottom: 12px; font-family: inherit;"
          placeholder={t('swarm.taskPlaceholder')}
          value={task}
          onInput={(e) => setTask((e.target as HTMLTextAreaElement).value)}
          disabled={running}
        />

        <label style="display: block; margin-bottom: 8px; color: var(--text-secondary);">
          {t('swarm.agentsLabel')}
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

        <button
          class="btn btn-neon"
          onClick={run}
          disabled={running || !task.trim() || agents.length < 2}
        >
          {running ? t('swarm.running') : t('swarm.start')}
        </button>
      </div>

      {taskState && (
        <div class="card" style="margin-top: 16px;">
          <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 12px;">
            <span>{t('swarm.taskId', { id: taskState.id })}</span>
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

          {totalToolCalls > 0 && (
            <div
              style={{
                padding: '8px 12px',
                marginBottom: 12,
                background: 'var(--bg-tertiary)',
                borderRadius: 8,
                fontSize: 12,
                color: 'var(--text-secondary)',
                display: 'flex',
                gap: 16,
                alignItems: 'center',
              }}
            >
              <span
                dangerouslySetInnerHTML={{
                  __html: t('swarm.toolCalls', { count: totalToolCalls }),
                }}
              />
              <span
                dangerouslySetInnerHTML={{
                  __html: t('swarm.totalDuration', { duration: totalToolDuration }),
                }}
              />
            </div>
          )}

          {viewMode === 'list' && outputs.length > 0 && (
            <div class="swarm-outputs">
              {outputs.map((o) => {
                const meta = AGENT_META[o.agent] || {
                  icon: '🐍',
                  color: 'var(--text-muted)',
                  descKey: '',
                };
                const failed = isFailure(o);
                const toolCalls = agentToolCalls[o.agent] || [];
                return (
                  <details
                    key={o.agent}
                    class={`card swarm-output ${failed ? 'swarm-output--failed' : ''}`}
                    style={`margin-bottom: 8px; border-left: 3px solid ${meta.color};`}
                    data-agent={o.agent}
                    data-testid={`swarm-output-${o.agent}`}
                  >
                    <summary class="swarm-output__summary">
                      <span style="font-size: 20px;">{meta.icon}</span>
                      <strong>{o.agent}</strong>
                      <span style="color: var(--text-muted); font-size: 12px;">
                        {meta.descKey ? t(meta.descKey as any) : ''}
                      </span>
                      {toolCalls.length > 0 && (
                        <span style={{ color: 'var(--text-muted)', fontSize: 11, marginLeft: 8 }}>
                          🔧 {toolCalls.length}
                        </span>
                      )}
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
                    {toolCalls.length > 0 && (
                      <div
                        style={{
                          marginTop: 8,
                          paddingTop: 8,
                          borderTop: '1px solid var(--border-subtle)',
                        }}
                      >
                        <div style={{ fontSize: 11, color: 'var(--text-muted)', marginBottom: 6 }}>
                          {t('swarm.toolCallCount', { count: toolCalls.length })}
                        </div>
                        {toolCalls.slice(0, 5).map((tc, j) => (
                          <div
                            key={j}
                            style={{
                              display: 'flex',
                              alignItems: 'center',
                              gap: 8,
                              padding: '4px 8px',
                              fontSize: 11,
                              fontFamily: 'Menlo, Consolas, monospace',
                              background: 'var(--bg-primary)',
                              borderRadius: 4,
                              marginBottom: 4,
                            }}
                          >
                            <span
                              style={{
                                color: tc.success ? 'var(--accent-success)' : 'var(--accent-error)',
                              }}
                            >
                              {tc.success ? '✓' : '✗'}
                            </span>
                            <span style={{ flex: 1 }}>{tc.tool_name}</span>
                            <span style={{ color: 'var(--text-muted)' }}>{tc.duration_ms}ms</span>
                          </div>
                        ))}
                        {toolCalls.length > 5 && (
                          <div
                            style={{
                              fontSize: 10,
                              color: 'var(--text-muted)',
                              textAlign: 'center',
                            }}
                          >
                            {t('swarm.moreTools', { count: toolCalls.length - 5 })}
                          </div>
                        )}
                      </div>
                    )}
                    <pre
                      style="white-space: pre-wrap; font-family: 'Menlo', 'Consolas', monospace; font-size: 12px; margin-top: 8px;"
                      data-testid={`swarm-output-${o.agent}-content`}
                    >
                      {o.content}
                    </pre>
                    {failed && (
                      <div
                        class="swarm-output__failure"
                        data-testid={`swarm-output-${o.agent}-failure`}
                      >
                        {o.error && (
                          <div class="swarm-output__error">
                            <strong>error:</strong> <code>{o.error}</code>
                          </div>
                        )}
                        {o.stdout && (
                          <details>
                            <summary>stdout (last {TAIL_LINES} lines)</summary>
                            <pre data-testid={`swarm-output-${o.agent}-stdout`}>
                              {tail(o.stdout, TAIL_LINES)}
                            </pre>
                          </details>
                        )}
                        {o.stderr && (
                          <details open>
                            <summary>stderr (last {TAIL_LINES} lines)</summary>
                            <pre data-testid={`swarm-output-${o.agent}-stderr`}>
                              {tail(o.stderr, TAIL_LINES)}
                            </pre>
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
                          {t('swarm.retryAgent')}
                        </button>
                      </div>
                    )}
                  </details>
                );
              })}
            </div>
          )}

          {viewMode === 'columns' && (
            <div
              style={{
                display: 'flex',
                gap: 12,
                height: 500,
                marginTop: 12,
              }}
            >
              {agents.map((agentName) => {
                const output = outputs.find((o) => o.agent === agentName);
                const toolCalls = agentToolCalls[agentName] || [];
                const status =
                  agentStatuses[agentName] ||
                  (output ? (isFailure(output) ? 'failed' : 'completed') : 'running');
                return (
                  <AgentColumn
                    key={agentName}
                    agentId={agentName}
                    agentRole={agentName}
                    message={output?.content || ''}
                    toolCalls={toolCalls}
                    isActive={running && status === 'running'}
                    status={status}
                  />
                );
              })}
            </div>
          )}

          {viewMode === 'events' && (
            <div style={{ height: 500, marginTop: 12 }}>
              <EventStreamViewer />
            </div>
          )}
        </div>
      )}

      {viewMode === 'master' && (
        <div class="card" style="margin-top: 16px;">
          <MasterEventTimeline />
        </div>
      )}
    </div>
  );
}
