//! T-E-C-03: UiAutomator 抽象层 — trait-based UI 自动化抽象。
//!
//! 提供跨平台 UI 自动化的核心抽象接口,支持点击、输入、滚动、查找元素、
//! 截图等操作。设计为 trait-based,以便不同平台(Windows / macOS / Linux)
//! 各自提供具体实现,测试代码使用 [`MockUiAutomator`] 注入。
//!
//! ## 架构
//!
//! * [`UiAutomator`] trait — 平台无关的核心抽象接口。
//! * [`WindowsUiAutomator`] — Windows 平台实现骨架,通过 `windows-sys` crate
//!   的 UIAutomation COM API(`Win32_UI_Accessibility` feature)操作元素。
//!   当前为骨架占位,所有方法返回 `NotImplemented` 错误,后续任务填充真实 API。
//! * [`MockUiAutomator`] — 用于单元测试的 mock 实现,记录所有操作以便断言。
//!
//! ## 平台支持
//!
//! * Windows — 骨架占位,返回 `Err`(`windows-sys` UIAutomation 集成待填充)。
//! * macOS / Linux — 暂无实现,可通过 trait 自行扩展。
//!
//! ## 后续 TODO
//!
//! * Windows: 接入 `IUIAutomation` / `IUIAutomationElement` COM 接口
//!   (需在 Cargo.toml 的 `windows-sys` features 中添加 `Win32_UI_Accessibility`)。
//! * macOS: 接入 AppKit `AXUIElement` Accessibility API。
//! * Linux: 接入 AT-SPI2 / `atspi` crate。
//!
//! ## 注册
//!
//! 本模块当前未在 `os/mod.rs` 中注册(主控统一处理)。注册时添加:
//! ```ignore
//! // in src-tauri/src/os/mod.rs
//! pub mod uiautomator;
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------
// 元素定位策略
// ----------------------------------------------------------------------

/// 元素定位策略枚举 — 描述如何在 UI 树中查找元素。
///
/// 跨平台抽象,各平台实现负责将策略映射到原生 API:
/// * Windows UIAutomation — `FindByProperty` / `TreeWalker`。
/// * macOS AXUIElement — `AXUIElementCopyAttributeValue`。
/// * Linux AT-SPI2 — `Accessible_query`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ElementSelector {
    /// 按 AutomationId / accessibility identifier 定位。
    ById(String),
    /// 按元素名称(Name / Title)定位。
    ByName(String),
    /// 按 XPath 表达式定位(UIAutomation 不原生支持,需自行遍历)。
    ByXPath(String),
    /// 按可见文本内容定位。
    ByText(String),
    /// 按控件类名(ClassName)定位。
    ByClass(String),
    /// 按控件角色(ControlType / Role)定位,如 "Button" / "Edit"。
    ByRole(String),
}

// ----------------------------------------------------------------------
// 滚动方向
// ----------------------------------------------------------------------

/// 滚动方向。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

// ----------------------------------------------------------------------
// 元素边界
// ----------------------------------------------------------------------

/// 元素边界矩形(屏幕坐标,像素)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ElementBounds {
    /// 左上角 X 坐标。
    pub x: i32,
    /// 左上角 Y 坐标。
    pub y: i32,
    /// 宽度。
    pub width: i32,
    /// 高度。
    pub height: i32,
}

impl ElementBounds {
    /// 创建新的边界矩形。
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 中心点坐标。
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

// ----------------------------------------------------------------------
// UI 元素
// ----------------------------------------------------------------------

/// UI 元素信息 — 平台无关结构体。
///
/// `children` 字段允许表达 UI 树形结构,便于遍历查找。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiElement {
    /// 元素 AutomationId(可选)。
    pub id: Option<String>,
    /// 元素名称 / 标题(可选)。
    pub name: Option<String>,
    /// 控件角色 / ControlType(可选),如 "Button" / "Edit" / "Text"。
    pub role: Option<String>,
    /// 元素边界矩形(屏幕坐标)。
    pub bounds: ElementBounds,
    /// 元素可见文本(可选)。
    pub text: Option<String>,
    /// 子元素列表。
    pub children: Vec<UiElement>,
}

impl UiElement {
    /// 创建一个空元素(所有字段为默认值)。
    pub fn empty() -> Self {
        Self {
            id: None,
            name: None,
            role: None,
            bounds: ElementBounds::default(),
            text: None,
            children: Vec::new(),
        }
    }

    /// 链式设置 id。
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// 链式设置 name。
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 链式设置 role。
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    /// 链式设置 bounds。
    pub fn with_bounds(mut self, bounds: ElementBounds) -> Self {
        self.bounds = bounds;
        self
    }

    /// 链式设置 text。
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// 链式添加子元素。
    pub fn with_child(mut self, child: UiElement) -> Self {
        self.children.push(child);
        self
    }
}

// ----------------------------------------------------------------------
// 窗口信息
// ----------------------------------------------------------------------

/// 窗口信息 — 平台无关结构体。
///
/// 注意:与 [`crate::os::controller::WindowInfo`] 不同,本结构体面向
/// UiAutomator 抽象层,包含 `bounds` 与 `process_name` 字段供自动化决策使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    /// 窗口标题。
    pub title: String,
    /// 窗口句柄(平台原生句柄,u64 以便 JSON 序列化)。
    pub handle: u64,
    /// 窗口边界矩形(屏幕坐标)。
    pub bounds: ElementBounds,
    /// 进程名(如 "chrome.exe" / "Code.exe")。
    pub process_name: String,
}

// ----------------------------------------------------------------------
// UiAutomator 核心 trait
// ----------------------------------------------------------------------

/// UiAutomator 核心 trait — 跨平台 UI 自动化抽象接口。
///
/// 所有方法同步返回。实现方需保证 `Send + Sync` 以便在多线程上下文中使用
/// (如 Tauri 异步命令中通过 `tokio::task::spawn_blocking` 调用)。
pub trait UiAutomator: Send + Sync {
    /// 按定位策略查找单个 UI 元素。
    ///
    /// 若匹配到多个元素,返回第一个。若无匹配,返回 `Err`。
    fn find_element(&self, selector: &ElementSelector) -> Result<UiElement>;

    /// 点击指定元素(通常点击其中心点)。
    fn click(&self, element: &UiElement) -> Result<()>;

    /// 向指定元素输入文本(模拟键盘输入)。
    fn type_text(&self, element: &UiElement, text: &str) -> Result<()>;

    /// 向当前焦点方向滚动指定量。
    ///
    /// `amount` 语义为滚动"步数"或"行数",具体映射由实现决定。
    fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<()>;

    /// 截取当前屏幕,返回 PNG 编码的字节流。
    fn screenshot(&self) -> Result<Vec<u8>>;

    /// 获取当前活动(前台)窗口信息。
    fn get_active_window(&self) -> Result<WindowInfo>;

    /// 列出所有可见窗口。
    fn list_windows(&self) -> Result<Vec<WindowInfo>>;
}

// ----------------------------------------------------------------------
// Windows 平台实现骨架
// ----------------------------------------------------------------------

/// Windows 平台 UiAutomator 实现 — 骨架占位。
///
/// 计划通过 `windows-sys` crate 的 UIAutomation COM API 实现:
/// * `CoInitializeEx` 初始化 COM。
/// * `CoCreateInstance(CLSID_CUIAutomation)` 获取 `IUIAutomation` 实例。
/// * `IUIAutomation::ElementFromPoint` / `GetRootElement` / `FindFirst`。
/// * `IUIAutomationElement::GetCurrentPropertyValue` 读取属性。
/// * `IUIAutomationInvokePattern::Invoke` 触发点击。
/// * `IUIAutomationValuePattern::SetValue` 输入文本。
/// * `IUIAutomationScrollPattern::Scroll` 滚动。
///
/// 当前所有方法返回 `NotImplemented` 错误,后续任务填充真实 API 调用。
///
/// ## 依赖
///
/// 需在 `Cargo.toml` 的 `[target.'cfg(target_os = "windows")'.dependencies]`
/// 中为 `windows-sys` 添加 `Win32_UI_Accessibility` feature。
#[cfg(windows)]
pub struct WindowsUiAutomator {
    // 占位字段:后续将持有 IUIAutomation COM 指针。
    _placeholder: (),
}

#[cfg(windows)]
impl WindowsUiAutomator {
    /// 创建新的 Windows UiAutomator 实例。
    ///
    /// 后续将在此初始化 COM(`CoInitializeEx`)并创建 `IUIAutomation` 实例。
    pub fn new() -> Result<Self> {
        Ok(Self { _placeholder: () })
    }
}

#[cfg(windows)]
impl Default for WindowsUiAutomator {
    fn default() -> Self {
        Self::new().expect("WindowsUiAutomator::new should not fail in skeleton form")
    }
}

#[cfg(windows)]
impl UiAutomator for WindowsUiAutomator {
    fn find_element(&self, _selector: &ElementSelector) -> Result<UiElement> {
        anyhow::bail!(
            "WindowsUiAutomator::find_element not yet implemented; skeleton only (T-E-C-03)"
        )
    }

    fn click(&self, _element: &UiElement) -> Result<()> {
        anyhow::bail!("WindowsUiAutomator::click not yet implemented; skeleton only (T-E-C-03)")
    }

    fn type_text(&self, _element: &UiElement, _text: &str) -> Result<()> {
        anyhow::bail!("WindowsUiAutomator::type_text not yet implemented; skeleton only (T-E-C-03)")
    }

    fn scroll(&self, _direction: ScrollDirection, _amount: u32) -> Result<()> {
        anyhow::bail!("WindowsUiAutomator::scroll not yet implemented; skeleton only (T-E-C-03)")
    }

    fn screenshot(&self) -> Result<Vec<u8>> {
        anyhow::bail!(
            "WindowsUiAutomator::screenshot not yet implemented; skeleton only (T-E-C-03)"
        )
    }

    fn get_active_window(&self) -> Result<WindowInfo> {
        anyhow::bail!(
            "WindowsUiAutomator::get_active_window not yet implemented; skeleton only (T-E-C-03)"
        )
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        anyhow::bail!(
            "WindowsUiAutomator::list_windows not yet implemented; skeleton only (T-E-C-03)"
        )
    }
}

// ----------------------------------------------------------------------
// Mock 实现(用于测试)
// ----------------------------------------------------------------------

/// Mock 操作记录 — 用于断言被调用的操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockAction {
    /// 查找元素,记录 selector。
    FindElement(ElementSelector),
    /// 点击元素,记录元素 name(或 "<unnamed>")。
    Click { element_name: String },
    /// 输入文本,记录元素 name 与文本。
    TypeText { element_name: String, text: String },
    /// 滚动,记录方向与量。
    Scroll {
        direction: ScrollDirection,
        amount: u32,
    },
    /// 截图。
    Screenshot,
    /// 获取活动窗口。
    GetActiveWindow,
    /// 列出窗口。
    ListWindows,
}

/// MockUiAutomator — 用于测试的 mock 实现。
///
/// 在 `find_element` / `list_windows` / `get_active_window` 上返回预设的罐装数据,
/// 同时通过内部 `Mutex` 记录所有操作,供测试断言。`screenshot` 返回预设字节流。
pub struct MockUiAutomator {
    /// 预设元素列表(`find_element` 在其中匹配)。
    elements: Vec<UiElement>,
    /// 预设窗口列表(`list_windows` 返回)。
    windows: Vec<WindowInfo>,
    /// 预设活动窗口(`get_active_window` 返回)。
    active_window: Option<WindowInfo>,
    /// 预设截图字节流(`screenshot` 返回)。
    screenshot_bytes: Vec<u8>,
    /// 已记录的操作列表。
    actions: std::sync::Mutex<Vec<MockAction>>,
}

impl MockUiAutomator {
    /// 创建空的 mock(无元素、无窗口、无活动窗口)。
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            windows: Vec::new(),
            active_window: None,
            screenshot_bytes: Vec::new(),
            actions: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 链式添加预设元素。
    pub fn with_element(mut self, element: UiElement) -> Self {
        self.elements.push(element);
        self
    }

    /// 链式设置预设窗口列表。
    pub fn with_windows(mut self, windows: Vec<WindowInfo>) -> Self {
        self.windows = windows;
        self
    }

    /// 链式设置预设活动窗口。
    pub fn with_active_window(mut self, window: WindowInfo) -> Self {
        self.active_window = Some(window);
        self
    }

    /// 链式设置预设截图字节流。
    pub fn with_screenshot_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.screenshot_bytes = bytes;
        self
    }

    /// 取出已记录的操作列表(清空内部记录)。
    pub fn take_actions(&self) -> Vec<MockAction> {
        std::mem::take(
            &mut *self
                .actions
                .lock()
                .expect("MockUiAutomator actions mutex poisoned"),
        )
    }

    /// 返回已记录的操作数量。
    pub fn action_count(&self) -> usize {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .len()
    }

    /// 元素是否匹配指定 selector(内部匹配逻辑)。
    fn matches(element: &UiElement, selector: &ElementSelector) -> bool {
        match selector {
            ElementSelector::ById(id) => element.id.as_deref() == Some(id.as_str()),
            ElementSelector::ByName(name) => element.name.as_deref() == Some(name.as_str()),
            ElementSelector::ByText(text) => element.text.as_deref() == Some(text.as_str()),
            ElementSelector::ByRole(role) => element.role.as_deref() == Some(role.as_str()),
            ElementSelector::ByClass(class) => element.role.as_deref() == Some(class.as_str()),
            ElementSelector::ByXPath(_) => {
                // Mock 不实现 XPath 求值,视为匹配第一个元素(由调用方控制)。
                true
            }
        }
    }

    /// 提取元素的可读标识(用于操作记录)。
    fn element_name(element: &UiElement) -> String {
        element
            .name
            .clone()
            .or_else(|| element.id.clone())
            .unwrap_or_else(|| "<unnamed>".to_string())
    }
}

impl Default for MockUiAutomator {
    fn default() -> Self {
        Self::new()
    }
}

impl UiAutomator for MockUiAutomator {
    fn find_element(&self, selector: &ElementSelector) -> Result<UiElement> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::FindElement(selector.clone()));
        self.elements
            .iter()
            .find(|e| Self::matches(e, selector))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "MockUiAutomator: no element matches selector {:?}",
                    selector
                )
            })
    }

    fn click(&self, element: &UiElement) -> Result<()> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::Click {
                element_name: Self::element_name(element),
            });
        Ok(())
    }

    fn type_text(&self, element: &UiElement, text: &str) -> Result<()> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::TypeText {
                element_name: Self::element_name(element),
                text: text.to_string(),
            });
        Ok(())
    }

    fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<()> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::Scroll { direction, amount });
        Ok(())
    }

    fn screenshot(&self) -> Result<Vec<u8>> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::Screenshot);
        Ok(self.screenshot_bytes.clone())
    }

    fn get_active_window(&self) -> Result<WindowInfo> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::GetActiveWindow);
        self.active_window
            .clone()
            .ok_or_else(|| anyhow::anyhow!("MockUiAutomator: no active window preset"))
    }

    fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        self.actions
            .lock()
            .expect("MockUiAutomator actions mutex poisoned")
            .push(MockAction::ListWindows);
        Ok(self.windows.clone())
    }
}

// ----------------------------------------------------------------------
// 单元测试
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 数据结构测试 ----

    #[test]
    fn element_selector_serialization_roundtrip() {
        let selectors = vec![
            ElementSelector::ById("login-btn".into()),
            ElementSelector::ByName("Submit".into()),
            ElementSelector::ByXPath("//Button[@id='ok']".into()),
            ElementSelector::ByText("Hello".into()),
            ElementSelector::ByClass("Button".into()),
            ElementSelector::ByRole("Edit".into()),
        ];
        for s in selectors {
            let json = serde_json::to_string(&s).expect("serialize");
            let back: ElementSelector = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(s, back, "roundtrip failed for {}", json);
        }
    }

    #[test]
    fn scroll_direction_all_variants() {
        let dirs = [
            ScrollDirection::Up,
            ScrollDirection::Down,
            ScrollDirection::Left,
            ScrollDirection::Right,
        ];
        for d in dirs {
            let json = serde_json::to_string(&d).expect("serialize");
            let back: ScrollDirection = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(d, back);
        }
    }

    #[test]
    fn element_bounds_default_is_zero() {
        let b = ElementBounds::default();
        assert_eq!(b, ElementBounds::new(0, 0, 0, 0));
    }

    #[test]
    fn element_bounds_center() {
        let b = ElementBounds::new(100, 200, 50, 60);
        let (cx, cy) = b.center();
        assert_eq!(cx, 125);
        assert_eq!(cy, 230);
    }

    #[test]
    fn ui_element_builder_chain() {
        let child = UiElement::empty().with_name("child").with_role("Text");
        let el = UiElement::empty()
            .with_id("parent")
            .with_name("Parent")
            .with_role("Button")
            .with_bounds(ElementBounds::new(10, 20, 30, 40))
            .with_text("Click me")
            .with_child(child);
        assert_eq!(el.id.as_deref(), Some("parent"));
        assert_eq!(el.name.as_deref(), Some("Parent"));
        assert_eq!(el.role.as_deref(), Some("Button"));
        assert_eq!(el.bounds, ElementBounds::new(10, 20, 30, 40));
        assert_eq!(el.text.as_deref(), Some("Click me"));
        assert_eq!(el.children.len(), 1);
        assert_eq!(el.children[0].name.as_deref(), Some("child"));
    }

    #[test]
    fn window_info_serialization() {
        let w = WindowInfo {
            title: "Nebula".into(),
            handle: 12345,
            bounds: ElementBounds::new(0, 0, 800, 600),
            process_name: "nebula.exe".into(),
        };
        let json = serde_json::to_string(&w).expect("serialize");
        let back: WindowInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(w.title, back.title);
        assert_eq!(w.handle, back.handle);
        assert_eq!(w.bounds, back.bounds);
        assert_eq!(w.process_name, back.process_name);
    }

    // ---- MockUiAutomator 行为测试 ----

    fn sample_element() -> UiElement {
        UiElement::empty()
            .with_id("username")
            .with_name("Username")
            .with_role("Edit")
            .with_text("")
            .with_bounds(ElementBounds::new(10, 10, 200, 30))
    }

    #[test]
    fn mock_find_element_by_id() {
        let mock = MockUiAutomator::new().with_element(sample_element());
        let el = mock
            .find_element(&ElementSelector::ById("username".into()))
            .expect("should find by id");
        assert_eq!(el.id.as_deref(), Some("username"));
        assert_eq!(el.role.as_deref(), Some("Edit"));
    }

    #[test]
    fn mock_find_element_by_name() {
        let mock = MockUiAutomator::new().with_element(sample_element());
        let el = mock
            .find_element(&ElementSelector::ByName("Username".into()))
            .expect("should find by name");
        assert_eq!(el.name.as_deref(), Some("Username"));
    }

    #[test]
    fn mock_find_element_by_text() {
        let el = UiElement::empty().with_text("Sign in");
        let mock = MockUiAutomator::new().with_element(el);
        let found = mock
            .find_element(&ElementSelector::ByText("Sign in".into()))
            .expect("should find by text");
        assert_eq!(found.text.as_deref(), Some("Sign in"));
    }

    #[test]
    fn mock_find_element_not_found_returns_err() {
        let mock = MockUiAutomator::new().with_element(sample_element());
        let res = mock.find_element(&ElementSelector::ById("nonexistent".into()));
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();
        assert!(
            err.contains("no element matches"),
            "unexpected err: {}",
            err
        );
    }

    #[test]
    fn mock_find_element_records_action() {
        let mock = MockUiAutomator::new().with_element(sample_element());
        let _ = mock.find_element(&ElementSelector::ById("username".into()));
        let actions = mock.take_actions();
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            MockAction::FindElement(ElementSelector::ById(s)) if s == "username"
        ));
    }

    #[test]
    fn mock_click_records_action_with_name() {
        let mock = MockUiAutomator::new();
        let el = sample_element();
        mock.click(&el).expect("click should succeed");
        let actions = mock.take_actions();
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            MockAction::Click { element_name } if element_name == "Username"
        ));
    }

    #[test]
    fn mock_type_text_records_action_with_text() {
        let mock = MockUiAutomator::new();
        let el = sample_element();
        mock.type_text(&el, "hello world")
            .expect("type_text should succeed");
        let actions = mock.take_actions();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            MockAction::TypeText { element_name, text } => {
                assert_eq!(element_name, "Username");
                assert_eq!(text, "hello world");
            }
            other => panic!("unexpected action: {:?}", other),
        }
    }

    #[test]
    fn mock_scroll_records_direction_and_amount() {
        let mock = MockUiAutomator::new();
        mock.scroll(ScrollDirection::Down, 5)
            .expect("scroll should succeed");
        let actions = mock.take_actions();
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            &actions[0],
            MockAction::Scroll {
                direction: ScrollDirection::Down,
                amount: 5
            }
        ));
    }

    #[test]
    fn mock_screenshot_returns_preset_bytes() {
        let preset = vec![0x89, 0x50, 0x4E, 0x47]; // PNG header
        let mock = MockUiAutomator::new().with_screenshot_bytes(preset.clone());
        let bytes = mock.screenshot().expect("screenshot should succeed");
        assert_eq!(bytes, preset);
        let actions = mock.take_actions();
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], MockAction::Screenshot));
    }

    #[test]
    fn mock_get_active_window_returns_preset() {
        let win = WindowInfo {
            title: "Active".into(),
            handle: 99,
            bounds: ElementBounds::new(0, 0, 100, 100),
            process_name: "app.exe".into(),
        };
        let mock = MockUiAutomator::new().with_active_window(win.clone());
        let got = mock.get_active_window().expect("should return preset");
        assert_eq!(got.title, win.title);
        assert_eq!(got.handle, win.handle);
        assert_eq!(got.process_name, win.process_name);
    }

    #[test]
    fn mock_get_active_window_without_preset_errors() {
        let mock = MockUiAutomator::new();
        let res = mock.get_active_window();
        assert!(res.is_err());
    }

    #[test]
    fn mock_list_windows_returns_preset() {
        let wins = vec![
            WindowInfo {
                title: "W1".into(),
                handle: 1,
                bounds: ElementBounds::default(),
                process_name: "a.exe".into(),
            },
            WindowInfo {
                title: "W2".into(),
                handle: 2,
                bounds: ElementBounds::default(),
                process_name: "b.exe".into(),
            },
        ];
        let mock = MockUiAutomator::new().with_windows(wins.clone());
        let got = mock.list_windows().expect("should return preset");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "W1");
        assert_eq!(got[1].handle, 2);
    }

    #[test]
    fn mock_action_count_and_take_clears() {
        let mock = MockUiAutomator::new();
        let el = sample_element();
        mock.click(&el).unwrap();
        mock.scroll(ScrollDirection::Up, 1).unwrap();
        assert_eq!(mock.action_count(), 2);
        let taken = mock.take_actions();
        assert_eq!(taken.len(), 2);
        // take 后内部清空
        assert_eq!(mock.action_count(), 0);
    }

    #[test]
    fn mock_default_is_empty() {
        let mock = MockUiAutomator::default();
        assert_eq!(mock.action_count(), 0);
        assert!(mock.list_windows().unwrap().is_empty());
        assert!(mock.get_active_window().is_err());
    }

    #[test]
    fn mock_find_element_by_role() {
        let el = UiElement::empty().with_role("Button");
        let mock = MockUiAutomator::new().with_element(el);
        let found = mock
            .find_element(&ElementSelector::ByRole("Button".into()))
            .expect("should find by role");
        assert_eq!(found.role.as_deref(), Some("Button"));
    }

    #[test]
    fn mock_find_element_xpath_matches_first() {
        let el = sample_element();
        let mock = MockUiAutomator::new().with_element(el);
        let found = mock
            .find_element(&ElementSelector::ByXPath("//Edit[@id='username']".into()))
            .expect("xpath should match first element in mock");
        assert_eq!(found.id.as_deref(), Some("username"));
    }

    // ---- Windows 骨架测试(仅 Windows 平台编译) ----

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_new_succeeds() {
        let _auto = WindowsUiAutomator::new().expect("new should succeed in skeleton form");
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_default_works() {
        let _auto = WindowsUiAutomator::default();
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_find_element_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let res = auto.find_element(&ElementSelector::ById("any".into()));
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("not yet implemented"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_click_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let el = UiElement::empty();
        let res = auto.click(&el);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("not yet implemented"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_type_text_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let el = UiElement::empty();
        let res = auto.type_text(&el, "x");
        assert!(res.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_scroll_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let res = auto.scroll(ScrollDirection::Down, 1);
        assert!(res.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_screenshot_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let res = auto.screenshot();
        assert!(res.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_get_active_window_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let res = auto.get_active_window();
        assert!(res.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_uiautomator_list_windows_not_implemented() {
        let auto = WindowsUiAutomator::new().unwrap();
        let res = auto.list_windows();
        assert!(res.is_err());
    }

    // ---- Trait 对象测试 ----

    #[test]
    fn mock_is_dyn_compatible() {
        // 验证 MockUiAutomator 可作为 Box<dyn UiAutomator> 使用。
        let mock: Box<dyn UiAutomator> =
            Box::new(MockUiAutomator::new().with_element(sample_element()));
        let el = mock
            .find_element(&ElementSelector::ById("username".into()))
            .expect("dyn call should find");
        assert_eq!(el.id.as_deref(), Some("username"));
    }
}
