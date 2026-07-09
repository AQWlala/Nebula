//! T-E-C-15: 语音交互 Tauri 命令。
//!
//! 6 个命令,与 [`VoiceEngine`] 配套:
//! - `voice_status()`         — 查询引擎状态(后端/运行中标志)
//! - `voice_listen()`         — 一次性监听,返回转录文本
//! - `voice_speak(text)`      — 朗读文本
//! - `voice_wake_start()`     — 启动唤醒词检测(事件经 Tauri event 推送前端)
//! - `voice_wake_stop()`      — 停止唤醒词检测
//! - `voice_wake_status()`    — 查询是否正在检测唤醒词
//!
//! ## 注册说明(⚠ 需在 tauri_setup.rs / commands/mod.rs 注册)
//! 本模块受并发约束**不修改** `commands/mod.rs` 与 `tauri_setup.rs`。
//! 集成时需:
//! 1. 在 `commands/mod.rs` 添加 `pub mod voice;` 与 `pub use voice::*;`。
//! 2. 在 `bootstrap` 阶段 `app.manage(voice::commands::VoiceState::from_config())`。
//! 3. 在 `tauri_setup.rs` 的 `generate_handler!` 追加:
//!    `commands::voice_status, commands::voice_listen, commands::voice_speak,
//!     commands::voice_wake_start, commands::voice_wake_stop,
//!     commands::voice_wake_status`
//!
//! 命令通过 `State<'_, VoiceState>` 取得 [`VoiceEngine`] 实例。

use tauri::{AppHandle, Emitter, State};
use tracing::{instrument, warn};

use crate::commands::error::CommandError;
use crate::voice::{DefaultVoiceEngine, DynVoiceEngine, VoiceConfig, VoiceEngineStatus};

/// 唤醒事件经 Tauri event 推送前端的事件名。
pub const WAKE_EVENT_NAME: &str = "nebula://voice-wake";

/// 托管在 Tauri managed-state 的语音引擎包装。
///
/// 由 bootstrap 阶段 `app.manage(VoiceState::from_config())` 注册。
#[derive(Clone)]
pub struct VoiceState {
    engine: DynVoiceEngine,
}

impl VoiceState {
    /// 从已有引擎构造。
    pub fn new(engine: DynVoiceEngine) -> Self {
        Self { engine }
    }

    /// 从 [`VoiceConfig`] 构造(按配置选择后端)。
    pub fn from_config(config: VoiceConfig) -> Self {
        Self::new(DefaultVoiceEngine::from_config(config))
    }

    /// 默认 noop 构造(无硬件/CI 环境)。
    pub fn noop() -> Self {
        Self::new(DefaultVoiceEngine::noop())
    }

    /// 内部引擎引用。
    pub fn engine(&self) -> &DynVoiceEngine {
        &self.engine
    }
}

/// 把 [`crate::voice::VoiceError`] 转为 [`CommandError`]。
fn map_voice_err(cmd: &str, e: crate::voice::VoiceError) -> CommandError {
    CommandError::internal(cmd, &anyhow::anyhow!(e))
}

/// 查询语音引擎状态。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "voice_status"))]
pub async fn voice_status(state: State<'_, VoiceState>) -> Result<VoiceEngineStatus, CommandError> {
    let status = state.engine.status().await;
    Ok(status)
}

/// 一次性监听:启动麦克风 → STT 转写 → 返回转录文本。
///
/// 阻塞至静音超时或流结束。前端可用于"按住说话"或唤醒后的单轮输入。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "voice_listen"))]
pub async fn voice_listen(state: State<'_, VoiceState>) -> Result<String, CommandError> {
    let transcription = state
        .engine
        .listen()
        .await
        .map_err(|e| map_voice_err("voice_listen", e))?;
    Ok(transcription.full_text)
}

/// 朗读文本:合成语音并播放。返回合成帧数。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "voice_speak"))]
pub async fn voice_speak(
    state: State<'_, VoiceState>,
    text: String,
) -> Result<usize, CommandError> {
    let count = state
        .engine
        .speak(&text)
        .await
        .map_err(|e| map_voice_err("voice_speak", e))?;
    Ok(count)
}

/// 启动唤醒词检测。后台任务持续监听,检测到唤醒词时通过
/// `nebula://voice-wake` Tauri event 推送 [`WakeWordEvent`] 给前端。
///
/// 返回是否成功启动(已在运行则报错)。
#[tauri::command]
#[instrument(skip(state, app), fields(otel.kind = "voice_wake_start"))]
pub async fn voice_wake_start(
    state: State<'_, VoiceState>,
    app: AppHandle,
) -> Result<(), CommandError> {
    let event_rx = state
        .engine
        .wake_start()
        .await
        .map_err(|e| map_voice_err("voice_wake_start", e))?;

    // spawn 后台任务:转发唤醒事件到前端。
    let app_clone = app.clone();
    tokio::spawn(async move {
        let mut rx = event_rx;
        while let Some(event) = rx.recv().await {
            if let Err(e) = app_clone.emit(WAKE_EVENT_NAME, &event) {
                warn!(target: "nebula::voice::commands", error = %e, "emit wake event failed");
            }
        }
    });
    Ok(())
}

/// 停止唤醒词检测。幂等。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "voice_wake_stop"))]
pub async fn voice_wake_stop(state: State<'_, VoiceState>) -> Result<(), CommandError> {
    state
        .engine
        .wake_stop()
        .await
        .map_err(|e| map_voice_err("voice_wake_stop", e))?;
    Ok(())
}

/// 查询是否正在检测唤醒词。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "voice_wake_status"))]
pub async fn voice_wake_status(state: State<'_, VoiceState>) -> Result<bool, CommandError> {
    Ok(state.engine.is_wake_detecting().await)
}

/// 仅供 commands 模块内部使用的 Serialize 占位(确保 WakeWordEvent 可序列化)。
/// 实际 WakeWordEvent 已 derive Serialize,此函数避免未使用 import 警告。
#[allow(dead_code)]
fn _ensure_wake_event_serializable(e: &crate::voice::WakeWordEvent) -> Vec<u8> {
    serde_json::to_vec(e).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::voice::{
        AudioFormat, NoopAudioCapture, NoopAudioSink, NoopSttEngine, NoopTtsEngine,
    };

    /// 构造启用的 VoiceState。
    fn enabled_state() -> VoiceState {
        let config = VoiceConfig {
            enabled: true,
            stt_backend: "noop".into(),
            tts_backend: "noop".into(),
            wake_backend: "noop".into(),
            audio_backend: "noop".into(),
            wake_auto_listen: false,
            capture_buffer: 8,
        };
        let format = AudioFormat::WHISPER_16K_MONO;
        let engine: DynVoiceEngine = Arc::new(DefaultVoiceEngine::new(
            config,
            Arc::new(NoopAudioCapture::new(format)),
            Arc::new(NoopSttEngine::new("测试转录")),
            Arc::new(NoopTtsEngine::new(format)),
            Arc::new(crate::voice::wake::NoopWakeWordDetector::new("hey nebula")),
            Arc::new(NoopAudioSink::new()),
        ));
        VoiceState::new(engine)
    }

    #[test]
    fn test_voice_state_noop_construct() {
        let s = VoiceState::noop();
        assert_eq!(s.engine().kind(), "default");
    }

    #[test]
    fn test_voice_state_from_config() {
        let cfg = VoiceConfig {
            enabled: true,
            stt_backend: "noop".into(),
            tts_backend: "noop".into(),
            wake_backend: "noop".into(),
            audio_backend: "noop".into(),
            wake_auto_listen: false,
            capture_buffer: 8,
        };
        let s = VoiceState::from_config(cfg);
        assert_eq!(s.engine().kind(), "default");
    }

    #[tokio::test]
    async fn test_engine_listen_via_state() {
        let s = enabled_state();
        let t = s.engine().listen().await.expect("listen");
        assert_eq!(t.full_text, "测试转录");
    }

    #[tokio::test]
    async fn test_engine_speak_via_state() {
        let s = enabled_state();
        let count = s.engine().speak("hi").await.expect("speak");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_engine_wake_lifecycle_via_state() {
        let s = enabled_state();
        assert!(!s.engine().is_wake_detecting().await);
        let _rx = s.engine().wake_start().await.expect("start");
        assert!(s.engine().is_wake_detecting().await);
        s.engine().wake_stop().await.expect("stop");
        assert!(!s.engine().is_wake_detecting().await);
    }
}
