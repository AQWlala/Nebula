//! T-E-C-15: 语音交互引擎(Voice Interaction Engine)。
//!
//! 统一 STT(语音转文字)+ TTS(文字转语音)+ 唤醒词检测 + 音频捕获/播放
//! 四大子系统,提供 [`VoiceEngine`] trait 与 [`DefaultVoiceEngine`] 默认实现。
//!
//! ## 架构
//! ```text
//!   麦克风 ──► AudioCapture ──► WakeWordDetector ──► (唤醒事件)
//!                                    │
//!                                    ▼ (唤醒后)
//!   麦克风 ──► AudioCapture ──► SttEngine ──► 用户文本
//!                                               │
//!                                               ▼
//!                                   LlmBridge (流式) ──► 回复 token
//!                                                          │
//!                                                          ▼
//!                                            TtsEngine ──► AudioSink ──► 扬声器
//! ```
//!
//! ## 模块
//! * [`audio`]   — `AudioCapture` / `AudioSink` trait(cpal 接口,不引入依赖)。
//! * [`stt`]     — `SttEngine` trait + Whisper.cpp / Ollama(stub) / Noop 后端。
//! * [`tts`]     — `TtsEngine` trait + 本地 TTS / Noop 后端。
//! * [`wake`]    — `WakeWordDetector` trait + Porcupine / Energy VAD / Noop 后端。
//! * [`pipeline`]— 流式管道 STT → LLM → TTS 编排。
//! * [`commands`]— Tauri 命令(voice_listen / voice_speak / voice_wake_* / voice_status)。
//!
//! ## 依赖说明
//! 本模块**不引入** cpal / whisper.cpp / piper / porcupine 等原生依赖,
//! 仅提供 trait 接口与框架后端。实际后端由 feature gate 启用:
//! - `voice-cpal`:cpal 音频捕获/播放。
//! - `voice-whisper`:whisper.cpp 子进程 STT。
//! - `voice-tts`:本地 TTS 子进程。
//! - `voice-porcupine`:Porcupine 唤醒词。
//!
//! ## 环境变量(前缀 `NEBULA_`)
//! - `NEBULA_VOICE_ENABLED`:总开关(默认 false,需用户显式开启)。
//! - `NEBULA_VOICE_STT_BACKEND`:STT 后端(whisper-cpp / ollama / noop,默认 noop)。
//! - `NEBULA_VOICE_TTS_BACKEND`:TTS 后端(local-tts / noop,默认 noop)。
//! - `NEBULA_VOICE_WAKE_BACKEND`:唤醒后端(porcupine / energy / noop,默认 noop)。
//! - `NEBULA_VOICE_AUDIO_BACKEND`:音频后端(cpal / noop,默认 noop)。
//! - 其余后端特定变量见各后端配置结构。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::voice::audio::{
    AudioError, AudioFormat, DynAudioCapture, DynAudioSink, NoopAudioCapture, NoopAudioSink,
};
use crate::voice::stt::{DynSttEngine, NoopSttEngine, SttError};
use crate::voice::tts::{DynTtsEngine, NoopTtsEngine, TtsError};
// 注:WakeWordEvent 仅由下方 line 71 的 `pub use` 引入作用域,
// 此处不再重复 import(否则 E0252: name defined multiple times)。
// pub use 同时提供内部可见性与外部 re-export,满足 line 240/415 的内部使用。
use crate::voice::wake::{DynWakeWordDetector, WakeError};

pub mod audio;
pub mod commands;
pub mod pipeline;
pub mod stt;
pub mod tts;
pub mod wake;

// 公共 re-export,便于外部按 `voice::Transcription` 等短路径访问。
pub use audio::{AudioCapture, AudioFrame, AudioSink};
pub use pipeline::{ConversationTurn, DynLlmBridge, LlmBridge, PipelineConfig, VoicePipeline};
pub use stt::{SttEngine, Transcription, TranscriptionEvent, TranscriptionSegment};
pub use tts::{SynthesisEvent, SynthesisRequest, TtsEngine};
pub use wake::{WakeWordDetector, WakeWordEvent};

// ---------------------------------------------------------------------------
// VoiceError:顶层错误。
// ---------------------------------------------------------------------------

/// 语音引擎顶层错误。聚合各子系统错误为统一枚举。
#[derive(Debug, Error)]
pub enum VoiceError {
    /// 音频捕获/播放错误。
    #[error("audio: {0}")]
    Audio(String),
    /// STT 转录错误。
    #[error("stt: {0}")]
    Stt(String),
    /// TTS 合成错误。
    #[error("tts: {0}")]
    Tts(String),
    /// 唤醒词检测错误。
    #[error("wake: {0}")]
    Wake(String),
    /// 管道编排错误。
    #[error("pipeline: {0}")]
    Pipeline(String),
    /// 配置错误(后端未启用 / 缺失资源)。
    #[error("config: {0}")]
    Config(String),
    /// 引擎未启用(`NEBULA_VOICE_ENABLED=0`)。
    #[error("voice engine disabled")]
    Disabled,
    /// 已在运行(重复 start)。
    #[error("voice engine already running")]
    AlreadyRunning,
    /// 未运行。
    #[error("voice engine not running")]
    NotRunning,
}

impl From<AudioError> for VoiceError {
    fn from(e: AudioError) -> Self {
        VoiceError::Audio(e.to_string())
    }
}

impl From<SttError> for VoiceError {
    fn from(e: SttError) -> Self {
        VoiceError::Stt(e.to_string())
    }
}

impl From<TtsError> for VoiceError {
    fn from(e: TtsError) -> Self {
        VoiceError::Tts(e.to_string())
    }
}

impl From<WakeError> for VoiceError {
    fn from(e: WakeError) -> Self {
        VoiceError::Wake(e.to_string())
    }
}

/// 语音引擎结果别名。
pub type VoiceResult<T> = Result<T, VoiceError>;

// ---------------------------------------------------------------------------
// VoiceConfig:从环境变量加载。
// ---------------------------------------------------------------------------

/// 语音引擎配置(从 `NEBULA_VOICE_*` 环境变量加载)。
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    /// 总开关。env: `NEBULA_VOICE_ENABLED`(默认 false)。
    pub enabled: bool,
    /// STT 后端类型(whisper-cpp / ollama / noop)。env: `NEBULA_VOICE_STT_BACKEND`。
    pub stt_backend: String,
    /// TTS 后端类型(local-tts / noop)。env: `NEBULA_VOICE_TTS_BACKEND`。
    pub tts_backend: String,
    /// 唤醒词后端类型(porcupine / energy / noop)。env: `NEBULA_VOICE_WAKE_BACKEND`。
    pub wake_backend: String,
    /// 音频后端类型(cpal / noop)。env: `NEBULA_VOICE_AUDIO_BACKEND`。
    pub audio_backend: String,
    /// 唤醒词触发后自动开始监听(否则需手动调 listen)。env: `NEBULA_VOICE_WAKE_AUTO_LISTEN`。
    pub wake_auto_listen: bool,
    /// 麦克风采集 buffer 容量(帧)。env: `NEBULA_VOICE_CAPTURE_BUFFER`。
    pub capture_buffer: usize,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl VoiceConfig {
    /// 从环境变量加载。
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("NEBULA_VOICE_ENABLED")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            stt_backend: std::env::var("NEBULA_VOICE_STT_BACKEND")
                .unwrap_or_else(|_| "noop".to_string()),
            tts_backend: std::env::var("NEBULA_VOICE_TTS_BACKEND")
                .unwrap_or_else(|_| "noop".to_string()),
            wake_backend: std::env::var("NEBULA_VOICE_WAKE_BACKEND")
                .unwrap_or_else(|_| "noop".to_string()),
            audio_backend: std::env::var("NEBULA_VOICE_AUDIO_BACKEND")
                .unwrap_or_else(|_| "noop".to_string()),
            wake_auto_listen: std::env::var("NEBULA_VOICE_WAKE_AUTO_LISTEN")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            capture_buffer: std::env::var("NEBULA_VOICE_CAPTURE_BUFFER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(64),
        }
    }
}

// ---------------------------------------------------------------------------
// VoiceEngineStatus:状态查询。
// ---------------------------------------------------------------------------

/// 语音引擎运行状态(供前端展示)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceEngineStatus {
    /// 引擎是否启用。
    pub enabled: bool,
    /// STT 后端类型。
    pub stt_backend: String,
    /// TTS 后端类型。
    pub tts_backend: String,
    /// 唤醒词后端类型。
    pub wake_backend: String,
    /// 音频后端类型。
    pub audio_backend: String,
    /// 是否正在监听(捕获 + STT)。
    pub listening: bool,
    /// 是否正在朗读(TTS 播放中)。
    pub speaking: bool,
    /// 是否正在检测唤醒词。
    pub wake_detecting: bool,
}

// ---------------------------------------------------------------------------
// VoiceEngine trait:listen / speak / wake。
// ---------------------------------------------------------------------------

/// 语音交互引擎 trait:统一 listen(听)/ speak(说)/ wake(唤醒)三大能力。
///
/// 实现方([`DefaultVoiceEngine`])组合各子引擎 trait 对象。
#[async_trait]
pub trait VoiceEngine: Send + Sync {
    /// 引擎标识。
    fn kind(&self) -> &'static str;

    /// 一次性监听:启动麦克风捕获 → STT 转写 → 返回用户文本。
    /// 流结束(静音超时或 capture stop)后返回。
    async fn listen(&self) -> VoiceResult<Transcription>;

    /// 朗读:把文本合成语音并播放。
    /// 返回合成帧数。
    async fn speak(&self, text: &str) -> VoiceResult<usize>;

    /// 启动唤醒词检测。后台任务持续监听麦克风,检测到唤醒词时推送事件。
    /// 返回事件接收端(调用方 poll 以触发后续 listen)。
    async fn wake_start(&self) -> VoiceResult<mpsc::Receiver<WakeWordEvent>>;

    /// 停止唤醒词检测。幂等。
    async fn wake_stop(&self) -> VoiceResult<()>;

    /// 是否正在检测唤醒词。
    async fn is_wake_detecting(&self) -> bool;

    /// 查询引擎状态。
    async fn status(&self) -> VoiceEngineStatus;
}

/// 动态分发别名。
pub type DynVoiceEngine = Arc<dyn VoiceEngine>;

// ---------------------------------------------------------------------------
// DefaultVoiceEngine:组合实现。
// ---------------------------------------------------------------------------

/// 默认语音引擎:组合 AudioCapture + STT + TTS + WakeWordDetector + AudioSink。
///
/// 各组件以 trait 对象注入,运行时按 [`VoiceConfig`] 选择后端。
/// 默认构造(`DefaultVoiceEngine::noop()`)全部使用 Noop 后端,用于无硬件环境/测试。
pub struct DefaultVoiceEngine {
    config: VoiceConfig,
    capture: DynAudioCapture,
    stt: DynSttEngine,
    tts: DynTtsEngine,
    wake: DynWakeWordDetector,
    sink: DynAudioSink,
    /// 唤醒检测运行状态(跨 await,用 tokio Mutex)。
    wake_running: tokio::sync::Mutex<bool>,
}

impl DefaultVoiceEngine {
    /// 从各组件构造。
    pub fn new(
        config: VoiceConfig,
        capture: DynAudioCapture,
        stt: DynSttEngine,
        tts: DynTtsEngine,
        wake: DynWakeWordDetector,
        sink: DynAudioSink,
    ) -> Self {
        Self {
            config,
            capture,
            stt,
            tts,
            wake,
            sink,
            wake_running: tokio::sync::Mutex::new(false),
        }
    }

    /// 全 Noop 构造(无硬件/测试用)。返回 Arc<dyn VoiceEngine>。
    pub fn noop() -> DynVoiceEngine {
        let config = VoiceConfig::from_env();
        let format = AudioFormat::WHISPER_16K_MONO;
        Arc::new(Self::new(
            config,
            Arc::new(NoopAudioCapture::new(format)),
            Arc::new(NoopSttEngine::new("")),
            Arc::new(NoopTtsEngine::new(format)),
            Arc::new(crate::voice::wake::NoopWakeWordDetector::new("hey nebula")),
            Arc::new(NoopAudioSink::new()),
        ))
    }

    /// 按 [`VoiceConfig`] 选择后端构造。
    ///
    /// **框架实现**:未启用的后端 feature 回退到 Noop,确保无原生依赖时仍可构造。
    pub fn from_config(config: VoiceConfig) -> DynVoiceEngine {
        let format = AudioFormat::WHISPER_16K_MONO;

        // STT 后端选择。
        let stt: DynSttEngine = match config.stt_backend.as_str() {
            "whisper-cpp" => Arc::new(stt::WhisperCppBackend::from_env()),
            "ollama" => Arc::new(stt::OllamaSttBackend::new(
                std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
            )),
            _ => Arc::new(NoopSttEngine::empty()),
        };

        // TTS 后端选择。
        let tts: DynTtsEngine = match config.tts_backend.as_str() {
            "local-tts" => Arc::new(tts::LocalTtsBackend::from_env()),
            _ => Arc::new(NoopTtsEngine::new(format)),
        };

        // 唤醒词后端选择。
        let wake: DynWakeWordDetector = match config.wake_backend.as_str() {
            "porcupine" => Arc::new(wake::PorcupineBackend::from_env()),
            "energy" => Arc::new(wake::EnergyDetectorBackend::with_defaults()),
            _ => Arc::new(wake::NoopWakeWordDetector::new("hey nebula")),
        };

        // 音频后端:cpal 未集成,统一 Noop。
        let capture: DynAudioCapture = match config.audio_backend.as_str() {
            "cpal" => {
                warn!(target: "nebula::voice", "cpal audio backend not yet integrated, falling back to noop");
                Arc::new(NoopAudioCapture::new(format))
            }
            _ => Arc::new(NoopAudioCapture::new(format)),
        };

        let sink: DynAudioSink = Arc::new(NoopAudioSink::new());

        Arc::new(Self::new(config, capture, stt, tts, wake, sink))
    }

    /// 内部:启动捕获并桥接到 STT 流式输入。
    /// 返回 (frame_rx, capture_guard):capture_guard drop 时停止捕获。
    async fn start_capture(&self) -> VoiceResult<(mpsc::Receiver<AudioFrame>, CaptureGuard)> {
        let rx = self.capture.start(self.config.capture_buffer).await?;
        Ok((
            rx,
            CaptureGuard {
                capture: Arc::clone(&self.capture),
            },
        ))
    }
}

/// 捕获守卫:drop 时停止捕获,确保资源释放。
struct CaptureGuard {
    capture: DynAudioCapture,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        // 同步 drop 中无法 await,用 tokio::spawn 触发 stop。
        let cap = Arc::clone(&self.capture);
        tokio::spawn(async move {
            let _ = cap.stop().await;
        });
    }
}

#[async_trait]
impl VoiceEngine for DefaultVoiceEngine {
    fn kind(&self) -> &'static str {
        "default"
    }

    async fn listen(&self) -> VoiceResult<Transcription> {
        if !self.config.enabled {
            return Err(VoiceError::Disabled);
        }
        // 启动捕获,守卫确保结束时停止。
        let (frame_rx, _guard) = self.start_capture().await?;
        // 用 pipeline 的 transcribe_only 聚合 STT 流。
        let pipe = VoicePipeline::new(
            Arc::clone(&self.stt),
            Arc::clone(&self.tts),
            None,
            None,
            PipelineConfig::default(),
        );
        pipe.transcribe_only(frame_rx).await
    }

    async fn speak(&self, text: &str) -> VoiceResult<usize> {
        if !self.config.enabled {
            return Err(VoiceError::Disabled);
        }
        let req = SynthesisRequest::new(text);
        let frames = self.tts.synthesize(&req).await?;
        let count = frames.len();
        self.sink.play(&frames).await?;
        Ok(count)
    }

    async fn wake_start(&self) -> VoiceResult<mpsc::Receiver<WakeWordEvent>> {
        if !self.config.enabled {
            return Err(VoiceError::Disabled);
        }
        let mut wake_running = self.wake_running.lock().await;
        if *wake_running {
            return Err(VoiceError::AlreadyRunning);
        }
        // 启动音频捕获,把帧喂给唤醒词检测。
        let frame_rx = self.capture.start(self.config.capture_buffer).await?;
        let event_rx = self.wake.start(frame_rx, 16).await?;
        *wake_running = true;
        info!(target: "nebula::voice", backend = %self.config.wake_backend, "wake word detection started");
        Ok(event_rx)
    }

    async fn wake_stop(&self) -> VoiceResult<()> {
        let mut wake_running = self.wake_running.lock().await;
        if !*wake_running {
            return Ok(()); // 幂等。
        }
        self.wake.stop().await?;
        self.capture.stop().await?;
        *wake_running = false;
        info!(target: "nebula::voice", "wake word detection stopped");
        Ok(())
    }

    async fn is_wake_detecting(&self) -> bool {
        *self.wake_running.lock().await
    }

    async fn status(&self) -> VoiceEngineStatus {
        VoiceEngineStatus {
            enabled: self.config.enabled,
            stt_backend: self.config.stt_backend.clone(),
            tts_backend: self.config.tts_backend.clone(),
            wake_backend: self.config.wake_backend.clone(),
            audio_backend: self.config.audio_backend.clone(),
            listening: false, // listen 是一次性,无持久状态。
            speaking: self.sink.is_playing().await,
            wake_detecting: self.is_wake_detecting().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造启用的 noop 引擎(覆盖 enabled 检查)。
    fn enabled_noop_engine() -> DynVoiceEngine {
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
        // 注:返回类型为 DynVoiceEngine(= Arc<dyn VoiceEngine>),
        // 需用 Arc::new 包装 DefaultVoiceEngine 以满足 trait object 类型。
        Arc::new(DefaultVoiceEngine::new(
            config,
            Arc::new(NoopAudioCapture::new(format)),
            Arc::new(NoopSttEngine::new("你好世界")),
            Arc::new(NoopTtsEngine::new(format)),
            Arc::new(wake::NoopWakeWordDetector::new("hey nebula")),
            Arc::new(NoopAudioSink::new()),
        ))
    }

    #[test]
    fn test_voice_config_defaults() {
        // 不修改 env,只验证结构(避免与并行 env 测试冲突)。
        let cfg = VoiceConfig::from_env();
        // 默认未启用(除非 env 设置)。
        let _ = cfg.enabled;
        // capture_buffer 默认 64,非 0(除非 env 显式设为 0)。
        assert_ne!(cfg.capture_buffer, 0);
    }

    #[tokio::test]
    async fn test_disabled_engine_returns_disabled() {
        let engine = DefaultVoiceEngine::noop();
        let err = engine.listen().await.unwrap_err();
        assert!(matches!(err, VoiceError::Disabled));
    }

    #[tokio::test]
    async fn test_enabled_listen_returns_transcription() {
        let engine = enabled_noop_engine();
        let t = engine.listen().await.expect("listen");
        // NoopSttEngine 在流关闭后产出 "你好世界"。
        assert_eq!(t.full_text, "你好世界");
    }

    #[tokio::test]
    async fn test_enabled_speak_returns_frame_count() {
        let engine = enabled_noop_engine();
        let count = engine.speak("测试").await.expect("speak");
        // NoopTtsEngine 对非空文本返回 1 帧。
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_wake_start_stop_lifecycle() {
        let engine = enabled_noop_engine();
        assert!(!engine.is_wake_detecting().await);
        let _events = engine.wake_start().await.expect("wake_start");
        assert!(engine.is_wake_detecting().await);
        // 重复 start 报错。
        let err = engine.wake_start().await.unwrap_err();
        assert!(matches!(err, VoiceError::AlreadyRunning));
        engine.wake_stop().await.expect("wake_stop");
        assert!(!engine.is_wake_detecting().await);
        // 重复 stop 幂等。
        engine.wake_stop().await.expect("idempotent stop");
    }

    #[tokio::test]
    async fn test_status_reflects_config() {
        let engine = enabled_noop_engine();
        let status = engine.status().await;
        assert!(status.enabled);
        assert_eq!(status.stt_backend, "noop");
        assert_eq!(status.tts_backend, "noop");
        assert_eq!(status.wake_backend, "noop");
        assert!(!status.wake_detecting);
    }

    #[test]
    fn test_from_config_noop_backends() {
        let config = VoiceConfig {
            enabled: true,
            stt_backend: "unknown".into(),
            tts_backend: "unknown".into(),
            wake_backend: "unknown".into(),
            audio_backend: "unknown".into(),
            wake_auto_listen: false,
            capture_buffer: 8,
        };
        // 未知后端回退到 noop,不应 panic。
        let engine = DefaultVoiceEngine::from_config(config);
        assert_eq!(engine.kind(), "default");
    }

    #[test]
    fn test_voice_error_from_suberrors() {
        let e: VoiceError = SttError::NotImplemented("x".into()).into();
        assert!(matches!(e, VoiceError::Stt(_)));
        let e: VoiceError = TtsError::NotFound("y".into()).into();
        assert!(matches!(e, VoiceError::Tts(_)));
        let e: VoiceError = WakeError::AlreadyRunning.into();
        assert!(matches!(e, VoiceError::Wake(_)));
        let e: VoiceError = AudioError::DeviceUnavailable("z".into()).into();
        assert!(matches!(e, VoiceError::Audio(_)));
    }
}
