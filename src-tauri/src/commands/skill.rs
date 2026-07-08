//! Skill commands — CRUD, import, marketplace, audit.

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::skills::types as skill_types;
use crate::AppState;

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_create"))]
pub async fn skill_create(
    state: State<'_, AppState>,
    request: skill_types::CreateSkillRequest,
) -> Result<skill_types::Skill, CommandError> {
    state
        .swarm
        .skills
        .create_skill(request)
        .map_err(|e| CommandError::db("skill_create", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_use"))]
pub async fn skill_use(
    state: State<'_, AppState>,
    request: skill_types::UseSkillRequest,
) -> Result<skill_types::SkillResult, CommandError> {
    // v1.1: Prompt injection scan on skill input.
    let input_text = format!("{:?}", request);
    let scan = crate::security::injection_guard::full_injection_scan(&input_text);
    if let Some(severity) = scan.max_severity {
        if severity >= crate::security::injection_guard::InjectionSeverity::Critical {
            tracing::warn!(
                target: "nebula.cmd",
                "blocked critical injection / credential leak in skill_use"
            );
            return Err(CommandError::validation("skill_use").with_details(
                "输入包含潜在的安全风险（注入攻击或凭证泄露），已被拦截".to_string(),
            ));
        }
    }

    state
        .swarm
        .skills
        .use_skill(request)
        .await
        .map_err(|e| CommandError::internal("skill_use", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_rate"))]
pub async fn skill_rate(
    state: State<'_, AppState>,
    request: skill_types::RateSkillRequest,
) -> Result<skill_types::Skill, CommandError> {
    state
        .swarm
        .skills
        .rate_skill(request)
        .map_err(|e| CommandError::db("skill_rate", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_list"))]
pub async fn skill_list(
    state: State<'_, AppState>,
    request: skill_types::ListSkillsRequest,
) -> Result<Vec<skill_types::Skill>, CommandError> {
    state
        .swarm
        .skills
        .list_skills(request)
        .map_err(|e| CommandError::db("skill_list", &e))
}

#[tauri::command]
#[instrument(skip(state, request), fields(otel.kind = "skill_search"))]
pub async fn skill_search(
    state: State<'_, AppState>,
    request: skill_types::SkillSearchRequest,
) -> Result<Vec<skill_types::Skill>, CommandError> {
    state
        .swarm
        .skills
        .search_skills(request)
        .map_err(|e| CommandError::db("skill_search", &e))
}

/// T-E-S-37: 返回所有 skill 的 tag 频次(按 count 降序)。
///
/// 供前端显示热门标签云 — 顶部显示前 N 个 tag + 频次,用户点击后切换 tag 过滤。
/// 返回 `Vec<TagCount>`(tag + count),空库时返回空 Vec。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_tags"))]
pub async fn skill_tags(
    state: State<'_, AppState>,
) -> Result<Vec<skill_types::TagCount>, CommandError> {
    Ok(state.swarm.skills.all_tags())
}

/// Stub: import skill from external registry (v1.2 eco compatibility).
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_import"))]
pub async fn skill_import(
    state: State<'_, AppState>,
    source: String,
    identifier: String,
) -> Result<crate::skills::importer::ImportResult, CommandError> {
    let source = match source.as_str() {
        "agentskills" => crate::skills::importer::SkillSource::AgentskillsIo,
        "clawhub" => crate::skills::importer::SkillSource::ClawHub,
        "teamskillshub" => crate::skills::importer::SkillSource::TeamSkillsHub,
        other => {
            return Err(CommandError::validation("skill_import")
                .with_details(format!("unknown source: {other}")))
        }
    };
    let importer = crate::skills::importer::SkillImporter::new(state.swarm.skills.store().clone());
    let result = match source {
        crate::skills::importer::SkillSource::AgentskillsIo => {
            importer.import_from_url(&identifier).await
        }
        crate::skills::importer::SkillSource::ClawHub => {
            importer.import_from_clawhub(&identifier).await
        }
        crate::skills::importer::SkillSource::TeamSkillsHub => {
            importer.import_from_teamskillshub(&identifier).await
        }
    };
    if result.success {
        Ok(result)
    } else {
        Err(CommandError::internal(
            "skill_import",
            &anyhow::anyhow!("import failed"),
        ))
    }
}

// -----------------------------------------------------------------------
// T-E-S-45: ClawHub bidirectional compatibility — skill export.
// -----------------------------------------------------------------------

/// T-E-S-45: 把指定 skill 导出为 agentskills.io `SKILL.md` 格式。
///
/// - `output_path = None`:返回 `{"content": "<SKILL.md 字符串>"}`。
/// - `output_path = Some(p)`:写入文件 `p`,返回 `{"path": "<p>"}`。
///
/// 字段映射与 `importer::from_skill_md` 对称(8 个核心字段无损往返)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_export_clawhub"))]
pub async fn skill_export_clawhub(
    state: State<'_, AppState>,
    skill_id: String,
    output_path: Option<String>,
) -> Result<serde_json::Value, CommandError> {
    let skill = state
        .swarm
        .skills
        .store()
        .get(&skill_id)
        .map_err(|e| CommandError::db("skill_export_clawhub", &e))?
        .ok_or_else(|| CommandError::not_found("skill"))?;

    let md = crate::skills::exporter::SkillExporter::to_skill_md(&skill)
        .map_err(|e| CommandError::internal("skill_export_clawhub", &e))?;

    match output_path {
        Some(path) => {
            let p = std::path::PathBuf::from(&path);
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        CommandError::internal(
                            "skill_export_clawhub",
                            &anyhow::anyhow!("failed to create parent dir: {e}"),
                        )
                    })?;
                }
            }
            std::fs::write(&p, &md).map_err(|e| {
                CommandError::internal(
                    "skill_export_clawhub",
                    &anyhow::anyhow!("failed to write file: {e}"),
                )
            })?;
            Ok(serde_json::json!({ "path": path }))
        }
        None => Ok(serde_json::json!({ "content": md })),
    }
}

// -----------------------------------------------------------------------
// v1.3 P2-7: skill marketplace.
// -----------------------------------------------------------------------

/// Search the skill marketplace.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_search"))]
pub async fn marketplace_search(
    state: State<'_, AppState>,
    query: crate::skills::marketplace::MarketplaceQuery,
) -> Result<crate::skills::marketplace::MarketplaceResponse, CommandError> {
    state
        .swarm
        .marketplace
        .search(&query)
        .map_err(|e| CommandError::internal("marketplace_search", &e))
}

/// Quick search — top 10 results for autocomplete.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_quick_search"))]
pub async fn marketplace_quick_search(
    state: State<'_, AppState>,
    text: String,
) -> Result<crate::skills::marketplace::MarketplaceResponse, CommandError> {
    let q = crate::skills::marketplace::MarketplaceQuery {
        text: Some(text),
        limit: 10,
        ..Default::default()
    };
    state
        .swarm
        .marketplace
        .search(&q)
        .map_err(|e| CommandError::internal("marketplace_quick_search", &e))
}

/// One-click install from remote registry.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_install"))]
pub async fn marketplace_install(
    state: State<'_, AppState>,
    source: String,
    identifier: String,
) -> Result<crate::skills::marketplace::SkillEntry, CommandError> {
    state
        .swarm
        .marketplace
        .install(&source, &identifier)
        .map_err(|e| CommandError::internal("marketplace_install", &e))
}

/// Check for skill updates.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_check_updates"))]
pub async fn marketplace_check_updates(
    state: State<'_, AppState>,
) -> Result<Vec<crate::skills::marketplace::UpdateInfo>, CommandError> {
    Ok(state.swarm.marketplace.check_updates())
}

/// Refresh marketplace index.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_refresh"))]
pub async fn marketplace_refresh(
    state: State<'_, AppState>,
) -> Result<crate::skills::marketplace::MarketplaceStats, CommandError> {
    state
        .swarm
        .marketplace
        .refresh()
        .map_err(|e| CommandError::internal("marketplace_refresh", &e))
}

/// Get marketplace stats.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_stats"))]
pub async fn marketplace_stats(
    state: State<'_, AppState>,
) -> Result<crate::skills::marketplace::MarketplaceStats, CommandError> {
    Ok(state.swarm.marketplace.stats())
}

/// Get all tags with frequencies.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_tags"))]
pub async fn marketplace_tags(
    state: State<'_, AppState>,
) -> Result<Vec<(String, usize)>, CommandError> {
    Ok(state.swarm.marketplace.all_tags())
}

/// Generate publish manifest for a skill.
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "marketplace_generate_manifest"))]
pub async fn marketplace_generate_manifest(
    state: State<'_, AppState>,
    skill_id: String,
) -> Result<crate::skills::marketplace::PublishManifest, CommandError> {
    state
        .swarm
        .marketplace
        .generate_manifest(&skill_id)
        .map_err(|e| CommandError::internal("marketplace_generate_manifest", &e))
}

// -----------------------------------------------------------------------
// T-E-S-46: skill publish (GitHub Gist / local file)
// -----------------------------------------------------------------------

/// T-E-S-46: 发布技能到社区市场。
///
/// `target` 取值:
/// * `"gist"`:上传 `SKILL.md` 到 GitHub Gist,需先在 keychain
///   `publisher:github` slot 配置 PAT。返回 `html_url`。
/// * `"file"`:导出 `SKILL.md` 到本地目录(`<db_dir>/skills_export/<id>.md`)。
///
/// 返回 JSON: `{ "target": ..., "url": ..., "file_path": ... }`。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_publish"))]
pub async fn skill_publish(
    state: State<'_, AppState>,
    skill_id: String,
    target: String,
) -> Result<serde_json::Value, CommandError> {
    use crate::security::keychain;
    use crate::skills::publisher::{
        skill_to_skill_md, FilePublisher, GistPublisher, SkillPublisher,
    };

    // 1. 读取 skill。
    let skill = state
        .swarm
        .skills
        .store()
        .get(&skill_id)
        .map_err(|e| CommandError::db("skill_publish", &e))?
        .ok_or_else(|| CommandError::not_found(format!("skill: {skill_id}")))?;

    // 2. 生成 SKILL.md(内联 to_skill_md,不依赖 exporter.rs)。
    let skill_md =
        skill_to_skill_md(&skill).map_err(|e| CommandError::internal("skill_publish", &e))?;

    // 3. 生成 PublishManifest 并校验。
    let manifest = state
        .swarm
        .marketplace
        .generate_manifest(&skill_id)
        .map_err(|e| CommandError::internal("skill_publish", &e))?;
    crate::skills::marketplace::SkillMarketplace::validate_manifest(&manifest)
        .map_err(|e| CommandError::internal("skill_publish", &e))?;

    // 4. 根据 target 分发。
    let result = match target.as_str() {
        "gist" => {
            let token = keychain::get_publisher_token("github")
                .map_err(|e| CommandError::internal("skill_publish", &e))?;
            let publisher =
                GistPublisher::new().map_err(|e| CommandError::internal("skill_publish", &e))?;
            publisher
                .publish(&skill_md, &manifest, token.as_deref())
                .await
                .map_err(|e| CommandError::internal("skill_publish", &e))?
        }
        "file" => {
            // 默认导出到 <db_dir>/skills_export/。
            let db_path = std::path::Path::new(&state.infra.config.db_path);
            let out_dir = db_path
                .parent()
                .map(|p| p.join("skills_export"))
                .unwrap_or_else(|| std::path::PathBuf::from("./skills_export"));
            let publisher = FilePublisher::new(&out_dir);
            publisher
                .publish(&skill_md, &manifest, None)
                .await
                .map_err(|e| CommandError::internal("skill_publish", &e))?
        }
        other => {
            return Err(CommandError::validation(format!(
                "unknown target: {other}; expected `gist` or `file`"
            )));
        }
    };

    serde_json::to_value(&result)
        .map_err(|e| CommandError::internal("skill_publish", &anyhow::anyhow!("{e}")))
}

// -----------------------------------------------------------------------
// v1.3: Skill audit log commands
// -----------------------------------------------------------------------

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_audit_list"))]
pub async fn skill_audit_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<crate::skills::audit::SkillAuditEntry>, CommandError> {
    let logger = state.swarm.skill_audit_logger.clone();
    tokio::task::spawn_blocking(move || {
        logger
            .list(limit.unwrap_or(50))
            .map_err(|e| CommandError::db("skill_audit_list", &e))
    })
    .await
    .map_err(|e| CommandError::internal("skill_audit_list", &anyhow::anyhow!("{e}")))?
}

#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_audit_list_for_skill"))]
pub async fn skill_audit_list_for_skill(
    state: State<'_, AppState>,
    skill_id: String,
    limit: Option<usize>,
) -> Result<Vec<crate::skills::audit::SkillAuditEntry>, CommandError> {
    let logger = state.swarm.skill_audit_logger.clone();
    tokio::task::spawn_blocking(move || {
        logger
            .list_for_skill(&skill_id, limit.unwrap_or(50))
            .map_err(|e| CommandError::db("skill_audit_list_for_skill", &e))
    })
    .await
    .map_err(|e| CommandError::internal("skill_audit_list_for_skill", &anyhow::anyhow!("{e}")))?
}

// -----------------------------------------------------------------------
// T-E-A-06 / T-E-S-20: Cost summary + exec approval list
// -----------------------------------------------------------------------

/// T-E-A-06: 返回 Token 费用按日聚合（供前端费用面板）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "cost_summary"))]
pub async fn cost_summary(
    state: State<'_, AppState>,
) -> Result<Vec<crate::llm::cost_tracker::DailyAggregate>, CommandError> {
    Ok(state.llm.cost_tracker.aggregate_by_day())
}

/// T-E-S-20: 返回当前所有 exec 审批请求快照（按创建时间升序）。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "exec_approval_list"))]
pub async fn exec_approval_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::skills::exec_approval::ExecApprovalRequest>, CommandError> {
    Ok(state.swarm.exec_approval.list_all())
}
