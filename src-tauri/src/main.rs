//! nebula Tauri 2.0 application entry point.
//!
//! 三模式入口:
//! * `--features headless` → Docker headless 模式(gRPC + REST,无窗口)。T-E-C-20。
//! * `nebula cost report [...]` → CLI 模式(T-E-A-08)。
//! * 无子命令 → GUI 启动(走 `nebula_lib::run`)。
//!
//! `windows_subsystem = "windows"` 保留(release 构建);CLI 模式下通过
//! `AttachConsole(-1)` 附加到父进程控制台以使 stdout 可见。

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

// ---------------------------------------------------------------------------
// T-E-C-20: headless Docker 模式 — gRPC + REST API,无 Tauri 窗口。
// 启用方式: cargo build --features headless --no-default-features
// ---------------------------------------------------------------------------
#[cfg(feature = "headless")]
fn main() {
    nebula_lib::init_tracing();
    tracing::info!(target: "nebula", mode = "headless", version = env!("CARGO_PKG_VERSION"), "starting nebula (headless)");

    let config = nebula_lib::AppConfig::from_env();
    tracing::info!(target: "nebula", db_path = ?config.db_path, "loaded configuration");

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    rt.block_on(async {
        let state = match nebula_lib::AppState::bootstrap_headless(config).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "nebula", error = ?e, "failed to bootstrap app state (headless)");
                std::process::exit(1);
            }
        };

        // 启动 gRPC server
        #[cfg(feature = "grpc")]
        if state.infra.config.grpc_enabled {
            match nebula_lib::grpc::start_server(
                state.infra.config.grpc_bind_addr.clone(),
                state.clone(),
            )
            .await
            {
                Ok(handle) => {
                    tracing::info!(
                        target: "nebula",
                        addr = %state.infra.config.grpc_bind_addr,
                        "gRPC server started (headless)"
                    );
                    *state.platform.grpc_server.lock() = Some(handle);
                }
                Err(e) => {
                    tracing::error!(
                        target: "nebula",
                        error = ?e,
                        "gRPC server failed to start (headless)"
                    );
                }
            }
        }

        // 启动 REST server
        #[cfg(feature = "rest-api")]
        if state.infra.config.rest_enabled {
            let addr: std::net::SocketAddr = match state.infra.config.rest_bind_addr.parse() {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(
                        target: "nebula",
                        addr = %state.infra.config.rest_bind_addr,
                        error = %e,
                        "invalid rest_bind_addr, aborting REST server startup"
                    );
                    std::process::exit(1);
                }
            };
            let rest_server = nebula_lib::api::rest::RestApiServer::new(
                addr,
                std::sync::Arc::new(state.clone()),
            );
            tokio::spawn(async move {
                if let Err(e) = rest_server.start().await {
                    tracing::error!(target: "nebula", error = ?e, "REST server failed (headless)");
                }
            });
            tracing::info!(
                target: "nebula",
                addr = %addr,
                "REST API server started (headless)"
            );
        }

        // 等待 Ctrl+C 信号
        tracing::info!(target: "nebula", "headless mode ready, waiting for ctrl-c");
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(target: "nebula", error = ?e, "ctrl_c handler failed, exiting");
        } else {
            tracing::info!(target: "nebula", "shutting down (headless)");
        }
    });
}

// ---------------------------------------------------------------------------
// GUI / CLI 模式(Tauri 桌面)
// ---------------------------------------------------------------------------
#[cfg(not(feature = "headless"))]
use clap::{Parser, Subcommand};

/// nebula CLI / GUI 入口。
#[cfg(not(feature = "headless"))]
#[derive(Parser, Debug)]
#[command(
    name = "nebula",
    about = "nebula — a local-first AI assistant",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Commands>,
}

/// 顶层子命令。
#[cfg(not(feature = "headless"))]
#[derive(Subcommand, Debug)]
enum Commands {
    /// 费用报告(T-E-A-08)。
    Cost {
        #[command(subcommand)]
        action: CostAction,
    },
    /// 技能发布(T-E-S-46)。
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
}

/// `cost` 下的子操作。
#[cfg(not(feature = "headless"))]
#[derive(Subcommand, Debug)]
enum CostAction {
    /// 输出本月各模型费用明细。
    Report {
        /// 月份过滤(YYYY-MM),默认当月。
        #[arg(long)]
        month: Option<String>,
        /// JSON 输出(便于脚本消费)。
        #[arg(long)]
        json: bool,
    },
}

/// `skill` 下的子操作(T-E-S-46)。
#[cfg(not(feature = "headless"))]
#[derive(Subcommand, Debug)]
enum SkillAction {
    /// 发布技能到社区市场(GitHub Gist / 本地文件)。
    Publish {
        /// 要发布的 skill ID。
        #[arg(long)]
        id: String,
        /// 发布目标:`gist`(默认) / `file` / `clawhub`(预留)。
        #[arg(long, default_value = "gist")]
        target: String,
        /// `--target file` 时的输出目录(默认 `./skills_export`)。
        #[arg(long)]
        out_dir: Option<String>,
        /// 覆盖 manifest.author(默认空)。
        #[arg(long)]
        author: Option<String>,
        /// 覆盖 manifest.version(默认 `0.1.0`)。
        #[arg(long)]
        version: Option<String>,
        /// 仅打印 SKILL.md 到 stdout,不发布。
        #[arg(long)]
        dry_run: bool,
        /// JSON 输出(便于脚本消费)。
        #[arg(long)]
        json: bool,
    },
}

#[cfg(not(feature = "headless"))]
fn main() {
    let cli = Cli::parse();

    if let Some(cmd) = cli.cmd {
        // CLI 模式:Windows 上需附加到父进程控制台,否则 windows_subsystem
        // = "windows" 的 release 构建 stdout 不可见。
        #[cfg(target_os = "windows")]
        // SAFETY: `AttachConsole` 接受一个进程 ID(u32::MAX 表示附加到父进程控制台),
        // 不接受任何指针参数,不会通过指针写入调用方内存。失败时返回 0,
        // 我们仅记录日志,不 panic —— 后续 println! 会写到无效句柄但不影响程序运行。
        unsafe {
            // 附加到父进程控制台失败是正常的(父进程可能无控制台,如服务宿主),
            // 仅记录,不 panic —— 后续 println! 会写到无效句柄,但不影响程序运行。
            let ok = windows_sys::Win32::System::Console::AttachConsole(u32::MAX);
            if ok == 0 {
                eprintln!("[nebula] AttachConsole failed (parent has no console)");
            }
        }
        if let Err(e) = run_cli(cmd) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    } else {
        // GUI 模式。
        nebula_lib::run();
    }
}

/// CLI 分发器。
#[cfg(not(feature = "headless"))]
fn run_cli(cmd: Commands) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Cost { action } => match action {
            CostAction::Report { month, json } => {
                run_cost_report(month, json)?;
            }
        },
        Commands::Skill { action } => match action {
            SkillAction::Publish {
                id,
                target,
                out_dir,
                author,
                version,
                dry_run,
                json,
            } => {
                run_skill_publish(&id, &target, out_dir, author, version, dry_run, json)?;
            }
        },
    }
    Ok(())
}

/// T-E-S-46: 执行 `nebula skill publish` CLI 子命令。
///
/// 流程:
/// 1. 解析 SQLite 路径(`NEBULA_DB` env 优先,否则 `./nebula.db`)。
/// 2. 打开 `SqliteStore` + `run_bundled_migrations`。
/// 3. `SkillStore::get(id)` 读取 skill。
/// 4. 生成 `SKILL.md`(内联 `skill_to_skill_md`)。
/// 5. 构造 `PublishManifest`(参考 `marketplace::generate_manifest` 的字段
///    映射,允许 `--author` / `--version` 覆盖)+ `validate_manifest`。
/// 6. 按 `target` 分发:
///    * `--dry-run`:打印 `SKILL.md` 到 stdout,结束。
///    * `--target file`:`FilePublisher` 写 `<out_dir>/<id>.md`。
///    * `--target gist`:读 keychain `publisher:github` token,`GistPublisher` 上传。
///
/// `--json` 标志在 file/gist 模式下输出 `PublishResult` 的 JSON;在 dry-run
/// 模式下输出 `{ "skill_md": "<...>" }`。
#[cfg(not(feature = "headless"))]
fn run_skill_publish(
    id: &str,
    target: &str,
    out_dir: Option<String>,
    author: Option<String>,
    version: Option<String>,
    dry_run: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use nebula_lib::memory::SqliteStore;
    use nebula_lib::security::keychain;
    use nebula_lib::skills::marketplace::{PublishManifest, SkillMarketplace};
    use nebula_lib::skills::publisher::{
        skill_to_skill_md, FilePublisher, GistPublisher, SkillPublisher,
    };
    use nebula_lib::skills::store::SkillStore;
    use std::collections::HashMap;

    // 1. 解析 SQLite 路径。CLI 模式无 AppState,用 NEBULA_DB env,
    //    缺省回退到相对路径 ./nebula.db(用户在仓库根目录运行)。
    let db_path = std::env::var("NEBULA_DB").unwrap_or_else(|_| "./nebula.db".to_string());

    // 2. 打开 SQLite + 跑 migrations(SqliteStore::open 内部已跑 001 + 后续;
    //    SkillStore::new 仅校验 skills 表存在)。
    let sqlite = SqliteStore::open(&db_path)?;
    let store = SkillStore::new(sqlite)?;

    // 3. 读取 skill。
    let skill = store
        .get(id)?
        .ok_or_else(|| format!("skill not found: {id}"))?;

    // 4. 生成 SKILL.md(内联 to_skill_md,不依赖 exporter.rs)。
    let skill_md = skill_to_skill_md(&skill)?;

    // 5. 构造 PublishManifest(参考 marketplace::generate_manifest 的字段映射)。
    //    允许 --author / --version 覆盖默认值。
    let manifest = PublishManifest {
        manifest_version: "1.0".to_string(),
        id: skill.id.clone(),
        name: skill.name.clone(),
        version: version.unwrap_or_else(|| "0.1.0".to_string()),
        description: skill.description.clone(),
        author: author.unwrap_or_default(),
        tags: skill.tags.clone(),
        source_url: None,
        dependencies: vec![],
        min_nebula_version: Some("1.3.0".to_string()),
        extra: HashMap::new(),
    };
    SkillMarketplace::validate_manifest(&manifest)?;

    // 6. 按 target 分发。
    if dry_run {
        // --dry-run:打印 SKILL.md 到 stdout,不发布。
        if json {
            let payload = serde_json::json!({ "skill_md": skill_md });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else {
            print!("{skill_md}");
        }
        return Ok(());
    }

    // file / gist 模式需异步运行 publish。CLI 模式无 tokio runtime,
    // 在此构造一个一次性 runtime。
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let result = match target {
        "file" => {
            let out = out_dir
                .clone()
                .unwrap_or_else(|| "./skills_export".to_string());
            let publisher = FilePublisher::new(&out);
            rt.block_on(publisher.publish(&skill_md, &manifest, None))?
        }
        "gist" => {
            let token = keychain::get_publisher_token("github")?;
            let publisher = GistPublisher::new()?;
            rt.block_on(publisher.publish(&skill_md, &manifest, token.as_deref()))?
        }
        other => {
            return Err(
                format!("unknown target: {other}; expected `gist`, `file` or `--dry-run`").into(),
            );
        }
    };

    if json {
        let payload = serde_json::to_value(&result)?;
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        match result.target.as_str() {
            "gist" => {
                let url = result.url.as_deref().unwrap_or("(no url)");
                println!("Published skill `{id}` to GitHub Gist: {url}");
            }
            "file" => {
                let path = result.file_path.as_deref().unwrap_or("(no path)");
                println!("Exported skill `{id}` to: {path}");
            }
            _ => println!("Published skill `{id}` to {}", result.target),
        }
    }

    Ok(())
}

/// T-E-A-08: 执行费用报告输出。
///
/// CLI 模式无 `AppState`;`CostTracker` 为内存态且当前无持久化
/// (持久化留作 T-E-A-13)。此处构造空 tracker,输出空表/空 JSON
/// 作为子命令框架验证。应用启动后产生的费用记录可通过 Tauri
/// 命令 `cost_report` 查询(走 `AppState.cost_tracker`)。
#[cfg(not(feature = "headless"))]
fn run_cost_report(month: Option<String>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    use chrono::Datelike;
    use nebula_lib::llm::cost_tracker::CostTracker;

    let tracker = CostTracker::new();
    let rows = tracker.aggregate_by_model(month.clone());

    let now = chrono::Utc::now();
    let month_str = month.unwrap_or_else(|| format!("{:04}-{:02}", now.year(), now.month()));

    if json {
        // 规范化 -0.0 → 0.0,避免 JSON 输出 "-0.0"。
        let total_cost: f64 = normalize_zero(rows.iter().map(|r| r.cost_usd).sum::<f64>());
        let payload = serde_json::json!({
            "month": month_str,
            "rows": rows,
            "total_cost_usd": total_cost,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_cost_table(&month_str, &rows);
    }
    Ok(())
}

/// 人类可读表格输出。
#[cfg(not(feature = "headless"))]
fn print_cost_table(month: &str, rows: &[nebula_lib::llm::cost_tracker::ModelCostRow]) {
    let sep = "═".repeat(60);
    println!("Model Cost Report ({})", month);
    println!("{}", sep);
    println!(
        "{:<22} {:<11} {:<7} {:<9} {:<10}",
        "Model", "Provider", "Calls", "Tokens", "Cost(USD)"
    );
    for r in rows {
        println!(
            "{:<22} {:<11} {:<7} {:<9} {:.4}",
            r.model,
            r.provider,
            r.call_count,
            r.total_tokens,
            normalize_zero(r.cost_usd)
        );
    }
    println!("{}", sep);
    let total_tokens: u64 = rows.iter().map(|r| r.total_tokens).sum();
    let total_cost: f64 = normalize_zero(rows.iter().map(|r| r.cost_usd).sum::<f64>());
    println!(
        "{:<22} {:<11} {:<7} {:<9} {:.4}",
        "Total", "", "", total_tokens, total_cost
    );
}

/// 将 `-0.0` 规范化为 `0.0`,避免输出 "-0.0000" / "-0.0"。
/// IEEE 754 中 `0.0 == -0.0` 为 true,此处利用该特性做替换。
#[cfg(not(feature = "headless"))]
fn normalize_zero(v: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}
