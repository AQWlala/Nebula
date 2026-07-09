//! T-E-C-06: VLM 模式 Browser Agent — 通过视觉语言模型操作网页。
//!
//! 框架化实现:VLM 模式通过截图 + 视觉语言模型(如 qwen2.5-vl:3b)识别
//! 页面元素坐标,再模拟点击/输入。适合动态 JS 页面、Canvas、反爬站点。
//!
//! ## 架构
//!
//! * [`VisionAnalyzer`] trait — 解耦 VLM 后端,使 VlmBrowserAgent 不依赖
//!   AppState/LlmGateway,可独立测试。生产环境注入 [`HttpVisionAnalyzer`]
//!   (调用 Ollama 兼容的 `/api/chat` 多模态端点),测试环境注入 mock。
//! * [`VlmBrowserAgent`] — 持有 `VisionAnalyzer` + 可选浏览器后端,实现
//!   [`BrowserAgent`] trait。截图捕获由 `vision` feature 门控(复用
//!   T-E-C-02 的 screenshots + image crate)。
//!
//! ## 当前状态(框架)
//!
//! 截图捕获、浏览器驱动接入(真实 CDP/WebView)未在此任务完成,留作后续:
//! * `screenshot` — vision feature 开启时捕获主屏;否则返回错误。
//! * `navigate`/`click`/`type_text` — 调用 VisionAnalyzer 分析截图,但真实
//!   点击/输入需浏览器驱动(CDP/WebDriver),当前返回分析结果或 NotImplemented。
//!
//! 这与 T-E-C-01 OS-Controller 的 macOS/Linux 骨架占位模式一致:trait 完整
//! 定义,平台/后端能力后续填充。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};

use crate::browser::agent::{
    BrowserAgent, BrowserElement, BrowserError, BrowserMode, BrowserPage, ClickResult,
    ExtractedContent, ScrollDirection, TypeResult,
};

/// VLM 视觉分析后端抽象 — 把 base64 截图 + prompt 交给视觉语言模型,返回文本。
///
/// 解耦 `LlmGateway` 以便单元测试注入 mock。生产实现见 [`HttpVisionAnalyzer`]。
#[async_trait]
pub trait VisionAnalyzer: Send + Sync {
    /// 分析截图,返回 VLM 生成的文本响应。
    ///
    /// `image_b64` 为无 data-uri 前缀的 base64 PNG 字符串。
    async fn analyze(&self, image_b64: &str, prompt: &str) -> Result<String, BrowserError>;

    /// 返回后端模型名(用于日志/诊断)。
    fn model_name(&self) -> &str;
}

/// Ollama 兼容的 HTTP VLM 后端 — 调用 `/api/chat` 多模态端点。
///
/// wire format 与 `crate::llm::gateway::LlmGateway::describe_image` 一致:
/// `{"model": "...", "messages": [{"role":"user","content":"...","images":[b64]}]}`。
/// 零新依赖:仅用 reqwest + serde_json(均为非 optional 依赖)。
///
/// 生产环境可直接复用 `LlmGateway::describe_image`,但为保持 VLM 模块自包含
/// 与可测性,这里提供独立的 HTTP 实现。
pub struct HttpVisionAnalyzer {
    /// Ollama 兼容 API 根 URL(如 `http://127.0.0.1:11434`)。
    base_url: String,
    /// 视觉模型名(如 `qwen2.5-vl:3b`)。
    model: String,
    /// HTTP 客户端(复用连接池)。
    client: reqwest::Client,
}

impl HttpVisionAnalyzer {
    /// 创建 HTTP VLM 后端。
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120)) // VLM 推理较慢,放宽超时。
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.into(),
            model: model.into(),
            client,
        }
    }

    /// 默认后端:本地 Ollama + qwen2.5-vl:3b(与 AppConfig::vision_model 默认一致)。
    pub fn default_ollama() -> Self {
        Self::new("http://127.0.0.1:11434", "qwen2.5-vl:3b")
    }
}

/// Ollama /api/chat 多模态请求体。
#[derive(Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
    images: Vec<&'a str>,
}

/// Ollama /api/chat 响应体。
#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaRespMessage,
}

#[derive(Deserialize)]
struct OllamaRespMessage {
    content: String,
}

#[async_trait]
impl VisionAnalyzer for HttpVisionAnalyzer {
    #[instrument(skip(self, image_b64), fields(model = %self.model))]
    async fn analyze(&self, image_b64: &str, prompt: &str) -> Result<String, BrowserError> {
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let req = OllamaChatRequest {
            model: &self.model,
            messages: vec![OllamaMessage {
                role: "user",
                content: prompt,
                images: vec![image_b64],
            }],
            stream: false,
        };
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| BrowserError::Network(format!("VLM analyze POST {url} failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BrowserError::VlmUnavailable(format!(
                "VLM HTTP {status}: {body}"
            )));
        }
        let parsed: OllamaChatResponse = resp
            .json()
            .await
            .map_err(|e| BrowserError::Parse(format!("VLM response parse failed: {e}")))?;
        Ok(parsed.message.content)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

/// VLM 模式 Browser Agent。
///
/// 持有可选的 [`VisionAnalyzer`] 后端。未注入时所有需要 VLM 的操作返回
/// [`BrowserError::VlmUnavailable`];注入后通过截图 + VLM 分析驱动操作。
///
/// 截图捕获由 `vision` feature 门控;未启用时 `screenshot`/`navigate` 返回错误,
/// 与 `commands::os::screenshot` 命令的行为一致(T-E-C-02 模式)。
pub struct VlmBrowserAgent {
    inner: Arc<RwLock<BrowserPage>>,
    /// VLM 后端(None = 未配置,操作返回 VlmUnavailable)。
    analyzer: Option<Arc<dyn VisionAnalyzer>>,
}

impl VlmBrowserAgent {
    /// 创建未配置 VLM 后端的 Agent(所有 VLM 操作返回错误)。
    ///
    /// 用于:仅需要 trait 占位的场景,或运行时再注入后端。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BrowserPage::empty())),
            analyzer: None,
        }
    }

    /// 注入 VLM 后端,返回新的 Agent。
    pub fn with_analyzer(mut self, analyzer: Arc<dyn VisionAnalyzer>) -> Self {
        self.analyzer = Some(analyzer);
        self
    }

    /// 用 [`HttpVisionAnalyzer`] 作为默认 Ollama 后端。
    pub fn with_default_ollama() -> Self {
        Self::new().with_analyzer(Arc::new(HttpVisionAnalyzer::default_ollama()))
    }

    /// 捕获主屏截图,返回 base64 PNG(vision feature 门控)。
    ///
    /// 复用 T-E-C-02 `commands::os::screenshot` 的实现逻辑:screenshots crate
    /// 抓屏 → image crate 编码 PNG → base64。
    #[cfg(feature = "vision")]
    async fn capture_screenshot(&self) -> Result<String, BrowserError> {
        use base64::Engine;
        use screenshots::Screen;

        let screens = Screen::all()
            .map_err(|e| BrowserError::Internal(format!("enumerate screens failed: {e}")))?;
        let screen = screens.into_iter().next().ok_or_else(|| {
            BrowserError::VlmUnavailable("no display available for screenshot".to_string())
        })?;
        let img = screen
            .capture()
            .map_err(|e| BrowserError::Internal(format!("screen capture failed: {e}")))?;
        let rgba: image::RgbaImage =
            image::ImageBuffer::from_raw(img.width(), img.height(), img.as_raw().clone())
                .ok_or_else(|| {
                    BrowserError::Parse("failed to construct RgbaImage from raw RGBA".to_string())
                })?;
        let mut buf = std::io::Cursor::new(Vec::<u8>::new());
        rgba.write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| BrowserError::Internal(format!("PNG encode failed: {e}")))?;
        let png_bytes = buf.into_inner();
        Ok(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
    }

    /// vision feature 未启用时,截图不可用。
    #[cfg(not(feature = "vision"))]
    async fn capture_screenshot(&self) -> Result<String, BrowserError> {
        Err(BrowserError::VlmUnavailable(
            "vision feature not enabled; rebuild with --features vision to capture screenshots"
                .to_string(),
        ))
    }

    /// 获取 VLM 后端引用(若配置)。
    fn analyzer(&self) -> Result<&Arc<dyn VisionAnalyzer>, BrowserError> {
        self.analyzer
            .as_ref()
            .ok_or_else(|| BrowserError::VlmUnavailable("no VisionAnalyzer configured".to_string()))
    }

    /// 让 VLM 在截图中定位元素,返回 VLM 的文本描述(坐标/选择建议)。
    ///
    /// 当前为框架:返回 VLM 原始文本响应,真实点击坐标解析留待浏览器驱动接入。
    async fn locate_element(&self, selector: &str) -> Result<String, BrowserError> {
        let analyzer = self.analyzer()?;
        let screenshot = self.capture_screenshot().await?;
        let prompt = format!(
            "You are a browser automation agent. Look at this screenshot and find the element \
             described by: \"{selector}\". Respond with the element's location (coordinates or \
             description) and whether it is visible. Be concise."
        );
        analyzer.analyze(&screenshot, &prompt).await
    }
}

impl Default for VlmBrowserAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BrowserAgent for VlmBrowserAgent {
    fn mode(&self) -> BrowserMode {
        BrowserMode::Vlm
    }

    #[instrument(skip(self), fields(url = %url))]
    async fn navigate(&self, url: &str) -> Result<BrowserPage, BrowserError> {
        if url.trim().is_empty() {
            return Err(BrowserError::InvalidArgument(
                "URL must not be empty".to_string(),
            ));
        }
        // VLM 模式导航需要浏览器驱动(打开 URL 并渲染)。
        // 框架阶段:若有 VLM 后端,尝试截图并分析当前屏幕;否则返回 NotImplemented。
        let screenshot_b64 = if self.analyzer.is_some() {
            match self.capture_screenshot().await {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!(url = %url, error = %e, "VLM navigate: screenshot failed");
                    None
                }
            }
        } else {
            return Err(BrowserError::VlmUnavailable(
                "VLM navigate requires a browser driver (CDP/WebView) which is not yet wired; \
                 configure a VisionAnalyzer and enable vision feature for screenshot analysis"
                    .to_string(),
            ));
        };

        let page = BrowserPage {
            url: url.to_string(),
            title: String::new(),
            status: 0,
            html: String::new(),
            text: String::new(),
            screenshot_b64,
        };
        *self.inner.write() = page.clone();
        debug!(url = %url, "VLM navigate: stored page snapshot (framework)");
        Ok(page)
    }

    #[instrument(skip(self), fields(selector = %selector))]
    async fn click(&self, selector: &str) -> Result<ClickResult, BrowserError> {
        // 真实点击需浏览器驱动执行 JS/dispatchEvent。框架阶段:用 VLM 定位元素,
        // 返回未导航结果(模拟点击未接入)。
        let _description = self.locate_element(selector).await?;
        // TODO(浏览器驱动接入): 把 VLM 返回的坐标转为真实 click 事件。
        Ok(ClickResult {
            selector: selector.to_string(),
            navigated: false,
            new_url: None,
        })
    }

    #[instrument(skip(self, text), fields(selector = %selector))]
    async fn type_text(&self, selector: &str, text: &str) -> Result<TypeResult, BrowserError> {
        // 同 click:VLM 定位 + 真实输入需浏览器驱动。框架阶段返回成功占位。
        let _description = self.locate_element(selector).await?;
        // TODO(浏览器驱动接入): 在定位的输入框中执行输入。
        Ok(TypeResult {
            selector: selector.to_string(),
            value: text.to_string(),
        })
    }

    async fn scroll(&self, _direction: ScrollDirection, _amount: u32) -> Result<(), BrowserError> {
        // VLM 模式滚动需浏览器驱动执行 window.scrollBy。框架阶段为 no-op。
        // 若有 VLM 后端,可截图后分析新视口,但真实滚动未接入。
        if self.analyzer.is_none() {
            return Err(BrowserError::VlmUnavailable(
                "VLM scroll requires a browser driver (not yet wired)".to_string(),
            ));
        }
        debug!("VLM scroll: no-op (browser driver not wired)");
        Ok(())
    }

    async fn screenshot(&self) -> Result<String, BrowserError> {
        self.capture_screenshot().await
    }

    #[instrument(skip(self), fields(selector = %selector))]
    async fn extract(&self, selector: &str) -> Result<ExtractedContent, BrowserError> {
        // VLM 模式无 DOM,extract 依赖 VLM 从截图识别元素。
        let analyzer = self.analyzer()?;
        let screenshot = self.capture_screenshot().await?;
        let prompt = format!(
            "You are a browser automation agent. Look at this screenshot and list all elements \
             matching: \"{selector}\". For each, provide its text and a description. Respond in \
             plain text, one element per line."
        );
        let response = analyzer.analyze(&screenshot, &prompt).await?;
        // 把 VLM 文本响应包装为单个 BrowserElement(text=response)。
        // 真实结构化解析(坐标/多元素拆分)留待后续。
        let elem = BrowserElement {
            tag: "vlm-identified".to_string(),
            id: None,
            classes: vec![],
            text: response.clone(),
            attributes: std::collections::HashMap::new(),
            selector: selector.to_string(),
        };
        Ok(ExtractedContent {
            selector: selector.to_string(),
            elements: vec![elem],
            text: response,
            raw_html: String::new(),
        })
    }

    async fn current_page(&self) -> Result<BrowserPage, BrowserError> {
        let guard = self.inner.read();
        if guard.url.is_empty() {
            return Err(BrowserError::InvalidArgument(
                "no page loaded; call navigate() first".to_string(),
            ));
        }
        Ok(guard.clone())
    }
}

// ── 单元测试 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock VisionAnalyzer — 返回固定文本,用于测试 VlmBrowserAgent 逻辑。
    struct MockAnalyzer {
        response: String,
    }

    #[async_trait]
    impl VisionAnalyzer for MockAnalyzer {
        async fn analyze(&self, _image_b64: &str, _prompt: &str) -> Result<String, BrowserError> {
            Ok(self.response.clone())
        }
        fn model_name(&self) -> &str {
            "mock-vlm"
        }
    }

    #[tokio::test]
    async fn test_vlm_agent_mode() {
        let agent = VlmBrowserAgent::new();
        assert_eq!(agent.mode(), BrowserMode::Vlm);
    }

    #[tokio::test]
    async fn test_vlm_navigate_empty_url_rejected() {
        let agent = VlmBrowserAgent::new();
        let err = agent
            .navigate("")
            .await
            .expect_err("empty URL should be rejected");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn test_vlm_navigate_without_analyzer_errors() {
        // 未配置 VLM 后端 → navigate 返回 VlmUnavailable。
        let agent = VlmBrowserAgent::new();
        let err = agent
            .navigate("https://example.com")
            .await
            .expect_err("navigate without analyzer should fail");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_vlm_screenshot_without_vision_feature() {
        // vision feature 未启用时(默认构建),截图返回 VlmUnavailable。
        let agent = VlmBrowserAgent::new();
        let err = agent
            .screenshot()
            .await
            .expect_err("screenshot without vision feature should fail");
        assert!(
            matches!(err, BrowserError::VlmUnavailable(_)),
            "expected VlmUnavailable, got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_vlm_current_page_before_navigate_errors() {
        let agent = VlmBrowserAgent::new();
        let err = agent
            .current_page()
            .await
            .expect_err("current_page before navigate should fail");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn test_vlm_scroll_without_analyzer_errors() {
        let agent = VlmBrowserAgent::new();
        let err = agent
            .scroll(ScrollDirection::Down, 1)
            .await
            .expect_err("scroll without analyzer should fail");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_vlm_extract_without_analyzer_errors() {
        let agent = VlmBrowserAgent::new();
        let err = agent
            .extract("button")
            .await
            .expect_err("extract without analyzer should fail");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_vlm_click_without_analyzer_errors() {
        let agent = VlmBrowserAgent::new();
        let err = agent
            .click("a")
            .await
            .expect_err("click without analyzer should fail");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_vlm_type_text_without_analyzer_errors() {
        let agent = VlmBrowserAgent::new();
        let err = agent
            .type_text("input", "hi")
            .await
            .expect_err("type_text without analyzer should fail");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    /// 配置 MockAnalyzer 后,extract 返回 mock 文本(截图因 vision feature
    /// 未启用会失败,验证错误透传为 VlmUnavailable)。
    #[tokio::test]
    async fn test_vlm_extract_with_mock_analyzer_no_vision() {
        let analyzer: Arc<dyn VisionAnalyzer> = Arc::new(MockAnalyzer {
            response: "button: Submit".to_string(),
        });
        let agent = VlmBrowserAgent::new().with_analyzer(analyzer);
        // vision feature 未启用 → 截图失败 → extract 透传错误。
        let err = agent
            .extract("button")
            .await
            .expect_err("extract should fail without vision feature");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    #[test]
    fn test_http_vision_analyzer_constructs() {
        let a = HttpVisionAnalyzer::new("http://localhost:11434", "qwen2.5-vl:3b");
        assert_eq!(a.model_name(), "qwen2.5-vl:3b");
    }

    #[test]
    fn test_http_vision_analyzer_default_ollama() {
        let a = HttpVisionAnalyzer::default_ollama();
        assert_eq!(a.model_name(), "qwen2.5-vl:3b");
    }

    #[test]
    fn test_vlm_agent_with_analyzer_builder() {
        let analyzer: Arc<dyn VisionAnalyzer> = Arc::new(MockAnalyzer {
            response: "test".to_string(),
        });
        let agent = VlmBrowserAgent::new().with_analyzer(analyzer);
        assert_eq!(agent.mode(), BrowserMode::Vlm);
        // analyzer 已注入 — analyzer() 不再报错。
        assert!(agent.analyzer().is_ok());
    }

    #[test]
    fn test_vlm_agent_default() {
        let agent = VlmBrowserAgent::default();
        assert_eq!(agent.mode(), BrowserMode::Vlm);
        assert!(agent.analyzer().is_err()); // 默认无后端。
    }
}
