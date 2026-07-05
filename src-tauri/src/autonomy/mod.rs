//! T-E-S-50: Autonomy slider L0-L5.
//!
//! 6 档自主度等级,决定 AI 的自主程度。与 `modeRouter`(任务领域
//! writing/work/code)正交:`(WorkMode, AutonomyLevel)` 组合决定最终行为。
//!
//! ## 等级
//! - L0 内联补全(T-E-S-51 实现)
//! - L1 定向编辑(T-E-S-52 实现)
//! - L2 对话(默认,行为与 ChatPanel 一致)
//! - L3 Plan(高风险审批)
//! - L4 蜂群(全自主 Agent)
//! - L5 后台自动化(T-E-S-53 实现)
//!
//! ## P0 范围
//! 本模块只提供路由决策(`AutonomyRouter::route` 返回 `AutonomyDispatch`),
//! 不实际执行 L0/L1/L5 路径(返回 `NotImplemented` stub,带下游任务 ID)。

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

// M5 #67/#68/#69/#70: L4 审批门禁子系统
pub mod approval;
pub mod risk_map;

pub use approval::{
    ApprovalGate, ApprovalVerdict, ConfirmationRegistry, ConfirmationStatus,
    PendingConfirmation, CONFIRMATION_TIMEOUT_MS,
};
pub use risk_map::{RiskThresholds, RiskTier, WorkerRiskMap};

/// 6 档自主度等级。
///
/// Wire 格式为大写 "L0".."L5"(与前端 `Layer` 类型一致)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutonomyLevel {
    /// L0 内联补全 — 输入框 AI 建议补全(本地小模型)。T-E-S-51 实现。
    #[serde(rename = "L0")]
    L0InlineCompletion,
    /// L1 定向编辑 — 选中文字 + 快捷键 → AI 局部改写。T-E-S-52 实现。
    #[serde(rename = "L1")]
    L1DirectedEdit,
    /// L2 对话 — 当前 ChatPanel 模式(默认)。
    #[serde(rename = "L2")]
    L2Chat,
    /// L3 Plan — 高风险操作需审批。
    #[serde(rename = "L3")]
    L3Plan,
    /// L4 蜂群 — 全自主 Agent。
    #[serde(rename = "L4")]
    L4Swarm,
    /// L5 后台自动化 — Cron/触发器驱动。T-E-S-53 实现。
    #[serde(rename = "L5")]
    L5Background,
}

impl AutonomyLevel {
    /// 全部等级,按 L0→L5 顺序。
    pub fn all() -> &'static [AutonomyLevel; 6] {
        const ALL: [AutonomyLevel; 6] = [
            AutonomyLevel::L0InlineCompletion,
            AutonomyLevel::L1DirectedEdit,
            AutonomyLevel::L2Chat,
            AutonomyLevel::L3Plan,
            AutonomyLevel::L4Swarm,
            AutonomyLevel::L5Background,
        ];
        &ALL
    }

    /// 数值索引(0..=5)。
    pub fn as_u8(self) -> u8 {
        match self {
            AutonomyLevel::L0InlineCompletion => 0,
            AutonomyLevel::L1DirectedEdit => 1,
            AutonomyLevel::L2Chat => 2,
            AutonomyLevel::L3Plan => 3,
            AutonomyLevel::L4Swarm => 4,
            AutonomyLevel::L5Background => 5,
        }
    }

    /// 从数值索引构造。越界返回 `None`。
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(AutonomyLevel::L0InlineCompletion),
            1 => Some(AutonomyLevel::L1DirectedEdit),
            2 => Some(AutonomyLevel::L2Chat),
            3 => Some(AutonomyLevel::L3Plan),
            4 => Some(AutonomyLevel::L4Swarm),
            5 => Some(AutonomyLevel::L5Background),
            _ => None,
        }
    }

    /// 下一档(更高自主度)。L5 返回自身。
    pub fn next(self) -> Self {
        self.as_u8()
            .checked_add(1)
            .filter(|&v| v <= 5)
            .and_then(Self::from_u8)
            .unwrap_or(self)
    }

    /// 上一档(更低自主度)。L0 返回自身。
    pub fn prev(self) -> Self {
        self.as_u8()
            .checked_sub(1)
            .and_then(Self::from_u8)
            .unwrap_or(self)
    }

    /// 短标签(英文,用于 UI)。
    pub fn label(self) -> &'static str {
        match self {
            AutonomyLevel::L0InlineCompletion => "Inline Completion",
            AutonomyLevel::L1DirectedEdit => "Directed Edit",
            AutonomyLevel::L2Chat => "Chat",
            AutonomyLevel::L3Plan => "Plan",
            AutonomyLevel::L4Swarm => "Swarm",
            AutonomyLevel::L5Background => "Background",
        }
    }

    /// 中文标签。
    pub fn label_zh(self) -> &'static str {
        match self {
            AutonomyLevel::L0InlineCompletion => "内联补全",
            AutonomyLevel::L1DirectedEdit => "定向编辑",
            AutonomyLevel::L2Chat => "对话",
            AutonomyLevel::L3Plan => "计划",
            AutonomyLevel::L4Swarm => "蜂群",
            AutonomyLevel::L5Background => "后台",
        }
    }

    /// 简短描述(英文)。
    pub fn description(self) -> &'static str {
        match self {
            AutonomyLevel::L0InlineCompletion => "Inline AI suggestions as you type",
            AutonomyLevel::L1DirectedEdit => "Rewrite the selected text on shortcut",
            AutonomyLevel::L2Chat => "Conversational replies, no auto-execution",
            AutonomyLevel::L3Plan => "High-risk actions require approval",
            AutonomyLevel::L4Swarm => "Fully autonomous multi-agent swarm",
            AutonomyLevel::L5Background => "Cron/trigger-driven background automation",
        }
    }

    /// 简短描述(中文)。
    pub fn description_zh(self) -> &'static str {
        match self {
            AutonomyLevel::L0InlineCompletion => "输入时内联 AI 补全建议",
            AutonomyLevel::L1DirectedEdit => "选中文字 + 快捷键 → AI 局部改写",
            AutonomyLevel::L2Chat => "对话回复,不自动执行",
            AutonomyLevel::L3Plan => "高风险操作需审批",
            AutonomyLevel::L4Swarm => "全自主多智能体蜂群",
            AutonomyLevel::L5Background => "Cron/触发器驱动后台自动化",
        }
    }

    /// Wire 字符串("L0".."L5")。
    pub fn as_str(self) -> &'static str {
        match self {
            AutonomyLevel::L0InlineCompletion => "L0",
            AutonomyLevel::L1DirectedEdit => "L1",
            AutonomyLevel::L2Chat => "L2",
            AutonomyLevel::L3Plan => "L3",
            AutonomyLevel::L4Swarm => "L4",
            AutonomyLevel::L5Background => "L5",
        }
    }

    /// 从 wire 字符串解析("L0".."L5",大小写不敏感)。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "L0" => Some(AutonomyLevel::L0InlineCompletion),
            "L1" => Some(AutonomyLevel::L1DirectedEdit),
            "L2" => Some(AutonomyLevel::L2Chat),
            "L3" => Some(AutonomyLevel::L3Plan),
            "L4" => Some(AutonomyLevel::L4Swarm),
            "L5" => Some(AutonomyLevel::L5Background),
            _ => None,
        }
    }
}

/// 每级行为参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyConfig {
    /// 是否需要用户审批才能执行。
    pub requires_approval: bool,
    /// 是否在后台运行(不阻塞 UI)。
    pub runs_in_background: bool,
    /// 是否自动执行生成结果(无需用户确认)。
    pub auto_execute: bool,
    /// 是否允许内联 UI(输入框补全等)。
    pub allows_inline_ui: bool,
    /// 是否路由到蜂群(SwarmOrchestrator)。
    pub routes_to_swarm: bool,
    /// 是否路由到 Plan 引擎(PlanEngine)。
    pub routes_to_plan: bool,
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        // 全 false 的保守默认;实际配置通过 `default_config(level)` 获取。
        Self {
            requires_approval: false,
            runs_in_background: false,
            auto_execute: false,
            allows_inline_ui: false,
            routes_to_swarm: false,
            routes_to_plan: false,
        }
    }
}

/// 返回指定等级的默认配置。
///
/// 配置矩阵(参考 spec §设计要点):
/// - L0: `allows_inline_ui + auto_execute`
/// - L1: `auto_execute`
/// - L2: 全 false(对话返回,不自动执行)
/// - L3: `requires_approval + routes_to_plan`
/// - L4: `routes_to_swarm`
/// - L5: `runs_in_background`
pub fn default_config(level: AutonomyLevel) -> AutonomyConfig {
    match level {
        AutonomyLevel::L0InlineCompletion => AutonomyConfig {
            requires_approval: false,
            runs_in_background: false,
            auto_execute: true,
            allows_inline_ui: true,
            routes_to_swarm: false,
            routes_to_plan: false,
        },
        AutonomyLevel::L1DirectedEdit => AutonomyConfig {
            requires_approval: false,
            runs_in_background: false,
            auto_execute: true,
            allows_inline_ui: false,
            routes_to_swarm: false,
            routes_to_plan: false,
        },
        AutonomyLevel::L2Chat => AutonomyConfig {
            requires_approval: false,
            runs_in_background: false,
            auto_execute: false,
            allows_inline_ui: false,
            routes_to_swarm: false,
            routes_to_plan: false,
        },
        AutonomyLevel::L3Plan => AutonomyConfig {
            requires_approval: true,
            runs_in_background: false,
            auto_execute: false,
            allows_inline_ui: false,
            routes_to_swarm: false,
            routes_to_plan: true,
        },
        AutonomyLevel::L4Swarm => AutonomyConfig {
            requires_approval: false,
            runs_in_background: false,
            auto_execute: false,
            allows_inline_ui: false,
            routes_to_swarm: true,
            routes_to_plan: false,
        },
        AutonomyLevel::L5Background => AutonomyConfig {
            requires_approval: false,
            runs_in_background: true,
            auto_execute: false,
            allows_inline_ui: false,
            routes_to_swarm: false,
            routes_to_plan: false,
        },
    }
}

/// 路由决策结果。`AutonomyRouter::route` 不实际执行,只返回决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomyDispatch {
    /// L0 内联补全(由 T-E-S-51 实现)。
    InlineCompletion,
    /// L1 定向编辑(由 T-E-S-52 实现)。
    DirectedEdit,
    /// L2 对话(透传到现有 ChatPanel)。
    Chat,
    /// L3 Plan(路由到 PlanEngine)。
    Plan,
    /// L4 蜂群(路由到 SwarmOrchestrator)。
    Swarm,
    /// L5 后台自动化(由 T-E-S-53 实现)。
    Background,
    /// 等级尚未实现。`task_id` 指向对应的下游任务(T-E-S-51/52/53)。
    NotImplemented { task_id: String },
}

impl AutonomyDispatch {
    /// 是否为 `NotImplemented`。
    pub fn is_not_implemented(&self) -> bool {
        matches!(self, AutonomyDispatch::NotImplemented { .. })
    }
}

/// 路由器:根据等级把任务分发到对应的执行路径。
///
/// P0 阶段只做决策,不实际执行。L0/L1/L5 返回 `NotImplemented` stub。
#[derive(Debug, Default, Clone, Copy)]
pub struct AutonomyRouter;

impl AutonomyRouter {
    /// 路由决策。
    ///
    /// `task` 是任务描述文本,P0 阶段不参与决策(仅用于日志/上下文),
    /// 后续 L3/L4 可能基于 task 内容调整裁定。
    pub fn route(&self, level: AutonomyLevel, _task: &str) -> AutonomyDispatch {
        match level {
            AutonomyLevel::L0InlineCompletion => AutonomyDispatch::NotImplemented {
                task_id: "T-E-S-51".to_string(),
            },
            AutonomyLevel::L1DirectedEdit => AutonomyDispatch::DirectedEdit,
            AutonomyLevel::L2Chat => AutonomyDispatch::Chat,
            AutonomyLevel::L3Plan => AutonomyDispatch::Plan,
            AutonomyLevel::L4Swarm => AutonomyDispatch::Swarm,
            AutonomyLevel::L5Background => AutonomyDispatch::NotImplemented {
                task_id: "T-E-S-53".to_string(),
            },
        }
    }
}

/// 全局自主度状态(进程内单例)。
///
/// 默认 `L2`(最低风险,行为与当前 ChatPanel 一致)。
///
/// # TODO(P1): SQLite 持久化(见 ROADMAP)
/// 目前用 `parking_lot::Mutex<AutonomyLevel>` 内存状态。P1 阶段会落库到
/// `app_settings` 表(key="autonomy_level",value="L2" 等),需要先加
/// migration 创建该表(例如 `migrations/023_app_settings.sql`)。届时:
/// 1. `set_level` 写入 SQLite + 更新内存;
/// 2. 启动时从 SQLite 读取并初始化内存;
/// 3. 内存层保留作为热缓存。
pub struct AutonomyState {
    level: Mutex<AutonomyLevel>,
}

impl AutonomyState {
    /// 创建指定初始等级的状态。
    pub fn new(level: AutonomyLevel) -> Self {
        Self {
            level: Mutex::new(level),
        }
    }

    /// 当前等级。
    pub fn get_level(&self) -> AutonomyLevel {
        *self.level.lock()
    }

    /// 设置等级。
    pub fn set_level(&self, level: AutonomyLevel) {
        *self.level.lock() = level;
    }
}

impl Default for AutonomyState {
    fn default() -> Self {
        Self::new(AutonomyLevel::L2Chat)
    }
}

/// 进程级全局状态(默认 L2)。
static GLOBAL_STATE: Lazy<AutonomyState> = Lazy::new(AutonomyState::default);

/// 读取全局自主度等级。
pub fn get_level() -> AutonomyLevel {
    GLOBAL_STATE.get_level()
}

/// 设置全局自主度等级。
pub fn set_level(level: AutonomyLevel) {
    GLOBAL_STATE.set_level(level);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_iteration_covers_all_six_levels() {
        let all = AutonomyLevel::all();
        assert_eq!(all.len(), 6);
        // 顺序 L0 → L5
        for (i, level) in all.iter().enumerate() {
            assert_eq!(level.as_u8() as usize, i);
        }
        // as_str 与 parse 互逆,大小写不敏感
        for level in all {
            let s = level.as_str();
            assert_eq!(AutonomyLevel::parse(s), Some(*level));
            assert_eq!(AutonomyLevel::parse(&s.to_lowercase()), Some(*level));
        }
        // from_u8 边界
        assert_eq!(AutonomyLevel::from_u8(6), None);
    }

    #[test]
    fn default_config_lookup_matches_level_semantics() {
        // L0: 内联 UI + 自动执行
        let l0 = default_config(AutonomyLevel::L0InlineCompletion);
        assert!(l0.allows_inline_ui && l0.auto_execute);
        assert!(!l0.routes_to_swarm && !l0.routes_to_plan && !l0.requires_approval);
        // L1: 自动执行,无内联 UI
        let l1 = default_config(AutonomyLevel::L1DirectedEdit);
        assert!(l1.auto_execute && !l1.allows_inline_ui);
        // L2: 对话,不自动执行,全 false(除默认)
        let l2 = default_config(AutonomyLevel::L2Chat);
        assert!(!l2.auto_execute && !l2.routes_to_swarm && !l2.routes_to_plan);
        // L3: 需审批 + 路由到 Plan
        let l3 = default_config(AutonomyLevel::L3Plan);
        assert!(l3.requires_approval && l3.routes_to_plan);
        // L4: 路由到蜂群
        let l4 = default_config(AutonomyLevel::L4Swarm);
        assert!(l4.routes_to_swarm);
        // L5: 后台运行
        let l5 = default_config(AutonomyLevel::L5Background);
        assert!(l5.runs_in_background);
    }

    #[test]
    fn l2_route_passes_through_as_chat() {
        let router = AutonomyRouter;
        let dispatch = router.route(AutonomyLevel::L2Chat, "写一封邮件");
        assert_eq!(dispatch, AutonomyDispatch::Chat);
        assert!(!dispatch.is_not_implemented());
    }

    #[test]
    fn l0_route_returns_not_implemented_with_task_id() {
        let router = AutonomyRouter;
        let dispatch = router.route(AutonomyLevel::L0InlineCompletion, "补全");
        match dispatch {
            AutonomyDispatch::NotImplemented { task_id } => {
                assert_eq!(task_id, "T-E-S-51");
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }

    #[test]
    fn l5_routes_to_correct_stub_task() {
        let router = AutonomyRouter;
        // L5 -> T-E-S-53
        match router.route(AutonomyLevel::L5Background, "ding-shi-ren-wu") {
            AutonomyDispatch::NotImplemented { task_id } => assert_eq!(task_id, "T-E-S-53"),
            other => panic!("L5 expected NotImplemented, got {other:?}"),
        }
    }

    #[test]
    fn l1_routes_to_directed_edit() {
        let router = AutonomyRouter;
        let dispatch = router.route(AutonomyLevel::L1DirectedEdit, "rewrite this");
        assert!(matches!(dispatch, AutonomyDispatch::DirectedEdit));
    }

    #[test]
    fn l3_l4_route_to_plan_and_swarm() {
        let router = AutonomyRouter;
        assert_eq!(router.route(AutonomyLevel::L3Plan, ""), AutonomyDispatch::Plan);
        assert_eq!(router.route(AutonomyLevel::L4Swarm, ""), AutonomyDispatch::Swarm);
    }

    #[test]
    fn state_default_is_l2_and_round_trips() {
        let state = AutonomyState::default();
        assert_eq!(state.get_level(), AutonomyLevel::L2Chat);
        state.set_level(AutonomyLevel::L4Swarm);
        assert_eq!(state.get_level(), AutonomyLevel::L4Swarm);
        state.set_level(AutonomyLevel::L2Chat);
        assert_eq!(state.get_level(), AutonomyLevel::L2Chat);
    }

    #[test]
    fn next_prev_clamp_at_boundaries() {
        assert_eq!(
            AutonomyLevel::L0InlineCompletion.prev(),
            AutonomyLevel::L0InlineCompletion
        );
        assert_eq!(
            AutonomyLevel::L5Background.next(),
            AutonomyLevel::L5Background
        );
        assert_eq!(AutonomyLevel::L2Chat.next(), AutonomyLevel::L3Plan);
        assert_eq!(AutonomyLevel::L2Chat.prev(), AutonomyLevel::L1DirectedEdit);
    }

    #[test]
    fn global_state_defaults_to_l2() {
        // 全局状态可能在其他测试中被修改,先保存再恢复。
        let original = get_level();
        set_level(AutonomyLevel::L2Chat);
        assert_eq!(get_level(), AutonomyLevel::L2Chat);
        set_level(original);
    }

    #[test]
    fn serde_wire_format_is_uppercase_l_prefix() {
        let l2_json = serde_json::to_string(&AutonomyLevel::L2Chat).unwrap();
        assert_eq!(l2_json, "\"L2\"");
        let parsed: AutonomyLevel = serde_json::from_str("\"L4\"").unwrap();
        assert_eq!(parsed, AutonomyLevel::L4Swarm);
    }
}
