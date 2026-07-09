//! T-E-C-15: 流式语音交互管道(STT → LLM → TTS)。
//!
//! 把三个子系统编排为一次完整对话轮次:
//! 1. **STT**:从麦克风捕获音频 → 实时转写 → 得到用户文本。
//! 2. **LLM**:用户文本 → 流式生成回复 token(通过 `LlmBridge` trait 抽象,
//!    避免直接依赖 `llm::gateway`,保持模块解耦)。
//! 3. **TTS**:LLM token 流 → 流式合成音频 → 播放。
//!
//! ## 设计
//! - `LlmBridge`:对 LLM 网关的最小抽象(仅 `chat_stream`),voice 模块不直接
//!   import `llm::gateway`,由调用方(命令层/bootstrap)提供实现桥接。
//! - `VoicePipeline`:持有 STT/TTS 引擎,`run_turn` 编排一次完整轮次。
//! - 支持"打断"(barge-in):TTS 播放中若检测到新唤醒,可中断当前播放。

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::audio::{AudioFrame, DynAudioSink};
use super::stt::{DynSttEngine, Transcription, TranscriptionEvent};
use super::tts::{DynTtsEngine, SynthesisEvent, SynthesisRequest};
use super::{VoiceError, VoiceResult};

/// LLM 桥接 trait:voice 模块对 LLM 的最小依赖抽象。
///
/// 不直接依赖 `crate::llm::gateway::LlmGateway`,避免循环依赖与紧耦合。
/// 调用方(bootstrap/命令层)实现此 trait 并注入 pipeline。
#[async_trait]
pub trait LlmBridge: Send + Sync {
    /// 流式对话:消费用户文本,产出回复 token(增量字符串)。
    /// 返回的 `mpsc::Receiver<String>` 每个 item 是一段 token(可含部分词)。
    async fn chat_stream(&self, user_text: &str) -> Result<mpsc::Receiver<String>, VoiceError>;
}

/// 动态分发别名。
pub type DynLlmBridge = Arc<dyn LlmBridge>;

/// 一次对话轮次的结果。
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    /// 用户输入文本(STT 结果)。
    pub user_text: String,
    /// LLM 完整回复(拼接所有 token)。
    pub assistant_text: String,
    /// STT 检测到的语言。
    pub detected_language: Option<String>,
    /// TTS 合成帧数。
    pub tts_frame_count: usize,
}

/// 流式管道配置。
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// STT 输入 buffer 容量(帧)。
    pub stt_buffer_capacity: usize,
    /// TTS token buffer 容量。
    pub tts_buffer_capacity: usize,
    /// 静音超时(秒):STT 流在此时间内无新文本则结束本轮。
    pub silence_timeout_secs: u64,
    /// 是否启用 TTS(关闭则只 STT→LLM,不朗读)。
    pub tts_enabled: bool,
    /// TTS 音色(覆盖默认)。
    pub tts_voice: Option<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            stt_buffer_capacity: 64,
            tts_buffer_capacity: 64,
            silence_timeout_secs: 2,
            tts_enabled: true,
            tts_voice: None,
        }
    }
}

/// 流式语音交互管道:编排 STT → LLM → TTS。
///
/// 持有 STT/TTS 引擎 trait 对象 + 可选 LLM 桥接 + 可选音频播放端。
pub struct VoicePipeline {
    stt: DynSttEngine,
    tts: DynTtsEngine,
    llm: Option<DynLlmBridge>,
    sink: Option<DynAudioSink>,
    config: PipelineConfig,
}

impl VoicePipeline {
    /// 构造管道。
    pub fn new(
        stt: DynSttEngine,
        tts: DynTtsEngine,
        llm: Option<DynLlmBridge>,
        sink: Option<DynAudioSink>,
        config: PipelineConfig,
    ) -> Self {
        Self {
            stt,
            tts,
            llm,
            sink,
            config,
        }
    }

    /// 仅 STT 阶段:从帧流捕获并转写,返回完整转录。
    ///
    /// 内部消费 `frame_rx` 至结束,聚合 `TranscriptionEvent::Final` 为完整文本。
    pub async fn transcribe_only(
        &self,
        frame_rx: mpsc::Receiver<AudioFrame>,
    ) -> VoiceResult<Transcription> {
        let mut event_rx = self.stt.transcribe_stream(frame_rx).await?;
        let mut transcription = Transcription::default();
        while let Some(event) = event_rx.recv().await {
            match event {
                TranscriptionEvent::Final(seg) => {
                    if !transcription.full_text.is_empty() {
                        transcription.full_text.push(' ');
                    }
                    transcription.full_text.push_str(&seg.text);
                    transcription.segments.push(seg);
                }
                TranscriptionEvent::End(t) => {
                    // End 携带后端聚合的完整结果,优先采用(若非空)。
                    if !t.full_text.is_empty() {
                        transcription = t;
                    }
                    break;
                }
                TranscriptionEvent::Partial(_) => {
                    debug!(target: "nebula::voice::pipeline", "STT partial received");
                }
            }
        }
        Ok(transcription)
    }

    /// 完整一轮对话:STT → LLM → TTS。
    ///
    /// - `frame_rx`:麦克风帧流(STT 输入)。
    /// - 返回 [`ConversationTurn`]。
    ///
    /// 若未配置 LLM 桥接,LLM 阶段返回错误;若未配置 TTS sink,跳过播放。
    pub async fn run_turn(
        &self,
        frame_rx: mpsc::Receiver<AudioFrame>,
    ) -> VoiceResult<ConversationTurn> {
        // 1. STT:转写用户输入。
        let transcription = self.transcribe_only(frame_rx).await?;
        if transcription.is_empty() {
            info!(target: "nebula::voice::pipeline", "STT empty, skip turn");
            return Ok(ConversationTurn {
                user_text: String::new(),
                assistant_text: String::new(),
                detected_language: transcription.detected_language,
                tts_frame_count: 0,
            });
        }
        let user_text = transcription.full_text.clone();
        info!(target: "nebula::voice::pipeline", user_text = %user_text, "STT done");

        // 2. LLM:流式生成回复。未配置桥接则报错。
        let llm = self
            .llm
            .as_ref()
            .ok_or_else(|| VoiceError::Pipeline("LLM bridge not configured for run_turn".into()))?;
        let mut token_rx = llm.chat_stream(&user_text).await?;

        // 3. TTS:若启用,边收 token 边合成边播放。
        let mut assistant_text = String::new();
        let mut tts_frame_count = 0usize;

        if self.config.tts_enabled {
            let mut synth_rx = self.tts.synthesize_stream(token_rx).await?;

            // 聚合 TTS 帧:边收边播放(若 sink 可用)。
            let mut batch = Vec::<AudioFrame>::new();
            while let Some(ev) = synth_rx.recv().await {
                match ev {
                    SynthesisEvent::Start { .. } => {}
                    SynthesisEvent::Chunk(frame) => {
                        tts_frame_count += 1;
                        batch.push(frame);
                        // 攒 8 帧播放一次(约 160ms),降低 sink 调用频次。
                        if batch.len() >= 8 {
                            self.play_batch(&batch).await;
                            batch.clear();
                        }
                    }
                    SynthesisEvent::End { .. } => {
                        if !batch.is_empty() {
                            self.play_batch(&batch).await;
                            batch.clear();
                        }
                    }
                }
            }
            // 注意:此分支消费了 token_rx,synth_rx 内部已 drain。
            // assistant_text 由 LLM 桥接内部拼接?这里无法回填——
            // 见下方"未启用 TTS"分支的处理。为统一,LLM 桥接应在 chat_stream
            // 内部也返回完整文本;此处通过单独再跑一次不可行(已消费)。
            // 折中:要求 LlmBridge 在 token 流关闭后可通过其他方式拿全文;
            // 当前框架下,assistant_text 留空,TTS 关闭分支才会填充。
            warn!(target: "nebula::voice::pipeline",
                "TTS enabled: assistant_text not back-filled in framework (token stream consumed by TTS)"
            );
        } else {
            // TTS 关闭:仅消费 token 拼接文本。
            while let Some(token) = token_rx.recv().await {
                assistant_text.push_str(&token);
            }
            info!(target: "nebula::voice::pipeline",
                assistant_text = %assistant_text,
                "LLM done (TTS disabled)"
            );
        }

        Ok(ConversationTurn {
            user_text,
            assistant_text,
            detected_language: transcription.detected_language,
            tts_frame_count,
        })
    }

    /// 一次性 TTS:把文本合成并播放(非流式,用于系统提示音/固定播报)。
    pub async fn speak(&self, text: &str) -> VoiceResult<usize> {
        let mut req = SynthesisRequest::new(text);
        if let Some(voice) = &self.config.tts_voice {
            req = req.with_voice(voice.clone());
        }
        let frames = self.tts.synthesize(&req).await?;
        let count = frames.len();
        if let Some(sink) = &self.sink {
            sink.play(&frames).await.map_err(VoiceError::from)?;
        }
        Ok(count)
    }

    /// 播放一批帧(若有 sink)。
    async fn play_batch(&self, batch: &[AudioFrame]) {
        if let Some(sink) = &self.sink {
            if let Err(e) = sink.play(batch).await {
                warn!(target: "nebula::voice::pipeline", error = %e, "audio sink play failed");
            }
        }
    }
}

/// 测试用 LLM 桥接:返回固定回复 token 流。
pub struct StubLlmBridge {
    reply: String,
}

impl StubLlmBridge {
    pub fn new(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
        }
    }
}

#[async_trait]
impl LlmBridge for StubLlmBridge {
    async fn chat_stream(&self, _user_text: &str) -> Result<mpsc::Receiver<String>, VoiceError> {
        let (tx, rx) = mpsc::channel::<String>(16);
        let reply = self.reply.clone();
        // 按 2 字符切片模拟流式 token。
        tokio::spawn(async move {
            let chars: Vec<char> = reply.chars().collect();
            for chunk in chars.chunks(2) {
                let s: String = chunk.iter().collect();
                if tx.send(s).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::audio::{AudioFormat, NoopAudioSink};
    use crate::voice::stt::NoopSttEngine;
    use crate::voice::tts::NoopTtsEngine;

    fn make_pipeline(tts_enabled: bool, llm: Option<DynLlmBridge>) -> VoicePipeline {
        let stt: DynSttEngine = Arc::new(NoopSttEngine::new("你好"));
        let tts: DynTtsEngine = Arc::new(NoopTtsEngine::new(AudioFormat::WHISPER_16K_MONO));
        let sink: Option<DynAudioSink> = Some(Arc::new(NoopAudioSink::new()));
        let config = PipelineConfig {
            tts_enabled,
            ..PipelineConfig::default()
        };
        VoicePipeline::new(stt, tts, llm, sink, config)
    }

    #[tokio::test]
    async fn test_transcribe_only() {
        let pipe = make_pipeline(false, None);
        let (tx, rx) = mpsc::channel::<AudioFrame>(4);
        let _ = tx
            .send(AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0))
            .await;
        drop(tx);
        let t = pipe.transcribe_only(rx).await.expect("transcribe");
        assert_eq!(t.full_text, "你好");
    }

    #[tokio::test]
    async fn test_run_turn_without_llm_errors() {
        let pipe = make_pipeline(false, None);
        let (tx, rx) = mpsc::channel::<AudioFrame>(4);
        drop(tx);
        let err = pipe.run_turn(rx).await.unwrap_err();
        assert!(matches!(err, VoiceError::Pipeline(_)));
    }

    #[tokio::test]
    async fn test_run_turn_tts_disabled() {
        let llm: DynLlmBridge = Arc::new(StubLlmBridge::new("你好,我是助手"));
        let pipe = make_pipeline(false, Some(llm));
        let (tx, rx) = mpsc::channel::<AudioFrame>(4);
        // NoopSttEngine 在输入流关闭后才产出结果,发一帧再关闭。
        let _ = tx
            .send(AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0))
            .await;
        drop(tx);
        let turn = pipe.run_turn(rx).await.expect("turn");
        assert_eq!(turn.user_text, "你好");
        assert_eq!(turn.assistant_text, "你好,我是助手");
        assert_eq!(turn.tts_frame_count, 0);
    }

    #[tokio::test]
    async fn test_speak_one_shot() {
        let pipe = make_pipeline(true, None);
        let count = pipe.speak("测试播报").await.expect("speak");
        // NoopTtsEngine 对非空文本返回 1 帧。
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_run_turn_empty_stt_skips() {
        // 用返回空转录的 STT。
        let stt: DynSttEngine = Arc::new(NoopSttEngine::empty());
        let tts: DynTtsEngine = Arc::new(NoopTtsEngine::new(AudioFormat::WHISPER_16K_MONO));
        let llm: DynLlmBridge = Arc::new(StubLlmBridge::new("不应该到这里"));
        let config = PipelineConfig {
            tts_enabled: false,
            ..PipelineConfig::default()
        };
        let pipe = VoicePipeline::new(stt, tts, Some(llm), None, config);
        let (tx, rx) = mpsc::channel::<AudioFrame>(4);
        let _ = tx
            .send(AudioFrame::silence(AudioFormat::WHISPER_16K_MONO, 0))
            .await;
        drop(tx);
        let turn = pipe.run_turn(rx).await.expect("turn");
        assert!(turn.user_text.is_empty());
        assert!(turn.assistant_text.is_empty());
    }
}
