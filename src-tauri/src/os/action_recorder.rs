//! T-E-C-11: 操作录制回放 — UI 操作录制与回放模块。
//!
//! 提供 UI 操作的录制、存储、回放能力。录制时捕获每个操作的元数据
//! (类型、目标、参数、耗时、成功/失败),回放时按录制顺序模拟执行。
//!
//! ## 架构
//!
//! * [`RecordedAction`] — 单个录制的操作记录(serde + Debug + Clone)。
//! * [`RecordingSession`] — 录制会话,包含元数据与操作序列。
//! * [`RecordingMetadata`] — 会话元数据(名称、时间戳、标签等)。
//! * [`PlaybackOptions`] — 回放选项(速度、间隔、重试等),支持 builder 模式。
//! * [`PlaybackState`] — 回放状态枚举(serde snake_case)。
//! * [`PlaybackResult`] — 回放结果。
//! * [`PlaybackError`] — 回放错误。
//! * [`ActionPlayer`] — 回放器,执行录制会话。
//! * [`RecordingFilter`] — 录制过滤器,按条件筛选操作。
//!
//! ## 设计取舍
//!
//! 本模块不依赖 `action_executor.rs` 的具体类型,使用自定义的简化 action
//! 类型([`RecordedAction`])以避免 feature/dependency 问题。回放为模拟
//! 执行(按录制的耗时休眠,按录制的成功/失败状态返回结果),实际 UI
//! 执行需由上层注入 `ActionExecutor`。
//!
//! ## 注册
//!
//! 本模块当前未在 `os/mod.rs` 中注册(遵循"只创建新文件"约束)。注册时添加:
//! ```ignore
//! // in src-tauri/src/os/mod.rs
//! pub mod action_recorder;
//! ```

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------
// RecordedAction — 单个录制的操作记录
// ----------------------------------------------------------------------

/// 单个录制的 UI 操作记录。
///
/// 记录操作的时间戳、类型、目标描述、参数、耗时、成功/失败状态及错误信息。
/// 不依赖 `action_executor::UiAction`,使用简化字段以避免 feature/dependency 问题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedAction {
    /// 操作时间戳(UTC)。
    pub timestamp: DateTime<Utc>,
    /// 操作类型(如 "click" / "type" / "scroll")。
    pub action_type: String,
    /// 操作目标的人类可读描述。
    pub target_description: String,
    /// 操作参数键值对(如 {"x": "100", "y": "200"})。
    pub parameters: HashMap<String, String>,
    /// 操作执行耗时(毫秒)。
    pub duration_ms: u64,
    /// 操作是否成功。
    pub success: bool,
    /// 失败时的错误信息。
    pub error: Option<String>,
}

impl RecordedAction {
    /// 创建一个新的操作记录(便于测试与手动构造)。
    pub fn new(action_type: impl Into<String>, target_description: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            action_type: action_type.into(),
            target_description: target_description.into(),
            parameters: HashMap::new(),
            duration_ms: 0,
            success: true,
            error: None,
        }
    }

    /// 链式设置时间戳。
    pub fn with_timestamp(mut self, ts: DateTime<Utc>) -> Self {
        self.timestamp = ts;
        self
    }

    /// 链式设置参数。
    pub fn with_parameter(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(key.into(), value.into());
        self
    }

    /// 链式设置耗时。
    pub fn with_duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    /// 链式设置成功/失败状态。
    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    /// 链式设置错误信息。
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        let err = error.into();
        self.error = Some(err);
        self.success = false;
        self
    }
}

// ----------------------------------------------------------------------
// RecordingMetadata — 会话元数据
// ----------------------------------------------------------------------

/// 录制会话元数据。
///
/// 包含会话名称、创建/结束时间、操作总数、描述与标签。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingMetadata {
    /// 会话名称。
    pub name: String,
    /// 创建时间(UTC)。
    pub created_at: DateTime<Utc>,
    /// 结束时间(UTC);录制中为 None。
    pub finished_at: Option<DateTime<Utc>>,
    /// 操作总数。
    pub total_actions: usize,
    /// 会话描述。
    pub description: String,
    /// 标签列表。
    pub tags: Vec<String>,
}

impl RecordingMetadata {
    /// 创建新的元数据(用于构造会话)。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            created_at: Utc::now(),
            finished_at: None,
            total_actions: 0,
            description: String::new(),
            tags: Vec::new(),
        }
    }
}

// ----------------------------------------------------------------------
// RecordingSession — 录制会话
// ----------------------------------------------------------------------

/// 录制会话 — 管理录制生命周期与操作序列。
///
/// 使用 `start()` / `stop()` 控制录制状态,`record_action()` 在录制期间追加操作。
/// 支持 JSON 序列化往返(`to_json` / `from_json`)与可执行宏脚本生成(`to_macro`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSession {
    /// 会话元数据。
    pub metadata: RecordingMetadata,
    /// 已录制的操作序列。
    actions: Vec<RecordedAction>,
    /// 录制开始时间(UTC);`start()` 时设置。
    started_at: Option<DateTime<Utc>>,
    /// 是否正在录制。
    #[serde(skip)]
    is_recording: bool,
}

impl RecordingSession {
    /// 创建新的录制会话(未开始录制状态)。
    pub fn new(name: &str) -> Self {
        Self {
            metadata: RecordingMetadata::new(name),
            actions: Vec::new(),
            started_at: None,
            is_recording: false,
        }
    }

    /// 开始录制。
    pub fn start(&mut self) {
        if !self.is_recording {
            self.is_recording = true;
            self.started_at = Some(Utc::now());
            tracing::info!(session = %self.metadata.name, "录制会话已开始");
        }
    }

    /// 停止录制。
    pub fn stop(&mut self) {
        if self.is_recording {
            self.is_recording = false;
            self.metadata.finished_at = Some(Utc::now());
            self.metadata.total_actions = self.actions.len();
            tracing::info!(
                session = %self.metadata.name,
                actions = self.actions.len(),
                "录制会话已停止"
            );
        }
    }

    /// 记录单个操作(仅在录制中有效)。
    pub fn record_action(&mut self, action: RecordedAction) {
        if self.is_recording {
            self.actions.push(action);
            self.metadata.total_actions = self.actions.len();
        } else {
            tracing::warn!(
                session = %self.metadata.name,
                "试图在未录制的会话中记录操作,已忽略"
            );
        }
    }

    /// 是否正在录制。
    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    /// 获取所有操作(只读引用)。
    pub fn actions(&self) -> &[RecordedAction] {
        &self.actions
    }

    /// 操作数量。
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// 会话名称。
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// 录制时长。
    ///
    /// 已停止: `finished_at - started_at`;录制中: `now - started_at`;未开始: 零。
    pub fn duration(&self) -> Duration {
        match (self.started_at, self.metadata.finished_at) {
            (Some(start), Some(end)) => {
                let ms = (end - start).num_milliseconds().max(0) as u64;
                Duration::from_millis(ms)
            }
            (Some(start), None) if self.is_recording => {
                let ms = (Utc::now() - start).num_milliseconds().max(0) as u64;
                Duration::from_millis(ms)
            }
            _ => Duration::ZERO,
        }
    }

    /// 序列化为 JSON 字符串(pretty 格式)。
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// 从 JSON 字符串反序列化。
    ///
    /// 反序列化后的会话 `is_recording` 始终为 `false`(serde skip + default)。
    pub fn from_json(json: &str) -> Result<Self> {
        let session: Self = serde_json::from_str(json)?;
        Ok(session)
    }

    /// 生成可执行宏脚本(人类可读格式)。
    ///
    /// 输出格式示例:
    /// ```text
    /// # Nebula Macro: session_name
    /// # Created: 2024-01-01T00:00:00Z
    /// # Total actions: 2
    ///
    /// [action 1]
    ///   type: click
    ///   target: "Button OK"
    ///   duration_ms: 100
    ///   success: true
    ///   params:
    ///     x = "100"
    ///     y = "200"
    /// ```
    pub fn to_macro(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# Nebula Macro: {}\n", self.metadata.name));
        out.push_str(&format!("# Created: {}\n", self.metadata.created_at));
        match self.metadata.finished_at {
            Some(finished) => out.push_str(&format!("# Finished: {}\n", finished)),
            None => out.push_str("# Finished: (recording)\n"),
        }
        out.push_str(&format!(
            "# Total actions: {}\n",
            self.metadata.total_actions
        ));
        if !self.metadata.description.is_empty() {
            out.push_str(&format!("# Description: {}\n", self.metadata.description));
        }
        if !self.metadata.tags.is_empty() {
            out.push_str(&format!("# Tags: {}\n", self.metadata.tags.join(", ")));
        }
        out.push('\n');

        for (i, action) in self.actions.iter().enumerate() {
            out.push_str(&format!("[action {}]\n", i + 1));
            out.push_str(&format!("  type: {}\n", action.action_type));
            out.push_str(&format!("  target: \"{}\"\n", action.target_description));
            out.push_str(&format!("  duration_ms: {}\n", action.duration_ms));
            out.push_str(&format!("  success: {}\n", action.success));
            if let Some(err) = &action.error {
                out.push_str(&format!("  error: \"{}\"\n", err));
            }
            if !action.parameters.is_empty() {
                out.push_str("  params:\n");
                for (key, value) in &action.parameters {
                    out.push_str(&format!("    {} = \"{}\"\n", key, value));
                }
            }
            out.push('\n');
        }

        out
    }
}

// ----------------------------------------------------------------------
// PlaybackOptions — 回放选项
// ----------------------------------------------------------------------

/// 回放选项 — 控制回放速度、间隔、重试等参数。
///
/// 使用 `new()` 获取默认配置,或 `builder()` 链式构建。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackOptions {
    /// 回放速度倍率(1.0 = 正常速度,2.0 = 两倍速,0.5 = 半速)。
    pub speed: f32,
    /// 操作间暂停时长(毫秒)。
    pub pause_between_actions_ms: u64,
    /// 单个操作最大重试次数。
    pub max_retries: u32,
    /// 遇到错误是否停止回放。
    pub stop_on_error: bool,
    /// 是否在每个步骤截图。
    pub screenshot_each_step: bool,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self {
            speed: 1.0,
            pause_between_actions_ms: 100,
            max_retries: 3,
            stop_on_error: true,
            screenshot_each_step: false,
        }
    }
}

impl PlaybackOptions {
    /// 创建默认配置。
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建链式 builder。
    pub fn builder() -> PlaybackOptionsBuilder {
        PlaybackOptionsBuilder::default()
    }
}

// ----------------------------------------------------------------------
// PlaybackOptionsBuilder — 链式 builder
// ----------------------------------------------------------------------

/// 回放选项的链式 builder。
#[derive(Debug, Clone)]
pub struct PlaybackOptionsBuilder {
    speed: f32,
    pause_between_actions_ms: u64,
    max_retries: u32,
    stop_on_error: bool,
    screenshot_each_step: bool,
}

impl Default for PlaybackOptionsBuilder {
    fn default() -> Self {
        Self {
            speed: 1.0,
            pause_between_actions_ms: 100,
            max_retries: 3,
            stop_on_error: true,
            screenshot_each_step: false,
        }
    }
}

impl PlaybackOptionsBuilder {
    /// 设置回放速度。
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// 设置操作间暂停时长(毫秒)。
    pub fn pause_between_actions_ms(mut self, ms: u64) -> Self {
        self.pause_between_actions_ms = ms;
        self
    }

    /// 设置最大重试次数。
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// 设置遇到错误是否停止。
    pub fn stop_on_error(mut self, stop: bool) -> Self {
        self.stop_on_error = stop;
        self
    }

    /// 设置是否每步截图。
    pub fn screenshot_each_step(mut self, screenshot: bool) -> Self {
        self.screenshot_each_step = screenshot;
        self
    }

    /// 构建最终的 `PlaybackOptions`。
    pub fn build(self) -> PlaybackOptions {
        PlaybackOptions {
            speed: self.speed,
            pause_between_actions_ms: self.pause_between_actions_ms,
            max_retries: self.max_retries,
            stop_on_error: self.stop_on_error,
            screenshot_each_step: self.screenshot_each_step,
        }
    }
}

// ----------------------------------------------------------------------
// PlaybackState — 回放状态
// ----------------------------------------------------------------------

/// 回放状态枚举(serde snake_case)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    /// 待开始。
    Pending,
    /// 正在播放。
    Playing,
    /// 已暂停。
    Paused,
    /// 已完成。
    Completed,
    /// 已失败。
    Failed,
    /// 已停止。
    Stopped,
}

impl Default for PlaybackState {
    fn default() -> Self {
        PlaybackState::Pending
    }
}

// ----------------------------------------------------------------------
// PlaybackError — 回放错误
// ----------------------------------------------------------------------

/// 回放过程中单个操作的错误记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackError {
    /// 操作在序列中的索引(0 基)。
    pub action_index: usize,
    /// 操作类型。
    pub action_type: String,
    /// 错误信息。
    pub error: String,
    /// 错误发生时间(UTC)。
    pub timestamp: DateTime<Utc>,
}

// ----------------------------------------------------------------------
// PlaybackResult — 回放结果
// ----------------------------------------------------------------------

/// 回放结果 — 包含执行统计与错误列表。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackResult {
    /// 会话名称。
    pub session_name: String,
    /// 总操作数。
    pub total_actions: usize,
    /// 已执行操作数。
    pub executed_actions: usize,
    /// 成功操作数。
    pub succeeded: usize,
    /// 失败操作数。
    pub failed: usize,
    /// 最终回放状态。
    pub state: PlaybackState,
    /// 错误列表。
    pub errors: Vec<PlaybackError>,
    /// 回放总耗时(毫秒)。
    pub duration_ms: u64,
}

// ----------------------------------------------------------------------
// ActionPlayer — 回放器
// ----------------------------------------------------------------------

/// 回放器 — 按录制顺序模拟执行操作序列。
///
/// 使用 `play()` / `play_range()` 播放会话,`pause()` / `resume()` / `stop()`
/// 控制回放状态。状态通过内部 `Mutex` 管理,支持从其他线程控制。
///
/// 回放为模拟执行:按录制的 `duration_ms`(经 `speed` 缩放)休眠,
/// 按录制的 `success` 状态返回结果。实际 UI 执行需上层注入 `ActionExecutor`。
pub struct ActionPlayer {
    /// 回放选项。
    options: PlaybackOptions,
    /// 当前回放状态(内部可变性,支持 pause/resume/stop 从其他线程控制)。
    state: Mutex<PlaybackState>,
}

impl ActionPlayer {
    /// 创建新的回放器,注入回放选项。
    pub fn new(options: PlaybackOptions) -> Self {
        Self {
            options,
            state: Mutex::new(PlaybackState::Pending),
        }
    }

    /// 播放整个录制会话。
    ///
    /// 返回 `Err` 仅表示"无法开始播放"(如已在播放中);
    /// 操作执行失败以 `PlaybackResult { state: Failed/Completed, errors: [...] }` 表达。
    pub async fn play(&self, session: &RecordingSession) -> Result<PlaybackResult> {
        self.check_not_active()?;
        let actions: Vec<&RecordedAction> = session.actions().iter().collect();
        Ok(self.play_actions(session.name(), &actions).await)
    }

    /// 播放单个操作(模拟执行)。
    ///
    /// 若录制的操作为失败状态,返回 `Err`;否则按 `duration_ms / speed` 休眠后返回 `Ok`。
    pub async fn play_single(&self, action: &RecordedAction) -> Result<()> {
        // 录制时操作失败 → 回放也失败
        if !action.success {
            if let Some(err) = &action.error {
                anyhow::bail!("recorded action failed: {}", err);
            } else {
                anyhow::bail!("recorded action failed (no error message)");
            }
        }

        // 模拟执行耗时(按速度缩放)
        if action.duration_ms > 0 {
            let speed = self.options.speed.max(0.01);
            let scaled = (action.duration_ms as f32 / speed) as u64;
            if scaled > 0 {
                tokio::time::sleep(Duration::from_millis(scaled)).await;
            }
        }

        Ok(())
    }

    /// 播放会话中指定范围的操作 `[start, end)`。
    ///
    /// `start` 包含,`end` 不包含(遵循 Rust 切片惯例)。
    pub async fn play_range(
        &self,
        session: &RecordingSession,
        start: usize,
        end: usize,
    ) -> Result<PlaybackResult> {
        let all = session.actions();
        if start > all.len() {
            anyhow::bail!("start index {} out of bounds (len={})", start, all.len());
        }
        if end > all.len() {
            anyhow::bail!("end index {} out of bounds (len={})", end, all.len());
        }
        if start > end {
            anyhow::bail!("start ({}) > end ({})", start, end);
        }

        self.check_not_active()?;
        let actions: Vec<&RecordedAction> = all[start..end].iter().collect();
        Ok(self.play_actions(session.name(), &actions).await)
    }

    /// 暂停回放(仅在 Playing 状态有效)。
    pub fn pause(&self) {
        let mut state = self.state.lock().expect("PlaybackState mutex poisoned");
        if *state == PlaybackState::Playing {
            *state = PlaybackState::Paused;
            tracing::info!("回放已暂停");
        }
    }

    /// 恢复回放(仅在 Paused 状态有效)。
    pub fn resume(&self) {
        let mut state = self.state.lock().expect("PlaybackState mutex poisoned");
        if *state == PlaybackState::Paused {
            *state = PlaybackState::Playing;
            tracing::info!("回放已恢复");
        }
    }

    /// 停止回放(设置 Stopped 状态,play 循环会在下一个动作前检测并退出)。
    pub fn stop(&self) {
        let mut state = self.state.lock().expect("PlaybackState mutex poisoned");
        *state = PlaybackState::Stopped;
        tracing::info!("回放已请求停止");
    }

    /// 当前回放状态。
    pub fn state(&self) -> PlaybackState {
        self.state
            .lock()
            .expect("PlaybackState mutex poisoned")
            .clone()
    }

    // ------------------------------------------------------------------
    // 内部辅助
    // ------------------------------------------------------------------

    /// 检查播放器是否空闲(非 Playing/Paused),否则返回 Err。
    fn check_not_active(&self) -> Result<()> {
        let state = self.state.lock().expect("PlaybackState mutex poisoned");
        if *state == PlaybackState::Playing || *state == PlaybackState::Paused {
            anyhow::bail!("player is already active (state={:?})", *state);
        }
        Ok(())
    }

    /// 设置回放状态。
    fn set_state(&self, new_state: PlaybackState) {
        let mut state = self.state.lock().expect("PlaybackState mutex poisoned");
        *state = new_state;
    }

    /// 播放操作切片(核心逻辑,被 `play` / `play_range` 复用)。
    async fn play_actions(
        &self,
        session_name: &str,
        actions: &[&RecordedAction],
    ) -> PlaybackResult {
        self.set_state(PlaybackState::Playing);
        let started = Instant::now();
        let total = actions.len();
        let mut executed = 0usize;
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut errors = Vec::new();
        let mut final_state = PlaybackState::Completed;

        for (i, action) in actions.iter().enumerate() {
            // 检查暂停/停止状态
            loop {
                let state = self
                    .state
                    .lock()
                    .expect("PlaybackState mutex poisoned")
                    .clone();
                match state {
                    PlaybackState::Paused => {
                        // 暂停中:短暂休眠后重试
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    PlaybackState::Stopped => {
                        // 停止:立即返回
                        final_state = PlaybackState::Stopped;
                        return self.build_result(
                            session_name,
                            total,
                            executed,
                            succeeded,
                            failed,
                            errors,
                            started,
                            final_state,
                        );
                    }
                    _ => break,
                }
            }

            // 带重试执行单个操作
            let mut action_ok = false;
            for attempt in 0..=self.options.max_retries {
                match self.play_single(action).await {
                    Ok(()) => {
                        action_ok = true;
                        break;
                    }
                    Err(e) => {
                        if attempt < self.options.max_retries {
                            tracing::warn!(
                                attempt = attempt + 1,
                                max_retries = self.options.max_retries,
                                action_index = i,
                                action_type = %action.action_type,
                                "操作回放失败,即将重试: {e:#}"
                            );
                        } else {
                            tracing::warn!(
                                attempt = attempt + 1,
                                action_index = i,
                                action_type = %action.action_type,
                                "操作回放失败,已达最大重试次数: {e:#}"
                            );
                        }
                    }
                }
            }

            executed += 1;
            if action_ok {
                succeeded += 1;
            } else {
                failed += 1;
                errors.push(PlaybackError {
                    action_index: i,
                    action_type: action.action_type.clone(),
                    error: action
                        .error
                        .clone()
                        .unwrap_or_else(|| "playback failed".to_string()),
                    timestamp: Utc::now(),
                });
                if self.options.stop_on_error {
                    final_state = PlaybackState::Failed;
                    self.set_state(final_state.clone());
                    return self.build_result(
                        session_name,
                        total,
                        executed,
                        succeeded,
                        failed,
                        errors,
                        started,
                        final_state,
                    );
                }
            }

            // 操作间暂停(最后一个操作后不暂停)
            if i + 1 < total && self.options.pause_between_actions_ms > 0 {
                let speed = self.options.speed.max(0.01);
                let pause = (self.options.pause_between_actions_ms as f32 / speed) as u64;
                if pause > 0 {
                    tokio::time::sleep(Duration::from_millis(pause)).await;
                }
            }
        }

        self.set_state(final_state.clone());
        self.build_result(
            session_name,
            total,
            executed,
            succeeded,
            failed,
            errors,
            started,
            final_state,
        )
    }

    /// 构建 PlaybackResult。
    fn build_result(
        &self,
        session_name: &str,
        total: usize,
        executed: usize,
        succeeded: usize,
        failed: usize,
        errors: Vec<PlaybackError>,
        started: Instant,
        state: PlaybackState,
    ) -> PlaybackResult {
        PlaybackResult {
            session_name: session_name.to_string(),
            total_actions: total,
            executed_actions: executed,
            succeeded,
            failed,
            state,
            errors,
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }
}

// ----------------------------------------------------------------------
// RecordingFilter — 录制过滤器
// ----------------------------------------------------------------------

/// 录制过滤器 — 按操作类型、最小时长、成功状态筛选操作。
///
/// 所有条件为 AND 关系:操作必须同时满足所有非空/非 None 条件才匹配。
/// 空过滤器(`action_types` 为空、`min_duration_ms` 为 None、`only_successful` 为 None)
/// 匹配所有操作。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecordingFilter {
    /// 限定操作类型列表(空 = 不限)。
    pub action_types: Vec<String>,
    /// 最小耗时阈值(毫秒);None = 不限。
    pub min_duration_ms: Option<u64>,
    /// 仅匹配成功/失败操作;None = 不限。
    pub only_successful: Option<bool>,
}

impl RecordingFilter {
    /// 创建空过滤器(匹配所有操作)。
    pub fn new() -> Self {
        Self::default()
    }

    /// 检查单个操作是否匹配过滤器。
    pub fn matches(&self, action: &RecordedAction) -> bool {
        // 操作类型过滤
        if !self.action_types.is_empty() && !self.action_types.contains(&action.action_type) {
            return false;
        }
        // 最小时长过滤
        if let Some(min) = self.min_duration_ms {
            if action.duration_ms < min {
                return false;
            }
        }
        // 成功状态过滤
        if let Some(want_success) = self.only_successful {
            if action.success != want_success {
                return false;
            }
        }
        true
    }

    /// 过滤会话中的操作,返回匹配的操作引用列表。
    pub fn filter_session<'a>(
        session: &'a RecordingSession,
        filter: &RecordingFilter,
    ) -> Vec<&'a RecordedAction> {
        session
            .actions()
            .iter()
            .filter(|a| filter.matches(a))
            .collect()
    }
}

// ----------------------------------------------------------------------
// 单元测试
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 测试辅助 ----

    /// 构造一个简单的操作记录。
    fn make_action(action_type: &str, success: bool, duration_ms: u64) -> RecordedAction {
        RecordedAction {
            timestamp: Utc::now(),
            action_type: action_type.to_string(),
            target_description: "test target".to_string(),
            parameters: HashMap::new(),
            duration_ms,
            success,
            error: if success {
                None
            } else {
                Some("test error".to_string())
            },
        }
    }

    /// 构造一个带参数的操作记录。
    fn make_action_with_params(
        action_type: &str,
        success: bool,
        duration_ms: u64,
        params: &[(&str, &str)],
    ) -> RecordedAction {
        let mut action = make_action(action_type, success, duration_ms);
        for (k, v) in params {
            action.parameters.insert(k.to_string(), v.to_string());
        }
        action
    }

    // ---- 1. RecordedAction 序列化往返 ----

    #[test]
    fn recorded_action_serde_roundtrip() {
        let action = make_action_with_params("click", true, 150, &[("x", "100"), ("y", "200")]);
        let json = serde_json::to_string(&action).expect("serialize");
        let back: RecordedAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.action_type, "click");
        assert_eq!(back.target_description, "test target");
        assert_eq!(back.duration_ms, 150);
        assert!(back.success);
        assert!(back.error.is_none());
        assert_eq!(back.parameters.get("x").map(|s| s.as_str()), Some("100"));
        assert_eq!(back.parameters.get("y").map(|s| s.as_str()), Some("200"));
    }

    #[test]
    fn recorded_action_failed_serde_roundtrip() {
        let action = make_action("type", false, 50);
        let json = serde_json::to_string(&action).expect("serialize");
        let back: RecordedAction = serde_json::from_str(&json).expect("deserialize");
        assert!(!back.success);
        assert_eq!(back.error.as_deref(), Some("test error"));
    }

    // ---- 2. RecordingSession start/stop/record ----

    #[test]
    fn recording_session_start_stop_record() {
        let mut session = RecordingSession::new("test_session");
        assert!(!session.is_recording());

        session.start();
        assert!(session.is_recording());

        // 录制中记录操作
        session.record_action(make_action("click", true, 10));
        session.record_action(make_action("type", true, 20));
        assert_eq!(session.action_count(), 2);

        session.stop();
        assert!(!session.is_recording());
        assert_eq!(session.action_count(), 2);
        // 停止后元数据应更新
        assert!(session.metadata.finished_at.is_some());
        assert_eq!(session.metadata.total_actions, 2);
    }

    // ---- 3. is_recording 状态 ----

    #[test]
    fn recording_session_is_recording_state() {
        let mut session = RecordingSession::new("state_test");
        // 初始未录制
        assert!(!session.is_recording());

        // start 后正在录制
        session.start();
        assert!(session.is_recording());

        // stop 后停止
        session.stop();
        assert!(!session.is_recording());

        // 重复 start/stop 不影响状态
        session.start();
        session.start(); // 重复 start 无效
        assert!(session.is_recording());
        session.stop();
        session.stop(); // 重复 stop 无效
        assert!(!session.is_recording());
    }

    // ---- 4. action_count ----

    #[test]
    fn recording_session_action_count() {
        let mut session = RecordingSession::new("count_test");
        assert_eq!(session.action_count(), 0);

        session.start();
        for i in 0..5 {
            session.record_action(make_action("click", true, i * 10));
        }
        assert_eq!(session.action_count(), 5);
        session.stop();
        assert_eq!(session.action_count(), 5);
    }

    // ---- 5. to_json/from_json 往返 ----

    #[test]
    fn recording_session_json_roundtrip() {
        let mut session = RecordingSession::new("json_test");
        session.start();
        session.record_action(make_action_with_params(
            "click",
            true,
            100,
            &[("x", "50"), ("y", "60")],
        ));
        session.record_action(make_action("type", false, 200));
        session.stop();

        let json = session.to_json().expect("to_json");
        let restored = RecordingSession::from_json(&json).expect("from_json");

        // 验证元数据
        assert_eq!(restored.name(), "json_test");
        assert_eq!(restored.metadata.total_actions, 2);
        assert!(restored.metadata.finished_at.is_some());

        // 验证操作序列
        assert_eq!(restored.action_count(), 2);
        assert_eq!(restored.actions()[0].action_type, "click");
        assert_eq!(restored.actions()[1].action_type, "type");
        assert!(!restored.actions()[1].success);

        // 反序列化后 is_recording 必须为 false
        assert!(!restored.is_recording());
    }

    #[test]
    fn recording_session_from_invalid_json_errors() {
        let result = RecordingSession::from_json("not valid json {{{");
        assert!(result.is_err());
    }

    // ---- 6. to_macro 生成内容 ----

    #[test]
    fn recording_session_to_macro_content() {
        let mut session = RecordingSession::new("macro_test");
        session.metadata.description = "A test macro".to_string();
        session.metadata.tags = vec!["ui".to_string(), "test".to_string()];
        session.start();
        session.record_action(make_action_with_params(
            "click",
            true,
            100,
            &[("x", "10"), ("y", "20")],
        ));
        session.record_action(make_action("type", false, 200));
        session.stop();

        let macro_str = session.to_macro();

        // 验证宏包含会话名称
        assert!(macro_str.contains("# Nebula Macro: macro_test"));
        // 验证包含描述与标签
        assert!(macro_str.contains("# Description: A test macro"));
        assert!(macro_str.contains("# Tags: ui, test"));
        // 验证包含操作总数
        assert!(macro_str.contains("# Total actions: 2"));
        // 验证包含操作类型
        assert!(macro_str.contains("type: click"));
        assert!(macro_str.contains("type: type"));
        // 验证包含参数
        assert!(macro_str.contains("x = \"10\""));
        assert!(macro_str.contains("y = \"20\""));
        // 验证包含成功/失败状态
        assert!(macro_str.contains("success: true"));
        assert!(macro_str.contains("success: false"));
        // 验证包含错误信息
        assert!(macro_str.contains("error: \"test error\""));
        // 验证包含动作序号
        assert!(macro_str.contains("[action 1]"));
        assert!(macro_str.contains("[action 2]"));
    }

    // ---- 7. PlaybackOptions 默认值 ----

    #[test]
    fn playback_options_defaults() {
        let opts = PlaybackOptions::new();
        assert!((opts.speed - 1.0).abs() < f32::EPSILON);
        assert_eq!(opts.pause_between_actions_ms, 100);
        assert_eq!(opts.max_retries, 3);
        assert!(opts.stop_on_error);
        assert!(!opts.screenshot_each_step);
    }

    #[test]
    fn playback_options_default_trait() {
        let opts = PlaybackOptions::default();
        assert!((opts.speed - 1.0).abs() < f32::EPSILON);
        assert_eq!(opts.pause_between_actions_ms, 100);
    }

    // ---- 8. PlaybackOptions builder ----

    #[test]
    fn playback_options_builder() {
        let opts = PlaybackOptions::builder()
            .speed(2.0)
            .pause_between_actions_ms(50)
            .max_retries(5)
            .stop_on_error(false)
            .screenshot_each_step(true)
            .build();

        assert!((opts.speed - 2.0).abs() < f32::EPSILON);
        assert_eq!(opts.pause_between_actions_ms, 50);
        assert_eq!(opts.max_retries, 5);
        assert!(!opts.stop_on_error);
        assert!(opts.screenshot_each_step);
    }

    // ---- 9. PlaybackState 序列化与转换 ----

    #[test]
    fn playback_state_serde_roundtrip() {
        let states = vec![
            PlaybackState::Pending,
            PlaybackState::Playing,
            PlaybackState::Paused,
            PlaybackState::Completed,
            PlaybackState::Failed,
            PlaybackState::Stopped,
        ];
        for state in &states {
            let json = serde_json::to_string(state).expect("serialize");
            let back: PlaybackState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(state, &back);
        }
    }

    #[test]
    fn playback_state_snake_case() {
        // 验证 snake_case 序列化
        let json = serde_json::to_string(&PlaybackState::Playing).expect("serialize");
        assert!(json.contains("\"playing\""));
        let json = serde_json::to_string(&PlaybackState::Completed).expect("serialize");
        assert!(json.contains("\"completed\""));
    }

    #[test]
    fn playback_state_default_is_pending() {
        let state = PlaybackState::default();
        assert_eq!(state, PlaybackState::Pending);
    }

    // ---- 10. PlaybackResult 构建 ----

    #[test]
    fn playback_result_construction() {
        let result = PlaybackResult {
            session_name: "test".to_string(),
            total_actions: 10,
            executed_actions: 8,
            succeeded: 6,
            failed: 2,
            state: PlaybackState::Completed,
            errors: vec![PlaybackError {
                action_index: 3,
                action_type: "click".to_string(),
                error: "not found".to_string(),
                timestamp: Utc::now(),
            }],
            duration_ms: 500,
        };

        assert_eq!(result.session_name, "test");
        assert_eq!(result.total_actions, 10);
        assert_eq!(result.executed_actions, 8);
        assert_eq!(result.succeeded, 6);
        assert_eq!(result.failed, 2);
        assert_eq!(result.state, PlaybackState::Completed);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.duration_ms, 500);

        // 序列化往返
        let json = serde_json::to_string(&result).expect("serialize");
        let back: PlaybackResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.session_name, "test");
        assert_eq!(back.total_actions, 10);
        assert_eq!(back.state, PlaybackState::Completed);
    }

    // ---- 11. PlaybackError 结构 ----

    #[test]
    fn playback_error_structure() {
        let error = PlaybackError {
            action_index: 5,
            action_type: "scroll".to_string(),
            error: "scroll failed".to_string(),
            timestamp: Utc::now(),
        };

        assert_eq!(error.action_index, 5);
        assert_eq!(error.action_type, "scroll");
        assert_eq!(error.error, "scroll failed");

        // 序列化往返
        let json = serde_json::to_string(&error).expect("serialize");
        let back: PlaybackError = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.action_index, 5);
        assert_eq!(back.action_type, "scroll");
        assert_eq!(back.error, "scroll failed");
    }

    // ---- 12. RecordingFilter 匹配 ----

    #[test]
    fn recording_filter_matches() {
        let action = make_action("click", true, 200);

        // 空过滤器匹配所有
        let empty = RecordingFilter::new();
        assert!(empty.matches(&action));

        // 按类型匹配
        let by_type = RecordingFilter {
            action_types: vec!["click".to_string()],
            ..Default::default()
        };
        assert!(by_type.matches(&action));

        // 按最小时长匹配
        let by_duration = RecordingFilter {
            min_duration_ms: Some(100),
            ..Default::default()
        };
        assert!(by_duration.matches(&action)); // 200 >= 100

        // 按成功状态匹配
        let by_success = RecordingFilter {
            only_successful: Some(true),
            ..Default::default()
        };
        assert!(by_success.matches(&action)); // action.success == true
    }

    // ---- 13. RecordingFilter 不匹配 ----

    #[test]
    fn recording_filter_does_not_match() {
        let action = make_action("click", true, 50);

        // 类型不匹配
        let wrong_type = RecordingFilter {
            action_types: vec!["type".to_string()],
            ..Default::default()
        };
        assert!(!wrong_type.matches(&action));

        // 时长不匹配
        let too_short = RecordingFilter {
            min_duration_ms: Some(100),
            ..Default::default()
        };
        assert!(!too_short.matches(&action)); // 50 < 100

        // 成功状态不匹配
        let only_failed = RecordingFilter {
            only_successful: Some(false),
            ..Default::default()
        };
        assert!(!only_failed.matches(&action)); // action.success == true, want false
    }

    #[test]
    fn recording_filter_combined_criteria() {
        let action = make_action("scroll", true, 300);

        // 组合条件:类型 + 时长 + 成功
        let combined = RecordingFilter {
            action_types: vec!["click".to_string(), "scroll".to_string()],
            min_duration_ms: Some(200),
            only_successful: Some(true),
        };
        assert!(combined.matches(&action));

        // 其中一个条件不满足
        let combined_fail = RecordingFilter {
            action_types: vec!["click".to_string(), "scroll".to_string()],
            min_duration_ms: Some(500), // 300 < 500
            only_successful: Some(true),
        };
        assert!(!combined_fail.matches(&action));
    }

    // ---- 14. filter_session 过滤 ----

    #[test]
    fn filter_session_filters() {
        let mut session = RecordingSession::new("filter_test");
        session.start();
        session.record_action(make_action("click", true, 100));
        session.record_action(make_action("type", false, 50));
        session.record_action(make_action("click", true, 300));
        session.record_action(make_action("scroll", true, 200));
        session.stop();

        // 过滤 click 类型
        let click_filter = RecordingFilter {
            action_types: vec!["click".to_string()],
            ..Default::default()
        };
        let clicks = RecordingFilter::filter_session(&session, &click_filter);
        assert_eq!(clicks.len(), 2);
        assert!(clicks.iter().all(|a| a.action_type == "click"));

        // 过滤时长 >= 200
        let long_filter = RecordingFilter {
            min_duration_ms: Some(200),
            ..Default::default()
        };
        let long_actions = RecordingFilter::filter_session(&session, &long_filter);
        assert_eq!(long_actions.len(), 2); // click(300) + scroll(200)
        assert!(long_actions.iter().all(|a| a.duration_ms >= 200));

        // 过滤失败操作
        let failed_filter = RecordingFilter {
            only_successful: Some(false),
            ..Default::default()
        };
        let failed = RecordingFilter::filter_session(&session, &failed_filter);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].action_type, "type");

        // 空过滤器返回全部
        let all = RecordingFilter::filter_session(&session, &RecordingFilter::new());
        assert_eq!(all.len(), 4);
    }

    // ---- 15. 空 session 处理 ----

    #[test]
    fn empty_session_handling() {
        let session = RecordingSession::new("empty");
        assert_eq!(session.action_count(), 0);
        assert!(!session.is_recording());
        assert!(session.actions().is_empty());
        assert_eq!(session.duration(), Duration::ZERO);

        // 空 session 的 JSON 往返
        let json = session.to_json().expect("to_json");
        let restored = RecordingSession::from_json(&json).expect("from_json");
        assert_eq!(restored.action_count(), 0);
        assert_eq!(restored.name(), "empty");

        // 空 session 的宏生成
        let macro_str = session.to_macro();
        assert!(macro_str.contains("# Nebula Macro: empty"));
        assert!(macro_str.contains("# Total actions: 0"));
        // 没有动作
        assert!(!macro_str.contains("[action"));

        // 空 session 的过滤
        let filter = RecordingFilter::new();
        let filtered = RecordingFilter::filter_session(&session, &filter);
        assert!(filtered.is_empty());
    }

    // ---- 16. 未录制时 record_action 被忽略 ----

    #[test]
    fn record_action_ignored_when_not_recording() {
        let mut session = RecordingSession::new("no_record");
        // 未 start,直接 record_action 应被忽略
        session.record_action(make_action("click", true, 10));
        assert_eq!(session.action_count(), 0);
    }

    // ---- 17. ActionPlayer 播放空 session ----

    #[tokio::test]
    async fn action_player_play_empty_session() {
        let player = ActionPlayer::new(PlaybackOptions::new());
        let session = RecordingSession::new("empty_play");

        let result = player.play(&session).await.expect("play");
        assert_eq!(result.total_actions, 0);
        assert_eq!(result.executed_actions, 0);
        assert_eq!(result.succeeded, 0);
        assert_eq!(result.failed, 0);
        assert_eq!(result.state, PlaybackState::Completed);
        assert!(result.errors.is_empty());
        assert_eq!(result.session_name, "empty_play");
    }

    // ---- 18. ActionPlayer 播放成功操作 ----

    #[tokio::test]
    async fn action_player_play_successful_actions() {
        let opts = PlaybackOptions::builder()
            .pause_between_actions_ms(1) // 测试中用极短间隔
            .max_retries(0)
            .build();
        let player = ActionPlayer::new(opts);

        let mut session = RecordingSession::new("success_play");
        session.start();
        session.record_action(make_action("click", true, 1)); // 极短耗时
        session.record_action(make_action("type", true, 1));
        session.stop();

        let result = player.play(&session).await.expect("play");
        assert_eq!(result.total_actions, 2);
        assert_eq!(result.executed_actions, 2);
        assert_eq!(result.succeeded, 2);
        assert_eq!(result.failed, 0);
        assert_eq!(result.state, PlaybackState::Completed);
        assert!(result.errors.is_empty());
    }

    // ---- 19. ActionPlayer 播放失败操作(stop_on_error) ----

    #[tokio::test]
    async fn action_player_play_failed_action_stops() {
        let opts = PlaybackOptions::builder()
            .pause_between_actions_ms(1)
            .max_retries(1)
            .stop_on_error(true)
            .build();
        let player = ActionPlayer::new(opts);

        let mut session = RecordingSession::new("fail_play");
        session.start();
        session.record_action(make_action("click", true, 1));
        session.record_action(make_action("type", false, 1)); // 失败操作
        session.record_action(make_action("scroll", true, 1)); // 不会执行
        session.stop();

        let result = player.play(&session).await.expect("play");
        assert_eq!(result.total_actions, 3);
        assert_eq!(result.executed_actions, 2); // 执行了 2 个(第 2 个失败后停止)
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.state, PlaybackState::Failed);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].action_index, 1); // 第 2 个操作(索引 1)
        assert_eq!(result.errors[0].action_type, "type");
    }

    // ---- 20. ActionPlayer 播放失败操作(不停止) ----

    #[tokio::test]
    async fn action_player_play_failed_action_continues() {
        let opts = PlaybackOptions::builder()
            .pause_between_actions_ms(1)
            .max_retries(0)
            .stop_on_error(false)
            .build();
        let player = ActionPlayer::new(opts);

        let mut session = RecordingSession::new("continue_play");
        session.start();
        session.record_action(make_action("click", true, 1));
        session.record_action(make_action("type", false, 1));
        session.record_action(make_action("scroll", true, 1));
        session.stop();

        let result = player.play(&session).await.expect("play");
        assert_eq!(result.total_actions, 3);
        assert_eq!(result.executed_actions, 3); // 全部执行
        assert_eq!(result.succeeded, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.state, PlaybackState::Completed); // 未停止 → 完成
        assert_eq!(result.errors.len(), 1);
    }

    // ---- 21. ActionPlayer play_range ----

    #[tokio::test]
    async fn action_player_play_range() {
        let opts = PlaybackOptions::builder()
            .pause_between_actions_ms(1)
            .max_retries(0)
            .build();
        let player = ActionPlayer::new(opts);

        let mut session = RecordingSession::new("range_play");
        session.start();
        for i in 0..5 {
            session.record_action(make_action("click", true, 1 + i));
        }
        session.stop();

        // 播放 [1, 4) → 3 个操作
        let result = player.play_range(&session, 1, 4).await.expect("play_range");
        assert_eq!(result.total_actions, 3);
        assert_eq!(result.executed_actions, 3);
        assert_eq!(result.succeeded, 3);
        assert_eq!(result.state, PlaybackState::Completed);
    }

    #[tokio::test]
    async fn action_player_play_range_invalid_bounds() {
        let player = ActionPlayer::new(PlaybackOptions::new());
        let session = RecordingSession::new("range_invalid");
        // start > end
        assert!(player.play_range(&session, 2, 1).await.is_err());
        // end > len
        assert!(player.play_range(&session, 0, 10).await.is_err());
    }

    // ---- 22. ActionPlayer pause/resume/stop 控制 ----

    #[test]
    fn action_player_state_control() {
        let player = ActionPlayer::new(PlaybackOptions::new());
        // 初始状态
        assert_eq!(player.state(), PlaybackState::Pending);

        // pause 在非 Playing 状态无效
        player.pause();
        assert_eq!(player.state(), PlaybackState::Pending);

        // stop 总是有效
        player.stop();
        assert_eq!(player.state(), PlaybackState::Stopped);
    }

    // ---- 23. ActionPlayer play_single 模拟 ----

    #[tokio::test]
    async fn action_player_play_single_success() {
        let player = ActionPlayer::new(PlaybackOptions::new());
        let action = make_action("click", true, 5);
        let result = player.play_single(&action).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn action_player_play_single_failure() {
        let player = ActionPlayer::new(PlaybackOptions::new());
        let action = make_action("click", false, 5);
        let result = player.play_single(&action).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("recorded action failed"));
    }

    // ---- 24. RecordedAction builder 链式构造 ----

    #[test]
    fn recorded_action_builder_chain() {
        let action = RecordedAction::new("click", "Button OK")
            .with_duration_ms(150)
            .with_success(true)
            .with_parameter("x", "100")
            .with_parameter("y", "200");

        assert_eq!(action.action_type, "click");
        assert_eq!(action.target_description, "Button OK");
        assert_eq!(action.duration_ms, 150);
        assert!(action.success);
        assert_eq!(action.parameters.get("x").map(|s| s.as_str()), Some("100"));
        assert_eq!(action.parameters.get("y").map(|s| s.as_str()), Some("200"));
        assert!(action.error.is_none());
    }

    #[test]
    fn recorded_action_with_error_sets_success_false() {
        let action = RecordedAction::new("type", "Field")
            .with_success(true)
            .with_error("element not found");
        // with_error 自动设置 success = false
        assert!(!action.success);
        assert_eq!(action.error.as_deref(), Some("element not found"));
    }
}
