/**
 * v1.0: CommandPalette tests.
 */
import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/preact';
import { CommandPalette, buildDefaultCommands, useCommandPaletteShortcut } from '../CommandPalette';
import { afterEach } from 'vitest';

afterEach(cleanup);

describe('CommandPalette', () => {
  it('renders nothing when closed', () => {
    const { container } = render(
      <CommandPalette open={false} onClose={() => {}} commands={[]} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders items when open', () => {
    const items = buildDefaultCommands(() => {}, {
      setMode: () => {},
      setSubMode: () => {},
      openSettings: () => {},
      triggerReflection: () => {},
    });
    const { getByText } = render(
      <CommandPalette open={true} onClose={() => {}} commands={items} />,
    );
    expect(getByText('Go to Chat')).toBeTruthy();
  });

  it('runs the picked command', () => {
    const onClose = vi.fn();
    const items = [
      { id: 'a', title: 'do thing', group: 'Test', run: vi.fn() },
    ];
    const { getByText } = render(
      <CommandPalette open={true} onClose={onClose} commands={items} />,
    );
    fireEvent.click(getByText('do thing'));
    expect(items[0].run).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it('closes on Escape', () => {
    const onClose = vi.fn();
    const { getByPlaceholderText } = render(
      <CommandPalette open={true} onClose={onClose} commands={[]} />,
    );
    const input = getByPlaceholderText(/Search commands/);
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });
});
