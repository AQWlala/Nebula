/**
 * Nebula全局状态管理
 *
 * v1.0.1 (P0#07): added `ollamaStatus` signal and `checkOllama()`
 * for the friendly offline banner in ChatPanel.  We poll every
 * 30s because the user may start/stop the daemon mid-session.
 */
import { signal } from '@preact/signals';
import {
  nebulaAPI,
  type Memory,
  type MetricsSnapshot,
  type MigrationStatus,
  type Reflection,
  type SwarmAgentResult,
} from '../lib/tauri';
import { DEFAULT_AUTONOMY_LEVEL, type AutonomyLevel } from '../lib/autonomy';

/** "ok" means the backend answered health with ollama reachable. */
export type HealthStatus = 'unknown' | 'ok' | 'down';

/** T-E-B-02: 顶层视图路由。移至 store 以便 ChatPanel `/journey` 等斜杠命令跨组件切换。
 *  原 App.tsx 模块级 signal 重构为 store signal,语义不变。 */
export type View =
  | 'chat'
  | 'swarm'
  | 'memory'
  | 'code'
  | 'skills'
  | 'dashboard'
  | 'credits'
  | 'diagnostics'
  | 'shadow'
  | 'longtask';

class nebulaStoreClass {
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
  /** v1.7: 外部打开的文件路径（双击 .md/.txt 或拖入文件时设置）。 */
  externalFilePath = signal<string | null>(null);
  /** T-E-D-06: 右键"问Nebula"时预填到 chat 输入框的文本。 */
  chatPrefill = signal<string | null>(null);
  /** T-S5-A-03: AI 自动模式开关(默认启用 LLM 路由,关闭退化为关键词)。 */
  aiAutoMode = signal<boolean>(true);
  /** T-S5-A-03: 最近一次自动路由的模式(供手动切换时比对误判)。 */
  lastAutoRoutedMode = signal<'writing' | 'work' | 'code' | null>(null);
  /** T-S5-A-03: 模式误分类计数(用户手动覆盖自动路由时递增)。 */
  modeMisclassification = signal<number>(0);
  /** T-E-S-50: 自主度滑块 L0-L5(默认 L2 对话,与 modeRouter 正交)。 */
  autonomyLevel = signal<AutonomyLevel>(DEFAULT_AUTONOMY_LEVEL);
  /** T-E-B-02: 记忆三视图切换('map'图谱 / 'list'Markdown列表 / 'timeline'时间轴)。
   *  放在 store 而非 App local state,以便 ChatPanel `/journey` 命令跨组件触发。 */
  memoryView = signal<'map' | 'list' | 'timeline'>('map');
  /** T-E-B-02: 顶层视图路由(原 App.tsx currentMode)。 */
  currentMode = signal<View>('code');

  async bootstrap(): Promise<void> {
    await nebulaAPI.bootstrap();
    const health = await nebulaAPI.health();
    this.version.value = health.version;
    this.ready.value = true;
    await this.refreshMemories();
    await this.refreshMetrics();
    await this.refreshReflections();
    await this.refreshMigrationStatus();
    await this.checkOllama();
    // T-E-S-50: 从后端同步当前自主度等级(默认 L2)。
    try {
      const { getLevel } = await import('../lib/autonomy');
      this.autonomyLevel.value = await getLevel();
    } catch {
      /* Tauri runtime unavailable; keep default L2 */
    }
  }

  async refreshMemories(limit = 20): Promise<void> {
    try {
      this.recentMemories.value = await nebulaAPI.memoryListRecent(limit);
    } catch (e) {
      console.error('refreshMemories failed:', e);
    }
  }

  async refreshMetrics(): Promise<void> {
    try {
      this.metrics.value = await nebulaAPI.metrics();
    } catch (e) {
      console.error('refreshMetrics failed:', e);
    }
  }

  async refreshMigrationStatus(): Promise<void> {
    try {
      this.migrationStatus.value = await nebulaAPI.migrationStatus();
    } catch (e) {
      console.error('refreshMigrationStatus failed:', e);
    }
  }

  async refreshReflections(limit = 20): Promise<void> {
    try {
      this.reflections.value = await nebulaAPI.listReflections(limit);
    } catch (e) {
      console.error('refreshReflections failed:', e);
    }
  }

  /** v0.2: manually trigger a reflection pass. */
  async triggerReflection(): Promise<Reflection[]> {
    const out = await nebulaAPI.reflectNow();
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
      const h = await nebulaAPI.healthFull();
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
      const result = await nebulaAPI.swarmExecute({
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

  /**
   * v1.7: 打开外部文件（双击 .md/.txt 或拖入窗口时调用）。
   * 设置 externalFilePath signal，CodeMode/WritingMode 监听后读取。
   */
  openExternalFile(path: string) {
    this.externalFilePath.value = path;
  }

  /**
   * T-E-D-06: 预填 chat 输入框文本(右键"问Nebula"时调用)。
   * ChatPanel 监听 chatPrefill signal 变化,读取后清空。
   */
  setChatPrefill(text: string) {
    this.chatPrefill.value = text;
  }
}

export const nebulaStore = new nebulaStoreClass();
