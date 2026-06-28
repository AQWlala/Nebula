/**
 * Tauri Command 封装
 *
 * v0.2 新增 4 个命令：
 * - reflect_now: 手动触发一次反思
 * - list_reflections: 列出最近的反思
 * - metrics: 读取进程级指标
 * - migration_status: 读取 schema 迁移状态
 *
 * v0.3 新增 Skill CRUD + DTO 修复：
 * - 所有 Tauri command 的命名参数统一为 `req: T`（Tauri 会把
 *   Rust 端的 `request: T` 自动序列化为 JS 端的 `req`）。
 * - 之前的 `memory_store({content, layer, memoryType})` 与
 *   Rust 签名不匹配，已修正为 `{req: StoreMemoryRequest}`。
 * - 新增 5 个 Skill 命令：create / use / rate / list / search。
 *
 * v0.5 新增三模式 + 编辑器 + OS + 同步：
 * - 写作模式：templates / documents / export。
 * - 工作模式：kanban + 时间追踪 + 会议纪要。
 * - 编辑器：read / write / list / git status / log / diff / commit。
 * - OS：clipboard / shell exec / notify。
 * - 同步：E2EE 加密 / 解密 / inbox。
 */
import { invoke } from '@tauri-apps/api/core';

// v1.0.1 P0#12: a thin wrapper around `invoke` that swallows the
// missing-Tauri-runtime case (e.g. when the component is rendered in
// a browser preview, Storybook, or unit test) and returns `null`.
// Use this for one-off commands; for typed access prefer the
// `NineSnakeAPI` static methods below.
export async function invokeTauri<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T | null> {
  try {
    return (await invoke(cmd, args)) as T;
  } catch {
    return null;
  }
}

export type Layer = 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5' | 'L6' | 'L7';
export type MemoryType =
  | 'Semantic'
  | 'Episodic'
  | 'Procedural'
  | 'Emotional'
  | 'Metacognitive';

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
}

export interface ChatRequest {
  message: string;
  conversation_id?: string;
}

export interface ChatResponse {
  model: string;
  role: string;
  content: string;
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

/** v1.0.1 (P0#08): per-agent result row.  The backend may now
 *  return stdout / stderr / elapsed_ms / status for each agent
 *  so the UI can show expandable failure details and a
 *  per-agent retry button.  All optional fields default to
 *  "unknown" — older backends only fill `agent` + `content`. */
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
  tag?: string | null;
  limit?: number;
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

export class NineSnakeAPI {
  static chat(req: ChatRequest): Promise<ChatResponse> {
    return invoke('chat', { request: { user_message: req.message, conversation_id: req.conversation_id } });
  }

  /**
   * v0.3 fix: the Rust command signature is
   * `memory_store(state, request: StoreMemoryRequest)`. Tauri maps
   * the snake-case parameter `request` to the JS key `request` (not
   * `req`), so we must send `{ request: ... }` — sending the raw
   * fields was the v0.1 / v0.2 bug.
   */
  static memoryStore(req: StoreMemoryRequest): Promise<StoreMemoryResponse> {
    return invoke('memory_store', { request: req });
  }

  static memorySearch(req: SearchRequest): Promise<SearchResponse> {
    return invoke('memory_search', { request: { query: req.query, k: req.k ?? 10, layer: req.layer } });
  }

  static memoryListRecent(limit: number): Promise<Memory[]> {
    return invoke('memory_list_recent', { limit });
  }

  static swarmExecute(task: SwarmTask): Promise<SwarmResult> {
    return invoke('swarm_execute', { task });
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

  static skillImport(url: string, source: string): Promise<ImportResult> {
    return invoke('skill_import', { identifier: url, source });
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
  // v1.3: Chat stream
  // -----------------------------------------------------------------------

  static chatStream(req: ChatRequest): Promise<StreamToken[]> {
    return invoke('chat_stream', { request: { user_message: req.message } });
  }
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
  | 'file:read' | 'file:write' | 'network' | 'subprocess'
  | 'env:read' | 'clipboard:read' | 'llm:call' | 'db:access';

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