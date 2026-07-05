//! T-E-S-35: 5 层插件模型 — Filter / Action / Pipe / Tool / Skill。
//!
//! 参考 Open WebUI 的 5 层插件模型,把Nebula既有的散落实现统一到一套
//! trait + 注册表之下:
//!
//! - **Filter**:请求/响应过滤(inlet/outlet)。既有实现散落在
//!   `security/injection_guard`、`ssrf_guard`、`memory/acl` 等;本模块
//!   定义统一 `Filter` trait,并提供 `InjectionFilter` 示例包装
//!   `security::injection_guard::full_injection_scan`。
//! - **Action**:一次性副作用动作,UI 按钮触发。无现成实现,本模块新建
//!   `NotifyAction` 示例包装 `os::notifications::send`。
//! - **Pipe**:同步处理管道,多步转换、上下文注入。既有实现散落在
//!   `semantic_cache`、`gateway 降级`、`assemble_context` 等;本模块
//!   定义统一 `Pipe` trait(P0 同步),并提供 `ContextInjectionPipe` 示例。
//! - **Tool**:LLM function-calling 工具。`tools::Tool` + `ToolRegistry`
//!   已完整,直接 re-export,零改动。
//! - **Skill**:可组合能力包。`skills::SkillEngine` 已完整,用 bridge
//!   模式适配:`SkillEngineBridge(Arc<SkillEngine>)` 实现 `plugins::Skill`,
//!   不改 SkillEngine 本身。
//!
//! P0 不接入实际 chat 路径(Filter/Pipe 的实际调用链由后续任务完成),
//! 不实现流式 Pipe 协议(P0 用同步 transform),不实现跨进程插件加载。

pub mod actions;
pub mod filters;
pub mod pipes;
pub mod registry;
pub mod skill_bridge;
pub mod traits;

// 5 trait 顶层导出。
pub use traits::{
    Action, ActionOutput, DynAction, DynFilter, DynPipe, DynSkill, Filter, FilterRequest,
    FilterResponse, FilterVerdict, Pipe, PipeInput, PipeOutput, Skill, SkillRecord, SkillResult,
    Tool, ToolInput, ToolOutput, ToolRegistry,
};

// 注册表顶层导出。
pub use registry::{ActionRegistry, FilterChain, PipeChain, PluginRegistry, SkillRegistry};

// 示例实现顶层导出。
pub use actions::NotifyAction;
pub use filters::InjectionFilter;
pub use pipes::ContextInjectionPipe;
pub use skill_bridge::SkillEngineBridge;
