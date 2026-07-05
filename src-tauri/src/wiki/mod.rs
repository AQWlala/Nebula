//! T-E-B-01: LLM Wiki 编译引擎。
//!
//! 每次对话后 AI "编译" 结构化 Markdown 笔记写入 `wiki/` 目录,
//! 含 YAML front-matter + `[[双向链接]]`。元数据持久化到 `wiki_notes` 表,
//! 正文缓存到 `body` 列供 FTS5 全文检索。
//!
//! ## 设计要点
//!
//! * **幂等性**:`compile_turn` 同 `turn_id` 不重复编译(UNIQUE 索引 + 短路)。
//! * **双写一致性**:先写 SQLite,后写文件;文件失败回滚 SQLite(补偿删除)。
//! * **LLM 失败降级**:解析失败用默认 title "未命名笔记",不返回 Err;
//!   LLM 调用失败返回 Err 供上层 warn。
//! * **slug 唯一**:UNIQUE 索引 + 冲突追加 `-2`/`-3` 后缀。
//! * **路径沙箱**:slugify 仅保留 `[a-z0-9-]`,LocalBackend 拒绝 `..`。
//! * **FTS5**:external content mode,触发器自动同步 `wiki_notes` → `wiki_notes_fts`。

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::ChatMessage;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::version_control::MemoryVersionControl;
use crate::storage::{DynStorageBackend, StorageError};

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 一条 wiki 笔记的元数据(对齐 spec §数据结构)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiNote {
    /// UUID v4,主键。
    pub id: String,
    /// 关联对话 turn_id(幂等键,可空表示 raw 编译)。
    pub turn_id: Option<String>,
    /// 笔记标题(LLM 生成,≤60 字)。
    pub title: String,
    /// 文件名安全 slug(小写中划线)。
    pub slug: String,
    /// 标签列表。
    pub tags: Vec<String>,
    /// 相对 storage 路径 `wiki/{slug}.md`。
    pub path: String,
    /// 创建时间(Unix 毫秒)。
    pub created_at: i64,
    /// 更新时间(Unix 毫秒)。
    pub updated_at: i64,
    /// 重要性 0.0..1.0(默认 0.5;T-E-B-06 引入,regenerate_index 按其降序排序)。
    pub importance: f32,
}

/// T-E-B-13: 知识卡片 — 聚合单条 wiki 笔记的元数据 + 正文 + 关联实体 + 反向链接。
///
/// 供前端 `wiki_get_card(slug)` 命令返回,KnowledgeCardDialog 弹窗渲染。
///
/// **字段说明**:
/// - `note`:笔记元数据(对齐 `WikiNote`,**不含 body**)。
/// - `body`:从 storage 读取的 markdown 正文(单独存放,供前端 markdown 渲染)。
///   注:`WikiNote` 结构体本身无 `body` 字段(元数据 + body 在 SQLite 分两列),
///   故此处单独携带,避免前端再发一次 `wiki_read` 请求。
/// - `definition`:正文第一行,用作卡片头部摘要。
/// - `related_entities`:正文中的 `[[xxx]]` 双向链接 slug 列表(宽松匹配,不去重)。
/// - `backlinks`:反向链接的 note id 列表(指向本笔记的笔记 UUID)。
/// - `source`:来源标识,当前固定 `"wiki"`(预留 `"memory"` / `"import"`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCard {
    /// 笔记元数据(无 body)。
    pub note: WikiNote,
    /// 从 storage 读取的 markdown 正文(供前端 markdown 渲染)。
    pub body: String,
    /// 定义(正文第一行),用于卡片头部摘要。
    pub definition: Option<String>,
    /// 正文中的 `[[xxx]]` 双向链接 slug 列表(不去重)。
    pub related_entities: Vec<String>,
    /// 反向链接的 note id 列表(指向本笔记的笔记 UUID)。
    pub backlinks: Vec<String>,
    /// 来源("wiki" / "memory" / "import")。
    pub source: String,
}

/// Wiki 编译器配置。
#[derive(Debug, Clone)]
pub struct WikiConfig {
    /// 是否启用(默认 true)。
    pub enabled: bool,
    /// 笔记子目录(默认 "wiki")。
    pub subdir: String,
}

impl Default for WikiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            subdir: "wiki".to_string(),
        }
    }
}

/// LLM 输出解析结果。
#[derive(Debug, Clone)]
pub struct CompiledNote {
    /// 标题。
    pub title: String,
    /// 标签列表。
    pub tags: Vec<String>,
    /// Markdown 正文(front-matter 之后)。
    pub markdown: String,
    /// 从正文中提取的 `[[双向链接]]` slug 列表(去重)。
    pub links: Vec<String>,
}

// ---------------------------------------------------------------------------
// LogEvent (T-E-B-06)
// ---------------------------------------------------------------------------

/// `_log.md` 追加事件(Created/Updated/Deleted)。
///
/// 序列化为 `{kind: "created", id, title, ts}` 形式供前端日志解析。
/// `to_markdown_line()` 输出追加到 `_log.md` 的单行 markdown。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEvent {
    Created {
        id: String,
        title: String,
        ts: i64,
    },
    Updated {
        id: String,
        title: String,
        ts: i64,
    },
    Deleted {
        id: String,
        title: String,
        ts: i64,
    },
}

impl LogEvent {
    /// 渲染为 `_log.md` 单行 markdown。
    ///
    /// 格式:`- [<RFC3339 ts>] <icon> <Kind> \`[[id]]\` — <title>`
    /// 时间戳解析失败时降级为 Unix 毫秒数字字符串。
    ///
    /// 注:`ts` 为 Unix **毫秒**(与 `WikiNote.created_at` 一致),
    /// 故用 `from_timestamp_millis` 而非 `from_timestamp`(后者按秒解释会溢出)。
    pub fn to_markdown_line(&self) -> String {
        let ts_str = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(self.ts())
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| self.ts().to_string());
        match self {
            LogEvent::Created { id, title, .. } => {
                format!("- [{ts_str}] ✨ Created `[[{id}]]` — {title}")
            }
            LogEvent::Updated { id, title, .. } => {
                format!("- [{ts_str}] ✏️ Updated `[[{id}]]` — {title}")
            }
            LogEvent::Deleted { id, title, .. } => {
                format!("- [{ts_str}] 🗑️ Deleted `{id}` — {title}")
            }
        }
    }

    /// 提取事件时间戳(Unix 毫秒)。
    fn ts(&self) -> i64 {
        match self {
            LogEvent::Created { ts, .. }
            | LogEvent::Updated { ts, .. }
            | LogEvent::Deleted { ts, .. } => *ts,
        }
    }

    /// 提取 note id(测试用)。
    #[allow(dead_code)]
    fn id(&self) -> &str {
        match self {
            LogEvent::Created { id, .. }
            | LogEvent::Updated { id, .. }
            | LogEvent::Deleted { id, .. } => id,
        }
    }

    /// 提取 title(测试用)。
    #[allow(dead_code)]
    fn title(&self) -> &str {
        match self {
            LogEvent::Created { title, .. }
            | LogEvent::Updated { title, .. }
            | LogEvent::Deleted { title, .. } => title,
        }
    }
}

// ---------------------------------------------------------------------------
// WikiCompiler
// ---------------------------------------------------------------------------

/// T-E-B-03: Wiki 笔记编辑时的记忆向量化抽象。
///
/// 默认实现(`SpongeEngine`)在用户编辑笔记后调用 `sponge.absorb_text`
/// 重新向量化到记忆系统;测试可注入 mock 实现以验证调用语义。
///
/// 通过 trait 抽象而非直接持有 `Arc<SpongeEngine>`,便于在单元测试中
/// 绕过 SpongeEngine 对 Ollama embedder + LanceDB 的硬依赖。
#[async_trait::async_trait]
pub trait MemoryRevectorizer: Send + Sync {
    /// 把新内容重新吸收到记忆系统(重新向量化)。
    async fn absorb_text(&self, content: &str) -> Result<()>;
}

/// `SpongeEngine` 默认实现 `MemoryRevectorizer`:把新 body 作为
/// `Semantic` / `L3` / `UserInput` 记忆吸收(provenance tool = `wiki-edit`)。
#[async_trait::async_trait]
impl MemoryRevectorizer for SpongeEngine {
    async fn absorb_text(&self, content: &str) -> Result<()> {
        use crate::memory::types::{MemoryLayer, MemoryType, SourceKind};
        self.absorb_text(
            MemoryType::Semantic,
            MemoryLayer::L3,
            content.to_string(),
            SourceKind::UserInput,
            Some("wiki-edit"),
        )
        .await
        .map(|_| ())
    }
}

/// LLM Wiki 编译引擎。
///
/// 持有 LlmGateway(生成笔记)、DynStorageBackend(写文件)、
/// SqliteStore(元数据 + FTS5)。
///
/// T-E-B-06:`_log_lock` 串行化 `_log.md` 追加写入,避免并发 append_log
/// 互相覆盖(StorageBackend 无原生 append 接口,read-modify-write 需串行)。
///
/// T-E-B-03:`sponge` + `version_control` 用于记忆双向同步。两者均为 `Option`,
/// 未注入时 `update_note_from_user` 走 graceful degrade(跳过对应步骤)。
pub struct WikiCompiler {
    llm: Arc<LlmGateway>,
    storage: DynStorageBackend,
    sqlite: Arc<SqliteStore>,
    config: WikiConfig,
    /// `_log.md` 追加锁;每次 append_log 持锁,确保 read-modify-write 原子。
    _log_lock: tokio::sync::Mutex<()>,
    /// T-E-B-03: 记忆向量化 sink(可选,未注入时跳过 re-vectorization)。
    sponge: Option<Arc<dyn MemoryRevectorizer>>,
    /// T-E-B-03: 版本控制器(可选,未注入时跳过 commit)。
    version_control: Option<Arc<MemoryVersionControl>>,
}

impl WikiCompiler {
    /// 构造编译器。
    pub fn new(
        llm: Arc<LlmGateway>,
        storage: DynStorageBackend,
        sqlite: Arc<SqliteStore>,
        config: WikiConfig,
    ) -> Self {
        Self {
            llm,
            storage,
            sqlite,
            config,
            _log_lock: tokio::sync::Mutex::new(()),
            sponge: None,
            version_control: None,
        }
    }

    /// T-E-B-03: 注入记忆双向同步依赖(sponge + version_control)。
    ///
    /// Builder 模式:在 `WikiCompiler::new(...)` 之后链式调用。
    /// 调用后 `update_note_from_user` 将:
    /// 1. UPDATE SQLite wiki_notes.body
    /// 2. `sponge.absorb_text(&new_body)` 重新向量化
    /// 3. `write_note_file` 重写 `{slug}.md`
    /// 4. `version_control.commit(...)` 写版本记录
    /// 5. `append_log(LogEvent::Updated)`
    pub fn with_memory_sync(
        mut self,
        sponge: Arc<dyn MemoryRevectorizer>,
        version_control: Arc<MemoryVersionControl>,
    ) -> Self {
        self.sponge = Some(sponge);
        self.version_control = Some(version_control);
        self
    }

    /// 返回编译器是否启用(暴露给 Tauri 命令层判断)。
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// 编译一次对话的笔记(幂等:同 turn_id 短路)。
    ///
    /// 流程:
    /// 1. `get_by_turn_id` 短路(已编译直接返回)。
    /// 2. 调 LlmGateway 生成结构化 Markdown。
    /// 3. 解析 front-matter + 提取双向链接。
    /// 4. 持久化(SQLite + 文件双写)。
    pub async fn compile_turn(
        &self,
        turn_id: &str,
        user_msg: &str,
        assistant_msg: &str,
    ) -> Result<WikiNote> {
        // 幂等短路。
        if let Some(existing) = self.get_by_turn_id(turn_id).await? {
            return Ok(existing);
        }

        let prompt = build_turn_prompt(user_msg, assistant_msg);
        let response = self
            .llm
            .chat(vec![ChatMessage::user(prompt)])
            .await
            .context("wiki compile_turn LLM chat failed")?;
        let compiled = parse_llm_output(&response.message.content);
        self.persist_note(Some(turn_id.to_string()), compiled)
            .await
    }

    /// 编译原始内容(无对话上下文)。
    ///
    /// - `title_hint = Some(title)`:直接用 title 作为标题,content 作为正文(不调 LLM)。
    /// - `title_hint = None`:调 LLM 编译 content 为结构化笔记。
    pub async fn compile_raw(
        &self,
        title_hint: Option<&str>,
        content: &str,
    ) -> Result<WikiNote> {
        let compiled = if let Some(title) = title_hint {
            CompiledNote {
                title: title.to_string(),
                tags: Vec::new(),
                markdown: content.to_string(),
                links: extract_links(content),
            }
        } else {
            let prompt = build_raw_prompt(content);
            let response = self
                .llm
                .chat(vec![ChatMessage::user(prompt)])
                .await
                .context("wiki compile_raw LLM chat failed")?;
            parse_llm_output(&response.message.content)
        };
        self.persist_note(None, compiled).await
    }

    /// 列出笔记(按 created_at DESC 分页)。
    pub async fn list(&self, limit: u32, offset: u32) -> Result<Vec<WikiNote>> {
        let conn = self.sqlite.raw_connection();
        let conn_guard = conn.lock();
        let mut stmt = conn_guard
            .prepare(
                "SELECT id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance
                 FROM wiki_notes ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map(params![limit as i64, offset as i64], row_to_note)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 读取笔记(返回元数据 + 文件内容)。
    pub async fn read(&self, id: &str) -> Result<(WikiNote, String)> {
        let note = {
            let conn = self.sqlite.raw_connection();
            let conn_guard = conn.lock();
            let result = conn_guard.query_row(
                "SELECT id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance
                 FROM wiki_notes WHERE id = ?1",
                params![id],
                row_to_note,
            );
            match result {
                Ok(n) => n,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Err(anyhow!("wiki note not found: {id}"));
                }
                Err(e) => return Err(anyhow!("sqlite read error: {e}")),
            }
        };

        let bytes = self
            .storage
            .read(&note.path)
            .await
            .map_err(|e| anyhow!("storage read {} error: {e}", note.path))?;
        let markdown = String::from_utf8(bytes)
            .map_err(|e| anyhow!("note content not utf8: {e}"))?;
        Ok((note, markdown))
    }

    /// FTS5 全文搜索。
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<WikiNote>> {
        let conn = self.sqlite.raw_connection();
        let conn_guard = conn.lock();
        let mut stmt = conn_guard
            .prepare(
                "SELECT w.id, w.turn_id, w.title, w.slug, w.tags_json, w.path, w.created_at, w.updated_at, w.importance
                 FROM wiki_notes_fts f
                 JOIN wiki_notes w ON w.rowid = f.rowid
                 WHERE wiki_notes_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map(params![query, limit as i64], row_to_note)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 按 ID 查询单条 WikiNote(内部辅助,供 get_backlinks 使用)。
    fn get_by_id(&self, id: &str) -> Result<Option<WikiNote>> {
        let conn = self.sqlite.raw_connection();
        let conn_guard = conn.lock();
        let result = conn_guard.query_row(
            "SELECT id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance
             FROM wiki_notes WHERE id = ?1",
            params![id],
            row_to_note,
        );
        match result {
            Ok(n) => Ok(Some(n)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("sqlite get_by_id error: {e}")),
        }
    }

    /// T-E-B-05: 更新双向链接关系(先删旧链接再插新链接)。
    ///
    /// 在 `persist_note` 成功后调用:解析笔记正文中的 `[[slug]]`,
    /// 查 wiki_notes 表确认目标存在,只插存在的链接(悬空链接忽略)。
    pub fn update_backlinks(&self, note_id: &str, links: &[String]) -> Result<()> {
        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        // 先删旧链接。
        g.execute(
            "DELETE FROM wiki_note_links WHERE source_id = ?1",
            params![note_id],
        )
        .map_err(|e| anyhow!("delete old backlinks error: {e}"))?;
        // 再插新链接(只插目标存在的)。
        for target_slug in links {
            // 查 wiki_notes 表确认 slug 对应的 note 存在。
            let target_id: Option<String> = g
                .query_row(
                    "SELECT id FROM wiki_notes WHERE slug = ?1 LIMIT 1",
                    params![target_slug],
                    |r| r.get(0),
                )
                .ok();
            if let Some(tid) = target_id {
                g.execute(
                    "INSERT OR IGNORE INTO wiki_note_links (source_id, target_id) VALUES (?1, ?2)",
                    params![note_id, tid],
                )
                .map_err(|e| anyhow!("insert backlink error: {e}"))?;
            }
        }
        Ok(())
    }

    /// T-E-B-05: 获取反向链接(所有指向 note_id 的笔记)。
    ///
    /// 查 wiki_note_links WHERE target_id = ?1 获取 source_id 列表,
    /// 再批量查 WikiNote 返回。
    pub fn get_backlinks(&self, note_id: &str) -> Result<Vec<WikiNote>> {
        let ids: Vec<String> = {
            let conn = self.sqlite.raw_connection();
            let g = conn.lock();
            let mut stmt = g
                .prepare("SELECT source_id FROM wiki_note_links WHERE target_id = ?1")
                .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
            let x = stmt.query_map(params![note_id], |r| r.get(0))
                .map_err(|e| anyhow!("sqlite query error: {e}"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| anyhow!("sqlite row error: {e}"))?;
            x
        };
        let notes: Vec<WikiNote> = ids
            .iter()
            .filter_map(|id| self.get_by_id(id).ok().flatten())
            .collect();
        Ok(notes)
    }

    /// T-E-B-13: 聚合知识卡片(note + body + definition + related_entities + backlinks)。
    ///
    /// 供前端 `wiki_get_card(slug)` 命令调用,KnowledgeCardDialog 弹窗渲染。
    ///
    /// **流程**:
    /// 1. 按 `slug` 查询笔记元数据(前端 `[[xxx]]` 链接的 xxx 即 slug,非 UUID)。
    /// 2. 从 storage 读取 markdown 正文(与 `read()` 一致)。
    /// 3. `definition` = 正文第一行(`body.lines().next()`)。
    /// 4. `related_entities` = `extract_wiki_links(body)`(宽松匹配,不去重)。
    /// 5. `backlinks` = `get_backlinks(note.id)` 映射为 id 列表。
    ///
    /// **slug vs id**:`get_by_id` / `get_backlinks` 内部用 UUID `id` 列查询,
    /// 但前端传入的是 slug(文件名安全标识),故本方法按 `slug` 列查笔记,
    /// 再用查到的 `note.id` 调 `get_backlinks`。
    pub async fn get_card(&self, slug: &str) -> Result<KnowledgeCard> {
        // 1. 按 slug 查询笔记元数据。
        let note = {
            let conn = self.sqlite.raw_connection();
            let conn_guard = conn.lock();
            let result = conn_guard.query_row(
                "SELECT id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance
                 FROM wiki_notes WHERE slug = ?1",
                params![slug],
                row_to_note,
            );
            match result {
                Ok(n) => n,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Err(anyhow!("note not found: {}", slug));
                }
                Err(e) => return Err(anyhow!("sqlite get_card query error: {e}")),
            }
        };

        // 2. 从 storage 读取正文(与 read() 一致)。
        let bytes = self
            .storage
            .read(&note.path)
            .await
            .map_err(|e| anyhow!("storage read {} error: {e}", note.path))?;
        let body = String::from_utf8(bytes)
            .map_err(|e| anyhow!("note content not utf8: {e}"))?;

        // 3. 定义 = 正文第一行。
        let definition = body.lines().next().map(String::from);

        // 4. 关联实体 = 正文中的 [[xxx]] 链接(宽松匹配)。
        let related_entities = extract_wiki_links(&body);

        // 5. 反向链接(指向本笔记的笔记 id 列表)。
        let backlinks: Vec<String> = self
            .get_backlinks(&note.id)?
            .into_iter()
            .map(|n| n.id)
            .collect();

        Ok(KnowledgeCard {
            note,
            body,
            definition,
            related_entities,
            backlinks,
            source: "wiki".to_string(),
        })
    }

    /// 删除笔记(幂等:不存在的 id 也返回 Ok)。
    ///
    /// 先删 SQLite(触发器自动清 FTS),后删文件(NotFound 视为 Ok)。
    /// T-E-B-06:删除成功后追加 `Deleted` 事件到 `_log.md`(非阻塞,失败仅 warn)。
    pub async fn delete(&self, id: &str) -> Result<()> {
        let (path_opt, title_opt): (Option<String>, Option<String>) = {
            let conn = self.sqlite.raw_connection();
            let conn_guard = conn.lock();
            let result = conn_guard.query_row(
                "SELECT path, title FROM wiki_notes WHERE id = ?1",
                params![id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            );
            match result {
                Ok((p, t)) => (Some(p), Some(t)),
                Err(rusqlite::Error::QueryReturnedNoRows) => (None, None),
                Err(e) => return Err(anyhow!("sqlite delete query error: {e}")),
            }
        };

        if let (Some(path), Some(title)) = (path_opt, title_opt) {
            // 用块作用域确保 MutexGuard 在 await 前 drop,避免 future 非 Send。
            {
                let conn = self.sqlite.raw_connection();
                let conn_guard = conn.lock();
                conn_guard
                    .execute("DELETE FROM wiki_notes WHERE id = ?1", params![id])
                    .map_err(|e| anyhow!("sqlite delete error: {e}"))?;
            }

            // 文件删除幂等:NotFound 视为 Ok。
            if let Err(e) = self.storage.delete(&path).await {
                if !matches!(e, StorageError::NotFound(_)) {
                    return Err(anyhow!("storage delete {} error: {e}", path));
                }
            }

            // T-E-B-06:追加 Deleted 事件(非阻塞)。
            let event = LogEvent::Deleted {
                id: id.to_string(),
                title,
                ts: chrono::Utc::now().timestamp_millis(),
            };
            let _ = self.append_log(event).await;
        }
        Ok(())
    }

    /// 按 turn_id 查询(幂等短路用)。
    pub async fn get_by_turn_id(&self, turn_id: &str) -> Result<Option<WikiNote>> {
        let conn = self.sqlite.raw_connection();
        let conn_guard = conn.lock();
        let result = conn_guard.query_row(
            "SELECT id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance
             FROM wiki_notes WHERE turn_id = ?1",
            params![turn_id],
            row_to_note,
        );
        match result {
            Ok(n) => Ok(Some(n)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("sqlite get_by_turn_id error: {e}")),
        }
    }

    /// T-E-B-03: 用户编辑笔记后的双向同步入口。
    ///
    /// 流程(对齐 spec §4.2):
    /// 1. SQLite `UPDATE wiki_notes SET body=?, updated_at=? WHERE id=?`
    ///    (块作用域确保 `MutexGuard` 在 `await` 前 drop,避免阻塞其他锁)。
    /// 2. 重新向量化 — 若 `sponge` 注入则 `sponge.absorb_text(&new_body)`,
    ///    失败仅 `warn!` 不阻断主路径(graceful degrade)。
    /// 3. 文件重写 `write_note_file(note_id, &new_body)`。
    /// 4. 版本记录 — 若 `version_control` 注入则 `vc.commit(...)`(best-effort)。
    /// 5. 触发 `LogEvent::Updated` 追加到 `_log.md`。
    pub async fn update_note_from_user(
        &self,
        note_id: &str,
        new_body: String,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        // 1. SQLite UPDATE + 顺带查 title / path(块作用域锁)。
        let (title, path): (String, String) = {
            let conn = self.sqlite.raw_connection();
            // 持锁后查 + 改;锁在块结束 drop,之后 await 才安全。
            let g = conn.lock();
            let row: (String, String) = g
                .query_row(
                    "SELECT title, path FROM wiki_notes WHERE id = ?1",
                    params![note_id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .map_err(|e| anyhow!("sqlite query title/path error: {e}"))?;
            let affected = g
                .execute(
                    "UPDATE wiki_notes SET body = ?1, updated_at = ?2 WHERE id = ?3",
                    params![&new_body, now, note_id],
                )
                .map_err(|e| anyhow!("sqlite update wiki_note error: {e}"))?;
            if affected == 0 {
                return Err(anyhow!("wiki note not found: {note_id}"));
            }
            row
        };
        // MutexGuard 已 drop,可安全 await。

        // 2. 重新向量化(若 sponge 注入)。失败仅 warn,不阻断。
        if let Some(sponge) = &self.sponge {
            if let Err(e) = sponge.absorb_text(&new_body).await {
                warn!(
                    target: "nebula.wiki.sync",
                    note_id, error = %e,
                    "re-vectorization failed (non-blocking)"
                );
            }
        }

        // 3. 文件重写。
        self.storage
            .write(&path, new_body.as_bytes())
            .await
            .map_err(|e| anyhow!("storage write {} error: {e}", path))?;

        // 4. 版本记录(best-effort)。
        if let Some(vc) = &self.version_control {
            let payload = serde_json::json!({
                "body": new_body,
                "ts": now,
                "source": "user_edit",
            });
            if let Err(e) = vc.commit(
                "wiki_update",
                note_id,
                &payload,
                "user",
                "wiki note updated by user",
            ) {
                warn!(
                    target: "nebula.wiki.sync",
                    note_id, error = %e,
                    "version_control.commit failed (non-blocking)"
                );
            }
        }

        // 5. 触发 LogEvent::Updated(非阻塞)。
        let event = LogEvent::Updated {
            id: note_id.to_string(),
            title,
            ts: now,
        };
        let _ = self.append_log(event).await;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 内部辅助
    // -----------------------------------------------------------------------

    /// 持久化笔记(双写一致性:SQLite 先,文件后;文件失败补偿删除 SQLite)。
    ///
    /// T-E-B-06:持久化成功后追加 `Created` 事件到 `_log.md`(非阻塞,失败仅 warn)。
    async fn persist_note(
        &self,
        turn_id: Option<String>,
        compiled: CompiledNote,
    ) -> Result<WikiNote> {
        let base_slug = slugify(&compiled.title);
        let slug = self.ensure_unique_slug(&base_slug);
        let path = self.resolve_note_path(&slug);
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let tags_json = serde_json::to_string(&compiled.tags)
            .map_err(|e| anyhow!("serialize tags error: {e}"))?;

        // T-E-B-06:importance 默认 0.5(后续可由 LLM 评分或前端设置覆盖)。
        let importance: f32 = 0.5;

        let note = WikiNote {
            id: id.clone(),
            turn_id,
            title: compiled.title.clone(),
            slug: slug.clone(),
            tags: compiled.tags.clone(),
            path: path.clone(),
            created_at: now,
            updated_at: now,
            importance,
        };

        // 1. 写 SQLite(触发器自动同步 FTS)。
        {
            let conn = self.sqlite.raw_connection();
            let conn_guard = conn.lock();
            conn_guard
                .execute(
                    "INSERT INTO wiki_notes (
                        id, turn_id, title, slug, tags_json, path, body, created_at, updated_at, importance
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        note.id,
                        note.turn_id,
                        note.title,
                        note.slug,
                        tags_json,
                        note.path,
                        compiled.markdown,
                        note.created_at,
                        note.updated_at,
                        note.importance,
                    ],
                )
                .map_err(|e| anyhow!("sqlite insert wiki_note error: {e}"))?;
        }

        // 2. 写文件;失败则补偿删除 SQLite 行(回滚)。
        if let Err(e) = self
            .storage
            .write(&path, compiled.markdown.as_bytes())
            .await
        {
            // 补偿删除:best-effort,失败仅 warn。
            let conn = self.sqlite.raw_connection();
            let conn_guard = conn.lock();
            let _ = conn_guard.execute(
                "DELETE FROM wiki_notes WHERE id = ?1",
                params![&note.id],
            );
            return Err(anyhow!("storage write {} error: {e}", path));
        }

        // T-E-B-06:追加 Created 事件(非阻塞)。
        let event = LogEvent::Created {
            id: note.id.clone(),
            title: note.title.clone(),
            ts: now,
        };
        let _ = self.append_log(event).await;

        // T-E-B-05:更新双向链接关系(非阻塞,失败仅 warn)。
        let links = extract_links(&compiled.markdown);
        let _ = self.update_backlinks(&note.id, &links);

        Ok(note)
    }

    /// 解析笔记存储路径 `{subdir}/{slug}.md`。
    fn resolve_note_path(&self, slug: &str) -> String {
        format!("{}/{}.md", self.config.subdir, slug)
    }

    /// slug 去重:查 SQLite,冲突追加 `-2`/`-3` 后缀。
    fn ensure_unique_slug(&self, slug: &str) -> String {
        if slug.is_empty() {
            return self.ensure_unique_slug("untitled");
        }
        let conn = self.sqlite.raw_connection();
        let conn_guard = conn.lock();
        let mut candidate = slug.to_string();
        let mut counter = 2;
        loop {
            let exists: bool = conn_guard
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM wiki_notes WHERE slug = ?1)",
                    params![&candidate],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n != 0)
                .unwrap_or(false);
            if !exists {
                return candidate;
            }
            candidate = format!("{}-{}", slug, counter);
            counter += 1;
        }
    }

    // -----------------------------------------------------------------------
    // T-E-B-06: _log.md + _index.md 自动维护
    // -----------------------------------------------------------------------

    /// 追加事件到 `<wiki_dir>/_log.md`(非阻塞:失败仅 warn 不传播)。
    ///
    /// 流程:
    /// 1. 持 `_log_lock`(串行化并发 append,避免 read-modify-write 互相覆盖)。
    /// 2. 读 `_log.md` 现有内容;若不存在则初始化为 H1 头 `# Wiki Update Log\n\n`。
    /// 3. 追加 `event.to_markdown_line()` + `\n`。
    /// 4. 整体写回(StorageBackend::write 已是 tmp+rename 原子写)。
    ///
    /// **非阻塞**:任何 IO 错误均 `warn!` 后返回 `Ok(())`,避免影响主路径
    /// (persist_note / delete_note)。`_log.md` 是辅助索引,丢失可重建。
    pub async fn append_log(&self, event: LogEvent) -> Result<()> {
        // 持锁直到完成 read-modify-write。块作用域确保 guard 在 await 前 drop。
        let _guard = self._log_lock.lock().await;
        let path = format!("{}/_log.md", self.config.subdir);

        // 1. 读现有内容(NotFound → 用 H1 头初始化)。
        let existing = match self.storage.read(&path).await {
            Ok(bytes) => String::from_utf8(bytes)
                .unwrap_or_else(|_| "# Wiki Update Log\n\n".to_string()),
            Err(StorageError::NotFound(_)) => "# Wiki Update Log\n\n".to_string(),
            Err(e) => {
                warn!(
                    target: "nebula.wiki.log",
                    path = %path,
                    error = %e,
                    "append_log: read existing _log.md failed; event dropped (non-blocking)"
                );
                return Ok(());
            }
        };

        // 2. 追加新行。
        let new_line = event.to_markdown_line();
        let new_content = format!("{existing}{new_line}\n");

        // 3. 整体写回。
        if let Err(e) = self.storage.write(&path, new_content.as_bytes()).await {
            warn!(
                target: "nebula.wiki.log",
                path = %path,
                error = %e,
                "append_log: write _log.md failed; event dropped (non-blocking)"
            );
            return Ok(());
        }
        Ok(())
    }

    /// 全量重生成 `<wiki_dir>/_index.md`(原子写:tmp+rename)。
    ///
    /// 流程:
    /// 1. `list_all_notes()` 拉全部 WikiNote。
    /// 2. 按 `importance` 降序 + `created_at` 升序稳定排序(同 importance 老笔记在前)。
    /// 3. 渲染 markdown:H1 头 + Notes 计数 + Top Importance + Recent (Top 20) + By Topic。
    /// 4. 原子写 `<wiki_dir>/_index.md`。
    ///
    /// **阻塞**:与 append_log 不同,失败返回 `Err`(供前端 toast 显示)。
    pub async fn regenerate_index(&self) -> Result<()> {
        let mut notes = self.list_all_notes().await?;
        // 稳定排序:importance DESC + created_at ASC(同 importance 老笔记在前)。
        notes.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.created_at.cmp(&b.created_at))
        });

        let markdown = self.render_index_markdown(&notes);
        let path = format!("{}/_index.md", self.config.subdir);

        // StorageBackend::write 已是 tmp+rename 原子写,无需额外 .tmp 处理。
        self.storage
            .write(&path, markdown.as_bytes())
            .await
            .map_err(|e| anyhow!("regenerate_index write {} error: {e}", path))?;
        Ok(())
    }

    /// 拉全部 WikiNote(无分页,供 regenerate_index 用)。
    async fn list_all_notes(&self) -> Result<Vec<WikiNote>> {
        let conn = self.sqlite.raw_connection();
        let conn_guard = conn.lock();
        let mut stmt = conn_guard
            .prepare(
                "SELECT id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance
                 FROM wiki_notes",
            )
            .map_err(|e| anyhow!("sqlite prepare error: {e}"))?;
        let rows = stmt
            .query_map([], row_to_note)
            .map_err(|e| anyhow!("sqlite query error: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| anyhow!("sqlite row error: {e}"))?;
        Ok(rows)
    }

    /// 渲染 `_index.md` markdown 内容(纯函数,便于单测)。
    fn render_index_markdown(&self, notes: &[WikiNote]) -> String {
        let now_rfc = chrono::Utc::now().to_rfc3339();
        let mut md = String::new();
        md.push_str("# 📚 Wiki Index\n\n");
        md.push_str("> Auto-generated by nebula WikiCompiler.\n");
        md.push_str(&format!("> Last updated: {now_rfc}\n"));
        md.push_str(&format!("> Notes: {}\n\n", notes.len()));

        // 🔥 Top Importance(全部,按已排序顺序)。
        md.push_str("## 🔥 Top Importance\n\n");
        if notes.is_empty() {
            md.push_str("_(none)_\n\n");
        } else {
            for n in notes {
                md.push_str(&format!(
                    "- [[{}]] — {} *(importance: {:.2})*\n",
                    n.slug, n.title, n.importance
                ));
            }
            md.push('\n');
        }

        // 🕒 Recent (Top 20) — 按 created_at DESC 取前 20。
        md.push_str("## 🕒 Recent (Top 20)\n\n");
        if notes.is_empty() {
            md.push_str("_(none)_\n\n");
        } else {
            let mut recent: Vec<&WikiNote> = notes.iter().collect();
            recent.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            for n in recent.iter().take(20) {
                let date = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(n.created_at)
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| n.created_at.to_string());
                md.push_str(&format!("- {date} [[{}]] — {}\n", n.slug, n.title));
            }
            md.push('\n');
        }

        // 🏷️ By Topic — 按 tag 分组(字母序)。
        md.push_str("## 🏷️ By Topic\n\n");
        if notes.is_empty() {
            md.push_str("_(none)_\n\n");
        } else {
            let mut topics: std::collections::BTreeMap<String, Vec<&WikiNote>> =
                std::collections::BTreeMap::new();
            for n in notes {
                for tag in &n.tags {
                    topics.entry(tag.clone()).or_default().push(n);
                }
            }
            if topics.is_empty() {
                md.push_str("_(no tags)_\n\n");
            } else {
                for (tag, group) in topics {
                    let links: Vec<String> = group
                        .iter()
                        .map(|n| format!("[[{}]]", n.slug))
                        .collect();
                    md.push_str(&format!("- #{tag}: {}\n", links.join(" ")));
                }
                md.push('\n');
            }
        }

        md
    }
}

// ---------------------------------------------------------------------------
// 纯函数辅助(slugify / parse_llm_output / extract_links / prompt 模板)
// ---------------------------------------------------------------------------

/// 将标题转为 slug:小写 + 非字母数字替换为 `-` + 折叠连续 `-` + 去首尾 `-`。
///
/// 非 ASCII(如中文)字符被移除;若结果为空,返回 "untitled"。
pub fn slugify(title: &str) -> String {
    let mut slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // 折叠连续 `-`。
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    // 去首尾 `-`。
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 解析 LLM 输出为 CompiledNote。
///
/// 期望格式:
/// ```text
/// ---
/// title: <标题>
/// tags: [tag1, tag2]
/// ---
/// <markdown 正文>
/// ```
///
/// 解析失败降级:用默认 title "未命名笔记",markdown 为原始输出。
pub fn parse_llm_output(raw: &str) -> CompiledNote {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return degraded_compiled_note(raw);
    }

    // 跳过开头的 `---`,找闭合的 `---`。
    let after_open: &str = &trimmed[3..];
    // 闭合 `---` 必须在行首(允许 `\n---` 或 `\r\n---`)。
    let end_pos = match after_open.find("\n---") {
        Some(pos) => pos,
        None => return degraded_compiled_note(raw),
    };

    let yaml_str = &after_open[..end_pos];
    let body_start = end_pos + 4; // 跳过 `\n---`
    let markdown = after_open[body_start..]
        .trim_start_matches(['\n', '\r'])
        .to_string();

    #[derive(Deserialize)]
    struct FrontMatter {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        tags: Option<Vec<String>>,
    }

    let front_matter: FrontMatter = match serde_yaml::from_str(yaml_str) {
        Ok(fm) => fm,
        Err(_) => return degraded_compiled_note(raw),
    };

    let title = front_matter
        .title
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "未命名笔记".to_string());
    let tags = front_matter.tags.unwrap_or_default();
    let links = extract_links(&markdown);

    CompiledNote {
        title,
        tags,
        markdown,
        links,
    }
}

/// 降级构造 CompiledNote(解析失败时用)。
fn degraded_compiled_note(raw: &str) -> CompiledNote {
    CompiledNote {
        title: "未命名笔记".to_string(),
        tags: Vec::new(),
        markdown: raw.to_string(),
        links: extract_links(raw),
    }
}

/// 从 Markdown 中提取 `[[双向链接]]` slug 列表(去重,保序)。
///
/// 仅匹配 `[a-z0-9-]+` 的 slug,大写/特殊字符不匹配。
pub fn extract_links(markdown: &str) -> Vec<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\[\[([a-z0-9-]+)\]\]").unwrap());

    let mut seen = std::collections::HashSet::new();
    let mut links = Vec::new();
    for cap in re.captures_iter(markdown) {
        if let Some(m) = cap.get(1) {
            let slug = m.as_str().to_string();
            if seen.insert(slug.clone()) {
                links.push(slug);
            }
        }
    }
    links
}

/// T-E-B-13: 从正文中提取 `[[xxx]]` 双向链接 slug 列表(**不去重**)。
///
/// 与 `extract_links` 的区别:
/// - 本函数匹配任意非 `]` 字符(正则 `\[\[([^\]]+)\]\]`),更宽松,
///   用于知识卡片展示所有提及实体(含大小写混合 / 中文等)。
/// - `extract_links` 仅匹配 `[a-z0-9-]+` 且去重,用于持久化 backlinks
///   关系(只插 slug 存在的链接)。
///
/// 不去重是为了让前端展示真实的提及次数(如 `[[rust]]` 出现 3 次则返回 3 条)。
pub fn extract_wiki_links(body: &str) -> Vec<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
    re.captures_iter(body)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// 构造对话编译 prompt(对齐 spec §LLM Prompt 模板)。
fn build_turn_prompt(user_msg: &str, assistant_msg: &str) -> String {
    format!(
        "你是一个知识管理助手。请把下面的对话编译成一篇结构化的 Markdown 笔记。\n\n\
         要求:\n\
         1. 第一行必须是 YAML front-matter:\n   \
         ---\n   \
         title: <简短标题,不超过 60 字>\n   \
         tags: [<标签1>, <标签2>, ...]\n   \
         ---\n\
         2. 正文用 Markdown,含 `## 概述` / `## 要点` / `## 关联`。\n\
         3. 凡提到关键概念,用 `[[实体名]]` 双向链接语法包裹(实体名小写中划线 slug)。\n\
         4. 输出 ONLY 笔记内容。\n\n\
         【对话】\n\
         User: {user_msg}\n\
         Assistant: {assistant_msg}"
    )
}

/// 构造原始内容编译 prompt。
fn build_raw_prompt(content: &str) -> String {
    format!(
        "你是一个知识管理助手。请把下面的内容编译成一篇结构化的 Markdown 笔记。\n\n\
         要求:\n\
         1. 第一行必须是 YAML front-matter:\n   \
         ---\n   \
         title: <简短标题,不超过 60 字>\n   \
         tags: [<标签1>, <标签2>, ...]\n   \
         ---\n\
         2. 正文用 Markdown,含 `## 概述` / `## 要点` / `## 关联`。\n\
         3. 凡提到关键概念,用 `[[实体名]]` 双向链接语法包裹(实体名小写中划线 slug)。\n\
         4. 输出 ONLY 笔记内容。\n\n\
         【内容】\n\
         {content}"
    )
}

/// rusqlite 行 → WikiNote 转换。
///
/// SELECT 列序(对齐所有 SELECT 语句):
/// `id, turn_id, title, slug, tags_json, path, created_at, updated_at, importance`
fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<WikiNote> {
    let tags_json: String = row.get(4)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    Ok(WikiNote {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        title: row.get(2)?,
        slug: row.get(3)?,
        tags,
        path: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        importance: row.get(8)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{LocalBackend, StorageBackend};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// 测试用 SqliteStore + LocalBackend + LlmGateway 装配。
    fn test_harness() -> (
        WikiCompiler,
        Arc<SqliteStore>,
        DynStorageBackend,
        std::path::PathBuf,
        Vec<std::path::PathBuf>,
    ) {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nebula_wiki_test_{}_{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let db_path = dir.join("test.db");
        let storage_root = dir.join("storage");

        let sqlite = Arc::new(SqliteStore::open(&db_path).expect("open sqlite"));
        let backend = LocalBackend::new(&storage_root).expect("create local backend");
        let storage: DynStorageBackend = Arc::new(backend);
        let llm = Arc::new(LlmGateway::new_test());
        let compiler = WikiCompiler::new(llm, storage.clone(), sqlite.clone(), WikiConfig::default());

        let paths = vec![db_path, dir.join("test.db-wal"), dir.join("test.db-shm")];
        (compiler, sqlite, storage, dir, paths)
    }

    fn cleanup(paths: Vec<std::path::PathBuf>, dir: std::path::PathBuf) {
        for p in paths {
            let _ = std::fs::remove_file(&p);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- slugify ---

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
    }

    #[test]
    fn slugify_collapse_hyphens() {
        assert_eq!(slugify("Hello---World"), "hello-world");
        assert_eq!(slugify("a   b"), "a-b");
    }

    #[test]
    fn slugify_non_ascii_fallback() {
        // 中文被替换为 `-`,折叠后为空,fallback 到 "untitled"。
        assert_eq!(slugify("你好"), "untitled");
        assert_eq!(slugify("Rust 编程"), "rust");
    }

    #[test]
    fn slugify_path_traversal_safe() {
        // `../` 中的 `.` 被 replace 为 `-`,最终 slug 不含路径遍历。
        let s = slugify("../etc/passwd");
        assert!(!s.contains(".."));
        assert!(!s.contains('/'));
        assert!(!s.contains('\\'));
    }

    // --- extract_links ---

    #[test]
    fn extract_links_basic_and_dedup() {
        let md = "参见 [[rust]] 与 [[tauri]],再次引用 [[rust]]。";
        let links = extract_links(md);
        assert_eq!(links, vec!["rust", "tauri"]);
    }

    #[test]
    fn extract_links_empty() {
        let md = "无链接的文本 [[UPPER]] [[特殊字符!]]";
        let links = extract_links(md);
        assert!(links.is_empty(), "uppercase/special should not match: {links:?}");
    }

    #[test]
    fn extract_links_multiple_unique() {
        let md = "[[a]] [[b]] [[c]] [[a]] [[b]]";
        let links = extract_links(md);
        assert_eq!(links, vec!["a", "b", "c"]);
    }

    // --- parse_llm_output ---

    #[test]
    fn parse_llm_output_normal() {
        let raw = "---\ntitle: Rust 笔记\ntags: [rust, programming]\n---\n## 概述\n这是 [[rust]] 笔记。";
        let compiled = parse_llm_output(raw);
        assert_eq!(compiled.title, "Rust 笔记");
        assert_eq!(compiled.tags, vec!["rust", "programming"]);
        assert!(compiled.markdown.contains("## 概述"));
        assert_eq!(compiled.links, vec!["rust"]);
    }

    #[test]
    fn parse_llm_output_no_frontmatter() {
        let raw = "## 概述\n没有 front-matter 的文本。";
        let compiled = parse_llm_output(raw);
        assert_eq!(compiled.title, "未命名笔记");
        assert!(compiled.tags.is_empty());
        assert!(compiled.markdown.contains("没有 front-matter"));
    }

    #[test]
    fn parse_llm_output_malformed_yaml() {
        let raw = "---\ntitle: [invalid yaml\ntags: {broken\n---\n正文";
        let compiled = parse_llm_output(raw);
        assert_eq!(compiled.title, "未命名笔记");
        assert!(compiled.tags.is_empty());
    }

    #[test]
    fn parse_llm_output_missing_closing_fence() {
        let raw = "---\ntitle: Test\n这个没有闭合";
        let compiled = parse_llm_output(raw);
        assert_eq!(compiled.title, "未命名笔记");
    }

    #[test]
    fn parse_llm_output_empty_title_fallback() {
        let raw = "---\ntitle: \"\"\ntags: []\n---\n正文";
        let compiled = parse_llm_output(raw);
        assert_eq!(compiled.title, "未命名笔记");
    }

    // --- resolve_note_path ---

    #[test]
    fn resolve_note_path_basic() {
        let compiler = WikiCompiler {
            llm: Arc::new(LlmGateway::new_test()),
            storage: Arc::new(LocalBackend::new(std::env::temp_dir()).unwrap()),
            sqlite: Arc::new(SqliteStore::open(":memory:").unwrap()),
            config: WikiConfig::default(),
            _log_lock: tokio::sync::Mutex::new(()),
            sponge: None,
            version_control: None,
        };
        assert_eq!(compiler.resolve_note_path("test"), "wiki/test.md");
        assert_eq!(compiler.resolve_note_path("rust-basics"), "wiki/rust-basics.md");
    }

    // --- 集成测试:compile_raw + read + delete + search ---

    #[tokio::test]
    async fn compile_raw_with_title_hint_roundtrip() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let note = compiler
            .compile_raw(Some("Rust 编程入门"), "# Rust\n这是 [[rust]] 笔记。")
            .await
            .expect("compile_raw");
        assert_eq!(note.title, "Rust 编程入门");
        assert_eq!(note.slug, "rust"); // 中文被移除,余下 "rust"
        assert_eq!(note.path, "wiki/rust.md");

        // read 验证文件内容。
        let (note2, markdown) = compiler.read(&note.id).await.expect("read");
        assert_eq!(note2.id, note.id);
        assert!(markdown.contains("# Rust"));
        assert!(markdown.contains("[[rust]]"));

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn compile_turn_idempotent() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 第一次 compile_raw 生成一条笔记,模拟 turn_id 已存在。
        let note = compiler
            .compile_raw(Some("测试标题"), "内容")
            .await
            .expect("compile_raw");

        // 直接插入一条 turn_id 关联(模拟已编译)。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            g.execute(
                "UPDATE wiki_notes SET turn_id = ?1 WHERE id = ?2",
                params!["turn-123", &note.id],
            )
            .unwrap();
        }

        // compile_turn 同 turn_id 应短路,不调 LLM。
        let existing = compiler
            .compile_turn("turn-123", "user", "assistant")
            .await
            .expect("compile_turn idempotent");
        assert_eq!(existing.id, note.id);

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn list_returns_notes_desc_by_created_at() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let n1 = compiler.compile_raw(Some("First"), "a").await.unwrap();
        // 微小延迟确保 created_at 不同。
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let n2 = compiler.compile_raw(Some("Second"), "b").await.unwrap();

        let list = compiler.list(10, 0).await.unwrap();
        assert_eq!(list.len(), 2);
        // DESC:第二个(更晚创建)在前。
        assert_eq!(list[0].id, n2.id);
        assert_eq!(list[1].id, n1.id);

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn delete_idempotent() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let note = compiler
            .compile_raw(Some("待删除"), "内容")
            .await
            .unwrap();

        // 第一次删除成功。
        compiler.delete(&note.id).await.expect("delete first");

        // read 应失败(已删除)。
        assert!(compiler.read(&note.id).await.is_err());

        // 第二次删除幂等(不报错)。
        compiler.delete(&note.id).await.expect("delete again idempotent");

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn search_fts5_finds_by_body_keyword() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let _n1 = compiler
            .compile_raw(Some("Rust 笔记"), "Rust 是一门系统级编程语言")
            .await
            .unwrap();
        let _n2 = compiler
            .compile_raw(Some("Python 笔记"), "Python 是动态类型语言")
            .await
            .unwrap();

        // FTS5 搜索 "rust"。
        let hits = compiler.search("rust", 10).await.expect("search");
        assert!(!hits.is_empty(), "should find rust note");
        assert!(hits.iter().any(|n| n.title.contains("Rust")));

        // FTS5 搜索 "python"。
        let hits = compiler.search("python", 10).await.expect("search");
        assert!(!hits.is_empty(), "should find python note");
        assert!(hits.iter().any(|n| n.title.contains("Python")));

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn ensure_unique_slug_appends_suffix() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 第一条 note slug = "test"。
        let _n1 = compiler.compile_raw(Some("test"), "内容1").await.unwrap();
        // 第二条相同 title,slug 应追加 -2。
        let n2 = compiler.compile_raw(Some("test"), "内容2").await.unwrap();
        assert_eq!(n2.slug, "test-2");

        // 第三条相同 title,slug 应追加 -3。
        let n3 = compiler.compile_raw(Some("test"), "内容3").await.unwrap();
        assert_eq!(n3.slug, "test-3");

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn get_by_turn_id_returns_existing() {
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let note = compiler.compile_raw(Some("标题"), "内容").await.unwrap();
        // 手动设置 turn_id。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            g.execute(
                "UPDATE wiki_notes SET turn_id = ?1 WHERE id = ?2",
                params!["turn-xyz", &note.id],
            )
            .unwrap();
        }

        let found = compiler.get_by_turn_id("turn-xyz").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, note.id);

        let missing = compiler.get_by_turn_id("nonexistent").await.unwrap();
        assert!(missing.is_none());

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn path_sandbox_rejects_traversal_in_storage() {
        // LocalBackend 直接拒绝 `..` 路径,验证沙箱。
        let dir = std::env::temp_dir().join("nebula_wiki_sandbox_test");
        std::fs::create_dir_all(&dir).unwrap();
        let backend = LocalBackend::new(&dir).unwrap();
        let result = backend.write("../escape.txt", b"evil").await;
        assert!(result.is_err(), "path traversal should be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn llm_failure_returns_err_not_panic() {
        // LlmGateway::new_test() 指向 127.0.0.1:11434,无 Ollama 运行时 chat 会失败。
        // compile_turn 应返回 Err 而非 panic。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 用一个未编译的 turn_id 触发 LLM 调用。
        let result = compiler
            .compile_turn("turn-fail-llm", "hello", "world")
            .await;
        // 即使 LLM 调用成功(Ollama 恰好运行),也不应 panic。
        // 在 CI 环境(无 Ollama)应返回 Err。
        if let Err(e) = &result {
            let msg = format!("{e}");
            // 确认是 LLM 相关错误,不是 panic。
            assert!(
                msg.to_lowercase().contains("llm")
                    || msg.to_lowercase().contains("chat")
                    || msg.to_lowercase().contains("ollama")
                    || msg.to_lowercase().contains("circuit")
                    || msg.to_lowercase().contains("connection"),
                "unexpected error: {msg}"
            );
        }
        // 关键:不应 panic。
        cleanup(paths, dir);
    }

    #[test]
    fn build_turn_prompt_contains_conversation() {
        let prompt = build_turn_prompt("你好", "你好,世界");
        assert!(prompt.contains("你好"));
        assert!(prompt.contains("你好,世界"));
        assert!(prompt.contains("YAML front-matter"));
        assert!(prompt.contains("[[实体名]]"));
    }

    #[test]
    fn wiki_config_default() {
        let cfg = WikiConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.subdir, "wiki");
    }

    // -----------------------------------------------------------------
    // T-E-B-06: _log.md + _index.md 单元测试
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_append_log_creates_file() {
        // 目录无 _log.md → 首次写入应创建头 + 1 行。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let event = LogEvent::Created {
            id: "abc-123".to_string(),
            title: "First Note".to_string(),
            ts: 1_700_000_000_000,
        };
        compiler.append_log(event).await.expect("append_log ok");

        let bytes = _storage
            .read("wiki/_log.md")
            .await
            .expect("_log.md should exist");
        let content = String::from_utf8(bytes).expect("utf8");
        assert!(
            content.starts_with("# Wiki Update Log\n\n"),
            "expected H1 header prefix; got: {content:?}"
        );
        assert!(
            content.contains("✨ Created `[[abc-123]]` — First Note"),
            "expected Created event line; got: {content:?}"
        );
        // 应只有一行事件(以 `- ` 开头)。
        let event_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(event_lines.len(), 1, "exactly 1 event line expected");

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_append_log_appends() {
        // 已有 _log.md → 追加而不覆盖。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let e1 = LogEvent::Created {
            id: "id-1".to_string(),
            title: "Note One".to_string(),
            ts: 1_700_000_000_000,
        };
        let e2 = LogEvent::Updated {
            id: "id-2".to_string(),
            title: "Note Two".to_string(),
            ts: 1_700_000_001_000,
        };
        compiler.append_log(e1).await.expect("append 1");
        compiler.append_log(e2).await.expect("append 2");

        let bytes = _storage
            .read("wiki/_log.md")
            .await
            .expect("_log.md should exist");
        let content = String::from_utf8(bytes).expect("utf8");
        // H1 头只出现一次(未被覆盖)。
        assert_eq!(
            content.matches("# Wiki Update Log").count(),
            1,
            "header should appear exactly once; got: {content:?}"
        );
        // 两行事件均存在。
        assert!(content.contains("✨ Created `[[id-1]]` — Note One"));
        assert!(content.contains("✏️ Updated `[[id-2]]` — Note Two"));
        let event_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(event_lines.len(), 2);

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_regenerate_index_empty() {
        // 0 条 note → 生成空 index(只有头 + Notes: 0)。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        compiler
            .regenerate_index()
            .await
            .expect("regenerate_index empty");

        let bytes = _storage
            .read("wiki/_index.md")
            .await
            .expect("_index.md should exist");
        let content = String::from_utf8(bytes).expect("utf8");
        assert!(content.contains("# 📚 Wiki Index"), "missing H1");
        assert!(content.contains("> Notes: 0"), "missing Notes: 0");
        // 三段都应有 _(none)_ 占位。
        assert!(content.contains("## 🔥 Top Importance"));
        assert!(content.contains("## 🕒 Recent (Top 20)"));
        assert!(content.contains("## 🏷️ By Topic"));
        let none_count = content.matches("_(none)_").count();
        assert_eq!(none_count, 3, "expected 3 _(none)_ placeholders");

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_regenerate_index_sorted() {
        // 3 条 note 不同 importance → 按 importance 降序。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let n1 = compiler
            .compile_raw(Some("Low"), "low importance note")
            .await
            .unwrap();
        // 微小延迟确保 created_at 不同。
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let n2 = compiler
            .compile_raw(Some("Mid"), "mid importance note")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let n3 = compiler
            .compile_raw(Some("High"), "high importance note")
            .await
            .unwrap();

        // 手动设置不同 importance(n1=0.1, n2=0.5, n3=0.9)。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            g.execute(
                "UPDATE wiki_notes SET importance = ?1 WHERE id = ?2",
                params![0.1f32, &n1.id],
            )
            .unwrap();
            g.execute(
                "UPDATE wiki_notes SET importance = ?1 WHERE id = ?2",
                params![0.5f32, &n2.id],
            )
            .unwrap();
            g.execute(
                "UPDATE wiki_notes SET importance = ?1 WHERE id = ?2",
                params![0.9f32, &n3.id],
            )
            .unwrap();
        }

        compiler
            .regenerate_index()
            .await
            .expect("regenerate_index 3 notes");

        let bytes = _storage
            .read("wiki/_index.md")
            .await
            .expect("_index.md should exist");
        let content = String::from_utf8(bytes).expect("utf8");
        assert!(content.contains("> Notes: 3"));

        // Top Importance 段应按 importance 降序出现:High → Mid → Low。
        let top_section = content
            .split("## 🔥 Top Importance")
            .nth(1)
            .and_then(|s| s.split("## 🕒 Recent").next())
            .expect("Top Importance section exists");
        let pos_high = top_section.find("High").expect("High in top");
        let pos_mid = top_section.find("Mid").expect("Mid in top");
        let pos_low = top_section.find("Low").expect("Low in top");
        assert!(
            pos_high < pos_mid && pos_mid < pos_low,
            "expected order High < Mid < Low in Top Importance; section: {top_section:?}"
        );

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_regenerate_index_atomic() {
        // 写入中途失败 → 旧 _index.md 不被破坏。
        // 策略:用 FailingWriteBackend 包装真 backend,write 返回 Err,
        // 但 read 仍透传到 inner。预写"OLD"内容,regenerate 应失败,
        // 旧内容应保持不变。
        let dir = std::env::temp_dir().join(format!(
            "nebula_wiki_atomic_test_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let storage_root = dir.join("storage");

        let sqlite = Arc::new(SqliteStore::open(&db_path).expect("open sqlite"));
        let real_backend = Arc::new(LocalBackend::new(&storage_root).expect("create local"));
        let failing: DynStorageBackend = Arc::new(FailingWriteBackend::new(real_backend.clone()));
        let llm = Arc::new(LlmGateway::new_test());
        let compiler = WikiCompiler::new(llm, failing, sqlite.clone(), WikiConfig::default());

        // 1. 用真 backend 预写 "OLD" 到 _index.md。
        real_backend
            .write("wiki/_index.md", b"OLD CONTENT")
            .await
            .expect("pre-write old");

        // 2. 用 failing backend 调 regenerate_index → 应返回 Err。
        let result = compiler.regenerate_index().await;
        assert!(result.is_err(), "regenerate_index should fail");

        // 3. 用真 backend 读 _index.md,内容应仍为 "OLD CONTENT"。
        let bytes = real_backend
            .read("wiki/_index.md")
            .await
            .expect("read _index.md");
        let content = String::from_utf8(bytes).expect("utf8");
        assert_eq!(
            content, "OLD CONTENT",
            "old _index.md should be preserved on write failure"
        );

        let paths = vec![db_path, dir.join("test.db-wal"), dir.join("test.db-shm")];
        cleanup(paths, dir);
    }

    #[test]
    fn test_log_event_markdown_line() {
        // 三种变体格式正确。
        let ts = 1_700_000_000_000i64;
        let created = LogEvent::Created {
            id: "id-c".to_string(),
            title: "TC".to_string(),
            ts,
        };
        let updated = LogEvent::Updated {
            id: "id-u".to_string(),
            title: "TU".to_string(),
            ts,
        };
        let deleted = LogEvent::Deleted {
            id: "id-d".to_string(),
            title: "TD".to_string(),
            ts,
        };
        let c_line = created.to_markdown_line();
        let u_line = updated.to_markdown_line();
        let d_line = deleted.to_markdown_line();
        assert!(c_line.starts_with("- ["), "created should start with '- [': {c_line}");
        assert!(c_line.contains("✨ Created"), "created icon: {c_line}");
        assert!(c_line.contains("`[[id-c]]`"), "created id link: {c_line}");
        assert!(c_line.contains("— TC"), "created title: {c_line}");

        assert!(u_line.contains("✏️ Updated"), "updated icon: {u_line}");
        assert!(u_line.contains("`[[id-u]]`"), "updated id link: {u_line}");
        assert!(u_line.contains("— TU"), "updated title: {u_line}");

        // Deleted 变体不使用 [[]] 链接格式(笔记已删除,链接无目标)。
        assert!(d_line.contains("🗑️ Deleted"), "deleted icon: {d_line}");
        assert!(d_line.contains("`id-d`"), "deleted id code: {d_line}");
        assert!(d_line.contains("— TD"), "deleted title: {d_line}");
        assert!(!d_line.contains("[[id-d]]"), "deleted should NOT use [[]]: {d_line}");

        // 时间戳应可解析为 RFC3339。
        let ts_str = c_line
            .trim_start_matches("- [")
            .split(']')
            .next()
            .expect("ts bracket");
        let _ = chrono::DateTime::parse_from_rfc3339(ts_str)
            .expect("ts should be RFC3339 parseable");
    }

    /// 包装 LocalBackend,write 永远失败,其余方法透传 inner。
    ///
    /// 用于 test_regenerate_index_atomic 验证写入失败时旧 _index.md 不被破坏。
    struct FailingWriteBackend {
        inner: DynStorageBackend,
    }

    impl FailingWriteBackend {
        fn new(inner: DynStorageBackend) -> Self {
            Self { inner }
        }
    }

    #[async_trait::async_trait]
    impl StorageBackend for FailingWriteBackend {
        fn kind(&self) -> &'static str {
            "failing"
        }

        async fn read(&self, path: &str) -> crate::storage::StorageResult<Vec<u8>> {
            self.inner.read(path).await
        }

        async fn write(
            &self,
            _path: &str,
            _bytes: &[u8],
        ) -> crate::storage::StorageResult<()> {
            Err(crate::storage::StorageError::Unavailable(
                "FailingWriteBackend: write always fails (test)".to_string(),
            ))
        }

        async fn delete(&self, path: &str) -> crate::storage::StorageResult<()> {
            self.inner.delete(path).await
        }

        async fn exists(&self, path: &str) -> crate::storage::StorageResult<bool> {
            self.inner.exists(path).await
        }

        async fn metadata(
            &self,
            path: &str,
        ) -> crate::storage::StorageResult<crate::storage::FileMetadata> {
            self.inner.metadata(path).await
        }

        async fn read_stream(
            &self,
            path: &str,
        ) -> crate::storage::StorageResult<
            Box<dyn futures::Stream<Item = crate::storage::StorageResult<bytes::Bytes>> + Send + Unpin>,
        > {
            self.inner.read_stream(path).await
        }

        async fn write_stream(
            &self,
            _path: &str,
            _stream: Box<
                dyn futures::Stream<Item = crate::storage::StorageResult<bytes::Bytes>>
                    + Send
                    + Unpin,
            >,
            _expected_size: Option<u64>,
        ) -> crate::storage::StorageResult<()> {
            Err(crate::storage::StorageError::Unavailable(
                "FailingWriteBackend: write_stream always fails (test)".to_string(),
            ))
        }

        async fn create_dir(&self, path: &str) -> crate::storage::StorageResult<()> {
            self.inner.create_dir(path).await
        }

        async fn remove_dir(&self, path: &str) -> crate::storage::StorageResult<()> {
            self.inner.remove_dir(path).await
        }

        async fn list(
            &self,
            prefix: &str,
        ) -> crate::storage::StorageResult<Vec<crate::storage::FileMetadata>> {
            self.inner.list(prefix).await
        }
    }

    // -----------------------------------------------------------------
    // T-E-B-05: 双向链接 [[]] 语法 — 单元测试
    // -----------------------------------------------------------------

    #[test]
    fn test_wiki_note_links_migration() {
        // 验证 migration 033 建表成功 + schema 验证。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let conn = compiler.sqlite.raw_connection();
        let g = conn.lock();
        // 检查 wiki_note_links 表存在。
        let has_table: bool = g
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='wiki_note_links'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        assert!(has_table, "wiki_note_links table should exist after migration");
        // 检查 idx_wiki_note_links_target 索引存在。
        let has_index: bool = g
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_wiki_note_links_target'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        assert!(has_index, "idx_wiki_note_links_target index should exist");
        drop(g);
        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_update_backlinks_inserts() {
        // persist 1 个含 [[other]] 的笔记 → wiki_note_links 有 1 行。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 先创建目标笔记(slug = "other")。
        let _target = compiler
            .compile_raw(Some("Other"), "target note content")
            .await
            .unwrap();
        // 创建含 [[other]] 链接的源笔记。
        let source = compiler
            .compile_raw(Some("Source"), "This links to [[other]].")
            .await
            .unwrap();
        // 验证 wiki_note_links 有一行。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            let count: i64 = g
                .query_row(
                    "SELECT COUNT(*) FROM wiki_note_links WHERE source_id = ?1",
                    params![&source.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "should have 1 link row");
        }
        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_update_backlinks_removes_stale() {
        // 改笔记内容删除 [[other]] → 旧行被删。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 创建目标笔记。
        let _target = compiler
            .compile_raw(Some("Other"), "target note content")
            .await
            .unwrap();
        // 创建含 [[other]] 的源笔记。
        let source = compiler
            .compile_raw(Some("Source"), "Links to [[other]].")
            .await
            .unwrap();
        // 验证链接已建立。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            let count: i64 = g
                .query_row(
                    "SELECT COUNT(*) FROM wiki_note_links WHERE source_id = ?1",
                    params![&source.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "should have 1 link after initial persist");
        }
        // 手动调用 update_backlinks 清空链接(模拟编辑后无链接)。
        compiler
            .update_backlinks(&source.id, &[])
            .expect("update_backlinks should succeed");
        // 验证链接已删除。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            let count: i64 = g
                .query_row(
                    "SELECT COUNT(*) FROM wiki_note_links WHERE source_id = ?1",
                    params![&source.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "stale link should be removed");
        }
        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_get_backlinks() {
        // A→B + C→B → get_backlinks(B) 返回 [A, C]。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 创建 B。
        let note_b = compiler
            .compile_raw(Some("B Target"), "target note")
            .await
            .unwrap();
        // 创建 A(含 [[b-target]] 指向 B)。
        // 注意:slugify("B Target") = "b-target"。
        let note_a = compiler
            .compile_raw(Some("A Source"), "Links to [[b-target]].")
            .await
            .unwrap();
        // 创建 C(含 [[b-target]] 指向 B)。
        let note_c = compiler
            .compile_raw(Some("C Source"), "Also links to [[b-target]].")
            .await
            .unwrap();
        // 验证 get_backlinks(B) 返回 [A, C]。
        let backlinks = compiler.get_backlinks(&note_b.id).expect("get_backlinks");
        let backlink_ids: Vec<&str> = backlinks.iter().map(|n| n.id.as_str()).collect();
        assert!(
            backlink_ids.contains(&note_a.id.as_str()),
            "backlinks should contain A"
        );
        assert!(
            backlink_ids.contains(&note_c.id.as_str()),
            "backlinks should contain C"
        );
        assert_eq!(backlinks.len(), 2, "should have exactly 2 backlinks");
        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_cascade_delete() {
        // delete B → wiki_note_links 中指向 B 的行也被删(CASCADE)。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 创建 B。
        let note_b = compiler
            .compile_raw(Some("B Target"), "target note")
            .await
            .unwrap();
        // 创建 A(含 [[b-target]] 指向 B)。
        let _note_a = compiler
            .compile_raw(Some("A Source"), "Links to [[b-target]].")
            .await
            .unwrap();
        // 验证链接已建立。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            let count: i64 = g
                .query_row(
                    "SELECT COUNT(*) FROM wiki_note_links WHERE target_id = ?1",
                    params![&note_b.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "should have 1 link to B");
        }
        // 删除 B。
        compiler.delete(&note_b.id).await.expect("delete B");
        // 验证指向 B 的链接行已被 CASCADE 删除。
        {
            let conn = compiler.sqlite.raw_connection();
            let g = conn.lock();
            let count: i64 = g
                .query_row(
                    "SELECT COUNT(*) FROM wiki_note_links WHERE target_id = ?1",
                    params![&note_b.id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "links to B should be cascade-deleted");
        }
        cleanup(paths, dir);
    }

    // -----------------------------------------------------------------
    // T-E-B-13: 知识卡片 — 单元测试
    // -----------------------------------------------------------------

    #[test]
    fn test_knowledge_card_serialization() {
        // KnowledgeCard 序列化 round-trip(JSON)。
        let note = WikiNote {
            id: "card-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            title: "测试卡片".to_string(),
            slug: "test-card".to_string(),
            tags: vec!["tag1".to_string()],
            path: "wiki/test-card.md".to_string(),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            importance: 0.5,
        };
        let card = KnowledgeCard {
            note: note.clone(),
            body: "## 概述\n这是 [[rust]] 笔记。".to_string(),
            definition: Some("## 概述".to_string()),
            related_entities: vec!["rust".to_string()],
            backlinks: vec!["note-a".to_string(), "note-c".to_string()],
            source: "wiki".to_string(),
        };
        let json = serde_json::to_string(&card).expect("serialize");
        let back: KnowledgeCard = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.note.id, "card-1");
        assert_eq!(back.note.title, "测试卡片");
        assert_eq!(back.note.slug, "test-card");
        assert_eq!(back.body, "## 概述\n这是 [[rust]] 笔记。");
        assert_eq!(back.definition.as_deref(), Some("## 概述"));
        assert_eq!(back.related_entities, vec!["rust"]);
        assert_eq!(back.backlinks, vec!["note-a", "note-c"]);
        assert_eq!(back.source, "wiki");
    }

    #[test]
    fn test_extract_wiki_links() {
        // 提取 [[xxx]] 链接(宽松匹配,不去重)。
        let body = "参见 [[rust]] 与 [[tauri]],再次引用 [[rust]]。还有 [[mixed-Case]]。";
        let links = extract_wiki_links(body);
        // 不去重,保留所有匹配(含大小写混合)。
        assert_eq!(
            links,
            vec!["rust", "tauri", "rust", "mixed-Case"],
            "should not dedup and should match mixed-case"
        );

        // 无链接。
        assert!(
            extract_wiki_links("普通文本无链接").is_empty(),
            "no links in plain text"
        );

        // 空字符串。
        assert!(extract_wiki_links("").is_empty(), "empty body");

        // 单层 [] 不匹配(必须双括号)。
        assert!(
            extract_wiki_links("这是 [rust] 单括号").is_empty(),
            "single bracket should not match"
        );
    }

    #[tokio::test]
    async fn test_get_card_aggregates() {
        // get_card(slug) 返回 note + body + definition + related_entities + backlinks。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        // 创建目标笔记 B(slug = "b-target")。
        let note_b = compiler
            .compile_raw(Some("B Target"), "B 的定义行\n\n正文含 [[rust]] 链接。")
            .await
            .unwrap();
        // 创建 A(含 [[b-target]] 指向 B)。
        let note_a = compiler
            .compile_raw(Some("A Source"), "Links to [[b-target]].")
            .await
            .unwrap();

        let card = compiler
            .get_card("b-target")
            .await
            .expect("get_card b-target");

        // note 字段(元数据)。
        assert_eq!(card.note.id, note_b.id);
        assert_eq!(card.note.slug, "b-target");
        assert_eq!(card.note.title, "B Target");

        // body 字段(从 storage 读取的正文)。
        assert!(card.body.contains("B 的定义行"), "body should contain first line");
        assert!(card.body.contains("[[rust]]"), "body should contain wiki link");

        // definition = 正文第一行。
        assert_eq!(
            card.definition.as_deref(),
            Some("B 的定义行"),
            "definition should be first line of body"
        );

        // related_entities = 正文中的 [[xxx]](宽松匹配)。
        assert_eq!(card.related_entities, vec!["rust"]);

        // backlinks = 指向 B 的笔记 id 列表(应含 A)。
        assert!(
            card.backlinks.contains(&note_a.id),
            "backlinks should contain A's id: {:?}",
            card.backlinks
        );
        assert_eq!(card.backlinks.len(), 1, "exactly 1 backlink from A");

        // source = "wiki"。
        assert_eq!(card.source, "wiki");

        cleanup(paths, dir);
    }

    #[tokio::test]
    async fn test_get_card_not_found() {
        // slug 不存在 → 返回 Err。
        let (compiler, _sqlite, _storage, dir, paths) = test_harness();
        let result = compiler.get_card("nonexistent-slug").await;
        assert!(result.is_err(), "get_card should err on missing slug");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("not found") || msg.contains("nonexistent-slug"),
            "error should mention not found / slug: {msg}"
        );
        cleanup(paths, dir);
    }

    // -----------------------------------------------------------------
    // T-E-B-03: 记忆双向同步 — update_note_from_user 单元测试
    // -----------------------------------------------------------------

    /// 测试用 sponge mock — 记录所有 absorb_text 调用内容,供断言。
    struct MockRevectorizer {
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl MockRevectorizer {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls_snapshot(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl MemoryRevectorizer for MockRevectorizer {
        async fn absorb_text(&self, content: &str) -> Result<()> {
            self.calls.lock().unwrap().push(content.to_string());
            Ok(())
        }
    }

    /// 给 compiler 注入 sponge mock + version_control,返回增强版 compiler +
    /// mock + vc 引用 + (dir, paths) 用于 cleanup。
    fn harness_with_sync(
    ) -> (
        WikiCompiler,
        Arc<MockRevectorizer>,
        Arc<MemoryVersionControl>,
        Arc<SqliteStore>,
        DynStorageBackend,
        std::path::PathBuf,
        Vec<std::path::PathBuf>,
    ) {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nebula_wiki_sync_test_{}_{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("sync.db");
        let storage_root = dir.join("storage");

        let sqlite = Arc::new(SqliteStore::open(&db_path).expect("open sqlite"));
        let backend = LocalBackend::new(&storage_root).expect("create local backend");
        let storage: DynStorageBackend = Arc::new(backend);
        let llm = Arc::new(LlmGateway::new_test());

        let sponge_mock = Arc::new(MockRevectorizer::new());
        let vc = Arc::new(MemoryVersionControl::new(sqlite.clone()));
        let compiler = WikiCompiler::new(llm, storage.clone(), sqlite.clone(), WikiConfig::default())
            .with_memory_sync(sponge_mock.clone(), vc.clone());

        let paths = vec![db_path, dir.join("sync.db-wal"), dir.join("sync.db-shm")];
        (compiler, sponge_mock, vc, sqlite, storage, dir, paths)
    }

    /// 1. test_update_note_from_user_sqlite — UPDATE 写入成功。
    ///
    /// 验证调用 update_note_from_user 后,SQLite 中 body 列被新内容覆盖,
    /// updated_at > created_at(确认时间戳被刷新)。
    #[tokio::test]
    async fn test_update_note_from_user_sqlite() {
        let (compiler, _sponge, _vc, sqlite, _storage, dir, paths) = harness_with_sync();

        // 创建一条笔记(初始 body = "原始内容")。
        let note = compiler
            .compile_raw(Some("测试笔记"), "原始内容")
            .await
            .expect("compile_raw");

        // 调用 update_note_from_user。
        compiler
            .update_note_from_user(&note.id, "更新后的内容".to_string())
            .await
            .expect("update_note_from_user");

        // 验证 SQLite 中 body 已更新。
        let body: String = {
            let conn = sqlite.raw_connection();
            let g = conn.lock();
            g.query_row(
                "SELECT body FROM wiki_notes WHERE id = ?1",
                params![&note.id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(body, "更新后的内容");

        // 验证 updated_at > created_at。
        let (created, updated): (i64, i64) = {
            let conn = sqlite.raw_connection();
            let g = conn.lock();
            g.query_row(
                "SELECT created_at, updated_at FROM wiki_notes WHERE id = ?1",
                params![&note.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert!(
            updated > created,
            "updated_at ({updated}) should be > created_at ({created})"
        );

        cleanup(paths, dir);
    }

    /// 2. test_update_note_from_user_triggers_updated_log — LogEvent::Updated 追加。
    ///
    /// 验证调用后 _log.md 包含 "✏️ Updated" 行 + 正确 note_id。
    #[tokio::test]
    async fn test_update_note_from_user_triggers_updated_log() {
        let (compiler, _sponge, _vc, _sqlite, storage, dir, paths) = harness_with_sync();

        let note = compiler
            .compile_raw(Some("待更新笔记"), "原始")
            .await
            .unwrap();

        compiler
            .update_note_from_user(&note.id, "新内容".to_string())
            .await
            .expect("update");

        // 读 _log.md,应同时含 Created(初始编译)+ Updated(本次编辑)。
        let bytes = storage.read("wiki/_log.md").await.expect("read _log.md");
        let log = String::from_utf8(bytes).expect("utf8");
        // 原始 Created 行。
        assert!(
            log.contains("✨ Created"),
            "log should contain Created event: {log:?}"
        );
        // 新追加的 Updated 行。
        assert!(
            log.contains("✏️ Updated"),
            "log should contain Updated event: {log:?}"
        );
        assert!(
            log.contains(&note.id),
            "log should contain note id {}: {log:?}",
            note.id
        );

        cleanup(paths, dir);
    }

    /// 3. test_update_note_from_user_version_recorded — vc.commit 调用。
    ///
    /// 验证调用后 memory_commits 表新增一行 action='wiki_update',
    /// target_id = note.id,author = 'user'。
    #[tokio::test]
    async fn test_update_note_from_user_version_recorded() {
        let (compiler, _sponge, vc, _sqlite, _storage, dir, paths) = harness_with_sync();

        let note = compiler
            .compile_raw(Some("版本测试"), "v1")
            .await
            .unwrap();

        compiler
            .update_note_from_user(&note.id, "v2 内容".to_string())
            .await
            .expect("update");

        // 查 memory_commits 表,应有 1 行 action='wiki_update'。
        let commits = vc.log(50).expect("vc.log");
        let wiki_commits: Vec<_> = commits
            .iter()
            .filter(|c| c.action == "wiki_update" && c.target_id == note.id)
            .collect();
        assert_eq!(
            wiki_commits.len(),
            1,
            "expected exactly 1 wiki_update commit; got: {wiki_commits:?}"
        );
        assert_eq!(wiki_commits[0].author, "user");
        assert_eq!(wiki_commits[0].message, "wiki note updated by user");
        // payload 应包含新 body。
        let payload_str = wiki_commits[0].payload.to_string();
        assert!(
            payload_str.contains("v2 内容"),
            "payload should contain new body: {payload_str}"
        );

        cleanup(paths, dir);
    }

    /// 4. test_update_note_from_user_revectorizes — sponge.absorb_text 调用。
    ///
    /// 验证调用后 mock sponge 收到 absorb_text(content=new_body) 调用,
    /// 次数 = 1,内容与传入的 new_body 一致。
    #[tokio::test]
    async fn test_update_note_from_user_revectorizes() {
        let (compiler, sponge_mock, _vc, _sqlite, _storage, dir, paths) = harness_with_sync();

        let note = compiler
            .compile_raw(Some("向量化测试"), "原内容")
            .await
            .unwrap();

        // 调用前 mock 应无调用。
        assert!(
            sponge_mock.calls_snapshot().is_empty(),
            "sponge should have 0 calls before update"
        );

        compiler
            .update_note_from_user(&note.id, "需要向量化的新内容".to_string())
            .await
            .expect("update");

        // 验证 mock 收到 1 次调用,内容为 new_body。
        let calls = sponge_mock.calls_snapshot();
        assert_eq!(calls.len(), 1, "sponge.absorb_text should be called once");
        assert_eq!(calls[0], "需要向量化的新内容");

        cleanup(paths, dir);
    }

    /// 5. test_update_note_from_user_file_rewrite — 文件被新内容覆盖。
    ///
    /// 验证调用后 storage 中 {slug}.md 文件内容被覆盖为新 body。
    #[tokio::test]
    async fn test_update_note_from_user_file_rewrite() {
        let (compiler, _sponge, _vc, _sqlite, storage, dir, paths) = harness_with_sync();

        let note = compiler
            .compile_raw(Some("文件重写测试"), "原文件内容")
            .await
            .unwrap();

        // 调用前文件应为初始内容。
        let before = String::from_utf8(storage.read(&note.path).await.unwrap()).unwrap();
        assert!(before.contains("原文件内容"));

        compiler
            .update_note_from_user(&note.id, "重写后的文件内容".to_string())
            .await
            .expect("update");

        // 验证文件已被新内容覆盖。
        let after = String::from_utf8(storage.read(&note.path).await.unwrap()).unwrap();
        assert_eq!(after, "重写后的文件内容");
        assert!(
            !after.contains("原文件内容"),
            "old content should be replaced"
        );

        cleanup(paths, dir);
    }

    /// 6. test_update_note_from_user_no_sponge — sponge=None 时不报错。
    ///
    /// 验证未注入 sponge / version_control 时,update_note_from_user 仍能完成
    /// SQLite UPDATE + 文件重写 + LogEvent::Updated 主路径(graceful degrade)。
    #[tokio::test]
    async fn test_update_note_from_user_no_sponge() {
        // 直接用 test_harness(未注入 sponge / vc)。
        let (compiler, sqlite, storage, dir, paths) = test_harness();

        let note = compiler
            .compile_raw(Some("降级测试"), "原")
            .await
            .unwrap();

        // 调用应成功,不报错。
        compiler
            .update_note_from_user(&note.id, "新内容无 sponge".to_string())
            .await
            .expect("update should succeed without sponge");

        // SQLite 已更新。
        let body: String = {
            let conn = sqlite.raw_connection();
            let g = conn.lock();
            g.query_row(
                "SELECT body FROM wiki_notes WHERE id = ?1",
                params![&note.id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(body, "新内容无 sponge");

        // 文件已重写。
        let file = String::from_utf8(storage.read(&note.path).await.unwrap()).unwrap();
        assert_eq!(file, "新内容无 sponge");

        // _log.md 也应有 Updated 事件(append_log 不依赖 sponge)。
        let log_bytes = storage.read("wiki/_log.md").await.expect("read _log.md");
        let log = String::from_utf8(log_bytes).unwrap();
        assert!(log.contains("✏️ Updated"), "log should have Updated event");

        cleanup(paths, dir);
    }
}
