//! T-D-B-18 (v3.1): 28 个场景化写作模板。
//!
//! 在 v0.5 的 6 个通用模板(见 [`super::templates`])基础上补齐两个场景族:
//!
//! * **自媒体场景**(14 个):微信公众号 / 小红书 / 抖音 / 知乎 / 微博 / B站 /
//!   头条 / 豆瓣 / 贴吧 / 即刻 / 视频号 / 公众号长文 / 小红书图文 / 知乎专栏。
//! * **长篇小说场景**(14 个):玄幻 / 都市 / 科幻 / 历史 / 言情 / 悬疑 / 武侠 /
//!   军事 / 游戏 / 科幻末世 / 同人 / 轻小说 / 童话 / 剧本杀。
//!
//! 每个场景模板相对通用模板额外携带三项,用于驱动 LLM 生成:
//!
//! * `prompt_template` — 填好占位符后送入 LLM 的提示词;
//! * `output_format` — 期望输出格式(`markdown` / `plain_text` / `structured`);
//! * `style_params` — 语气 / 篇幅 / 读者 / 视角 / 额外参数。
//!
//! 与 T-D-B-17 的集成:本组模板是 [`crate::swarm::AgentScenario::Writing`]
//! 场景的模板后端,通过 [`writing_scenario_template_ids`] 暴露稳定 ID 列表,
//! 上层(platform / orchestrator)可据此按场景筛选并注入 Writer agent。

use std::collections::BTreeMap;

use super::templates::{
    TemplatePlaceholder, WritingScenarioCategory, WritingStyleParams, WritingTemplate,
};

// ---------------------------------------------------------------------
// 自媒体场景(14 个)
// ---------------------------------------------------------------------

/// 返回全部 14 个自媒体场景模板。
pub fn self_media_library() -> Vec<WritingTemplate> {
    vec![
        wechat_article(),
        xiaohongshu_note(),
        douyin_script(),
        zhihu_answer(),
        weibo_post(),
        bilibili_script(),
        toutiao_article(),
        douban_review(),
        tieba_post(),
        jike_moment(),
        wechat_channels(),
        wechat_longform(),
        xiaohongshu_image_text(),
        zhihu_column(),
    ]
}

/// 返回全部 14 个长篇小说场景模板。
pub fn novel_library() -> Vec<WritingTemplate> {
    vec![
        novel_xuanhuan(),
        novel_urban(),
        novel_scifi(),
        novel_history(),
        novel_romance(),
        novel_mystery(),
        novel_wuxia(),
        novel_military(),
        novel_game(),
        novel_apocalypse(),
        novel_fanfic(),
        novel_light(),
        novel_fairy_tale(),
        novel_jubensha(),
    ]
}

/// 返回全部 28 个场景模板(自媒体 + 长篇小说)。
pub fn scenario_library() -> Vec<WritingTemplate> {
    let mut all = self_media_library();
    all.extend(novel_library());
    all
}

/// T-D-B-17 桥接:返回 [`crate::swarm::AgentScenario::Writing`] 场景下的
/// 28 个稳定模板 ID(顺序:14 自媒体 + 14 长篇小说)。
pub fn writing_scenario_template_ids() -> Vec<&'static str> {
    [
        // 自媒体(14)
        "wechat-article",
        "xiaohongshu-note",
        "douyin-script",
        "zhihu-answer",
        "weibo-post",
        "bilibili-script",
        "toutiao-article",
        "douban-review",
        "tieba-post",
        "jike-moment",
        "wechat-channels",
        "wechat-longform",
        "xiaohongshu-image-text",
        "zhihu-column",
        // 长篇小说(14)
        "novel-xuanhuan",
        "novel-urban",
        "novel-scifi",
        "novel-history",
        "novel-romance",
        "novel-mystery",
        "novel-wuxia",
        "novel-military",
        "novel-game",
        "novel-apocalypse",
        "novel-fanfic",
        "novel-light",
        "novel-fairy-tale",
        "novel-jubensha",
    ]
    .to_vec()
}

// ===== 自媒体:逐个模板 =====

fn wechat_article() -> WritingTemplate {
    WritingTemplate {
        id: "wechat-article".into(),
        label: "微信公众号推文".into(),
        description: "钩子开头 + 价值正文 + 金句收尾的公众号推文骨架".into(),
        icon: "💬".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头钩子(痛点/故事)".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "正文(分点展开)".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "insight".into(),
                hint: "金句/洞察".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "cta".into(),
                hint: "引导关注/在看".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

{{hook}}

{{body}}

> {{insight}}

{{cta}}
"#
        .into(),
        prompt_template: r#"你是资深公众号主理人。请按以下要求创作一篇微信公众号推文。

标题:{{title}}
开头钩子:{{hook}}
正文要点:{{body}}
金句/洞察:{{insight}}
结尾引导:{{cta}}

要求:口语化、有信息增量、段落短小适合手机阅读,正文不少于 800 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("亲和专业".into()),
            length: Some("中(800-1500字)".into()),
            audience: Some("公众号读者".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn xiaohongshu_note() -> WritingTemplate {
    WritingTemplate {
        id: "xiaohongshu-note".into(),
        label: "小红书笔记".into(),
        description: "标题党 + emoji 排版 + 种草要点的笔记骨架".into(),
        icon: "📕".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "标题(带钩子/数字)".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头钩子".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "points".into(),
                hint: "种草要点(分点)".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "cta".into(),
                hint: "互动引导".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "tags".into(),
                hint: "话题标签".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

{{hook}}

{{points}}

{{cta}}

{{tags}}
"#
        .into(),
        prompt_template: r#"你是小红书爆款笔记写手。请创作一篇种草笔记。

标题:{{title}}
开头钩子:{{hook}}
种草要点:{{points}}
互动引导:{{cta}}
话题标签:{{tags}}

要求:标题带数字/痛点钩子,正文用 emoji 分点排版,语气活泼接地气,300-600 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("活泼接地气".into()),
            length: Some("短(300-600字)".into()),
            audience: Some("小红书用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn douyin_script() -> WritingTemplate {
    WritingTemplate {
        id: "douyin-script".into(),
        label: "抖音口播文案".into(),
        description: "3 秒钩子 + 节奏正文 + 行动号召的口播稿".into(),
        icon: "🎵".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "前3秒钩子".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "口播正文".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "cta".into(),
                hint: "行动号召".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hashtags".into(),
                hint: "话题标签".into(),
                multiline: false,
            },
        ],
        body: r#"## 抖音口播稿

**【3秒钩子】** {{hook}}

**【正文】**
{{body}}

**【行动号召】** {{cta}}

{{hashtags}}
"#
        .into(),
        prompt_template: r#"你是抖音口播文案高手。请创作一段 15-60 秒的口播稿。

前3秒钩子:{{hook}}
正文:{{body}}
行动号召:{{cta}}
话题标签:{{hashtags}}

要求:开头必须有强钩子,句子短、节奏快、有网感,口语化可直接念出。"#
            .into(),
        output_format: "plain_text".into(),
        style_params: WritingStyleParams {
            tone: Some("节奏快有网感".into()),
            length: Some("短(15-60秒口播)".into()),
            audience: Some("抖音用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn zhihu_answer() -> WritingTemplate {
    WritingTemplate {
        id: "zhihu-answer".into(),
        label: "知乎回答".into(),
        description: "结论先行 + 论证展开 + 总结的知乎回答骨架".into(),
        icon: "💡".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "question".into(),
                hint: "问题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头结论/故事".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "论证展开".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "summary".into(),
                hint: "总结".into(),
                multiline: false,
            },
        ],
        body: r#"## {{question}}

{{hook}}

{{body}}

**总结**:{{summary}}
"#
        .into(),
        prompt_template: r#"你是知乎高赞答主。请就以下问题写一个回答。

问题:{{question}}
开头:{{hook}}
论证:{{body}}
总结:{{summary}}

要求:结论先行,论证有数据/案例支撑,语气专业理性,不抖机灵,800-1500 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("专业理性".into()),
            length: Some("中长(800-1500字)".into()),
            audience: Some("知乎用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn weibo_post() -> WritingTemplate {
    WritingTemplate {
        id: "weibo-post".into(),
        label: "微博".into(),
        description: "140 字内有梗有观点的微博文案".into(),
        icon: "🐦".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头抓人".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "正文(140字内)".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hashtags".into(),
                hint: "话题标签".into(),
                multiline: false,
            },
        ],
        body: r#"{{hook}}

{{body}}

{{hashtags}}
"#
        .into(),
        prompt_template: r#"你是微博文案写手。请写一条微博。

开头:{{hook}}
正文:{{body}}
话题标签:{{hashtags}}

要求:140 字以内,简练有梗、观点鲜明,带 1-3 个话题标签。"#
            .into(),
        output_format: "plain_text".into(),
        style_params: WritingStyleParams {
            tone: Some("简练有梗".into()),
            length: Some("短(140字内)".into()),
            audience: Some("微博用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn bilibili_script() -> WritingTemplate {
    WritingTemplate {
        id: "bilibili-script".into(),
        label: "B站视频文案".into(),
        description: "片头钩子 + 主体解说 + 三连引导的 B 站文案".into(),
        icon: "📺".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "视频标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "片头钩子".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "主体解说".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "outro".into(),
                hint: "结尾/三连引导".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hashtags".into(),
                hint: "话题标签".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

**【片头】** {{hook}}

**【正文】**
{{body}}

**【结尾】** {{outro}}

{{hashtags}}
"#
        .into(),
        prompt_template: r#"你是 B 站 UP 主文案。请创作一支视频的口播文案。

标题:{{title}}
片头钩子:{{hook}}
主体解说:{{body}}
结尾/三连引导:{{outro}}
话题标签:{{hashtags}}

要求:片头 5 秒抓人,正文有信息量或趣味,语气二次元有梗,带三连引导。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("二次元有梗".into()),
            length: Some("中(3-10分钟视频文案)".into()),
            audience: Some("B站用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn toutiao_article() -> WritingTemplate {
    WritingTemplate {
        id: "toutiao-article".into(),
        label: "头条号文章".into(),
        description: "信息量导向、通俗易读的头条号文章骨架".into(),
        icon: "📰".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头钩子".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "正文".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "takeaway".into(),
                hint: "关键收获".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

{{hook}}

{{body}}

**关键收获**:{{takeaway}}
"#
        .into(),
        prompt_template: r#"你是头条号创作者。请写一篇信息量导向的文章。

标题:{{title}}
开头钩子:{{hook}}
正文:{{body}}
关键收获:{{takeaway}}

要求:通俗易读、信息密度高、段落短,适合下沉市场读者,1000-2000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("通俗信息量".into()),
            length: Some("中长(1000-2000字)".into()),
            audience: Some("头条读者".into()),
            perspective: Some("第三人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn douban_review() -> WritingTemplate {
    WritingTemplate {
        id: "douban-review".into(),
        label: "豆瓣影评/书评".into(),
        description: "作品切入 + 细读分析 + 评分判断的豆瓣长评".into(),
        icon: "🎬".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "评论标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "work".into(),
                hint: "作品名".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "细读分析".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "rating".into(),
                hint: "评分(1-5星)".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "verdict".into(),
                hint: "总评判断".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

**作品**:{{work}}

{{body}}

**评分**:{{rating}}

**总评**:{{verdict}}
"#
        .into(),
        prompt_template: r#"你是豆瓣资深评论者。请写一篇影评/书评。

评论标题:{{title}}
作品名:{{work}}
细读分析:{{body}}
评分:{{rating}}
总评:{{verdict}}

要求:从作品细节切入,有思辨与审美判断,避免剧透关键反转,文艺但不晦涩。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("文艺思辨".into()),
            length: Some("中(800-1500字)".into()),
            audience: Some("豆瓣用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn tieba_post() -> WritingTemplate {
    WritingTemplate {
        id: "tieba-post".into(),
        label: "贴吧帖子".into(),
        description: "接地气、求互动的贴吧发帖骨架".into(),
        icon: "🗑️".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "正文".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "call".into(),
                hint: "求互动/问吧友".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

{{body}}

{{call}}
"#
        .into(),
        prompt_template: r#"你是贴吧老哥。请发一个帖子。

标题:{{title}}
正文:{{body}}
求互动:{{call}}

要求:语气接地气、像真人发帖,末尾抛出问题引导吧友回复,200-500 字。"#
            .into(),
        output_format: "plain_text".into(),
        style_params: WritingStyleParams {
            tone: Some("接地气".into()),
            length: Some("短中(200-500字)".into()),
            audience: Some("贴吧老哥".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn jike_moment() -> WritingTemplate {
    WritingTemplate {
        id: "jike-moment".into(),
        label: "即刻动态".into(),
        description: "真实有思考的即刻短动态".into(),
        icon: "⚡".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "thought".into(),
                hint: "一句话想法".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "展开".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "tags".into(),
                hint: "话题标签".into(),
                multiline: false,
            },
        ],
        body: r#"{{thought}}

{{body}}

{{tags}}
"#
        .into(),
        prompt_template: r#"你是即刻用户。请发一条动态。

一句话想法:{{thought}}
展开:{{body}}
话题标签:{{tags}}

要求:真实、有独立思考,不端着,100-300 字,可带 1-2 个标签。"#
            .into(),
        output_format: "plain_text".into(),
        style_params: WritingStyleParams {
            tone: Some("真实有思考".into()),
            length: Some("短(100-300字)".into()),
            audience: Some("即刻用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn wechat_channels() -> WritingTemplate {
    WritingTemplate {
        id: "wechat-channels".into(),
        label: "视频号文案".into(),
        description: "温情共鸣向的视频号口播文案".into(),
        icon: "📹".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头共鸣钩子".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "口播正文".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "cta".into(),
                hint: "行动号召".into(),
                multiline: false,
            },
        ],
        body: r#"## 视频号口播稿

**【钩子】** {{hook}}

**【正文】**
{{body}}

**【行动号召】** {{cta}}
"#
        .into(),
        prompt_template: r#"你是视频号创作者。请创作一段温情共鸣向的口播文案。

开头钩子:{{hook}}
正文:{{body}}
行动号召:{{cta}}

要求:开头戳中情感共鸣,正文有温度、有生活感,30-90 秒口播。"#
            .into(),
        output_format: "plain_text".into(),
        style_params: WritingStyleParams {
            tone: Some("温情有共鸣".into()),
            length: Some("短(30-90秒口播)".into()),
            audience: Some("视频号用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn wechat_longform() -> WritingTemplate {
    WritingTemplate {
        id: "wechat-longform".into(),
        label: "公众号长文".into(),
        description: "深度长文:背景 + 论证 + 洞察 + 行动".into(),
        icon: "📚".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "开头钩子".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "深度正文".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "insight".into(),
                hint: "核心洞察".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "cta".into(),
                hint: "结尾引导".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

{{hook}}

{{body}}

## 核心洞察

{{insight}}

{{cta}}
"#
        .into(),
        prompt_template: r#"你是公众号深度长文作者。请写一篇深度文章。

标题:{{title}}
开头钩子:{{hook}}
深度正文:{{body}}
核心洞察:{{insight}}
结尾引导:{{cta}}

要求:信息密度高、有数据与案例、有独立判断,2000-5000 字,适合深度阅读。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("深度专业".into()),
            length: Some("长(2000-5000字)".into()),
            audience: Some("公众号深度读者".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn xiaohongshu_image_text() -> WritingTemplate {
    WritingTemplate {
        id: "xiaohongshu-image-text".into(),
        label: "小红书图文".into(),
        description: "图文卡片式:封面标题 + 分页要点 + 引导".into(),
        icon: "🖼️".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "封面标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "封面副标题/钩子".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "slides".into(),
                hint: "分页要点(每页一段)".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "cta".into(),
                hint: "互动引导".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "tags".into(),
                hint: "话题标签".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

> {{hook}}

{{slides}}

{{cta}}

{{tags}}
"#
        .into(),
        prompt_template: r#"你是小红书图文博主。请创作一套图文卡片内容。

封面标题:{{title}}
封面副标题/钩子:{{hook}}
分页要点:{{slides}}
互动引导:{{cta}}
话题标签:{{tags}}

要求:每页一个要点、emoji 排版,精致种草风,适合多图滑动阅读。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("精致种草".into()),
            length: Some("短(图文卡片)".into()),
            audience: Some("小红书用户".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn zhihu_column() -> WritingTemplate {
    WritingTemplate {
        id: "zhihu-column".into(),
        label: "知乎专栏".into(),
        description: "专栏级深度文章:引子 + 主体 + 结论".into(),
        icon: "📝".into(),
        category: WritingScenarioCategory::SelfMedia,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "专栏标题".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "引子".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "body".into(),
                hint: "主体论证".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "takeaway".into(),
                hint: "结论/要点".into(),
                multiline: false,
            },
        ],
        body: r#"# {{title}}

{{hook}}

{{body}}

**要点**:{{takeaway}}
"#
        .into(),
        prompt_template: r#"你是知乎专栏作者。请写一篇专栏文章。

标题:{{title}}
引子:{{hook}}
主体论证:{{body}}
结论/要点:{{takeaway}}

要求:深度专栏风,论证严密、有引用与出处,1500-3000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("深度专栏".into()),
            length: Some("中长(1500-3000字)".into()),
            audience: Some("知乎专栏读者".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

// ---------------------------------------------------------------------
// 长篇小说场景(14 个)
// ---------------------------------------------------------------------

/// 小说场景共享的第三人称风格参数构造器。
fn novel_style(tone: &str, audience: &str, extras: &[(&str, &str)]) -> WritingStyleParams {
    let mut ex = BTreeMap::new();
    for (k, v) in extras {
        ex.insert((*k).to_string(), (*v).to_string());
    }
    WritingStyleParams {
        tone: Some(tone.into()),
        length: Some("章节(2000-4000字)".into()),
        audience: Some(audience.into()),
        perspective: Some("第三人称".into()),
        extras: ex,
    }
}

fn novel_xuanhuan() -> WritingTemplate {
    WritingTemplate {
        id: "novel-xuanhuan".into(),
        label: "玄幻小说".into(),
        description: "修炼体系 + 金手指 + 爽感升级的玄幻章节骨架".into(),
        icon: "🐉".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "world".into(),
                hint: "世界观/修炼体系".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "climax".into(),
                hint: "高潮/爽点".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{world}}

{{conflict}}

{{climax}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是网文玄幻写手。请写一章玄幻小说。

章节:{{chapter}}
主角:{{protagonist}}
世界观/修炼体系:{{world}}
冲突:{{conflict}}
高潮/爽点:{{climax}}
章末钩子:{{hook}}

要求:热血爽感、节奏明快、有金手指设定,章末留钩子,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style(
            "热血爽感",
            "网文读者",
            &[("体系", "修炼体系"), ("金手指", "是")],
        ),
    }
}

fn novel_urban() -> WritingTemplate {
    WritingTemplate {
        id: "novel-urban".into(),
        label: "都市小说".into(),
        description: "都市背景 + 现实爽感 + 反转的都市章节骨架".into(),
        icon: "🏙️".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "setting".into(),
                hint: "都市场景设定".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "turn".into(),
                hint: "反转".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{setting}}

{{conflict}}

{{turn}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是都市网文写手。请写一章都市小说。

章节:{{chapter}}
主角:{{protagonist}}
场景设定:{{setting}}
冲突:{{conflict}}
反转:{{turn}}
章末钩子:{{hook}}

要求:写实带爽感、贴近都市生活、有反转,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("写实爽感", "都市网文读者", &[]),
    }
}

fn novel_scifi() -> WritingTemplate {
    WritingTemplate {
        id: "novel-scifi".into(),
        label: "科幻小说".into(),
        description: "硬核设定 + 科学冲突 + 揭示的科幻章节骨架".into(),
        icon: "🚀".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "world".into(),
                hint: "科幻设定".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "reveal".into(),
                hint: "揭示/反转".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{world}}

{{conflict}}

{{reveal}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是科幻小说写手。请写一章科幻小说。

章节:{{chapter}}
主角:{{protagonist}}
科幻设定:{{world}}
冲突:{{conflict}}
揭示/反转:{{reveal}}
章末钩子:{{hook}}

要求:设定硬核自洽、想象大胆、逻辑严密,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("硬核想象", "科幻读者", &[]),
    }
}

fn novel_history() -> WritingTemplate {
    WritingTemplate {
        id: "novel-history".into(),
        label: "历史小说".into(),
        description: "时代背景 + 历史冲突 + 命运转折的历史章节骨架".into(),
        icon: "🏛️".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "era".into(),
                hint: "时代背景".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "历史冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "turn".into(),
                hint: "命运转折".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{era}}

{{conflict}}

{{turn}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是历史小说写手。请写一章历史小说。

章节:{{chapter}}
主角:{{protagonist}}
时代背景:{{era}}
历史冲突:{{conflict}}
命运转折:{{turn}}
章末钩子:{{hook}}

要求:史实考据、行文厚重、人物命运与时代交织,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("厚重考据", "历史小说读者", &[]),
    }
}

fn novel_romance() -> WritingTemplate {
    WritingTemplate {
        id: "novel-romance".into(),
        label: "言情小说".into(),
        description: "情感张力 + 关系转折的言情章节骨架".into(),
        icon: "💞".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "love_interest".into(),
                hint: "感情对象".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "tension".into(),
                hint: "情感张力".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "turn".into(),
                hint: "关系转折".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}} × {{love_interest}}

{{tension}}

{{turn}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是言情小说写手。请写一章言情小说。

章节:{{chapter}}
主角:{{protagonist}}
感情对象:{{love_interest}}
情感张力:{{tension}}
关系转折:{{turn}}
章末钩子:{{hook}}

要求:情感细腻动人、对话有张力、有甜有虐,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("细腻动人".into()),
            length: Some("章节(2000-4000字)".into()),
            audience: Some("言情读者".into()),
            perspective: Some("第三人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn novel_mystery() -> WritingTemplate {
    WritingTemplate {
        id: "novel-mystery".into(),
        label: "悬疑小说".into(),
        description: "谜题 + 线索 + 反转的悬疑章节骨架".into(),
        icon: "🔍".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角(侦探/当事人)".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "mystery".into(),
                hint: "谜题/案件".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "clue".into(),
                hint: "线索".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "twist".into(),
                hint: "反转".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{mystery}}

{{clue}}

{{twist}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是悬疑小说写手。请写一章悬疑小说。

章节:{{chapter}}
主角:{{protagonist}}
谜题/案件:{{mystery}}
线索:{{clue}}
反转:{{twist}}
章末钩子:{{hook}}

要求:节奏紧张烧脑、线索埋设合理、反转出人意料又自洽,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("紧张烧脑", "悬疑读者", &[]),
    }
}

fn novel_wuxia() -> WritingTemplate {
    WritingTemplate {
        id: "novel-wuxia".into(),
        label: "武侠小说".into(),
        description: "江湖 + 侠义 + 决斗的武侠章节骨架".into(),
        icon: "⚔️".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "jianghu".into(),
                hint: "江湖背景".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "恩怨冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "duel".into(),
                hint: "决斗/交锋".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{jianghu}}

{{conflict}}

{{duel}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是武侠小说写手。请写一章武侠小说。

章节:{{chapter}}
主角:{{protagonist}}
江湖背景:{{jianghu}}
恩怨冲突:{{conflict}}
决斗/交锋:{{duel}}
章末钩子:{{hook}}

要求:侠义古韵、招式描写有画面感、文白相间,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("侠义古韵", "武侠读者", &[]),
    }
}

fn novel_military() -> WritingTemplate {
    WritingTemplate {
        id: "novel-military".into(),
        label: "军事小说".into(),
        description: "部队 + 任务 + 铁血冲突的军事章节骨架".into(),
        icon: "🎖️".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "unit".into(),
                hint: "部队/单位".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "mission".into(),
                hint: "任务".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突/战斗".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}({{unit}})

{{mission}}

{{conflict}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是军事小说写手。请写一章军事小说。

章节:{{chapter}}
主角:{{protagonist}}
部队/单位:{{unit}}
任务:{{mission}}
冲突/战斗:{{conflict}}
章末钩子:{{hook}}

要求:铁血硬核、战术细节扎实、有战友情谊,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("铁血硬核", "军事小说读者", &[]),
    }
}

fn novel_game() -> WritingTemplate {
    WritingTemplate {
        id: "novel-game".into(),
        label: "游戏小说".into(),
        description: "网游/电竞 + 玩法 + 燃点的游戏小说章节骨架".into(),
        icon: "🎮".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "game".into(),
                hint: "游戏/赛事设定".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "play".into(),
                hint: "玩法/操作高潮".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{game}}

{{conflict}}

{{play}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是游戏(网游/电竞)小说写手。请写一章游戏小说。

章节:{{chapter}}
主角:{{protagonist}}
游戏/赛事设定:{{game}}
冲突:{{conflict}}
玩法/操作高潮:{{play}}
章末钩子:{{hook}}

要求:爽快燃、操作/战术描写有画面感、节奏明快,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("爽快燃", "网游小说读者", &[]),
    }
}

fn novel_apocalypse() -> WritingTemplate {
    WritingTemplate {
        id: "novel-apocalypse".into(),
        label: "科幻末世小说".into(),
        description: "灾难 + 求生 + 威胁的末世章节骨架".into(),
        icon: "☢️".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "disaster".into(),
                hint: "灾难设定".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "survival".into(),
                hint: "求生策略".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "threat".into(),
                hint: "威胁/危机".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{disaster}}

{{survival}}

{{threat}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是末世科幻写手。请写一章末世小说。

章节:{{chapter}}
主角:{{protagonist}}
灾难设定:{{disaster}}
求生策略:{{survival}}
威胁/危机:{{threat}}
章末钩子:{{hook}}

要求:压抑求生但带希望、资源与人性冲突真实,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("压抑求生带希望", "末世小说读者", &[]),
    }
}

fn novel_fanfic() -> WritingTemplate {
    WritingTemplate {
        id: "novel-fanfic".into(),
        label: "同人小说".into(),
        description: "原作还原 + 二次创作的同人章节骨架".into(),
        icon: "🔁".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "source".into(),
                hint: "原作/世界线".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "turn".into(),
                hint: "转折".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{source}}

{{conflict}}

{{turn}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是同人小说写手。请写一章同人小说。

章节:{{chapter}}
主角:{{protagonist}}
原作/世界线:{{source}}
冲突:{{conflict}}
转折:{{turn}}
章末钩子:{{hook}}

要求:还原原作人物性格与设定、二次创作有新意,OOC 需合理化,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: novel_style("还原原作", "同人读者", &[]),
    }
}

fn novel_light() -> WritingTemplate {
    WritingTemplate {
        id: "novel-light".into(),
        label: "轻小说".into(),
        description: "设定 + 吐槽 + 中二笑点的轻小说章节骨架".into(),
        icon: "✨".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "chapter".into(),
                hint: "章节名/序号".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "premise".into(),
                hint: "设定/前提".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "conflict".into(),
                hint: "冲突".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "gag".into(),
                hint: "吐槽/笑点".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "hook".into(),
                hint: "章末钩子".into(),
                multiline: true,
            },
        ],
        body: r#"# {{chapter}}

> POV: {{protagonist}}

{{premise}}

{{conflict}}

{{gag}}

{{hook}}
"#
        .into(),
        prompt_template: r#"你是轻小说写手。请写一章轻小说。

章节:{{chapter}}
主角:{{protagonist}}
设定/前提:{{premise}}
冲突:{{conflict}}
吐槽/笑点:{{gag}}
章末钩子:{{hook}}

要求:第一人称、轻快中二、吐槽密集、有反差萌,2000-4000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("轻快中二".into()),
            length: Some("章节(2000-4000字)".into()),
            audience: Some("轻小说读者".into()),
            perspective: Some("第一人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn novel_fairy_tale() -> WritingTemplate {
    WritingTemplate {
        id: "novel-fairy-tale".into(),
        label: "童话".into(),
        description: "场景 + 冒险 + 寓意 + 结局的童话骨架".into(),
        icon: "🧚".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "故事名".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "protagonist".into(),
                hint: "主角".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "setting".into(),
                hint: "场景设定".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "adventure".into(),
                hint: "冒险经历".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "moral".into(),
                hint: "寓意".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "ending".into(),
                hint: "结局".into(),
                multiline: true,
            },
        ],
        body: r#"# {{title}}

> 主角:{{protagonist}}

{{setting}}

{{adventure}}

**寓意**:{{moral}}

{{ending}}
"#
        .into(),
        prompt_template: r#"你是童话作家。请写一篇童话。

故事名:{{title}}
主角:{{protagonist}}
场景设定:{{setting}}
冒险经历:{{adventure}}
寓意:{{moral}}
结局:{{ending}}

要求:温暖纯真、想象奇幻、有明确寓意、结局美好,适合儿童与亲子阅读,800-2000 字。"#
            .into(),
        output_format: "markdown".into(),
        style_params: WritingStyleParams {
            tone: Some("温暖纯真".into()),
            length: Some("中(800-2000字)".into()),
            audience: Some("儿童与家长".into()),
            perspective: Some("第三人称".into()),
            extras: BTreeMap::new(),
        },
    }
}

fn novel_jubensha() -> WritingTemplate {
    WritingTemplate {
        id: "novel-jubensha".into(),
        label: "剧本杀".into(),
        description: "背景 + 角色 + 剧情 + 线索 + 结局的剧本杀剧本骨架".into(),
        icon: "🎭".into(),
        category: WritingScenarioCategory::Novel,
        placeholders: vec![
            TemplatePlaceholder {
                name: "title".into(),
                hint: "剧本名".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "character_count".into(),
                hint: "玩家人数".into(),
                multiline: false,
            },
            TemplatePlaceholder {
                name: "background".into(),
                hint: "故事背景".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "characters".into(),
                hint: "角色列表(每人简介)".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "plot".into(),
                hint: "剧情推进".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "clues".into(),
                hint: "线索设计".into(),
                multiline: true,
            },
            TemplatePlaceholder {
                name: "ending".into(),
                hint: "真相/结局".into(),
                multiline: true,
            },
        ],
        body: r#"# {{title}}

> 玩家人数:{{character_count}}

## 故事背景

{{background}}

## 角色列表

{{characters}}

## 剧情推进

{{plot}}

## 线索设计

{{clues}}

## 真相/结局

{{ending}}
"#
        .into(),
        prompt_template: r#"你是剧本杀编剧。请设计一个剧本杀剧本。

剧本名:{{title}}
玩家人数:{{character_count}}
故事背景:{{background}}
角色列表:{{characters}}
剧情推进:{{plot}}
线索设计:{{clues}}
真相/结局:{{ending}}

要求:悬疑沉浸、每个角色都有动机与秘密、线索逻辑自洽、可盘玩性高。"#
            .into(),
        output_format: "structured".into(),
        style_params: WritingStyleParams {
            tone: Some("悬疑沉浸".into()),
            length: Some("长(完整剧本)".into()),
            audience: Some("剧本杀玩家".into()),
            perspective: Some("上帝视角".into()),
            extras: BTreeMap::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_media_library_has_fourteen() {
        let lib = self_media_library();
        assert_eq!(lib.len(), 14, "expected 14 self-media templates");
        for t in &lib {
            assert_eq!(
                t.category,
                WritingScenarioCategory::SelfMedia,
                "{} should be SelfMedia",
                t.id
            );
        }
    }

    #[test]
    fn novel_library_has_fourteen() {
        let lib = novel_library();
        assert_eq!(lib.len(), 14, "expected 14 novel templates");
        for t in &lib {
            assert_eq!(
                t.category,
                WritingScenarioCategory::Novel,
                "{} should be Novel",
                t.id
            );
        }
    }

    #[test]
    fn scenario_library_has_twenty_eight() {
        let lib = scenario_library();
        assert_eq!(lib.len(), 28, "expected 28 scenario templates");
    }

    #[test]
    fn scenario_ids_match_constructors() {
        // writing_scenario_template_ids 必须与 scenario_library 的 ID 完全一致(顺序相同)
        let lib_ids: Vec<String> = scenario_library().into_iter().map(|t| t.id).collect();
        let declared = writing_scenario_template_ids();
        assert_eq!(
            lib_ids, declared,
            "declared ids must match constructor order"
        );
    }

    #[test]
    fn scenario_templates_have_unique_ids() {
        let lib = scenario_library();
        let mut ids: Vec<&str> = lib.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), lib.len(), "duplicate scenario template ids");
    }

    #[test]
    fn scenario_placeholders_match_body_tokens() {
        for t in scenario_library() {
            for p in &t.placeholders {
                let token = format!("{{{{{}}}}}", p.name);
                assert!(
                    t.body.contains(&token),
                    "scenario template {} missing body token {}",
                    t.id,
                    token
                );
            }
        }
    }

    #[test]
    fn known_self_media_ids_present() {
        let ids: std::collections::HashSet<String> =
            self_media_library().into_iter().map(|t| t.id).collect();
        for id in [
            "wechat-article",
            "xiaohongshu-note",
            "douyin-script",
            "zhihu-answer",
            "weibo-post",
            "bilibili-script",
            "toutiao-article",
            "douban-review",
            "tieba-post",
            "jike-moment",
            "wechat-channels",
            "wechat-longform",
            "xiaohongshu-image-text",
            "zhihu-column",
        ] {
            assert!(ids.contains(id), "missing self-media id {id}");
        }
    }

    #[test]
    fn known_novel_ids_present() {
        let ids: std::collections::HashSet<String> =
            novel_library().into_iter().map(|t| t.id).collect();
        for id in [
            "novel-xuanhuan",
            "novel-urban",
            "novel-scifi",
            "novel-history",
            "novel-romance",
            "novel-mystery",
            "novel-wuxia",
            "novel-military",
            "novel-game",
            "novel-apocalypse",
            "novel-fanfic",
            "novel-light",
            "novel-fairy-tale",
            "novel-jubensha",
        ] {
            assert!(ids.contains(id), "missing novel id {id}");
        }
    }
}
