//! P0-1: 模型配置中心 Tauri 命令。
//!
//! 为前端 `ModelConfigPanel` 提供 provider API Key 状态查询、连通性测试、
//! 模型自动发现、自定义 provider 增删、默认 provider/model 设置、WorkType
//! 路由配置等命令。
//!
//! ## 命令清单
//! * `get_provider_key_status` — 查询 keychain 中是否存在该 provider 的 key(返回 bool)。
//! * `test_provider_connection` — 发送测试请求,返回延迟和状态。
//! * `discover_models` — 自动拉取可用模型列表。
//! * `add_custom_provider` — 添加自定义 provider,返回 provider_id。
//! * `remove_provider` — 删除 provider。
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

use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tracing::instrument;

use crate::llm::models_config::{ProviderConfig, ProviderKind, WorkTypeOverrideEntry};
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
/// - Ollama:GET `{base_url}/api/tags`,解析 `{"models": [{"name": "..."}]}`。
/// - 远端 provider:GET `{base_url}/v1/models`,解析 `{"data": [{"id": "..."}]}`。
///
/// SSRF 防护:同 `test_provider_connection`。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "discover_models"))]
pub async fn discover_models(
    state: State<'_, AppState>,
    provider_id: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<ModelInfo>, String> {
    if base_url.trim().is_empty() {
        return Err("base_url 为空".to_string());
    }

    let kind = resolve_provider_kind(&state, &provider_id);
    let guard = build_ssrf_guard(kind);
    guard.validate_url(&base_url).map_err(|e| {
        format!("SSRF 校验失败: {e}")
    })?;

    let (fetch_url, timeout) = match kind {
        ProviderKind::Ollama => (join_ollama_tags_url(&base_url), Duration::from_secs(5)),
        _ => (join_models_url(&base_url), Duration::from_secs(10)),
    };

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

    let mut req = client.get(&fetch_url);
    if let Some(key) = api_key.as_ref().filter(|k| !k.trim().is_empty()) {
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
            // Ollama /api/tags → {"models": [{"name": "llama3.1:8b", ...}]}
            body.get("models")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let id = item.get("name")?.as_str()?.to_string();
                            Some(ModelInfo {
                                name: id.clone(),
                                id,
                                context_length: None,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
        _ => {
            // OpenAI 兼容 /v1/models → {"data": [{"id": "gpt-4o", ...}]}
            body.get("data")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let id = item.get("id")?.as_str()?.to_string();
                            Some(ModelInfo {
                                name: id.clone(),
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
