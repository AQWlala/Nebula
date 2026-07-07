//! Data export/import commands.

use chrono::{TimeZone, Utc};
use docx_rs::*;
use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageForExport {
    pub role: String,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatExportOptions {
    pub title: Option<String>,
    pub include_timestamps: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatExportResult {
    pub file_path: String,
    pub byte_size: usize,
}

#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "export_chat_docx"))]
pub async fn export_chat_docx(
    _state: State<'_, AppState>,
    messages: Vec<ChatMessageForExport>,
    options: ChatExportOptions,
) -> Result<ChatExportResult, CommandError> {
    let title = options
        .title
        .unwrap_or_else(|| "Nebula对话导出".to_string());
    let include_timestamps = options.include_timestamps.unwrap_or(false);

    let mut doc = Docx::new()
        .default_fonts(RunFonts::new().east_asia("Microsoft YaHei").ascii("Arial"))
        .page_size(11906, 16838);

    doc = doc.add_paragraph(
        Paragraph::new()
            .add_run(Run::new().add_text(&title).bold().size(32))
            .align(AlignmentType::Center),
    );

    let now = Utc::now();
    let now_str = format!(
        "导出时间: {} 消息数: {}",
        now.format("%Y-%m-%d %H:%M:%S"),
        messages.len()
    );
    doc = doc.add_paragraph(
        Paragraph::new()
            .add_run(Run::new().add_text(&now_str).size(20).color("808080"))
            .align(AlignmentType::Center),
    );

    doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(" ")));

    for msg in &messages {
        let (role_label, color) = if msg.role == "user" {
            ("用户", "2196F3")
        } else {
            ("Nebula", "4CAF50")
        };

        doc = doc.add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(role_label).bold().size(24).color(color)),
        );

        if include_timestamps && msg.timestamp > 0 {
            if let Some(dt) = Utc.timestamp_millis_opt(msg.timestamp).single() {
                let ts_str = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                doc = doc.add_paragraph(
                    Paragraph::new().add_run(Run::new().add_text(&ts_str).size(18).color("808080")),
                );
            }
        }

        let content = &msg.content;
        for line in content.lines() {
            doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(line).size(22)));
        }

        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(" ")));
    }

    let tmp_dir = std::env::temp_dir();
    let filename = format!("chat-export-{}.docx", Utc::now().timestamp_millis());
    let file_path = tmp_dir.join(&filename);

    let file = std::fs::File::create(&file_path)
        .map_err(|e| CommandError::internal("export_chat_docx", &anyhow::anyhow!("{:?}", e)))?;

    doc.build()
        .pack(std::io::BufWriter::new(file))
        .map_err(|e| CommandError::internal("export_chat_docx", &anyhow::anyhow!("{:?}", e)))?;

    let metadata = std::fs::metadata(&file_path)
        .map_err(|e| CommandError::internal("export_chat_docx", &anyhow::anyhow!("{:?}", e)))?;
    let byte_size = metadata.len() as usize;

    Ok(ChatExportResult {
        file_path: file_path.to_string_lossy().to_string(),
        byte_size,
    })
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "export_memories"))]
pub async fn export_memories(
    state: State<'_, AppState>,
    format: String,
    path: String,
) -> Result<crate::memory::export::ExportManifest, CommandError> {
    let exporter = crate::memory::export::DataExporter::new((*state.sqlite).clone());
    let p = std::path::PathBuf::from(&path);
    match format.as_str() {
        "jsonld" | "json-ld" => exporter
            .export_jsonld(&p)
            .await
            .map_err(|e| CommandError::internal("export_memories", &e)),
        _ => Err(CommandError::validation("export_memories")
            .with_details(format!("unsupported format: {format}"))),
    }
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "import_memories"))]
pub async fn import_memories(
    state: State<'_, AppState>,
    path: String,
) -> Result<crate::memory::export::ImportResult, CommandError> {
    let exporter = crate::memory::export::DataExporter::new((*state.sqlite).clone());
    let p = std::path::PathBuf::from(&path);
    exporter
        .import_jsonld(&p)
        .await
        .map_err(|e| CommandError::internal("import_memories", &e))
}
