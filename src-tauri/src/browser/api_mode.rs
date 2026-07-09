//! T-E-C-06: API 模式 Browser Agent — 通过 HTTP API 操作网页。
//!
//! 完整实现:使用 `reqwest`(已有非 optional 依赖)抓取页面,用 `regex`
//! 做轻量 HTML 解析(避免引入 scraper/select 等新依赖,遵循零新依赖原则)。
//! URL 安全校验复用 [`crate::security::ssrf_guard::SsrfGuard`]。
//!
//! ## 适用场景
//! 静态页面抓取、表单提交(GET/POST)、数据提取。不能执行 JavaScript,
//! 对 SPA / 动态渲染页面请用 VLM 模式。
//!
//! ## 选择器语法(子集)
//! * `*` — 所有元素
//! * `tag` — 指定标签(如 `a` / `input`)
//! * `#id` — 按 id 属性
//! * `.class` — 按 class 属性
//! * `tag.class` / `tag#id` — 组合
//!
//! ## 状态管理
//! Agent 内部用 `parking_lot::RwLock` 保存当前页面快照与待提交表单字段,
//! 支持并发读、互斥写。`type_text` 暂存字段值,`click` 提交表单时一并带上。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;
use regex::Regex;
use tracing::{debug, instrument};

use crate::browser::agent::{
    BrowserAgent, BrowserElement, BrowserError, BrowserMode, BrowserPage, ClickResult,
    ExtractedContent, ScrollDirection, TypeResult,
};
use crate::security::ssrf_guard::SsrfGuard;

/// 待提交的表单字段(name → value),由 `type_text` 暂存,`click` 提交时消费。
type PendingFields = HashMap<String, String>;

/// API 模式 Browser Agent 内部状态。
struct ApiAgentInner {
    /// 当前页面快照。
    page: BrowserPage,
    /// 暂存的表单字段(`type_text` 写入,表单提交 `click` 消费)。
    pending_fields: PendingFields,
}

/// API 模式 Browser Agent — 通过 HTTP 抓取 + HTML 解析操作网页。
///
/// 无浏览器依赖,适合静态页面。构造时可选传入自定义 `reqwest::Client`
/// (如需代理/自定义 UA);默认构造一个 30s 超时、桌面浏览器 UA 的客户端。
///
/// # 示例
/// ```no_run
/// use nebula_lib::browser::api_mode::ApiBrowserAgent;
/// use nebula_lib::browser::agent::BrowserAgent;
/// # async fn demo() -> anyhow::Result<()> {
/// let agent = ApiBrowserAgent::new();
/// let page = agent.navigate("https://example.com").await?;
/// let content = agent.extract("a").await?;
/// println!("找到 {} 个链接", content.elements.len());
/// # Ok(()) }
/// ```
pub struct ApiBrowserAgent {
    inner: Arc<RwLock<ApiAgentInner>>,
    /// HTTP 客户端(复用连接池)。
    client: reqwest::Client,
    /// SSRF 守卫。默认拒绝私网/loopback;构造时可放开(如访问本地服务)。
    ssrf: SsrfGuard,
}

impl ApiBrowserAgent {
    /// 创建默认 Agent:30s 超时、桌面 UA、SSRF 拒绝私网与 loopback。
    pub fn new() -> Self {
        let client = default_client();
        Self {
            inner: Arc::new(RwLock::new(ApiAgentInner {
                page: BrowserPage::empty(),
                pending_fields: HashMap::new(),
            })),
            client,
            ssrf: SsrfGuard::new(),
        }
    }

    /// 允许访问 loopback 地址(127.0.0.0/8 + ::1)。
    ///
    /// 用于测试本地服务或开发环境;私网(10.x/172.16.x/192.168.x)仍被拒绝。
    pub fn allow_loopback(mut self) -> Self {
        self.ssrf = SsrfGuard::new().with_allow_loopback(true);
        self
    }

    /// 允许访问私网地址(含 loopback)。
    ///
    /// **安全提示**:仅在可信环境使用,生产环境暴露此能力会导致 SSRF 风险。
    pub fn allow_private(mut self) -> Self {
        self.ssrf = SsrfGuard::new()
            .with_allow_private(true)
            .with_allow_loopback(true);
        self
    }

    /// 使用自定义 HTTP 客户端构造(用于注入代理/证书/自定义 UA)。
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(RwLock::new(ApiAgentInner {
                page: BrowserPage::empty(),
                pending_fields: HashMap::new(),
            })),
            client,
            ssrf: SsrfGuard::new(),
        }
    }

    /// 校验 URL 并发起 GET 请求,返回响应文本与最终 URL(跟随重定向后)。
    async fn fetch_get(&self, url: &str) -> Result<(String, String, u16), BrowserError> {
        if url.trim().is_empty() {
            return Err(BrowserError::InvalidArgument(
                "URL must not be empty".to_string(),
            ));
        }
        // SSRF 校验 — 阻止访问内网/loopback(除非构造时显式放开)。
        self.ssrf
            .validate_url(url)
            .map_err(|e| BrowserError::SsrfBlocked(format!("{e:#}")))?;

        debug!(url = %url, "API 模式 GET 请求");
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| BrowserError::Network(format!("GET {url} failed: {e}")))?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let text = resp
            .text()
            .await
            .map_err(|e| BrowserError::Network(format!("read body failed: {e}")))?;
        Ok((text, final_url, status))
    }

    /// 校验 URL 并发起 POST 请求(表单提交)。
    async fn fetch_post(
        &self,
        url: &str,
        fields: &PendingFields,
    ) -> Result<(String, String, u16), BrowserError> {
        if url.trim().is_empty() {
            return Err(BrowserError::InvalidArgument(
                "URL must not be empty".to_string(),
            ));
        }
        self.ssrf
            .validate_url(url)
            .map_err(|e| BrowserError::SsrfBlocked(format!("{e:#}")))?;

        debug!(url = %url, fields = fields.len(), "API 模式 POST 表单提交");
        // reqwest 的 .form() 自动做 application/x-www-form-urlencoded 编码,
        // 无需手动调用 serde_urlencode_compat(后者仅用于 GET 查询串拼接)。
        let resp = self
            .client
            .post(url)
            .form(fields)
            .send()
            .await
            .map_err(|e| BrowserError::Network(format!("POST {url} failed: {e}")))?;
        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let text = resp
            .text()
            .await
            .map_err(|e| BrowserError::Network(format!("read body failed: {e}")))?;
        Ok((text, final_url, status))
    }

    /// 把抓取的 HTML 写入当前页面快照。
    fn store_page(&self, html: String, url: String, status: u16) -> BrowserPage {
        let title = extract_title(&html).unwrap_or_default();
        let text = strip_tags(&html);
        let page = BrowserPage {
            url,
            title,
            status,
            html,
            text,
            screenshot_b64: None,
        };
        let mut guard = self.inner.write();
        guard.page = page.clone();
        page
    }

    /// 从当前页面 HTML 中查找匹配选择器的元素。
    fn find_elements(&self, selector: &str) -> Result<Vec<BrowserElement>, BrowserError> {
        let guard = self.inner.read();
        let html = &guard.page.html;
        if html.is_empty() {
            return Err(BrowserError::InvalidArgument(
                "no page loaded; call navigate() first".to_string(),
            ));
        }
        let sel = parse_selector(selector)?;
        Ok(select_elements(html, &sel))
    }
}

impl Default for ApiBrowserAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BrowserAgent for ApiBrowserAgent {
    fn mode(&self) -> BrowserMode {
        BrowserMode::Api
    }

    #[instrument(skip(self), fields(url = %url))]
    async fn navigate(&self, url: &str) -> Result<BrowserPage, BrowserError> {
        let (html, final_url, status) = self.fetch_get(url).await?;
        // 导航到新页面时清空暂存表单字段。
        {
            let mut guard = self.inner.write();
            guard.pending_fields.clear();
        }
        Ok(self.store_page(html, final_url, status))
    }

    #[instrument(skip(self), fields(selector = %selector))]
    async fn click(&self, selector: &str) -> Result<ClickResult, BrowserError> {
        let elements = self.find_elements(selector)?;
        let target = elements
            .into_iter()
            .next()
            .ok_or_else(|| BrowserError::NotFound(selector.to_string()))?;

        // 点击 <a href>:跟随链接导航。
        if target.tag == "a" {
            if let Some(href) = target.attributes.get("href") {
                let base = {
                    let guard = self.inner.read();
                    guard.page.url.clone()
                };
                let abs = resolve_url(&base, href);
                if let Some(abs) = abs {
                    let new_page = self.navigate(&abs).await?;
                    return Ok(ClickResult {
                        selector: selector.to_string(),
                        navigated: true,
                        new_url: Some(new_page.url),
                    });
                }
            }
            // 无 href 或无法解析的链接 — 视为无导航点击。
            return Ok(ClickResult {
                selector: selector.to_string(),
                navigated: false,
                new_url: None,
            });
        }

        // 点击 <button>/<input type=submit>:提交所属表单。
        if target.tag == "button"
            || (target.tag == "input"
                && target
                    .attributes
                    .get("type")
                    .map(|t| t.eq_ignore_ascii_case("submit"))
                    .unwrap_or(false))
        {
            return self.submit_form(&target).await;
        }

        // 其他元素:API 模式无法模拟交互,返回未导航。
        Ok(ClickResult {
            selector: selector.to_string(),
            navigated: false,
            new_url: None,
        })
    }

    #[instrument(skip(self, text), fields(selector = %selector))]
    async fn type_text(&self, selector: &str, text: &str) -> Result<TypeResult, BrowserError> {
        let elements = self.find_elements(selector)?;
        let target = elements
            .into_iter()
            .next()
            .ok_or_else(|| BrowserError::NotFound(selector.to_string()))?;

        // 仅 input/textarea/select 可填入。其他元素报错。
        let is_input = matches!(target.tag.as_str(), "input" | "textarea" | "select");
        if !is_input {
            return Err(BrowserError::InvalidArgument(format!(
                "element <{}> is not a form field; only input/textarea/select can receive text",
                target.tag
            )));
        }
        // 表单字段以 name 属性为键;无 name 时报错(无法提交)。
        let name = target.attributes.get("name").cloned().ok_or_else(|| {
            BrowserError::InvalidArgument(format!(
                "element <{}> has no name attribute; cannot bind to form field",
                target.tag
            ))
        })?;

        let mut guard = self.inner.write();
        guard.pending_fields.insert(name, text.to_string());

        Ok(TypeResult {
            selector: selector.to_string(),
            value: text.to_string(),
        })
    }

    async fn scroll(&self, _direction: ScrollDirection, _amount: u32) -> Result<(), BrowserError> {
        // API 模式整页已加载,滚动为 no-op(与真实浏览器不同,无懒加载触发)。
        debug!("API 模式 scroll 为 no-op(整页已加载)");
        Ok(())
    }

    async fn screenshot(&self) -> Result<String, BrowserError> {
        // API 模式不渲染页面,无法截图。需切换 VLM 模式。
        Err(BrowserError::VlmUnavailable(
            "API mode cannot screenshot; use VLM mode (BrowserMode::Vlm)".to_string(),
        ))
    }

    #[instrument(skip(self), fields(selector = %selector))]
    async fn extract(&self, selector: &str) -> Result<ExtractedContent, BrowserError> {
        let elements = self.find_elements(selector)?;
        if elements.is_empty() {
            return Err(BrowserError::NotFound(selector.to_string()));
        }
        let text = elements
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let raw_html = elements
            .iter()
            .map(render_element_html)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ExtractedContent {
            selector: selector.to_string(),
            elements,
            text,
            raw_html,
        })
    }

    async fn current_page(&self) -> Result<BrowserPage, BrowserError> {
        let guard = self.inner.read();
        if guard.page.url.is_empty() {
            return Err(BrowserError::InvalidArgument(
                "no page loaded; call navigate() first".to_string(),
            ));
        }
        Ok(guard.page.clone())
    }
}

impl ApiBrowserAgent {
    /// 提交包含目标提交按钮的表单。
    ///
    /// 查找当前页面 HTML 中包含该按钮的 `<form>`,提取 action/method,
    /// 合并表单内所有字段的默认值与暂存的 `pending_fields`,发起请求。
    async fn submit_form(&self, submit_btn: &BrowserElement) -> Result<ClickResult, BrowserError> {
        let (html, base_url, mut fields) = {
            let guard = self.inner.read();
            (
                guard.page.html.clone(),
                guard.page.url.clone(),
                guard.pending_fields.clone(),
            )
        };

        let form = find_enclosing_form(&html, submit_btn).ok_or_else(|| {
            BrowserError::Parse(format!(
                "no <form> encloses submit element <{}>",
                submit_btn.tag
            ))
        })?;

        // 收集表单内所有可提交字段的默认值(input/textarea/select 的 value)。
        for fe in &form.fields {
            // 暂存字段优先(用户通过 type_text 设置的值覆盖默认值)。
            fields
                .entry(fe.name.clone())
                .or_insert_with(|| fe.value.clone());
        }

        let action = form.action.as_deref().unwrap_or(&base_url);
        let abs = resolve_url(&base_url, action).unwrap_or_else(|| action.to_string());

        let (resp_html, final_url, status) = if form.method.eq_ignore_ascii_case("post") {
            self.fetch_post(&abs, &fields).await?
        } else {
            // GET 表单:把字段拼到 query string。
            let get_url = build_get_url(&abs, &fields);
            self.fetch_get(&get_url).await?
        };

        // 提交后清空暂存字段。
        {
            let mut guard = self.inner.write();
            guard.pending_fields.clear();
        }
        self.store_page(resp_html, final_url, status);

        Ok(ClickResult {
            selector: submit_btn.selector.clone(),
            navigated: true,
            new_url: Some(self.inner.read().page.url.clone()),
        })
    }
}

// ── 内部辅助:HTTP 客户端构造 ─────────────────────────────────────────

/// 构造默认 HTTP 客户端:30s 超时 + 桌面浏览器 UA。
fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ",
            "AppleWebKit/537.36 (KHTML, like Gecko) ",
            "Chrome/124.0.0.0 Safari/537.36"
        ))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 把表单字段编码为 application/x-www-form-urlencoded 字符串。
///
/// 不依赖 serde_urlencoded crate(零新依赖),用简单的 percent-encoding。
/// 复用 reqwest 已传递依赖的 `form_urlencoded`(reqwest 依赖 url crate)。
fn serde_urlencode_compat(fields: &PendingFields) -> String {
    use url::form_urlencoded;
    let mut encoder = form_urlencoded::Serializer::new(String::new());
    for (k, v) in fields {
        encoder.append_pair(k, v);
    }
    encoder.finish()
}

/// 构造 GET 表单 URL(把字段拼到 query string)。
fn build_get_url(base: &str, fields: &PendingFields) -> String {
    let qs = serde_urlencode_compat(fields);
    if qs.is_empty() {
        return base.to_string();
    }
    if base.contains('?') {
        format!("{base}&{qs}")
    } else {
        format!("{base}?{qs}")
    }
}

// ── 内部辅助:URL 解析 ───────────────────────────────────────────────

/// 将可能相对的 href 解析为绝对 URL(基于 base_url)。
fn resolve_url(base_url: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with("javascript:") || href.starts_with("#") {
        return None;
    }
    // 绝对 URL(http:// / https:// / mailto: 等)直接返回。
    if let Ok(u) = url::Url::parse(href) {
        return Some(u.to_string());
    }
    // 相对 URL:基于 base 解析。
    let base = url::Url::parse(base_url).ok()?;
    base.join(href).ok().map(|u| u.to_string())
}

// ── 内部辅助:HTML 解析(基于 regex 的轻量解析) ─────────────────────

/// 从 HTML 提取 `<title>` 内容。
fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    let caps = re.captures(html)?;
    Some(decode_entities(caps.get(1)?.as_str().trim()))
}

/// 去除所有 HTML 标签,返回纯文本。保留标签间空白。
fn strip_tags(html: &str) -> String {
    // 先去除 script/style 内容(避免 JS/CSS 代码污染文本)。
    // 注:Rust regex crate 不支持反向引用(\1),故用两条独立正则分别处理。
    let no_script = Regex::new(r"(?is)<script[^>]*>.*?</script>")
        .map(|re| re.replace_all(html, " ").to_string())
        .unwrap_or_else(|_| html.to_string());
    let no_style = Regex::new(r"(?is)<style[^>]*>.*?</style>")
        .map(|re| re.replace_all(&no_script, " ").to_string())
        .unwrap_or_else(|_| no_script);
    // 去除所有标签。
    let no_tags = Regex::new(r"(?s)<[^>]+>")
        .map(|re| re.replace_all(&no_style, " ").to_string())
        .unwrap_or_else(|_| no_style);
    // 折叠连续空白。
    let collapsed = Regex::new(r"\s+")
        .map(|re| re.replace_all(&no_tags, " ").to_string())
        .unwrap_or_else(|_| no_tags);
    decode_entities(&collapsed).trim().to_string()
}

/// 解码常见 HTML 实体(&amp; &lt; &gt; &quot; &#39; &nbsp;)。
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

/// 解析后的选择器。
#[derive(Debug, Clone)]
struct ParsedSelector {
    /// 标签名(小写),None 表示通配 `*`。
    tag: Option<String>,
    /// id 约束(None = 不限)。
    id: Option<String>,
    /// class 约束(匹配元素需包含所有指定 class)。
    classes: Vec<String>,
}

/// 解析简单 CSS 选择器为 [`ParsedSelector`]。
///
/// 支持: `*` / `tag` / `#id` / `.class` / `tag.class` / `tag#id` / `tag.c1.c2`
fn parse_selector(selector: &str) -> Result<ParsedSelector, BrowserError> {
    let s = selector.trim();
    if s.is_empty() {
        return Err(BrowserError::InvalidArgument(
            "selector must not be empty".to_string(),
        ));
    }
    if s == "*" {
        return Ok(ParsedSelector {
            tag: None,
            id: None,
            classes: vec![],
        });
    }

    // 提取 tag 部分(开头到第一个 # 或 . 之前)。
    let tag_end = s.find(['#', '.']).unwrap_or(s.len());
    let tag = if tag_end == 0 {
        None
    } else {
        let t = s[..tag_end].to_lowercase();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };

    // 解析剩余的 #id / .class 片段。
    let rest = &s[tag_end..];
    let mut id = None;
    let mut classes = Vec::new();
    let mut iter = rest.chars().peekable();
    // 注意:不能用 take_while — 它会消费终止谓词的首字符(经典 Rust 坑),
    // 导致连续的 .class1.class2 丢失 class2。用 peek 手动收集,不消费分隔符。
    while let Some(&c) = iter.peek() {
        if c == '#' || c == '.' {
            iter.next(); // 消费 # 或 .
            let mut v = String::new();
            while let Some(&ch) = iter.peek() {
                if ch == '#' || ch == '.' {
                    break;
                }
                v.push(ch);
                iter.next();
            }
            if !v.is_empty() {
                if c == '#' {
                    id = Some(v);
                } else {
                    classes.push(v);
                }
            }
        } else {
            // 非法字符,跳过。
            iter.next();
        }
    }

    if tag.is_none() && id.is_none() && classes.is_empty() {
        return Err(BrowserError::InvalidArgument(format!(
            "invalid selector: {selector}"
        )));
    }
    Ok(ParsedSelector { tag, id, classes })
}

/// 检查元素是否匹配选择器。
fn matches_selector(tag: &str, attrs: &HashMap<String, String>, sel: &ParsedSelector) -> bool {
    if let Some(t) = &sel.tag {
        if tag != t {
            return false;
        }
    }
    if let Some(id) = &sel.id {
        match attrs.get("id") {
            Some(elem_id) if elem_id == id => {}
            _ => return false,
        }
    }
    if !sel.classes.is_empty() {
        let elem_classes: Vec<&str> = attrs
            .get("class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default();
        for want in &sel.classes {
            if !elem_classes.iter().any(|ec| ec == want) {
                return false;
            }
        }
    }
    true
}

/// 一个 HTML 开始标签的解析结果。
#[derive(Debug, Clone)]
struct StartTag {
    tag: String,
    attrs: HashMap<String, String>,
    /// 标签结束位置(>`后的偏移)。
    end: usize,
    /// 是否自闭合(<br/>)。
    self_closing: bool,
}

/// 解析单个开始标签字符串(如 `<a href="x" class="y">`)的属性。
fn parse_tag_attrs(tag_str: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    // 匹配 attr="value" / attr='value' / attr=value / attr
    let re = Regex::new(r#"(?s)([a-zA-Z_:][a-zA-Z0-9_:.-]*)\s*(?:=\s*("[^"]*"|'[^']*'|[^\s>]+))?"#)
        .expect("attr regex should compile");
    for caps in re.captures_iter(tag_str) {
        let name = caps.get(1).map(|m| m.as_str().to_lowercase());
        let value = caps.get(2).map(|m| {
            let v = m.as_str();
            // 去除引号。
            if (v.starts_with('"') && v.ends_with('"'))
                || (v.starts_with('\'') && v.ends_with('\''))
            {
                v[1..v.len() - 1].to_string()
            } else {
                v.to_string()
            }
        });
        if let Some(name) = name {
            attrs.entry(name).or_insert(value.unwrap_or_default());
        }
    }
    attrs
}

/// 扫描 HTML,提取所有开始标签。
fn scan_start_tags(html: &str) -> Vec<StartTag> {
    let tag_re = Regex::new(r"(?s)<([a-zA-Z][a-zA-Z0-9]*)((?:[^>]*)?)/?>")
        .expect("tag regex should compile");
    let mut tags = Vec::new();
    for caps in tag_re.captures_iter(html) {
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        let tag = caps
            .get(1)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        let attr_str = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let self_closing = whole.trim_end().ends_with('/');
        let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
        let end = start + whole.len();
        let attrs = parse_tag_attrs(attr_str);
        tags.push(StartTag {
            tag,
            attrs,
            end,
            self_closing,
        });
    }
    tags
}

/// Void 元素(无闭合标签,内容为空)。
fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// 找到指定开始标签对应的闭合标签位置,返回元素内容区间(不含标签本身)。
///
/// 通过标签深度计数处理嵌套同名标签。若为 void 元素或自闭合,返回空区间。
fn find_element_content_range(html: &str, start_tag: &StartTag) -> (usize, usize) {
    if is_void_element(&start_tag.tag) || start_tag.self_closing {
        return (start_tag.end, start_tag.end);
    }
    let close_tag = format!("</{}>", start_tag.tag);
    let open_prefix = format!("<{}", start_tag.tag);
    let mut depth = 1;
    let mut pos = start_tag.end;
    let bytes = html.as_bytes();
    while pos < bytes.len() {
        // 查找下一个同名开/闭标签。
        let rest = &html[pos..];
        let next_close = rest.find(&close_tag);
        let next_open = rest.find(&open_prefix).filter(|&i| {
            // 确认是标签开始(<tag 后跟空格/>/=),而非 <tag2 误匹配。
            let after = rest.as_bytes().get(i + open_prefix.len()).copied();
            matches!(
                after,
                Some(b' ') | Some(b'>') | Some(b'/') | Some(b'\t') | Some(b'\n')
            )
        });
        match (next_close, next_open) {
            (Some(c), Some(o)) => {
                if o < c {
                    depth += 1;
                    pos += o + open_prefix.len();
                } else {
                    depth -= 1;
                    if depth == 0 {
                        let content_end = pos + c;
                        return (start_tag.end, content_end);
                    }
                    pos += c + close_tag.len();
                }
            }
            (Some(c), None) => {
                depth -= 1;
                if depth == 0 {
                    let content_end = pos + c;
                    return (start_tag.end, content_end);
                }
                pos += c + close_tag.len();
            }
            (None, Some(o)) => {
                depth += 1;
                pos += o + open_prefix.len();
            }
            (None, None) => break,
        }
    }
    // 未找到闭合标签,内容到文末。
    (start_tag.end, html.len())
}

/// 从 HTML 中选择匹配选择器的元素,构造 [`BrowserElement`] 列表。
fn select_elements(html: &str, sel: &ParsedSelector) -> Vec<BrowserElement> {
    let tags = scan_start_tags(html);
    let mut results = Vec::new();
    for tag in &tags {
        if !matches_selector(&tag.tag, &tag.attrs, sel) {
            continue;
        }
        let (content_start, content_end) = find_element_content_range(html, tag);
        let content = if content_end > content_start {
            &html[content_start..content_end]
        } else {
            ""
        };
        let text = if is_void_element(&tag.tag) || tag.self_closing {
            String::new()
        } else {
            strip_tags(content)
        };
        let selector = recommend_selector(&tag.tag, &tag.attrs);
        results.push(BrowserElement {
            tag: tag.tag.clone(),
            id: tag.attrs.get("id").cloned(),
            classes: tag
                .attrs
                .get("class")
                .map(|c| c.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            text,
            attributes: tag.attrs.clone(),
            selector,
        });
    }
    results
}

/// 为元素生成推荐选择器(优先 #id,其次 tag.class,最后 tag)。
fn recommend_selector(tag: &str, attrs: &HashMap<String, String>) -> String {
    if let Some(id) = attrs.get("id") {
        if !id.is_empty() {
            return format!("#{id}");
        }
    }
    if let Some(class) = attrs.get("class") {
        let first: Option<&str> = class.split_whitespace().next();
        if let Some(c) = first {
            return format!("{tag}.{c}");
        }
    }
    tag.to_string()
}

/// 把 [`BrowserElement`] 渲染回简化 HTML(用于 raw_html 输出)。
fn render_element_html(e: &BrowserElement) -> String {
    let mut attrs = String::new();
    for (k, v) in &e.attributes {
        attrs.push(' ');
        attrs.push_str(k);
        attrs.push_str("=\"");
        attrs.push_str(v);
        attrs.push('"');
    }
    if e.text.is_empty() {
        format!("<{}{} />", e.tag, attrs)
    } else {
        format!("<{}{}>{}</{}>", e.tag, attrs, e.text, e.tag)
    }
}

/// 表单字段默认值。
#[derive(Debug, Clone)]
struct FormField {
    name: String,
    value: String,
}

/// 解析后的表单。
#[derive(Debug, Clone)]
struct ParsedForm {
    action: Option<String>,
    method: String,
    fields: Vec<FormField>,
}

/// 在 HTML 中查找包含指定提交按钮的 `<form>`,解析其 action/method/字段。
fn find_enclosing_form(html: &str, submit: &BrowserElement) -> Option<ParsedForm> {
    let tags = scan_start_tags(html);
    // 先找所有 <form> 开始标签及其内容范围。
    let forms: Vec<(StartTag, usize, usize)> = tags
        .iter()
        .filter(|t| t.tag == "form")
        .map(|t| {
            let (cs, ce) = find_element_content_range(html, t);
            (t.clone(), cs, ce)
        })
        .collect();

    // 提交按钮的属性签名:用 id+class+text 定位(简化匹配)。
    let submit_id = submit.attributes.get("id").cloned();
    let submit_name = submit.attributes.get("name").cloned();

    // 遍历表单,找一个包含与提交按钮匹配的 <button>/<input type=submit>。
    let opt_re =
        Regex::new(r"(?is)<option[^>]*>(.*?)</option>").expect("option regex should compile");
    for (form_tag, content_start, content_end) in &forms {
        let form_content = &html[*content_start..*content_end];
        let inner_tags = scan_start_tags(form_content);
        // 检查表单内是否有匹配的提交按钮。
        let has_submit = inner_tags.iter().any(|t| {
            if t.tag != submit.tag {
                return false;
            }
            if t.tag == "input" {
                let ty = t.attrs.get("type").map(|s| s.as_str()).unwrap_or("");
                if !ty.eq_ignore_ascii_case("submit") {
                    return false;
                }
            }
            // 匹配 id 或 name(若提交按钮有)。
            if let Some(id) = &submit_id {
                if t.attrs.get("id").map(|s| s.as_str()) == Some(id.as_str()) {
                    return true;
                }
            }
            if let Some(name) = &submit_name {
                if t.attrs.get("name").map(|s| s.as_str()) == Some(name.as_str()) {
                    return true;
                }
            }
            // 无 id/name 时,只要表单内有同类型提交按钮即视为匹配。
            submit_id.is_none() && submit_name.is_none()
        });
        if !has_submit {
            continue;
        }
        // 收集表单内所有可提交字段。
        let mut fields = Vec::new();
        for t in &inner_tags {
            match t.tag.as_str() {
                "input" => {
                    let ty = t.attrs.get("type").map(|s| s.as_str()).unwrap_or("text");
                    // submit/button/image/reset 不作为数据字段。
                    if matches!(
                        ty.to_lowercase().as_str(),
                        "submit" | "button" | "image" | "reset"
                    ) {
                        continue;
                    }
                    if let Some(name) = t.attrs.get("name") {
                        let value = t.attrs.get("value").cloned().unwrap_or_default();
                        // checkbox/radio 仅在 checked 时提交(简化:始终包含)。
                        fields.push(FormField {
                            name: name.clone(),
                            value,
                        });
                    }
                }
                "textarea" => {
                    if let Some(name) = t.attrs.get("name") {
                        let (cs, ce) = find_element_content_range(form_content, t);
                        let value = if ce > cs {
                            decode_entities(form_content[cs..ce].trim())
                        } else {
                            String::new()
                        };
                        fields.push(FormField {
                            name: name.clone(),
                            value,
                        });
                    }
                }
                "select" => {
                    if let Some(name) = t.attrs.get("name") {
                        // 简化:取第一个 <option> 的文本作为值。
                        let (cs, ce) = find_element_content_range(form_content, t);
                        let sel_content = if ce > cs { &form_content[cs..ce] } else { "" };
                        let value = opt_re
                            .captures(sel_content)
                            .and_then(|c| Some(strip_tags(c.get(1)?.as_str())))
                            .unwrap_or_default();
                        fields.push(FormField {
                            name: name.clone(),
                            value,
                        });
                    }
                }
                _ => {}
            }
        }
        return Some(ParsedForm {
            action: form_tag.attrs.get("action").cloned(),
            method: form_tag
                .attrs
                .get("method")
                .cloned()
                .unwrap_or_else(|| "GET".to_string()),
            fields,
        });
    }
    None
}

// ── 单元测试 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_selector_tag() {
        let sel = parse_selector("a").unwrap();
        assert_eq!(sel.tag.as_deref(), Some("a"));
        assert!(sel.id.is_none());
        assert!(sel.classes.is_empty());
    }

    #[test]
    fn test_parse_selector_id() {
        let sel = parse_selector("#main").unwrap();
        assert!(sel.tag.is_none());
        assert_eq!(sel.id.as_deref(), Some("main"));
    }

    #[test]
    fn test_parse_selector_class() {
        let sel = parse_selector(".btn").unwrap();
        assert!(sel.tag.is_none());
        assert_eq!(sel.classes, vec!["btn".to_string()]);
    }

    #[test]
    fn test_parse_selector_tag_class_combo() {
        let sel = parse_selector("input.form-control.required").unwrap();
        assert_eq!(sel.tag.as_deref(), Some("input"));
        assert_eq!(
            sel.classes,
            vec!["form-control".to_string(), "required".to_string()]
        );
    }

    #[test]
    fn test_parse_selector_tag_id_combo() {
        let sel = parse_selector("div#header").unwrap();
        assert_eq!(sel.tag.as_deref(), Some("div"));
        assert_eq!(sel.id.as_deref(), Some("header"));
    }

    #[test]
    fn test_parse_selector_wildcard() {
        let sel = parse_selector("*").unwrap();
        assert!(sel.tag.is_none());
        assert!(sel.id.is_none());
        assert!(sel.classes.is_empty());
    }

    #[test]
    fn test_parse_selector_empty_rejected() {
        assert!(parse_selector("").is_err());
        assert!(parse_selector("   ").is_err());
    }

    #[test]
    fn test_extract_title() {
        let html = "<html><head><title>  Hello &amp; World  </title></head></html>";
        let title = extract_title(html).unwrap();
        assert_eq!(title, "Hello & World");
    }

    #[test]
    fn test_extract_title_missing() {
        let html = "<html><body>no title</body></html>";
        assert!(extract_title(html).is_none());
    }

    #[test]
    fn test_strip_tags_basic() {
        let html = "<p>Hello <b>world</b></p><script>var x=1;</script>";
        let text = strip_tags(html);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn test_strip_tags_preserves_entities() {
        let html = "<p>a &lt; b &amp; c &gt; d</p>";
        let text = strip_tags(html);
        assert_eq!(text, "a < b & c > d");
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_entities("&quot;hi&quot;"), "\"hi\"");
        assert_eq!(decode_entities("&#39;x&#x27;"), "'x'");
        assert_eq!(decode_entities("a&nbsp;b"), "a b");
    }

    #[test]
    fn test_select_elements_by_tag() {
        let html = r#"<div><a href="/a">A</a><a href="/b">B</a><p>text</p></div>"#;
        let sel = parse_selector("a").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 2);
        assert_eq!(elems[0].tag, "a");
        assert_eq!(elems[0].text, "A");
        assert_eq!(
            elems[0].attributes.get("href").map(|s| s.as_str()),
            Some("/a")
        );
        assert_eq!(elems[1].text, "B");
    }

    #[test]
    fn test_select_elements_by_id() {
        let html = r#"<div id="main">main</div><div id="side">side</div>"#;
        let sel = parse_selector("#main").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 1);
        assert_eq!(elems[0].text, "main");
    }

    #[test]
    fn test_select_elements_by_class() {
        let html = r#"<p class="intro">a</p><p class="body">b</p><span class="intro">c</span>"#;
        let sel = parse_selector(".intro").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 2);
        assert_eq!(elems[0].tag, "p");
        assert_eq!(elems[1].tag, "span");
    }

    #[test]
    fn test_select_elements_tag_class_combo() {
        let html = r#"<input class="btn primary"><button class="btn">x</button>"#;
        let sel = parse_selector("input.btn").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 1);
        assert_eq!(elems[0].tag, "input");
    }

    #[test]
    fn test_select_elements_wildcard() {
        let html = r#"<a><b><c></c></b></a>"#;
        let sel = parse_selector("*").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 3);
    }

    #[test]
    fn test_select_nested_same_tag() {
        let html = r#"<div>outer <div>inner</div></div>"#;
        let sel = parse_selector("div").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 2);
        assert_eq!(elems[0].text, "outer inner");
        assert_eq!(elems[1].text, "inner");
    }

    #[test]
    fn test_select_void_elements_no_content() {
        let html = r#"<img src="x.png"><br><input type="text">"#;
        let sel = parse_selector("img").unwrap();
        let elems = select_elements(html, &sel);
        assert_eq!(elems.len(), 1);
        assert!(elems[0].text.is_empty());
    }

    #[test]
    fn test_recommend_selector() {
        let mut attrs = HashMap::new();
        attrs.insert("id".to_string(), "nav".to_string());
        assert_eq!(recommend_selector("div", &attrs), "#nav");

        let mut attrs = HashMap::new();
        attrs.insert("class".to_string(), "btn primary".to_string());
        assert_eq!(recommend_selector("a", &attrs), "a.btn");

        let attrs = HashMap::new();
        assert_eq!(recommend_selector("p", &attrs), "p");
    }

    #[test]
    fn test_resolve_url_absolute() {
        let u = resolve_url("https://example.com/page", "https://other.com/x");
        assert_eq!(u.as_deref(), Some("https://other.com/x"));
    }

    #[test]
    fn test_resolve_url_relative() {
        let u = resolve_url("https://example.com/dir/page", "sub");
        assert_eq!(u.as_deref(), Some("https://example.com/dir/sub"));
    }

    #[test]
    fn test_resolve_url_root_relative() {
        let u = resolve_url("https://example.com/dir/page", "/root");
        assert_eq!(u.as_deref(), Some("https://example.com/root"));
    }

    #[test]
    fn test_resolve_url_javascript_ignored() {
        assert!(resolve_url("https://example.com", "javascript:void(0)").is_none());
        assert!(resolve_url("https://example.com", "#anchor").is_none());
        assert!(resolve_url("https://example.com", "").is_none());
    }

    #[test]
    fn test_build_get_url() {
        let mut fields = HashMap::new();
        fields.insert("q".to_string(), "hello world".to_string());
        fields.insert("page".to_string(), "1".to_string());
        let url = build_get_url("https://example.com/search", &fields);
        assert!(url.starts_with("https://example.com/search?"));
        assert!(url.contains("q=hello+world"));
        assert!(url.contains("page=1"));
    }

    #[test]
    fn test_build_get_url_existing_query() {
        let mut fields = HashMap::new();
        fields.insert("x".to_string(), "1".to_string());
        let url = build_get_url("https://example.com/?a=b", &fields);
        assert!(url.contains("?a=b&"));
        assert!(url.contains("x=1"));
    }

    #[test]
    fn test_find_enclosing_form() {
        let html = r#"<form action="/login" method="post">
            <input name="user" value="">
            <input name="pass" type="password">
            <button name="submit" type="submit">Login</button>
        </form>"#;
        let submit = BrowserElement {
            tag: "button".to_string(),
            id: None,
            classes: vec![],
            text: "Login".to_string(),
            attributes: {
                let mut m = HashMap::new();
                m.insert("name".to_string(), "submit".to_string());
                m.insert("type".to_string(), "submit".to_string());
                m
            },
            selector: "button".to_string(),
        };
        let form = find_enclosing_form(html, &submit).expect("form should be found");
        assert_eq!(form.action.as_deref(), Some("/login"));
        assert_eq!(form.method, "post");
        assert_eq!(form.fields.len(), 2);
        assert!(form.fields.iter().any(|f| f.name == "user"));
        assert!(form.fields.iter().any(|f| f.name == "pass"));
    }

    #[test]
    fn test_default_client_constructs() {
        let _client = default_client();
    }

    #[tokio::test]
    async fn test_api_agent_mode() {
        let agent = ApiBrowserAgent::new();
        assert_eq!(agent.mode(), BrowserMode::Api);
    }

    #[tokio::test]
    async fn test_navigate_empty_url_rejected() {
        let agent = ApiBrowserAgent::new();
        let err = agent
            .navigate("")
            .await
            .expect_err("empty URL should be rejected");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn test_navigate_ssrf_blocked() {
        // 默认拒绝私网/loopback — 访问 localhost 应被 SSRF 守卫拦截。
        let agent = ApiBrowserAgent::new();
        let err = agent
            .navigate("http://127.0.0.1:1/")
            .await
            .expect_err("loopback should be SSRF-blocked");
        assert!(
            matches!(err, BrowserError::SsrfBlocked(_)),
            "expected SsrfBlocked, got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_navigate_allow_loopback() {
        // 显式 allow_loopback 后,loopback 不再被拦截(连接会失败但不该是 SsrfBlocked)。
        let agent = ApiBrowserAgent::new().allow_loopback();
        let err = agent
            .navigate("http://127.0.0.1:1/")
            .await
            .expect_err("connection should fail");
        assert!(
            matches!(err, BrowserError::Network(_)),
            "expected Network error (connection refused), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_extract_before_navigate_errors() {
        let agent = ApiBrowserAgent::new();
        let err = agent
            .extract("a")
            .await
            .expect_err("extract before navigate should error");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn test_current_page_before_navigate_errors() {
        let agent = ApiBrowserAgent::new();
        let err = agent
            .current_page()
            .await
            .expect_err("current_page before navigate should error");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn test_screenshot_unavailable_in_api_mode() {
        let agent = ApiBrowserAgent::new();
        let err = agent
            .screenshot()
            .await
            .expect_err("screenshot should fail in API mode");
        assert!(matches!(err, BrowserError::VlmUnavailable(_)));
    }

    #[tokio::test]
    async fn test_scroll_is_noop() {
        let agent = ApiBrowserAgent::new();
        // scroll 在未导航时也应成功(no-op)。
        agent
            .scroll(ScrollDirection::Down, 5)
            .await
            .expect("scroll should be a no-op success");
    }

    /// 用内嵌 HTML 验证完整 API 模式流程:navigate(模拟) → extract → type_text → click。
    ///
    /// 由于 navigate 需要真实 HTTP,这里用 `with_client` + mock 不便,改用
    /// 直接验证解析逻辑的集成:构造 Agent,手动 store_page(通过 navigate
    /// 的内部逻辑无法直接调用),因此用 extract 验证解析链路。
    #[tokio::test]
    async fn test_extract_full_flow() {
        let agent = ApiBrowserAgent::new();
        // 手动注入页面快照(模拟 navigate 后的状态)。
        let html = r#"<html><head><title>Test Page</title></head><body>
            <a href="/link1" class="nav" id="first">Link One</a>
            <a href="/link2" class="nav">Link Two</a>
            <input name="q" type="text" value="" class="search">
            <form action="/search" method="get">
                <input name="query" type="text" value="default">
                <button type="submit" name="go">Search</button>
            </form>
        </body></html>"#;
        agent.store_page(html.to_string(), "https://example.com/".to_string(), 200);

        // extract 链接。
        let content = agent.extract(".nav").await.expect("extract .nav");
        assert_eq!(content.elements.len(), 2);
        assert_eq!(content.elements[0].text, "Link One");
        assert_eq!(content.elements[1].text, "Link Two");

        // extract by id。
        let content = agent.extract("#first").await.expect("extract #first");
        assert_eq!(content.elements.len(), 1);
        assert_eq!(
            content.elements[0]
                .attributes
                .get("href")
                .map(|s| s.as_str()),
            Some("/link1")
        );

        // extract form button。
        let content = agent.extract("button").await.expect("extract button");
        assert_eq!(content.elements.len(), 1);
        assert_eq!(content.elements[0].text, "Search");

        // current_page 返回注入的快照。
        let page = agent.current_page().await.expect("current_page");
        assert_eq!(page.title, "Test Page");
        assert_eq!(page.status, 200);
        assert!(page.text.contains("Link One"));
    }

    /// 验证 type_text 暂存字段 + find_enclosing_form 解析的表单字段默认值合并逻辑
    /// (不发起真实 HTTP,仅验证字段收集)。
    #[tokio::test]
    async fn test_type_text_stores_field() {
        let agent = ApiBrowserAgent::new();
        let html = r#"<form action="/search" method="get">
            <input name="q" type="text" value="">
            <button type="submit" name="go">Go</button>
        </form>"#;
        agent.store_page(html.to_string(), "https://example.com/".to_string(), 200);

        let result = agent
            .type_text("input", "hello")
            .await
            .expect("type_text should store field");
        assert_eq!(result.value, "hello");

        // 验证字段已暂存(通过 click 提交会触发,但 click 需 HTTP;
        // 这里只验证 type_text 不报错且返回正确值)。
    }

    /// type_text 对非表单元素应报错。
    #[tokio::test]
    async fn test_type_text_rejects_non_input() {
        let agent = ApiBrowserAgent::new();
        let html = r#"<div id="content">text</div>"#;
        agent.store_page(html.to_string(), "https://example.com/".to_string(), 200);

        let err = agent
            .type_text("#content", "hello")
            .await
            .expect_err("type_text on div should fail");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    /// type_text 对无 name 属性的 input 应报错。
    #[tokio::test]
    async fn test_type_text_rejects_nameless_input() {
        let agent = ApiBrowserAgent::new();
        let html = r#"<input type="text" value="">"#;
        agent.store_page(html.to_string(), "https://example.com/".to_string(), 200);

        let err = agent
            .type_text("input", "hello")
            .await
            .expect_err("type_text on nameless input should fail");
        assert!(matches!(err, BrowserError::InvalidArgument(_)));
    }

    /// click 未匹配元素应返回 NotFound。
    #[tokio::test]
    async fn test_click_not_found() {
        let agent = ApiBrowserAgent::new();
        let html = r#"<div>content</div>"#;
        agent.store_page(html.to_string(), "https://example.com/".to_string(), 200);

        let err = agent
            .click("a")
            .await
            .expect_err("click non-existent should fail");
        assert!(matches!(err, BrowserError::NotFound(_)));
    }

    /// extract 未匹配应返回 NotFound。
    #[tokio::test]
    async fn test_extract_not_found() {
        let agent = ApiBrowserAgent::new();
        let html = r#"<div>content</div>"#;
        agent.store_page(html.to_string(), "https://example.com/".to_string(), 200);

        let err = agent
            .extract("a")
            .await
            .expect_err("extract non-existent should fail");
        assert!(matches!(err, BrowserError::NotFound(_)));
    }
}
