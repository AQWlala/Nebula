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
 *
 * 重构（macOS 风格 agent-card）:
 *  - 页面头使用 .page-header / .page-title / .page-subtitle / .page-actions
 *  - 当前任务高亮卡 .swarm-current + .swarm-pulse + 进度条
 *  - Agent 卡片网格 .swarm-grid / .agent-card[.running|.done|.failed]
 *  - 历史任务表
 *  - 所有任务创建、状态监听、日志流、重试功能保持不变
 */
import { useState, useEffect, useRef } from 'preact/hooks';
import { nebulaStore } from '../stores/nebulaStore';
import { subscribeEvents, type SwarmAgentResult, type AgentToolCall } from '../lib/tauri';
import { AgentColumn } from './AgentColumn';
import { EventStreamViewer } from './EventStreamViewer';
import { MasterEventTimeline } from './MasterEventTimeline';
import { t } from '../i18n';

const AGENT_META: Record<string, {
  icon: string;
  color: string;
  avatarBg: string;
  descKey: string;
}> = {
  coder: { icon: '💻', color: 'var(--accent-purple)', avatarBg: 'rgba(40,200,64,0.2)', descKey: 'swarm.coderDesc' },
  writer: { icon: '✍️', color: 'var(--accent-neon)', avatarBg: 'rgba(255,159,10,0.2)', descKey: 'swarm.writerDesc' },
  reviewer: { icon: '🔍', color: 'var(--accent-warning)', avatarBg: 'rgba(167,139,246,0.2)', descKey: 'swarm.reviewerDesc' },
};

const TAIL_LINES = 20;

// agent-card 状态对应的中文标签
const STATUS_LABEL: Record<string, string> = {
  running: '运行中',
  done: '已完成',
  failed: '失败',
  queued: '排队中',
};

function tail(text: string | null | undefined, n: number): string {
  if (!text) return '';
  const lines = text.split(/\r?\n/);
  if (lines.length <= n) return text;
  return lines.slice(-n).join('\n');
}

function isFailure(row: SwarmAgentResult): boolean {
  return row.status === 'failed' || row.status === 'error';
}

/** 历史任务状态 → 中文标签 */
function historyLabel(status: string): string {
  if (status === 'success') return '完成';
  if (status === 'failed' || status === 'error') return '失败';
  if (status === 'running') return '进行中';
  return status;
}

/** 历史任务状态 badge 的配色 */
function historyBadgeColors(status: string): { background: string; color: string } {
  if (status === 'success') return { background: 'rgba(40,200,64,0.13)', color: '#28c840' };
  if (status === 'failed' || status === 'error') return { background: 'rgba(255,95,87,0.13)', color: '#ff5f57' };
  if (status === 'running') return { background: 'rgba(10,132,255,0.13)', color: '#0A84FF' };
  return { background: 'rgba(255,255,255,0.1)', color: 'rgba(255,255,255,0.4)' };
}

export function SwarmView() {
  const [task, setTask] = useState('');
  const [agents, setAgents] = useState(['coder', 'writer', 'reviewer']);
  const [viewMode, setViewMode] = useState<'list' | 'columns' | 'events' | 'master'>('list');
  const [agentToolCalls, setAgentToolCalls] = useState<Record<string, AgentToolCall[]>>({});
  const [agentStatuses, setAgentStatuses] = useState<
    Record<string, 'running' | 'completed' | 'failed'>
  >({});
  const taskRef = useRef<HTMLTextAreaElement>(null);
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

  /** "新建任务"按钮：清空并聚焦任务输入框，便于快速录入 */
  function focusNewTask() {
    setTask('');
    taskRef.current?.focus();
  }

  /** 根据 agent 名称与输出推导 agent-card 的状态（running/done/failed/queued） */
  function cardStatusFor(agent: string, output?: SwarmAgentResult): string {
    const s = agentStatuses[agent];
    if (s === 'running') return 'running';
    if (s === 'completed') return 'done';
    if (s === 'failed') return 'failed';
    if (output) return isFailure(output) ? 'failed' : 'done';
    return 'queued';
  }

  const totalToolCalls = Object.values(agentToolCalls).reduce(
    (sum, calls) => sum + calls.length,
    0
  );
  const totalToolDuration = Object.values(agentToolCalls).reduce(
    (sum, calls) => sum + calls.reduce((s, c) => s + c.duration_ms, 0),
    0
  );

  // 当前任务进度：按已结束（done/failed）的 agent 数估算
  const completedCount = agents.filter((a) => {
    const s = cardStatusFor(a, outputs.find((o) => o.agent === a));
    return s === 'done' || s === 'failed';
  }).length;
  const progressPct =
    agents.length > 0 ? Math.round((completedCount / agents.length) * 100) : 0;

  // 历史任务表数据：当前任务作为最近一条（无任务时为空）
  const historyRows = taskState
    ? [
        {
          name: task || t('swarm.taskId', { id: taskState.id }),
          time: '刚刚',
          status: taskState.status,
        },
      ]
    : [];

  return (
    <div class="panel">
      {/* 页面头：标题 + 副标题 + 工具按钮 */}
      <div class="page-header" style={{ margin: '-20px -20px 16px' }}>
        <div>
          <div class="page-title">{t('swarm.title')}</div>
          <div class="page-subtitle">{t('swarm.subtitle')}</div>
        </div>
        <div class="page-actions">
          {taskState && (
            <>
              <button
                class={`tool-btn ${viewMode === 'list' ? 'tool-btn-primary' : ''}`}
                onClick={() => setViewMode('list')}
              >
                {t('swarm.listView')}
              </button>
              <button
                class={`tool-btn ${viewMode === 'columns' ? 'tool-btn-primary' : ''}`}
                onClick={() => setViewMode('columns')}
              >
                {t('swarm.columnsView')}
              </button>
              <button
                class={`tool-btn ${viewMode === 'events' ? 'tool-btn-primary' : ''}`}
                onClick={() => setViewMode('events')}
              >
                {t('swarm.eventsView')}
              </button>
            </>
          )}
          <button
            class={`tool-btn ${viewMode === 'master' ? 'tool-btn-primary' : ''}`}
            onClick={() => setViewMode('master')}
            title={t('swarm.masterModeTitle')}
          >
            🎯 Master
          </button>
          <button
            class="tool-btn tool-btn-primary"
            onClick={focusNewTask}
            disabled={running}
          >
            + 新建任务
          </button>
        </div>
      </div>

      {/* 当前任务高亮卡：pulse 指示灯 + 任务名 + 进度条 */}
      {taskState && (
        <div class="swarm-current">
          <div class="swarm-pulse" />
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, fontSize: 13 }}>
              {task || t('swarm.taskId', { id: taskState.id })}
            </div>
            <div style={{ fontSize: 11, color: 'rgba(255,255,255,0.4)' }}>
              {agents.length} 个 Agent · 进度 {progressPct}%
            </div>
          </div>
          <div class="task-progress-bar" style={{ width: 180 }}>
            <div class="task-progress-fill" style={{ width: `${progressPct}%` }} />
          </div>
        </div>
      )}

      {/* 任务配置区（保留任务创建功能） */}
      <div class="swarm-config card">
        <label style={{ display: 'block', marginBottom: 8, color: 'var(--text-secondary)' }}>
          {t('swarm.taskDesc')}
        </label>
        <textarea
          ref={taskRef}
          rows={3}
          style="width: 100%; margin-bottom: 12px; font-family: inherit;"
          placeholder={t('swarm.taskPlaceholder')}
          value={task}
          onInput={(e) => setTask((e.target as HTMLTextAreaElement).value)}
          disabled={running}
        />

        <label style={{ display: 'block', marginBottom: 8, color: 'var(--text-secondary)' }}>
          {t('swarm.agentsLabel')}
        </label>
        <div style={{ display: 'flex', gap: 8, marginBottom: 12 }}>
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

      {/* 工具调用聚合统计（保留） */}
      {taskState && totalToolCalls > 0 && (
        <div
          style={{
            padding: '8px 12px',
            marginTop: 16,
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

      {/* Agent 卡片网格（列表视图） */}
      {viewMode === 'list' && outputs.length > 0 && (
        <>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              margin: '16px 0 10px',
            }}
          >
            <span style={{ fontSize: 13.5, fontWeight: 650 }}>Agent 实时状态</span>
          </div>
          <div class="swarm-grid" style={{ marginBottom: 22 }}>
            {agents.map((agentName) => {
              const output = outputs.find((o) => o.agent === agentName);
              const meta = AGENT_META[agentName] || {
                icon: '🐍',
                color: 'var(--text-muted)',
                avatarBg: 'rgba(255,255,255,0.06)',
                descKey: '',
              };
              const failed = output ? isFailure(output) : false;
              const toolCalls = agentToolCalls[agentName] || [];
              const cardStatus = cardStatusFor(agentName, output);
              const cardClass = `agent-card ${cardStatus === 'queued' ? '' : cardStatus}${
                failed ? ' swarm-output--failed' : ''
              }`;
              const elapsedMs =
                output?.elapsed_ms ?? toolCalls.reduce((s, c) => s + c.duration_ms, 0);
              const elapsedStr = elapsedMs
                ? elapsedMs >= 1000
                  ? `${(elapsedMs / 1000).toFixed(1)}s`
                  : `${elapsedMs}ms`
                : '—';
              const logText =
                output?.content ||
                (cardStatus === 'running'
                  ? '> 初始化中...\n> 等待 Agent 输出'
                  : '> 尚未启动');
              return (
                <div
                  key={agentName}
                  class={cardClass}
                  data-agent={agentName}
                  data-testid={`swarm-output-${agentName}`}
                  style={cardStatus === 'queued' ? { opacity: 0.6 } : undefined}
                >
                  <div class="agent-header">
                    <div class="agent-avatar" style={{ background: meta.avatarBg }}>
                      {meta.icon}
                    </div>
                    <div>
                      <div class="agent-name">
                        {agentName.charAt(0).toUpperCase() + agentName.slice(1)}
                      </div>
                      <div class="agent-role">
                        {meta.descKey ? t(meta.descKey as any) : agentName}
                      </div>
                    </div>
                    <span
                      class={`agent-status ${cardStatus === 'queued' ? 'queued' : cardStatus}`}
                    >
                      {STATUS_LABEL[cardStatus]}
                    </span>
                  </div>
                  <pre
                    class="agent-log"
                    style={{ margin: 0 }}
                    data-testid={`swarm-output-${agentName}-content`}
                  >
                    {logText}
                  </pre>
                  <div class="agent-stats">
                    <span>耗时: {elapsedStr}</span>
                    <span>Token: —</span>
                    {toolCalls.length > 0 && <span>🔧 {toolCalls.length}</span>}
                  </div>
                  {failed && (
                    <div
                      class="swarm-output__failure"
                      data-testid={`swarm-output-${agentName}-failure`}
                    >
                      {output?.error && (
                        <div class="swarm-output__error">
                          <strong>error:</strong> <code>{output.error}</code>
                        </div>
                      )}
                      {output?.stdout && (
                        <details>
                          <summary>stdout (last {TAIL_LINES} lines)</summary>
                          <pre data-testid={`swarm-output-${agentName}-stdout`}>
                            {tail(output.stdout, TAIL_LINES)}
                          </pre>
                        </details>
                      )}
                      {output?.stderr && (
                        <details open>
                          <summary>stderr (last {TAIL_LINES} lines)</summary>
                          <pre data-testid={`swarm-output-${agentName}-stderr`}>
                            {tail(output.stderr, TAIL_LINES)}
                          </pre>
                        </details>
                      )}
                      {output?.elapsed_ms != null && (
                        <div class="swarm-output__meta">
                          elapsed: <code>{output.elapsed_ms} ms</code>
                        </div>
                      )}
                      <button
                        type="button"
                        class="btn"
                        style={{ marginTop: 8 }}
                        data-testid={`swarm-output-${agentName}-retry`}
                        disabled={running || !task.trim()}
                        onClick={() => retrySingle(agentName)}
                      >
                        {t('swarm.retryAgent')}
                      </button>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </>
      )}

      {/* 分栏视图（保留） */}
      {viewMode === 'columns' && (
        <div style={{ display: 'flex', gap: 12, height: 500, marginTop: 12 }}>
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

      {/* 事件流视图（保留） */}
      {viewMode === 'events' && (
        <div style={{ height: 500, marginTop: 12 }}>
          <EventStreamViewer />
        </div>
      )}

      {/* 历史任务表：任务名 + 时间 + 状态 badge */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          margin: '22px 0 10px',
        }}
      >
        <span style={{ fontSize: 13.5, fontWeight: 650 }}>历史任务</span>
      </div>
      <div
        style={{
          background: 'rgba(255,255,255,0.025)',
          border: '1px solid rgba(255,255,255,0.05)',
          borderRadius: 10,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: '1fr 130px 90px',
            gap: 12,
            padding: '10px 16px',
            borderBottom: historyRows.length
              ? '1px solid rgba(255,255,255,0.04)'
              : 'none',
            fontSize: '10.5px',
            fontWeight: 600,
            color: 'rgba(255,255,255,0.3)',
            textTransform: 'uppercase',
            letterSpacing: '.05em',
          }}
        >
          任务 · 时间 · 状态
        </div>
        {historyRows.length === 0 ? (
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 130px 90px',
              gap: 12,
              padding: '10px 16px',
              fontSize: '12.5px',
              alignItems: 'center',
            }}
          >
            <span style={{ color: 'rgba(255,255,255,0.3)' }}>暂无历史任务</span>
            <span />
            <span />
          </div>
        ) : (
          historyRows.map((row, i) => {
            const colors = historyBadgeColors(row.status);
            return (
              <div
                key={i}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '1fr 130px 90px',
                  gap: 12,
                  padding: '10px 16px',
                  fontSize: '12.5px',
                  alignItems: 'center',
                }}
              >
                <span>{row.name}</span>
                <span style={{ color: 'rgba(255,255,255,0.3)' }}>{row.time}</span>
                <span>
                  <span
                    style={{
                      fontSize: '10.5px',
                      padding: '2px 8px',
                      borderRadius: '100px',
                      background: colors.background,
                      color: colors.color,
                    }}
                  >
                    {historyLabel(row.status)}
                  </span>
                </span>
              </div>
            );
          })
        )}
      </div>

      {/* Master 编排时间线（保留，独立卡片） */}
      {viewMode === 'master' && (
        <div class="card" style={{ marginTop: 16 }}>
          <MasterEventTimeline />
        </div>
      )}
    </div>
  );
}
