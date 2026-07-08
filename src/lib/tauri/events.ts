import { invoke, Channel } from '@tauri-apps/api/core';
import type { DoctorReport, EventEnvelope, ClipboardEvent } from './types';

export async function doctorRun(): Promise<DoctorReport> {
  return invoke<DoctorReport>('doctor_run');
}

export async function openFloatingProgress(taskId: string, title?: string): Promise<void> {
  await invoke('open_floating_progress', { taskId, title: title ?? null });
}

export async function swarmCancel(taskId: string): Promise<boolean> {
  return invoke<boolean>('swarm_cancel', { taskId });
}

export async function subscribeEvents(cb: (envelope: EventEnvelope) => void): Promise<() => void> {
  const channel = new Channel<EventEnvelope>();
  channel.onmessage = (envelope) => cb(envelope);
  invoke('subscribe_events', { on_event: channel }).catch(() => {});
  return () => {
    channel.onmessage = () => {};
  };
}

export async function listenClipboardDetected(
  cb: (event: ClipboardEvent) => void
): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  const unlisten = await listen<ClipboardEvent>('nebula://clipboard-detected', (event) => {
    if (event.payload) cb(event.payload);
  });
  return unlisten;
}
