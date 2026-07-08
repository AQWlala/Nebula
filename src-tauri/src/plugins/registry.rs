//! T-E-S-35: PluginRegistry + 5 个子注册表。
//!
//! 5 层:
//! - `FilterChain` — 有序 filter 链,两阶段(request/response)
//! - `ActionRegistry` — 按名查找的 action 表
//! - `PipeChain` — 有序 pipe 链(持有 `Arc<ToolRegistry>` 供需要查 tool 的 pipe 使用)
//! - `SkillRegistry` — 按名查找的 skill 插件表
//! - `ToolRegistry` — 直接复用 `crate::tools::ToolRegistry`(第 5 层,re-export)
//!
//! `PluginRegistry` 聚合上述 4 个子注册表 + `Arc<ToolRegistry>`,
//! 提供 register_* 编排入口与 apply_filters_*/run_pipe/invoke_action 编排方法。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use tracing::{info, warn};

use super::traits::{
    ActionOutput, DynAction, DynFilter, DynPipe, DynSkill, FilterRequest, FilterResponse,
    FilterVerdict, PipeInput, PipeOutput, SkillRecord, SkillResult, ToolRegistry,
};

// ---------------------------------------------------------------------------
// FilterChain
// ---------------------------------------------------------------------------

/// Filter 子注册表:有序链,按注册顺序执行。
pub struct FilterChain {
    filters: RwLock<Vec<DynFilter>>,
}

impl Default for FilterChain {
    fn default() -> Self {
        Self {
            filters: RwLock::new(Vec::new()),
        }
    }
}

impl FilterChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, filter: DynFilter) {
        let name = filter.name().to_string();
        self.filters.write().push(filter);
        info!(target: "nebula.plugins", filter = %name, "filter registered");
    }

    /// 请求阶段:按顺序应用所有 filter。任一 `Reject` 则短路返回 Err;
    /// 任一 `Modify` 则替换内容后继续;`Allow` 继续。
    pub async fn apply_request(&self, mut request: FilterRequest) -> Result<FilterRequest> {
        let filters: Vec<DynFilter> = self.filters.read().clone();
        for f in filters {
            let name = f.name().to_string();
            match f.filter_request(&request).await {
                FilterVerdict::Allow => {}
                FilterVerdict::Modify(new_req) => {
                    request = new_req;
                }
                FilterVerdict::Reject(reason) => {
                    warn!(
                        target: "nebula.plugins",
                        filter = %name, reason = %reason, "request rejected"
                    );
                    return Err(anyhow!("filter {} rejected request: {}", name, reason));
                }
            }
        }
        Ok(request)
    }

    /// 响应阶段:同上。
    pub async fn apply_response(&self, mut response: FilterResponse) -> Result<FilterResponse> {
        let filters: Vec<DynFilter> = self.filters.read().clone();
        for f in filters {
            let name = f.name().to_string();
            match f.filter_response(&response).await {
                FilterVerdict::Allow => {}
                FilterVerdict::Modify(new_resp) => {
                    response = new_resp;
                }
                FilterVerdict::Reject(reason) => {
                    warn!(
                        target: "nebula.plugins",
                        filter = %name, reason = %reason, "response rejected"
                    );
                    return Err(anyhow!("filter {} rejected response: {}", name, reason));
                }
            }
        }
        Ok(response)
    }

    pub fn len(&self) -> usize {
        self.filters.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.filters.read().is_empty()
    }
}

// ---------------------------------------------------------------------------
// ActionRegistry
// ---------------------------------------------------------------------------

/// Action 子注册表:按名查找,后注册覆盖先注册。
pub struct ActionRegistry {
    actions: RwLock<HashMap<String, DynAction>>,
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self {
            actions: RwLock::new(HashMap::new()),
        }
    }
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, action: DynAction) {
        let name = action.name().to_string();
        self.actions.write().insert(name.clone(), action);
        info!(target: "nebula.plugins", action = %name, "action registered");
    }

    pub async fn invoke(&self, name: &str, params: serde_json::Value) -> Result<ActionOutput> {
        let action = self
            .actions
            .read()
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("unknown action: {}", name))?;
        action.invoke(params).await
    }

    pub fn names(&self) -> Vec<String> {
        self.actions.read().keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// PipeChain
// ---------------------------------------------------------------------------

/// Pipe 子注册表:有序链。持有 `Arc<ToolRegistry>` 供需要查 tool 的
/// pipe 实现(如路由、缓存短路)使用。P0 示例 pipe 不依赖它。
pub struct PipeChain {
    pipes: RwLock<Vec<DynPipe>>,
    tools: Arc<ToolRegistry>,
}

impl PipeChain {
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self {
            pipes: RwLock::new(Vec::new()),
            tools,
        }
    }

    pub fn register(&self, pipe: DynPipe) {
        let name = pipe.name().to_string();
        self.pipes.write().push(pipe);
        info!(target: "nebula.plugins", pipe = %name, "pipe registered");
    }

    /// 串联执行所有 pipe,前一个输出作为后一个输入。
    pub fn run_pipe(&self, input: PipeInput) -> Result<PipeOutput> {
        let pipes: Vec<DynPipe> = self.pipes.read().clone();
        if pipes.is_empty() {
            return Ok(PipeOutput {
                prompt: input.prompt,
                context: input.context,
            });
        }
        let mut current_input = input;
        let mut output = PipeOutput {
            prompt: String::new(),
            context: None,
        };
        for p in pipes {
            output = p.transform(current_input)?;
            current_input = PipeInput {
                prompt: output.prompt.clone(),
                context: output.context.clone(),
            };
        }
        Ok(output)
    }

    /// 共享的 ToolRegistry 句柄(第 5 层)。
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    pub fn len(&self) -> usize {
        self.pipes.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.pipes.read().is_empty()
    }
}

// ---------------------------------------------------------------------------
// SkillRegistry
// ---------------------------------------------------------------------------

/// Skill 子注册表:按插件名查找。每个 skill 插件本身是一个能力引擎
/// (如 `SkillEngineBridge`),可执行 use_skill/list_skills/search_skills。
pub struct SkillRegistry {
    skills: RwLock<HashMap<String, DynSkill>>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self {
            skills: RwLock::new(HashMap::new()),
        }
    }
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 按 `skill.name()` 注册。后注册覆盖先注册。
    pub fn register(&self, skill: DynSkill) {
        let name = skill.name().to_string();
        self.skills.write().insert(name.clone(), skill);
        info!(target: "nebula.plugins", skill = %name, "skill plugin registered");
    }

    pub fn get(&self, name: &str) -> Option<DynSkill> {
        self.skills.read().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.skills.read().keys().cloned().collect()
    }

    /// 便捷:通过指定插件执行 skill。
    pub async fn use_skill(
        &self,
        plugin_name: &str,
        id: &str,
        params: HashMap<String, String>,
    ) -> Result<SkillResult> {
        let skill = self
            .get(plugin_name)
            .ok_or_else(|| anyhow!("unknown skill plugin: {}", plugin_name))?;
        skill.use_skill(id, params).await
    }

    /// 便捷:通过指定插件列出 skills。
    pub async fn list_skills(
        &self,
        plugin_name: &str,
        language: Option<String>,
        tag: Option<String>,
        limit: u32,
    ) -> Result<Vec<SkillRecord>> {
        let skill = self
            .get(plugin_name)
            .ok_or_else(|| anyhow!("unknown skill plugin: {}", plugin_name))?;
        skill.list_skills(language, tag, limit).await
    }

    /// 便捷:通过指定插件搜索 skills。
    pub async fn search_skills(
        &self,
        plugin_name: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SkillRecord>> {
        let skill = self
            .get(plugin_name)
            .ok_or_else(|| anyhow!("unknown skill plugin: {}", plugin_name))?;
        skill.search_skills(query, limit).await
    }
}

// ---------------------------------------------------------------------------
// PluginRegistry
// ---------------------------------------------------------------------------

/// 插件总注册表:聚合 4 个子注册表 + `Arc<ToolRegistry>`(第 5 层)。
///
/// 编排方法:`apply_filters_request` / `apply_filters_response` / `run_pipe`
/// / `invoke_action`。Skill 层通过 `skill(name)` 取得插件后调用其方法。
pub struct PluginRegistry {
    pub filters: FilterChain,
    pub actions: ActionRegistry,
    pub pipes: PipeChain,
    pub skills: SkillRegistry,
}

impl PluginRegistry {
    /// 用已有的 `ToolRegistry` 构造(共享同一 Arc,避免重复注册 tool)。
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self {
            filters: FilterChain::new(),
            actions: ActionRegistry::new(),
            pipes: PipeChain::new(tools.clone()),
            skills: SkillRegistry::new(),
        }
    }

    // -- 注册入口 --

    pub fn register_filter(&self, filter: DynFilter) {
        self.filters.register(filter);
    }

    pub fn register_action(&self, action: DynAction) {
        self.actions.register(action);
    }

    pub fn register_pipe(&self, pipe: DynPipe) {
        self.pipes.register(pipe);
    }

    pub fn register_skill(&self, skill: DynSkill) {
        self.skills.register(skill);
    }

    // -- 编排方法 --

    pub async fn apply_filters_request(&self, request: FilterRequest) -> Result<FilterRequest> {
        self.filters.apply_request(request).await
    }

    pub async fn apply_filters_response(&self, response: FilterResponse) -> Result<FilterResponse> {
        self.filters.apply_response(response).await
    }

    pub fn run_pipe(&self, input: PipeInput) -> Result<PipeOutput> {
        self.pipes.run_pipe(input)
    }

    pub async fn invoke_action(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> Result<ActionOutput> {
        self.actions.invoke(name, params).await
    }

    // -- 访问器 --

    pub fn tools(&self) -> &Arc<ToolRegistry> {
        self.pipes.tools()
    }

    pub fn skill(&self, name: &str) -> Option<DynSkill> {
        self.skills.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::traits::{
        Action, Filter, FilterRequest, FilterResponse, FilterVerdict, Pipe, PipeInput,
    };

    // ---- Filter chain ----

    struct AllowAll;
    #[async_trait::async_trait]
    impl Filter for AllowAll {
        fn name(&self) -> &str {
            "allow_all"
        }
        async fn filter_request(&self, _r: &FilterRequest) -> FilterVerdict<FilterRequest> {
            FilterVerdict::Allow
        }
        async fn filter_response(&self, _r: &FilterResponse) -> FilterVerdict<FilterResponse> {
            FilterVerdict::Allow
        }
    }

    struct PrefixModifier;
    #[async_trait::async_trait]
    impl Filter for PrefixModifier {
        fn name(&self) -> &str {
            "prefix"
        }
        async fn filter_request(&self, r: &FilterRequest) -> FilterVerdict<FilterRequest> {
            FilterVerdict::Modify(FilterRequest::new(format!("[mod] {}", r.prompt)))
        }
        async fn filter_response(&self, _r: &FilterResponse) -> FilterVerdict<FilterResponse> {
            FilterVerdict::Allow
        }
    }

    struct RejectAll;
    #[async_trait::async_trait]
    impl Filter for RejectAll {
        fn name(&self) -> &str {
            "reject_all"
        }
        async fn filter_request(&self, _r: &FilterRequest) -> FilterVerdict<FilterRequest> {
            FilterVerdict::Reject("blocked".into())
        }
        async fn filter_response(&self, _r: &FilterResponse) -> FilterVerdict<FilterResponse> {
            FilterVerdict::Reject("blocked".into())
        }
    }

    #[tokio::test]
    async fn filter_chain_allow_passes_through() {
        let chain = FilterChain::new();
        chain.register(Arc::new(AllowAll));
        let req = FilterRequest::new("hi");
        let out = chain.apply_request(req).await.expect("task should complete");
        assert_eq!(out.prompt, "hi");
    }

    #[tokio::test]
    async fn filter_chain_modify_replaces_content() {
        let chain = FilterChain::new();
        chain.register(Arc::new(PrefixModifier));
        let req = FilterRequest::new("hi");
        let out = chain.apply_request(req).await.expect("task should complete");
        assert_eq!(out.prompt, "[mod] hi");
    }

    #[tokio::test]
    async fn filter_chain_reject_short_circuits() {
        let chain = FilterChain::new();
        chain.register(Arc::new(AllowAll));
        chain.register(Arc::new(RejectAll));
        let err = chain
            .apply_request(FilterRequest::new("hi"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("reject_all"));
    }

    // ---- Action registry ----

    struct EchoAction;
    #[async_trait::async_trait]
    impl Action for EchoAction {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes the params"
        }
        async fn invoke(&self, params: serde_json::Value) -> Result<ActionOutput> {
            Ok(ActionOutput {
                success: true,
                message: "ok".into(),
                data: Some(params),
            })
        }
    }

    #[tokio::test]
    async fn action_registry_invoke() {
        let reg = ActionRegistry::new();
        reg.register(Arc::new(EchoAction));
        let out = reg
            .invoke("echo", serde_json::json!({"x": 1}))
            .await
            .expect("test op should succeed");
        assert!(out.success);
        assert_eq!(out.data.expect("assertion value")["x"], 1);
    }

    #[tokio::test]
    async fn action_registry_unknown_returns_err() {
        let reg = ActionRegistry::new();
        let err = reg.invoke("nope", serde_json::json!({})).await.unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    // ---- Pipe chain ----

    struct UppercasePipe;
    impl Pipe for UppercasePipe {
        fn name(&self) -> &str {
            "uppercase"
        }
        fn transform(&self, input: PipeInput) -> Result<PipeOutput> {
            Ok(PipeOutput {
                prompt: input.prompt.to_uppercase(),
                context: input.context,
            })
        }
    }

    #[test]
    fn pipe_chain_runs_in_order() {
        let tools = Arc::new(ToolRegistry::new());
        let chain = PipeChain::new(tools);
        chain.register(Arc::new(UppercasePipe));
        let out = chain.run_pipe(PipeInput::new("hello")).expect("create should succeed");
        assert_eq!(out.prompt, "HELLO");
    }

    // ---- PluginRegistry 整体 ----

    #[tokio::test]
    async fn plugin_registry_orchestrates_filter_and_action() {
        let tools = Arc::new(ToolRegistry::new());
        let reg = PluginRegistry::new(tools);
        reg.register_filter(Arc::new(AllowAll));
        reg.register_action(Arc::new(EchoAction));

        let req = reg
            .apply_filters_request(FilterRequest::new("hi"))
            .await
            .expect("test op should succeed");
        assert_eq!(req.prompt, "hi");

        let out = reg
            .invoke_action("echo", serde_json::json!({}))
            .await
            .expect("test op should succeed");
        assert!(out.success);
    }
}
