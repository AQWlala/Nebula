//! T-E-C-15: TTS(Text-to-Speech,文字转语音)引擎抽象。
//!
//! 提供 `TtsEngine` trait 与框架后端:
//! - `LocalTtsBackend`:本地 TTS 引擎子进程框架(如
//!   [piper](https://github.com/rhasspy/piper)/espeak-ng/edge-tts)。
//!   通过 `tokio::process` 调用,把文本送 stdin、从 stdout 读 PCM。
//!   **不实际执行**——需运行时安装引擎并配置 `NEBULA_TTS_CLI_PATH`。
//! - `NoopTtsEngine`:测试 stub,记录输入文本不发声。
//!
//! ## 流式设计
//! `synthesize_stream` 消费文本 token 流(来自 LLM 流式输出),持续产出
//! `SynthesisEvent`(`Start`/`Chunk`/`End`),实现"边生成边朗读"。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use super::audio::{AudioFormat, AudioFrame};

/// TTS 错误。
#[derive(Debug, Error)]
pub enum TtsError {
    /// 后端未实现(feature 未启用)。
    #[error("tts backend not implemented: {0}")]
    NotImplemented(String),
    /// 引擎/二进制未找到。
    #[error("engine or binary not found: {0}")]
    NotFound(String),
    /// 子进程执行失败。
    #[error("subprocess failed: {0}")]
    Subprocess(String),
    /// 语音/音色资源缺失。
    #[error("voice resource missing: {0}")]
    VoiceMissing(String),
}

/// 合成请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisRequest {
    /// 待合成文本。
    pub text: String,
    /// 音色 ID 或名称(后端特定,如 "zh_CN-huayan-medium")。
    pub voice: Option<String>,
    /// 语速(0.5~2.0,1.0=正常)。None 用后端默认。
    pub speed: Option<f32>,
    /// 音调(0.5~2.0)。None 用后端默认。
    pub pitch: Option<f32>,
    /// 输出音频格式(采样率/声道)。None 用后端默认。
    pub format: Option<AudioFormat>,
}

impl SynthesisRequest {
    /// 构造简单请求(仅文本)。
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice: None,
            speed: None,
            pitch: None,
            format: None,
        }
    }

    /// 指定音色。
    pub fn with_voice(mut self, voice: impl Into<String>) -> Self {
        self.voice = Some(voice.into());
        self
    }
}

/// 流式合成事件。
#[derive(Debug, Clone)]
pub enum SynthesisEvent {
    /// 合成开始(携带总文本长度提示)。
    Start { text_len: usize },
    /// 一段音频帧产出。
    Chunk(AudioFrame),
    /// 合成结束(总帧数)。
    End { frame_count: usize },
}

/// TTS 引擎 trait。
///
/// 两种用法:
/// 1. 一次性:`synthesize(req)` 返回完整帧序列(适合短文本)。
/// 2. 流式:`synthesize_stream(token_rx)` 消费文本 token 流,产出
///    `SynthesisEvent`(适合 LLM 流式输出边生成边朗读)。
#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// 后端标识(`"piper"` / `"espeak"` / `"noop"`)。
    fn kind(&self) -> &'static str;

    /// 默认输出音频格式。
    fn output_format(&self) -> AudioFormat;

    /// 一次性合成:返回完整音频帧序列。
    async fn synthesize(&self, req: &SynthesisRequest) -> Result<Vec<AudioFrame>, TtsError>;

    /// 流式合成:消费文本 token,产出合成事件。
    /// 后端内部 spawn task 聚合 token 为句子再合成。
    async fn synthesize_stream(
        &self,
        token_rx: mpsc::Receiver<String>,
    ) -> Result<mpsc::Receiver<SynthesisEvent>, TtsError>;
}

/// 动态分发别名。
pub type DynTtsEngine = Arc<dyn TtsEngine>;

// ---------------------------------------------------------------------------
// LocalTtsBackend:本地 TTS 子进程框架(不实际执行)。
// ---------------------------------------------------------------------------

/// 本地 TTS 后端配置。
#[derive(Debug, Clone)]
pub struct LocalTtsConfig {
    /// TTS 引擎可执行文件路径(piper / espeak-ng)。env: `NEBULA_TTS_CLI_PATH`。
    pub cli_path: String,
    /// 默认音色资源路径(piper .onnx + .json)。env: `NEBULA_TTS_VOICE`。
    pub voice: String,
    /// 默认语速。env: `NEBULA_TTS_SPEED`。
    pub speed: f32,
    /// 输出采样率。env: `NEBULA_TTS_SAMPLE_RATE`。
    pub sample_rate: u32,
}

impl Default for LocalTtsConfig {
    fn default() -> Self {
        Self {
            cli_path: std::env::var("NEBULA_TTS_CLI_PATH").unwrap_or_else(|_| "piper".to_string()),
            voice: std::env::var("NEBULA_TTS_VOICE")
                .unwrap_or_else(|_| "zh_CN-huayan-medium".to_string()),
            speed: std::env::var("NEBULA_TTS_SPEED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
            sample_rate: std::env::var("NEBULA_TTS_SAMPLE_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(22050),
        }
    }
}

/// 本地 TTS 后端:子进程框架。
///
/// **框架实现**:配置完整,`synthesize` 校验二进制存在性,缺失返回 `NotFound`。
/// 实际合成(写 stdin → 读 stdout PCM → 分帧)由 `voice-tts` feature 提供。
pub struct LocalTtsBackend {
    config: LocalTtsConfig,
}

impl LocalTtsBackend {
    pub fn new(config: LocalTtsConfig) -> Self {
        Self { config }
    }

    /// 从环境变量构造。
    pub fn from_env() -> Self {
        Self::new(LocalTtsConfig::default())
    }

    /// 输出格式。
    fn fmt(&self) -> AudioFormat {
        AudioFormat {
            sample_rate: self.config.sample_rate,
            channels: 1,
            frame_ms: 20,
        }
    }
}

#[async_trait]
impl TtsEngine for LocalTtsBackend {
    fn kind(&self) -> &'static str {
        "local-tts"
    }

    fn output_format(&self) -> AudioFormat {
        self.fmt()
    }

    async fn synthesize(&self, _req: &SynthesisRequest) -> Result<Vec<AudioFrame>, TtsError> {
        let which = super::stt::which_exists(&self.config.cli_path);
        if !which {
            return Err(TtsError::NotFound(format!(
                "tts engine not found at '{}' (set NEBULA_TTS_CLI_PATH); \
                 enable voice-tts feature for full impl",
                self.config.cli_path
            )));
        }
        Err(TtsError::NotImplemented(
            "local tts synthesis requires voice-tts feature".into(),
        ))
    }

    async fn synthesize_stream(
        &self,
        mut token_rx: mpsc::Receiver<String>,
    ) -> Result<mpsc::Receiver<SynthesisEvent>, TtsError> {
        let which = super::stt::which_exists(&self.config.cli_path);
        if !which {
            // drain 以防发送端阻塞。
            tokio::spawn(async move { while token_rx.recv().await.is_some() {} });
            return Err(TtsError::NotFound(format!(
                "tts engine not found at '{}'",
                self.config.cli_path
            )));
        }
        let (tx, out_rx) = mpsc::channel::<SynthesisEvent>(16);
        tokio::spawn(async move {
            // 框架:drain token,实际流式合成需 voice-tts feature。
            let mut count = 0usize;
            while token_rx.recv().await.is_some() {
                count += 1;
            }
            let _ = tx.send(SynthesisEvent::End { frame_count: count }).await;
        });
        Ok(out_rx)
    }
}

// ---------------------------------------------------------------------------
// NoopTtsEngine:测试 stub。
// ---------------------------------------------------------------------------

/// 测试用 TTS stub。记录合成请求,返回静音帧。
pub struct NoopTtsEngine {
    format: AudioFormat,
    /// 累计合成次数(测试断言用)。
    pub call_count: std::sync::atomic::AtomicUsize,
}

impl NoopTtsEngine {
    pub fn new(format: AudioFormat) -> Self {
        Self {
            format,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TtsEngine for NoopTtsEngine {
    fn kind(&self) -> &'static str {
        "noop"
    }

    fn output_format(&self) -> AudioFormat {
        self.format
    }

    async fn synthesize(&self, req: &SynthesisRequest) -> Result<Vec<AudioFrame>, TtsError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // 文本为空时返回空帧序列。
        if req.text.trim().is_empty() {
            return Ok(Vec::new());
        }
        // 返回一帧静音代表"已合成"。
        Ok(vec![AudioFrame::silence(self.format, 0)])
    }

    async fn synthesize_stream(
        &self,
        mut token_rx: mpsc::Receiver<String>,
    ) -> Result<mpsc::Receiver<SynthesisEvent>, TtsError> {
        let fmt = self.format;
        let (tx, out_rx) = mpsc::channel::<SynthesisEvent>(16);
        tokio::spawn(async move {
            let mut frame_count = 0usize;
            let _ = tx.send(SynthesisEvent::Start { text_len: 0 }).await;
            while let Some(token) = token_rx.recv().await {
                let _ = token.len(); // 累计文本长度(框架,占位)
                                     // 每 token 产出一帧静音(框架)。
                let _ = tx
                    .send(SynthesisEvent::Chunk(AudioFrame::silence(
                        fmt,
                        frame_count as u64,
                    )))
                    .await;
                frame_count += 1;
            }
            let _ = tx.send(SynthesisEvent::End { frame_count }).await;
        });
        Ok(out_rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthesis_request_builder() {
        let req = SynthesisRequest::new("你好").with_voice("zh-huayan");
        assert_eq!(req.text, "你好");
        assert_eq!(req.voice.as_deref(), Some("zh-huayan"));
    }

    #[tokio::test]
    async fn test_noop_tts_synthesize() {
        let tts = NoopTtsEngine::new(AudioFormat::WHISPER_16K_MONO);
        let req = SynthesisRequest::new("测试");
        let frames = tts.synthesize(&req).await.expect("synthesize");
        assert_eq!(frames.len(), 1);
        assert_eq!(tts.call_count.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_noop_tts_empty_text() {
        let tts = NoopTtsEngine::new(AudioFormat::WHISPER_16K_MONO);
        let req = SynthesisRequest::new("   ");
        let frames = tts.synthesize(&req).await.expect("synthesize");
        assert!(frames.is_empty());
    }

    #[tokio::test]
    async fn test_noop_tts_stream() {
        let tts = NoopTtsEngine::new(AudioFormat::WHISPER_16K_MONO);
        let (tx, rx) = mpsc::channel::<String>(4);
        let _ = tx.send("你".to_string()).await;
        let _ = tx.send("好".to_string()).await;
        drop(tx);
        let mut out = tts.synthesize_stream(rx).await.expect("stream");
        let mut chunks = 0;
        let mut got_end = false;
        while let Some(ev) = out.recv().await {
            match ev {
                SynthesisEvent::Chunk(_) => chunks += 1,
                SynthesisEvent::End { frame_count } => {
                    assert_eq!(frame_count, 2);
                    got_end = true;
                }
                _ => {}
            }
        }
        assert_eq!(chunks, 2);
        assert!(got_end);
    }

    #[tokio::test]
    async fn test_local_tts_missing_binary() {
        let config = LocalTtsConfig {
            cli_path: "/nonexistent/piper-xyz".to_string(),
            voice: "v".to_string(),
            speed: 1.0,
            sample_rate: 22050,
        };
        let tts = LocalTtsBackend::new(config);
        let req = SynthesisRequest::new("测试");
        let err = tts.synthesize(&req).await.unwrap_err();
        assert!(matches!(err, TtsError::NotFound(_)));
    }
}
