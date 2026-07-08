//! T-E-S-62: doctor 健康检查 — 全子系统状态自检。
//!
//! 设计文档:`.trae/specs/wire-stage8-p2-doctor/spec.md`
//!
//! 与 `commands::core::health`/`health_full` 的区别:doctor 不止检查
//! Ollama,而是并发检查 10 个子系统(AppConfig / Keychain / SQLite /
//! LanceDB / Ollama / LlmGateway / Sidecar / IPC / 日志目录 / 备份目录),
//! 返回结构化 `DoctorReport`(ok/warn/fail 分级 + 修复建议),由前端
//! DoctorView 渲染。
//!
//! ## 设计约束
//!
//! * **诊断非错误**:任一子检查失败不 panic,降级为 `Fail`/`Warn`。
//! * **并发执行**:子检查通过 `tokio::join!` 并发轮询,不串行阻塞。
//! * **超时控制**:每项子检查 ≤ 2s(用 `tokio::time::timeout`),整体 ≤ 10s。
//! * **自动修复建议**:`Fail`/`Warn` 项附中文 `suggestion`。

use std::future::Future;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::AppState;

/// 单项子检查的超时上限。超时返回 `Fail("检查超时")` 而非 panic。
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// doctor 健康检查总报告。
#[derive(Debug, Serialize)]
pub struct DoctorReport {
    /// 报告生成时间(Unix 时间戳,秒)。
    pub timestamp: i64,
    /// 聚合后的整体状态:任一 fail → fail;任一 warn → warn;全 ok → ok。
    pub overall: CheckStatus,
    /// 各子检查结果(顺序固定)。
    pub checks: Vec<DoctorCheck>,
    /// 总耗时(毫秒)。
    pub duration_ms: u64,
}

/// 单项子检查结果。
#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    /// 子检查名(如 "sqlite" / "ollama" / "lancedb")。
    pub name: String,
    /// 状态分级。
    pub status: CheckStatus,
    /// 简短状态描述(中文)。
    pub message: String,
    /// fail/warn 时的修复建议(中文),ok 时为 None。
    pub suggestion: Option<String>,
    /// 该项子检查耗时(毫秒)。
    pub latency_ms: u64,
}

/// 子检查状态分级。`serde(rename_all = "lowercase")` 序列化为
/// `"ok"` / `"warn"` / `"fail"`,与前端 TS 类型对齐。
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// 聚合所有子检查的状态为 overall。
///
/// 优先级:Fail > Warn > Ok。任一 Fail → Fail;无 Fail 但有 Warn → Warn;
/// 全 Ok → Ok。空切片视为 Ok。
pub fn aggregate_status(checks: &[DoctorCheck]) -> CheckStatus {
    if checks.iter().any(|c| c.status == CheckStatus::Fail) {
        CheckStatus::Fail
    } else if checks.iter().any(|c| c.status == CheckStatus::Warn) {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    }
}

/// 包装单个子检查,附加 2s 超时。超时返回 `Fail("检查超时")`。
///
/// 泛型接受任意 `Future<Output = DoctorCheck>`,避免动态分发开销。
/// `name` 用于超时分支构造 `DoctorCheck`(正常分支由子检查函数自填 name)。
async fn run_one<F>(name: &'static str, fut: F) -> DoctorCheck
where
    F: Future<Output = DoctorCheck>,
{
    match tokio::time::timeout(CHECK_TIMEOUT, fut).await {
        Ok(check) => check,
        Err(_) => DoctorCheck {
            name: name.to_string(),
            status: CheckStatus::Fail,
            message: "检查超时(>2s)".to_string(),
            suggestion: Some("该子系统响应缓慢,请检查对应服务是否正常运行".to_string()),
            latency_ms: CHECK_TIMEOUT.as_millis() as u64,
        },
    }
}

/// 主入口:并发执行 10 个子检查,聚合为 `DoctorReport`。
///
/// 子检查通过 `tokio::join!` 并发轮询(非串行),每项独立 2s 超时。
/// 整体执行 ≤ 10s(实践中 ≈ 单项最慢耗时,通常 ≤ 2s)。
pub async fn run_doctor(state: &AppState) -> DoctorReport {
    let start = Instant::now();
    let timestamp = chrono::Local::now().timestamp();

    // tokio::join! 并发轮询所有 run_one 包装器;每个 run_one 内部
    // 独立 2s 超时。所有子检查共享 &AppState 的不可变借用(Sync)。
    let (app_config, keychain, sqlite, lancedb, ollama, gateway, sidecar, ipc, logs, backup) = tokio::join!(
        run_one("app_config", check_app_config(state)),
        run_one("keychain", check_keychain(state)),
        run_one("sqlite", check_sqlite(state)),
        run_one("lancedb", check_lancedb(state)),
        run_one("ollama", check_ollama(state)),
        run_one("gateway", check_gateway(state)),
        run_one("sidecar", check_sidecar(state)),
        run_one("ipc", check_ipc(state)),
        run_one("logs", check_logs()),
        run_one("backup", check_backup()),
    );

    let checks = vec![
        app_config, keychain, sqlite, lancedb, ollama, gateway, sidecar, ipc, logs, backup,
    ];
    let overall = aggregate_status(&checks);

    DoctorReport {
        timestamp,
        overall,
        checks,
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

// ---------------------------------------------------------------------------
// 子检查实现
// ---------------------------------------------------------------------------

/// 1. AppConfig 加载状态。state.config 在 bootstrap 时已加载,此处仅验证可读。
async fn check_app_config(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let db_path = state.infra.config.db_path.clone();
    let lance_path = state.infra.config.lance_path.clone();
    DoctorCheck {
        name: "app_config".to_string(),
        status: CheckStatus::Ok,
        message: format!("已加载(db={}, lance={})", db_path, lance_path),
        suggestion: None,
        latency_ms: start.elapsed().as_millis() as u64,
    }
}

/// 2. Keychain — 解析 DeepSeek API key。keychain 优先,env var 兜底。
/// 有 key → Ok;无 key → Warn(本地 Ollama 仍可用)。
async fn check_keychain(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let provider = state.infra.config.llm_provider.clone();
    // keychain 访问是 OS 阻塞调用,放 spawn_blocking 避免阻塞 runtime。
    let result = tokio::task::spawn_blocking(crate::security::keychain::resolve_deepseek_key).await;
    match result {
        Ok(Some(_)) => DoctorCheck {
            name: "keychain".to_string(),
            status: CheckStatus::Ok,
            message: format!("DeepSeek API key 已配置(provider={})", provider),
            suggestion: None,
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Ok(None) => DoctorCheck {
            name: "keychain".to_string(),
            status: CheckStatus::Warn,
            message: "用户未配置 DeepSeek key(本地 Ollama 仍可用)".to_string(),
            suggestion: Some("如需使用 DeepSeek,请在设置中配置 API key".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Err(e) => DoctorCheck {
            name: "keychain".to_string(),
            status: CheckStatus::Fail,
            message: format!("keychain 任务失败: {}", e),
            suggestion: Some("检查 OS keychain 服务是否可用".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// 3. SQLite — 读取内嵌 migration 状态,全部 applied → Ok。
async fn check_sqlite(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let sqlite = state.memory.sqlite.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = sqlite.raw_connection();
        let g = conn.lock();
        crate::memory::migration::bundled_migration_status(&g)
    })
    .await;
    match result {
        Ok(Ok(status)) => {
            let total = status.applied.len();
            let applied = status.applied.iter().filter(|m| m.applied).count();
            if total == 0 || applied == total {
                DoctorCheck {
                    name: "sqlite".to_string(),
                    status: CheckStatus::Ok,
                    message: format!(
                        "迁移已应用({}/{}, v{})",
                        applied, total, status.current_version
                    ),
                    suggestion: None,
                    latency_ms: start.elapsed().as_millis() as u64,
                }
            } else {
                DoctorCheck {
                    name: "sqlite".to_string(),
                    status: CheckStatus::Warn,
                    message: format!("部分迁移未应用({}/{})", applied, total),
                    suggestion: Some("请重启应用以运行 pending 迁移".to_string()),
                    latency_ms: start.elapsed().as_millis() as u64,
                }
            }
        }
        Ok(Err(e)) => DoctorCheck {
            name: "sqlite".to_string(),
            status: CheckStatus::Fail,
            message: format!("读取迁移状态失败: {}", e),
            suggestion: Some("检查 SQLite 数据库文件权限".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
        Err(e) => DoctorCheck {
            name: "sqlite".to_string(),
            status: CheckStatus::Fail,
            message: format!("spawn_blocking 失败: {}", e),
            suggestion: Some("系统资源不足,请重试".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// 4. LanceDB — 通过 VectorStore::health_check 验证后端可用性。
///
/// T-E-S-42: 改用 trait health_check,按 AppConfig.vector_store_backend
/// 分发(Lance 验证表句柄 / Qdrant GET / / Chroma GET /api/v1/heartbeat)。
/// 保留 lance_path 目录检查作为辅助信息(向后兼容)。
async fn check_lancedb(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let lance_path = state.infra.config.lance_path.clone();
    // T-E-S-42: 调用 VectorStore::health_check 验证后端可用性。
    // trait 方法对 Lance/Qdrant/Chroma 各有实现,doctor 不需要关心具体后端。
    let health = state.memory.lance.health_check().await;
    let dir_exists = tokio::fs::metadata(&lance_path)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false);
    match health {
        Ok(()) => {
            let backend = match state.infra.config.vector_store_backend {
                crate::memory::vector_store::VectorStoreBackend::Lance => "LanceDB",
                crate::memory::vector_store::VectorStoreBackend::Qdrant => "Qdrant",
                crate::memory::vector_store::VectorStoreBackend::Chroma => "ChromaDB",
            };
            DoctorCheck {
                name: "lancedb".to_string(),
                status: CheckStatus::Ok,
                message: format!(
                    "{} 后端健康(backend={}, path={})",
                    backend, backend, lance_path
                ),
                suggestion: None,
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        Err(e) => {
            // health_check 失败:目录可能尚未创建(Lance 内存降级模式),
            // 或 Qdrant/Chroma 服务不可达。降级为 Warn 并附修复建议。
            let suggestion = match state.infra.config.vector_store_backend {
                crate::memory::vector_store::VectorStoreBackend::Lance => {
                    "检查 lance_path 目录权限或磁盘空间".to_string()
                }
                crate::memory::vector_store::VectorStoreBackend::Qdrant => {
                    "请启动 Qdrant 服务(docker run qdrant/qdrant)".to_string()
                }
                crate::memory::vector_store::VectorStoreBackend::Chroma => {
                    "请启动 ChromaDB 服务(chroma run --host 127.0.0.1 --port 8000)".to_string()
                }
            };
            DoctorCheck {
                name: "lancedb".to_string(),
                status: CheckStatus::Warn,
                message: format!(
                    "向量后端健康检查失败: {} (path={}, dir_exists={})",
                    e, lance_path, dir_exists
                ),
                suggestion: Some(suggestion),
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
    }
}

/// 5. Ollama — ping /api/tags,2s 超时(由 run_one 包装)。失败 → Fail。
async fn check_ollama(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let client = state.llm.llm.ollama_client();
    let ping = client.ping().await;
    if ping {
        DoctorCheck {
            name: "ollama".to_string(),
            status: CheckStatus::Ok,
            message: format!("Ollama 服务可用({})", state.infra.config.ollama_url),
            suggestion: None,
            latency_ms: start.elapsed().as_millis() as u64,
        }
    } else {
        DoctorCheck {
            name: "ollama".to_string(),
            status: CheckStatus::Fail,
            message: "Ollama 未启动或不可达".to_string(),
            suggestion: Some("请启动 Ollama 服务(ollama serve)".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// 6. LlmGateway — 检查日预算是否超限(CircuitBreaker 内部状态未公开暴露,
/// `raw_state` 为 `#[cfg(test)]`;此处用 `is_over_daily_budget` 作为代理)。
async fn check_gateway(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let over_budget = state.llm.llm.is_over_daily_budget();
    if over_budget {
        DoctorCheck {
            name: "gateway".to_string(),
            status: CheckStatus::Warn,
            message: "已达日预算上限(LLM 调用降级到本地 Ollama)".to_string(),
            suggestion: Some("在设置中调高日预算或等待次日重置".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        }
    } else {
        DoctorCheck {
            name: "gateway".to_string(),
            status: CheckStatus::Ok,
            message: "LlmGateway 正常(预算未超限)".to_string(),
            suggestion: None,
            latency_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// 7. Sidecar — 遍历 6 个 SidecarKind,统计 running/stopped。
/// 进程内模式下所有 kind 标记为 Running,任一 stopped → Warn。
async fn check_sidecar(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let mut running = 0usize;
    let mut stopped_names: Vec<String> = Vec::new();
    for kind in crate::sidecar::SidecarKind::all() {
        if state.platform.sidecar_manager.is_running(kind) {
            running += 1;
        } else {
            stopped_names.push(kind.as_str().to_string());
        }
    }
    let total = crate::sidecar::SidecarKind::all().len();
    if stopped_names.is_empty() {
        DoctorCheck {
            name: "sidecar".to_string(),
            status: CheckStatus::Ok,
            message: format!("所有 sidecar 运行中({}/{})", running, total),
            suggestion: None,
            latency_ms: start.elapsed().as_millis() as u64,
        }
    } else {
        DoctorCheck {
            name: "sidecar".to_string(),
            status: CheckStatus::Warn,
            message: format!(
                "{} 个 sidecar 未运行: {}",
                stopped_names.len(),
                stopped_names.join(", ")
            ),
            suggestion: Some("进程内模式会自动降级;如需独立进程请在设置中启动 sidecar".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// 8. IPC — 构造 IpcLayer 并调 all_healthy()(检查 5 个 sidecar IPC 通道)。
/// AppState 未常驻 IpcLayer,此处从 sidecar_manager 临时构造。
async fn check_ipc(state: &AppState) -> DoctorCheck {
    let start = Instant::now();
    let layer = crate::sidecar::ipc::IpcLayer::new(state.platform.sidecar_manager.clone());
    let healthy = layer.all_healthy().await;
    if healthy {
        DoctorCheck {
            name: "ipc".to_string(),
            status: CheckStatus::Ok,
            message: "所有 sidecar IPC 通道健康".to_string(),
            suggestion: None,
            latency_ms: start.elapsed().as_millis() as u64,
        }
    } else {
        DoctorCheck {
            name: "ipc".to_string(),
            status: CheckStatus::Warn,
            message: "部分 sidecar IPC 通道不可用".to_string(),
            suggestion: Some("检查 sidecar 进程状态".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        }
    }
}

/// 9. 日志目录可写 — 写测试文件到 log dir,成功删除 → Ok。
async fn check_logs() -> DoctorCheck {
    let start = Instant::now();
    let dir = match log_dir() {
        Some(d) => d,
        None => {
            return DoctorCheck {
                name: "logs".to_string(),
                status: CheckStatus::Warn,
                message: "无法确定日志目录(平台未知)".to_string(),
                suggestion: Some("设置 NEBULA_LOG_DIR 环境变量".to_string()),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };
    let _ = tokio::fs::create_dir_all(&dir).await;
    let probe = dir.join("doctor_probe.tmp");
    match tokio::fs::write(&probe, b"doctor probe").await {
        Ok(_) => {
            let _ = tokio::fs::remove_file(&probe).await;
            DoctorCheck {
                name: "logs".to_string(),
                status: CheckStatus::Ok,
                message: format!("日志目录可写({})", dir.display()),
                suggestion: None,
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        Err(e) => DoctorCheck {
            name: "logs".to_string(),
            status: CheckStatus::Fail,
            message: format!("日志目录不可写: {}", e),
            suggestion: Some("检查目录权限或磁盘空间".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// 10. 备份目录 — 检查 default_backup_root() 存在 + 最近备份 mtime。
async fn check_backup() -> DoctorCheck {
    let start = Instant::now();
    let backup_root = crate::backup::scheduler::default_backup_root();
    if !backup_root.exists() {
        return DoctorCheck {
            name: "backup".to_string(),
            status: CheckStatus::Warn,
            message: format!("备份目录不存在({})", backup_root.display()),
            suggestion: Some("首次备份将在每日 02:00 自动创建".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        };
    }
    let mut entries = match tokio::fs::read_dir(&backup_root).await {
        Ok(e) => e,
        Err(e) => {
            return DoctorCheck {
                name: "backup".to_string(),
                status: CheckStatus::Warn,
                message: format!("读取备份目录失败: {}", e),
                suggestion: Some("检查目录权限".to_string()),
                latency_ms: start.elapsed().as_millis() as u64,
            };
        }
    };
    let mut latest: Option<std::time::SystemTime> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(meta) = entry.metadata().await {
            if meta.is_dir() {
                if let Ok(mtime) = meta.modified() {
                    latest = Some(latest.map(|t| t.max(mtime)).unwrap_or(mtime));
                }
            }
        }
    }
    match latest {
        Some(mtime) => {
            let age_secs = mtime.elapsed().map(|d| d.as_secs()).unwrap_or(u64::MAX);
            // 超过 7 天的备份视为 Warn(可能备份调度器未运行)。
            let status = if age_secs > 7 * 24 * 3600 {
                CheckStatus::Warn
            } else {
                CheckStatus::Ok
            };
            let suggestion = if status == CheckStatus::Warn {
                Some("备份调度器可能未运行,请检查后台任务".to_string())
            } else {
                None
            };
            DoctorCheck {
                name: "backup".to_string(),
                status,
                message: format!("最近备份: {} 秒前", age_secs),
                suggestion,
                latency_ms: start.elapsed().as_millis() as u64,
            }
        }
        None => DoctorCheck {
            name: "backup".to_string(),
            status: CheckStatus::Warn,
            message: "备份目录为空(尚无备份)".to_string(),
            suggestion: Some("首次备份将在每日 02:00 自动创建".to_string()),
            latency_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// 解析日志目录(优先 NEBULA_LOG_DIR,否则用平台默认)。
/// 与 `lib.rs::default_log_dir` 同逻辑,因后者为私有函数,此处复制实现。
fn log_dir() -> Option<std::path::PathBuf> {
    if let Ok(d) = std::env::var("NEBULA_LOG_DIR") {
        return Some(std::path::PathBuf::from(d));
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|d| std::path::PathBuf::from(d).join("nebula").join("logs"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| std::path::PathBuf::from(d).join("Library/Logs/nebula"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOME")
            .ok()
            .map(|d| std::path::PathBuf::from(d).join(".local/share/nebula/logs"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, status: CheckStatus) -> DoctorCheck {
        DoctorCheck {
            name: name.to_string(),
            status,
            message: String::new(),
            suggestion: None,
            latency_ms: 0,
        }
    }

    #[test]
    fn test_overall_aggregation_ok() {
        let checks = vec![check("a", CheckStatus::Ok), check("b", CheckStatus::Ok)];
        assert_eq!(aggregate_status(&checks), CheckStatus::Ok);
    }

    #[test]
    fn test_overall_aggregation_warn() {
        let checks = vec![
            check("a", CheckStatus::Ok),
            check("b", CheckStatus::Warn),
            check("c", CheckStatus::Ok),
        ];
        assert_eq!(aggregate_status(&checks), CheckStatus::Warn);
    }

    #[test]
    fn test_overall_aggregation_fail() {
        let checks = vec![check("a", CheckStatus::Ok), check("b", CheckStatus::Fail)];
        assert_eq!(aggregate_status(&checks), CheckStatus::Fail);
    }

    #[test]
    fn test_overall_aggregation_priority() {
        // 同时有 Warn 和 Fail → Fail(Fail 优先级高于 Warn)。
        let checks = vec![
            check("a", CheckStatus::Warn),
            check("b", CheckStatus::Fail),
            check("c", CheckStatus::Warn),
        ];
        assert_eq!(aggregate_status(&checks), CheckStatus::Fail);
    }

    #[test]
    fn test_aggregation_empty_is_ok() {
        let checks: Vec<DoctorCheck> = vec![];
        assert_eq!(aggregate_status(&checks), CheckStatus::Ok);
    }

    /// 超时场景:用 sleep(3s) 模拟超时函数,验证 run_one 的 2s 超时
    /// 包装返回 Fail 而非 panic,且 suggestion 非 None。
    #[tokio::test]
    async fn test_timeout_returns_fail() {
        let slow = async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            check("slow", CheckStatus::Ok)
        };
        let result = run_one("slow", slow).await;
        assert_eq!(result.status, CheckStatus::Fail, "超时应返回 Fail");
        assert_eq!(result.name, "slow");
        assert_eq!(result.message, "检查超时(>2s)");
        assert!(
            result.suggestion.is_some(),
            "Fail 项的 suggestion 必须非 None"
        );
    }

    /// 正常完成的子检查应原样透传(不被超时包装篡改)。
    #[tokio::test]
    async fn test_run_one_passes_through_ok() {
        let quick = async { check("quick", CheckStatus::Ok) };
        let result = run_one("quick", quick).await;
        assert_eq!(result.status, CheckStatus::Ok);
        assert_eq!(result.name, "quick");
        assert!(result.suggestion.is_none());
    }

    /// serde 序列化:CheckStatus 小写化,与前端 TS 类型对齐。
    #[test]
    fn test_check_status_serde_lowercase() {
        let ok = serde_json::to_string(&CheckStatus::Ok).expect("serialize should succeed");
        let warn = serde_json::to_string(&CheckStatus::Warn).expect("serialize should succeed");
        let fail = serde_json::to_string(&CheckStatus::Fail).expect("serialize should succeed");
        assert_eq!(ok, "\"ok\"");
        assert_eq!(warn, "\"warn\"");
        assert_eq!(fail, "\"fail\"");
    }

    /// DoctorReport 序列化包含所有字段。
    #[test]
    fn test_doctor_report_serializes() {
        let report = DoctorReport {
            timestamp: 1_700_000_000,
            overall: CheckStatus::Warn,
            checks: vec![check("x", CheckStatus::Warn)],
            duration_ms: 42,
        };
        let json = serde_json::to_string(&report).expect("serialize should succeed");
        assert!(json.contains("\"overall\":\"warn\""));
        assert!(json.contains("\"timestamp\":1700000000"));
        assert!(json.contains("\"duration_ms\":42"));
        assert!(json.contains("\"name\":\"x\""));
    }
}
