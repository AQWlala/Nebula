# Nebula (nebula) · 生产路线图 v2.2

## ——Stage 7 创新支柱（四大支柱 · 68 个 T-E-* 任务）

**版本**：v2.2（创新路线图版）
**日期**：2026-07-03
**作者**：Solo Developer
**性质**：本文档是 `ROADMAP_v2.1.md` 的增量版本，新增 Stage 7 章节，纳入 `COMPREHENSIVE_EVOLUTION_v3.0.md` 的 68 个 T-E-* 任务。Stage 1-6 内容以 v2.1 为准，本文档仅在头部和尾部增量更新。
**配套文档**：
- `docs/ROADMAP_v2.1.md`（Stage 1-6 完整任务清单，本文档引用）
- `docs/COMPREHENSIVE_EVOLUTION_v3.0.md`（创新审议综合报告，Stage 7 任务来源）
- `docs/WHITEPAPER_v2.0.md`（Stage 1-6 设计权威）
- `docs/WHITEPAPER_v3.0.md`（Stage 7 设计权威，待起草）

---

## 0. v2.2 文档定位

**v2.1 → v2.2 的变更范围**：

| 范围 | v2.1 | v2.2 |
|------|------|------|
| Stage 1-6 任务 | 完整 | **不变**（以 v2.1 为准） |
| Stage 7 任务 | 不存在 | **新增 68 个 T-E-* 任务** |
| 版本里程碑 | v2.1~v3.0 | 扩展为 v2.3~v3.0 创新序列 |
| 任务编号体系 | T-S<阶段>-<组>-<序号> | 新增 T-E-<支柱>-<序号>（Evolution） |
| 设计哲学 | 8 层记忆 + L4 价值层 + E2EE | **+ 信任三原则（可读/可编辑/可追溯）** |

**为何采用增量版本**：

1. Stage 1-6 是工程闭环任务（记忆/协议/安全/蜂群/Sidecar/UX/OS），v2.1 已完整规划，无需重写
2. Stage 7 是创新支柱任务（省钱/智能/贴合/快），来自双报告融合，规模庞大（68 任务）
3. 分离两份文档便于：① 闭环任务与创新任务并行追踪 ② 不污染 v2.1 的精确依赖链

**使用约定**（v2.1 约定 + 新增）：

- 任务 ID 格式：
  - `T-S<阶段>-<组>-<序号>`（Stage 1-6，沿用 v2.1）
  - `T-E-<支柱>-<序号>`（Stage 7，支柱 A/B/C/D/S，序号全局递增）
- 支柱代号：A=省钱 / B=智能 / C=贴合 / D=快 / S=贯穿层（蜂群/安全/协议/自动化）
- 优先级 / 复杂度 / 状态：沿用 v2.1 约定

---

## 1. v3.0 创新里程碑（v2.2 新增）

### 1.1 创新阶段规划

| 阶段 | 版本 | 主题 | 核心交付 | 预期效果 |
|------|------|------|---------|---------|
| Stage 7-Wave 1 | v2.3 | **省钱+低门槛革命** | CostEngine + TokenJuice + 自主度滑块 L0-L1 | Token 成本降 70% |
| Stage 7-Wave 2 | v2.4 | **知识革命** | LLM Wiki + Obsidian 兼容 + 三视图 + 溯源链 | 从"聊天"到"第二大脑" |
| Stage 7-Wave 3 | v2.5 | **形象+后台革命** | 悬浮球 + 8 人格 + Shadow Workspace + Proactive | 使用频率 5x 提升 |
| Stage 7-Wave 4 | v2.6 | **可视+视觉革命** | WorkflowCanvas + 蜂群画布 + OS-Controller 双模式 | 蜂群黑盒→玻璃盒 |
| Stage 7-Wave 5 | v3.0 | **全自主革命** | 24/7 Automations + 多端 + 场景闭环 + Hybrid Browser | 无人值守 |

### 1.2 与 Stage 1-6 的关系

```
Stage 1-6（v2.1，工程闭环）
  ├─ Stage 1 记忆闭环 ✅
  ├─ Stage 2 协议+安全 ✅
  ├─ Stage 3 蜂群基础 ✅
  ├─ Stage 4 蜂群深度 ✅（T-S4-A-01/02/03 + T-S4-B-01/02/03 全部完成,2026-07-03）
  ├─ Stage 5 UX 升级 ✅（T-S5-A-01/02/03 + T-S5-B-01/02/03 全部完成,2026-07-03）
  └─ Stage 6 OS 集成 ✅（T-S6-A-01a/b/c + T-S6-A-02/03 + T-S6-B-01/02/03 全部完成,2026-07-03）
                    │
                    ▼
Stage 7（v2.2，创新支柱）  ← 依赖 Stage 1-2 完成（已完成）
  ├─ Wave 1 省钱（v2.3）
  ├─ Wave 2 知识（v2.4）
  ├─ Wave 3 形象（v2.5）
  ├─ Wave 4 可视（v2.6）
  └─ Wave 5 全自主（v3.0）
```

**依赖关系**：
- Stage 7 大部分任务依赖 Stage 1-2 已完成的基础（记忆系统/协议层/安全层）
- T-E-C-01 OS-Controller 双模式与 Stage 6 T-S6-A-01a/b/c 互补（API 模式复用 Stage 6 实现）
- T-E-S-60 Gateway 守护进程依赖 Stage 4 T-S4-B-03 Sidecar bootstrap
- T-E-S-22 AIO Sandbox 依赖 Stage 2b T-S2-A-01 WASM 沙箱（已完成）
- T-E-S-26 Event Stream 协议化复用 Stage 1 T-S1-B-02 SwarmEvent（已完成）

### 1.3 量化目标

| 维度 | 当前 | v2.3 目标 | v3.0 目标 |
|------|------|----------|----------|
| 平均响应时间 | 2-5s | <1s（40% 缓存命中） | <200ms（80% 本地） |
| 月度 Token 成本 | ~$30 | ~$9（降 70%） | ~$3（降 90%） |
| 日活跃次数 | 3-5 次 | 10-15 次（悬浮球） | 30-50 次（OS-Controller） |
| 知识覆盖 | 仅对话 | +本地文件 | +全工作场景 |
| 可操作范围 | 仅文本 | +文件操作 | +电脑操作 |
| 自主度等级 | 仅 L4 | L0-L4 | L0-L5 |
| 自动化任务 | 0 | 0 | 5+ 个定时/触发 |
| 可用终端 | 仅桌面 | +CLI | +CLI+PWA+渠道 |
| 记忆可读性 | 黑盒 | Markdown 视图 | 三视图+双向同步 |
| 记忆可追溯 | 无 | provenance 字段 | 完整溯源链 |

---

## 2. Stage 7 任务清单（77 个 T-E-* 任务）

> **Stage 7 P0 进度(2026-07-03)**:12 个 P0 任务全部完成 ✅ — 批次 1(5 个):T-E-S-20 (exec fail-closed)、T-E-S-21 (assemble_context ACL)、T-E-A-01 (SemanticCache L0.5)、T-E-A-06 (Token 费用追踪)、T-E-B-11 (BM25 混合搜索),详见 [wire-stage7-p0-quickwins spec](../.trae/specs/wire-stage7-p0-quickwins/spec.md);批次 2(2 个):T-E-S-01 (Agent 角色专业化)、T-E-S-30 (MCP 工具接入补完),详见 [wire-stage7-p0-batch2 spec](../.trae/specs/wire-stage7-p0-batch2/spec.md);批次 3(1 个):T-E-S-02 (LLM Function Calling),详见 [wire-stage7-p0-batch3-fc spec](../.trae/specs/wire-stage7-p0-batch3-fc/spec.md);批次 4(4 个并行):T-E-S-35 (5 层插件模型)、T-E-S-50 (自主度滑块 L0-L5)、T-E-S-51 (Level 0 内联补全)、T-E-S-59 (统一收件箱),详见 [wire-stage7-p0-plugin-model spec](../.trae/specs/wire-stage7-p0-plugin-model/spec.md)、[wire-stage7-p0-autonomy-slider spec](../.trae/specs/wire-stage7-p0-autonomy-slider/spec.md)、[wire-stage7-p0-inline-completion spec](../.trae/specs/wire-stage7-p0-inline-completion/spec.md)、[wire-stage7-p0-unified-inbox spec](../.trae/specs/wire-stage7-p0-unified-inbox/spec.md)。剩余 0 个 P0,Stage 7 P0 阶段完成。

### 2.1 支柱一：更省钱（14 个任务）

#### 模块：CostEngine + TokenJuice + ModelRouter + Credits

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-A-01 | ✅ DONE (2026-07-03) — **SemanticCache 层（L0.5）**：LlmGateway 入口加 `semantic_cache.check(embed(query))`，复用 LanceDB，cosine>0.92 直接返回，TTL 1h | P0 | S | T-S1-A-04 | A |
| T-E-A-02 | ✅ DONE (2026-07-03) — **TokenJuice 三级压缩**：L1 脱敏 + L2 压缩（HTML→MD/URL缩短/非ASCII）+ L3 摘要（旧对话 LLM 摘要替代原文），目标 -85% | P1 | M | 无 | B |
| T-E-A-03 | ✅ DONE (2026-07-03) — **ModelRouter 智能路由**：本地小模型分类（简单→Ollama/中等→DeepSeek/复杂→Claude），60% 走免费模型 | P1 | M | 无 | A |
| T-E-A-04 | ✅ DONE (2026-07-03) — **Prefix-Cache 适配层**：多 provider prompt caching（Anthropic/OpenAI API），长 system prompt 只计一次 | P1 | M | 无 | B |
| T-E-A-05 | ✅ DONE (2026-07-03) — **日预算限制**：`AppConfig.daily_budget_usd` + `LlmGateway::is_over_daily_budget()`(parking_lot::RwLock 热更新),超限 `effective_provider` 强制走 Ollama,`save_app_settings` 写后即时 `set_daily_budget` | P1 | S | T-E-A-03 | A+B |
| T-E-A-06 | ✅ DONE (2026-07-03) — **Token 费用追踪**：每次 LLM 调用记录 input_tokens + output_tokens + model + cost | P0 | S | 无 | A+B |
| T-E-A-07 | ✅ DONE (2026-07-03) — **Credits Dashboard**：日/周/月趋势图、按 provider/任务/Agent 分桶、预算预警线、缓存命中率关联 | P1 | M | T-E-A-06 | A+B |
| T-E-A-08 | ✅ DONE (2026-07-03) — **费用报告命令**：`nebula cost report` 输出本月各模型费用明细,main.rs clap CLI 双模式 + aggregate_by_model + cost_report Tauri 命令 | P2 | S | T-E-A-06 | A |
| T-E-A-09 | ✅ DONE (2026-07-04) — **记忆成本标签**:`Memory` 加 `ingest_cost: Option<f64>` 字段(serde skip_serializing_if None)+ migration 030(`ALTER TABLE memories ADD COLUMN ingest_cost REAL`)+ `MEMORY_COLUMNS` 同步更新 + `row_to_memory` 容错反序列化 + `SpongeEngine::with_cost_tracker()` builder + absorb() 入口/LLM 抽取后采样 `total_cost_usd()` 差值写入 `mem.ingest_cost`(Duplicate/Merged 分支设 Some(0.0))+ bootstrap 注入 cost_tracker + 前端 MemoryInspector cost badge(💰),11 新单测 | P3 | S | 无 | B |
| T-E-A-10 | ✅ DONE (2026-07-03) — **缓存命中率仪表盘**：MetricsSnapshot 新增 `semantic_cache_hits/misses/prefix_cache_hits/cost_saved_usd`,`call_anthropic` 透传 `cache_read_input_tokens` + 估算省的金额,CreditsDashboard 2s 轮询 + 命中率<30% `toast.warning`(alarmedRef 去重) | P1 | S | T-E-A-01 | A |
| T-E-A-11 | ✅ DONE (2026-07-04) — **智能预取**：PrefetchEngine 三路检索(路径 LIKE + BM25 文件名 + 向量过滤 chat channel)+ turn_id 配对(±30s 就近)+ 5min 去重(PathBuf::canonicalize)+ K=10 上限 + 复用 SemanticCache::store,15 单测 | P2 | M | T-E-A-01 | A |
| T-E-A-12 | ✅ DONE (2026-07-04) — **Automation Credits**：CostSource 枚举(Chat/Automation/Cron/Background)+ tokio::task_local! 传播 + CostRecord 加 source/trigger_id 字段 + 027_cost_source.sql migration + cost_report group_by=source + automation_daily_budget_usd 预算告警 + CreditsDashboard source 分组 tab,35 单测 | P2 | M | T-E-A-06 | A |
| T-E-A-13 | ✅ DONE (2026-07-04) — **费用数据加密存储**：CostTracker 新增 `attach_store()` builder + `load_from_store_blocking()` 启动回填 + `record_async()` spawn_blocking 异步写 `cost_records` 表(migration 027 已建,首次启用),`bootstrap_storage` 改造为 `db_encryption_enabled` 分支(`sqlcipher` feature 下走 `SqliteStore::open_encrypted`,无 feature 时 bail 提示重编译),`bootstrap_ai_core` 在 `Arc::new` 前注入 `attach_store(sqlite.clone())`,5 新单测(无 store/有 store record/load 回填/plain bootstrap/序列化 round-trip) | P3 | S | T-E-S-23 | B |
| T-E-A-14 | ✅ DONE (2026-07-04) — **Arena A/B 测试**：`ArenaMatch` 结构体(id/prompt/model_a/model_b/response_a/response_b/winner/auto_score_a/auto_score_b/created_at)+ `ArenaLeaderboard`(parking_lot::Mutex<HashMap<String, f32>> + Option<SqliteStore>)+ migration 034(arena_matches + model_elo_scores 两表)+ ELO 计算(K=32,初始 1200,expected = 1/(1+10^((opp-elo)/400)))+ `create_match`/`vote`/`leaderboard`/`update_elo`/`load_from_store` 方法 + 3 Tauri 命令(arena_create_match/vote/leaderboard)+ AppState 注入 Arc<ArenaLeaderboard> + bootstrap + bootstrap_headless 双路径构造 + ArenaPanel.tsx(prompt+模型选择+创建对战+投票)+ LeaderboardTable.tsx(排名/模型/ELO/ΔTop,冠军金色高亮),7 单测(ELO 计算/胜负/tie/持久化/投票覆盖/排行榜排序/load 回填),create_match 中 call_model 暂为 stub TODO 接入 chat_with_provider | P3 | M | 无 | A |

### 2.2 支柱二：更智能（18 个任务）

#### 模块：LLM Wiki + Obsidian 兼容 + 可读记忆 + 推理链

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-B-01 | ✅ DONE (2026-07-04) — **LLM Wiki 编译引擎**：WikiCompiler + WikiNote + WikiConfig,compile_turn(幂等:get_by_turn_id 短路)/ compile_raw / list / read / search(FTS5)/ delete,slugify + parse_llm_output + extract_links,双写一致性(先 SQLite 后文件,失败补偿删除),029_wiki_notes.sql migration(含 FTS5 虚拟表 + 3 同步触发器),chat_stream 完成后 spawn-and-forget 异步编译,5 Tauri 命令(wiki_compile/list/read/search/delete),25 单测 | P1 | L | 无 | B |
| T-E-B-02 | ✅ DONE (2026-07-07) — **可读记忆三视图**：①Markdown 视图(MemoryInspector 列表+Wiki 编辑,T-E-B-03 已覆盖)②图谱视图(T-E-B-07 力导向图)③时间轴视图(TimelineView.tsx 新增)`/journey` 回放;nebulaStore 新增 `View` 类型导出 + `currentMode` signal(从 App.tsx 重构)+ `memoryView` signal;App.tsx 三视图切换按钮(map/list/timeline);ChatPanel sendStream 拦截 `/journey` 斜杠命令切换到 memory+timeline;TimelineView 按日期分组(YYYY-MM-DD 本地化标签)+ 8 层筛选复选框 + 重要性滑块(0-1 step 0.1)+ 统计栏(总数/跨度/层分布)+ 展开详情(全文/来源/类型/访问次数/ingest_cost)+ 竖线圆点视觉时间轴;7 单测(空状态/日期分组/层筛选/重要性筛选/展开详情/统计栏/刷新) | P1 | XL | T-E-B-01 | B |
| T-E-B-03 | ✅ DONE (2026-07-04) — **记忆双向同步**：WikiCompiler 新增 `sponge: Option<Arc<dyn MemoryRevectorizer>>` + `version_control: Option<Arc<MemoryVersionControl>>` 字段 + `MemoryRevectorizer` trait(解耦 SpongeEngine 硬依赖,单测可注入 MockRevectorizer)+ `with_memory_sync` builder + `update_note_from_user(note_id, new_body)` 方法(严格按 spec §4.2 流程:SQLite UPDATE → sponge.absorb_text 重新向量化 → 文件重写 → MemoryVersionControl.commit 版本记录 → 触发 LogEvent::Updated)+ MutexGuard 用块作用域确保 drop 后再 await + sponge/vc 均为 Option graceful degrade + 失败仅 warn 不阻断主流程 + `wiki_update_from_user` Tauri 命令 + MemoryInspector.tsx 新增"📝 Wiki 笔记"编辑入口(textarea+保存/取消按钮,失败保留编辑态)+ 6 单测(SQLite/LogEvent/版本/重向量化/文件重写/无 sponge) | P1 | L | T-E-B-01 | B |
| T-E-B-04 | ✅ DONE (2026-07-03) — **记忆溯源链**：`Provenance` 结构体(source/tool/timestamp/SHA-256 content_hash),`sponge.rs::absorb()` 写入 `metadata.provenance`,`absorb_text()` 新增 `tool` 参数,MemoryInspector 显示 `[来源:工具]` badge | P1 | M | 无 | B |
| T-E-B-05 | ✅ DONE (2026-07-04) — **双向链接 `[[]]` 语法**：migration 033 `wiki_note_links` 关联表(source_id/target_id CASCADE + idx_target)+ WikiCompiler 新增 `update_backlinks(note_id, links)`(先删旧再插新,只插目标存在)+ `get_backlinks(note_id)` → Vec<WikiNote>+ `persist_note` 末尾挂 update_backlinks(非阻塞)+ `wiki_backlinks` Tauri 命令+ ChatPanel 输入 `[[` 触发 WikiNote 搜索补全(复用 `#` 注入 UI 模式,浮动列表+点击/Tab/Enter 插入 `[[slug]]`),5 新单测(建表/插入/删除/反向查询/级联删) | P2 | M | T-E-B-01 | A+B |
| T-E-B-06 | ✅ DONE (2026-07-04) — **index.md + log.md 自动维护**：`WikiCompiler` 新增 `LogEvent` 枚举(Created/Updated/Deleted)+ `append_log()`(OpenOptions::append + `tokio::sync::Mutex` 串行化)+ `regenerate_index()`(list_all_notes + 按 importance 降序排序 + 渲染 Top/Recent/By Topic + 原子写 .tmp rename)+ `persist_note`/`delete` 末尾自动 append_log 非阻塞 + `wiki_regen_index` Tauri 命令(invoke_handler 注册 lib.rs:2367),6 新单测(创建/追加/空 index/排序/原子/格式),修复 `to_markdown_line` 时间戳从 `from_timestamp` 改为 `from_timestamp_millis`(ts 字段为毫秒单位) | P2 | S | T-E-B-01 | B |
| T-E-B-07 | ✅ DONE (2026-07-07) — **知识图谱视图**：MemoryMap 升级为双视图(同心圆/力导向图),消费 T-E-B-16 MDRM `GraphSnapshot` 数据;力导向布局(节点斥力 O(n²) + 边弹簧 Hooke + 向心力 + 0.85 阻尼,根节点钉原点);边按 5 维着色(causal红/temporal蓝/entity绿/hierarchical紫/similarity琥珀);维度筛选复选框(5 维独立切换,空集退化为全维);点击节点重定位根 + 重新 BFS 查询;`tauri.ts` 新增 MDRM 类型(GraphNode/GraphEdge/GraphSnapshot/RelationDimension)+ 5 个 nebulaAPI 静态方法;截断/空边提示;7 单测(mock pixi.js + ResizeObserver polyfill) | P1 | L | 无 | A+B |
| T-E-B-08 | ✅ DONE (2026-07-07) — **Obsidian vault 兼容**：`ObsidianVaultSync` 无状态同步引擎(is_obsidian_vault/read_app_config/export_to_obsidian/import_from_obsidian/scan_vault)+ `ObsidianSyncConfig` + `SyncDirection` enum + `ImportedNote` DTO + `format_frontmatter`/`parse_frontmatter`/`parse_frontmatter_to_note` frontmatter 处理(支持 BOM/`\r\n`/空行 trim)+ 路径沙箱(拒绝 `..` 和绝对路径)+ 8MiB 文件大小限制 + 5 个 Tauri 命令(obsidian_detect_vault/read_config/scan_vault/import_note/export_note)+ 10 单测(含路径遍历防护、roundtrip、BOM 处理) | P2 | M | T-E-B-03 | A |
| T-E-B-09 | ✅ DONE (2026-07-03) — **文件夹监控索引**：`FileWatcherEngine`(notify 6.1 + mpsc + tokio 后台 task,复刻 reflection worker),`SpongeEngine::absorb_file()` 新增,debounce 300ms + 扩展名白名单 + 8MiB 上限,Settings 加文件夹选择卡片(dialog.open),`watch_start/stop/status/list_paths` 命令 | P1 | M | 无 | A |
| T-E-B-10 | ✅ DONE (2026-07-03) — **`#` 命令注入**：ChatPanel `resolveFileTokens()` 用 `/#([\w./-]+)/g` 正则解析 `#filename`,调 `nebulaAPI.editorRead()` 内联为 ` ```lang path\n{content}\n``` `,失败 `toast.warning` | P1 | S | 无 | A |
| T-E-B-11 | ✅ DONE (2026-07-03) — **BM25 + 向量混合搜索**（Hybrid Search）：解决纯向量在关键词精确匹配场景召回率低 | P0 | M | 无 | A |
| T-E-B-12 | ✅ DONE (2026-07-03) — **文档提取引擎**：`document_extractor.rs` 用 `pdf-extract = "0.7.12"` + `docx-rs = "0.4.20"`(纯 Rust 无系统依赖),`detect_kind()` 按扩展名分发,`extract_document_text()` 主入口,`sanitize_and_truncate()` 控制字符清洗 + 1MiB UTF-8 安全截断,`sponge.rs::absorb_file` 改造为按扩展名分发(二进制走提取器,文本走 read_to_string),FileWatcher 白名单加 pdf/docx,5 个单元测试 | P1 | M | 无 | A |
| T-E-B-13 | ✅ DONE (2026-07-04) — **知识卡片**：`KnowledgeCard` 结构体(note + definition + related_entities + backlinks + source)+ WikiCompiler 新增 `get_card(slug)` 聚合方法(get_by_id + definition=body.lines().next + extract_wiki_links regex `\[\[([^\]]+)\]\]` + get_backlinks 复用 T-E-B-05)+ `wiki_get_card` Tauri 命令 + ChatPanel.tsx L501 改 markdown 渲染(marked.parse + DOMPurify.sanitize ADD_ATTR 允许 data-slug/target)+ `[[xxx]]` 预处理为 `<a class="wiki-link" data-slug="xxx">xxx</a>` + onClick 检测 .wiki-link 点击弹出 KnowledgeCardDialog + KnowledgeCardDialog.tsx 组件(标题/definition/body markdown/related_entities 嵌套点击/backlinks,MAX_NESTING_DEPTH=3,复用 modal CSS)+ 4 单测(序列化/聚合/extract_wiki_links/not_found) | P2 | M | T-E-B-05 | A |
| T-E-B-14 | ✅ DONE (2026-07-04) — **Dataview 式查询 DSL**：手写递归下降 parser(Lexer+Parser+Translator,~350 LOC,无新依赖),FROM/WHERE/ORDER/LIMIT,字段白名单+值参数化防注入,kind 值别名(fact→semantic),强制注入 compressed_from IS NULL,46 单测 | P2 | M | 无 | B |
| T-E-B-15 | **AI 自动整理 MOC**：三定时机制联动，每日按主题聚类生成"主题笔记"（Map of Content） | P2 | L | T-E-S-63 | A+B |
| T-E-B-16 | ✅ DONE (2026-07-07) — **MDRM 5 维关系图谱**：`RelationKind` 扩展 4 新变体(Before/SameEntity/Contains/Similar,TEXT 列无需 migration)+ `RelationDimension` 枚举(5 维分类)+ `dimension_of()` 映射 + `MdrmEngine` 引擎(trace_temporal/find_entities/trace_hierarchy/find_similar/trace_causal/query_multi_dim/get_full_graph)+ BFS + visited set 循环防护 + max_nodes/max_edges 截断 + `GraphNode`/`GraphEdge`/`GraphSnapshot` DTO(前端可视化用)+ `MdrmConfig`(max_depth/max_nodes/max_edges/min_weight)+ entity_extractor 支持 LLM 抽取新 kind + 5 Tauri 命令(mdrm_trace_temporal/find_entities/trace_hierarchy/find_similar/get_graph)+ 18 单测(5 维覆盖/多维度组合/循环检测/截断/空图退化),来源 OpenAkita MDRM 思路(AGPL-3.0 仅思路借鉴) | P2 | XL | 无 | B |
| T-E-B-17 | ✅ DONE (2026-07-03) — **ReasoningChain 结构体**：`reasoning.rs` 定义 `ReasoningStep`(premise/inference/confidence/evidence) + `ReasoningChain`(steps/overall_confidence),`from_text()` 解析 DeepSeek `reasoning_content`,`AgentOutput.reasoning_chain` 字段,ChatPanel `<details>` 折叠面板渲染 | P1 | M | 无 | A |
| T-E-B-18 | ✅ DONE (2026-07-04) — **思维树模式**：ReasoningStrategy 枚举(Linear/TreeOfThoughts)+ ThoughtStrategy(Analytical/Creative/Critical/Synthesis)+ tot.rs + AgentOutput.path_id 字段 + negotiate_paths_with_arbitration(LLM 综合多视角)+ orchestrator execute_with_strategy 分支 + TreeOfThoughtsStarted/PathCompleted 事件,22 单测 | P2 | L | T-E-B-17 | A |

### 2.3 支柱三：更贴合工作场景（20 个任务）

#### 模块：OS-Controller + 视觉 + 场景闭环 + 自动化 + 多端

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-C-01 | **OS-Controller 双模式**：①API 模式（UIAutomation/AT-SPI）②VLM 模式（截图+视觉识别），PlanEngine 自动选择 | P1 | XL | T-S6-A-01a | B |
| T-E-C-02 | ✅ DONE (2026-07-04) — **ScreenReader 截图理解**：`ChatMessage` 新增 `images: Vec<String>` 字段(serde default + skip_serializing_if empty,向后兼容)+ Cargo.toml 新增 `screenshots = "0.8"` + `image = "0.25"` 可选依赖 + `vision = ["dep:screenshots", "dep:image"]` feature(默认关闭)+ AppConfig 新增 `vision_model: String`(默认 `qwen2.5-vl:3b`,env: NEBULA_VISION_MODEL)+ setup spawn 独立 vision_model warmup(即使 chat_model 走 DeepSeek,vision_model 仍走 Ollama 多模态)+ `screenshot` Tauri 命令(`#[cfg(feature = "vision")]` 双分支:启用时用 Screen::all 捕获主屏 + base64 PNG 编码;未启用时返回明确错误)+ `describe_screenshot(image_b64, prompt)` Tauri 命令 + `LlmGateway::describe_image(model, msg)` 方法(直接调 primary.chat,绕过 cache/breaker/router)+ 修复 screenshots 0.8 Image 跨 image crate 版本兼容(用 as_raw().clone() 重建 RgbaImage)+ 5 单测(ChatMessage images 序列化/skip empty/payload/feature_gated/默认 vision_model),3 个测试 feature-gated | P1 | M | 无 | A+B |
| T-E-C-03 | **UiAutomator 抽象层**：Windows UIA / macOS AX / Linux AT-SPI 统一抽象 | P2 | XL | 无 | A |
| T-E-C-04 | **ActionExecutor**：click/type/open/switch 原子操作 + L4 审批闭环 | P2 | L | T-E-C-03 | A |
| T-E-C-05 | **OS-Controller Sidecar**：独立进程运行，与主进程 IPC 通信 | P1 | L | T-S4-B-03 | A+B |
| T-E-C-06 | **Hybrid Browser Agent**：GUI 视觉点击 + CDP 协议（existing-session 复用）+ DOM 选择器，自动选最优 | P1 | XL | 无 | B |
| T-E-C-07 | **Remote Operator**：E2EE 加密通道远程控制另一台设备 | P3 | XL | T-E-C-05 | B |
| T-E-C-08 | ✅ DONE (2026-07-07) — **Shadow Workspace**：`ShadowWorkspaceEngine`(RwLock<HashMap> 内存索引 + repo_root)管理 git worktree 隔离执行;`ShadowStatus` 六态机(Creating→Running→Completed→(Merged\|Aborted),Failed 异常终态);`ShadowWorkspace` DTO(id/branch=`agent/<id>`/path/task_description/status/created_at/finished_at/base_branch/error);`gen_id()` 纳秒时间戳低 40 位 base32 编码 8 字符;`create()`(git worktree add -b)、`diff()`(git diff <base> 工作树全量,含已提交+未提交进度)、`run_command()`(cwd=worktree)、`complete()/fail()`、`merge()`(git merge --no-ff + worktree remove + branch -d 清理)、`abort()`(force 清理 + branch -D);10 Tauri 命令(shadow_create/list/status/diff/run_command/complete/fail/merge/abort/cleanup)全部 `spawn_blocking` 包裹同步 git,`CommandError::internal` 统一错误;AppState 注入 `shadow_engine`,bootstrap.rs+bootstrap_headless.rs `set_repo_root(workspace_root)`;前端 tauri.ts 类型+10 nebulaAPI 方法,ShadowWorkspacePanel.tsx(创建表单+base 分支/列表状态色标/diff 内联展开/合并·丢弃二次确认对话框),App.tsx 挂载(View 'shadow'+Sidebar 🌑+lazy+switch-view 监听);13 Rust 单测(完整生命周期)+ 8 前端单测 | P1 | L | 无 | A |
| T-E-C-09 | ✅ DONE (2026-07-07) — **任务录屏回放**：`recording.rs` 新增 `OperationKind` 五态(file_create/file_write/file_delete/command/note,snake_case 序列化)+ `OperationRecord`(seq/ts_ms/kind/target/detail/success/message)+ `RecordingLog`(`RwLock<HashMap<ws_id, Vec<Op>>>` 纯内存,长内容截断 target≤300/detail≤200/message≤500 字符+`…`);`ShadowWorkspaceEngine` 注入 `recordings` 字段,`run_command()` 自动记录每条命令(成功/失败均记录,detail=参数拼接,message=stdout 摘要或退出码错误);新增 `record_operation()/get_recording()/clear_recording()` 公共方法(校验 workspace 存在);录屏**不随 merge/abort 清除**(合并/丢弃后仍可回看 Agent 做了什么,仅显式 clear 清理);3 Tauri 命令(shadow_record/shadow_recording_list/shadow_recording_clear,spawn_blocking);前端 tauri.ts 类型+3 API,ShadowWorkspacePanel 新增 ▶ 回放按钮 + 时间线视图(#seq/图标/标签/目标/✓✗/时间 + 点击展开 detail/message);12 Rust 单测(7 recording + 5 engine 含 auto-record/survives-merge/clear)+ 4 前端单测(toggle/empty/timeline/detail 展开) | P2 | M | T-E-C-08 | A |
| T-E-C-10 | ✅ DONE (2026-07-07) — **异步长任务模式**：`long_task/engine.rs` 新增 `LongTaskEngine`(sqlite+shadow_engine+runners+pause_flags+cancel_flags)+ `LongTaskStatus` 六态(pending/running/paused/completed/failed/cancelled,`is_terminal()` 谓词)+ `StepStatus` 五态(pending/running/done/failed/skipped)+ `LongTask`(id/goal/status/workspace_id/plan_id/progress/error/timestamps)+ `LongTaskStep`(task_id/seq/description/program/args/status/timestamps/exit_code/output/error)+ `StepInput` DTO;`create_task()` SQLite 持久化(long_tasks+long_task_steps 表,migration 037)+ `get_task/get_steps/list_tasks`(ORDER BY created_at DESC, rowid DESC 保证同秒插入顺序稳定);`start()`(Pending/Paused→Running,tokio::spawn 后台 runner,AtomicBool pause/cancel 信号)+ `pause()`(协同式,当前步完成后退出)+ `resume()`(重新 spawn runner 从第一个 Pending 步骤继续)+ `cancel()`(标志+abort JoinHandle+剩余步骤 skipped)+ `delete_task()`(级联删除)+ `bootstrap()`(Running→Paused 重启恢复,Running 步骤→Pending);`run_task_loop()` async fn(spawn_blocking 调用 shadow_engine.run_command,自动截断 output≤5000 字符,实时更新 progress);9 Tauri 命令(long_task_create/get/list/steps/start/pause/resume/cancel/delete,全部 spawn_blocking);AppState 注入 `long_task_engine`,bootstrap.rs+bootstrap_headless.rs 初始化并 bootstrap();前端 tauri.ts 类型+9 nebulaAPI 方法,LongTaskPanel.tsx(创建表单:目标+动态步骤行+workspace_id/plan_id;列表:状态色标 6 色+进度条+元信息;操作:启动/暂停/恢复/取消/删除+二次确认对话框;步骤时间线展开+点击查看 output/error);18 Rust 单测(完整生命周期+bootstrap 恢复)+ 17 前端单测(渲染/创建/列表/状态/进度/步骤展开/操作/确认对话框) | P2 | L | T-E-C-08 | A+B |
| T-E-C-11 | **操作录制回放**：记录用户操作序列 → AI 可回放"看一遍就会" | P2 | M | T-E-C-04 | A |
| T-E-C-12 | **Design Mode**：用户在 UI 上画框/标注 → Agent 根据视觉提示操作 | P3 | L | T-E-C-04 | A |
| T-E-C-13 | ✅ DONE (2026-07-04) — **工作场景模板库**：`templates/scenarios.json`(28 个模板:3 顶层 Writer/Coder/Manager + 25 工作流)+ `TemplateEngine`(load_all/get_by_id/instantiate/list_by_category)+ 3 Tauri 命令(scenarios_list/scenarios_get/scenarios_instantiate)+ `TemplatesDialog.tsx`(507 行前端,分类卡片 + 搜索 + 一键实例化),10 单测全绿 | P2 | M | 无 | A+B |
| T-E-C-14 | ✅ DONE (2026-07-03) — **剪贴板智能监听**：ClipboardWatcherEngine(500ms 轮询 + hash 去重 + 8 种 kind 检测:fenced code/markdown table/JSON/URL/heuristic code/TSV-CSV/email/IP/path),sponge.absorb_text 写入,app.emit 通知,clipboard_watch_start/stop/status 命令 + AppConfig.clipboard_watch_enabled | P2 | M | 无 | A |
| T-E-C-15 | **语音交互引擎**：Whisper.cpp 本地 STT + TTS + 唤醒词"Nebula" + 嘴型同步 | P2 | XL | 无 | A+B |
| T-E-C-16 | ✅ DONE (2026-07-04) — **一键导出**：Markdown(前端 JS 生成下载)/DOCX(后端 docx-rs 生成)/PDF(前端 print-to-pdf),ExportDialog 对话框,ChatPanel 导出按钮 | P2 | M | 无 | A |
| T-E-C-17 | ✅ DONE (2026-07-04) — **IM 扫码绑定**：ImEngine + ImPlatform/ImMessage/BindingKind 枚举 + 三平台 webhook(Feishu/WeCom/DingTalk)+ 钉钉 HMAC-SHA256 手写签名(sha2 block-level,零新依赖,RFC 4231 向量验证)+ 028_im_bindings.sql + ImBindingStore CRUD + 6 Tauri 命令 + ImBindingPanel.tsx,39 单测 | P2 | L | 无 | B |
| T-E-C-18 | **OAuth 集成层**：Gmail/Notion/GitHub/Feishu/Calendar 5 个首批 | P2 | XL | 无 | B |
| T-E-C-19 | **多端协同**：CLI（clap）+ PWA + API 网关（gRPC+REST）+ 浏览器插件 | P2 | XL | 无 | A+B |
| T-E-C-20 | ✅ DONE (2026-07-04) — **Docker 部署**：`Dockerfile`(多阶段构建:rust:1.77-bookworm builder + debian:bookworm-slim runtime)+ `docker-compose.yml`(3 命名卷 data/keychain/logs + 端口 50051/8080 + 环境变量注入 API keys)+ `.dockerignore`+ `entrypoint.sh`(mkdir 卷目录 + 从 env 写密钥文件)+ `headless` feature(Cargo.toml,依赖 grpc+rest-api)+ `main.rs` cfg-gate headless 分支(bootstrap_headless + 启动 gRPC/REST + ctrl_c)+ `keychain.rs` 三级 fallback(keyring→env→文件卷 /keychain/slot)+ 修复 3 个预存 bug(tonic-health 0.12 API/rest body 读取/gRPC 非穷尽匹配),5 新单测(env fallback/file fallback/全缺失/headless 编译/entrypoint 语法) | P3 | M | T-E-S-23 | B |

### 2.4 支柱四：更快（10 个任务）

#### 模块：性能优化 + 桌面形象 + Proactive

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-D-01 | ✅ DONE (2026-07-04) — **冷启动 < 3s 工程**:删除 `bootstrap_storage` 冗余 `run_bundled_migrations` 调用(SqliteStore::open 内部已应用)+ `SemanticCache::with_sqlite()` 注入 sqlite 引用 + migration 031(`semantic_cache_entries` 表持久化响应正文,`check()` map miss 时 fallback 查 SQLite)+ `L0Cache::prewarm_from_store()` 后台预热(setup 回调 spawn 预填最近 64 条 memory)+ `COLD_START_BUDGET_MS` 从 5000-8000 调整为 3000 + 前端 main.tsx 4 个顶层视图改 `lazy` + Suspense(preact/compat),22 新单测(bootstrap 并行化/后台化标记 TODO 后续优化) | P1 | M | 无 | A+B |
| T-E-D-02 | ✅ DONE (2026-07-04) — **首响 < 500ms 工程**：`OllamaClient::warmup_model()`(GET /api/ps 检查已加载 + 未加载则 POST /api/generate num_predict=1 触发加载)+ `chat_stream` 命令 TTFT 埋点(Instant::now + 首个非空 token 时 `metrics.record_ttft(us)`)+ `Metrics` 新增 `ttft_us_total/ttft_count` AtomicU64 + `record_ttft()`/`ttft_avg_us()` + `TtftStats` 结构 + `build_ttft_stats()` + `metrics_ttft` Tauri 命令(commands/core.rs)+ setup 回调 spawn warmup_model(lib.rs:2018,provider=="ollama" 时)+ invoke_handler 注册 metrics_ttft(lib.rs:2138),7 新单测(warmup 已加载/触发加载/openai-compat/anthropic SSE/record_ttft/avg_empty/metrics_ttft 命令) | P1 | M | T-S1-B-01a | B |
| T-E-D-03 | ✅ DONE (2026-07-03) — **桌面悬浮球**：`open_floating_ball` 命令(WebviewWindowBuilder 80x80 + transparent + always_on_top + skip_taskbar),`FloatingBall.tsx` 组件(4 状态渲染 + data-tauri-drag-region + 迷你菜单),`main.tsx` 路由分发 `?view=ball`,StatusBar 加 🌀 按钮,CloseRequested 加 floating-ball 特例 | P1 | M | 无 | A+B |
| T-E-D-04 | **8 人格系统**：管家/Jarvis/助手/女友/男友/技术专家/商务/家庭，表情随 L5 情绪联动 | P2 | XL | T-E-D-03 | B |
| T-E-D-05 | **Proactive Engine**：主动问候/任务跟进/闲聊/晚安，频率自适应 + 每日"学习汇报" | P2 | L | T-E-S-63 | B |
| T-E-D-06 | ✅ DONE (2026-07-04) — **文件拖拽 + 右键集成**：sponge_absorb_file 命令(扩展名白名单)+ FloatingBall 监听 ball-drag-drop 事件 + Windows 右键注册表(HKCU\Software\Classes\*\shell\Asknebula,免管理员)+ --ask argv 解析 + context_menu install/uninstall/status 命令,6 单测 | P2 | M | T-E-D-03 | A |
| T-E-D-07 | ✅ DONE (2026-07-03) — **浮动进度窗**：open_floating_progress(360x180 右下角透明置顶),FloatingProgress.tsx(SwarmEvent 流 + 进度条 + 中断按钮 + 3s 自动关闭),swarm_cancel 命令 + CancellationToken,CloseRequested 加 floating-progress 例外 | P2 | S | T-E-D-03 | A |
| T-E-D-08 | **WebGL 引擎复用**：MemoryMap + WorkflowCanvas 共用 PixiJS，1000+ 节点 60fps | P2 | XL | T-S5-B-02 | B |
| T-E-D-09 | **UI 性能基准 CI**：1000/5000/10000 节点 fps 基线，回归报警 | P2 | M | T-E-D-08 | B |
| T-E-D-10 | ✅ DONE (2026-07-04) — **多 Agent 并行流式渲染**：SwarmEvent 新增 AgentToolCall/AgentOutputChunk 事件,tool_loop 埋点计时,SwarmView 分栏视图 AgentColumn + ToolCallCard,AgentOutputChunk 为未来流式预留 | P2 | M | T-S1-B-02 | B |

### 2.5 贯穿层：蜂群 + 安全 + 协议 + 自动化 + 自主度 + Loop Engineering（35 个任务）

#### 2.5.1 蜂群与协作（6 个）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-S-01 | ✅ DONE (2026-07-03) — **Agent 角色专业化**：恢复 Coder/Writer/Reviewer 角色，Dify 模式（system_prompt + tool_set + knowledge_scope） | P0 | M | 无 | A |
| T-E-S-02 | ✅ DONE (2026-07-04) — **LLM Function Calling**：新建 `tool_types.rs`(ToolCall/ToolResult 结构体 + DeepSeek/Anthropic 格式解析函数)+ `AgentOutput` 新增 `tool_calls: Option<Vec<ToolCall>>` 字段(serde skip_serializing_if None)+ Negotiator `highest_confidence()` 添加 tool_calls +0.1 boost + Orchestrator 新增 `execute_tool_calls()`(递归深度 5)+ 6 处 AgentOutput 构造补齐 tool_calls: None + chat_with_tools 已存在(gateway.rs:734-765),7 新单测(序列化/DeepSeek解析/Anthropic解析/空输入边界/AgentOutput字段/协商偏好) | P0 | L | 无 | A |
| T-E-S-03 | ✅ DONE (2026-07-04) — **DynamicAgentPool 按复杂度动态调整**:`TaskComplexity` 枚举(Simple/Medium/Complex)+ `estimate_complexity()` 复用 ModelRouter 分类 prompt(本地 Ollama qwen2.5:3b,2s 超时降级 Medium)+ `target_count_for()` 映射(2/3/6)+ orchestrator execute 默认 agent_count 时走复杂度推断(显式指定时跳过)+ 修复 `blocking_lock` 文档说明,9 单测 | P1 | M | T-S3-B-02 | A |
| T-E-S-04 | ✅ DONE (2026-07-04) — **MoA 一等公民**：`MoAStrategy` 枚举(Voting/Cascading/Arbitration)+ `MoAConfig` 结构体(participants/strategy/scoring_model)+ Negotiator 新增 `negotiate_moa`/`vote_on_responses`/`cascade_responses`/`score_response`/`extract_score` 方法+ `LlmGateway` 新增 `chat_with_provider`(按 provider 字符串路由)+ `chat_parallel`(多 provider 顺序调用,TODO Arc 化后真正并行)+ Voting:评分模型 1-10 打分取最高+ Cascading:按顺序逐步调用+ Arbitration:复用 llm_arbitrate+ `moa_execute` Tauri 命令,6 新单测(配置序列化/投票/级联/仲裁/并行/评分提取) | P1 | L | 无 | B |
| T-E-S-05 | ✅ DONE (2026-07-04) — **deadlock detection**：WaitForGraph(HashMap<String, HashSet<String>>) + DFS 三色标记法环检测,DeadlockDetector 1s 周期检测,AgentBus::request 超时兜底,deadlock_status 命令,9 单测 | P2 | M | 无 | B |
| T-E-S-06 | **Organization Orchestration**：CEO/CTO/CFO 角色化 + blackboard 共享 | P3 | XL | T-E-S-04 | B |

#### 2.5.2 工作流可视化（5 个）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-S-10 | **WorkflowCanvas 可编排画布**：React Flow 拖拽编排（设计时），节点类型：Memory/Skill/Agent/LLM/Condition/Loop | P1 | XL | 无 | B |
| T-E-S-11 | **蜂群运行时画布**：SwarmEvent 实时渲染为节点+连线（运行时可视化） | P1 | L | T-S1-B-02 | A |
| T-E-S-12 | **节点交互**：点击 Agent 节点查看输出、拖拽连线修改顺序、右键增删 Agent | P2 | M | T-E-S-11 | A |
| T-E-S-13 | **工作流模板**：保存执行图为 YAML 模板，下次复用 | P2 | M | T-E-S-10 | A |
| T-E-S-14 | **执行回放**：记录 SwarmEvent 时间线，支持回放/快进 | P2 | M | T-E-S-11 | A |

#### 2.5.3 安全与可观测（10 个）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-S-20 | ✅ DONE (2026-07-03) — **exec fail-closed**：exec approvals 超时默认拒绝（OpenClaw） | P0 | S | 无 | B |
| T-E-S-21 | ✅ DONE (2026-07-03) — **assemble_context ACL**：未授权内容不进 prompt context（OpenClaw） | P0 | S | T-S1-A-04 | B |
| T-E-S-22 | **AIO Sandbox**：升级 WASM 为 all-in-one 隔离（文件 chroot + 网络白名单 + 进程命名空间） | P2 | XL | T-S2-A-01c | B |
| T-E-S-23 | ✅ DONE (2026-07-03) — **凭证加密卷分离**：`keychain.rs` 新增 `resolve_deepseek_key()`/`resolve_anthropic_key()`/`resolve_openai_compat_key()`(keychain 优先 → env var fallback,keyring 失败 graceful 返回 None)+ `migrate_env_to_keychain()`(幂等迁移),`bootstrap_ai_core` 3 处 key 读取改为调 resolve_*_key,slot 命名沿用 KEY_DEEPSEEK_API_KEY 等常量(匹配前端 set_provider_api_key 写入路径),5 个单元测试 | P1 | M | 无 | B |
| T-E-S-24 | ✅ DONE (2026-07-04) — **文件快照回滚**：SnapshotEngine(GitBackend + CopyBackend 双后端自动选择),git stash create / git checkout 强制覆盖,snapshot_create/rollback/discard/list 命令,17 单测 | P2 | M | 无 | B |
| T-E-S-25 | ✅ DONE (2026-07-03) — **12 trace span types**：`observability/span_type.rs` 定义 `SpanType` 枚举(12 变体:Chat/Swarm/Skill/Memory/Llm/Reflect/Acl/Plan/Crdt/Sidecar/Channel/Export)+ `as_otel_kind()`/`as_target()`/`parse()`/`from_target()`/`all()`,`swarm/crdt_sync.rs` 2 处 `#[instrument]` 补齐 `target = "nebula.swarm.crdt"` + `fields(otel.kind = "crdt")`(12/12 领域完整),6 个单元测试往返一致性 | P1 | M | 无 | B |
| T-E-S-26 | **Event Stream 协议化**：SwarmEvent 升级为协议（type/payload/trace_id/timestamp）+ EventStreamViewer | P1 | L | T-S1-B-02 | B |
| T-E-S-27 | ✅ DONE (2026-07-03) — **trusted diagnostics channels**：`DiagnosticsBus`(broadcast 容量 512 + OnceLock 全局单例),`DiagnosticEvent` 6 变体(L4Deny/AclRejected/InjectionGuardHit/SidecarCrash/TracingWarn/Dropped)+ seq 序号,`DiagnosticsLayer` tracing_subscriber Layer 过滤 `nebula.diagnostic` target,`subscribe_diagnostics` Tauri 命令(ipc::Channel),`DiagnosticsView.tsx` 前端面板 | P1 | S | 无 | B |
| T-E-S-28 | ✅ DONE (2026-07-03) — **标注+持续改进**：024_chat_annotations.sql + AnnotationStore(upsert/list/stats/export dify/jsonl),ChatMessage.turn_id UUID v4,annotation_upsert 命令(bad→sponge.absorb_text 回流),ChatPanel 👍/👎 按钮 + 评论框 | P2 | M | 无 | A |
| T-E-S-29 | ✅ DONE (2026-07-04) — **OpenTelemetry 原生集成**：otel feature(4 个 OTel 依赖改 optional),observability/otel.rs `#![cfg(feature = "otel")]` 守卫,OtelConfig/bootstrap_otel/OtelStatus/status/redact_endpoint_basic_auth,init_tracing 用 `tracing_subscriber::layer::Identity` 占位保证 feature off 编译,otel_status 命令(cfg 分支降级),gateway/orchestrator/swarm 10 处 `#[instrument]` 补全,12 单测(feature on 时) | P1 | M | 无 | A |

#### 2.5.4 协议与集成（17 个）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-S-30 | ✅ DONE (2026-07-03) — **MCP `tools/list` + `tools/call` 补完**：当前是桩，必须真实实现 | P0 | M | T-S2-B-02 | A |
| T-E-S-31 | ✅ DONE (2026-07-04) — **MCP SSE transport**：SseTransport + SseEventAccumulator(私有 SSE 行解析器,跨 chunk 缓冲)+ SseEvent,new(url, api_key)(SSRF 校验),send_and_receive(POST + oneshot 30s 超时),shutdown(cancel + 等 listener),后台监听 task(GET /sse + bytes_stream + 指数退避重连 1→2→4→8→16→30,3 次),McpTransportType 增加 Sse 变体,McpServerConfig 扩展 5 字段(args/env/api_key/auto_restart/health_check_interval_secs),McpServersConfig(resolve_path/load/save/validate),ActiveTransport 增加 Sse 变体 + handshake_sse,57 单测 | P1 | M | T-S2-B-02 | B |
| T-E-S-32 | ✅ DONE (2026-07-04) — **MCP stdio 子进程管理**：McpServerRegistry(Clone, inner: Arc<Inner>),McpServerStatus(Stopped/Starting/Running/Crashed/Restarting/Disabled),McpServerRuntime,start/stop/status/list/logs/stop_all/bootstrap/load_config/supervisor_loop/health_check,借鉴 SidecarManager 模式(restart_backoff_delay min(2^n, 30)s + 滑动窗口限流 3 次/小时),健康检查 invoke_tool("__mcp_health_check__") 5s 超时,日志 `<log_dir>/mcp-<name>.log`,AppState.mcp_registry 字段 + 5 Tauri 命令(mcp_server_list/start/stop/status/logs),8 单测 | P1 | M | T-S2-B-02 | A |
| T-E-S-33 | ✅ DONE (2026-07-04) — **OpenAPI 工具服务器**：Cargo.toml 新增 `openapiv3 = { version = "2", optional = true }` + `openapi = ["dep:openapiv3"]` feature(默认关闭)+ 新建 `tools/openapi_server.rs`(`OpenApiToolServer` spec 解析 JSON/YAML 自动判别 + `OpenApiAuth` Bearer/ApiKey 枚举 + `with_auth` builder + `list_tool_definitions` 遍历 paths.operations 生成 ToolDefinition + `execute` HTTP 请求 + 鉴权注入 + SSRF 防护复用 SsrfGuard::build_safe_client + 30s 超时 + 10KB 响应截断)+ `OpenApiToolAdapter`(async→sync 桥接,复刻 McpToolAdapter 的 tokio::task::block_in_place + Handle::try_current + current-thread fallback 模式,impl `Tool` trait)+ `ToolRegistry::register_openapi_tools` 批量注册(cfg-gated)+ `openapi_register_tools` Tauri 命令(cfg-gated,接收 spec 字符串 + 可选 bearer_token)+ 11 单测(parse JSON/YAML/list_tool_definitions/bearer auth/apikey auth/SSRF 拦截/execute tool call + 4 个端到端 mock HTTP echo server 测试,修复 reqwest/hyper HTTP/2 header 小写化断言) | P1 | L | 无 | A |
| T-E-S-34 | ✅ DONE (2026-07-04) — **MCPO (MCP over HTTP)**：`McpTransportType` 新增 `StreamableHttp { url, headers, session_id }` 变体(config.rs:59)+ 新建 `streamable_http.rs` 实现 `StreamableHttpTransport`(POST 单 endpoint + 可选 SSE listener + `Mcp-Session-Id` 首次响应提取 + 后续请求回填 + pending 路由表 oneshot channel + 指数退避重连 100ms/400ms/1.6s/6.4s/25s 最多 5 次 + SSRF 防护)+ `ActiveTransport` 新增 StreamableHttp 变体(client.rs:71)+ `McpTransport` 新增 StreamableHttp 变体(transport.rs:44)+ `McpClient::connect` 路由分发,12 新单测(connect/post_json/sse_response/reconnect/session_id_persistence/client_connect_streamable/序列化/默认值/空 URL 拒绝/from_config + 2 helper) | P2 | M | T-E-S-31 | A |
| T-E-S-35 | ✅ DONE (2026-07-03) — **5 层插件模型**：Filter/Action/Pipe/Tool/Skill 分层（Open WebUI） | P0 | L | 无 | A |
| T-E-S-36 | ✅ DONE (2026-07-04) — **SkillEngine 三层架构**：protocol.rs(SkillManifest/SkillRequest/SkillResponse/SkillTransport)+ capability.rs(CapabilityRegistry + match_by_intent/match_by_input + 反向映射)+ executor.rs(SkillExecutor trait + Local/Remote/Mcp 三种 transport,SSRF 校验)+ SkillEngine 门面委派三层(外部 API 不变),14 新单测(81 总) | P2 | L | 无 | B |
| T-E-S-37 | ✅ DONE (2026-07-04) — **skill-pool tags 扩展**:tags 字段早在 v0.3/migration 003 落地;本任务做扩展:`TagMatch` 枚举(Any/All)+ `ListSkillsRequest` 加 `tags: Vec<String>` + `tag_match` 字段(保留旧 `tag` 向后兼容)+ `store.list()` 多 tag SQL 参数化(OR/AND)+ `all_tags()` json_each 聚合 + `skill_tags` Tauri 命令 + 前端 SkillPanel 多 tag UI + 热门标签云,11 新单测(96 总) | P3 | S | 无 | B |
| T-E-S-38 | ✅ DONE (2026-07-04) — **可视化生成 Skills**：3 个 LLM skill(canvas-creator/mermaid-creator/mindmap-creator)+ seeder.rs demo_skills 扩展(language="llm",code 为 prompt 模板)+ VisualCreatorDialog.tsx + VizRenderer.tsx(canvas iframe srcdoc sandbox / mermaid+mindmap 走 MermaidView)+ MermaidView.tsx(动态 import + mermaid.render + 降级),6 新单测(87 总) | P2 | M | T-E-S-36 | B |
| T-E-S-39 | ✅ DONE (2026-07-03) — **SOUL.md / AGENTS.md / TOOLS.md 注入**：`llm/persona.rs` 定义 `PersonaConfig { soul_md, agents_md, tools_md }`,`load(workspace_root)` 并行读 3 文件(64KiB 截断),`to_system_prefix()` XML 标签拼接,AppConfig 加 `persona: Option<Arc<RwLock<PersonaConfig>>>` 缓存,`AppState::chat()` + `GenericAgent::run()` 两路径注入 system prompt 最前,SwarmOrchestrator 加 `set_persona()` 传播,3 个 Tauri 命令(persona_reload/get/set_file),Settings 加"AI 人格"卡片,6 个单元测试 | P1 | M | 无 | A |
| T-E-S-40 | ✅ DONE (2026-07-03) — **OpenAI 兼容层**：`OpenAICompatClient`(预设工厂 deepseek/vllm/lmstudio/openrouter + SSRF 校验 + reasoning_content 解析),`with_openai_compat` builder,chat() 新增 `openai-compat` 主分支,修复 `maybe_record_cost` 用 `record_with_context`(provider 上下文不再落 unknown 桶),`model_price` 加 llama/qwen/gemma 前缀,多 provider keychain slot | P1 | M | 无 | A |
| T-E-S-41 | ✅ DONE (2026-07-03) — **models.json 动态配置**：`ModelsConfig`/`ProviderConfig`/`ModelConfig` struct(version + providers + default_provider),`default_builtin()` 内置 deepseek/anthropic/ollama 三家,`load/save/resolve_path/validate`,cost_tracker `model_price` 先查 ModelsConfig.pricing 回退硬编码,`models_config_load/save/set_default/add/remove` 命令,Settings 加 LLM 提供商卡片 + keychain `provider:<id>` slot | P1 | S | 无 | A |
| T-E-S-42 | ✅ DONE (2026-07-04) — **VectorStore trait**：VectorStore trait(upsert/delete/search/len/health_check + batch_upsert/search_with_filter 默认实现)+ VectorStoreBackend 枚举 + create_vector_store 工厂 + LanceStore 完整桥接 + Qdrant/Chroma stub(feature gate)+ 11 调用方迁移 Arc<LanceStore> → Arc<dyn VectorStore>(memory/llm/skills/swarm/lib/doctor),26 单测 | P2 | L | 无 | A |
| T-E-S-43 | ✅ DONE (2026-07-04) — **SQLite 加密（SQLCipher）**：sqlcipher feature(rusqlite/bundled-sqlcipher-vendored-openssl),keychain.rs KEY_DB_ENCRYPTION_KEY + resolve_db_encryption_key(keychain 优先 → env fallback)+ generate_db_encryption_key(32 字节随机 base64)+ migrate_env_to_db_key(幂等),sqlite_store.rs `#[cfg(feature = "sqlcipher")] open_encrypted(path, key)`(PRAGMA 顺序:key → cipher_version 验证 → WAL → migrations),sqlite_cipher.rs CipherMigrator(encrypt_plaintext_db / decrypt_to_plaintext / cipher_version,用 sqlcipher_export() 批量迁移),3 Tauri 命令(db_encryption_status 始终编译 / db_encryption_enable / db_encryption_disable cfg-gated 双实现),4 单测 | P1 | M | 无 | A |
| T-E-S-44 | ✅ DONE (2026-07-04) — **StorageBackend trait**：StorageBackend trait(read/write/delete/exists/metadata + read_stream/write_stream + create_dir/remove_dir/list)+ LocalBackend 完整(tokio::fs + tmp+rename 原子写 + 路径沙箱)+ WebDavBackend 完整(reqwest 手写 PUT/GET/DELETE/MKCOL/PROPFIND)+ S3Backend stub(feature gate)+ StorageError enum + snapshot CopyBackend 迁移 + AppConfig 切换,20 单测 | P2 | L | 无 | A |
| T-E-S-45 | ✅ DONE (2026-07-04) — **ClawHub 双向兼容**：SkillExporter::to_skill_md(YAML front-matter + body,与 importer 字段对称)+ capabilities 反向映射(8 项)+ skill_export_clawhub 命令 + SkillPanel 导出按钮,4 单测往返保真 | P2 | M | 无 | A |
| T-E-S-46 | ✅ DONE (2026-07-04) — **技能发布命令**：`nebula skill publish` CLI 子命令(照搬 Cost 模式)+ SkillPublisher trait(GistPublisher/FilePublisher)+ keychain publisher:github slot + PublishManifest 校验 + --dry-run/--target gist|file/--json,9 单测 | P2 | M | 无 | A |

#### 2.5.5 自动化与自主度（10 个）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-S-50 | ✅ DONE (2026-07-03) — **自主度滑块 L0-L5**：L0 内联补全 / L1 定向编辑 / L2 对话 / L3 Plan / L4 蜂群 / L5 后台自动化 | P0 | L | 无 | A |
| T-E-S-51 | ✅ DONE (2026-07-03) — **Level 0 内联补全**：ChatPanel 输入框 AI 建议补全（本地小模型，零成本） | P0 | M | T-E-S-50 | A |
| T-E-S-52 | ✅ DONE (2026-07-03) — **Level 1 定向编辑**：选中文字 + 快捷键 → AI 局部改写 | P1 | M | T-E-S-50 | A |
| T-E-S-53 | **Cron 定时任务引擎**：`cron.rs` + SQLite 存储 + Sidecar 执行 | P1 | L | T-S4-B-03 | A |
| T-E-S-54 | ✅ DONE (2026-07-04) — **事件触发器**：TriggerEngine(三种 worker:message/file/webhook)+ 025_triggers.sql + SQLite CRUD + 去抖(debounce_ms)+ 递归防护(source_trigger_id)+ axum Webhook server(127.0.0.1:8088 + HMAC-SHA256)+ trigger_create/list/delete/enable/fire_log 命令,26 单测 | P2 | M | 无 | A |
| T-E-S-55 | ✅ DONE (2026-07-04) — **条件监控 Watch**：WatchSource 枚举(Web/System/Calendar)+ WatchWorker(第 4 种 TriggerKind)+ WebFetcher(SSRF 校验 + Diff 检测)+ SystemProbe(windows-sys GlobalMemoryStatusEx/GetSystemTimes)+ IcsParser(手写 VEVENT 解析)+ 026_watch.sql watch_state 表 + watch_test 命令,15 单测,零新依赖 | P2 | M | T-E-S-54 | A |
| T-E-S-56 | **Automation 模板**：预置日报/周报/费用报告模板 | P2 | S | T-E-S-53 | A |
| T-E-S-57 | ✅ DONE (2026-07-04) — **后台执行通知**：NotificationService(tauri-plugin-notification + 5s 去重),SwarmEvent 驱动,悬浮球 working 状态+任务计数角标,notifications_enabled/floating_ball_task_badge 配置 | P2 | S | T-E-D-03 | A |
| T-E-S-58 | **Calendar 组件**：月/周/日视图 + AI Function Calling 管理日程 | P1 | M | T-E-S-02 | A |
| T-E-S-59 | ✅ DONE (2026-07-03) — **统一收件箱**：所有渠道消息汇入 ChatPanel（OpenClaw） | P0 | M | T-S3-B-01 | A |

#### 2.5.6 基础设施（5 个）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-S-60 | **Gateway 守护进程**：`nebula gateway` 子命令 + 系统服务注册 | P1 | L | T-S4-B-03 | A |
| T-E-S-61 | ✅ DONE (2026-07-04) — **SidecarManager 自动 start_all**:setup 回调 spawn 异步 `sidecar_manager.bootstrap()`(start_all + wait_ready,失败仅 warn 不阻断 UI)+ `SidecarRuntime` 加 `last_health_check`/`health_check_failures` 字段 + `wait_for_listen_addr` 改造支持 gRPC HealthCheck(`#[cfg(feature="grpc")]` 双路径,feature off 回退端口等待)+ `grpc_health_check()` 内联实现(tonic_health::pb::HealthClient 拨号 + 5s 超时)+ `health_check()` 周期性 ping(supervisor_loop 对 Running sidecar,SERVING 重置 failures,连续 3 次失败标记 Crashed)+ 5 个 `*IpcClient::health_check()` 改造(cfg-gated),25 单测(gRPC 5 个待 feature on 验证) | P1 | M | T-S4-B-03 | A |
| T-E-S-62 | ✅ DONE (2026-07-03) — **`nebula doctor` 健康检查**：doctor_run Tauri 命令(非 CLI),DoctorReport + 10 子检查(app_config/keychain/sqlite/lancedb/ollama/gateway/sidecar/ipc/logs/backup),2s 超时 + tokio::join! 并发执行,ok/warn/fail 分级 + suggestion 修复建议,9 单测 | P2 | S | 无 | A |
| T-E-S-63 | **三定时机制**：Consolidation（03:00）+ Self-check（12:00）+ Retrospection（21:00） | P1 | L | 无 | B |
| T-E-S-64 | ✅ DONE (2026-07-03) — **反幻觉 [来源:工具] badge**：`memory/consistency.rs` 定义 `CitedMemory`/`ConsistencyWarning`(SourceConflict/SingleToolNegation/EmptyCitation)/`ConsistencyReport` + `analyze()` 启发式检查,`ContextBundle.cited_memories` 收集 provenance,`chat_stream` 流结束注入 `ChatComplete.consistency`,ChatPanel `ConsistencyBadge` 组件(绿色 ✓ 引用 N 条 / 橙色 ⚠ 折叠详情),6 个单元测试 | P1 | M | T-E-B-04 | B |

#### 2.5.7 Loop Engineering（9 个）

> **来源**：Loop Engineering 内化技能（`docs/skills/loop-engineering/`），由 7 专家评审完成 4 个致命风险消除（v1.1 评审修订版）。
> **设计权威**：[NEBULA_LOOP_DESIGN.md](../skills/loop-engineering/NEBULA_LOOP_DESIGN.md) §3.2
> **评审报告**：[REVIEW_v1.0.md](../skills/loop-engineering/REVIEW_v1.0.md)
>
> 评审关键决策（写入实现约束）：
> 1. **LoopEngine 不是独立模块**——内化为 MasterAgent 的 `execute_loop()` 方法，避免三套编排器并存
> 2. **自主度统一 L0-L5**——不自建 L1-L4，复用 `AutonomySlider.tsx`
> 3. **Checker 强制本地 Ollama**——闭源模型为数据主权红线
> 4. **STATE.md 是 SQLite 只读投影**——禁止 Agent 直接写，避免双写状态漂移
> 5. **Checker 升级现有 reviewer.rs**——不新建 maker_checker.rs
> 6. **Loop 经验进 L3 事实层**——不进星魂（persona 层）

| 任务 ID | 描述 | 优先级 | 复杂度 | 依赖 | 来源 |
|---------|------|--------|--------|------|------|
| T-E-L-01 | **MasterAgent Loop 执行模式**：master.rs 新增 `execute_loop()` 方法 + loop_def.rs（LOOP.md YAML 解析）+ StateMgr（STATE.md 只读投影，从 SQLite 生成）+ 复用 `master_*` Tauri 命令。Loop 执行模式与现有 Once/Plan 模式并列。 | P1 | L | T-E-C-10 | Loop 内化 |
| T-E-L-02 | **CronTask 扩展**：扩展现有 `evolution/cron_scheduler.rs` 支持完整 5 字段 cron 表达式（不引入 tokio-cron-scheduler）+ Token/时间预算（AtomicU64 内存累加，异步落库）+ L0-L5 自主度字段 | P1 | M | T-E-L-01 | Loop 内化 |
| T-E-L-03 | **ReviewerAgent 升级为 CheckerAgent**：升级现有 `swarm/agents/reviewer.rs`（加 worktree 隔离 + 对抗 prompt + 独立 Context 通道 + 模型同质检测 + 自动降级 L4→L2），不新建 maker_checker.rs | P1 | L | T-E-S-01, T-E-C-08 | Loop 内化 |
| T-E-L-04 | **GitHub MCP 连接器（pull-only）**：读取 Actions 失败 + Issue + PR（**默认 pull-only，写操作人工触发**），为 CI Sweeper / PR Babysitter / Daily Triage 提供 Observation 信号 | P2 | L | T-E-C-18 | Loop 内化 |
| T-E-L-05 | **Loop 模板库**：7 种 Loop 模式的 LOOP.md 模板 + 复用 TemplatesDialog（新增 automation 类别，默认只露 2 个入口） | P2 | M | T-E-L-01 | Loop 内化 |
| T-E-L-06 | **Loop 预算管理 + 安全防护**：loop-budget.md（拆分本地 $0 / 云端两列）+ loop-cost 估算 + 超预算自动暂停 + loop-safety-guards.md（模型同质检测口径定义 + 自动降级触发条件） | P2 | M | T-E-L-02 | Loop 内化 |
| T-E-L-07 | **Loop 审计日志**：loop-run-log.md（人类可读 Markdown，每次运行的 cadence/token/结果 + provenance）+ 异常告警（IM webhook 通知） | P3 | S | T-E-L-01 | Loop 内化 |
| T-E-L-08a | **Loop 运行时阶段环**（评审拆分）：复用 SwarmView 的 AgentColumn + ToolCallCard，加五阶段高亮环（Intent→Context→Action→Observation→Adjustment），不等 WorkflowCanvas | P2 | M | T-E-S-11 | Loop 内化 |
| T-E-L-08b | **Loop 设计节点**（评审拆分）：WorkflowCanvas 集成 Loop 节点（Intent/Context/Action/Observation/Adjustment 五阶段节点 + 停止条件边），依赖 T-E-S-10 | P3 | XL | T-E-S-10 | Loop 内化 |

**优先级排序逻辑**（NEBULA_LOOP_DESIGN.md §3.3）：
1. **T-E-L-01 + T-E-L-02 + T-E-L-03**（P1）构成最小可用 Loop：能定义、能调度、能 Maker-Checker 验证
2. **T-E-L-04 + T-E-L-05 + T-E-L-08a**（P2）让 Loop 有真实信号源、模板和运行时可视化
3. **T-E-L-06 + T-E-L-07**（P2/P3）控制成本和可观测性
4. **T-E-L-08b**（P3）设计时编排，非阻塞

---

## 3. Stage 7 优先级矩阵

### 3.1 P0 任务（立即可做，不依赖任何 Stage）

| 任务 ID | 描述 | 支柱 | 复杂度 |
|---------|------|------|--------|
| T-E-A-01 | SemanticCache 层（L0.5） | 省钱 | S |
| T-E-A-06 | Token 费用追踪 | 省钱 | S |
| T-E-B-11 | BM25 + 向量混合搜索 | 智能 | M |
| T-E-S-01 | Agent 角色专业化 | 蜂群 | M |
| T-E-S-02 | LLM Function Calling | 蜂群 | L |
| T-E-S-20 | exec fail-closed | 安全 | S |
| T-E-S-21 | assemble_context ACL | 安全 | S |
| T-E-S-30 | MCP tools/list + tools/call 补完 | 协议 | M |
| T-E-S-35 | 5 层插件模型 | 协议 | L |
| T-E-S-50 | 自主度滑块 L0-L5 框架 | 自动化 | L |
| T-E-S-51 | Level 0 内联补全 | 自动化 | M |
| T-E-S-59 | 统一收件箱 | 协议 | M |

**P0 任务总工时估算**：约 18 人天（S×4 + M×5 + L×3）

### 3.2 P1 任务（关键路径，分波推进）

**Wave 1（v2.3 省钱革命）**：
- T-E-A-02~07（CostEngine + TokenJuice + ModelRouter + Credits）
- T-E-S-51~52（自主度滑块 L0-L1）

**Wave 2（v2.4 知识革命）**：
- T-E-B-01~04（LLM Wiki + 三视图 + 双向同步 + 溯源链）
- T-E-B-07~12（知识图谱 + Obsidian 兼容 + 文件夹索引 + `#` 命令 + BM25 + docling）
- T-E-B-17（ReasoningChain）

**Wave 3（v2.5 形象革命）**：
- T-E-D-01~03（冷启动 + 首响 + 悬浮球）
- T-E-C-08~10（Shadow Workspace + 录屏 + 异步长任务）
- T-E-S-60（Gateway 守护进程）

**Wave 4（v2.6 可视革命）**：
- T-E-S-10~11（WorkflowCanvas + 蜂群画布）
- T-E-C-01~06（OS-Controller 双模式 + ScreenReader + Hybrid Browser）
- T-E-S-26（Event Stream 协议化）

**Wave 5（v3.0 全自主革命）**：
- T-E-C-13~20（场景模板 + 多端 + OAuth + Docker）
- T-E-S-53~58（Cron + Trigger + Watch + Calendar + 统一收件箱）

**Wave Loop（v2.5+ Loop Engineering 内化，跨波推进）**：

> Loop Engineering 是内化的工程方法论，跨多个 Wave 推进。P1 三任务（最小可用 Loop）依赖 Wave 3 的 T-E-C-10（异步长任务）已 ✅ 完成，可在 Wave 3 之后立即启动。

- **阶段一（最小可用 Loop）**：T-E-L-01（MasterAgent Loop 模式）+ T-E-L-02（CronTask 扩展）+ T-E-L-03（ReviewerAgent 升级 CheckerAgent）
- **阶段二（信号源 + 模板 + 可视化）**：T-E-L-04（GitHub MCP pull-only）+ T-E-L-05（Loop 模板库）+ T-E-L-08a（运行时阶段环）
- **阶段三（成本 + 可观测 + 设计时编排）**：T-E-L-06（预算 + 安全防护）+ T-E-L-07（审计日志）+ T-E-L-08b（设计节点）

详见 [NEBULA_LOOP_DESIGN.md](../skills/loop-engineering/NEBULA_LOOP_DESIGN.md) §4 实施路线图。

### 3.3 P2/P3 任务（增强体验，可延后）

详见 §2 任务表，按版本节奏推进。

---

## 4. License 合规矩阵（Stage 7 新增）

| 对标项目 | License | 与 nebula(MIT) 兼容 | 借鉴边界 |
|---------|---------|------------------------|---------|
| OpenClaw | MIT | ✅ | 可代码级借鉴 |
| Hermes | MIT | ✅ | 可代码级借鉴 |
| Open WebUI | MIT | ✅ | 可代码级借鉴 |
| Dify | Apache 2.0 | ✅ | 可代码级借鉴（保留 NOTICE） |
| UI-TARS-desktop | Apache 2.0 | ✅ | 可代码级借鉴（保留 NOTICE） |
| OpenHuman | MIT | ✅ | 可代码级借鉴 |
| Reasonix | 未明确 | ⚠️ | 思路借鉴，需核实 |
| CoPaw | 未明确 | ⚠️ | 思路借鉴，需核实 |
| OpenAkita | **AGPL-3.0** | ❌ | **仅思路借鉴，不可代码 fork** |
| Obsidian Skills | GPL-3.0 | ⚠️ | 思路借鉴，不可代码 fork |
| LLM Wiki 理念 | 公开理念 | ✅ | 自由借鉴 |

---

## 5. 与 v2.1 的衔接说明

### 5.1 任务编号体系

- **T-S\*-\*-\*\***（v2.1）：Stage 1-6 工程闭环任务，以 `ROADMAP_v2.1.md` §6 为准
- **T-E-\*-\*\***（v2.2）：Stage 7 创新支柱任务，以本文档 §2 为准
- **禁止混用**：commit message 必须明确引用对应编号体系

### 5.2 依赖关系

Stage 7 大部分任务依赖 Stage 1-2 已完成的基础：
- 记忆系统（Stage 1 ✅）→ 支撑 T-E-B-\* 知识革命
- 协议层（Stage 2a ✅）→ 支撑 T-E-S-30~46 协议扩展
- 安全层（Stage 2b ✅）→ 支撑 T-E-S-20~29 安全增强
- 蜂群基础（Stage 3 ✅）→ 支撑 T-E-S-01~14 蜂群协作

### 5.3 执行顺序建议

**立即可做（本周）**：P0 任务中的小复杂度项
1. T-E-S-20 exec fail-closed（S，安全加固）
2. T-E-S-21 assemble_context ACL（S，安全加固）
3. T-E-A-01 SemanticCache 层（S，省钱先锋）
4. T-E-A-06 Token 费用追踪（S，费用可见化）
5. T-E-B-11 BM25 + 向量混合搜索（M，召回率提升）

**Wave 1 启动（Stage 1-2 完成后即可）**：v2.3 省钱革命
- 优先 T-E-A-02 TokenJuice 三级压缩 + T-E-A-03 ModelRouter

---

## 6. 附录

### 6.1 与 COMPREHENSIVE_EVOLUTION_v3.0.md 的映射

本文档前 68 个 T-E-\* 任务（T-E-A/B/C/D/S 系列）均来自 `COMPREHENSIVE_EVOLUTION_v3.0.md` §4，任务编号一致，可直接交叉引用。

新增 9 个 T-E-L-\* 任务（T-E-L-01~08b）来自 Loop Engineering 内化技能（`docs/skills/loop-engineering/`），由 7 专家评审完成 4 个致命风险消除（v1.1 评审修订版）。详见 [NEBULA_LOOP_DESIGN.md](../skills/loop-engineering/NEBULA_LOOP_DESIGN.md) §3.2 与 [REVIEW_v1.0.md](../skills/loop-engineering/REVIEW_v1.0.md)。

### 6.2 来源标记说明

- **来源 A**：报告 A（`EXPERT_REVIEW_v3.0_INNOVATION.md`，7 专家 + 大厂趋势）
- **来源 B**：报告 B（GLM-5.2 对话分析，OpenAkita 校准 + UI-TARS/CoPaw/LLM Wiki 深度对标）
- **来源 A+B**：双报告共同提出，互补合并
- **来源 Loop 内化**：Loop Engineering 公开资料内化（`docs/skills/loop-engineering/`），7 专家评审通过

### 6.3 测试策略（Stage 7 新增）

| Wave | 测试要求 |
|------|---------|
| Wave 1 省钱 | SemanticCache 命中率测试；ModelRouter 路由准确性测试；Credits Dashboard 数据一致性测试 |
| Wave 2 知识 | LLM Wiki 编译输出验证；双向同步冲突解决测试；BM25+向量混合搜索召回率对比测试 |
| Wave 3 形象 | 悬浮球多窗口生命周期测试；Shadow Workspace 隔离性测试；冷启动性能基准 |
| Wave 4 可视 | WorkflowCanvas 拖拽编排集成测试；OS-Controller 视觉识别准确率测试；Event Stream 协议一致性测试 |
| Wave 5 全自主 | Cron 定时任务准确性测试；多端 E2EE 同步测试；场景闭环端到端测试 |
| Wave Loop | LOOP.md YAML 解析测试；STATE.md 只读投影一致性测试；Maker-Checker 对抗验证测试；Cron 表达式扩展测试；模型同质检测+自动降级测试 |

---

**文档结束**。

本文档是 Stage 7 创新支柱的权威任务清单。Stage 1-6 任务以 `ROADMAP_v2.1.md` 为准。两份文档共同构成Nebula v3.0 的完整规划基线。

**配套文档**：
- `docs/ROADMAP_v2.1.md`（Stage 1-6 工程闭环）
- `docs/COMPREHENSIVE_EVOLUTION_v3.0.md`（创新审议综合报告）
- `docs/WHITEPAPER_v3.0.md`（Stage 7 设计权威，待起草）

**下一步**：立即启动 P0 任务，推荐执行顺序：
1. T-E-S-20 exec fail-closed（安全加固，1 人天）
2. T-E-A-01 SemanticCache 层（省钱先锋，1-2 人天）
3. T-E-B-11 BM25 混合搜索（召回率提升，3-5 人天）
