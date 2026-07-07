//! T-E-S-41: models.json 动态配置 Tauri 命令。
//!
//! 5 个命令:
//! * `models_config_load` — 读取当前 ModelsConfig(优先用 AppState 里的
//!   RwLock 内存副本,而非每次重新读盘)。
//! * `models_config_save` — 校验 + 落盘 + 热更新 AppState.models_config
//!   + 推送 override 到 cost_tracker,让 `model_price()` 立即看到新 pricing。
//! * `models_config_set_default` — 仅热更新 default_provider/default_model
//!   (RwLock 写),不落盘(下次启动会从 models.json 读回旧默认值;若要
//!   持久化默认值,前端应先 set_default 再 save)。
//! * `models_config_add_provider` — 添加一个 provider(校验 id 唯一),
//!   热更新 + 落盘。
//! * `models_config_remove_provider` — 删除非内置、非默认的 provider,
//!   热更新 + 落盘。
//!
//! 所有命令都返回最新的 ModelsConfig 快照,方便前端单次调用即可刷新 UI。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::llm::models_config::{ModelsConfig, ProviderConfig};
use crate::security::SsrfGuard;
use crate::AppState;

/// 读取当前 ModelsConfig。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "models_config_load"))]
pub async fn models_config_load(state: State<'_, AppState>) -> Result<ModelsConfig, CommandError> {
    Ok(state.models_config.read().clone())
}

/// 校验并保存整个 ModelsConfig。
///
/// 流程:
/// 1. 调用 `validate()` 校验 id 唯一、default_provider/default_model 存在。
/// 2. 写入 `models_config_path`(覆盖)。
/// 3. 把新配置写入 `AppState.models_config` RwLock(热更新)。
/// 4. 调用 `update_models_config_override()` 让 `model_price()` 立即看到
///    新 pricing(无需重启)。
#[tauri::command]
#[instrument(skip(state, config), fields(otel.kind = "models_config_save"))]
pub async fn models_config_save(
    state: State<'_, AppState>,
    config: ModelsConfig,
) -> Result<ModelsConfig, CommandError> {
    config
        .validate()
        .map_err(|e| CommandError::validation(format!("models_config validation: {e}")))?;
    let path = state.config.models_config_path.clone();
    config
        .save(&path)
        .map_err(|e| CommandError::internal("models_config_save", &e))?;
    // 热更新内存副本。
    {
        let mut guard = state.models_config.write();
        *guard = config.clone();
    }
    // 让 cost_tracker::model_price() 立即看到新 pricing。
    crate::llm::cost_tracker::update_models_config_override(config.clone());
    Ok(config)
}

/// 热更新 default_provider / default_model(不落盘)。
///
/// 前端若要持久化,应在调用本命令后再调用 `models_config_save` 把
/// 整个配置写盘。本命令仅修改内存中的 RwLock 副本,下次进程启动
/// 会从 models.json 读回旧默认值。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "models_config_set_default"))]
pub async fn models_config_set_default(
    state: State<'_, AppState>,
    default_provider: String,
    default_model: String,
) -> Result<ModelsConfig, CommandError> {
    let mut guard = state.models_config.write();
    guard.default_provider = default_provider.clone();
    guard.default_model = default_model.clone();
    // 校验新值合法(避免 RwLock 内留下非法状态)。
    guard
        .validate()
        .map_err(|e| CommandError::validation(format!("models_config_set_default: {e}")))?;
    let snapshot = guard.clone();
    // 同步给 cost_tracker(虽然 default 不影响 pricing,但保持一致)。
    crate::llm::cost_tracker::update_models_config_override(snapshot.clone());
    Ok(snapshot)
}

/// M7a #86 / P1-22: 从磁盘重新加载 models.json 到内存。
///
/// 适用场景:用户手动编辑 models.json 文件后,希望不重启应用即生效。
/// 流程:
/// 1. 从 `models_config_path` 读取文件(不存在回退 default_builtin)。
/// 2. 校验配置合法性(validate())。
/// 3. 写入 `AppState.models_config` RwLock(热更新)。
/// 4. 调用 `update_models_config_override()` 让 `model_price()` 立即看到新 pricing。
///
/// 注意:本命令不修改 UnifiedModelDispatcher 的 ModelPolicy(其 overrides
/// 在 Dispatcher 构造时快照)。若需更新 ModelPolicy,需重启应用或
/// 重新构造 Dispatcher(未来扩展)。当前仅更新 pricing 和内存副本。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "models_config_reload"))]
pub async fn models_config_reload(
    state: State<'_, AppState>,
) -> Result<ModelsConfig, CommandError> {
    let path = state.config.models_config_path.clone();
    let config = crate::llm::models_config::ModelsConfig::load(&path);
    config
        .validate()
        .map_err(|e| CommandError::validation(format!("models_config_reload: {e}")))?;
    // 热更新内存副本。
    {
        let mut guard = state.models_config.write();
        *guard = config.clone();
    }
    // 让 cost_tracker::model_price() 立即看到新 pricing。
    crate::llm::cost_tracker::update_models_config_override(config.clone());
    tracing::info!(
        target: "nebula.models_config",
        path = %path.display(),
        "models.json reloaded from disk"
    );
    Ok(config)
}

/// 添加一个 provider(校验 id 唯一)。返回最新快照。
#[tauri::command]
#[instrument(skip(state, provider), fields(otel.kind = "models_config_add_provider"))]
pub async fn models_config_add_provider(
    state: State<'_, AppState>,
    provider: ProviderConfig,
) -> Result<ModelsConfig, CommandError> {
    let mut guard = state.models_config.write();
    if guard.providers.iter().any(|p| p.id == provider.id) {
        return Err(CommandError::validation(format!(
            "provider id `{}` already exists",
            provider.id
        )));
    }
    guard.providers.push(provider);
    guard
        .validate()
        .map_err(|e| CommandError::validation(format!("models_config_add_provider: {e}")))?;
    let snapshot = guard.clone();
    drop(guard);
    // 落盘。
    let path = state.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| CommandError::internal("models_config_add_provider", &e))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot.clone());
    Ok(snapshot)
}

/// 删除一个 provider。内置(is_builtin=true)或当前 default 不可删。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "models_config_remove_provider"))]
pub async fn models_config_remove_provider(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<ModelsConfig, CommandError> {
    let mut guard = state.models_config.write();
    let target = guard
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| CommandError::not_found(format!("provider `{provider_id}`")))?;
    if target.is_builtin {
        return Err(CommandError::validation(format!(
            "cannot remove builtin provider `{provider_id}`"
        )));
    }
    if guard.default_provider == provider_id {
        return Err(CommandError::validation(format!(
            "cannot remove default provider `{provider_id}`; switch default first"
        )));
    }
    guard.providers.retain(|p| p.id != provider_id);
    guard
        .validate()
        .map_err(|e| CommandError::validation(format!("models_config_remove_provider: {e}")))?;
    let snapshot = guard.clone();
    drop(guard);
    let path = state.config.models_config_path.clone();
    snapshot
        .save(&path)
        .map_err(|e| CommandError::internal("models_config_remove_provider", &e))?;
    crate::llm::cost_tracker::update_models_config_override(snapshot.clone());
    Ok(snapshot)
}

/// T-E-S-41: 写入用户自定义 provider 的 API key 到 OS keychain。
/// slot 名为 `provider:<provider_id>`,与 `KEY_API_KEY` 分开。
/// 空 value 视为删除(与 `set_api_key` 行为对齐)。
#[tauri::command]
#[instrument(fields(otel.kind = "set_provider_key"))]
pub async fn set_provider_key(provider_id: String, value: String) -> Result<(), CommandError> {
    if value.trim().is_empty() {
        crate::security::keychain::delete_provider_key(&provider_id)
            .map_err(|e| CommandError::internal("set_provider_key", &e))?;
        return Ok(());
    }
    crate::security::keychain::set_provider_key(&provider_id, &value)
        .map_err(|e| CommandError::internal("set_provider_key", &e))
}

/// T-E-S-41: 读取用户自定义 provider 的 API key(掩码后)。
/// 与 `get_api_key` 一致:永远不返回完整 key,只返回掩码版本。
#[tauri::command]
#[instrument(fields(otel.kind = "get_provider_key"))]
pub async fn get_provider_key(
    provider_id: String,
) -> Result<Option<crate::commands::core::MaskedApiKey>, CommandError> {
    let raw = crate::security::keychain::get_provider_key(&provider_id)
        .map_err(|e| CommandError::internal("get_provider_key", &e))?;
    Ok(raw.map(|key| {
        let len = key.len();
        let prefix_len = key.len().min(3);
        let suffix_len = key.len().min(3);
        let prefix = key[..prefix_len].to_string();
        let suffix = if len > 6 {
            &key[len - suffix_len..]
        } else {
            ""
        };
        let masked = if len > 6 {
            format!("{}****{}", &key[..prefix_len], suffix)
        } else if len > 0 {
            format!("{}****", &key[..prefix_len])
        } else {
            String::new()
        };
        crate::commands::core::MaskedApiKey {
            masked,
            length: len,
            prefix,
        }
    }))
}

/// M6 #83: Provider 连通性测试结果。
#[derive(Debug, serde::Serialize)]
pub struct ProviderTestResult {
    /// 是否连通。
    pub ok: bool,
    /// HTTP 状态码(若收到响应)。
    pub status_code: Option<u16>,
    /// 延迟(毫秒)。
    pub latency_ms: u64,
    /// 错误信息(ok=false 时)。
    pub error: Option<String>,
}

/// M6 #83: 测试 provider 连通性。
///
/// 策略:
/// - Ollama:调用 `OllamaClient::ping()` (GET `{base_url}/api/tags`,2s 超时)
/// - 远端 provider(OpenAI-compat / Anthropic / Custom):
///   GET `{base_url}/v1/models`(OpenAI 标准 list models 端点),5s 超时。
///   不需要 API key(仅测试 TCP/HTTP 可达性,401/403 也算连通)。
/// - base_url 为 None 的 provider:返回 ok=false。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "models_config_test_provider"))]
pub async fn models_config_test_provider(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<ProviderTestResult, CommandError> {
    let config = state.models_config.read().clone();
    let provider = config.find_provider(&provider_id).ok_or_else(|| {
        CommandError::internal(
            "models_config_test_provider",
            &anyhow::anyhow!("provider not found: {}", provider_id),
        )
    })?;

    let start = std::time::Instant::now();

    // Ollama:用内置 OllamaClient ping。
    if provider.kind == crate::llm::models_config::ProviderKind::Ollama {
        let ollama_client = state.llm.ollama_client();
        match tokio::time::timeout(std::time::Duration::from_secs(2), ollama_client.ping()).await {
            Ok(true) => {
                return Ok(ProviderTestResult {
                    ok: true,
                    status_code: Some(200),
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: None,
                });
            }
            Ok(false) => {
                return Ok(ProviderTestResult {
                    ok: false,
                    status_code: None,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: Some("ollama ping failed (server not responding)".to_string()),
                });
            }
            Err(_) => {
                return Ok(ProviderTestResult {
                    ok: false,
                    status_code: None,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: Some("ollama ping timeout (2s)".to_string()),
                });
            }
        }
    }

    // 远端 provider:HTTP GET {base_url}/v1/models。
    let base_url = match &provider.base_url {
        Some(url) if !url.is_empty() => url.clone(),
        _ => {
            return Ok(ProviderTestResult {
                ok: false,
                status_code: None,
                latency_ms: 0,
                error: Some("provider has no base_url".to_string()),
            });
        }
    };

    // M7b #94: SSRF 校验 — LLM provider 可能是本地 vLLM/LMStudio,需 allow_loopback。
    SsrfGuard::new()
        .with_allow_loopback(true)
        .validate_url(&base_url)
        .map_err(|e| {
            CommandError::internal(
                "models_config_test_provider",
                &anyhow::anyhow!("SSRF validation failed for provider URL: {e}"),
            )
        })?;

    let test_url = if base_url.ends_with('/') {
        format!("{}v1/models", base_url)
    } else {
        format!("{}/v1/models", base_url)
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| {
            CommandError::internal(
                "models_config_test_provider",
                &anyhow::anyhow!("http client build failed: {e}"),
            )
        })?;

    match client.get(&test_url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // 任何 HTTP 响应都算连通(401/403 也说明 server 可达)。
            Ok(ProviderTestResult {
                ok: true,
                status_code: Some(status),
                latency_ms: start.elapsed().as_millis() as u64,
                error: None,
            })
        }
        Err(e) => Ok(ProviderTestResult {
            ok: false,
            status_code: None,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("connection failed: {e}")),
        }),
    }
}
