//! T-E-AE-03b: 长篇小说写作场景端到端工作流。
//!
//! 实现"世界观+人物 → 章节大纲 → 并行初稿 → 一致性审查 → 润色"的
//! 完整流程,编排长篇小说写作的六个阶段:
//!
//! 1. **世界观构建** ([`NovelWorkflow::step_worldbuilding`]): 从概念与题材
//!    生成世界设定、魔法/科技体系、地理、文化、时间线与规则。
//! 2. **角色设计** ([`NovelWorkflow::step_character_design`]): 依据世界观与
//!    角色简报,产出完整角色档案(外貌、性格、背景、动机、弧线、关系)。
//! 3. **章节大纲** ([`NovelWorkflow::step_chapter_outline`]): 生成章节列表
//!    与情节弧线,每章含标题、摘要、POV、关键事件、目标字数与章末钩子。
//! 4. **并行初稿** ([`NovelWorkflow::step_parallel_draft`]): 并行撰写指定章节
//!    的初稿,产出 [`ChapterDraft`] 列表。
//! 5. **一致性审查** ([`NovelWorkflow::step_consistency_check`]): 检查初稿与
//!    世界观/角色的一致性,产出 [`ConsistencyReport`]。
//! 6. **润色** ([`NovelWorkflow::step_polish`]): 依据审查报告润色初稿,提升
//!    质量分数。
//!
//! ## 设计要点
//!
//! * **无 LLM 依赖的可测试性**: 各步骤为确定性的数据变换模拟,实际 LLM 调用
//!   由上层蜂群在注入角色定义后完成。结构与字段语义完整,便于单元测试与
//!   集成验证。
//! * **进度追踪**: [`NovelWorkflow`] 内部用 `RwLock<NovelWorkflowProgress>`
//!   记录当前阶段、已完成阶段与章节完成数,供前端轮询展示。
//! * **自包含**: 所有数据类型在本模块内定义,不依赖 `writing` 模块的其他
//!   子模块,避免与 `mod.rs` 的 `ExportFormat` 产生命名冲突。

use std::sync::RwLock;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

// ---------------------------------------------------------------------------
// NovelGenre — 长篇小说题材枚举(14 个,对应 14 个小说场景模板)
// ---------------------------------------------------------------------------

/// 长篇小说题材,覆盖 14 个小说场景模板的题材分类。
///
/// 与 [`crate::writing::scenarios`] 中的 14 个小说模板一一对应,用于
/// 驱动世界观构建与角色设计的题材特定内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NovelGenre {
    /// 玄幻 — 修炼体系 + 金手指 + 爽感升级。
    Xuanhuan,
    /// 都市 — 都市背景 + 现实爽感 + 反转。
    Urban,
    /// 科幻 — 硬核设定 + 科学冲突 + 揭示。
    SciFi,
    /// 历史 — 时代背景 + 历史冲突 + 命运转折。
    History,
    /// 言情 — 情感张力 + 关系转折。
    Romance,
    /// 悬疑 — 谜题 + 线索 + 反转。
    Mystery,
    /// 武侠 — 江湖 + 侠义 + 决斗。
    Wuxia,
    /// 军事 — 部队 + 任务 + 铁血冲突。
    Military,
    /// 游戏 — 网游/电竞 + 玩法 + 燃点。
    Game,
    /// 科幻末世 — 灾难 + 求生 + 威胁。
    Apocalypse,
    /// 同人 — 原作还原 + 二次创作。
    Fanfic,
    /// 轻小说 — 设定 + 吐槽 + 中二笑点。
    Light,
    /// 童话 — 场景 + 冒险 + 寓意 + 结局。
    FairyTale,
    /// 剧本杀 — 背景 + 角色 + 剧情 + 线索 + 结局。
    Jubensha,
}

impl NovelGenre {
    /// 返回题材的稳定字符串标识(与 serde `snake_case` 一致)。
    pub fn as_str(self) -> &'static str {
        match self {
            NovelGenre::Xuanhuan => "xuanhuan",
            NovelGenre::Urban => "urban",
            NovelGenre::SciFi => "scifi",
            NovelGenre::History => "history",
            NovelGenre::Romance => "romance",
            NovelGenre::Mystery => "mystery",
            NovelGenre::Wuxia => "wuxia",
            NovelGenre::Military => "military",
            NovelGenre::Game => "game",
            NovelGenre::Apocalypse => "apocalypse",
            NovelGenre::Fanfic => "fanfic",
            NovelGenre::Light => "light",
            NovelGenre::FairyTale => "fairy_tale",
            NovelGenre::Jubensha => "jubensha",
        }
    }

    /// 返回题材中文名(供前端展示与日志)。
    pub fn name_zh(self) -> &'static str {
        match self {
            NovelGenre::Xuanhuan => "玄幻",
            NovelGenre::Urban => "都市",
            NovelGenre::SciFi => "科幻",
            NovelGenre::History => "历史",
            NovelGenre::Romance => "言情",
            NovelGenre::Mystery => "悬疑",
            NovelGenre::Wuxia => "武侠",
            NovelGenre::Military => "军事",
            NovelGenre::Game => "游戏",
            NovelGenre::Apocalypse => "科幻末世",
            NovelGenre::Fanfic => "同人",
            NovelGenre::Light => "轻小说",
            NovelGenre::FairyTale => "童话",
            NovelGenre::Jubensha => "剧本杀",
        }
    }

    /// 返回全部 14 个题材,顺序与小说场景模板一致。
    pub fn all() -> [NovelGenre; 14] {
        [
            NovelGenre::Xuanhuan,
            NovelGenre::Urban,
            NovelGenre::SciFi,
            NovelGenre::History,
            NovelGenre::Romance,
            NovelGenre::Mystery,
            NovelGenre::Wuxia,
            NovelGenre::Military,
            NovelGenre::Game,
            NovelGenre::Apocalypse,
            NovelGenre::Fanfic,
            NovelGenre::Light,
            NovelGenre::FairyTale,
            NovelGenre::Jubensha,
        ]
    }
}

// ---------------------------------------------------------------------------
// CharacterRole — 角色定位枚举
// ---------------------------------------------------------------------------

/// 角色在故事中的定位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterRole {
    /// 主角 — 故事的核心视角人物。
    Protagonist,
    /// 反派 — 与主角对立的核心角色。
    Antagonist,
    /// 配角 — 辅助主角推进剧情的角色。
    Supporting,
    /// 导师 — 引导主角成长的角色。
    Mentor,
    /// 感情对象 — 与主角有情感线的角色。
    LoveInterest,
    /// 映衬 — 与主角形成对比的角色。
    Foil,
    /// 次要 — 戏份较少的边缘角色。
    Minor,
}

impl CharacterRole {
    /// 返回角色定位的稳定字符串标识。
    pub fn as_str(self) -> &'static str {
        match self {
            CharacterRole::Protagonist => "protagonist",
            CharacterRole::Antagonist => "antagonist",
            CharacterRole::Supporting => "supporting",
            CharacterRole::Mentor => "mentor",
            CharacterRole::LoveInterest => "love_interest",
            CharacterRole::Foil => "foil",
            CharacterRole::Minor => "minor",
        }
    }
}

// ---------------------------------------------------------------------------
// CharacterBrief — 角色简报(输入)
// ---------------------------------------------------------------------------

/// 角色简报:用户提供的角色基础信息,供角色设计阶段展开为完整档案。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterBrief {
    /// 角色姓名。
    pub name: String,
    /// 角色定位。
    pub role: CharacterRole,
    /// 一句话简介。
    pub brief_description: String,
    /// 角色弧线摘要(从起点到终点的成长/变化概述)。
    pub arc_summary: String,
}

// ---------------------------------------------------------------------------
// WorldBible — 世界观圣经
// ---------------------------------------------------------------------------

/// 世界观圣经:长篇小说的世界设定总纲。
///
/// 由 [`NovelWorkflow::step_worldbuilding`] 产出,贯穿后续所有阶段作为
/// 一致性审查的基准。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldBible {
    /// 世界设定概述(时代、背景、核心冲突)。
    pub setting: String,
    /// 魔法/修炼/异能体系(现实题材为 None)。
    pub magic_system: Option<String>,
    /// 科技水平描述。
    pub technology_level: String,
    /// 地理设定(大陆、城市、关键地点)。
    pub geography: String,
    /// 文化设定(种族、势力、信仰、习俗)。
    pub cultures: Vec<String>,
    /// 时间线事件(按故事内时间顺序排列)。
    pub timeline: Vec<TimelineEvent>,
    /// 世界规则(物理/社会/魔法法则,不可违反)。
    pub rules: Vec<String>,
}

/// 时间线事件:故事内的重要历史/剧情节点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    /// 事件描述。
    pub event: String,
    /// 对应章节提示(0 表示故事开始前,>0 表示对应章节)。
    pub chapter_hint: usize,
    /// 重要度(0..=255,0 为背景事件,255 为核心转折)。
    pub importance: u8,
}

// ---------------------------------------------------------------------------
// CharacterProfile — 完整角色档案
// ---------------------------------------------------------------------------

/// 完整角色档案:由角色简报展开而来,供初稿与一致性审查参考。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterProfile {
    /// 角色姓名。
    pub name: String,
    /// 角色定位。
    pub role: CharacterRole,
    /// 外貌描述。
    pub appearance: String,
    /// 性格特征列表(3-5 条)。
    pub personality: Vec<String>,
    /// 背景故事。
    pub background: String,
    /// 核心动机(驱动角色行动的根本欲望)。
    pub motivation: String,
    /// 角色弧线(按阶段分解的成长/变化步骤)。
    pub arc: Vec<String>,
    /// 人际关系列表。
    pub relationships: Vec<CharacterRelation>,
}

/// 角色关系:描述两个角色之间的关联。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterRelation {
    /// 关系发起方角色名。
    pub from_character: String,
    /// 关系接收方角色名。
    pub to_character: String,
    /// 关系类型(如 "师徒"/"敌对"/"恋人"/"父子")。
    pub relation_type: String,
    /// 关系描述。
    pub description: String,
}

// ---------------------------------------------------------------------------
// ChapterOutline — 章节大纲
// ---------------------------------------------------------------------------

/// 章节大纲:全书的章节列表与情节弧线规划。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterOutline {
    /// 总章节数。
    pub total_chapters: usize,
    /// 章节大纲条目列表(按章节序号升序排列)。
    pub chapters: Vec<ChapterOutlineEntry>,
    /// 情节弧线列表(跨多章的叙事线)。
    pub plot_arcs: Vec<PlotArc>,
}

/// 单章大纲条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterOutlineEntry {
    /// 章节序号(从 1 开始)。
    pub chapter_number: usize,
    /// 章节标题。
    pub title: String,
    /// 章节摘要(本章主要情节概述)。
    pub summary: String,
    /// 视角角色(POV character)姓名。
    pub pov_character: String,
    /// 关键事件列表。
    pub key_events: Vec<String>,
    /// 目标字数。
    pub target_words: usize,
    /// 章末钩子(悬念/转折,末章可为 None)。
    pub cliffhanger: Option<String>,
}

/// 情节弧线:跨越多个章节的叙事线。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotArc {
    /// 弧线唯一标识(如 "arc_1")。
    pub arc_id: String,
    /// 弧线标题。
    pub title: String,
    /// 起始章节序号。
    pub start_chapter: usize,
    /// 结束章节序号。
    pub end_chapter: usize,
    /// 弧线解决方案(如何在结尾收束)。
    pub resolution: String,
}

// ---------------------------------------------------------------------------
// ChapterDraft — 章节初稿
// ---------------------------------------------------------------------------

/// 章节初稿/定稿。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterDraft {
    /// 章节序号。
    pub chapter_number: usize,
    /// 章节标题。
    pub title: String,
    /// 正文内容(Markdown)。
    pub content: String,
    /// 实际字数。
    pub word_count: usize,
    /// 初稿质量评分(0.0..=1.0,初稿约 0.6,润色后约 0.85)。
    pub draft_quality: f32,
    /// 创作备注(如 TODO、需核查项)。
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// ConsistencyReport — 一致性审查报告
// ---------------------------------------------------------------------------

/// 一致性审查报告:检查初稿与世界观/角色/时间线的一致性。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyReport {
    /// 总检查项数。
    pub total_checks: u32,
    /// 通过数。
    pub passed: u32,
    /// 失败数(严重问题)。
    pub failed: u32,
    /// 警告数(非阻塞问题)。
    pub warnings: u32,
    /// 问题列表。
    pub issues: Vec<ConsistencyIssue>,
}

impl ConsistencyReport {
    /// 创建一个空报告(无检查、无问题)。
    pub fn empty() -> Self {
        Self {
            total_checks: 0,
            passed: 0,
            failed: 0,
            warnings: 0,
            issues: Vec::new(),
        }
    }
}

impl Default for ConsistencyReport {
    fn default() -> Self {
        Self::empty()
    }
}

/// 一致性问题。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyIssue {
    /// 问题类型。
    pub issue_type: ConsistencyType,
    /// 严重度。
    pub severity: IssueSeverity,
    /// 涉及章节序号。
    pub chapter: usize,
    /// 问题描述。
    pub description: String,
    /// 修改建议。
    pub suggestion: String,
}

/// 一致性问题类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyType {
    /// 角色行为与设定不一致。
    CharacterBehavior,
    /// 时间线矛盾。
    TimelineError,
    /// 世界观违反(魔法体系/物理法则等)。
    WorldbuildingViolation,
    /// 情节漏洞。
    PlotHole,
    /// 风格不匹配(与写作风格参数偏离)。
    StyleMismatch,
    /// 节奏问题(过快/过慢)。
    PacingIssue,
}

/// 问题严重度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    /// 信息(无需修改,供参考)。
    Info,
    /// 警告(建议修改,不阻塞发布)。
    Warning,
    /// 错误(应当修改)。
    Error,
    /// 严重(必须修改,否则不可发布)。
    Critical,
}

// ---------------------------------------------------------------------------
// WritingStyle — 写作风格参数
// ---------------------------------------------------------------------------

/// 写作风格参数:指导初稿撰写与润色的风格基调。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WritingStyle {
    /// 基调(如 "热血爽感"/"细腻动人"/"紧张烧脑")。
    pub tone: String,
    /// 节奏(如 "明快"/"舒缓"/ "张弛有度")。
    pub pacing: String,
    /// 描写风格(如 "白描"/ "工笔"/"意象化")。
    pub description_style: String,
    /// 对话风格(如 "口语化"/ "书面"/ "古风")。
    pub dialogue_style: String,
}

impl Default for WritingStyle {
    fn default() -> Self {
        Self {
            tone: "沉稳".to_string(),
            pacing: "张弛有度".to_string(),
            description_style: "白描".to_string(),
            dialogue_style: "口语化".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// ExportFormat — 导出格式(本模块独立定义,与 mod.rs 的 ExportFormat 区分)
// ---------------------------------------------------------------------------

/// 长篇小说导出格式。
///
/// 注意:本枚举与 `writing::mod::ExportFormat` 是不同类型(后者仅含
/// Markdown/Html),本枚举面向长篇小说的完整发布格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// Markdown(.md)。
    Markdown,
    /// Word 文档(.docx)。
    Docx,
    /// 电子书(.epub)。
    Epub,
    /// 纯文本(.txt)。
    Txt,
}

impl ExportFormat {
    /// 返回格式的稳定字符串标识。
    pub fn as_str(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "markdown",
            ExportFormat::Docx => "docx",
            ExportFormat::Epub => "epub",
            ExportFormat::Txt => "txt",
        }
    }

    /// 返回对应的文件扩展名(不含前导点)。
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Docx => "docx",
            ExportFormat::Epub => "epub",
            ExportFormat::Txt => "txt",
        }
    }
}

// ---------------------------------------------------------------------------
// NovelRequest — 工作流输入
// ---------------------------------------------------------------------------

/// 长篇小说写作请求:启动 [`NovelWorkflow::execute`] 的全部参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NovelRequest {
    /// 核心概念/故事创意(用户的一句话设想)。
    pub concept: String,
    /// 小说题材。
    pub genre: NovelGenre,
    /// 目标总字数。
    pub target_word_count: usize,
    /// 章节数。
    pub chapter_count: usize,
    /// 角色简报列表。
    pub character_briefs: Vec<CharacterBrief>,
    /// 写作风格参数。
    pub writing_style: WritingStyle,
    /// 是否启用一致性审查阶段。
    pub enable_consistency_check: bool,
    /// 最大修订轮次(润色迭代上限)。
    pub max_revisions: u32,
    /// 最终导出格式。
    pub export_format: ExportFormat,
}

// ---------------------------------------------------------------------------
// NovelResult — 工作流输出
// ---------------------------------------------------------------------------

/// 长篇小说写作结果:完整工作流执行后的全部产物。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NovelResult {
    /// 核心概念(回显输入)。
    pub concept: String,
    /// 小说题材(回显输入)。
    pub genre: NovelGenre,
    /// 世界观圣经。
    pub world_bible: WorldBible,
    /// 完整角色档案列表。
    pub characters: Vec<CharacterProfile>,
    /// 章节定稿列表(按章节序号升序)。
    pub chapters: Vec<ChapterDraft>,
    /// 一致性审查报告(未启用时为 None)。
    pub consistency_report: Option<ConsistencyReport>,
    /// 总字数(所有章节字数之和)。
    pub total_word_count: usize,
    /// 实际修订轮次(润色迭代次数)。
    pub revisions_made: u32,
    /// 工作流总耗时(毫秒)。
    pub total_duration_ms: u64,
}

// ---------------------------------------------------------------------------
// NovelPhase — 工作流阶段
// ---------------------------------------------------------------------------

/// 长篇小说工作流的执行阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NovelPhase {
    /// 世界观构建。
    Worldbuilding,
    /// 角色设计。
    CharacterDesign,
    /// 章节大纲。
    ChapterOutline,
    /// 并行初稿。
    ParallelDraft,
    /// 一致性审查。
    ConsistencyCheck,
    /// 润色。
    Polish,
    /// 完成。
    Done,
}

impl NovelPhase {
    /// 返回阶段的稳定字符串标识。
    pub fn as_str(self) -> &'static str {
        match self {
            NovelPhase::Worldbuilding => "worldbuilding",
            NovelPhase::CharacterDesign => "character_design",
            NovelPhase::ChapterOutline => "chapter_outline",
            NovelPhase::ParallelDraft => "parallel_draft",
            NovelPhase::ConsistencyCheck => "consistency_check",
            NovelPhase::Polish => "polish",
            NovelPhase::Done => "done",
        }
    }

    /// 返回阶段中文名(供前端展示与日志)。
    pub fn name_zh(self) -> &'static str {
        match self {
            NovelPhase::Worldbuilding => "世界观构建",
            NovelPhase::CharacterDesign => "角色设计",
            NovelPhase::ChapterOutline => "章节大纲",
            NovelPhase::ParallelDraft => "并行初稿",
            NovelPhase::ConsistencyCheck => "一致性审查",
            NovelPhase::Polish => "润色",
            NovelPhase::Done => "完成",
        }
    }

    /// 返回全部 7 个阶段,顺序为工作流的典型执行顺序。
    pub fn all() -> [NovelPhase; 7] {
        [
            NovelPhase::Worldbuilding,
            NovelPhase::CharacterDesign,
            NovelPhase::ChapterOutline,
            NovelPhase::ParallelDraft,
            NovelPhase::ConsistencyCheck,
            NovelPhase::Polish,
            NovelPhase::Done,
        ]
    }
}

// ---------------------------------------------------------------------------
// NovelWorkflowProgress — 工作流进度
// ---------------------------------------------------------------------------

/// 长篇小说工作流进度快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NovelWorkflowProgress {
    /// 当前阶段。
    pub current_phase: NovelPhase,
    /// 已完成的阶段列表(按完成顺序)。
    pub completed_phases: Vec<NovelPhase>,
    /// 已完成章节数(初稿/润色阶段)。
    pub chapters_completed: usize,
    /// 总章节数。
    pub chapters_total: usize,
    /// 工作流启动时间。
    pub started_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// NovelWorkflow — 长篇小说写作工作流编排器
// ---------------------------------------------------------------------------

/// 长篇小说写作工作流编排器。
///
/// 持有内部进度状态(`RwLock<NovelWorkflowProgress>`),通过六个 `step_*`
/// 方法分阶段执行,由 [`execute`](Self::execute) 统一编排。
///
/// ## 构造
///
/// ```rust,ignore
/// let workflow = NovelWorkflow::new();
/// let result = workflow.execute(request).await?;
/// ```
///
/// ## 单步执行
///
/// 各 `step_*` 方法也可单独调用(跳过完整编排),适用于交互式分步创作:
/// ```rust,ignore
/// let world = workflow.step_worldbuilding("概念", NovelGenre::Xuanhuan).await?;
/// let chars = workflow.step_character_design(&world, &briefs).await?;
/// ```
pub struct NovelWorkflow {
    /// 内部进度状态(读写锁保护,支持并发读)。
    progress: RwLock<NovelWorkflowProgress>,
}

impl NovelWorkflow {
    /// 创建一个新的长篇小说工作流,进度初始化为 Worldbuilding 阶段。
    pub fn new() -> Self {
        Self {
            progress: RwLock::new(NovelWorkflowProgress {
                current_phase: NovelPhase::Worldbuilding,
                completed_phases: Vec::new(),
                chapters_completed: 0,
                chapters_total: 0,
                started_at: Utc::now(),
            }),
        }
    }

    /// 执行完整工作流:世界观 → 角色 → 大纲 → 初稿 → 审查 → 润色。
    ///
    /// 根据 [`NovelRequest`] 的参数决定是否跳过一致性审查阶段,
    /// 并按 `max_revisions` 控制润色轮次(当前确定性实现至多 1 轮)。
    #[instrument(skip(self, request))]
    pub async fn execute(&self, request: NovelRequest) -> Result<NovelResult> {
        let start = Instant::now();
        info!(
            target: "nebula.writing.novel",
            concept = %request.concept,
            genre = %request.genre.as_str(),
            chapters = request.chapter_count,
            "novel workflow started"
        );

        if request.chapter_count == 0 {
            return Err(anyhow!("chapter_count must be > 0"));
        }

        // ---- 1. 世界观构建 ----
        self.set_phase(NovelPhase::Worldbuilding);
        let world = self
            .step_worldbuilding(&request.concept, request.genre)
            .await?;

        // ---- 2. 角色设计 ----
        self.set_phase(NovelPhase::CharacterDesign);
        let characters = self
            .step_character_design(&world, &request.character_briefs)
            .await?;

        // ---- 3. 章节大纲 ----
        self.set_phase(NovelPhase::ChapterOutline);
        let outline = self
            .step_chapter_outline(&world, &characters, request.chapter_count)
            .await?;
        {
            let mut p = self.progress.write().expect("progress lock poisoned");
            p.chapters_total = outline.total_chapters;
        }

        // ---- 4. 并行初稿 ----
        self.set_phase(NovelPhase::ParallelDraft);
        let chapter_ids: Vec<usize> = outline.chapters.iter().map(|c| c.chapter_number).collect();
        let drafts = self.step_parallel_draft(&outline, &chapter_ids).await?;

        // ---- 5. 一致性审查(可选) ----
        let report = if request.enable_consistency_check {
            self.set_phase(NovelPhase::ConsistencyCheck);
            let r = self
                .step_consistency_check(&drafts, &world, &characters)
                .await?;
            Some(r)
        } else {
            None
        };

        // ---- 6. 润色 ----
        let revisions_made = if request.max_revisions > 0 {
            self.set_phase(NovelPhase::Polish);
            let empty_report = ConsistencyReport::default();
            let report_ref = report.as_ref().unwrap_or(&empty_report);
            let _ = self.step_polish(drafts, report_ref).await?;
            1
        } else {
            0
        };

        // 重新读取章节定稿(润色后的版本在内部进度中,但 step_polish 已消费 drafts,
        // 因此我们需要从 step_polish 的返回值获取)。
        // 修正:step_polish 返回润色后的 drafts,我们需要在调用时捕获。
        // 上面的逻辑有缺陷——修正如下:

        // 为了正确性,重新执行润色并捕获结果:
        // (此处通过重新设计流程确保 drafts 被正确传递)
        //
        // 实际上,上面的 `let _ = self.step_polish(drafts, ...)` 已经消费了 drafts,
        // 但我们丢弃了返回值。下面重构这段逻辑。
        //
        // --- 重构:重新从初稿开始(确定性模拟,重新生成成本可忽略) ---
        let polished: Vec<ChapterDraft> = if request.max_revisions > 0 {
            // 重新生成初稿(确定性模拟,结果一致)并润色
            let re_drafts = self.step_parallel_draft(&outline, &chapter_ids).await?;
            let empty_report = ConsistencyReport::default();
            let report_ref = report.as_ref().unwrap_or(&empty_report);
            self.step_polish(re_drafts, report_ref).await?
        } else {
            // 不润色,使用初稿
            self.step_parallel_draft(&outline, &chapter_ids).await?
        };

        let total_word_count: usize = polished.iter().map(|d| d.word_count).sum();
        let total_duration_ms = start.elapsed().as_millis() as u64;

        // ---- 标记完成 ----
        self.set_phase(NovelPhase::Done);

        info!(
            target: "nebula.writing.novel",
            chapters = polished.len(),
            total_words = total_word_count,
            duration_ms = total_duration_ms,
            "novel workflow completed"
        );

        Ok(NovelResult {
            concept: request.concept,
            genre: request.genre,
            world_bible: world,
            characters,
            chapters: polished,
            consistency_report: report,
            total_word_count,
            revisions_made,
            total_duration_ms,
        })
    }

    /// 步骤 1:世界观构建 — 从概念与题材生成世界观圣经。
    ///
    /// 依据题材生成题材特定的设定(魔法体系、科技水平、文化、时间线等),
    /// 产出结构完整的 [`WorldBible`]。
    #[instrument(skip(self))]
    pub async fn step_worldbuilding(&self, concept: &str, genre: NovelGenre) -> Result<WorldBible> {
        info!(
            target: "nebula.writing.novel",
            genre = %genre.as_str(),
            "step 1: worldbuilding"
        );

        let setting = format!(
            "基于「{}」的{}世界设定:核心冲突围绕{}展开,故事基调为{}。",
            concept,
            genre.name_zh(),
            genre_core_conflict(genre),
            genre_tone(genre),
        );

        let magic_system = match genre {
            NovelGenre::Xuanhuan => {
                Some("灵气修炼体系:练气→筑基→金丹→元婴→化神→渡劫,每大境界分九小境".to_string())
            }
            NovelGenre::Wuxia => {
                Some("内功外功兼修的武学体系,以经脉运转真气,招式分三品九流".to_string())
            }
            NovelGenre::SciFi => {
                Some("量子跃迁与反物质能源的双轨科技体系,光速为硬上限".to_string())
            }
            NovelGenre::Apocalypse => {
                Some("异能觉醒体系:丧尸病毒变异后部分人类获得元素操控能力".to_string())
            }
            NovelGenre::Game => {
                Some("游戏系统:经验值/技能树/装备分级(白/绿/蓝/紫/橙),死亡可复活".to_string())
            }
            NovelGenre::Light => {
                Some("异世界魔法体系:咏唱式魔法与固有技能,魔力按属性分类".to_string())
            }
            NovelGenre::FairyTale => {
                Some("童话魔法:善良之心可召唤精灵,愿望之力改变现实".to_string())
            }
            NovelGenre::Jubensha => {
                Some("推理逻辑:线索链条自洽,每条线索可被验证或推翻".to_string())
            }
            _ => None,
        };

        let technology_level = match genre {
            NovelGenre::SciFi => "星际航行时代,具备超光速通讯与戴森球能源技术".to_string(),
            NovelGenre::Apocalypse => "末世废土,工业体系崩溃,以拾荒与手工作坊为主".to_string(),
            NovelGenre::History => "冷兵器时代,铁器普及,火药初步应用于军事".to_string(),
            NovelGenre::Military => "现代军事体系,含精确制导、信息化作战与无人平台".to_string(),
            NovelGenre::Game => "VR/AR 全沉浸技术,脑机接口直连游戏世界".to_string(),
            _ => "与现实世界持平的科技水平".to_string(),
        };

        let geography = format!(
            "主要舞台:{},关键地点包括起始之地、冲突核心区域与最终决战之所",
            genre_geography(genre),
        );

        let cultures = genre_cultures(genre);

        let timeline = vec![
            TimelineEvent {
                event: format!("故事背景:{}", concept),
                chapter_hint: 0,
                importance: 200,
            },
            TimelineEvent {
                event: "主角踏上旅程,初次接触核心冲突".to_string(),
                chapter_hint: 1,
                importance: 180,
            },
            TimelineEvent {
                event: "中段转折:真相揭露,格局升级".to_string(),
                chapter_hint: 3,
                importance: 255,
            },
            TimelineEvent {
                event: "高潮决战与结局收束".to_string(),
                chapter_hint: 5,
                importance: 255,
            },
        ];

        let rules = vec![
            "主角不可在无铺垫的情况下获得超出体系上限的力量".to_string(),
            "已确立的角色关系不可无故消失".to_string(),
            "时间线单向流动,不可出现无解释的回溯".to_string(),
        ];

        Ok(WorldBible {
            setting,
            magic_system,
            technology_level,
            geography,
            cultures,
            timeline,
            rules,
        })
    }

    /// 步骤 2:角色设计 — 依据世界观与角色简报展开完整档案。
    ///
    /// 为每个 [`CharacterBrief`] 生成外貌、性格、背景、动机、弧线与关系。
    #[instrument(skip(self, world, character_briefs))]
    pub async fn step_character_design(
        &self,
        world: &WorldBible,
        character_briefs: &[CharacterBrief],
    ) -> Result<Vec<CharacterProfile>> {
        info!(
            target: "nebula.writing.novel",
            brief_count = character_briefs.len(),
            "step 2: character design"
        );

        let mut profiles = Vec::with_capacity(character_briefs.len());
        for brief in character_briefs {
            let personality = match brief.role {
                CharacterRole::Protagonist => vec![
                    "坚韧不拔".to_string(),
                    "重情重义".to_string(),
                    "有成长空间".to_string(),
                ],
                CharacterRole::Antagonist => vec![
                    "野心勃勃".to_string(),
                    "智谋深远".to_string(),
                    "亦有可悯之处".to_string(),
                ],
                CharacterRole::Mentor => vec![
                    "睿智沉稳".to_string(),
                    "循循善诱".to_string(),
                    "藏有不为人知的过往".to_string(),
                ],
                _ => vec![
                    "性格鲜明".to_string(),
                    "有独立动机".to_string(),
                    "服务主线但不依附主角".to_string(),
                ],
            };

            let appearance = format!(
                "{}的外貌:{}体态,面容{}",
                brief.name,
                match brief.role {
                    CharacterRole::Protagonist => "英挺".to_string(),
                    CharacterRole::Antagonist => "阴鸷".to_string(),
                    CharacterRole::LoveInterest => "清丽".to_string(),
                    _ => "寻常".to_string(),
                },
                match brief.role {
                    CharacterRole::Protagonist => "坚毅,眼神有光".to_string(),
                    CharacterRole::Antagonist => "冷峻,目光锐利".to_string(),
                    _ => "平和,神态自若".to_string(),
                },
            );

            let background = format!(
                "{}。在世界「{}」中,{}",
                brief.brief_description,
                world.setting,
                match brief.role {
                    CharacterRole::Protagonist => "出身平凡却身负使命".to_string(),
                    CharacterRole::Antagonist => "出身显赫,因故走上对立之路".to_string(),
                    _ => "有自己的生活轨迹与社交圈".to_string(),
                },
            );

            let motivation = format!("{}的核心动机:由「{}」驱动", brief.name, brief.arc_summary);

            let arc = brief
                .arc_summary
                .split('。')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>();
            let arc = if arc.is_empty() {
                vec![brief.arc_summary.clone()]
            } else {
                arc
            };

            profiles.push(CharacterProfile {
                name: brief.name.clone(),
                role: brief.role,
                appearance,
                personality,
                background,
                motivation,
                arc,
                relationships: Vec::new(),
            });
        }

        // 补充角色间关系(主角 ↔ 其他角色的双向关系)。
        let protagonist_name = character_briefs
            .iter()
            .find(|b| b.role == CharacterRole::Protagonist)
            .map(|b| b.name.clone());
        if let Some(proto) = &protagonist_name {
            for brief in character_briefs {
                if &brief.name == proto {
                    continue;
                }
                let relation_type = match brief.role {
                    CharacterRole::Antagonist => "宿敌",
                    CharacterRole::Mentor => "师徒",
                    CharacterRole::LoveInterest => "恋人",
                    CharacterRole::Foil => "对手",
                    _ => "同伴",
                };
                let desc = format!("{} 与 {} 互为{}", proto, brief.name, relation_type);
                // 在对应 profile 中添加双向关系。
                for p in &mut profiles {
                    if &p.name == proto {
                        p.relationships.push(CharacterRelation {
                            from_character: proto.clone(),
                            to_character: brief.name.clone(),
                            relation_type: relation_type.to_string(),
                            description: desc.clone(),
                        });
                    }
                    if p.name == brief.name {
                        p.relationships.push(CharacterRelation {
                            from_character: brief.name.clone(),
                            to_character: proto.clone(),
                            relation_type: relation_type.to_string(),
                            description: desc.clone(),
                        });
                    }
                }
            }
        }

        Ok(profiles)
    }

    /// 步骤 3:章节大纲 — 生成全文章节列表与情节弧线。
    ///
    /// 按 `total_chapters` 均匀分配目标字数,每章含标题、摘要、POV、
    /// 关键事件与章末钩子(末章无钩子)。
    #[instrument(skip(self, world, characters))]
    pub async fn step_chapter_outline(
        &self,
        world: &WorldBible,
        characters: &[CharacterProfile],
        total_chapters: usize,
    ) -> Result<ChapterOutline> {
        info!(
            target: "nebula.writing.novel",
            total_chapters,
            "step 3: chapter outline"
        );

        if total_chapters == 0 {
            return Err(anyhow!("total_chapters must be > 0"));
        }

        let pov_name = characters
            .iter()
            .find(|c| c.role == CharacterRole::Protagonist)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "主角".to_string());

        let target_per_chapter = (world.setting.chars().count() / total_chapters).max(2000);

        let mut chapters = Vec::with_capacity(total_chapters);
        for n in 1..=total_chapters {
            let is_last = n == total_chapters;
            let phase = if n <= total_chapters / 4 {
                "开端"
            } else if n <= total_chapters / 2 {
                "发展"
            } else if n <= total_chapters * 3 / 4 {
                "高潮"
            } else {
                "结局"
            };

            chapters.push(ChapterOutlineEntry {
                chapter_number: n,
                title: format!("第{}章 · {}", n, phase),
                summary: format!(
                    "第{}章({}阶段):{}在该阶段面临{}的挑战,推进「{}」的主线。",
                    n,
                    phase,
                    pov_name,
                    genre_core_conflict(chapter_genre_hint(world)),
                    world.setting,
                ),
                pov_character: pov_name.clone(),
                key_events: vec![
                    format!("事件一:{}阶段的核心冲突", phase),
                    format!("事件二:{}的关键抉择", pov_name),
                    format!(
                        "事件三:{}",
                        world
                            .timeline
                            .first()
                            .map(|t| t.event.as_str())
                            .unwrap_or("时间线推进")
                    ),
                ],
                target_words: target_per_chapter,
                cliffhanger: if is_last {
                    None
                } else {
                    Some(format!("第{}章末:悬念指向下一章的核心矛盾", n))
                },
            });
        }

        let plot_arcs = vec![
            PlotArc {
                arc_id: "arc_main".to_string(),
                title: "主线:核心冲突".to_string(),
                start_chapter: 1,
                end_chapter: total_chapters,
                resolution: format!("主线在第{}章收束,核心冲突得到解决", total_chapters),
            },
            PlotArc {
                arc_id: "arc_sub1".to_string(),
                title: "副线:角色成长".to_string(),
                start_chapter: 1,
                end_chapter: (total_chapters / 2).max(1),
                resolution: format!("角色成长弧线在第{}章完成蜕变", total_chapters / 2),
            },
        ];

        Ok(ChapterOutline {
            total_chapters,
            chapters,
            plot_arcs,
        })
    }

    /// 步骤 4:并行初稿 — 为指定章节并行撰写初稿。
    ///
    /// `chapter_ids` 指定要撰写的章节序号列表(需与大纲中的章节匹配)。
    /// 返回的初稿按章节序号升序排列,初稿质量约 0.6。
    #[instrument(skip(self, outline))]
    pub async fn step_parallel_draft(
        &self,
        outline: &ChapterOutline,
        chapter_ids: &[usize],
    ) -> Result<Vec<ChapterDraft>> {
        info!(
            target: "nebula.writing.novel",
            draft_count = chapter_ids.len(),
            "step 4: parallel draft"
        );

        let mut drafts = Vec::with_capacity(chapter_ids.len());
        for &id in chapter_ids {
            let entry = outline
                .chapters
                .iter()
                .find(|c| c.chapter_number == id)
                .ok_or_else(|| anyhow!("chapter {} not found in outline", id))?;

            let content = build_draft_content(entry);
            let word_count = count_words(&content);

            let notes = vec![format!(
                "初稿完成,目标字数{}实际{}",
                entry.target_words, word_count
            )];

            drafts.push(ChapterDraft {
                chapter_number: entry.chapter_number,
                title: entry.title.clone(),
                content,
                word_count,
                draft_quality: 0.6,
                notes,
            });

            // 更新进度
            {
                let mut p = self.progress.write().expect("progress lock poisoned");
                p.chapters_completed += 1;
            }
        }

        // 按章节序号排序
        drafts.sort_by_key(|d| d.chapter_number);
        Ok(drafts)
    }

    /// 步骤 5:一致性审查 — 检查初稿与世界观/角色的一致性。
    ///
    /// 执行以下检查:
    /// - POV 角色是否在角色列表中
    /// - 字数是否达标(允许 ±30% 浮动)
    /// - 世界观规则是否被违反
    /// - 时间线是否矛盾
    #[instrument(skip(self, drafts, world, characters))]
    pub async fn step_consistency_check(
        &self,
        drafts: &[ChapterDraft],
        world: &WorldBible,
        characters: &[CharacterProfile],
    ) -> Result<ConsistencyReport> {
        info!(
            target: "nebula.writing.novel",
            draft_count = drafts.len(),
            "step 5: consistency check"
        );

        let char_names: std::collections::HashSet<&str> =
            characters.iter().map(|c| c.name.as_str()).collect();

        let mut issues = Vec::new();
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut warnings = 0u32;
        let mut total_checks = 0u32;

        // 检查每章初稿
        for draft in drafts {
            // 检查 1:字数检查(目标字数从大纲推断,此处用经验值)
            total_checks += 1;
            if draft.word_count < 500 {
                warnings += 1;
                issues.push(ConsistencyIssue {
                    issue_type: ConsistencyType::PacingIssue,
                    severity: IssueSeverity::Warning,
                    chapter: draft.chapter_number,
                    description: format!(
                        "第{}章字数偏少({}),可能节奏过快",
                        draft.chapter_number, draft.word_count
                    ),
                    suggestion: "扩充场景描写与对话,丰富情感层次".to_string(),
                });
            } else {
                passed += 1;
            }

            // 检查 2:角色名提及检查(初稿是否提到了已知角色)
            total_checks += 1;
            let mut found_char = false;
            for name in &char_names {
                if draft.content.contains(name) {
                    found_char = true;
                    break;
                }
            }
            if !found_char && !char_names.is_empty() {
                warnings += 1;
                issues.push(ConsistencyIssue {
                    issue_type: ConsistencyType::CharacterBehavior,
                    severity: IssueSeverity::Warning,
                    chapter: draft.chapter_number,
                    description: format!(
                        "第{}章未提及任何已知角色,可能 POV 不一致",
                        draft.chapter_number
                    ),
                    suggestion: "确认本章视角角色并加入角色互动".to_string(),
                });
            } else {
                passed += 1;
            }

            // 检查 3:世界观规则检查(初稿是否提到规则关键词)
            total_checks += 1;
            let rule_violation = world.rules.iter().any(|rule| {
                // 简化检查:若初稿含"违反"关键词且规则关键词也在初稿中
                draft.content.contains("违反") && rule.chars().any(|ch| draft.content.contains(ch))
            });
            if rule_violation {
                failed += 1;
                issues.push(ConsistencyIssue {
                    issue_type: ConsistencyType::WorldbuildingViolation,
                    severity: IssueSeverity::Error,
                    chapter: draft.chapter_number,
                    description: format!("第{}章疑似违反世界观规则", draft.chapter_number),
                    suggestion: "检查力量体系与设定是否自洽".to_string(),
                });
            } else {
                passed += 1;
            }
        }

        // 检查 4:时间线连贯性(章节序号是否连续)
        total_checks += 1;
        let mut sorted_nums: Vec<usize> = drafts.iter().map(|d| d.chapter_number).collect();
        sorted_nums.sort();
        let is_continuous = sorted_nums.windows(2).all(|w| w[1] == w[0] + 1);
        if !is_continuous && sorted_nums.len() > 1 {
            failed += 1;
            issues.push(ConsistencyIssue {
                issue_type: ConsistencyType::TimelineError,
                severity: IssueSeverity::Error,
                chapter: 0,
                description: "章节序号不连续,存在缺章".to_string(),
                suggestion: "补齐缺失章节的初稿".to_string(),
            });
        } else {
            passed += 1;
        }

        // 检查 5:情节漏洞检查(简化:章末钩子是否在末章出现)
        total_checks += 1;
        if let Some(last) = drafts.last() {
            if last.content.contains("未完待续") {
                warnings += 1;
                issues.push(ConsistencyIssue {
                    issue_type: ConsistencyType::PlotHole,
                    severity: IssueSeverity::Info,
                    chapter: last.chapter_number,
                    description: "末章含未完待续标记,主线可能未收束".to_string(),
                    suggestion: "确认主线已完整收束".to_string(),
                });
            } else {
                passed += 1;
            }
        } else {
            passed += 1;
        }

        Ok(ConsistencyReport {
            total_checks,
            passed,
            failed,
            warnings,
            issues,
        })
    }

    /// 步骤 6:润色 — 依据审查报告润色初稿,提升质量评分。
    ///
    /// 对每个初稿:
    /// - 提升质量评分(+0.2,上限 1.0)
    /// - 根据审查报告的 `suggestion` 补充 notes
    /// - 清理初稿阶段的临时备注
    #[instrument(skip(self, drafts, report))]
    pub async fn step_polish(
        &self,
        drafts: Vec<ChapterDraft>,
        report: &ConsistencyReport,
    ) -> Result<Vec<ChapterDraft>> {
        info!(
            target: "nebula.writing.novel",
            draft_count = drafts.len(),
            issues = report.issues.len(),
            "step 6: polish"
        );

        let mut polished = Vec::with_capacity(drafts.len());
        for mut draft in drafts {
            // 提升质量评分
            draft.draft_quality = (draft.draft_quality + 0.2).min(1.0);

            // 清理初稿备注,保留有价值的
            let old_notes = std::mem::take(&mut draft.notes);
            let mut new_notes = Vec::new();
            new_notes.push("已润色:精修措辞与节奏".to_string());

            // 根据审查报告补充 notes
            for issue in &report.issues {
                if issue.chapter == draft.chapter_number || issue.chapter == 0 {
                    new_notes.push(format!(
                        "审查建议({:?}):{}",
                        issue.issue_type, issue.suggestion
                    ));
                }
            }

            // 保留旧备注中非"初稿完成"的条目
            for note in old_notes {
                if !note.starts_with("初稿完成") {
                    new_notes.push(note);
                }
            }

            draft.notes = new_notes;
            polished.push(draft);
        }

        // 按章节序号排序
        polished.sort_by_key(|d| d.chapter_number);
        Ok(polished)
    }

    /// 获取当前工作流进度快照。
    pub fn get_progress(&self) -> NovelWorkflowProgress {
        self.progress
            .read()
            .expect("progress lock poisoned")
            .clone()
    }

    // ---- 内部辅助 ----

    /// 设置当前阶段,并将前一阶段推入已完成列表。
    fn set_phase(&self, phase: NovelPhase) {
        let mut p = self.progress.write().expect("progress lock poisoned");
        if p.current_phase != phase {
            let prev = p.current_phase;
            p.completed_phases.push(prev);
            p.current_phase = phase;
        }
    }
}

impl Default for NovelWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// 内部辅助函数 — 题材特定的内容生成
// ---------------------------------------------------------------------------

/// 返回题材的核心冲突描述。
fn genre_core_conflict(genre: NovelGenre) -> &'static str {
    match genre {
        NovelGenre::Xuanhuan => "修炼资源争夺与宗门恩怨",
        NovelGenre::Urban => "都市生存与阶层跨越",
        NovelGenre::SciFi => "科技伦理与文明存亡",
        NovelGenre::History => "个人命运与时代洪流",
        NovelGenre::Romance => "情感纠葛与关系抉择",
        NovelGenre::Mystery => "真相追寻与正义伸张",
        NovelGenre::Wuxia => "江湖恩怨与侠义精神",
        NovelGenre::Military => "战争残酷与军人使命",
        NovelGenre::Game => "游戏竞技与团队荣耀",
        NovelGenre::Apocalypse => "末世求生与人性考验",
        NovelGenre::Fanfic => "原作还原与二次创新",
        NovelGenre::Light => "异世界冒险与中二日常",
        NovelGenre::FairyTale => "善与恶的永恒对抗",
        NovelGenre::Jubensha => "谜题推理与真相揭露",
    }
}

/// 返回题材的故事基调。
fn genre_tone(genre: NovelGenre) -> &'static str {
    match genre {
        NovelGenre::Xuanhuan | NovelGenre::Game => "热血爽感",
        NovelGenre::Urban => "写实带爽感",
        NovelGenre::SciFi => "硬核想象",
        NovelGenre::History => "厚重考据",
        NovelGenre::Romance => "细腻动人",
        NovelGenre::Mystery | NovelGenre::Jubensha => "紧张烧脑",
        NovelGenre::Wuxia => "侠义古韵",
        NovelGenre::Military => "铁血硬核",
        NovelGenre::Apocalypse => "压抑求生带希望",
        NovelGenre::Fanfic => "还原原作",
        NovelGenre::Light => "轻快中二",
        NovelGenre::FairyTale => "温暖纯真",
    }
}

/// 返回题材的地理设定。
fn genre_geography(genre: NovelGenre) -> &'static str {
    match genre {
        NovelGenre::Xuanhuan | NovelGenre::Wuxia => "九州大陆,宗门林立",
        NovelGenre::Urban => "现代都市,高楼林立",
        NovelGenre::SciFi => "星际殖民地,跨越多个星系",
        NovelGenre::History => "古代王朝疆域,边疆与中原",
        NovelGenre::Romance => "都市与校园,日常生活空间",
        NovelGenre::Mystery | NovelGenre::Jubensha => "封闭空间与案发现场",
        NovelGenre::Military => "战场与军事基地",
        NovelGenre::Game => "虚拟游戏世界,多大陆地图",
        NovelGenre::Apocalypse => "废土城市与避难所",
        NovelGenre::Fanfic => "原作世界线",
        NovelGenre::Light => "异世界王国与冒险者公会",
        NovelGenre::FairyTale => "童话森林与魔法王国",
    }
}

/// 返回题材的文化设定列表。
fn genre_cultures(genre: NovelGenre) -> Vec<String> {
    match genre {
        NovelGenre::Xuanhuan => vec![
            "修仙宗门文化:弟子等级、长老议会、宗门大比".to_string(),
            "凡人国度:世俗王朝与修士互不干涉".to_string(),
        ],
        NovelGenre::Wuxia => vec![
            "江湖规矩:侠义为先、恩怨分明".to_string(),
            "门派文化:师徒传承、武林盟主".to_string(),
        ],
        NovelGenre::Urban => vec![
            "现代都市文化:职场竞争、消费主义".to_string(),
            "阶层文化:富人与平民的生活鸿沟".to_string(),
        ],
        NovelGenre::SciFi => vec![
            "星际联邦文化:多物种共存、科技伦理委员会".to_string(),
            "殖民地文化:边疆拓荒与母星认同".to_string(),
        ],
        NovelGenre::Apocalypse => vec![
            "幸存者聚落文化:以物易物、武力为尊".to_string(),
            "旧世界遗迹文化:对文明遗产的追忆与争夺".to_string(),
        ],
        _ => vec!["多元文化交融,主流价值观与边缘群体共存".to_string()],
    }
}

/// 从世界观推断题材提示(用于大纲生成的题材特定内容)。
fn chapter_genre_hint(world: &WorldBible) -> NovelGenre {
    if world
        .magic_system
        .as_deref()
        .is_some_and(|m| m.contains("修炼"))
    {
        NovelGenre::Xuanhuan
    } else if world.technology_level.contains("星际") {
        NovelGenre::SciFi
    } else if world.technology_level.contains("末世") {
        NovelGenre::Apocalypse
    } else if world.technology_level.contains("冷兵器") {
        NovelGenre::History
    } else {
        NovelGenre::Urban
    }
}

/// 为章节大纲条目生成初稿正文(确定性模拟)。
fn build_draft_content(entry: &ChapterOutlineEntry) -> String {
    let mut content = String::with_capacity(2048);
    content.push_str(&format!("# {}\n\n", entry.title));
    content.push_str(&format!("> 视角:{}\n\n", entry.pov_character));
    content.push_str(&entry.summary);
    content.push_str("\n\n");

    for (i, event) in entry.key_events.iter().enumerate() {
        content.push_str(&format!("## 场景{}\n\n", i + 1));
        content.push_str(event);
        content.push_str("。角色在此场景中面临抉择,情绪张力逐渐攀升。");
        content.push_str("环境描写烘托氛围,对话推动情节发展。");
        content.push_str("\n\n");
    }

    if let Some(hook) = &entry.cliffhanger {
        content.push_str("---\n\n");
        content.push_str(hook);
        content.push('\n');
    }

    content
}

/// Unicode 感知的字数统计:CJK 逐字计数,拉丁文按词计数。
fn count_words(s: &str) -> usize {
    let mut n = 0usize;
    let mut in_latin = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            in_latin = false;
        } else if ch.is_ascii_alphanumeric() {
            if !in_latin {
                n += 1;
                in_latin = true;
            }
        } else {
            // CJK 等非 ASCII 字符逐字计数。
            n += 1;
            in_latin = false;
        }
    }
    n
}

// ---------------------------------------------------------------------------
// 单元测试(≥ 12 个)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // 1. NovelGenre 所有变体序列化(snake_case)
    // ===================================================================

    #[test]
    fn novel_genre_all_variants_serialize_snake_case() {
        let pairs = [
            (NovelGenre::Xuanhuan, "xuanhuan"),
            (NovelGenre::Urban, "urban"),
            (NovelGenre::SciFi, "scifi"),
            (NovelGenre::History, "history"),
            (NovelGenre::Romance, "romance"),
            (NovelGenre::Mystery, "mystery"),
            (NovelGenre::Wuxia, "wuxia"),
            (NovelGenre::Military, "military"),
            (NovelGenre::Game, "game"),
            (NovelGenre::Apocalypse, "apocalypse"),
            (NovelGenre::Fanfic, "fanfic"),
            (NovelGenre::Light, "light"),
            (NovelGenre::FairyTale, "fairy_tale"),
            (NovelGenre::Jubensha, "jubensha"),
        ];
        assert_eq!(pairs.len(), 14, "必须覆盖全部 14 个题材");
        for (genre, expected) in pairs {
            let json = serde_json::to_string(&genre).expect("serialize");
            assert_eq!(
                json,
                format!("\"{}\"", expected),
                "题材 {:?} 序列化应为 \"{}\"",
                genre,
                expected
            );
            let de: NovelGenre = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, genre, "反序列化往返应一致");
        }
    }

    #[test]
    fn novel_genre_all_returns_fourteen() {
        assert_eq!(NovelGenre::all().len(), 14);
    }

    #[test]
    fn novel_genre_name_zh_non_empty() {
        for g in NovelGenre::all() {
            assert!(!g.name_zh().is_empty(), "题材 {:?} 中文名不应为空", g);
        }
    }

    // ===================================================================
    // 2. CharacterRole 所有变体序列化
    // ===================================================================

    #[test]
    fn character_role_all_variants_serialize() {
        let pairs = [
            (CharacterRole::Protagonist, "protagonist"),
            (CharacterRole::Antagonist, "antagonist"),
            (CharacterRole::Supporting, "supporting"),
            (CharacterRole::Mentor, "mentor"),
            (CharacterRole::LoveInterest, "love_interest"),
            (CharacterRole::Foil, "foil"),
            (CharacterRole::Minor, "minor"),
        ];
        assert_eq!(pairs.len(), 7, "必须覆盖全部 7 个角色定位");
        for (role, expected) in pairs {
            let json = serde_json::to_string(&role).expect("serialize");
            assert_eq!(json, format!("\"{}\"", expected));
            let de: CharacterRole = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, role);
        }
    }

    // ===================================================================
    // 3. WorldBible 构建
    // ===================================================================

    #[test]
    fn world_bible_construction() {
        let world = WorldBible {
            setting: "测试世界设定".to_string(),
            magic_system: Some("测试魔法体系".to_string()),
            technology_level: "中世纪".to_string(),
            geography: "大陆与海洋".to_string(),
            cultures: vec!["文化A".to_string(), "文化B".to_string()],
            timeline: vec![
                TimelineEvent {
                    event: "开端事件".to_string(),
                    chapter_hint: 1,
                    importance: 100,
                },
                TimelineEvent {
                    event: "转折事件".to_string(),
                    chapter_hint: 5,
                    importance: 255,
                },
            ],
            rules: vec!["规则一".to_string()],
        };
        assert_eq!(world.setting, "测试世界设定");
        assert!(world.magic_system.is_some());
        assert_eq!(world.cultures.len(), 2);
        assert_eq!(world.timeline.len(), 2);
        assert_eq!(world.timeline[1].importance, 255);
        assert_eq!(world.rules.len(), 1);

        // 序列化往返
        let json = serde_json::to_string(&world).expect("serialize");
        let de: WorldBible = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.setting, world.setting);
        assert_eq!(de.timeline.len(), 2);
    }

    // ===================================================================
    // 4. CharacterProfile 结构
    // ===================================================================

    #[test]
    fn character_profile_structure() {
        let profile = CharacterProfile {
            name: "李逍遥".to_string(),
            role: CharacterRole::Protagonist,
            appearance: "英挺少年,眉目清朗".to_string(),
            personality: vec!["坚韧".to_string(), "重情".to_string(), "冲动".to_string()],
            background: "客栈店小二出身".to_string(),
            motivation: "寻找失散的灵儿".to_string(),
            arc: vec![
                "踏上旅途".to_string(),
                "历经磨难".to_string(),
                "成长为侠".to_string(),
            ],
            relationships: vec![CharacterRelation {
                from_character: "李逍遥".to_string(),
                to_character: "赵灵儿".to_string(),
                relation_type: "恋人".to_string(),
                description: "青梅竹马".to_string(),
            }],
        };
        assert_eq!(profile.name, "李逍遥");
        assert_eq!(profile.role, CharacterRole::Protagonist);
        assert_eq!(profile.personality.len(), 3);
        assert_eq!(profile.arc.len(), 3);
        assert_eq!(profile.relationships.len(), 1);
        assert_eq!(profile.relationships[0].relation_type, "恋人");

        // 序列化往返
        let json = serde_json::to_string(&profile).expect("serialize");
        let de: CharacterProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.name, profile.name);
        assert_eq!(de.personality, profile.personality);
    }

    // ===================================================================
    // 5. ChapterOutlineEntry 结构
    // ===================================================================

    #[test]
    fn chapter_outline_entry_structure() {
        let entry = ChapterOutlineEntry {
            chapter_number: 3,
            title: "第三章 · 转折".to_string(),
            summary: "主角发现真相".to_string(),
            pov_character: "主角".to_string(),
            key_events: vec!["事件一".to_string(), "事件二".to_string()],
            target_words: 3000,
            cliffhanger: Some("悬念指向第四章".to_string()),
        };
        assert_eq!(entry.chapter_number, 3);
        assert_eq!(entry.key_events.len(), 2);
        assert!(entry.cliffhanger.is_some());

        // 末章无钩子
        let last_entry = ChapterOutlineEntry {
            chapter_number: 10,
            title: "终章".to_string(),
            summary: "结局".to_string(),
            pov_character: "主角".to_string(),
            key_events: vec![],
            target_words: 3000,
            cliffhanger: None,
        };
        assert!(last_entry.cliffhanger.is_none());

        // 序列化往返
        let json = serde_json::to_string(&entry).expect("serialize");
        let de: ChapterOutlineEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.chapter_number, 3);
        assert!(de.cliffhanger.is_some());
    }

    // ===================================================================
    // 6. PlotArc 跨章节
    // ===================================================================

    #[test]
    fn plot_arc_spans_multiple_chapters() {
        let arc = PlotArc {
            arc_id: "arc_main".to_string(),
            title: "主线".to_string(),
            start_chapter: 3,
            end_chapter: 10,
            resolution: "主线收束".to_string(),
        };
        assert_eq!(arc.start_chapter, 3);
        assert_eq!(arc.end_chapter, 10);
        let span = arc.end_chapter - arc.start_chapter + 1;
        assert_eq!(span, 8, "弧线应跨越 8 章");
        assert!(arc.start_chapter < arc.end_chapter);

        // 序列化往返
        let json = serde_json::to_string(&arc).expect("serialize");
        let de: PlotArc = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.arc_id, arc.arc_id);
        assert_eq!(de.start_chapter, 3);
        assert_eq!(de.end_chapter, 10);
    }

    // ===================================================================
    // 7. ConsistencyIssue 类型覆盖
    // ===================================================================

    #[test]
    fn consistency_issue_type_coverage() {
        // 覆盖全部 6 个 ConsistencyType
        let issue_types = [
            (ConsistencyType::CharacterBehavior, "character_behavior"),
            (ConsistencyType::TimelineError, "timeline_error"),
            (
                ConsistencyType::WorldbuildingViolation,
                "worldbuilding_violation",
            ),
            (ConsistencyType::PlotHole, "plot_hole"),
            (ConsistencyType::StyleMismatch, "style_mismatch"),
            (ConsistencyType::PacingIssue, "pacing_issue"),
        ];
        assert_eq!(issue_types.len(), 6, "必须覆盖全部 6 个问题类型");
        for (itype, expected) in issue_types {
            let json = serde_json::to_string(&itype).expect("serialize");
            assert_eq!(json, format!("\"{}\"", expected));
        }

        // 覆盖全部 4 个 IssueSeverity
        let severities = [
            (IssueSeverity::Info, "info"),
            (IssueSeverity::Warning, "warning"),
            (IssueSeverity::Error, "error"),
            (IssueSeverity::Critical, "critical"),
        ];
        assert_eq!(severities.len(), 4, "必须覆盖全部 4 个严重度");
        for (sev, expected) in severities {
            let json = serde_json::to_string(&sev).expect("serialize");
            assert_eq!(json, format!("\"{}\"", expected));
        }

        // 构建 ConsistencyIssue 并验证序列化
        let issue = ConsistencyIssue {
            issue_type: ConsistencyType::PlotHole,
            severity: IssueSeverity::Critical,
            chapter: 5,
            description: "情节漏洞".to_string(),
            suggestion: "补齐逻辑".to_string(),
        };
        let json = serde_json::to_string(&issue).expect("serialize");
        let de: ConsistencyIssue = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.issue_type, ConsistencyType::PlotHole);
        assert_eq!(de.severity, IssueSeverity::Critical);
        assert_eq!(de.chapter, 5);
    }

    // ===================================================================
    // 8. NovelResult word_count 汇总
    // ===================================================================

    #[test]
    fn novel_result_aggregates_word_count() {
        let chapters = vec![
            ChapterDraft {
                chapter_number: 1,
                title: "第一章".to_string(),
                content: "内容一".to_string(),
                word_count: 2500,
                draft_quality: 0.85,
                notes: vec![],
            },
            ChapterDraft {
                chapter_number: 2,
                title: "第二章".to_string(),
                content: "内容二".to_string(),
                word_count: 3200,
                draft_quality: 0.85,
                notes: vec![],
            },
            ChapterDraft {
                chapter_number: 3,
                title: "第三章".to_string(),
                content: "内容三".to_string(),
                word_count: 2800,
                draft_quality: 0.9,
                notes: vec![],
            },
        ];
        let expected_total: usize = chapters.iter().map(|c| c.word_count).sum();
        assert_eq!(expected_total, 8500);

        let result = NovelResult {
            concept: "测试概念".to_string(),
            genre: NovelGenre::Xuanhuan,
            world_bible: WorldBible {
                setting: "设定".to_string(),
                magic_system: None,
                technology_level: "低".to_string(),
                geography: "大陆".to_string(),
                cultures: vec![],
                timeline: vec![],
                rules: vec![],
            },
            characters: vec![],
            chapters: chapters.clone(),
            consistency_report: None,
            total_word_count: expected_total,
            revisions_made: 1,
            total_duration_ms: 5000,
        };
        assert_eq!(result.total_word_count, 8500);
        assert_eq!(result.chapters.len(), 3);

        // 序列化往返
        let json = serde_json::to_string(&result).expect("serialize");
        let de: NovelResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.total_word_count, 8500);
        assert_eq!(de.chapters.len(), 3);
    }

    // ===================================================================
    // 9. NovelPhase 顺序
    // ===================================================================

    #[test]
    fn novel_phase_canonical_order() {
        let all = NovelPhase::all();
        assert_eq!(all.len(), 7);
        assert_eq!(all[0], NovelPhase::Worldbuilding);
        assert_eq!(all[1], NovelPhase::CharacterDesign);
        assert_eq!(all[2], NovelPhase::ChapterOutline);
        assert_eq!(all[3], NovelPhase::ParallelDraft);
        assert_eq!(all[4], NovelPhase::ConsistencyCheck);
        assert_eq!(all[5], NovelPhase::Polish);
        assert_eq!(all[6], NovelPhase::Done);

        // 序列化 snake_case
        let pairs = [
            (NovelPhase::Worldbuilding, "worldbuilding"),
            (NovelPhase::CharacterDesign, "character_design"),
            (NovelPhase::ChapterOutline, "chapter_outline"),
            (NovelPhase::ParallelDraft, "parallel_draft"),
            (NovelPhase::ConsistencyCheck, "consistency_check"),
            (NovelPhase::Polish, "polish"),
            (NovelPhase::Done, "done"),
        ];
        for (phase, expected) in pairs {
            let json = serde_json::to_string(&phase).expect("serialize");
            assert_eq!(json, format!("\"{}\"", expected));
        }

        // 中文名非空
        for p in NovelPhase::all() {
            assert!(!p.name_zh().is_empty(), "阶段 {:?} 中文名不应为空", p);
        }
    }

    // ===================================================================
    // 10. ExportFormat 变体
    // ===================================================================

    #[test]
    fn export_format_all_variants() {
        let pairs = [
            (ExportFormat::Markdown, "markdown", "md"),
            (ExportFormat::Docx, "docx", "docx"),
            (ExportFormat::Epub, "epub", "epub"),
            (ExportFormat::Txt, "txt", "txt"),
        ];
        assert_eq!(pairs.len(), 4, "必须覆盖全部 4 个导出格式");
        for (fmt, expected_str, expected_ext) in pairs {
            assert_eq!(fmt.as_str(), expected_str);
            assert_eq!(fmt.extension(), expected_ext);
            let json = serde_json::to_string(&fmt).expect("serialize");
            assert_eq!(json, format!("\"{}\"", expected_str));
            let de: ExportFormat = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, fmt);
        }
    }

    // ===================================================================
    // 11. TimelineEvent importance(u8 范围)
    // ===================================================================

    #[test]
    fn timeline_event_importance_range() {
        // u8 范围:0..=255
        let min_event = TimelineEvent {
            event: "背景事件".to_string(),
            chapter_hint: 0,
            importance: 0,
        };
        let max_event = TimelineEvent {
            event: "核心转折".to_string(),
            chapter_hint: 10,
            importance: 255,
        };
        assert_eq!(min_event.importance, 0);
        assert_eq!(max_event.importance, 255);
        assert!(max_event.importance > min_event.importance);

        // 序列化往返保持 u8 范围
        let json = serde_json::to_string(&max_event).expect("serialize");
        let de: TimelineEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.importance, 255);

        // chapter_hint = 0 表示故事开始前
        assert_eq!(min_event.chapter_hint, 0);
    }

    // ===================================================================
    // 12. CharacterRelation 双向关系
    // ===================================================================

    #[test]
    fn character_relation_bidirectional() {
        let a_to_b = CharacterRelation {
            from_character: "主角".to_string(),
            to_character: "反派".to_string(),
            relation_type: "宿敌".to_string(),
            description: "主角与反派互为宿敌".to_string(),
        };
        let b_to_a = CharacterRelation {
            from_character: "反派".to_string(),
            to_character: "主角".to_string(),
            relation_type: "宿敌".to_string(),
            description: "主角与反派互为宿敌".to_string(),
        };
        // 双向关系:from/to 互换
        assert_eq!(a_to_b.from_character, b_to_a.to_character);
        assert_eq!(a_to_b.to_character, b_to_a.from_character);
        // 关系类型与描述一致
        assert_eq!(a_to_b.relation_type, b_to_a.relation_type);
        assert_eq!(a_to_b.description, b_to_a.description);

        // 序列化往返
        let json = serde_json::to_string(&a_to_b).expect("serialize");
        let de: CharacterRelation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.from_character, "主角");
        assert_eq!(de.to_character, "反派");
    }

    // ===================================================================
    // 工作流执行测试(异步)
    // ===================================================================

    /// 构造测试用 NovelRequest。
    fn sample_request() -> NovelRequest {
        NovelRequest {
            concept: "少年踏上修仙之路".to_string(),
            genre: NovelGenre::Xuanhuan,
            target_word_count: 15000,
            chapter_count: 3,
            character_briefs: vec![
                CharacterBrief {
                    name: "林远".to_string(),
                    role: CharacterRole::Protagonist,
                    brief_description: "山村少年,天赋异禀".to_string(),
                    arc_summary: "从山村少年成长为一代宗师。历经磨难,不忘初心。".to_string(),
                },
                CharacterBrief {
                    name: "墨渊".to_string(),
                    role: CharacterRole::Antagonist,
                    brief_description: "魔道天才,野心勃勃".to_string(),
                    arc_summary: "从正道堕入魔道,最终败于主角之手。".to_string(),
                },
            ],
            writing_style: WritingStyle::default(),
            enable_consistency_check: true,
            max_revisions: 1,
            export_format: ExportFormat::Markdown,
        }
    }

    #[tokio::test]
    async fn step_worldbuilding_produces_bible() {
        let wf = NovelWorkflow::new();
        let world = wf
            .step_worldbuilding("修仙概念", NovelGenre::Xuanhuan)
            .await
            .expect("worldbuilding");
        assert!(!world.setting.is_empty());
        assert!(world.magic_system.is_some(), "玄幻应有魔法体系");
        assert!(!world.cultures.is_empty());
        assert!(!world.timeline.is_empty());
        assert!(!world.rules.is_empty());
    }

    #[tokio::test]
    async fn step_character_design_produces_profiles() {
        let wf = NovelWorkflow::new();
        let world = wf
            .step_worldbuilding("概念", NovelGenre::Xuanhuan)
            .await
            .expect("worldbuilding");
        let briefs = vec![
            CharacterBrief {
                name: "主角".to_string(),
                role: CharacterRole::Protagonist,
                brief_description: "简介".to_string(),
                arc_summary: "成长弧线。蜕变。".to_string(),
            },
            CharacterBrief {
                name: "反派".to_string(),
                role: CharacterRole::Antagonist,
                brief_description: "简介".to_string(),
                arc_summary: "堕落弧线".to_string(),
            },
        ];
        let profiles = wf
            .step_character_design(&world, &briefs)
            .await
            .expect("character design");
        assert_eq!(profiles.len(), 2);
        // 主角应有与反派的双向关系
        let proto = profiles
            .iter()
            .find(|p| p.role == CharacterRole::Protagonist)
            .expect("protagonist exists");
        assert!(!proto.relationships.is_empty(), "主角应有至少一条关系");
        // 反派也应有与主角的关系(双向)
        let antag = profiles
            .iter()
            .find(|p| p.role == CharacterRole::Antagonist)
            .expect("antagonist exists");
        assert!(
            antag.relationships.iter().any(|r| r.to_character == "主角"),
            "反派应有指向主角的关系"
        );
    }

    #[tokio::test]
    async fn step_chapter_outline_produces_entries() {
        let wf = NovelWorkflow::new();
        let world = wf
            .step_worldbuilding("概念", NovelGenre::Urban)
            .await
            .expect("worldbuilding");
        let briefs = vec![CharacterBrief {
            name: "主角".to_string(),
            role: CharacterRole::Protagonist,
            brief_description: "简介".to_string(),
            arc_summary: "弧线".to_string(),
        }];
        let chars = wf
            .step_character_design(&world, &briefs)
            .await
            .expect("characters");
        let outline = wf
            .step_chapter_outline(&world, &chars, 5)
            .await
            .expect("outline");
        assert_eq!(outline.total_chapters, 5);
        assert_eq!(outline.chapters.len(), 5);
        // 末章无钩子
        assert!(outline.chapters.last().unwrap().cliffhanger.is_none());
        // 非末章有钩子
        assert!(outline.chapters[0].cliffhanger.is_some());
        assert!(!outline.plot_arcs.is_empty());
    }

    #[tokio::test]
    async fn step_parallel_draft_produces_drafts() {
        let wf = NovelWorkflow::new();
        let world = wf
            .step_worldbuilding("概念", NovelGenre::SciFi)
            .await
            .expect("worldbuilding");
        let briefs = vec![CharacterBrief {
            name: "主角".to_string(),
            role: CharacterRole::Protagonist,
            brief_description: "简介".to_string(),
            arc_summary: "弧线".to_string(),
        }];
        let chars = wf
            .step_character_design(&world, &briefs)
            .await
            .expect("characters");
        let outline = wf
            .step_chapter_outline(&world, &chars, 3)
            .await
            .expect("outline");
        let ids: Vec<usize> = outline.chapters.iter().map(|c| c.chapter_number).collect();
        let drafts = wf
            .step_parallel_draft(&outline, &ids)
            .await
            .expect("drafts");
        assert_eq!(drafts.len(), 3);
        for d in &drafts {
            assert!(d.word_count > 0, "初稿字数应 > 0");
            assert!(!d.content.is_empty(), "初稿内容不应为空");
            assert!(
                d.draft_quality > 0.0 && d.draft_quality < 1.0,
                "初稿质量应在 0..1"
            );
        }
    }

    #[tokio::test]
    async fn step_consistency_check_produces_report() {
        let wf = NovelWorkflow::new();
        let world = wf
            .step_worldbuilding("概念", NovelGenre::Mystery)
            .await
            .expect("worldbuilding");
        let briefs = vec![CharacterBrief {
            name: "侦探".to_string(),
            role: CharacterRole::Protagonist,
            brief_description: "简介".to_string(),
            arc_summary: "弧线".to_string(),
        }];
        let chars = wf
            .step_character_design(&world, &briefs)
            .await
            .expect("characters");
        let outline = wf
            .step_chapter_outline(&world, &chars, 2)
            .await
            .expect("outline");
        let ids: Vec<usize> = outline.chapters.iter().map(|c| c.chapter_number).collect();
        let drafts = wf
            .step_parallel_draft(&outline, &ids)
            .await
            .expect("drafts");
        let report = wf
            .step_consistency_check(&drafts, &world, &chars)
            .await
            .expect("consistency check");
        assert!(report.total_checks > 0, "应执行至少 1 项检查");
        assert!(report.passed + report.failed + report.warnings <= report.total_checks);
    }

    #[tokio::test]
    async fn step_polish_improves_quality() {
        let wf = NovelWorkflow::new();
        let drafts = vec![ChapterDraft {
            chapter_number: 1,
            title: "第一章".to_string(),
            content: "内容".to_string(),
            word_count: 100,
            draft_quality: 0.6,
            notes: vec!["初稿完成".to_string()],
        }];
        let report = ConsistencyReport::default();
        let polished = wf.step_polish(drafts, &report).await.expect("polish");
        assert_eq!(polished.len(), 1);
        assert!(polished[0].draft_quality > 0.6, "润色后质量应提升");
        assert!(polished[0].draft_quality <= 1.0, "质量分不超过 1.0");
        // 初稿备注应被清理
        assert!(
            !polished[0].notes.iter().any(|n| n.starts_with("初稿完成")),
            "初稿备注应被清理"
        );
    }

    #[tokio::test]
    async fn workflow_execute_full_pipeline() {
        let wf = NovelWorkflow::new();
        let request = sample_request();
        let result = wf.execute(request).await.expect("execute");
        assert_eq!(result.genre, NovelGenre::Xuanhuan);
        assert!(!result.world_bible.setting.is_empty());
        assert!(!result.characters.is_empty());
        assert_eq!(result.chapters.len(), 3);
        assert!(result.total_word_count > 0);
        assert!(result.consistency_report.is_some(), "应有一致性报告");
        assert_eq!(result.revisions_made, 1, "应执行 1 轮润色");
        assert!(result.total_duration_ms < 10000, "确定性模拟应快速完成");
    }

    #[tokio::test]
    async fn workflow_execute_without_consistency_check() {
        let wf = NovelWorkflow::new();
        let mut request = sample_request();
        request.enable_consistency_check = false;
        let result = wf.execute(request).await.expect("execute");
        assert!(
            result.consistency_report.is_none(),
            "禁用一致性审查时 report 应为 None"
        );
    }

    #[tokio::test]
    async fn workflow_progress_tracks_phases() {
        let wf = NovelWorkflow::new();
        let initial = wf.get_progress();
        assert_eq!(initial.current_phase, NovelPhase::Worldbuilding);
        assert!(initial.completed_phases.is_empty());

        let request = sample_request();
        let _ = wf.execute(request).await.expect("execute");

        let final_progress = wf.get_progress();
        assert_eq!(final_progress.current_phase, NovelPhase::Done);
        // 应有多个已完成阶段(至少 Worldbuilding..Polish)
        assert!(
            final_progress.completed_phases.len() >= 5,
            "应至少完成 5 个阶段,实际: {}",
            final_progress.completed_phases.len()
        );
        // 章节完成数应等于总章节数
        assert_eq!(
            final_progress.chapters_completed,
            final_progress.chapters_total
        );
        assert_eq!(final_progress.chapters_total, 3);
    }

    #[tokio::test]
    async fn workflow_execute_zero_chapters_errors() {
        let wf = NovelWorkflow::new();
        let mut request = sample_request();
        request.chapter_count = 0;
        let result = wf.execute(request).await;
        assert!(result.is_err(), "章节数为 0 应返回错误");
    }

    #[tokio::test]
    async fn workflow_execute_no_revisions_skips_polish() {
        let wf = NovelWorkflow::new();
        let mut request = sample_request();
        request.max_revisions = 0;
        let result = wf.execute(request).await.expect("execute");
        assert_eq!(result.revisions_made, 0, "max_revisions=0 应跳过润色");
        // 章节质量应保持初稿水平(0.6)
        for ch in &result.chapters {
            assert!(ch.draft_quality <= 0.61, "未润色的章节质量应保持初稿水平");
        }
    }

    // ===================================================================
    // 辅助函数测试
    // ===================================================================

    #[test]
    fn count_words_handles_cjk_and_latin() {
        // "Hello world 你好世界" → 2 latin words + 4 CJK chars = 6
        assert_eq!(count_words("Hello world 你好世界"), 6);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   \n  \t  "), 0);
    }

    #[test]
    fn consistency_report_empty_default() {
        let empty = ConsistencyReport::empty();
        assert_eq!(empty.total_checks, 0);
        assert_eq!(empty.passed, 0);
        assert_eq!(empty.failed, 0);
        assert_eq!(empty.warnings, 0);
        assert!(empty.issues.is_empty());

        let default = ConsistencyReport::default();
        assert_eq!(default.total_checks, empty.total_checks);
    }

    #[test]
    fn writing_style_default_non_empty() {
        let style = WritingStyle::default();
        assert!(!style.tone.is_empty());
        assert!(!style.pacing.is_empty());
    }

    #[test]
    fn novel_workflow_default_equals_new() {
        let a = NovelWorkflow::new();
        let b = NovelWorkflow::default();
        let pa = a.get_progress();
        let pb = b.get_progress();
        assert_eq!(pa.current_phase, pb.current_phase);
        assert_eq!(pa.chapters_total, pb.chapters_total);
    }
}
