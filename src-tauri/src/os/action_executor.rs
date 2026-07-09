//! T-E-C-04: ActionExecutor — 统一 UI 动作执行器。
//!
//! 将高层"动作意图"([`UiAction`])转换为底层 OS 操作序列(调用
//! [`crate::os::uiautomator::UiAutomator`] trait 的具体实现)。
//!
//! ## 架构
//!
//! * [`UiAction`] — 高层动作意图枚举,serde snake_case 序列化,跨平台中立。
//! * [`ActionTarget`] — 动作目标(坐标 / 元素 / 图像)。
//! * [`ActionExecutor`] — 统一执行器,支持单动作、序列、重试、条件执行。
//! * [`ExecutionPlan`] — 可序列化的执行计划(YAML 往返),供配置驱动场景使用。
//!
//! ## 设计取舍
//!
//! 底层 [`UiAutomator`] trait 当前仅暴露 `find_element` / `click` / `type_text`
//! / `scroll` / `screenshot` 等原语,不直接支持右键、拖拽、悬停、按键注入。
//! 本执行器对支持的动作直接翻译;对暂不支持的动作返回结构化错误
//! (动作本身合法,仅后端尚未实现),以便后续任务填充底层能力时无需改动本层。
//!
//! ## 注册
//!
//! 本模块当前未在 `os/mod.rs` 中注册(遵循"只创建新文件"约束)。注册时添加:
//! ```ignore
//! // in src-tauri/src/os/mod.rs
//! pub mod action_executor;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// 复用既有 UiAutomator 抽象层。
use crate::os::uiautomator as ua;
// 引入 trait 以便方法解析 + blanket impl 引用。
use ua::UiAutomator;

// ----------------------------------------------------------------------
// UiAutomatorTrait — 既有 UiAutomator trait 的对象安全别名
// ----------------------------------------------------------------------

/// `UiAutomatorTrait` — 指向既有 [`ua::UiAutomator`] trait 的对象安全别名。
///
/// 任务契约要求 `ActionExecutor::new(uiautomator: Arc<dyn UiAutomatorTrait>)`。
/// 既有 trait 名为 `UiAutomator`;为满足签名且不修改既有文件,此处采用
/// "扩展 trait + blanket impl"模式:`UiAutomatorTrait` 继承 `UiAutomator` 且
/// 无额外方法,任何实现了 `UiAutomator` 的类型自动实现 `UiAutomatorTrait`,
/// 因此 `Arc<dyn UiAutomatorTrait>` 可作为 trait object 使用,既有
/// [`ua::MockUiAutomator`] / [`ua::WindowsUiAutomator`] 均可直接注入。
pub trait UiAutomatorTrait: UiAutomator {}
impl<T: UiAutomator> UiAutomatorTrait for T {}

// ----------------------------------------------------------------------
// 键盘修饰键 / 滚动方向 / 边界框
// ----------------------------------------------------------------------

/// 键盘修饰键。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KeyModifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

/// 滚动方向。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// 边界矩形(屏幕坐标,像素)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BoundingBox {
    /// 左上角 X 坐标。
    pub x: i32,
    /// 左上角 Y 坐标。
    pub y: i32,
    /// 宽度。
    pub width: i32,
    /// 高度。
    pub height: i32,
}

impl BoundingBox {
    /// 创建新的边界矩形。
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

// ----------------------------------------------------------------------
// ElementSelector — 灵活的元素定位条件(可组合)
// ----------------------------------------------------------------------

/// 元素定位条件 — 结构体形式,各字段可选,支持组合(取首个命中的策略翻译为
/// 底层 [`ua::ElementSelector`] 单一策略)。
///
/// 翻译优先级:`by_id` > `by_text` > `by_partial_text` > `by_class` > `by_role`。
/// `by_partial_text` 在底层 trait 无对应策略时降级为 `ByText`(best-effort)。
/// `index` 用于在多个匹配中选取,但底层 trait 仅返回首个匹配,故 `index > 0`
/// 为 best-effort(当前不生效)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ElementSelector {
    /// 按 AutomationId / accessibility identifier 定位。
    pub by_id: Option<String>,
    /// 按可见文本(精确匹配)定位。
    pub by_text: Option<String>,
    /// 按可见文本(子串匹配)定位(底层降级为精确匹配)。
    pub by_partial_text: Option<String>,
    /// 按控件类名(ClassName)定位。
    pub by_class: Option<String>,
    /// 按控件角色(ControlType / Role)定位,如 "Button" / "Edit"。
    pub by_role: Option<String>,
    /// 在多个匹配中选取的索引(0 基);当前后端仅返回首个,best-effort。
    pub index: Option<usize>,
}

impl ElementSelector {
    /// 创建空选择器(所有条件为 None)。
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否至少设置了一个定位条件(不含 `index`)。
    pub fn has_criterion(&self) -> bool {
        self.by_id.is_some()
            || self.by_text.is_some()
            || self.by_partial_text.is_some()
            || self.by_class.is_some()
            || self.by_role.is_some()
    }
}

// ----------------------------------------------------------------------
// ActionTarget — 动作目标
// ----------------------------------------------------------------------

/// 动作目标 — 坐标模式 / 元素模式 / 图像模式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionTarget {
    /// 屏幕坐标(像素)。
    Coordinates { x: i32, y: i32 },
    /// 通过元素选择器定位。
    Element { selector: ElementSelector },
    /// 通过图像模板匹配定位(需 vision 后端)。
    Image { path: String, confidence: f32 },
}

// ----------------------------------------------------------------------
// UiAction — 高层动作意图
// ----------------------------------------------------------------------

/// 高层 UI 动作意图枚举(serde snake_case)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UiAction {
    /// 单击。
    Click { target: ActionTarget },
    /// 双击。
    DoubleClick { target: ActionTarget },
    /// 右键单击。
    RightClick { target: ActionTarget },
    /// 输入文本。
    Type {
        target: ActionTarget,
        text: String,
        clear_first: bool,
    },
    /// 按键(含修饰键组合)。
    KeyPress {
        keys: Vec<String>,
        modifiers: Vec<KeyModifier>,
    },
    /// 滚动。
    Scroll {
        target: Option<ActionTarget>,
        direction: ScrollDirection,
        amount: u32,
    },
    /// 拖拽(从 `from` 到 `to`,耗时 `duration_ms`)。
    Drag {
        from: ActionTarget,
        to: ActionTarget,
        duration_ms: u64,
    },
    /// 悬停。
    Hover { target: ActionTarget },
    /// 聚焦元素。
    Focus { target: ActionTarget },
    /// 等待(纯延时)。
    Wait { duration_ms: u64 },
    /// 截图(可选区域;区域裁剪需 vision 后端)。
    Screenshot { region: Option<BoundingBox> },
}

impl UiAction {
    /// 返回动作类型名(对应 serde snake_case 变体名)。
    pub fn type_name(&self) -> &'static str {
        match self {
            UiAction::Click { .. } => "click",
            UiAction::DoubleClick { .. } => "double_click",
            UiAction::RightClick { .. } => "right_click",
            UiAction::Type { .. } => "type",
            UiAction::KeyPress { .. } => "key_press",
            UiAction::Scroll { .. } => "scroll",
            UiAction::Drag { .. } => "drag",
            UiAction::Hover { .. } => "hover",
            UiAction::Focus { .. } => "focus",
            UiAction::Wait { .. } => "wait",
            UiAction::Screenshot { .. } => "screenshot",
        }
    }
}

// ----------------------------------------------------------------------
// ActionCondition — 条件执行的前置条件
// ----------------------------------------------------------------------

/// 条件执行的前置条件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCondition {
    /// 元素存在。
    ElementExists(ElementSelector),
    /// 元素可见(bounds 非 0)。
    ElementVisible(ElementSelector),
    /// 元素启用(底层 trait 无 enabled 查询,降级为 ElementExists)。
    ElementEnabled(ElementSelector),
    /// 延时后恒为真(用于在条件求值前等待)。
    Delay(u64),
    /// 恒真。
    Always,
}

// ----------------------------------------------------------------------
// ActionResult — 动作执行结果
// ----------------------------------------------------------------------

/// 动作执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    /// 是否成功。
    pub success: bool,
    /// 动作类型名(对应 `UiAction::type_name`)。
    pub action_type: String,
    /// 执行耗时(毫秒)。
    pub duration_ms: u64,
    /// 失败时的错误信息。
    pub error: Option<String>,
    /// 执行前截图(base64;仅显式请求时填充)。
    pub screenshot_before: Option<String>,
    /// 执行后截图(base64;`Screenshot` 动作填充捕获结果)。
    pub screenshot_after: Option<String>,
    /// 额外元数据(如 `retries_used` / `attempts` / `condition`)。
    pub metadata: HashMap<String, String>,
}

impl ActionResult {
    /// 构造一个成功结果(便于测试)。
    pub fn success(action_type: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            success: true,
            action_type: action_type.into(),
            duration_ms,
            error: None,
            screenshot_before: None,
            screenshot_after: None,
            metadata: HashMap::new(),
        }
    }

    /// 构造一个失败结果(便于测试)。
    pub fn failure(
        action_type: impl Into<String>,
        duration_ms: u64,
        error: impl Into<String>,
    ) -> Self {
        Self {
            success: false,
            action_type: action_type.into(),
            duration_ms,
            error: Some(error.into()),
            screenshot_before: None,
            screenshot_after: None,
            metadata: HashMap::new(),
        }
    }
}

// ----------------------------------------------------------------------
// ExecutionPlan — 可序列化的执行计划
// ----------------------------------------------------------------------

/// 执行计划 — 一组动作 + 元信息,支持 YAML 往返。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// 动作序列(按序执行)。
    pub actions: Vec<UiAction>,
    /// 预估总耗时(毫秒)。
    pub estimated_duration_ms: u64,
    /// 是否需要逐动作截图(执行器当前仅在 `Screenshot` 动作填充截图)。
    pub requires_screenshot: bool,
}

impl ExecutionPlan {
    /// 从 YAML 字符串解析执行计划。
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let plan: Self = serde_yaml::from_str(yaml)?;
        Ok(plan)
    }

    /// 序列化为 YAML 字符串。
    pub fn to_yaml(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }
}

// ----------------------------------------------------------------------
// ActionExecutor — 统一执行器
// ----------------------------------------------------------------------

/// 统一 UI 动作执行器 — 将 [`UiAction`] 翻译为底层 [`UiAutomator`] 操作。
///
/// 持有一个 [`UiAutomatorTrait`] trait object,无状态(线程安全)。
pub struct ActionExecutor {
    /// 底层 UiAutomator 后端。
    uiautomator: Arc<dyn UiAutomatorTrait>,
}

impl ActionExecutor {
    /// 创建新的执行器,注入底层 UiAutomator 后端。
    pub fn new(uiautomator: Arc<dyn UiAutomatorTrait>) -> Self {
        Self { uiautomator }
    }

    /// 执行单个动作。
    ///
    /// 返回 `Err` 仅表示"动作无法被尝试"(校验失败 / 内部错误);
    /// 后端执行失败以 `Ok(ActionResult { success: false, error: Some(..) })` 表达,
    /// 以便上层(如 [`execute_with_retry`](Self::execute_with_retry))区分。
    pub async fn execute(&self, action: UiAction) -> Result<ActionResult> {
        // 先校验动作合法性(校验失败直接 Err)。
        self.validate_action(&action)?;

        let started = Instant::now();
        let action_type = action.type_name().to_string();
        let mut metadata: HashMap<String, String> = HashMap::new();

        // 派发到底层;成功 / 失败统一记入 (success, error)。
        let dispatch_outcome = self.dispatch(&action, &mut metadata).await;
        let (success, error) = match dispatch_outcome {
            Ok(()) => (true, None),
            Err(e) => (false, Some(format!("{e:#}"))),
        };

        // Screenshot 动作:dispatch 已将 base64 写入 metadata,此处提升至 screenshot_after。
        let screenshot_after = if matches!(action, UiAction::Screenshot { .. }) && success {
            metadata.remove("screenshot_base64")
        } else {
            None
        };

        let duration_ms = started.elapsed().as_millis() as u64;

        Ok(ActionResult {
            success,
            action_type,
            duration_ms,
            error,
            screenshot_before: None,
            screenshot_after,
            metadata,
        })
    }

    /// 执行动作序列(不短路:即使某动作失败仍继续后续动作,各结果独立记录)。
    pub async fn execute_sequence(&self, actions: Vec<UiAction>) -> Result<Vec<ActionResult>> {
        let mut results = Vec::with_capacity(actions.len());
        for action in actions {
            let result = self.execute(action).await?;
            results.push(result);
        }
        Ok(results)
    }

    /// 带重试执行 — 最多尝试 `max_retries + 1` 次;首次成功即返回。
    ///
    /// 成功:返回 `Ok(ActionResult { success: true, .. })`,metadata 记 `retries_used` /
    /// `attempts`。全部失败:返回最后一次 `Ok(ActionResult { success: false, .. })`;
    /// 若中间出现校验 `Err`(理论上首次校验后不应再 Err),则返回该 `Err`。
    pub async fn execute_with_retry(
        &self,
        action: UiAction,
        max_retries: u32,
    ) -> Result<ActionResult> {
        let mut last_result: Option<ActionResult> = None;
        let mut last_err: Option<anyhow::Error> = None;
        let mut attempts: u32 = 0;

        for attempt in 0..=max_retries {
            attempts = attempt + 1;
            match self.execute(action.clone()).await {
                Ok(result) if result.success => {
                    let mut r = result;
                    r.metadata
                        .insert("retries_used".to_string(), attempt.to_string());
                    r.metadata
                        .insert("attempts".to_string(), attempts.to_string());
                    return Ok(r);
                }
                Ok(result) => {
                    last_result = Some(result);
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
            // 退避(最后一次不等待);真实实现可改指数退避。
            if attempt < max_retries {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }

        // 重试耗尽:优先返回最后一次的失败结果(保留 ActionResult 结构)。
        if let Some(mut result) = last_result {
            result
                .metadata
                .insert("retries_used".to_string(), max_retries.to_string());
            result
                .metadata
                .insert("attempts".to_string(), attempts.to_string());
            return Ok(result);
        }
        if let Some(e) = last_err {
            return Err(e);
        }
        // max_retries + 1 >= 1,理论上不可达。
        anyhow::bail!("execute_with_retry exhausted with no result (max_retries={max_retries})")
    }

    /// 条件执行 — 先求值 `condition`,满足则执行 `action`,否则返回"已跳过"的成功结果。
    ///
    /// 跳过语义:返回 `Ok(ActionResult { success: true, metadata["condition"]="skipped" })`,
    /// 即"条件不满足"不算失败,便于序列化中跳过步骤不中断整体流程。
    pub async fn execute_conditional(
        &self,
        action: UiAction,
        condition: ActionCondition,
    ) -> Result<ActionResult> {
        let met = self.evaluate_condition(&condition).await;
        if !met {
            let mut metadata = HashMap::new();
            metadata.insert("condition".to_string(), "skipped".to_string());
            return Ok(ActionResult {
                success: true,
                action_type: action.type_name().to_string(),
                duration_ms: 0,
                error: None,
                screenshot_before: None,
                screenshot_after: None,
                metadata,
            });
        }
        let mut result = self.execute(action).await?;
        result
            .metadata
            .insert("condition".to_string(), "met".to_string());
        Ok(result)
    }

    /// 校验动作合法性(结构 / 取值范围;不涉及后端可达性)。
    pub fn validate_action(&self, action: &UiAction) -> Result<()> {
        match action {
            UiAction::Click { target }
            | UiAction::DoubleClick { target }
            | UiAction::RightClick { target }
            | UiAction::Hover { target }
            | UiAction::Focus { target } => self.validate_target(target),
            UiAction::Type { target, text, .. } => {
                self.validate_target(target)?;
                // text 允许空串(可能仅用于 clear_first)。
                let _ = text;
                Ok(())
            }
            UiAction::KeyPress { keys, modifiers } => {
                if keys.is_empty() {
                    anyhow::bail!("key_press: keys must not be empty");
                }
                // modifiers 可为空(无修饰键)。
                let _ = modifiers;
                Ok(())
            }
            UiAction::Scroll {
                target,
                direction,
                amount,
            } => {
                if *amount == 0 {
                    anyhow::bail!("scroll: amount must be > 0");
                }
                let _ = direction;
                if let Some(t) = target {
                    self.validate_target(t)?;
                }
                Ok(())
            }
            UiAction::Drag {
                from,
                to,
                duration_ms,
            } => {
                self.validate_target(from)?;
                self.validate_target(to)?;
                if *duration_ms == 0 {
                    anyhow::bail!("drag: duration_ms must be > 0");
                }
                Ok(())
            }
            UiAction::Wait { duration_ms } => {
                if *duration_ms == 0 {
                    anyhow::bail!("wait: duration_ms must be > 0");
                }
                Ok(())
            }
            UiAction::Screenshot { region } => {
                if let Some(b) = region {
                    if b.width <= 0 || b.height <= 0 {
                        anyhow::bail!(
                            "screenshot: region must have positive width/height (got {}x{})",
                            b.width,
                            b.height
                        );
                    }
                }
                Ok(())
            }
        }
    }

    // ------------------------------------------------------------------
    // 内部辅助
    // ------------------------------------------------------------------

    /// 校验单个动作目标。
    fn validate_target(&self, target: &ActionTarget) -> Result<()> {
        match target {
            ActionTarget::Coordinates { x, y } => {
                if *x < 0 || *y < 0 {
                    anyhow::bail!("coordinates must be non-negative (got x={}, y={})", x, y);
                }
                Ok(())
            }
            ActionTarget::Element { selector } => {
                if !selector.has_criterion() {
                    anyhow::bail!("element selector must set at least one criterion (by_id/by_text/by_partial_text/by_class/by_role)");
                }
                Ok(())
            }
            ActionTarget::Image { path, confidence } => {
                if path.is_empty() {
                    anyhow::bail!("image target: path must not be empty");
                }
                if *confidence < 0.0 || *confidence > 1.0 {
                    anyhow::bail!(
                        "image target: confidence must be in [0.0, 1.0] (got {})",
                        confidence
                    );
                }
                Ok(())
            }
        }
    }

    /// 将动作派发到底层 UiAutomator(成功返回 Ok(()),失败返回 Err)。
    async fn dispatch(
        &self,
        action: &UiAction,
        metadata: &mut HashMap<String, String>,
    ) -> Result<()> {
        match action {
            UiAction::Click { target } => {
                let el = self.resolve_target(target)?;
                self.uiautomator.click(&el)
            }
            UiAction::DoubleClick { target } => {
                let el = self.resolve_target(target)?;
                // 底层 trait 无 double-click,降级为两次单击(best-effort)。
                self.uiautomator.click(&el)?;
                self.uiautomator.click(&el)
            }
            UiAction::RightClick { .. } => {
                anyhow::bail!("right_click not supported by current UiAutomator backend")
            }
            UiAction::Type {
                target,
                text,
                clear_first,
            } => {
                let el = self.resolve_target(target)?;
                if *clear_first {
                    // 底层 trait 无清空接口;best-effort 跳过(待键盘注入填充)。
                    tracing::debug!(
                        "type clear_first requested but backend has no clear API; best-effort skip"
                    );
                }
                self.uiautomator.type_text(&el, text)
            }
            UiAction::KeyPress { keys, modifiers } => {
                anyhow::bail!(
                    "key_press not supported by current UiAutomator backend; keys={:?}, modifiers={:?}",
                    keys,
                    modifiers
                )
            }
            UiAction::Scroll {
                target: _,
                direction,
                amount,
            } => {
                // 底层 scroll 为窗口级,忽略 target(best-effort)。
                let backend_dir = convert_direction(*direction);
                self.uiautomator.scroll(backend_dir, *amount)
            }
            UiAction::Drag { .. } => {
                anyhow::bail!(
                    "drag not supported by current UiAutomator backend (requires mouse-down/move/up)"
                )
            }
            UiAction::Hover { .. } => {
                anyhow::bail!("hover not supported by current UiAutomator backend")
            }
            UiAction::Focus { target } => {
                let el = self.resolve_target(target)?;
                // 底层 trait 无 focus 接口;用 click 模拟聚焦(best-effort)。
                self.uiautomator.click(&el)
            }
            UiAction::Wait { duration_ms } => {
                tokio::time::sleep(Duration::from_millis(*duration_ms)).await;
                Ok(())
            }
            UiAction::Screenshot { region } => {
                // 区域裁剪需 vision 后端;当前忽略 region(best-effort,截全屏)。
                let _ = region;
                let bytes = self.uiautomator.screenshot()?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                metadata.insert("screenshot_bytes".to_string(), bytes.len().to_string());
                metadata.insert("screenshot_base64".to_string(), b64);
                Ok(())
            }
        }
    }

    /// 将 [`ActionTarget`] 解析为底层 [`ua::UiElement`]。
    fn resolve_target(&self, target: &ActionTarget) -> Result<ua::UiElement> {
        match target {
            ActionTarget::Coordinates { x, y } => {
                // 合成一个位于 (x, y) 的 1x1 元素,中心点即 (x, y)。
                Ok(ua::UiElement::empty().with_bounds(ua::ElementBounds::new(*x, *y, 1, 1)))
            }
            ActionTarget::Element { selector } => {
                let backend_selector = convert_selector(selector)?;
                self.uiautomator.find_element(&backend_selector)
            }
            ActionTarget::Image { path, confidence } => {
                anyhow::bail!(
                    "image-based target not supported by current UiAutomator backend; \
                     path={}, confidence={:.2}; requires vision integration",
                    path,
                    confidence
                )
            }
        }
    }

    /// 求值条件(异步:可能包含 Delay)。
    async fn evaluate_condition(&self, condition: &ActionCondition) -> bool {
        match condition {
            ActionCondition::ElementExists(sel) => {
                let backend = match convert_selector(sel) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
                self.uiautomator.find_element(&backend).is_ok()
            }
            ActionCondition::ElementVisible(sel) => {
                let backend = match convert_selector(sel) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
                match self.uiautomator.find_element(&backend) {
                    Ok(el) => el.bounds.width > 0 && el.bounds.height > 0,
                    Err(_) => false,
                }
            }
            ActionCondition::ElementEnabled(sel) => {
                // 底层 trait 无 enabled 查询;降级为 ElementExists(best-effort)。
                let backend = match convert_selector(sel) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
                self.uiautomator.find_element(&backend).is_ok()
            }
            ActionCondition::Delay(ms) => {
                tokio::time::sleep(Duration::from_millis(*ms)).await;
                true
            }
            ActionCondition::Always => true,
        }
    }
}

// ----------------------------------------------------------------------
// 私有转换辅助
// ----------------------------------------------------------------------

/// 将本模块 [`ElementSelector`] 翻译为底层 [`ua::ElementSelector`](单策略)。
///
/// 优先级:`by_id` > `by_text` > `by_partial_text` > `by_class` > `by_role`。
fn convert_selector(sel: &ElementSelector) -> Result<ua::ElementSelector> {
    if let Some(id) = &sel.by_id {
        return Ok(ua::ElementSelector::ById(id.clone()));
    }
    if let Some(text) = &sel.by_text {
        return Ok(ua::ElementSelector::ByText(text.clone()));
    }
    if let Some(ptext) = &sel.by_partial_text {
        // 底层无 partial_text,降级为 ByText(best-effort)。
        return Ok(ua::ElementSelector::ByText(ptext.clone()));
    }
    if let Some(class) = &sel.by_class {
        return Ok(ua::ElementSelector::ByClass(class.clone()));
    }
    if let Some(role) = &sel.by_role {
        return Ok(ua::ElementSelector::ByRole(role.clone()));
    }
    anyhow::bail!("ElementSelector has no criterion set")
}

/// 将本模块 [`ScrollDirection`] 翻译为底层 [`ua::ScrollDirection`]。
fn convert_direction(d: ScrollDirection) -> ua::ScrollDirection {
    match d {
        ScrollDirection::Up => ua::ScrollDirection::Up,
        ScrollDirection::Down => ua::ScrollDirection::Down,
        ScrollDirection::Left => ua::ScrollDirection::Left,
        ScrollDirection::Right => ua::ScrollDirection::Right,
    }
}

// ----------------------------------------------------------------------
// base64 Engine 引入(Screenshot 动作用到)
// ----------------------------------------------------------------------

/// base64 引用(显式 use 以启用 `Engine::encode` 方法)。
#[allow(unused_imports)]
use base64::Engine as _;

// ----------------------------------------------------------------------
// 单元测试
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // ---- UiAction 各变体序列化往返 ----

    #[test]
    fn ui_action_click_serde_roundtrip() {
        let a = UiAction::Click {
            target: ActionTarget::Coordinates { x: 10, y: 20 },
        };
        let json = serde_json::to_string(&a).expect("serialize");
        // snake_case 变体名。
        assert!(json.contains("\"click\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_double_click_serde_roundtrip() {
        let a = UiAction::DoubleClick {
            target: ActionTarget::Coordinates { x: 1, y: 2 },
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"double_click\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_right_click_serde_roundtrip() {
        let a = UiAction::RightClick {
            target: ActionTarget::Coordinates { x: 5, y: 5 },
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"right_click\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_type_serde_roundtrip() {
        let a = UiAction::Type {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("user".into()),
                    ..Default::default()
                },
            },
            text: "hello".into(),
            clear_first: true,
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"type\""));
        assert!(json.contains("\"clear_first\":true"));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_key_press_serde_roundtrip() {
        let a = UiAction::KeyPress {
            keys: vec!["Enter".into(), "Tab".into()],
            modifiers: vec![KeyModifier::Ctrl, KeyModifier::Shift],
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"key_press\""));
        assert!(json.contains("\"ctrl\""));
        assert!(json.contains("\"shift\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_scroll_serde_roundtrip() {
        let a = UiAction::Scroll {
            target: None,
            direction: ScrollDirection::Down,
            amount: 5,
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"scroll\""));
        assert!(json.contains("\"down\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_drag_serde_roundtrip() {
        let a = UiAction::Drag {
            from: ActionTarget::Coordinates { x: 0, y: 0 },
            to: ActionTarget::Coordinates { x: 100, y: 100 },
            duration_ms: 500,
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"drag\""));
        assert!(json.contains("\"duration_ms\":500"));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_hover_serde_roundtrip() {
        let a = UiAction::Hover {
            target: ActionTarget::Coordinates { x: 3, y: 4 },
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"hover\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_focus_serde_roundtrip() {
        let a = UiAction::Focus {
            target: ActionTarget::Coordinates { x: 7, y: 8 },
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"focus\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_wait_serde_roundtrip() {
        let a = UiAction::Wait { duration_ms: 250 };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"wait\""));
        assert!(json.contains("\"duration_ms\":250"));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);
    }

    #[test]
    fn ui_action_screenshot_serde_roundtrip() {
        let a = UiAction::Screenshot {
            region: Some(BoundingBox::new(10, 20, 300, 200)),
        };
        let json = serde_json::to_string(&a).expect("serialize");
        assert!(json.contains("\"screenshot\""));
        let back: UiAction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(a, back);

        // region = None 也可往返。
        let a2 = UiAction::Screenshot { region: None };
        let json2 = serde_json::to_string(&a2).expect("serialize");
        let back2: UiAction = serde_json::from_str(&json2).expect("deserialize");
        assert_eq!(a2, back2);
    }

    // ---- ActionTarget 坐标 / 元素 / 图像模式 ----

    #[test]
    fn action_target_coordinates_serde_roundtrip() {
        let t = ActionTarget::Coordinates { x: 42, y: -1 };
        // 注意:此处序列化往返只验证 serde,不涉及合法性校验。
        let json = serde_json::to_string(&t).expect("serialize");
        assert!(json.contains("\"coordinates\""));
        let back: ActionTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, back);
    }

    #[test]
    fn action_target_element_serde_roundtrip() {
        let t = ActionTarget::Element {
            selector: ElementSelector {
                by_role: Some("Button".into()),
                index: Some(2),
                ..Default::default()
            },
        };
        let json = serde_json::to_string(&t).expect("serialize");
        assert!(json.contains("\"element\""));
        assert!(json.contains("\"by_role\":\"Button\""));
        assert!(json.contains("\"index\":2"));
        let back: ActionTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, back);
    }

    #[test]
    fn action_target_image_serde_roundtrip() {
        let t = ActionTarget::Image {
            path: "assets/btn.png".into(),
            confidence: 0.85,
        };
        let json = serde_json::to_string(&t).expect("serialize");
        assert!(json.contains("\"image\""));
        assert!(json.contains("\"confidence\":0.85"));
        let back: ActionTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, back);
    }

    // ---- ElementSelector 组合 ----

    #[test]
    fn element_selector_combination_serde() {
        let s = ElementSelector {
            by_id: Some("login".into()),
            by_text: Some("Sign in".into()),
            by_partial_text: Some("Sign".into()),
            by_class: Some("Button".into()),
            by_role: Some("Button".into()),
            index: Some(0),
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back: ElementSelector = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, back);
        // 组合时 has_criterion 为 true。
        assert!(s.has_criterion());

        // 空选择器 has_criterion 为 false。
        let empty = ElementSelector::new();
        assert!(!empty.has_criterion());
        let empty_json = serde_json::to_string(&empty).expect("serialize empty");
        let empty_back: ElementSelector = serde_json::from_str(&empty_json).expect("deserialize");
        assert_eq!(empty, empty_back);
    }

    // ---- KeyModifier 组合 ----

    #[test]
    fn key_modifier_combination_serde() {
        let mods = vec![
            KeyModifier::Ctrl,
            KeyModifier::Shift,
            KeyModifier::Alt,
            KeyModifier::Super,
        ];
        let json = serde_json::to_string(&mods).expect("serialize");
        assert!(json.contains("\"ctrl\""));
        assert!(json.contains("\"shift\""));
        assert!(json.contains("\"alt\""));
        assert!(json.contains("\"super\""));
        let back: Vec<KeyModifier> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(mods, back);
    }

    // ---- ActionCondition 各变体 ----

    #[test]
    fn action_condition_variants_serde() {
        let conds = vec![
            ActionCondition::ElementExists(ElementSelector {
                by_id: Some("a".into()),
                ..Default::default()
            }),
            ActionCondition::ElementVisible(ElementSelector {
                by_text: Some("b".into()),
                ..Default::default()
            }),
            ActionCondition::ElementEnabled(ElementSelector {
                by_role: Some("Button".into()),
                ..Default::default()
            }),
            ActionCondition::Delay(100),
            ActionCondition::Always,
        ];
        for c in &conds {
            let json = serde_json::to_string(c).expect("serialize");
            let back: ActionCondition = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(c, &back);
        }
        // 校验 snake_case 变体名。
        let json = serde_json::to_string(&ActionCondition::Always).expect("serialize");
        assert!(json.contains("\"always\""));
        let json = serde_json::to_string(&ActionCondition::ElementExists(ElementSelector::new()))
            .expect("serialize");
        assert!(json.contains("\"element_exists\""));
        let json = serde_json::to_string(&ActionCondition::Delay(50)).expect("serialize");
        assert!(json.contains("\"delay\""));
    }

    // ---- ActionResult 构建 ----

    #[test]
    fn action_result_construction() {
        let ok = ActionResult::success("click", 12);
        assert!(ok.success);
        assert_eq!(ok.action_type, "click");
        assert_eq!(ok.duration_ms, 12);
        assert!(ok.error.is_none());
        assert!(ok.metadata.is_empty());

        let err = ActionResult::failure("type", 5, "element not found");
        assert!(!err.success);
        assert_eq!(err.action_type, "type");
        assert_eq!(err.error.as_deref(), Some("element not found"));

        // 完整结构体序列化往返。
        let mut md = HashMap::new();
        md.insert("retries_used".to_string(), "2".to_string());
        let r = ActionResult {
            success: true,
            action_type: "scroll".into(),
            duration_ms: 33,
            error: None,
            screenshot_before: None,
            screenshot_after: Some("base64data".into()),
            metadata: md,
        };
        let json = serde_json::to_string(&r).expect("serialize");
        let back: ActionResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.action_type, "scroll");
        assert_eq!(back.duration_ms, 33);
        assert_eq!(back.screenshot_after.as_deref(), Some("base64data"));
        assert_eq!(
            back.metadata.get("retries_used").map(|s| s.as_str()),
            Some("2")
        );
    }

    // ---- ExecutionPlan YAML 序列化往返 ----

    #[test]
    fn execution_plan_yaml_roundtrip() {
        let plan = ExecutionPlan {
            actions: vec![
                UiAction::Click {
                    target: ActionTarget::Coordinates { x: 10, y: 10 },
                },
                UiAction::Type {
                    target: ActionTarget::Element {
                        selector: ElementSelector {
                            by_id: Some("q".into()),
                            ..Default::default()
                        },
                    },
                    text: "nebula".into(),
                    clear_first: true,
                },
                UiAction::Wait { duration_ms: 100 },
            ],
            estimated_duration_ms: 1500,
            requires_screenshot: true,
        };
        let yaml = plan.to_yaml().expect("to_yaml");
        assert!(yaml.contains("actions:"));
        assert!(yaml.contains("estimated_duration_ms: 1500"));
        assert!(yaml.contains("requires_screenshot: true"));

        let back = ExecutionPlan::from_yaml(&yaml).expect("from_yaml");
        assert_eq!(back.actions.len(), 3);
        assert_eq!(back.estimated_duration_ms, 1500);
        assert!(back.requires_screenshot);
        // 校验动作顺序与类型。
        assert_eq!(back.actions[0].type_name(), "click");
        assert_eq!(back.actions[1].type_name(), "type");
        assert_eq!(back.actions[2].type_name(), "wait");
        // 校验 type 动作的文本字段往返。
        match &back.actions[1] {
            UiAction::Type {
                text, clear_first, ..
            } => {
                assert_eq!(text, "nebula");
                assert!(*clear_first);
            }
            other => panic!("expected Type, got {:?}", other),
        }
    }

    #[test]
    fn execution_plan_from_invalid_yaml_errors() {
        let res = ExecutionPlan::from_yaml("not: valid: yaml: [");
        assert!(res.is_err());
    }

    // ---- validate_action 各种场景 ----

    fn executor_with_mock(mock: ua::MockUiAutomator) -> ActionExecutor {
        ActionExecutor::new(Arc::new(mock))
    }

    #[test]
    fn validate_action_valid_click_coordinates() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Click {
            target: ActionTarget::Coordinates { x: 0, y: 0 },
        };
        assert!(ex.validate_action(&a).is_ok());
    }

    #[test]
    fn validate_action_valid_click_element() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("btn".into()),
                    ..Default::default()
                },
            },
        };
        assert!(ex.validate_action(&a).is_ok());
    }

    #[test]
    fn validate_action_invalid_negative_coordinates() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Click {
            target: ActionTarget::Coordinates { x: -1, y: 5 },
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(err.contains("non-negative"), "unexpected err: {}", err);
    }

    #[test]
    fn validate_action_invalid_image_empty_path() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Click {
            target: ActionTarget::Image {
                path: String::new(),
                confidence: 0.9,
            },
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("path must not be empty"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn validate_action_invalid_image_confidence() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        // > 1.0 非法。
        let a = UiAction::Click {
            target: ActionTarget::Image {
                path: "x.png".into(),
                confidence: 1.5,
            },
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("confidence must be in"),
            "unexpected err: {}",
            err
        );

        // < 0.0 非法。
        let a2 = UiAction::Click {
            target: ActionTarget::Image {
                path: "x.png".into(),
                confidence: -0.1,
            },
        };
        assert!(ex.validate_action(&a2).is_err());

        // 边界 0.0 与 1.0 合法。
        let a3 = UiAction::Click {
            target: ActionTarget::Image {
                path: "x.png".into(),
                confidence: 0.0,
            },
        };
        assert!(ex.validate_action(&a3).is_ok());
        let a4 = UiAction::Click {
            target: ActionTarget::Image {
                path: "x.png".into(),
                confidence: 1.0,
            },
        };
        assert!(ex.validate_action(&a4).is_ok());
    }

    #[test]
    fn validate_action_invalid_element_no_criterion() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    index: Some(3),
                    ..Default::default()
                },
            },
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("at least one criterion"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn validate_action_invalid_scroll_amount_zero() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Scroll {
            target: None,
            direction: ScrollDirection::Up,
            amount: 0,
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("amount must be > 0"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn validate_action_invalid_drag_duration_zero() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Drag {
            from: ActionTarget::Coordinates { x: 0, y: 0 },
            to: ActionTarget::Coordinates { x: 10, y: 10 },
            duration_ms: 0,
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("duration_ms must be > 0"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn validate_action_invalid_wait_zero() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::Wait { duration_ms: 0 };
        assert!(ex.validate_action(&a).is_err());
    }

    #[test]
    fn validate_action_invalid_screenshot_region() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        // width <= 0 非法。
        let a = UiAction::Screenshot {
            region: Some(BoundingBox::new(0, 0, 0, 100)),
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("positive width/height"),
            "unexpected err: {}",
            err
        );
        // height <= 0 非法。
        let a2 = UiAction::Screenshot {
            region: Some(BoundingBox::new(0, 0, 100, -1)),
        };
        assert!(ex.validate_action(&a2).is_err());
        // region = None 合法。
        let a3 = UiAction::Screenshot { region: None };
        assert!(ex.validate_action(&a3).is_ok());
    }

    #[test]
    fn validate_action_keypress_empty_keys_invalid() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let a = UiAction::KeyPress {
            keys: vec![],
            modifiers: vec![KeyModifier::Ctrl],
        };
        let err = ex.validate_action(&a).unwrap_err().to_string();
        assert!(
            err.contains("keys must not be empty"),
            "unexpected err: {}",
            err
        );
        // 有键合法(modifiers 可空)。
        let a2 = UiAction::KeyPress {
            keys: vec!["c".into()],
            modifiers: vec![],
        };
        assert!(ex.validate_action(&a2).is_ok());
    }

    // ---- execute_sequence 顺序 ----

    #[tokio::test]
    async fn execute_sequence_preserves_order() {
        // 准备两个可被 mock 命中的元素。
        let el1 = ua::UiElement::empty()
            .with_id("ok")
            .with_name("OK")
            .with_role("Button")
            .with_bounds(ua::ElementBounds::new(0, 0, 10, 10));
        let el2 = ua::UiElement::empty()
            .with_id("field")
            .with_name("Field")
            .with_role("Edit")
            .with_text("input")
            .with_bounds(ua::ElementBounds::new(0, 0, 10, 10));
        let mock = ua::MockUiAutomator::new()
            .with_element(el1)
            .with_element(el2);
        let ex = executor_with_mock(mock);

        let actions = vec![
            UiAction::Click {
                target: ActionTarget::Element {
                    selector: ElementSelector {
                        by_role: Some("Button".into()),
                        ..Default::default()
                    },
                },
            },
            UiAction::Type {
                target: ActionTarget::Element {
                    selector: ElementSelector {
                        by_text: Some("input".into()),
                        ..Default::default()
                    },
                },
                text: "hi".into(),
                clear_first: false,
            },
            UiAction::Scroll {
                target: None,
                direction: ScrollDirection::Down,
                amount: 3,
            },
        ];

        let results = ex.execute_sequence(actions).await.expect("sequence");
        assert_eq!(results.len(), 3);
        // 全部成功。
        assert!(results.iter().all(|r| r.success), "all should succeed");
        // 动作类型按序。
        assert_eq!(results[0].action_type, "click");
        assert_eq!(results[1].action_type, "type");
        assert_eq!(results[2].action_type, "scroll");
    }

    #[tokio::test]
    async fn execute_sequence_does_not_short_circuit() {
        // 空 mock:Click 元素会失败,但序列仍应继续执行后续 Wait。
        let mock = ua::MockUiAutomator::new();
        let ex = executor_with_mock(mock);

        let actions = vec![
            UiAction::Click {
                target: ActionTarget::Element {
                    selector: ElementSelector {
                        by_id: Some("missing".into()),
                        ..Default::default()
                    },
                },
            },
            UiAction::Wait { duration_ms: 5 },
        ];
        let results = ex.execute_sequence(actions).await.expect("sequence");
        assert_eq!(results.len(), 2);
        // 第一个失败,第二个成功(未短路)。
        assert!(!results[0].success);
        assert!(results[1].success);
    }

    // ---- retry 逻辑验证 ----

    #[tokio::test]
    async fn execute_with_retry_succeeds_first_try() {
        // mock 含目标元素 → Click 首次即成功。
        let el = ua::UiElement::empty()
            .with_id("btn")
            .with_name("Btn")
            .with_bounds(ua::ElementBounds::new(0, 0, 5, 5));
        let mock = ua::MockUiAutomator::new().with_element(el);
        let ex = executor_with_mock(mock);

        let action = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("btn".into()),
                    ..Default::default()
                },
            },
        };
        let result = ex.execute_with_retry(action, 3).await.expect("retry");
        assert!(result.success);
        // 首次成功 → attempts == 1,retries_used == 0。
        assert_eq!(
            result.metadata.get("attempts").map(|s| s.as_str()),
            Some("1")
        );
        assert_eq!(
            result.metadata.get("retries_used").map(|s| s.as_str()),
            Some("0")
        );
    }

    #[tokio::test]
    async fn execute_with_retry_always_fails() {
        // 空 mock → find_element 永远 Err → 每次执行失败。
        let mock = ua::MockUiAutomator::new();
        let ex = executor_with_mock(mock);

        let action = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("nope".into()),
                    ..Default::default()
                },
            },
        };
        // max_retries=2 → 共 3 次尝试。
        let result = ex.execute_with_retry(action, 2).await.expect("retry");
        assert!(!result.success);
        assert!(result.error.is_some());
        assert_eq!(
            result.metadata.get("attempts").map(|s| s.as_str()),
            Some("3")
        );
        assert_eq!(
            result.metadata.get("retries_used").map(|s| s.as_str()),
            Some("2")
        );
    }

    /// Flaky mock:前 `fail_first` 次 `find_element` 失败,之后返回预设元素。
    struct FlakyMockUiAutomator {
        remaining_failures: AtomicU32,
        element: ua::UiElement,
    }

    impl FlakyMockUiAutomator {
        fn new(fail_first: u32, element: ua::UiElement) -> Self {
            Self {
                remaining_failures: AtomicU32::new(fail_first),
                element,
            }
        }
    }

    impl ua::UiAutomator for FlakyMockUiAutomator {
        fn find_element(&self, _sel: &ua::ElementSelector) -> Result<ua::UiElement> {
            let cur = self.remaining_failures.load(Ordering::SeqCst);
            if cur > 0 {
                self.remaining_failures.store(cur - 1, Ordering::SeqCst);
                anyhow::bail!("flaky find_element failure (was {})", cur);
            }
            Ok(self.element.clone())
        }
        fn click(&self, _el: &ua::UiElement) -> Result<()> {
            Ok(())
        }
        fn type_text(&self, _el: &ua::UiElement, _t: &str) -> Result<()> {
            Ok(())
        }
        fn scroll(&self, _d: ua::ScrollDirection, _a: u32) -> Result<()> {
            Ok(())
        }
        fn screenshot(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn get_active_window(&self) -> Result<ua::WindowInfo> {
            anyhow::bail!("no active window in flaky mock")
        }
        fn list_windows(&self) -> Result<Vec<ua::WindowInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn execute_with_retry_flaky_eventually_succeeds() {
        // 前 2 次失败,第 3 次成功。
        let el = ua::UiElement::empty()
            .with_id("btn")
            .with_bounds(ua::ElementBounds::new(0, 0, 5, 5));
        let flaky = FlakyMockUiAutomator::new(2, el);
        let ex = ActionExecutor::new(Arc::new(flaky));

        let action = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("btn".into()),
                    ..Default::default()
                },
            },
        };
        let result = ex.execute_with_retry(action, 5).await.expect("retry");
        assert!(result.success);
        // 第 3 次尝试成功 → attempts == 3,retries_used == 2。
        assert_eq!(
            result.metadata.get("attempts").map(|s| s.as_str()),
            Some("3")
        );
        assert_eq!(
            result.metadata.get("retries_used").map(|s| s.as_str()),
            Some("2")
        );
    }

    // ---- execute Wait / Screenshot ----

    #[tokio::test]
    async fn execute_wait_action_timing() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let result = ex
            .execute(UiAction::Wait { duration_ms: 60 })
            .await
            .expect("execute");
        assert!(result.success);
        assert_eq!(result.action_type, "wait");
        // tokio::time::sleep 保证至少等待指定时长,elapsed >= 60ms(留少许余量)。
        assert!(
            result.duration_ms >= 50,
            "expected duration >= 50ms, got {}",
            result.duration_ms
        );
    }

    #[tokio::test]
    async fn execute_screenshot_populates_screenshot_after() {
        let preset = vec![0x89u8, 0x50, 0x4E, 0x47]; // PNG 头。
        let mock = ua::MockUiAutomator::new().with_screenshot_bytes(preset.clone());
        let ex = executor_with_mock(mock);

        let result = ex
            .execute(UiAction::Screenshot { region: None })
            .await
            .expect("execute");
        assert!(result.success);
        assert_eq!(result.action_type, "screenshot");
        // screenshot_after 应为预设字节的 base64。
        let after = result
            .screenshot_after
            .as_deref()
            .expect("screenshot_after should be set");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(after)
            .expect("base64 decode");
        assert_eq!(decoded, preset);
        // metadata 记录字节数。
        assert_eq!(
            result.metadata.get("screenshot_bytes").map(|s| s.as_str()),
            Some("4")
        );
    }

    #[tokio::test]
    async fn execute_unsupported_action_returns_failed_result() {
        // RightClick 后端不支持 → execute 返回 Ok(failed result),非 Err。
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let result = ex
            .execute(UiAction::RightClick {
                target: ActionTarget::Coordinates { x: 1, y: 1 },
            })
            .await
            .expect("execute should not Err on unsupported action");
        assert!(!result.success);
        assert_eq!(result.action_type, "right_click");
        let err = result.error.expect("error should be set");
        assert!(err.contains("not supported"), "unexpected err: {}", err);
    }

    // ---- execute_conditional ----

    #[tokio::test]
    async fn execute_conditional_skipped_when_element_missing() {
        // mock 含 "real" 元素;条件查询 "missing"(不存在)→ 跳过动作。
        let el = ua::UiElement::empty()
            .with_id("real")
            .with_bounds(ua::ElementBounds::new(0, 0, 5, 5));
        let mock = ua::MockUiAutomator::new().with_element(el);
        let ex = executor_with_mock(mock);

        let action = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("real".into()),
                    ..Default::default()
                },
            },
        };
        let condition = ActionCondition::ElementExists(ElementSelector {
            by_id: Some("missing".into()),
            ..Default::default()
        });
        let result = ex
            .execute_conditional(action, condition)
            .await
            .expect("cond");
        // 跳过 → 视为成功(success=true),metadata 标记 skipped。
        assert!(result.success);
        assert_eq!(
            result.metadata.get("condition").map(|s| s.as_str()),
            Some("skipped")
        );
    }

    #[tokio::test]
    async fn execute_conditional_met_when_element_exists() {
        let el = ua::UiElement::empty()
            .with_id("real")
            .with_bounds(ua::ElementBounds::new(0, 0, 5, 5));
        let mock = ua::MockUiAutomator::new().with_element(el);
        let ex = executor_with_mock(mock);

        let action = UiAction::Click {
            target: ActionTarget::Element {
                selector: ElementSelector {
                    by_id: Some("real".into()),
                    ..Default::default()
                },
            },
        };
        let condition = ActionCondition::ElementExists(ElementSelector {
            by_id: Some("real".into()),
            ..Default::default()
        });
        let result = ex
            .execute_conditional(action, condition)
            .await
            .expect("cond");
        // 条件满足 → 执行动作 → 成功。
        assert!(result.success);
        assert_eq!(
            result.metadata.get("condition").map(|s| s.as_str()),
            Some("met")
        );
        assert_eq!(result.action_type, "click");
    }

    #[tokio::test]
    async fn execute_conditional_always_executes_action() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let result = ex
            .execute_conditional(UiAction::Wait { duration_ms: 5 }, ActionCondition::Always)
            .await
            .expect("cond");
        assert!(result.success);
        assert_eq!(
            result.metadata.get("condition").map(|s| s.as_str()),
            Some("met")
        );
    }

    #[tokio::test]
    async fn execute_conditional_delay_then_always_true() {
        let ex = executor_with_mock(ua::MockUiAutomator::new());
        let started = Instant::now();
        let result = ex
            .execute_conditional(
                UiAction::Wait { duration_ms: 5 },
                ActionCondition::Delay(40),
            )
            .await
            .expect("cond");
        // Delay 条件先等 40ms,再执行 Wait 5ms → 总计 >= 45ms。
        assert!(result.success);
        assert!(started.elapsed().as_millis() >= 40);
    }
}
