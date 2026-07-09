//! T-E-C-06: Hybrid Browser Agent — API + VLM 双模式浏览器自动化。
//!
//! 本模块提供统一的 [`BrowserAgent`] 抽象与两种实现:
//!
//! | 模式 | 实现 | 适用场景 | 依赖 |
//! |------|------|---------|------|
//! | API  | [`ApiBrowserAgent`] | 静态页面抓取、表单提交、数据提取 | reqwest + regex(均为已有非 optional 依赖) |
//! | VLM  | [`VlmBrowserAgent`] | 动态 JS 页面、Canvas、反爬站点 | vision feature + VLM 后端(Ollama) |
//!
//! ## 设计目标(来自 ROADMAP)
//!
//! > Hybrid Browser Agent:GUI 视觉点击 + CDP 协议(existing-session 复用)
//! > + DOM 选择器,自动选最优。
//!
//! 本任务完成 API 模式(DOM 选择器)的完整逻辑与 VLM 模式(视觉点击)的框架。
//! CDP 协议接入留作后续任务(需引入 CDP client 或复用 Tauri WebView)。
//!
//! ## 用法
//!
//! ```no_run
//! use nebula_lib::browser::{create_agent, BrowserMode, BrowserAgent};
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! // 自动选择:默认 API 模式(零额外依赖)。
//! let agent = create_agent(BrowserMode::Api);
//! let page = agent.navigate("https://example.com").await?;
//! println!("title: {}", page.title);
//! # Ok(()) }
//! ```
//!
//! ## 集成说明
//!
//! Tauri 命令(`browser_navigate` / `browser_click` 等)定义在本模块底部,
//! 遵循 T-E-C-01 OS-Controller 的模式(命令直接定义在功能模块内,而非
//! `commands/` 目录)。这些命令需在 `tauri_setup.rs` 的 `generate_handler!`
//! 中注册后才能被前端调用 — 该注册步骤由后续集成任务完成(本任务受约束
//! 不修改 `tauri_setup.rs` / `commands/mod.rs`)。

pub mod agent;
pub mod api_mode;
pub mod vlm_mode;

// 公共 API re-export — 下游只需 `use nebula_lib::browser::*`。
pub use agent::{
    BrowserAgent, BrowserElement, BrowserError, BrowserMode, BrowserPage, ClickResult,
    ExtractedContent, ScrollDirection, TypeResult,
};
pub use api_mode::ApiBrowserAgent;
pub use vlm_mode::{HttpVisionAnalyzer, VisionAnalyzer, VlmBrowserAgent};

use std::sync::Arc;

/// 工厂:按模式创建 [`BrowserAgent`] 实例。
///
/// * `BrowserMode::Api` — 创建默认 [`ApiBrowserAgent`](SSRF 严格模式)。
/// * `BrowserMode::Vlm` — 创建 [`VlmBrowserAgent`],注入默认 Ollama VLM 后端
///   ([`HttpVisionAnalyzer::default_ollama`])。截图捕获需 `vision` feature。
pub fn create_agent(mode: BrowserMode) -> Arc<dyn BrowserAgent> {
    match mode {
        BrowserMode::Api => Arc::new(ApiBrowserAgent::new()),
        BrowserMode::Vlm => Arc::new(VlmBrowserAgent::with_default_ollama()),
    }
}

// ── Tauri 命令 ───────────────────────────────────────────────────────
//
// 遵循 T-E-C-01 OS-Controller 模式:命令直接定义在功能模块内。
// 这些命令需在 tauri_setup.rs 的 generate_handler! 中注册(后续集成任务)。
// 当前编译为 `#[allow(dead_code)]`(未注册时编译器会警告 dead code)。

/// 导航到指定 URL。`mode` 为 "api" 或 "vlm"。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_navigate(mode: String, url: String) -> Result<BrowserPage, String> {
    let agent = parse_mode(&mode)?;
    agent.navigate(&url).await.map_err(|e| format!("{e:#}"))
}

/// 点击匹配选择器的元素。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_click(mode: String, selector: String) -> Result<ClickResult, String> {
    let agent = parse_mode(&mode)?;
    agent.click(&selector).await.map_err(|e| format!("{e:#}"))
}

/// 在匹配选择器的输入框中填入文本。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_type_text(
    mode: String,
    selector: String,
    text: String,
) -> Result<TypeResult, String> {
    let agent = parse_mode(&mode)?;
    agent
        .type_text(&selector, &text)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// 滚动页面。`direction` 为 "up" 或 "down"。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_scroll(mode: String, direction: String, amount: u32) -> Result<(), String> {
    let agent = parse_mode(&mode)?;
    let dir = match direction.to_lowercase().as_str() {
        "up" => ScrollDirection::Up,
        "down" => ScrollDirection::Down,
        other => {
            return Err(format!(
                "invalid direction: {other} (expected 'up' or 'down')"
            ))
        }
    };
    agent
        .scroll(dir, amount)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// 截图,返回 base64 PNG(仅 VLM 模式可用)。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_screenshot(mode: String) -> Result<String, String> {
    let agent = parse_mode(&mode)?;
    agent.screenshot().await.map_err(|e| format!("{e:#}"))
}

/// 提取匹配选择器的元素。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_extract(mode: String, selector: String) -> Result<ExtractedContent, String> {
    let agent = parse_mode(&mode)?;
    agent.extract(&selector).await.map_err(|e| format!("{e:#}"))
}

/// 返回当前页面快照(不重新请求网络)。
#[tauri::command]
#[allow(dead_code)]
pub async fn browser_current_page(mode: String) -> Result<BrowserPage, String> {
    let agent = parse_mode(&mode)?;
    agent.current_page().await.map_err(|e| format!("{e:#}"))
}

/// 解析模式字符串为 BrowserAgent 实例。
fn parse_mode(mode: &str) -> Result<Arc<dyn BrowserAgent>, String> {
    match mode.to_lowercase().as_str() {
        "api" => Ok(Arc::new(ApiBrowserAgent::new())),
        "vlm" => Ok(Arc::new(VlmBrowserAgent::with_default_ollama())),
        other => Err(format!(
            "invalid browser mode: {other} (expected 'api' or 'vlm')"
        )),
    }
}

// ── 模块级集成测试 ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_agent_api() {
        let agent = create_agent(BrowserMode::Api);
        assert_eq!(agent.mode(), BrowserMode::Api);
    }

    #[test]
    fn test_create_agent_vlm() {
        let agent = create_agent(BrowserMode::Vlm);
        assert_eq!(agent.mode(), BrowserMode::Vlm);
    }

    #[test]
    fn test_parse_mode_api() {
        let agent = parse_mode("api").expect("api mode should parse");
        assert_eq!(agent.mode(), BrowserMode::Api);
    }

    #[test]
    fn test_parse_mode_vlm() {
        let agent = parse_mode("vlm").expect("vlm mode should parse");
        assert_eq!(agent.mode(), BrowserMode::Vlm);
    }

    #[test]
    fn test_parse_mode_case_insensitive() {
        let agent = parse_mode("API").expect("API should parse case-insensitive");
        assert_eq!(agent.mode(), BrowserMode::Api);
    }

    #[test]
    fn test_parse_mode_invalid() {
        assert!(parse_mode("cdp").is_err());
        assert!(parse_mode("").is_err());
    }

    /// 验证两种模式实现同一 trait,可互换使用。
    #[tokio::test]
    async fn test_trait_object_interchangeable() {
        let agents: Vec<Arc<dyn BrowserAgent>> = vec![
            create_agent(BrowserMode::Api),
            create_agent(BrowserMode::Vlm),
        ];
        for (i, agent) in agents.iter().enumerate() {
            // 空 URL 都应返回 InvalidArgument(两种模式一致)。
            let err = agent
                .navigate("")
                .await
                .expect_err("empty URL should be rejected");
            assert!(
                matches!(err, BrowserError::InvalidArgument(_)),
                "agent {i} (mode={}) should reject empty URL with InvalidArgument",
                agent.mode()
            );
        }
    }

    /// 验证 BrowserMode 序列化为小写字符串(供 Tauri 命令参数)。
    #[test]
    fn test_browser_mode_serde() {
        let json = serde_json::to_string(&BrowserMode::Api).unwrap();
        assert_eq!(json, "\"api\"");
        let json = serde_json::to_string(&BrowserMode::Vlm).unwrap();
        assert_eq!(json, "\"vlm\"");

        let mode: BrowserMode = serde_json::from_str("\"vlm\"").unwrap();
        assert_eq!(mode, BrowserMode::Vlm);
    }

    #[test]
    fn test_browser_mode_display() {
        assert_eq!(format!("{}", BrowserMode::Api), "api");
        assert_eq!(format!("{}", BrowserMode::Vlm), "vlm");
    }

    /// 验证 Tauri 命令 parse_mode 逻辑(via 直接调用内部函数)。
    #[tokio::test]
    async fn test_browser_navigate_command_logic() {
        // 直接测试 parse_mode + navigate 的组合(命令本身的薄封装)。
        let agent = parse_mode("api").unwrap();
        let err = agent
            .navigate("not-a-url")
            .await
            .expect_err("invalid URL should fail");
        // SSRF 守卫或网络错误均可(not-a-url 无法解析为合法 URL)。
        assert!(matches!(
            err,
            BrowserError::SsrfBlocked(_)
                | BrowserError::Network(_)
                | BrowserError::InvalidArgument(_)
        ));
    }
}
