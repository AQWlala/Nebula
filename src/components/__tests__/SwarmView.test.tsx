/**
 * v1.0.1 (P0#08): SwarmView failure panel + per-agent retry tests.
 */
import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup, act } from '@testing-library/preact';
import { SwarmView } from '../SwarmView';
import { nebulaStore } from '../../stores/nebulaStore';
import type { SwarmAgentResult } from '../../lib/tauri';

beforeEach(() => {
  cleanup();
  nebulaStore.currentTask.value = null;
  nebulaStore.swarmOutputs.value = [];
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  nebulaStore.currentTask.value = null;
  nebulaStore.swarmOutputs.value = [];
});

function makeOutputs(): SwarmAgentResult[] {
  return [
    {
      agent: 'coder',
      content: 'function is_palindrome(s) { return s === s.reverse(); }',
      status: 'ok',
    },
    {
      agent: 'writer',
      content: '## Summary\n...',
      status: 'ok',
    },
    {
      agent: 'reviewer',
      content: '(no output)',
      status: 'failed',
      error: 'tool_use: file not found',
      // Keep stderr short so the last-20-lines tail does not
      // drop the "cannot open foo.rs" header.  We still want
      // the data to look like a real compiler dump.
      stdout: 'reading foo.rs\nok\n',
      stderr: 'error: cannot open foo.rs\nstack backtrace:\n 0x0001',
      elapsed_ms: 1234,
    },
  ];
}

describe('SwarmView (P0#08)', () => {
  it('expanding_failure_card_shows_stderr', () => {
    nebulaStore.currentTask.value = { id: 'task-1', status: 'failed' };
    nebulaStore.swarmOutputs.value = makeOutputs();

    const { getByTestId, queryByTestId } = render(<SwarmView />);
    const card = getByTestId('swarm-output-reviewer');
    // Initially the stderr block is rendered but the inner <pre>
    // is hidden until the <details> wrapper is opened.
    expect(queryByTestId('swarm-output-reviewer-stderr')).toBeTruthy();
    expect(queryByTestId('swarm-output-reviewer-failure')).toBeTruthy();
    // Open the failure block.
    fireEvent.click(card);
    // After opening, the content is still mounted (it's a CSS
    // hide via <details>).  The important assertion is that the
    // failure panel exposes stderr text.  Verify the text is
    // present in the document.
    const stderr = getByTestId('swarm-output-reviewer-stderr');
    expect(stderr.textContent).toMatch(/cannot open foo\.rs/);
  });

  it('retry_button_calls_swarm_run_with_single_agent', async () => {
    nebulaStore.currentTask.value = { id: 'task-1', status: 'failed' };
    nebulaStore.swarmOutputs.value = makeOutputs();

    const runSwarmSingle = vi
      .spyOn(nebulaStore, 'runSwarmSingle')
      .mockResolvedValue();

    const { getByTestId, getByPlaceholderText } = render(<SwarmView />);
    // The user must enter a task description so the retry button
    // is enabled (the component gates retries on a non-empty
    // description).
    const textarea = getByPlaceholderText(/Rust/) as HTMLTextAreaElement;
    fireEvent.input(textarea, { target: { value: 'palindrome check' } });

    const retry = getByTestId('swarm-output-reviewer-retry');
    await act(async () => {
      fireEvent.click(retry);
    });
    expect(runSwarmSingle).toHaveBeenCalledWith('palindrome check', 'reviewer');
  });
});
