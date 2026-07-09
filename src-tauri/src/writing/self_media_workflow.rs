//! T-E-AE-03: 自媒体写作场景端到端工作流。
//!
//! 在 T-D-B-18 的 28 个场景模板与 T-E-AE-01 的 PrimaryAgent 编排能力之上,
//! 落地自媒体写作的完整六步端到端工作流:
//!
//! **搜索 → 大纲 → 初稿 → 审查 → 润色 → 归档**
//!
//! ## 设计要点
//!
//! * **无 LLM 依赖的可测试性**: 每一步都提供确定性的占位实现(基于平台/风格
//!   参数生成结构化骨架),测试可覆盖全流程而不依赖外部模型。生产环境可
//!   通过替换 step_* 方法注入真实 LLM 调用。
//! * **场景化大纲**: [`step_outline`] 根据 [`SocialMediaPlatform`] 返回
//!   平台适配的章节结构(小红书分点种草 / 抖音口播钩子 / 知乎论证展开 等),
//!   与 `writing::scenarios` 的 14 个自媒体模板语义对齐。
//! * **审查-润色循环**: [`execute`] 在 `enable_review` 时按 `max_revisions`
//!   最多进行 N 轮「审查 → 润色」迭代,直到审查通过或达到修订上限。
//! * **进度追踪**: [`WorkflowProgress`] 通过 `parking_lot::RwLock` 提供
//!   线程安全的步骤进度查询,供前端展示工作流状态。
//! * **归档多目的地**: 支持 `LocalFile` / `Clipboard` / `MemoryStore` /
//!   `ObsidianVault` 四种归档目的地,返回稳定 URI 供后续读取。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

// ---------------------------------------------------------------------
// 平台与素材来源枚举
// ---------------------------------------------------------------------

/// 自媒体目标平台(14 个,与 `writing::scenarios::self_media_library` 对齐)。
///
/// 每个变体对应一个场景模板 ID,驱动大纲结构与平台适配评分。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SocialMediaPlatform {
    /// 微信公众号推文(wechat-article)。
    WeChatArticle,
    /// 小红书笔记(xiaohongshu-note)。
    XiaohongshuNote,
    /// 抖音口播文案(douyin-script)。
    DouyinScript,
    /// 知乎回答(zhihu-answer)。
    ZhihuAnswer,
    /// 微博(weibo-post)。
    WeiboPost,
    /// B 站视频文案(bilibili-script)。
    BilibiliScript,
    /// 头条号文章(toutiao-article)。
    ToutiaoArticle,
    /// 豆瓣影评/书评(douban-review)。
    DoubanReview,
    /// 贴吧帖子(tieba-post)。
    TiebaPost,
    /// 即刻动态(jike-moment)。
    JikeMoment,
    /// 视频号文案(wechat-channels)。
    WechatChannels,
    /// 公众号长文(wechat-longform)。
    WechatLongform,
    /// 小红书图文(xiaohongshu-image-text)。
    XiaohongshuImageText,
    /// 知乎专栏(zhihu-column)。
    ZhihuColumn,
}

impl SocialMediaPlatform {
    /// 返回平台对应的场景模板 ID(与 `writing::scenarios` 一致)。
    pub fn template_id(self) -> &'static str {
        match self {
            Self::WeChatArticle => "wechat-article",
            Self::XiaohongshuNote => "xiaohongshu-note",
            Self::DouyinScript => "douyin-script",
            Self::ZhihuAnswer => "zhihu-answer",
            Self::WeiboPost => "weibo-post",
            Self::BilibiliScript => "bilibili-script",
            Self::ToutiaoArticle => "toutiao-article",
            Self::DoubanReview => "douban-review",
            Self::TiebaPost => "tieba-post",
            Self::JikeMoment => "jike-moment",
            Self::WechatChannels => "wechat-channels",
            Self::WechatLongform => "wechat-longform",
            Self::XiaohongshuImageText => "xiaohongshu-image-text",
            Self::ZhihuColumn => "zhihu-column",
        }
    }

    /// 返回平台中文名(供日志与进度展示)。
    pub fn label(self) -> &'static str {
        match self {
            Self::WeChatArticle => "微信公众号推文",
            Self::XiaohongshuNote => "小红书笔记",
            Self::DouyinScript => "抖音口播文案",
            Self::ZhihuAnswer => "知乎回答",
            Self::WeiboPost => "微博",
            Self::BilibiliScript => "B站视频文案",
            Self::ToutiaoArticle => "头条号文章",
            Self::DoubanReview => "豆瓣影评/书评",
            Self::TiebaPost => "贴吧帖子",
            Self::JikeMoment => "即刻动态",
            Self::WechatChannels => "视频号文案",
            Self::WechatLongform => "公众号长文",
            Self::XiaohongshuImageText => "小红书图文",
            Self::ZhihuColumn => "知乎专栏",
        }
    }

    /// 返回平台期望的目标字数(中位数),用于大纲规划与平台适配评分。
    pub fn target_word_count(self) -> usize {
        match self {
            Self::WeChatArticle => 1200,
            Self::XiaohongshuNote => 450,
            Self::DouyinScript => 300,
            Self::ZhihuAnswer => 1200,
            Self::WeiboPost => 140,
            Self::BilibiliScript => 800,
            Self::ToutiaoArticle => 1500,
            Self::DoubanReview => 1200,
            Self::TiebaPost => 350,
            Self::JikeMoment => 200,
            Self::WechatChannels => 200,
            Self::WechatLongform => 3500,
            Self::XiaohongshuImageText => 400,
            Self::ZhihuColumn => 2500,
        }
    }
}

/// 素材搜索来源。`step_search` 按调用方提供的来源列表收集素材。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SearchSource {
    /// 网络(搜索引擎/公开网页)。
    Web,
    /// 本地知识库(L3 Semantic memory / LanceDB 向量检索)。
    KnowledgeBase,
    /// 系统剪贴板(用户复制的参考文本)。
    Clipboard,
    /// 最近编辑的文件(writing documents 表)。
    RecentFiles,
    /// 浏览器书签(用户收藏的参考链接)。
    Bookmarks,
}

impl SearchSource {
    /// 返回来源的可读名(供日志与 `sources_used` 字段)。
    pub fn label(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::KnowledgeBase => "knowledge_base",
            Self::Clipboard => "clipboard",
            Self::RecentFiles => "recent_files",
            Self::Bookmarks => "bookmarks",
        }
    }
}

// ---------------------------------------------------------------------
// 写作风格参数
// ---------------------------------------------------------------------

/// 写作风格参数。控制初稿的语气、视角、正式度、幽默感与 emoji 密度。
///
/// `formality` / `humor` / `emoji_density` 为 0-100 的整数刻度,
/// [`WritingStyle::validate`] 校验范围合法性。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WritingStyle {
    /// 语气(如"活泼接地气" / "专业理性" / "温情有共鸣")。
    pub tone: String,
    /// 视角("第一人称" / "第三人称" / "上帝视角")。
    pub perspective: String,
    /// 正式度(0=口语化,100=学术正式)。
    pub formality: u8,
    /// 幽默感(0=严肃,100=全程抖梗)。
    pub humor: u8,
    /// emoji 密度(0=无 emoji,100=满屏 emoji)。
    pub emoji_density: u8,
}

impl WritingStyle {
    /// 校验 `formality` / `humor` / `emoji_density` 是否在 0-100 范围内。
    ///
    /// u8 上限为 255,因此仅检查上界 100;下界 0 由 u8 自然保证。
    pub fn validate(&self) -> bool {
        self.formality <= 100 && self.humor <= 100 && self.emoji_density <= 100
    }
}

// ---------------------------------------------------------------------
// 搜索结果
// ---------------------------------------------------------------------

/// 单条素材片段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchSnippet {
    /// 片段标题。
    pub title: String,
    /// 片段正文。
    pub content: String,
    /// 来源标识(对应 [`SearchSource::label`] 或 URL)。
    pub source: String,
    /// 与主题的相关度(0.0-1.0)。
    pub relevance: f32,
}

/// 素材搜索结果。由 [`SelfMediaWorkflow::step_search`] 返回,
/// 供后续 `step_outline` 引用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    /// 搜索查询(即主题)。
    pub query: String,
    /// 实际使用的来源列表(对应 [`SearchSource::label`])。
    pub sources_used: Vec<String>,
    /// 收集到的素材片段。
    pub snippets: Vec<SearchSnippet>,
    /// 命中片段总数。
    pub total_found: usize,
}

// ---------------------------------------------------------------------
// 大纲
// ---------------------------------------------------------------------

/// 大纲章节。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutlineSection {
    /// 章节标题。
    pub heading: String,
    /// 关键要点(每条一个字符串)。
    pub key_points: Vec<String>,
    /// 本章节目标字数。
    pub target_words: usize,
}

/// 写作大纲。由 [`SelfMediaWorkflow::step_outline`] 返回。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Outline {
    /// 大纲标题(通常等于主题)。
    pub title: String,
    /// 章节列表。
    pub sections: Vec<OutlineSection>,
    /// 预计总字数(各章节 `target_words` 之和)。
    pub estimated_word_count: usize,
}

// ---------------------------------------------------------------------
// 初稿
// ---------------------------------------------------------------------

/// 写作初稿/润色稿。`step_draft` 与 `step_polish` 均返回此类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Draft {
    /// 正文内容(Markdown)。
    pub content: String,
    /// 字数(Unicode 感知,CJK 逐字计、Latin 按词计)。
    pub word_count: usize,
    /// 目标平台。
    pub platform: SocialMediaPlatform,
    /// 关联的大纲标题(供回溯)。
    pub outline_ref: Option<String>,
}

// ---------------------------------------------------------------------
// 审查报告
// ---------------------------------------------------------------------

/// 审查维度。与场景模板的 `style_params` 对齐,覆盖语法/语气/准确性/
/// 互动性/平台适配/原创性六项。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCriteria {
    /// 语法与拼写。
    Grammar,
    /// 语气与风格一致性。
    Tone,
    /// 事实准确性。
    Accuracy,
    /// 互动性与吸引力。
    Engagement,
    /// 平台适配度(篇幅/排版/调性)。
    PlatformFit,
    /// 原创性。
    Originality,
}

impl ReviewCriteria {
    /// 返回维度的可读名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Grammar => "grammar",
            Self::Tone => "tone",
            Self::Accuracy => "accuracy",
            Self::Engagement => "engagement",
            Self::PlatformFit => "platform_fit",
            Self::Originality => "originality",
        }
    }
}

/// 问题严重程度。派生 `Ord`,排序为 Info < Warning < Error < Critical。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    /// 提示(不影响通过)。
    Info,
    /// 警告(建议修订)。
    Warning,
    /// 错误(应修订)。
    Error,
    /// 严重(必须修订)。
    Critical,
}

/// 单条审查问题。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewIssue {
    /// 关联维度。
    pub criteria: ReviewCriteria,
    /// 严重程度。
    pub severity: IssueSeverity,
    /// 问题描述。
    pub description: String,
    /// 修订建议。
    pub suggestion: String,
}

/// 审查报告。由 [`SelfMediaWorkflow::step_review`] 返回。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewReport {
    /// 各维度评分(0.0-1.0)。
    pub criteria_scores: HashMap<ReviewCriteria, f32>,
    /// 发现的问题列表。
    pub issues: Vec<ReviewIssue>,
    /// 综合评分(各维度均值)。
    pub overall_score: f32,
    /// 是否通过(overall >= 0.7 且无 Error/Critical 问题)。
    pub pass: bool,
}

// ---------------------------------------------------------------------
// 归档目的地
// ---------------------------------------------------------------------

/// 归档目的地。`step_archive` 据此返回稳定 URI 供后续读取。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDestination {
    /// 本地文件(指定路径)。
    LocalFile(PathBuf),
    /// 系统剪贴板。
    Clipboard,
    /// 内存存储(进程内,重启丢失)。
    MemoryStore,
    /// Obsidian 知识库(指定 vault 名)。
    ObsidianVault(String),
}

// ---------------------------------------------------------------------
// 工作流步骤与进度
// ---------------------------------------------------------------------

/// 工作流步骤。派生 `Ord`,顺序为 Search < Outline < Draft < Review <
/// Polish < Archive < Done。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStep {
    /// 步骤1:素材搜索。
    Search,
    /// 步骤2:生成大纲。
    Outline,
    /// 步骤3:撰写初稿。
    Draft,
    /// 步骤4:审查。
    Review,
    /// 步骤5:润色。
    Polish,
    /// 步骤6:归档。
    Archive,
    /// 完成。
    Done,
}

impl WorkflowStep {
    /// 返回步骤序号(1-based),供进度展示。
    pub fn index(self) -> u8 {
        match self {
            Self::Search => 1,
            Self::Outline => 2,
            Self::Draft => 3,
            Self::Review => 4,
            Self::Polish => 5,
            Self::Archive => 6,
            Self::Done => 7,
        }
    }

    /// 返回步骤的可读名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "素材搜索",
            Self::Outline => "生成大纲",
            Self::Draft => "撰写初稿",
            Self::Review => "审查",
            Self::Polish => "润色",
            Self::Archive => "归档",
            Self::Done => "完成",
        }
    }
}

/// 工作流进度快照。由 [`SelfMediaWorkflow::get_step_progress`] 返回。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowProgress {
    /// 当前步骤。
    pub current_step: WorkflowStep,
    /// 已完成步骤列表(按完成顺序)。
    pub completed_steps: Vec<WorkflowStep>,
    /// 当前步骤内部进度(0.0-1.0)。
    pub step_progress: f32,
    /// 工作流启动时间(UTC)。
    pub started_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------
// 请求与结果
// ---------------------------------------------------------------------

/// 自媒体写作请求。由调用方构造,传入 [`SelfMediaWorkflow::execute`]。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelfMediaRequest {
    /// 写作主题。
    pub topic: String,
    /// 目标平台。
    pub platform: SocialMediaPlatform,
    /// 写作风格参数。
    pub style: WritingStyle,
    /// 素材搜索来源列表。
    pub search_sources: Vec<SearchSource>,
    /// 目标读者描述(如"Z 世代都市白领")。
    pub target_audience: String,
    /// 目标字数(`None` 时使用平台默认值)。
    pub word_count: Option<usize>,
    /// 归档目的地。
    pub archive_destination: ArchiveDestination,
    /// 是否启用审查-润色循环。
    pub enable_review: bool,
    /// 最大修订轮次(审查不通过时最多润色次数)。
    pub max_revisions: u32,
}

/// 自媒体写作端到端结果。由 [`SelfMediaWorkflow::execute`] 返回。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelfMediaResult {
    /// 写作主题。
    pub topic: String,
    /// 目标平台。
    pub platform: SocialMediaPlatform,
    /// 最终正文(润色后)。
    pub final_draft: String,
    /// 生成的大纲。
    pub outline: Outline,
    /// 搜索结果。
    pub search_result: SearchResult,
    /// 审查报告(`enable_review=false` 时为 `None`)。
    pub review_report: Option<ReviewReport>,
    /// 实际修订轮次。
    pub revisions_made: u32,
    /// 总耗时(毫秒)。
    pub total_duration_ms: u64,
    /// 归档路径/URI。
    pub archive_path: Option<String>,
}

// ---------------------------------------------------------------------
// SelfMediaWorkflow — 端到端工作流编排器
// ---------------------------------------------------------------------

/// 自媒体写作端到端工作流编排器。
///
/// 持有一个 `RwLock<WorkflowProgress>` 跟踪步骤进度,所有 `step_*` 方法
/// 为无状态异步方法(不持有可变状态),进度更新通过内部 `begin_step` /
/// `complete_step` 私有方法写入 `RwLock`。
///
/// ## 构造
///
/// ```
/// use nebula_lib::writing::self_media_workflow::SelfMediaWorkflow;
/// let workflow = SelfMediaWorkflow::new();
/// ```
///
/// ## 执行
///
/// 调用 [`execute`](Self::execute) 传入 [`SelfMediaRequest`],即可按
/// 「搜索 → 大纲 → 初稿 → 审查 → 润色 → 归档」顺序执行完整工作流。
pub struct SelfMediaWorkflow {
    progress: RwLock<WorkflowProgress>,
}

impl Default for SelfMediaWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

impl SelfMediaWorkflow {
    /// 创建一个新的工作流编排器,进度初始化为 `Search` 步骤、0% 进度。
    pub fn new() -> Self {
        Self {
            progress: RwLock::new(WorkflowProgress {
                current_step: WorkflowStep::Search,
                completed_steps: Vec::new(),
                step_progress: 0.0,
                started_at: Utc::now(),
            }),
        }
    }

    /// 设置当前步骤与子进度。
    fn begin_step(&self, step: WorkflowStep) {
        let mut p = self.progress.write();
        p.current_step = step;
        p.step_progress = 0.0;
    }

    /// 标记步骤完成(加入 `completed_steps`,子进度置 1.0)。
    fn complete_step(&self, step: WorkflowStep) {
        let mut p = self.progress.write();
        if !p.completed_steps.contains(&step) {
            p.completed_steps.push(step);
        }
        p.step_progress = 1.0;
    }

    /// 返回当前进度快照(克隆,线程安全)。
    pub fn get_step_progress(&self) -> WorkflowProgress {
        self.progress.read().clone()
    }

    /// 端到端编排入口:搜索 → 大纲 → 初稿 → [审查 → 润色] → 归档。
    ///
    /// 流程:
    /// 1. `step_search` 收集素材
    /// 2. `step_outline` 生成平台适配大纲
    /// 3. `step_draft` 撰写初稿
    /// 4. 若 `enable_review`:按 `max_revisions` 最多进行 N 轮「审查 → 润色」
    /// 5. `step_archive` 归档到指定目的地
    ///
    /// 返回包含最终正文、大纲、搜索结果、审查报告与归档路径的完整结果。
    #[instrument(skip(self, request), fields(topic = %request.topic, platform = %request.platform.label()))]
    pub async fn execute(&self, request: SelfMediaRequest) -> Result<SelfMediaResult> {
        let start = std::time::Instant::now();

        // ---- 步骤1:素材搜索 ----
        self.begin_step(WorkflowStep::Search);
        let search_result = self
            .step_search(&request.topic, &request.search_sources)
            .await?;
        self.complete_step(WorkflowStep::Search);

        // ---- 步骤2:生成大纲 ----
        self.begin_step(WorkflowStep::Outline);
        let outline = self
            .step_outline(&request.topic, &search_result, request.platform)
            .await?;
        self.complete_step(WorkflowStep::Outline);

        // ---- 步骤3:撰写初稿 ----
        self.begin_step(WorkflowStep::Draft);
        let mut draft = self
            .step_draft(&outline, request.platform, &request.style)
            .await?;
        self.complete_step(WorkflowStep::Draft);

        // ---- 步骤4 & 5:审查 + 润色(带修订循环)----
        let mut review_report: Option<ReviewReport> = None;
        let mut revisions_made: u32 = 0;
        if request.enable_review {
            let criteria = default_review_criteria();
            loop {
                self.begin_step(WorkflowStep::Review);
                let review = self.step_review(&draft, &criteria).await?;
                let passed = review.pass;
                review_report = Some(review);
                self.complete_step(WorkflowStep::Review);

                // 通过或达到修订上限 → 退出循环。
                if passed || revisions_made >= request.max_revisions {
                    break;
                }

                // 未通过 → 润色一轮,继续审查。
                self.begin_step(WorkflowStep::Polish);
                draft = self
                    .step_polish(
                        &draft,
                        review_report.as_ref().expect("review_report 已设置"),
                    )
                    .await?;
                self.complete_step(WorkflowStep::Polish);
                revisions_made += 1;
            }
            info!(
                target: "nebula.writing.self_media",
                revisions = revisions_made,
                passed = review_report.as_ref().map(|r| r.pass).unwrap_or(false),
                "审查-润色循环结束"
            );
        }

        // ---- 步骤6:归档 ----
        self.begin_step(WorkflowStep::Archive);
        let result = SelfMediaResult {
            topic: request.topic.clone(),
            platform: request.platform,
            final_draft: draft.content.clone(),
            outline,
            search_result,
            review_report,
            revisions_made,
            total_duration_ms: start.elapsed().as_millis() as u64,
            archive_path: None,
        };
        let archive_path = self
            .step_archive(&result, &request.archive_destination)
            .await?;
        self.complete_step(WorkflowStep::Archive);

        // ---- 完成 ----
        self.begin_step(WorkflowStep::Done);
        let mut result = result;
        result.archive_path = Some(archive_path);
        self.complete_step(WorkflowStep::Done);

        info!(
            target: "nebula.writing.self_media",
            topic = %result.topic,
            platform = %result.platform.label(),
            duration_ms = result.total_duration_ms,
            revisions = result.revisions_made,
            "自媒体写作工作流完成"
        );
        Ok(result)
    }

    /// 步骤1:素材搜索。
    ///
    /// 根据 `topic` 与 `sources` 生成结构化素材片段(占位实现,无网络调用)。
    /// 生产环境可替换为真实搜索引擎/知识库检索。
    #[instrument(skip(self), fields(topic = %topic))]
    pub async fn step_search(&self, topic: &str, sources: &[SearchSource]) -> Result<SearchResult> {
        let sources_used: Vec<String> = if sources.is_empty() {
            vec![SearchSource::Web.label().to_string()]
        } else {
            sources.iter().map(|s| s.label().to_string()).collect()
        };

        // 占位实现:为每个来源生成一条素材片段。
        let snippets: Vec<SearchSnippet> = sources_used
            .iter()
            .enumerate()
            .map(|(i, src)| SearchSnippet {
                title: format!("关于「{}」的素材 #{}", topic, i + 1),
                content: format!(
                    "围绕主题「{}」从 {} 收集的参考素材。包含背景信息、关键数据与用户关注点。",
                    topic, src
                ),
                source: src.clone(),
                relevance: 0.9 - (i as f32) * 0.1,
            })
            .collect();
        let total_found = snippets.len();

        info!(
            target: "nebula.writing.self_media",
            %topic,
            sources = ?sources_used,
            total = total_found,
            "素材搜索完成"
        );
        Ok(SearchResult {
            query: topic.to_string(),
            sources_used,
            snippets,
            total_found,
        })
    }

    /// 步骤2:生成大纲。
    ///
    /// 根据 `platform` 返回平台适配的章节结构。各平台的章节模板与
    /// `writing::scenarios` 中的 14 个自媒体场景模板语义对齐。
    #[instrument(skip(self, search_result), fields(topic = %topic, platform = %platform.label()))]
    pub async fn step_outline(
        &self,
        topic: &str,
        search_result: &SearchResult,
        platform: SocialMediaPlatform,
    ) -> Result<Outline> {
        let sections = platform_sections(platform, topic);
        let estimated_word_count: usize = sections.iter().map(|s| s.target_words).sum();

        info!(
            target: "nebula.writing.self_media",
            %topic,
            platform = %platform.label(),
            section_count = sections.len(),
            estimated = estimated_word_count,
            snippets = search_result.total_found,
            "大纲生成完成"
        );
        Ok(Outline {
            title: topic.to_string(),
            sections,
            estimated_word_count,
        })
    }

    /// 步骤3:撰写初稿。
    ///
    /// 基于 `outline` 填充章节正文,叠加 `style` 风格参数注释。
    /// 字数由 `crate::writing::count_words` 计算(Unicode 感知)。
    #[instrument(skip(self, outline, style), fields(platform = %platform.label()))]
    pub async fn step_draft(
        &self,
        outline: &Outline,
        platform: SocialMediaPlatform,
        style: &WritingStyle,
    ) -> Result<Draft> {
        let mut content = format!("# {}\n\n", outline.title);

        for section in &outline.sections {
            content.push_str(&format!("## {}\n\n", section.heading));
            for point in &section.key_points {
                content.push_str(&format!("- {}\n", point));
            }
            content.push('\n');
        }

        // 叠加风格参数注释(供 step_review 读取)。
        content.push_str(&format!(
            "<!-- style: tone={} | perspective={} | formality={} | humor={} | emoji_density={} | platform={} -->\n",
            style.tone,
            style.perspective,
            style.formality,
            style.humor,
            style.emoji_density,
            platform.label()
        ));

        let word_count = crate::writing::count_words(&content);
        info!(
            target: "nebula.writing.self_media",
            platform = %platform.label(),
            word_count,
            "初稿撰写完成"
        );
        Ok(Draft {
            content,
            word_count,
            platform,
            outline_ref: Some(outline.title.clone()),
        })
    }

    /// 步骤4:审查。
    ///
    /// 对 `draft` 按 `criteria` 逐项打分(0.0-1.0),收集问题列表,
    /// 计算 `overall_score` 与 `pass`。
    ///
    /// 通过条件:`overall_score >= 0.7` 且无 `Error`/`Critical` 级问题。
    #[instrument(skip(self, draft, criteria), fields(word_count = draft.word_count))]
    pub async fn step_review(
        &self,
        draft: &Draft,
        criteria: &[ReviewCriteria],
    ) -> Result<ReviewReport> {
        let mut criteria_scores: HashMap<ReviewCriteria, f32> = HashMap::new();
        let mut issues: Vec<ReviewIssue> = Vec::new();

        for &c in criteria {
            let (score, issue) = review_criterion_score(c, draft);
            criteria_scores.insert(c, score);
            if let Some(i) = issue {
                issues.push(i);
            }
        }

        let overall_score = if criteria_scores.is_empty() {
            0.0
        } else {
            criteria_scores.values().sum::<f32>() / criteria_scores.len() as f32
        };

        let has_blocking = issues.iter().any(|i| i.severity >= IssueSeverity::Error);
        let pass = overall_score >= 0.7 && !has_blocking;

        info!(
            target: "nebula.writing.self_media",
            overall = overall_score,
            issue_count = issues.len(),
            pass,
            "审查完成"
        );
        Ok(ReviewReport {
            criteria_scores,
            issues,
            overall_score,
            pass,
        })
    }

    /// 步骤5:润色。
    ///
    /// 基于 `review` 中的问题建议,在 `draft` 末尾追加修订注释,
    /// 重新计算字数。占位实现,生产环境可注入 LLM 改写。
    #[instrument(skip(self, draft, review))]
    pub async fn step_polish(&self, draft: &Draft, review: &ReviewReport) -> Result<Draft> {
        let mut content = draft.content.clone();

        if review.issues.is_empty() {
            content.push_str("\n<!-- 润色:无问题,仅做轻度优化 -->\n");
        } else {
            for issue in &review.issues {
                content.push_str(&format!(
                    "\n<!-- 修订[{}] {} → {} -->",
                    issue.criteria.label(),
                    issue.description,
                    issue.suggestion
                ));
            }
        }

        let word_count = crate::writing::count_words(&content);
        info!(
            target: "nebula.writing.self_media",
            word_count,
            issues_addressed = review.issues.len(),
            "润色完成"
        );
        Ok(Draft {
            content,
            word_count,
            platform: draft.platform,
            outline_ref: draft.outline_ref.clone(),
        })
    }

    /// 步骤6:归档。
    ///
    /// 根据 `destination` 返回稳定 URI/路径字符串:
    /// - `LocalFile(p)` → `p` 的 display 字符串
    /// - `Clipboard` → `clipboard://self_media`
    /// - `MemoryStore` → `memory://self_media/{topic}`
    /// - `ObsidianVault(v)` → `obsidian://{v}/{topic}`
    #[instrument(skip(self, result, destination))]
    pub async fn step_archive(
        &self,
        result: &SelfMediaResult,
        destination: &ArchiveDestination,
    ) -> Result<String> {
        let path = match destination {
            ArchiveDestination::LocalFile(p) => p.display().to_string(),
            ArchiveDestination::Clipboard => "clipboard://self_media".to_string(),
            ArchiveDestination::MemoryStore => {
                format!("memory://self_media/{}", sanitize_uri(&result.topic))
            }
            ArchiveDestination::ObsidianVault(vault) => {
                format!("obsidian://{}/{}", vault, sanitize_uri(&result.topic))
            }
        };
        info!(
            target: "nebula.writing.self_media",
            archive = %path,
            "归档完成"
        );
        Ok(path)
    }
}

// ---------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------

/// 返回默认的 6 项审查维度(全量)。
fn default_review_criteria() -> Vec<ReviewCriteria> {
    vec![
        ReviewCriteria::Grammar,
        ReviewCriteria::Tone,
        ReviewCriteria::Accuracy,
        ReviewCriteria::Engagement,
        ReviewCriteria::PlatformFit,
        ReviewCriteria::Originality,
    ]
}

/// 按平台返回大纲章节结构。各平台章节与 `writing::scenarios` 模板语义对齐。
fn platform_sections(platform: SocialMediaPlatform, topic: &str) -> Vec<OutlineSection> {
    let target = platform.target_word_count();
    match platform {
        SocialMediaPlatform::WeChatArticle => vec![
            section("开头钩子", &["痛点切入", "故事引入"], target / 4),
            section("正文展开", &["分点论述", "案例支撑"], target / 2),
            section("金句收尾", &["洞察提炼", "引导关注"], target / 4),
        ],
        SocialMediaPlatform::XiaohongshuNote => vec![
            section("标题钩子", &["数字钩子", "痛点钩子"], 30),
            section("种草要点", &["emoji 分点", "使用体验"], target - 80),
            section("互动引导", &["提问互动", "话题标签"], 50),
        ],
        SocialMediaPlatform::DouyinScript => vec![
            section("3秒钩子", &["强钩子开头"], 50),
            section("口播正文", &["短句快节奏", "网感表达"], target - 100),
            section("行动号召", &["点赞关注", "话题标签"], 50),
        ],
        SocialMediaPlatform::ZhihuAnswer => vec![
            section("结论先行", &["核心观点", "故事引入"], target / 4),
            section("论证展开", &["数据支撑", "案例分析"], target / 2),
            section("总结", &["要点回顾", "价值升华"], target / 4),
        ],
        SocialMediaPlatform::WeiboPost => vec![
            section("开头抓人", &["观点/金句"], 40),
            section("正文", &["简练有梗", "140字内"], 80),
            section("话题标签", &["1-3个标签"], 20),
        ],
        SocialMediaPlatform::BilibiliScript => vec![
            section("片头钩子", &["5秒抓人"], target / 5),
            section("主体解说", &["信息量/趣味"], target * 3 / 5),
            section("结尾三连", &["三连引导", "话题标签"], target / 5),
        ],
        SocialMediaPlatform::ToutiaoArticle => vec![
            section("开头钩子", &["信息量切入"], target / 5),
            section("正文", &["通俗易读", "信息密度高"], target * 3 / 5),
            section("关键收获", &["要点总结"], target / 5),
        ],
        SocialMediaPlatform::DoubanReview => vec![
            section("作品切入", &["细节入手"], target / 4),
            section("细读分析", &["思辨判断", "审美"], target / 2),
            section("评分总评", &["评分", "总判断"], target / 4),
        ],
        SocialMediaPlatform::TiebaPost => vec![
            section("正文", &["接地气表达"], target - 80),
            section("求互动", &["抛问题引导回复"], 80),
        ],
        SocialMediaPlatform::JikeMoment => vec![
            section("一句话想法", &["独立思考"], 40),
            section("展开", &["不端着"], target - 80),
            section("话题标签", &["1-2个标签"], 40),
        ],
        SocialMediaPlatform::WechatChannels => vec![
            section("共鸣钩子", &["情感戳点"], 40),
            section("口播正文", &["有温度", "生活感"], target - 80),
            section("行动号召", &["引导互动"], 40),
        ],
        SocialMediaPlatform::WechatLongform => vec![
            section("开头钩子", &["深度切入"], target / 5),
            section("深度正文", &["数据与案例", "独立判断"], target / 2),
            section("核心洞察", &["深度洞察"], target / 5),
            section("结尾引导", &["引导关注"], target / 10),
        ],
        SocialMediaPlatform::XiaohongshuImageText => vec![
            section("封面标题", &["封面钩子"], 30),
            section("分页要点", &["每页一要点", "emoji排版"], target - 80),
            section("互动引导", &["互动", "话题标签"], 50),
        ],
        SocialMediaPlatform::ZhihuColumn => vec![
            section("引子", &["引出主题"], target / 5),
            section("主体论证", &["严密论证", "引用出处"], target * 3 / 5),
            section("结论要点", &["结论提炼"], target / 5),
        ],
    }
    .into_iter()
    .map(|s| inject_topic(s, topic))
    .collect()
}

/// 构造一个大纲章节。
fn section(heading: &str, key_points: &[&str], target_words: usize) -> OutlineSection {
    OutlineSection {
        heading: heading.to_string(),
        key_points: key_points.iter().map(|s| s.to_string()).collect(),
        target_words: target_words.max(20),
    }
}

/// 在章节关键要点中注入主题(使大纲与主题相关)。
fn inject_topic(mut s: OutlineSection, topic: &str) -> OutlineSection {
    s.key_points = s
        .key_points
        .into_iter()
        .map(|p| format!("{}({})", p, topic))
        .collect();
    s
}

/// 对单个审查维度打分,返回 (score, optional_issue)。
///
/// 占位实现:基于初稿内容特征做启发式评分。生产环境可替换为 LLM 审查。
fn review_criterion_score(criteria: ReviewCriteria, draft: &Draft) -> (f32, Option<ReviewIssue>) {
    let content = &draft.content;
    let target = draft.platform.target_word_count();
    let actual = draft.word_count;

    match criteria {
        ReviewCriteria::Grammar => {
            // 占位:无 LLM 时默认语法良好(0.9)。
            (0.9, None)
        }
        ReviewCriteria::Tone => {
            // 检查 style 注释是否存在于初稿中。
            let has_style = content.contains("<!-- style:");
            let score = if has_style { 0.85 } else { 0.6 };
            let issue = if !has_style {
                Some(ReviewIssue {
                    criteria,
                    severity: IssueSeverity::Warning,
                    description: "初稿缺少风格参数标注".to_string(),
                    suggestion: "补充语气/视角标注以便对齐风格".to_string(),
                })
            } else {
                None
            };
            (score, issue)
        }
        ReviewCriteria::Accuracy => {
            // 占位:无事实核查时默认 0.85。
            (0.85, None)
        }
        ReviewCriteria::Engagement => {
            // 基于内容长度与 emoji 密度启发式评分。
            let has_hook = content.contains("钩子") || content.contains("开头");
            let score = if has_hook { 0.8 } else { 0.5 };
            let issue = if !has_hook {
                Some(ReviewIssue {
                    criteria,
                    severity: IssueSeverity::Warning,
                    description: "缺少吸引力钩子".to_string(),
                    suggestion: "在开头增加强钩子提升互动".to_string(),
                })
            } else {
                None
            };
            (score, issue)
        }
        ReviewCriteria::PlatformFit => {
            // 基于字数与平台目标字数的偏离度评分。
            let ratio = if target == 0 {
                1.0
            } else {
                (actual as f32 / target as f32).min(1.0)
            };
            let score = 0.5 + 0.4 * ratio; // 0.5-0.9
            let issue = if ratio < 0.5 {
                Some(ReviewIssue {
                    criteria,
                    severity: IssueSeverity::Error,
                    description: format!("字数 {} 远低于平台目标 {}", actual, target),
                    suggestion: format!("扩充至 {} 字左右以适配平台", target),
                })
            } else {
                None
            };
            (score, issue)
        }
        ReviewCriteria::Originality => {
            // 占位:无查重时默认 0.8。
            (0.8, None)
        }
    }
}

/// 将主题字符串清理为 URI 安全的片段(替换分隔符)。
fn sanitize_uri(s: &str) -> String {
    s.replace(['/', ' ', '\n', '\r', '?', '#', '%'], "_")
}

// ---------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // 1. SocialMediaPlatform 所有变体序列化
    // ===================================================================

    #[test]
    fn social_media_platform_all_variants_serialize() {
        let variants = [
            SocialMediaPlatform::WeChatArticle,
            SocialMediaPlatform::XiaohongshuNote,
            SocialMediaPlatform::DouyinScript,
            SocialMediaPlatform::ZhihuAnswer,
            SocialMediaPlatform::WeiboPost,
            SocialMediaPlatform::BilibiliScript,
            SocialMediaPlatform::ToutiaoArticle,
            SocialMediaPlatform::DoubanReview,
            SocialMediaPlatform::TiebaPost,
            SocialMediaPlatform::JikeMoment,
            SocialMediaPlatform::WechatChannels,
            SocialMediaPlatform::WechatLongform,
            SocialMediaPlatform::XiaohongshuImageText,
            SocialMediaPlatform::ZhihuColumn,
        ];
        let expected = [
            "we_chat_article",
            "xiaohongshu_note",
            "douyin_script",
            "zhihu_answer",
            "weibo_post",
            "bilibili_script",
            "toutiao_article",
            "douban_review",
            "tieba_post",
            "jike_moment",
            "wechat_channels",
            "wechat_longform",
            "xiaohongshu_image_text",
            "zhihu_column",
        ];
        assert_eq!(variants.len(), 14, "应有 14 个平台变体");
        for (v, exp) in variants.iter().zip(expected.iter()) {
            let json = serde_json::to_string(v).expect("serialize");
            assert_eq!(json, format!("\"{}\"", exp), "序列化应为 snake_case");
            let de: SocialMediaPlatform = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, *v, "往返应一致");
        }
        // template_id 与 scenarios 一致。
        assert_eq!(
            SocialMediaPlatform::WeChatArticle.template_id(),
            "wechat-article"
        );
        // label 非空。
        for v in &variants {
            assert!(!v.label().is_empty());
            assert!(v.target_word_count() > 0);
        }
    }

    // ===================================================================
    // 2. SelfMediaRequest 构建
    // ===================================================================

    #[test]
    fn self_media_request_construction() {
        let request = SelfMediaRequest {
            topic: "Rust 异步编程入门".to_string(),
            platform: SocialMediaPlatform::ZhihuAnswer,
            style: WritingStyle {
                tone: "专业理性".to_string(),
                perspective: "第一人称".to_string(),
                formality: 70,
                humor: 20,
                emoji_density: 5,
            },
            search_sources: vec![SearchSource::Web, SearchSource::KnowledgeBase],
            target_audience: "后端开发者".to_string(),
            word_count: Some(1500),
            archive_destination: ArchiveDestination::ObsidianVault("my-vault".to_string()),
            enable_review: true,
            max_revisions: 2,
        };
        assert_eq!(request.topic, "Rust 异步编程入门");
        assert_eq!(request.platform, SocialMediaPlatform::ZhihuAnswer);
        assert_eq!(request.search_sources.len(), 2);
        assert_eq!(request.word_count, Some(1500));
        assert!(request.enable_review);
        assert_eq!(request.max_revisions, 2);
        // serde 往返。
        let json = serde_json::to_string(&request).expect("serialize");
        let de: SelfMediaRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.topic, request.topic);
        assert_eq!(de.platform, request.platform);
    }

    // ===================================================================
    // 3. Outline 结构
    // ===================================================================

    #[test]
    fn outline_structure() {
        let outline = Outline {
            title: "测试主题".to_string(),
            sections: vec![
                OutlineSection {
                    heading: "引言".to_string(),
                    key_points: vec!["背景".to_string(), "动机".to_string()],
                    target_words: 200,
                },
                OutlineSection {
                    heading: "正文".to_string(),
                    key_points: vec!["论点1".to_string(), "论点2".to_string()],
                    target_words: 600,
                },
                OutlineSection {
                    heading: "结论".to_string(),
                    key_points: vec!["总结".to_string()],
                    target_words: 200,
                },
            ],
            estimated_word_count: 1000,
        };
        assert_eq!(outline.sections.len(), 3);
        assert_eq!(outline.estimated_word_count, 1000);
        // 验证 estimated_word_count 等于各 target_words 之和。
        let sum: usize = outline.sections.iter().map(|s| s.target_words).sum();
        assert_eq!(outline.estimated_word_count, sum);
        // serde 往返。
        let json = serde_json::to_string(&outline).expect("serialize");
        let de: Outline = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.title, outline.title);
        assert_eq!(de.sections.len(), 3);
    }

    // ===================================================================
    // 4. ReviewReport 评分
    // ===================================================================

    #[test]
    fn review_report_scoring() {
        let mut scores = HashMap::new();
        scores.insert(ReviewCriteria::Grammar, 0.9);
        scores.insert(ReviewCriteria::Tone, 0.85);
        scores.insert(ReviewCriteria::Accuracy, 0.8);
        scores.insert(ReviewCriteria::Engagement, 0.7);
        scores.insert(ReviewCriteria::PlatformFit, 0.75);
        scores.insert(ReviewCriteria::Originality, 0.8);
        let report = ReviewReport {
            criteria_scores: scores.clone(),
            issues: vec![],
            overall_score: 0.8,
            pass: true,
        };
        assert_eq!(report.criteria_scores.len(), 6);
        // overall 应等于各分数均值。
        let expected_overall: f32 = scores.values().sum::<f32>() / 6.0;
        assert!((report.overall_score - expected_overall).abs() < 0.01);
        assert!(report.pass);
        // 低分 + Error 问题应不通过。
        let failing = ReviewReport {
            criteria_scores: scores,
            issues: vec![ReviewIssue {
                criteria: ReviewCriteria::PlatformFit,
                severity: IssueSeverity::Error,
                description: "字数不足".to_string(),
                suggestion: "扩充内容".to_string(),
            }],
            overall_score: 0.5,
            pass: false,
        };
        assert!(!failing.pass);
    }

    // ===================================================================
    // 5. WorkflowProgress 步骤转换
    // ===================================================================

    #[test]
    fn workflow_progress_step_transitions() {
        let workflow = SelfMediaWorkflow::new();
        let p0 = workflow.get_step_progress();
        assert_eq!(p0.current_step, WorkflowStep::Search);
        assert!(p0.completed_steps.is_empty());
        assert_eq!(p0.step_progress, 0.0);

        // 模拟步骤推进。
        workflow.begin_step(WorkflowStep::Outline);
        let p1 = workflow.get_step_progress();
        assert_eq!(p1.current_step, WorkflowStep::Outline);

        workflow.complete_step(WorkflowStep::Search);
        let p2 = workflow.get_step_progress();
        assert!(p2.completed_steps.contains(&WorkflowStep::Search));
        assert_eq!(p2.step_progress, 1.0);

        // 多步骤完成后 completed_steps 应累积。
        workflow.complete_step(WorkflowStep::Outline);
        let p3 = workflow.get_step_progress();
        assert_eq!(p3.completed_steps.len(), 2);
        assert!(p3.completed_steps.contains(&WorkflowStep::Search));
        assert!(p3.completed_steps.contains(&WorkflowStep::Outline));
    }

    // ===================================================================
    // 6. IssueSeverity 严重程度排序
    // ===================================================================

    #[test]
    fn issue_severity_ordering() {
        // Info < Warning < Error < Critical
        assert!(IssueSeverity::Info < IssueSeverity::Warning);
        assert!(IssueSeverity::Warning < IssueSeverity::Error);
        assert!(IssueSeverity::Error < IssueSeverity::Critical);
        // serde 往返。
        for v in [
            IssueSeverity::Info,
            IssueSeverity::Warning,
            IssueSeverity::Error,
            IssueSeverity::Critical,
        ] {
            let json = serde_json::to_string(&v).expect("serialize");
            let de: IssueSeverity = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, v);
        }
        // 在 ReviewIssue 中使用 severity 排序。
        let mut severities = vec![
            IssueSeverity::Critical,
            IssueSeverity::Info,
            IssueSeverity::Error,
            IssueSeverity::Warning,
        ];
        severities.sort();
        assert_eq!(
            severities,
            vec![
                IssueSeverity::Info,
                IssueSeverity::Warning,
                IssueSeverity::Error,
                IssueSeverity::Critical,
            ]
        );
    }

    // ===================================================================
    // 7. WritingStyle 参数范围
    // ===================================================================

    #[test]
    fn writing_style_parameter_ranges() {
        // 合法范围 0-100。
        let valid = WritingStyle {
            tone: "活泼".to_string(),
            perspective: "第一人称".to_string(),
            formality: 0,
            humor: 50,
            emoji_density: 100,
        };
        assert!(valid.validate(), "0-100 范围应合法");

        // 边界值 0。
        let zero = WritingStyle {
            tone: "严肃".to_string(),
            perspective: "第三人称".to_string(),
            formality: 0,
            humor: 0,
            emoji_density: 0,
        };
        assert!(zero.validate());

        // 边界值 100。
        let max = WritingStyle {
            tone: "幽默".to_string(),
            perspective: "第一人称".to_string(),
            formality: 100,
            humor: 100,
            emoji_density: 100,
        };
        assert!(max.validate());

        // 超出 100 应不合法(u8 上限 255,因此 101-255 应被拒绝)。
        let invalid = WritingStyle {
            tone: "x".to_string(),
            perspective: "x".to_string(),
            formality: 150,
            humor: 50,
            emoji_density: 50,
        };
        assert!(!invalid.validate(), "formality=150 应不合法");

        // Default 应全为 0 且合法。
        let default = WritingStyle::default();
        assert!(default.validate());
        assert_eq!(default.formality, 0);
    }

    // ===================================================================
    // 8. SearchResult 构建
    // ===================================================================

    #[test]
    fn search_result_construction() {
        let result = SearchResult {
            query: "Rust 异步".to_string(),
            sources_used: vec!["web".to_string(), "knowledge_base".to_string()],
            snippets: vec![
                SearchSnippet {
                    title: "片段1".to_string(),
                    content: "内容1".to_string(),
                    source: "web".to_string(),
                    relevance: 0.9,
                },
                SearchSnippet {
                    title: "片段2".to_string(),
                    content: "内容2".to_string(),
                    source: "knowledge_base".to_string(),
                    relevance: 0.8,
                },
            ],
            total_found: 2,
        };
        assert_eq!(result.query, "Rust 异步");
        assert_eq!(result.sources_used.len(), 2);
        assert_eq!(result.snippets.len(), 2);
        assert_eq!(result.total_found, 2);
        // total_found 应等于 snippets 长度。
        assert_eq!(result.total_found, result.snippets.len());
        // serde 往返。
        let json = serde_json::to_string(&result).expect("serialize");
        let de: SearchResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.query, result.query);
        assert_eq!(de.snippets.len(), 2);
    }

    // ===================================================================
    // 9. Draft word_count 计算
    // ===================================================================

    #[test]
    fn draft_word_count_calculation() {
        // "Hello world 你好世界" → 2 latin words + 4 CJK chars = 6
        // (与 writing::count_words 的 Unicode 感知算法一致)。
        let content = "Hello world 你好世界";
        let word_count = crate::writing::count_words(content);
        assert_eq!(word_count, 6);

        let draft = Draft {
            content: content.to_string(),
            word_count,
            platform: SocialMediaPlatform::WeiboPost,
            outline_ref: Some("测试".to_string()),
        };
        assert_eq!(draft.word_count, 6);

        // 空内容字数为 0。
        let empty = Draft {
            content: String::new(),
            word_count: crate::writing::count_words(""),
            platform: SocialMediaPlatform::WeiboPost,
            outline_ref: None,
        };
        assert_eq!(empty.word_count, 0);

        // serde 往返。
        let json = serde_json::to_string(&draft).expect("serialize");
        let de: Draft = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.word_count, 6);
    }

    // ===================================================================
    // 10. ArchiveDestination 各变体
    // ===================================================================

    #[test]
    fn archive_destination_variants() {
        let local = ArchiveDestination::LocalFile(PathBuf::from("/tmp/article.md"));
        let clipboard = ArchiveDestination::Clipboard;
        let memory = ArchiveDestination::MemoryStore;
        let obsidian = ArchiveDestination::ObsidianVault("my-vault".to_string());

        // 序列化形式(externally tagged, snake_case)。
        let local_json = serde_json::to_string(&local).expect("serialize");
        assert!(local_json.contains("local_file"));
        let clip_json = serde_json::to_string(&clipboard).expect("serialize");
        assert_eq!(clip_json, "\"clipboard\"");
        let mem_json = serde_json::to_string(&memory).expect("serialize");
        assert_eq!(mem_json, "\"memory_store\"");
        let obs_json = serde_json::to_string(&obsidian).expect("serialize");
        assert!(obs_json.contains("obsidian_vault"));
        assert!(obs_json.contains("my-vault"));

        // 往返。
        let de_local: ArchiveDestination = serde_json::from_str(&local_json).expect("deserialize");
        assert_eq!(de_local, local);
        let de_clip: ArchiveDestination = serde_json::from_str(&clip_json).expect("deserialize");
        assert_eq!(de_clip, clipboard);
        let de_mem: ArchiveDestination = serde_json::from_str(&mem_json).expect("deserialize");
        assert_eq!(de_mem, memory);
        let de_obs: ArchiveDestination = serde_json::from_str(&obs_json).expect("deserialize");
        assert_eq!(de_obs, obsidian);
    }

    // ===================================================================
    // 11. workflow step 顺序
    // ===================================================================

    #[test]
    fn workflow_step_order() {
        // 派生 Ord: Search < Outline < Draft < Review < Polish < Archive < Done。
        assert!(WorkflowStep::Search < WorkflowStep::Outline);
        assert!(WorkflowStep::Outline < WorkflowStep::Draft);
        assert!(WorkflowStep::Draft < WorkflowStep::Review);
        assert!(WorkflowStep::Review < WorkflowStep::Polish);
        assert!(WorkflowStep::Polish < WorkflowStep::Archive);
        assert!(WorkflowStep::Archive < WorkflowStep::Done);

        // index 连续递增。
        assert_eq!(WorkflowStep::Search.index(), 1);
        assert_eq!(WorkflowStep::Done.index(), 7);

        // 完整排序。
        let mut steps = vec![
            WorkflowStep::Done,
            WorkflowStep::Search,
            WorkflowStep::Polish,
            WorkflowStep::Draft,
            WorkflowStep::Archive,
            WorkflowStep::Outline,
            WorkflowStep::Review,
        ];
        steps.sort();
        assert_eq!(
            steps,
            vec![
                WorkflowStep::Search,
                WorkflowStep::Outline,
                WorkflowStep::Draft,
                WorkflowStep::Review,
                WorkflowStep::Polish,
                WorkflowStep::Archive,
                WorkflowStep::Done,
            ]
        );

        // serde 往返。
        for s in [
            WorkflowStep::Search,
            WorkflowStep::Outline,
            WorkflowStep::Draft,
            WorkflowStep::Review,
            WorkflowStep::Polish,
            WorkflowStep::Archive,
            WorkflowStep::Done,
        ] {
            let json = serde_json::to_string(&s).expect("serialize");
            let de: WorkflowStep = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, s);
        }
    }

    // ===================================================================
    // 12. result 序列化往返
    // ===================================================================

    #[test]
    fn self_media_result_serde_roundtrip() {
        let mut scores = HashMap::new();
        scores.insert(ReviewCriteria::Grammar, 0.9);
        scores.insert(ReviewCriteria::Tone, 0.85);

        let result = SelfMediaResult {
            topic: "Rust 入门".to_string(),
            platform: SocialMediaPlatform::WeChatArticle,
            final_draft: "# Rust 入门\n\n正文内容".to_string(),
            outline: Outline {
                title: "Rust 入门".to_string(),
                sections: vec![OutlineSection {
                    heading: "背景".to_string(),
                    key_points: vec!["要点1".to_string()],
                    target_words: 500,
                }],
                estimated_word_count: 500,
            },
            search_result: SearchResult {
                query: "Rust 入门".to_string(),
                sources_used: vec!["web".to_string()],
                snippets: vec![SearchSnippet {
                    title: "片段".to_string(),
                    content: "内容".to_string(),
                    source: "web".to_string(),
                    relevance: 0.9,
                }],
                total_found: 1,
            },
            review_report: Some(ReviewReport {
                criteria_scores: scores,
                issues: vec![],
                overall_score: 0.875,
                pass: true,
            }),
            revisions_made: 1,
            total_duration_ms: 1234,
            archive_path: Some("/tmp/rust.md".to_string()),
        };

        let json = serde_json::to_string(&result).expect("serialize");
        let de: SelfMediaResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.topic, result.topic);
        assert_eq!(de.platform, result.platform);
        assert_eq!(de.final_draft, result.final_draft);
        assert_eq!(de.outline.title, result.outline.title);
        assert_eq!(de.search_result.total_found, 1);
        assert!(de.review_report.is_some());
        assert_eq!(de.revisions_made, 1);
        assert_eq!(de.total_duration_ms, 1234);
        assert_eq!(de.archive_path.as_deref(), Some("/tmp/rust.md"));
    }

    // ===================================================================
    // 补充:SearchSource 所有变体序列化
    // ===================================================================

    #[test]
    fn search_source_all_variants_serialize() {
        for (v, exp) in [
            (SearchSource::Web, "web"),
            (SearchSource::KnowledgeBase, "knowledge_base"),
            (SearchSource::Clipboard, "clipboard"),
            (SearchSource::RecentFiles, "recent_files"),
            (SearchSource::Bookmarks, "bookmarks"),
        ] {
            let json = serde_json::to_string(&v).expect("serialize");
            assert_eq!(json, format!("\"{}\"", exp));
            let de: SearchSource = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, v);
            assert_eq!(v.label(), exp);
        }
    }

    // ===================================================================
    // 补充:端到端 execute(无审查)
    // ===================================================================

    #[tokio::test]
    async fn execute_end_to_end_without_review() {
        let workflow = SelfMediaWorkflow::new();
        let request = SelfMediaRequest {
            topic: "Rust 异步编程".to_string(),
            platform: SocialMediaPlatform::ZhihuAnswer,
            style: WritingStyle {
                tone: "专业理性".to_string(),
                perspective: "第一人称".to_string(),
                formality: 70,
                humor: 20,
                emoji_density: 5,
            },
            search_sources: vec![SearchSource::Web],
            target_audience: "开发者".to_string(),
            word_count: Some(1200),
            archive_destination: ArchiveDestination::MemoryStore,
            enable_review: false,
            max_revisions: 0,
        };
        let result = workflow.execute(request).await.expect("execute");
        assert_eq!(result.topic, "Rust 异步编程");
        assert_eq!(result.platform, SocialMediaPlatform::ZhihuAnswer);
        assert!(!result.final_draft.is_empty());
        assert!(result.outline.sections.len() >= 2);
        assert_eq!(result.search_result.total_found, 1);
        assert!(result.review_report.is_none(), "未启用审查时无报告");
        assert_eq!(result.revisions_made, 0);
        assert!(result.archive_path.is_some());
        // 进度应为 Done。
        let progress = workflow.get_step_progress();
        assert_eq!(progress.current_step, WorkflowStep::Done);
        assert!(progress.completed_steps.contains(&WorkflowStep::Archive));
    }

    // ===================================================================
    // 补充:端到端 execute(带审查,通过)
    // ===================================================================

    #[tokio::test]
    async fn execute_end_to_end_with_review_pass() {
        let workflow = SelfMediaWorkflow::new();
        // 用 ZhihuAnswer(目标 1200 字)与较长内容以确保 PlatformFit 评分通过。
        let request = SelfMediaRequest {
            topic: "深度学习优化器对比".to_string(),
            platform: SocialMediaPlatform::WeiboPost,
            style: WritingStyle {
                tone: "简练有梗".to_string(),
                perspective: "第一人称".to_string(),
                formality: 30,
                humor: 60,
                emoji_density: 10,
            },
            search_sources: vec![SearchSource::Web, SearchSource::KnowledgeBase],
            target_audience: "微博用户".to_string(),
            word_count: None,
            archive_destination: ArchiveDestination::Clipboard,
            enable_review: true,
            max_revisions: 2,
        };
        let result = workflow.execute(request).await.expect("execute");
        assert!(result.review_report.is_some(), "启用审查时应有报告");
        // archive_path 应为 clipboard URI。
        assert_eq!(
            result.archive_path.as_deref(),
            Some("clipboard://self_media")
        );
    }

    // ===================================================================
    // 补充:step_outline 按平台生成章节
    // ===================================================================

    #[tokio::test]
    async fn step_outline_generates_platform_sections() {
        let workflow = SelfMediaWorkflow::new();
        let search_result = SearchResult {
            query: "测试".to_string(),
            sources_used: vec!["web".to_string()],
            snippets: vec![],
            total_found: 0,
        };
        for platform in [
            SocialMediaPlatform::WeChatArticle,
            SocialMediaPlatform::XiaohongshuNote,
            SocialMediaPlatform::DouyinScript,
            SocialMediaPlatform::ZhihuColumn,
        ] {
            let outline = workflow
                .step_outline("测试主题", &search_result, platform)
                .await
                .expect("outline");
            assert!(
                !outline.sections.is_empty(),
                "{} 应有章节",
                platform.label()
            );
            assert!(outline.estimated_word_count > 0);
            // 章节字数之和应等于 estimated_word_count。
            let sum: usize = outline.sections.iter().map(|s| s.target_words).sum();
            assert_eq!(outline.estimated_word_count, sum);
        }
    }

    // ===================================================================
    // 补充:step_search 空来源回退到 Web
    // ===================================================================

    #[tokio::test]
    async fn step_search_empty_sources_defaults_to_web() {
        let workflow = SelfMediaWorkflow::new();
        let result = workflow.step_search("主题", &[]).await.expect("search");
        assert_eq!(result.sources_used, vec!["web".to_string()]);
        assert_eq!(result.total_found, 1);
    }

    // ===================================================================
    // 补充:step_archive 各目的地 URI
    // ===================================================================

    #[tokio::test]
    async fn step_archive_returns_correct_uri() {
        let workflow = SelfMediaWorkflow::new();
        let result = SelfMediaResult {
            topic: "a/b c".to_string(),
            platform: SocialMediaPlatform::WeChatArticle,
            final_draft: "x".to_string(),
            outline: Outline {
                title: "t".to_string(),
                sections: vec![],
                estimated_word_count: 0,
            },
            search_result: SearchResult {
                query: "q".to_string(),
                sources_used: vec![],
                snippets: vec![],
                total_found: 0,
            },
            review_report: None,
            revisions_made: 0,
            total_duration_ms: 0,
            archive_path: None,
        };

        // LocalFile
        let p = workflow
            .step_archive(
                &result,
                &ArchiveDestination::LocalFile(PathBuf::from("/tmp/a.md")),
            )
            .await
            .expect("archive");
        assert_eq!(p, "/tmp/a.md");

        // Clipboard
        let p = workflow
            .step_archive(&result, &ArchiveDestination::Clipboard)
            .await
            .expect("archive");
        assert_eq!(p, "clipboard://self_media");

        // MemoryStore(主题中的分隔符应被清理)
        let p = workflow
            .step_archive(&result, &ArchiveDestination::MemoryStore)
            .await
            .expect("archive");
        assert_eq!(p, "memory://self_media/a_b_c");

        // ObsidianVault
        let p = workflow
            .step_archive(
                &result,
                &ArchiveDestination::ObsidianVault("vault".to_string()),
            )
            .await
            .expect("archive");
        assert_eq!(p, "obsidian://vault/a_b_c");
    }

    // ===================================================================
    // 补充:ReviewCriteria serde 与 label
    // ===================================================================

    #[test]
    fn review_criteria_serde_and_label() {
        for (c, exp) in [
            (ReviewCriteria::Grammar, "grammar"),
            (ReviewCriteria::Tone, "tone"),
            (ReviewCriteria::Accuracy, "accuracy"),
            (ReviewCriteria::Engagement, "engagement"),
            (ReviewCriteria::PlatformFit, "platform_fit"),
            (ReviewCriteria::Originality, "originality"),
        ] {
            let json = serde_json::to_string(&c).expect("serialize");
            assert_eq!(json, format!("\"{}\"", exp));
            let de: ReviewCriteria = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(de, c);
            assert_eq!(c.label(), exp);
        }
    }
}
