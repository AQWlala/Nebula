import { useState } from 'preact/hooks';
import type { AgentToolCall } from '../lib/tauri';

interface ToolCallCardProps {
  toolCall: AgentToolCall;
}

export function ToolCallCard({ toolCall }: ToolCallCardProps) {
  const [expanded, setExpanded] = useState(false);

  const statusIcon = toolCall.success ? '✓' : '✗';
  const statusColor = toolCall.success ? 'var(--accent-success)' : 'var(--accent-error)';

  return (
    <div
      class="tool-call-card"
      style={{
        border: `1px solid ${toolCall.success ? 'var(--border-subtle)' : 'var(--accent-error)'}`,
        borderRadius: 8,
        marginBottom: 6,
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '6px 10px',
          background: 'var(--bg-tertiary)',
          cursor: 'pointer',
          userSelect: 'none',
        }}
        onClick={() => setExpanded(!expanded)}
      >
        <span style={{ color: statusColor, fontWeight: 600, fontSize: 14 }}>
          {statusIcon}
        </span>
        <span style={{ fontFamily: 'Menlo, Consolas, monospace', fontSize: 12, flex: 1 }}>
          {toolCall.tool_name}
        </span>
        <span style={{ color: 'var(--text-muted)', fontSize: 11 }}>
          {toolCall.duration_ms}ms
        </span>
        <span style={{ color: 'var(--text-muted)', fontSize: 10 }}>
          {expanded ? '▲' : '▼'}
        </span>
      </div>
      {expanded && (
        <div
          style={{
            padding: '8px 10px',
            background: 'var(--bg-primary)',
            borderTop: '1px solid var(--border-subtle)',
          }}
        >
          {toolCall.success && toolCall.output_preview && (
            <pre
              style={{
                margin: 0,
                whiteSpace: 'pre-wrap',
                fontFamily: 'Menlo, Consolas, monospace',
                fontSize: 11,
                color: 'var(--text-secondary)',
                maxHeight: 200,
                overflow: 'auto',
              }}
            >
              {toolCall.output_preview}
            </pre>
          )}
          {!toolCall.success && toolCall.error && (
            <pre
              style={{
                margin: 0,
                whiteSpace: 'pre-wrap',
                fontFamily: 'Menlo, Consolas, monospace',
                fontSize: 11,
                color: 'var(--accent-error)',
                maxHeight: 200,
                overflow: 'auto',
              }}
            >
              {toolCall.error}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}
