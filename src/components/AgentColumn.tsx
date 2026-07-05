import { useState } from 'preact/hooks';
import { ToolCallCard } from './ToolCallCard';
import type { AgentToolCall } from '../lib/tauri';
import { t } from '../i18n';

interface AgentColumnProps {
  agentId: string;
  agentRole: string;
  message: string;
  toolCalls: AgentToolCall[];
  isActive: boolean;
  status?: 'running' | 'completed' | 'failed';
}

const AGENT_META: Record<string, { icon: string; color: string }> = {
  generic: { icon: '🐍', color: 'var(--accent-purple)' },
  coder: { icon: '💻', color: 'var(--accent-purple)' },
  writer: { icon: '✍️', color: 'var(--accent-neon)' },
  reviewer: { icon: '🔍', color: 'var(--accent-warning)' },
  planner: { icon: '📋', color: 'var(--accent-blue)' },
  researcher: { icon: '🔬', color: 'var(--accent-cyan)' },
};

export function AgentColumn({
  agentId,
  agentRole,
  message,
  toolCalls,
  isActive,
  status = 'running',
}: AgentColumnProps) {
  const [showToolCalls, setShowToolCalls] = useState(true);
  const meta = AGENT_META[agentRole.toLowerCase()] || {
    icon: '🤖',
    color: 'var(--text-muted)',
  };

  const totalToolDuration = toolCalls.reduce((sum, tc) => sum + tc.duration_ms, 0);
  const successCount = toolCalls.filter((tc) => tc.success).length;
  const failCount = toolCalls.length - successCount;

  return (
    <div
      class="agent-column"
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        minWidth: 0,
        border: isActive ? `2px solid ${meta.color}` : '1px solid var(--border-subtle)',
        borderRadius: 12,
        overflow: 'hidden',
        background: 'var(--bg-secondary)',
        transition: 'all 0.2s ease',
      }}
    >
      <div
        style={{
          padding: '12px 16px',
          background: 'var(--bg-tertiary)',
          borderBottom: '1px solid var(--border-subtle)',
          display: 'flex',
          alignItems: 'center',
          gap: 10,
        }}
      >
        <span style={{ fontSize: 24 }}>{meta.icon}</span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontWeight: 600,
              fontSize: 14,
              color: 'var(--text-primary)',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
            }}
          >
            {agentRole}
            <span
              style={{
                fontSize: 11,
                color: 'var(--text-muted)',
                fontFamily: 'Menlo, Consolas, monospace',
              }}
            >
              {agentId}
            </span>
            {status === 'running' && (
              <span
                style={{
                  fontSize: 10,
                  padding: '2px 6px',
                  borderRadius: 10,
                  background: 'var(--accent-warning)',
                  color: 'var(--bg-primary)',
                  fontWeight: 500,
                }}
              >
                {t('agentColumn.statusRunning')}
              </span>
            )}
            {status === 'completed' && (
              <span
                style={{
                  fontSize: 10,
                  padding: '2px 6px',
                  borderRadius: 10,
                  background: 'var(--accent-success)',
                  color: 'var(--bg-primary)',
                  fontWeight: 500,
                }}
              >
                {t('agentColumn.statusCompleted')}
              </span>
            )}
            {status === 'failed' && (
              <span
                style={{
                  fontSize: 10,
                  padding: '2px 6px',
                  borderRadius: 10,
                  background: 'var(--accent-error)',
                  color: 'var(--bg-primary)',
                  fontWeight: 500,
                }}
              >
                {t('agentColumn.statusFailed')}
              </span>
            )}
          </div>
        </div>
      </div>

      {toolCalls.length > 0 && (
        <div
          style={{
            padding: '8px 16px',
            background: 'var(--bg-secondary)',
            borderBottom: '1px solid var(--border-subtle)',
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            fontSize: 11,
            color: 'var(--text-muted)',
            cursor: 'pointer',
            userSelect: 'none',
          }}
          onClick={() => setShowToolCalls(!showToolCalls)}
        >
          <span>
            {t('agentColumn.toolCalls', { count: toolCalls.length })}
          </span>
          <span style={{ color: 'var(--accent-success)' }}>
            ✓ {successCount}
          </span>
          <span style={{ color: 'var(--accent-error)' }}>
            ✗ {failCount}
          </span>
          <span style={{ marginLeft: 'auto' }}>
            {t('agentColumn.totalDuration', { duration: totalToolDuration })}
          </span>
          <span>{showToolCalls ? '▲' : '▼'}</span>
        </div>
      )}

      {showToolCalls && toolCalls.length > 0 && (
        <div
          style={{
            padding: '8px 12px',
            background: 'var(--bg-primary)',
            borderBottom: '1px solid var(--border-subtle)',
            maxHeight: 200,
            overflowY: 'auto',
          }}
        >
          {toolCalls.map((tc) => (
            <ToolCallCard key={`${tc.task_id}-${tc.tool_name}-${tc.start_ts}`} toolCall={tc} />
          ))}
        </div>
      )}

      <div
        style={{
          flex: 1,
          padding: '12px 16px',
          overflowY: 'auto',
          minHeight: 100,
        }}
      >
        {message ? (
          <pre
            style={{
              margin: 0,
              whiteSpace: 'pre-wrap',
              fontFamily: 'inherit',
              fontSize: 13,
              lineHeight: 1.6,
              color: 'var(--text-primary)',
            }}
          >
            {message}
          </pre>
        ) : (
          <div
            style={{
              color: 'var(--text-muted)',
              fontSize: 13,
              fontStyle: 'italic',
            }}
          >
            {t('agentColumn.thinking')}
          </div>
        )}
      </div>
    </div>
  );
}
