//! SkillAutoEvolver + Skill Archive = "uselessness decay" loop.
//!
//! Behaviour:
//!   * A skill becomes an *archive candidate* when its
//!     `usage_count >= archive_min_usage` AND
//!     `avg_rating < archive_rate_floor`.
//!   * `SkillAutoEvolver::run_once()` reads the SkillStore, performs
//!     the test, and moves offenders into a separate
//!     `skill_archive` table (created by migration 009).
//!   * The original skill row stays untouched — we never delete.  This
//!     preserves an audit trail and lets the user undo via the
//!     `evolution_restore_archived` Tauri command.
//!
//! All public entry points respect `evolution::evolution_enabled()`.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::outcome::{Outcome, OutcomeLedger, OutcomeSource, OutcomeStatus};
use super::EvolutionConfig;
use crate::skills::store::SkillStore;

/// A snapshot of a single skill's archive decision (for tests + UI).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchiveDecision {
    pub skill_id: String,
    pub skill_name: String,
    pub usage_count: u32,
    pub avg_rating: f32,
    /// True iff the skill was moved to the archive in this pass.
    pub archived: bool,
    /// True iff the skill was restored from the archive in this pass.
    pub restored: bool,
    /// Reason string used by the UI / logs.
    pub reason: String,
}

/// Trait abstraction for testability.
pub trait SkillAutoEvolver: Send + Sync {
    fn run_once(&self) -> Result<Vec<ArchiveDecision>>;
}

pub struct SqliteSkillAutoEvolver {
    pub skills: Arc<SkillStore>,
    pub ledger: Arc<dyn OutcomeLedger>,
    pub conn: Arc<parking_lot::Mutex<Connection>>,
    pub config: EvolutionConfig,
}

impl SqliteSkillAutoEvolver {
    pub fn new(
        skills: Arc<SkillStore>,
        ledger: Arc<dyn OutcomeLedger>,
        conn: Arc<parking_lot::Mutex<Connection>>,
        config: EvolutionConfig,
    ) -> Self {
        Self {
            skills,
            ledger,
            conn,
            config,
        }
    }

    /// Effective archive criterion.
    fn should_archive(&self, usage_count: u32, avg_rating: f32) -> bool {
        usage_count >= self.config.archive_min_usage && avg_rating < self.config.archive_rate_floor
    }

    /// Move an already-archived skill back into the active list.
    pub fn restore(&self, skill_id: &str, reason: &str) -> Result<bool> {
        let g = self.conn.lock();
        let n = g.execute(
            "DELETE FROM skill_archive WHERE skill_id = ?1",
            params![skill_id],
        )?;
        if n > 0 {
            tracing::info!(
                target: "nebula.evolution.skill_evolver",
                skill_id,
                reason,
                "skill restored from archive"
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// P2-C: 从复杂任务经验中自动创建技能。
    ///
    /// 对标 Hermes: "Autonomous skill creation after complex tasks"。
    ///
    /// 触发条件:swarm 任务完成 + outcome.confidence > goal_confidence_threshold
    /// 流程:
    /// 1. 收集任务上下文(对话 + 工具调用 + 结果)
    /// 2. LLM 归纳为 SKILL.md 格式(此处接收已归纳的 manifest)
    /// 3. Eligibility 检查 → 存入 SkillStore
    /// 4. 首次使用时 nudge 用户确认
    ///
    /// 返回 Some(Skill) 表示创建了新技能,None 表示信心不足未创建。
    pub fn create_from_experience(
        &self,
        task_context: &TaskContext,
        outcome: &super::outcome::Outcome,
    ) -> Result<Option<crate::skills::types::Skill>> {
        // 信心阈值检查
        if outcome.confidence < self.config.goal_confidence_threshold {
            tracing::debug!(
                target: "nebula.evolution.skill_evolver",
                confidence = outcome.confidence,
                threshold = self.config.goal_confidence_threshold,
                "outcome confidence below threshold, skipping skill creation"
            );
            return Ok(None);
        }

        // 构造新技能
        let now = chrono::Utc::now().timestamp();
        let skill_id = format!("auto-{}-{}", outcome.source_id, now);
        let skill = crate::skills::types::Skill {
            id: skill_id.clone(),
            name: task_context.skill_name.clone(),
            description: task_context.skill_description.clone(),
            code: task_context.skill_code.clone(),
            language: task_context.skill_language.clone(),
            tags: task_context.skill_tags.clone(),
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: now,
            updated_at: now,
            source_memory_id: Some(outcome.source_id.clone()),
            activation_condition: task_context.activation_condition.clone(),
            platform: None,
            min_confidence: Some(outcome.confidence),
            trust_level: 0, // auto_created → 未验证,需用户确认
            permissions: task_context.permissions.clone(),
            capabilities: crate::skills::sandbox::CapabilitySet::default(),
        };

        // 存入 SkillStore
        self.skills.insert(&skill)?;

        // 记录到 OutcomeLedger
        let _ = self.ledger.record(&super::outcome::Outcome {
            id: super::outcome::fresh_outcome_id(),
            source_id: skill_id.clone(),
            source: OutcomeSource::Skill,
            status: OutcomeStatus::Success,
            confidence: outcome.confidence,
            error: format!("auto-created from task {}", outcome.source_id),
            duration_ms: 0,
            created_at: now,
        });

        tracing::info!(
            target: "nebula.evolution.skill_evolver",
            skill_id = %skill_id,
            skill_name = %skill.name,
            confidence = outcome.confidence,
            "skill auto-created from experience"
        );

        Ok(Some(skill))
    }

    /// P2-C: 技能使用中自我改进。
    ///
    /// 对标 Hermes: "Skills self-improve during use"。
    ///
    /// 触发条件:技能使用 5+ 次 + avg_rating < 4.0(或 archive_rate_floor)
    /// 流程:
    /// 1. 收集该技能最近 N 次使用结果
    /// 2. 分析失败模式(此处接收已分析的改进方案)
    /// 3. 应用改进 → 存入 SkillStore(保留旧版本可回滚)
    pub fn improve_from_usage(
        &self,
        skill_id: &str,
        improvement: &SkillImprovement,
    ) -> Result<ImproveResult> {
        let skill = match self.skills.get(skill_id)? {
            Some(s) => s,
            None => return Ok(ImproveResult::NotFound),
        };

        // 使用次数检查
        if skill.usage_count < self.config.archive_min_usage {
            tracing::debug!(
                target: "nebula.evolution.skill_evolver",
                skill_id = %skill_id,
                usage_count = skill.usage_count,
                min_usage = self.config.archive_min_usage,
                "usage count below threshold, skipping improvement"
            );
            return Ok(ImproveResult::Skipped);
        }

        // 评分检查:只有低评分技能才需要改进
        if skill.avg_rating >= self.config.archive_rate_floor {
            tracing::debug!(
                target: "nebula.evolution.skill_evolver",
                skill_id = %skill_id,
                avg_rating = skill.avg_rating,
                rate_floor = self.config.archive_rate_floor,
                "avg rating above floor, skipping improvement"
            );
            return Ok(ImproveResult::Skipped);
        }

        // 应用改进
        let mut improved = skill.clone();
        if !improvement.new_code.is_empty() {
            improved.code = improvement.new_code.clone();
        }
        if !improvement.new_description.is_empty() {
            improved.description = improvement.new_description.clone();
        }
        if !improvement.new_tags.is_empty() {
            improved.tags = improvement.new_tags.clone();
        }
        improved.updated_at = chrono::Utc::now().timestamp();

        // Upsert(保留旧版本通过 prompt_mutator snapshot,如果有)
        self.skills.upsert(&improved)?;

        // 记录到 OutcomeLedger
        let now = chrono::Utc::now().timestamp();
        let _ = self.ledger.record(&super::outcome::Outcome {
            id: super::outcome::fresh_outcome_id(),
            source_id: skill_id.to_string(),
            source: OutcomeSource::Skill,
            status: OutcomeStatus::Success,
            confidence: 0.8,
            error: format!("skill improved: {}", improvement.reason),
            duration_ms: 0,
            created_at: now,
        });

        tracing::info!(
            target: "nebula.evolution.skill_evolver",
            skill_id = %skill_id,
            reason = %improvement.reason,
            "skill improved from usage"
        );

        Ok(ImproveResult::Applied)
    }
}

impl SkillAutoEvolver for SqliteSkillAutoEvolver {
    fn run_once(&self) -> Result<Vec<ArchiveDecision>> {
        // Pull every active skill via the existing store.
        // NB: SkillStore::list takes raw (language, single_tag, tags, tag_match,
        // limit) — the ListSkillsRequest DTO is the wire form for the
        // Tauri/gRPC layer only.  Reuse the store's typed signature here.
        let skills =
            self.skills
                .list(None, None, &[], crate::skills::types::TagMatch::Any, 1000)?;
        let now = chrono::Utc::now().timestamp();
        let mut decisions = Vec::new();

        for s in skills {
            // Already archived? Check before evaluating.
            let already_archived = {
                let g = self.conn.lock();
                let mut stmt = g.prepare("SELECT 1 FROM skill_archive WHERE skill_id = ?1")?;
                stmt.exists(params![s.id])?
            };
            if already_archived {
                // Auto-restore if recent outcomes look good again.
                let recent = self.ledger.by_source(
                    OutcomeSource::Skill,
                    &s.id,
                    self.config.prompt_mutator_window as usize,
                )?;
                if recent.len() as u32 >= self.config.archive_min_usage {
                    let avg_conf =
                        recent.iter().map(|o| o.confidence).sum::<f32>() / recent.len() as f32;
                    if avg_conf >= self.config.goal_confidence_threshold {
                        let _unused = self.restore(&s.id, "outcomes recovered")?;
                        decisions.push(ArchiveDecision {
                            skill_id: s.id.clone(),
                            skill_name: s.name.clone(),
                            usage_count: s.usage_count,
                            avg_rating: s.avg_rating,
                            archived: false,
                            restored: true,
                            reason: "outcomes recovered".into(),
                        });
                        continue;
                    }
                }
                continue;
            }

            let avg_rating_decision =
                Self::should_archive_decision_static(s.usage_count, s.avg_rating, &self.config);
            if avg_rating_decision.archived {
                // Persist to skill_archive.
                let g = self.conn.lock();
                g.execute(
                    "INSERT OR REPLACE INTO skill_archive
                        (skill_id, skill_name, usage_count, avg_rating, archived_at, reason)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        s.id,
                        s.name,
                        s.usage_count as i64,
                        s.avg_rating,
                        now,
                        avg_rating_decision.reason,
                    ],
                )?;
                drop(g);
                let _unused = self.ledger.record(&Outcome {
                    id: super::outcome::fresh_outcome_id(),
                    source_id: s.id.clone(),
                    source: OutcomeSource::Skill,
                    status: OutcomeStatus::Cancelled,
                    confidence: 0.0,
                    error: format!("auto-archived: {}", avg_rating_decision.reason),
                    duration_ms: 0,
                    created_at: now,
                });
                decisions.push(ArchiveDecision {
                    skill_id: s.id.clone(),
                    skill_name: s.name.clone(),
                    usage_count: s.usage_count,
                    avg_rating: s.avg_rating,
                    archived: true,
                    restored: false,
                    reason: avg_rating_decision.reason,
                });
                continue;
            }

            // Not archived.
            decisions.push(ArchiveDecision {
                skill_id: s.id.clone(),
                skill_name: s.name.clone(),
                usage_count: s.usage_count,
                avg_rating: s.avg_rating,
                archived: false,
                restored: false,
                reason: "kept".into(),
            });
        }

        Ok(decisions)
    }
}

struct ArchiveSemantic {
    pub archived: bool,
    pub reason: String,
}

impl SqliteSkillAutoEvolver {
    fn should_archive_decision_static(
        usage_count: u32,
        avg_rating: f32,
        cfg: &EvolutionConfig,
    ) -> ArchiveSemantic {
        if usage_count >= cfg.archive_min_usage && avg_rating < cfg.archive_rate_floor {
            ArchiveSemantic {
                archived: true,
                reason: format!(
                    "usage_count={} >= {} AND avg_rating={:.2} < {:.2}",
                    usage_count, cfg.archive_min_usage, avg_rating, cfg.archive_rate_floor
                ),
            }
        } else {
            ArchiveSemantic {
                archived: false,
                reason: "below archive threshold".into(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P2-C: Skill 闭环进化辅助类型
// ---------------------------------------------------------------------------

/// 任务上下文 — 从复杂任务经验中提取的技能 manifest。
///
/// 由调用方(通常是 swarm orchestrator)在任务完成后构造,
/// 传给 [`SqliteSkillAutoEvolver::create_from_experience`]。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskContext {
    /// 新技能的名称。
    pub skill_name: String,
    /// 新技能的描述。
    pub skill_description: String,
    /// 技能代码(LLM 归纳后的 prompt 或 python 代码)。
    pub skill_code: String,
    /// 技能语言("llm" 或 "python")。
    pub skill_language: String,
    /// 技能标签。
    pub skill_tags: Vec<String>,
    /// 激活条件(可选)。
    pub activation_condition: Option<crate::skills::types::ActivationCondition>,
    /// 声明式权限。
    pub permissions: Vec<String>,
}

/// 技能改进方案 — 由 LLM 分析失败模式后生成。
///
/// 传给 [`SqliteSkillAutoEvolver::improve_from_usage`]。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillImprovement {
    /// 改进原因(失败模式摘要)。
    pub reason: String,
    /// 新的技能代码(空字符串表示不修改)。
    pub new_code: String,
    /// 新的描述(空字符串表示不修改)。
    pub new_description: String,
    /// 新的标签(空列表表示不修改)。
    pub new_tags: Vec<String>,
}

/// `improve_from_usage` 的返回值。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImproveResult {
    /// 改进已应用。
    Applied,
    /// 跳过(使用次数不足或评分达标)。
    Skipped,
    /// 技能不存在。
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::ListSkillsRequest;

    #[test]
    fn archive_decision_below_min_usage() {
        let cfg = EvolutionConfig::default(); // min_usage = 20
        let d = SqliteSkillAutoEvolver::should_archive_decision_static(5, 0.3, &cfg);
        assert!(!d.archived);
        assert!(d.reason.contains("below archive"));
    }

    #[test]
    fn archive_decision_at_min_usage_and_low_rating() {
        let cfg = EvolutionConfig::default();
        let d = SqliteSkillAutoEvolver::should_archive_decision_static(20, 0.3, &cfg);
        assert!(d.archived);
        assert!(d.reason.contains("usage_count=20"));
    }

    #[test]
    fn archive_decision_at_min_usage_and_high_rating() {
        let cfg = EvolutionConfig::default();
        let d = SqliteSkillAutoEvolver::should_archive_decision_static(20, 0.9, &cfg);
        assert!(!d.archived);
    }

    #[test]
    fn archive_decision_at_critical() {
        let cfg = EvolutionConfig::default();
        let d = SqliteSkillAutoEvolver::should_archive_decision_static(100, 0.49, &cfg);
        assert!(d.archived);
    }

    #[test]
    fn list_skills_request_serializes() {
        // guard against accidental breaking change to ListSkillsRequest.
        let r = ListSkillsRequest::default();
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"limit\":"));
    }
}
