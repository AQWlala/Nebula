//! T-E-C-13: 工作场景模板库 — 预置 Writer/Coder/Manager 三套场景角色模板 + 工作流模板。
//!
//! 模板存放在 `src-tauri/templates/scenarios.json`,通过 `include_str!`
//! 在编译时嵌入二进制;启动时 `serde_json::from_str` 解析为
//! [`TemplateEngine`]。前端通过 `scenario_list` / `scenario_get` /
//! `scenario_instantiate` 命令查询并一键启动 Swarm。
//!
//! ## 设计要点
//! * **零新依赖**:纯 serde_json + 既有 [`SwarmTask`]。
//! * **静态嵌入**:`include_str!("../../templates/scenarios.json")` 编译时嵌入,
//!   启动时 `serde_json::from_str` 解析一次,运行时只读。
//! * **instantiate**:把 `user_input` 填入 `user_prompt_template`,
//!   返回可传给 `SwarmOrchestrator::execute` 的 [`SwarmTask`]。
//!
//! [`SwarmTask`]: crate::swarm::orchestrator::SwarmTask

use serde::{Deserialize, Serialize};

use crate::swarm::agents::AgentKind;
use crate::swarm::orchestrator::SwarmTask;

// ---------------------------------------------------------------------------
// 枚举与数据结构
// ---------------------------------------------------------------------------

/// 模板分类(前端按此分组渲染)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioCategory {
    /// 写作类:博客 / 文档 / 发布说明 / 教程 等。
    Writing,
    /// 编码类:PR 审查 / 重构 / 测试 / 架构 等。
    Coding,
    /// 管理类:周报 / 站会 / 回顾 / OKR 等。
    Management,
}

/// 顶层角色(模板对应的"人格基底")。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioRole {
    /// 写手基底(writer-base 及 writing 类工作流)。
    Writer,
    /// 编码基底(coder-base 及 coding 类工作流)。
    Coder,
    /// 管理基底(manager-base 及 management 类工作流,用 planner+researcher)。
    Manager,
}

/// 单个 agent 规格(传给 [`SwarmOrchestrator`] 的 agent 种类 + 角色标签)。
///
/// [`SwarmOrchestrator`]: crate::swarm::orchestrator::SwarmOrchestrator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    /// agent 种类(序列化为 lowercase:coder/writer/reviewer/researcher/planner/generic)。
    pub kind: AgentKind,
    /// 该 agent 在本场景中的角色标签(如 "primary" / "checker" / "drafter")。
    pub role: String,
    /// 可选的 prompt 覆盖(覆盖角色 agent 默认 system_prompt)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_override: Option<String>,
}

/// 一个工作场景模板。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioTemplate {
    /// 稳定 id(如 "tech-blog" / "writer-base")。
    pub id: String,
    /// 中文显示名。
    pub name: String,
    /// 一句话描述。
    pub description: String,
    /// 分类(前端按此分组)。
    pub category: ScenarioCategory,
    /// 顶层角色。
    pub role: ScenarioRole,
    /// agent 规格列表。
    pub agents: Vec<AgentSpec>,
    /// 系统提示(注入到 SwarmTask.description 前缀)。
    pub system_prompt: String,
    /// 用户提示模板,含 `{{user_input}}` 占位符。
    pub user_prompt_template: String,
    /// 标签(供前端搜索/筛选)。
    #[serde(default)]
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// TemplateEngine
// ---------------------------------------------------------------------------

/// 静态模板引擎 — 编译时嵌入 `scenarios.json`,启动时解析一次。
///
/// 用 [`TemplateEngine::load`] 构造;之后所有查询都是纯内存只读操作。
pub struct TemplateEngine {
    templates: Vec<ScenarioTemplate>,
}

impl TemplateEngine {
    /// 从编译时嵌入的 JSON 加载所有模板。
    ///
    /// 解析失败(数据损坏)返回 `anyhow::Error`;正常情况下编译时已
    /// 校验过 JSON 合法性,运行时不会失败。
    pub fn load() -> anyhow::Result<Self> {
        // 编译时嵌入,运行时零 IO。
        let json = include_str!("../../templates/scenarios.json");
        let templates: Vec<ScenarioTemplate> = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("failed to parse scenarios.json: {e}"))?;
        Ok(Self { templates })
    }

    /// 返回全部模板(按 JSON 中的顺序)。
    pub fn list(&self) -> &[ScenarioTemplate] {
        &self.templates
    }

    /// 按分类过滤模板。
    pub fn list_by_category(&self, category: ScenarioCategory) -> Vec<&ScenarioTemplate> {
        self.templates
            .iter()
            .filter(|t| t.category == category)
            .collect()
    }

    /// 按 id 查找模板。
    pub fn get_by_id(&self, id: &str) -> Option<&ScenarioTemplate> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// 实例化模板:把 `user_input` 填入 `user_prompt_template`,
    /// 返回可传给 `SwarmOrchestrator::execute` 的 [`SwarmTask`]。
    ///
    /// `user_prompt_template` 中 `{{user_input}}` 占位符被替换为用户输入;
    /// system_prompt 拼接到 description 前缀,作为 task 上下文。
    /// 模板的 agent kind 列表写入 `SwarmTask.agents`,orchestrator 会
    /// 优先使用显式 agents 而非默认 agent_count。
    ///
    /// 返回 `None` 表示 `id` 不存在。
    pub fn instantiate(&self, id: &str, user_input: &str) -> Option<SwarmTask> {
        let tpl = self.get_by_id(id)?;
        let agent_kinds: Vec<String> = tpl
            .agents
            .iter()
            .map(|a| a.kind.as_str().to_string())
            .collect();
        let user_prompt = tpl
            .user_prompt_template
            .replace("{{user_input}}", user_input);
        let description = format!(
            "[场景模板:{}] {}\n\n系统提示:\n{}\n\n用户输入:\n{}",
            tpl.id, tpl.name, tpl.system_prompt, user_prompt
        );
        let mut task = SwarmTask::new(description);
        if !agent_kinds.is_empty() {
            task.agents = agent_kinds;
        }
        Some(task)
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
// T-D-B-17: 测试验证废弃 AgentKind::Planner 的反序列化向后兼容,放行废弃警告。
#[allow(deprecated)]
mod tests {
    use super::*;

    /// 加载 TemplateEngine 并验证模板数量。
    ///
    /// 注:任务描述 headline 写 "25 个模板(3 顶层 + 22 工作流)",
    /// 但子条目显式列举了 8+9+8=25 个工作流 + 3 个顶层角色 = 28。
    /// 本实现按显式枚举落地,共 28 个。
    #[test]
    fn engine_loads_all_templates() {
        let engine = TemplateEngine::load().expect("scenarios.json must parse");
        assert_eq!(
            engine.templates.len(),
            28,
            "expected 28 templates (3 base + 25 workflows), got {}",
            engine.templates.len()
        );
    }

    /// ScenarioTemplate 序列化/反序列化往返。
    #[test]
    fn scenario_template_roundtrip_serde() {
        let engine = TemplateEngine::load().expect("load");
        let tech_blog = engine
            .get_by_id("tech-blog")
            .expect("tech-blog template exists");
        let json = serde_json::to_string(tech_blog).expect("serialize");
        let parsed: ScenarioTemplate = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.id, "tech-blog");
        assert_eq!(parsed.category, ScenarioCategory::Writing);
        assert_eq!(parsed.role, ScenarioRole::Writer);
        assert!(!parsed.agents.is_empty());
        assert!(parsed.user_prompt_template.contains("{{user_input}}"));
    }

    /// list_by_category 各分类数量正确。
    #[test]
    fn list_by_category_returns_correct_counts() {
        let engine = TemplateEngine::load().expect("load");
        // Writing: 8 工作流 + writer-base = 9
        let writing = engine.list_by_category(ScenarioCategory::Writing);
        assert_eq!(writing.len(), 9, "writing category should have 9 templates");
        // Coding: 9 工作流 + coder-base = 10
        let coding = engine.list_by_category(ScenarioCategory::Coding);
        assert_eq!(coding.len(), 10, "coding category should have 10 templates");
        // Management: 8 工作流 + manager-base = 9
        let management = engine.list_by_category(ScenarioCategory::Management);
        assert_eq!(
            management.len(),
            9,
            "management category should have 9 templates"
        );
        // 总数 = 9 + 10 + 9 = 28
        assert_eq!(writing.len() + coding.len() + management.len(), 28);
    }

    /// get_by_id 命中 / 未命中。
    #[test]
    fn get_by_id_hit_and_miss() {
        let engine = TemplateEngine::load().expect("load");
        assert!(engine.get_by_id("tech-blog").is_some());
        assert!(engine.get_by_id("writer-base").is_some());
        assert!(engine.get_by_id("manager-base").is_some());
        assert!(engine.get_by_id("nonexistent-id").is_none());
        assert!(engine.get_by_id("").is_none());
    }

    /// instantiate 返回合法 SwarmTask:agents 列表非空 + description 含用户输入。
    #[test]
    fn instantiate_returns_valid_swarm_task() {
        let engine = TemplateEngine::load().expect("load");
        let task = engine
            .instantiate("tech-blog", "写一篇关于 Rust 异步运行时的文章")
            .expect("instantiate tech-blog");
        assert!(!task.description.is_empty());
        assert!(!task.agents.is_empty(), "agents list must be non-empty");
        // description 应包含模板 id 与用户输入。
        assert!(task.description.contains("tech-blog"));
        assert!(task.description.contains("Rust 异步运行时"));
        // tech-blog 模板 agents 含 writer + reviewer。
        assert!(task.agents.contains(&"writer".to_string()));
        assert!(task.agents.contains(&"reviewer".to_string()));
    }

    /// instantiate 对未知 id 返回 None。
    #[test]
    fn instantiate_returns_none_for_unknown_id() {
        let engine = TemplateEngine::load().expect("load");
        assert!(engine.instantiate("does-not-exist", "hello").is_none());
    }

    /// 三个顶层角色模板含正确 agents 组合;manager-base 含 planner + researcher。
    #[test]
    fn base_role_templates_have_correct_agent_combos() {
        let engine = TemplateEngine::load().expect("load");

        // writer-base:含 writer + reviewer。
        let writer_base = engine.get_by_id("writer-base").expect("writer-base");
        let kinds: Vec<&str> = writer_base.agents.iter().map(|a| a.kind.as_str()).collect();
        assert!(
            kinds.contains(&"writer"),
            "writer-base must contain a writer agent"
        );
        assert_eq!(writer_base.role, ScenarioRole::Writer);

        // coder-base:含 coder + reviewer。
        let coder_base = engine.get_by_id("coder-base").expect("coder-base");
        let kinds: Vec<&str> = coder_base.agents.iter().map(|a| a.kind.as_str()).collect();
        assert!(
            kinds.contains(&"coder"),
            "coder-base must contain a coder agent"
        );
        assert_eq!(coder_base.role, ScenarioRole::Coder);

        // manager-base:含 planner + researcher(spec 强制要求)。
        let manager_base = engine.get_by_id("manager-base").expect("manager-base");
        let kinds: Vec<&str> = manager_base
            .agents
            .iter()
            .map(|a| a.kind.as_str())
            .collect();
        assert!(
            kinds.contains(&"planner"),
            "manager-base must contain a planner agent"
        );
        assert!(
            kinds.contains(&"researcher"),
            "manager-base must contain a researcher agent"
        );
        assert_eq!(manager_base.role, ScenarioRole::Manager);
    }

    /// 所有模板的必填字段完整(id/name/description/agents/system_prompt/user_prompt_template)。
    #[test]
    fn all_templates_have_required_fields() {
        let engine = TemplateEngine::load().expect("load");
        for t in engine.list() {
            assert!(!t.id.is_empty(), "template id empty");
            assert!(!t.name.is_empty(), "template {} name empty", t.id);
            assert!(
                !t.description.is_empty(),
                "template {} description empty",
                t.id
            );
            assert!(!t.agents.is_empty(), "template {} agents empty", t.id);
            assert!(
                !t.system_prompt.is_empty(),
                "template {} system_prompt empty",
                t.id
            );
            assert!(
                !t.user_prompt_template.is_empty(),
                "template {} user_prompt_template empty",
                t.id
            );
            // user_prompt_template 必须含 {{user_input}} 占位符。
            assert!(
                t.user_prompt_template.contains("{{user_input}}"),
                "template {} user_prompt_template missing {{{{user_input}}}} placeholder",
                t.id
            );
        }
    }

    /// AgentSpec.kind 反序列化验证(确认 AgentKind 的 lowercase 序列化与 JSON 一致)。
    #[test]
    fn agent_spec_kind_deserializes_from_lowercase() {
        let json = r#"{"kind":"planner","role":"facilitator"}"#;
        let spec: AgentSpec = serde_json::from_str(json).expect("deserialize AgentSpec");
        assert_eq!(spec.kind, AgentKind::Planner);
        assert_eq!(spec.role, "facilitator");
        assert!(spec.prompt_override.is_none());
    }

    /// 模板 id 唯一(无重复)。
    #[test]
    fn template_ids_are_unique() {
        let engine = TemplateEngine::load().expect("load");
        let mut ids: Vec<&str> = engine.list().iter().map(|t| t.id.as_str()).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate template ids detected");
    }
}
