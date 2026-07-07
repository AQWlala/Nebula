//! T-E-S-28: 对话标注 Tauri 命令(upsert / list / stats / export)。
//!
//! 这 4 个命令封装 `memory::annotations::AnnotationStore`,供前端
//! ChatPanel.tsx 的 👍/👎 按钮调用。`annotation_upsert` 在 bad 标注
//! 且带 comment 时,触发 `sponge.absorb_text` 把用户反馈回流到
//! L1 Episodic 记忆,让 AI 在后续对话中知晓用户偏好。

use tauri::State;
use tracing::warn;

use crate::commands::error::CommandError;
use crate::memory::annotations::{Annotation, AnnotationStats, AnnotationStore};
use crate::memory::types::{MemoryLayer, MemoryType, SourceKind};
use crate::AppState;

/// T-E-S-28: 写入/更新一条对话标注。
///
/// `annotation` 取值 `"good"` / `"bad"`(SQL CHECK 约束)。`UNIQUE(turn_id)`
/// + `INSERT OR REPLACE` 保证幂等:用户反复点击 👍/👎 只保留最新一条。
///
/// 若 `annotation == "bad"` 且 `comment` 非空,触发 `sponge.absorb_text`
/// 把用户反馈回流到 L1 Episodic 记忆(让 AI 在后续对话中知晓用户偏好)。
/// 吸收失败不阻断标注写入(best-effort,仅 warn 日志)。
#[tauri::command]
pub async fn annotation_upsert(
    state: State<'_, AppState>,
    turn_id: String,
    annotation: String,
    comment: Option<String>,
    agent_role: Option<String>,
    model: Option<String>,
    conversation_id: Option<String>,
) -> Result<(), CommandError> {
    // 参数校验:annotation 必须是 good/bad(与 SQL CHECK 约束对齐)。
    if annotation != "good" && annotation != "bad" {
        return Err(CommandError::validation(format!(
            "annotation must be `good` or `bad`, got `{annotation}`"
        )));
    }
    if turn_id.trim().is_empty() {
        return Err(CommandError::validation("turn_id must not be empty"));
    }

    let ann = Annotation {
        turn_id: turn_id.clone(),
        annotation: annotation.clone(),
        comment: comment.clone(),
        agent_role: agent_role.clone(),
        model: model.clone(),
        conversation_id: conversation_id.clone(),
        created_at: chrono::Utc::now().timestamp_millis(),
    };

    // 同步 SQLite 写入走 spawn_blocking(与 SqliteStore 现有模式一致)。
    // store.upsert 返回 anyhow::Result,直接传 &e 给 CommandError::internal;
    // spawn_blocking 的 JoinError 用 anyhow::anyhow! 包装(参考 device.rs 模式)。
    let db = state.sqlite.clone();
    let ann_for_db = ann.clone();
    tokio::task::spawn_blocking(move || {
        let store = AnnotationStore::new(db);
        store
            .upsert(&ann_for_db)
            .map_err(|e| CommandError::internal("annotation_upsert", &e))
    })
    .await
    .map_err(|e| {
        CommandError::internal("annotation_upsert", &anyhow::anyhow!("join error: {e}"))
    })??;

    // T-E-S-28: bad 标注 + 非空 comment → 触发 sponge.absorb_text 回流到记忆。
    // 用 SourceKind::UserInput(无 Feedback 变体);tool = Some("annotation")
    // 写入 metadata.provenance_tool 供审计追踪。
    // 吸收失败不阻断标注写入(best-effort)。
    if annotation == "bad" {
        if let Some(ref c) = comment {
            if !c.trim().is_empty() {
                let feedback = format!("User feedback (turn {}): {}", turn_id, c);
                let sponge = state.sponge.clone();
                let feedback_for_absorb = feedback.clone();
                tokio::spawn(async move {
                    if let Err(e) = sponge
                        .absorb_text(
                            MemoryType::Episodic,
                            MemoryLayer::L1,
                            feedback_for_absorb,
                            SourceKind::UserInput,
                            Some("annotation"),
                        )
                        .await
                    {
                        warn!(
                            target: "nebula.cmd",
                            error = ?e,
                            "sponge absorb_text for bad annotation feedback failed (best-effort, not blocking)"
                        );
                    }
                });
            }
        }
    }

    Ok(())
}

/// T-E-S-28: 列出最近 `limit` 条标注(新在前)。`limit = null` 时返回最近 1000 条。
#[tauri::command]
pub async fn annotation_list(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<Annotation>, CommandError> {
    let db = state.sqlite.clone();
    tokio::task::spawn_blocking(move || {
        let store = AnnotationStore::new(db);
        store
            .list(limit)
            .map_err(|e| CommandError::internal("annotation_list", &e))
    })
    .await
    .map_err(|e| CommandError::internal("annotation_list", &anyhow::anyhow!("join error: {e}")))?
}

/// T-E-S-28: 聚合统计 good/bad 总数 + 按 model/agent 分桶。
#[tauri::command]
pub async fn annotation_stats(state: State<'_, AppState>) -> Result<AnnotationStats, CommandError> {
    let db = state.sqlite.clone();
    tokio::task::spawn_blocking(move || {
        let store = AnnotationStore::new(db);
        store
            .stats()
            .map_err(|e| CommandError::internal("annotation_stats", &e))
    })
    .await
    .map_err(|e| CommandError::internal("annotation_stats", &anyhow::anyhow!("join error: {e}")))?
}

/// T-E-S-28: 导出标注数据。
///
/// `format` 取值:
/// - `"jsonl"`: 每行一个 `Annotation` 的 JSON 序列化(原始行格式)。
/// - `"dify"`: Dify 训练数据集 JSONL,每行
///   `{"conversation": "...", "message": "...", "score": 0|1, "feedback": "..."}`
#[tauri::command]
pub async fn annotation_export(
    state: State<'_, AppState>,
    format: String,
) -> Result<String, CommandError> {
    let db = state.sqlite.clone();
    tokio::task::spawn_blocking(move || {
        let store = AnnotationStore::new(db);
        store
            .export(&format)
            .map_err(|e| CommandError::internal("annotation_export", &e))
    })
    .await
    .map_err(|e| CommandError::internal("annotation_export", &anyhow::anyhow!("join error: {e}")))?
}
