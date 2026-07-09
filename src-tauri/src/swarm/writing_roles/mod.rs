//! T-EA-AE-06: 子智能体重定义 — 写作场景 6 个子智能体角色。
//!
//! 本模块为写作场景(Novel / 技术博客 / 学术 / 营销 / 邮件等)重新定义
//! 6 个专业子智能体角色,每个角色有独立的能力定义、提示词模板、输出
//! 规范与质量标准。与 `swarm/agents` 中编程场景的 5 角色coder/
//! writer/reviewer/researcher/planner)平行,不修改任何现有角色文件。
//!
//! ## 6 个写作角色
//!
//! | ID            | 中文名     | 职责                                   |
//! |---------------|-----------|----------------------------------------|
//! | `Outliner`    | 大纲师     | 构建故事/文章骨架,分解章节节拍          |
//! | `Drafter`     | 起草师     | 快速生成初稿,优先速度而非打磨           |
//! | `Reviewer`    | 审稿师     | 质量审查,产出可操作的优先级反馈         |
//! | `Editor`      | 编辑师     | 润色优化,处理审稿反馈,保留作者声音      |
//! | `FactChecker` | 事实核查师 | 验证事实准确性,产出核查报告             |
//! | `Formatter`   | 格式化师   | 格式规范与排版,产出可发布稿件           |
//!
//! ## 设计要点
//!
//! * **自包含**: 不依赖 LLM 网关或异步 runtime,角色定义与管线编排为纯数据
//!   + 同步逻辑,便于单元测试与跨模块复用。
//! * **协作图**: 每个角色声明 `collaboration_with`,形成有向协作图,供
//!   [`WritingRoleRegistry::get_collaborators`] 查询下游协作者。
//! * **管线建议**: [`WritingRoleRegistry::suggest_pipeline`] 按场景关键词
//!   返回推荐角色序列(如小说场景走全 6 角色,邮件场景走 4 角色)。
//! * **管线执行**: [`WritingPipeline::execute`] 按角色顺序逐阶段变换输入,
//!   产出 [`PipelineResult`](含 `final_output` / `stage_outputs` /
//!   `total_duration`)。执行为确定性的阶段变换模拟,实际 LLM 调用由上层
//!   蜂群在拿到角色定义后注入。

use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// WritingRoleId — 6 个写作角色枚举
// ---------------------------------------------------------------------------

/// T-EA-AE-06: 写作场景子智能体角色 ID。
///
/// 6 个变体对应 6 个专业化写作角色,枚举本身只承载身份,能力定义由
/// [`WritingRole`] 结构体表达,通过 [`WritingRoleRegistry`] 查询。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingRoleId {
    /// 大纲师 — 构建故事/文章骨架。
    Outliner,
    /// 起草师 — 快速生成初稿。
    Drafter,
    /// 审稿师 — 质量审查和反馈。
    Reviewer,
    /// 编辑师 — 润色和优化。
    Editor,
    /// 事实核查师 — 验证事实准确性。
    FactChecker,
    /// 格式化师 — 格式规范和排版。
    Formatter,
}

impl WritingRoleId {
    /// 返回角色的稳定字符串标识(与 serde `snake_case` 一致)。
    pub fn as_str(self) -> &'static str {
        match self {
            WritingRoleId::Outliner => "outliner",
            WritingRoleId::Drafter => "drafter",
            WritingRoleId::Reviewer => "reviewer",
            WritingRoleId::Editor => "editor",
            WritingRoleId::FactChecker => "fact_checker",
            WritingRoleId::Formatter => "formatter",
        }
    }

    /// 返回角色中文名(供前端展示与日志)。
    pub fn name_zh(self) -> &'static str {
        match self {
            WritingRoleId::Outliner => "大纲师",
            WritingRoleId::Drafter => "起草师",
            WritingRoleId::Reviewer => "审稿师",
            WritingRoleId::Editor => "编辑师",
            WritingRoleId::FactChecker => "事实核查师",
            WritingRoleId::Formatter => "格式化师",
        }
    }

    /// 返回全部 6 个角色,顺序为写作管线的典型推荐顺序。
    pub fn all() -> [WritingRoleId; 6] {
        [
            WritingRoleId::Outliner,
            WritingRoleId::Drafter,
            WritingRoleId::Reviewer,
            WritingRoleId::Editor,
            WritingRoleId::FactChecker,
            WritingRoleId::Formatter,
        ]
    }
}

impl std::fmt::Display for WritingRoleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WritingRoleId {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "outliner" | "大纲师" => Ok(WritingRoleId::Outliner),
            "drafter" | "起草师" => Ok(WritingRoleId::Drafter),
            "reviewer" | "审稿师" => Ok(WritingRoleId::Reviewer),
            "editor" | "编辑师" => Ok(WritingRoleId::Editor),
            "fact_checker" | "factchecker" | "事实核查师" => Ok(WritingRoleId::FactChecker),
            "formatter" | "格式化师" => Ok(WritingRoleId::Formatter),
            other => Err(format!("unknown writing role id: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// WritingRole — 单个写作角色的完整定义
// ---------------------------------------------------------------------------

/// T-EA-AE-06: 写作角色定义。
///
/// 每个角色携带完整的能力契约:`system_prompt`(≥ 250 字符)、`output_format`、
/// `quality_criteria` 与协作图(`collaboration_with`)。由
/// [`WritingRoleRegistry`] 集中产出,供蜂群在写作场景下查询与注入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WritingRole {
    /// 角色唯一标识。
    pub id: WritingRoleId,
    /// 角色显示名(中文)。
    pub name: String,
    /// 一句话角色描述。
    pub role_description: String,
    /// 核心能力清单(每条为可独立验证的能力点)。
    pub core_abilities: Vec<String>,
    /// Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界 + 输出要求)。
    /// 约束:长度 ≥ 250 字符。
    pub system_prompt: String,
    /// 输出格式描述(如 "Markdown 大纲, H1..H3 + checkbox 节拍")。
    pub output_format: String,
    /// 质量验收标准(每条为可机器/人工校验的判据)。
    pub quality_criteria: Vec<String>,
    /// 协作角色列表(本角色在管线中常与之交接的角色)。
    pub collaboration_with: Vec<WritingRoleId>,
    /// 该角色允许的最大并发任务数(0 表示禁止并发,≥1 表示可并行数)。
    pub max_concurrent_tasks: usize,
}

impl WritingRole {
    /// 返回 `system_prompt` 字符数(用于断言 ≥ 250)。
    pub fn system_prompt_len(&self) -> usize {
        self.system_prompt.chars().count()
    }

    /// 返回 `core_abilities` 数量。
    pub fn ability_count(&self) -> usize {
        self.core_abilities.len()
    }

    /// 返回 `quality_criteria` 数量。
    pub fn criteria_count(&self) -> usize {
        self.quality_criteria.len()
    }
}

// ---------------------------------------------------------------------------
// System prompt 常量(每个 ≥ 250 字符)
// ---------------------------------------------------------------------------

/// Outliner(大纲师)system prompt。
const OUTLINER_PROMPT: &str = "You are the Outliner (大纲师) in the nebula writing swarm.\n\
     Role: architect the skeleton of stories, articles, and long-form content. Build a\n\
     hierarchical outline with chapter beats, scene goals, character arcs, and narrative\n\
     rhythm. Decompose the writing task into a tree of beats (max depth 3); each beat must\n\
     carry a goal, a conflict, and a hook. Hand off actionable beat sheets to the Drafter\n\
     and align tone/audience with the Editor before drafting begins.\n\
     Tools: memory_search, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L5 (lessons learned).\n\
     Output strict Markdown with heading levels H1..H3 and checkbox beats (- [ ]).";

/// Drafter(起草师)system prompt。
const DRAFTER_PROMPT: &str = "You are the Drafter (起草师) in the nebula writing swarm.\n\
     Role: produce a fast first draft from the outline handed over by the Outliner.\n\
     Prioritise velocity over polish — get the story or argument onto the page, hit every\n\
     beat in the outline, and maintain voice continuity across chapters. Do not stop to\n\
     fact-check or format; hand the rough draft to the Reviewer and FactChecker for\n\
     downstream refinement. Surface any outline gaps as inline TODOs rather than blocking.\n\
     Tools: editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L3 (concrete facts).\n\
     Prefer concrete sensory detail over abstract summary. Output plain prose paragraphs.";

/// Reviewer(审稿师)system prompt。
const REVIEWER_PROMPT: &str = "You are the Reviewer (审稿师) in the nebula writing swarm.\n\
     Role: critically evaluate the latest draft for narrative coherence, pacing, voice\n\
     consistency, logical holes, and emotional payoff. Produce actionable, prioritised\n\
     feedback tagged P0 (blockers) / P1 (polish) / P2 (nice-to-have) — never rewrite the\n\
     draft yourself. Coordinate with the Editor to confirm which P0/P1 items are addressed\n\
     before sign-off, and with the FactChecker to cross-check contested claims.\n\
     Tools: editor_read, tool_invoke (read-only — no write/shell).\n\
     Knowledge scope: L2 (cross-session experience).\n\
     End your response with one of: APPROVE / REVISE / REJECT.";

/// Editor(编辑师)system prompt。
const EDITOR_PROMPT: &str = "You are the Editor (编辑师) in the nebula writing swarm.\n\
     Role: polish and optimise prose — refine word choice, sentence rhythm, transitions,\n\
     and imagery. Address every P0/P1 item from the Reviewer's feedback while preserving\n\
     the Drafter's voice and intent; never restructure plot or argument. Coordinate with\n\
     the Formatter to ensure structural edits survive downstream formatting, and with the\n\
     Outliner to keep heading levels consistent with the original skeleton.\n\
     Tools: editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L3 (concrete facts).\n\
     Output the edited prose followed by a short change-log of the top 3 edits made.";

/// FactChecker(事实核查师)system prompt。
const FACT_CHECKER_PROMPT: &str =
    "You are the FactChecker (事实核查师) in the nebula writing swarm.\n\
     Role: verify every factual claim, date, quotation, statistic, and cultural reference\n\
     in the draft. Flag unverified claims with a confidence score (0.0..1.0) and a\n\
     suggested authoritative source. Do not edit the prose — produce a verification\n\
     report the Editor can act on. Escalate contested claims to the Researcher scenario\n\
     for source gathering when confidence is below 0.5.\n\
     Tools: memory_search, tool_invoke.\n\
     Knowledge scope: L1 (session history), L2 (cross-session experience), L4 (distilled\n\
     knowledge).\n\
     Output a Markdown table with columns: claim | verdict | confidence | source.";

/// Formatter(格式化师)system prompt。
const FORMATTER_PROMPT: &str = "You are the Formatter (格式化师) in the nebula writing swarm.\n\
     Role: apply publication-ready Markdown structure — heading hierarchy, blockquotes,\n\
     footnotes, tables of contents, scene-break markers, and citation formatting. Convert\n\
     the Editor's prose into a clean, navigable manuscript. Preserve every word of the\n\
     prose; only structural markup may change. Coordinate with the Outliner to keep heading\n\
     levels consistent with the original outline, and with the Editor to confirm no prose\n\
     was accidentally altered.\n\
     Tools: editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L3 (concrete facts).\n\
     Always explain formatting choices in 2-3 sentences at the end.";

// ---------------------------------------------------------------------------
// WritingRoleRegistry — 角色注册表
// ---------------------------------------------------------------------------

/// T-EA-AE-06: 写作角色注册表,集中管理 6 个角色定义。
///
/// 用 [`WritingRoleRegistry::new`] 构造(每次返回等价的独立副本,角色定义
/// 本身为 `const` 数据派生,重建成本可忽略),通过 `get` / `list` /
/// `get_collaborators` / `suggest_pipeline` 查询。
#[derive(Debug, Clone)]
pub struct WritingRoleRegistry {
    roles: Vec<WritingRole>,
}

impl WritingRoleRegistry {
    /// 初始化 6 个写作角色。重复调用返回等价的独立副本。
    pub fn new() -> Self {
        Self {
            roles: build_all_roles(),
        }
    }

    /// 按 ID 查询角色,未知 ID 返回 `None`。
    pub fn get(&self, id: &WritingRoleId) -> Option<&WritingRole> {
        self.roles.iter().find(|r| &r.id == id)
    }

    /// 返回全部 6 个角色(顺序与 [`WritingRoleId::all`] 一致)。
    pub fn list(&self) -> Vec<&WritingRole> {
        self.roles.iter().collect()
    }

    /// 返回指定角色的协作角色列表(`collaboration_with`)。
    /// 未知角色返回空列表。
    pub fn get_collaborators(&self, id: &WritingRoleId) -> Vec<&WritingRole> {
        match self.get(id) {
            Some(role) => role
                .collaboration_with
                .iter()
                .filter_map(|cid| self.get(cid))
                .collect(),
            None => Vec::new(),
        }
    }

    /// 按场景关键词建议写作管线角色序列。
    ///
    /// 支持的关键词(大小写不敏感,接受中英文):
    /// - `novel` / `小说` / `长篇` → 全 6 角色(大纲→起草→审稿→编辑→核查→格式化)
    /// - `tech-blog` / `技术博客` → 6 角色(核查前置到起草之后)
    /// - `academic` / `学术` → 6 角色(核查前置到起草之后)
    /// - `marketing` / `营销` → 5 角色(跳过事实核查)
    /// - `email` / `邮件` → 4 角色(大纲→起草→编辑→格式化)
    /// - `short` / `短` → 3 角色(起草→编辑→格式化)
    /// - 其他/空 → 默认 5 角色(大纲→起草→审稿→编辑→格式化)
    pub fn suggest_pipeline(&self, scenario: &str) -> Vec<WritingRoleId> {
        let key = scenario.trim().to_ascii_lowercase();
        match key.as_str() {
            "novel" | "小说" | "长篇" | "长篇小说" => vec![
                WritingRoleId::Outliner,
                WritingRoleId::Drafter,
                WritingRoleId::Reviewer,
                WritingRoleId::Editor,
                WritingRoleId::FactChecker,
                WritingRoleId::Formatter,
            ],
            "tech-blog" | "techblog" | "技术博客" | "blog" => vec![
                WritingRoleId::Outliner,
                WritingRoleId::Drafter,
                WritingRoleId::FactChecker,
                WritingRoleId::Reviewer,
                WritingRoleId::Editor,
                WritingRoleId::Formatter,
            ],
            "academic" | "学术" | "论文" => vec![
                WritingRoleId::Outliner,
                WritingRoleId::Drafter,
                WritingRoleId::FactChecker,
                WritingRoleId::Reviewer,
                WritingRoleId::Editor,
                WritingRoleId::Formatter,
            ],
            "marketing" | "营销" | "文案" => vec![
                WritingRoleId::Outliner,
                WritingRoleId::Drafter,
                WritingRoleId::Reviewer,
                WritingRoleId::Editor,
                WritingRoleId::Formatter,
            ],
            "email" | "邮件" | "商务邮件" => vec![
                WritingRoleId::Outliner,
                WritingRoleId::Drafter,
                WritingRoleId::Editor,
                WritingRoleId::Formatter,
            ],
            "short" | "短" | "短篇" | "短文" => {
                vec![
                    WritingRoleId::Drafter,
                    WritingRoleId::Editor,
                    WritingRoleId::Formatter,
                ]
            }
            _ => vec![
                WritingRoleId::Outliner,
                WritingRoleId::Drafter,
                WritingRoleId::Reviewer,
                WritingRoleId::Editor,
                WritingRoleId::Formatter,
            ],
        }
    }
}

impl Default for WritingRoleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 构造全部 6 个写作角色定义。
fn build_all_roles() -> Vec<WritingRole> {
    vec![
        outliner_role(),
        drafter_role(),
        reviewer_role(),
        editor_role(),
        fact_checker_role(),
        formatter_role(),
    ]
}

fn outliner_role() -> WritingRole {
    WritingRole {
        id: WritingRoleId::Outliner,
        name: "大纲师".to_string(),
        role_description: "构建故事/文章骨架,分解章节节拍与叙事节奏".to_string(),
        core_abilities: vec![
            "将写作任务分解为章节/场景节拍树(最大深度 3)".to_string(),
            "为每个节拍标注目标、冲突与钩子".to_string(),
            "规划角色弧线与叙事节奏".to_string(),
            "生成结构化 Markdown 大纲(H1..H3 + checkbox 节拍)".to_string(),
        ],
        system_prompt: OUTLINER_PROMPT.to_string(),
        output_format: "Markdown 大纲,使用 H1/H2/H3 标题层级与 `- [ ]` checkbox 节拍".to_string(),
        quality_criteria: vec![
            "每个节拍必须包含 goal/conflict/hook 三要素".to_string(),
            "节拍树最大深度不超过 3 层".to_string(),
            "覆盖完整故事弧线(开端/发展/高潮/结局)".to_string(),
            "与 Editor 确认语气与目标读者后再交付".to_string(),
        ],
        collaboration_with: vec![WritingRoleId::Drafter, WritingRoleId::Editor],
        max_concurrent_tasks: 1,
    }
}

fn drafter_role() -> WritingRole {
    WritingRole {
        id: WritingRoleId::Drafter,
        name: "起草师".to_string(),
        role_description: "快速生成初稿,优先速度而非打磨,覆盖大纲全部节拍".to_string(),
        core_abilities: vec![
            "依据大纲快速产出初稿,逐节拍覆盖".to_string(),
            "维持跨章节的声音与视角连续性".to_string(),
            "以感官细节替代抽象概述".to_string(),
            "对大纲缺口以行内 TODO 标注而非阻塞".to_string(),
        ],
        system_prompt: DRAFTER_PROMPT.to_string(),
        output_format: "纯文本散文段落(无格式标记),TODO 以 `<!-- TODO: ... -->` 内联".to_string(),
        quality_criteria: vec![
            "大纲中每个节拍在初稿中均有对应内容".to_string(),
            "不引入与大纲矛盾的情节点".to_string(),
            "视角(POV)与既定设定一致".to_string(),
            "未经核查的事实以 `[需核查]` 标记".to_string(),
        ],
        collaboration_with: vec![
            WritingRoleId::Outliner,
            WritingRoleId::Reviewer,
            WritingRoleId::Editor,
        ],
        max_concurrent_tasks: 2,
    }
}

fn reviewer_role() -> WritingRole {
    WritingRole {
        id: WritingRoleId::Reviewer,
        name: "审稿师".to_string(),
        role_description: "质量审查,产出按优先级分级的可操作反馈,不重写稿件".to_string(),
        core_abilities: vec![
            "评估叙事连贯性、节奏与情感张力".to_string(),
            "识别逻辑漏洞与声音不一致".to_string(),
            "产出 P0/P1/P2 优先级反馈".to_string(),
            "与 Editor 确认 P0/P1 项是否已处理".to_string(),
        ],
        system_prompt: REVIEWER_PROMPT.to_string(),
        output_format:
            "反馈报告:按 P0/P1/P2 分组,每条含位置/问题/建议,末行为 APPROVE|REVISE|REJECT"
                .to_string(),
        quality_criteria: vec![
            "每条反馈标注其在稿件中的位置(章节/段落)".to_string(),
            "反馈按 P0/P1/P2 分级,不混合".to_string(),
            "不直接重写稿件正文".to_string(),
            "结论行必须为 APPROVE / REVISE / REJECT 之一".to_string(),
        ],
        collaboration_with: vec![
            WritingRoleId::Drafter,
            WritingRoleId::Editor,
            WritingRoleId::FactChecker,
        ],
        max_concurrent_tasks: 3,
    }
}

fn editor_role() -> WritingRole {
    WritingRole {
        id: WritingRoleId::Editor,
        name: "编辑师".to_string(),
        role_description: "润色优化,处理审稿反馈,保留作者声音与意图".to_string(),
        core_abilities: vec![
            "精修措辞、句式节奏、过渡与意象".to_string(),
            "处理 Reviewer 的全部 P0/P1 反馈".to_string(),
            "保留 Drafter 的声音与叙事意图".to_string(),
            "产出编辑变更日志(Top 3)".to_string(),
        ],
        system_prompt: EDITOR_PROMPT.to_string(),
        output_format: "编辑后正文 + 末尾 `## Change Log` 列出 Top 3 修改".to_string(),
        quality_criteria: vec![
            "Reviewer 标记的全部 P0 项已处理或有理由说明".to_string(),
            "不改变情节点或论证结构".to_string(),
            "变更日志至少 1 条,至多 3 条".to_string(),
            "与 Formatter 确认结构标记未被破坏".to_string(),
        ],
        collaboration_with: vec![
            WritingRoleId::Reviewer,
            WritingRoleId::Formatter,
            WritingRoleId::Outliner,
        ],
        max_concurrent_tasks: 2,
    }
}

fn fact_checker_role() -> WritingRole {
    WritingRole {
        id: WritingRoleId::FactChecker,
        name: "事实核查师".to_string(),
        role_description: "验证事实准确性,产出含置信度与来源的核查报告".to_string(),
        core_abilities: vec![
            "核查日期、引文、统计数据与文化参考".to_string(),
            "为每条主张标注置信度(0.0..1.0)与建议来源".to_string(),
            "对低置信主张(<0.5)升级到 Researcher 场景".to_string(),
            "产出可被 Editor 直接采用的核查表".to_string(),
        ],
        system_prompt: FACT_CHECKER_PROMPT.to_string(),
        output_format: "Markdown 表格: claim | verdict | confidence | source".to_string(),
        quality_criteria: vec![
            "稿件中每条可核查主张均出现在核查表中".to_string(),
            "verdict 仅取 verified / unverified / disputed".to_string(),
            "confidence 为 0.0..1.0 的数值".to_string(),
            "低置信(<0.5)主张必须附建议来源或升级标记".to_string(),
        ],
        collaboration_with: vec![WritingRoleId::Reviewer, WritingRoleId::Editor],
        max_concurrent_tasks: 4,
    }
}

fn formatter_role() -> WritingRole {
    WritingRole {
        id: WritingRoleId::Formatter,
        name: "格式化师".to_string(),
        role_description: "格式规范与排版,产出可发布的最终稿件".to_string(),
        core_abilities: vec![
            "应用标题层级、引用、脚注与目录".to_string(),
            "添加场景分隔标记与引文格式".to_string(),
            "保证正文一字不易,仅调整结构标记".to_string(),
            "与 Outliner 对齐标题层级".to_string(),
        ],
        system_prompt: FORMATTER_PROMPT.to_string(),
        output_format: "可发布 Markdown:含目录、标题层级、脚注与场景分隔;末尾 2-3 句格式说明"
            .to_string(),
        quality_criteria: vec![
            "正文文字与 Editor 交付一致(仅结构标记可变)".to_string(),
            "标题层级与 Outliner 大纲一致".to_string(),
            "包含自动生成的目录".to_string(),
            "末尾附 2-3 句格式选择说明".to_string(),
        ],
        collaboration_with: vec![WritingRoleId::Editor, WritingRoleId::Outliner],
        max_concurrent_tasks: 2,
    }
}

// ---------------------------------------------------------------------------
// PipelineStage / PipelineResult / WritingPipeline
// ---------------------------------------------------------------------------

/// T-EA-AE-06: 写作管线阶段。
///
/// 每个阶段绑定一个角色,声明其输入变换策略与输出校验策略(均为描述性
/// 字符串,供日志与可观测性使用,实际变换逻辑在 [`WritingPipeline::execute`]
/// 中按角色分发)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    /// 该阶段绑定的角色。
    pub role: WritingRoleId,
    /// 输入变换策略描述(如 "wrap_in_outline_skeleton")。
    pub input_transform: String,
    /// 输出校验策略描述(如 "verify_beat_coverage")。
    pub output_check: String,
}

/// T-EA-AE-06: 写作管线执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// 最终输出(最后一个阶段的产出)。
    pub final_output: String,
    /// 各阶段的输出快照(按执行顺序)。
    pub stage_outputs: Vec<StageOutput>,
    /// 管线总耗时。
    #[serde(with = "duration_ms_serde")]
    pub total_duration: Duration,
}

/// 单个阶段的输出快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageOutput {
    /// 产出该阶段的角色。
    pub role: WritingRoleId,
    /// 该阶段的输出文本。
    pub output: String,
    /// 该阶段耗时。
    #[serde(with = "duration_ms_serde")]
    pub duration: Duration,
}

/// T-EA-AE-06: 写作管线 — 按角色顺序逐阶段变换输入。
///
/// 管线为确定性的阶段变换模拟(不调用 LLM),供上层蜂群在拿到角色定义后
/// 注入真实模型。`stages` 字段在 [`WritingPipeline::execute`] 调用后填充,
/// 可用于事后检视实际执行的阶段序列。
#[derive(Debug, Clone, Default)]
pub struct WritingPipeline {
    /// 已执行的阶段序列(`execute` 调用后填充)。
    pub stages: Vec<PipelineStage>,
}

impl WritingPipeline {
    /// 创建空管线。
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    /// 根据角色列表构建阶段序列(不执行)。
    ///
    /// 每个角色映射到固定的 `input_transform` / `output_check` 描述。
    pub fn build_stages(roles: &[WritingRoleId]) -> Vec<PipelineStage> {
        roles.iter().map(|role| stage_for_role(*role)).collect()
    }

    /// 执行写作管线:按 `roles` 顺序逐阶段变换 `input`,返回最终结果。
    ///
    /// - 空角色列表返回错误。
    /// - 每个阶段的输出作为下一阶段的输入。
    /// - `self.stages` 在调用后被填充为实际执行的阶段序列。
    pub fn execute(&mut self, input: &str, roles: &[WritingRoleId]) -> Result<PipelineResult> {
        if roles.is_empty() {
            return Err(anyhow!(
                "writing pipeline requires at least one role, got empty roles list"
            ));
        }

        let stages = Self::build_stages(roles);
        let pipeline_start = Instant::now();
        let mut current = input.to_string();
        let mut stage_outputs = Vec::with_capacity(stages.len());

        for stage in &stages {
            let stage_start = Instant::now();
            let output = run_stage_transform(stage.role, &current);
            let duration = stage_start.elapsed();
            stage_outputs.push(StageOutput {
                role: stage.role,
                output: output.clone(),
                duration,
            });
            current = output;
        }

        self.stages = stages;

        Ok(PipelineResult {
            final_output: current,
            stage_outputs,
            total_duration: pipeline_start.elapsed(),
        })
    }
}

/// 返回指定角色的阶段描述(输入变换 + 输出校验)。
fn stage_for_role(role: WritingRoleId) -> PipelineStage {
    match role {
        WritingRoleId::Outliner => PipelineStage {
            role,
            input_transform: "decompose_into_beat_tree".to_string(),
            output_check: "verify_beat_coverage_and_depth".to_string(),
        },
        WritingRoleId::Drafter => PipelineStage {
            role,
            input_transform: "expand_beats_to_prose".to_string(),
            output_check: "verify_every_beat_addressed".to_string(),
        },
        WritingRoleId::Reviewer => PipelineStage {
            role,
            input_transform: "annotate_with_p0_p1_p2_feedback".to_string(),
            output_check: "verify_verdict_line_present".to_string(),
        },
        WritingRoleId::Editor => PipelineStage {
            role,
            input_transform: "apply_p0_p1_edits_preserve_voice".to_string(),
            output_check: "verify_change_log_present".to_string(),
        },
        WritingRoleId::FactChecker => PipelineStage {
            role,
            input_transform: "extract_claims_and_verify".to_string(),
            output_check: "verify_verification_table_complete".to_string(),
        },
        WritingRoleId::Formatter => PipelineStage {
            role,
            input_transform: "apply_publication_markdown".to_string(),
            output_check: "verify_toc_and_heading_hierarchy".to_string(),
        },
    }
}

/// 阶段变换的确定性模拟:按角色对输入施加可预测的变换。
///
/// 该函数不调用 LLM,仅产出带角色标记的结构化文本,供管线编排测试与
/// 上层集成验证使用。每个角色的变换保持稳定以便断言。
fn run_stage_transform(role: WritingRoleId, input: &str) -> String {
    match role {
        WritingRoleId::Outliner => {
            format!(
                "## Outline (by Outliner)\n\n- [ ] Beat 1: goal / conflict / hook\n- [ ] Beat 2: goal / conflict / hook\n\n<!-- source: {input} -->"
            )
        }
        WritingRoleId::Drafter => {
            format!(
                "### Draft (by Drafter)\n\n{input}\n\n<!-- draft covers all outline beats; TODOs inlined where needed -->"
            )
        }
        WritingRoleId::Reviewer => {
            format!(
                "### Review (by Reviewer)\n\nP0:\n- [position] issue → suggestion\nP1:\n- [position] issue → suggestion\n\n--- original ---\n{input}\n--- end original ---\n\nVERDICT: REVISE"
            )
        }
        WritingRoleId::Editor => {
            format!(
                "### Edited (by Editor)\n\n{input}\n\n## Change Log\n1. refined sentence rhythm\n2. tightened transitions\n3. preserved author voice"
            )
        }
        WritingRoleId::FactChecker => {
            format!(
                "### Fact Check (by FactChecker)\n\n| claim | verdict | confidence | source |\n|---|---|---|---|\n| sample claim | verified | 0.9 | src |\n\n<!-- checked against: {input} -->"
            )
        }
        WritingRoleId::Formatter => {
            format!(
                "# Final Manuscript\n\n## Table of Contents\n1. Section\n\n---\n\n{input}\n\n<!-- formatting: TOC + heading hierarchy + scene breaks applied -->"
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Duration 毫秒序列化辅助(serde with 属性)
// ---------------------------------------------------------------------------

mod duration_ms_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_millis().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u128::deserialize(d)?;
        Ok(Duration::from_millis(ms as u64))
    }
}

// ---------------------------------------------------------------------------
// 单元测试(≥ 15 个)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WritingRoleId 枚举测试 ----

    #[test]
    fn writing_role_id_as_str_roundtrip() {
        for id in WritingRoleId::all() {
            let s = id.as_str();
            let parsed: WritingRoleId = s.parse().expect("as_str output must parse back");
            assert_eq!(parsed, id, "roundtrip failed for {id:?}");
        }
    }

    #[test]
    fn writing_role_id_parses_chinese_aliases() {
        assert_eq!(
            "大纲师".parse::<WritingRoleId>().unwrap(),
            WritingRoleId::Outliner
        );
        assert_eq!(
            "起草师".parse::<WritingRoleId>().unwrap(),
            WritingRoleId::Drafter
        );
        assert_eq!(
            "审稿师".parse::<WritingRoleId>().unwrap(),
            WritingRoleId::Reviewer
        );
        assert_eq!(
            "编辑师".parse::<WritingRoleId>().unwrap(),
            WritingRoleId::Editor
        );
        assert_eq!(
            "事实核查师".parse::<WritingRoleId>().unwrap(),
            WritingRoleId::FactChecker
        );
        assert_eq!(
            "格式化师".parse::<WritingRoleId>().unwrap(),
            WritingRoleId::Formatter
        );
    }

    #[test]
    fn writing_role_id_from_str_rejects_unknown() {
        assert!("unknown".parse::<WritingRoleId>().is_err());
        assert!("".parse::<WritingRoleId>().is_err());
        assert!("outline".parse::<WritingRoleId>().is_err());
    }

    #[test]
    fn writing_role_id_serde_snake_case_roundtrip() {
        for id in WritingRoleId::all() {
            let json = serde_json::to_string(&id).expect("serialize");
            // snake_case: fact_checker 而非 FactChecker
            assert!(
                json.contains(id.as_str()),
                "JSON {json} should contain {}",
                id.as_str()
            );
            let de: WritingRoleId = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, id);
        }
        // 单独验证 fact_checker 的 snake_case 形态
        let json = serde_json::to_string(&WritingRoleId::FactChecker).unwrap();
        assert_eq!(json, "\"fact_checker\"");
    }

    #[test]
    fn writing_role_id_all_returns_six_in_canonical_order() {
        let all = WritingRoleId::all();
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], WritingRoleId::Outliner);
        assert_eq!(all[5], WritingRoleId::Formatter);
    }

    // ---- Registry 基础测试 ----

    #[test]
    fn registry_new_has_six_roles() {
        let reg = WritingRoleRegistry::new();
        assert_eq!(reg.list().len(), 6, "registry must contain exactly 6 roles");
        for id in WritingRoleId::all() {
            assert!(reg.get(&id).is_some(), "role {id:?} must be present");
        }
    }

    #[test]
    fn registry_get_returns_complete_role_fields() {
        let reg = WritingRoleRegistry::new();
        let outliner = reg.get(&WritingRoleId::Outliner).expect("outliner exists");
        assert_eq!(outliner.id, WritingRoleId::Outliner);
        assert_eq!(outliner.name, "大纲师");
        assert!(!outliner.role_description.is_empty());
        assert!(!outliner.core_abilities.is_empty());
        assert!(!outliner.output_format.is_empty());
        assert!(!outliner.quality_criteria.is_empty());
        assert!(outliner.max_concurrent_tasks >= 1);
    }

    // ---- system_prompt ≥ 250 字符测试(每个角色一条) ----

    #[test]
    fn outliner_system_prompt_meets_min_length() {
        let reg = WritingRoleRegistry::new();
        let role = reg.get(&WritingRoleId::Outliner).unwrap();
        assert!(
            role.system_prompt_len() >= 250,
            "outliner system_prompt too short: {} chars",
            role.system_prompt_len()
        );
    }

    #[test]
    fn drafter_system_prompt_meets_min_length() {
        let reg = WritingRoleRegistry::new();
        let role = reg.get(&WritingRoleId::Drafter).unwrap();
        assert!(
            role.system_prompt_len() >= 250,
            "drafter system_prompt too short: {} chars",
            role.system_prompt_len()
        );
    }

    #[test]
    fn reviewer_system_prompt_meets_min_length() {
        let reg = WritingRoleRegistry::new();
        let role = reg.get(&WritingRoleId::Reviewer).unwrap();
        assert!(
            role.system_prompt_len() >= 250,
            "reviewer system_prompt too short: {} chars",
            role.system_prompt_len()
        );
    }

    #[test]
    fn editor_system_prompt_meets_min_length() {
        let reg = WritingRoleRegistry::new();
        let role = reg.get(&WritingRoleId::Editor).unwrap();
        assert!(
            role.system_prompt_len() >= 250,
            "editor system_prompt too short: {} chars",
            role.system_prompt_len()
        );
    }

    #[test]
    fn fact_checker_system_prompt_meets_min_length() {
        let reg = WritingRoleRegistry::new();
        let role = reg.get(&WritingRoleId::FactChecker).unwrap();
        assert!(
            role.system_prompt_len() >= 250,
            "fact_checker system_prompt too short: {} chars",
            role.system_prompt_len()
        );
    }

    #[test]
    fn formatter_system_prompt_meets_min_length() {
        let reg = WritingRoleRegistry::new();
        let role = reg.get(&WritingRoleId::Formatter).unwrap();
        assert!(
            role.system_prompt_len() >= 250,
            "formatter system_prompt too short: {} chars",
            role.system_prompt_len()
        );
    }

    // ---- 协作图测试 ----

    #[test]
    fn get_collaborators_returns_expected_for_each_role() {
        let reg = WritingRoleRegistry::new();
        // Outliner 协作 Drafter + Editor
        let outliner_collabs = reg.get_collaborators(&WritingRoleId::Outliner);
        assert_eq!(outliner_collabs.len(), 2);
        let collab_ids: Vec<WritingRoleId> = outliner_collabs.iter().map(|r| r.id).collect();
        assert!(collab_ids.contains(&WritingRoleId::Drafter));
        assert!(collab_ids.contains(&WritingRoleId::Editor));

        // FactChecker 协作 Reviewer + Editor
        let fc_collabs = reg.get_collaborators(&WritingRoleId::FactChecker);
        assert_eq!(fc_collabs.len(), 2);
    }

    #[test]
    fn collaboration_graph_is_non_empty_for_all_roles() {
        // 6 个角色均应声明至少 2 个协作者(管线交接的基础)。
        let reg = WritingRoleRegistry::new();
        for id in WritingRoleId::all() {
            let role = reg.get(&id).expect("role exists");
            assert!(
                role.collaboration_with.len() >= 2,
                "role {:?} should have >= 2 collaborators, got {}",
                id,
                role.collaboration_with.len()
            );
        }
    }

    // ---- suggest_pipeline 测试 ----

    #[test]
    fn suggest_pipeline_novel_returns_full_six_roles() {
        let reg = WritingRoleRegistry::new();
        let pipe = reg.suggest_pipeline("novel");
        assert_eq!(pipe.len(), 6);
        assert_eq!(pipe[0], WritingRoleId::Outliner);
        assert_eq!(pipe[1], WritingRoleId::Drafter);
        assert_eq!(pipe.last().copied().unwrap(), WritingRoleId::Formatter);
    }

    #[test]
    fn suggest_pipeline_chinese_keyword_works() {
        let reg = WritingRoleRegistry::new();
        let pipe = reg.suggest_pipeline("长篇小说");
        assert_eq!(pipe.len(), 6);
        let pipe_short = reg.suggest_pipeline("短文");
        assert_eq!(
            pipe_short,
            vec![
                WritingRoleId::Drafter,
                WritingRoleId::Editor,
                WritingRoleId::Formatter
            ]
        );
    }

    #[test]
    fn suggest_pipeline_tech_blog_puts_fact_checker_early() {
        let reg = WritingRoleRegistry::new();
        let pipe = reg.suggest_pipeline("tech-blog");
        assert_eq!(pipe.len(), 6);
        // 技术博客:核查在审稿之前(索引 2 < 3)
        let fc_pos = pipe
            .iter()
            .position(|r| *r == WritingRoleId::FactChecker)
            .unwrap();
        let rev_pos = pipe
            .iter()
            .position(|r| *r == WritingRoleId::Reviewer)
            .unwrap();
        assert!(
            fc_pos < rev_pos,
            "fact checker should come before reviewer in tech-blog"
        );
    }

    #[test]
    fn suggest_pipeline_default_skips_fact_checker() {
        let reg = WritingRoleRegistry::new();
        let pipe = reg.suggest_pipeline("unknown-scenario");
        assert_eq!(pipe.len(), 5);
        assert!(!pipe.contains(&WritingRoleId::FactChecker));
        assert_eq!(pipe[0], WritingRoleId::Outliner);
        assert_eq!(pipe.last().copied().unwrap(), WritingRoleId::Formatter);
    }

    // ---- Pipeline 执行测试 ----

    #[test]
    fn pipeline_execute_rejects_empty_roles() {
        let mut pipe = WritingPipeline::new();
        let result = pipe.execute("some input", &[]);
        assert!(result.is_err(), "empty roles should error");
    }

    #[test]
    fn pipeline_execute_single_role_produces_output() {
        let mut pipe = WritingPipeline::new();
        let result = pipe
            .execute("hello world", &[WritingRoleId::Outliner])
            .expect("single role should succeed");
        assert!(!result.final_output.is_empty());
        assert_eq!(result.stage_outputs.len(), 1);
        assert_eq!(result.stage_outputs[0].role, WritingRoleId::Outliner);
        // Outliner 输出应含节拍标记
        assert!(result.final_output.contains("Beat 1"));
    }

    #[test]
    fn pipeline_execute_full_pipeline_chains_stages() {
        let mut pipe = WritingPipeline::new();
        let roles = vec![
            WritingRoleId::Outliner,
            WritingRoleId::Drafter,
            WritingRoleId::Reviewer,
            WritingRoleId::Editor,
            WritingRoleId::FactChecker,
            WritingRoleId::Formatter,
        ];
        let result = pipe
            .execute("initial input", &roles)
            .expect("full pipeline");
        assert_eq!(result.stage_outputs.len(), 6);
        // 最终输出应含 Formatter 的标记
        assert!(result.final_output.contains("Final Manuscript"));
        // 各阶段角色顺序正确
        let role_seq: Vec<WritingRoleId> = result.stage_outputs.iter().map(|s| s.role).collect();
        assert_eq!(role_seq, roles);
    }

    #[test]
    fn pipeline_execute_populates_stages_field() {
        let mut pipe = WritingPipeline::new();
        assert!(pipe.stages.is_empty(), "fresh pipeline has no stages");
        let roles = vec![WritingRoleId::Drafter, WritingRoleId::Editor];
        let _ = pipe.execute("input", &roles).expect("execute");
        assert_eq!(
            pipe.stages.len(),
            2,
            "stages should be populated after execute"
        );
        assert_eq!(pipe.stages[0].role, WritingRoleId::Drafter);
        assert_eq!(pipe.stages[1].role, WritingRoleId::Editor);
        // 阶段描述非空
        for stage in &pipe.stages {
            assert!(!stage.input_transform.is_empty());
            assert!(!stage.output_check.is_empty());
        }
    }

    #[test]
    fn pipeline_result_total_duration_is_non_negative() {
        let mut pipe = WritingPipeline::new();
        let result = pipe
            .execute("input", &[WritingRoleId::Formatter])
            .expect("execute");
        assert!(
            result.total_duration.as_nanos() < u128::MAX,
            "duration should be sane"
        );
        // 总耗时 ≥ 0(Instant::elapsed 不会回退)
    }

    #[test]
    fn pipeline_result_serializes_to_json() {
        let mut pipe = WritingPipeline::new();
        let result = pipe
            .execute("input", &[WritingRoleId::Drafter, WritingRoleId::Editor])
            .expect("execute");
        let json = serde_json::to_string(&result).expect("serialize result");
        assert!(json.contains("final_output"));
        assert!(json.contains("stage_outputs"));
        assert!(json.contains("total_duration"));
        // 反序列化往返
        let de: PipelineResult = serde_json::from_str(&json).expect("deserialize result");
        assert_eq!(de.final_output, result.final_output);
        assert_eq!(de.stage_outputs.len(), 2);
    }

    // ---- 阶段变换确定性测试 ----

    #[test]
    fn stage_transform_is_deterministic_per_role() {
        // 同角色 + 同输入应产出完全相同的输出。
        let a = run_stage_transform(WritingRoleId::Drafter, "x");
        let b = run_stage_transform(WritingRoleId::Drafter, "x");
        assert_eq!(a, b);
        // 不同角色产出不同
        let c = run_stage_transform(WritingRoleId::Editor, "x");
        assert_ne!(a, c);
    }

    #[test]
    fn build_stages_returns_one_stage_per_role() {
        let roles = WritingRoleId::all();
        let stages = WritingPipeline::build_stages(&roles);
        assert_eq!(stages.len(), 6);
        for (i, stage) in stages.iter().enumerate() {
            assert_eq!(stage.role, roles[i]);
            assert!(!stage.input_transform.is_empty());
            assert!(!stage.output_check.is_empty());
        }
    }

    // ---- 角色名称唯一性测试 ----

    #[test]
    fn role_names_are_unique() {
        let reg = WritingRoleRegistry::new();
        let names: Vec<&str> = reg.list().iter().map(|r| r.name.as_str()).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "role names must be unique");
    }

    #[test]
    fn role_core_abilities_and_criteria_non_empty() {
        let reg = WritingRoleRegistry::new();
        for role in reg.list() {
            assert!(
                role.ability_count() >= 3,
                "role {:?} should have >= 3 core abilities",
                role.id
            );
            assert!(
                role.criteria_count() >= 3,
                "role {:?} should have >= 3 quality criteria",
                role.id
            );
        }
    }

    #[test]
    fn fact_checker_has_highest_concurrency() {
        // 事实核查可并行处理多条主张,应允许最多并发。
        let reg = WritingRoleRegistry::new();
        let fc = reg.get(&WritingRoleId::FactChecker).unwrap();
        let outliner = reg.get(&WritingRoleId::Outliner).unwrap();
        assert!(fc.max_concurrent_tasks > outliner.max_concurrent_tasks);
        assert_eq!(outliner.max_concurrent_tasks, 1, "outliner is sequential");
    }

    #[test]
    fn default_registry_equals_new() {
        let a = WritingRoleRegistry::new();
        let b = WritingRoleRegistry::default();
        assert_eq!(a.list().len(), b.list().len());
    }
}
