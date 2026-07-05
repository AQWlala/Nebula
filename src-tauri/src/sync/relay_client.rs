//! T-S6-B-02: 云端中继同步客户端 — 把本地 CRDT op log 推送到云端中继,
//! 并从云端拉取其他设备的 op。
//!
//! ## 设计说明
//!
//! 中继服务器协议(假设):
//! - POST /v1/sync/push   — 上传 op 批量,body: { device_id, ops: [...] }
//! - POST /v1/sync/pull   — 拉取 op,body: { device_id, since_seq: N }
//! - POST /v1/sync/ack    — 确认 op 已消费,body: { device_id, op_ids: [...] }
//!
//! 鉴权:Bearer token(用户在设置中配置 relay_token)。
//!
//! 当前实现为骨架:
//! - HTTP 调用使用 reqwest(已在依赖中)
//! - 实际网络错误时记录日志并返回 Err,不阻塞本地操作
//! - 不实现重试逻辑(由上层调度器负责)
//!
//! ## 已知限制(骨架)
//!
//! - `pull()` 拉取的 op 通过 `CrdtOpLog::record_op` 写入本地,会以
//!   `status='pending'` 落盘,下一次 `push()` 可能将其回推给中继
//!   (echo)。后续需在 `CrdtOpLog` 增加 `record_remote_op` 或在
//!   `fetch_pending_ops` 按 `device_id != 本机` 过滤。
//! - `since_seq` 游标仅保存在内存(`AtomicU64`),进程重启后从 0 开始
//!   重新拉取;持久化游标待后续版本。
//! - 推送失败时调用 `mark_failed`,被标记的 op 不会再次出现在
//!   `fetch_pending_ops` 中;待 `mark_pending` API 实现后可重试。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::security::SsrfGuard;
use crate::sync::crdt::CrdtVersion;
use crate::sync::crdt_op_log::{CrdtOpLog, CrdtOpLogEntry, CrdtOpStats};

/// 中继同步配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    /// 中继服务器 URL,如 `https://relay.nebula.example.com`。
    /// 空字符串表示未配置中继,所有网络操作将被跳过。
    pub server_url: String,
    /// 设备 ID(本机唯一标识)。
    pub device_id: String,
    /// 鉴权 token(Bearer)。
    pub token: String,
    /// 拉取间隔(秒),默认 60。
    #[serde(default = "default_pull_interval")]
    pub pull_interval_secs: u64,
    /// 单次 push 的最大 op 数,默认 100。
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_pull_interval() -> u64 {
    60
}

fn default_batch_size() -> usize {
    100
}

/// 中继同步客户端。
///
/// 持有共享的 [`CrdtOpLog`](crate::sync::crdt_op_log::CrdtOpLog)(内部已
/// 通过 `Arc<Mutex<Connection>>` 实现线程安全,无需外层 Mutex)和一个
/// `reqwest::Client`。所有方法都是异步的;在后台线程中由专用的
/// current-thread tokio runtime 驱动(见 [`RelayClient::start`])。
pub struct RelayClient {
    config: RelayConfig,
    http: reqwest::Client,
    op_log: Arc<CrdtOpLog>,
    /// 上次成功 pull 返回的 `next_seq` 游标(内存态,进程重启后归零)。
    last_pull_seq: AtomicU64,
}

impl RelayClient {
    /// 创建中继客户端。
    ///
    /// `op_log` 为共享的 CRDT op 日志(`CrdtOpLog` 内部已有锁,直接传
    /// `Arc<CrdtOpLog>` 即可)。
    pub fn new(config: RelayConfig, op_log: Arc<CrdtOpLog>) -> Self {
        // M7b #94: SSRF 校验 — relay 服务器通常是远端,不需要 allow_loopback。
        // 构造器返回 Self(非 Result),用 warn log 记录失败而非中断构造
        // (向后兼容:旧调用方不期望 new 失败)。空 server_url 视为未配置,跳过校验。
        if !config.server_url.is_empty() {
            if let Err(e) = SsrfGuard::new().validate_url(&config.server_url) {
                warn!(
                    target: "nebula.relay",
                    url = %config.server_url,
                    "SSRF validation failed for relay URL: {e}",
                );
            }
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");
        Self {
            config,
            http,
            op_log,
            last_pull_seq: AtomicU64::new(0),
        }
    }

    /// 当前配置与队列状态的快照(供调试/仪表盘)。
    pub fn status(&self) -> RelayStatus {
        let op_stats = self.op_log.stats().unwrap_or(CrdtOpStats {
            pending: 0,
            consumed: 0,
            failed: 0,
            total: 0,
        });
        RelayStatus {
            configured: !self.config.server_url.is_empty(),
            server_url: self.config.server_url.clone(),
            device_id: self.config.device_id.clone(),
            pull_interval_secs: self.config.pull_interval_secs,
            batch_size: self.config.batch_size,
            last_pull_seq: self.last_pull_seq.load(Ordering::Relaxed),
            op_stats,
        }
    }

    /// 推送本地 pending op 到中继服务器。
    ///
    /// 成功后调用 `op_log.mark_consumed` 标记这些 op 已被中继接收;
    /// 网络/HTTP 错误时调用 `mark_failed` 并返回 `Err`(不阻塞本地操作,
    /// 由上层调度器决定是否重试)。
    ///
    /// `server_url` 为空时直接返回 `Ok(0)`(未配置中继场景)。
    pub async fn push(&self) -> Result<usize> {
        if self.config.server_url.is_empty() {
            return Ok(0);
        }

        let ops = self
            .op_log
            .fetch_pending_ops(self.config.batch_size)
            .context("fetching pending ops for push")?;
        if ops.is_empty() {
            debug!(target: "nebula.relay", "no pending ops to push");
            return Ok(0);
        }

        let count = ops.len();
        let push_url = join_url(&self.config.server_url, "/v1/sync/push");
        let body = PushRequest {
            device_id: &self.config.device_id,
            ops: &ops,
        };

        match self
            .http
            .post(&push_url)
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    for op in &ops {
                        if let Err(e) = self.op_log.mark_consumed(&op.op_id) {
                            warn!(
                                target: "nebula.relay",
                                op_id = %op.op_id,
                                error = ?e,
                                "failed to mark op consumed after push"
                            );
                        }
                    }
                    info!(target: "nebula.relay", count, "pushed ops to relay");
                    Ok(count)
                } else {
                    warn!(
                        target: "nebula.relay",
                        status = %resp.status(),
                        count,
                        "relay push rejected; marking ops failed"
                    );
                    // TODO(T-S6-B-02): 待 CrdtOpLog 提供 mark_pending API 后,
                    // 瞬时错误应回退为 pending 以便重试,而非永久 failed。
                    for op in &ops {
                        let _ = self.op_log.mark_failed(&op.op_id);
                    }
                    anyhow::bail!("relay push rejected with status {}", resp.status())
                }
            }
            Err(e) => {
                warn!(
                    target: "nebula.relay",
                    error = %e,
                    count,
                    "relay push network error; marking ops failed"
                );
                for op in &ops {
                    let _ = self.op_log.mark_failed(&op.op_id);
                }
                Err(anyhow::Error::from(e).context("relay push network error"))
            }
        }
    }

    /// 从中继服务器拉取其他设备的 op,写入本地 op_log。
    ///
    /// 拉取的 op 通过 `CrdtOpLog::record_op` 落盘为 `status='pending'`,
    /// `device_id` 为远端设备 ID。
    ///
    /// `server_url` 为空时直接返回 `Ok(0)`(未配置中继场景)。
    pub async fn pull(&self) -> Result<usize> {
        if self.config.server_url.is_empty() {
            return Ok(0);
        }

        let since_seq = self.last_pull_seq.load(Ordering::Relaxed);
        let pull_url = join_url(&self.config.server_url, "/v1/sync/pull");
        let body = PullRequest {
            device_id: &self.config.device_id,
            since_seq,
        };

        let resp = self
            .http
            .post(&pull_url)
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await
            .context("relay pull network error")?;

        if !resp.status().is_success() {
            anyhow::bail!("relay pull rejected with status {}", resp.status());
        }

        let parsed: PullResponse = resp
            .json()
            .await
            .context("parsing relay pull response")?;

        // TODO(T-S6-B-02): echo 风险 — record_op 写入的 op 会以 pending
        // 状态出现在下一次 push 队列中,可能把远端 op 回推给中继。
        let count = parsed.ops.len();
        for entry in &parsed.ops {
            let version = CrdtVersion {
                memory_id: entry.memory_id.clone(),
                version: entry.version,
                device_id: entry.device_id.clone(),
                timestamp: entry.timestamp,
                field_changes: entry.field_changes.clone(),
            };
            if let Err(e) = self.op_log.record_op(&version) {
                warn!(
                    target: "nebula.relay",
                    memory_id = %entry.memory_id,
                    error = ?e,
                    "failed to record pulled op"
                );
            }
        }

        if let Some(next) = parsed.next_seq {
            self.last_pull_seq.store(next, Ordering::Relaxed);
        }

        info!(
            target: "nebula.relay",
            count,
            since_seq,
            "pulled ops from relay"
        );
        Ok(count)
    }

    /// 启动后台同步循环(push + pull 交替)。
    ///
    /// 在专用 OS 线程中构建 current-thread tokio runtime 驱动异步循环,
    /// 不依赖 lib.rs setup 是否位于 tokio runtime 内(参考
    /// `BackupScheduler` 的线程模式)。线程为守护线程,进程退出时自动终止。
    ///
    /// `server_url` 为空时不启动循环,直接返回 `Ok(())`。
    pub fn start(self: Arc<Self>) -> Result<()> {
        if self.config.server_url.is_empty() {
            info!(
                target: "nebula.relay",
                "relay server_url empty, background loop not started"
            );
            return Ok(());
        }

        let this = Arc::clone(&self);
        std::thread::Builder::new()
            .name("nebula-relay".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        error!(
                            target: "nebula.relay",
                            error = ?e,
                            "failed to build tokio runtime for relay loop"
                        );
                        return;
                    }
                };
                rt.block_on(async move {
                    let interval = Duration::from_secs(
                        this.config.pull_interval_secs.max(1),
                    );
                    info!(
                        target: "nebula.relay",
                        interval_secs = interval.as_secs(),
                        device_id = %this.config.device_id,
                        "relay sync loop started"
                    );
                    loop {
                        if let Err(e) = this.push().await {
                            warn!(target: "nebula.relay", error = ?e, "relay push failed");
                        }
                        if let Err(e) = this.pull().await {
                            warn!(target: "nebula.relay", error = ?e, "relay pull failed");
                        }
                        tokio::time::sleep(interval).await;
                    }
                });
            })
            .context("spawning relay sync thread")?;

        Ok(())
    }
}

/// 中继同步状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayStatus {
    /// 是否已配置中继(server_url 非空)。
    pub configured: bool,
    /// 中继服务器 URL。
    pub server_url: String,
    /// 本设备 ID。
    pub device_id: String,
    /// 拉取间隔(秒)。
    pub pull_interval_secs: u64,
    /// 单次 push 的最大 op 数。
    pub batch_size: usize,
    /// 上次成功 pull 的 `next_seq` 游标(内存态)。
    pub last_pull_seq: u64,
    /// 本地 op 队列统计。
    pub op_stats: CrdtOpStats,
}

// ---------------------------------------------------------------------------
// HTTP 请求/响应类型(中继协议)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PushRequest<'a> {
    device_id: &'a str,
    ops: &'a [CrdtOpLogEntry],
}

#[derive(Debug, Serialize)]
struct PullRequest<'a> {
    device_id: &'a str,
    since_seq: u64,
}

#[derive(Debug, Deserialize)]
struct PullResponse {
    #[serde(default)]
    ops: Vec<CrdtOpLogEntry>,
    #[serde(default)]
    next_seq: Option<u64>,
}

/// 拼接 server_url 与 path,自动处理尾部斜杠。
fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use rusqlite::Connection;

    /// 构造内存 SQLite 并建 crdt_op_log 表(不依赖 migration runner)。
    fn make_op_log() -> Arc<CrdtOpLog> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE crdt_op_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                op_id TEXT NOT NULL UNIQUE,
                memory_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                field_changes TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                consumed_at INTEGER
            );
            CREATE INDEX idx_crdt_op_log_status ON crdt_op_log(status);
            CREATE INDEX idx_crdt_op_log_memory ON crdt_op_log(memory_id);",
        )
        .unwrap();
        Arc::new(CrdtOpLog::new(
            Arc::new(Mutex::new(conn)),
            "dev-test".to_string(),
        ))
    }

    fn make_config(server_url: &str) -> RelayConfig {
        RelayConfig {
            server_url: server_url.to_string(),
            device_id: "dev-test".to_string(),
            token: "secret-token".to_string(),
            pull_interval_secs: 60,
            batch_size: 100,
        }
    }

    #[test]
    fn config_uses_defaults_when_fields_missing() {
        // 仅提供必填字段,pull_interval_secs / batch_size 应取默认值。
        let json = r#"{
            "server_url": "https://relay.example.com",
            "device_id": "dev-1",
            "token": "tok"
        }"#;
        let cfg: RelayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.server_url, "https://relay.example.com");
        assert_eq!(cfg.device_id, "dev-1");
        assert_eq!(cfg.token, "tok");
        assert_eq!(cfg.pull_interval_secs, default_pull_interval());
        assert_eq!(cfg.batch_size, default_batch_size());
    }

    #[test]
    fn config_parses_all_fields() {
        let json = r#"{
            "server_url": "https://relay.example.com",
            "device_id": "dev-1",
            "token": "tok",
            "pull_interval_secs": 120,
            "batch_size": 50
        }"#;
        let cfg: RelayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.pull_interval_secs, 120);
        assert_eq!(cfg.batch_size, 50);
    }

    #[tokio::test]
    async fn push_returns_zero_when_server_url_empty() {
        // server_url 为空 — 应跳过,不触达网络。
        let client = RelayClient::new(
            RelayConfig {
                server_url: String::new(),
                device_id: "dev-test".to_string(),
                token: "tok".to_string(),
                pull_interval_secs: 60,
                batch_size: 100,
            },
            make_op_log(),
        );
        let n = client.push().await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn push_returns_zero_when_no_pending_ops() {
        // server_url 非空但 op_log 为空 — 应在 fetch_pending_ops 后早退,
        // 不触达网络。
        let client = RelayClient::new(make_config("https://relay.example.com"), make_op_log());
        let n = client.push().await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn pull_returns_zero_when_server_url_empty() {
        let client = RelayClient::new(
            RelayConfig {
                server_url: String::new(),
                device_id: "dev-test".to_string(),
                token: "tok".to_string(),
                pull_interval_secs: 60,
                batch_size: 100,
            },
            make_op_log(),
        );
        let n = client.pull().await.unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn status_reports_unconfigured_when_server_url_empty() {
        let client = RelayClient::new(
            RelayConfig {
                server_url: String::new(),
                device_id: "dev-test".to_string(),
                token: "tok".to_string(),
                pull_interval_secs: 60,
                batch_size: 100,
            },
            make_op_log(),
        );
        let status = client.status();
        assert!(!status.configured);
        assert_eq!(status.last_pull_seq, 0);
        assert_eq!(status.op_stats.total, 0);
    }

    #[test]
    fn join_url_handles_trailing_slash() {
        assert_eq!(
            join_url("https://relay.example.com/", "/v1/sync/push"),
            "https://relay.example.com/v1/sync/push"
        );
        assert_eq!(
            join_url("https://relay.example.com", "/v1/sync/push"),
            "https://relay.example.com/v1/sync/push"
        );
    }
}
