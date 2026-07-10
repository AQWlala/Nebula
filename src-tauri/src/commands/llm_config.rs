//! P0-1: 模型配置中心 Tauri 命令。
//!
//! 为前端 `ModelConfigPanel` 提供 provider API Key 状态查询、连通性测试、
//! 模型自动发现、自定义 provider 增删、默认 provider/model 设置、WorkType
//! 路由配置等命令。
//!
//! ## 命令清单
//! * `get_provider_key_status` — 查询 keychain 中是否存在该 provider 的 key(返回 bool)。
//! * `test_provider_connection` — 发送测试请求,返回延迟和状态。
//! * `discover_models` — 自动拉取可用模型列表(Ollama /api/tags、OpenAI /v1/models、Anthropic 硬编码)。
//! * `discover_all_models` — P1-2: 并行发现所有已配置 provider 的模型列表。
//! * `auto_populate_models` — P1-2: 自动发现并将新模型写入 models.json(去重)。
//! * `add_custom_provider` — 添加自定义 provider,返回 provider_id。
//! * `remove_provider` — 删除 provider。
//! * `update_provider` — P1-3: 热更新 provider 字段(name / base_url / kind)。
//! * `set_default_provider` — 设置默认 provider 和 model。
//! * `set_worktype_routing` — 按 WorkType 设置路由。
//!
//! ## 关于 `set_provider_key`
//! `set_provider_key` 命令已存在于 `commands::models_config`(同名同义,slot
//! 命名 `provider:<id>`),并已在 `generate_handler!` 注册。此处不重复定义,
//! 前端直接调用既有 `set_provider_key` 命令即可,避免命令名冲突。
//!
//! ## 错误处理
//! 按 P0-1 规约,所有命令返回 `Result<T, String>`,错误以中文描述返回,不 panic。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::Datelike;
use serde::Serialize;
use tauri::State;
use tracing::instrument;

use crate::llm::models_config::{ModelConfig, ProviderConfig, ProviderKind, WorkTypeOverrideEntry};
use crate::security::SsrfGuard;
use crate::AppState;

/// 连通性测试结果。
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionTestResult {
    /// 是否连通。
    pub success: bool,
    /// 延迟(毫秒)。
    pub latency_ms: u64,
    /// 错误信息(success=false 时)。
    pub error: Option<String>,
}

/// 自动发现的模型信息。
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    /// 模型 id(如 `deepseek-chat`、`llama3.1:8b`)。
    pub id: String,
    /// 模型显示名(与 id 相同时直接用 id)。
    pub name: String,
    /// 上下文窗口(token 数),未知为 None。
    pub context_length: Option<u32>,
}

/// P1-2: 单个 provider 的模型发现结果(用于 `discover_all_models`)。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderModels {
    /// Provider id。
    pub provider_id: String,
    /// Provider 显示名。
    pub provider_name: String,
    /// 发现的模型列表(`error` 非 None 时为空)。
    pub models: Vec<ModelInfo>,
    /// 失败时的错误信息;成功时为 None。
    pub error: Option<String>,
}

/// P1-2: Anthropic 无 `/v1/models` 端点,返回硬编码模型列表。
///
/// 列表来自 Anthropic 官方公布的模型 id(含日期后缀),上下文窗口均为 200K。
fn anthropic_default_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "claude-sonnet-4-5-20250929".to_string(),
            name: "Claude Sonnet 4.5".to_string(),
            context_length: Some(200_000),
        },
        ModelInfo {
            id: "claude-opus-4-1-20250805".to_string(),
            name: "Claude Opus 4.1".to_string(),
            context_length: Some(200_000),
        },
        ModelInfo {
            id: "claude-haiku-4-5-20251022".to_string(),
            name: "Claude Haiku 4.5".to_string(),
            context_length: Some(200_000),
        },
    ]
}

/// 查询 keychain 中是否存在该 provider 的 key(不返回明文,只返回 bool)。
///
/// 复用 `security::keychain::get_provider_key`,slot 命名 `provider:<id>`。
#[tauri::command]
#[instrument(fields(otel.kind = "get_provider_key_status"))]
pub async fn get_provider_key_status(provider_id: String) -> Result<bool, String> {
    match crate::security::keychain::get_provider_key(&provider_id) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(format!("读取 keychain 失败: {e}")),
    }
}

/// 从 AppState 中查找 provider 的 kind;未找到时按 provider_id 推断。
///
/// 推断规则:provider_id 含 "ollama" → Ollama,否则 → OpenAiCompat(远端)。
fn resolve_provider_kind(state: &AppState, provider_id: &str) -> ProviderKind {
    let cfg = state.llm.models_config.read();
    if let Some(p) = cfg.find_provider(provider_id) {
        return p.kind;
    }
    if provider_id.to_lowercase().contains("ollama") {
        ProviderKind::Ollama
    } else {
        ProviderKind::OpenAiCompat
    }
}

/// 构建 SSRF 校验器。
///
/// 复用 `models_config.rs` 中 `validate()` 的内网地址拒绝逻辑:
/// - Ollama:允许 loopback + 私网(本地服务合法)。
/// - 远端 provider:允许 loopback(本地 vLLM/LMStudio 合法),拒绝其他私网。
fn build_ssrf_guard(kind: ProviderKind) -> SsrfGuard {
    match kind {
        ProviderKind::Ollama => SsrfGuard::new().with_allow_private(true),
        _ => SsrfGuard::new().with_allow_loopback(true),
    }
}

/// 拼接远端 provider 的 list-models 端点 URL。
///
/// 若 base_url 已以 `/v1` 结尾,直接追加 `/models`;否则追加 `/v1/models`。
fn join_models_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/v1/models")
    }
}

/// 拼接 Ollama 的 tags 端点 URL。
fn join_ollama_tags_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{trimmed}/api/tags")
}

/// 测试 provider 连通性。
///
/// 策略:
/// - Ollama:GET `{base_url}/api/tags`,2s 超时。
/// - 远端 provider:GET `{base_url}/v1/models`,5s 超时。
///   任何 HTTP 响应(含 401/403)都算连通,说明 server 可达。
///
/// SSRF 防护:复用 `models_config.rs` 中 `validate()` 的内网地址拒绝逻辑。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "test_provider_connection"))]
pub async fn test_provider_connection(
    state: State<'_, AppState>,
    provider_id: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<ConnectionTestResult, String> {
    if base_url.trim().is_empty() {
        return Ok(ConnectionTestResult {
            success: false,
            latency_ms: 0,
            error: Some("base_url 为空".to_string()),
        });
    }

    let kind = resolve_provider_kind(&state, &provider_id);
    let guard = build_ssrf_guard(kind);
    guard.validate_url(&base_url).map_err(|e| {
        format!("SSRF 校验失败: {e}")
    })?;

    let (test_url, timeout) = match kind {
        ProviderKind::Ollama => (join_ollama_tags_url(&base_url), Duration::from_secs(2)),
        _ => (join_models_url(&base_url), Duration::from_secs(5)),
    };

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

    let mut req = client.get(&test_url);
    if let Some(key) = api_key.as_ref().filter(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key);
    }

    let start = Instant::now();
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            // 任何 HTTP 响应都算连通(401/403 也说明 server 可达),不附带错误信息。
            let _ = status;
            Ok(ConnectionTestResult {
                success: true,
                latency_ms: start.elapsed().as_millis() as u64,
                error: None,
            })
        }
        Err(e) => Ok(ConnectionTestResult {
            success: false,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("连接失败: {e}")),
        }),
    }
}

/// 自动拉取可用模型列表。
///
/// 策略:
/// - Anthropic:无 `/v1/models` 端点,返回硬编码模型列表(无需网络)。
/// - Ollama:GET `{base_url}/api/tags`,解析 `{"models": [{"name", "details": {...}}]}`。
/// - OpenAI 兼容 / Custom:GET `{base_url}/v1/models`,解析 `{"data": [{"id", "owned_by", ...}]}`。
///
/// 超时 10 秒。SSRF 防护:同 `test_provider_connection`。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "discover_models"))]
pub async fn discover_models(
    state: State<'_, AppState>,
    provider_id: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<ModelInfo>, String> {
    fetch_models_from_provider(&state, &provider_id, &base_url, api_key.as_deref()).await
}

/// P1-2: `discover_models` 的核心逻辑(无 Tauri State 包装,便于内部复用)。
///
/// 与 `discover_models` 命令的区别:接受 `&AppState` 而非 `State<'_, AppState>`,
/// 供 `discover_models_for_provider` / `auto_populate_models` 复用。
async fn fetch_models_from_provider(
    state: &AppState,
    provider_id: &str,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>, String> {
    if base_url.trim().is_empty() {
        return Err("base_url 为空".to_string());
    }

    let kind = resolve_provider_kind(state, provider_id);

    // Anthropic: 无 /v1/models 端点,直接返回硬编码列表(跳过 SSRF / 网络)。
    if kind == ProviderKind::Anthropic {
        return Ok(anthropic_default_models());
    }

    let guard = build_ssrf_guard(kind);
    guard.validate_url(base_url).map_err(|e| {
        format!("SSRF 校验失败: {e}")
    })?;

    let (fetch_url, timeout) = match kind {
        ProviderKind::Ollama => (join_ollama_tags_url(base_url), Duration::from_secs(10)),
        _ => (join_models_url(base_url), Duration::from_secs(10)),
    };

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

    let mut req = client.get(&fetch_url);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("服务端返回 HTTP {}", status.as_u16()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应 JSON 失败: {e}"))?;

    let models = match kind {
        ProviderKind::Ollama => {
            // Ollama /api/tags → {"models": [{"name": "llama3.1:8b", "details": {"parameter_size": "8B", "quantization_level": "Q4_0"}}]}
            body.get("models")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let id = item.get("name")?.as_str()?.to_string();
                            // 从 details 中拼接显示名(如 "llama3.1:8b (8B Q4_0)")。
                            let details = item.get("details");
                            let param_size = details
                                .and_then(|d| d.get("parameter_size"))
                                .and_then(|p| p.as_str());
                            let quant = details
                                .and_then(|d| d.get("quantization_level"))
                                .and_then(|q| q.as_str());
                            let name = match (param_size, quant) {
                                (Some(p), Some(q)) => format!("{id} ({p} {q})"),
                                (Some(p), None) => format!("{id} ({p})"),
                                _ => id.clone(),
                            };
                            Some(ModelInfo {
                                name,
                                id,
                                context_length: None,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
        _ => {
            // OpenAI 兼容 /v1/models → {"data": [{"id": "gpt-4o", "owned_by": "openai", "context_length": 128000}]}
            body.get("data")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let id = item.get("id")?.as_str()?.to_string();
                            // 用 owned_by 作为显示名补充(如 "gpt-4o (openai)");无则与 id 相同。
                            let owned_by = item
                                .get("owned_by")
                                .and_then(|o| o.as_str());
                            let name = match owned_by {
                                Some(o) if !o.is_empty() => format!("{id} ({o})"),
                                _ => id.clone(),
                            };
                            Some(ModelInfo {
                                name,
                                id,
                                context_length: item
                                    .get("context_length")
                                    .and_then(|c| c.as_u64())
                                    .and_then(|c| u32::try_from(c).ok()),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
    };

    Ok(models)
}

/// P1-2: 按 provider_id 解析配置(base_url + api_key)后调用模型发现。
///
/// 与 `discover_models` 命令的区别:不需要前端传 base_url/api_key,
/// 而是从 `AppState.models_config` 和 keychain 自动解析。
/// 供 `discover_all_models` / `auto_populate_models` 内部复用。
async fn discover_models_for_provider(
    state: &AppState,
    provider_id: &str,
) -> Result<Vec<ModelInfo>, String> {
    let (kind, base_url, has_key_slot) = {
        let cfg = state.llm.models_config.read();
        let p = cfg
            .find_provider(provider_id)
            .ok_or_else(|| format!("provider `{provider_id}` 不存在"))?;
        // Ollama 的 base_url 可能为 None,回退到 config.ollama_url。
        let base_url = p
            .base_url
            .clone()
            .or_else(|| {
                if p.kind == ProviderKind::Ollama {
                    Some(state.infra.config.ollama_url.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| format!("provider `{provider_id}` 未配置 base_url"))?;
        (p.kind, base_url, p.api_key_keychain_slot.is_some())
    };

    // Anthropic: 直接返回硬编码列表(无需网络)。
    if kind == ProviderKind::Anthropic {
        return Ok(anthropic_default_models());
    }

    // 从 keychain 获取 API key(若有 slot)。
    let api_key = if has_key_slot {
        crate::security::keychain::get_provider_key(provider_id)
            .map_err(|e| format!("读取 keychain 失败: {e}"))?
    } else {
        None
    };

    fetch_models_from_provider(state, provider_id, &base_url, api_key.as_deref()).await
}

/// 将名称转为 provider id(slug)。
///
/// 规则:小写,非字母数字字符替换为连字符,去除首尾连字符。
fn slugify(name: &str) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    slug.trim_matches('-').to_string()
}

/// 解析 kind 字符串为 ProviderKind。
fn parse_kind(kind: &str) -> Result<ProviderKind, String> {
    match kind.trim().to_lowercase().as_str() {
        "ollama" => Ok(ProviderKind::Ollama),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "openai-compat" | "openai_compat" | "openai" => Ok(ProviderKind::OpenAiCompat),
        "custom" => Ok(ProviderKind::Custom),
        other => Err(format!(
            "未知的 provider kind `{other}`(应为 ollama/anthropic/openai-compat/custom)"
        )),
    }
}

/// 添加自定义 provider,返回 provider_id。
///
/// provider_id 由 name slugify 生成;若与已有 id 冲突,追加数字后缀。
/// 新 provider 默认无模型(前端可通过 discover_models 拉取后补写)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "add_custom_provider"))]
pub async fn add_custom_provider(
    state: State<'_, AppState>,
    name: String,
    base_url: String,
    kind: String,
) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("provider 名称不能为空".to_string());
    }
    let provider_kind = parse_kind(&kind)?;
    let base = slugify(&name);
    if base.is_empty() {
        return Err("provider 名称无法转为有效 id".to_string());
    }

    // SSRF 校验 base_url(远端 provider 才需要;Ollama 允许内网)。
    if !base_url.trim().is_empty() {
        let guard = build_ssrf_guard(provider_kind);
        guard.validate_url(&base_url).map_err(|e| {
            format!("SSRF 校验失败: {e}")
        })?;
    }

    let mut guard = state.llm.models_config.write();
    // 生成唯一 id(冲突时追加数字后缀)。
    let mut provider_id = base.clone();
    let mut suffix = 1;
    while guard.providers.iter().any(|p| p.id == provider_id) {
        provider_id = format!("{base}-{suffix}");
        suffix += 1;
    }

    let provider = ProviderConfig {
        id: provider_id.clone(),
        kind: provider_kind,
        display_name: name.trim().to_string(),
        base_url: if base_url.trim().is_empty() {
            None
        } else {
            Some(base_url.trim().to_string())
        },
        api_key_keychain_slot: Some(format!("provider:{provider_id}")),
        api_key_env: None,
        supports_tools: true,
        supports_streaming: true,
        is_builtin: false,
        models: Vec::new(),
    };
    guard.providers.push(provider);
    guard
        .validate()
        .map_err(|e| format!("配置校验失败: {e}"))?;
    let snapshot = guard.clone();
    drop(guard);

    let path = state.infra.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| format!("保存 models.json 失败: {e}"))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot);

    // P1-3: 热重建 gateway 客户端(新 provider 通常无 API key,客户端为 None;
    // 若 keychain 中已有同名 slot 的 key 则直接生效)。
    let api_key = crate::security::keychain::get_provider_key(&provider_id)
        .ok()
        .flatten();
    state.llm.llm.rebuild_provider_client(
        &provider_id,
        provider_kind,
        if base_url.trim().is_empty() {
            None
        } else {
            Some(base_url.trim())
        },
        api_key.as_deref(),
    );

    Ok(provider_id)
}

/// 删除 provider。
///
/// 内置(is_builtin=true)或当前默认 provider 不可删除。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "remove_provider"))]
pub async fn remove_provider(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<(), String> {
    let mut guard = state.llm.models_config.write();
    let target = guard
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider `{provider_id}` 不存在"))?;
    if target.is_builtin {
        return Err(format!("内置 provider `{provider_id}` 不可删除"));
    }
    if guard.default_provider == provider_id {
        return Err(format!(
            "provider `{provider_id}` 是默认 provider,请先切换默认后再删除"
        ));
    }
    // 同时清理该 provider 的 keychain 条目(幂等)。
    let _ = crate::security::keychain::delete_provider_key(&provider_id);
    // P1-3: 记录被删除 provider 的 kind,用于清除 gateway 客户端。
    let removed_kind = target.kind;
    guard.providers.retain(|p| p.id != provider_id);
    guard
        .validate()
        .map_err(|e| format!("配置校验失败: {e}"))?;
    let snapshot = guard.clone();
    drop(guard);

    let path = state.infra.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| format!("保存 models.json 失败: {e}"))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot);

    // P1-3: 清除 gateway 中对应的客户端(传 None 表示清除)。
    state.llm.llm.rebuild_provider_client(
        &provider_id,
        removed_kind,
        None,
        None,
    );

    Ok(())
}

/// 设置默认 provider 和 model。
///
/// 校验 provider 存在且 model 在该 provider 的 models 列表中。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "set_default_provider"))]
pub async fn set_default_provider(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
) -> Result<(), String> {
    let mut guard = state.llm.models_config.write();
    let provider = guard
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider `{provider_id}` 不存在"))?;
    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(format!(
            "model `{model_id}` 不在 provider `{provider_id}` 的模型列表中"
        ));
    }
    guard.default_provider = provider_id;
    guard.default_model = model_id;
    guard
        .validate()
        .map_err(|e| format!("配置校验失败: {e}"))?;
    let snapshot = guard.clone();
    drop(guard);

    let path = state.infra.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| format!("保存 models.json 失败: {e}"))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot);
    Ok(())
}

/// 按 WorkType 设置路由。
///
/// 在 `work_type_overrides` 中为指定 work_type 设置 provider。
/// model 字段取该 provider 的第一个模型(若无模型则用 default_model)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "set_worktype_routing"))]
pub async fn set_worktype_routing(
    state: State<'_, AppState>,
    work_type: String,
    provider_id: String,
) -> Result<(), String> {
    if work_type.trim().is_empty() {
        return Err("work_type 不能为空".to_string());
    }
    let mut guard = state.llm.models_config.write();
    let provider = guard
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider `{provider_id}` 不存在"))?;
    // 选 model:优先 provider 的第一个模型,否则回退 default_model。
    let model = provider
        .models
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| guard.default_model.clone());

    let entry = WorkTypeOverrideEntry {
        provider: provider_id,
        model,
        temperature: None,
        max_tokens: None,
    };
    guard.work_type_overrides.insert(work_type, entry);
    let snapshot = guard.clone();
    drop(guard);

    let path = state.infra.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| format!("保存 models.json 失败: {e}"))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot);
    Ok(())
}

/// P1-3: 热更新 provider 字段(name / base_url / kind)。
///
/// 修改 `ModelsConfig` 中指定 provider 的字段并落盘,
/// 然后调用 `LlmGateway::rebuild_provider_client` 重建对应客户端,
/// 使新的 base_url / kind 立即生效,无需重启。
///
/// 所有参数(除 provider_id 外)为 `Option`,仅更新提供的字段。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "update_provider"))]
pub async fn update_provider(
    state: State<'_, AppState>,
    provider_id: String,
    name: Option<String>,
    base_url: Option<String>,
    kind: Option<String>,
) -> Result<(), String> {
    // 解析 kind 字符串(若提供)。
    let parsed_kind = match &kind {
        Some(k) => Some(parse_kind(k)?),
        None => None,
    };

    // SSRF 校验新 base_url(若提供且非空)。
    // 校验时使用新 kind(若有)或现有 kind。
    let effective_kind = parsed_kind.unwrap_or_else(|| {
        let cfg = state.llm.models_config.read();
        cfg.find_provider(&provider_id)
            .map(|p| p.kind)
            .unwrap_or(ProviderKind::OpenAiCompat)
    });
    if let Some(url) = &base_url {
        if !url.trim().is_empty() {
            let guard = build_ssrf_guard(effective_kind);
            guard.validate_url(url).map_err(|e| {
                format!("SSRF 校验失败: {e}")
            })?;
        }
    }

    let mut guard = state.llm.models_config.write();
    guard
        .update_provider(&provider_id, name, base_url, parsed_kind)
        .map_err(|e| format!("更新 provider 失败: {e}"))?;
    guard
        .validate()
        .map_err(|e| format!("配置校验失败: {e}"))?;

    // 获取更新后的 provider 快照(用于热重建 gateway 客户端)。
    let snapshot = guard.clone();
    let provider = snapshot
        .find_provider(&provider_id)
        .ok_or_else(|| format!("provider `{provider_id}` 不存在"))?
        .clone();
    drop(guard);

    let path = state.infra.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| format!("保存 models.json 失败: {e}"))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot);

    // P1-3: 从 keychain 读取 API key,热重建 gateway 客户端。
    let api_key = crate::security::keychain::get_provider_key(&provider_id)
        .ok()
        .flatten();
    state.llm.llm.rebuild_provider_client(
        &provider_id,
        provider.kind,
        provider.base_url.as_deref(),
        api_key.as_deref(),
    );

    Ok(())
}

/// P1-2: 批量发现所有已配置 provider 的模型列表(并行)。
///
/// 遍历 `models_config.providers`,对每个 provider 调用
/// `discover_models_for_provider`。失败的 provider 不会中断整体,
/// 而是在返回值中附带 `error` 字段。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "discover_all_models"))]
pub async fn discover_all_models(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderModels>, String> {
    // 先快照 provider 列表(释放读锁后再并行调用,避免长时间持锁)。
    let providers: Vec<(String, String)> = {
        let cfg = state.llm.models_config.read();
        cfg.providers
            .iter()
            .map(|p| (p.id.clone(), p.display_name.clone()))
            .collect()
    };

    // 克隆 AppState(内部全是 Arc,clone 开销低)供并行 future 持有。
    let app_state = state.inner().clone();
    let futures: Vec<_> = providers
        .into_iter()
        .map(|(id, name)| {
            let app_state = app_state.clone();
            async move {
                match discover_models_for_provider(&app_state, &id).await {
                    Ok(models) => ProviderModels {
                        provider_id: id,
                        provider_name: name,
                        models,
                        error: None,
                    },
                    Err(e) => ProviderModels {
                        provider_id: id,
                        provider_name: name,
                        models: Vec::new(),
                        error: Some(e),
                    },
                }
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;
    Ok(results)
}

/// P1-2: 自动发现并将新模型添加到 models.json 的对应 provider 下(去重)。
///
/// 流程:
/// 1. 调用 `discover_models_for_provider` 获取模型列表。
/// 2. 去重:已存在的 model id 跳过。
/// 3. 将新模型追加到 `provider.models`,保存到 models.json。
/// 4. 返回新增模型数量。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "auto_populate_models"))]
pub async fn auto_populate_models(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<usize, String> {
    // 1. 发现模型(内部持读锁,调用前释放)。
    let discovered = discover_models_for_provider(&state, &provider_id).await?;

    // 2. 去重 + 追加 + 落盘。
    let mut guard = state.llm.models_config.write();
    let provider = guard
        .providers
        .iter_mut()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider `{provider_id}` 不存在"))?;

    let existing: HashSet<String> = provider
        .models
        .iter()
        .map(|m| m.id.clone())
        .collect();
    let mut added = 0usize;
    for m in &discovered {
        if existing.contains(&m.id) {
            continue;
        }
        provider.models.push(ModelConfig {
            id: m.id.clone(),
            display_name: m.name.clone(),
            context_window: m.context_length.map(|c| c as u64),
            supports_reasoning: None,
            pricing: None,
        });
        added += 1;
    }

    if added == 0 {
        // 没有新模型也要释放写锁,避免 deadlock。
        drop(guard);
        return Ok(0);
    }

    guard
        .validate()
        .map_err(|e| format!("配置校验失败: {e}"))?;
    let snapshot = guard.clone();
    drop(guard);

    let path = state.infra.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| format!("保存 models.json 失败: {e}"))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot);
    Ok(added)
}

// ---------------------------------------------------------------------------
// P1-1: 模型健康面板命令
// ---------------------------------------------------------------------------

/// P1-1: 单个 provider 的健康指标快照(供前端健康面板展示)。
#[derive(Debug, Clone, Serialize)]
pub struct ModelHealthInfo {
    /// Provider id(如 "deepseek" / "ollama")。
    pub provider_id: String,
    /// Provider 显示名。
    pub provider_name: String,
    /// Provider kind 字符串("ollama" / "openai-compat" / "anthropic" / "custom")。
    pub provider_kind: String,
    /// 是否已配置 API key(Ollama 无需 key,始终为 true)。
    pub is_configured: bool,
    /// 最近一次请求延迟(毫秒)。None 表示尚无请求。
    pub latency_ms: Option<u64>,
    /// 当日(UTC)累计费用(USD)。
    pub cost_today_usd: f64,
    /// 当月(UTC)累计费用(USD)。
    pub cost_month_usd: f64,
    /// 语义缓存命中率 [0.0, 1.0](全局指标,非 per-provider)。
    pub cache_hit_rate: f32,
    /// 当日(UTC)请求次数。
    pub request_count_today: u64,
    /// 断路器状态("Closed" / "Open" / "HalfOpen")。
    pub circuit_breaker_status: String,
    /// 最近一次错误信息(成功后清空)。None 表示无错误。
    pub last_error: Option<String>,
    /// 最近一次请求的 Unix 时间戳(秒)。None 表示尚无请求。
    pub last_request_at: Option<u64>,
}

/// P1-1: 获取所有 provider 的健康指标。
///
/// 数据来源:
/// - `latency_ms` / `circuit_breaker_status` / `last_error` / `last_request_at`
///   从 `ModelHealthTracker` 读取(Gateway 在每次 chat 调用时记录)。
/// - `cost_today_usd` / `cost_month_usd` / `request_count_today` 从 `CostTracker`
///   按 provider 过滤计算。
/// - `cache_hit_rate` 从全局 `metrics` 读取(语义缓存命中率,全局指标)。
/// - `is_configured` 检查 keychain(与 `get_provider_key_status` 逻辑一致)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "get_model_health"))]
pub async fn get_model_health(state: State<'_, AppState>) -> Result<Vec<ModelHealthInfo>, String> {
    let cfg = state.llm.models_config.read().clone();
    let tracker = &state.llm.model_health_tracker;
    let cost_tracker = &state.llm.cost_tracker;
    let cb_status = state.llm.llm.circuit_breaker_status().to_string();
    let cache_hit_rate = crate::metrics::global().snapshot().semantic_cache_hit_ratio();

    // 从 CostTracker 获取所有记录,按 provider 聚合当日/当月费用与请求次数。
    let now = chrono::Utc::now();
    let today = now.date_naive();
    let (cur_year, cur_month) = (now.year(), now.month());
    let all_records = cost_tracker.all();

    let mut result = Vec::with_capacity(cfg.providers.len());
    for p in &cfg.providers {
        // 检查 keychain 是否已配置 key(Ollama 无需 key,视为已配置)。
        let is_configured = if p.kind == crate::llm::models_config::ProviderKind::Ollama {
            true
        } else {
            matches!(crate::security::keychain::get_provider_key(&p.id), Ok(Some(_)))
        };

        // 从 ModelHealthTracker 读取指标(若该 provider 有记录)。
        let metrics = tracker.get_metrics(&p.id);
        let latency_ms = metrics.as_ref().and_then(|m| m.latency_ms);
        let last_error = metrics.as_ref().and_then(|m| m.last_error.clone());
        let last_request_at = metrics.as_ref().and_then(|m| m.last_request_at);

        // 断路器状态:ModelHealthTracker 中记录的是 gateway 配置的 provider 的状态。
        // 若该 provider 正是 gateway 的主 provider,使用 tracker 中的状态;
        // 否则默认 "Closed"(非主 provider 不受断路器保护)。
        let circuit_breaker = metrics
            .as_ref()
            .map(|m| m.circuit_breaker_status.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Closed".to_string());

        // 按 provider id 过滤 CostTracker 记录,计算当日/当月费用与请求次数。
        let mut cost_today = 0.0f64;
        let mut cost_month = 0.0f64;
        let mut req_count_today = 0u64;
        for r in &all_records {
            // provider 字段匹配(可能为 None,跳过)。
            let rec_provider = r.provider.as_deref().unwrap_or("unknown");
            if rec_provider != p.id {
                continue;
            }
            if r.timestamp.date_naive() == today {
                cost_today += r.cost_usd;
                req_count_today += 1;
            }
            let (ry, rm) = r.year_month();
            if ry == cur_year && rm == cur_month {
                cost_month += r.cost_usd;
            }
        }

        result.push(ModelHealthInfo {
            provider_id: p.id.clone(),
            provider_name: p.display_name.clone(),
            provider_kind: format!("{:?}", p.kind),
            is_configured,
            latency_ms,
            cost_today_usd: cost_today,
            cost_month_usd: cost_month,
            cache_hit_rate,
            request_count_today: req_count_today,
            circuit_breaker_status: circuit_breaker,
            last_error,
            last_request_at,
        });
    }

    // 若没有任何 provider,也返回 gateway 当前 provider 的断路器状态。
    let _ = cb_status; // 避免未使用警告
    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_chinese_and_spaces() {
        assert_eq!(slugify("My Provider"), "my-provider");
        assert_eq!(slugify("  OpenAI  "), "openai");
        // Rust 的 char::is_alphanumeric() 对中文字符返回 true(Unicode 字母),故保留原样。
        assert_eq!(slugify("本地模型"), "本地模型");
        assert_eq!(slugify("ollama"), "ollama");
    }

    #[test]
    fn parse_kind_accepts_variants() {
        assert_eq!(parse_kind("ollama").unwrap(), ProviderKind::Ollama);
        assert_eq!(parse_kind("anthropic").unwrap(), ProviderKind::Anthropic);
        assert_eq!(parse_kind("openai-compat").unwrap(), ProviderKind::OpenAiCompat);
        assert_eq!(parse_kind("openai_compat").unwrap(), ProviderKind::OpenAiCompat);
        assert_eq!(parse_kind("custom").unwrap(), ProviderKind::Custom);
        assert!(parse_kind("unknown").is_err());
    }

    #[test]
    fn join_models_url_avoids_double_v1() {
        assert_eq!(
            join_models_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            join_models_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            join_models_url("https://example.com"),
            "https://example.com/v1/models"
        );
    }

    #[test]
    fn join_ollama_tags_url_appends_tags() {
        assert_eq!(
            join_ollama_tags_url("http://127.0.0.1:11434"),
            "http://127.0.0.1:11434/api/tags"
        );
        assert_eq!(
            join_ollama_tags_url("http://127.0.0.1:11434/"),
            "http://127.0.0.1:11434/api/tags"
        );
    }

    #[test]
    fn connection_test_result_serializes() {
        let r = ConnectionTestResult {
            success: true,
            latency_ms: 42,
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"latency_ms\":42"));
    }

    #[test]
    fn model_info_serializes() {
        let m = ModelInfo {
            id: "gpt-4o".to_string(),
            name: "gpt-4o".to_string(),
            context_length: Some(128_000),
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"id\":\"gpt-4o\""));
        assert!(json.contains("\"context_length\":128000"));
    }
}
