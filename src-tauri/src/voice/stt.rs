//! T-E-C-15: STT(Speech-to-Text,语音转文字)引擎抽象。
//!
//! 提供 `SttEngine` trait 与两个框架后端:
//! - `WhisperCppBackend`:基于 [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
//!   的 `whisper-cli` 子进程方案。Nebula 通过 `tokio::process` 调用本地
//!   `whisper-cli`,把 PCM 写入临时 wav 文件再解析 SRT/JSON 输出。
//!   **不实际执行**——需运行时安装 whisper.cpp 并配置 `NEBULA_WHISPER_CLI_PATH`。
//! - `OllamaSttBackend`:stub。**Ollama 当前不支持原生语音/音频模型**
//!   (仅 `/api/chat`、`/api/generate`、`/api/embeddings`),此 stub 始终返回
//!   `NotImplemented`,保留接口以备未来 Ollama 多模态扩展。
//! - `NoopSttEngine`:测试 stub,可注入预设转录结果。
//!
//! ## 流式设计
//! `transcribe_stream` 接收 `mpsc::Receiver<AudioFrame>`,持续产出 `TranscriptionEvent`:
//! - `Partial`:部分识别(实时显示)。
//! - `Final`:整句完成(送入 LLM)。
//! - `End`:流结束。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use super::audio::{AudioFormat, AudioFrame};

/// STT 错误。
#[derive(Debug, Error)]
pub enum SttError {
    /// 后端未实现(Ollama stub / feature 未启用)。
    #[error("stt backend not implemented: {0}")]
    NotImplemented(String),
    /// 模型/二进制未找到(whisper-cli 缺失)。
    #[error("model or binary not found: {0}")]
    NotFound(String),
    /// 子进程执行失败。
    #[error("subprocess failed: {0}")]
    Subprocess(String),
    /// 解析输出失败(SRT/JSON 格式异常)。
    #[error("parse output failed: {0}")]
    Parse(String),
    /// 转录超时。
    #[error("transcribe timeout after {0:?}")]
    Timeout(std::time::Duration),
}

/// 转录片段(一句话)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptionSegment {
    /// 识别文本。
    pub text: String,
    /// 起始时间(秒)。
    pub start: f64,
    /// 结束时间(秒)。
    pub end: f64,
    /// 置信度(0.0~1.0),部分后端不提供则 None。
    pub confidence: Option<f32>,
    /// 语言代码(如 "zh"/"en"),自动检测时由后端填充。
    pub language: Option<String>,
}

/// 转录结果(一次 `transcribe` 调用的完整输出)。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Transcription {
    /// 所有片段(按时间顺序)。
    pub segments: Vec<TranscriptionSegment>,
    /// 拼接后的完整文本。
    pub full_text: String,
    /// 检测到的主语言。
    pub detected_language: Option<String>,
}

impl Transcription {
    /// 从纯文本构造(无时间戳/置信度),用于测试与简单后端。
    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            full_text: text.clone(),
            segments: vec![TranscriptionSegment {
                text,
                start: 0.0,
                end: 0.0,
                confidence: None,
                language: None,
            }],
            detected_language: None,
        }
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.full_text.trim().is_empty()
    }
}

/// 流式转录事件。
#[derive(Debug, Clone)]
pub enum TranscriptionEvent {
    /// 部分识别(实时,可能被后续覆盖)。
    Partial(TranscriptionSegment),
    /// 整句完成(最终结果)。
    Final(TranscriptionSegment),
    /// 流结束(携带完整转录)。
    End(Transcription),
}

/// STT 引擎 trait。
///
/// 两种用法:
/// 1. 一次性:`transcribe(frames)` 传入完整音频,返回 `Transcription`。
/// 2. 流式:`transcribe_stream(rx)` 持续消费帧,产出 `TranscriptionEvent`。
#[async_trait]
pub trait SttEngine: Send + Sync {
    /// 后端标识(`"whisper-cpp"` / `"ollama"` / `"noop"`)。
    fn kind(&self) -> &'static str;

    /// 期望的输入音频格式(后端据此协商 capture 格式)。
    fn expected_format(&self) -> AudioFormat;

    /// 一次性转录:传入完整帧序列,返回完整结果。
    async fn transcribe(&self, frames: &[AudioFrame]) -> Result<Transcription, SttError>;

    /// 流式转录:消费帧接收端,返回事件接收端。
    /// 后端内部 spawn task 持续读取 `rx` 并推送事件。
    async fn transcribe_stream(
        &self,
        rx: mpsc::Receiver<AudioFrame>,
    ) -> Result<mpsc::Receiver<TranscriptionEvent>, SttError>;
}

/// 动态分发别名。
pub type DynSttEngine = Arc<dyn SttEngine>;

// ---------------------------------------------------------------------------
// WhisperCppBackend:whisper-cli 子进程框架(不实际执行)。
// ---------------------------------------------------------------------------

/// Whisper.cpp 后端配置。
#[derive(Debug, Clone)]
pub struct WhisperCppConfig {
    /// `whisper-cli` 可执行文件路径。env: `NEBULA_WHISPER_CLI_PATH`。
    pub cli_path: String,
    /// 模型文件路径(ggml-medium.bin 等)。env: `NEBULA_WHISPER_MODEL`。
    pub model_path: String,
    /// 目标语言(`"zh"` / `"en"` / `"auto"`)。env: `NEBULA_WHISPER_LANG`。
    pub language: String,
    /// 是否启用 GPU(offload layers)。env: `NEBULA_WHISPER_GPU=1`。
    pub use_gpu: bool,
    /// 临时 wav 目录。env: `NEBULA_WHISPER_TMP_DIR`。
    pub tmp_dir: String,
}

impl Default for WhisperCppConfig {
    fn default() -> Self {
        Self {
            cli_path: std::env::var("NEBULA_WHISPER_CLI_PATH")
                .unwrap_or_else(|_| "whisper-cli".to_string()),
            model_path: std::env::var("NEBULA_WHISPER_MODEL")
                .unwrap_or_else(|_| "ggml-medium.bin".to_string()),
            language: std::env::var("NEBULA_WHISPER_LANG").unwrap_or_else(|_| "auto".to_string()),
            use_gpu: std::env::var("NEBULA_WHISPER_GPU")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            tmp_dir: std::env::var("NEBULA_WHISPER_TMP_DIR")
                .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().to_string()),
        }
    }
}

/// Whisper.cpp 后端:通过子进程调用 `whisper-cli`。
///
/// **框架实现**:构造与配置完整,`transcribe` 会校验二进制是否存在,
/// 缺失时返回 `NotFound`(不 panic)。实际转录逻辑在 feature gate
/// `voice-whisper` 开启时由扩展模块提供。
pub struct WhisperCppBackend {
    config: WhisperCppConfig,
}

impl WhisperCppBackend {
    pub fn new(config: WhisperCppConfig) -> Self {
        Self { config }
    }

    /// 从环境变量构造。
    pub fn from_env() -> Self {
        Self::new(WhisperCppConfig::default())
    }

    /// 构建 whisper-cli 命令参数(框架,供实际执行使用)。
    #[allow(dead_code)]
    fn build_args(&self, wav_path: &str) -> Vec<String> {
        let mut args = vec![
            "-m".to_string(),
            self.config.model_path.clone(),
            "-f".to_string(),
            wav_path.to_string(),
            "-l".to_string(),
            self.config.language.clone(),
            "-oj".to_string(), // 输出 JSON
        ];
        if self.config.use_gpu {
            args.push("-ngl".to_string());
            args.push("99".to_string());
        }
        args
    }
}

#[async_trait]
impl SttEngine for WhisperCppBackend {
    fn kind(&self) -> &'static str {
        "whisper-cpp"
    }

    fn expected_format(&self) -> AudioFormat {
        // Whisper.cpp 要求 16kHz mono。
        AudioFormat::WHISPER_16K_MONO
    }

    async fn transcribe(&self, _frames: &[AudioFrame]) -> Result<Transcription, SttError> {
        // 框架实现:校验二进制存在性,缺失时返回 NotFound。
        // 实际转录(写 wav → 调子进程 → 解析 JSON)由 voice-whisper feature 提供。
        let which = which_exists(&self.config.cli_path);
        if !which {
            return Err(SttError::NotFound(format!(
                "whisper-cli not found at '{}' (set NEBULA_WHISPER_CLI_PATH); \
                 enable voice-whisper feature for full impl",
                self.config.cli_path
            )));
        }
        // 二进制存在但本框架未启用实际执行:返回 NotImplemented(避免静默空结果)。
        Err(SttError::NotImplemented(
            "whisper-cpp transcription requires voice-whisper feature".into(),
        ))
    }

    async fn transcribe_stream(
        &self,
        mut rx: mpsc::Receiver<AudioFrame>,
    ) -> Result<mpsc::Receiver<TranscriptionEvent>, SttError> {
        // 流式框架:校验后启动 drain task(消费帧避免 channel 堆积),返回 End 事件。
        let which = which_exists(&self.config.cli_path);
        if !which {
            // 仍需 drain rx 以防发送端阻塞。
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            return Err(SttError::NotFound(format!(
                "whisper-cli not found at '{}'",
                self.config.cli_path
            )));
        }
        let (tx, out_rx) = mpsc::channel::<TranscriptionEvent>(16);
        tokio::spawn(async move {
            // 框架:drain 输入帧,实际流式识别需 voice-whisper feature。
            while rx.recv().await.is_some() {}
            let _ = tx
                .send(TranscriptionEvent::End(Transcription::default()))
                .await;
        });
        Ok(out_rx)
    }
}

// ---------------------------------------------------------------------------
// OllamaSttBackend:stub(Ollama 不支持音频模型)。
// ---------------------------------------------------------------------------

/// Ollama STT 后端 stub。
///
/// **Ollama 当前不支持原生语音/音频模型**(仅文本/视觉多模态)。
/// 此 stub 保留接口对称性,始终返回 `NotImplemented`,以备未来 Ollama 扩展
/// 音频 modality 时无需改动 trait 调用方。
pub struct OllamaSttBackend {
    /// Ollama base URL(预留)。
    pub base_url: String,
}

impl OllamaSttBackend {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

#[async_trait]
impl SttEngine for OllamaSttBackend {
    fn kind(&self) -> &'static str {
        "ollama"
    }

    fn expected_format(&self) -> AudioFormat {
        AudioFormat::WHISPER_16K_MONO
    }

    async fn transcribe(&self, _frames: &[AudioFrame]) -> Result<Transcription, SttError> {
        Err(SttError::NotImplemented(
            "Ollama does not support native audio/speech models \
             (only /api/chat, /api/generate, /api/embeddings); \
             use WhisperCppBackend instead"
                .into(),
        ))
    }

    async fn transcribe_stream(
        &self,
        mut rx: mpsc::Receiver<AudioFrame>,
    ) -> Result<mpsc::Receiver<TranscriptionEvent>, SttError> {
        // drain 以防发送端阻塞。
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        Err(SttError::NotImplemented(
            "Ollama streaming STT not supported".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// NoopSttEngine:测试 stub。
// ---------------------------------------------------------------------------

/// 测试用 STT stub。可注入预设转录结果,便于 pipeline 单测。
pub struct NoopSttEngine {
    format: AudioFormat,
    preset: Transcription,
}

impl NoopSttEngine {
    /// 构造返回固定文本的 stub。
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            format: AudioFormat::WHISPER_16K_MONO,
            preset: Transcription::from_text(text),
        }
    }

    /// 构造返回空转录的 stub。
    pub fn empty() -> Self {
        Self {
            format: AudioFormat::WHISPER_16K_MONO,
            preset: Transcription::default(),
        }
    }
}

#[async_trait]
impl SttEngine for NoopSttEngine {
    fn kind(&self) -> &'static str {
        "noop"
    }

    fn expected_format(&self) -> AudioFormat {
        self.format
    }

    async fn transcribe(&self, _frames: &[AudioFrame]) -> Result<Transcription, SttError> {
        Ok(self.preset.clone())
    }

    async fn transcribe_stream(
        &self,
        mut rx: mpsc::Receiver<AudioFrame>,
    ) -> Result<mpsc::Receiver<TranscriptionEvent>, SttError> {
        let preset = self.preset.clone();
        let (tx, out_rx) = mpsc::channel::<TranscriptionEvent>(16);
        tokio::spawn(async move {
            // drain 输入帧,然后推送 Final + End。
            while rx.recv().await.is_some() {}
            if let Some(seg) = preset.segments.first() {
                let _ = tx.send(TranscriptionEvent::Final(seg.clone())).await;
            }
            let _ = tx.send(TranscriptionEvent::End(preset)).await;
        });
        Ok(out_rx)
    }
}

/// 检查可执行文件是否在 PATH 或绝对路径中存在(不执行)。
///
/// pub(crate) 以供 tts 模块复用(LocalTtsBackend 校验 TTS 二进制)。
pub(crate) fn which_exists(cmd: &str) -> bool {
    // 绝对/相对路径直接检查文件。
    if cmd.contains(std::path::MAIN_SEPARATOR) || cmd.ends_with(".exe") {
        return std::path::Path::new(cmd).exists();
    }
    // 否则在 PATH 中查找。
    let path = match std::env::var("PATH") {
        Ok(p) => p,
        Err(_) => return false,
    };
    for dir in path.split(if cfg!(windows) { ';' } else { ':' }) {
        let candidate = std::path::Path::new(dir).join(cmd);
        let with_ext = if cfg!(windows) && !cmd.ends_with(".exe") {
            candidate.with_extension("exe")
        } else {
            candidate.clone()
        };
        if with_ext.is_file() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcription_from_text() {
        let t = Transcription::from_text("你好世界");
        assert_eq!(t.full_text, "你好世界");
        assert_eq!(t.segments.len(), 1);
        assert!(!t.is_empty());
        assert!(Transcription::default().is_empty());
    }

    #[tokio::test]
    async fn test_noop_stt_transcribe() {
        let stt = NoopSttEngine::new("测试文本");
        let frames = vec![AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0)];
        let result = stt.transcribe(&frames).await.expect("transcribe");
        assert_eq!(result.full_text, "测试文本");
    }

    #[tokio::test]
    async fn test_noop_stt_stream() {
        let stt = NoopSttEngine::new("流式测试");
        let (tx, rx) = mpsc::channel::<AudioFrame>(4);
        // 发送一帧后关闭输入。
        let _ = tx
            .send(AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0))
            .await;
        drop(tx);
        let mut out = stt.transcribe_stream(rx).await.expect("stream");
        // 收集事件至 End。
        let mut got_end = false;
        while let Some(ev) = out.recv().await {
            if let TranscriptionEvent::End(t) = ev {
                assert_eq!(t.full_text, "流式测试");
                got_end = true;
            }
        }
        assert!(got_end, "should receive End event");
    }

    #[tokio::test]
    async fn test_ollama_stt_returns_not_implemented() {
        let stt = OllamaSttBackend::new("http://127.0.0.1:11434");
        let frames = vec![AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0)];
        let err = stt.transcribe(&frames).await.unwrap_err();
        assert!(matches!(err, SttError::NotImplemented(_)));
    }

    #[tokio::test]
    async fn test_whisper_backend_missing_binary() {
        // 用不存在的路径,确保返回 NotFound。
        let config = WhisperCppConfig {
            cli_path: "/nonexistent/whisper-cli-xyz".to_string(),
            model_path: "model.bin".to_string(),
            language: "auto".to_string(),
            use_gpu: false,
            tmp_dir: std::env::temp_dir().to_string_lossy().to_string(),
        };
        let stt = WhisperCppBackend::new(config);
        let frames = vec![AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0)];
        let err = stt.transcribe(&frames).await.unwrap_err();
        assert!(matches!(err, SttError::NotFound(_)));
    }

    #[test]
    fn test_whisper_build_args() {
        let config = WhisperCppConfig {
            cli_path: "whisper-cli".to_string(),
            model_path: "ggml-medium.bin".to_string(),
            language: "zh".to_string(),
            use_gpu: true,
            tmp_dir: "/tmp".to_string(),
        };
        let backend = WhisperCppBackend::new(config);
        let args = backend.build_args("audio.wav");
        assert!(args.contains(&"-m".to_string()));
        assert!(args.contains(&"ggml-medium.bin".to_string()));
        assert!(args.contains(&"-l".to_string()));
        assert!(args.contains(&"zh".to_string()));
        assert!(args.contains(&"-ngl".to_string()));
    }
}
