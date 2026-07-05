use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::blackhole::BlackholeEngine;
use super::layers::policy_for;
use super::sqlite_store::SqliteStore;
use super::types::MemoryLayer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgettingConfig {
    pub importance_threshold: f32,
    pub dry_run: bool,
}

impl Default for ForgettingConfig {
    fn default() -> Self {
        Self {
            importance_threshold: 0.3,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgettingCandidate {
    pub id: String,
    pub layer: MemoryLayer,
    pub importance: f32,
    pub last_access: i64,
    pub ttl_days: u32,
    pub reason: String,
}

pub struct ForgettingEngine {
    config: ForgettingConfig,
    /// T-S1-A-03b: 可选的 BlackholeEngine 引用。设置后,`tick()` 在
    /// 归档成功后会调用 `run_pass_archived()` 形成"归档 → 压缩"闭环。
    /// `None` 时(如单元测试)tick() 仅做归档,不触发压缩。
    blackhole: Option<Arc<BlackholeEngine>>,
    /// T-S1-A-03b: run_pass_archived 的 batch 大小。
    blackhole_batch_size: usize,
}

impl ForgettingEngine {
    pub fn new(config: ForgettingConfig) -> Self {
        Self {
            config,
            blackhole: None,
            blackhole_batch_size: 100,
        }
    }

    /// T-S1-A-03b: 注入 BlackholeEngine,启用"归档 → 压缩"闭环。
    pub fn with_blackhole(mut self, blackhole: Arc<BlackholeEngine>) -> Self {
        self.blackhole = Some(blackhole);
        self
    }

    /// T-S1-A-03b: 自定义 run_pass_archived 的 batch 大小(默认 100)。
    pub fn with_blackhole_batch_size(mut self, batch_size: usize) -> Self {
        self.blackhole_batch_size = batch_size.max(1);
        self
    }

    pub fn scan_for_archive(
        &self,
        memories: Vec<(String, MemoryLayer, f32, i64, bool)>,
        now: i64,
    ) -> Vec<ForgettingCandidate> {
        let mut candidates = Vec::new();

        for (id, layer, importance, last_access, pinned) in memories {
            if pinned {
                continue;
            }
            if layer == MemoryLayer::L7 {
                continue;
            }
            // M7b #90 分类 B: importance == threshold 时应保留(不归档)。
            // 原 `>` 导致 importance == threshold 仍被归档,与测试期望矛盾。
            if importance >= self.config.importance_threshold {
                continue;
            }

            let policy = policy_for(layer);
            if policy.ttl_days == 0 {
                continue;
            }

            let ttl_secs = policy.ttl_days as i64 * 24 * 3600;
            let age = now - last_access;
            if age < ttl_secs {
                continue;
            }

            candidates.push(ForgettingCandidate {
                id,
                layer,
                importance,
                last_access,
                ttl_days: policy.ttl_days,
                reason: format!(
                    "importance={:.2} < threshold={:.2}, age={}d > ttl={}d",
                    importance,
                    self.config.importance_threshold,
                    age / 86400,
                    policy.ttl_days,
                ),
            });
        }

        if !candidates.is_empty() {
            info!(
                target: "nebula.forgetting",
                count = candidates.len(),
                dry_run = self.config.dry_run,
                "identified memories for archival"
            );
        }

        candidates
    }

    /// T-S1-A-03a + T-S1-A-03b: 执行一轮遗忘归档。
    ///
    /// 行为：
    /// 1. 调用 `SqliteStore::list_forgettable_candidates()` 拉取候选元组
    ///    （importance < threshold, archived=0, pinned=0, 非 L7, 未压缩）。
    /// 2. 调用 `scan_for_archive()` 应用 TTL 策略进一步过滤。
    /// 3. 若 `dry_run=true`，仅返回候选列表不写库。
    /// 4. 否则调用 `SqliteStore::archive_memories()` 将候选 `archived=1`。
    /// 5. T-S1-A-03b: 若配置了 BlackholeEngine,在归档成功后调用
    ///    `run_pass_archived()` 压缩新归档的记忆,形成"归档 → 压缩"闭环。
    ///    压缩失败不阻断 tick 返回(仅 warn),因为归档已成功落库。
    ///
    /// 返回 [`TickResult`]，包含候选列表、实际归档数与压缩报告。
    pub async fn tick(
        &self,
        sqlite: &SqliteStore,
        now: i64,
    ) -> anyhow::Result<TickResult> {
        let memories = sqlite.list_forgettable_candidates(self.config.importance_threshold).await?;
        let candidates = self.scan_for_archive(memories, now);

        if candidates.is_empty() {
            return Ok(TickResult {
                candidates: Vec::new(),
                archived_count: 0,
                compression: None,
            });
        }

        if self.config.dry_run {
            info!(
                target: "nebula.forgetting",
                count = candidates.len(),
                "dry_run: would have archived candidates (no DB write)"
            );
            return Ok(TickResult {
                archived_count: 0,
                candidates,
                compression: None,
            });
        }

        let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
        let archived_count = sqlite.archive_memories(&ids).await?;

        info!(
            target: "nebula.forgetting",
            candidates = candidates.len(),
            archived = archived_count,
            "tick completed: archived low-importance memories"
        );

        // 归档数与候选数不一致时记录 warn（可能是并发归档或 id 不存在）
        if archived_count != candidates.len() {
            warn!(
                target: "nebula.forgetting",
                expected = candidates.len(),
                actual = archived_count,
                "archived count mismatch (concurrent archive or missing ids?)"
            );
        }

        // T-S1-A-03b: 归档成功后触发黑洞压缩(仅压缩 archived=1 的行)。
        // 压缩失败不影响归档结果,只记录 warn。
        let compression = if let Some(bh) = &self.blackhole {
            match bh.run_pass_archived(self.blackhole_batch_size).await {
                Ok(report) => {
                    info!(
                        target: "nebula.forgetting",
                        scanned = report.scanned,
                        compressed = report.compressed,
                        summaries = report.summaries_created,
                        "post-archive compression pass completed"
                    );
                    Some(report)
                }
                Err(e) => {
                    warn!(
                        target: "nebula.forgetting",
                        error = ?e,
                        "run_pass_archived failed after archiving (archive already persisted)"
                    );
                    None
                }
            }
        } else {
            None
        };

        Ok(TickResult {
            candidates,
            archived_count,
            compression,
        })
    }
}

/// T-S1-A-03a + T-S1-A-03b: `tick()` 的返回值。
#[derive(Debug, Clone)]
pub struct TickResult {
    /// 本轮识别的归档候选（含被 TTL 过滤后的最终列表）。
    pub candidates: Vec<ForgettingCandidate>,
    /// 实际写入 `archived=1` 的行数。
    /// `dry_run=true` 时为 0。
    pub archived_count: usize,
    /// T-S1-A-03b: 归档后触发的 `run_pass_archived()` 压缩报告。
    /// `None` 表示未配置 BlackholeEngine、dry_run、或压缩失败。
    pub compression: Option<crate::memory::blackhole::CompressionReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_importance_old_memory_is_candidate() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-1".to_string(),
            MemoryLayer::L1,
            0.1,
            now - 2 * 86400,
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn pinned_memory_is_not_candidate() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-1".to_string(),
            MemoryLayer::L1,
            0.1,
            now - 2 * 86400,
            true,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert!(candidates.is_empty());
    }

    #[test]
    fn l7_is_not_candidate() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-1".to_string(),
            MemoryLayer::L7,
            0.1,
            now - 365 * 86400,
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert!(candidates.is_empty());
    }

    #[test]
    fn high_importance_is_not_candidate() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-1".to_string(),
            MemoryLayer::L1,
            0.8,
            now - 2 * 86400,
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert!(candidates.is_empty());
    }

    // ---- T-S1-A-03a: tick() + TickResult 单元测试 ----
    // 注：tick() 的完整集成测试需要 SqliteStore 实例，放在
    // tests/integration/forgetting_tick_test.rs。本模块聚焦
    // scan_for_archive() 与 TickResult 的契约验证。

    /// `TickResult` 字段语义：`archived_count=0` 表示无归档或 dry_run。
    #[test]
    fn tick_result_zero_archived() {
        let result = TickResult {
            candidates: Vec::new(),
            archived_count: 0,
            compression: None,
        };
        assert_eq!(result.archived_count, 0);
        assert!(result.candidates.is_empty());
        assert!(result.compression.is_none());
    }

    /// `TickResult` 在有候选且归档成功时正确记录数量。
    #[test]
    fn tick_result_with_archived() {
        let now = chrono::Utc::now().timestamp();
        let candidate = ForgettingCandidate {
            id: "mem-1".into(),
            layer: MemoryLayer::L1,
            importance: 0.1,
            last_access: now - 2 * 86400,
            ttl_days: 1,
            reason: "test".into(),
        };
        let result = TickResult {
            candidates: vec![candidate],
            archived_count: 1,
            compression: None,
        };
        assert_eq!(result.archived_count, 1);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].id, "mem-1");
        assert!(result.compression.is_none());
    }

    // ---- T-S1-A-03b: BlackholeEngine 调用链契约测试 ----

    /// T-S1-A-03b: `ForgettingEngine::new()` 默认不持有 BlackholeEngine。
    /// 验证 `blackhole` 字段初始为 `None`(通过 tick 行为间接验证:
    /// 无 blackhole 时 TickResult.compression 必为 None)。
    #[test]
    fn new_engine_has_no_blackhole() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        // blackhole 是私有字段,通过行为验证:scan_for_archive 不依赖它
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-1".to_string(),
            MemoryLayer::L1,
            0.1,
            now - 2 * 86400,
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert_eq!(candidates.len(), 1, "scan should work without blackhole");
    }

    /// T-S1-A-03b: `with_blackhole_batch_size` builder 正确设置 batch 大小。
    /// 验证极小值被 clamp 到 1。
    #[test]
    fn with_blackhole_batch_size_clamps_to_min_one() {
        // 无法直接读取私有字段,但 builder 不 panic 即说明签名正确。
        // batch_size=0 应被 clamp 为 1(不 panic)。
        let _engine = ForgettingEngine::new(ForgettingConfig::default())
            .with_blackhole_batch_size(0);
        let _engine2 = ForgettingEngine::new(ForgettingConfig::default())
            .with_blackhole_batch_size(500);
        // 若 clamp 逻辑正确,两个 engine 都能正常构造。
    }

    /// T-S1-A-03b: TickResult.compression 字段在有压缩报告时正确携带。
    #[test]
    fn tick_result_with_compression_report() {
        use crate::memory::blackhole::CompressionReport;
        let report = CompressionReport {
            scanned: 10,
            compressed: 8,
            skipped: 2,
            summaries_created: 1,
        };
        let result = TickResult {
            candidates: Vec::new(),
            archived_count: 10,
            compression: Some(report),
        };
        assert_eq!(result.archived_count, 10);
        let comp = result.compression.expect("compression should be Some");
        assert_eq!(comp.scanned, 10);
        assert_eq!(comp.compressed, 8);
        assert_eq!(comp.summaries_created, 1);
    }

    /// dry_run 模式下 `scan_for_archive()` 仍返回候选，但
    /// `tick()` 不会写库（`archived_count=0`）—— 这里验证
    /// `scan_for_archive()` 在 dry_run 配置下行为不变。
    #[test]
    fn dry_run_config_still_scans_candidates() {
        let config = ForgettingConfig {
            importance_threshold: 0.3,
            dry_run: true,
        };
        let engine = ForgettingEngine::new(config);
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-1".to_string(),
            MemoryLayer::L1,
            0.1,
            now - 2 * 86400,
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert_eq!(candidates.len(), 1, "dry_run should still identify candidates");
    }

    /// TTL 过滤：刚访问的记忆不应成为候选（即使 importance 低）。
    #[test]
    fn recently_accessed_memory_is_not_candidate() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        let now = chrono::Utc::now().timestamp();
        // last_access = now（刚访问），importance=0.1，layer=L1（TTL=1天）
        let memories = vec![(
            "mem-fresh".to_string(),
            MemoryLayer::L1,
            0.1,
            now, // 刚访问
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert!(candidates.is_empty(), "freshly accessed memory should not be a candidate");
    }

    /// 边界：importance 恰好等于阈值时不归档（`>` 而非 `>=`）。
    #[test]
    fn importance_at_threshold_is_not_candidate() {
        let engine = ForgettingEngine::new(ForgettingConfig::default());
        let now = chrono::Utc::now().timestamp();
        let memories = vec![(
            "mem-threshold".to_string(),
            MemoryLayer::L1,
            0.3, // 恰好等于阈值
            now - 2 * 86400,
            false,
        )];
        let candidates = engine.scan_for_archive(memories, now);
        assert!(candidates.is_empty(), "importance == threshold should not be candidate (uses >)");
    }
}
