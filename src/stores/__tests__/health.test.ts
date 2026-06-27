/**
 * v1.0.1 (P0#07): NineSnakeStore.checkOllama unit tests.
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { NineSnakeStore } from '../nineSnakeStore';
import { NineSnakeAPI } from '../../lib/tauri';

beforeEach(() => {
  // Reset the status back to "unknown" so each test starts clean.
  NineSnakeStore.ollamaStatus.value = 'unknown';
  vi.restoreAllMocks();
});

describe('NineSnakeStore.checkOllama (P0#07)', () => {
  it('checkOllama_down_sets_signal_to_down', async () => {
    vi.spyOn(NineSnakeAPI, 'health').mockRejectedValue(new Error('connection refused'));
    const result = await NineSnakeStore.checkOllama();
    expect(result).toBe('down');
    expect(NineSnakeStore.ollamaStatus.value).toBe('down');
  });

  it('checkOllama ok (ollama: "ok") keeps signal green', async () => {
    vi.spyOn(NineSnakeAPI, 'health').mockResolvedValue({
      status: 'ok',
      version: '1.0.0',
      ollama: 'ok',
    });
    const result = await NineSnakeStore.checkOllama();
    expect(result).toBe('ok');
    expect(NineSnakeStore.ollamaStatus.value).toBe('ok');
  });

  it('checkOllama absent ollama field is treated as down', async () => {
    vi.spyOn(NineSnakeAPI, 'health').mockResolvedValue({
      status: 'ok',
      version: '1.0.0',
      ollama: 'down',
    });
    const result = await NineSnakeStore.checkOllama();
    expect(result).toBe('down');
    expect(NineSnakeStore.ollamaStatus.value).toBe('down');
  });
});
