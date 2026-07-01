//! v2.0 真正的 Self-Reflection — L5 元认知层升级。
//!
//! 设计文档 v7.0 §2.1 L5 Metacognitive Layer：
//! v0 假意识 → 真正 Self-Reflection。
//!
//! ## 三种反思模式
//!
//! 1. **价值对齐反思** (`value_alignment`) — 基于 L4 价值层评估最近行为
//! 2. **任务结局反思** (`outcome_review`) — 基于成功/失败模式总结经验
//! 3. **自我改进反思** (`self_improvement`) — 生成具体改进建议
//!
//! ## 与 v0 的区别
//!
//! v0 只是对记忆做摘要总结；v2.0 的反思是**主动的、批判性的、行动导向的**：
//! - 不只是"发生了什么"，而是"这件事对不对"
//! - 不只是"我学到了什么"，而是"我应该怎么改进"
//! - 接入 L4 价值层做价值判断
//! - 接入 outcome 数据做闭环验证

use std::sync::Arc;

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::memory::values::{ActionKind, ValuesLayer, Verdict};
use crate::memory::SqliteStore;

use super::types::Memory;
use super::ReflectConfig;

/// 自我反思的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionKind {
    /// 价值对齐反思：最近的行为是否符合价值观？
    ValueAlignment,
    /// 任务结局反思：从成功/失败中学习什么？
    OutcomeReview,
    /// 自我改进反思：我应该如何改进？
    SelfImprovement,
}

impl ReflectionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReflectionKind::ValueAlignment => "value_alignment",
            ReflectionKind::OutcomeReview => "outcome_review",
            ReflectionKind::SelfImprovement => "self_improvement",
        }
    }

    pub fn all() -> [ReflectionKind; 3] {
        [
            ReflectionKind::ValueAlignment,
            ReflectionKind::OutcomeReview,
            ReflectionKind::SelfImprovement,
        ]
    }
}

/// 单条反思结论。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfReflection {
    /// 反思类型。
    pub kind: ReflectionKind,
    /// 反思标题（一句话总结）。
    pub title: String,
    /// 详细内容。
    pub content: String,
    /// 关键洞见 / 教训。
    pub insights: Vec<String>,
    /// 具体行动建议。
    pub action_items: Vec<String>,
    /// 反思置信度 (0-1)。
    pub confidence: f32,
    /// 严重程度 / 重要性 (0-1)。
    pub severity: f32,
    /// 相关记忆 ID。
    pub related_memory_ids: Vec<String>,
}

/// v2.0 自我反思引擎 — L5 真正的元认知。
pub struct SelfReflectionEngine {
    sqlite: Arc<SqliteStore>,
    values: ValuesLayer,
    config: ReflectConfig,
}

impl SelfReflectionEngine {
    pub fn new(sqlite: Arc<SqliteStore>, values: ValuesLayer, config: ReflectConfig) -> Self {
        Self {
            sqlite,
            values,
            config,
        }
    }

    /// 执行一次完整的自我反思（所有三种类型）。
    pub async fn reflect_all(&self) -> Result<Vec<SelfReflection>> {
        let mut results = Vec::new();

        let recent = self.recent_memories(20).await?;
        if recent.is_empty() {
            debug!(target: "nine_snake.self_reflect", "no recent memories for reflection");
            return Ok(results);
        }

        // 1. 价值对齐反思
        match self.reflect_value_alignment(&recent) {
            Ok(r) => results.push(r),
            Err(e) => {
                debug!(target: "nine_snake.self_reflect", error = %e,
                    "value alignment reflection failed");
            }
        }

        // 2. 任务结局反思（基于 outcome 数据）
        match self.reflect_outcomes() {
            Ok(Some(r)) => results.push(r),
            Ok(None) => {
                debug!(target: "nine_snake.self_reflect", "no outcomes for review");
            }
            Err(e) => {
                debug!(target: "nine_snake.self_reflect", error = %e,
                    "outcome reflection failed");
            }
        }

        // 3. 自我改进反思
        match self.reflect_self_improvement(&results) {
            Ok(r) => results.push(r),
            Err(e) => {
                debug!(target: "nine_snake.self_reflect", error = %e,
                    "self-improvement reflection failed");
            }
        }

        info!(target: "nine_snake.self_reflect", count = results.len(),
            "self-reflection pass complete");

        Ok(results)
    }

    /// 价值对齐反思：检查最近的记忆是否与 L4 价值观一致。
    pub fn reflect_value_alignment(
        &self,
        recent: &[Memory],
    ) -> Result<SelfReflection> {
        let mut violations = Vec::new();
        let mut warnings = Vec::new();
        let mut ok_count = 0;

        for mem in recent.iter().take(10) {
            let verdict = self.values.evaluate(&mem.content, ActionKind::Generic);
            match verdict {
                Verdict::Allow => ok_count += 1,
                Verdict::Confirm { prompt } => {
                    warnings.push(format!("[{}] {}", mem.id, prompt));
                }
                Verdict::Plan { prompt } => {
                    warnings.push(format!("[{}] 需要方案: {}", mem.id, prompt));
                }
                Verdict::Deny { reason } => {
                    violations.push(format!("[{}] {}", mem.id, reason));
                }
            }
        }

        let total = recent.len().min(10);
        let alignment_score = if total == 0 {
            1.0
        } else {
            ok_count as f32 / total as f32
        };

        let severity = if !violations.is_empty() {
            0.8
        } else if !warnings.is_empty() {
            0.5
        } else {
            0.2
        };

        let mut insights = Vec::new();
        if violations.is_empty() && warnings.is_empty() {
            insights.push("近期行为与价值观保持良好一致".to_string());
            insights.push(format!("价值对齐度: {:.0}%", alignment_score * 100.0));
        } else {
            insights.push(format!(
                "发现 {} 项潜在违规，{} 项需关注",
                violations.len(),
                warnings.len()
            ));
            insights.push(format!("价值对齐度: {:.0}%", alignment_score * 100.0));
        }

        let mut action_items = Vec::new();
        if !violations.is_empty() {
            action_items.push("立即审查违规记忆内容".to_string());
            action_items.push("考虑是否需要清理相关记忆".to_string());
        }
        if !warnings.is_empty() {
            action_items.push("对高风险操作增加前置确认".to_string());
        }
        action_items.push("持续监控价值对齐情况".to_string());

        let title = if violations.is_empty() && warnings.is_empty() {
            "价值对齐状态良好".to_string()
        } else if !violations.is_empty() {
            format!("检测到 {} 项价值违规", violations.len())
        } else {
            format!("{} 项操作需关注风险", warnings.len())
        };

        let mut content = String::new();
        content.push_str(&format!("## 价值对齐报告\n\n"));
        content.push_str(&format!("- 审查记忆数: {}\n", total));
        content.push_str(&format!("- 对齐率: {:.0}%\n", alignment_score * 100.0));
        content.push_str(&format!("- 违规数: {}\n", violations.len()));
        content.push_str(&format!("- 风险提示数: {}\n\n", warnings.len()));

        if !violations.is_empty() {
            content.push_str("### 违规项\n\n");
            for v in &violations {
                content.push_str(&format!("- {}\n", v));
            }
            content.push('\n');
        }

        if !warnings.is_empty() {
            content.push_str("### 风险提示\n\n");
            for w in &warnings {
                content.push_str(&format!("- {}\n", w));
            }
            content.push('\n');
        }

        Ok(SelfReflection {
            kind: ReflectionKind::ValueAlignment,
            title,
            content,
            insights,
            action_items,
            confidence: 0.85,
            severity,
            related_memory_ids: recent.iter().map(|m| m.id.clone()).collect(),
        })
    }

    /// 任务结局反思：从 outcome 数据中学习。
    pub fn reflect_outcomes(&self) -> Result<Option<SelfReflection>> {
        // 读取最近的 outcome 记录
        let outcomes = self.load_recent_outcomes(30)?;
        if outcomes.is_empty() {
            return Ok(None);
        }

        let success_count = outcomes
            .iter()
            .filter(|o| o.status == "success")
            .count();
        let fail_count = outcomes
            .iter()
            .filter(|o| o.status == "fail")
            .count();
        let total = outcomes.len();
        let success_rate = if total == 0 {
            0.0
        } else {
            success_count as f32 / total as f32
        };

        let mut insights = Vec::new();
        insights.push(format!("近期任务成功率: {:.0}% ({}/{})",
            success_rate * 100.0, success_count, total));

        if success_rate >= 0.8 {
            insights.push("任务执行表现优秀，保持当前策略".to_string());
        } else if success_rate >= 0.5 {
            insights.push("成功率中等，有提升空间".to_string());
        } else {
            insights.push("成功率偏低，需要重点改进".to_string());
        }

        let mut action_items = Vec::new();
        if fail_count > 0 {
            action_items.push("分析失败任务的根本原因".to_string());
            action_items.push("总结失败模式并制定规避策略".to_string());
        }
        if success_count > 0 {
            action_items.push("提炼成功经验并标准化".to_string());
        }
        action_items.push("建立任务复盘机制".to_string());

        let severity = if success_rate < 0.3 {
            0.9
        } else if success_rate < 0.6 {
            0.6
        } else {
            0.3
        };

        let title = format!("任务结局复盘: {:.0}% 成功率", success_rate * 100.0);

        let mut content = String::new();
        content.push_str("## 任务结局复盘\n\n");
        content.push_str(&format!("- 总任务数: {}\n", total));
        content.push_str(&format!("- 成功: {}\n", success_count));
        content.push_str(&format!("- 失败: {}\n", fail_count));
        content.push_str(&format!("- 成功率: {:.1}%\n\n", success_rate * 100.0));

        content.push_str("### 关键洞见\n\n");
        for insight in &insights {
            content.push_str(&format!("- {}\n", insight));
        }

        Ok(Some(SelfReflection {
            kind: ReflectionKind::OutcomeReview,
            title,
            content,
            insights,
            action_items,
            confidence: 0.75,
            severity,
            related_memory_ids: Vec::new(),
        }))
    }

    /// 自我改进反思：基于前两种反思生成改进计划。
    pub fn reflect_self_improvement(
        &self,
        prior: &[SelfReflection],
    ) -> Result<SelfReflection> {
        let high_severity = prior
            .iter()
            .filter(|r| r.severity >= 0.7)
            .count();
        let medium_severity = prior
            .iter()
            .filter(|r| r.severity >= 0.4 && r.severity < 0.7)
            .count();

        let all_actions: Vec<String> = prior
            .iter()
            .flat_map(|r| r.action_items.clone())
            .collect();

        let mut insights = Vec::new();
        insights.push(format!(
            "发现 {} 项高优先级改进点，{} 项中优先级",
            high_severity, medium_severity
        ));
        insights.push(format!("共 {} 条具体行动建议", all_actions.len()));

        let mut action_items = all_actions;
        action_items.truncate(5); // 最多 5 条优先行动
        action_items.push("建立每周自我复盘习惯".to_string());
        action_items.push("跟踪改进措施的执行效果".to_string());

        let severity = if high_severity > 0 {
            0.8
        } else if medium_severity > 0 {
            0.5
        } else {
            0.2
        };

        let title = if high_severity > 0 {
            "需要立即改进".to_string()
        } else if medium_severity > 0 {
            "有改进空间".to_string()
        } else {
            "持续保持".to_string()
        };

        let mut content = String::new();
        content.push_str("## 自我改进计划\n\n");
        content.push_str(&format!("### 优先级评估\n\n"));
        content.push_str(&format!("- 高优先级: {} 项\n", high_severity));
        content.push_str(&format!("- 中优先级: {} 项\n\n", medium_severity));

        content.push_str("### 优先行动\n\n");
        for (i, action) in action_items.iter().enumerate() {
            content.push_str(&format!("{}. {}\n", i + 1, action));
        }

        Ok(SelfReflection {
            kind: ReflectionKind::SelfImprovement,
            title,
            content,
            insights,
            action_items,
            confidence: 0.7,
            severity,
            related_memory_ids: Vec::new(),
        })
    }

    // ------------------------------------------------------------------
    // 内部方法
    // ------------------------------------------------------------------

    async fn recent_memories(&self, limit: usize) -> Result<Vec<Memory>> {
        let sqlite = self.sqlite.clone();
        let join = tokio::task::spawn_blocking(move || -> Result<Vec<Memory>> {
            let conn = sqlite.raw_connection();
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id, memory_type, layer, content, summary_50, summary_150, summary_500, summary_2000,
                        importance, access_count, last_access, created_at, source, metadata,
                        compressed_from, compression_gen, pinned
                 FROM memories
                 WHERE compressed_from IS NULL
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], super::reflect::row_to_memory_full)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        });
        let res = join
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking error: {}", e))??;
        Ok(res)
    }

    fn load_recent_outcomes(&self, limit: usize) -> Result<Vec<OutcomeRecord>> {
        let conn = self.sqlite.raw_connection();
        let conn = conn.lock();

        // 检查表是否存在（self-evolution feature 未启用时可能没有）
        let table_exists: bool = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='task_outcomes'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !table_exists {
            return Ok(Vec::new());
        }

        let mut stmt = conn.prepare(
            "SELECT id, source, status, duration_ms, created_at, metadata_json
             FROM task_outcomes
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;

        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(OutcomeRecord {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    status: row.get(2)?,
                    duration_ms: row.get(3)?,
                    created_at: row.get(4)?,
                    metadata_json: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }
}

#[derive(Debug, Clone)]
struct OutcomeRecord {
    #[allow(dead_code)]
    id: String,
    source: String,
    status: String,
    #[allow(dead_code)]
    duration_ms: Option<i64>,
    #[allow(dead_code)]
    created_at: i64,
    #[allow(dead_code)]
    metadata_json: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflection_kind_as_str() {
        assert_eq!(ReflectionKind::ValueAlignment.as_str(), "value_alignment");
        assert_eq!(ReflectionKind::OutcomeReview.as_str(), "outcome_review");
        assert_eq!(ReflectionKind::SelfImprovement.as_str(), "self_improvement");
    }

    #[test]
    fn reflection_kind_all_has_three() {
        assert_eq!(ReflectionKind::all().len(), 3);
    }

    #[test]
    fn self_reflection_has_correct_layer_and_type() {
        // 验证反思类型的元数据正确
        let r = SelfReflection {
            kind: ReflectionKind::ValueAlignment,
            title: "test".into(),
            content: "test".into(),
            insights: vec![],
            action_items: vec![],
            confidence: 0.5,
            severity: 0.5,
            related_memory_ids: vec![],
        };
        assert_eq!(r.kind.as_str(), "value_alignment");
        assert!(r.confidence >= 0.0 && r.confidence <= 1.0);
        assert!(r.severity >= 0.0 && r.severity <= 1.0);
    }
}
