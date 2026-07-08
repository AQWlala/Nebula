/**
 * P0-C: nebulaStore core state management tests.
 *
 * Covers: initial signal values, direct mutations, and
 * signal-type correctness for mode/autonomy/ready signals.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { nebulaStore } from '../nebulaStore';

describe('nebulaStore – signal defaults', () => {
  it('ready starts false', () => {
    expect(nebulaStore.ready.value).toBe(false);
  });

  it('version starts "unknown"', () => {
    expect(nebulaStore.version.value).toBe('unknown');
  });

  it('mode defaults to writing', () => {
    expect(nebulaStore.mode.value).toBe('writing');
  });

  it('ollamaStatus starts unknown', () => {
    expect(nebulaStore.ollamaStatus.value).toBe('unknown');
  });

  it('autonomyLevel starts at default', () => {
    expect(nebulaStore.autonomyLevel.value).toBe('L2'); // L2 default
  });

  it('aiAutoMode defaults to true', () => {
    expect(nebulaStore.aiAutoMode.value).toBe(true);
  });

  it('currentTask starts null', () => {
    expect(nebulaStore.currentTask.value).toBeNull();
  });

  it('recentMemories starts empty', () => {
    expect(nebulaStore.recentMemories.value).toEqual([]);
  });
});

describe('nebulaStore – signal mutations', () => {
  beforeEach(() => {
    // reset to defaults after each test
    nebulaStore.ready.value = false;
    nebulaStore.mode.value = 'writing';
    nebulaStore.aiAutoMode.value = true;
    nebulaStore.currentTask.value = null;
  });

  it('ready can be set directly', () => {
    nebulaStore.ready.value = true;
    expect(nebulaStore.ready.value).toBe(true);
  });

  it('mode switches to code', () => {
    nebulaStore.mode.value = 'code';
    expect(nebulaStore.mode.value).toBe('code');
  });

  it('mode switches to work', () => {
    nebulaStore.mode.value = 'work';
    expect(nebulaStore.mode.value).toBe('work');
  });

  it('currentTask can hold a task object', () => {
    nebulaStore.currentTask.value = { id: 't1', status: 'running' };
    expect(nebulaStore.currentTask.value).toEqual({ id: 't1', status: 'running' });
  });

  it('aiAutoMode toggle works', () => {
    nebulaStore.aiAutoMode.value = false;
    expect(nebulaStore.aiAutoMode.value).toBe(false);
    nebulaStore.aiAutoMode.value = true;
    expect(nebulaStore.aiAutoMode.value).toBe(true);
  });
});
