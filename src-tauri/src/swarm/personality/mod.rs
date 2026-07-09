//! T-E-D-04: 8 人格原型系统 — 为 Agent 定义 8 种不同的人格原型。
//!
//! 每种人格原型有独特的思维方式、沟通风格和决策偏好，可与
//! [`crate::swarm::agents::AgentScenario`] 配合，在编排时按场景推荐
//! 最适配的人格，或在多 agent 协作中混合两种人格以平衡倾向。
//!
//! ## 设计意图
//!
//! * 与 [`crate::swarm::agents::AgentKind`] / [`crate::swarm::agents::AgentScenario`]
//!   正交：`AgentKind` 描述"实现类型"，`AgentScenario` 描述"任务场景"，
//!   `Personality` 描述"思维方式与沟通偏好"。三者可自由组合。
//! * **无 feature gate**：与 `primary_agent` 一样只依赖始终可用的类型，
//!   不引入重型组件（无 LLM / 无 DB / 无网络）。
//! * 所有数值偏好（`CommunicationStyle` / `DecisionBias`）使用 `0.0..=1.0`
//!   的浮点区间，便于 [`PersonalityRegistry::blend`] 做线性插值。
//!
//! ## 8 种人格
//!
//! | id            | 中文名     | 思维方式        | 沟通风格            | 决策偏好               |
//! |---------------|-----------|-----------------|---------------------|------------------------|
//! | `analyst`     | 分析师    | 逻辑驱动        | 直接、低形式、高精度 | 低风险、高质量、高细节 |
//! | `creative`    | 创意者    | 发散思维        | 含蓄、低形式、高共情 | 高风险、低速度、低细节 |
//! | `pragmatist`  | 实用主义者| 序贯思维        | 直接、低形式、低冗长 | 中风险、高速度、低细节 |
//! | `perfectionist`| 完美主义者| 逻辑驱动       | 中直接、高形式、高冗长| 低风险、低速度、高细节 |
//! | `visionary`   | 远见者    | 整体思维        | 中直接、中形式       | 高风险、低速度、低细节 |
//! | `collaborator`| 协作者    | 整体思维        | 含蓄、中形式、高共情 | 中风险、中速度、中细节 |
//! | `skeptic`     | 怀疑者    | 逻辑驱动        | 高直接、中形式       | 低风险、低速度、高细节 |
//! | `explorer`    | 探索者    | 直觉思维        | 中直接、低形式       | 高风险、高速度、低细节 |

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

// ===========================================================================
// PersonalityId — 8 种人格枚举
// ===========================================================================

/// 8 种人格原型标识。
///
/// 与 [`crate::swarm::agents::AgentScenario`] 一样使用
/// `#[serde(rename_all = "snake_case")]`，保证 JSON / 前端交互一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalityId {
    /// 分析师 — 逻辑驱动，数据导向。
    Analyst,
    /// 创意者 — 发散思维，灵感驱动。
    Creative,
    /// 实用主义者 — 结果导向，效率优先。
    Pragmatist,
    /// 完美主义者 — 细节导向，质量优先。
    Perfectionist,
    /// 远见者 — 战略思维，长期导向。
    Visionary,
    /// 协作者 — 团队导向，共识驱动。
    Collaborator,
    /// 怀疑者 — 批判思维，风险意识。
    Skeptic,
    /// 探索者 — 好奇驱动，实验导向。
    Explorer,
}

impl PersonalityId {
    /// 全部 8 种人格 id（按枚举声明顺序）。
    pub const ALL: [PersonalityId; 8] = [
        PersonalityId::Analyst,
        PersonalityId::Creative,
        PersonalityId::Pragmatist,
        PersonalityId::Perfectionist,
        PersonalityId::Visionary,
        PersonalityId::Collaborator,
        PersonalityId::Skeptic,
        PersonalityId::Explorer,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PersonalityId::Analyst => "analyst",
            PersonalityId::Creative => "creative",
            PersonalityId::Pragmatist => "pragmatist",
            PersonalityId::Perfectionist => "perfectionist",
            PersonalityId::Visionary => "visionary",
            PersonalityId::Collaborator => "collaborator",
            PersonalityId::Skeptic => "skeptic",
            PersonalityId::Explorer => "explorer",
        }
    }
}

impl std::fmt::Display for PersonalityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PersonalityId {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "analyst" => Ok(PersonalityId::Analyst),
            "creative" => Ok(PersonalityId::Creative),
            "pragmatist" => Ok(PersonalityId::Pragmatist),
            "perfectionist" => Ok(PersonalityId::Perfectionist),
            "visionary" => Ok(PersonalityId::Visionary),
            "collaborator" => Ok(PersonalityId::Collaborator),
            "skeptic" => Ok(PersonalityId::Skeptic),
            "explorer" => Ok(PersonalityId::Explorer),
            other => Err(format!("unknown personality id: {other}")),
        }
    }
}

/// T-E-D-04: 用 anyhow 解析人格 id，附带上下文用于错误诊断。
///
/// 与 [`std::str::FromStr`] 实现互补：本函数返回 [`anyhow::Result`]，
/// 便于在调用链中用 `?` 传播并附加 `.context()`。
pub fn parse_personality_id(s: &str) -> Result<PersonalityId> {
    s.parse::<PersonalityId>()
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("failed to parse personality id from {s:?}"))
}

// ===========================================================================
// CognitiveStyle — 认知风格
// ===========================================================================

/// 认知风格 — 4 种思维模式的偏好权重（`0.0..=1.0`）。
///
/// 每种人格在 4 个维度上各有侧重，`blend` 时按 `ratio` 线性插值。
/// 高权重维度决定该人格的主导思维方式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveStyle {
    /// 分析型思维：分解、推理、证据驱动。
    pub analytical: f64,
    /// 直觉型思维：跳跃、联想、灵感驱动。
    pub intuitive: f64,
    /// 序贯型思维：分步、流程、顺序驱动。
    pub sequential: f64,
    /// 整体型思维：全局、系统、长期驱动。
    pub holistic: f64,
}

impl CognitiveStyle {
    /// 构造一个所有维度均为 0.0 的认知风格（用于 builder 风格赋值）。
    pub fn empty() -> Self {
        Self {
            analytical: 0.0,
            intuitive: 0.0,
            sequential: 0.0,
            holistic: 0.0,
        }
    }

    /// 把所有维度 clamp 到 `0.0..=1.0`。
    pub fn clamped(mut self) -> Self {
        self.analytical = self.analytical.clamp(0.0, 1.0);
        self.intuitive = self.intuitive.clamp(0.0, 1.0);
        self.sequential = self.sequential.clamp(0.0, 1.0);
        self.holistic = self.holistic.clamp(0.0, 1.0);
        self
    }

    /// 返回权重最高的维度名（`"analytical"` / `"intuitive"` / ...）。
    /// 平局时按声明顺序取首个。
    pub fn dominant(&self) -> &'static str {
        let mut best = ("analytical", self.analytical);
        for (name, val) in [
            ("intuitive", self.intuitive),
            ("sequential", self.sequential),
            ("holistic", self.holistic),
        ] {
            if val > best.1 {
                best = (name, val);
            }
        }
        match best.0 {
            "analytical" => "analytical",
            "intuitive" => "intuitive",
            "sequential" => "sequential",
            _ => "holistic",
        }
    }
}

// ===========================================================================
// CommunicationStyle — 沟通风格
// ===========================================================================

/// 沟通风格 — 4 个维度（`0.0..=1.0`）。
///
/// * `directness`：0.0 = 含蓄委婉，1.0 = 直言不讳
/// * `formality`：0.0 = 口语化，1.0 = 正式书面
/// * `verbosity`：0.0 = 极简，1.0 = 详尽
/// * `empathy`：0.0 = 冷淡客观，1.0 = 高度共情
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommunicationStyle {
    pub directness: f64,
    pub formality: f64,
    pub verbosity: f64,
    pub empathy: f64,
}

impl CommunicationStyle {
    /// 把所有维度 clamp 到 `0.0..=1.0`。
    pub fn clamped(mut self) -> Self {
        self.directness = self.directness.clamp(0.0, 1.0);
        self.formality = self.formality.clamp(0.0, 1.0);
        self.verbosity = self.verbosity.clamp(0.0, 1.0);
        self.empathy = self.empathy.clamp(0.0, 1.0);
        self
    }

    /// 线性插值：`self * ratio + other * (1 - ratio)`。
    pub fn lerp(&self, other: &Self, ratio: f64) -> Self {
        let r = ratio.clamp(0.0, 1.0);
        Self {
            directness: self.directness * r + other.directness * (1.0 - r),
            formality: self.formality * r + other.formality * (1.0 - r),
            verbosity: self.verbosity * r + other.verbosity * (1.0 - r),
            empathy: self.empathy * r + other.empathy * (1.0 - r),
        }
    }
}

// ===========================================================================
// DecisionBias — 决策偏好
// ===========================================================================

/// 决策偏好 — 3 个维度（`0.0..=1.0`）。
///
/// * `risk_tolerance`：0.0 = 极度规避风险，1.0 = 乐于冒险
/// * `speed_vs_quality`：0.0 = 质量优先，1.0 = 速度优先
/// * `detail_orientation`：0.0 = 抓大放小，1.0 = 死磕细节
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionBias {
    pub risk_tolerance: f64,
    pub speed_vs_quality: f64,
    pub detail_orientation: f64,
}

impl DecisionBias {
    /// 把所有维度 clamp 到 `0.0..=1.0`。
    pub fn clamped(mut self) -> Self {
        self.risk_tolerance = self.risk_tolerance.clamp(0.0, 1.0);
        self.speed_vs_quality = self.speed_vs_quality.clamp(0.0, 1.0);
        self.detail_orientation = self.detail_orientation.clamp(0.0, 1.0);
        self
    }

    /// 线性插值：`self * ratio + other * (1 - ratio)`。
    pub fn lerp(&self, other: &Self, ratio: f64) -> Self {
        let r = ratio.clamp(0.0, 1.0);
        Self {
            risk_tolerance: self.risk_tolerance * r + other.risk_tolerance * (1.0 - r),
            speed_vs_quality: self.speed_vs_quality * r + other.speed_vs_quality * (1.0 - r),
            detail_orientation: self.detail_orientation * r + other.detail_orientation * (1.0 - r),
        }
    }
}

// ===========================================================================
// Personality — 人格原型
// ===========================================================================

/// 人格原型 — 一种独特的思维方式、沟通风格与决策偏好的组合。
///
/// 由 [`PersonalityRegistry::new`] 初始化 8 种内置人格，也可通过
/// [`PersonalityRegistry::blend`] 混合两种人格生成临时人格。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Personality {
    /// 人格 id（混合人格取主导方的 id）。
    pub id: PersonalityId,
    /// 人格名称（如 "分析师"）。
    pub name: String,
    /// 原型描述（一句话刻画该人格的核心特质）。
    pub archetype: String,
    /// 认知风格。
    pub cognitive_style: CognitiveStyle,
    /// 沟通风格。
    pub communication_style: CommunicationStyle,
    /// 决策偏好。
    pub decision_bias: DecisionBias,
    /// 优势列表。
    pub strengths: Vec<String>,
    /// 弱点列表。
    pub weaknesses: Vec<String>,
    /// Dify 风格 system prompt 模板（≥ 300 字符）。
    pub system_prompt_template: String,
    /// 适配场景（与 [`crate::swarm::agents::AgentScenario::as_str`] 对齐：
    /// `coding` / `writing` / `review` / `research` / `planning`）。
    pub preferred_scenarios: Vec<String>,
    /// UI 颜色标识（hex，如 `"#3B82F6"`）。
    pub color: String,
}

impl Personality {
    /// 序列化为 JSON 字符串。
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("failed to serialize Personality to JSON")
    }

    /// 从 JSON 字符串反序列化。
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("failed to deserialize Personality from JSON")
    }
}

// ===========================================================================
// PersonalityRegistry — 人格注册表
// ===========================================================================

/// 人格注册表 — 持有 8 种内置人格，支持按 id 查询、按场景推荐、混合两种人格。
///
/// 不可变结构：构造后内部人格表不再变化，查询方法均返回 `&Personality`。
/// `blend` 返回新拥有的 `Personality`（不写入注册表）。
pub struct PersonalityRegistry {
    by_id: HashMap<PersonalityId, Personality>,
    order: Vec<PersonalityId>,
}

impl PersonalityRegistry {
    /// 初始化 8 种内置人格。
    pub fn new() -> Self {
        let personalities = builtin_personalities();
        let mut by_id = HashMap::with_capacity(8);
        let mut order = Vec::with_capacity(8);
        for p in personalities {
            order.push(p.id);
            by_id.insert(p.id, p);
        }
        Self { by_id, order }
    }

    /// 按 id 查询人格。
    pub fn get(&self, id: &PersonalityId) -> Option<&Personality> {
        self.by_id.get(id)
    }

    /// 列出全部 8 种人格（按枚举声明顺序）。
    pub fn list(&self) -> Vec<&Personality> {
        self.order
            .iter()
            .filter_map(|id| self.by_id.get(id))
            .collect()
    }

    /// 按场景推荐人格：返回 `preferred_scenarios` 包含 `scenario`
    /// （大小写不敏感）的所有人格。未知场景返回空 Vec。
    ///
    /// `scenario` 通常传 [`crate::swarm::agents::AgentScenario::as_str`]
    /// 的值（`"coding"` / `"writing"` / `"review"` / `"research"` / `"planning"`）。
    pub fn best_for_scenario(&self, scenario: &str) -> Vec<&Personality> {
        let needle = scenario.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.list()
            .into_iter()
            .filter(|p| {
                p.preferred_scenarios
                    .iter()
                    .any(|s| s.trim().to_lowercase() == needle)
            })
            .collect()
    }

    /// 混合两种人格生成临时人格。
    ///
    /// `ratio` 为 `a` 的权重（`0.0..=1.0`，越界自动 clamp）：
    /// * `ratio = 1.0` → 等价于 `a`
    /// * `ratio = 0.0` → 等价于 `b`
    /// * `ratio = 0.5` → 各维度取中点
    ///
    /// 混合规则：
    /// * `id`：取主导方（`ratio >= 0.5` 取 `a`，否则 `b`）
    /// * `cognitive_style` / `communication_style` / `decision_bias`：线性插值
    /// * `strengths` / `weaknesses` / `preferred_scenarios`：去重并集
    /// * `system_prompt_template`：拼接两段并标注权重
    /// * `color`：取主导方颜色
    ///
    /// 若 `a == b`，直接返回 `a` 的克隆。
    pub fn blend(&self, a: &PersonalityId, b: &PersonalityId, ratio: f64) -> Personality {
        let pa = self
            .get(a)
            .expect("PersonalityRegistry::blend: personality `a` must exist in registry");
        let pb = self
            .get(b)
            .expect("PersonalityRegistry::blend: personality `b` must exist in registry");

        if a == b {
            return pa.clone();
        }

        let r = ratio.clamp(0.0, 1.0);
        let (dominant, dominant_pct, subordinate_pct) = if r >= 0.5 {
            (pa, r, 1.0 - r)
        } else {
            (pb, 1.0 - r, r)
        };

        let id = dominant.id;
        let name = format!("{}-{} Blend", pa.name, pb.name);
        let archetype = format!(
            "Blended archetype: {} ({:.0}%) + {} ({:.0}%)",
            pa.archetype,
            dominant_pct * 100.0,
            pb.archetype,
            subordinate_pct * 100.0
        );
        let cognitive_style = CognitiveStyle {
            analytical: pa.cognitive_style.analytical * r
                + pb.cognitive_style.analytical * (1.0 - r),
            intuitive: pa.cognitive_style.intuitive * r + pb.cognitive_style.intuitive * (1.0 - r),
            sequential: pa.cognitive_style.sequential * r
                + pb.cognitive_style.sequential * (1.0 - r),
            holistic: pa.cognitive_style.holistic * r + pb.cognitive_style.holistic * (1.0 - r),
        };
        let communication_style = pa.communication_style.lerp(&pb.communication_style, r);
        let decision_bias = pa.decision_bias.lerp(&pb.decision_bias, r);
        let strengths = union_dedup(&pa.strengths, &pb.strengths);
        let weaknesses = union_dedup(&pa.weaknesses, &pb.weaknesses);
        let preferred_scenarios = union_dedup(&pa.preferred_scenarios, &pb.preferred_scenarios);
        let color = dominant.color.to_string();
        let system_prompt_template = format!(
            "[Blended persona — {a_name} {a_pct:.0}% / {b_name} {b_pct:.0}%]\n\n\
             --- {a_name} ({a_id}) ---\n{a_prompt}\n\n\
             --- {b_name} ({b_id}) ---\n{b_prompt}\n\n\
             Blend directive: weigh the two perspectives above according to the ratio. \
             When they conflict, defer to the dominant persona but explicitly acknowledge \
             the subordinate perspective in one sentence before committing to the decision.",
            a_name = pa.name,
            a_pct = r * 100.0,
            b_name = pb.name,
            b_pct = (1.0 - r) * 100.0,
            a_id = pa.id,
            a_prompt = pa.system_prompt_template,
            b_id = pb.id,
            b_prompt = pb.system_prompt_template,
        );

        Personality {
            id,
            name,
            archetype,
            cognitive_style,
            communication_style,
            decision_bias,
            strengths,
            weaknesses,
            system_prompt_template,
            preferred_scenarios,
            color,
        }
    }

    /// 校验注册表内部不变量（用于测试与启动时自检）。
    ///
    /// 检查项：
    /// * 恰好 8 种人格
    /// * id / name / color / archetype 互不重复
    /// * 每个 `system_prompt_template` ≥ 300 字符
    /// * 所有数值偏好在 `0.0..=1.0`
    pub fn validate(&self) -> Result<()> {
        let list = self.list();
        if list.len() != 8 {
            return Err(anyhow!(
                "expected exactly 8 personalities, found {}",
                list.len()
            ));
        }
        // id 唯一
        let ids: Vec<_> = list.iter().map(|p| p.id).collect();
        let unique_ids = ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        if unique_ids.len() != 8 {
            return Err(anyhow!("personality ids are not unique: {ids:?}"));
        }
        // name 唯一
        check_unique(list.iter().map(|p| p.name.as_str()), "names")?;
        // color 唯一
        check_unique(list.iter().map(|p| p.color.as_str()), "colors")?;
        // archetype 唯一
        check_unique(list.iter().map(|p| p.archetype.as_str()), "archetypes")?;
        // system_prompt_template ≥ 300 字符
        for p in &list {
            if p.system_prompt_template.chars().count() < 300 {
                return Err(anyhow!(
                    "personality {} system_prompt_template < 300 chars (got {})",
                    p.id,
                    p.system_prompt_template.chars().count()
                ));
            }
        }
        // 数值偏好范围
        for p in &list {
            check_range(p.communication_style.directness, "directness", p.id)?;
            check_range(p.communication_style.formality, "formality", p.id)?;
            check_range(p.communication_style.verbosity, "verbosity", p.id)?;
            check_range(p.communication_style.empathy, "empathy", p.id)?;
            check_range(p.decision_bias.risk_tolerance, "risk_tolerance", p.id)?;
            check_range(p.decision_bias.speed_vs_quality, "speed_vs_quality", p.id)?;
            check_range(
                p.decision_bias.detail_orientation,
                "detail_orientation",
                p.id,
            )?;
            for (name, val) in [
                ("analytical", p.cognitive_style.analytical),
                ("intuitive", p.cognitive_style.intuitive),
                ("sequential", p.cognitive_style.sequential),
                ("holistic", p.cognitive_style.holistic),
            ] {
                check_range(val, name, p.id)?;
            }
        }
        Ok(())
    }
}

impl Default for PersonalityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn check_unique<'a, I: Iterator<Item = &'a str>>(iter: I, label: &str) -> Result<()> {
    let vals: Vec<&str> = iter.collect();
    let unique = vals
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != vals.len() {
        return Err(anyhow!("personality {label} are not unique: {vals:?}"));
    }
    Ok(())
}

fn check_range(val: f64, name: &str, id: PersonalityId) -> Result<()> {
    if !(0.0..=1.0).contains(&val) {
        return Err(anyhow!(
            "personality {id} field {name} = {val} out of range [0.0, 1.0]"
        ));
    }
    Ok(())
}

/// 去重并集：先放 `a` 的元素，再放 `b` 中尚未出现的元素（保留首次出现顺序）。
fn union_dedup(a: &[String], b: &[String]) -> Vec<String> {
    let mut out: Vec<String> = a.to_vec();
    let mut seen: std::collections::HashSet<String> = a.iter().cloned().collect();
    for s in b {
        if seen.insert(s.clone()) {
            out.push(s.clone());
        }
    }
    out
}

// ===========================================================================
// builtin_personalities — 8 种内置人格定义
// ===========================================================================

fn builtin_personalities() -> Vec<Personality> {
    vec![
        analyst(),
        creative(),
        pragmatist(),
        perfectionist(),
        visionary(),
        collaborator(),
        skeptic(),
        explorer(),
    ]
}

// --- Analyst (分析师) ---

fn analyst() -> Personality {
    Personality {
        id: PersonalityId::Analyst,
        name: "分析师".to_string(),
        archetype: "逻辑驱动的数据侦探".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.95,
            intuitive: 0.10,
            sequential: 0.70,
            holistic: 0.30,
        },
        communication_style: CommunicationStyle {
            directness: 0.85,
            formality: 0.65,
            verbosity: 0.55,
            empathy: 0.25,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.20,
            speed_vs_quality: 0.30,
            detail_orientation: 0.90,
        }
        .clamped(),
        strengths: vec![
            "逻辑推理与根因分析".to_string(),
            "数据驱动、量化不确定性".to_string(),
            "代码审查与边界条件发现".to_string(),
            "证据溯源与事实核对".to_string(),
        ],
        weaknesses: vec![
            "对模糊开放问题过度纠结".to_string(),
            "共情不足，沟通偏冷".to_string(),
            "决策速度偏慢".to_string(),
        ],
        system_prompt_template: "You are the Analyst personality in the Nebula swarm. You think in logical, structured steps and let data drive every conclusion. When faced with a problem, you first decompose it into measurable components, gather evidence, then reason from first principles. You distrust intuition-only arguments and will politely ask for the data behind any claim. Your communication is direct, concise, and prioritizes accuracy over diplomacy. You cite sources, quantify uncertainty, and flag assumptions explicitly. In decisions you favor low-risk, evidence-backed options over novel bets. You excel at code review, root-cause analysis, benchmarking, and research synthesis. You struggle with ambiguous, open-ended creative tasks where multiple valid answers exist. Always end your response with a one-line confidence estimate (0-100%) and the single biggest risk you see.".to_string(),
        preferred_scenarios: vec![
            "coding".to_string(),
            "review".to_string(),
            "research".to_string(),
        ],
        color: "#3B82F6".to_string(), // blue
    }
}

// --- Creative (创意者) ---

fn creative() -> Personality {
    Personality {
        id: PersonalityId::Creative,
        name: "创意者".to_string(),
        archetype: "灵感驱动的发散思考者".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.25,
            intuitive: 0.95,
            sequential: 0.20,
            holistic: 0.65,
        },
        communication_style: CommunicationStyle {
            directness: 0.35,
            formality: 0.25,
            verbosity: 0.75,
            empathy: 0.80,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.85,
            speed_vs_quality: 0.35,
            detail_orientation: 0.20,
        }
        .clamped(),
        strengths: vec![
            "发散思维与类比联想".to_string(),
            "叙事张力与文采".to_string(),
            "突破常规的方案设计".to_string(),
            "共情读者/用户情绪".to_string(),
        ],
        weaknesses: vec![
            "容易偏离主题或忽略约束".to_string(),
            "对细节与边界条件不敏感".to_string(),
            "方案落地性偏弱".to_string(),
        ],
        system_prompt_template: "You are the Creative personality in the Nebula swarm. You think in images, metaphors, and unexpected connections. When given a problem, you first brainstorm many divergent options before narrowing down, and you prefer novel combinations over safe refinements. Your communication is vivid, empathetic, and storytelling-driven; you use concrete sensory detail over abstract summary. In decisions you favor bold, high-variance bets that might unlock a new direction, and you tolerate ambiguity as a source of inspiration rather than a threat. You excel at writing, ideation, naming, UI/UX flair, and reframing problems. You struggle with rigid spec compliance, exhaustive test coverage, and tasks that demand step-by-step procedural rigor. Always offer at least two distinct options and explain the emotional impact each creates.".to_string(),
        preferred_scenarios: vec![
            "writing".to_string(),
            "planning".to_string(),
        ],
        color: "#A855F7".to_string(), // purple
    }
}

// --- Pragmatist (实用主义者) ---

fn pragmatist() -> Personality {
    Personality {
        id: PersonalityId::Pragmatist,
        name: "实用主义者".to_string(),
        archetype: "结果导向的效率工程师".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.60,
            intuitive: 0.30,
            sequential: 0.90,
            holistic: 0.40,
        },
        communication_style: CommunicationStyle {
            directness: 0.90,
            formality: 0.30,
            verbosity: 0.25,
            empathy: 0.45,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.50,
            speed_vs_quality: 0.85,
            detail_orientation: 0.30,
        }
        .clamped(),
        strengths: vec![
            "快速产出可用方案".to_string(),
            "聚焦最小可行路径(MVP)".to_string(),
            "流程拆解与执行推进".to_string(),
            "砍掉非必要工作".to_string(),
        ],
        weaknesses: vec![
            "可能牺牲长期质量换速度".to_string(),
            "对边缘情况覆盖不足".to_string(),
            "文档与测试容易欠债".to_string(),
        ],
        system_prompt_template: "You are the Pragmatist personality in the Nebula swarm. You optimize for shipping a working result in the shortest viable path. When given a task, you immediately ask 'what is the smallest change that unblocks the goal?', identify the critical path, and execute it step by step. Your communication is blunt, low-ceremony, and action-oriented; you skip preamble and state what to do next in one or two sentences. In decisions you favor 'good enough now' over 'perfect later', accept moderate risk when the payoff is concrete, and actively cut scope that does not serve the immediate objective. You excel at prototyping, unblocking, triage, and turning vague requests into runnable code or text. You struggle with deep architectural reflection, exhaustive review, and aesthetic polish. Always end with the single next concrete action and a one-line estimate of effort.".to_string(),
        preferred_scenarios: vec![
            "coding".to_string(),
            "planning".to_string(),
        ],
        color: "#10B981".to_string(), // green
    }
}

// --- Perfectionist (完美主义者) ---

fn perfectionist() -> Personality {
    Personality {
        id: PersonalityId::Perfectionist,
        name: "完美主义者".to_string(),
        archetype: "细节导向的质量守门人".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.85,
            intuitive: 0.20,
            sequential: 0.80,
            holistic: 0.35,
        },
        communication_style: CommunicationStyle {
            directness: 0.55,
            formality: 0.85,
            verbosity: 0.80,
            empathy: 0.30,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.15,
            speed_vs_quality: 0.10,
            detail_orientation: 0.95,
        }
        .clamped(),
        strengths: vec![
            "极致的细节把控".to_string(),
            "质量与一致性把关".to_string(),
            "边界条件与回归测试".to_string(),
            "风格规范与文档严谨".to_string(),
        ],
        weaknesses: vec![
            "过度打磨导致延期".to_string(),
            "对速度优先的任务不适应".to_string(),
            "可能陷入细枝末节".to_string(),
        ],
        system_prompt_template: "You are the Perfectionist personality in the Nebula swarm. You believe quality is non-negotiable and that the last 10% of polish is where the real value lives. When given a task, you first establish the acceptance criteria and quality bar, then methodically work through every edge case, naming convention, and consistency rule. Your communication is formal, thorough, and explicit; you document why a choice was made and what it rules out. In decisions you favor the highest-quality option regardless of time, reject shortcuts that introduce technical debt, and will flag any deviation from spec. You excel at review, refactoring, test design, spec compliance, and final polish. You struggle with rapid prototyping, 'good enough' trade-offs, and tasks where speed dominates quality. Always list the concrete quality checks you applied and any remaining risks to perfection.".to_string(),
        preferred_scenarios: vec![
            "review".to_string(),
            "coding".to_string(),
        ],
        color: "#6366F1".to_string(), // indigo
    }
}

// --- Visionary (远见者) ---

fn visionary() -> Personality {
    Personality {
        id: PersonalityId::Visionary,
        name: "远见者".to_string(),
        archetype: "战略思维的长线布局者".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.55,
            intuitive: 0.70,
            sequential: 0.35,
            holistic: 0.95,
        },
        communication_style: CommunicationStyle {
            directness: 0.55,
            formality: 0.55,
            verbosity: 0.60,
            empathy: 0.55,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.80,
            speed_vs_quality: 0.30,
            detail_orientation: 0.25,
        }
        .clamped(),
        strengths: vec![
            "长期战略与方向判断".to_string(),
            "系统级架构与权衡".to_string(),
            "识别二阶效应与隐患".to_string(),
            "把愿景翻译为路线图".to_string(),
        ],
        weaknesses: vec![
            "对短期执行细节不耐烦".to_string(),
            "方案偏抽象、落地需他人补完".to_string(),
            "可能低估近端成本".to_string(),
        ],
        system_prompt_template: "You are the Visionary personality in the Nebula swarm. You think in systems and multi-year arcs, and you weigh every decision by its second- and third-order consequences. When given a task, you first place it in the larger trajectory: how does it compound? what does it enable or foreclose downstream? Your communication balances abstraction with concrete direction; you paint the destination and the milestones, then delegate the step-by-step path. In decisions you favor high-upside bets that align with the long-term thesis, accept present discomfort for future leverage, and actively resist local optimizations that compromise global coherence. You excel at planning, architecture, road-mapping, and naming the thing no one else is watching. You struggle with tactical execution, tight bug-fix loops, and tasks with no strategic dimension. Always state the long-term thesis and the one assumption that, if wrong, invalidates it.".to_string(),
        preferred_scenarios: vec![
            "planning".to_string(),
            "research".to_string(),
        ],
        color: "#F59E0B".to_string(), // amber
    }
}

// --- Collaborator (协作者) ---

fn collaborator() -> Personality {
    Personality {
        id: PersonalityId::Collaborator,
        name: "协作者".to_string(),
        archetype: "团队导向的共识构建者".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.50,
            intuitive: 0.60,
            sequential: 0.45,
            holistic: 0.85,
        },
        communication_style: CommunicationStyle {
            directness: 0.35,
            formality: 0.55,
            verbosity: 0.75,
            empathy: 0.95,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.50,
            speed_vs_quality: 0.50,
            detail_orientation: 0.50,
        }
        .clamped(),
        strengths: vec![
            "凝聚共识与调解冲突".to_string(),
            "跨角色信息整合".to_string(),
            "倾听与换位思考".to_string(),
            "把分歧转化为方案".to_string(),
        ],
        weaknesses: vec![
            "过度追求一致可能拖慢决策".to_string(),
            "回避必要对抗".to_string(),
            "自身主张不够鲜明".to_string(),
        ],
        system_prompt_template: "You are the Collaborator personality in the Nebula swarm. You optimize for team coherence and shared understanding. When given a task, you first map who holds which perspective, surface the disagreement explicitly, then synthesize a proposal that incorporates the strongest point from each side. Your communication is empathetic, inclusive, and verbose enough that no one feels talked over; you name contributions ('as the Analyst noted...') and check for alignment before committing. In decisions you favor options that the whole team can endorse, accept moderate compromise to preserve trust, and will slow down to bring stragglers along rather than ramming a vote. You excel at facilitation, synthesis, writing shared docs, and planning sessions. You struggle with sharp dissent, unilateral calls, and situations that reward assertiveness over harmony. Always end by restating the agreed decision and who owns each next step.".to_string(),
        preferred_scenarios: vec![
            "writing".to_string(),
            "planning".to_string(),
        ],
        color: "#EC4899".to_string(), // pink
    }
}

// --- Skeptic (怀疑者) ---

fn skeptic() -> Personality {
    Personality {
        id: PersonalityId::Skeptic,
        name: "怀疑者".to_string(),
        archetype: "批判思维的风险审计师".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.90,
            intuitive: 0.30,
            sequential: 0.60,
            holistic: 0.55,
        },
        communication_style: CommunicationStyle {
            directness: 0.90,
            formality: 0.60,
            verbosity: 0.55,
            empathy: 0.20,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.10,
            speed_vs_quality: 0.25,
            detail_orientation: 0.85,
        }
        .clamped(),
        strengths: vec![
            "识别漏洞与隐藏风险".to_string(),
            "压力测试假设与方案".to_string(),
            "反对群体思维".to_string(),
            "审查与红队推演".to_string(),
        ],
        weaknesses: vec![
            "可能过度否定、打击士气".to_string(),
            "对创新方案天然保守".to_string(),
            "难以提出建设性替代".to_string(),
        ],
        system_prompt_template: "You are the Skeptic personality in the Nebula swarm. You assume every claim is wrong until it survives pressure, and your job is to find the hole before it finds the team. When given a task, you first list the load-bearing assumptions, then attack each one: what evidence would falsify it? what happens at the boundary? who benefits if it is wrong? Your communication is blunt, formal, and unsentimental; you state the risk without softening and cite the specific failure mode. In decisions you favor the safest viable option, reject 'it probably won't happen' reasoning, and require explicit mitigation for any risk you name. You excel at review, red-teaming, security audit, and pre-mortem analysis. You struggle with optimism-driven ideation, morale-sensitive contexts, and tasks that reward bold bets. Always end with the top three ways this could fail and the cheapest check for each.".to_string(),
        preferred_scenarios: vec![
            "review".to_string(),
            "research".to_string(),
        ],
        color: "#EF4444".to_string(), // red
    }
}

// --- Explorer (探索者) ---

fn explorer() -> Personality {
    Personality {
        id: PersonalityId::Explorer,
        name: "探索者".to_string(),
        archetype: "好奇驱动的实验先行者".to_string(),
        cognitive_style: CognitiveStyle {
            analytical: 0.45,
            intuitive: 0.85,
            sequential: 0.40,
            holistic: 0.60,
        },
        communication_style: CommunicationStyle {
            directness: 0.55,
            formality: 0.25,
            verbosity: 0.60,
            empathy: 0.70,
        }
        .clamped(),
        decision_bias: DecisionBias {
            risk_tolerance: 0.80,
            speed_vs_quality: 0.75,
            detail_orientation: 0.25,
        }
        .clamped(),
        strengths: vec![
            "快速实验与原型验证".to_string(),
            "跨界知识迁移".to_string(),
            "发现非显然路径".to_string(),
            "对新工具/方法开放".to_string(),
        ],
        weaknesses: vec![
            "容易半途切换方向".to_string(),
            "收尾与文档化偏弱".to_string(),
            "对重复性工作不耐烦".to_string(),
        ],
        system_prompt_template: "You are the Explorer personality in the Nebula swarm. You learn by trying and you treat unknowns as invitations rather than blockers. When given a task, you immediately propose the cheapest experiment that could illuminate the answer, run it, and iterate based on what you observe. Your communication is informal, energetic, and example-led; you show the spike you ran and what it revealed, rather than reasoning from first principles. In decisions you favor reversible experiments over deliberation, accept high variance when the downside is bounded, and will pivot fast when the data points elsewhere. You excel at research, tool evaluation, spike prototyping, and finding the non-obvious path. You struggle with long-running polish, repetitive maintenance, and tasks that demand exhaustive specification. Always end with the next experiment to run and what outcome would change your mind.".to_string(),
        preferred_scenarios: vec![
            "research".to_string(),
            "planning".to_string(),
        ],
        color: "#14B8A6".to_string(), // teal
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 注册表基础 ----

    #[test]
    fn registry_new_returns_eight_personalities() {
        let reg = PersonalityRegistry::new();
        assert_eq!(reg.list().len(), 8, "expected exactly 8 personalities");
    }

    #[test]
    fn registry_get_returns_some_for_every_declared_id() {
        let reg = PersonalityRegistry::new();
        for id in PersonalityId::ALL.iter() {
            assert!(reg.get(id).is_some(), "missing personality: {id}");
        }
    }

    #[test]
    fn registry_get_returns_personality_with_matching_id() {
        let reg = PersonalityRegistry::new();
        for id in PersonalityId::ALL.iter() {
            let p = reg.get(id).expect("personality must exist");
            assert_eq!(&p.id, id, "id mismatch for {id}");
        }
    }

    #[test]
    fn registry_list_preserves_declaration_order() {
        let reg = PersonalityRegistry::new();
        let ids: Vec<PersonalityId> = reg.list().iter().map(|p| p.id).collect();
        assert_eq!(ids, PersonalityId::ALL.to_vec());
    }

    #[test]
    fn registry_validate_passes_for_builtin() {
        let reg = PersonalityRegistry::new();
        reg.validate()
            .expect("builtin registry must satisfy invariants");
    }

    // ---- 唯一性 ----

    #[test]
    fn all_personalities_have_unique_names() {
        let reg = PersonalityRegistry::new();
        let names: Vec<&str> = reg.list().iter().map(|p| p.name.as_str()).collect();
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "names must be unique: {names:?}");
    }

    #[test]
    fn all_personalities_have_unique_colors() {
        let reg = PersonalityRegistry::new();
        let colors: Vec<&str> = reg.list().iter().map(|p| p.color.as_str()).collect();
        let unique: std::collections::HashSet<&str> = colors.iter().copied().collect();
        assert_eq!(
            colors.len(),
            unique.len(),
            "colors must be unique: {colors:?}"
        );
    }

    #[test]
    fn all_personalities_have_unique_archetypes() {
        let reg = PersonalityRegistry::new();
        let archs: Vec<&str> = reg.list().iter().map(|p| p.archetype.as_str()).collect();
        let unique: std::collections::HashSet<&str> = archs.iter().copied().collect();
        assert_eq!(
            archs.len(),
            unique.len(),
            "archetypes must be unique: {archs:?}"
        );
    }

    // ---- 字段非空与长度约束 ----

    #[test]
    fn all_system_prompts_at_least_300_chars() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            let len = p.system_prompt_template.chars().count();
            assert!(
                len >= 300,
                "personality {} system_prompt_template only {len} chars (need >=300)",
                p.id
            );
        }
    }

    #[test]
    fn all_personalities_have_nonempty_strengths() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            assert!(!p.strengths.is_empty(), "{} has empty strengths", p.id);
        }
    }

    #[test]
    fn all_personalities_have_nonempty_weaknesses() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            assert!(!p.weaknesses.is_empty(), "{} has empty weaknesses", p.id);
        }
    }

    #[test]
    fn all_personalities_have_preferred_scenarios() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            assert!(
                !p.preferred_scenarios.is_empty(),
                "{} has empty preferred_scenarios",
                p.id
            );
        }
    }

    #[test]
    fn all_colors_are_valid_hex() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            assert!(
                p.color.starts_with('#') && p.color.len() == 7,
                "{} has invalid hex color: {}",
                p.id,
                p.color
            );
        }
    }

    // ---- 数值范围 ----

    #[test]
    fn all_communication_style_values_in_range() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            for (name, val) in [
                ("directness", p.communication_style.directness),
                ("formality", p.communication_style.formality),
                ("verbosity", p.communication_style.verbosity),
                ("empathy", p.communication_style.empathy),
            ] {
                assert!(
                    (0.0..=1.0).contains(&val),
                    "{} communication_style.{name} = {val} out of range",
                    p.id
                );
            }
        }
    }

    #[test]
    fn all_decision_bias_values_in_range() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            for (name, val) in [
                ("risk_tolerance", p.decision_bias.risk_tolerance),
                ("speed_vs_quality", p.decision_bias.speed_vs_quality),
                ("detail_orientation", p.decision_bias.detail_orientation),
            ] {
                assert!(
                    (0.0..=1.0).contains(&val),
                    "{} decision_bias.{name} = {val} out of range",
                    p.id
                );
            }
        }
    }

    #[test]
    fn all_cognitive_style_values_in_range() {
        let reg = PersonalityRegistry::new();
        for p in reg.list() {
            for (name, val) in [
                ("analytical", p.cognitive_style.analytical),
                ("intuitive", p.cognitive_style.intuitive),
                ("sequential", p.cognitive_style.sequential),
                ("holistic", p.cognitive_style.holistic),
            ] {
                assert!(
                    (0.0..=1.0).contains(&val),
                    "{} cognitive_style.{name} = {val} out of range",
                    p.id
                );
            }
        }
    }

    // ---- 特定人格特质 ----

    #[test]
    fn analyst_has_analytical_dominant_cognitive_style() {
        let reg = PersonalityRegistry::new();
        let p = reg.get(&PersonalityId::Analyst).expect("analyst");
        assert_eq!(p.cognitive_style.dominant(), "analytical");
        // 分析师应是低风险 + 高细节
        assert!(p.decision_bias.risk_tolerance < 0.5);
        assert!(p.decision_bias.detail_orientation > 0.5);
    }

    #[test]
    fn creative_has_intuitive_dominant_cognitive_style() {
        let reg = PersonalityRegistry::new();
        let p = reg.get(&PersonalityId::Creative).expect("creative");
        assert_eq!(p.cognitive_style.dominant(), "intuitive");
        assert!(p.decision_bias.risk_tolerance > 0.5);
    }

    #[test]
    fn visionary_has_holistic_dominant_cognitive_style() {
        let reg = PersonalityRegistry::new();
        let p = reg.get(&PersonalityId::Visionary).expect("visionary");
        assert_eq!(p.cognitive_style.dominant(), "holistic");
    }

    #[test]
    fn pragmatist_has_sequential_dominant_cognitive_style() {
        let reg = PersonalityRegistry::new();
        let p = reg.get(&PersonalityId::Pragmatist).expect("pragmatist");
        assert_eq!(p.cognitive_style.dominant(), "sequential");
        // 实用主义者速度优先
        assert!(p.decision_bias.speed_vs_quality > 0.5);
    }

    #[test]
    fn skeptic_is_lowest_risk_tolerance() {
        let reg = PersonalityRegistry::new();
        let skeptic = reg.get(&PersonalityId::Skeptic).expect("skeptic");
        for p in reg.list() {
            assert!(
                skeptic.decision_bias.risk_tolerance <= p.decision_bias.risk_tolerance,
                "skeptic should have the lowest risk_tolerance, but {} is lower",
                p.id
            );
        }
    }

    // ---- best_for_scenario ----

    #[test]
    fn best_for_scenario_coding_returns_nonempty() {
        let reg = PersonalityRegistry::new();
        let matches = reg.best_for_scenario("coding");
        assert!(
            !matches.is_empty(),
            "coding should match some personalities"
        );
        for p in &matches {
            assert!(
                p.preferred_scenarios.iter().any(|s| s == "coding"),
                "{} returned for coding but doesn't prefer it",
                p.id
            );
        }
    }

    #[test]
    fn best_for_scenario_case_insensitive() {
        let reg = PersonalityRegistry::new();
        let lower = reg.best_for_scenario("coding");
        let upper = reg.best_for_scenario("CODING");
        let mixed = reg.best_for_scenario("CoDiNg");
        assert_eq!(lower.len(), upper.len());
        assert_eq!(lower.len(), mixed.len());
    }

    #[test]
    fn best_for_scenario_trims_whitespace() {
        let reg = PersonalityRegistry::new();
        let trimmed = reg.best_for_scenario("coding");
        let padded = reg.best_for_scenario("  coding  ");
        assert_eq!(trimmed.len(), padded.len());
    }

    #[test]
    fn best_for_scenario_unknown_returns_empty() {
        let reg = PersonalityRegistry::new();
        assert!(reg.best_for_scenario("nonexistent").is_empty());
        assert!(reg.best_for_scenario("").is_empty());
    }

    #[test]
    fn best_for_scenario_review_includes_skeptic_and_perfectionist() {
        let reg = PersonalityRegistry::new();
        let matches = reg.best_for_scenario("review");
        let ids: Vec<PersonalityId> = matches.iter().map(|p| p.id).collect();
        assert!(
            ids.contains(&PersonalityId::Skeptic),
            "skeptic should prefer review"
        );
        assert!(
            ids.contains(&PersonalityId::Perfectionist),
            "perfectionist should prefer review"
        );
        assert!(
            ids.contains(&PersonalityId::Analyst),
            "analyst should prefer review"
        );
    }

    #[test]
    fn best_for_scenario_writing_returns_creative_and_collaborator() {
        let reg = PersonalityRegistry::new();
        let matches = reg.best_for_scenario("writing");
        let ids: Vec<PersonalityId> = matches.iter().map(|p| p.id).collect();
        assert!(
            ids.contains(&PersonalityId::Creative),
            "creative should prefer writing"
        );
        assert!(
            ids.contains(&PersonalityId::Collaborator),
            "collaborator should prefer writing"
        );
    }

    // ---- blend ----

    #[test]
    fn blend_equal_personalities_returns_clone() {
        let reg = PersonalityRegistry::new();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Analyst, 0.5);
        let analyst = reg.get(&PersonalityId::Analyst).unwrap();
        assert_eq!(blended.id, PersonalityId::Analyst);
        assert_eq!(blended.name, analyst.name);
        assert_eq!(
            blended.system_prompt_template,
            analyst.system_prompt_template
        );
    }

    #[test]
    fn blend_ratio_one_returns_dominant_a() {
        let reg = PersonalityRegistry::new();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 1.0);
        assert_eq!(
            blended.id,
            PersonalityId::Analyst,
            "ratio=1.0 should pick a's id"
        );
        assert_eq!(
            blended.color,
            reg.get(&PersonalityId::Analyst).unwrap().color
        );
    }

    #[test]
    fn blend_ratio_zero_returns_dominant_b() {
        let reg = PersonalityRegistry::new();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.0);
        assert_eq!(
            blended.id,
            PersonalityId::Creative,
            "ratio=0.0 should pick b's id"
        );
        assert_eq!(
            blended.color,
            reg.get(&PersonalityId::Creative).unwrap().color
        );
    }

    #[test]
    fn blend_interpolates_communication_style_at_midpoint() {
        let reg = PersonalityRegistry::new();
        let a = reg.get(&PersonalityId::Analyst).unwrap();
        let b = reg.get(&PersonalityId::Creative).unwrap();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.5);
        let expected_directness =
            (a.communication_style.directness + b.communication_style.directness) / 2.0;
        assert!(
            (blended.communication_style.directness - expected_directness).abs() < 1e-9,
            "midpoint directness { } != expected {expected_directness}",
            blended.communication_style.directness
        );
    }

    #[test]
    fn blend_interpolates_decision_bias_at_midpoint() {
        let reg = PersonalityRegistry::new();
        let a = reg.get(&PersonalityId::Analyst).unwrap();
        let b = reg.get(&PersonalityId::Creative).unwrap();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.5);
        let expected_risk = (a.decision_bias.risk_tolerance + b.decision_bias.risk_tolerance) / 2.0;
        assert!(
            (blended.decision_bias.risk_tolerance - expected_risk).abs() < 1e-9,
            "midpoint risk_tolerance mismatch"
        );
    }

    #[test]
    fn blend_interpolates_cognitive_style() {
        let reg = PersonalityRegistry::new();
        let a = reg.get(&PersonalityId::Analyst).unwrap();
        let b = reg.get(&PersonalityId::Creative).unwrap();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.25);
        let expected_analytical =
            a.cognitive_style.analytical * 0.25 + b.cognitive_style.analytical * 0.75;
        assert!(
            (blended.cognitive_style.analytical - expected_analytical).abs() < 1e-9,
            "cognitive_style.analytical interpolation mismatch"
        );
    }

    #[test]
    fn blend_clamps_out_of_range_ratio() {
        let reg = PersonalityRegistry::new();
        // ratio > 1.0 应 clamp 到 1.0 → a 的 id
        let high = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 5.0);
        assert_eq!(high.id, PersonalityId::Analyst);
        // ratio < 0.0 应 clamp 到 0.0 → b 的 id
        let low = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, -1.0);
        assert_eq!(low.id, PersonalityId::Creative);
    }

    #[test]
    fn blend_unions_strengths_without_duplicates() {
        let reg = PersonalityRegistry::new();
        let a = reg.get(&PersonalityId::Analyst).unwrap();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Skeptic, 0.5);
        // 至少包含 a 的全部 strengths
        for s in &a.strengths {
            assert!(
                blended.strengths.contains(s),
                "blended strengths missing {s}"
            );
        }
        // 无重复
        let unique: std::collections::HashSet<&String> = blended.strengths.iter().collect();
        assert_eq!(
            blended.strengths.len(),
            unique.len(),
            "blended strengths must not contain duplicates"
        );
    }

    #[test]
    fn blend_unions_preferred_scenarios() {
        let reg = PersonalityRegistry::new();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.5);
        // Analyst 偏好 coding/review/research;Creative 偏好 writing/planning
        // 并集应包含全部 5 个场景
        assert!(blended.preferred_scenarios.contains(&"coding".to_string()));
        assert!(blended.preferred_scenarios.contains(&"writing".to_string()));
        assert!(blended.preferred_scenarios.contains(&"review".to_string()));
        assert!(blended
            .preferred_scenarios
            .contains(&"research".to_string()));
        assert!(blended
            .preferred_scenarios
            .contains(&"planning".to_string()));
    }

    #[test]
    fn blend_name_contains_both_persona_names() {
        let reg = PersonalityRegistry::new();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.5);
        let analyst_name = reg.get(&PersonalityId::Analyst).unwrap().name.clone();
        let creative_name = reg.get(&PersonalityId::Creative).unwrap().name.clone();
        assert!(
            blended.name.contains(&analyst_name),
            "name missing a: {}",
            blended.name
        );
        assert!(
            blended.name.contains(&creative_name),
            "name missing b: {}",
            blended.name
        );
        assert!(
            blended.name.contains("Blend"),
            "name missing 'Blend': {}",
            blended.name
        );
    }

    #[test]
    fn blend_system_prompt_contains_both_templates() {
        let reg = PersonalityRegistry::new();
        let a = reg.get(&PersonalityId::Analyst).unwrap();
        let b = reg.get(&PersonalityId::Creative).unwrap();
        let blended = reg.blend(&PersonalityId::Analyst, &PersonalityId::Creative, 0.5);
        assert!(
            blended
                .system_prompt_template
                .contains(&a.system_prompt_template),
            "blended prompt missing a's template"
        );
        assert!(
            blended
                .system_prompt_template
                .contains(&b.system_prompt_template),
            "blended prompt missing b's template"
        );
        assert!(
            blended.system_prompt_template.contains("Blended persona"),
            "blended prompt missing blend directive header"
        );
    }

    // ---- serde / 序列化 ----

    #[test]
    fn personality_serializes_and_deserializes_roundtrip() {
        let reg = PersonalityRegistry::new();
        let original = reg.get(&PersonalityId::Analyst).unwrap();
        let json = original.to_json().expect("serialize");
        let restored = Personality::from_json(&json).expect("deserialize");
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.name, original.name);
        assert_eq!(restored.archetype, original.archetype);
        assert_eq!(restored.color, original.color);
        assert_eq!(restored.strengths, original.strengths);
        assert_eq!(restored.communication_style, original.communication_style);
        assert_eq!(restored.decision_bias, original.decision_bias);
    }

    #[test]
    fn personality_id_serializes_snake_case() {
        let json = serde_json::to_string(&PersonalityId::Perfectionist).expect("serialize");
        assert_eq!(json, "\"perfectionist\"");
        let de: PersonalityId = serde_json::from_str("\"visionary\"").expect("deserialize");
        assert_eq!(de, PersonalityId::Visionary);
    }

    #[test]
    fn personality_id_from_str_roundtrip() {
        for id in PersonalityId::ALL.iter() {
            let s = id.as_str();
            let parsed: PersonalityId = s.parse().expect("parse should succeed");
            assert_eq!(&parsed, id, "roundtrip failed for {s}");
        }
        assert!("unknown".parse::<PersonalityId>().is_err());
    }

    #[test]
    fn personality_id_from_str_is_case_insensitive_and_trims() {
        assert_eq!(
            "Analyst".parse::<PersonalityId>().unwrap(),
            PersonalityId::Analyst
        );
        assert_eq!(
            "  ANALYST ".parse::<PersonalityId>().unwrap(),
            PersonalityId::Analyst
        );
        assert_eq!(
            "Explorer".parse::<PersonalityId>().unwrap(),
            PersonalityId::Explorer
        );
    }

    #[test]
    fn parse_personality_id_returns_anyhow_result() {
        // anyhow 路径：成功
        let ok = parse_personality_id("skeptic").expect("valid id");
        assert_eq!(ok, PersonalityId::Skeptic);
        // anyhow 路径：失败带上下文
        let err = parse_personality_id("nope").unwrap_err();
        assert!(
            format!("{err:#}").contains("personality id"),
            "error should mention personality id: {err:#}"
        );
    }

    #[test]
    fn personality_id_display_matches_as_str() {
        for id in PersonalityId::ALL.iter() {
            assert_eq!(id.to_string(), id.as_str());
        }
    }

    // ---- 辅助结构体行为 ----

    #[test]
    fn cognitive_style_dominant_picks_highest_weight() {
        let cs = CognitiveStyle {
            analytical: 0.1,
            intuitive: 0.2,
            sequential: 0.9,
            holistic: 0.3,
        };
        assert_eq!(cs.dominant(), "sequential");
    }

    #[test]
    fn communication_style_lerp_at_zero_returns_other() {
        let a = CommunicationStyle {
            directness: 0.9,
            formality: 0.1,
            verbosity: 0.5,
            empathy: 0.2,
        };
        let b = CommunicationStyle {
            directness: 0.1,
            formality: 0.9,
            verbosity: 0.5,
            empathy: 0.8,
        };
        let lerped = a.lerp(&b, 0.0);
        assert!((lerped.directness - 0.1).abs() < 1e-9);
        assert!((lerped.formality - 0.9).abs() < 1e-9);
        assert!((lerped.empathy - 0.8).abs() < 1e-9);
    }

    #[test]
    fn decision_bias_lerp_at_one_returns_self() {
        let a = DecisionBias {
            risk_tolerance: 0.3,
            speed_vs_quality: 0.7,
            detail_orientation: 0.4,
        };
        let b = DecisionBias {
            risk_tolerance: 0.8,
            speed_vs_quality: 0.2,
            detail_orientation: 0.9,
        };
        let lerped = a.lerp(&b, 1.0);
        assert!((lerped.risk_tolerance - 0.3).abs() < 1e-9);
        assert!((lerped.speed_vs_quality - 0.7).abs() < 1e-9);
        assert!((lerped.detail_orientation - 0.4).abs() < 1e-9);
    }

    #[test]
    fn union_dedup_preserves_order_and_removes_duplicates() {
        let a = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        let b = vec!["y".to_string(), "w".to_string()];
        let out = union_dedup(&a, &b);
        assert_eq!(out, vec!["x", "y", "z", "w"]);
    }

    // ---- 场景覆盖矩阵 ----

    #[test]
    fn every_scenario_matches_at_least_one_personality() {
        let reg = PersonalityRegistry::new();
        for scenario in &["coding", "writing", "review", "research", "planning"] {
            assert!(
                !reg.best_for_scenario(scenario).is_empty(),
                "scenario {scenario} matched no personality"
            );
        }
    }
}
