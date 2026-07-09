//! Configuration for the Nebula application.
//!
//! [`AppConfig`] is loaded from environment variables (with sensible
//! defaults) by [`AppConfig::from_env`].  It is consumed by the
//! bootstrap phase in [`crate::bootstrap`] and stored inside
//! [`crate::AppState`].

use crate::memory::vector_store::VectorStoreBackend;

/// Configuration sourced from environment variables (with sensible defaults).
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Path to the SQLite database file.
    pub db_path: String,
    /// Path to the LanceDB vector store directory.
    pub lance_path: String,
    /// Base URL of the Ollama HTTP server. 仍用于 embedding (bge-small-zh)。
    /// 若要禁用本地 Ollama,设为空字符串。
    pub ollama_url: String,
    /// 默认 chat 模型名。v1.2 起默认 `deepseek-chat`。
    /// 可选值:deepseek-chat / deepseek-reasoner / qwen2.5:3b / claude-3-5-haiku 等。
    pub chat_model: String,
    /// 默认 embedding 模型名 (仍走 Ollama)。
    pub embed_model: String,
    /// 主 LLM provider (deepseek / ollama / openai-compat / anthropic)。
    /// v1.2 起默认 `deepseek`。决定 chat 请求优先走哪条路径。
    pub llm_provider: String,
    /// DeepSeek API base URL (主路径)。
    pub deepseek_api_url: String,
    /// DeepSeek API key (从 DEEPSEEK_API_KEY 读取)。
    pub deepseek_api_key: Option<String>,
    /// 可选的远程 fallback URL (OpenAI 兼容,如 Azure OpenAI / 自建端点)。
    pub remote_fallback_url: Option<String>,
    /// Number of days of inactivity before the black-hole engine may compress.
    pub blackhole_threshold_days: u32,
    /// Embedding vector dimensionality.
    pub embedding_dim: usize,
    /// v0.2: background reflection worker period in seconds. `0`
    /// disables the worker.
    pub reflect_interval_secs: u64,
    /// v0.2: reflection window size in days.
    pub reflect_window_days: i64,
    /// v0.2: minimum source-memory importance for reflection.
    pub reflect_min_importance: f32,
    /// v0.3: enable the in-process gRPC server. Default `true` (set
    /// `NEBULA_GRPC=0` to disable). The gRPC port is configured
    /// via `grpc_bind_addr`.
    pub grpc_enabled: bool,
    /// v0.3: bind address for the gRPC server. Default
    /// `127.0.0.1:50051`.
    pub grpc_bind_addr: String,
    /// T-S2-B-03a: enable the REST API server. Default `false`
    /// (set `NEBULA_REST=1` to enable). Requires `rest-api` feature.
    pub rest_enabled: bool,
    /// T-S2-B-03a: bind address for the REST API server. Default
    /// `127.0.0.1:8080`.
    pub rest_bind_addr: String,
    /// T-S2-B-03a: Bearer token / API key for REST API auth.
    /// If empty, auth is bypassed (development mode only).
    /// Set `NEBULA_REST_TOKEN` to require auth.
    pub rest_api_token: Option<String>,
    /// v0.5: workspace root for the editor.  All file operations
    /// are sandboxed to this directory.
    pub editor_workspace: String,
    /// v0.5: directory used by the local sync transport.  Defaults
    /// to `<config_dir>/sync_inbox`.
    pub sync_inbox: String,
    /// T-E-S-20: exec 类操作审批超时(秒)。超时后 fail-closed 自动拒绝。
    /// 默认 60,可设 `NEBULA_EXEC_APPROVAL_TIMEOUT_SECS` 覆盖。
    pub exec_approval_timeout_secs: u64,
    /// T-E-A-01: 是否启用 L0.5 语义缓存。默认 true。
    /// 设 `NEBULA_SEMANTIC_CACHE=0` 关闭。
    pub semantic_cache_enabled: bool,
    /// T-E-A-06: 是否启用 Token 费用追踪。默认 true。
    /// 设 `NEBULA_COST_TRACKER=0` 关闭。
    pub cost_tracker_enabled: bool,
    /// T-E-A-02: 是否启用 TokenJuice 三级压缩。默认 true。
    /// 设 `NEBULA_TOKEN_JUICE=0` 关闭。
    pub token_juice_enabled: bool,
    /// T-E-A-03: 是否启用 ModelRouter 智能路由。默认 true。
    /// 设 `NEBULA_ROUTER=0` 关闭。
    pub router_enabled: bool,
    /// T-E-A-03: ModelRouter 分类器模型名。默认 `qwen2.5:3b`。
    pub router_classifier_model: String,
    /// T-E-A-04: 是否启用 Prefix-Cache 适配层。默认 true。
    /// 设 `NEBULA_PREFIX_CACHE=0` 关闭。
    pub prefix_cache_enabled: bool,
    /// T-E-A-05: 日预算(USD),0=不限制。env: NEBULA_DAILY_BUDGET_USD
    pub daily_budget_usd: f64,
    /// T-E-A-12: 自动化(触发器/Cron/后台)每日预算(USD),None=不限制。
    /// 超阈值时 emit `budget_exceeded` 事件。env: NEBULA_AUTOMATION_DAILY_BUDGET_USD
    pub automation_daily_budget_usd: Option<f64>,
    /// T-E-L-06: Loop 月度美元预算上限。
    ///
    /// 环境变量:NEBULA_LOOP_MONTHLY_BUDGET_USD
    /// None 或 0.0 = 不限制。
    /// 达到 80% → emit loop_budget_warning;达到 100% → pause_all + emit loop_budget_exceeded。
    pub loop_monthly_budget_usd: Option<f64>,
    /// T-E-L-06: Loop 月度 Token 预算上限。
    ///
    /// 环境变量:NEBULA_LOOP_MONTHLY_BUDGET_TOKENS
    /// None 或 0 = 不限制。
    pub loop_monthly_budget_tokens: Option<u64>,
    /// T-E-B-09: 启动时监控的文件夹列表(逗号分隔)。
    /// env: NEBULA_WATCH_PATHS
    pub watch_paths: Vec<String>,
    /// T-E-C-14: 是否启用剪贴板智能监听。默认 false(需用户显式开启)。
    /// env: NEBULA_CLIPBOARD_WATCH=1
    pub clipboard_watch_enabled: bool,
    /// T-E-S-40: OpenAI 兼容 provider base URL(vLLM/LMStudio/OpenRouter/自建)。
    /// env: NEBULA_OPENAI_COMPAT_URL。配置后 llm_provider 应设为 "openai-compat"。
    pub openai_compat_base_url: Option<String>,
    /// T-E-S-40: OpenAI 兼容 provider API key(可空,适配 LMStudio 本地)。
    /// env: NEBULA_OPENAI_COMPAT_KEY。
    pub openai_compat_api_key: Option<String>,
    /// T-E-S-40: OpenAI 兼容 provider 默认模型名(如 "openai/gpt-4o-mini" / "llama-3.1-8b-instruct")。
    /// env: NEBULA_OPENAI_COMPAT_MODEL。
    pub openai_compat_model: Option<String>,
    /// T-E-S-27: 是否启用 Trusted Diagnostics 通道。默认 true。
    /// 设 `NEBULA_DIAGNOSTICS=0` 关闭。
    pub diagnostics_channel_enabled: bool,
    /// T-E-S-27: DiagnosticsBus broadcast 容量。默认 512。
    /// env: NEBULA_DIAGNOSTICS_BUFFER_CAPACITY
    pub diagnostics_buffer_capacity: usize,
    /// T-E-S-41: models.json 路径(`<app_data_dir>/models.json`)。
    /// 由 `ModelsConfig::resolve_path()` 解析;失败回退 `./models.json`。
    pub models_config_path: std::path::PathBuf,
    /// T-E-S-39: SOUL.md/AGENTS.md/TOOLS.md persona 缓存(运行时共享)。
    /// None 直到首次加载;bootstrap 预加载或 persona_reload 命令后填充。
    pub persona: Option<std::sync::Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>>,
    /// M1 任务 #23: SoulCompiler 实例（cfg-gated）。
    ///
    /// 仅在 `soul-system` feature 编译且 `SOUL_SYSTEM_ENABLED` 运行时开启时可用。
    /// 优先级：Soul > PersonaConfig。当 Soul 编译成功时，CompiledSoul.system_prompt
    /// 替代 PersonaConfig 的 `<soul>` 部分；AGENTS.md / TOOLS.md 仍由 PersonaConfig 提供。
    #[cfg(feature = "soul-system")]
    pub soul_compiler: Option<std::sync::Arc<crate::soul::SoulCompiler>>,
    /// T-E-S-57: 是否启用系统通知。默认 true。
    /// 设 `NEBULA_NOTIFICATIONS=0` 关闭。
    pub notifications_enabled: bool,
    /// T-E-S-57: 是否在悬浮球上显示任务计数角标。默认 true。
    /// 设 `NEBULA_BALL_BADGE=0` 关闭。
    pub floating_ball_task_badge: bool,
    /// T-E-S-44: 存储后端配置(local/s3/webdav)。
    pub storage_backend: crate::storage::StorageConfig,
    /// T-E-S-42: 向量存储后端类型(lance / qdrant / chroma)。默认 lance。
    /// env: NEBULA_VECTOR_STORE_BACKEND
    pub vector_store_backend: VectorStoreBackend,
    /// T-E-S-42: Qdrant REST API 根 URL(如 `http://127.0.0.1:6333`)。
    /// 仅当 `vector_store_backend = qdrant` 时使用。
    /// env: NEBULA_QDRANT_URL
    pub qdrant_url: Option<String>,
    /// T-E-S-42: ChromaDB REST API 根 URL(如 `http://127.0.0.1:8000`)。
    /// 仅当 `vector_store_backend = chroma` 时使用。
    /// env: NEBULA_CHROMA_URL
    pub chroma_url: Option<String>,
    /// T-E-B-01: 是否启用 LLM Wiki 编译引擎。默认 true。
    /// 设 `NEBULA_WIKI=0` 关闭(chat_stream 后不编译笔记)。
    pub wiki_enabled: bool,
    /// T-E-B-01: Wiki 笔记子目录名(默认 "wiki")。
    /// env: NEBULA_WIKI_SUBDIR
    pub wiki_subdir: String,
    /// T-E-S-43: 是否启用 DB 加密(SQLCipher)。默认 false。
    /// env: NEBULA_DB_ENCRYPTION=1
    /// 注:实际加密由 `db_encryption_enable` 命令触发迁移,
    /// 此字段仅指示当前是否应使用加密模式打开 DB。
    pub db_encryption_enabled: bool,
    /// T-E-S-32: mcp_servers.json 配置路径(可选)。
    /// None 时使用默认路径 `<app_data_dir>/mcp_servers.json`。
    /// env: NEBULA_MCP_SERVERS_PATH
    pub mcp_servers_path: Option<std::path::PathBuf>,
    /// T-E-C-02: 视觉模型名称(默认 qwen2.5-vl:3b),供 ScreenReader
    /// 通过 Ollama 多模态 API 理解截图。env: NEBULA_VISION_MODEL
    pub vision_model: String,
}

impl AppConfig {
    /// Loads configuration from environment variables, falling back to
    /// defaults appropriate for a first-run local development setup.
    pub fn from_env() -> Self {
        Self {
            db_path: std::env::var("NEBULA_DB").unwrap_or_else(|_| "nebula.db".to_string()),
            lance_path: std::env::var("NEBULA_LANCE")
                .unwrap_or_else(|_| "nebula_lance".to_string()),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string()),
            chat_model: std::env::var("NEBULA_CHAT_MODEL")
                .unwrap_or_else(|_| "deepseek-chat".to_string()),
            embed_model: std::env::var("NEBULA_EMBED_MODEL")
                .unwrap_or_else(|_| "BAAI/bge-small-zh-v1.5".to_string()),
            // v1.2: 默认走 DeepSeek;设 NEBULA_LLM_PROVIDER=ollama 可回退本地。
            llm_provider: std::env::var("NEBULA_LLM_PROVIDER")
                .unwrap_or_else(|_| "deepseek".to_string()),
            deepseek_api_url: std::env::var("DEEPSEEK_API_URL")
                .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string()),
            deepseek_api_key: std::env::var("DEEPSEEK_API_KEY").ok(),
            remote_fallback_url: std::env::var("NEBULA_REMOTE_URL").ok(),
            blackhole_threshold_days: std::env::var("NEBULA_BH_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            embedding_dim: std::env::var("NEBULA_EMBED_DIM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(512),
            reflect_interval_secs: std::env::var("NEBULA_REFLECT_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_INTERVAL_SECS),
            reflect_window_days: std::env::var("NEBULA_REFLECT_WINDOW_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_WINDOW_DAYS),
            reflect_min_importance: std::env::var("NEBULA_REFLECT_MIN_IMPORTANCE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::memory::reflect::DEFAULT_REFLECT_MIN_IMPORTANCE),
            grpc_enabled: std::env::var("NEBULA_GRPC")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            grpc_bind_addr: std::env::var("NEBULA_GRPC_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:50051".to_string()),
            rest_enabled: std::env::var("NEBULA_REST")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            rest_bind_addr: std::env::var("NEBULA_REST_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            rest_api_token: std::env::var("NEBULA_REST_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
            editor_workspace: std::env::var("NEBULA_WORKSPACE").unwrap_or_else(|_| ".".to_string()),
            sync_inbox: std::env::var("NEBULA_SYNC_INBOX")
                .unwrap_or_else(|_| "sync_inbox".to_string()),
            exec_approval_timeout_secs: std::env::var("NEBULA_EXEC_APPROVAL_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            semantic_cache_enabled: std::env::var("NEBULA_SEMANTIC_CACHE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            cost_tracker_enabled: std::env::var("NEBULA_COST_TRACKER")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            token_juice_enabled: std::env::var("NEBULA_TOKEN_JUICE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            router_enabled: std::env::var("NEBULA_ROUTER")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            router_classifier_model: std::env::var("NEBULA_ROUTER_MODEL")
                .unwrap_or_else(|_| "qwen2.5:3b".to_string()),
            prefix_cache_enabled: std::env::var("NEBULA_PREFIX_CACHE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            daily_budget_usd: std::env::var("NEBULA_DAILY_BUDGET_USD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            // T-E-A-12: 自动化每日预算(None=不限制),超阈值 emit budget_exceeded。
            automation_daily_budget_usd: std::env::var("NEBULA_AUTOMATION_DAILY_BUDGET_USD")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| *v > 0.0),
            // T-E-L-06: Loop 月度美元预算(None 或 0.0=不限制)。
            loop_monthly_budget_usd: std::env::var("NEBULA_LOOP_MONTHLY_BUDGET_USD")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &f64| *v > 0.0),
            // T-E-L-06: Loop 月度 Token 预算(None 或 0=不限制)。
            loop_monthly_budget_tokens: std::env::var("NEBULA_LOOP_MONTHLY_BUDGET_TOKENS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &u64| *v > 0),
            // T-E-S-40: OpenAI 兼容 provider 配置(可选)。
            openai_compat_base_url: std::env::var("NEBULA_OPENAI_COMPAT_URL").ok(),
            openai_compat_api_key: std::env::var("NEBULA_OPENAI_COMPAT_KEY").ok(),
            openai_compat_model: std::env::var("NEBULA_OPENAI_COMPAT_MODEL").ok(),
            // T-E-S-27: Trusted Diagnostics 配置(默认开启,容量 512)。
            diagnostics_channel_enabled: std::env::var("NEBULA_DIAGNOSTICS")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            diagnostics_buffer_capacity: std::env::var("NEBULA_DIAGNOSTICS_BUFFER_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(512),
            // T-E-B-09: 文件夹监控路径(逗号分隔),默认空 Vec。
            watch_paths: std::env::var("NEBULA_WATCH_PATHS")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            // T-E-C-14: 剪贴板监听开关,默认 false(用户需显式开启)。
            clipboard_watch_enabled: std::env::var("NEBULA_CLIPBOARD_WATCH")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            // T-E-S-41: models.json 路径(<app_data_dir>/models.json)。
            models_config_path: crate::llm::models_config::ModelsConfig::resolve_path(),
            // T-E-S-39: persona 缓存,由 bootstrap 预加载填充。
            persona: None,
            // M1 任务 #23: SoulCompiler 实例（cfg-gated）。
            // 由 bootstrap 在 dispatcher 构造后填充（若 soul_system_enabled）。
            #[cfg(feature = "soul-system")]
            soul_compiler: None,
            // T-E-S-57: 通知开关,默认开启。
            notifications_enabled: std::env::var("NEBULA_NOTIFICATIONS")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            // T-E-S-57: 悬浮球任务角标开关,默认开启。
            floating_ball_task_badge: std::env::var("NEBULA_BALL_BADGE")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            // T-E-S-44: 存储后端配置。env: NEBULA_STORAGE_BACKEND
            // (local/s3/webdav,默认 local)。
            storage_backend: {
                let kind =
                    std::env::var("NEBULA_STORAGE_BACKEND").unwrap_or_else(|_| "local".to_string());
                let root = std::env::var("NEBULA_STORAGE_ROOT").unwrap_or_else(|_| {
                    std::env::temp_dir()
                        .join("nebula-storage")
                        .to_string_lossy()
                        .to_string()
                });
                crate::storage::StorageConfig {
                    kind,
                    root,
                    webdav_url: std::env::var("NEBULA_WEBDAV_URL").ok(),
                    webdav_username: std::env::var("NEBULA_WEBDAV_USERNAME").ok(),
                    webdav_password: std::env::var("NEBULA_WEBDAV_PASSWORD").ok(),
                    s3_bucket: std::env::var("NEBULA_S3_BUCKET").ok(),
                    s3_region: std::env::var("NEBULA_S3_REGION").ok(),
                    s3_endpoint: std::env::var("NEBULA_S3_ENDPOINT").ok(),
                }
            },
            // T-E-S-42: 向量存储后端类型(lance / qdrant / chroma)。默认 lance。
            vector_store_backend: {
                let raw = std::env::var("NEBULA_VECTOR_STORE_BACKEND")
                    .unwrap_or_else(|_| "lance".to_string());
                match raw.to_lowercase().as_str() {
                    "qdrant" => VectorStoreBackend::Qdrant,
                    "chroma" => VectorStoreBackend::Chroma,
                    _ => VectorStoreBackend::Lance,
                }
            },
            // T-E-S-42: Qdrant REST API 根 URL(可选)。
            qdrant_url: std::env::var("NEBULA_QDRANT_URL").ok(),
            // T-E-S-42: ChromaDB REST API 根 URL(可选)。
            chroma_url: std::env::var("NEBULA_CHROMA_URL").ok(),
            // T-E-B-01: Wiki 编译引擎开关,默认开启。
            wiki_enabled: std::env::var("NEBULA_WIKI")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true),
            // T-E-B-01: Wiki 笔记子目录,默认 "wiki"。
            wiki_subdir: std::env::var("NEBULA_WIKI_SUBDIR").unwrap_or_else(|_| "wiki".to_string()),
            // T-E-S-43: DB 加密开关,默认 false(用户需显式开启)。
            db_encryption_enabled: std::env::var("NEBULA_DB_ENCRYPTION")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            // T-E-S-32: mcp_servers.json 路径(可选)。
            mcp_servers_path: std::env::var("NEBULA_MCP_SERVERS_PATH")
                .ok()
                .map(std::path::PathBuf::from),
            // T-E-C-02: 视觉模型名(默认 qwen2.5-vl:3b),env: NEBULA_VISION_MODEL。
            vision_model: std::env::var("NEBULA_VISION_MODEL")
                .unwrap_or_else(|_| "qwen2.5-vl:3b".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// T-D-T-02: 确保环境变量测试串行执行,避免并行污染。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 清理所有可能影响 AppConfig::from_env() 的环境变量。
    fn cleanup_env() {
        let keys = [
            "NEBULA_DB",
            "NEBULA_LANCE",
            "OLLAMA_URL",
            "NEBULA_CHAT_MODEL",
            "NEBULA_EMBED_MODEL",
            "NEBULA_LLM_PROVIDER",
            "DEEPSEEK_API_URL",
            "DEEPSEEK_API_KEY",
            "NEBULA_REMOTE_URL",
            "NEBULA_BH_DAYS",
            "NEBULA_EMBED_DIM",
            "NEBULA_REFLECT_INTERVAL",
            "NEBULA_REFLECT_WINDOW_DAYS",
            "NEBULA_REFLECT_MIN_IMPORTANCE",
            "NEBULA_GRPC",
            "NEBULA_GRPC_ADDR",
            "NEBULA_REST",
            "NEBULA_REST_ADDR",
            "NEBULA_REST_TOKEN",
            "NEBULA_WORKSPACE",
            "NEBULA_SYNC_INBOX",
            "NEBULA_EXEC_APPROVAL_TIMEOUT_SECS",
            "NEBULA_SEMANTIC_CACHE",
            "NEBULA_COST_TRACKER",
            "NEBULA_TOKEN_JUICE",
            "NEBULA_ROUTER",
            "NEBULA_ROUTER_MODEL",
            "NEBULA_PREFIX_CACHE",
            "NEBULA_DB_ENCRYPTION",
            "NEBULA_MCP_SERVERS_PATH",
            "NEBULA_VISION_MODEL",
        ];
        for k in &keys {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn test_default_config_values() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        let config = AppConfig::from_env();

        // 路径默认值
        assert_eq!(config.db_path, "nebula.db");
        assert_eq!(config.lance_path, "nebula_lance");
        assert_eq!(config.editor_workspace, ".");
        assert_eq!(config.sync_inbox, "sync_inbox");

        // LLM 默认值
        assert_eq!(config.chat_model, "deepseek-chat");
        assert_eq!(config.llm_provider, "deepseek");
        assert_eq!(config.ollama_url, "http://127.0.0.1:11434");
        assert_eq!(config.deepseek_api_url, "https://api.deepseek.com/v1");
        assert_eq!(config.deepseek_api_key, None);
        assert_eq!(config.remote_fallback_url, None);

        // gRPC 默认开启
        assert!(config.grpc_enabled);
        assert_eq!(config.grpc_bind_addr, "127.0.0.1:50051");

        // REST 默认关闭
        assert!(!config.rest_enabled);
        assert_eq!(config.rest_bind_addr, "127.0.0.1:8080");
        assert_eq!(config.rest_api_token, None);

        // 数值默认值
        assert_eq!(config.blackhole_threshold_days, 30);
        assert_eq!(config.embedding_dim, 512);
        assert_eq!(config.exec_approval_timeout_secs, 60);
    }

    #[test]
    fn test_grpc_disabled_via_env() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        std::env::set_var("NEBULA_GRPC", "0");
        let config = AppConfig::from_env();
        assert!(!config.grpc_enabled);
        std::env::remove_var("NEBULA_GRPC");
    }

    #[test]
    fn test_rest_enabled_via_env() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        std::env::set_var("NEBULA_REST", "1");
        let config = AppConfig::from_env();
        assert!(config.rest_enabled);
        std::env::remove_var("NEBULA_REST");
    }

    #[test]
    fn test_rest_token_filtering() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        // 空字符串应被过滤为 None
        std::env::set_var("NEBULA_REST_TOKEN", "");
        let config = AppConfig::from_env();
        assert_eq!(config.rest_api_token, None);
        // 非空字符串应保留
        std::env::set_var("NEBULA_REST_TOKEN", "secret-token");
        let config = AppConfig::from_env();
        assert_eq!(config.rest_api_token.as_deref(), Some("secret-token"));
        std::env::remove_var("NEBULA_REST_TOKEN");
    }

    #[test]
    fn test_custom_paths_via_env() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        std::env::set_var("NEBULA_DB", "/custom/data.db");
        std::env::set_var("NEBULA_LANCE", "/custom/lance");
        std::env::set_var("NEBULA_GRPC_ADDR", "0.0.0.0:50051");
        std::env::set_var("NEBULA_REST_ADDR", "0.0.0.0:8080");
        let config = AppConfig::from_env();
        assert_eq!(config.db_path, "/custom/data.db");
        assert_eq!(config.lance_path, "/custom/lance");
        assert_eq!(config.grpc_bind_addr, "0.0.0.0:50051");
        assert_eq!(config.rest_bind_addr, "0.0.0.0:8080");
        std::env::remove_var("NEBULA_DB");
        std::env::remove_var("NEBULA_LANCE");
        std::env::remove_var("NEBULA_GRPC_ADDR");
        std::env::remove_var("NEBULA_REST_ADDR");
    }

    #[test]
    fn test_numeric_parsing_via_env() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        std::env::set_var("NEBULA_BH_DAYS", "90");
        std::env::set_var("NEBULA_EMBED_DIM", "1024");
        std::env::set_var("NEBULA_EXEC_APPROVAL_TIMEOUT_SECS", "120");
        let config = AppConfig::from_env();
        assert_eq!(config.blackhole_threshold_days, 90);
        assert_eq!(config.embedding_dim, 1024);
        assert_eq!(config.exec_approval_timeout_secs, 120);
        std::env::remove_var("NEBULA_BH_DAYS");
        std::env::remove_var("NEBULA_EMBED_DIM");
        std::env::remove_var("NEBULA_EXEC_APPROVAL_TIMEOUT_SECS");
    }

    #[test]
    fn test_numeric_invalid_falls_back_to_default() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        // 无效数值应回退到默认值
        std::env::set_var("NEBULA_BH_DAYS", "not-a-number");
        let config = AppConfig::from_env();
        assert_eq!(config.blackhole_threshold_days, 30);
        std::env::remove_var("NEBULA_BH_DAYS");
    }

    #[test]
    fn test_feature_toggles_via_env() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        // 关闭语义缓存
        std::env::set_var("NEBULA_SEMANTIC_CACHE", "0");
        let config = AppConfig::from_env();
        assert!(!config.semantic_cache_enabled);

        // 关闭费用追踪
        std::env::set_var("NEBULA_COST_TRACKER", "false");
        let config = AppConfig::from_env();
        assert!(!config.cost_tracker_enabled);

        // 关闭 TokenJuice
        std::env::set_var("NEBULA_TOKEN_JUICE", "0");
        let config = AppConfig::from_env();
        assert!(!config.token_juice_enabled);

        std::env::remove_var("NEBULA_SEMANTIC_CACHE");
        std::env::remove_var("NEBULA_COST_TRACKER");
        std::env::remove_var("NEBULA_TOKEN_JUICE");
    }

    #[test]
    fn test_api_key_from_env() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        cleanup_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-test-key-123");
        let config = AppConfig::from_env();
        assert_eq!(
            config.deepseek_api_key.as_deref(),
            Some("sk-test-key-123")
        );
        std::env::remove_var("DEEPSEEK_API_KEY");
    }
}
