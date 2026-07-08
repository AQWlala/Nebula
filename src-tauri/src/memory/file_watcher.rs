//! T-E-B-09: 文件夹监控索引引擎。
//!
//! [`FileWatcherEngine`] 监控用户指定的一组目录,当受支持扩展名的
//! 文件被创建或修改时,通过 [`SpongeEngine::absorb_file`] 把内容
//! 吸收到 L3 语义记忆层。设计目标:
//!
//! * **非阻塞** — `notify` 回调线程只做 `try_send`,真正的文件读取 +
//!   embedding 在 tokio 消费者 task 中执行。
//! * **可热更新** — `reload_paths()` 替换 watcher 集合而不取消
//!   消费者 task,保存设置后立即生效。
//! * **可优雅停机** — `stop()` 通过 `CancellationToken` 通知消费者
//!   退出,并在 250ms 内 join,与 reflection worker 一致。
//!
//! 参考:`src-tauri/src/editor/file_ops.rs:261-305`(watcher 模板)、
//! `src-tauri/src/memory/reflect.rs:567-600`(select! worker 模板)。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::sponge::SpongeEngine;
use super::types::{MemoryLayer, MemoryType, SourceKind};

/// 单个被监控文件的最大字节数(8 MiB),与 `editor::file_ops::MAX_FILE_BYTES` 一致。
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// 监控跳过的目录名(与 `editor::file_ops::SKIP_DIRS` 保持一致)。
/// 这些目录下的文件事件会被消费者丢弃。
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    ".DS_Store",
];

/// 受支持的白名单扩展名。二进制 / 大文件 / 未知扩展名会被跳过。
const ALLOWED_EXTENSIONS: &[&str] = &[
    "md", "txt", "rst", "org", "code", "json", "yaml", "toml", "pdf", "docx",
];

/// 同一路径的去抖窗口:在 300ms 内的多次事件只处理一次。
const DEBOUNCE: Duration = Duration::from_millis(300);

/// mpsc 通道容量。满时 `try_send` 丢弃事件并 `warn!`。
const CHANNEL_CAPACITY: usize = 256;

/// 停机等待 worker 退出的超时(与 reflection worker 一致)。
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);

/// 监控状态快照,序列化给前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStatus {
    /// `true` 表示至少有一个 watcher 在运行且消费者 task 未被取消。
    pub active: bool,
    /// 当前正在监控的目录(canonicalized 字符串形式)。
    pub paths: Vec<String>,
}

/// 文件夹监控引擎。`Arc<FileWatcherEngine>` 在 `AppState` 中共享。
pub struct FileWatcherEngine {
    sponge: Arc<SpongeEngine>,
    watchers: Mutex<Vec<RecommendedWatcher>>,
    cancel_token: CancellationToken,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
    current_paths: RwLock<Vec<PathBuf>>,
    /// 持有 sender 使得 `reload_paths` 可以在不重建消费者 task 的情况下
    /// 替换底层 watcher。`None` 表示从未 `start` 或已 `stop`。
    sender: Mutex<Option<mpsc::Sender<PathBuf>>>,
    /// 由 `start` 创建、`spawn_worker` 取走的 receiver。
    /// `None` 表示从未 start、已 stop,或消费者 task 已启动。
    receiver: Mutex<Option<mpsc::Receiver<PathBuf>>>,
}

impl FileWatcherEngine {
    /// 构造一个未启动的引擎。调用方随后通过 `start` + `spawn_worker`
    /// 启动监控。
    pub fn new(sponge: Arc<SpongeEngine>) -> Self {
        Self {
            sponge,
            watchers: Mutex::new(Vec::new()),
            cancel_token: CancellationToken::new(),
            worker_handle: Mutex::new(None),
            current_paths: RwLock::new(Vec::new()),
            sender: Mutex::new(None),
            receiver: Mutex::new(None),
        }
    }

    /// 为每个 `paths` 创建 `RecommendedWatcher`(notify 6.1),
    /// 事件通过 `mpsc::channel(256)` 发送给消费者 task。
    /// 首次调用会创建 channel 并把 receiver 存入 self,供 `spawn_worker` 取走。
    /// 重复调用会复用现有 sender(支持热更新)。
    pub fn start(&self, paths: Vec<PathBuf>) {
        // 1. 校验 + canonicalize + 拒绝 SKIP_DIRS
        let valid = validate_paths(paths);

        // 2. 获取 / 创建 sender(以及配套 receiver)
        let tx = {
            let mut guard = self.sender.lock();
            match guard.clone() {
                Some(existing_tx) => existing_tx,
                None => {
                    let (tx, rx) = mpsc::channel::<PathBuf>(CHANNEL_CAPACITY);
                    // 把 receiver 存入 self,spawn_worker 会 take 它
                    *self.receiver.lock() = Some(rx);
                    *guard = Some(tx.clone());
                    tx
                }
            }
        };

        // 3. 为每个 path 创建 RecommendedWatcher
        let new_watchers = build_watchers(&valid, tx);

        // 4. 替换 watchers(旧 watcher drop 即停止)
        *self.watchers.lock() = new_watchers;

        // 5. 更新 current_paths
        *self.current_paths.write() = valid;
    }

    /// 停止所有 watcher + 取消消费者 task。
    /// idempotent;安全可从 shutdown 路径多次调用。
    pub async fn stop(&self) {
        // 1. 取消消费者 task
        self.cancel_token.cancel();

        // 2. 清空 watchers(立即停止产生新事件)
        self.watchers.lock().clear();

        // 3. 清空 sender + receiver(防止 reload 后残留)
        *self.sender.lock() = None;
        *self.receiver.lock() = None;

        // 4. 清空 current_paths
        *self.current_paths.write() = Vec::new();

        // 5. 等待 worker 退出(250ms 超时)
        let handle = self.worker_handle.lock().take();
        if let Some(h) = handle {
            match tokio::time::timeout(SHUTDOWN_TIMEOUT, h).await {
                Ok(_) => info!(target: "nebula.file_watcher", "worker stopped"),
                Err(_) => warn!(
                    target: "nebula.file_watcher",
                    "worker did not stop within {:?}",
                    SHUTDOWN_TIMEOUT
                ),
            }
        }
    }

    /// 热更新监控路径:替换 watcher 集合,保留消费者 task。
    /// 等价于 "drop 旧 watchers + start new watchers",
    /// **不**取消 cancel_token,因此消费者 task 继续运行。
    /// 若 engine 从未 start 过,会退化为 `start`。
    pub fn reload_paths(&self, new_paths: Vec<PathBuf>) {
        // 清空旧 watcher(立即停止产生新事件)
        self.watchers.lock().clear();

        let tx_opt = self.sender.lock().clone();
        let Some(tx) = tx_opt else {
            // 从未 start 过 —— 直接走 start 路径
            warn!(
                target: "nebula.file_watcher",
                "reload_paths called before start; delegating to start"
            );
            self.start(new_paths);
            return;
        };

        let valid = validate_paths(new_paths);
        let new_watchers = build_watchers(&valid, tx);
        *self.watchers.lock() = new_watchers;
        *self.current_paths.write() = valid;
    }

    /// 启动消费者 task。返回 `JoinHandle` 供调用方管理生命周期。
    /// 参考 `src-tauri/src/memory/reflect.rs:567-600` 的 select! 模式。
    pub fn spawn_worker(self: Arc<Self>) -> Option<JoinHandle<()>> {
        let rx = self.receiver.lock().take();
        let Some(rx) = rx else {
            warn!(
                target: "nebula.file_watcher",
                "spawn_worker called but no receiver available (already running or not started)"
            );
            return None;
        };
        let sponge = self.sponge.clone();
        let cancel_token = self.cancel_token.clone();
        let handle = tokio::spawn(async move {
            info!(target: "nebula.file_watcher", "worker started");
            let mut rx = rx;
            let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        info!(target: "nebula.file_watcher", "worker received cancellation");
                        break;
                    }
                    Some(path) = rx.recv() => {
                        pending.insert(path, Instant::now());
                    }
                    _ = tokio::time::sleep(DEBOUNCE) => {
                        if pending.is_empty() {
                            continue;
                        }
                        let now = Instant::now();
                        let ready: Vec<PathBuf> = pending
                            .iter()
                            .filter(|(_, t)| now.duration_since(**t) >= DEBOUNCE)
                            .map(|(p, _)| p.clone())
                            .collect();
                        for path in ready {
                            pending.remove(&path);
                            process_path(&sponge, &path).await;
                        }
                    }
                }
            }
            info!(target: "nebula.file_watcher", "worker exiting");
        });
        Some(handle)
    }

    /// 返回当前监控状态快照。
    pub fn status(&self) -> WatchStatus {
        let paths = self
            .current_paths
            .read()
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let active = !self.watchers.lock().is_empty() && !self.cancel_token.is_cancelled();
        WatchStatus { active, paths }
    }

    /// 返回当前监控路径列表(字符串形式)。
    pub fn list_paths(&self) -> Vec<String> {
        self.current_paths
            .read()
            .iter()
            .map(|p| p.display().to_string())
            .collect()
    }

    /// 把 `JoinHandle` 存入 self,供 `stop()` 等待。
    /// 由 bootstrap 在 `spawn_worker` 之后调用。
    pub fn set_worker_handle(&self, handle: Option<JoinHandle<()>>) {
        *self.worker_handle.lock() = handle;
    }
}

/// 校验 + canonicalize 一组路径,返回有效目录列表。
/// 拒绝非目录、canonicalize 失败、以及自身位于 SKIP_DIRS 的路径。
fn validate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter_map(|p| match p.canonicalize() {
            Ok(c) if c.is_dir() => {
                if is_skip_dir_component(&c) {
                    warn!(
                        target: "nebula.file_watcher",
                        path = %c.display(),
                        "rejecting watch target (skip-dir)"
                    );
                    None
                } else {
                    Some(c)
                }
            }
            Ok(c) => {
                warn!(
                    target: "nebula.file_watcher",
                    path = %c.display(),
                    "watch target is not a directory; skipping"
                );
                None
            }
            Err(e) => {
                warn!(
                    target: "nebula.file_watcher",
                    path = %p.display(),
                    error = ?e,
                    "canonicalize failed; skipping"
                );
                None
            }
        })
        .collect()
}

/// 为每个 path 创建一个 `RecommendedWatcher`,所有 watcher 共享 `tx`。
fn build_watchers(paths: &[PathBuf], tx: mpsc::Sender<PathBuf>) -> Vec<RecommendedWatcher> {
    let mut watchers: Vec<RecommendedWatcher> = Vec::with_capacity(paths.len());
    for path in paths {
        let tx_clone = tx.clone();
        let mut watcher: RecommendedWatcher =
            match notify::recommended_watcher(move |res: notify::Result<Event>| {
                handle_event(&res, &tx_clone);
            }) {
                Ok(w) => w,
                Err(e) => {
                    warn!(
                        target: "nebula.file_watcher",
                        path = %path.display(),
                        error = ?e,
                        "failed to create watcher"
                    );
                    continue;
                }
            };
        if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
            warn!(
                target: "nebula.file_watcher",
                path = %path.display(),
                error = ?e,
                "watcher.watch failed"
            );
            continue;
        }
        info!(target: "nebula.file_watcher", path = %path.display(), "watcher started");
        watchers.push(watcher);
    }
    watchers
}

/// notify 事件回调:过滤 Create/Modify,把文件路径 `try_send` 到 channel。
fn handle_event(res: &notify::Result<Event>, tx: &mpsc::Sender<PathBuf>) {
    match res {
        Ok(ev) => {
            if !matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                return;
            }
            for path in &ev.paths {
                // 只发送文件事件(目录创建会被后续文件事件覆盖)
                if path.is_file() {
                    if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(path.clone()) {
                        warn!(
                            target: "nebula.file_watcher",
                            path = %path.display(),
                            "event channel full; dropping event"
                        );
                    }
                }
            }
        }
        Err(e) => {
            warn!(target: "nebula.file_watcher", error = ?e, "watcher error");
        }
    }
}

/// 检查路径的任一组件是否是 SKIP_DIRS 中的名称。
/// 用于:(a) start 时拒绝直接 watch 一个 SKIP_DIR;
///      (b) 消费者处理时跳过 SKIP_DIRS 子树下的文件。
fn is_skip_dir_component(path: &Path) -> bool {
    for comp in path.components() {
        if let std::path::Component::Normal(name) = comp {
            if let Some(s) = name.to_str() {
                if SKIP_DIRS.contains(&s) {
                    return true;
                }
            }
        }
    }
    false
}

/// 消费者处理单个文件路径:校验扩展名 + 大小 + 跳过 SKIP_DIRS,
/// 然后调 `sponge.absorb_file(path, Semantic, L3, External)`。
async fn process_path(sponge: &SpongeEngine, path: &Path) {
    // 1. 跳过 SKIP_DIRS 子树
    if is_skip_dir_component(path) {
        return;
    }

    // 2. 扩展名白名单
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_ascii_lowercase(),
        None => return,
    };
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return;
    }

    // 3. 文件大小校验
    let metadata = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) => {
            warn!(
                target: "nebula.file_watcher",
                path = %path.display(),
                error = ?e,
                "metadata failed; skipping"
            );
            return;
        }
    };
    if metadata.len() > MAX_FILE_BYTES {
        warn!(
            target: "nebula.file_watcher",
            path = %path.display(),
            size = metadata.len(),
            max = MAX_FILE_BYTES,
            "file exceeds 8MiB; skipping"
        );
        return;
    }

    // 4. 吸收到记忆系统
    match sponge
        .absorb_file(
            path,
            MemoryType::Semantic,
            MemoryLayer::L3,
            SourceKind::External,
        )
        .await
    {
        Ok(result) => {
            info!(
                target: "nebula.file_watcher",
                path = %path.display(),
                id = %result.id(),
                "absorbed file"
            );
        }
        Err(e) => {
            warn!(
                target: "nebula.file_watcher",
                path = %path.display(),
                error = ?e,
                "absorb_file failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_dir_component_detects_git() {
        assert!(is_skip_dir_component(Path::new("/foo/.git")));
        assert!(is_skip_dir_component(Path::new("/foo/.git/HEAD")));
        assert!(is_skip_dir_component(Path::new("/foo/node_modules/bar")));
        assert!(!is_skip_dir_component(Path::new("/foo/bar/baz")));
    }

    #[test]
    fn allowed_extensions_lowercase() {
        assert!(ALLOWED_EXTENSIONS.contains(&"md"));
        assert!(ALLOWED_EXTENSIONS.contains(&"json"));
        assert!(!ALLOWED_EXTENSIONS.contains(&"exe"));
        assert!(!ALLOWED_EXTENSIONS.contains(&"png"));
    }

    #[test]
    fn watch_status_serializes() {
        let s = WatchStatus {
            active: true,
            paths: vec!["/tmp".to_string()],
        };
        let json = serde_json::to_string(&s).expect("serialize should succeed");
        assert!(json.contains("\"active\":true"));
        assert!(json.contains("/tmp"));
    }
}
