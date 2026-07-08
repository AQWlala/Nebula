import { invoke } from '@tauri-apps/api/core';

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
  ingest_cost?: number | null;
}

export interface ChatRequest {
  message: string;
  conversation_id?: string;
}

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
  consistency?: ConsistencyReport;
}

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
  agents: string[];
  max_retries?: number;
}

export type ScenarioCategory = 'writing' | 'coding' | 'management';

export type ScenarioRole = 'writer' | 'coder' | 'manager';

export type ScenarioAgentKind =
  'generic' | 'coder' | 'writer' | 'reviewer' | 'researcher' | 'planner';

export interface AgentSpec {
  kind: ScenarioAgentKind;
  role: string;
  prompt_override?: string | null;
}

export interface ScenarioTemplate {
  id: string;
  name: string;
  description: string;
  category: ScenarioCategory;
  role: ScenarioRole;
  agents: AgentSpec[];
  system_prompt: string;
  user_prompt_template: string;
  tags: string[];
}

export interface InstantiateScenarioRequest {
  id: string;
  user_input: string;
}

export type LoopAutonomyLevel = 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5';

export interface LoopTemplateSummary {
  name: string;
  description: string;
  autonomy: LoopAutonomyLevel;
  cadence: string;
  budget_tokens: number;
  budget_minutes: number;
}

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

export interface PerfSample {
  rss_bytes?: number | null;
  virt_bytes?: number | null;
  cpu_pct?: number | null;
  over_budget?: boolean;
  ts_ms?: number;
}

export interface SidecarStatusInfo {
  kind: string;
  status: string;
  running: boolean;
  pid?: number | null;
  listenAddr?: string | null;
}

export type ReflectionKind = 'value_alignment' | 'outcome_review' | 'self_improvement';

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

export interface MetricsSnapshot {
  embedding_cache_hits: number;
  embedding_cache_misses: number;
  memory_stores_total: number;
  memory_searches_total: number;
  blackhole_compressions_total: number;
  reflections_generated_total: number;
  swarm_executions_total: number;
  chat_total: number;
  memory_search_latency_us_total: number;
  memory_search_latency_count: number;
  llm_chat_latency_us_total: number;
  llm_chat_latency_count: number;
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
  semantic_cache_hits: number;
  semantic_cache_misses: number;
  token_cost_usd: number;
  prefix_cache_hits: number;
  prefix_cache_cached_tokens: number;
  cost_saved_usd: number;
}

export interface MigrationStatus {
  current_version: number;
  applied: { version: number; name: string; applied: boolean }[];
}

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
  tag?: string | null;
  tags?: string[];
  tag_match?: 'any' | 'all';
  limit?: number;
}

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

export interface ArenaMatch {
  id: string;
  prompt: string;
  model_a: string;
  model_b: string;
  response_a: string | null;
  response_b: string | null;
  winner: 'a' | 'b' | 'tie' | null;
  auto_score_a: number | null;
  auto_score_b: number | null;
  created_at: number;
}

export type LeaderboardRow = [string, number];

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

export type RelationDimension = 'causal' | 'temporal' | 'entity' | 'hierarchical' | 'similarity';

export type GraphNodeRole = 'root' | 'inner' | 'leaf';

export interface GraphNode {
  id: string;
  depth: number;
  role: GraphNodeRole;
  layer: Layer;
  summary: string;
  importance: number;
}

export interface GraphEdge {
  src_id: string;
  dst_id: string;
  kind: string;
  dimension: RelationDimension;
  weight: number;
}

export interface GraphSnapshot {
  root_id: string;
  dimensions: RelationDimension[];
  nodes: GraphNode[];
  edges: GraphEdge[];
  truncated: boolean;
}

export interface MdrmQueryParams {
  max_depth?: number;
  max_nodes?: number;
  max_edges?: number;
  min_weight?: number;
}

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

export type OperationKind = 'file_create' | 'file_write' | 'file_delete' | 'command' | 'note';

export interface OperationRecord {
  seq: number;
  ts_ms: number;
  kind: OperationKind;
  target: string;
  detail: string;
  success: boolean;
  message: string;
}

export type LongTaskStatus =
  'pending' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled';

export type StepStatus = 'pending' | 'running' | 'done' | 'failed' | 'skipped';

export interface LongTask {
  id: string;
  goal: string;
  status: LongTaskStatus;
  workspace_id: string | null;
  plan_id: string | null;
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

export interface StepInput {
  description: string;
  program: string;
  args?: string[];
}

export interface PersonaConfig {
  soul_md: string | null;
  agents_md: string | null;
  tools_md: string | null;
}

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
  due_at?: number | null;
}

export interface MeetingMinutes {
  decisions: string[];
  actions: string[];
}

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

export interface ClipboardEvent {
  content_preview: string;
  content_full: string;
  kind: ClipboardKind;
  ts: number;
  hash: number;
}

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

export interface DeviceInfo {
  device_id: string;
  public_key_b64: string;
  paired_at: number;
  revoked: boolean;
  revoked_at: number | null;
}

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

export interface StreamToken {
  text: string;
  done: boolean;
  incomplete: boolean;
}

export type BallState = 'idle' | 'thinking' | 'executing' | 'notification' | 'working';

export interface FloatingBallStatePayload {
  state: BallState;
  task_count?: number;
}

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

export interface EventEnvelope {
  event_type: string;
  payload: SwarmEvent;
  trace_id: string;
  timestamp: number;
}

export interface ChatComplete {
  model: string;
  content: string;
  role: string;
  reasoning_chain?: ReasoningChain;
  consistency?: ConsistencyReport;
  turn_id?: string;
}

export interface Annotation {
  turn_id: string;
  annotation: 'good' | 'bad';
  comment?: string | null;
  agent_role?: string | null;
  model?: string | null;
  conversation_id?: string | null;
  created_at: number;
}

export interface AnnotationStats {
  good: number;
  bad: number;
  total: number;
  by_model: Record<string, [number, number]>;
  by_agent: Record<string, [number, number]>;
}

export interface SnapshotInfo {
  id: string;
  backend: 'git' | 'copy';
  working_dir: string;
  files: string[];
  created_at: number;
}

export interface SpongeAbsorbFileResult {
  id: string;
  kind: 'inserted' | 'merged' | 'duplicate' | 'deactivated';
  similarity: number | null;
  path: string;
}

export interface ContextMenuStatus {
  installed: boolean;
  error?: string | null;
}

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

export interface MaskedApiKey {
  masked: string;
  length: number;
  prefix: string;
}

export type ProviderKind = 'openai-compat' | 'anthropic' | 'ollama' | 'custom';

export interface Pricing {
  input_usd_per_1m: number;
  output_usd_per_1m: number;
}

export interface ModelConfig {
  id: string;
  display_name: string;
  context_window?: number | null;
  supports_reasoning?: boolean | null;
  pricing?: Pricing | null;
}

export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  display_name: string;
  base_url?: string | null;
  api_key_keychain_slot?: string | null;
  api_key_env?: string | null;
  supports_tools: boolean;
  supports_streaming: boolean;
  is_builtin: boolean;
  models: ModelConfig[];
}

export interface ModelsConfig {
  version: number;
  default_provider: string;
  default_model: string;
  providers: ProviderConfig[];
  local_provider: string;
  local_classifier_model: string;
  local_evolution_model: string;
  local_soul_model: string;
  worker_local_model: string;
  work_type_overrides: Record<string, WorkTypeOverrideEntry>;
}

export type WorkType =
  | 'chat'
  | 'swarm_worker'
  | 'swarm_synthesize'
  | 'master_task'
  | 'evolution'
  | 'soul_compile'
  | 'classifier';

export interface WorkTypeOverrideEntry {
  provider: string;
  model: string;
  temperature?: number | null;
  max_tokens?: number | null;
}

export interface ProviderTestResult {
  ok: boolean;
  status_code: number | null;
  latency_ms: number;
  error: string | null;
}

export interface WatchStatus {
  active: boolean;
  paths: string[];
}

export type DiagnosticOrigin =
  'kernel' | 'l4_value_layer' | 'acl' | 'injection_guard' | 'sidecar' | 'tracing_hook';

export type TrustLevel = 'signed' | 'trusted' | 'unverified';

export type DiagnosticEvent =
  | { kind: 'l4_deny'; memory_id: string; reason: string; seq: number }
  | { kind: 'acl_rejected'; user: string; resource: string; seq: number }
  | { kind: 'injection_guard_hit'; input: string; pattern: string; seq: number }
  | { kind: 'sidecar_crash'; name: string; exit_code: number; seq: number }
  | { kind: 'tracing_warn'; target: string; message: string; seq: number }
  | { kind: 'dropped'; count: number; seq: number };

export interface DiagnosticsSnapshot {
  lastSeq: number;
  events: DiagnosticEvent[];
  capacity: number;
  enabled: boolean;
}

export type DoctorStatus = 'ok' | 'warn' | 'fail';

export interface DoctorCheck {
  name: string;
  status: DoctorStatus;
  message: string;
  suggestion?: string | null;
  latency_ms: number;
}

export interface DoctorReport {
  timestamp: number;
  overall: DoctorStatus;
  checks: DoctorCheck[];
  duration_ms: number;
}

export type ExecuteMode = 'standard' | 'bypass' | 'plan';

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
      prompt: string;
      confirmation_id: string;
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

export interface MasterReport {
  task_id: string;
  input: string;
  output: string;
  total_sub_tasks: number;
  successful_sub_tasks: number;
  failed_sub_tasks: number;
  elapsed_ms: number;
  bypassed: boolean;
}

export type ConfirmationStatus = 'confirmed' | 'expired' | 'not_found' | 'already_used';

export interface PendingConfirmation {
  confirmation_id: string;
  action_kind: string;
  risk_tier: string;
  prompt: string;
  diff: string | null;
  created_at: number;
  confirmed_at: number | null;
}

export const CONFIRMATION_TIMEOUT_MS = 5 * 60 * 1000;

export type EvolutionPhase = 'extract' | 'compile' | 'reflect' | 'soul';

export interface EvolutionLogEntry {
  entry_id: string;
  phase: EvolutionPhase;
  timestamp: string;
  master_id: string;
  memory_id: string;
  content_bytes: number;
  soul_md_path: string;
}

export interface RollbackResult {
  requested_count: number;
  rolled_back: number;
  failed: number;
  entry_ids: string[];
  warnings: string[];
}

export type ImPlatform = 'feishu' | 'wecom' | 'dingtalk';

export type ImMessageLevel = 'info' | 'warning' | 'error';

export type BindingKind =
  | { kind: 'webhook'; url: string }
  | {
      kind: 'oauth_user';
      open_id: string;
      display_name: string;
      has_refresh_token: boolean;
    };

export interface ImBinding {
  id: string;
  platform: ImPlatform;
  kind: BindingKind;
  display_name: string;
  enabled: boolean;
  created_at: number;
  last_used_at: number | null;
}

export interface CreateImWebhookBindingRequest {
  platform: ImPlatform;
  url: string;
  display_name?: string;
}

export interface ImBroadcastRequest {
  title: string;
  body: string;
  markdown?: string;
  level?: ImMessageLevel;
}

export interface ImBroadcastResult {
  success: number;
  failure: number;
}

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

export interface KnowledgeCard {
  note: WikiNote;
  body: string;
  definition: string | null;
  related_entities: string[];
  backlinks: string[];
  source: string;
}

export interface WikiNoteReadResponse {
  note: WikiNote;
  markdown: string;
}
