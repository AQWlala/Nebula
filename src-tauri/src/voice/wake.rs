//! T-E-C-15: 唤醒词检测(Wake Word Detection)抽象。
//!
//! 提供 `WakeWordDetector` trait 与两个框架后端:
//! - `PorcupineBackend`:[Picovoice Porcupine](https://picovoice.ai/platform/porcupine/)
//!   框架。Porcupine 是高精度本地唤醒词引擎,需 access key + .ppn 模型。
//!   **不实际执行**——需运行时配置 `NEBULA_PORCUPINE_ACCESS_KEY`。
//! - `EnergyDetectorBackend`:基于 RMS 能量的简单 VAD(Voice Activity Detection)。
//!   无唤醒词识别能力,仅检测"有人说话"(能量超阈值),作为零依赖兜底方案。
//! - `NoopWakeWordDetector`:测试 stub,可注入预设唤醒事件。
//!
//! ## 设计
//! `start` 启动后台检测任务,持续从 `AudioCapture` 读帧并检测;
//! 检测到唤醒词时通过回调 `mpsc::Sender<WakeWordEvent>` 推送事件。
//! `stop` 停止检测。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use super::audio::{AudioFormat, AudioFrame};

/// 唤醒词检测错误。
#[derive(Debug, Error)]
pub enum WakeError {
    /// 后端未实现(feature 未启用 / 无 access key)。
    #[error("wake backend not implemented: {0}")]
    NotImplemented(String),
    /// 模型/access key 缺失(Porcupine)。
    #[error("porcupine resource missing: {0}")]
    ResourceMissing(String),
    /// 已在运行(重复 start)。
    #[error("already running")]
    AlreadyRunning,
    /// 未运行(stop 时无操作则返回 Ok,此处指内部状态不一致)。
    #[error("not running")]
    NotRunning,
}

/// 唤醒事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeWordEvent {
    /// 触发的唤醒词(如 "hey nebula")。
    pub keyword: String,
    /// 检测时间戳(Unix 毫秒)。
    pub timestamp_ms: i64,
    /// 置信度(0.0~1.0)。能量检测无置信度,固定 1.0。
    pub confidence: f32,
}

/// 唤醒词检测 trait。
///
/// `start` 接收帧输入 channel,启动后台检测任务,返回事件接收端。
/// 调用方(如 VoiceEngine)poll 事件端,收到唤醒后启动 STT 监听。
#[async_trait]
pub trait WakeWordDetector: Send + Sync {
    /// 后端标识(`"porcupine"` / `"energy"` / `"noop"`)。
    fn kind(&self) -> &'static str;

    /// 期望输入格式。
    fn expected_format(&self) -> AudioFormat;

    /// 启动检测。`frame_rx` 来自 AudioCapture,`event_tx_capacity` 控制事件背压。
    /// 返回事件接收端。重复 start 返回 `AlreadyRunning`。
    async fn start(
        &self,
        frame_rx: mpsc::Receiver<AudioFrame>,
        event_tx_capacity: usize,
    ) -> Result<mpsc::Receiver<WakeWordEvent>, WakeError>;

    /// 停止检测。幂等:未运行返回 Ok。
    async fn stop(&self) -> Result<(), WakeError>;

    /// 是否正在检测。
    async fn is_running(&self) -> bool;
}

/// 动态分发别名。
pub type DynWakeWordDetector = Arc<dyn WakeWordDetector>;

// ---------------------------------------------------------------------------
// PorcupineBackend:框架(不实际执行)。
// ---------------------------------------------------------------------------

/// Porcupine 后端配置。
#[derive(Debug, Clone)]
pub struct PorcupineConfig {
    /// Picovoice access key。env: `NEBULA_PORCUPINE_ACCESS_KEY`。
    pub access_key: String,
    /// 唤醒词模型文件(.ppn)。env: `NEBULA_PORCUPINE_MODEL`。
    pub model_path: String,
    /// 内置唤醒词关键字(如 "porcupine"/"terminator")。env: `NEBULA_PORCUPINE_KEYWORD`。
    pub keyword: String,
    /// 灵敏度(0.0~1.0,越高越易触发但误触多)。env: `NEBULA_PORCUPINE_SENSITIVITY`。
    pub sensitivity: f32,
}

impl Default for PorcupineConfig {
    fn default() -> Self {
        Self {
            access_key: std::env::var("NEBULA_PORCUPINE_ACCESS_KEY").unwrap_or_default(),
            model_path: std::env::var("NEBULA_PORCUPINE_MODEL")
                .unwrap_or_else(|_| "hey_nebula.ppn".to_string()),
            keyword: std::env::var("NEBULA_PORCUPINE_KEYWORD")
                .unwrap_or_else(|_| "hey nebula".to_string()),
            sensitivity: std::env::var("NEBULA_PORCUPINE_SENSITIVITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.5),
        }
    }
}

/// Porcupine 唤醒词后端:框架。
///
/// **不实际执行**:校验 access key 存在性,缺失返回 `ResourceMissing`。
/// 实际检测需 `voice-porcupine` feature(绑定 picovoice SDK)。
pub struct PorcupineBackend {
    config: PorcupineConfig,
    running: tokio::sync::Mutex<bool>,
}

impl PorcupineBackend {
    pub fn new(config: PorcupineConfig) -> Self {
        Self {
            config,
            running: tokio::sync::Mutex::new(false),
        }
    }

    /// 从环境变量构造。
    pub fn from_env() -> Self {
        Self::new(PorcupineConfig::default())
    }
}

#[async_trait]
impl WakeWordDetector for PorcupineBackend {
    fn kind(&self) -> &'static str {
        "porcupine"
    }

    fn expected_format(&self) -> AudioFormat {
        // Porcupine 要求 16kHz mono 16-bit。
        AudioFormat::WHISPER_16K_MONO
    }

    async fn start(
        &self,
        mut frame_rx: mpsc::Receiver<AudioFrame>,
        event_tx_capacity: usize,
    ) -> Result<mpsc::Receiver<WakeWordEvent>, WakeError> {
        let mut running = self.running.lock().await;
        if *running {
            return Err(WakeError::AlreadyRunning);
        }
        if self.config.access_key.is_empty() {
            // drain 以防发送端阻塞。
            tokio::spawn(async move { while frame_rx.recv().await.is_some() {} });
            return Err(WakeError::ResourceMissing(
                "NEBULA_PORCUPINE_ACCESS_KEY not set; \
                 enable voice-porcupine feature with a valid access key"
                    .into(),
            ));
        }
        *running = true;
        let (tx, event_rx) = mpsc::channel::<WakeWordEvent>(event_tx_capacity);
        let keyword = self.config.keyword.clone();
        // 框架:drain 帧,实际检测需 voice-porcupine feature。
        tokio::spawn(async move {
            while frame_rx.recv().await.is_some() {}
            // 框架不产生唤醒事件。
            let _ = tx;
        });
        // 标记关键字供日志(避免未使用警告)。
        tracing::debug!(keyword = %keyword, "porcupine wake backend started (framework)");
        Ok(event_rx)
    }

    async fn stop(&self) -> Result<(), WakeError> {
        *self.running.lock().await = false;
        Ok(())
    }

    async fn is_running(&self) -> bool {
        *self.running.lock().await
    }
}

// ---------------------------------------------------------------------------
// EnergyDetectorBackend:零依赖能量 VAD。
// ---------------------------------------------------------------------------

/// 能量检测后端配置。
#[derive(Debug, Clone)]
pub struct EnergyDetectorConfig {
    /// 触发阈值(RMS,0.0~1.0)。超过即视为"有人说话"。
    pub threshold: f32,
    /// 持续触发多少帧才确认唤醒(防抖)。
    pub min_frames: u32,
    /// 触发后冷却时间(帧数),避免连续误触发。
    pub cooldown_frames: u32,
    /// 唤醒词标签(无实际识别,仅标记)。
    pub keyword: String,
}

impl Default for EnergyDetectorConfig {
    fn default() -> Self {
        Self {
            threshold: 0.05,
            min_frames: 3,
            cooldown_frames: 50,
            keyword: "energy-wake".to_string(),
        }
    }
}

/// 能量检测唤醒后端:基于 RMS 的简单 VAD。
///
/// 无唤醒词识别能力,仅当连续 `min_frames` 帧 RMS 超阈值时触发唤醒。
/// 作为零依赖兜底方案(Porcupine 不可用时)。
pub struct EnergyDetectorBackend {
    config: EnergyDetectorConfig,
    running: tokio::sync::Mutex<bool>,
}

impl EnergyDetectorBackend {
    pub fn new(config: EnergyDetectorConfig) -> Self {
        Self {
            config,
            running: tokio::sync::Mutex::new(false),
        }
    }

    /// 用默认配置构造。
    pub fn with_defaults() -> Self {
        Self::new(EnergyDetectorConfig::default())
    }
}

#[async_trait]
impl WakeWordDetector for EnergyDetectorBackend {
    fn kind(&self) -> &'static str {
        "energy"
    }

    fn expected_format(&self) -> AudioFormat {
        AudioFormat::WHISPER_16K_MONO
    }

    async fn start(
        &self,
        mut frame_rx: mpsc::Receiver<AudioFrame>,
        event_tx_capacity: usize,
    ) -> Result<mpsc::Receiver<WakeWordEvent>, WakeError> {
        let mut running = self.running.lock().await;
        if *running {
            return Err(WakeError::AlreadyRunning);
        }
        *running = true;
        drop(running);

        let config = self.config.clone();
        let (tx, event_rx) = mpsc::channel::<WakeWordEvent>(event_tx_capacity);
        tokio::spawn(async move {
            let mut consecutive = 0u32;
            let mut cooldown = 0u32;
            while let Some(frame) = frame_rx.recv().await {
                if cooldown > 0 {
                    cooldown = cooldown.saturating_sub(1);
                    consecutive = 0;
                    continue;
                }
                if frame.rms() >= config.threshold {
                    consecutive += 1;
                    if consecutive >= config.min_frames {
                        let event = WakeWordEvent {
                            keyword: config.keyword.clone(),
                            timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            confidence: 1.0,
                        };
                        if tx.send(event).await.is_err() {
                            break; // 接收端关闭,退出。
                        }
                        consecutive = 0;
                        cooldown = config.cooldown_frames;
                    }
                } else {
                    consecutive = 0;
                }
            }
        });
        Ok(event_rx)
    }

    async fn stop(&self) -> Result<(), WakeError> {
        *self.running.lock().await = false;
        Ok(())
    }

    async fn is_running(&self) -> bool {
        *self.running.lock().await
    }
}

// ---------------------------------------------------------------------------
// NoopWakeWordDetector:测试 stub。
// ---------------------------------------------------------------------------

/// 测试用唤醒词 stub。可注入预设唤醒事件。
pub struct NoopWakeWordDetector {
    format: AudioFormat,
    preset_keyword: String,
    running: tokio::sync::Mutex<bool>,
}

impl NoopWakeWordDetector {
    pub fn new(keyword: impl Into<String>) -> Self {
        Self {
            format: AudioFormat::WHISPER_16K_MONO,
            preset_keyword: keyword.into(),
            running: tokio::sync::Mutex::new(false),
        }
    }
}

#[async_trait]
impl WakeWordDetector for NoopWakeWordDetector {
    fn kind(&self) -> &'static str {
        "noop"
    }

    fn expected_format(&self) -> AudioFormat {
        self.format
    }

    async fn start(
        &self,
        mut frame_rx: mpsc::Receiver<AudioFrame>,
        event_tx_capacity: usize,
    ) -> Result<mpsc::Receiver<WakeWordEvent>, WakeError> {
        let mut running = self.running.lock().await;
        if *running {
            return Err(WakeError::AlreadyRunning);
        }
        *running = true;
        drop(running);

        let keyword = self.preset_keyword.clone();
        let (tx, event_rx) = mpsc::channel::<WakeWordEvent>(event_tx_capacity);
        tokio::spawn(async move {
            // 框架:收到首帧后推送一次唤醒事件,然后 drain。
            let mut fired = false;
            while let Some(frame) = frame_rx.recv().await {
                if !fired && frame.rms() > 0.0 {
                    let event = WakeWordEvent {
                        keyword: keyword.clone(),
                        timestamp_ms: chrono::Utc::now().timestamp_millis(),
                        confidence: 1.0,
                    };
                    let _ = tx.send(event).await;
                    fired = true;
                }
            }
        });
        Ok(event_rx)
    }

    async fn stop(&self) -> Result<(), WakeError> {
        *self.running.lock().await = false;
        Ok(())
    }

    async fn is_running(&self) -> bool {
        *self.running.lock().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_energy_detector_triggers_on_loud_frame() {
        let detector = EnergyDetectorBackend::with_defaults();
        let (tx, rx) = mpsc::channel::<AudioFrame>(16);
        // 发送 5 帧 loud 音频(RMS=0.5 > 0.05),min_frames=3 → 应触发。
        for i in 0..5 {
            let frame = AudioFrame {
                samples: vec![0.5; 320],
                format: AudioFormat::WHISPER_16K_MONO,
                sequence: i,
            };
            let _ = tx.send(frame).await;
        }
        drop(tx);
        let mut events = detector.start(rx, 8).await.expect("start");
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("timeout")
            .expect("event");
        assert_eq!(event.keyword, "energy-wake");
        assert_eq!(event.confidence, 1.0);
        detector.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_energy_detector_ignores_silence() {
        let detector = EnergyDetectorBackend::with_defaults();
        let (tx, rx) = mpsc::channel::<AudioFrame>(16);
        // 全静音(RMS=0 < 0.05),不应触发。
        for i in 0..10 {
            let _ = tx
                .send(AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, i))
                .await;
        }
        drop(tx);
        let mut events = detector.start(rx, 8).await.expect("start");
        // drain 结束后应无事件。
        let none = tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await;
        assert!(none.is_ok() && none.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_porcupine_missing_access_key() {
        let config = PorcupineConfig {
            access_key: String::new(),
            model_path: "m.ppn".into(),
            keyword: "kw".into(),
            sensitivity: 0.5,
        };
        let detector = PorcupineBackend::new(config);
        let (_tx, rx) = mpsc::channel::<AudioFrame>(4);
        let err = detector.start(rx, 8).await.unwrap_err();
        assert!(matches!(err, WakeError::ResourceMissing(_)));
    }

    #[tokio::test]
    async fn test_porcupine_already_running() {
        let config = PorcupineConfig {
            access_key: "fake-key".into(),
            model_path: "m.ppn".into(),
            keyword: "kw".into(),
            sensitivity: 0.5,
        };
        let detector = PorcupineBackend::new(config);
        let (_tx, rx) = mpsc::channel::<AudioFrame>(4);
        let _events = detector.start(rx, 8).await.expect("start");
        let (_tx2, rx2) = mpsc::channel::<AudioFrame>(4);
        let err = detector.start(rx2, 8).await.unwrap_err();
        assert!(matches!(err, WakeError::AlreadyRunning));
        detector.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_noop_wake_fires_on_first_loud_frame() {
        let detector = NoopWakeWordDetector::new("hey nebula");
        let (tx, rx) = mpsc::channel::<AudioFrame>(8);
        let _ = tx
            .send(AudioFrame {
                samples: vec![0.3; 320],
                format: AudioFormat::WHISPER_16K_MONO,
                sequence: 0,
            })
            .await;
        drop(tx);
        let mut events = detector.start(rx, 8).await.expect("start");
        let event = events.recv().await.expect("event");
        assert_eq!(event.keyword, "hey nebula");
    }
}
