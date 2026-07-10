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
        // P1-6: OpenClaw 兼容源 — 与 agentskills.io SKILL.md 格式完全兼容。
        "openclaw" => crate::skills::importer::SkillSource::OpenClaw,
        other => {
            return Err(CommandError::validation("skill_import")
                .with_details(format!("unknown source: {other}")))
        }
    };
    let importer = crate::skills::importer::SkillImporter::new(state.swarm.skills.store().clone());
    // T-D-B-10: 若 source=teamskillshub 且配置了 NEBULA_TEAM_SKILLS_HUB_URL,
    // 注入 TeamSkillsHubClient 使 import_from_teamskillshub 不再返回 stub 错误。
    let importer = if matches!(source, crate::skills::importer::SkillSource::TeamSkillsHub) {
        if let Ok(base_url) = std::env::var("NEBULA_TEAM_SKILLS_HUB_URL") {
            if !base_url.is_empty() {
                let hub = crate::skills::hub_client::TeamSkillsHubClient::new(&base_url);
                importer.with_hub_client(Some(hub))
            } else {
                importer
            }
        } else {
            importer
        }
    } else {
        importer
    };
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
        // P1-6: OpenClaw 社区市场安装。
        crate::skills::importer::SkillSource::OpenClaw => {
            importer.import_from_openclaw(&identifier).await
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
// T-D-B-10: Skill 生态补齐 — 发现层 + 规范层 + 导入层 Tauri 命令
// -----------------------------------------------------------------------

/// T-D-B-10: 扫描 4 层目录(project / user / system / workspace)+ 可选
/// 额外路径,发现 `SKILL.md` 文件并写入 store。
///
/// 返回 `Vec<DiscoveryResult>`(每个被发现的 SKILL.md 一项,含路径 /
/// skill_id / 状态 / 错误信息),供前端展示加载进度与失败原因。
///
/// `extra_paths`:可选的额外扫描目录(如用户在设置中指定的自定义技能目录)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_discover"))]
pub async fn skill_discover(
    state: State<'_, AppState>,
    extra_paths: Option<Vec<String>>,
) -> Result<Vec<crate::skills::discover::DiscoveryResult>, CommandError> {
    let store = state.swarm.skills.store().clone();
    let extra: Vec<std::path::PathBuf> = extra_paths
        .unwrap_or_default()
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect();
    let discoverer = crate::skills::discover::SkillDiscoverer::new().with_extra_paths(&extra);
    tokio::task::spawn_blocking(move || Ok(discoverer.discover_with_details(&store)))
        .await
        .map_err(|e| CommandError::internal("skill_discover", &anyhow::anyhow!("{e}")))?
}

/// T-D-B-10: 返回当前会扫描的目录列表(不实际执行扫描)。
///
/// 供前端在设置页显示"将在以下目录查找 SKILL.md"。包含 4 层默认路径
/// + 已通过 `with_extra_paths` 注入的额外路径(仅返回实际存在的目录)。
#[tauri::command]
#[instrument(skip_all, fields(otel.kind = "skill_scan_paths"))]
pub async fn skill_scan_paths(_state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    let discoverer = crate::skills::discover::SkillDiscoverer::new();
    Ok(discoverer
        .scan_paths()
        .iter()
        .map(|p| p.display().to_string())
        .collect())
}

/// T-D-B-10: 校验一段 SKILL.md 内容,返回规范校验报告。
///
/// 校验 3 个层面:
/// 1. **结构层**:YAML frontmatter 存在且可解析;`name`/`version` 必填;
///    `version` 必须是 semver (X.Y.Z)。
/// 2. **规范层**:`transport` 合法;`status` 枚举值合法;`min_nebula_version`
///    (若提供)是 semver。
/// 3. **资格层**:`eligibility.bins` 在 PATH 中可找到;`eligibility.env`
///    已设置且非空;`eligibility.os` 白名单命中当前 OS。
///
/// 返回 `SkillSpecReport`(valid/eligible/manifest/errors/warnings/
/// eligibility_failures)。前端可用此命令在导入前预检 SKILL.md。
#[tauri::command]
#[instrument(fields(otel.kind = "skill_validate_md"))]
pub async fn skill_validate_md(
    content: String,
) -> Result<crate::skills::protocol::SkillSpecReport, CommandError> {
    Ok(crate::skills::protocol::SkillSpecValidator::validate_skill_md(&content))
}

/// T-D-B-10: 从 TeamSkillsHub 按 asset ID 导入技能。
///
/// 需要 `NEBULA_TEAM_SKILLS_HUB_URL` 环境变量配置 hub 根地址
/// (如 `https://skills.example.com`)。未配置时返回验证错误。
///
/// 与通用 `skill_import`(source=`teamskillshub`)的区别:本命令显式
/// 构造 `TeamSkillsHubClient` 并注入 `SkillImporter`,使
/// `import_from_teamskillshub` 不再返回 stub 错误。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "skill_import_teamskillshub"))]
pub async fn skill_import_teamskillshub(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<crate::skills::importer::ImportResult, CommandError> {
    let base_url = match std::env::var("NEBULA_TEAM_SKILLS_HUB_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            return Err(CommandError::validation(
                "skill_import_teamskillshub: NEBULA_TEAM_SKILLS_HUB_URL not set",
            ));
        }
    };
    let hub = crate::skills::hub_client::TeamSkillsHubClient::new(&base_url);
    let importer = crate::skills::importer::SkillImporter::new(state.swarm.skills.store().clone())
        .with_hub_client(Some(hub));
    let result = importer.import_from_teamskillshub(&asset_id).await;
    if result.success {
        Ok(result)
    } else {
        Err(CommandError::internal(
            "skill_import_teamskillshub",
            &anyhow::anyhow!(
                "import failed: {}",
                result.error.as_deref().unwrap_or("unknown error")
            ),
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
