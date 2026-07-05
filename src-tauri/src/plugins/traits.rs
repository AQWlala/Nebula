//! T-E-S-35: 5 层插件模型 — trait 定义。
//!
//! 参考 Open WebUI 的 5 层插件模型:
//! - **Filter**:请求/响应过滤(inlet/outlet),如敏感词、ACL、Prompt 注入
//! - **Action**:一次性副作用动作,UI 按钮触发,如发通知
//! - **Pipe**:同步处理管道,多步转换、上下文注入
//! - **Tool**:LLM function-calling 工具(直接 re-export `crate::tools::Tool`)
//! - **Skill**:可组合能力包,可导入、共享、评分、市场化
//!
//! 设计约束:
//! - Tool 层零改动:`pub use crate::tools::Tool;`
//! - Skill 层用 bridge 适配既有 SkillEngine,不改 SkillEngine 本身
//! - Filter 用 async + 两阶段,`FilterVerdict` 枚举 `{ Allow, Modify(T), Reject(String) }`
//! - Pipe 优先同步(P0),流式留 P1
//! - Action 与 Agent 解耦:一次性、无状态

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// Tool 层零改动:直接 re-export 既有 trait + 注册表 + 输入输出。
pub use crate::tools::{Tool, ToolInput, ToolOutput, ToolRegistry};

// Skill 数据记录(SkillEngine 返回的 DTO)。重命名以避免与本模块
// 定义的 `Skill` trait 冲突。
pub use crate::skills::Skill as SkillRecord;
pub use crate::skills::SkillResult;

// ---------------------------------------------------------------------------
// Filter 层
// ---------------------------------------------------------------------------

/// Filter 对请求/响应的裁决。
///
/// - `Allow` — 放行,内容不变
/// - `Modify(T)` — 替换内容后放行
/// - `Reject(String)` — 拒绝,附带理由(供 UI/日志展示)
#[derive(Debug, Clone)]
pub enum FilterVerdict<T> {
    Allow,
    Modify(T),
    Reject(String),
}

impl<T> FilterVerdict<T> {
    pub fn is_allow(&self) -> bool {
        matches!(self, FilterVerdict::Allow)
    }

    pub fn is_reject(&self) -> bool {
        matches!(self, FilterVerdict::Reject(_))
    }
}

/// 进入 LLM 前的请求载体(inlet)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRequest {
    pub prompt: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

impl FilterRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            user_id: None,
            session_id: None,
        }
    }
}

/// LLM 返回后的响应载体(outlet)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterResponse {
    pub content: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

impl FilterResponse {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            user_id: None,
            session_id: None,
        }
    }
}

/// Filter trait:在 LLM 调用前后对内容做安全/合规过滤。
///
/// 两阶段设计:`filter_request` 处理用户输入(inlet),
/// `filter_response` 处理 LLM 输出(outlet)。方法取引用,链式执行时
/// 无需 clone 原始载体(仅 `Modify` 分支构造新值)。
#[async_trait]
pub trait Filter: Send + Sync {
    fn name(&self) -> &str;
    async fn filter_request(&self, request: &FilterRequest) -> FilterVerdict<FilterRequest>;
    async fn filter_response(&self, response: &FilterResponse) -> FilterVerdict<FilterResponse>;
}

// ---------------------------------------------------------------------------
// Action 层
// ---------------------------------------------------------------------------

/// Action 调用结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutput {
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

impl ActionOutput {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            data: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            data: None,
        }
    }
}

/// Action trait:一次性、无状态的副作用动作(UI 按钮触发)。
///
/// 与 Agent 解耦:不持有 TeamContext,不参与多轮对话,不维护内部状态。
#[async_trait]
pub trait Action: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn invoke(&self, params: serde_json::Value) -> Result<ActionOutput>;
}

// ---------------------------------------------------------------------------
// Pipe 层
// ---------------------------------------------------------------------------

/// Pipe 输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipeInput {
    pub prompt: String,
    #[serde(default)]
    pub context: Option<String>,
}

impl PipeInput {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            context: None,
        }
    }
}

/// Pipe 输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipeOutput {
    pub prompt: String,
    #[serde(default)]
    pub context: Option<String>,
}

impl PipeOutput {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            context: None,
        }
    }
}

/// Pipe trait:同步转换管道(P0;P1 再考虑流式 Stream)。
///
/// 多个 Pipe 串联执行,前一个的输出作为后一个的输入。
pub trait Pipe: Send + Sync {
    fn name(&self) -> &str;
    fn transform(&self, input: PipeInput) -> Result<PipeOutput>;
}

// ---------------------------------------------------------------------------
// Skill 层
// ---------------------------------------------------------------------------

/// Skill trait:可组合能力包,方法对齐 SkillEngine 公共 API。
///
/// 通过 bridge 模式适配既有 SkillEngine,不改 SkillEngine 本身。
#[async_trait]
pub trait Skill: Send + Sync {
    fn name(&self) -> &str;
    async fn use_skill(&self, id: &str, params: HashMap<String, String>) -> Result<SkillResult>;
    async fn list_skills(
        &self,
        language: Option<String>,
        tag: Option<String>,
        limit: u32,
    ) -> Result<Vec<SkillRecord>>;
    async fn search_skills(&self, query: &str, limit: u32) -> Result<Vec<SkillRecord>>;
}

// ---------------------------------------------------------------------------
// 便利类型别名(dyn trait 对象)
// ---------------------------------------------------------------------------

pub type DynFilter = Arc<dyn Filter>;
pub type DynAction = Arc<dyn Action>;
pub type DynPipe = Arc<dyn Pipe>;
pub type DynSkill = Arc<dyn Skill>;

#[cfg(test)]
mod tests {
    use super::*;

    /// FilterVerdict 基本语义。
    #[test]
    fn filter_verdict_predicates() {
        let allow: FilterVerdict<i32> = FilterVerdict::Allow;
        let modify: FilterVerdict<i32> = FilterVerdict::Modify(42);
        let reject: FilterVerdict<i32> = FilterVerdict::Reject("nope".into());

        assert!(allow.is_allow());
        assert!(!allow.is_reject());

        assert!(!modify.is_allow());
        assert!(!modify.is_reject());
        if let FilterVerdict::Modify(v) = modify {
            assert_eq!(v, 42);
        }

        assert!(!reject.is_allow());
        assert!(reject.is_reject());
    }

    /// 载体结构构造。
    #[test]
    fn request_response_constructors() {
        let req = FilterRequest::new("hello");
        assert_eq!(req.prompt, "hello");
        assert!(req.user_id.is_none());

        let resp = FilterResponse::new("world");
        assert_eq!(resp.content, "world");
    }

    /// ActionOutput 工厂。
    #[test]
    fn action_output_factories() {
        let ok = ActionOutput::ok("done");
        assert!(ok.success);
        assert_eq!(ok.message, "done");

        let err = ActionOutput::err("boom");
        assert!(!err.success);
    }

    /// PipeInput/Output 构造。
    #[test]
    fn pipe_io_constructors() {
        let i = PipeInput::new("q");
        assert_eq!(i.prompt, "q");
        assert!(i.context.is_none());

        let o = PipeOutput::new("a");
        assert_eq!(o.prompt, "a");
    }

    /// 一个最小 Filter 实现,验证 trait 可被实例化与调用。
    struct AllowAllFilter;
    #[async_trait]
    impl Filter for AllowAllFilter {
        fn name(&self) -> &str {
            "allow_all"
        }
        async fn filter_request(&self, _req: &FilterRequest) -> FilterVerdict<FilterRequest> {
            FilterVerdict::Allow
        }
        async fn filter_response(&self, _resp: &FilterResponse) -> FilterVerdict<FilterResponse> {
            FilterVerdict::Allow
        }
    }

    #[tokio::test]
    async fn dummy_filter_allows() {
        let f = AllowAllFilter;
        let req = FilterRequest::new("hi");
        let v = f.filter_request(&req).await;
        assert!(v.is_allow());
    }
}
