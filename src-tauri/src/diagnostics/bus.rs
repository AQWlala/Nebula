//! T-E-S-27: DiagnosticsBus — 独立 broadcast 通道,与 AgentBus 解耦。
//!
//! 用 `OnceLock` 全局单例(类比 `metrics::global()`),让
//! `init_tracing` 中的 `DiagnosticsLayer` 与 `AppState::bootstrap`
//! 都能拿到同一个实例,避免循环依赖:
//!
//! * `init_tracing` 在 `AppState::bootstrap` 之前调用,需要先拿到
//!   bus 才能 install layer;
//! * `AppState::bootstrap` 之后,前端 Tauri 命令需要从
//!   `AppState.diagnostics` 拿到同一个 bus 实例做 subscribe。
//!
//! 用 `OnceLock` 让两者都通过 `global()` 拿同一实例即可解决。

use std::sync::{Arc, OnceLock};

use tokio::sync::broadcast;
use tracing::debug;

use super::events::DiagnosticEvent;

/// 默认 broadcast 容量(spec: 512)。
pub const DEFAULT_CAPACITY: usize = 512;

/// T-E-S-27: 诊断事件总线。
///
/// 独立于 `AgentBus` 的 `broadcast::Sender<DiagnosticEvent>`,
/// 专门承载可信诊断信息。`emit` 自动维护单调递增的 `seq` 序号。
pub struct DiagnosticsBus {
    tx: broadcast::Sender<DiagnosticEvent>,
    seq: std::sync::atomic::AtomicU64,
    capacity: usize,
}

impl DiagnosticsBus {
    /// 用默认容量 512 构造。
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// 用指定容量构造。
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        let (tx, _) = broadcast::channel(cap);
        Self {
            tx,
            seq: std::sync::atomic::AtomicU64::new(0),
            capacity: cap,
        }
    }

    /// 返回通道容量。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 返回 broadcast sender 的克隆(供 spawn 的任务 emit 事件)。
    pub fn sender(&self) -> broadcast::Sender<DiagnosticEvent> {
        self.tx.clone()
    }

    /// 订阅诊断事件流。返回的 Receiver 可在 Tauri 命令中循环
    /// `recv().await` 并通过 `tauri::ipc::Channel::send` 推送给前端。
    pub fn subscribe(&self) -> broadcast::Receiver<DiagnosticEvent> {
        self.tx.subscribe()
    }

    /// 发出一条诊断事件。`seq` 字段会被自动填充为单调递增值。
    ///
    /// 没有订阅者不算错误:诊断仍会被记录到 tracing layer 的
    /// `nebula.diagnostic` target 中,只是当前没有前端在监听。
    pub fn emit(&self, mut event: DiagnosticEvent) {
        let seq = self
            .seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        // 把 seq 写入事件。所有变体都有 seq 字段。
        match &mut event {
            DiagnosticEvent::L4Deny { seq: s, .. }
            | DiagnosticEvent::AclRejected { seq: s, .. }
            | DiagnosticEvent::InjectionGuardHit { seq: s, .. }
            | DiagnosticEvent::SidecarCrash { seq: s, .. }
            | DiagnosticEvent::TracingWarn { seq: s, .. }
            | DiagnosticEvent::Dropped { seq: s, .. } => {
                *s = seq;
            }
        }
        if self.tx.send(event).is_err() {
            debug!(
                target: "nebula.diagnostic",
                "no active diagnostics subscribers"
            );
        }
    }

    /// 发出一条 `Dropped` 元事件,表示消费者 Lagged 丢失了 `count` 条。
    /// 用于 `subscribe_diagnostics` 命令在 `RecvError::Lagged` 时调用。
    pub fn emit_dropped(&self, count: u64) {
        self.emit(DiagnosticEvent::Dropped { count, seq: 0 });
    }
}

impl Default for DiagnosticsBus {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级全局单例。`init_tracing` 与 `AppState::bootstrap` 共用。
///
/// 返回 `&'static Arc<DiagnosticsBus>` 而非 `&'static DiagnosticsBus`,
/// 这样 `AppState` 可以通过 `Arc::clone(global())` 持有 `Arc<DiagnosticsBus>`
/// (与 AppState 其他字段风格一致),而 `DiagnosticsLayer` 可以通过
/// `&**global()` 拿到 `&'static DiagnosticsBus`。
///
/// 容量从 `NEBULA_DIAGNOSTICS_BUFFER_CAPACITY` 环境变量读取
/// (默认 512),与 `AppConfig::from_env` 读取同一环境变量保持一致。
pub fn global() -> &'static Arc<DiagnosticsBus> {
    static GLOBAL: OnceLock<Arc<DiagnosticsBus>> = OnceLock::new();
    GLOBAL.get_or_init(|| {
        let cap = std::env::var("NEBULA_DIAGNOSTICS_BUFFER_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_CAPACITY);
        Arc::new(DiagnosticsBus::with_capacity(cap))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[tokio::test]
    async fn emit_and_subscribe_receives_event() {
        let bus = DiagnosticsBus::with_capacity(8);
        let mut rx = bus.subscribe();
        bus.emit(DiagnosticEvent::L4Deny {
            memory_id: "m1".into(),
            reason: "conflict".into(),
            seq: 0,
        });
        let evt = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("recv timed out")
            .expect("channel closed");
        assert_eq!(evt.seq(), 1);
        match evt {
            DiagnosticEvent::L4Deny { memory_id, .. } => {
                assert_eq!(memory_id, "m1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn seq_increases_monotonically() {
        let bus = DiagnosticsBus::with_capacity(8);
        let mut rx = bus.subscribe();
        for _ in 0..3 {
            bus.emit(DiagnosticEvent::TracingWarn {
                target: "t".into(),
                message: "m".into(),
                seq: 0,
            });
        }
        let mut seqs = Vec::new();
        for _ in 0..3 {
            let evt = rx.recv().await.expect("recv");
            seqs.push(evt.seq());
        }
        assert_eq!(seqs, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn emit_with_no_subscribers_is_ok() {
        let bus = DiagnosticsBus::with_capacity(4);
        // 没有订阅者,emit 不应 panic。
        bus.emit(DiagnosticEvent::AclRejected {
            user: "u".into(),
            resource: "r".into(),
            seq: 0,
        });
    }

    #[tokio::test]
    async fn emit_dropped_uses_dropped_variant() {
        let bus = DiagnosticsBus::with_capacity(4);
        let mut rx = bus.subscribe();
        bus.emit_dropped(7);
        let evt = rx.recv().await.expect("recv");
        match evt {
            DiagnosticEvent::Dropped { count, seq } => {
                assert_eq!(count, 7);
                assert_eq!(seq, 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn global_returns_same_instance() {
        let a = global() as *const _;
        let b = global() as *const _;
        assert_eq!(a, b);
    }

    #[test]
    fn capacity_clamped_to_one() {
        let bus = DiagnosticsBus::with_capacity(0);
        assert_eq!(bus.capacity(), 1);
    }
}
