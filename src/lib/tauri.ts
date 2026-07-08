/**
 * Tauri Command 封装 + 类型定义
 *
 * v0.3 新增 5 个 Skill CRUD 命令
 * v0.5 新增三模式 + 编辑器 + OS + 同步
 *
 * FUTURE: 此文件 24KB 已偏大，建议按领域拆分为
 *   api/chat.ts     — Chat, StreamToken
 *   api/memory.ts   — Memory, Search, Store, Reflection
 *   api/skill.ts    — Skill, SkillResult, SkillAudit
 *   api/swarm.ts    — SwarmTask, SwarmAgentResult
 *   api/work.ts     — WorkTask, MeetingMinutes
 *   api/writing.ts  — WritingTemplate, Document
 *   api/editor.ts   — FileEntry, GitStatus
 *   api/sync.ts     — Encrypt/Decrypt envelopes
 *   api/os.ts       — ShellExec, Clipboard
 *   types.ts        — 共享类型 (ErrorCode, Layer, MemoryType 等)
 *
 * 拆分时用 barrel export (index.ts) 保持 import 路径不变。
 */
import { invoke, Channel } from '@tauri-apps/api/core';

// v1.0.1 P0#12: a thin wrapper around `invoke` that swallows the
// missing-Tauri-runtime case (e.g. when the component is rendered in
// a browser preview, Storybook, or unit test) and returns `null`.
// Use this for one-off commands; for typed access prefer the
// `nebulaAPI` static methods below.
export async function invokeTauri<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T | null> {
  try {
    return (await invoke(cmd, args)) as T;
  } catch {
    return null;
  }
}

// L0-L5 are active in v1.x; L6-L7 reserved for v1.5+
export type Layer = 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5' | 'L6' | 'L7';
export type MemoryType = 'Semantic' | 'Episodic' | 'Procedural' | 'Emotional' | 'Metacognitive';

export interface Memory {
  id: string;
  memory_type: MemoryType;
  layer: Layer;
  content: string;
  summary: { s50: string; s150: string; s500: string; s2000: string };
  importance: number;
  access_count: number;
  last_access: number;
  created_at: number;
  source: string;
  pinned: boolean;
  compressed_from: string | null;
  compression_gen: number;
  archived: boolean;
  metadata: Record<string, unknown>;
  // T-E-A-09: 写入时记录的吸收成本(USD)。
  // - null/undefined: 未追踪(旧记忆 / cost_tracker 未注入)
  // - 0: 已追踪但为零(本地 Ollama + 未启用 EntityExtractor)
  // - >0: 实际 LLM 抽取费用
  ingest_cost?: number | null;
}

export interface ChatRequest {
  message: string;
  conversation_id?: string;
}

// T-E-B-17: ReasoningChain 类型定义(镜像 src-tauri/src/llm/reasoning.rs)
export interface ReasoningStep {
  premise: string;
  inference: string;
  confidence: number;
  evidence?: string;
}
export interface ReasoningChain {
  steps: ReasoningStep[];
  overall_confidence: number;
}

// T-E-S-64: 反幻觉一致性报告类型(镜像 src-tauri/src/memory/consistency.rs)。
export interface CitedMemory {
  id: string;
  source: string;
  tool?: string | null;
  content_hash?: string | null;
  snippet: string;
}

export type ConsistencyWarning =
  | { kind: 'source_conflict'; ids: string[] }
  | { kind: 'single_tool_negation'; tool: string }
  | { kind: 'empty_citation' };

export interface ConsistencyReport {
  cited: CitedMemory[];
  warnings: ConsistencyWarning[];
  risk_score: number;
}

export interface ChatResponse {
  model: string;
  role: string;
  content: string;
  reasoning_chain?: ReasoningChain;
  /** T-E-S-64: 反幻觉一致性报告(可选)。 */
  consistency?: ConsistencyReport;
}

/** v0.3: explicit DTO matching the Rust `StoreMemoryRequest`. */
export interface StoreMemoryRequest {
  content: string;
  memory_type: MemoryType;
  layer: Layer;
  source?: string;
  metadata?: Record<string, unknown> | null;
}

export interface StoreMemoryResponse {
  id: string;
  merged: boolean;
  similarity: number | null;
}

export interface SearchRequest {
  query: string;
  k?: number;
  layer?: Layer;
}

export interface SearchResponse {
  hits: { memory: Memory; score: number }[];
}

export interface SwarmTask {
  description: string;
  agents: string[]; // ['coder', 'writer', 'reviewer']
  max_retries?: number;
}

// -----------------------------------------------------------------------
// T-E-C-13: 工作场景模板库类型(镜像 src-tauri/src/scenarios/mod.rs)
// -----------------------------------------------------------------------

/** 模板分类(serde rename_all = "snake_case")。 */
export type ScenarioCategory = 'writing' | 'coding' | 'management';

/** 顶层角色(serde rename_all = "snake_case")。 */
export type ScenarioRole = 'writer' | 'coder' | 'manager';

/** AgentKind lowercase 序列化(generic/coder/writer/reviewer/researcher/planner)。 */
export type ScenarioAgentKind =
  'generic' | 'coder' | 'writer' | 'reviewer' | 'researcher' | 'planner';

/** 单个 agent 规格(传给 SwarmOrchestrator 的 agent 种类 + 角色标签)。 */
export interface AgentSpec {
  kind: ScenarioAgentKind;
  role: string;
  prompt_override?: string | null;
}

/** 一个工作场景模板。 */
export interface ScenarioTemplate {
  /** 稳定 id(如 "tech-blog" / "writer-base")。 */
  id: string;
  /** 中文显示名。 */
  name: string;
  /** 一句话描述。 */
  description: string;
  /** 分类(前端按此分组)。 */
  category: ScenarioCategory;
  /** 顶层角色。 */
  role: ScenarioRole;
  /** agent 规格列表。 */
  agents: AgentSpec[];
  /** 系统提示(注入到 SwarmTask.description 前缀)。 */
  system_prompt: string;
  /** 用户提示模板,含 `{{user_input}}` 占位符。 */
  user_prompt_template: string;
  /** 标签(供前端搜索/筛选)。 */
  tags: string[];
}

/** scenario_instantiate 命令的请求 DTO。 */
export interface InstantiateScenarioRequest {
  id: string;
  user_input: string;
}

// -----------------------------------------------------------------------
// T-E-L-05: Loop 模板库类型(镜像 src-tauri/src/commands/master.rs)
// -----------------------------------------------------------------------

/** Loop 自主度等级(serde rename_all = "UPPERCASE")。 */
export type LoopAutonomyLevel = 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5';

/** Loop 模板摘要(loop_templates_list 命令返回)。 */
export interface LoopTemplateSummary {
  /** Loop 名称(唯一标识,如 "ci-sweeper")。 */
  name: string;
  /** Loop 描述(人类可读)。 */
  description: string;
  /** 自主度等级 L0-L5。 */
  autonomy: LoopAutonomyLevel;
  /** cron 表达式(如 "0 * * * *")或 "on-webhook"。 */
  cadence: string;
  /** 单次执行 Token 预算。 */
  budget_tokens: number;
  /** 单次执行时间预算(分钟)。 */
  budget_minutes: number;
}

/** v1.0.1 (P0#08): per-agent result row.  The backend may now
 *  return stdout / stderr / elapsed_ms / status for each agent
 *  so the UI can show expandable failure details and a
 *  per-agent retry button.  All optional fields default to
 *  "unknown" 鈥?older backends only fill `agent` + `content`. */
export interface SwarmAgentResult {
  agent: string;
  content: string;
  status?: 'ok' | 'failed' | 'error';
  error?: string | null;
  stdout?: string | null;
  stderr?: string | null;
  elapsed_ms?: number | null;
}

export interface SwarmResult {
  task_id: string;
  outputs: SwarmAgentResult[];
  duration_ms: number;
  success: boolean;
}

/** v0.2: a meta-cognitive reflection (L5 layer). */
export interface Reflection {
  id: string;
  source_memories: string[];
  content: string;
  layer: 'L5';
  memory_type: 'Metacognitive';
  importance: number;
  trigger_kind: string;
  lessons: string[];
  confidence: number;
  created_at: number;
}

/** v0.2: process-wide perf monitor sample. */
export interface PerfSample {
  rss_bytes?: number | null;
  virt_bytes?: number | null;
  cpu_pct?: number | null;
  over_budget?: boolean;
  ts_ms?: number;
}

/** v2.0: Sidecar 状态信息 */
export interface SidecarStatusInfo {
  kind: string;
  status: string;
  running: boolean;
  pid?: number | null;
  listenAddr?: string | null;
}

/** v2.0: Self-Reflection 类型 */
export type ReflectionKind = 'value_alignment' | 'outcome_review' | 'self_improvement';

/** v2.0: 自我反思结果 */
export interface SelfReflection {
  kind: ReflectionKind;
  title: string;
  content: string;
  insights: string[];
  actionItems: string[];
  confidence: number;
  severity: number;
  relatedMemoryIds: string[];
}

/** v0.2: process-wide metrics snapshot. */
export interface MetricsSnapshot {
  embedding_cache_hits: number;
  embedding_cache_misses: number;
  memory_stores_total: number;
  memory_searches_total: number;
  blackhole_compressions_total: number;
  reflections_generated_total: number;
  swarm_executions_total: number;
  chat_total: number;
  // v1.8: 延迟累加器（微秒）+ 采样次数，前端可计算平均值。
  memory_search_latency_us_total: number;
  memory_search_latency_count: number;
  llm_chat_latency_us_total: number;
  llm_chat_latency_count: number;
  // T-S1-B-03: 5 项新可观测性指标。
  token_prompt_total: number;
  token_completion_total: number;
  l0_hits: number;
  l0_misses: number;
  l4_allow_total: number;
  l4_confirm_total: number;
  l4_plan_total: number;
  l4_deny_total: number;
  acl_allow_total: number;
  acl_deny_total: number;
  reflections_skipped_total: number;
  // T-E-A-01 / T-E-A-06 / T-E-A-04 / T-E-A-10: 缓存与费用指标。
  semantic_cache_hits: number;
  semantic_cache_misses: number;
  token_cost_usd: number;
  prefix_cache_hits: number;
  prefix_cache_cached_tokens: number;
  cost_saved_usd: number;
}

/** v0.2: migration status snapshot. */
export interface MigrationStatus {
  current_version: number;
  applied: { version: number; name: string; applied: boolean }[];
}

// ---------------------------------------------------------------------------
// v0.3: Skill CRUD DTOs
// ---------------------------------------------------------------------------

export interface Skill {
  id: string;
  name: string;
  description: string;
  code: string;
  language: string;
  tags: string[];
  usage_count: number;
  avg_rating: number;
  rating_count: number;
  created_at: number;
  updated_at: number;
  source_memory_id: string | null;
}

export interface SkillResult {
  skill_id: string;
  output: string;
  execution_time_ms: number;
  tokens_used: number;
}

export interface CreateSkillRequest {
  name: string;
  description: string;
  code: string;
  language: string;
  tags?: string[];
  source_memory_id?: string | null;
}

export interface UseSkillRequest {
  id: string;
  params?: Record<string, string> | null;
}

export interface RateSkillRequest {
  id: string;
  rating: number;
}

export interface ListSkillsRequest {
  language?: string | null;
  /** 旧字段(单 tag,向后兼容)。`tags` 非空时此字段被忽略。 */
  tag?: string | null;
  /** T-E-S-37: 多 tag 过滤(任一/全部匹配,由 `tag_match` 决定)。 */
  tags?: string[];
  /** T-E-S-37: 多 tag 匹配模式,默认 `'any'`(OR 语义)。 */
  tag_match?: 'any' | 'all';
  limit?: number;
}

/** T-E-S-37: tag 频次聚合行,镜像 src-tauri/src/skills/types.rs::TagCount。 */
export interface TagCount {
  tag: string;
  count: number;
}

export interface SkillSearchRequest {
  query: string;
  limit?: number;
}

export interface ImportResult {
  success: boolean;
  skill_id?: string;
  skill?: Skill;
  source?: string;
  error?: string;
}

// ---------------------------------------------------------------------------
// T-E-A-14: Arena A/B 测试 — 类型定义
// ---------------------------------------------------------------------------

/**
 * T-E-A-14: 单场对战记录。对应 SQLite `arena_matches` 表。
 *
 * 后端 `ArenaMatch` 结构体的前端镜像,字段一一对应。
 * `winner` 取值 `"a"` / `"b"` / `"tie"`,`null` 表示未判定。
 */
export interface ArenaMatch {
  id: string;
  prompt: string;
  model_a: string;
  model_b: string;
  response_a: string | null;
  response_b: string | null;
  /** "a" / "b" / "tie",null 表示未判定(等待人工投票或自动评分未跑)。 */
  winner: 'a' | 'b' | 'tie' | null;
  auto_score_a: number | null;
  auto_score_b: number | null;
  created_at: number;
}

/**
 * T-E-A-14: 排行榜行 — `[model, elo]` 元组。
 * 后端返回 `Vec<(String, f32)>`,前端按 tuple 接收。
 */
export type LeaderboardRow = [string, number];

/**
 * v0.2: fine-grained error code union. Wire format is the
 * lower-snake string form (e.g. `db`, `lance`, `llm`, `memory`).
 */
export type ErrorCode =
  | 'db'
  | 'lance'
  | 'llm'
  | 'memory'
  | 'swarm'
  | 'validation'
  | 'not_found'
  | 'permission'
  | 'internal'
  | 'unavailable';

export interface CommandError {
  code: ErrorCode;
  message: string;
  details?: string | null;
}

// -----------------------------------------------------------------------
// T-E-B-16 / T-E-B-07: MDRM 5 维关系图谱类型
// (镜像 src-tauri/src/memory/mdrm_graph.rs,serde snake_case)
// -----------------------------------------------------------------------

/** 5 维关系维度(与后端 `RelationDimension::as_str()` 一致)。 */
export type RelationDimension = 'causal' | 'temporal' | 'entity' | 'hierarchical' | 'similarity';

/** 节点角色(后端 `#[serde(rename_all = "snake_case")]`)。 */
export type GraphNodeRole = 'root' | 'inner' | 'leaf';

/** 图节点 — 前端可视化用。 */
export interface GraphNode {
  id: string;
  depth: number;
  role: GraphNodeRole;
  layer: Layer;
  summary: string;
  importance: number;
}

/** 图边 — 描述两个记忆间的关系。 */
export interface GraphEdge {
  src_id: string;
  dst_id: string;
  /** 关系类型字符串(causes/supports/contradicts/references/same_entity/derived_from/contains/before/similar)。 */
  kind: string;
  /** 所属维度字符串(causal/temporal/entity/hierarchical/similarity)。 */
  dimension: RelationDimension;
  /** 边权重 0.0-1.0。 */
  weight: number;
}

/** 图快照 — MDRM 查询结果,前端力导向图直接消费。 */
export interface GraphSnapshot {
  /** 查询起点 ID。 */
  root_id: string;
  /** 请求的维度列表。 */
  dimensions: RelationDimension[];
  /** 节点列表(去重,按 depth 升序)。 */
  nodes: GraphNode[];
  /** 边列表。 */
  edges: GraphEdge[];
  /** 是否因达到 max_nodes/max_edges 截断。 */
  truncated: boolean;
}

/** MDRM 查询参数(可选字段,缺省用后端 default)。 */
export interface MdrmQueryParams {
  max_depth?: number;
  max_nodes?: number;
  max_edges?: number;
  min_weight?: number;
}

// T-E-C-08: Shadow Workspace 类型(镜像 src-tauri/src/shadow_workspace/engine.rs)
export type ShadowStatus = 'creating' | 'running' | 'completed' | 'failed' | 'merged' | 'aborted';

export interface ShadowWorkspace {
  id: string;
  branch: string;
  path: string;
  task_description: string;
  status: ShadowStatus;
  created_at: number;
  finished_at: number | null;
  base_branch: string;
  error: string | null;
}

// T-E-C-09: 任务录屏回放类型(镜像 src-tauri/src/shadow_workspace/recording.rs)
export type OperationKind = 'file_create' | 'file_write' | 'file_delete' | 'command' | 'note';

export interface OperationRecord {
  /** workspace 内自增序号(从 1 开始)。 */
  seq: number;
  /** 操作时间(Unix 毫秒)。 */
  ts_ms: number;
  kind: OperationKind;
  /** 操作目标:File* 为相对路径,Command 为程序名,Note 为空。 */
  target: string;
  /** 详情:File* 为内容摘要,Command 为参数,Note 为备注全文。 */
  detail: string;
  success: boolean;
  /** 附加消息:Command 为输出摘要,失败时为错误描述。 */
  message: string;
}

// T-E-C-10: 异步长任务类型(镜像 src-tauri/src/long_task/engine.rs)
export type LongTaskStatus =
  'pending' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled';

export type StepStatus = 'pending' | 'running' | 'done' | 'failed' | 'skipped';

export interface LongTask {
  id: string;
  goal: string;
  status: LongTaskStatus;
  /** 关联的 Shadow Workspace ID(可选)。 */
  workspace_id: string | null;
  /** 关联的 PlanEngine 请求 ID(可选)。 */
  plan_id: string | null;
  /** 0-100 完成百分比。 */
  progress: number;
  error: string | null;
  created_at: number;
  updated_at: number;
  started_at: number | null;
  finished_at: number | null;
}

export interface LongTaskStep {
  task_id: string;
  seq: number;
  description: string;
  program: string;
  args: string[];
  status: StepStatus;
  started_at: number | null;
  finished_at: number | null;
  exit_code: number | null;
  output: string | null;
  error: string | null;
}

/** 创建任务时的步骤输入。 */
export interface StepInput {
  description: string;
  program: string;
  args?: string[];
}

export class nebulaAPI {
  static chat(req: ChatRequest): Promise<ChatResponse> {
    return invoke('chat', {
      request: { user_message: req.message, conversation_id: req.conversation_id },
    });
  }

  /**
   * v0.3 fix: the Rust command signature is
   * `memory_store(state, request: StoreMemoryRequest)`. Tauri maps
   * the snake-case parameter `request` to the JS key `request` (not
   * `req`), so we must send `{ request: ... }` 鈥?sending the raw
   * fields was the v0.1 / v0.2 bug.
   */
  static memoryStore(req: StoreMemoryRequest): Promise<StoreMemoryResponse> {
    return invoke('memory_store', { request: req });
  }

  static memorySearch(req: SearchRequest): Promise<SearchResponse> {
    return invoke('memory_search', {
      request: { query: req.query, k: req.k ?? 10, layer: req.layer },
    });
  }

  static memoryListRecent(limit: number): Promise<Memory[]> {
    return invoke('memory_list_recent', { limit });
  }

  static swarmExecute(task: SwarmTask): Promise<SwarmResult> {
    return invoke('swarm_execute', { task });
  }

  // -----------------------------------------------------------------------
  // T-E-C-13: 工作场景模板库(scenario_list / scenario_get / scenario_instantiate)
  // -----------------------------------------------------------------------

  /**
   * T-E-C-13: 列出场景模板。
   *
   * `category` 为 `null` 时返回全部模板;为指定分类时只返回该分类下的模板。
   * 前端按分类分组渲染时传 `ScenarioCategory` 字符串。
   *
   * @param category 可选分类过滤('writing' / 'coding' / 'management')
   */
  static scenarioList(category?: ScenarioCategory | null): Promise<ScenarioTemplate[]> {
    return invoke('scenario_list', { category: category ?? null });
  }

  /**
   * T-E-C-13: 按 id 查询单个场景模板。
   * 返回 `null` 表示 id 不存在。
   */
  static scenarioGet(id: string): Promise<ScenarioTemplate | null> {
    return invoke('scenario_get', { id });
  }

  /**
   * T-E-C-13: 实例化场景模板 — 把 `user_input` 填入模板,
   * 返回可传给 `swarmExecute` 的 `SwarmTask`。
   *
   * 前端典型流程:
   * 1. `scenarioInstantiate({ id, user_input })` → 拿到 `SwarmTask`
   * 2. `swarmExecute(task)` → 启动蜂群
   *
   * 返回 `null` 表示 `id` 不存在(前端应展示"模板不存在"提示)。
   */
  static scenarioInstantiate(req: InstantiateScenarioRequest): Promise<SwarmTask | null> {
    return invoke('scenario_instantiate', { request: req });
  }

  // -----------------------------------------------------------------------
  // T-E-L-05: Loop 模板库(loop_templates_list / loop_template_get)
  // -----------------------------------------------------------------------

  /**
   * T-E-L-05: 列出 7 种 Loop 模板摘要。
   *
   * 由 `master-orchestrator` feature 门控;未启用时后端返回
   * `command not found` 错误,前端应 catch 并降级为空列表。
   */
  static loopTemplatesList(): Promise<LoopTemplateSummary[]> {
    return invoke('loop_templates_list');
  }

  static llmComplete(prompt: string, model?: string): Promise<string> {
    return invoke('llm_complete', { prompt, model });
  }

  static bootstrap(): Promise<void> {
    return invoke('bootstrap');
  }

  static health(): Promise<{ status: string; version: string; ollama: string }> {
    return invoke('health');
  }

  static healthFull(): Promise<{ status: string; version: string; ollama: string }> {
    return invoke('health_full');
  }

  /** v0.2: manually trigger a reflection pass. */
  static reflectNow(): Promise<Reflection[]> {
    return invoke('reflect_now');
  }

  /** v0.2: list recent reflections, newest first. */
  static listReflections(limit = 20): Promise<Reflection[]> {
    return invoke('list_reflections', { limit });
  }

  /** v2.0: 执行一次真正的 Self-Reflection（价值对齐 + 结局复盘 + 自我改进） */
  static selfReflectNow(): Promise<SelfReflection[]> {
    return invoke('self_reflect_now');
  }

  /** v2.0: 获取所有 sidecar 的状态 */
  static sidecarListStatus(): Promise<SidecarStatusInfo[]> {
    return invoke('sidecar_list_status');
  }

  /** v2.0: 启动指定 sidecar */
  static sidecarStart(kind: string): Promise<boolean> {
    return invoke('sidecar_start', { kind });
  }

  /** v2.0: 停止指定 sidecar */
  static sidecarStop(kind: string): Promise<boolean> {
    return invoke('sidecar_stop', { kind });
  }

  /** v2.0: 重启指定 sidecar */
  static sidecarRestart(kind: string): Promise<boolean> {
    return invoke('sidecar_restart', { kind });
  }

  /** v0.2: snapshot the process-wide metrics. */
  static metrics(): Promise<MetricsSnapshot> {
    return invoke('metrics');
  }

  /** v0.2: read the current migration status. */
  static migrationStatus(): Promise<MigrationStatus> {
    return invoke('migration_status');
  }

  // -----------------------------------------------------------------------
  // v0.3: Skill CRUD
  // -----------------------------------------------------------------------

  static skillCreate(req: CreateSkillRequest): Promise<Skill> {
    return invoke('skill_create', { request: req });
  }

  static skillUse(req: UseSkillRequest): Promise<SkillResult> {
    return invoke('skill_use', { request: req });
  }

  static skillRate(req: RateSkillRequest): Promise<Skill> {
    return invoke('skill_rate', { request: req });
  }

  static skillList(req: ListSkillsRequest = {}): Promise<Skill[]> {
    return invoke('skill_list', { request: req });
  }

  static skillSearch(req: SkillSearchRequest): Promise<Skill[]> {
    return invoke('skill_search', { request: req });
  }

  /**
   * T-E-S-37: 返回所有 skill 的 tag 频次(按 count 降序)。
   *
   * 用于前端显示热门标签云:顶部展示前 N 个 tag + 出现次数,用户点击后
   * 切换对应的 tag 过滤。空库时返回空数组。
   */
  static skillTags(): Promise<TagCount[]> {
    return invoke('skill_tags');
  }

  static skillImport(url: string, source: string): Promise<ImportResult> {
    return invoke('skill_import', { identifier: url, source });
  }

  /**
   * T-E-S-45: 把指定 skill 导出为 agentskills.io `SKILL.md` 格式
   * (YAML front-matter + Markdown body)。
   *
   * - `outputPath` 省略/为 null:返回 `{ content: "<SKILL.md 字符串>" }`。
   * - `outputPath` 为文件路径:写入该文件,返回 `{ path: "<outputPath>" }`。
   *
   * 字段映射与 `skillImport` 对称 —— 8 个核心字段
   * (name/description/category=language/tags/trust_level/permissions/capabilities/body=code)
   * 可通过 `skillImport` 无损往返。
   *
   * @param skillId    要导出的 skill id
   * @param outputPath 目标文件路径(可选)。省略则只返回字符串内容。
   */
  static skillExportClawhub(
    skillId: string,
    outputPath?: string | null
  ): Promise<{ content?: string; path?: string }> {
    return invoke('skill_export_clawhub', {
      skill_id: skillId,
      output_path: outputPath ?? null,
    });
  }

  // -----------------------------------------------------------------------
  // v0.5: Writing mode
  // -----------------------------------------------------------------------

  static writingListTemplates(): Promise<WritingTemplate[]> {
    return invoke('writing_list_templates');
  }

  static writingGetTemplate(id: string): Promise<WritingTemplate | null> {
    return invoke('writing_get_template', { id });
  }

  static writingCreateDocument(req: CreateDocumentRequest): Promise<Document> {
    return invoke('writing_create_document', { request: req });
  }

  static writingUpdateDocument(id: string, content: string): Promise<Document> {
    return invoke('writing_update_document', { id, content });
  }

  static writingGetDocument(id: string): Promise<Document | null> {
    return invoke('writing_get_document', { id });
  }

  static writingListDocuments(limit = 50): Promise<Document[]> {
    return invoke('writing_list_documents', { limit });
  }

  static writingDeleteDocument(id: string): Promise<boolean> {
    return invoke('writing_delete_document', { id });
  }

  static writingExport(id: string, format: 'markdown' | 'html'): Promise<DocumentExport> {
    return invoke('writing_export', { request: { id, format } });
  }

  // -----------------------------------------------------------------------
  // v0.5: Work mode
  // -----------------------------------------------------------------------

  static workCreateTask(req: CreateTaskRequest): Promise<WorkTask> {
    return invoke('work_create_task', { request: req });
  }

  static workGetTask(id: string): Promise<WorkTask | null> {
    return invoke('work_get_task', { id });
  }

  static workListTasks(status?: 'todo' | 'doing' | 'done', limit = 100): Promise<WorkTask[]> {
    return invoke('work_list_tasks', { status, limit });
  }

  static workSetStatus(id: string, status: 'todo' | 'doing' | 'done'): Promise<WorkTask> {
    return invoke('work_set_status', { id, status });
  }

  static workUpdateTask(req: UpdateTaskRequest): Promise<WorkTask> {
    return invoke('work_update_task', { request: req });
  }

  static workDeleteTask(id: string): Promise<boolean> {
    return invoke('work_delete_task', { id });
  }

  static workRecommendPriority(title: string, dueAt: number | null): Promise<number> {
    return invoke('work_recommend_priority', { request: { title, due_at: dueAt } });
  }

  static workSummariseMeeting(transcript: string): Promise<MeetingMinutes> {
    return invoke('work_summarise_meeting', { transcript });
  }

  static workStartTimer(id: string): Promise<WorkTask> {
    return invoke('work_start_timer', { id });
  }

  static workStopTimer(): Promise<WorkTask | null> {
    return invoke('work_stop_timer');
  }

  static workAddTime(id: string, elapsedMs: number): Promise<WorkTask> {
    return invoke('work_add_time', { id, elapsed_ms: elapsedMs });
  }

  static workActiveTimer(): Promise<string | null> {
    return invoke('work_active_timer');
  }

  // -----------------------------------------------------------------------
  // v0.5: Editor
  // -----------------------------------------------------------------------

  static editorWorkspaceRoot(): Promise<string> {
    return invoke('editor_workspace_root');
  }

  static editorRead(path: string): Promise<FileContent> {
    return invoke('editor_read', { path });
  }

  static editorWrite(path: string, content: string): Promise<FileContent> {
    return invoke('editor_write', { path, content });
  }

  static editorList(maxDepth = 8): Promise<FileEntry[]> {
    return invoke('editor_list', { maxDepth });
  }

  static gitStatus(): Promise<GitStatus> {
    return invoke('git_status');
  }

  static gitLog(limit = 20): Promise<GitLogEntry[]> {
    return invoke('git_log', { limit });
  }

  static gitDiff(path = ''): Promise<GitDiff> {
    return invoke('git_diff', { path });
  }

  static gitCommit(message: string): Promise<string> {
    return invoke('git_commit', { message });
  }

  // -----------------------------------------------------------------------
  // v0.5: OS
  // -----------------------------------------------------------------------

  static osClipboardRead(): Promise<string> {
    return invoke('os_clipboard_read');
  }

  static osClipboardWrite(text: string): Promise<void> {
    return invoke('os_clipboard_write', { text });
  }

  static osShellExec(req: ShellExecRequest): Promise<ShellOutput> {
    return invoke('os_shell_exec', { request: req });
  }

  static osNotify(req: NotifyRequest): Promise<void> {
    return invoke('os_notify', { request: req });
  }

  // -----------------------------------------------------------------------
  // v0.5: Sync (E2EE)
  // -----------------------------------------------------------------------

  static syncMakeIdentity(): Promise<{ public_key: string; secret_key: string }> {
    return invoke('sync_make_identity');
  }

  static syncEncrypt(req: EncryptRequest): Promise<EncryptResponse> {
    return invoke('sync_encrypt', { request: req });
  }

  static syncDecrypt(req: DecryptRequest): Promise<DecryptResponse> {
    return invoke('sync_decrypt', { request: req });
  }

  static syncSend(req: SendSealedRequest): Promise<SendSealedResponse> {
    return invoke('sync_send', { request: req });
  }

  static syncAck(envelopeId: string): Promise<boolean> {
    return invoke('sync_ack', { envelope_id: envelopeId });
  }

  // -----------------------------------------------------------------------
  // T-S5-A-01: Device management (list / revoke)
  // -----------------------------------------------------------------------

  static deviceList(): Promise<DeviceInfo[]> {
    return invoke('list_devices');
  }

  static deviceRevoke(deviceId: string): Promise<boolean> {
    return invoke('revoke_device', { device_id: deviceId });
  }

  // -----------------------------------------------------------------------
  // v1.3: DID identity
  // -----------------------------------------------------------------------

  static generateDid(publicKeyB64?: string): Promise<GenerateDidResponse> {
    return invoke('generate_did', { public_key_b64: publicKeyB64 });
  }

  static resolveDid(did: string): Promise<ResolveDidResponse> {
    return invoke('resolve_did', { did });
  }

  // -----------------------------------------------------------------------
  // v1.3: Skill audit
  // -----------------------------------------------------------------------

  static skillAuditList(limit = 50): Promise<SkillAuditEntry[]> {
    return invoke('skill_audit_list', { limit });
  }

  static skillAuditListForSkill(skillId: string, limit = 50): Promise<SkillAuditEntry[]> {
    return invoke('skill_audit_list_for_skill', { skill_id: skillId, limit });
  }

  // -----------------------------------------------------------------------
  // v1.3 + T-S1-B-01b: Chat stream (Tauri 2.0 Channel 模式)
  // -----------------------------------------------------------------------

  /**
   * T-S1-B-01b: 流式 chat，使用 Tauri 2.0 `ipc::Channel` 双向通道。
   *
   * - `onToken` 回调在每个 token 到达时被调用（逐字渲染）。
   * - 返回的 Promise 在流结束后 resolve，值为完整拼接的 `ChatComplete`。
   * - 前端可通过 `abortSignal` 中止：signal abort 后前端停止处理回调，
   *   后端会在下次 `on_token.send()` 失败时 break 出 token 推送循环。
   *   注意：Tauri Channel 本身不支持 AbortSignal，这里通过在回调中
   *   检查 `signal.aborted` 并抛错来中断 Promise 链。
   *
   * 兼容性：保留旧 `chatStream(req)` 数组签名作为 fallback，
   * 见 `chatStreamLegacy`。
   */
  static chatStream(
    req: ChatRequest,
    onToken: (token: StreamToken) => void,
    abortSignal?: AbortSignal
  ): Promise<ChatComplete> {
    const channel = new Channel<StreamToken>();
    channel.onmessage = (token) => {
      if (abortSignal?.aborted) return;
      onToken(token);
    };
    return invoke<ChatComplete>('chat_stream', {
      request: { user_message: req.message },
      on_token: channel,
    });
  }

  // -----------------------------------------------------------------------
  // T-S5-B-01: 浮动窗/画中画 — 打开独立的浮动聊天窗口。
  // -----------------------------------------------------------------------

  static floatingChatOpen(): Promise<void> {
    return invoke('open_floating_chat');
  }

  // -----------------------------------------------------------------------
  // T-E-D-03: 桌面悬浮球 — 80x80 状态指示器窗口。
  // -----------------------------------------------------------------------

  static floatingBallOpen(): Promise<void> {
    return invoke('open_floating_ball');
  }

  /**
   * 订阅悬浮球状态推送 (`nebula://ball-state` 事件)。
   * 返回 unsubscribe 函数,在组件卸载时调用以释放监听器。
   *
   * 状态机:
   *  - idle: 空闲(绿色稳定)
   *  - thinking: 思考中(橙色脉冲)
   *  - executing: 执行中(霓虹橙快速闪烁)
   *  - notification: 有通知(红色)
   */
  static async subscribeBallState(onState: (s: BallState) => void): Promise<() => void> {
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<BallState>('nebula://ball-state', (event) => {
      if (event.payload) onState(event.payload);
    });
    return unlisten;
  }

  // -----------------------------------------------------------------------
  // T-E-S-51: Level 0 内联补全(本地小模型,零成本)。
  // -----------------------------------------------------------------------

  /**
   * T-E-S-51: 请求内联补全建议。
   *
   * 后端直连 Ollama 本地小模型(num_predict=20, temperature=0.2),
   * **不经过 LlmGateway / CostTracker**,不计费。
   *
   * 返回 `string | null`:`null` 表示无建议(prefix 太短、防抖命中、
   * Ollama 离线、模型返回空/回声)。**失败静默** — 不会抛错。
   */
  static inlineComplete(prefix: string): Promise<string | null> {
    return invoke('inline_complete', { prefix });
  }

  // -----------------------------------------------------------------------
  // T-E-S-59: 统一收件箱(跨渠道消息聚合,feature-gated behind `channels`)。
  // -----------------------------------------------------------------------

  /**
   * T-E-S-59: 列出收件箱消息(按时间戳降序)。
   * @param limit  最大条数(默认 50)
   * @param offset 偏移量(默认 0)
   * @param channel 可选渠道过滤(telegram / discord / webchat / jiuwenswarm)
   */
  static inboxList(limit = 50, offset = 0, channel?: string | null): Promise<UnifiedMessage[]> {
    return invoke('inbox_list', { limit, offset, channel: channel ?? null });
  }

  /**
   * T-E-S-59: 主动发起新消息到指定渠道。
   * @param targetChannel 目标渠道(telegram / discord / webchat / jiuwenswarm)
   * @param body          消息正文
   */
  static inboxSend(targetChannel: string, body: string): Promise<void> {
    return invoke('inbox_send', { targetChannel, body });
  }

  /**
   * T-E-S-59: 回复原消息(根据原消息 source_channel 路由出站)。
   * @param messageId 原消息 id
   * @param body      回复正文
   */
  static inboxReply(messageId: string, body: string): Promise<void> {
    return invoke('inbox_reply', { messageId, body });
  }

  /**
   * T-E-S-59: 标记一组消息为已读。
   * @param ids 消息 id 列表
   */
  static inboxMarkRead(ids: string[]): Promise<void> {
    return invoke('inbox_mark_read', { ids });
  }

  /**
   * T-E-S-59: 返回未读消息数。
   */
  static inboxUnreadCount(): Promise<number> {
    return invoke('inbox_unread_count');
  }

  // -----------------------------------------------------------------------
  // T-E-S-52: Level 1 定向编辑 — 选中文字 + 快捷键 -> AI 局部改写。
  // -----------------------------------------------------------------------

  /**
   * T-E-S-52: 请求定向编辑。后端走 LlmGateway::generate(本地 Ollama,不计费)。
   * 失败返回 Err(前端 toast 提示,与 L0 失败静默不同)。
   * @param selected 用户选中的文本
   */
  static directedEdit(selected: string): Promise<string> {
    return invoke('directed_edit', { selected });
  }

  // -----------------------------------------------------------------------
  // T-E-S-40: 多 provider keychain 命令(deepseek/openai-compat/anthropic)。
  // -----------------------------------------------------------------------

  /**
   * T-E-S-40: 写入指定 provider 的 API key 到 OS keychain。
   * `value` 为空时视为删除(对齐 set_api_key 语义)。
   * @param provider "deepseek" | "openai-compat" | "anthropic"
   * @param value    API key 明文(空串 = 删除)
   */
  static setProviderApiKey(provider: string, value: string): Promise<void> {
    return invoke('set_provider_api_key', { provider, value });
  }

  /**
   * T-E-S-40: 读取指定 provider 的 API key(掩码版)。
   * 返回 `null` 表示该 provider 未配置。
   * @param provider "deepseek" | "openai-compat" | "anthropic"
   */
  static getProviderApiKey(provider: string): Promise<MaskedApiKey | null> {
    return invoke('get_provider_api_key', { provider });
  }

  // -----------------------------------------------------------------------
  // T-E-B-09: 文件夹监控索引 — 监控目录变更自动吸收到 L3 语义记忆。
  // -----------------------------------------------------------------------

  /**
   * T-E-B-09: 启动(或热更新)文件夹监控。
   * 若 engine 尚未启动,会同时 start + spawn_worker;
   * 若已启动,会 reload_paths 替换 watcher 集合。
   * @param paths 要监控的目录绝对路径列表
   */
  static watchStart(paths: string[]): Promise<void> {
    return invoke('watch_start', { paths });
  }

  /**
   * T-E-B-09: 停止文件夹监控 + 取消消费者 task。
   */
  static watchStop(): Promise<void> {
    return invoke('watch_stop');
  }

  /**
   * T-E-B-09: 查询当前监控状态。
   */
  static watchStatus(): Promise<WatchStatus> {
    return invoke('watch_status');
  }

  /**
   * T-E-B-09: 仅返回当前监控路径列表(字符串形式)。
   */
  static watchListPaths(): Promise<string[]> {
    return invoke('watch_list_paths');
  }

  // -----------------------------------------------------------------------
  // T-E-C-14: 剪贴板智能监听 — 后台轮询 + 内容检测 + sponge 吸收。
  // -----------------------------------------------------------------------

  /**
   * T-E-C-14: 启动剪贴板监听。
   *
   * 后台 task 每 500ms 轮询剪贴板,对内容做 hash 去重 + 类型检测,
   * 把"有结构的"内容(URL/代码/表格/JSON 等)写入 L2 Episodic 记忆,
   * 并通过 `nebula://clipboard-detected` 事件通知前端。
   * 短文本(< 10 字符)与 Other 类型被忽略。
   */
  static clipboardWatchStart(): Promise<void> {
    return invoke('clipboard_watch_start');
  }

  /**
   * T-E-C-14: 停止剪贴板监听。Idempotent:未运行时也返回 Ok。
   */
  static clipboardWatchStop(): Promise<void> {
    return invoke('clipboard_watch_stop');
  }

  /**
   * T-E-C-14: 查询剪贴板监听是否正在运行。
   */
  static clipboardWatchStatus(): Promise<boolean> {
    return invoke('clipboard_watch_status');
  }

  // -----------------------------------------------------------------------
  // T-E-S-27: Trusted Diagnostics Channels — 独立可信诊断事件流。
  // -----------------------------------------------------------------------

  /**
   * T-E-S-27: 订阅实时诊断事件流(参考 chatStream 的 Channel 模式)。
   *
   * 使用 Tauri 2.0 `ipc::Channel` 双向通道:前端调用后立即开始监听,
   * 后端循环 `recv().await` 并把 `DiagnosticEvent` 推送给前端。
   * 前端关闭通道(返回页面或取消订阅)时 `on_event.send()` 失败,
   * 后端循环自动退出。Lagged 时后端会发 `Dropped` 元事件。
   *
   * @param onEvent 每条 DiagnosticEvent 的回调
   * @returns Promise 在订阅结束时 resolve(后端通道关闭时)
   */
  static diagnosticsSubscribe(onEvent: (event: DiagnosticEvent) => void): Promise<void> {
    const channel = new Channel<DiagnosticEvent>();
    channel.onmessage = (event) => onEvent(event);
    return invoke('subscribe_diagnostics', { on_event: channel });
  }

  /**
   * T-E-S-27: 返回最近 `limit` 条诊断事件快照(默认 50,最大 500)。
   *
   * 通过临时订阅 bus、限时收集 `limit` 条事件实现。
   * 当 `diagnostics_channel_enabled = false` 时返回空列表。
   *
   * @param limit 最大条数(默认 50)
   */
  static diagnosticsSnapshot(limit?: number): Promise<DiagnosticsSnapshot> {
    return invoke('diagnostics_snapshot', { limit: limit ?? null });
  }

  /**
   * T-E-S-27: 返回诊断日志所在目录路径(供前端用 shell plugin 打开)。
   *
   * 日志路径跟随平台默认:`%LOCALAPPDATA%\nebula\logs\`(Windows) /
   * `~/Library/Logs/nebula/`(macOS) / `~/.local/share/nebula/logs/`(Linux)。
   * 返回 `null` 表示无法确定路径。
   */
  static diagnosticsOpenLogs(): Promise<string | null> {
    return invoke('diagnostics_open_logs');
  }

  // -----------------------------------------------------------------------
  // T-E-S-41: models.json 动态配置(5 个 models_config_* 命令 + 2 个
  // provider key 命令)。provider 列表修改重启生效;default 可热更新。
  // -----------------------------------------------------------------------

  /**
   * T-E-S-41: 读取当前 ModelsConfig(从 AppState 的 RwLock 内存副本,
   * 而非每次重新读盘)。
   */
  static modelsConfigLoad(): Promise<ModelsConfig> {
    return invoke('models_config_load');
  }

  /**
   * T-E-S-41: 校验 + 落盘 + 热更新 AppState.models_config + 推送
   * override 到 cost_tracker(让 model_price() 立即看到新 pricing)。
   * 返回最新快照,前端可直接刷新 UI。
   */
  static modelsConfigSave(config: ModelsConfig): Promise<ModelsConfig> {
    return invoke('models_config_save', { config });
  }

  /**
   * T-E-S-41: 仅热更新 default_provider / default_model(不落盘)。
   * 若要持久化,前端应在本命令后再调用 modelsConfigSave 写盘。
   */
  static modelsConfigSetDefault(
    defaultProvider: string,
    defaultModel: string
  ): Promise<ModelsConfig> {
    return invoke('models_config_set_default', { defaultProvider, defaultModel });
  }

  /**
   * M7a #86 / P1-22: 从磁盘重新加载 models.json 到内存。
   *
   * 适用场景:用户手动编辑 models.json 文件后,不重启应用即生效。
   * 流程:读取文件 → 校验 → 热更新内存副本 → 同步 cost_tracker pricing。
   *
   * 注意:本命令不修改 UnifiedModelDispatcher 的 ModelPolicy(其 overrides
   * 在 Dispatcher 构造时快照)。若需更新 ModelPolicy,需重启应用。
   */
  static modelsConfigReload(): Promise<ModelsConfig> {
    return invoke('models_config_reload');
  }

  /**
   * T-E-S-41: 添加一个 provider(校验 id 唯一)。热更新 + 落盘。
   */
  static modelsConfigAddProvider(provider: ProviderConfig): Promise<ModelsConfig> {
    return invoke('models_config_add_provider', { provider });
  }

  /**
   * T-E-S-41: 删除非内置、非默认的 provider。热更新 + 落盘。
   */
  static modelsConfigRemoveProvider(providerId: string): Promise<ModelsConfig> {
    return invoke('models_config_remove_provider', { providerId });
  }

  /**
   * M6 #83: 测试 provider 连通性。
   * - Ollama: GET {base_url}/api/tags,2s 超时。
   * - 远端: GET {base_url}/v1/models,5s 超时,401/403 也算连通。
   * 不需要 API key(仅测试 TCP/HTTP 可达性)。
   */
  static modelsConfigTestProvider(providerId: string): Promise<ProviderTestResult> {
    return invoke('models_config_test_provider', { providerId });
  }

  /**
   * T-E-S-41: 写入用户自定义 provider 的 API key 到 OS keychain。
   * slot 名为 `provider:<providerId>`,与 KEY_API_KEY 分开。
   * 空 value 视为删除(与 setApiKey 语义对齐)。
   */
  static setProviderKey(providerId: string, value: string): Promise<void> {
    return invoke('set_provider_key', { providerId, value });
  }

  /**
   * T-E-S-41: 读取用户自定义 provider 的 API key(掩码版)。
   * 返回 null 表示该 provider 未配置。
   */
  static getProviderKey(providerId: string): Promise<MaskedApiKey | null> {
    return invoke('get_provider_key', { providerId });
  }

  // -----------------------------------------------------------------------
  // T-E-S-39: SOUL.md / AGENTS.md / TOOLS.md persona injection
  // -----------------------------------------------------------------------

  /**
   * T-E-S-39: 从工作区根目录重新加载 persona 文件并热更新缓存。
   * 返回最新的 PersonaConfig 快照。
   */
  static personaReload(): Promise<PersonaConfig> {
    return invoke('persona_reload');
  }

  /**
   * T-E-S-39: 读取当前 persona 配置快照。
   */
  static personaGet(): Promise<PersonaConfig> {
    return invoke('persona_get');
  }

  /**
   * T-E-S-39: 手动设置单个 persona 文件内容(热更新内存缓存,不落盘)。
   * kind: "soul" / "agents" / "tools"(不区分大小写)。
   * content: null 清除该字段;字符串设置内容。
   */
  static personaSetFile(kind: string, content: string | null): Promise<PersonaConfig> {
    return invoke('persona_set_file', { kind, content });
  }

  // -----------------------------------------------------------------------
  // T-E-S-28: 对话标注(upsert / list / stats / export)
  // -----------------------------------------------------------------------

  /**
   * T-E-S-28: 写入/更新一条对话标注(👍/👎)。
   *
   * `annotation` 取值 `"good"` / `"bad"`。`UNIQUE(turn_id)` + `INSERT OR REPLACE`
   * 保证幂等:用户反复点击 👍/👎 只保留最新一条。
   *
   * 后端在 `annotation == "bad"` 且 `comment` 非空时,会触发 `sponge.absorb_text`
   * 把用户反馈回流到 L1 Episodic 记忆(让 AI 在后续对话中知晓用户偏好)。
   * 吸收失败不阻断标注写入(best-effort)。
   *
   * 参数对齐后端 `commands::annotations::annotation_upsert` 的 snake_case 形参名。
   */
  static annotationUpsert(params: {
    turn_id: string;
    annotation: 'good' | 'bad';
    comment?: string | null;
    agent_role?: string | null;
    model?: string | null;
    conversation_id?: string | null;
  }): Promise<void> {
    return invoke('annotation_upsert', {
      turn_id: params.turn_id,
      annotation: params.annotation,
      comment: params.comment ?? null,
      agent_role: params.agent_role ?? null,
      model: params.model ?? null,
      conversation_id: params.conversation_id ?? null,
    });
  }

  /**
   * T-E-S-28: 列出最近 `limit` 条标注(新在前)。
   * `limit` 省略或为 null 时后端返回最近 1000 条。
   */
  static annotationList(limit?: number | null): Promise<Annotation[]> {
    return invoke('annotation_list', { limit: limit ?? null });
  }

  /**
   * T-E-S-28: 聚合统计 good/bad 总数 + 按 model/agent 分桶。
   * 用于持续改进分析(如计算各模型的满意度比例)。
   */
  static annotationStats(): Promise<AnnotationStats> {
    return invoke('annotation_stats');
  }

  /**
   * T-E-S-28: 导出标注数据为 JSONL 字符串。
   *
   * `format` 取值:
   * - `"jsonl"`: 每行一个 `Annotation` 的 JSON 序列化(原始行格式)。
   * - `"dify"`: Dify 训练数据集 JSONL,每行
   *   `{"conversation": "...", "message": "...", "score": 0|1, "feedback": "..."}`
   *   score = (annotation == "good" ? 1 : 0)。
   */
  static annotationExport(format: 'jsonl' | 'dify'): Promise<string> {
    return invoke('annotation_export', { format });
  }

  // -----------------------------------------------------------------------
  // T-E-C-16: 对话导出（Markdown 前端实现 / DOCX 后端实现 / PDF print）
  // -----------------------------------------------------------------------

  /**
   * T-E-C-16: 导出对话为 DOCX 格式（后端用 docx-rs 生成）。
   *
   * @param messages 消息数组
   * @param options 导出选项
   * @returns 包含文件路径的结果
   */
  static exportChatDocx(params: {
    messages: { role: 'user' | 'assistant'; content: string; timestamp: number }[];
    options: { title?: string; include_timestamps?: boolean };
  }): Promise<{ file_path: string; byte_size: number }> {
    return invoke('export_chat_docx', {
      messages: params.messages,
      options: params.options,
    });
  }

  // -----------------------------------------------------------------------
  // T-E-S-24: 文件快照回滚
  // -----------------------------------------------------------------------

  /**
   * T-E-S-24: 创建文件快照。
   * @param working_dir 工作目录
   * @param files 要快照的文件路径列表
   * @returns 快照 ID
   */
  static snapshotCreate(workingDir: string, files: string[]): Promise<string> {
    return invoke('snapshot_create', { working_dir: workingDir, files });
  }

  /**
   * T-E-S-24: 回滚到指定快照。
   * @param id 快照 ID
   */
  static snapshotRollback(id: string): Promise<void> {
    return invoke('snapshot_rollback', { id });
  }

  /**
   * T-E-S-24: 丢弃指定快照。
   * @param id 快照 ID
   */
  static snapshotDiscard(id: string): Promise<void> {
    return invoke('snapshot_discard', { id });
  }

  /**
   * T-E-S-24: 列出所有活跃快照。
   * @returns 快照列表
   */
  static snapshotList(): Promise<SnapshotInfo[]> {
    return invoke('snapshot_list');
  }

  // -----------------------------------------------------------------------
  // T-E-D-06: 文件拖拽吸收 — sponge_absorb_file 命令封装。
  // -----------------------------------------------------------------------

  /**
   * T-E-D-06: 将文件吸收到记忆系统(由悬浮球 drag-drop 监听器调用)。
   *
   * 后端先用扩展名白名单过滤
   * (txt/md/json/yaml/toml/csv/py/js/ts/rs/go/java/c/cpp/h/sql/xml/html/css/pdf/docx),
   * 非白名单返回 `validation` 错误;通过后调用 `SpongeEngine::absorb_file`,
   * pdf/docx 由内部 `document_extractor` 提取文本,其余走 `tokio::fs::read_to_string`。
   *
   * @param path 文件绝对路径
   * @returns `{ id, kind, similarity, path }`
   *  - `kind`: `"inserted" | "merged" | "duplicate" | "deactivated"`
   *  - `similarity`: 0-1 浮点或 null(inserted/deactivated 时为 null)
   */
  static spongeAbsorbFile(path: string): Promise<SpongeAbsorbFileResult> {
    return invoke('sponge_absorb_file', { path });
  }

  // -----------------------------------------------------------------------
  // T-E-D-06: Windows 右键菜单 "问Nebula" — install / uninstall / status。
  // -----------------------------------------------------------------------

  /**
   * T-E-D-06: 安装 Windows 右键菜单 "问Nebula"(写 HKCU 注册表,免管理员)。
   * 非 Windows 平台返回 `{ installed: false, error: "not supported" }`。
   */
  static contextMenuInstall(): Promise<ContextMenuStatus> {
    return invoke('context_menu_install');
  }

  /**
   * T-E-D-06: 卸载 Windows 右键菜单 "问Nebula"(删除 HKCU 注册表项)。
   * 非 Windows 平台返回 `{ installed: false, error: "not supported" }`。
   */
  static contextMenuUninstall(): Promise<ContextMenuStatus> {
    return invoke('context_menu_uninstall');
  }

  /**
   * T-E-D-06: 查询 Windows 右键菜单 "问Nebula" 当前安装状态。
   */
  static contextMenuStatus(): Promise<ContextMenuStatus> {
    return invoke('context_menu_status');
  }

  // -----------------------------------------------------------------------
  // T-E-C-17: IM 扫码绑定(Feishu/WeCom/DingTalk webhook)
  // -----------------------------------------------------------------------

  /**
   * T-E-C-17: 创建 webhook 绑定。
   *
   * 后端会先做 SSRF 校验(拒绝 192.168 / 10.x / 127.x / 169.254 等),
   * 通过后落盘到 `im_bindings` 表,返回完整 `ImBinding`(含生成的 UUID id)。
   *
   * @param platform 平台标识('feishu' / 'wecom' / 'dingtalk')
   * @param url      webhook URL(公网,SSRF 校验)
   * @param displayName 可读名称(如 "团队群"),可空
   */
  static imCreateWebhookBinding(req: CreateImWebhookBindingRequest): Promise<ImBinding> {
    return invoke('im_create_webhook_binding', { request: req });
  }

  /**
   * T-E-C-17: 列出所有 IM 绑定(按 created_at ASC)。
   */
  static imListBindings(): Promise<ImBinding[]> {
    return invoke('im_list_bindings');
  }

  /**
   * T-E-C-17: 删除 IM 绑定(幂等:不存在的 id 也返回 Ok)。
   */
  static imDeleteBinding(id: string): Promise<void> {
    return invoke('im_delete_binding', { id });
  }

  /**
   * T-E-C-17: 启用/禁用 IM 绑定。
   */
  static imSetEnabled(id: string, enabled: boolean): Promise<void> {
    return invoke('im_set_enabled', { id, enabled });
  }

  /**
   * T-E-C-17: 单条绑定的测试发送(同步返回结果)。
   *
   * 成功时后端会更新 `last_used_at`。失败时抛出 `CommandError`。
   */
  static imTestSend(id: string, title: string, body: string): Promise<void> {
    return invoke('im_test_send', { id, title, body });
  }

  /**
   * T-E-C-17: 广播到所有已启用绑定(并发发送,部分失败不影响其他)。
   *
   * 返回成功数 + 失败数;失败详情通过 tracing::warn 记录。
   */
  static imBroadcast(req: ImBroadcastRequest): Promise<ImBroadcastResult> {
    return invoke('im_broadcast', { request: req });
  }

  // -----------------------------------------------------------------------
  // T-E-B-05: Wiki 双向链接命令
  // -----------------------------------------------------------------------

  /**
   * T-E-B-01: 列出 wiki 笔记(分页,created_at DESC)。
   * 供 [[ 自动补全调用:输入 `[[` 后取 limit 条候选。
   */
  static wikiList(limit?: number, offset?: number): Promise<WikiNote[]> {
    return invoke('wiki_list', { limit: limit ?? 50, offset: offset ?? 0 });
  }

  /**
   * T-E-B-05: 获取反向链接(所有指向 note_id 的笔记)。
   * 供笔记详情页展示"被哪些笔记引用"。
   */
  static wikiBacklinks(noteId: string): Promise<WikiNote[]> {
    return invoke('wiki_backlinks', { note_id: noteId });
  }

  /**
   * T-E-B-13: 获取知识卡片(聚合 note + body + definition + related_entities + backlinks)。
   *
   * 供 KnowledgeCardDialog 弹窗调用:点击 `[[xxx]]` wiki-link 后,
   * 前端传 slug 调本命令,后端聚合返回 `KnowledgeCard`。
   *
   * @param slug 笔记的文件名安全 slug(前端 `[[xxx]]` 链接的 xxx 部分,非 UUID)
   */
  static wikiGetCard(slug: string): Promise<KnowledgeCard> {
    return invoke('wiki_get_card', { slug });
  }

  /**
   * T-E-B-01: 读取 wiki 笔记(元数据 + Markdown 正文)。
   * 供编辑入口初始化 textarea:加载当前 body 作为初始文本。
   */
  static wikiRead(id: string): Promise<WikiNoteReadResponse> {
    return invoke('wiki_read', { id });
  }

  /**
   * T-E-B-03: 用户编辑 wiki 笔记后的双向同步。
   *
   * 后端流程:
   * 1. SQLite UPDATE `wiki_notes.body` + `updated_at`
   * 2. `sponge.absorb_text(&new_body)` 重新向量化(失败仅 warn,不阻断)
   * 3. `storage.write(&path, new_body)` 重写 markdown 文件
   * 4. `version_control.commit(...)` 写版本记录(失败仅 warn,不阻断)
   * 5. `append_log(LogEvent::Updated)` 追加到 `_log.md`
   *
   * @param noteId 笔记 UUID
   * @param newBody 新的 Markdown 正文
   */
  static wikiUpdateFromUser(noteId: string, newBody: string): Promise<void> {
    return invoke('wiki_update_from_user', { noteId, newBody });
  }

  // -----------------------------------------------------------------------
  // T-E-A-14: Arena A/B 测试命令
  // -----------------------------------------------------------------------

  /**
   * T-E-A-14: 创建一场 A/B 对战。
   *
   * 后端并行调用 model_a / model_b 生成响应,可选自动评分,
   * 持久化到 `arena_matches` 表。返回 match_id(UUID v4)。
   *
   * @param prompt 对战 prompt(同一 prompt 喂给两个模型)
   * @param modelA 模型 A 标识(如 `"deepseek-chat"` / `"qwen2.5:7b"`)
   * @param modelB 模型 B 标识
   * @returns match_id(UUID v4)
   */
  static arenaCreateMatch(prompt: string, modelA: string, modelB: string): Promise<string> {
    return invoke('arena_create_match', { prompt, modelA, modelB });
  }

  /**
   * T-E-A-14: 人工投票覆盖 winner。
   *
   * @param matchId 对战 ID(arenaCreateMatch 返回值)
   * @param winner `"a"` / `"b"` / `"tie"`
   */
  static arenaVote(matchId: string, winner: 'a' | 'b' | 'tie'): Promise<void> {
    return invoke('arena_vote', { matchId, winner });
  }

  /**
   * T-E-A-14: 获取按 ELO 降序的排行榜。
   *
   * @returns `LeaderboardRow[]` — `[model, elo]` 元组数组,按 elo 降序
   */
  static arenaLeaderboard(): Promise<LeaderboardRow[]> {
    return invoke('arena_leaderboard');
  }

  // ---------------------------------------------------------------------
  // M6 #82: Master 编排 + L4 审批命令
  // ---------------------------------------------------------------------

  /**
   * M6 #82: 启动 MasterOrchestrator 编排,通过 Tauri 2.0 ipc::Channel
   * 实时推送 11 个 MasterEvent 变体给前端。
   *
   * 使用模式(参考 `chatStream` 的 Channel 模式):
   * ```
   * const report = await nebulaAPI.masterRun(
   *   { input, mode: 'standard' },
   *   (event) => {
   *     // 按 event.kind 分支渲染时间线
   *     if (event.kind === 'user_confirmation_required') {
   *       // 弹审批 modal,用户确认后调 masterConfirm(event.confirmation_id)
   *     }
   *   },
   *   controller.signal,
   * );
   * ```
   *
   * @param req.input   用户原始输入
   * @param req.mode    执行模式 standard/bypass/plan
   * @param onEvent     每个 MasterEvent 的回调(流式)
   * @param signal       AbortSignal,abort 后停止处理后续事件
   * @returns MasterReport — 流结束后返回最终综合输出 + 子任务统计
   */
  static masterRun(
    req: { input: string; mode?: ExecuteMode },
    onEvent: (event: MasterEvent) => void,
    signal?: AbortSignal
  ): Promise<MasterReport> {
    const channel = new Channel<MasterEvent>();
    channel.onmessage = (event) => {
      if (signal?.aborted) return;
      onEvent(event);
    };
    return invoke<MasterReport>('master_run', {
      input: req.input,
      mode: req.mode ?? 'standard',
      onMasterEvent: channel,
    }).finally(() => {
      channel.onmessage = () => {};
    });
  }

  /**
   * M6 #82: 用户确认 L4 审批请求(防重放 + 5min 超时)。
   *
   * 首次提交返回 `confirmed`;二次提交返回 `already_used`(防重放);
   * 超过 5 分钟返回 `expired`;不存在的 id 返回 `not_found`。
   */
  static masterConfirm(confirmationId: string): Promise<ConfirmationStatus> {
    return invoke('master_confirm', { confirmationId });
  }

  /**
   * M6 #82: 查询 confirmation 状态(供前端显示倒计时 / 防重放提示)。
   */
  static masterConfirmationStatus(confirmationId: string): Promise<ConfirmationStatus> {
    return invoke('master_confirmation_status', { confirmationId });
  }

  /**
   * M6 #82: 列出当前 pending 的审批请求(供 UI 渲染待确认列表)。
   * 前端按 `created_at` + 5min 自行过滤已过期条目。
   */
  static masterPendingConfirmations(): Promise<PendingConfirmation[]> {
    return invoke('master_pending_confirmations');
  }

  // ---------------------------------------------------------------------
  // M6 #78: 进化日志 + 回滚命令
  // ---------------------------------------------------------------------

  /**
   * M6 #78: 列出全部进化日志条目(按写入顺序)。
   *
   * 返回 `EvolutionLogEntry[]`,空数组表示无进化记录或日志文件不存在。
   *
   * 注意:此命令仅在 `evolution-engine` feature 启用时编译,
   * feature off 时 invoke 会 reject "command not found"。
   * 前端应通过 `evolutionEnabled()` 先查询运行时状态,再决定是否调用。
   */
  static evolutionLogList(): Promise<EvolutionLogEntry[]> {
    return invoke('evolution_log_list');
  }

  /**
   * M6 #78: 查询单条进化日志条目(通过 entry_id)。
   *
   * @returns 找不到返回 null
   */
  static evolutionLogGet(entryId: string): Promise<EvolutionLogEntry | null> {
    return invoke('evolution_log_get', { entryId });
  }

  /**
   * M6 #78: 回滚最近 N 条 Phase 4 (Soul) 进化写入。
   *
   * 仅回滚 SOUL.md evolution-append section 内的段落,不回滚 L2/L3/L5 记忆
   * (历史事实不可破坏审计链)。回滚后从 evolution_log.md 删除对应条目。
   *
   * `n = 0` 时无操作。`n` 超过实际条目数时按实际数量回滚,不报错。
   *
   * @returns RollbackResult — 含实际回滚条数 + 失败 warnings
   */
  static evolutionRollback(n: number): Promise<RollbackResult> {
    return invoke('evolution_rollback', { n });
  }

  /**
   * M6 #78: 查询进化引擎运行时开关状态。
   *
   * 返回 `true` 表示进化引擎已启用(可执行 4 Phase),`false` 表示已禁用。
   * 注意:即使返回 `true`,若 `evolution-engine` feature 未编译,4 Phase 命令仍不可用。
   * 此命令本身在 `self-evolution` feature 启用时可用(包含 evolution-engine)。
   */
  static evolutionEnabled(): Promise<boolean> {
    return invoke('evolution_enabled');
  }

  /**
   * M6 #78: 设置进化引擎运行时开关。
   *
   * `enabled = true` 启用,`false` 禁用。禁用后 EvolutionEngine 不会自动执行 4 Phase。
   * 此开关仅影响运行时,不影响 feature flag 编译期决策。
   */
  static evolutionSetEnabled(enabled: boolean): Promise<void> {
    return invoke('evolution_set_enabled', { enabled });
  }

  /**
   * M7b #97: 查询 Soul 系统运行时开关状态。
   *
   * 返回 `true` 表示 Soul 系统已启用(SoulCompiler 可执行),`false` 表示已禁用。
   * 注意:即使返回 `true`,若 `soul-system` feature 未编译,命令仍不可用。
   * 此命令本身在 `soul-system` feature 启用时可用。
   */
  static soulSystemEnabled(): Promise<boolean> {
    return invoke('soul_system_enabled');
  }

  /**
   * M7b #97: 设置 Soul 系统运行时开关。
   *
   * `enabled = true` 启用,`false` 禁用。禁用后 SoulCompiler 不会自动执行。
   * 此开关仅影响运行时,不影响 feature flag 编译期决策。
   */
  static soulSystemSetEnabled(enabled: boolean): Promise<void> {
    return invoke('soul_system_set_enabled', { enabled });
  }

  // -----------------------------------------------------------------------
  // T-E-B-16 / T-E-B-07: MDRM 5 维关系图谱命令
  // -----------------------------------------------------------------------

  /**
   * T-E-B-07: 多维度组合查询(供 MemoryMap 力导向图视图)。
   *
   * `dims` 缺省或空数组 → 全 5 维遍历(等价 `get_full_graph`)。
   * 返回 `GraphSnapshot`,前端直接渲染节点 + 边。
   *
   * @param memoryId 查询起点记忆 ID
   * @param dims     维度过滤(空/缺省=全部)
   * @param params   可选查询参数(深度/节点上限/边上限/权重下限)
   */
  static mdrmGetGraph(
    memoryId: string,
    dims?: RelationDimension[] | null,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_get_graph', {
      memoryId,
      dims: dims ?? null,
      params: params ?? null,
    });
  }

  /** T-E-B-16: 时序维度 — 沿 `Before` 边追溯时间链。 */
  static mdrmTraceTemporal(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_trace_temporal', { memoryId, params: params ?? null });
  }

  /** T-E-B-16: 实体维度 — 查找同实体记忆簇(SameEntity/References)。 */
  static mdrmFindEntities(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_find_entities', { memoryId, params: params ?? null });
  }

  /** T-E-B-16: 层级维度 — 追溯 Contains/DerivedFrom 层级。 */
  static mdrmTraceHierarchy(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_trace_hierarchy', { memoryId, params: params ?? null });
  }

  /** T-E-B-16: 相似度维度 — 查找相似记忆(Similar)。 */
  static mdrmFindSimilar(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_find_similar', { memoryId, params: params ?? null });
  }

  // -----------------------------------------------------------------------
  // T-E-C-08: Shadow Workspace — Agent 隔离执行环境(git worktree)
  // -----------------------------------------------------------------------

  static shadowCreate(
    taskDescription: string,
    baseBranch?: string | null
  ): Promise<ShadowWorkspace> {
    return invoke('shadow_create', {
      taskDescription,
      baseBranch: baseBranch ?? null,
    });
  }

  static shadowList(): Promise<ShadowWorkspace[]> {
    return invoke('shadow_list');
  }

  static shadowStatus(workspaceId: string): Promise<ShadowWorkspace | null> {
    return invoke('shadow_status', { workspaceId });
  }

  static shadowDiff(workspaceId: string): Promise<string> {
    return invoke('shadow_diff', { workspaceId });
  }

  static shadowRunCommand(workspaceId: string, program: string, args: string[]): Promise<string> {
    return invoke('shadow_run_command', { workspaceId, program, args });
  }

  static shadowComplete(workspaceId: string): Promise<ShadowWorkspace> {
    return invoke('shadow_complete', { workspaceId });
  }

  static shadowFail(workspaceId: string, error: string): Promise<ShadowWorkspace> {
    return invoke('shadow_fail', { workspaceId, error });
  }

  static shadowMerge(workspaceId: string): Promise<ShadowWorkspace> {
    return invoke('shadow_merge', { workspaceId });
  }

  static shadowAbort(workspaceId: string): Promise<ShadowWorkspace> {
    return invoke('shadow_abort', { workspaceId });
  }

  static shadowCleanup(workspaceId: string): Promise<void> {
    return invoke('shadow_cleanup', { workspaceId });
  }

  // T-E-C-09: 任务录屏回放
  static shadowRecord(
    workspaceId: string,
    kind: OperationKind,
    target: string,
    detail: string,
    success: boolean,
    message: string
  ): Promise<OperationRecord> {
    return invoke('shadow_record', { workspaceId, kind, target, detail, success, message });
  }

  static shadowRecordingList(workspaceId: string): Promise<OperationRecord[]> {
    return invoke('shadow_recording_list', { workspaceId });
  }

  static shadowRecordingClear(workspaceId: string): Promise<void> {
    return invoke('shadow_recording_clear', { workspaceId });
  }

  // -----------------------------------------------------------------------
  // T-E-C-10: 异步长任务 — 后台分步执行跨小时/跨天的复杂任务
  // -----------------------------------------------------------------------

  static longTaskCreate(
    goal: string,
    steps: StepInput[],
    workspaceId?: string | null,
    planId?: string | null
  ): Promise<LongTask> {
    return invoke('long_task_create', {
      goal,
      steps,
      workspaceId: workspaceId ?? null,
      planId: planId ?? null,
    });
  }

  static longTaskGet(taskId: string): Promise<LongTask | null> {
    return invoke('long_task_get', { taskId });
  }

  static longTaskList(status?: LongTaskStatus | null): Promise<LongTask[]> {
    return invoke('long_task_list', { status: status ?? null });
  }

  static longTaskSteps(taskId: string): Promise<LongTaskStep[]> {
    return invoke('long_task_steps', { taskId });
  }

  static longTaskStart(taskId: string): Promise<LongTask> {
    return invoke('long_task_start', { taskId });
  }

  static longTaskPause(taskId: string): Promise<LongTask> {
    return invoke('long_task_pause', { taskId });
  }

  static longTaskResume(taskId: string): Promise<LongTask> {
    return invoke('long_task_resume', { taskId });
  }

  static longTaskCancel(taskId: string): Promise<LongTask> {
    return invoke('long_task_cancel', { taskId });
  }

  static longTaskDelete(taskId: string): Promise<boolean> {
    return invoke('long_task_delete', { taskId });
  }
}

// -----------------------------------------------------------------------
// T-E-S-39: Persona types
// -----------------------------------------------------------------------

export interface PersonaConfig {
  soul_md: string | null;
  agents_md: string | null;
  tools_md: string | null;
}

// -----------------------------------------------------------------------
// v0.5: Writing-mode types
// -----------------------------------------------------------------------

export interface WritingTemplate {
  id: string;
  label: string;
  description: string;
  icon: string;
  body: string;
  placeholders: { name: string; hint: string; multiline: boolean }[];
}

export interface Document {
  id: string;
  title: string;
  template_id: string;
  content: string;
  word_count: number;
  memory_id: string | null;
  created_at: number;
  updated_at: number;
  metadata: Record<string, unknown> | null;
}

export interface DocumentExport {
  id: string;
  document_id: string;
  format: 'markdown' | 'html';
  body: string;
  byte_size: number;
  exported_at: number;
}

export interface CreateDocumentRequest {
  title: string;
  template_id: string;
  content: string;
  metadata?: Record<string, unknown> | null;
}

// -----------------------------------------------------------------------
// v0.5: Work-mode types
// -----------------------------------------------------------------------

export type WorkTaskStatus = 'todo' | 'doing' | 'done';

export interface WorkTask {
  id: string;
  title: string;
  description: string;
  status: WorkTaskStatus;
  priority: number;
  due_at: number | null;
  time_spent_ms: number;
  created_at: number;
  updated_at: number;
  completed_at: number | null;
  metadata: Record<string, unknown> | null;
}

export interface CreateTaskRequest {
  title: string;
  description: string;
  priority?: number;
  due_at?: number | null;
}

export interface UpdateTaskRequest {
  id: string;
  title?: string;
  description?: string;
  priority?: number;
  /** null = clear, undefined = leave unchanged */
  due_at?: number | null;
}

export interface MeetingMinutes {
  decisions: string[];
  actions: string[];
}

// -----------------------------------------------------------------------
// v0.5: Editor types
// -----------------------------------------------------------------------

export interface FileEntry {
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
}

export interface FileContent {
  path: string;
  content: string;
  size: number;
  modified: number;
}

export interface GitStatus {
  branch: string;
  entries: { code: string; path: string }[];
  clean: boolean;
}

export interface GitLogEntry {
  hash: string;
  short: string;
  subject: string;
  author: string;
  time: number;
}

export interface GitDiff {
  path: string;
  body: string;
}

// -----------------------------------------------------------------------
// v0.5: OS types
// -----------------------------------------------------------------------

export interface ShellExecRequest {
  argv?: string[];
  command?: string;
  cwd?: string;
  timeout_ms?: number;
}

export interface ShellOutput {
  argv: string[];
  stdout: string;
  stderr: string;
  exit_code: number;
  elapsed_ms: number;
  timed_out: boolean;
}

export interface NotifyRequest {
  title: string;
  body: string;
  level?: 'info' | 'success' | 'warning' | 'error';
}

// -----------------------------------------------------------------------
// T-E-C-14: 剪贴板智能监听类型(镜像 src-tauri/src/os/clipboard_watcher.rs)
// -----------------------------------------------------------------------

/**
 * T-E-C-14: 剪贴板内容类型(serde tag = "type", rename_all = "lowercase")。
 *
 * 后端用 `#[serde(tag = "type", rename_all = "lowercase")]` 序列化,
 * 前端据此做分支渲染。`code` 变体携带可选 `language`(fenced code block
 * 的语言标记,启发式检测时为 null)。
 */
export type ClipboardKind =
  | { type: 'code'; language: string | null }
  | { type: 'markdowntable' }
  | { type: 'json' }
  | { type: 'url' }
  | { type: 'tsvcsv' }
  | { type: 'email' }
  | { type: 'ip' }
  | { type: 'path' }
  | { type: 'other' };

/**
 * T-E-C-14: 剪贴板事件(镜像 src-tauri/src/os/clipboard_watcher.rs::ClipboardEvent)。
 *
 * 后端检测到有结构的剪贴板内容时,通过 `nebula://clipboard-detected`
 * 事件推送给前端。前端 toast 显示 `content_preview`,用户点击后把
 * `content_full` 注入到 ChatPanel input。
 */
export interface ClipboardEvent {
  /** 内容预览(前 200 字符),供 toast 显示。 */
  content_preview: string;
  /** 完整内容,前端点击通知后注入到 ChatPanel input。 */
  content_full: string;
  /** 检测到的内容类型。 */
  kind: ClipboardKind;
  /** Unix 毫秒时间戳。 */
  ts: number;
  /** 内容哈希,前端可据此去重。 */
  hash: number;
}

// -----------------------------------------------------------------------
// v0.5: Sync types
// -----------------------------------------------------------------------

export interface EncryptedEnvelope {
  v: number;
  salt: number[];
  nonce: number[];
  ciphertext: number[];
}

export interface EncryptRequest {
  plaintext_b64: string;
  local_secret_b64: string;
  peer_public_b64: string;
}

export interface EncryptResponse {
  envelope: EncryptedEnvelope;
  envelope_b64: string;
  fingerprint: string;
}

export interface DecryptRequest {
  envelope: EncryptedEnvelope;
  local_secret_b64: string;
  peer_public_b64: string;
}

export interface DecryptResponse {
  plaintext_b64: string;
}

export interface SendSealedRequest {
  plaintext_b64: string;
  local_secret_b64: string;
  peer_public_b64: string;
}

export interface SendSealedResponse {
  envelope_id: string;
  fingerprint: string;
}

// -----------------------------------------------------------------------
// T-S5-A-01: Device management types
// -----------------------------------------------------------------------

export interface DeviceInfo {
  device_id: string;
  public_key_b64: string;
  paired_at: number;
  revoked: boolean;
  revoked_at: number | null;
}

// -----------------------------------------------------------------------
// v1.3 P1-4: Injection scan result types
// -----------------------------------------------------------------------

export type InjectionSeverity = 'Low' | 'Medium' | 'High' | 'Critical';

export interface InjectionHit {
  detector: string;
  snippet: string;
  offset: number;
  severity: InjectionSeverity;
}

export interface DangerousCommandHit {
  pattern: string;
  snippet: string;
  offset: number;
}

export interface InvisibleChar {
  index: number;
  code_point: number;
  name: string;
}

export interface CredentialLeakHit {
  pattern: string;
  snippet: string;
  offset: number;
  severity: InjectionSeverity;
}

export interface InjectionScanResult {
  safe: boolean;
  injection_hits: InjectionHit[];
  dangerous_commands: DangerousCommandHit[];
  credential_leaks: CredentialLeakHit[];
  invisible_chars: InvisibleChar[];
  max_severity: InjectionSeverity | null;
  elapsed_us: number;
}

// -----------------------------------------------------------------------
// v1.3 P1-3: Sandbox types
// -----------------------------------------------------------------------

export type RiskLevel = 'Low' | 'Medium' | 'High';

export type SandboxPolicy = 'Strict' | 'Permissive' | 'LlmOnly';

export type CapabilityKind =
  | 'file:read'
  | 'file:write'
  | 'network'
  | 'subprocess'
  | 'env:read'
  | 'clipboard:read'
  | 'llm:call'
  | 'db:access';

export interface SandboxConfig {
  capabilities: { granted: CapabilityKind[] };
  policy: SandboxPolicy;
  timeout_ms: number;
  mem_limit_bytes: number;
  allow_filesystem: boolean;
}

// -----------------------------------------------------------------------
// v1.3: DID identity types
// -----------------------------------------------------------------------

export interface DidDocument {
  '@context': string[];
  id: string;
  verification_method?: {
    id: string;
    type: string;
    controller: string;
    publicKeyBase58: string;
  }[];
  authentication?: string[];
  key_agreement?: string[];
}

export interface GenerateDidResponse {
  did: string;
  public_key_b64: string;
  document: DidDocument;
}

export interface ResolveDidResponse {
  did: string;
  document: DidDocument;
}

// -----------------------------------------------------------------------
// v1.3: Skill audit types
// -----------------------------------------------------------------------

export interface SkillAuditEntry {
  id: string;
  skill_id: string;
  executed_at: number;
  input_summary: string;
  output_summary: string;
  duration_ms: number;
  sandbox_type: string;
  security_scan_result: string;
  success: boolean;
}

// -----------------------------------------------------------------------
// v1.3: Stream token types
// -----------------------------------------------------------------------

export interface StreamToken {
  text: string;
  done: boolean;
  incomplete: boolean;
}

/**
 * T-E-D-03: 桌面悬浮球状态机。
 * - idle: 空闲(绿色稳定)
 * - thinking: 思考中(橙色脉冲)
 * - executing: 执行中(霓虹橙快速闪烁)
 * - notification: 有通知(红色)
 * - working: 有后台任务(旋转动画 + 任务计数角标)
 */
export type BallState = 'idle' | 'thinking' | 'executing' | 'notification' | 'working';

export interface FloatingBallStatePayload {
  state: BallState;
  task_count?: number;
}

/**
 * T-E-D-10: Agent 工具调用事件（镜像 src-tauri/src/swarm/events.rs::SwarmEvent::AgentToolCall）。
 */
export interface AgentToolCall {
  agent_id: string;
  agent_role: string;
  tool_name: string;
  start_ts: number;
  end_ts: number;
  duration_ms: number;
  success: boolean;
  output_preview: string | null;
  error: string | null;
  task_id: string;
}

/**
 * T-E-D-07: Swarm 执行事件流(镜像 src-tauri/src/swarm/events.rs::SwarmEvent)。
 *
 * 后端用 `#[serde(tag = "kind", rename_all = "snake_case")]` 序列化,
 * 前端据此做分支渲染。`agent_kind` / `chosen_kind` 取值见 AgentKind
 * (lowercase:"generic" / "coder" / "writer" / "reviewer" 等)。
 * `method` 取值:"llm_arbitration" / "first_wins" 等。
 */
export type SwarmEvent =
  | { kind: 'agent_started'; agent_kind: string; task_id: string; timestamp: number }
  | {
      kind: 'agent_completed';
      agent_kind: string;
      task_id: string;
      success: boolean;
      error: string | null;
      timestamp: number;
    }
  | { kind: 'negotiation_started'; task_id: string; candidate_count: number; timestamp: number }
  | {
      kind: 'arbitration_resolved';
      task_id: string;
      chosen_kind: string;
      method: string;
      conflict_detected: boolean;
      timestamp: number;
    }
  | {
      kind: 'swarm_completed';
      task_id: string;
      success_count: number;
      failure_count: number;
      approved: boolean;
      timestamp: number;
    }
  | {
      kind: 'agent_tool_call';
      agent_id: string;
      agent_role: string;
      tool_name: string;
      start_ts: number;
      end_ts: number;
      duration_ms: number;
      success: boolean;
      output_preview: string | null;
      error: string | null;
      task_id: string;
    }
  | { kind: 'agent_output_chunk'; agent_id: string; delta: string; ts: number; task_id: string }
  | { kind: 'deadlock_detected'; cycle: string[]; task_id: string; timestamp: number }
  | { kind: 'tree_of_thoughts_started'; branches: number; task_id: string; timestamp: number }
  | {
      kind: 'path_completed';
      path_id: string;
      strategy: string;
      task_id: string;
      timestamp: number;
    };

/**
 * T-E-S-26: EventEnvelope — 协议化事件信封(镜像 src-tauri/src/swarm/events.rs::EventEnvelope)。
 *
 * 每个事件携带:
 * - `event_type`: 事件类型名(如 "AgentStarted", "SwarmCompleted")
 * - `payload`: 原始事件数据(SwarmEvent 序列化为 JSON Value)
 * - `trace_id`: OTel trace_id 或 fallback UUID(32 字符 hex)
 * - `timestamp`: Unix 毫秒时间戳
 */
export interface EventEnvelope {
  event_type: string;
  payload: SwarmEvent;
  trace_id: string;
  timestamp: number;
}

/**
 * T-S1-B-01b: `chat_stream` 命令的最终返回值（流结束后）。
 * 与后端 `commands::chat::ChatComplete` 对齐。
 */
export interface ChatComplete {
  model: string;
  content: string;
  role: string;
  reasoning_chain?: ReasoningChain;
  /** T-E-S-64: 反幻觉一致性报告(可选,由 chat_stream 在流结束后生成)。 */
  consistency?: ConsistencyReport;
  /** T-E-S-28: 本次 assistant 回复的 turn_id(UUID v4)。
   *  前端用它关联 👍/👎 标注按钮,调用 annotationUpsert 时回传。 */
  turn_id?: string;
}

// -----------------------------------------------------------------------
// T-E-S-28: 对话标注(good/bad)+ 持续改进
// -----------------------------------------------------------------------

/**
 * T-E-S-28: 一条对话标注(镜像 src-tauri/src/memory/annotations.rs::Annotation)。
 *
 * 后端 Rust 字段为 snake_case(serde 默认不重命名),前端须用相同键名。
 * `annotation` 取值 `"good"` / `"bad"`,与 SQL CHECK 约束对齐。
 * `created_at` 为 Unix 毫秒时间戳(chrono::Utc::now().timestamp_millis())。
 */
export interface Annotation {
  turn_id: string;
  annotation: 'good' | 'bad';
  comment?: string | null;
  agent_role?: string | null;
  model?: string | null;
  conversation_id?: string | null;
  created_at: number;
}

/**
 * T-E-S-28: 标注聚合统计(镜像 src-tauri/src/memory/annotations.rs::AnnotationStats)。
 *
 * `by_model` / `by_agent` 的 value 是 `[good_count, bad_count]` 二元数组
 * (Rust 端用 `(u32, u32)` tuple,serde 序列化为 JSON 数组)。
 * `model` / `agent_role` 为 NULL 的行归入 `"(unknown)"` 桶。
 */
export interface AnnotationStats {
  good: number;
  bad: number;
  total: number;
  by_model: Record<string, [number, number]>;
  by_agent: Record<string, [number, number]>;
}

// -----------------------------------------------------------------------
// T-E-S-24: 文件快照回滚类型
// -----------------------------------------------------------------------

export interface SnapshotInfo {
  id: string;
  backend: 'git' | 'copy';
  working_dir: string;
  files: string[];
  created_at: number;
}

// ---------------------------------------------------------------------------
// T-E-D-06: 文件拖拽吸收 + Windows 右键菜单类型
// ---------------------------------------------------------------------------

/**
 * T-E-D-06: `sponge_absorb_file` 命令返回值(镜像
 * src-tauri/src/commands/memory.rs::sponge_absorb_file 的 serde_json::Value)。
 *
 * `kind` 取值:`"inserted"` / `"merged"` / `"duplicate"` / `"deactivated"`。
 * `similarity` 在 `merged` / `duplicate` 时为 0-1 浮点,其余为 null。
 */
export interface SpongeAbsorbFileResult {
  id: string;
  kind: 'inserted' | 'merged' | 'duplicate' | 'deactivated';
  similarity: number | null;
  path: string;
}

/**
 * T-E-D-06: Windows 右键菜单安装/卸载/状态查询返回值。
 * `installed` 表示当前 HKCU 注册表项是否存在;
 * `error` 在非 Windows 平台或注册表访问失败时填充。
 */
export interface ContextMenuStatus {
  installed: boolean;
  error?: string | null;
}

// -----------------------------------------------------------------------
// v1.3: Skill meta extensions
// -----------------------------------------------------------------------

export type ActivationCondition =
  | { keyword: { pattern: string } }
  | { intent: { category: string } }
  | { context: { key: string; value: string } }
  | 'always';

export interface CreateSkillRequestV2 extends CreateSkillRequest {
  activation_condition?: ActivationCondition | null;
  platform?: string[] | null;
  min_confidence?: number | null;
}

// -----------------------------------------------------------------------
// T-E-S-59: 统一收件箱类型(跨渠道消息聚合)
// -----------------------------------------------------------------------

/**
 * T-E-S-59: 跨渠道统一消息(镜像 src-tauri/src/channel/inbox.rs::UnifiedMessage)。
 *
 * `source_channel` 取值:telegram / discord / webchat / web / jiuwenswarm。
 * `inbound=true` 表示收到的消息,`false` 表示发出的消息。
 * `read=false` 表示未读(前端可加粗显示 + 渠道色条)。
 */
export interface UnifiedMessage {
  id: string;
  source_channel: string;
  sender: string;
  content: string;
  timestamp_ms: number;
  conversation_id?: string | null;
  inbound: boolean;
  read: boolean;
  original_message_id?: string | null;
}

// -----------------------------------------------------------------------
// T-E-S-40: 多 provider keychain 掩码 API key 类型
// -----------------------------------------------------------------------

/**
 * T-E-S-40: 掩码 API key(镜像 src-tauri/src/commands/core.rs::MaskedApiKey)。
 *
 * 后端永不返回明文 — 仅返回 masked(如 `sk-****678`)+ 长度 + 前缀,
 * 供前端显示 "已配置" 状态。
 */
export interface MaskedApiKey {
  masked: string;
  length: number;
  prefix: string;
}

// -----------------------------------------------------------------------
// T-E-S-41: models.json 动态配置类型(镜像 src-tauri/src/llm/models_config.rs)
// -----------------------------------------------------------------------

/** Provider kind 闭集,对应 LlmGateway 4 条 dispatch 路径(kebab-case serde)。 */
export type ProviderKind = 'openai-compat' | 'anthropic' | 'ollama' | 'custom';

/** 模型定价(USD / 1M tokens)。 */
export interface Pricing {
  input_usd_per_1m: number;
  output_usd_per_1m: number;
}

/** 单个模型条目。 */
export interface ModelConfig {
  id: string;
  display_name: string;
  /** 上下文窗口(token 数),未知为 null。 */
  context_window?: number | null;
  /** 是否支持 reasoning 输出,未知为 null。 */
  supports_reasoning?: boolean | null;
  /** 单价(USD / 1M tokens),未知为 null(回退硬编码表)。 */
  pricing?: Pricing | null;
}

/** 单个 provider 条目。 */
export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  display_name: string;
  /** API base URL(Ollama 可为 null,回退 config.ollama_url)。 */
  base_url?: string | null;
  /** Keychain slot 名(命名 `provider:<id>`),为 null 表示无需 API key。 */
  api_key_keychain_slot?: string | null;
  /** 回退读取的环境变量名(如 `DEEPSEEK_API_KEY`)。 */
  api_key_env?: string | null;
  supports_tools: boolean;
  supports_streaming: boolean;
  /** 内置 provider(deepseek/anthropic/ollama)为 true,前端不可删除。 */
  is_builtin: boolean;
  models: ModelConfig[];
}

/** models.json 顶层结构。 */
export interface ModelsConfig {
  version: number;
  default_provider: string;
  default_model: string;
  providers: ProviderConfig[];
  /** ADR-003 v2: 本地 provider id(默认 "ollama")。 */
  local_provider: string;
  /** 本地分类器模型(默认 "qwen2.5:3b")。 */
  local_classifier_model: string;
  /** 本地进化模型(默认 "qwen2.5:7b")。 */
  local_evolution_model: string;
  /** 本地 Soul 编译模型(默认 "qwen2.5:3b")。 */
  local_soul_model: string;
  /** Swarm Worker 本地模型(默认 "qwen2.5:7b")。 */
  worker_local_model: string;
  /** 按 WorkType 分配的 override(key = WorkType::as_str())。 */
  work_type_overrides: Record<string, WorkTypeOverrideEntry>;
}

/**
 * M6 #83: WorkType 枚举(镜像 src-tauri/src/llm/dispatcher.rs::WorkType)。
 * serde rename_all = "snake_case"。
 * Evolution / SoulCompile / Classifier 为 local_only(强制本地路由)。
 */
export type WorkType =
  | 'chat'
  | 'swarm_worker'
  | 'swarm_synthesize'
  | 'master_task'
  | 'evolution'
  | 'soul_compile'
  | 'classifier';

/** 单个 WorkType override 条目(镜像 WorkTypeOverrideEntry)。 */
export interface WorkTypeOverrideEntry {
  provider: string;
  model: string;
  temperature?: number | null;
  max_tokens?: number | null;
}

/**
 * M6 #83: Provider 连通性测试结果(镜像 commands/models_config.rs::ProviderTestResult)。
 * - Ollama: GET {base_url}/api/tags,2s 超时。
 * - 远端: GET {base_url}/v1/models,5s 超时,401/403 也算连通。
 */
export interface ProviderTestResult {
  ok: boolean;
  status_code: number | null;
  latency_ms: number;
  error: string | null;
}

// T-E-B-09: 文件夹监控状态快照(镜像 src-tauri/src/memory/file_watcher.rs::WatchStatus)。
export interface WatchStatus {
  /** `true` 表示至少有一个 watcher 在运行且消费者 task 未被取消。 */
  active: boolean;
  /** 当前正在监控的目录(canonicalized 字符串形式)。 */
  paths: string[];
}

// -----------------------------------------------------------------------
// T-E-S-27: Trusted Diagnostics Channels 类型(镜像 src-tauri/src/diagnostics/events.rs)
// -----------------------------------------------------------------------

/** 诊断事件来源层。 */
export type DiagnosticOrigin =
  'kernel' | 'l4_value_layer' | 'acl' | 'injection_guard' | 'sidecar' | 'tracing_hook';

/** 可信级别。 */
export type TrustLevel = 'signed' | 'trusted' | 'unverified';

/**
 * T-E-S-27: 诊断事件(serde tag = "kind")。
 *
 * 每个变体携带 `seq: u64` 单调递增序号,前端可据此去重/排序。
 */
export type DiagnosticEvent =
  | { kind: 'l4_deny'; memory_id: string; reason: string; seq: number }
  | { kind: 'acl_rejected'; user: string; resource: string; seq: number }
  | { kind: 'injection_guard_hit'; input: string; pattern: string; seq: number }
  | { kind: 'sidecar_crash'; name: string; exit_code: number; seq: number }
  | { kind: 'tracing_warn'; target: string; message: string; seq: number }
  | { kind: 'dropped'; count: number; seq: number };

/** T-E-S-27: 诊断事件快照(镜像 src-tauri/src/commands/diagnostics.rs::DiagnosticsSnapshot)。 */
export interface DiagnosticsSnapshot {
  /** 当前已发出的最大 seq 序号。 */
  lastSeq: number;
  /** 事件列表(按 seq 降序,最新在前)。 */
  events: DiagnosticEvent[];
  /** 总容量(broadcast channel capacity)。 */
  capacity: number;
  /** 是否启用 diagnostics channel。 */
  enabled: boolean;
}

// ---------------------------------------------------------------------------
// T-E-S-62: doctor 健康检查类型(镜像 src-tauri/src/diagnostics/doctor.rs)
// ---------------------------------------------------------------------------

/** 子检查状态分级(serde rename_all = "lowercase")。 */
export type DoctorStatus = 'ok' | 'warn' | 'fail';

/** 单项子检查结果。 */
export interface DoctorCheck {
  /** 子检查名(如 "sqlite" / "ollama" / "lancedb")。 */
  name: string;
  /** 状态分级。 */
  status: DoctorStatus;
  /** 简短状态描述(中文)。 */
  message: string;
  /** fail/warn 时的修复建议(中文),ok 时为 undefined。 */
  suggestion?: string | null;
  /** 该项子检查耗时(毫秒)。 */
  latency_ms: number;
}

/** doctor 健康检查总报告。 */
export interface DoctorReport {
  /** 报告生成时间(Unix 时间戳,秒)。 */
  timestamp: number;
  /** 聚合后的整体状态:任一 fail → fail;任一 warn → warn;全 ok → ok。 */
  overall: DoctorStatus;
  /** 各子检查结果(顺序固定)。 */
  checks: DoctorCheck[];
  /** 总耗时(毫秒)。 */
  duration_ms: number;
}

/**
 * T-E-S-62: 运行 doctor 全子系统健康检查。
 *
 * 后端并发执行 10 个子检查(AppConfig / Keychain / SQLite / LanceDB /
 * Ollama / Gateway / Sidecar / IPC / 日志目录 / 备份目录),每项 ≤ 2s 超时,
 * 整体 ≤ 10s。返回结构化 `DoctorReport`,由前端 DoctorView 渲染。
 *
 * 任一子检查失败不导致整体命令失败 — 失败的子检查在 `checks[]` 中标记为
 * `fail`,前端据此提示用户。
 */
export async function doctorRun(): Promise<DoctorReport> {
  return invoke<DoctorReport>('doctor_run');
}

// ---------------------------------------------------------------------------
// T-E-D-07: 浮动进度窗 — 命令封装 + SwarmEvent 流订阅。
// ---------------------------------------------------------------------------

/**
 * T-E-D-07: 打开浮动进度窗 (360x180,右下角置顶透明)。
 *
 * @param taskId swarm 任务 ID(用于匹配 SwarmEvent 流)
 * @param title  可选任务标题,缺省为 "任务执行中"
 */
export async function openFloatingProgress(taskId: string, title?: string): Promise<void> {
  await invoke('open_floating_progress', { taskId, title: title ?? null });
}

/**
 * T-E-D-07: 取消正在执行的 swarm 任务。
 *
 * 通过 task_id 查找后端的 CancellationToken 并触发取消。返回 `true` 表示
 * 该任务存在并已取消;`false` 表示任务不存在(可能已完成或从未创建)。
 *
 * @param taskId 要取消的 swarm 任务 ID
 */
export async function swarmCancel(taskId: string): Promise<boolean> {
  return invoke<boolean>('swarm_cancel', { taskId });
}

/**
 * T-E-D-07 / T-E-S-26: 订阅 Swarm 执行事件流(Tauri 2.0 `ipc::Channel` 双向通道)。
 *
 * 后端 `subscribe_events` 命令在 swarm 执行的关键节点推送
 * `EventEnvelope<serde_json::Value>`(含 event_type / payload / trace_id / timestamp)。
 * 前端用此 wrapper 订阅,在 `cb` 中按 `envelope.payload.kind` 分支渲染进度。
 *
 * @param cb 每条 EventEnvelope 的回调
 * @returns unsubscribe 函数,调用后停止处理后续事件(组件卸载时调用)
 */
export async function subscribeEvents(cb: (envelope: EventEnvelope) => void): Promise<() => void> {
  const channel = new Channel<EventEnvelope>();
  channel.onmessage = (envelope) => cb(envelope);
  // Fire-and-forget:invoke 在后端循环结束(通道关闭 / send 失败)时 resolve。
  // 不 await 它 — 我们需要立即返回 unsubscribe 函数。
  invoke('subscribe_events', { on_event: channel }).catch(() => {
    /* 通道关闭 / 后端错误:静默忽略,组件已通过 unlisten 停止处理 */
  });
  return () => {
    // 停止处理后续事件(后端 send 仍会尝试,但 onmessage 不再回调)。
    channel.onmessage = () => {};
  };
}

// ---------------------------------------------------------------------------
// T-E-C-14: 剪贴板智能监听 — 事件订阅封装。
// ---------------------------------------------------------------------------

/**
 * T-E-C-14: 订阅 `nebula://clipboard-detected` 事件。
 *
 * 后端 `ClipboardWatcherEngine` 检测到有结构的剪贴板内容时,通过此事件
 * 推送 `ClipboardEvent` 给前端。前端在回调中显示 toast,用户点击后把
 * `content_full` 注入到 ChatPanel input。
 *
 * 用 `@tauri-apps/api/event` 的 `listen` 模式(与 subscribeBallState 一致)。
 *
 * @param cb 每条 ClipboardEvent 的回调
 * @returns unsubscribe 函数,在组件卸载时调用以释放监听器
 */
export async function listenClipboardDetected(
  cb: (event: ClipboardEvent) => void
): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  const unlisten = await listen<ClipboardEvent>('nebula://clipboard-detected', (event) => {
    if (event.payload) cb(event.payload);
  });
  return unlisten;
}

// ---------------------------------------------------------------------------
// M6 #82: Master 编排 + L4 审批 — 类型与 API(镜像 src-tauri/src/swarm/master.rs)
// ---------------------------------------------------------------------------

/**
 * M6 #82: MasterOrchestrator 子任务执行模式。
 * 与 Rust `ExecuteMode` 对齐(serde rename_all="snake_case")。
 * - `standard`: 完整 RAG + Negotiator 协商
 * - `bypass`: 跳过 Negotiator,选最高置信度结果(零 LLM 仲裁)
 * - `plan`: L4 门禁预检(走 Standard 路径 + PlanEngine 准奏)
 */
export type ExecuteMode = 'standard' | 'bypass' | 'plan';

/**
 * M6 #82: MasterEvent — Master 编排生命周期事件(11 个变体)。
 *
 * 与后端 `src-tauri/src/swarm/master.rs::MasterEvent` 对齐,
 * 使用 `serde(tag = "kind", rename_all = "snake_case")` 序列化。
 *
 * 前端通过 `nebulaAPI.masterRun()` 的 `onEvent` 回调消费,
 * 按 `event.kind` 分支渲染时间线 + 处理 `user_confirmation_required` 审批交互。
 */
export type MasterEvent =
  | { kind: 'decompose_started'; task_id: string; input_summary: string; timestamp: number }
  | {
      kind: 'decompose_completed';
      task_id: string;
      node_count: number;
      edge_count: number;
      timestamp: number;
    }
  | { kind: 'decompose_failed'; task_id: string; error: string; timestamp: number }
  | {
      kind: 'layer_started';
      task_id: string;
      layer_index: number;
      node_count: number;
      timestamp: number;
    }
  | {
      kind: 'layer_completed';
      task_id: string;
      layer_index: number;
      success_count: number;
      failure_count: number;
      timestamp: number;
    }
  | {
      kind: 'sub_task_started';
      task_id: string;
      sub_task_id: string;
      worker_count: number;
      timestamp: number;
    }
  | {
      kind: 'sub_task_completed';
      task_id: string;
      sub_task_id: string;
      success: boolean;
      error: string | null;
      elapsed_ms: number;
      timestamp: number;
    }
  | { kind: 'synthesize_started'; task_id: string; result_count: number; timestamp: number }
  | { kind: 'synthesize_completed'; task_id: string; output_length: number; timestamp: number }
  | {
      kind: 'dag_failed';
      task_id: string;
      failed_sub_task_id: string;
      reason: string;
      timestamp: number;
    }
  | {
      kind: 'user_confirmation_required';
      task_id: string;
      sub_task_id: string;
      /** 给用户看的决策提示 */
      prompt: string;
      /** 防重放 nonce(UUID v4) */
      confirmation_id: string;
      /** 创建时间(用于 5 分钟超时判定) */
      created_at: number;
      timestamp: number;
    }
  | {
      kind: 'master_completed';
      task_id: string;
      total_sub_tasks: number;
      successful_sub_tasks: number;
      elapsed_ms: number;
      timestamp: number;
    };

/**
 * M6 #82: MasterOrchestrator 编排结果。
 *
 * `master_run` 命令在所有事件推送完毕后返回此对象,
 * 包含最终综合输出 + 子任务统计 + 是否降级为直通。
 */
export interface MasterReport {
  task_id: string;
  input: string;
  output: string;
  total_sub_tasks: number;
  successful_sub_tasks: number;
  failed_sub_tasks: number;
  elapsed_ms: number;
  /** 是否降级为直通(无拆解,直接 chat) */
  bypassed: boolean;
}

/**
 * M6 #82: Confirmation 状态(serde rename_all="snake_case")。
 * - `confirmed`: 已确认(首次提交)/ 在 5 分钟内可确认
 * - `expired`: 已过期(>5min)
 * - `not_found`: confirmation_id 不存在或已被消费
 * - `already_used`: 已被消费过(防重放)
 */
export type ConfirmationStatus = 'confirmed' | 'expired' | 'not_found' | 'already_used';

/**
 * M6 #82: 单条 pending confirmation 记录。
 * 由 `master_pending_confirmations` 返回,前端按 `created_at` + 5min 自行过滤。
 */
export interface PendingConfirmation {
  confirmation_id: string;
  /** ActionKind(serde snake_case,如 ai_self_modify / external_side_effect 等) */
  action_kind: string;
  /** RiskTier: low / medium / high */
  risk_tier: string;
  prompt: string;
  /** diff 展示(仅 AI 自改时非空) */
  diff: string | null;
  created_at: number;
  confirmed_at: number | null;
}

/** 5 分钟超时(毫秒),与后端 CONFIRMATION_TIMEOUT_MS 对齐。 */
export const CONFIRMATION_TIMEOUT_MS = 5 * 60 * 1000;

// ---------------------------------------------------------------------------
// M6 #78: 进化日志 + 回滚 — 类型与 API(镜像 src-tauri/src/evolution/engine/)
// ---------------------------------------------------------------------------

/**
 * M6 #78: EvolutionEngine 4 Phase 阶段标识。
 *
 * 与 Rust `EvolutionPhase` 对齐(serde rename_all="snake_case")。
 * - `extract`  : Phase 1 — 经验提取(L1 → L2 Experience)
 * - `compile`  : Phase 2 — 知识编译(L2 → L3 Facts)
 * - `reflect`  : Phase 3 — 元认知反思(L2+L3 → L5 Lessons)
 * - `soul`     : Phase 4 — Soul 反哺(L5 → SOUL.md evolution-append)
 */
export type EvolutionPhase = 'extract' | 'compile' | 'reflect' | 'soul';

/**
 * M6 #78: 单条进化日志条目(镜像 Rust `EvolutionLogEntry`)。
 *
 * 后端 `evolution_log_list` / `evolution_log_get` 命令返回此类型,
 * 由 EvolutionEngine 在每个 Phase 完成时写入 `evolution_log.md`。
 */
export interface EvolutionLogEntry {
  /** 唯一 ID(格式:`evolve_<YYYY-MM-DDTHH-MM-SSZ>_<phase>`) */
  entry_id: string;
  /** Phase 类型(决定可回滚性:仅 `soul` 阶段可回滚) */
  phase: EvolutionPhase;
  /** UTC 时间戳(RFC3339) */
  timestamp: string;
  /** master_id(domain 标识,与记忆 domain 字段对应) */
  master_id: string;
  /** 写入的 memory ID(Phase 4 Soul 为空字符串) */
  memory_id: string;
  /** 写入内容字节数 */
  content_bytes: number;
  /** SOUL.md 路径(仅 Phase 4 Soul 非空,其他阶段为空字符串) */
  soul_md_path: string;
}

/**
 * M6 #78: 回滚结果(镜像 Rust `RollbackResult`)。
 *
 * 后端 `evolution_rollback(n)` 返回此对象,
 * 前端用于展示回滚进度和失败原因(若 any)。
 */
export interface RollbackResult {
  /** 请求回滚的条数 N */
  requested_count: number;
  /** 实际回滚的条数 */
  rolled_back: number;
  /** 失败的条数(仅记 warning,不阻断) */
  failed: number;
  /** 每条回滚的 entry_id */
  entry_ids: string[];
  /** 总体 warnings(失败的 entry 原因) */
  warnings: string[];
}

// ---------------------------------------------------------------------------
// T-E-C-17: IM 扫码绑定类型(镜像 src-tauri/src/im/mod.rs)
// ---------------------------------------------------------------------------

/**
 * IM 平台标识。serde lowercase,与 Rust `ImPlatform` 对齐。
 */
export type ImPlatform = 'feishu' | 'wecom' | 'dingtalk';

/**
 * IM 消息等级(影响部分平台的颜色标记)。
 */
export type ImMessageLevel = 'info' | 'warning' | 'error';

/**
 * 绑定类型。Rust 端 `#[serde(tag = "kind", rename_all = "snake_case")]`,
 * Phase 1 仅 Webhook;OAuthUser 为 Phase 2 预留。
 */
export type BindingKind =
  | { kind: 'webhook'; url: string }
  | {
      kind: 'oauth_user';
      open_id: string;
      display_name: string;
      has_refresh_token: boolean;
    };

/**
 * 一条 IM 绑定记录(镜像 Rust `ImBinding`)。
 */
export interface ImBinding {
  id: string;
  platform: ImPlatform;
  kind: BindingKind;
  display_name: string;
  enabled: boolean;
  /** 创建时间(Unix 毫秒)。 */
  created_at: number;
  /** 上次成功发送时间(Unix 毫秒),未发送过为 null。 */
  last_used_at: number | null;
}

/**
 * `im_create_webhook_binding` 命令的请求 DTO。
 */
export interface CreateImWebhookBindingRequest {
  /** 平台('feishu' / 'wecom' / 'dingtalk')。 */
  platform: ImPlatform;
  /** Webhook URL(公网,SSRF 校验)。 */
  url: string;
  /** 用户可读名称(如 "团队群"),可空。 */
  display_name?: string;
}

/**
 * `im_broadcast` 命令的请求 DTO。
 */
export interface ImBroadcastRequest {
  title: string;
  body: string;
  /** 可选 markdown 正文(优先于 body,平台支持时使用)。 */
  markdown?: string;
  /** 等级(默认 'info')。 */
  level?: ImMessageLevel;
}

/**
 * `im_broadcast` 命令的返回结果。
 */
export interface ImBroadcastResult {
  /** 成功发送的绑定数。 */
  success: number;
  /** 失败的绑定数(详情通过 tracing::warn 记录)。 */
  failure: number;
}

// -----------------------------------------------------------------------
// T-E-B-01 / T-E-B-05: Wiki 笔记类型
// -----------------------------------------------------------------------

/** Wiki 笔记元数据(对齐 Rust WikiNote 结构)。 */
export interface WikiNote {
  id: string;
  turn_id: string | null;
  title: string;
  slug: string;
  tags: string[];
  path: string;
  created_at: number;
  updated_at: number;
  importance: number;
}

/**
 * T-E-B-13: 知识卡片(镜像 src-tauri/src/wiki/mod.rs::KnowledgeCard)。
 *
 * 后端 `wiki_get_card(slug)` 命令返回此类型,供 KnowledgeCardDialog 渲染。
 *
 * - `note`:笔记元数据(无 body)。
 * - `body`:从 storage 读取的 markdown 正文(供前端 markdown 渲染)。
 * - `definition`:正文第一行,用作卡片头部摘要。
 * - `related_entities`:正文中的 `[[xxx]]` 双向链接 slug 列表(不去重)。
 * - `backlinks`:反向链接的 note id 列表(指向本笔记的笔记 UUID)。
 * - `source`:来源标识("wiki" / "memory" / "import")。
 */
export interface KnowledgeCard {
  note: WikiNote;
  body: string;
  definition: string | null;
  related_entities: string[];
  backlinks: string[];
  source: string;
}

/**
 * T-E-B-01 / T-E-B-03: `wiki_read` 命令响应(笔记元数据 + Markdown 正文)。
 * 供 MemoryInspector 编辑入口加载当前 body 作为 textarea 初始值。
 */
export interface WikiNoteReadResponse {
  note: WikiNote;
  markdown: string;
}
