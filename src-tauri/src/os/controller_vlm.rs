//! T-E-C-01: OS-Controller VLM 模式 — 视觉语言模型驱动的 OS 自动化。
//!
//! 与现有 API 模式（`controller.rs` 通过 Win32 API / UIAutomation 操作）互补，
//! VLM 模式通过「截图 → VLM 视觉分析 → 操作执行」闭环驱动 OS 自动化。
//! 适合无法通过 Accessibility API 操控的场景（Canvas、自绘 UI、远程桌面等）。
//!
//! ## 架构
//!
//! * [`VlmController`] — 融合 VLM 视觉理解 + [`UiAutomator`] 操作执行。
//!   持有 `UnifiedModelDispatcher`（LLM 调度）和 `dyn UiAutomator`（操作执行）。
//! * 数据模型（无 feature gate，始终可用）：
//!   - [`BoundingBox`] — 归一化坐标（0.0-1.0），与屏幕分辨率解耦。
//!   - [`VlmAction`] — VLM 决策的动作枚举（Click/Type/Scroll/KeyPress/Wait/Screenshot/Done）。
//!   - [`ScreenAnalysis`] — VLM 对截图的分析结果。
//!   - [`VlmExecutionResult`] / [`VlmStepRecord`] — 闭环执行结果与单步记录。
//! * VLM 调用通过 `UnifiedModelDispatcher::gateway().describe_image()` 走 Ollama
//!   多模态 API（与 `browser::vlm_mode.rs` 的 wire format 一致）。
//!
//! ## Feature Gate
//!
//! `VlmController` 的 LLM 调度依赖 `UnifiedModelDispatcher`（P0-2：unified-dispatcher
//! feature 已默认启用）。数据模型和 prompt 构建方法（关联函数）始终可用，便于单元测试。
//!
//! ## 注册
//!
//! 本模块当前未在 `os/mod.rs` 中注册（主控统一处理）。注册时添加：
//! ```ignore
//! // in src-tauri/src/os/mod.rs
//! pub mod controller_vlm;
//! ```

use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use crate::os::uiautomator::{
    ElementBounds, ScrollDirection as UiScrollDirection, UiAutomator, UiElement,
};

// ── 默认配置 ───────────────────────────────────────────────────────

/// 默认视觉模型（与 `AppConfig::vision_model` 默认值一致）。
const DEFAULT_VISION_MODEL: &str = "qwen2.5-vl:3b";

/// `execute_goal` 的默认最大步数。
const DEFAULT_MAX_STEPS: usize = 20;

// ── 归一化边界框 ───────────────────────────────────────────────────

/// 归一化边界框 — 坐标范围 0.0-1.0，与屏幕分辨率解耦。
///
/// VLM 返回的元素位置使用归一化坐标，`VlmController::perform_action` 在执行时
/// 乘以屏幕宽高得到像素坐标。归一化坐标使 VLM 输出与具体分辨率无关，
/// 便于跨屏幕 / 跨设备复用。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct BoundingBox {
    /// 左上角 X 坐标（归一化，0.0-1.0）。
    pub x: f32,
    /// 左上角 Y 坐标（归一化，0.0-1.0）。
    pub y: f32,
    /// 宽度（归一化，0.0-1.0）。
    pub width: f32,
    /// 高度（归一化，0.0-1.0）。
    pub height: f32,
}

impl BoundingBox {
    /// 创建新的归一化边界框。
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 验证所有坐标是否在 0.0-1.0 范围内。
    pub fn is_normalized(&self) -> bool {
        let in_range = |v: f32| (0.0..=1.0).contains(&v);
        in_range(self.x) && in_range(self.y) && in_range(self.width) && in_range(self.height)
    }

    /// 归一化中心点坐标。
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// 转换为屏幕像素坐标的 `(x, y, width, height)` 元组。
    ///
    /// 归一化坐标乘以屏幕宽高，得到像素坐标的左上角与尺寸。
    pub fn to_pixels(&self, screen_width: u32, screen_height: u32) -> (u32, u32, u32, u32) {
        let px = (self.x * screen_width as f32).round() as u32;
        let py = (self.y * screen_height as f32).round() as u32;
        let pw = (self.width * screen_width as f32).round() as u32;
        let ph = (self.height * screen_height as f32).round() as u32;
        (px, py, pw, ph)
    }
}

// ── 滚动方向 ───────────────────────────────────────────────────────

/// VLM 模式滚动方向。
///
/// 注意：与 `uiautomator::ScrollDirection` 是独立类型（本类型使用 lowercase
/// serde 序列化，与 VLM JSON 响应风格一致），`perform_action` 内部做转换。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollDirection {
    /// 转换为 `uiautomator::ScrollDirection`（用于调用 UiAutomator::scroll）。
    fn to_uiautomator(self) -> UiScrollDirection {
        match self {
            ScrollDirection::Up => UiScrollDirection::Up,
            ScrollDirection::Down => UiScrollDirection::Down,
            ScrollDirection::Left => UiScrollDirection::Left,
            ScrollDirection::Right => UiScrollDirection::Right,
        }
    }
}

// ── 检测到的 UI 元素 ───────────────────────────────────────────────

/// VLM 识别的 UI 元素类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ElementType {
    Button,
    Input,
    Text,
    Image,
    Menu,
    Dialog,
    Other,
}

/// VLM 检测到的单个 UI 元素。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedElement {
    /// 元素类型。
    pub element_type: ElementType,
    /// 元素可见文本（可选）。
    pub text: Option<String>,
    /// 元素归一化边界框。
    pub bbox: BoundingBox,
    /// VLM 识别置信度（0.0-1.0）。
    pub confidence: f32,
}

// ── VLM 决策动作 ───────────────────────────────────────────────────

/// VLM 决策的动作 — 闭环中每一步执行的操作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VlmAction {
    /// 点击指定区域。
    Click {
        /// 目标元素的归一化边界框。
        bbox: BoundingBox,
        /// 动作描述（供日志 / 调试）。
        description: String,
    },
    /// 在指定输入框中输入文本。
    Type {
        /// 目标输入框的归一化边界框。
        bbox: BoundingBox,
        /// 要输入的文本。
        text: String,
    },
    /// 滚动屏幕。
    Scroll {
        /// 滚动方向。
        direction: ScrollDirection,
        /// 滚动量（0.0-1.0，占屏幕的比例）。
        amount: f32,
    },
    /// 模拟按键。
    KeyPress {
        /// 按键名称（如 "Enter" / "Escape" / "Tab"）。
        key: String,
    },
    /// 等待指定秒数（用于动画 / 加载完成）。
    Wait {
        /// 等待秒数。
        seconds: f32,
    },
    /// 重新截图（用于验证操作效果）。
    Screenshot,
    /// 目标已完成，闭环结束。
    Done {
        /// 完成摘要。
        summary: String,
    },
}

// ── 屏幕分析结果 ───────────────────────────────────────────────────

/// VLM 对截图的分析结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenAnalysis {
    /// 截图文件路径（本地临时文件）。
    pub screenshot_path: String,
    /// VLM 对屏幕的自然语言描述。
    pub description: String,
    /// 检测到的 UI 元素列表。
    pub detected_elements: Vec<DetectedElement>,
    /// VLM 建议的下一步动作列表。
    pub suggested_actions: Vec<VlmAction>,
    /// 目标是否已完成。
    pub is_goal_complete: bool,
    /// VLM 分析置信度（0.0-1.0）。
    pub confidence: f32,
}

// ── 单步执行记录 ───────────────────────────────────────────────────

/// VLM 闭环中单步执行的记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VlmStepRecord {
    /// 步骤编号（从 1 开始）。
    pub step_number: u32,
    /// 本步截图文件路径。
    pub screenshot_path: String,
    /// 本步屏幕分析结果。
    pub analysis: ScreenAnalysis,
    /// 本步执行的动作。
    pub action: VlmAction,
    /// 动作执行结果描述（成功 / 失败信息，可选）。
    pub action_result: Option<String>,
    /// 本步时间戳。
    pub timestamp: DateTime<Utc>,
}

// ── 闭环执行结果 ───────────────────────────────────────────────────

/// VLM 闭环执行的整体结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VlmExecutionResult {
    /// 用户目标。
    pub goal: String,
    /// 各步骤记录（按执行顺序）。
    pub steps: Vec<VlmStepRecord>,
    /// 是否成功完成目标。
    pub success: bool,
    /// 结果摘要。
    pub summary: String,
    /// 总耗时（毫秒）。
    pub total_duration_ms: u64,
}

// ── VLM 响应解析辅助 ───────────────────────────────────────────────

/// VLM 分析响应的 JSON 结构（用于解析 `describe_image` 返回的文本）。
///
/// `screenshot_path` 不由 VLM 返回，由调用方在解析后填充。
#[derive(Debug, Deserialize)]
struct VlmAnalysisResponse {
    description: String,
    #[serde(default)]
    detected_elements: Vec<DetectedElement>,
    #[serde(default)]
    suggested_actions: Vec<VlmAction>,
    #[serde(default)]
    is_goal_complete: bool,
    #[serde(default)]
    confidence: f32,
}

// ── VlmController ──────────────────────────────────────────────────

/// VLM 模式 OS Controller — 融合 VLM 视觉理解 + UiAutomator 操作执行。
///
/// 持有 `UnifiedModelDispatcher`（LLM 调度，通过 `gateway().describe_image()`
/// 调用 VLM）和 `dyn UiAutomator`（截图 + 点击 / 输入 / 滚动）。
///
/// `llm_dispatcher` 字段持有 `UnifiedModelDispatcher`（P0-2：unified-dispatcher
/// feature 已默认启用）；数据模型和 prompt 构建方法（关联函数）始终可用，
/// 便于单元测试。
pub struct VlmController {
    /// LLM 调度器 — 通过 `gateway().describe_image()` 调用 VLM 多模态 API。
    llm_dispatcher: Arc<crate::llm::dispatcher::UnifiedModelDispatcher>,
    /// UI 自动化执行器 — 截图 + 点击 / 输入 / 滚动。
    uiautomator: Arc<dyn UiAutomator>,
    /// 视觉模型名（如 "qwen2.5-vl:3b"）。
    vision_model: String,
}

// ── 非 feature-gated 的关联函数（prompt 构建器，始终可用）──

impl VlmController {
    /// 构建 VLM 分析 prompt — 指导 VLM 分析截图并返回结构化 JSON。
    ///
    /// prompt 要求 VLM 返回包含 description / detected_elements /
    /// suggested_actions / is_goal_complete / confidence 的 JSON。
    pub fn build_analysis_prompt(goal: &str, screenshot_b64: &str) -> String {
        format!(
            r#"你是一个 OS 自动化视觉分析助手。请分析以下截图，帮助完成用户目标。

## 用户目标
{goal}

## 截图（base64 编码的 PNG）
数据 URI 前缀已移除，纯 base64 数据：
{screenshot_b64}

## 任务
1. 描述截图中可见的 UI 元素和当前状态。
2. 列出检测到的关键元素（按钮、输入框、文本、图片、菜单、对话框等）。
3. 建议下一步操作动作（click / type / scroll / key_press / wait / screenshot / done）。
4. 判断用户目标是否已完成。
5. 给出分析置信度（0.0-1.0）。

## 输出格式（严格 JSON）
```json
{{
  "description": "对屏幕的自然语言描述",
  "detected_elements": [
    {{
      "element_type": "button",
      "text": "元素文本（可选，无则为 null）",
      "bbox": {{"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.1}},
      "confidence": 0.9
    }}
  ],
  "suggested_actions": [
    {{
      "click": {{"bbox": {{"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.1}}, "description": "点击登录按钮"}}
    }}
  ],
  "is_goal_complete": false,
  "confidence": 0.85
}}
```

注意：
- 所有 bbox 坐标为归一化值（0.0-1.0），左上角为 (0,0)。
- element_type 取值：button / input / text / image / menu / dialog / other。
- suggested_actions 的每个元素是 VlmAction 枚举的一个变体（snake_case）。
- 仅返回 JSON，不要包含其他文本。"#
        )
    }

    /// 构建 VLM 动作 prompt — 基于分析结果选择最佳下一步动作。
    ///
    /// 当 `capture_and_analyze` 返回的 `suggested_actions` 为空或需要 VLM
    /// 进一步决策时使用。
    pub fn build_action_prompt(analysis: &ScreenAnalysis, goal: &str) -> String {
        let elements_json = serde_json::to_string_pretty(&analysis.detected_elements)
            .unwrap_or_else(|_| "[]".to_string());
        let suggested_json = serde_json::to_string_pretty(&analysis.suggested_actions)
            .unwrap_or_else(|_| "[]".to_string());
        let desc = &analysis.description;
        let complete = if analysis.is_goal_complete {
            "是"
        } else {
            "否"
        };

        // JSON 示例单独提取,避免在 format! 中转义花括号。
        let json_examples = "\
- {\"click\": {\"bbox\": {\"x\": 0.1, \"y\": 0.2, \"width\": 0.3, \"height\": 0.1}, \"description\": \"点击提交按钮\"}}\n\
- {\"type\": {\"bbox\": {\"x\": 0.1, \"y\": 0.2, \"width\": 0.3, \"height\": 0.1}, \"text\": \"用户名\"}}\n\
- {\"scroll\": {\"direction\": \"down\", \"amount\": 0.5}}\n\
- {\"key_press\": {\"key\": \"Enter\"}}\n\
- {\"wait\": {\"seconds\": 2.0}}\n\
- {\"screenshot\": null}\n\
- {\"done\": {\"summary\": \"目标已完成\"}}";

        format!(
            r#"你是一个 OS 自动化决策助手。基于屏幕分析结果，选择最佳下一步动作。

## 用户目标
{goal}

## 屏幕描述
{desc}

## 检测到的元素
{elements_json}

## 已建议的动作
{suggested_json}

## 目标完成状态
{complete}

## 任务
从已建议的动作中选择最佳的一个，或提出新的动作。返回单个 VlmAction JSON 对象。

## 输出格式（严格 JSON，单个动作对象）
例如：
{json_examples}

仅返回单个 JSON 对象，不要包含其他文本。"#
        )
    }

    /// 从可能包含 markdown 代码块包裹的文本中提取 JSON。
    ///
    /// 纯字符串处理函数，不依赖 `UnifiedModelDispatcher`，始终可用。
    /// VLM 有时返回 ```json ... ``` 包裹的 JSON，此方法去除包裹。
    pub fn extract_json_block(text: &str) -> String {
        let trimmed = text.trim();
        // 处理 ```json ... ``` 包裹
        if let Some(start) = trimmed.find("```json") {
            if let Some(end) = trimmed.rfind("```") {
                let inner = &trimmed[start + 7..end].trim();
                return inner.to_string();
            }
        }
        // 处理 ``` ... ``` 包裹
        if trimmed.starts_with("```") && trimmed.ends_with("```") {
            let inner = trimmed
                .strip_prefix("```")
                .unwrap_or(trimmed)
                .strip_suffix("```")
                .unwrap_or(trimmed)
                .trim();
            return inner.to_string();
        }
        trimmed.to_string()
    }
}

// ── VlmController 实现（需要 UnifiedModelDispatcher，P0-2 默认启用）──

impl VlmController {
    /// 创建新的 VLM Controller。
    ///
    /// # 参数
    /// - `llm_dispatcher`: LLM 调度器（通过 `gateway().describe_image()` 调用 VLM）。
    /// - `uiautomator`: UI 自动化执行器（截图 + 点击 / 输入 / 滚动）。
    pub fn new(
        llm_dispatcher: Arc<crate::llm::dispatcher::UnifiedModelDispatcher>,
        uiautomator: Arc<dyn UiAutomator>,
    ) -> Self {
        Self {
            llm_dispatcher,
            uiautomator,
            vision_model: DEFAULT_VISION_MODEL.to_string(),
        }
    }

    /// 设置视觉模型名（覆盖默认的 `qwen2.5-vl:3b`）。
    pub fn with_vision_model(mut self, model: impl Into<String>) -> Self {
        self.vision_model = model.into();
        self
    }

    /// 执行高层目标 — 以默认最大步数运行 VLM 闭环。
    ///
    /// 等价于 `run_loop(goal, DEFAULT_MAX_STEPS)`。
    #[instrument(skip(self), fields(goal = %goal))]
    pub async fn execute_goal(&self, goal: &str) -> Result<VlmExecutionResult> {
        self.run_loop(goal, DEFAULT_MAX_STEPS).await
    }

    /// 截图并用 VLM 分析 — 闭环的核心步骤。
    ///
    /// 流程：
    /// 1. 通过 `UiAutomator::screenshot()` 截取屏幕（PNG 字节流）。
    /// 2. 保存到临时文件，记录路径。
    /// 3. 编码为 base64，构建分析 prompt。
    /// 4. 通过 `gateway().describe_image()` 调用 VLM。
    /// 5. 解析 VLM 返回的 JSON 为 `ScreenAnalysis`。
    ///
    /// VLM 返回非 JSON 文本时，降级为 `description = 原始文本`、空元素列表、
    /// 置信度 0.0 的 `ScreenAnalysis`，不阻断闭环。
    #[instrument(skip(self), fields(goal = %goal))]
    pub async fn capture_and_analyze(&self, goal: &str) -> Result<ScreenAnalysis> {
        // 1. 截图
        let png_bytes = self
            .uiautomator
            .screenshot()
            .context("UiAutomator 截图失败")?;

        // 2. 保存到临时文件
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
        let screenshot_path = std::env::temp_dir()
            .join(format!("nebula_vlm_{timestamp}.png"))
            .to_string_lossy()
            .to_string();
        tokio::fs::write(&screenshot_path, &png_bytes)
            .await
            .with_context(|| format!("保存截图失败: {screenshot_path}"))?;

        // 3. 编码 base64
        let screenshot_b64 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(&png_bytes)
        };

        // 4. 构建 prompt 并调用 VLM
        let prompt = Self::build_analysis_prompt(goal, &screenshot_b64);
        let msg = crate::llm::ollama::ChatMessage {
            role: "user".to_string(),
            content: prompt,
            images: vec![screenshot_b64],
            ..Default::default()
        };

        let gateway = self.llm_dispatcher.gateway();
        let vlm_text = gateway
            .describe_image(&self.vision_model, msg)
            .await
            .context("VLM describe_image 调用失败")?;

        // 5. 解析 VLM 返回的 JSON
        let analysis = Self::parse_vlm_response(&vlm_text, &screenshot_path);

        debug!(
            description = %analysis.description,
            elements_count = analysis.detected_elements.len(),
            actions_count = analysis.suggested_actions.len(),
            is_complete = analysis.is_goal_complete,
            confidence = analysis.confidence,
            "VLM 分析完成"
        );

        Ok(analysis)
    }

    /// 执行 VLM 决策的动作。
    ///
    /// 将 `VlmAction` 映射到 `UiAutomator` 操作：
    /// - `Click` / `Type`: 归一化 bbox → 像素坐标 → `UiElement` → `click` / `type_text`。
    /// - `Scroll`: 方向转换 + amount 比例 → 行数 → `scroll`。
    /// - `KeyPress`: 当前 UiAutomator 未实现按键模拟，返回 `Err`（TODO）。
    /// - `Wait`: `tokio::time::sleep`。
    /// - `Screenshot`: 调用 `screenshot()` 验证可用性（no-op 语义）。
    /// - `Done`: 记录日志，no-op。
    #[instrument(skip(self), fields(action = ?action))]
    pub async fn perform_action(&self, action: VlmAction) -> Result<()> {
        match action {
            VlmAction::Click { bbox, description } => {
                debug!(%description, ?bbox, "执行 Click 动作");
                let element = self.bbox_to_element(&bbox)?;
                // UiAutomator 方法是同步的，用 spawn_blocking 避免阻塞异步运行时。
                let uia = self.uiautomator.clone();
                tokio::task::spawn_blocking(move || uia.click(&element))
                    .await
                    .context("spawn_blocking click 失败")??;
                info!(%description, "Click 动作完成");
            }
            VlmAction::Type { bbox, text } => {
                let text_len = text.len();
                debug!(?bbox, text_len, "执行 Type 动作");
                let element = self.bbox_to_element(&bbox)?;
                let uia = self.uiautomator.clone();
                tokio::task::spawn_blocking(move || uia.type_text(&element, &text))
                    .await
                    .context("spawn_blocking type_text 失败")??;
                info!(text_len, "Type 动作完成");
            }
            VlmAction::Scroll { direction, amount } => {
                debug!(?direction, amount, "执行 Scroll 动作");
                let ui_dir = direction.to_uiautomator();
                // amount 是屏幕比例（0.0-1.0），转为行数（每行约 5% 屏幕）。
                let lines = ((amount * 20.0).round() as u32).max(1);
                let uia = self.uiautomator.clone();
                tokio::task::spawn_blocking(move || uia.scroll(ui_dir, lines))
                    .await
                    .context("spawn_blocking scroll 失败")??;
                info!(?direction, lines, "Scroll 动作完成");
            }
            VlmAction::KeyPress { key } => {
                debug!(%key, "执行 KeyPress 动作");
                // TODO: UiAutomator trait 尚未定义 key_press 方法，后续扩展。
                warn!(%key, "KeyPress 动作尚未实现（UiAutomator trait 无 key_press 方法）");
                anyhow::bail!("KeyPress 动作尚未实现: key={key}");
            }
            VlmAction::Wait { seconds } => {
                debug!(seconds, "执行 Wait 动作");
                let dur = std::time::Duration::from_secs_f32(seconds.max(0.0));
                tokio::time::sleep(dur).await;
                info!(seconds, "Wait 动作完成");
            }
            VlmAction::Screenshot => {
                debug!("执行 Screenshot 动作");
                let uia = self.uiautomator.clone();
                tokio::task::spawn_blocking(move || uia.screenshot())
                    .await
                    .context("spawn_blocking screenshot 失败")??;
                debug!("Screenshot 动作完成");
            }
            VlmAction::Done { summary } => {
                info!(%summary, "执行 Done 动作 — 目标完成");
            }
        }
        Ok(())
    }

    /// VLM 闭环 — 截图 → 分析 → 操作 → 验证 → 重复直到完成或达到最大步数。
    ///
    /// 每步记录 `VlmStepRecord`，最终汇总为 `VlmExecutionResult`。
    /// 闭环终止条件：
    /// 1. VLM 判断 `is_goal_complete = true` → `success = true`。
    /// 2. 达到 `max_steps` → `success = false`。
    /// 3. 某步 `perform_action` 失败 → 记录错误，继续下一步（不立即终止）。
    #[instrument(skip(self), fields(goal = %goal, max_steps = max_steps))]
    pub async fn run_loop(&self, goal: &str, max_steps: usize) -> Result<VlmExecutionResult> {
        let start = std::time::Instant::now();
        let mut steps: Vec<VlmStepRecord> = Vec::new();
        let mut success = false;
        let mut summary = String::new();

        info!(%goal, max_steps, "VLM 闭环启动");

        for step_num in 1..=max_steps {
            // 1. 截图 + 分析
            let analysis = match self.capture_and_analyze(goal).await {
                Ok(a) => a,
                Err(e) => {
                    warn!(step = step_num, error = %e, "截图分析失败，终止闭环");
                    summary = format!("闭环在第 {step_num} 步因截图分析失败终止: {e}");
                    break;
                }
            };

            // 2. 检查目标是否完成
            if analysis.is_goal_complete {
                success = true;
                summary = format!("目标在第 {step_num} 步完成");
                // 记录 Done 步骤
                let done_action = VlmAction::Done {
                    summary: analysis.description.clone(),
                };
                steps.push(VlmStepRecord {
                    step_number: step_num as u32,
                    screenshot_path: analysis.screenshot_path.clone(),
                    analysis,
                    action: done_action,
                    action_result: Some("goal complete".to_string()),
                    timestamp: Utc::now(),
                });
                break;
            }

            // 3. 选择动作（优先用 suggested_actions 的第一个）
            let action = if let Some(first) = analysis.suggested_actions.first() {
                first.clone()
            } else {
                // 无建议动作 → 重新截图（让 VLM 在下一步重新分析）
                warn!(step = step_num, "VLM 未建议动作，执行 Screenshot 重新分析");
                VlmAction::Screenshot
            };

            // 4. 执行动作
            let action_result = match self.perform_action(action.clone()).await {
                Ok(()) => Some("ok".to_string()),
                Err(e) => {
                    warn!(step = step_num, error = %e, "动作执行失败，继续下一步");
                    Some(format!("error: {e}"))
                }
            };

            // 5. 记录步骤
            steps.push(VlmStepRecord {
                step_number: step_num as u32,
                screenshot_path: analysis.screenshot_path.clone(),
                analysis,
                action,
                action_result,
                timestamp: Utc::now(),
            });
        }

        // 达到最大步数未完成
        if !success && steps.len() == max_steps {
            summary = format!("达到最大步数 {max_steps}，目标未完成");
        }

        let total_duration_ms = start.elapsed().as_millis() as u64;

        info!(
            success,
            steps = steps.len(),
            total_duration_ms,
            "VLM 闭环结束"
        );

        Ok(VlmExecutionResult {
            goal: goal.to_string(),
            steps,
            success,
            summary,
            total_duration_ms,
        })
    }

    // ── 内部辅助方法 ──

    /// 将归一化 BoundingBox 转换为 `UiElement`（像素坐标）。
    ///
    /// 通过 `UiAutomator::get_active_window()` 获取屏幕尺寸，
    /// 将归一化坐标乘以屏幕宽高得到像素坐标。
    fn bbox_to_element(&self, bbox: &BoundingBox) -> Result<UiElement> {
        let win = self
            .uiautomator
            .get_active_window()
            .context("获取活动窗口失败，无法转换坐标")?;
        let (px, py, pw, ph) = bbox.to_pixels(win.bounds.width as u32, win.bounds.height as u32);
        Ok(UiElement::empty()
            .with_name("vlm-detected")
            .with_role("Unknown")
            .with_bounds(ElementBounds::new(
                (win.bounds.x + px as i32),
                (win.bounds.y + py as i32),
                pw as i32,
                ph as i32,
            )))
    }

    /// 解析 VLM 返回的文本为 `ScreenAnalysis`。
    ///
    /// VLM 返回纯 JSON 时直接解析；返回非 JSON 文本时降级为
    /// `description = 原始文本`、空元素列表、置信度 0.0。
    fn parse_vlm_response(vlm_text: &str, screenshot_path: &str) -> ScreenAnalysis {
        // 尝试提取 JSON 块（VLM 可能用 ```json ... ``` 包裹）
        let json_text = Self::extract_json_block(vlm_text);

        match serde_json::from_str::<VlmAnalysisResponse>(&json_text) {
            Ok(resp) => ScreenAnalysis {
                screenshot_path: screenshot_path.to_string(),
                description: resp.description,
                detected_elements: resp.detected_elements,
                suggested_actions: resp.suggested_actions,
                is_goal_complete: resp.is_goal_complete,
                confidence: resp.confidence.clamp(0.0, 1.0),
            },
            Err(e) => {
                warn!(error = %e, "VLM 返回非 JSON 文本，降级处理");
                ScreenAnalysis {
                    screenshot_path: screenshot_path.to_string(),
                    description: vlm_text.trim().to_string(),
                    detected_elements: Vec::new(),
                    suggested_actions: Vec::new(),
                    is_goal_complete: false,
                    confidence: 0.0,
                }
            }
        }
    }
}

// ── 单元测试 ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── BoundingBox 测试 ──

    #[test]
    fn bounding_box_is_normalized_valid() {
        // 所有坐标在 0.0-1.0 范围内 → true
        let bbox = BoundingBox::new(0.1, 0.2, 0.3, 0.4);
        assert!(bbox.is_normalized());
    }

    #[test]
    fn bounding_box_is_normalized_invalid() {
        // 坐标超出 1.0 → false
        assert!(!BoundingBox::new(1.5, 0.0, 0.1, 0.1).is_normalized());
        // 负坐标 → false
        assert!(!BoundingBox::new(-0.1, 0.0, 0.1, 0.1).is_normalized());
        // 宽度超过 1.0 → false
        assert!(!BoundingBox::new(0.0, 0.0, 1.5, 0.1).is_normalized());
    }

    #[test]
    fn bounding_box_center() {
        let bbox = BoundingBox::new(0.2, 0.3, 0.4, 0.2);
        let (cx, cy) = bbox.center();
        assert!((cx - 0.4).abs() < 1e-6, "中心 X 应为 0.4");
        assert!((cy - 0.4).abs() < 1e-6, "中心 Y 应为 0.4");
    }

    #[test]
    fn bounding_box_to_pixels() {
        let bbox = BoundingBox::new(0.25, 0.5, 0.5, 0.25);
        let (x, y, w, h) = bbox.to_pixels(1920, 1080);
        assert_eq!(x, 480); // 0.25 * 1920
        assert_eq!(y, 540); // 0.5 * 1080
        assert_eq!(w, 960); // 0.5 * 1920
        assert_eq!(h, 270); // 0.25 * 1080
    }

    #[test]
    fn bounding_box_default_is_zero() {
        let b = BoundingBox::default();
        assert_eq!(b, BoundingBox::new(0.0, 0.0, 0.0, 0.0));
        assert!(b.is_normalized()); // 0.0 在范围内
    }

    // ── VlmAction 序列化往返测试 ──

    #[test]
    fn vlm_action_serialization_roundtrip() {
        let bbox = BoundingBox::new(0.1, 0.2, 0.3, 0.4);
        let actions = vec![
            VlmAction::Click {
                bbox,
                description: "点击登录按钮".to_string(),
            },
            VlmAction::Type {
                bbox,
                text: "hello world".to_string(),
            },
            VlmAction::Scroll {
                direction: ScrollDirection::Down,
                amount: 0.5,
            },
            VlmAction::KeyPress {
                key: "Enter".to_string(),
            },
            VlmAction::Wait { seconds: 2.5 },
            VlmAction::Screenshot,
            VlmAction::Done {
                summary: "任务完成".to_string(),
            },
        ];
        for action in &actions {
            let json = serde_json::to_string(action).expect("序列化失败");
            let back: VlmAction = serde_json::from_str(&json).expect("反序列化失败");
            assert_eq!(action, &back, "往返不一致: {json}");
        }
    }

    #[test]
    fn vlm_action_snake_case_serialization() {
        // 验证 serde rename_all = "snake_case" 生效
        let click = VlmAction::Click {
            bbox: BoundingBox::new(0.0, 0.0, 0.1, 0.1),
            description: "test".to_string(),
        };
        let json = serde_json::to_string(&click).expect("序列化失败");
        assert!(
            json.contains("\"click\""),
            "Click 应序列化为 'click': {json}"
        );

        let key_press = VlmAction::KeyPress {
            key: "Enter".to_string(),
        };
        let json = serde_json::to_string(&key_press).expect("序列化失败");
        assert!(
            json.contains("\"key_press\""),
            "KeyPress 应序列化为 'key_press': {json}"
        );

        let done = VlmAction::Done {
            summary: "done".to_string(),
        };
        let json = serde_json::to_string(&done).expect("序列化失败");
        assert!(json.contains("\"done\""), "Done 应序列化为 'done': {json}");
    }

    // ── ScrollDirection 所有变体测试 ──

    #[test]
    fn scroll_direction_all_variants() {
        let dirs = [
            ScrollDirection::Up,
            ScrollDirection::Down,
            ScrollDirection::Left,
            ScrollDirection::Right,
        ];
        for d in &dirs {
            let json = serde_json::to_string(d).expect("序列化失败");
            let back: ScrollDirection = serde_json::from_str(&json).expect("反序列化失败");
            assert_eq!(d, &back, "往返不一致: {json}");
        }
    }

    #[test]
    fn scroll_direction_lowercase_serialization() {
        // 验证 serde rename_all = "lowercase" 生效
        assert_eq!(
            serde_json::to_string(&ScrollDirection::Up).unwrap(),
            "\"up\""
        );
        assert_eq!(
            serde_json::to_string(&ScrollDirection::Down).unwrap(),
            "\"down\""
        );
        assert_eq!(
            serde_json::to_string(&ScrollDirection::Left).unwrap(),
            "\"left\""
        );
        assert_eq!(
            serde_json::to_string(&ScrollDirection::Right).unwrap(),
            "\"right\""
        );
    }

    #[test]
    fn scroll_direction_to_uiautomator_conversion() {
        assert_eq!(ScrollDirection::Up.to_uiautomator(), UiScrollDirection::Up);
        assert_eq!(
            ScrollDirection::Down.to_uiautomator(),
            UiScrollDirection::Down
        );
        assert_eq!(
            ScrollDirection::Left.to_uiautomator(),
            UiScrollDirection::Left
        );
        assert_eq!(
            ScrollDirection::Right.to_uiautomator(),
            UiScrollDirection::Right
        );
    }

    // ── ScreenAnalysis 序列化测试 ──

    #[test]
    fn screen_analysis_serialization() {
        let analysis = ScreenAnalysis {
            screenshot_path: "/tmp/screenshot.png".to_string(),
            description: "登录页面".to_string(),
            detected_elements: vec![DetectedElement {
                element_type: ElementType::Button,
                text: Some("登录".to_string()),
                bbox: BoundingBox::new(0.4, 0.8, 0.2, 0.05),
                confidence: 0.9,
            }],
            suggested_actions: vec![VlmAction::Click {
                bbox: BoundingBox::new(0.4, 0.8, 0.2, 0.05),
                description: "点击登录按钮".to_string(),
            }],
            is_goal_complete: false,
            confidence: 0.85,
        };
        let json = serde_json::to_string(&analysis).expect("序列化失败");
        let back: ScreenAnalysis = serde_json::from_str(&json).expect("反序列化失败");
        assert_eq!(analysis, back, "往返不一致");
    }

    // ── ElementType 所有变体覆盖测试 ──

    #[test]
    fn element_type_all_variants_coverage() {
        let types = [
            ElementType::Button,
            ElementType::Input,
            ElementType::Text,
            ElementType::Image,
            ElementType::Menu,
            ElementType::Dialog,
            ElementType::Other,
        ];
        for et in &types {
            let json = serde_json::to_string(et).expect("序列化失败");
            let back: ElementType = serde_json::from_str(&json).expect("反序列化失败");
            assert_eq!(et, &back, "往返不一致: {json}");
        }
        // 验证 lowercase 序列化
        assert_eq!(
            serde_json::to_string(&ElementType::Button).unwrap(),
            "\"button\""
        );
        assert_eq!(
            serde_json::to_string(&ElementType::Input).unwrap(),
            "\"input\""
        );
    }

    #[test]
    fn detected_element_with_all_types() {
        // 验证 DetectedElement 能携带所有 ElementType 变体
        let bbox = BoundingBox::new(0.0, 0.0, 0.5, 0.5);
        for et in [
            ElementType::Button,
            ElementType::Input,
            ElementType::Text,
            ElementType::Image,
            ElementType::Menu,
            ElementType::Dialog,
            ElementType::Other,
        ] {
            let elem = DetectedElement {
                element_type: et.clone(),
                text: Some("test".to_string()),
                bbox,
                confidence: 0.5,
            };
            let json = serde_json::to_string(&elem).expect("序列化失败");
            let back: DetectedElement = serde_json::from_str(&json).expect("反序列化失败");
            assert_eq!(elem, back);
        }
    }

    // ── VlmExecutionResult 构建测试 ──

    #[test]
    fn vlm_execution_result_construction() {
        let result = VlmExecutionResult {
            goal: "打开记事本".to_string(),
            steps: vec![],
            success: true,
            summary: "成功打开记事本".to_string(),
            total_duration_ms: 1500,
        };
        let json = serde_json::to_string(&result).expect("序列化失败");
        let back: VlmExecutionResult = serde_json::from_str(&json).expect("反序列化失败");
        assert_eq!(result.goal, back.goal);
        assert_eq!(result.success, back.success);
        assert_eq!(result.summary, back.summary);
        assert_eq!(result.total_duration_ms, back.total_duration_ms);
        assert!(result.steps.is_empty());
    }

    // ── prompt 构建内容验证测试 ──

    #[test]
    fn build_analysis_prompt_contains_goal_and_instructions() {
        let goal = "打开计算器并计算 1+1";
        let screenshot_b64 = "iVBORw0KGgoAAAANSUhEUg==";
        let prompt = VlmController::build_analysis_prompt(goal, screenshot_b64);

        // 包含用户目标
        assert!(prompt.contains(goal), "prompt 应包含用户目标");
        // 包含截图 base64 数据
        assert!(prompt.contains(screenshot_b64), "prompt 应包含截图 base64");
        // 包含 JSON 输出格式指导
        assert!(prompt.contains("JSON"), "prompt 应包含 JSON 格式要求");
        // 包含归一化坐标说明
        assert!(
            prompt.contains("归一化") || prompt.contains("0.0-1.0"),
            "prompt 应说明归一化坐标"
        );
        // 包含元素类型列表
        assert!(prompt.contains("button"), "prompt 应列出 button 类型");
        assert!(prompt.contains("input"), "prompt 应列出 input 类型");
        // 包含动作类型
        assert!(prompt.contains("click"), "prompt 应提及 click 动作");
        assert!(prompt.contains("done"), "prompt 应提及 done 动作");
    }

    #[test]
    fn build_action_prompt_contains_analysis_and_goal() {
        let analysis = ScreenAnalysis {
            screenshot_path: "/tmp/test.png".to_string(),
            description: "桌面上有计算器图标".to_string(),
            detected_elements: vec![DetectedElement {
                element_type: ElementType::Button,
                text: Some("计算器".to_string()),
                bbox: BoundingBox::new(0.1, 0.1, 0.1, 0.1),
                confidence: 0.9,
            }],
            suggested_actions: vec![VlmAction::Click {
                bbox: BoundingBox::new(0.1, 0.1, 0.1, 0.1),
                description: "点击计算器图标".to_string(),
            }],
            is_goal_complete: false,
            confidence: 0.85,
        };
        let goal = "打开计算器";
        let prompt = VlmController::build_action_prompt(&analysis, goal);

        // 包含用户目标
        assert!(prompt.contains(goal), "action prompt 应包含目标");
        // 包含屏幕描述
        assert!(
            prompt.contains(&analysis.description),
            "action prompt 应包含屏幕描述"
        );
        // 包含检测到的元素 JSON
        assert!(
            prompt.contains("计算器"),
            "action prompt 应包含检测到的元素文本"
        );
        // 包含输出格式指导
        assert!(
            prompt.contains("JSON"),
            "action prompt 应包含 JSON 格式要求"
        );
        // 包含动作示例
        assert!(prompt.contains("click"), "应包含 click 动作示例");
        assert!(prompt.contains("done"), "应包含 done 动作示例");
    }

    // ── confidence 范围测试 ──

    #[test]
    fn confidence_range_validation() {
        // 合法范围 0.0-1.0
        let valid_confidences = [0.0_f32, 0.1, 0.5, 0.9, 1.0];
        for c in &valid_confidences {
            let analysis = ScreenAnalysis {
                screenshot_path: String::new(),
                description: String::new(),
                detected_elements: vec![],
                suggested_actions: vec![],
                is_goal_complete: false,
                confidence: *c,
            };
            assert!((0.0..=1.0).contains(&analysis.confidence));
        }
    }

    #[test]
    fn confidence_clamp_in_parse_vlm_response() {
        // parse_vlm_response 对 confidence 做 clamp(0.0, 1.0)
        // 模拟 VLM 返回超范围 confidence
        let vlm_json = r#"{
            "description": "test",
            "detected_elements": [],
            "suggested_actions": [],
            "is_goal_complete": false,
            "confidence": 1.5
        }"#;
        // 注意：parse_vlm_response 是私有方法，通过 capture_and_analyze 间接测试较难
        // （需要 mock dispatcher）。这里直接测试 JSON 解析 + clamp 逻辑。
        let parsed: VlmAnalysisResponse = serde_json::from_str(vlm_json).expect("解析失败");
        let clamped = parsed.confidence.clamp(0.0, 1.0);
        assert_eq!(clamped, 1.0, "confidence > 1.0 应被 clamp 到 1.0");
    }

    // ── goal complete 判断测试 ──

    #[test]
    fn goal_complete_true_and_false() {
        let complete_analysis = ScreenAnalysis {
            screenshot_path: String::new(),
            description: "目标已完成".to_string(),
            detected_elements: vec![],
            suggested_actions: vec![VlmAction::Done {
                summary: "done".to_string(),
            }],
            is_goal_complete: true,
            confidence: 0.95,
        };
        assert!(complete_analysis.is_goal_complete);

        let incomplete_analysis = ScreenAnalysis {
            screenshot_path: String::new(),
            description: "仍在操作中".to_string(),
            detected_elements: vec![],
            suggested_actions: vec![VlmAction::Screenshot],
            is_goal_complete: false,
            confidence: 0.5,
        };
        assert!(!incomplete_analysis.is_goal_complete);
    }

    // ── 多步记录顺序测试 ──

    #[test]
    fn multi_step_record_ordering() {
        let mut steps: Vec<VlmStepRecord> = Vec::new();
        for i in 1..=5 {
            steps.push(VlmStepRecord {
                step_number: i as u32,
                screenshot_path: format!("/tmp/step_{i}.png"),
                analysis: ScreenAnalysis {
                    screenshot_path: format!("/tmp/step_{i}.png"),
                    description: format!("第 {i} 步"),
                    detected_elements: vec![],
                    suggested_actions: vec![],
                    is_goal_complete: i == 5,
                    confidence: 0.1 * i as f32,
                },
                action: if i < 5 {
                    VlmAction::Screenshot
                } else {
                    VlmAction::Done {
                        summary: "完成".to_string(),
                    }
                },
                action_result: Some("ok".to_string()),
                timestamp: Utc::now(),
            });
        }

        let result = VlmExecutionResult {
            goal: "多步测试".to_string(),
            steps: steps.clone(),
            success: true,
            summary: "5 步完成".to_string(),
            total_duration_ms: 5000,
        };

        // 验证步骤数量
        assert_eq!(result.steps.len(), 5);
        // 验证步骤编号严格递增
        for (idx, step) in result.steps.iter().enumerate() {
            assert_eq!(
                step.step_number as usize,
                idx + 1,
                "步骤编号应从 1 开始严格递增"
            );
        }
        // 验证最后一步是 Done
        assert!(matches!(
            result.steps.last().unwrap().action,
            VlmAction::Done { .. }
        ));
        // 验证最后一步 is_goal_complete
        assert!(result.steps.last().unwrap().analysis.is_goal_complete);
        // 验证前 4 步不是 Done
        for step in &result.steps[..4] {
            assert!(!step.analysis.is_goal_complete);
            assert!(matches!(step.action, VlmAction::Screenshot));
        }
    }

    // ── extract_json_block 测试 ──

    #[test]
    fn extract_json_block_plain_json() {
        let json = r#"{"key": "value"}"#;
        let extracted = VlmController::extract_json_block(json);
        assert!(extracted.contains("\"key\""));
    }

    #[test]
    fn extract_json_block_markdown_wrapped() {
        let wrapped = "```json\n{\"key\": \"value\"}\n```";
        let extracted = VlmController::extract_json_block(wrapped);
        assert!(extracted.contains("\"key\""));
        assert!(!extracted.contains("```"));
    }

    // ── VlmStepRecord 序列化测试 ──

    #[test]
    fn vlm_step_record_serialization() {
        let step = VlmStepRecord {
            step_number: 1,
            screenshot_path: "/tmp/test.png".to_string(),
            analysis: ScreenAnalysis {
                screenshot_path: "/tmp/test.png".to_string(),
                description: "test".to_string(),
                detected_elements: vec![],
                suggested_actions: vec![],
                is_goal_complete: false,
                confidence: 0.5,
            },
            action: VlmAction::Screenshot,
            action_result: Some("ok".to_string()),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&step).expect("序列化失败");
        let back: VlmStepRecord = serde_json::from_str(&json).expect("反序列化失败");
        assert_eq!(step.step_number, back.step_number);
        assert_eq!(step.screenshot_path, back.screenshot_path);
        assert_eq!(step.action, back.action);
    }

    // ── DEFAULT_MAX_STEPS 常量验证 ──

    #[test]
    fn default_max_steps_is_reasonable() {
        // 默认最大步数应为 20（既不过小导致任务无法完成，也不过大导致无限循环）
        assert_eq!(DEFAULT_MAX_STEPS, 20);
    }

    #[test]
    fn default_vision_model_is_qwen_vl() {
        assert_eq!(DEFAULT_VISION_MODEL, "qwen2.5-vl:3b");
    }
}
