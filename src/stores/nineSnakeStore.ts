/**
 * 九头蛇全局状态管理
 *
 * v1.0.1 (P0#07): added `ollamaStatus` signal and `checkOllama()`
 * for the friendly offline banner in ChatPanel.  We poll every
 * 30s because the user may start/stop the daemon mid-session.
 */
import { signal } from '@preact/signals';
import {
  NineSnakeAPI,
  type Memory,
  type MetricsSnapshot,
  type MigrationStatus,
  type Reflection,
  type SwarmAgentResult,
} from '../lib/tauri';

/** "ok" means the backend answered health with ollama reachable. */
export type HealthStatus = 'unknown' | 'ok' | 'down';

class NineSnakeStoreClass {
  ready = signal(false);
  version = signal<string>('unknown');
  recentMemories = signal<Memory[]>([]);
  currentTask = signal<{ id: string; status: string; agent?: string } | null>(null);
  /** v1.0.1 (P0#08): per-agent output rows with optional failure
   *  metadata.  Old shape `{ agent, content }[]` is a structural
   *  subset so existing callers still type-check. */
  swarmOutputs = signal<SwarmAgentResult[]>([]);
  /** v0.2: process-wide metrics. */
  metrics = signal<MetricsSnapshot | null>(null);
  /** v0.2: schema migration status. */
  migrationStatus = signal<MigrationStatus | null>(null);
  /** v0.2: most recent L5 reflections, newest first. */
  reflections = signal<Reflection[]>([]);
  /** v0.5: currently-active top-level mode. */
  mode = signal<'writing' | 'work' | 'code'>('writing');
  /** v1.0.1 (P0#07): Ollama reachability. */
  ollamaStatus = signal<HealthStatus>('unknown');

  async bootstrap(): Promise<void> {
    await NineSnakeAPI.bootstrap();
    const health = await NineSnakeAPI.health();
    this.version.value = health.version;
    this.ready.value = true;
    await this.refreshMemories();
    await this.refreshMetrics();
    await this.refreshReflections();
    await this.refreshMigrationStatus();
    await this.checkOllama();
  }

  async refreshMemories(limit = 20): Promise<void> {
    try {
      this.recentMemories.value = await NineSnakeAPI.memoryListRecent(limit);
    } catch (e) {
      console.error('refreshMemories failed:', e);
    }
  }

  async refreshMetrics(): Promise<void> {
    try {
      this.metrics.value = await NineSnakeAPI.metrics();
    } catch (e) {
      console.error('refreshMetrics failed:', e);
    }
  }

  async refreshMigrationStatus(): Promise<void> {
    try {
      this.migrationStatus.value = await NineSnakeAPI.migrationStatus();
    } catch (e) {
      console.error('refreshMigrationStatus failed:', e);
    }
  }

  async refreshReflections(limit = 20): Promise<void> {
    try {
      this.reflections.value = await NineSnakeAPI.listReflections(limit);
    } catch (e) {
      console.error('refreshReflections failed:', e);
    }
  }

  /** v0.2: manually trigger a reflection pass. */
  async triggerReflection(): Promise<Reflection[]> {
    const out = await NineSnakeAPI.reflectNow();
    await this.refreshReflections();
    await this.refreshMetrics();
    return out;
  }

  /**
   * v1.0.1 (P0#07): refresh the Ollama reachability signal.
   * Returns the new status.  Never throws — a failing health
   * check simply flips the signal to 'down'.
   */
  async checkOllama(): Promise<HealthStatus> {
    try {
      // The backend's `health` command includes an `ollama` field
      // (see src-tauri/src/llm/ollama.rs).  We treat absence as
      // "down" so v0.5 backends that don't report it still get a
      // honest "we don't know" → banner shown, rather than a
      // silent success.
      const h = await NineSnakeAPI.health();
      this.ollamaStatus.value = h?.ollama === 'ok' ? 'ok' : 'down';
    } catch {
      this.ollamaStatus.value = 'down';
    }
    return this.ollamaStatus.value;
  }

  async runSwarm(description: string, agents: string[] = ['coder', 'writer', 'reviewer']) {
    const taskId = `task-${Date.now()}`;
    this.currentTask.value = { id: taskId, status: 'running' };
    this.swarmOutputs.value = [];

    try {
      const result = await NineSnakeAPI.swarmExecute({
        description,
        agents,
        max_retries: 2,
      });
      this.swarmOutputs.value = result.outputs;
      this.currentTask.value = { id: taskId, status: result.success ? 'success' : 'failed' };
      await this.refreshMemories();
      await this.refreshMetrics();
    } catch (e) {
      this.currentTask.value = { id: taskId, status: 'error' };
      throw e;
    }
  }

  /**
   * v1.0.1 (P0#08): retry a single agent.  The backend does not
   * yet have a per-agent command, so we model "retry this agent"
   * as a fresh swarm run with only that agent.  The original task
   * description is reused (the UI keeps it in the input).
   */
  async runSwarmSingle(description: string, agent: string) {
    return this.runSwarm(description, [agent]);
  }
}

export const NineSnakeStore = new NineSnakeStoreClass();
