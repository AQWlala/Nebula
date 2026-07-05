# Nebula · API 参考 (API Reference)

> v1.0. 包含所有 Tauri commands 与 gRPC RPCs。

---

## 1. Tauri commands (43 个)

### 1.1 健康 / 启动 (v1.0)

| Command | 参数 | 返回 | 说明 |
| ------- | ---- | ---- | ---- |
| `bootstrap` | — | `void` | 前端握手。确认 Tauri 运行时在线。 |
| `health` | — | `{ status, version }` | 健康检查 + 版本号。 |
| `startup_report` | — | `StartupReport` | 冷启动分阶段耗时。 |
| `perf_sample` | — | `PerfSample` | 当前 RSS / CPU。 |
| `load_app_settings` | — | `AppSettingsDto` | 读 `settings.json`。 |
| `save_app_settings` | `AppSettingsDto` | `void` | 写 `settings.json`。 |

### 1.2 对话 / LLM

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `chat` | `{ user_message, system?, temperature? }` | `{ model, role, content }` |
| `llm_complete` | `(prompt: string, model?: string)` | `string` |
| `llm_chat` | `(messages: [role, content][], model?: string)` | `{ role, content, model, eval_count, total_duration_ns }` |
| `llm_embed` | `(text: string)` | `number[]` |

### 1.3 记忆 (L0–L7)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `memory_store` | `StoreMemoryRequest` | `StoreMemoryResponse` |
| `memory_search` | `SearchMemoryRequest` | `SearchMemoryHit[]` |
| `memory_get` | `(id: string)` | `Memory \| null` |
| `memory_list_recent` | `(limit: number)` | `Memory[]` |
| `memory_get_many` | `(ids: string[])` | `Memory[]` |
| `memory_update_importance` | `(id, importance)` | `Memory` |
| `memory_delete` | `(id: string)` | `boolean` |
| `memory_stats` | — | `{ total, by_layer }` |

`StoreMemoryRequest`：

```ts
{
  content: string;
  memory_type: 'Semantic' | 'Episodic' | 'Procedural' | 'Emotional' | 'Metacognitive';
  layer: 'L0' | 'L1' | 'L2' | 'L3' | 'L4' | 'L5' | 'L6' | 'L7';
  source?: string;
  metadata?: Record<string, unknown>;
}
```

### 1.4 蜂群 (Swarm)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `swarm_execute` | `SwarmTask` | `OrchestrationReport` |
| `swarm_list_agents` | — | `[kind, name, system, description][]` |
| `swarm_get_agent` | `(kind: string)` | `SwarmAgentInfo \| null` |

### 1.5 反思 (Reflection, L5)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `reflect_now` | — | `Reflection[]` |
| `list_reflections` | `(limit?: number)` | `Reflection[]` |
| `get_reflection` | `(id: string)` | `Reflection \| null` |

### 1.6 技能 (Skill)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `skill_create` | `CreateSkillRequest` | `Skill` |
| `skill_use` | `UseSkillRequest` | `SkillResult` |
| `skill_rate` | `RateSkillRequest` | `Skill` |
| `skill_list` | `ListSkillsRequest` | `Skill[]` |
| `skill_search` | `SkillSearchRequest` | `Skill[]` |

### 1.7 写作 (Writing, v0.5)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `writing_list_templates` | — | `WritingTemplate[]` |
| `writing_get_template` | `(id)` | `WritingTemplate \| null` |
| `writing_create_document` | `CreateDocumentRequest` | `Document` |
| `writing_update_document` | `(id, content)` | `Document` |
| `writing_get_document` | `(id)` | `Document \| null` |
| `writing_list_documents` | `(limit?)` | `Document[]` |
| `writing_delete_document` | `(id)` | `boolean` |
| `writing_export` | `(id, 'markdown' \| 'html')` | `DocumentExport` |

### 1.8 工作 (Work, v0.5)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `work_create_task` | `CreateTaskRequest` | `WorkTask` |
| `work_get_task` | `(id)` | `WorkTask \| null` |
| `work_list_tasks` | `(status?, limit?)` | `WorkTask[]` |
| `work_set_status` | `(id, status)` | `WorkTask` |
| `work_update_task` | `UpdateTaskRequest` | `WorkTask` |
| `work_delete_task` | `(id)` | `boolean` |
| `work_recommend_priority` | `(title, due_at?)` | `number` |
| `work_summarise_meeting` | `(transcript)` | `{ decisions[], actions[] }` |
| `work_start_timer` | `(id)` | `WorkTask` |
| `work_stop_timer` | — | `WorkTask \| null` |
| `work_add_time` | `(id, elapsed_ms)` | `WorkTask` |
| `work_active_timer` | — | `string \| null` |

### 1.9 编辑器 (Editor, v0.5)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `editor_workspace_root` | — | `string` |
| `editor_read` | `(path)` | `FileContent` |
| `editor_write` | `(path, content)` | `FileContent` |
| `editor_list` | `(maxDepth?)` | `FileEntry[]` |
| `git_status` | — | `GitStatus` |
| `git_log` | `(limit?)` | `GitLogEntry[]` |
| `git_diff` | `(path?)` | `GitDiff` |
| `git_commit` | `(message)` | `string` |

### 1.10 OS (v0.5)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `os_clipboard_read` | — | `string` |
| `os_clipboard_write` | `(text)` | `void` |
| `os_shell_exec` | `ShellExecRequest` | `ShellOutput` |
| `os_notify` | `NotifyRequest` | `void` |

### 1.11 同步 (Sync, v0.5)

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `sync_make_identity` | — | `{ public_key, secret_key }` |
| `sync_encrypt` | `EncryptRequest` | `EncryptResponse` |
| `sync_decrypt` | `DecryptRequest` | `DecryptResponse` |
| `sync_send` | `SendSealedRequest` | `SendSealedResponse` |
| `sync_recv` | `RecvRequest` | `RecvResponse` |
| `sync_ack` | `(envelope_id)` | `boolean` |

### 1.12 元数据

| Command | 参数 | 返回 |
| ------- | ---- | ---- |
| `metrics` | — | `MetricsSnapshot` |
| `migration_status` | — | `MigrationStatus` |

---

## 2. 错误契约

每个 command 的错误都是 `CommandError`：

```ts
{
  code: 'db' | 'lance' | 'llm' | 'memory' | 'swarm'
      | 'validation' | 'not_found' | 'permission'
      | 'internal' | 'unavailable';
  message: string;        // 安全消息，不含路径/密钥
  details?: string | null;
}
```

`code: 'internal'` 的 message 永远不包含绝对路径、用户目录名或任何 PII。

---

## 3. gRPC RPCs (22 个)

定义在 `src-tauri/proto/nebula.proto`。

> **v1.0 P0#12 状态**：22 个 RPC 的 **trait 方法体** 在
> `src/grpc/server.rs::nebulaServiceImpl` 中已完整实现，并
> 通过 Tauri command 委托到底层业务逻辑。但当前版本的
> **wire-shim**（`src/grpc/server.rs::handle_connection`）仍
> 为占位实现 — 通过 `grpcurl` 或 tonic 客户端发起的请求会立即
> 收到 `unimplemented` 状态。完整 HTTP/2 + gRPC 帧解码推迟到
> v1.1。集成测试 `tests/integration/grpc_wire_test.rs` 守护
> bind + accept 路径以及 22 个 trait 方法的完整性。

### 3.1 MemoryService

```protobuf
service MemoryService {
  rpc Store(StoreMemoryRequest) returns (StoreMemoryResponse);
  rpc Get(GetMemoryRequest) returns (Memory);
  rpc GetMany(GetManyRequest) returns (GetManyResponse);
  rpc Search(SearchRequest) returns (SearchResponse);
  rpc ListRecent(ListRecentRequest) returns (ListRecentResponse);
  rpc UpdateImportance(UpdateImportanceRequest) returns (Memory);
  rpc Delete(DeleteRequest) returns (DeleteResponse);
  rpc Stats(StatsRequest) returns (StatsResponse);
}
```

### 3.2 ChatService

```protobuf
service ChatService {
  rpc Chat(ChatRequest) returns (ChatResponse);
  rpc ChatStream(ChatRequest) returns (stream ChatChunk);
}
```

### 3.3 SwarmService

```protobuf
service SwarmService {
  rpc Execute(SwarmTask) returns (OrchestrationReport);
  rpc ListAgents(ListAgentsRequest) returns (ListAgentsResponse);
  rpc GetAgent(GetAgentRequest) returns (Agent);
}
```

### 3.4 SkillService

```protobuf
service SkillService {
  rpc Create(CreateSkillRequest) returns (Skill);
  rpc Use(UseSkillRequest) returns (SkillResult);
  rpc Rate(RateSkillRequest) returns (Skill);
  rpc List(ListSkillsRequest) returns (ListSkillsResponse);
  rpc Search(SkillSearchRequest) returns (SkillSearchResponse);
}
```

### 3.5 ReflectionService

```protobuf
service ReflectionService {
  rpc Trigger(TriggerRequest) returns (TriggerResponse);
  rpc ListRecent(ListRecentRequest) returns (ListRecentResponse);
  rpc Get(GetReflectionRequest) returns (Reflection);
}
```

### 3.6 Health

```protobuf
service Health {
  rpc Health(HealthRequest) returns (HealthResponse);
}
```

---

## 4. 前端封装

`src/lib/tauri.ts` 暴露一个 `nebulaAPI` 类，每个 command 一个静态方法：

```ts
import { nebulaAPI } from './lib/tauri';

const reply = await nebulaAPI.chat({ user_message: 'hi' });
const mems = await nebulaAPI.memoryListRecent(20);
```

类型全部导出 — IDE 自动补全。

---

## 5. 版本

| 版本 | 兼容 |
| ---- | ---- |
| 1.0 | ✅ 当前 |
| 0.5 | ✅ 兼容（所有 command 保留） |
| 0.3 | ✅ 兼容（gRPC 保持） |

破坏性变更会在 CHANGELOG 标注 `**BREAKING**`。
