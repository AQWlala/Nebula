//! 进化回滚（M4 任务 #62）— `/evolve rollback N` 命令实现。
//!
//! 从 `evolution_log.md` 查找最近的 N 条 Phase 4 (Soul) 条目，
//! 从 SOUL.md 的 evolution-append Section 删除对应行。
//!
//! ## 回滚流程
//!
//! 1. 读取 `evolution_log.md`，过滤 `phase = soul` 的条目，取最近 N 条
//! 2. 对每条 entry：
//!    a. 在 SOUL.md 中查找对应的 `## [<entry_id>]` 段落
//!    b. 删除该段落（含段落标记）
//! 3. 原子写回 SOUL.md
//! 4. 从 `evolution_log.md` 中删除对应条目
//!
//! ## 设计要点
//!
//! - **仅回滚 SOUL.md 写入**：不回滚 L2/L3/L5 记忆（这些是历史事实，删除反而破坏审计链）
//! - **原子写入**：经 `soul::atomic_write` 保证原子性（P1-14）
//! - **回滚日志清理**：成功回滚后从 `evolution_log.md` 删除对应条目
//! - **失败不破坏前序状态**：单条回滚失败仅记 warning，继续下一条

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use thiserror::Error;
use tracing::info;

use super::log::{EvolutionLog, EvolutionLogEntry, EvolutionLogError};
use super::pipeline::EvolutionPhase;

/// 回滚错误类型。
#[derive(Debug, Error)]
pub enum RollbackError {
    #[error("evolution log error: {0}")]
    Log(#[from] EvolutionLogError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("soul write error: {0}")]
    SoulWrite(String),

    #[error("evolution log entry not found: {0}")]
    EntryNotFound(String),

    #[error("evolution disabled at runtime")]
    Disabled,
}

/// 回滚结果。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RollbackResult {
    /// 请求回滚的条数 N。
    pub requested_count: usize,
    /// 实际回滚的条数。
    pub rolled_back: usize,
    /// 失败的条数（仅记 warning，不阻断）。
    pub failed: usize,
    /// 每条回滚的 entry_id。
    pub entry_ids: Vec<String>,
    /// 总体 warnings。
    pub warnings: Vec<String>,
}

/// 进化回滚器。
pub struct Roller {
    log: Arc<EvolutionLog>,
    soul_md_path: PathBuf,
}

impl std::fmt::Debug for Roller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Roller")
            .field("soul_md_path", &self.soul_md_path)
            .finish()
    }
}

impl Roller {
    pub fn new(log: Arc<EvolutionLog>, soul_md_path: PathBuf) -> Self {
        Self { log, soul_md_path }
    }

    /// 回滚最近 N 条 Phase 4 (Soul) 进化写入。
    ///
    /// 流程：
    /// 1. 读取 `evolution_log.md`，过滤 `phase = soul` 条目，取最近 N 条
    /// 2. 对每条：从 SOUL.md 删除对应 `## [<entry_id>]` 段落
    /// 3. 原子写回 SOUL.md
    /// 4. 从日志删除对应条目
    ///
    /// `n = 0` 时无操作返回空结果。
    /// `n` 超过实际条目数时按实际数量回滚，不报错。
    pub async fn rollback(&self, n: usize) -> Result<RollbackResult, RollbackError> {
        if !super::super::evolution_enabled() {
            return Err(RollbackError::Disabled);
        }

        if n == 0 {
            return Ok(RollbackResult {
                requested_count: 0,
                rolled_back: 0,
                failed: 0,
                entry_ids: Vec::new(),
                warnings: Vec::new(),
            });
        }

        // 1. 读取所有日志条目，过滤 Soul 阶段，取最近 N 条
        let all_entries = self.log.list_all()?;
        let soul_entries: Vec<EvolutionLogEntry> = all_entries
            .into_iter()
            .filter(|e| e.phase == EvolutionPhase::Soul)
            .collect();

        if soul_entries.is_empty() {
            return Ok(RollbackResult {
                requested_count: n,
                rolled_back: 0,
                failed: 0,
                entry_ids: Vec::new(),
                warnings: vec!["no Soul phase entries to rollback".to_string()],
            });
        }

        // 取最近 N 条（倒序后取前 N，再反转为原顺序）
        let to_rollback: Vec<EvolutionLogEntry> = soul_entries
            .into_iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let mut rolled_back = 0;
        let mut failed = 0;
        let mut entry_ids = Vec::new();
        let mut warnings = Vec::new();

        // 2. 对每条 entry：从 SOUL.md 删除对应段落
        // 先一次性读取 SOUL.md，多次删除后一次性写回
        let mut soul_content = match std::fs::read_to_string(&self.soul_md_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(RollbackError::SoulWrite(format!(
                    "SOUL.md not found at {}",
                    self.soul_md_path.display()
                )));
            }
            Err(e) => return Err(RollbackError::Io(e)),
        };

        for entry in &to_rollback {
            match self.remove_entry_from_soul_md(&mut soul_content, &entry.entry_id) {
                Ok(true) => {
                    rolled_back += 1;
                    entry_ids.push(entry.entry_id.clone());
                }
                Ok(false) => {
                    failed += 1;
                    warnings.push(format!(
                        "entry {} not found in SOUL.md (already rolled back?)",
                        entry.entry_id
                    ));
                }
                Err(e) => {
                    failed += 1;
                    warnings.push(format!("rollback entry {} failed: {e}", entry.entry_id));
                }
            }
        }

        // 3. 原子写回 SOUL.md（仅在内容有变更时）
        if rolled_back > 0 {
            #[cfg(feature = "soul-system")]
            {
                crate::soul::atomic_write::atomic_write(&self.soul_md_path, &soul_content)
                    .map_err(|e| RollbackError::SoulWrite(format!("atomic_write: {e}")))?;
            }
            // soul-system 未启用时回退到普通写入
            #[cfg(not(feature = "soul-system"))]
            {
                std::fs::write(&self.soul_md_path, &soul_content)
                    .map_err(|e| RollbackError::SoulWrite(format!("fs::write: {e}")))?;
            }
        }

        // 4. 从日志删除对应条目
        for entry_id in &entry_ids {
            match self.log.remove_entry(entry_id).await {
                Ok(true) => {}
                Ok(false) => {
                    warnings.push(format!("log entry {} already removed", entry_id));
                }
                Err(e) => {
                    warnings.push(format!("remove log entry {} failed: {e}", entry_id));
                }
            }
        }

        info!(target: "nebula.evolution.rollback",
            requested = n,
            rolled_back,
            failed,
            "rollback complete");

        Ok(RollbackResult {
            requested_count: n,
            rolled_back,
            failed,
            entry_ids,
            warnings,
        })
    }

    /// 从 SOUL.md 内容中删除指定 entry_id 对应的段落。
    ///
    /// 段落定位规则：查找 `## [<entry_id>] Evolution` 标记，
    /// 删除到下一个 `## [` 标记或 `<!-- END SECTION: evolution-append -->` 之前。
    ///
    /// 返回 `Ok(true)` 表示删除成功，`Ok(false)` 表示未找到对应段落。
    pub(crate) fn remove_entry_from_soul_md(
        &self,
        soul_md: &mut String,
        entry_id: &str,
    ) -> Result<bool, RollbackError> {
        let marker = format!("## [{entry_id}]");
        let start = match soul_md.find(&marker) {
            Some(i) => i,
            None => return Ok(false),
        };

        // 段落结束：下一个 `## [` 或 END SECTION 标记
        let after_start = start + marker.len();
        let end = soul_md[after_start..]
            .find("\n## [")
            .map(|i| after_start + i + 1)
            .or_else(|| {
                soul_md[after_start..]
                    .find("<!-- END SECTION: evolution-append -->")
                    .map(|i| after_start + i)
            })
            .unwrap_or(soul_md.len());

        soul_md.replace_range(start..end, "");
        Ok(true)
    }
}
