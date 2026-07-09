//! T-E-C-06: Hybrid Browser Agent — 核心抽象层。
//!
//! 定义 [`BrowserAgent`] trait 与所有模式共享的数据类型。两种实现:
//! * [`crate::browser::api_mode::ApiBrowserAgent`] — API 模式,通过 HTTP 抓取 +
//!   HTML 解析操作网页(reqwest,无浏览器依赖)。
//! * [`crate::browser::vlm_mode::VlmBrowserAgent`] — VLM 模式,通过截图 + 视觉
//!   语言模型识别页面元素并操作(框架化,需 vision feature 与 VLM 后端)。
//!
//! 设计参考 T-E-C-01 OS-Controller 双模式:同一 trait,两种策略,PlanEngine
//! 可按页面特征自动选最优(`BrowserMode::Auto` 留作后续编排)。

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 浏览器自动化模式。
///
/// * `Api` — HTTP API 模式:直接抓取页面 HTML,通过解析 DOM 操作。
///   适合静态页面、表单提交、数据提取;无渲染开销,但不能执行 JS。
/// * `Vlm` — 视觉语言模型模式:截图后由 VLM(如 qwen2.5-vl)识别元素坐标,
///   再模拟点击/输入。适合动态 JS 页面、Canvas、反爬站点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserMode {
    /// HTTP API 模式 — reqwest 抓取 + HTML 解析。
    Api,
    /// 视觉语言模型模式 — 截图 + VLM 识别。
    Vlm,
}

impl std::fmt::Display for BrowserMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrowserMode::Api => write!(f, "api"),
            BrowserMode::Vlm => write!(f, "vlm"),
        }
    }
}

/// 滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollDirection {
    /// 向上滚动。
    Up,
    /// 向下滚动。
    Down,
}

/// 当前页面快照。
///
/// API 模式下 `html` / `text` 来自抓取响应;VLM 模式下 `screenshot_b64`
/// 来自截图,`html` 可能为空(VLM 不依赖 DOM)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserPage {
    /// 当前页面 URL(可能因重定向与请求 URL 不同)。
    pub url: String,
    /// `<title>` 标签内容(API 模式解析得到;VLM 模式可空)。
    pub title: String,
    /// HTTP 状态码(如 200/404)。VLM 模式无 HTTP 概念时为 0。
    pub status: u16,
    /// 原始 HTML 文本。
    pub html: String,
    /// 从 HTML 提取的纯文本(去标签)。
    pub text: String,
    /// base64 编码的 PNG 截图(VLM 模式填充,API 模式为 None)。
    pub screenshot_b64: Option<String>,
}

impl BrowserPage {
    /// 构造一个空白页面(URL 未导航)。
    pub fn empty() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            status: 0,
            html: String::new(),
            text: String::new(),
            screenshot_b64: None,
        }
    }
}

/// 页面元素描述 — 选择器匹配到的单个元素。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserElement {
    /// 标签名(小写,如 "a" / "input" / "button")。
    pub tag: String,
    /// `id` 属性值。
    pub id: Option<String>,
    /// `class` 属性拆分后的列表。
    pub classes: Vec<String>,
    /// 元素文本内容(已 trim)。
    pub text: String,
    /// 关键属性键值对(href / name / type / value 等)。
    pub attributes: HashMap<String, String>,
    /// 推荐的选择器(优先 #id,其次 tag.class)。
    pub selector: String,
}

/// `extract` 操作返回的提取内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedContent {
    /// 使用的选择器。
    pub selector: String,
    /// 匹配到的元素列表。
    pub elements: Vec<BrowserElement>,
    /// 所有匹配元素的拼接纯文本。
    pub text: String,
    /// 所有匹配元素的外层 HTML 拼接。
    pub raw_html: String,
}

/// `click` 操作返回的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickResult {
    /// 被点击元素的选择器。
    pub selector: String,
    /// 是否触发了导航(API 模式下点击 `<a>` 或提交表单会导航)。
    pub navigated: bool,
    /// 导航后的新 URL(若 navigated=true)。
    pub new_url: Option<String>,
}

/// `type_text` 操作返回的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeResult {
    /// 被填充元素的选择器。
    pub selector: String,
    /// 填入的值。
    pub value: String,
}

/// 浏览器 Agent 统一错误类型。
#[derive(Debug, Error)]
pub enum BrowserError {
    /// 网络/HTTP 请求失败。
    #[error("network error: {0}")]
    Network(String),
    /// HTML 解析失败。
    #[error("parse error: {0}")]
    Parse(String),
    /// 选择器未匹配到任何元素。
    #[error("element not found for selector: {0}")]
    NotFound(String),
    /// SSRF 守卫拦截(目标地址为内网/loopback 等不安全地址)。
    #[error("SSRF blocked: {0}")]
    SsrfBlocked(String),
    /// VLM 模式不可用(未配置后端 / vision feature 未启用)。
    #[error("VLM mode unavailable: {0}")]
    VlmUnavailable(String),
    /// 参数非法(空 URL / 非法选择器等)。
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// 其他内部错误。
    #[error("internal: {0}")]
    Internal(String),
}

impl BrowserError {
    /// 从 anyhow::Error 构造 Internal 错误。
    pub fn internal(err: &anyhow::Error) -> Self {
        BrowserError::Internal(format!("{err:#}"))
    }
}

/// Hybrid Browser Agent 统一接口。
///
/// 两种模式实现同一接口,使上层(PlanEngine / 工具调用)无需关心底层策略。
/// 所有方法异步,可安全并发调用(实现需自行处理内部可变性)。
///
/// # 方法语义
///
/// * `navigate` — 加载指定 URL,返回页面快照。
/// * `click` — 点击匹配选择器的元素(API 模式:跟随链接/提交表单)。
/// * `type_text` — 在匹配的输入框填入文本(API 模式:暂存,待表单提交)。
/// * `scroll` — 滚动页面(API 模式为 no-op,因整页已加载)。
/// * `screenshot` — 返回 base64 PNG(API 模式返回错误,需 VLM 模式)。
/// * `extract` — 提取匹配选择器的元素列表与文本。
/// * `current_page` — 返回当前已加载页面快照(不重新请求)。
#[async_trait]
pub trait BrowserAgent: Send + Sync {
    /// 返回当前 Agent 的模式。
    fn mode(&self) -> BrowserMode;

    /// 导航到指定 URL,返回页面快照。
    async fn navigate(&self, url: &str) -> Result<BrowserPage, BrowserError>;

    /// 点击匹配选择器的元素。
    async fn click(&self, selector: &str) -> Result<ClickResult, BrowserError>;

    /// 在匹配选择器的输入框中填入文本。
    async fn type_text(&self, selector: &str, text: &str) -> Result<TypeResult, BrowserError>;

    /// 滚动页面。
    async fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<(), BrowserError>;

    /// 截图,返回 base64 编码的 PNG。
    async fn screenshot(&self) -> Result<String, BrowserError>;

    /// 提取匹配选择器的元素。
    async fn extract(&self, selector: &str) -> Result<ExtractedContent, BrowserError>;

    /// 返回当前页面快照(不重新请求网络)。
    async fn current_page(&self) -> Result<BrowserPage, BrowserError>;
}
