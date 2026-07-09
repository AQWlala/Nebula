//! Writing-mode commands — templates, documents, export.

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::writing::{
    Document, DocumentExport, ExportFormat, RenderedScenarioTemplate, WritingScenarioCategory,
    WritingTemplate,
};
use crate::AppState;

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_list_templates"))]
pub async fn writing_list_templates(
    state: State<'_, AppState>,
) -> Result<Vec<WritingTemplate>, CommandError> {
    Ok(state.platform.writing.list_templates())
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_get_template"))]
pub async fn writing_get_template(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<WritingTemplate>, CommandError> {
    Ok(state.platform.writing.get_template(&id))
}

/// T-D-B-18: 按场景类别列出模板。
///
/// `category` 接受 `general` / `self_media` / `novel`(及中文别名「通用」/
/// 「自媒体」/「长篇小说」/「小说」),未知值回退到 `general`。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_list_templates_by_category"))]
pub async fn writing_list_templates_by_category(
    state: State<'_, AppState>,
    category: String,
) -> Result<Vec<WritingTemplate>, CommandError> {
    let cat = WritingScenarioCategory::parse(&category);
    Ok(state.platform.writing.list_templates_by_category(cat))
}

/// T-D-B-18: 场景概览,供前端模板选择器分组展示。
/// 返回与 `AgentScenario::Writing` 对应的 28 个场景模板(14 自媒体 + 14 长篇小说)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_list_scenarios"))]
pub async fn writing_list_scenarios(
    state: State<'_, AppState>,
) -> Result<Vec<WritingTemplate>, CommandError> {
    Ok(state.platform.writing.writing_scenario_templates())
}

/// T-D-B-18: 渲染场景模板,返回填好占位符的 body 与 LLM 提示词。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyScenarioRequest {
    pub template_id: String,
    /// 占位符名 → 值。
    pub values: std::collections::HashMap<String, String>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "writing_apply_scenario_template"))]
pub async fn writing_apply_scenario_template(
    state: State<'_, AppState>,
    request: ApplyScenarioRequest,
) -> Result<RenderedScenarioTemplate, CommandError> {
    let engine = state.platform.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .apply_scenario_template(&request.template_id, &request.values)
            .map_err(|e| CommandError::internal("writing_apply_scenario_template", &e))
    })
    .await
    .map_err(|e| {
        CommandError::internal("writing_apply_scenario_template", &anyhow::anyhow!("{e}"))
    })?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocumentRequest {
    pub title: String,
    pub template_id: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "writing_create_document"))]
pub async fn writing_create_document(
    state: State<'_, AppState>,
    request: CreateDocumentRequest,
) -> Result<Document, CommandError> {
    let engine = state.platform.writing.clone();
    let req = request;
    tokio::task::spawn_blocking(move || {
        engine
            .create_document(req.title, req.template_id, req.content, req.metadata)
            .map_err(|e| CommandError::internal("writing_create_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_create_document", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state, content), fields(otel.kind = "writing_update_document"))]
pub async fn writing_update_document(
    state: State<'_, AppState>,
    id: String,
    content: String,
) -> Result<Document, CommandError> {
    let engine = state.platform.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .update_document(&id, content)
            .map_err(|e| CommandError::internal("writing_update_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_update_document", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_get_document"))]
pub async fn writing_get_document(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Document>, CommandError> {
    let engine = state.platform.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .get_document(&id)
            .map_err(|e| CommandError::internal("writing_get_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_get_document", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_list_documents"))]
pub async fn writing_list_documents(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<Document>, CommandError> {
    let engine = state.platform.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .list_documents(limit.unwrap_or(50))
            .map_err(|e| CommandError::internal("writing_list_documents", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_list_documents", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "writing_delete_document"))]
pub async fn writing_delete_document(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let engine = state.platform.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .delete_document(&id)
            .map_err(|e| CommandError::internal("writing_delete_document", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_delete_document", &anyhow::anyhow!("{e}")))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    pub id: String,
    /// "markdown" | "html" | "md" | "htm"
    pub format: String,
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "writing_export"))]
pub async fn writing_export(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<DocumentExport, CommandError> {
    let format = ExportFormat::from_str(&request.format)
        .map_err(|e| CommandError::validation("writing_export").with_details(e.to_string()))?;
    let engine = state.platform.writing.clone();
    tokio::task::spawn_blocking(move || {
        engine
            .export(&request.id, format)
            .map_err(|e| CommandError::internal("writing_export", &e))
    })
    .await
    .map_err(|e| CommandError::internal("writing_export", &anyhow::anyhow!("{e}")))?
}
