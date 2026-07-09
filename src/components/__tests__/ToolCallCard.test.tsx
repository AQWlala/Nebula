import { describe, it, expect } from 'vitest';
import { render, fireEvent } from '@testing-library/preact';
import { ToolCallCard } from '../ToolCallCard';
import type { AgentToolCall } from '../../lib/tauri';

const successCall: AgentToolCall = {
  agent_id: 'agent-1',
  agent_role: 'coder',
  tool_name: 'shell_exec',
  start_ts: 1000,
  end_ts: 1042,
  duration_ms: 42,
  success: true,
  output_preview: 'hello world',
  error: null,
  task_id: 'task-1',
};

const failedCall: AgentToolCall = {
  agent_id: 'agent-2',
  agent_role: 'coder',
  tool_name: 'file_read',
  start_ts: 2000,
  end_ts: 2010,
  duration_ms: 10,
  success: false,
  output_preview: null,
  error: 'permission denied',
  task_id: 'task-2',
};

describe('ToolCallCard', () => {
  it('renders tool name and duration', () => {
    const { getByText } = render(<ToolCallCard toolCall={successCall} />);
    expect(getByText('shell_exec')).toBeTruthy();
    expect(getByText('42ms')).toBeTruthy();
  });

  it('shows ✓ icon for successful calls', () => {
    const { getByText } = render(<ToolCallCard toolCall={successCall} />);
    expect(getByText('✓')).toBeTruthy();
  });

  it('shows ✗ icon for failed calls', () => {
    const { getByText } = render(<ToolCallCard toolCall={failedCall} />);
    expect(getByText('✗')).toBeTruthy();
  });

  it('does not show output preview when collapsed', () => {
    const { queryByText } = render(<ToolCallCard toolCall={successCall} />);
    expect(queryByText('hello world')).toBeNull();
  });

  it('expands to show output preview on click', () => {
    const { queryByText, container } = render(<ToolCallCard toolCall={successCall} />);
    // 点击 header 切换展开
    const header = container.querySelector('.tool-call-card > div');
    expect(header).not.toBeNull();
    fireEvent.click(header!);
    expect(queryByText('hello world')).not.toBeNull();
  });

  it('expands to show error for failed calls', () => {
    const { queryByText, container } = render(<ToolCallCard toolCall={failedCall} />);
    const header = container.querySelector('.tool-call-card > div');
    fireEvent.click(header!);
    expect(queryByText('permission denied')).not.toBeNull();
  });

  it('collapses back on second click', () => {
    const { queryByText, container } = render(<ToolCallCard toolCall={successCall} />);
    const header = container.querySelector('.tool-call-card > div');
    // 展开
    fireEvent.click(header!);
    expect(queryByText('hello world')).not.toBeNull();
    // 折叠
    fireEvent.click(header!);
    expect(queryByText('hello world')).toBeNull();
  });

  it('shows ▼ when collapsed and ▲ when expanded', () => {
    const { getByText, container } = render(<ToolCallCard toolCall={successCall} />);
    expect(getByText('▼')).toBeTruthy();
    const header = container.querySelector('.tool-call-card > div');
    fireEvent.click(header!);
    expect(getByText('▲')).toBeTruthy();
  });
});
