/**
 * v1.0.1 (P0#07): nebulaStore.checkOllama unit tests.
 */
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { nebulaStore } from '../nebulaStore';
import { nebulaAPI } from '../../lib/tauri';

beforeEach(() => {
  // Reset the status back to "unknown" so each test starts clean.
  nebulaStore.ollamaStatus.value = 'unknown';
  vi.restoreAllMocks();
});

describe('nebulaStore.checkOllama (P0#07)', () => {
  it('checkOllama_down_sets_signal_to_down', async () => {
    vi.spyOn(nebulaAPI, 'healthFull').mockRejectedValue(new Error('connection refused'));
    const result = await nebulaStore.checkOllama();
    expect(result).toBe('down');
    expect(nebulaStore.ollamaStatus.value).toBe('down');
  });

  it('checkOllama ok (ollama: "ok") keeps signal green', async () => {
    vi.spyOn(nebulaAPI, 'healthFull').mockResolvedValue({
      status: 'ok',
      version: '1.0.0',
      ollama: 'ok',
    });
    const result = await nebulaStore.checkOllama();
    expect(result).toBe('ok');
    expect(nebulaStore.ollamaStatus.value).toBe('ok');
  });

  it('checkOllama absent ollama field is treated as down', async () => {
    vi.spyOn(nebulaAPI, 'healthFull').mockResolvedValue({
      status: 'ok',
      version: '1.0.0',
      ollama: 'down',
    });
    const result = await nebulaStore.checkOllama();
    expect(result).toBe('down');
    expect(nebulaStore.ollamaStatus.value).toBe('down');
  });
});
