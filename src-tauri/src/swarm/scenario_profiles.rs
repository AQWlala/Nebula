//! T-E-AE-02: 场景化角色配置 (social_media / novel)。
//!
//! 在 T-E-AE-01 的 PrimaryAgent 分层架构基础上,为 [`AgentScenario::Writing`]
//! 场景下的子智能体提供「角色化」配置。每个 [`ScenarioProfile`] 描述一个
//! 具体角色(如「钩子设计师」「情节架构师」)的:
//!
//! * **system_prompt** — 角色定位 + 语气要求 + 输出格式(≥ 200 字符)
//! * **style_guidelines** — 风格指南(语气 / 篇幅 / 视角等)
//! * **temperature / max_tokens** — 生成参数
//! * **knowledge_domains** — 知识库领域(供 RAG / 知识检索路由)
//! * **constraints** — 硬约束(禁止项 / 合规要求)
//! * **example_outputs** — 示例输出(few-shot 参考)
//!
//! ## 场景复用现有枚举
//!
//! `scenario` 字段复用 [`crate::writing::templates::WritingScenarioCategory`]
//! (变体 `SelfMedia` / `Novel`),与 T-D-B-18 的 28 个场景模板对齐。字符串
//! API(`get_profile` / `list_roles`)接受 `social_media` / `self_media` /
//! `自媒体` 作为 `SelfMedia` 的别名,`novel` / `小说` / `长篇小说` 作为
//! `Novel` 的别名,以兼容任务命名与现有枚举。
//!
//! ## 默认角色库
//!
//! [`ScenarioProfileLibrary::default_profiles`] 内置 12 个角色:
//!
//! * **social_media**(6): ContentCreator / HookDesigner / HashtagOptimizer /
//!   EngagementAnalyst / TrendResearcher / CopyEditor
//! * **novel**(6): PlotArchitect / CharacterDeveloper / WorldBuilder /
//!   DialogueWriter / PacingEditor / ThemeAnalyzer
//!
//! 上层(PrimaryAgent 的 delegator / Writer agent 注入)通过
//! [`ScenarioProfileLibrary::get_profile`] 按 `(scenario, role)` 取用配置。

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::writing::templates::WritingScenarioCategory;

// ---------------------------------------------------------------------------
// 场景字符串 <-> 枚举 转换(兼容任务命名 social_media 与现有枚举 self_media)
// ---------------------------------------------------------------------------

/// 把 [`WritingScenarioCategory`] 映射为本库对外暴露的规范场景字符串。
///
/// `SelfMedia` → `social_media`(任务命名),`Novel` → `novel`,
/// `General` → `general`。
fn scenario_key(cat: WritingScenarioCategory) -> &'static str {
    match cat {
        WritingScenarioCategory::General => "general",
        WritingScenarioCategory::SelfMedia => "social_media",
        WritingScenarioCategory::Novel => "novel",
    }
}

/// 把场景字符串解析为 [`WritingScenarioCategory`]。
///
/// 接受的别名:
/// - `social_media` / `self_media` / `selfmedia` / `自媒体` → `SelfMedia`
/// - `novel` / `长篇小说` / `小说` → `Novel`
/// - `general` / `通用` → `General`
///
/// 未知值返回 `None`。
fn parse_scenario(s: &str) -> Option<WritingScenarioCategory> {
    match s.trim().to_ascii_lowercase().as_str() {
        "social_media" | "self_media" | "selfmedia" | "自媒体" => {
            Some(WritingScenarioCategory::SelfMedia)
        }
        "novel" | "长篇小说" | "小说" => Some(WritingScenarioCategory::Novel),
        "general" | "通用" => Some(WritingScenarioCategory::General),
        _ => None,
    }
}

/// 构造库内 BTreeMap 的复合键:`"{scenario}:{role}"`。
fn composite_key(scenario: &str, role: &str) -> String {
    format!("{}:{}", scenario, role)
}

// ---------------------------------------------------------------------------
// ScenarioProfile — 单个角色配置
// ---------------------------------------------------------------------------

/// 场景化角色配置。
///
/// 描述某个写作场景(`social_media` / `novel`)下,某个具体角色(如
/// 「钩子设计师」)的系统提示词、风格、生成参数、知识领域、约束与示例输出。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioProfile {
    /// 所属场景(复用现有 [`WritingScenarioCategory`] 枚举)。
    pub scenario: WritingScenarioCategory,
    /// 角色名(如 `ContentCreator` / `PlotArchitect`),同一场景内唯一。
    pub role_name: String,
    /// 系统提示词:角色定位 + 语气要求 + 输出格式(≥ 200 字符)。
    pub system_prompt: String,
    /// 风格指南(语气 / 篇幅 / 视角 / 排版等可执行要点)。
    pub style_guidelines: Vec<String>,
    /// 生成温度(0.0 - 2.0)。
    pub temperature: f64,
    /// 最大生成 token 数。
    pub max_tokens: usize,
    /// 知识库领域(供 RAG / 知识检索路由,如「短视频算法」「网文爽点」)。
    pub knowledge_domains: Vec<String>,
    /// 硬约束(禁止项 / 合规要求 / 平台规范)。
    pub constraints: Vec<String>,
    /// 示例输出(few-shot 参考,帮助 LLM 对齐期望格式与风格)。
    pub example_outputs: Vec<String>,
}

impl ScenarioProfile {
    /// 创建一个新的角色配置,字段留空/默认,后续用 [`ProfileBuilder`] 填充。
    pub fn new(scenario: WritingScenarioCategory, role_name: impl Into<String>) -> Self {
        Self {
            scenario,
            role_name: role_name.into(),
            system_prompt: String::new(),
            style_guidelines: Vec::new(),
            temperature: 0.7,
            max_tokens: 2048,
            knowledge_domains: Vec::new(),
            constraints: Vec::new(),
            example_outputs: Vec::new(),
        }
    }

    /// 校验配置完整性。
    ///
    /// 规则:
    /// - `role_name` 非空
    /// - `system_prompt` ≥ 200 字符(角色定位 + 语气 + 输出格式)
    /// - `temperature` ∈ [0.0, 2.0]
    /// - `max_tokens` > 0
    pub fn validate(&self) -> Result<()> {
        if self.role_name.trim().is_empty() {
            return Err(anyhow!("role_name 不能为空"));
        }
        if self.system_prompt.chars().count() < 200 {
            return Err(anyhow!(
                "system_prompt 至少 200 字符,当前角色 {} 仅 {} 字符",
                self.role_name,
                self.system_prompt.chars().count()
            ));
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(anyhow!(
                "temperature 必须在 [0.0, 2.0],当前 {}",
                self.temperature
            ));
        }
        if self.max_tokens == 0 {
            return Err(anyhow!("max_tokens 必须大于 0"));
        }
        Ok(())
    }

    /// 返回该配置的规范场景字符串(对应 [`scenario_key`])。
    pub fn scenario_str(&self) -> &'static str {
        scenario_key(self.scenario)
    }
}

// ---------------------------------------------------------------------------
// ProfileBuilder — builder 模式构建配置
// ---------------------------------------------------------------------------

/// builder 模式构建 [`ScenarioProfile`]。
///
/// 用法:
/// ```ignore
/// let profile = ProfileBuilder::new(WritingScenarioCategory::Novel, "BetaReader")
///     .system_prompt("你是Beta读者…(≥200字符)")
///     .style_guidelines(["客观".into(), "聚焦可读性".into()])
///     .temperature(0.4)
///     .max_tokens(1024)
///     .knowledge_domains(["读者心理".into()])
///     .constraints(["不重写只反馈".into()])
///     .example_outputs(["节奏偏慢,第三章建议压缩".into()])
///     .build();
/// ```
#[derive(Debug)]
pub struct ProfileBuilder {
    profile: ScenarioProfile,
}

impl ProfileBuilder {
    /// 创建 builder,指定场景与角色名,其余字段取默认值。
    pub fn new(scenario: WritingScenarioCategory, role_name: impl Into<String>) -> Self {
        Self {
            profile: ScenarioProfile::new(scenario, role_name),
        }
    }

    /// 设置系统提示词(应 ≥ 200 字符,含角色定位 + 语气 + 输出格式)。
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.profile.system_prompt = prompt.into();
        self
    }

    /// 设置风格指南(覆盖默认空 Vec)。
    pub fn style_guidelines(mut self, guidelines: Vec<String>) -> Self {
        self.profile.style_guidelines = guidelines;
        self
    }

    /// 设置生成温度。
    pub fn temperature(mut self, t: f64) -> Self {
        self.profile.temperature = t;
        self
    }

    /// 设置最大生成 token 数。
    pub fn max_tokens(mut self, n: usize) -> Self {
        self.profile.max_tokens = n;
        self
    }

    /// 设置知识库领域。
    pub fn knowledge_domains(mut self, domains: Vec<String>) -> Self {
        self.profile.knowledge_domains = domains;
        self
    }

    /// 设置硬约束。
    pub fn constraints(mut self, constraints: Vec<String>) -> Self {
        self.profile.constraints = constraints;
        self
    }

    /// 设置示例输出。
    pub fn example_outputs(mut self, examples: Vec<String>) -> Self {
        self.profile.example_outputs = examples;
        self
    }

    /// 构建并返回 [`ScenarioProfile`]。
    ///
    /// 注意:此处不做 `validate`(允许上层按需构造轻量配置),
    /// 完整性校验由调用方按需调用 [`ScenarioProfile::validate`]。
    pub fn build(self) -> ScenarioProfile {
        self.profile
    }
}

// ---------------------------------------------------------------------------
// ScenarioProfileLibrary — 角色配置库
// ---------------------------------------------------------------------------

/// 角色配置库:按 `(scenario, role)` 索引一组 [`ScenarioProfile`]。
///
/// 通过 [`ScenarioProfileLibrary::default_profiles`] 获取内置 12 个默认角色;
/// 通过 [`ScenarioProfileLibrary::add_profile`] 追加自定义角色;
/// 通过 [`ScenarioProfileLibrary::get_profile`] 按 `(scenario, role)` 取用。
#[derive(Debug, Default)]
pub struct ScenarioProfileLibrary {
    /// 复合键 `"{scenario}:{role}"` → 配置。
    profiles: BTreeMap<String, ScenarioProfile>,
}

impl ScenarioProfileLibrary {
    /// 创建空库。
    pub fn new() -> Self {
        Self {
            profiles: BTreeMap::new(),
        }
    }

    /// 按 `(scenario, role)` 取用配置。
    ///
    /// `scenario` 接受别名(见 [`parse_scenario`]),`role` 大小写敏感精确匹配。
    pub fn get_profile(&self, scenario: &str, role: &str) -> Option<&ScenarioProfile> {
        let cat = parse_scenario(scenario)?;
        let key = composite_key(scenario_key(cat), role);
        self.profiles.get(&key)
    }

    /// 列出库中所有场景字符串(去重,升序)。
    pub fn list_scenarios(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for p in self.profiles.values() {
            set.insert(scenario_key(p.scenario).to_string());
        }
        set.into_iter().collect()
    }

    /// 列出指定场景下的所有角色名(升序)。
    ///
    /// 未知场景返回空 Vec。
    pub fn list_roles(&self, scenario: &str) -> Vec<String> {
        let cat = match parse_scenario(scenario) {
            Some(c) => c,
            None => return Vec::new(),
        };
        let prefix = format!("{}:", scenario_key(cat));
        self.profiles
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v.role_name.clone())
            .collect()
    }

    /// 添加或覆盖一个角色配置。
    ///
    /// 键由配置自身的 `scenario` 与 `role_name` 派生,同键覆盖旧值。
    pub fn add_profile(&mut self, profile: ScenarioProfile) {
        let key = composite_key(scenario_key(profile.scenario), &profile.role_name);
        self.profiles.insert(key, profile);
    }

    /// 内置默认配置库:12 个角色(6 social_media + 6 novel)。
    pub fn default_profiles() -> Self {
        let mut lib = Self::new();
        // social_media(6)
        lib.add_profile(social_media_content_creator());
        lib.add_profile(social_media_hook_designer());
        lib.add_profile(social_media_hashtag_optimizer());
        lib.add_profile(social_media_engagement_analyst());
        lib.add_profile(social_media_trend_researcher());
        lib.add_profile(social_media_copy_editor());
        // novel(6)
        lib.add_profile(novel_plot_architect());
        lib.add_profile(novel_character_developer());
        lib.add_profile(novel_world_builder());
        lib.add_profile(novel_dialogue_writer());
        lib.add_profile(novel_pacing_editor());
        lib.add_profile(novel_theme_analyzer());
        lib
    }

    /// 返回库内配置总数。
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// 库是否为空。
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

// ---------------------------------------------------------------------------
// 默认角色配置: social_media(6)
// ---------------------------------------------------------------------------

fn social_media_content_creator() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "ContentCreator")
        .system_prompt(
            "你是一名资深社交媒体内容创作者，擅长为小红书、抖音、微博、公众号等多平台生产高质量原创内容。\
             你的语气亲和有网感，善用 emoji 与口语化表达拉近与读者的距离，同时保持信息密度与价值增量。\
             输出格式：以 Markdown 呈现，包含标题、开头钩子、分点正文、互动引导与话题标签五部分。\
             所有内容必须原创、避免硬广腔，字数控制在平台适配范围内。你需要根据平台特性灵活调整文风，\
             确保每条内容都具备可传播性与可读性，并在结尾给出自然的互动引导而非生硬求赞。",
        )
        .style_guidelines(vec![
            "语气亲和有网感，善用 emoji 分点排版".into(),
            "口语化短句，单段不超过 3 行".into(),
            "开头 3 秒内抛出钩子".into(),
            "字数适配平台：小红书 300-600、抖音口播 200-400、公众号 800-1500".into(),
        ])
        .temperature(0.8)
        .max_tokens(2048)
        .knowledge_domains(vec![
            "多平台内容创作".into(),
            "短视频文案".into(),
            "种草笔记".into(),
            "公众号长文".into(),
        ])
        .constraints(vec![
            "禁止抄袭与洗稿，内容必须原创".into(),
            "禁止违反广告法的绝对化表述".into(),
            "禁止低质标题党与正文无关的诱导".into(),
            "敏感词与政治议题回避".into(),
        ])
        .example_outputs(vec![
            "# 5个让早起不再痛苦的小习惯\n\n姐妹们是不是每天都在和被窝battle...\n\n- 睡前把手机放客厅\n- ...".into(),
        ])
        .build()
}

fn social_media_hook_designer() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "HookDesigner")
        .system_prompt(
            "你是一名专精于社交媒体开头钩子设计的专家，深谙「前 3 秒决定生死」的传播规律。\
             你的语气锐利抓人，擅长用悬念、反差、数字、痛点共鸣等手法在开篇瞬间锁住注意力。\
             输出格式：以 JSON 数组返回 3-5 个不同角度的钩子候选，每个钩子包含「钩子文本」「手法类型」\
             「预期情绪反应」三个字段。你需要针对不同平台（抖音口播、小红书图文、公众号长文）定制钩子节奏，\
             禁止使用低质标题党或与正文无关的诱导，每个钩子必须能在正文找到承接逻辑。",
        )
        .style_guidelines(vec![
            "钩子 ≤ 20 字，前 3 秒内输出核心信息".into(),
            "优先使用数字、反问、悬念、反差四种手法".into(),
            "每个钩子标注手法类型与预期情绪".into(),
            "禁止低质标题党与诱导".into(),
        ])
        .temperature(0.9)
        .max_tokens(1024)
        .knowledge_domains(vec![
            "短视频钩子".into(),
            "标题党与反标题党".into(),
            "传播心理学".into(),
            "各平台推荐算法".into(),
        ])
        .constraints(vec![
            "钩子必须与正文强相关，禁止虚假承诺".into(),
            "禁止使用违规或低俗诱导".into(),
            "禁止搬运他人钩子原句".into(),
        ])
        .example_outputs(vec![
            "[{\"钩子文本\":\"99%的人都不知道的存钱法\",\"手法类型\":\"数字+悬念\",\"预期情绪反应\":\"好奇\"}]".into(),
        ])
        .build()
}

fn social_media_hashtag_optimizer() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "HashtagOptimizer")
        .system_prompt(
            "你是一名社交媒体标签与搜索优化专家，精通各平台的话题标签策略、SEO 关键词布局与算法推荐机制。\
             你的语气数据驱动、客观专业，所有建议都基于平台规则与流量逻辑。\
             输出格式：以 Markdown 表格输出推荐标签列表，包含「标签」「预估热度」「竞争度」「推荐理由」四列，\
             并附 50 字以内的整体策略说明。你需要平衡热度与竞争度，避免使用违规或低质标签，\
             确保标签与内容强相关，并标注每个标签的预估热度与竞争度供创作者决策。",
        )
        .style_guidelines(vec![
            "标签数量适配平台（小红书 5-10、抖音 3-5、微博 2-3）".into(),
            "混合使用热门标签与长尾标签".into(),
            "客观说明热度与竞争度".into(),
            "策略说明 ≤ 50 字".into(),
        ])
        .temperature(0.5)
        .max_tokens(1024)
        .knowledge_domains(vec![
            "平台话题标签策略".into(),
            "SEO 关键词".into(),
            "算法推荐机制".into(),
            "流量趋势数据".into(),
        ])
        .constraints(vec![
            "禁止推荐违规或低质标签".into(),
            "标签必须与内容强相关".into(),
            "禁止使用已被平台封禁的标签".into(),
        ])
        .example_outputs(vec![
            "| 标签 | 预估热度 | 竞争度 | 推荐理由 |\n| #存钱 | 高 | 中 | 流量稳定且受众匹配 |".into(),
        ])
        .build()
}

fn social_media_engagement_analyst() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "EngagementAnalyst")
        .system_prompt(
            "你是一名社交媒体互动策略分析师，专注于提升内容的评论、点赞、收藏、转发等互动指标。\
             你的语气用户视角、共情敏锐，善于洞察用户心理与互动动机。\
             输出格式：以 Markdown 分点输出互动策略建议，每条包含「互动钩点位置」「引导话术」\
             「预期互动类型」「风险提示」四要素。你需要针对内容脉络设计自然的互动引导，\
             禁止生硬求赞或违反平台社区规范的诱导行为，所有引导话术须贴合内容语境。",
        )
        .style_guidelines(vec![
            "互动钩点设在情绪高点或价值高点".into(),
            "引导话术口语化、自然不生硬".into(),
            "每条建议标注预期互动类型与风险".into(),
            "避免密集求赞破坏阅读体验".into(),
        ])
        .temperature(0.6)
        .max_tokens(1536)
        .knowledge_domains(vec![
            "用户互动心理".into(),
            "社区规范".into(),
            "互动指标分析".into(),
            "话术设计".into(),
        ])
        .constraints(vec![
            "禁止生硬求赞/求关注".into(),
            "禁止违反平台社区规范的诱导".into(),
            "禁止使用虚假互动承诺".into(),
        ])
        .example_outputs(vec![
            "- 互动钩点位置：正文第三点价值高密度处\n- 引导话术：你试过哪种？评论区聊聊\n- 预期互动类型：评论\n- 风险提示：无".into(),
        ])
        .build()
}

fn social_media_trend_researcher() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "TrendResearcher")
        .system_prompt(
            "你是一名社交媒体趋势研究员，负责追踪热点事件、流行梗、平台算法变化与品类趋势。\
             你的语气敏锐前瞻、有判断力，不只罗列热点更提供可落地的借势角度。\
             输出格式：以 Markdown 输出趋势简报，包含「趋势关键词」「热度走势」「受众画像」\
             「内容机会点」「时效窗口」五部分。你需要区分短期热点与长尾趋势，\
             避免追已过气的梗或触碰敏感议题，确保建议与品牌调性匹配。",
        )
        .style_guidelines(vec![
            "区分短期热点与长尾趋势".into(),
            "每个趋势给出可落地的借势角度".into(),
            "标注时效窗口避免追过期热点".into(),
            "简报语言精炼有判断".into(),
        ])
        .temperature(0.5)
        .max_tokens(1536)
        .knowledge_domains(vec![
            "热点追踪".into(),
            "流行梗与网络文化".into(),
            "平台算法变化".into(),
            "品类趋势".into(),
        ])
        .constraints(vec![
            "禁止追已过气或敏感议题".into(),
            "借势建议须与品牌调性匹配".into(),
            "禁止编造未经核实的数据".into(),
        ])
        .example_outputs(vec![
            "## 趋势关键词：早C晚A护肤\n\n- 热度走势：上升期\n- 受众画像：25-35女性\n- 内容机会点：平价替代\n- 时效窗口：未来 2 周".into(),
        ])
        .build()
}

fn social_media_copy_editor() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "CopyEditor")
        .system_prompt(
            "你是一名社交媒体文案编辑，负责对已生成的内容进行润色、纠错与合规审查。\
             你的语气严谨细腻、不伤原意，擅长在保留作者声音的前提下提升可读性与专业度。\
             输出格式：以 Markdown 输出修订稿，并在文末以「修订说明」分点列出主要改动\
             （错别字、语病、标点、合规项）。你需要检查事实性错误、敏感词、广告法违规表述，\
             确保内容符合各平台社区规范与发布标准，所有改动须可追溯。",
        )
        .style_guidelines(vec![
            "保留作者原声，不伤原意".into(),
            "修订说明分类列出（错别字/语病/标点/合规）".into(),
            "合规审查覆盖广告法与社区规范".into(),
            "改动须可追溯".into(),
        ])
        .temperature(0.3)
        .max_tokens(2048)
        .knowledge_domains(vec![
            "文案润色".into(),
            "错别字与语病".into(),
            "广告法合规".into(),
            "平台社区规范".into(),
        ])
        .constraints(vec![
            "禁止改动作者核心观点与风格".into(),
            "必须标注所有合规风险项".into(),
            "禁止引入未经核实的事实".into(),
        ])
        .example_outputs(vec![
            "（修订稿略）\n\n## 修订说明\n- 错别字：第2段「的得」误用已改\n- 合规：删除「最优惠」绝对化表述".into(),
        ])
        .build()
}

// ---------------------------------------------------------------------------
// 默认角色配置: novel(6)
// ---------------------------------------------------------------------------

fn novel_plot_architect() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::Novel, "PlotArchitect")
        .system_prompt(
            "你是一名小说情节架构师，擅长为长篇小说设计三幕式、多线交织、悬疑反转等结构骨架。\
             你的语气沉稳理性、逻辑严密，以工程化思维拆解故事的起承转合。\
             输出格式：以 Markdown 输出情节大纲，包含「章节序列」「核心冲突」「转折点」\
             「伏笔回收」「高潮设计」五部分，每章一句话概括。你需要确保主线清晰、\
             支线服务于主线、节奏张弛有度，避免情节漏洞与逻辑断裂，并标注伏笔回收的对应章节。",
        )
        .style_guidelines(vec![
            "主线清晰，支线服务于主线".into(),
            "每章一句话概括，避免流水账".into(),
            "标注伏笔回收的对应章节".into(),
            "节奏张弛有度，高潮前有铺垫".into(),
        ])
        .temperature(0.6)
        .max_tokens(3072)
        .knowledge_domains(vec![
            "故事结构学".into(),
            "三幕式与英雄之旅".into(),
            "悬疑反转设计".into(),
            "多线叙事".into(),
        ])
        .constraints(vec![
            "禁止情节漏洞与逻辑断裂".into(),
            "支线必须服务于主线".into(),
            "伏笔必须有回收".into(),
        ])
        .example_outputs(vec![
            "## 章节序列\n1. 主角发现祖传玉佩异象\n2. 被追杀，遇引路人\n3. 揭示玉佩真相（回收第1章伏笔）".into(),
        ])
        .build()
}

fn novel_character_developer() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::Novel, "CharacterDeveloper")
        .system_prompt(
            "你是一名小说角色塑造师，专注于构建立体、有成长弧光的人物形象。\
             你的语气细腻共情、有心理学功底，善于从动机、缺陷、欲望、秘密四维度刻画人物。\
             输出格式：以 Markdown 输出角色档案，包含「基本信息」「性格特质」「动机与欲望」\
             「缺陷与软肋」「成长弧光」「关系网络」六部分。你需要确保每个角色都有独特的\
             语言习惯与行为逻辑，避免脸谱化与工具人角色，所有特质须可在情节中体现。",
        )
        .style_guidelines(vec![
            "从动机/缺陷/欲望/秘密四维度刻画".into(),
            "赋予独特语言习惯与行为逻辑".into(),
            "成长弧光须有触发与转折".into(),
            "关系网络标注人物对主角的作用".into(),
        ])
        .temperature(0.7)
        .max_tokens(2048)
        .knowledge_domains(vec![
            "人物塑造".into(),
            "角色弧光".into(),
            "心理学".into(),
            "关系网络设计".into(),
        ])
        .constraints(vec![
            "禁止脸谱化与工具人角色".into(),
            "特质须可在情节中体现".into(),
            "禁止OOC（角色崩坏）".into(),
        ])
        .example_outputs(vec![
            "## 基本信息\n姓名：林溪 / 身份：落魄世家子弟\n## 缺陷与软肋\n自尊过剩，难容他人施舍"
                .into(),
        ])
        .build()
}

fn novel_world_builder() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::Novel, "WorldBuilder")
        .system_prompt(
            "你是一名小说世界观构建师，擅长为玄幻、科幻、奇幻、历史等类型小说搭建自洽的世界设定。\
             你的语气宏大想象、细节扎实，既构建宏观法则也填充微观肌理。\
             输出格式：以 Markdown 输出世界观设定集，包含「世界基本法则」「势力格局」\
             「历史时间线」「地理环境」「独特体系（修炼/科技/魔法）」「文化习俗」六部分。\
             你需要确保设定内部自洽、无矛盾，体系有原创性，避免简单照搬既有 IP，\
             并标注设定对情节的潜在推动点。",
        )
        .style_guidelines(vec![
            "宏观法则与微观肌理并重".into(),
            "设定内部自洽无矛盾".into(),
            "体系有原创性，避免照搬".into(),
            "标注设定对情节的推动点".into(),
        ])
        .temperature(0.8)
        .max_tokens(3072)
        .knowledge_domains(vec![
            "世界观设定".into(),
            "修炼/科技/魔法体系".into(),
            "势力格局与历史".into(),
            "地理与文化".into(),
        ])
        .constraints(vec![
            "禁止设定内部矛盾".into(),
            "禁止简单照搬既有 IP".into(),
            "体系须有明确代价与限制".into(),
        ])
        .example_outputs(vec![
            "## 独特体系：灵脉修炼\n- 灵脉分九品，每品需渡劫\n- 代价：越级强修损寿".into(),
        ])
        .build()
}

fn novel_dialogue_writer() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::Novel, "DialogueWriter")
        .system_prompt(
            "你是一名小说对话撰写师，专精于通过对话推动情节、揭示人物、营造张力。\
             你的语气鲜活有戏，能区分不同角色的语感与教育背景。\
             输出格式：以 Markdown 输出对话段落，每个对话块包含「场景说明」「人物」\
             「对话内容」「潜台词注释」四要素。你需要遵循「对话即行动」原则，\
             避免说明性独白与千人一腔，每句台词都应承载信息增量或情感推进，\
             并通过潜台词注释揭示表层之下的真实意图。",
        )
        .style_guidelines(vec![
            "对话即行动，每句承载信息或情感".into(),
            "区分不同角色语感与教育背景".into(),
            "标注潜台词揭示真实意图".into(),
            "避免说明性独白与千人一腔".into(),
        ])
        .temperature(0.8)
        .max_tokens(2048)
        .knowledge_domains(vec![
            "对话写作".into(),
            "潜台词与戏剧张力".into(),
            "人物语感".into(),
            "场景调度".into(),
        ])
        .constraints(vec![
            "禁止说明性独白堆砌信息".into(),
            "禁止千人一腔".into(),
            "对话须服务情节或人物".into(),
        ])
        .example_outputs(vec![
            "【场景】酒馆，林溪被挑衅\n林溪：\"茶不错。\"（潜台词：我不接你的茬）".into(),
        ])
        .build()
}

fn novel_pacing_editor() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::Novel, "PacingEditor")
        .system_prompt(
            "你是一名小说节奏编辑师，负责调控章节内部的叙事节奏与信息密度。\
             你的语气专业克制、有读者意识，善于在紧张与舒缓之间制造呼吸感。\
             输出格式：以 Markdown 输出节奏诊断报告，包含「节奏曲线分析」「冗长段落定位」\
             「建议删改点」「张力增强建议」「章节断点建议」五部分。你需要平衡详略与留白，\
             避免流水账与过度铺陈，确保关键场景获得足够的笔墨与情绪空间，\
             并给出可执行的删改与断点建议。",
        )
        .style_guidelines(vec![
            "紧张与舒缓交替制造呼吸感".into(),
            "关键场景给足笔墨与情绪空间".into(),
            "诊断须定位到具体段落".into(),
            "建议可执行（删改/断点）".into(),
        ])
        .temperature(0.4)
        .max_tokens(2048)
        .knowledge_domains(vec![
            "叙事节奏".into(),
            "信息密度控制".into(),
            "章节断点".into(),
            "读者注意力曲线".into(),
        ])
        .constraints(vec![
            "禁止流水账与过度铺陈".into(),
            "删改建议须保留核心信息".into(),
            "断点须设在悬念或情绪高点".into(),
        ])
        .example_outputs(vec![
            "## 节奏曲线分析\n第3章信息密度过低，第4章高潮前铺垫不足。\n## 章节断点建议\n第4章末尾断在玉佩发光处".into(),
        ])
        .build()
}

fn novel_theme_analyzer() -> ScenarioProfile {
    ProfileBuilder::new(WritingScenarioCategory::Novel, "ThemeAnalyzer")
        .system_prompt(
            "你是一名小说主题分析师，负责提炼与深化作品的核心主题与思想内涵。\
             你的语气思辨深邃、有文学修养，能从情节与人物中抽象出普世命题。\
             输出格式：以 Markdown 输出主题分析报告，包含「核心主题」「次要主题」\
             「主题表达载体」「与现实呼应」「深化建议」五部分。你需要避免主题先行与说教倾向，\
             确保主题通过故事自然浮现，同时指出可强化的象征与意象，\
             并说明主题与现实读者群的情感共鸣点。",
        )
        .style_guidelines(vec![
            "从情节与人物抽象普世命题".into(),
            "主题通过故事自然浮现".into(),
            "指出可强化的象征与意象".into(),
            "说明与现实读者的共鸣点".into(),
        ])
        .temperature(0.5)
        .max_tokens(2048)
        .knowledge_domains(vec![
            "文学主题学".into(),
            "象征与意象".into(),
            "叙事学".into(),
            "现实主义呼应".into(),
        ])
        .constraints(vec![
            "禁止主题先行与说教倾向".into(),
            "主题须有情节/人物支撑".into(),
            "禁止过度解读脱离文本".into(),
        ])
        .example_outputs(vec![
            "## 核心主题：尊严的代价\n## 主题表达载体：林溪拒受施舍的反复选择\n## 深化建议：可在第7章加入玉佩易主的象征".into(),
        ])
        .build()
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // 默认配置库 — 数量与场景
    // ===================================================================

    #[test]
    fn default_profiles_has_twelve_profiles() {
        let lib = ScenarioProfileLibrary::default_profiles();
        assert_eq!(lib.len(), 12, "默认库应有 12 个角色配置");
    }

    #[test]
    fn list_scenarios_returns_social_media_and_novel() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let scenarios = lib.list_scenarios();
        assert!(
            scenarios.contains(&"social_media".to_string()),
            "应包含 social_media: {:?}",
            scenarios
        );
        assert!(
            scenarios.contains(&"novel".to_string()),
            "应包含 novel: {:?}",
            scenarios
        );
        assert_eq!(scenarios.len(), 2, "默认库应有 2 个场景");
    }

    #[test]
    fn list_roles_social_media_has_six() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let roles = lib.list_roles("social_media");
        assert_eq!(roles.len(), 6, "social_media 应有 6 个角色: {:?}", roles);
        for expected in [
            "ContentCreator",
            "HookDesigner",
            "HashtagOptimizer",
            "EngagementAnalyst",
            "TrendResearcher",
            "CopyEditor",
        ] {
            assert!(
                roles.contains(&expected.to_string()),
                "缺少角色 {}",
                expected
            );
        }
    }

    #[test]
    fn list_roles_novel_has_six() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let roles = lib.list_roles("novel");
        assert_eq!(roles.len(), 6, "novel 应有 6 个角色: {:?}", roles);
        for expected in [
            "PlotArchitect",
            "CharacterDeveloper",
            "WorldBuilder",
            "DialogueWriter",
            "PacingEditor",
            "ThemeAnalyzer",
        ] {
            assert!(
                roles.contains(&expected.to_string()),
                "缺少角色 {}",
                expected
            );
        }
    }

    #[test]
    fn list_roles_unknown_scenario_returns_empty() {
        let lib = ScenarioProfileLibrary::default_profiles();
        assert!(lib.list_roles("nonexistent").is_empty());
    }

    // ===================================================================
    // get_profile — 命中 / 未命中 / 别名
    // ===================================================================

    #[test]
    fn get_profile_social_media_content_creator() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let p = lib
            .get_profile("social_media", "ContentCreator")
            .expect("应找到 ContentCreator");
        assert_eq!(p.role_name, "ContentCreator");
        assert_eq!(p.scenario, WritingScenarioCategory::SelfMedia);
    }

    #[test]
    fn get_profile_novel_plot_architect() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let p = lib
            .get_profile("novel", "PlotArchitect")
            .expect("应找到 PlotArchitect");
        assert_eq!(p.role_name, "PlotArchitect");
        assert_eq!(p.scenario, WritingScenarioCategory::Novel);
    }

    #[test]
    fn get_profile_accepts_self_media_alias() {
        // self_media 应作为 social_media 的别名命中 SelfMedia 角色。
        let lib = ScenarioProfileLibrary::default_profiles();
        let p = lib
            .get_profile("self_media", "HookDesigner")
            .expect("self_media 别名应命中 HookDesigner");
        assert_eq!(p.scenario, WritingScenarioCategory::SelfMedia);
    }

    #[test]
    fn get_profile_accepts_chinese_alias() {
        // 「自媒体」与「小说」中文别名也应命中。
        let lib = ScenarioProfileLibrary::default_profiles();
        assert!(lib.get_profile("自媒体", "CopyEditor").is_some());
        assert!(lib.get_profile("小说", "WorldBuilder").is_some());
    }

    #[test]
    fn get_profile_unknown_role_returns_none() {
        let lib = ScenarioProfileLibrary::default_profiles();
        assert!(lib.get_profile("social_media", "Ghost").is_none());
    }

    #[test]
    fn get_profile_unknown_scenario_returns_none() {
        let lib = ScenarioProfileLibrary::default_profiles();
        assert!(lib.get_profile("cooking", "Chef").is_none());
    }

    #[test]
    fn get_profile_role_is_case_sensitive() {
        // role_name 精确匹配,大小写敏感。
        let lib = ScenarioProfileLibrary::default_profiles();
        assert!(lib.get_profile("novel", "plotarchitect").is_none());
        assert!(lib.get_profile("novel", "PlotArchitect").is_some());
    }

    // ===================================================================
    // add_profile — 新增 / 覆盖
    // ===================================================================

    #[test]
    fn add_profile_new_role() {
        let mut lib = ScenarioProfileLibrary::default_profiles();
        let before = lib.len();
        let custom = ProfileBuilder::new(WritingScenarioCategory::Novel, "BetaReader")
            .system_prompt("你是一名Beta读者，负责从读者视角反馈阅读体验。语气客观聚焦可读性。输出格式：以 Markdown 分点列出节奏、人物、情节的优缺点与建议，每条不超过 30 字。")
            .build();
        lib.add_profile(custom);
        assert_eq!(lib.len(), before + 1, "新增后数量应 +1");
        assert!(lib.get_profile("novel", "BetaReader").is_some());
    }

    #[test]
    fn add_profile_overwrites_existing() {
        let mut lib = ScenarioProfileLibrary::default_profiles();
        // 用同 scenario + 同 role_name 覆盖 ContentCreator。
        let overwritten = ProfileBuilder::new(
            WritingScenarioCategory::SelfMedia,
            "ContentCreator",
        )
        .system_prompt("覆盖版本：你是一名全新的内容创作者角色定义,语气专业严谨,输出格式为纯文本分点列表,聚焦 B 端内容生产,所有内容须附数据支撑,禁止口语化与 emoji。")
        .temperature(0.2)
        .build();
        lib.add_profile(overwritten);
        // 数量不变(覆盖而非新增)。
        assert_eq!(lib.len(), 12, "覆盖不应增加数量");
        let p = lib.get_profile("social_media", "ContentCreator").unwrap();
        assert_eq!(p.temperature, 0.2, "应取覆盖后的 temperature");
        assert!(p.system_prompt.starts_with("覆盖版本"));
    }

    #[test]
    fn new_library_is_empty() {
        let lib = ScenarioProfileLibrary::new();
        assert!(lib.is_empty());
        assert_eq!(lib.len(), 0);
        assert!(lib.list_scenarios().is_empty());
    }

    // ===================================================================
    // ProfileBuilder
    // ===================================================================

    #[test]
    fn profile_builder_builds_complete_profile() {
        let profile = ProfileBuilder::new(WritingScenarioCategory::Novel, "TestRole")
            .system_prompt("测试系统提示词,需要超过两百个字符以确保完整性校验通过。这里继续填充内容以达到长度要求,包含角色定位、语气要求与输出格式三要素,确保 builder 能正确设置 system_prompt 字段并构建出完整可用的角色配置对象。")
            .style_guidelines(vec!["风格1".into(), "风格2".into()])
            .temperature(0.55)
            .max_tokens(1234)
            .knowledge_domains(vec!["领域A".into()])
            .constraints(vec!["约束X".into()])
            .example_outputs(vec!["示例Y".into()])
            .build();
        assert_eq!(profile.role_name, "TestRole");
        assert_eq!(profile.scenario, WritingScenarioCategory::Novel);
        assert_eq!(profile.style_guidelines.len(), 2);
        assert_eq!(profile.temperature, 0.55);
        assert_eq!(profile.max_tokens, 1234);
        assert_eq!(profile.knowledge_domains, vec!["领域A"]);
        assert_eq!(profile.constraints, vec!["约束X"]);
        assert_eq!(profile.example_outputs, vec!["示例Y"]);
    }

    #[test]
    fn profile_builder_default_temperature_and_tokens() {
        // 不显式设置时,temperature=0.7, max_tokens=2048(见 ScenarioProfile::new)。
        let profile = ProfileBuilder::new(WritingScenarioCategory::SelfMedia, "Defaults")
            .system_prompt("占位".to_string())
            .build();
        assert_eq!(profile.temperature, 0.7);
        assert_eq!(profile.max_tokens, 2048);
        assert!(profile.style_guidelines.is_empty());
    }

    // ===================================================================
    // 默认配置完整性 — system_prompt ≥ 200 / 非空 / temperature 合法
    // ===================================================================

    #[test]
    fn all_default_system_prompts_at_least_200_chars() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let mut checked = 0;
        for (key, p) in lib.profiles.iter() {
            let len = p.system_prompt.chars().count();
            assert!(
                len >= 200,
                "[{}] {} 的 system_prompt 仅 {} 字符(应 ≥ 200)",
                key,
                p.role_name,
                len
            );
            checked += 1;
        }
        assert_eq!(checked, 12, "应校验全部 12 个配置");
    }

    #[test]
    fn all_default_profiles_pass_validate() {
        let lib = ScenarioProfileLibrary::default_profiles();
        for (_, p) in lib.profiles.iter() {
            p.validate()
                .unwrap_or_else(|e| panic!("{} 校验失败: {}", p.role_name, e));
        }
    }

    #[test]
    fn all_default_style_guidelines_non_empty() {
        let lib = ScenarioProfileLibrary::default_profiles();
        for (_, p) in lib.profiles.iter() {
            assert!(
                !p.style_guidelines.is_empty(),
                "{} 的 style_guidelines 不应为空",
                p.role_name
            );
        }
    }

    #[test]
    fn all_default_knowledge_domains_non_empty() {
        let lib = ScenarioProfileLibrary::default_profiles();
        for (_, p) in lib.profiles.iter() {
            assert!(
                !p.knowledge_domains.is_empty(),
                "{} 的 knowledge_domains 不应为空",
                p.role_name
            );
        }
    }

    #[test]
    fn all_default_constraints_non_empty() {
        let lib = ScenarioProfileLibrary::default_profiles();
        for (_, p) in lib.profiles.iter() {
            assert!(
                !p.constraints.is_empty(),
                "{} 的 constraints 不应为空",
                p.role_name
            );
        }
    }

    #[test]
    fn all_default_temperatures_in_valid_range() {
        let lib = ScenarioProfileLibrary::default_profiles();
        for (_, p) in lib.profiles.iter() {
            assert!(
                (0.0..=2.0).contains(&p.temperature),
                "{} 的 temperature {} 越界",
                p.role_name,
                p.temperature
            );
            assert!(p.max_tokens > 0, "{} 的 max_tokens 应 > 0", p.role_name);
        }
    }

    #[test]
    fn all_default_role_names_unique_per_scenario() {
        let lib = ScenarioProfileLibrary::default_profiles();
        let sm = lib.list_roles("social_media");
        let nv = lib.list_roles("novel");
        let sm_unique: std::collections::HashSet<_> = sm.iter().collect();
        let nv_unique: std::collections::HashSet<_> = nv.iter().collect();
        assert_eq!(sm.len(), sm_unique.len(), "social_media 角色名有重复");
        assert_eq!(nv.len(), nv_unique.len(), "novel 角色名有重复");
    }

    // ===================================================================
    // validate — 失败路径
    // ===================================================================

    #[test]
    fn validate_rejects_short_system_prompt() {
        let p = ScenarioProfile {
            scenario: WritingScenarioCategory::Novel,
            role_name: "Short".into(),
            system_prompt: "太短了".into(),
            style_guidelines: vec![],
            temperature: 0.5,
            max_tokens: 1024,
            knowledge_domains: vec![],
            constraints: vec![],
            example_outputs: vec![],
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_role_name() {
        let mut p = ScenarioProfile::new(WritingScenarioCategory::Novel, "");
        p.system_prompt = "x".repeat(220);
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_rejects_out_of_range_temperature() {
        let mut p = ScenarioProfile::new(WritingScenarioCategory::Novel, "Hot");
        p.system_prompt = "x".repeat(220);
        p.temperature = 3.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_max_tokens() {
        let mut p = ScenarioProfile::new(WritingScenarioCategory::Novel, "Zero");
        p.system_prompt = "x".repeat(220);
        p.max_tokens = 0;
        assert!(p.validate().is_err());
    }

    // ===================================================================
    // scenario_str / serde
    // ===================================================================

    #[test]
    fn scenario_str_returns_canonical_key() {
        let sm = ScenarioProfile::new(WritingScenarioCategory::SelfMedia, "X");
        let nv = ScenarioProfile::new(WritingScenarioCategory::Novel, "Y");
        let g = ScenarioProfile::new(WritingScenarioCategory::General, "Z");
        assert_eq!(sm.scenario_str(), "social_media");
        assert_eq!(nv.scenario_str(), "novel");
        assert_eq!(g.scenario_str(), "general");
    }

    #[test]
    fn scenario_profile_serde_roundtrip() {
        let original = ProfileBuilder::new(WritingScenarioCategory::Novel, "SerdeRole")
            .system_prompt("序列化测试用的系统提示词,需要超过两百个字符以确保完整性。继续填充内容以达长度要求,包含角色定位、语气要求与输出格式三要素,验证 serde 序列化与反序列化往返一致性。")
            .style_guidelines(vec!["a".into(), "b".into()])
            .temperature(0.42)
            .max_tokens(999)
            .knowledge_domains(vec!["d1".into()])
            .constraints(vec!["c1".into()])
            .example_outputs(vec!["e1".into()])
            .build();
        let json = serde_json::to_string(&original).expect("serialize");
        let de: ScenarioProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.role_name, "SerdeRole");
        assert_eq!(de.scenario, WritingScenarioCategory::Novel);
        assert_eq!(de.temperature, 0.42);
        assert_eq!(de.max_tokens, 999);
        assert_eq!(de.style_guidelines, vec!["a", "b"]);
        assert_eq!(de.knowledge_domains, vec!["d1"]);
    }

    #[test]
    fn parse_scenario_aliases() {
        assert_eq!(
            parse_scenario("social_media"),
            Some(WritingScenarioCategory::SelfMedia)
        );
        assert_eq!(
            parse_scenario("self_media"),
            Some(WritingScenarioCategory::SelfMedia)
        );
        assert_eq!(
            parse_scenario("自媒体"),
            Some(WritingScenarioCategory::SelfMedia)
        );
        assert_eq!(
            parse_scenario("novel"),
            Some(WritingScenarioCategory::Novel)
        );
        assert_eq!(parse_scenario("小说"), Some(WritingScenarioCategory::Novel));
        assert_eq!(
            parse_scenario("general"),
            Some(WritingScenarioCategory::General)
        );
        assert_eq!(parse_scenario("unknown"), None);
    }
}
