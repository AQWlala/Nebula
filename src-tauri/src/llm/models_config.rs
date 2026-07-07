//! T-E-S-41: models.json 动态配置。
//!
//! 用户自行添加 LLM 提供商,通过 `models.json` 配置文件管理
//! provider/model 列表。文件位于应用数据目录
//!(`resolve_app_data_dir()` + "models.json"),与 `settings.json` 分开。
//!
//! ## 优先级
//! 环境变量 > models.json > 内置默认(向后兼容)。
//!
//! ## 热更新
//! provider 列表修改重启生效(reqwest::Client 不易热替换);
//! `default_provider` / `default_model` 通过 `AppState.models_config`
//! 的 `Arc<parking_lot::RwLock<ModelsConfig>>` 支持热更新。
//!
//! ## 内置默认
//! `default_builtin()` 返回 deepseek / anthropic / ollama 三家。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Provider kind 闭集,对应 LlmGateway 的 4 条 dispatch 路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// OpenAI 兼容端点(DeepSeek / 自建 OpenAI 兼容服务)。
    ///
    /// 注意:kebab-case 默认产生 `"open-ai-compat"`(按词边界拆分),
    /// 但整个代码库(前端、i18n、文档、keychain slot)一致使用 `"openai-compat"`(单连字符)。
    /// 此处显式 rename 以保持兼容。
    #[serde(rename = "openai-compat")]
    OpenAiCompat,
    /// Anthropic Claude(Messages API)。
    Anthropic,
    /// 本地 Ollama(`/api/chat`)。
    Ollama,
    /// 自定义(用户扩展,默认走 OpenAI 兼容 dispatch)。
    Custom,
}

/// 模型定价(USD / 1M tokens)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pricing {
    pub input_usd_per_1m: f64,
    pub output_usd_per_1m: f64,
}

/// 单个模型条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    pub display_name: String,
    /// 上下文窗口(token 数),未知为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// 是否支持 reasoning 输出,未知为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning: Option<bool>,
    /// 单价(USD / 1M tokens),未知为 None(回退硬编码表)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
}

/// 单个 provider 条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub display_name: String,
    /// API base URL(Ollama 也可为空,回退 config.ollama_url)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Keychain slot 名(命名 `provider:<id>`),与 KEY_API_KEY 分开。
    /// 为 None 表示该 provider 无需 API key(如本地 Ollama)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_keychain_slot: Option<String>,
    /// 回退读取的环境变量名(如 `DEEPSEEK_API_KEY`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub is_builtin: bool,
    pub models: Vec<ModelConfig>,
}

fn default_true() -> bool {
    true
}

/// ADR-003 §5.1: `work_type_overrides` 中的单个条目。
///
/// 用户在 `models.json` 中为每种 WorkType 自定义 provider/model 覆盖。
/// 此结构始终编译（不受 `unified-dispatcher` feature 限制），因为
/// `models.json` 的解析/迁移逻辑需要在 feature off 时也能工作。
///
/// 当 `unified-dispatcher` feature 开启时，`dispatcher.rs` 的 `ModelPolicy`
/// 会将 JSON 的 `HashMap<String, WorkTypeOverrideEntry>`（字符串键）
/// 转换为 `HashMap<WorkType, WorkTypeOverrideEntry>`（枚举键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkTypeOverrideEntry {
    /// 指定 provider ID（如 "ollama", "deepseek", "anthropic"）。
    pub provider: String,
    /// 指定模型 ID（如 "qwen2.5:3b", "deepseek-chat", "claude-sonnet-4"）。
    pub model: String,
    /// 温度（可选，覆盖默认）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// 最大 token（可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

fn default_local_provider() -> String {
    "ollama".to_string()
}
fn default_classifier_model() -> String {
    "qwen2.5:3b".to_string()
}
fn default_evolution_model() -> String {
    "qwen2.5:7b".to_string()
}
fn default_soul_model() -> String {
    "qwen2.5:3b".to_string()
}
fn default_worker_model() -> String {
    "qwen2.5:7b".to_string()
}

/// models.json 顶层结构。
///
/// v1: `version` + `default_provider` + `default_model` + `providers`
/// v2 (ADR-003): 新增 `local_provider` / `local_classifier_model` /
/// `local_evolution_model` / `local_soul_model` / `worker_local_model` /
/// `work_type_overrides`，全部带 serde 默认值，向后兼容 v1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    pub version: u32,
    pub default_provider: String,
    pub default_model: String,
    pub providers: Vec<ProviderConfig>,

    // ADR-003 v2 新增字段（全部带默认值，v1 配置缺失时自动回退）。
    /// 本地 provider ID（通常 "ollama"）。
    #[serde(default = "default_local_provider")]
    pub local_provider: String,
    /// 本地分类器模型（通常 "qwen2.5:3b"）。
    #[serde(default = "default_classifier_model")]
    pub local_classifier_model: String,
    /// 本地进化模型（通常 "qwen2.5:7b" 或 "qwen2.5:14b"）。
    #[serde(default = "default_evolution_model")]
    pub local_evolution_model: String,
    /// 本地 Soul 编译模型。
    #[serde(default = "default_soul_model")]
    pub local_soul_model: String,
    /// SwarmWorker 默认本地模型。
    #[serde(default = "default_worker_model")]
    pub worker_local_model: String,
    /// WorkType 级别的用户覆盖配置。
    /// key 是 WorkType::as_str() 的字符串形式（如 "chat", "swarm_worker"）。
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub work_type_overrides: HashMap<String, WorkTypeOverrideEntry>,
}

impl ModelsConfig {
    /// 内置默认配置:deepseek / anthropic / ollama 三家。
    /// 用于首次启动(models.json 不存在)和向后兼容。
    ///
    /// ADR-003 v2: version 现在为 2，包含本地模型字段和 work_type_overrides。
    pub fn default_builtin() -> Self {
        Self {
            version: 2,
            default_provider: "deepseek".to_string(),
            default_model: "deepseek-chat".to_string(),
            providers: vec![
                ProviderConfig {
                    id: "deepseek".to_string(),
                    kind: ProviderKind::OpenAiCompat,
                    display_name: "DeepSeek".to_string(),
                    base_url: Some("https://api.deepseek.com/v1".to_string()),
                    api_key_keychain_slot: Some("provider:deepseek".to_string()),
                    api_key_env: Some("DEEPSEEK_API_KEY".to_string()),
                    supports_tools: true,
                    supports_streaming: true,
                    is_builtin: true,
                    models: vec![
                        ModelConfig {
                            id: "deepseek-chat".to_string(),
                            display_name: "DeepSeek Chat".to_string(),
                            context_window: Some(64_000),
                            supports_reasoning: Some(false),
                            pricing: Some(Pricing {
                                input_usd_per_1m: 0.14,
                                output_usd_per_1m: 0.28,
                            }),
                        },
                        ModelConfig {
                            id: "deepseek-reasoner".to_string(),
                            display_name: "DeepSeek Reasoner".to_string(),
                            context_window: Some(64_000),
                            supports_reasoning: Some(true),
                            pricing: Some(Pricing {
                                input_usd_per_1m: 0.55,
                                output_usd_per_1m: 2.19,
                            }),
                        },
                    ],
                },
                ProviderConfig {
                    id: "anthropic".to_string(),
                    kind: ProviderKind::Anthropic,
                    display_name: "Anthropic Claude".to_string(),
                    base_url: Some("https://api.anthropic.com".to_string()),
                    api_key_keychain_slot: Some("provider:anthropic".to_string()),
                    api_key_env: Some("NEBULA_ANTHROPIC_KEY".to_string()),
                    supports_tools: true,
                    supports_streaming: true,
                    is_builtin: true,
                    models: vec![
                        ModelConfig {
                            id: "claude-3-5-sonnet".to_string(),
                            display_name: "Claude 3.5 Sonnet".to_string(),
                            context_window: Some(200_000),
                            supports_reasoning: Some(false),
                            pricing: Some(Pricing {
                                input_usd_per_1m: 3.00,
                                output_usd_per_1m: 15.00,
                            }),
                        },
                        ModelConfig {
                            id: "claude-3-5-haiku".to_string(),
                            display_name: "Claude 3.5 Haiku".to_string(),
                            context_window: Some(200_000),
                            supports_reasoning: Some(false),
                            pricing: Some(Pricing {
                                input_usd_per_1m: 0.80,
                                output_usd_per_1m: 4.00,
                            }),
                        },
                    ],
                },
                ProviderConfig {
                    id: "ollama".to_string(),
                    kind: ProviderKind::Ollama,
                    display_name: "Ollama (本地)".to_string(),
                    base_url: None,
                    api_key_keychain_slot: None,
                    api_key_env: None,
                    supports_tools: false,
                    supports_streaming: true,
                    is_builtin: true,
                    models: vec![
                        ModelConfig {
                            id: "qwen2.5:3b".to_string(),
                            display_name: "Qwen2.5 3B".to_string(),
                            context_window: Some(32_000),
                            supports_reasoning: Some(false),
                            pricing: Some(Pricing {
                                input_usd_per_1m: 0.0,
                                output_usd_per_1m: 0.0,
                            }),
                        },
                        ModelConfig {
                            id: "llama3.1:8b".to_string(),
                            display_name: "Llama 3.1 8B".to_string(),
                            context_window: Some(128_000),
                            supports_reasoning: Some(false),
                            pricing: Some(Pricing {
                                input_usd_per_1m: 0.0,
                                output_usd_per_1m: 0.0,
                            }),
                        },
                    ],
                },
            ],
            // ADR-003 v2 新增字段默认值。
            local_provider: default_local_provider(),
            local_classifier_model: default_classifier_model(),
            local_evolution_model: default_evolution_model(),
            local_soul_model: default_soul_model(),
            worker_local_model: default_worker_model(),
            work_type_overrides: HashMap::new(),
        }
    }

    /// 解析 models.json 路径(`<app_data_dir>/models.json`)。
    /// 失败时回退到当前目录(`./models.json`)以保证测试可运行。
    pub fn resolve_path() -> PathBuf {
        match crate::backup::commands::resolve_app_data_dir() {
            Ok(dir) => dir.join("models.json"),
            Err(e) => {
                warn!(
                    target: "nebula.llm.models_config",
                    error = %e,
                    "resolve_app_data_dir failed; falling back to ./models.json"
                );
                PathBuf::from("models.json")
            }
        }
    }

    /// 从指定路径加载;文件不存在时回退 `default_builtin()`。
    /// 解析失败也回退 `default_builtin()` 并 warn(避免单点故障阻断启动)。
    ///
    /// ADR-003 #16: v1→v2 自动迁移。
    /// - 加载后若 `version < 2`，补写 v2 默认字段并备份原文件为 `.v1.bak`。
    /// - 宽松解析：v1 配置缺少 v2 字段时，serde `#[serde(default = ...)]`
    ///   自动回退默认值，不会崩溃。
    /// - 缺字段仅 warn 不阻断启动。
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<ModelsConfig>(&bytes) {
                Ok(cfg) => {
                    debug!(
                        target: "nebula.llm.models_config",
                        path = %path.display(),
                        providers = cfg.providers.len(),
                        version = cfg.version,
                        "models.json loaded"
                    );
                    // #16: v1→v2 迁移
                    if cfg.version < 2 {
                        Self::migrate_v1_to_v2(cfg, path)
                    } else {
                        cfg
                    }
                }
                Err(e) => {
                    warn!(
                        target: "nebula.llm.models_config",
                        path = %path.display(),
                        error = %e,
                        "models.json parse failed; falling back to default_builtin"
                    );
                    Self::default_builtin()
                }
            },
            Err(_) => {
                debug!(
                    target: "nebula.llm.models_config",
                    path = %path.display(),
                    "models.json not found; using default_builtin"
                );
                Self::default_builtin()
            }
        }
    }

    /// ADR-003 #16: v1→v2 迁移。
    ///
    /// v1 配置缺少 `local_provider` / `local_classifier_model` /
    /// `local_evolution_model` / `local_soul_model` / `worker_local_model` /
    /// `work_type_overrides` 字段。由于这些字段在 struct 上都有
    /// `#[serde(default = ...)]`，serde 解析时会自动回退到默认值。
    ///
    /// 迁移步骤：
    /// 1. 备份原文件为 `<path>.v1.bak`（失败仅 warn 不阻断）。
    /// 2. 补写 v2 字段（version = 2 + 默认值）。
    /// 3. 保存升级后的配置到原路径（失败仅 warn 不阻断）。
    ///
    /// 迁移失败不阻断启动：返回的 cfg 仍然可用（v1 字段已正确解析）。
    fn migrate_v1_to_v2(mut cfg: ModelsConfig, path: &Path) -> ModelsConfig {
        warn!(
            target: "nebula.llm.models_config",
            path = %path.display(),
            old_version = cfg.version,
            "models.json v1 detected; migrating to v2"
        );

        // 1. 备份原文件（best-effort，失败不阻断）
        let bak_path = PathBuf::from(format!("{}.v1.bak", path.display()));
        if let Err(e) = std::fs::copy(path, &bak_path) {
            warn!(
                target: "nebula.llm.models_config",
                path = %path.display(),
                bak_path = %bak_path.display(),
                error = %e,
                "failed to backup v1 models.json (non-fatal)"
            );
        }

        // 2. 补写 v2 字段（version + 默认值）
        cfg.version = 2;
        // 其余 v2 字段已由 serde default 填充，无需手动设置。
        // 但为确保一致性，显式设置（防止 serde default 与期望不一致）：
        if cfg.local_provider.is_empty() {
            cfg.local_provider = default_local_provider();
        }
        if cfg.local_classifier_model.is_empty() {
            cfg.local_classifier_model = default_classifier_model();
        }
        if cfg.local_evolution_model.is_empty() {
            cfg.local_evolution_model = default_evolution_model();
        }
        if cfg.local_soul_model.is_empty() {
            cfg.local_soul_model = default_soul_model();
        }
        if cfg.worker_local_model.is_empty() {
            cfg.worker_local_model = default_worker_model();
        }

        // 3. 保存升级后的配置（best-effort，失败不阻断）
        if let Err(e) = cfg.save(path) {
            warn!(
                target: "nebula.llm.models_config",
                path = %path.display(),
                error = %e,
                "failed to save migrated v2 models.json (non-fatal)"
            );
        }

        cfg
    }

    /// 写入到指定路径(创建父目录,pretty JSON)。
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir for {}", path.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(self).context("serializing models.json")?;
        std::fs::write(path, bytes)
            .with_context(|| format!("writing models.json to {}", path.display()))?;
        Ok(())
    }

    /// 校验:provider id 唯一、default_provider 存在。
    /// 不返回错误时表示通过。
    ///
    /// ADR-003 #17: 新增 provider base_url SSRF 校验。
    /// - 远端 provider（DeepSeek/Anthropic/OpenAI-compat/Custom）：
    ///   base_url 不得指向内网地址（loopback/private/link-local/CGNAT/broadcast）。
    /// - 本地 provider（Ollama）：允许 loopback/private 地址
    ///   （`http://127.0.0.1:11434` 是合法配置）。
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        // #17: SSRF 校验 — 复用 SsrfGuard。
        // Ollama provider 允许内网地址（本地服务），其他 provider 禁止。
        let ssrf_guard_remote = crate::security::SsrfGuard::new();
        let ssrf_guard_local = crate::security::SsrfGuard::new().with_allow_private(true);

        for p in &self.providers {
            if !seen.insert(p.id.as_str()) {
                anyhow::bail!("duplicate provider id: {}", p.id);
            }
            // model id 在同一 provider 内唯一。
            let mut seen_models = std::collections::HashSet::new();
            for m in &p.models {
                if !seen_models.insert(m.id.as_str()) {
                    anyhow::bail!("duplicate model id `{}` in provider `{}`", m.id, p.id);
                }
            }
            // #17: SSRF 校验 provider base_url。
            if let Some(ref url) = p.base_url {
                if !url.is_empty() {
                    let guard = if p.kind == ProviderKind::Ollama {
                        &ssrf_guard_local
                    } else {
                        &ssrf_guard_remote
                    };
                    match guard.validate_url(url) {
                        Ok(()) => {}
                        Err(e) => {
                            let err_str = format!("{e}");
                            // 仅对实际的 SSRF 违规（private/loopback/CGNAT 等）bail。
                            // DNS 解析失败（如 CI/离线环境）仅 warn，因为：
                            // 1. 实际的 SSRF 防护在 HTTP 请求层（SsrfGuard::build_safe_client）
                            // 2. DNS 在配置时和请求时可能解析到不同 IP（DNS rebinding）
                            if err_str.starts_with("SSRF:") {
                                warn!(
                                    target: "nebula.llm.models_config",
                                    provider = %p.id,
                                    base_url = %url,
                                    error = %e,
                                    "SSRF validation failed for provider base_url"
                                );
                                anyhow::bail!(
                                    "provider `{}` base_url SSRF validation failed: {}",
                                    p.id,
                                    e
                                );
                            } else {
                                warn!(
                                    target: "nebula.llm.models_config",
                                    provider = %p.id,
                                    base_url = %url,
                                    error = %e,
                                    "SSRF DNS resolution failed (non-fatal, will recheck at request time)"
                                );
                            }
                        }
                    }
                }
            }
        }
        if !self.providers.iter().any(|p| p.id == self.default_provider) {
            anyhow::bail!(
                "default_provider `{}` not found in providers list",
                self.default_provider
            );
        }
        // default_model 必须存在于 default_provider 的 models 列表里。
        let default_provider = self
            .providers
            .iter()
            .find(|p| p.id == self.default_provider)
            .expect("checked above");
        if !default_provider
            .models
            .iter()
            .any(|m| m.id == self.default_model)
        {
            anyhow::bail!(
                "default_model `{}` not found in provider `{}`",
                self.default_model,
                self.default_provider
            );
        }
        Ok(())
    }

    /// 按 provider id 查找。
    pub fn find_provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// 查找指定 provider 下的 model(精确匹配 id,否则前缀匹配)。
    pub fn find_model(&self, provider_id: &str, model_id: &str) -> Option<&ModelConfig> {
        let p = self.find_provider(provider_id)?;
        if let Some(m) = p.models.iter().find(|m| m.id == model_id) {
            return Some(m);
        }
        // 前缀匹配(如 `claude-3-5-sonnet-20241022` → `claude-3-5-sonnet`)。
        p.models.iter().find(|m| model_id.starts_with(&m.id))
    }
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self::default_builtin()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_builtin_has_three_providers() {
        let cfg = ModelsConfig::default_builtin();
        // ADR-003 v2: default_builtin 现在是 version 2
        assert_eq!(cfg.version, 2);
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-chat");
        assert_eq!(cfg.providers.len(), 3);
        // ADR-003 v2 新增字段
        assert_eq!(cfg.local_provider, "ollama");
        assert_eq!(cfg.local_classifier_model, "qwen2.5:3b");
        assert_eq!(cfg.local_evolution_model, "qwen2.5:7b");
        assert_eq!(cfg.local_soul_model, "qwen2.5:3b");
        assert_eq!(cfg.worker_local_model, "qwen2.5:7b");
        assert!(cfg.work_type_overrides.is_empty());
        let ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"deepseek"));
        assert!(ids.contains(&"anthropic"));
        assert!(ids.contains(&"ollama"));
        // 内置三家全部 is_builtin=true。
        assert!(cfg.providers.iter().all(|p| p.is_builtin));
    }

    #[test]
    fn default_builtin_validates() {
        let cfg = ModelsConfig::default_builtin();
        cfg.validate().expect("default_builtin should validate");
    }

    #[test]
    fn load_missing_file_falls_back_to_default() {
        let path = PathBuf::from("/tmp/nebula_definitely_missing_models.json");
        let cfg = ModelsConfig::load(&path);
        assert_eq!(cfg.providers.len(), 3);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "nebula_models_config_roundtrip_{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let mut cfg = ModelsConfig::default_builtin();
        // 添加一个自定义 provider 验证往返。
        cfg.providers.push(ProviderConfig {
            id: "custom-openai".to_string(),
            kind: ProviderKind::OpenAiCompat,
            display_name: "My Custom".to_string(),
            base_url: Some("https://my.endpoint/v1".to_string()),
            api_key_keychain_slot: Some("provider:custom-openai".to_string()),
            api_key_env: Some("MY_CUSTOM_API_KEY".to_string()),
            supports_tools: true,
            supports_streaming: true,
            is_builtin: false,
            models: vec![ModelConfig {
                id: "my-model".to_string(),
                display_name: "My Model".to_string(),
                context_window: Some(8_000),
                supports_reasoning: Some(false),
                pricing: Some(Pricing {
                    input_usd_per_1m: 1.0,
                    output_usd_per_1m: 2.0,
                }),
            }],
        });

        cfg.save(&path).expect("save");
        let loaded = ModelsConfig::load(&path);
        assert_eq!(loaded.providers.len(), 4);
        let custom = loaded
            .find_provider("custom-openai")
            .expect("custom provider");
        assert!(!custom.is_builtin);
        assert_eq!(custom.kind, ProviderKind::OpenAiCompat);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn validate_rejects_duplicate_provider_id() {
        let mut cfg = ModelsConfig::default_builtin();
        cfg.providers.push(cfg.providers[0].clone());
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err}").contains("duplicate provider id"));
    }

    #[test]
    fn validate_rejects_missing_default_provider() {
        let mut cfg = ModelsConfig::default_builtin();
        cfg.default_provider = "nonexistent".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err}").contains("default_provider"));
    }

    #[test]
    fn validate_rejects_missing_default_model() {
        let mut cfg = ModelsConfig::default_builtin();
        cfg.default_model = "nonexistent-model".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(format!("{err}").contains("default_model"));
    }

    #[test]
    fn find_model_uses_prefix_match() {
        let cfg = ModelsConfig::default_builtin();
        // 完整 id 匹配。
        let m = cfg
            .find_model("anthropic", "claude-3-5-sonnet")
            .expect("exact");
        assert_eq!(m.id, "claude-3-5-sonnet");
        // 前缀匹配(模拟带日期后缀的版本号)。
        let m = cfg
            .find_model("anthropic", "claude-3-5-sonnet-20241022")
            .expect("prefix");
        assert_eq!(m.id, "claude-3-5-sonnet");
    }

    #[test]
    fn provider_kind_serde_kebab_case() {
        let json = r#""openai-compat""#;
        let k: ProviderKind = serde_json::from_str(json).unwrap();
        assert_eq!(k, ProviderKind::OpenAiCompat);
        let s = serde_json::to_string(&ProviderKind::Anthropic).unwrap();
        assert_eq!(s, r#""anthropic""#);
        let s = serde_json::to_string(&ProviderKind::Ollama).unwrap();
        assert_eq!(s, r#""ollama""#);
        let s = serde_json::to_string(&ProviderKind::Custom).unwrap();
        assert_eq!(s, r#""custom""#);
    }

    #[test]
    fn save_creates_parent_dir() {
        let dir = std::env::temp_dir().join(format!(
            "nebula_models_config_subdir_{}",
            std::process::id()
        ));
        let path = dir.join("nested/models.json");
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = ModelsConfig::default_builtin();
        cfg.save(&path).expect("save should create parent dirs");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ADR-003 #16: v1→v2 迁移测试
    #[test]
    fn migrate_v1_to_v2_backs_up_and_upgrades() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "nebula_models_v1_migrate_{}.json",
            std::process::id()
        ));
        // 与 migrate_v1_to_v2 内部使用的路径计算保持一致：
        // `PathBuf::from(format!("{}.v1.bak", path.display()))`
        // 不使用 with_extension，因为其对多段扩展名的语义易混淆。
        let bak_path = PathBuf::from(format!("{}.v1.bak", path.display()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bak_path);

        // 构造一个 v1 配置（缺少 v2 字段）
        let v1_json = r#"{
            "version": 1,
            "default_provider": "deepseek",
            "default_model": "deepseek-chat",
            "providers": [
                {
                    "id": "deepseek",
                    "kind": "openai-compat",
                    "display_name": "DeepSeek",
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key_env": "DEEPSEEK_API_KEY",
                    "supports_tools": true,
                    "supports_streaming": true,
                    "is_builtin": true,
                    "models": [
                        {"id": "deepseek-chat", "display_name": "DeepSeek Chat"}
                    ]
                },
                {
                    "id": "ollama",
                    "kind": "ollama",
                    "display_name": "Ollama",
                    "supports_tools": false,
                    "supports_streaming": true,
                    "is_builtin": true,
                    "models": []
                }
            ]
        }"#;
        std::fs::write(&path, v1_json).expect("write v1 json");

        // 加载 → 触发迁移
        let cfg = ModelsConfig::load(&path);

        // v2 字段已填充
        assert_eq!(cfg.version, 2);
        assert_eq!(cfg.local_provider, "ollama");
        assert_eq!(cfg.local_classifier_model, "qwen2.5:3b");
        assert_eq!(cfg.local_evolution_model, "qwen2.5:7b");
        assert_eq!(cfg.local_soul_model, "qwen2.5:3b");
        assert_eq!(cfg.worker_local_model, "qwen2.5:7b");
        assert!(cfg.work_type_overrides.is_empty());

        // 备份文件已创建
        assert!(bak_path.exists(), "v1 backup file should exist");

        // 清理
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bak_path);
    }

    #[test]
    fn migrate_v2_config_not_re_migrated() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nebula_models_v2_noop_{}.json", std::process::id()));
        let bak_path = PathBuf::from(format!("{}.v1.bak", path.display()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bak_path);

        // 保存一个 v2 配置
        let cfg = ModelsConfig::default_builtin();
        cfg.save(&path).expect("save v2");

        // 加载 → 不应触发迁移
        let loaded = ModelsConfig::load(&path);
        assert_eq!(loaded.version, 2);

        // 不应创建备份文件
        assert!(!bak_path.exists(), "v2 config should not be re-migrated");

        let _ = std::fs::remove_file(&path);
    }

    // ADR-003 #17: SSRF 校验测试
    #[test]
    fn ssrf_rejects_loopback_for_remote_provider() {
        let mut cfg = ModelsConfig::default_builtin();
        // 把 deepseek 的 base_url 改为 loopback
        cfg.providers[0].base_url = Some("http://127.0.0.1:8080/v1".to_string());
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("SSRF") || msg.contains("loopback"),
            "should reject loopback for remote provider, got: {msg}"
        );
    }

    #[test]
    fn ssrf_rejects_private_for_remote_provider() {
        let mut cfg = ModelsConfig::default_builtin();
        // 把 deepseek 的 base_url 改为 private
        cfg.providers[0].base_url = Some("http://10.0.0.1/v1".to_string());
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("SSRF") || msg.contains("private"),
            "should reject private for remote provider, got: {msg}"
        );
    }

    #[test]
    fn ssrf_allows_loopback_for_ollama() {
        let mut cfg = ModelsConfig::default_builtin();
        // ollama 的 base_url 改为 loopback（应该允许）
        let ollama = cfg.providers.iter_mut().find(|p| p.id == "ollama").unwrap();
        ollama.base_url = Some("http://127.0.0.1:11434".to_string());
        // 不应因 SSRF 报错（可能因其他原因报错，但不应是 SSRF）
        match cfg.validate() {
            Ok(()) => {}
            Err(e) => {
                let msg = format!("{e}");
                assert!(
                    !msg.contains("SSRF"),
                    "ollama loopback should be allowed, got: {msg}"
                );
            }
        }
    }

    #[test]
    fn work_type_overrides_serde_roundtrip() {
        let mut cfg = ModelsConfig::default_builtin();
        cfg.work_type_overrides.insert(
            "evolution_extract".to_string(),
            WorkTypeOverrideEntry {
                provider: "ollama".to_string(),
                model: "qwen2.5:14b".to_string(),
                temperature: Some(0.5),
                max_tokens: None,
            },
        );
        let json = serde_json::to_string(&cfg).expect("serialize");
        let loaded: ModelsConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(loaded.work_type_overrides.len(), 1);
        let entry = loaded.work_type_overrides.get("evolution_extract").unwrap();
        assert_eq!(entry.provider, "ollama");
        assert_eq!(entry.model, "qwen2.5:14b");
        assert_eq!(entry.temperature, Some(0.5));
    }
}
