//! T-E-S-54: 文件触发器 — 复制 `memory/file_watcher.rs` 的
//! notify + mpsc + debounce 模板,但每个 trigger 一个独立 worker。
//!
//! 设计差异(与 FileWatcherEngine 对比):
//! * 每个 `FileTriggerWorker` 实例对应一个 trigger,而非全局单例。
//! * 消费者 task 调 `engine.dispatch(trigger_id, payload)` 而非 sponge.absorb。
//! * debounce 300ms(同 file_watcher.rs)。
//! * glob 匹配(简化版:只支持 `*` 通配符)。
//! * `&self` 接口 + `Mutex<Option<JoinHandle>>` 内部可变性。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// 同一路径的去抖窗口(参考 `file_watcher.rs::DEBOUNCE`)。
const DEBOUNCE: Duration = Duration::from_millis(300);

/// mpsc 通道容量(参考 `file_watcher.rs::CHANNEL_CAPACITY`)。
const CHANNEL_CAPACITY: usize = 256;

/// 单个文件触发器 worker。每个 trigger 实例持有一组 watcher + 一个消费者 task。
pub struct FileTriggerWorker {
    cancel: CancellationToken,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    watchers: Arc<Mutex<Vec<RecommendedWatcher>>>,
}

impl FileTriggerWorker {
    /// 构造未启动的 worker。
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            handle: Arc::new(Mutex::new(None)),
            watchers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 启动 worker:为每个 path 创建 watcher,spawn 消费者 task。
    /// 若已有运行中的 task,先停止再启动。
    pub fn start(
        &mut self,
        paths: Vec<PathBuf>,
        patterns: Vec<String>,
        events: Vec<String>,
        trigger_id: String,
        engine: Arc<super::TriggerEngine>,
    ) {
        // 先停止已有 task。
        self.stop();

        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel::<(PathBuf, String)>(CHANNEL_CAPACITY);

        // 创建 watcher。
        let watchers = build_watchers(&paths, tx);
        *self.watchers.lock() = watchers;

        let handle_storage = self.handle.clone();
        let cancel_clone = self.cancel.clone();
        let handle = tokio::spawn(async move {
            info!(
                target: "nebula.triggers.file",
                trigger_id = %trigger_id,
                patterns_count = patterns.len(),
                "file trigger worker started"
            );
            let mut rx = rx;
            let mut pending: HashMap<PathBuf, (Instant, String)> = HashMap::new();
            loop {
                tokio::select! {
                    biased;
                    _ = cancel_clone.cancelled() => {
                        info!(
                            target: "nebula.triggers.file",
                            trigger_id = %trigger_id,
                            "worker received cancellation"
                        );
                        break;
                    }
                    Some((path, event_kind)) = rx.recv() => {
                        pending.insert(path, (Instant::now(), event_kind));
                    }
                    _ = tokio::time::sleep(DEBOUNCE) => {
                        if pending.is_empty() {
                            continue;
                        }
                        let now = Instant::now();
                        let ready: Vec<(PathBuf, String)> = pending
                            .iter()
                            .filter(|(_, (t, _))| now.duration_since(*t) >= DEBOUNCE)
                            .map(|(p, (_, k))| (p.clone(), k.clone()))
                            .collect();
                        for (path, event_kind) in ready {
                            pending.remove(&path);
                            // glob 匹配。
                            if !matches_patterns(&path, &patterns) {
                                continue;
                            }
                            // events 类型过滤(空表示全部)。
                            if !events.is_empty() && !events.iter().any(|e| e == &event_kind) {
                                continue;
                            }
                            let payload = serde_json::json!({
                                "path": path.display().to_string(),
                                "event_kind": event_kind,
                                "source_trigger_id": trigger_id,
                            });
                            let engine = Arc::clone(&engine);
                            let tid = trigger_id.clone();
                            let payload_clone = payload.clone();
                            tokio::spawn(async move {
                                engine.dispatch(&tid, payload_clone).await;
                            });
                            debug!(
                                target: "nebula.triggers.file",
                                trigger_id = %trigger_id,
                                path = %path.display(),
                                event_kind = %event_kind,
                                "dispatched"
                            );
                        }
                    }
                }
            }
            info!(
                target: "nebula.triggers.file",
                trigger_id = %trigger_id,
                "worker exiting"
            );
            let _ = cancel; // keep cancel alive
        });
        *handle_storage.lock() = Some(handle);
    }

    /// 停止 worker:取消 token + abort task + 清空 watcher。
    pub fn stop(&self) {
        self.cancel.cancel();
        if let Some(h) = self.handle.lock().take() {
            h.abort();
        }
        self.watchers.lock().clear();
    }
}

impl Default for FileTriggerWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FileTriggerWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 为每个 path 创建一个 `RecommendedWatcher`,所有 watcher 共享 `tx`。
/// 参考 `file_watcher.rs::build_watchers`。
fn build_watchers(
    paths: &[PathBuf],
    tx: mpsc::Sender<(PathBuf, String)>,
) -> Vec<RecommendedWatcher> {
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
                        target: "nebula.triggers.file",
                        path = %path.display(),
                        error = ?e,
                        "failed to create watcher"
                    );
                    continue;
                }
            };
        if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
            warn!(
                target: "nebula.triggers.file",
                path = %path.display(),
                error = ?e,
                "watcher.watch failed"
            );
            continue;
        }
        info!(
            target: "nebula.triggers.file",
            path = %path.display(),
            "watcher started"
        );
        watchers.push(watcher);
    }
    watchers
}

/// notify 事件回调:把文件路径 + event_kind 字符串 try_send 到 channel。
fn handle_event(res: &notify::Result<Event>, tx: &mpsc::Sender<(PathBuf, String)>) {
    match res {
        Ok(ev) => {
            let event_kind = event_kind_str(&ev.kind).to_string();
            if event_kind.is_empty() {
                return;
            }
            for path in &ev.paths {
                if path.is_file() {
                    if let Err(mpsc::error::TrySendError::Full(_)) =
                        tx.try_send((path.clone(), event_kind.clone()))
                    {
                        warn!(
                            target: "nebula.triggers.file",
                            path = %path.display(),
                            "event channel full; dropping event"
                        );
                    }
                }
            }
        }
        Err(e) => {
            warn!(target: "nebula.triggers.file", error = ?e, "watcher error");
        }
    }
}

/// 把 notify EventKind 映射为字符串标签(create/modify/remove)。
/// 其他 kind 返回空串(被调用方过滤)。
fn event_kind_str(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Create(_) => "create",
        EventKind::Modify(_) => "modify",
        EventKind::Remove(_) => "remove",
        _ => "",
    }
}

/// 简化版 glob 匹配:支持 `*` 通配符(不匹配路径分隔符)。
/// 空 patterns 列表表示匹配所有文件。
pub fn matches_patterns(path: &Path, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(s) => s,
        None => return false,
    };
    let path_str = path.to_string_lossy();
    patterns
        .iter()
        .any(|p| glob_match(p, file_name) || glob_match(p, &path_str))
}

/// 简化 glob 匹配:`*` 匹配任意非分隔符字符序列。
/// 参考 `glob` crate 的简化实现,无外部依赖。
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, &txt)
}

fn glob_match_inner(pat: &[char], txt: &[char]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let (mut star_pi, mut star_ti): (Option<usize>, usize) = (None, 0);
    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == txt[ti] || pat[pi] == '?') {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_patterns_empty_matches_all() {
        assert!(matches_patterns(Path::new("/tmp/foo.txt"), &[]));
        assert!(matches_patterns(Path::new("/tmp/bar.rs"), &[]));
    }

    #[test]
    fn test_matches_patterns_star_md() {
        let patterns = vec!["*.md".to_string()];
        assert!(matches_patterns(Path::new("/tmp/README.md"), &patterns));
        assert!(!matches_patterns(Path::new("/tmp/README.txt"), &patterns));
    }

    #[test]
    fn test_matches_patterns_multiple() {
        let patterns = vec!["*.md".to_string(), "*.txt".to_string()];
        assert!(matches_patterns(Path::new("/tmp/a.md"), &patterns));
        assert!(matches_patterns(Path::new("/tmp/b.txt"), &patterns));
        assert!(!matches_patterns(Path::new("/tmp/c.rs"), &patterns));
    }

    #[test]
    fn test_glob_match_simple() {
        assert!(glob_match("*.md", "README.md"));
        assert!(glob_match("*.md", "a.md"));
        assert!(!glob_match("*.md", "a.txt"));
        assert!(glob_match("foo*", "foobar"));
        assert!(glob_match("*bar", "foobar"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
    }

    #[test]
    fn test_event_kind_str() {
        assert_eq!(
            event_kind_str(&EventKind::Create(notify::event::CreateKind::File)),
            "create"
        );
        assert_eq!(
            event_kind_str(&EventKind::Modify(notify::event::ModifyKind::Any)),
            "modify"
        );
        assert_eq!(
            event_kind_str(&EventKind::Remove(notify::event::RemoveKind::File)),
            "remove"
        );
        assert_eq!(
            event_kind_str(&EventKind::Access(notify::event::AccessKind::Any)),
            ""
        );
    }

    #[test]
    fn test_file_trigger_worker_new_starts_empty() {
        let w = FileTriggerWorker::new();
        assert!(w.handle.lock().is_none());
        assert!(w.watchers.lock().is_empty());
    }

    #[test]
    fn test_file_trigger_worker_stop_idempotent() {
        let w = FileTriggerWorker::new();
        w.stop(); // 不应 panic
        w.stop(); // 重复停止也不 panic
    }
}
