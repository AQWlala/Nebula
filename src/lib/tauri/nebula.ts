import { invoke, Channel } from '@tauri-apps/api/core';
import type {
  ChatRequest, ChatResponse, StoreMemoryRequest, StoreMemoryResponse,
  SearchRequest, SearchResponse, Memory, SwarmTask, SwarmResult,
  ScenarioCategory, ScenarioTemplate, InstantiateScenarioRequest,
  LoopTemplateSummary, Reflection, SelfReflection, SidecarStatusInfo,
  MetricsSnapshot, MigrationStatus, Skill, SkillResult, CreateSkillRequest,
  UseSkillRequest, RateSkillRequest, ListSkillsRequest, SkillSearchRequest,
  ImportResult, TagCount, StreamToken, ChatComplete, BallState,
  WritingTemplate, Document, CreateDocumentRequest, DocumentExport,
  WorkTask, CreateTaskRequest, UpdateTaskRequest, MeetingMinutes,
  FileEntry, FileContent, GitStatus, GitLogEntry, GitDiff,
  ShellExecRequest, ShellOutput, NotifyRequest,
  EncryptRequest, EncryptResponse, DecryptRequest, DecryptResponse,
  SendSealedRequest, SendSealedResponse, DeviceInfo,
  GenerateDidResponse, ResolveDidResponse,
  SkillAuditEntry, PersonaConfig, MaskedApiKey, ModelsConfig,
  ProviderConfig, ProviderTestResult,
  ConnectionTestResult, ModelInfo,
  UnifiedMessage, Annotation, AnnotationStats,
  SnapshotInfo, SpongeAbsorbFileResult, ContextMenuStatus,
  DiagnosticsSnapshot, DiagnosticEvent, WatchStatus,
  MasterEvent, MasterReport,
  ConfirmationStatus, PendingConfirmation, EvolutionLogEntry,
  RollbackResult, ImBinding, CreateImWebhookBindingRequest,
  ImBroadcastRequest, ImBroadcastResult, WikiNote, KnowledgeCard,
  WikiNoteReadResponse, ShadowWorkspace, OperationRecord,
  LongTask, LongTaskStep, StepInput, GraphSnapshot,
  RelationDimension, MdrmQueryParams, LeaderboardRow,
  ExecuteMode, LongTaskStatus, OperationKind,
} from './types';

export class nebulaAPI {
  static chat(req: ChatRequest): Promise<ChatResponse> {
    return invoke('chat', {
      request: { user_message: req.message, conversation_id: req.conversation_id },
    });
  }

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

  static scenarioList(category?: ScenarioCategory | null): Promise<ScenarioTemplate[]> {
    return invoke('scenario_list', { category: category ?? null });
  }

  static scenarioGet(id: string): Promise<ScenarioTemplate | null> {
    return invoke('scenario_get', { id });
  }

  static scenarioInstantiate(req: InstantiateScenarioRequest): Promise<SwarmTask | null> {
    return invoke('scenario_instantiate', { request: req });
  }

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

  static reflectNow(): Promise<Reflection[]> {
    return invoke('reflect_now');
  }

  static listReflections(limit = 20): Promise<Reflection[]> {
    return invoke('list_reflections', { limit });
  }

  static selfReflectNow(): Promise<SelfReflection[]> {
    return invoke('self_reflect_now');
  }

  static sidecarListStatus(): Promise<SidecarStatusInfo[]> {
    return invoke('sidecar_list_status');
  }

  static sidecarStart(kind: string): Promise<boolean> {
    return invoke('sidecar_start', { kind });
  }

  static sidecarStop(kind: string): Promise<boolean> {
    return invoke('sidecar_stop', { kind });
  }

  static sidecarRestart(kind: string): Promise<boolean> {
    return invoke('sidecar_restart', { kind });
  }

  static metrics(): Promise<MetricsSnapshot> {
    return invoke('metrics');
  }

  static migrationStatus(): Promise<MigrationStatus> {
    return invoke('migration_status');
  }

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

  static skillTags(): Promise<TagCount[]> {
    return invoke('skill_tags');
  }

  static skillImport(url: string, source: string): Promise<ImportResult> {
    return invoke('skill_import', { identifier: url, source });
  }

  static skillExportClawhub(
    skillId: string,
    outputPath?: string | null
  ): Promise<{ content?: string; path?: string }> {
    return invoke('skill_export_clawhub', {
      skill_id: skillId,
      output_path: outputPath ?? null,
    });
  }

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

  static deviceList(): Promise<DeviceInfo[]> {
    return invoke('list_devices');
  }

  static deviceRevoke(deviceId: string): Promise<boolean> {
    return invoke('revoke_device', { device_id: deviceId });
  }

  static generateDid(publicKeyB64?: string): Promise<GenerateDidResponse> {
    return invoke('generate_did', { public_key_b64: publicKeyB64 });
  }

  static resolveDid(did: string): Promise<ResolveDidResponse> {
    return invoke('resolve_did', { did });
  }

  static skillAuditList(limit = 50): Promise<SkillAuditEntry[]> {
    return invoke('skill_audit_list', { limit });
  }

  static skillAuditListForSkill(skillId: string, limit = 50): Promise<SkillAuditEntry[]> {
    return invoke('skill_audit_list_for_skill', { skill_id: skillId, limit });
  }

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

  static floatingChatOpen(): Promise<void> {
    return invoke('open_floating_chat');
  }

  static floatingBallOpen(): Promise<void> {
    return invoke('open_floating_ball');
  }

  static async subscribeBallState(onState: (s: BallState) => void): Promise<() => void> {
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<BallState>('nebula://ball-state', (event) => {
      if (event.payload) onState(event.payload);
    });
    return unlisten;
  }

  static inlineComplete(prefix: string): Promise<string | null> {
    return invoke('inline_complete', { prefix });
  }

  static inboxList(limit = 50, offset = 0, channel?: string | null): Promise<UnifiedMessage[]> {
    return invoke('inbox_list', { limit, offset, channel: channel ?? null });
  }

  static inboxSend(targetChannel: string, body: string): Promise<void> {
    return invoke('inbox_send', { targetChannel, body });
  }

  static inboxReply(messageId: string, body: string): Promise<void> {
    return invoke('inbox_reply', { messageId, body });
  }

  static inboxMarkRead(ids: string[]): Promise<void> {
    return invoke('inbox_mark_read', { ids });
  }

  static inboxUnreadCount(): Promise<number> {
    return invoke('inbox_unread_count');
  }

  static directedEdit(selected: string): Promise<string> {
    return invoke('directed_edit', { selected });
  }

  static setProviderApiKey(provider: string, value: string): Promise<void> {
    return invoke('set_provider_api_key', { provider, value });
  }

  static getProviderApiKey(provider: string): Promise<MaskedApiKey | null> {
    return invoke('get_provider_api_key', { provider });
  }

  static watchStart(paths: string[]): Promise<void> {
    return invoke('watch_start', { paths });
  }

  static watchStop(): Promise<void> {
    return invoke('watch_stop');
  }

  static watchStatus(): Promise<WatchStatus> {
    return invoke('watch_status');
  }

  static watchListPaths(): Promise<string[]> {
    return invoke('watch_list_paths');
  }

  static clipboardWatchStart(): Promise<void> {
    return invoke('clipboard_watch_start');
  }

  static clipboardWatchStop(): Promise<void> {
    return invoke('clipboard_watch_stop');
  }

  static clipboardWatchStatus(): Promise<boolean> {
    return invoke('clipboard_watch_status');
  }

  static diagnosticsSubscribe(onEvent: (event: DiagnosticEvent) => void): Promise<void> {
    const channel = new Channel<DiagnosticEvent>();
    channel.onmessage = (event) => onEvent(event);
    return invoke('subscribe_diagnostics', { on_event: channel });
  }

  static diagnosticsSnapshot(limit?: number): Promise<DiagnosticsSnapshot> {
    return invoke('diagnostics_snapshot', { limit: limit ?? null });
  }

  static diagnosticsOpenLogs(): Promise<string | null> {
    return invoke('diagnostics_open_logs');
  }

  static modelsConfigLoad(): Promise<ModelsConfig> {
    return invoke('models_config_load');
  }

  static modelsConfigSave(config: ModelsConfig): Promise<ModelsConfig> {
    return invoke('models_config_save', { config });
  }

  static modelsConfigSetDefault(
    defaultProvider: string,
    defaultModel: string
  ): Promise<ModelsConfig> {
    return invoke('models_config_set_default', { defaultProvider, defaultModel });
  }

  static modelsConfigReload(): Promise<ModelsConfig> {
    return invoke('models_config_reload');
  }

  static modelsConfigAddProvider(provider: ProviderConfig): Promise<ModelsConfig> {
    return invoke('models_config_add_provider', { provider });
  }

  static modelsConfigRemoveProvider(providerId: string): Promise<ModelsConfig> {
    return invoke('models_config_remove_provider', { providerId });
  }

  static modelsConfigTestProvider(providerId: string): Promise<ProviderTestResult> {
    return invoke('models_config_test_provider', { providerId });
  }

  static setProviderKey(providerId: string, value: string): Promise<void> {
    return invoke('set_provider_key', { providerId, value });
  }

  static getProviderKey(providerId: string): Promise<MaskedApiKey | null> {
    return invoke('get_provider_key', { providerId });
  }

  // P0-1: 模型配置中心命令封装。

  /** 查询 keychain 中是否存在该 provider 的 key(不返回明文,只返回 bool)。 */
  static getProviderKeyStatus(providerId: string): Promise<boolean> {
    return invoke('get_provider_key_status', { providerId });
  }

  /** 测试 provider 连通性,返回延迟和状态。 */
  static testProviderConnection(
    providerId: string,
    baseUrl: string,
    apiKey: string | null
  ): Promise<ConnectionTestResult> {
    return invoke('test_provider_connection', {
      providerId,
      baseUrl,
      apiKey: apiKey ?? null,
    });
  }

  /** 自动拉取可用模型列表。 */
  static discoverModels(
    providerId: string,
    baseUrl: string,
    apiKey: string | null
  ): Promise<ModelInfo[]> {
    return invoke('discover_models', {
      providerId,
      baseUrl,
      apiKey: apiKey ?? null,
    });
  }

  /** 添加自定义 provider,返回新生成的 provider_id。 */
  static addCustomProvider(name: string, baseUrl: string, kind: string): Promise<string> {
    return invoke('add_custom_provider', { name, baseUrl, kind });
  }

  /** 删除 provider(内置或默认 provider 不可删除)。 */
  static removeProvider(providerId: string): Promise<void> {
    return invoke('remove_provider', { providerId });
  }

  /** 设置默认 provider 和 model。 */
  static setDefaultProvider(providerId: string, modelId: string): Promise<void> {
    return invoke('set_default_provider', { providerId, modelId });
  }

  /** 按 WorkType 设置路由。 */
  static setWorktypeRouting(workType: string, providerId: string): Promise<void> {
    return invoke('set_worktype_routing', { workType, providerId });
  }

  static personaReload(): Promise<PersonaConfig> {
    return invoke('persona_reload');
  }

  static personaGet(): Promise<PersonaConfig> {
    return invoke('persona_get');
  }

  static personaSetFile(kind: string, content: string | null): Promise<PersonaConfig> {
    return invoke('persona_set_file', { kind, content });
  }

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

  static annotationList(limit?: number | null): Promise<Annotation[]> {
    return invoke('annotation_list', { limit: limit ?? null });
  }

  static annotationStats(): Promise<AnnotationStats> {
    return invoke('annotation_stats');
  }

  static annotationExport(format: 'jsonl' | 'dify'): Promise<string> {
    return invoke('annotation_export', { format });
  }

  static exportChatDocx(params: {
    messages: { role: 'user' | 'assistant'; content: string; timestamp: number }[];
    options: { title?: string; include_timestamps?: boolean };
  }): Promise<{ file_path: string; byte_size: number }> {
    return invoke('export_chat_docx', {
      messages: params.messages,
      options: params.options,
    });
  }

  static snapshotCreate(workingDir: string, files: string[]): Promise<string> {
    return invoke('snapshot_create', { working_dir: workingDir, files });
  }

  static snapshotRollback(id: string): Promise<void> {
    return invoke('snapshot_rollback', { id });
  }

  static snapshotDiscard(id: string): Promise<void> {
    return invoke('snapshot_discard', { id });
  }

  static snapshotList(): Promise<SnapshotInfo[]> {
    return invoke('snapshot_list');
  }

  static spongeAbsorbFile(path: string): Promise<SpongeAbsorbFileResult> {
    return invoke('sponge_absorb_file', { path });
  }

  static contextMenuInstall(): Promise<ContextMenuStatus> {
    return invoke('context_menu_install');
  }

  static contextMenuUninstall(): Promise<ContextMenuStatus> {
    return invoke('context_menu_uninstall');
  }

  static contextMenuStatus(): Promise<ContextMenuStatus> {
    return invoke('context_menu_status');
  }

  static imCreateWebhookBinding(req: CreateImWebhookBindingRequest): Promise<ImBinding> {
    return invoke('im_create_webhook_binding', { request: req });
  }

  static imListBindings(): Promise<ImBinding[]> {
    return invoke('im_list_bindings');
  }

  static imDeleteBinding(id: string): Promise<void> {
    return invoke('im_delete_binding', { id });
  }

  static imSetEnabled(id: string, enabled: boolean): Promise<void> {
    return invoke('im_set_enabled', { id, enabled });
  }

  static imTestSend(id: string, title: string, body: string): Promise<void> {
    return invoke('im_test_send', { id, title, body });
  }

  static imBroadcast(req: ImBroadcastRequest): Promise<ImBroadcastResult> {
    return invoke('im_broadcast', { request: req });
  }

  static wikiList(limit?: number, offset?: number): Promise<WikiNote[]> {
    return invoke('wiki_list', { limit: limit ?? 50, offset: offset ?? 0 });
  }

  static wikiBacklinks(noteId: string): Promise<WikiNote[]> {
    return invoke('wiki_backlinks', { note_id: noteId });
  }

  static wikiGetCard(slug: string): Promise<KnowledgeCard> {
    return invoke('wiki_get_card', { slug });
  }

  static wikiRead(id: string): Promise<WikiNoteReadResponse> {
    return invoke('wiki_read', { id });
  }

  static wikiUpdateFromUser(noteId: string, newBody: string): Promise<void> {
    return invoke('wiki_update_from_user', { noteId, newBody });
  }

  static arenaCreateMatch(prompt: string, modelA: string, modelB: string): Promise<string> {
    return invoke('arena_create_match', { prompt, modelA, modelB });
  }

  static arenaVote(matchId: string, winner: 'a' | 'b' | 'tie'): Promise<void> {
    return invoke('arena_vote', { matchId, winner });
  }

  static arenaLeaderboard(): Promise<LeaderboardRow[]> {
    return invoke('arena_leaderboard');
  }

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

  static masterConfirm(confirmationId: string): Promise<ConfirmationStatus> {
    return invoke('master_confirm', { confirmationId });
  }

  static masterConfirmationStatus(confirmationId: string): Promise<ConfirmationStatus> {
    return invoke('master_confirmation_status', { confirmationId });
  }

  static masterPendingConfirmations(): Promise<PendingConfirmation[]> {
    return invoke('master_pending_confirmations');
  }

  static evolutionLogList(): Promise<EvolutionLogEntry[]> {
    return invoke('evolution_log_list');
  }

  static evolutionLogGet(entryId: string): Promise<EvolutionLogEntry | null> {
    return invoke('evolution_log_get', { entryId });
  }

  static evolutionRollback(n: number): Promise<RollbackResult> {
    return invoke('evolution_rollback', { n });
  }

  static evolutionEnabled(): Promise<boolean> {
    return invoke('evolution_enabled');
  }

  static evolutionSetEnabled(enabled: boolean): Promise<void> {
    return invoke('evolution_set_enabled', { enabled });
  }

  static soulSystemEnabled(): Promise<boolean> {
    return invoke('soul_system_enabled');
  }

  static soulSystemSetEnabled(enabled: boolean): Promise<void> {
    return invoke('soul_system_set_enabled', { enabled });
  }

  // T-D-C-08: master-orchestrator 运行时开关
  static masterOrchestratorEnabled(): Promise<boolean> {
    return invoke('master_orchestrator_enabled');
  }

  static masterOrchestratorSetEnabled(enabled: boolean): Promise<void> {
    return invoke('master_orchestrator_set_enabled', { enabled });
  }

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

  static mdrmTraceTemporal(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_trace_temporal', { memoryId, params: params ?? null });
  }

  static mdrmFindEntities(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_find_entities', { memoryId, params: params ?? null });
  }

  static mdrmTraceHierarchy(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_trace_hierarchy', { memoryId, params: params ?? null });
  }

  static mdrmFindSimilar(
    memoryId: string,
    params?: MdrmQueryParams | null
  ): Promise<GraphSnapshot> {
    return invoke('mdrm_find_similar', { memoryId, params: params ?? null });
  }

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
