//! T-E-C-15: 音频捕获抽象层。
//!
//! 提供与 `cpal`(跨平台音频库)等价的 trait 接口,**不实际引入 cpal 依赖**。
//! 具体后端(cpal / WASAPI / CoreAudio)在 feature gate 开启时实现本 trait。
//!
//! ## 设计
//! - `AudioFormat`:采样率 / 声道数 / 位深,STT/TTS 后端据此协商格式。
//! - `AudioFrame`:一帧 PCM 样本(归一化 f32,范围 [-1.0, 1.0]),通用格式避免
//!   后端各自定义;后端内部做 f32↔i16 转换。
//! - `AudioCapture`:异步流式捕获 trait,`start` 返回接收帧的 channel,
//!   `stop` 停止采集;与 `AudioSink`(播放)对称。
//! - `NoopAudioCapture` / `NoopAudioSink`:默认 stub,返回空流,保证无硬件时编译运行。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

/// PCM 音频格式描述。STT/TTS 后端据此协商采样率与声道。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    /// 采样率(Hz),Whisper.cpp 标准 16000。
    pub sample_rate: u32,
    /// 声道数(1=mono, 2=stereo)。语音交互默认单声道。
    pub channels: u16,
    /// 每帧时长(毫秒),用于推算帧大小。如 20ms@16kHz mono = 320 samples。
    pub frame_ms: u32,
}

impl AudioFormat {
    /// Whisper.cpp 标准格式:16kHz mono,20ms 帧(320 samples/帧)。
    pub const WHISPER_16K_MONO: Self = Self {
        sample_rate: 16000,
        channels: 1,
        frame_ms: 20,
    };

    /// 单帧样本数 = sample_rate * frame_ms / 1000 * channels。
    pub fn samples_per_frame(&self) -> usize {
        ((self.sample_rate as u64 * self.frame_ms as u64) / 1000) as usize * self.channels as usize
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self::WHISPER_16K_MONO
    }
}

/// 一帧 PCM 音频数据。样本为归一化 f32(范围 [-1.0, 1.0])。
///
/// 采用 f32 而非 i16 作为通用格式:cpal 默认输出 f32,Whisper.cpp 接受 f32,
/// 避免各后端重复转换。播放后端在内部做 f32→设备格式转换。
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// 归一化 PCM 样本。
    pub samples: Vec<f32>,
    /// 本帧的格式(可能与全局不同,如重采样后)。
    pub format: AudioFormat,
    /// 帧序号(从 0 递增),用于 STT 时间戳对齐。
    pub sequence: u64,
}

impl AudioFrame {
    /// 构造一帧空数据(静音)。
    pub fn silence(format: AudioFormat, sequence: u64) -> Self {
        let samples = vec![0.0; format.samples_per_frame()];
        Self {
            samples,
            format,
            sequence,
        }
    }

    /// 本帧时长(秒)= samples / (sample_rate * channels)。
    pub fn duration_secs(&self) -> f64 {
        if self.format.channels == 0 || self.format.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f64 / (self.format.sample_rate as f64 * self.format.channels as f64)
    }

    /// RMS 能量(0.0~1.0),用于能量检测唤醒词的 VAD。
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = self.samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum_sq / self.samples.len() as f64).sqrt() as f32
    }
}

/// 音频捕获错误。捕获/播放失败时由后端返回。
#[derive(Debug, Error)]
pub enum AudioError {
    /// 音频设备不可用(无麦克风 / 被占用)。
    #[error("audio device unavailable: {0}")]
    DeviceUnavailable(String),
    /// 设备枚举失败。
    #[error("device enumerate failed: {0}")]
    EnumerateFailed(String),
    /// 流启动失败。
    #[error("stream start failed: {0}")]
    StreamStartFailed(String),
    /// 后端未实现(feature 未启用)。
    #[error("audio backend not implemented: {0}")]
    NotImplemented(String),
    /// 底层 IO 错误。
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// 异步流式音频捕获 trait。
///
/// `start` 返回一个 `mpsc::Receiver<AudioFrame>`,调用方持续 poll 即可获得
/// 实时麦克风数据;`stop` 停止采集并关闭 channel。
///
/// 与 cpal 的 `Stream` API 对齐:cpal 的回调把样本推入 channel,
/// 本 trait 把"回调 + channel"封装为异步接口。
#[async_trait]
pub trait AudioCapture: Send + Sync {
    /// 后端标识(`"cpal"` / `"wasapi"` / `"noop"`)。
    fn kind(&self) -> &'static str;

    /// 当前协商的音频格式。STT 后端据此解析帧。
    fn format(&self) -> AudioFormat;

    /// 启动捕获,返回帧接收端。重复 start 返回错误。
    /// `buffer_capacity` 控制背压(建议 32~128 帧)。
    async fn start(&self, buffer_capacity: usize)
        -> Result<mpsc::Receiver<AudioFrame>, AudioError>;

    /// 停止捕获。幂等:未运行时返回 Ok。
    async fn stop(&self) -> Result<(), AudioError>;

    /// 是否正在捕获。
    async fn is_running(&self) -> bool;
}

/// 异步音频播放 trait(TTS 输出端)。
///
/// `play` 接收帧序列并阻塞播放至完成;`play_stream` 接收 channel 持续播放。
#[async_trait]
pub trait AudioSink: Send + Sync {
    /// 后端标识。
    fn kind(&self) -> &'static str;

    /// 播放一串帧(同步至播放完成)。
    async fn play(&self, frames: &[AudioFrame]) -> Result<(), AudioError>;

    /// 停止当前播放。幂等。
    async fn stop(&self) -> Result<(), AudioError>;

    /// 是否正在播放。
    async fn is_playing(&self) -> bool;
}

/// 动态分发别名。
pub type DynAudioCapture = Arc<dyn AudioCapture>;
pub type DynAudioSink = Arc<dyn AudioSink>;

// ---------------------------------------------------------------------------
// Noop 后端:无硬件时编译运行的 stub。
// ---------------------------------------------------------------------------

/// 空操作音频捕获后端。`start` 立即关闭 channel(无数据),用于无硬件环境/测试。
pub struct NoopAudioCapture {
    format: AudioFormat,
    running: tokio::sync::Mutex<bool>,
}

impl NoopAudioCapture {
    pub fn new(format: AudioFormat) -> Self {
        Self {
            format,
            running: tokio::sync::Mutex::new(false),
        }
    }
}

#[async_trait]
impl AudioCapture for NoopAudioCapture {
    fn kind(&self) -> &'static str {
        "noop"
    }

    fn format(&self) -> AudioFormat {
        self.format
    }

    async fn start(
        &self,
        _buffer_capacity: usize,
    ) -> Result<mpsc::Receiver<AudioFrame>, AudioError> {
        let mut running = self.running.lock().await;
        if *running {
            return Err(AudioError::StreamStartFailed(
                "noop capture already running".into(),
            ));
        }
        *running = true;
        // 立即关闭 channel:调用方 poll 得到 None(流结束)。
        let (_tx, rx) = mpsc::channel::<AudioFrame>(1);
        Ok(rx)
    }

    async fn stop(&self) -> Result<(), AudioError> {
        let mut running = self.running.lock().await;
        *running = false;
        Ok(())
    }

    async fn is_running(&self) -> bool {
        *self.running.lock().await
    }
}

/// 空操作音频播放后端。`play` 直接返回 Ok(不发声)。
pub struct NoopAudioSink {
    playing: tokio::sync::Mutex<bool>,
}

impl NoopAudioSink {
    pub fn new() -> Self {
        Self {
            playing: tokio::sync::Mutex::new(false),
        }
    }
}

impl Default for NoopAudioSink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioSink for NoopAudioSink {
    fn kind(&self) -> &'static str {
        "noop"
    }

    async fn play(&self, frames: &[AudioFrame]) -> Result<(), AudioError> {
        if frames.is_empty() {
            return Ok(());
        }
        let mut playing = self.playing.lock().await;
        *playing = true;
        // 模拟播放时长(不真正等待,仅置位)。真实后端在此阻塞至播放完成。
        *playing = false;
        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        *self.playing.lock().await = false;
        Ok(())
    }

    async fn is_playing(&self) -> bool {
        *self.playing.lock().await
    }
}

// 注:From<AudioError> for VoiceError 的实现统一放在 mod.rs,
// 避免在多个子模块重复 impl 导致冲突。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_whisper_default() {
        let f = AudioFormat::WHISPER_16K_MONO;
        assert_eq!(f.sample_rate, 16000);
        assert_eq!(f.channels, 1);
        // 20ms @ 16kHz mono = 320 samples
        assert_eq!(f.samples_per_frame(), 320);
    }

    #[test]
    fn test_audio_frame_silence_and_rms() {
        let f = AudioFormat::WHISPER_16K_MONO;
        let frame = AudioFrame::silence(f, 0);
        assert_eq!(frame.samples.len(), 320);
        // 静音 RMS = 0
        assert_eq!(frame.rms(), 0.0);
        assert!((frame.duration_secs() - 0.02).abs() < 1e-6);
    }

    #[test]
    fn test_audio_frame_rms_nonzero() {
        let f = AudioFormat::WHISPER_16K_MONO;
        let frame = AudioFrame {
            samples: vec![0.5, -0.5, 0.5, -0.5],
            format: f,
            sequence: 1,
        };
        // RMS of [0.5,-0.5,0.5,-0.5] = sqrt((0.25*4)/4) = 0.5
        assert!((frame.rms() - 0.5).abs() < 1e-5);
    }

    #[tokio::test]
    async fn test_noop_capture_lifecycle() {
        let cap = NoopAudioCapture::new(AudioFormat::WHISPER_16K_MONO);
        assert!(!cap.is_running().await);
        let mut rx = cap.start(8).await.expect("start");
        assert!(cap.is_running().await);
        // noop 立即关闭,recv 得到 None
        let none = rx.recv().await;
        assert!(none.is_none());
        cap.stop().await.expect("stop");
        assert!(!cap.is_running().await);
    }

    #[tokio::test]
    async fn test_noop_sink_play() {
        let sink = NoopAudioSink::new();
        let frame = AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0);
        sink.play(&[frame]).await.expect("play");
        assert!(!sink.is_playing().await);
    }

    #[tokio::test]
    async fn test_noop_capture_double_start_errors() {
        let cap = NoopAudioCapture::new(AudioFormat::WHISPER_16K_MONO);
        let _rx = cap.start(8).await.expect("first start");
        let err = cap.start(8).await.unwrap_err();
        assert!(matches!(err, AudioError::StreamStartFailed(_)));
        cap.stop().await.unwrap();
    }
}
