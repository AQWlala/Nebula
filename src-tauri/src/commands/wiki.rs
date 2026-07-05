//! T-E-B-01: Wiki Tauri 命令(5 个)。
//!
//! - `wiki_compile`  — 编译对话为 wiki 笔记(turn_id 幂等)
//! - `wiki_list`     — 列出笔记(分页,created_at DESC)
//! - `wiki_read`     — 读取笔记(元数据 + Markdown 正文)
//! - `wiki_search`   — FTS5 全文搜索
//! - `wiki_delete`   — 删除笔记(幂等)
//! - `wiki_regen_index` (T-E-B-06) — 全量重生成 `_index.md`(供前端"刷新目录"按钮)

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::wiki::{KnowledgeCard, WikiNote};
use crate::AppState;

/// `wiki_read` 命令的响应 DTO。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiNoteReadResponse {
    pub note: WikiNote,
    pub markdown: String,
}

/// 编译对话为 wiki 笔记。
///
/// - `turn_id = Some`:走 `compile_turn`(幂等,同 turn_id 不重复编译)。
/// - `turn_id = None`:走 `compile_raw`(用 user_message + assistant_message 拼接为原始内容)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_compile"))]
pub async fn wiki_compile(
    state: State<'_, AppState>,
    turn_id: Option<String>,
    user_message: String,
    assistant_message: String,
) -> Result<WikiNote, CommandError> {
    let compiler = state.wiki.clone();
    if !compiler_enabled(&state) {
        return Err(CommandError::validation("wiki compiler disabled"));
    }
    let note = if let Some(tid) = turn_id.as_deref() {
        compiler
            .compile_turn(tid, &user_message, &assistant_message)
            .await
            .map_err(|e| CommandError::llm("wiki_compile", &e))?
    } else {
        let content = if user_message.is_empty() {
            assistant_message.clone()
        } else {
            format!("{user_message}\n\n{assistant_message}")
        };
        compiler
            .compile_raw(None, &content)
            .await
            .map_err(|e| CommandError::llm("wiki_compile", &e))?
    };
    Ok(note)
}

/// 列出 wiki 笔记(分页,created_at DESC)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_list"))]
pub async fn wiki_list(
    state: State<'_, AppState>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<WikiNote>, CommandError> {
    let compiler = state.wiki.clone();
    let limit = limit.unwrap_or(50);
    let offset = offset.unwrap_or(0);
    let notes = compiler
        .list(limit, offset)
        .await
        .map_err(|e| CommandError::db("wiki_list", &e))?;
    Ok(notes)
}

/// 读取 wiki 笔记(元数据 + Markdown 正文)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_read"))]
pub async fn wiki_read(
    state: State<'_, AppState>,
    id: String,
) -> Result<WikiNoteReadResponse, CommandError> {
    let compiler = state.wiki.clone();
    let (note, markdown) = compiler
        .read(&id)
        .await
        .map_err(|e| CommandError::not_found(format!("wiki note {id}: {e}")))?;
    Ok(WikiNoteReadResponse { note, markdown })
}

/// FTS5 全文搜索。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_search"))]
pub async fn wiki_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<WikiNote>, CommandError> {
    let compiler = state.wiki.clone();
    let limit = limit.unwrap_or(20);
    let notes = compiler
        .search(&query, limit)
        .await
        .map_err(|e| CommandError::db("wiki_search", &e))?;
    Ok(notes)
}

/// 删除 wiki 笔记(幂等)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_delete"))]
pub async fn wiki_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    let compiler = state.wiki.clone();
    compiler
        .delete(&id)
        .await
        .map_err(|e| CommandError::db("wiki_delete", &e))?;
    Ok(())
}

/// T-E-B-03: 用户编辑 wiki 笔记后的双向同步命令。
///
/// 调用 `WikiCompiler::update_note_from_user`,内部执行:
/// 1. SQLite UPDATE `wiki_notes.body` + `updated_at`
/// 2. `sponge.absorb_text(&new_body)` 重新向量化(graceful degrade:失败仅 warn)
/// 3. `storage.write(&path, new_body)` 重写 markdown 文件
/// 4. `version_control.commit(...)` 写版本记录(graceful degrade:失败仅 warn)
/// 5. `append_log(LogEvent::Updated)` 追加到 `_log.md`
///
/// sponge / version_control 未注入时,对应步骤被跳过,SQLite + 文件重写 +
/// LogEvent::Updated 仍执行 — 保证编辑主路径不受可选依赖影响。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_update_from_user"))]
pub async fn wiki_update_from_user(
    state: State<'_, AppState>,
    note_id: String,
    new_body: String,
) -> Result<(), CommandError> {
    let compiler = state.wiki.clone();
    compiler
        .update_note_from_user(&note_id, new_body)
        .await
        .map_err(|e| CommandError::db("wiki_update_from_user", &e))?;
    Ok(())
}

/// 全量重生成 `_index.md`(T-E-B-06)。
///
/// 供前端"刷新目录"按钮调用:拉全部 wiki_notes → 按 importance DESC +
/// created_at ASC 排序 → 原子写 `<wiki_dir>/_index.md`。
/// 失败返回 `CommandError` 供前端 toast 显示。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_regen_index"))]
pub async fn wiki_regen_index(state: State<'_, AppState>) -> Result<(), CommandError> {
    let compiler = state.wiki.clone();
    if !compiler_enabled(&state) {
        return Err(CommandError::validation("wiki compiler disabled"));
    }
    compiler
        .regenerate_index()
        .await
        .map_err(|e| CommandError::db("wiki_regen_index", &e))?;
    Ok(())
}

/// T-E-B-05: 获取反向链接(所有指向 note_id 的笔记)。
///
/// 查 wiki_note_links WHERE target_id = ?1 获取 source_id 列表,
/// 再批量查 WikiNote 返回。供前端笔记详情页展示"被哪些笔记引用"。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_backlinks"))]
pub async fn wiki_backlinks(
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<WikiNote>, CommandError> {
    let compiler = state.wiki.clone();
    compiler
        .get_backlinks(&note_id)
        .map_err(|e| CommandError::db("wiki_backlinks", &e))
}

/// T-E-B-13: 获取知识卡片(聚合 note + body + definition + related_entities + backlinks)。
///
/// 供前端 KnowledgeCardDialog 弹窗调用:点击 `[[xxx]]` wiki-link 后,
/// 前端传 `slug` 调本命令,后端聚合返回 `KnowledgeCard`,弹窗渲染
/// 标题 / 定义 / 正文(markdown) / 关联实体 / 反向链接。
///
/// `slug` 为笔记的文件名安全 slug(前端 `[[xxx]]` 链接的 xxx 部分,非 UUID)。
/// slug 不存在返回 `not_found` 错误。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "wiki_get_card"))]
pub async fn wiki_get_card(
    state: State<'_, AppState>,
    slug: String,
) -> Result<KnowledgeCard, CommandError> {
    let compiler = state.wiki.clone();
    compiler
        .get_card(&slug)
        .await
        .map_err(|e| CommandError::not_found(format!("wiki card {slug}: {e}")))
}

/// 检查 wiki 编译器是否启用(配置开关)。
fn compiler_enabled(state: &State<'_, AppState>) -> bool {
    state.wiki.is_enabled()
}
