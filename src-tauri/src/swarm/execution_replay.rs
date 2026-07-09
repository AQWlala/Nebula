//! T-E-S-14: 蜂群任务执行回放 — 录制与回放蜂群任务执行过程。
//!
//! 本模块提供蜂群执行过程的帧式录制与回放能力,支持逐步回放、
//! 快进、跳转,便于调试与复盘。
//!
//! ## 核心类型
//!
//! - [`ExecutionRecording`] — 执行录制(帧序列 + 元数据)
//! - [`ReplayFrame`] — 单个回放帧(事件快照)
//! - [`ReplayPlayer`] — 回放播放器(状态机: 播放 / 暂停 / 停止 / 完成)
//! - [`ReplaySummary`] — 录制摘要(高亮 + 统计)
//! - [`ReplayHighlight`] / [`HighlightType`] — 关键帧高亮
//!
//! ## 数据流
//!
//! ```text
//! SwarmEvent ──add_frame()──┐
//!                            ▼
//!                   ExecutionRecording
//!                            │
//!              ┌─────────────┼─────────────┐
//!              ▼             ▼             ▼
//!         to_json()    ReplayPlayer    ReplaySummary
//!         (持久化)     (回放控制)      (摘要统计)
//! ```

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info, instrument};

// ---------------------------------------------------------------------------
// PlayerState — 播放器状态枚举
// ---------------------------------------------------------------------------

/// T-E-S-14: 回放播放器状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerState {
    /// 播放中。
    Playing,
    /// 已暂停。
    Paused,
    /// 已停止(重置到首帧)。
    Stopped,
    /// 已播放完毕(到达最后一帧)。
    Finished,
}

// ---------------------------------------------------------------------------
// HighlightType — 高亮类型枚举
// ---------------------------------------------------------------------------

/// T-E-S-14: 回放高亮类型 — 标记关键帧的语义类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightType {
    /// 错误。
    Error,
    /// 警告。
    Warning,
    /// 里程碑。
    Milestone,
    /// 决策点。
    Decision,
    /// 工具调用。
    ToolCall,
    /// 输出。
    Output,
}

// ---------------------------------------------------------------------------
// ReplayFrame — 单个回放帧
// ---------------------------------------------------------------------------

/// T-E-S-14: 单个回放帧 — 记录某一时刻的事件快照。
///
/// 每帧对应蜂群执行过程中的一个事件(agent 启动 / 工具调用 / 输出等),
/// 携带时间戳、耗时、事件类型与可选的状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayFrame {
    /// 帧索引(从 0 开始)。
    pub frame_index: u32,
    /// 帧产生时间(UTC)。
    pub timestamp: DateTime<Utc>,
    /// 距录制开始的耗时(毫秒)。
    pub elapsed_ms: u64,
    /// 事件类型名(如 "AgentStarted"、"AgentToolCall")。
    pub event_type: String,
    /// 关联的 agent / 节点 ID(无关联时为 None)。
    #[serde(default)]
    pub node_id: Option<String>,
    /// 动作描述(如 "read_file"、"delegate_subtask")。
    pub action: String,
    /// 状态快照(可选 JSON,用于回放时恢复画布)。
    #[serde(default)]
    pub state_snapshot: Option<Value>,
    /// 截图路径(可选,用于可视化回放)。
    #[serde(default)]
    pub screenshot_path: Option<String>,
}

impl ReplayFrame {
    /// 创建一个新的 `ReplayFrame`,默认 `elapsed_ms = 0`,无快照/截图。
    pub fn new(frame_index: u32, event_type: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            frame_index,
            timestamp: Utc::now(),
            elapsed_ms: 0,
            event_type: event_type.into(),
            node_id: None,
            action: action.into(),
            state_snapshot: None,
            screenshot_path: None,
        }
    }

    /// Builder: 设置时间戳。
    pub fn with_timestamp(mut self, ts: DateTime<Utc>) -> Self {
        self.timestamp = ts;
        self
    }

    /// Builder: 设置耗时(毫秒)。
    pub fn with_elapsed_ms(mut self, ms: u64) -> Self {
        self.elapsed_ms = ms;
        self
    }

    /// Builder: 设置关联节点 ID。
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }

    /// Builder: 设置状态快照。
    pub fn with_state_snapshot(mut self, snapshot: Value) -> Self {
        self.state_snapshot = Some(snapshot);
        self
    }

    /// Builder: 设置截图路径。
    pub fn with_screenshot(mut self, path: impl Into<String>) -> Self {
        self.screenshot_path = Some(path.into());
        self
    }
}

// ---------------------------------------------------------------------------
// ExecutionRecording — 执行录制
// ---------------------------------------------------------------------------

/// T-E-S-14: 执行录制 — 一段蜂群任务执行过程的帧序列。
///
/// 由 `add_frame()` 逐帧追加,`finish()` 结束录制。可序列化为 JSON
/// 持久化,或交给 [`ReplayPlayer`] 回放。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecording {
    /// 会话 ID(标识本次蜂群执行会话)。
    pub session_id: String,
    /// 录制开始时间(UTC)。
    pub started_at: DateTime<Utc>,
    /// 录制结束时间(UTC,`None` 表示尚未结束)。
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    /// 帧序列(按时间顺序)。
    #[serde(default)]
    pub frames: Vec<ReplayFrame>,
    /// 附加元数据(键值对)。
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl ExecutionRecording {
    /// 创建一个新的执行录制,`started_at` 设为当前时间。
    #[instrument(target = "nebula.swarm.replay", skip(session_id))]
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            started_at: Utc::now(),
            finished_at: None,
            frames: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// 追加一个回放帧。
    pub fn add_frame(&mut self, frame: ReplayFrame) {
        debug!(
            target: "nebula.swarm.replay",
            index = frame.frame_index,
            event_type = %frame.event_type,
            "添加回放帧"
        );
        self.frames.push(frame);
    }

    /// 结束录制,将 `finished_at` 设为当前时间。
    pub fn finish(&mut self) {
        self.finished_at = Some(Utc::now());
        info!(
            target: "nebula.swarm.replay",
            frames = self.frames.len(),
            "录制结束"
        );
    }

    /// 返回录制总时长。
    ///
    /// 优先使用 `finished_at - started_at`;若尚未结束则回退到最后一帧的
    /// `elapsed_ms`;无帧时返回 `Duration::ZERO`。
    pub fn duration(&self) -> Duration {
        if let Some(finished) = self.finished_at {
            (finished - self.started_at)
                .to_std()
                .unwrap_or(Duration::ZERO)
        } else if let Some(last) = self.frames.last() {
            Duration::from_millis(last.elapsed_ms)
        } else {
            Duration::ZERO
        }
    }

    /// 返回帧数。
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// 序列化为 JSON 字符串。
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| anyhow!("序列化 ExecutionRecording 失败: {e}"))
    }

    /// 从 JSON 字符串反序列化。
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| anyhow!("反序列化 ExecutionRecording 失败: {e}"))
    }
}

// ---------------------------------------------------------------------------
// ReplayPlayer — 回放播放器
// ---------------------------------------------------------------------------

/// T-E-S-14: 回放播放器 — 基于 [`ExecutionRecording`] 的状态机式回放控制。
///
/// 支持播放 / 暂停 / 停止 / 跳转 / 逐步前进后退,以及变速回放。
///
/// ## 状态转换
///
/// ```text
/// Stopped ──play()──► Playing ──pause()──► Paused ──play()──► Playing
///    ▲                  │                                       │
///    └──── stop() ──────┴───────────────────────────────────────┘
///                       │
///                  step_forward() 到末尾
///                       ▼
///                    Finished
/// ```
pub struct ReplayPlayer {
    /// 关联的录制。
    recording: ExecutionRecording,
    /// 当前帧索引。
    current_index: usize,
    /// 播放器状态。
    state: PlayerState,
    /// 播放速度(0.5 / 1.0 / 2.0 / 4.0)。
    speed: f32,
}

impl ReplayPlayer {
    /// 用一段录制构造播放器,初始状态为 `Stopped`,速度 1.0x。
    #[instrument(target = "nebula.swarm.replay", skip(recording))]
    pub fn new(recording: ExecutionRecording) -> Self {
        let frames = recording.frames.len();
        debug!(
            target: "nebula.swarm.replay",
            frames,
            "构造 ReplayPlayer"
        );
        Self {
            recording,
            current_index: 0,
            state: PlayerState::Stopped,
            speed: 1.0,
        }
    }

    /// 开始 / 恢复播放。
    ///
    /// 仅在 `Stopped` / `Paused` 状态下生效;`Finished` 状态需先 `stop()` 重置。
    #[instrument(target = "nebula.swarm.replay", skip(self))]
    pub fn play(&mut self) {
        if self.state == PlayerState::Finished {
            debug!(target: "nebula.swarm.replay", "play: 已完成,忽略");
            return;
        }
        self.state = PlayerState::Playing;
        info!(
            target: "nebula.swarm.replay",
            index = self.current_index,
            "回放开始/恢复"
        );
    }

    /// 暂停播放(仅在 `Playing` 状态下生效)。
    #[instrument(target = "nebula.swarm.replay", skip(self))]
    pub fn pause(&mut self) {
        if self.state == PlayerState::Playing {
            self.state = PlayerState::Paused;
            debug!(target: "nebula.swarm.replay", "回放暂停");
        }
    }

    /// 停止播放并重置到首帧。
    #[instrument(target = "nebula.swarm.replay", skip(self))]
    pub fn stop(&mut self) {
        self.state = PlayerState::Stopped;
        self.current_index = 0;
        debug!(target: "nebula.swarm.replay", "回放停止,重置到首帧");
    }

    /// 跳转到指定帧(自动 clamp 到有效范围)。
    pub fn seek(&mut self, frame_index: usize) {
        let total = self.recording.frames.len();
        if total == 0 {
            self.current_index = 0;
            return;
        }
        self.current_index = frame_index.min(total - 1);
        debug!(
            target: "nebula.swarm.replay",
            index = self.current_index,
            total,
            "跳转"
        );
    }

    /// 前进一步,返回新帧的引用。
    ///
    /// 已到末帧时将状态置为 `Finished` 并返回 `None`。
    pub fn step_forward(&mut self) -> Option<&ReplayFrame> {
        let total = self.recording.frames.len();
        if total == 0 {
            return None;
        }
        if self.current_index + 1 < total {
            self.current_index += 1;
            debug!(
                target: "nebula.swarm.replay",
                index = self.current_index,
                "step_forward"
            );
            Some(&self.recording.frames[self.current_index])
        } else {
            // 已到最后一帧,标记为完成。
            self.state = PlayerState::Finished;
            debug!(target: "nebula.swarm.replay", "step_forward 到达末尾,回放完成");
            None
        }
    }

    /// 后退一步,返回新帧的引用。
    ///
    /// 已在首帧时返回 `None`。
    pub fn step_backward(&mut self) -> Option<&ReplayFrame> {
        if self.current_index > 0 {
            self.current_index -= 1;
            debug!(
                target: "nebula.swarm.replay",
                index = self.current_index,
                "step_backward"
            );
            Some(&self.recording.frames[self.current_index])
        } else {
            debug!(target: "nebula.swarm.replay", "step_backward 已在首帧");
            None
        }
    }

    /// 设置播放速度(推荐 0.5 / 1.0 / 2.0 / 4.0,非正数回退到 1.0)。
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = if speed > 0.0 { speed } else { 1.0 };
        debug!(target: "nebula.swarm.replay", speed = self.speed, "设置回放速度");
    }

    /// 返回当前帧的引用。
    pub fn current_frame(&self) -> Option<&ReplayFrame> {
        self.recording.frames.get(self.current_index)
    }

    /// 返回当前帧索引。
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// 是否正在播放。
    pub fn is_playing(&self) -> bool {
        self.state == PlayerState::Playing
    }

    /// 返回播放进度(0.0-1.0)。
    ///
    /// 空录制返回 0.0;单帧录制返回 1.0;否则 `current_index / (total - 1)`。
    pub fn progress(&self) -> f32 {
        let total = self.recording.frames.len();
        if total == 0 {
            return 0.0;
        }
        if total == 1 {
            return 1.0;
        }
        (self.current_index as f32) / ((total - 1) as f32)
    }

    /// 返回总帧数。
    pub fn total_frames(&self) -> usize {
        self.recording.frames.len()
    }

    /// 返回当前播放器状态。
    pub fn state(&self) -> PlayerState {
        self.state
    }

    /// 返回当前播放速度。
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// 返回关联录制的引用。
    pub fn recording(&self) -> &ExecutionRecording {
        &self.recording
    }
}

// ---------------------------------------------------------------------------
// ReplayHighlight — 回放高亮
// ---------------------------------------------------------------------------

/// T-E-S-14: 回放高亮 — 标记录制中的关键帧。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayHighlight {
    /// 高亮所在帧索引。
    pub frame_index: usize,
    /// 高亮类型。
    pub highlight_type: HighlightType,
    /// 描述文本。
    pub description: String,
    /// 关联节点 ID(可选)。
    #[serde(default)]
    pub node_id: Option<String>,
}

impl ReplayHighlight {
    /// 创建一个新的 `ReplayHighlight`。
    pub fn new(
        frame_index: usize,
        highlight_type: HighlightType,
        description: impl Into<String>,
    ) -> Self {
        Self {
            frame_index,
            highlight_type,
            description: description.into(),
            node_id: None,
        }
    }

    /// Builder: 设置关联节点 ID。
    pub fn with_node_id(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = Some(node_id.into());
        self
    }
}

// ---------------------------------------------------------------------------
// ReplaySummary — 录制摘要
// ---------------------------------------------------------------------------

/// T-E-S-14: 录制摘要 — 从 [`ExecutionRecording`] 生成的统计与高亮。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaySummary {
    /// 会话 ID。
    pub session_id: String,
    /// 总帧数。
    pub total_frames: usize,
    /// 总时长(毫秒)。
    pub duration_ms: u64,
    /// 关键帧高亮列表。
    #[serde(default)]
    pub highlights: Vec<ReplayHighlight>,
    /// 错误数。
    #[serde(default)]
    pub errors_count: u32,
    /// 警告数。
    #[serde(default)]
    pub warnings_count: u32,
    /// 工具调用数。
    #[serde(default)]
    pub tool_calls_count: u32,
}

impl ReplaySummary {
    /// 从录制生成摘要,自动扫描帧提取高亮与统计。
    pub fn from_recording(recording: &ExecutionRecording) -> Self {
        let mut highlights = Vec::new();
        let mut errors_count = 0u32;
        let mut warnings_count = 0u32;
        let mut tool_calls_count = 0u32;

        for (idx, frame) in recording.frames.iter().enumerate() {
            let event_lower = frame.event_type.to_lowercase();
            let action_lower = frame.action.to_lowercase();

            // 根据事件类型 / 动作推断高亮类别。
            let htype = if event_lower.contains("error") || action_lower.contains("error") {
                errors_count += 1;
                Some(HighlightType::Error)
            } else if event_lower.contains("warning") || action_lower.contains("warning") {
                warnings_count += 1;
                Some(HighlightType::Warning)
            } else if event_lower.contains("milestone") || action_lower.contains("milestone") {
                Some(HighlightType::Milestone)
            } else if event_lower.contains("decision") || action_lower.contains("decision") {
                Some(HighlightType::Decision)
            } else if event_lower.contains("tool") || action_lower.contains("tool") {
                tool_calls_count += 1;
                Some(HighlightType::ToolCall)
            } else if event_lower.contains("output") || action_lower.contains("output") {
                Some(HighlightType::Output)
            } else {
                None
            };

            if let Some(ht) = htype {
                highlights.push(ReplayHighlight {
                    frame_index: idx,
                    highlight_type: ht,
                    description: format!("{}: {}", frame.event_type, frame.action),
                    node_id: frame.node_id.clone(),
                });
            }
        }

        Self {
            session_id: recording.session_id.clone(),
            total_frames: recording.frame_count(),
            duration_ms: recording.duration().as_millis() as u64,
            highlights,
            errors_count,
            warnings_count,
            tool_calls_count,
        }
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 辅助:构造指定数量的测试帧。
    fn make_frames(count: usize) -> Vec<ReplayFrame> {
        let base = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        (0..count)
            .map(|i| {
                ReplayFrame::new(i as u32, "AgentStarted", "start")
                    .with_timestamp(base)
                    .with_elapsed_ms((i * 100) as u64)
            })
            .collect()
    }

    /// 辅助:构造一个含 `count` 帧的录制(已结束)。
    fn make_recording(count: usize) -> ExecutionRecording {
        let mut rec = ExecutionRecording::new("test-session");
        for frame in make_frames(count) {
            rec.add_frame(frame);
        }
        rec.finish();
        rec
    }

    // ===================================================================
    // ExecutionRecording 测试
    // ===================================================================

    // 1. 添加帧
    #[test]
    fn test_recording_add_frame() {
        let mut rec = ExecutionRecording::new("s-1");
        assert_eq!(rec.frame_count(), 0);
        assert!(rec.frames.is_empty());

        rec.add_frame(ReplayFrame::new(0, "AgentStarted", "start"));
        rec.add_frame(ReplayFrame::new(1, "AgentCompleted", "complete"));
        assert_eq!(rec.frame_count(), 2);
        assert_eq!(rec.frames[0].frame_index, 0);
        assert_eq!(rec.frames[1].frame_index, 1);
        assert_eq!(rec.frames[0].event_type, "AgentStarted");
    }

    // 2. 结束录制
    #[test]
    fn test_recording_finish() {
        let mut rec = ExecutionRecording::new("s-2");
        assert!(rec.finished_at.is_none());
        rec.finish();
        assert!(rec.finished_at.is_some());
        // 再次 finish 覆盖时间戳,仍为 Some。
        rec.finish();
        assert!(rec.finished_at.is_some());
    }

    // 3. 时长计算(基于 finished_at)
    #[test]
    fn test_recording_duration_with_finished_at() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 5).unwrap();
        let mut rec = ExecutionRecording::new("s-3");
        rec.started_at = start;
        rec.finished_at = Some(end);
        // 5 秒 = 5000 毫秒
        assert_eq!(rec.duration().as_millis(), 5000);
    }

    // 4. 时长计算(未结束,回退到最后一帧 elapsed_ms)
    #[test]
    fn test_recording_duration_unfinished_falls_back_to_last_frame() {
        let mut rec = ExecutionRecording::new("s-4");
        rec.add_frame(ReplayFrame::new(0, "E", "a").with_elapsed_ms(100));
        rec.add_frame(ReplayFrame::new(1, "E", "b").with_elapsed_ms(350));
        // 未 finish,应回退到最后一帧的 elapsed_ms = 350
        assert_eq!(rec.duration().as_millis(), 350);
    }

    // 5. 帧数
    #[test]
    fn test_recording_frame_count() {
        let rec = make_recording(5);
        assert_eq!(rec.frame_count(), 5);
        let empty = ExecutionRecording::new("empty");
        assert_eq!(empty.frame_count(), 0);
    }

    // ===================================================================
    // ReplayFrame 序列化往返
    // ===================================================================

    // 6. ReplayFrame JSON 序列化往返
    #[test]
    fn test_replay_frame_serde_round_trip() {
        let frame = ReplayFrame::new(3, "AgentToolCall", "read_file")
            .with_elapsed_ms(500)
            .with_node_id("agent-coder")
            .with_state_snapshot(serde_json::json!({"status": "running"}))
            .with_screenshot("/tmp/shot.png");

        let json = serde_json::to_string(&frame).expect("序列化");
        let de: ReplayFrame = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(de.frame_index, 3);
        assert_eq!(de.event_type, "AgentToolCall");
        assert_eq!(de.action, "read_file");
        assert_eq!(de.elapsed_ms, 500);
        assert_eq!(de.node_id.as_deref(), Some("agent-coder"));
        assert_eq!(
            de.state_snapshot,
            Some(serde_json::json!({"status": "running"}))
        );
        assert_eq!(de.screenshot_path.as_deref(), Some("/tmp/shot.png"));
    }

    // ===================================================================
    // ExecutionRecording JSON 序列化往返
    // ===================================================================

    // 7. ExecutionRecording JSON 序列化往返
    #[test]
    fn test_recording_json_round_trip() {
        let mut rec = ExecutionRecording::new("json-session");
        rec.add_frame(
            ReplayFrame::new(0, "AgentStarted", "start")
                .with_elapsed_ms(0)
                .with_node_id("agent-1"),
        );
        rec.add_frame(ReplayFrame::new(1, "AgentCompleted", "done").with_elapsed_ms(200));
        rec.metadata.insert("task_id".into(), "t-1".into());
        rec.finish();

        let json = rec.to_json().expect("to_json");
        let de = ExecutionRecording::from_json(&json).expect("from_json");
        assert_eq!(de.session_id, "json-session");
        assert_eq!(de.frame_count(), 2);
        assert_eq!(de.frames[0].event_type, "AgentStarted");
        assert_eq!(de.frames[1].elapsed_ms, 200);
        assert_eq!(de.frames[0].node_id.as_deref(), Some("agent-1"));
        assert!(de.finished_at.is_some());
        assert_eq!(de.metadata.get("task_id").map(|s| s.as_str()), Some("t-1"));
    }

    // ===================================================================
    // ReplayPlayer play / pause / stop 测试
    // ===================================================================

    // 8. play / pause 状态转换
    #[test]
    fn test_player_play_pause() {
        let rec = make_recording(3);
        let mut player = ReplayPlayer::new(rec);
        assert_eq!(player.state(), PlayerState::Stopped);
        assert!(!player.is_playing());

        player.play();
        assert_eq!(player.state(), PlayerState::Playing);
        assert!(player.is_playing());

        player.pause();
        assert_eq!(player.state(), PlayerState::Paused);
        assert!(!player.is_playing());

        // 再次 play 恢复
        player.play();
        assert_eq!(player.state(), PlayerState::Playing);
        assert!(player.is_playing());
    }

    // 9. stop 重置
    #[test]
    fn test_player_stop_resets() {
        let rec = make_recording(3);
        let mut player = ReplayPlayer::new(rec);
        player.play();
        player.seek(2);
        assert_eq!(player.current_index(), 2);

        player.stop();
        assert_eq!(player.state(), PlayerState::Stopped);
        assert_eq!(player.current_index(), 0);
    }

    // ===================================================================
    // ReplayPlayer seek 测试
    // ===================================================================

    // 10. seek 跳转(含 clamp)
    #[test]
    fn test_player_seek() {
        let rec = make_recording(5);
        let mut player = ReplayPlayer::new(rec);
        assert_eq!(player.current_index(), 0);

        player.seek(3);
        assert_eq!(player.current_index(), 3);
        assert_eq!(player.current_frame().unwrap().frame_index, 3);

        // 超出范围 clamp 到最后一帧
        player.seek(100);
        assert_eq!(player.current_index(), 4);
    }

    // ===================================================================
    // ReplayPlayer step_forward / step_backward 测试
    // ===================================================================

    // 11. step_forward 逐步前进
    #[test]
    fn test_player_step_forward() {
        let rec = make_recording(3);
        let mut player = ReplayPlayer::new(rec);
        // 初始在帧 0
        assert_eq!(player.current_index(), 0);

        // 前进到帧 1
        let f1 = player.step_forward().expect("应有帧 1");
        assert_eq!(f1.frame_index, 1);
        assert_eq!(player.current_index(), 1);

        // 前进到帧 2
        let f2 = player.step_forward().expect("应有帧 2");
        assert_eq!(f2.frame_index, 2);
        assert_eq!(player.current_index(), 2);

        // 已到末帧,step_forward 返回 None 并标记 Finished
        assert!(player.step_forward().is_none());
        assert_eq!(player.state(), PlayerState::Finished);
    }

    // 12. step_backward 逐步后退
    #[test]
    fn test_player_step_backward() {
        let rec = make_recording(3);
        let mut player = ReplayPlayer::new(rec);
        player.seek(2);
        assert_eq!(player.current_index(), 2);

        // 后退到帧 1
        let f1 = player.step_backward().expect("应有帧 1");
        assert_eq!(f1.frame_index, 1);

        // 后退到帧 0
        let f0 = player.step_backward().expect("应有帧 0");
        assert_eq!(f0.frame_index, 0);

        // 已在首帧,返回 None
        assert!(player.step_backward().is_none());
        assert_eq!(player.current_index(), 0);
    }

    // ===================================================================
    // set_speed 测试
    // ===================================================================

    // 13. set_speed 设置播放速度
    #[test]
    fn test_player_set_speed() {
        let rec = make_recording(2);
        let mut player = ReplayPlayer::new(rec);
        // 默认 1.0x
        assert!((player.speed() - 1.0).abs() < f32::EPSILON);

        player.set_speed(2.0);
        assert!((player.speed() - 2.0).abs() < f32::EPSILON);

        player.set_speed(0.5);
        assert!((player.speed() - 0.5).abs() < f32::EPSILON);

        player.set_speed(4.0);
        assert!((player.speed() - 4.0).abs() < f32::EPSILON);

        // 非正数回退到 1.0
        player.set_speed(-1.0);
        assert!((player.speed() - 1.0).abs() < f32::EPSILON);
        player.set_speed(0.0);
        assert!((player.speed() - 1.0).abs() < f32::EPSILON);
    }

    // ===================================================================
    // progress 计算
    // ===================================================================

    // 14. progress 进度计算
    #[test]
    fn test_player_progress() {
        // 5 帧:索引 0→0.0, 2→0.5, 4→1.0
        let rec = make_recording(5);
        let mut player = ReplayPlayer::new(rec);
        assert!((player.progress() - 0.0).abs() < f32::EPSILON);

        player.seek(2);
        assert!((player.progress() - 0.5).abs() < f32::EPSILON);

        player.seek(4);
        assert!((player.progress() - 1.0).abs() < f32::EPSILON);

        // 单帧:progress = 1.0
        let single = make_recording(1);
        let p2 = ReplayPlayer::new(single);
        assert!((p2.progress() - 1.0).abs() < f32::EPSILON);
    }

    // ===================================================================
    // PlayerState 转换
    // ===================================================================

    // 15. PlayerState 完整状态转换
    #[test]
    fn test_player_state_transitions() {
        let rec = make_recording(2);
        let mut player = ReplayPlayer::new(rec);

        // Stopped → Playing
        assert_eq!(player.state(), PlayerState::Stopped);
        player.play();
        assert_eq!(player.state(), PlayerState::Playing);

        // Playing → Paused
        player.pause();
        assert_eq!(player.state(), PlayerState::Paused);

        // Paused → Playing
        player.play();
        assert_eq!(player.state(), PlayerState::Playing);

        // Playing → Stopped
        player.stop();
        assert_eq!(player.state(), PlayerState::Stopped);

        // 走到末尾 → Finished
        player.seek(1);
        player.step_forward();
        assert_eq!(player.state(), PlayerState::Finished);

        // Finished 时 play 无效(需先 stop)
        player.play();
        assert_eq!(player.state(), PlayerState::Finished);

        // stop 后可重新 play
        player.stop();
        assert_eq!(player.state(), PlayerState::Stopped);
        player.play();
        assert_eq!(player.state(), PlayerState::Playing);
    }

    // ===================================================================
    // ReplayHighlight 构建
    // ===================================================================

    // 16. ReplayHighlight 构建与 builder
    #[test]
    fn test_replay_highlight_builder() {
        let h =
            ReplayHighlight::new(5, HighlightType::Error, "执行失败").with_node_id("agent-coder");
        assert_eq!(h.frame_index, 5);
        assert_eq!(h.highlight_type, HighlightType::Error);
        assert_eq!(h.description, "执行失败");
        assert_eq!(h.node_id.as_deref(), Some("agent-coder"));

        // HighlightType 序列化(snake_case)
        let cases = [
            (HighlightType::Error, "error"),
            (HighlightType::Warning, "warning"),
            (HighlightType::Milestone, "milestone"),
            (HighlightType::Decision, "decision"),
            (HighlightType::ToolCall, "tool_call"),
            (HighlightType::Output, "output"),
        ];
        for (ht, expected) in cases {
            let s = serde_json::to_string(&ht).expect("序列化");
            assert!(s.contains(expected), "期望 {expected} 出现在 {s}");
            let de: HighlightType = serde_json::from_str(&s).expect("反序列化");
            assert_eq!(de, ht);
        }
    }

    // ===================================================================
    // ReplaySummary 从录制生成
    // ===================================================================

    // 17. ReplaySummary 从录制生成摘要
    #[test]
    fn test_replay_summary_from_recording() {
        let mut rec = ExecutionRecording::new("summary-session");
        rec.add_frame(ReplayFrame::new(0, "AgentStarted", "start").with_elapsed_ms(0));
        rec.add_frame(ReplayFrame::new(1, "AgentToolCall", "read_file").with_elapsed_ms(100));
        rec.add_frame(ReplayFrame::new(2, "Error", "exec_failed").with_elapsed_ms(200));
        rec.add_frame(ReplayFrame::new(3, "Warning", "deprecated_api").with_elapsed_ms(300));
        rec.add_frame(ReplayFrame::new(4, "AgentOutput", "result_chunk").with_elapsed_ms(400));
        rec.finish();

        let summary = ReplaySummary::from_recording(&rec);
        assert_eq!(summary.session_id, "summary-session");
        assert_eq!(summary.total_frames, 5);
        assert_eq!(summary.errors_count, 1);
        assert_eq!(summary.warnings_count, 1);
        assert_eq!(summary.tool_calls_count, 1);
        // 高亮应包含: tool_call(帧1) + error(帧2) + warning(帧3) + output(帧4) = 4
        assert_eq!(summary.highlights.len(), 4);
        assert_eq!(
            summary.highlights[0].highlight_type,
            HighlightType::ToolCall
        );
        assert_eq!(summary.highlights[1].highlight_type, HighlightType::Error);
        assert_eq!(summary.highlights[2].highlight_type, HighlightType::Warning);
        assert_eq!(summary.highlights[3].highlight_type, HighlightType::Output);
        // duration_ms 基于最后一帧 elapsed_ms(finished_at 与 started_at 差值可能极小)
        assert!(summary.duration_ms > 0 || rec.frames.last().unwrap().elapsed_ms == 0);
    }

    // ===================================================================
    // 空录制处理
    // ===================================================================

    // 18. 空录制与空播放器处理
    #[test]
    fn test_empty_recording_and_player() {
        let rec = ExecutionRecording::new("empty-session");
        assert_eq!(rec.frame_count(), 0);
        assert_eq!(rec.duration(), Duration::ZERO);
        assert!(rec.frames.is_empty());
        assert!(rec.finished_at.is_none());

        // 空录制也能正常序列化
        let json = rec.to_json().expect("空录制 to_json");
        let de = ExecutionRecording::from_json(&json).expect("空录制 from_json");
        assert_eq!(de.frame_count(), 0);
        assert_eq!(de.session_id, "empty-session");

        // 空播放器
        let mut player = ReplayPlayer::new(rec);
        assert_eq!(player.total_frames(), 0);
        assert_eq!(player.current_index(), 0);
        assert!(player.current_frame().is_none());
        assert!((player.progress() - 0.0).abs() < f32::EPSILON);
        assert!(player.step_forward().is_none());
        assert!(player.step_backward().is_none());

        // seek 在空录制上不 panic
        player.seek(10);
        assert_eq!(player.current_index(), 0);

        // play/pause/stop 在空录制上不 panic
        player.play();
        assert_eq!(player.state(), PlayerState::Playing);
        player.stop();
        assert_eq!(player.state(), PlayerState::Stopped);
    }
}
