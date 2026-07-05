//! T-S6-A-02: 电源管理 — 监听系统睡眠/唤醒事件,暂停/恢复 LLM 与蜂群任务。
//!
//! ## 设计说明
//!
//! Tauri 2.x 在 Windows 上没有直接的睡眠/唤醒事件 API。本模块采用
//! 跨平台兼容的启发式方法:每 10s 检测一次系统时间跳变,如果两次
//! tick 之间实际经过的时间远大于 10s(> 60s),说明系统刚从睡眠中
//! 唤醒(睡眠期间我们的定时器不会触发)。检测到唤醒后:
//!
//! 1. 进入 Paused 状态,通知前端。
//! 2. LLM 调用与蜂群任务在调用前应检查 `is_active()`,暂停期间跳过。
//! 3. 唤醒后调用 `resume()`,若暂停时长 > 5 分钟,emit 一个
//!    `nebula://trigger-reflection` 事件,供反思引擎补跑。
//!
//! ## 补跑反思集成点
//!
//! 完整的补跑反思逻辑需要在 `SwarmOrchestrator` 或 `memory::reflect`
//! 模块中监听 `nebula://trigger-reflection` 事件并执行。本模块
//! 仅提供事件触发点,不直接调用反思引擎,保持模块解耦。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};
use tracing::info;

/// 触发补跑反思的暂停时长阈值(秒)。暂停超过 5 分钟才补跑。
const REFLECT_TRIGGER_THRESHOLD_SECS: i64 = 300;

/// 后台监测线程的 tick 间隔。
const TICK_INTERVAL_SECS: u64 = 10;

/// 判定系统睡眠的时间跳变阈值(秒)。两次 tick 之间实际经过
/// 的时间超过此值,即认为是睡眠唤醒。
const SLEEP_GAP_THRESHOLD_SECS: u64 = 60;

/// 电源状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// 活跃(正常运行)
    Active,
    /// 已暂停(系统睡眠或用户触发)
    Paused,
}

/// 电源管理器 — 跟踪系统睡眠/唤醒,协调 LLM/蜂群任务的暂停与恢复。
///
/// 设计上 `PowerManager` 通过 `Arc<PowerManager>` 注册到 Tauri 的
/// managed state,后台监测线程持有内部 `Arc` 的克隆,因此
/// `start` 取 `&self` 而非 `self`。
pub struct PowerManager {
    /// true = active, false = paused。使用 AtomicBool 方便跨线程读取。
    state: Arc<AtomicBool>,
    /// 上次 tick 的时刻,用于检测时间跳变。
    last_tick: Arc<Mutex<Instant>>,
    /// Tauri 应用句柄,用于 emit 事件给前端。
    app: AppHandle,
    /// 暂停发生的时间戳(unix secs),用于唤醒后计算暂停时长。
    paused_at: Arc<Mutex<Option<i64>>>,
}

impl PowerManager {
    /// 创建一个新的电源管理器,初始状态为 Active。
    pub fn new(app: AppHandle) -> Self {
        Self {
            state: Arc::new(AtomicBool::new(true)),
            last_tick: Arc::new(Mutex::new(Instant::now())),
            app,
            paused_at: Arc::new(Mutex::new(None)),
        }
    }

    /// 启动后台监测线程,每 `TICK_INTERVAL_SECS` 秒检测一次时间跳变。
    ///
    /// 如果检测到 > `SLEEP_GAP_THRESHOLD_SECS` 的时间跳跃,判定为
    /// 系统刚从睡眠唤醒,自动进入 Paused 状态。
    ///
    /// 取 `&self` 以便在 `Arc<PowerManager>` 上调用:
    /// `power_mgr.clone().start();`
    pub fn start(&self) {
        let state = self.state.clone();
        let last_tick = self.last_tick.clone();
        let paused_at = self.paused_at.clone();
        let app = self.app.clone();

        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(TICK_INTERVAL_SECS));
                let now = Instant::now();
                let mut tick = last_tick.lock();
                let elapsed = now.duration_since(*tick);
                *tick = now;

                // 如果 10s 间隔实际经过了 > 60s,说明系统刚从睡眠唤醒
                if elapsed.as_secs() > SLEEP_GAP_THRESHOLD_SECS {
                    let was_active = state.load(Ordering::SeqCst);
                    if was_active {
                        info!(
                            target: "nebula.power",
                            gap_secs = elapsed.as_secs(),
                            "system wake detected, entering paused state"
                        );
                        state.store(false, Ordering::SeqCst);
                        *paused_at.lock() = Some(
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0),
                        );
                        // emit 事件通知前端
                        let _ = app.emit("nebula://power-state", "paused");
                    }
                }
            }
        });
    }

    /// 主动暂停(用户触发或系统事件)。
    ///
    /// 调用后 `is_active()` 返回 false,LLM/蜂群任务应当跳过。
    pub fn pause(&self) {
        self.state.store(false, Ordering::SeqCst);
        *self.paused_at.lock() = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        );
        info!(target: "nebula.power", "power state -> paused");
        let _ = self.app.emit("nebula://power-state", "paused");
    }

    /// 恢复(唤醒后调用)。
    ///
    /// 返回暂停持续时间(秒)。如果暂停时长 > `REFLECT_TRIGGER_THRESHOLD_SECS`,
    /// 会 emit `nebula://trigger-reflection` 事件,供反思引擎补跑。
    ///
    /// 若调用时本就处于 Active 状态,返回 `None`。
    pub fn resume(&self) -> Option<i64> {
        let was_paused = !self.state.load(Ordering::SeqCst);
        if !was_paused {
            return None;
        }
        self.state.store(true, Ordering::SeqCst);

        // 计算暂停时长,并清空 paused_at(take 同时取出值并置 None)
        let paused_duration: Option<i64> = {
            let mut guard = self.paused_at.lock();
            guard.take().and_then(|paused_at| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if paused_at > 0 {
                    Some(now - paused_at)
                } else {
                    None
                }
            })
        };

        info!(
            target: "nebula.power",
            paused_secs = ?paused_duration,
            "power state -> active"
        );
        let _ = self.app.emit("nebula://power-state", "active");

        // 补跑反思:暂停时长超过阈值时,触发反思事件。
        // 注意:这里只 emit 事件,不直接调用反思引擎,保持模块解耦。
        // 完整的补跑逻辑应在 SwarmOrchestrator 或 reflect 模块中
        // 监听此事件并执行。
        if let Some(secs) = paused_duration {
            if secs >= REFLECT_TRIGGER_THRESHOLD_SECS {
                info!(
                    target: "nebula.power",
                    paused_secs = secs,
                    threshold = REFLECT_TRIGGER_THRESHOLD_SECS,
                    "pause exceeded threshold, triggering reflection catch-up"
                );
                let _ = self.app.emit("nebula://trigger-reflection", secs);
            }
        }

        paused_duration
    }

    /// 查询当前是否活跃(LLM/蜂群可运行)。
    ///
    /// LLM 网关与蜂群编排器在发起调用前应检查此方法,
    /// 暂停期间应当跳过或排队。
    pub fn is_active(&self) -> bool {
        self.state.load(Ordering::SeqCst)
    }

    /// 查询当前是否暂停。
    pub fn is_paused(&self) -> bool {
        !self.is_active()
    }

    /// 获取当前电源状态。
    pub fn state(&self) -> PowerState {
        if self.is_active() {
            PowerState::Active
        } else {
            PowerState::Paused
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------
//
// 这些命令需要在 lib.rs 的 `invoke_handler` 宏中注册后才能被前端调用。
// 注册方式:在 `tauri::generate_handler![...]` 列表中加入
//   os::power::power_state,
//   os::power::power_pause,
//   os::power::power_resume,
//
// 当前未注册,用 `#[allow(dead_code)]` 抑制告警。

/// 查询当前电源状态。返回 "active" 或 "paused"。
#[tauri::command]
#[allow(dead_code)]
pub async fn power_state(
    state: tauri::State<'_, Arc<PowerManager>>,
) -> Result<String, String> {
    Ok(if state.is_active() {
        "active".to_string()
    } else {
        "paused".to_string()
    })
}

/// 主动暂停电源管理器(进入 Paused 状态)。
#[tauri::command]
#[allow(dead_code)]
pub async fn power_pause(
    state: tauri::State<'_, Arc<PowerManager>>,
) -> Result<(), String> {
    state.pause();
    Ok(())
}

/// 恢复电源管理器(进入 Active 状态)。
///
/// 返回暂停时长(秒);若调用时本就 Active,返回 `None`。
/// 暂停时长超过 5 分钟时会触发 `nebula://trigger-reflection` 事件。
#[tauri::command]
#[allow(dead_code)]
pub async fn power_resume(
    state: tauri::State<'_, Arc<PowerManager>>,
) -> Result<Option<i64>, String> {
    Ok(state.resume())
}
