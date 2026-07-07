//! T-E-A-14: Arena A/B 测试 — 模型对战 + ELO 评分 + SQLite 持久化。
//!
//! 参考 [`crate::llm::cost_tracker::CostTracker`] 的 `attach_store` 模式:
//! * 进程内 `Mutex<HashMap<String, f32>>`(model → elo);
//! * `Option<SqliteStore>` 持久化后端(`Some` 时 `create_match` / `update_elo`
//!   / `vote` 异步 spawn_blocking 写 SQLite);
//! * **MutexGuard 不跨 await** — `parking_lot::MutexGuard` 是 `!Send`,
//!   块作用域 drop 后再进入 spawn_blocking。
//!
//! ## ELO 公式
//!
//! * K = 32,初始 1200;
//! * expected_a = 1 / (1 + 10^((elo_b - elo_a) / 400));
//! * new_elo = old_elo + K * (score - expected),其中
//!   winner=a → (1, 0),winner=b → (0, 1),tie → (0.5, 0.5)。
//!
//! ## 模型调用与自动评分
//!
//! 当前为 stub:
//! * `call_model` 返回空字符串(TODO: 接入 LlmGateway::chat_with_provider);
//! * `auto_score` 返回 None(TODO: 接入 swarm::negotiator::score_response,
//!   当前该方法为私有)。

use std::collections::HashMap;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::memory::sqlite_store::SqliteStore;

/// ELO 初始分(与 migration 034 `model_elo_scores.elo DEFAULT 1200` 对齐)。
pub const ELO_INIT: f32 = 1200.0;
/// ELO K 因子(标准 32,与 Chess.com / LMSYS Chatbot Arena 一致)。
pub const ELO_K: f32 = 32.0;

/// 单场对战记录。对应 SQLite `arena_matches` 表(migration 034)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArenaMatch {
    pub id: String,
    pub prompt: String,
    pub model_a: String,
    pub model_b: String,
    pub response_a: Option<String>,
    pub response_b: Option<String>,
    /// "a" / "b" / "tie",`None` 表示未判定(等待人工投票或自动评分未跑)。
    pub winner: Option<String>,
    pub auto_score_a: Option<f32>,
    pub auto_score_b: Option<f32>,
    pub created_at: i64,
}

/// 进程内 Arena 排行榜。`Arc<ArenaLeaderboard>` 由 [`crate::AppState`]
/// 持有,Tauri 命令通过 `state.arena` 调用。
///
/// **并发模型**:
/// * `elo_scores: Mutex<HashMap<String, f32>>` 用 `parking_lot::Mutex`,
///   所有持锁操作在块作用域内完成,`MutexGuard` 不跨 await 点;
/// * `store: Option<SqliteStore>` 用 `Arc<Mutex<Connection>>`(clone 廉价),
///   SQLite 写入走 `spawn_blocking`(`MutexGuard` 跨 await 会编译失败,
///   参考 `cost_tracker::CostTracker::record_async` 的同模式)。
pub struct ArenaLeaderboard {
    elo_scores: Mutex<HashMap<String, f32>>,
    store: Option<SqliteStore>,
}

impl Default for ArenaLeaderboard {
    fn default() -> Self {
        Self {
            elo_scores: Mutex::new(HashMap::new()),
            store: None,
        }
    }
}

impl ArenaLeaderboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// builder 风格注入 SQLite 持久化后端。
    ///
    /// bootstrap 阶段构造:`ArenaLeaderboard::new().with_store(store.clone())`,
    /// 然后立即调用 `load_from_store()` 把 `model_elo_scores` 表回填内存。
    pub fn with_store(mut self, store: SqliteStore) -> Self {
        self.store = Some(store);
        self
    }

    /// 启动时从 SQLite `model_elo_scores` 表回填内存 `elo_scores`。
    /// `store: None` 时返回 Ok(0)。
    pub async fn load_from_store(&self) -> Result<()> {
        let store = match &self.store {
            Some(s) => s.clone(),
            None => return Ok(()),
        };
        // spawn_blocking:SQLite 同步 I/O,避免在 async 上下文阻塞。
        // parking_lot::MutexGuard 不能跨 await,所以全部读在块内完成。
        let rows: Vec<(String, f32)> =
            tokio::task::spawn_blocking(move || -> Result<Vec<(String, f32)>> {
                let conn = store.raw_connection();
                let g = conn.lock();
                let mut stmt = g.prepare("SELECT model, elo FROM model_elo_scores")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?)))?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row?);
                }
                Ok(out)
            })
            .await
            .context("spawn_blocking for arena load_from_store")??;

        let mut guard = self.elo_scores.lock();
        for (model, elo) in rows {
            guard.insert(model, elo);
        }
        Ok(())
    }

    /// 创建一场对战:调用两个模型生成响应,可选自动评分,
    /// 持久化 arena_matches 行,若有 winner 则更新 ELO。返回 match_id。
    ///
    /// **当前为 stub**:
    /// * `call_model` 返回空字符串(TODO: 接入 LlmGateway);
    /// * `auto_score` 返回 None(TODO: 接入 swarm::negotiator::score_response);
    /// * `winner` 因此为 None(等待人工投票)。
    pub async fn create_match(
        &self,
        prompt: String,
        model_a: String,
        model_b: String,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        // TODO(T-E-A-14): 接入 LlmGateway::chat_with_provider 并行调用 model_a/model_b。
        // 当前 stub:返回空字符串,允许测试 create_match_persists 不依赖 LLM。
        let response_a: Option<String> = Some(String::new());
        let response_b: Option<String> = Some(String::new());

        // TODO(T-E-A-14): 接入 swarm::negotiator::score_response 做自动评分。
        // 当前 stub:auto_score_a/b 为 None,winner 也为 None(由人工投票决定)。
        let score_a: Option<f32> = None;
        let score_b: Option<f32> = None;
        let winner: Option<String> = match (score_a, score_b) {
            (Some(a), Some(b)) => {
                if (a - b).abs() < 1e-6 {
                    Some("tie".to_string())
                } else if a > b {
                    Some("a".to_string())
                } else {
                    Some("b".to_string())
                }
            }
            _ => None,
        };

        // 持久化 arena_matches 行(spawn_blocking:parking_lot MutexGuard 不跨 await)。
        if let Some(store) = &self.store {
            let store = store.clone();
            let id_c = id.clone();
            let prompt_c = prompt.clone();
            let ma_c = model_a.clone();
            let mb_c = model_b.clone();
            let ra_c = response_a.clone();
            let rb_c = response_b.clone();
            let winner_c = winner.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = store.raw_connection();
                let g = conn.lock();
                if let Err(e) = g.execute(
                    "INSERT INTO arena_matches \
                     (id, prompt, model_a, model_b, response_a, response_b, winner, \
                      auto_score_a, auto_score_b, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id_c, prompt_c, ma_c, mb_c, ra_c, rb_c, winner_c, score_a, score_b, now,
                    ],
                ) {
                    warn!(
                        target: "nebula.arena",
                        error = %e,
                        "sqlite insert arena_matches failed; record stays in memory only"
                    );
                }
            })
            .await;
        }

        // 若自动评分得出了 winner,立即更新 ELO(人工投票路径见 vote)。
        if let Some(w) = winner.as_deref() {
            self.update_elo(&model_a, &model_b, w).await?;
        }

        Ok(id)
    }

    /// 更新两个模型的 ELO(K=32)。`winner` 取值 `"a"` / `"b"` / `"tie"`。
    ///
    /// 块作用域持锁计算新 ELO,drop guard 后 spawn_blocking 写 SQLite
    /// (parking_lot::MutexGuard 是 !Send,不能跨 await)。
    pub async fn update_elo(&self, model_a: &str, model_b: &str, winner: &str) -> Result<()> {
        // 块作用域:持锁计算 + 写内存,Guard 在块结束 drop。
        let (new_elo_a, new_elo_b) = {
            let mut scores = self.elo_scores.lock();
            let elo_a = *scores.get(model_a).unwrap_or(&ELO_INIT);
            let elo_b = *scores.get(model_b).unwrap_or(&ELO_INIT);
            let expected_a = 1.0 / (1.0 + 10.0_f32.powf((elo_b - elo_a) / 400.0));
            let expected_b = 1.0 - expected_a;
            let (sa, sb) = match winner {
                "a" => (1.0, 0.0),
                "b" => (0.0, 1.0),
                _ => (0.5, 0.5),
            };
            let new_a = elo_a + ELO_K * (sa - expected_a);
            let new_b = elo_b + ELO_K * (sb - expected_b);
            scores.insert(model_a.to_string(), new_a);
            scores.insert(model_b.to_string(), new_b);
            (new_a, new_b)
        };

        // 持久化 spawn_blocking:不在持锁状态下跨 await。
        if let Some(store) = &self.store {
            let store = store.clone();
            let ma = model_a.to_string();
            let mb = model_b.to_string();
            let now = chrono::Utc::now().timestamp_millis();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = store.raw_connection();
                let g = conn.lock();
                // ON CONFLICT(model) DO UPDATE:存在则更新 elo + matches_played+1,
                // 否则插入新行(model, elo, 1, now)。
                let res: std::result::Result<(), rusqlite::Error> = (|| {
                    g.execute(
                        "INSERT INTO model_elo_scores (model, elo, matches_played, updated_at) \
                         VALUES (?1, ?2, 1, ?3) \
                         ON CONFLICT(model) DO UPDATE SET \
                            elo = excluded.elo, \
                            matches_played = model_elo_scores.matches_played + 1, \
                            updated_at = excluded.updated_at",
                        params![ma, new_elo_a, now],
                    )?;
                    g.execute(
                        "INSERT INTO model_elo_scores (model, elo, matches_played, updated_at) \
                         VALUES (?1, ?2, 1, ?3) \
                         ON CONFLICT(model) DO UPDATE SET \
                            elo = excluded.elo, \
                            matches_played = model_elo_scores.matches_played + 1, \
                            updated_at = excluded.updated_at",
                        params![mb, new_elo_b, now],
                    )?;
                    Ok(())
                })();
                if let Err(e) = res {
                    warn!(
                        target: "nebula.arena",
                        error = %e,
                        "sqlite upsert model_elo_scores failed; elos stay in memory only"
                    );
                }
            })
            .await;
        }
        Ok(())
    }

    /// 人工投票覆盖 `arena_matches.winner` 并触发 ELO 更新。
    ///
    /// 简化实现:不撤销旧 winner 的 ELO 影响(spec R2 提到人工投票优先级
    /// 高于自动评分;此处假设 vote 仅在 winner=NULL 时调用)。
    pub async fn vote(&self, match_id: &str, winner: String) -> Result<()> {
        // 读回 model_a/model_b(若 store 存在),用于驱动 ELO 更新。
        let (model_a, model_b): (Option<String>, Option<String>) = if let Some(store) = &self.store
        {
            let store = store.clone();
            let mid = match_id.to_string();
            tokio::task::spawn_blocking(move || -> Result<(Option<String>, Option<String>)> {
                let conn = store.raw_connection();
                let g = conn.lock();
                let row: rusqlite::Result<(Option<String>, Option<String>)> = g.query_row(
                    "SELECT model_a, model_b FROM arena_matches WHERE id = ?1",
                    params![mid],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                        ))
                    },
                );
                match row {
                    Ok(v) => Ok(v),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
                    Err(e) => Err(anyhow::anyhow!(e)),
                }
            })
            .await
            .context("spawn_blocking for arena vote readback")?
            .unwrap_or((None, None))
        } else {
            (None, None)
        };

        // 持久化新 winner。
        if let Some(store) = &self.store {
            let store = store.clone();
            let mid = match_id.to_string();
            let w = winner.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = store.raw_connection();
                let g = conn.lock();
                if let Err(e) = g.execute(
                    "UPDATE arena_matches SET winner = ?1 WHERE id = ?2",
                    params![w, mid],
                ) {
                    warn!(
                        target: "nebula.arena",
                        error = %e,
                        "sqlite update arena_matches winner failed"
                    );
                }
            })
            .await;
        }

        // 更新 ELO(若读回了 model_a/model_b)。
        if let (Some(ma), Some(mb)) = (model_a, model_b) {
            self.update_elo(&ma, &mb, &winner).await?;
        }

        Ok(())
    }

    /// 返回按 ELO 降序的排行榜(`Vec<(model, elo)>`)。
    pub async fn leaderboard(&self) -> Vec<(String, f32)> {
        // 块作用域持锁,clone 出结果后 drop guard。
        let snapshot: Vec<(String, f32)> = {
            let guard = self.elo_scores.lock();
            guard.iter().map(|(m, e)| (m.clone(), *e)).collect()
        };
        let mut out = snapshot;
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }
}

// ---------------------------------------------------------------------------
// Tests — 7 个单测覆盖 ELO 计算 / 持久化 / 投票 / 排序 / 回填
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 辅助:构造一个临时 SqliteStore(migration 034 创建 arena_matches / model_elo_scores 表)。
    /// 返回 (SqliteStore, PathBuf)。文件在 std::env::temp_dir() 下,
    /// 用 PID + nanos 命名保证并发安全(参考 cost_tracker tests::make_temp_store)。
    fn make_temp_store() -> (SqliteStore, std::path::PathBuf) {
        let tmp = std::env::temp_dir().join(format!(
            "nebula_arena_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&tmp);
        let store = SqliteStore::open(&tmp).expect("open sqlite store for arena test");
        (store, tmp)
    }

    /// 辅助:查询 arena_matches 表行数。
    fn arena_matches_count(store: &SqliteStore) -> i64 {
        let conn = store.raw_connection();
        let g = conn.lock();
        g.query_row("SELECT COUNT(*) FROM arena_matches", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("count arena_matches")
    }

    /// 辅助:查询 model_elo_scores 表行数。
    fn elo_scores_count(store: &SqliteStore) -> i64 {
        let conn = store.raw_connection();
        let g = conn.lock();
        g.query_row("SELECT COUNT(*) FROM model_elo_scores", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("count model_elo_scores")
    }

    /// 辅助:查询某场对战的 winner。
    fn winner_of(store: &SqliteStore, match_id: &str) -> Option<String> {
        let conn = store.raw_connection();
        let g = conn.lock();
        g.query_row(
            "SELECT winner FROM arena_matches WHERE id = ?1",
            params![match_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
    }

    /// T1: ELO 公式 — expected_a = 1/(1+10^((elo_b-elo_a)/400))。
    #[test]
    fn test_elo_calculation() {
        // 双方同分 → expected_a = 0.5。
        let expected = 1.0 / (1.0 + 10.0_f32.powf((1200.0 - 1200.0) / 400.0));
        assert!(
            (expected - 0.5).abs() < 1e-6,
            "expected_a at equal elos should be 0.5"
        );

        // elo_a=1400, elo_b=1200 → expected_a ≈ 0.76(更高 elo 的一方应胜率更高)。
        let expected_a = 1.0 / (1.0 + 10.0_f32.powf((1200.0 - 1400.0) / 400.0));
        assert!(
            (expected_a - 0.76).abs() < 0.02,
            "expected_a(1400,1200) ≈ 0.76, got {}",
            expected_a
        );

        // 对称性:expected_a + expected_b = 1。
        let expected_b = 1.0 - expected_a;
        assert!((expected_a + expected_b - 1.0).abs() < 1e-6);

        // K=32 因子:winner=a,sa=1 → new_elo_a = 1400 + 32 * (1 - 0.76) ≈ 1407.68。
        let new_a = 1400.0 + ELO_K * (1.0 - expected_a);
        assert!(
            (new_a - 1407.68).abs() < 0.1,
            "new_elo_a after win should be ~1407.68, got {}",
            new_a
        );
    }

    /// T2: winner=a → elo_a 升,elo_b 降。
    #[tokio::test]
    async fn test_elo_update_winner_a() {
        let lb = Arc::new(ArenaLeaderboard::new());
        // 双方初始 1200(未注册时 fallback 到 ELO_INIT)。
        lb.update_elo("model_a", "model_b", "a").await.unwrap();
        let board = lb.leaderboard().await;
        let elo_a = board.iter().find(|(m, _)| m == "model_a").map(|(_, e)| *e);
        let elo_b = board.iter().find(|(m, _)| m == "model_b").map(|(_, e)| *e);
        let elo_a = elo_a.expect("model_a should be in leaderboard");
        let elo_b = elo_b.expect("model_b should be in leaderboard");
        // winner=a → expected_a=0.5, sa=1 → new_elo_a = 1200 + 32*0.5 = 1216。
        assert!(
            elo_a > ELO_INIT,
            "winner elo_a should increase above 1200, got {}",
            elo_a
        );
        assert!(
            (elo_a - 1216.0).abs() < 0.1,
            "expected new_elo_a=1216, got {}",
            elo_a
        );
        // loser=b → sb=0 → new_elo_b = 1200 - 16 = 1184。
        assert!(
            elo_b < ELO_INIT,
            "loser elo_b should decrease below 1200, got {}",
            elo_b
        );
        assert!(
            (elo_b - 1184.0).abs() < 0.1,
            "expected new_elo_b=1184, got {}",
            elo_b
        );
    }

    /// T3: winner=tie → 双方趋近中间(各 +/-0,因为 expected=0.5 score=0.5)。
    #[tokio::test]
    async fn test_elo_update_tie() {
        let lb = Arc::new(ArenaLeaderboard::new());
        // 双方初始 1200,expected_a=0.5,sa=0.5 → delta=0(平局对等分选手无变化)。
        lb.update_elo("model_x", "model_y", "tie").await.unwrap();
        let board = lb.leaderboard().await;
        let elo_x = board
            .iter()
            .find(|(m, _)| m == "model_x")
            .map(|(_, e)| *e)
            .expect("model_x should be in leaderboard");
        let elo_y = board
            .iter()
            .find(|(m, _)| m == "model_y")
            .map(|(_, e)| *e)
            .expect("model_y should be in leaderboard");
        // 等分选手平局:ELO 不变(=1200)。
        assert!(
            (elo_x - ELO_INIT).abs() < 1e-6,
            "equal-rating tie should leave elo unchanged, got {}",
            elo_x
        );
        assert!(
            (elo_y - ELO_INIT).abs() < 1e-6,
            "equal-rating tie should leave elo unchanged, got {}",
            elo_y
        );

        // 不等分选手平局:高分方略降、低分方略升(双方趋近中间)。
        let lb2 = Arc::new(ArenaLeaderboard::new());
        // 先让 model_high 赢 model_low 一次,拉开差距。
        lb2.update_elo("model_high", "model_low", "a")
            .await
            .unwrap();
        let board_mid = lb2.leaderboard().await;
        let elo_high_before = board_mid
            .iter()
            .find(|(m, _)| m == "model_high")
            .map(|(_, e)| *e)
            .expect("model_high");
        let elo_low_before = board_mid
            .iter()
            .find(|(m, _)| m == "model_low")
            .map(|(_, e)| *e)
            .expect("model_low");
        // 然后让两人平局:high 应降,low 应升(双方趋近中间)。
        lb2.update_elo("model_high", "model_low", "tie")
            .await
            .unwrap();
        let board_after = lb2.leaderboard().await;
        let elo_high_after = board_after
            .iter()
            .find(|(m, _)| m == "model_high")
            .map(|(_, e)| *e)
            .expect("model_high after");
        let elo_low_after = board_after
            .iter()
            .find(|(m, _)| m == "model_low")
            .map(|(_, e)| *e)
            .expect("model_low after");
        assert!(
            elo_high_after < elo_high_before,
            "high-rating tie should decrease elo ({} → {})",
            elo_high_before,
            elo_high_after
        );
        assert!(
            elo_low_after > elo_low_before,
            "low-rating tie should increase elo ({} → {})",
            elo_low_before,
            elo_low_after
        );
    }

    /// T4: create_match 持久化 arena_matches 行。
    #[tokio::test]
    async fn test_create_match_persists() {
        let (store, _tmp) = make_temp_store();
        let lb = Arc::new(ArenaLeaderboard::new().with_store(store.clone()));
        lb.load_from_store().await.unwrap();
        let before = arena_matches_count(&store);
        assert_eq!(before, 0, "fresh store should have 0 arena_matches");

        let match_id = lb
            .create_match(
                "hello".to_string(),
                "deepseek-chat".to_string(),
                "qwen2.5:7b".to_string(),
            )
            .await
            .expect("create_match");
        assert!(!match_id.is_empty(), "match_id should be non-empty");

        // spawn_blocking 完成后 SQLite 应有 1 行。
        let after = arena_matches_count(&store);
        assert_eq!(
            after, 1,
            "arena_matches table should have 1 row after create_match"
        );

        // 验证列内容(prompt / model_a / model_b)。
        let conn = store.raw_connection();
        let g = conn.lock();
        let row = g
            .query_row(
                "SELECT prompt, model_a, model_b FROM arena_matches WHERE id = ?1",
                params![match_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("query arena_matches row");
        assert_eq!(row.0, "hello");
        assert_eq!(row.1, "deepseek-chat");
        assert_eq!(row.2, "qwen2.5:7b");
    }

    /// T5: vote 覆盖 winner(从 NULL → "a")。
    #[tokio::test]
    async fn test_vote_overrides_winner() {
        let (store, _tmp) = make_temp_store();
        let lb = Arc::new(ArenaLeaderboard::new().with_store(store.clone()));
        lb.load_from_store().await.unwrap();

        // 创建对战(stub 路径 winner=None)。
        let match_id = lb
            .create_match(
                "vote-test".to_string(),
                "model_p".to_string(),
                "model_q".to_string(),
            )
            .await
            .unwrap();
        // 验证初始 winner 为 None(stub 路径未自动评分)。
        assert!(
            winner_of(&store, &match_id).is_none(),
            "stub create_match should leave winner NULL"
        );

        // 人工投票 winner=a。
        lb.vote(&match_id, "a".to_string()).await.unwrap();
        assert_eq!(
            winner_of(&store, &match_id).as_deref(),
            Some("a"),
            "vote should override winner to 'a'"
        );

        // ELO 也应更新:model_p 胜 → elo_p 升。
        let board = lb.leaderboard().await;
        let elo_p = board
            .iter()
            .find(|(m, _)| m == "model_p")
            .map(|(_, e)| *e)
            .expect("model_p should be in leaderboard after vote");
        assert!(
            elo_p > ELO_INIT,
            "winner model_p should have elo > 1200 after vote, got {}",
            elo_p
        );
    }

    /// T6: leaderboard 按 ELO 降序返回。
    #[tokio::test]
    async fn test_leaderboard_sorted() {
        let lb = Arc::new(ArenaLeaderboard::new());
        // 构造三条记录:ELO 各异。
        // 第一场:model_alpha 胜 model_beta(alpha↑ beta↓)。
        lb.update_elo("model_alpha", "model_beta", "a")
            .await
            .unwrap();
        // 第二场:再让 model_alpha 胜 model_gamma(alpha↑↑ gamma↓)。
        lb.update_elo("model_alpha", "model_gamma", "a")
            .await
            .unwrap();
        // 此时:model_alpha(2 胜) > model_beta(0 胜 1 负) > model_gamma(0 胜 1 负,首战对 model_alpha)
        // 实际数值:alpha 赢两次,model_beta 输一次,model_gamma 输一次但对手是已升级的 alpha。
        let board = lb.leaderboard().await;
        assert!(
            board.len() >= 3,
            "leaderboard should have at least 3 models, got {}",
            board.len()
        );
        // 按降序验证:第一个 elo >= 第二个 >= 第三个。
        for i in 1..board.len() {
            assert!(
                board[i - 1].1 >= board[i].1,
                "leaderboard not sorted desc: [{}] elo {} < [{}] elo {}",
                i - 1,
                board[i - 1].1,
                i,
                board[i].1
            );
        }
        // model_alpha 应排在第一位(两连胜)。
        assert_eq!(
            board[0].0, "model_alpha",
            "model_alpha (2 wins) should be #1, got {}",
            board[0].0
        );
    }

    /// T7: load_from_store 从 SQLite 回填 elo_scores。
    #[tokio::test]
    async fn test_load_from_store() {
        let (store, _tmp) = make_temp_store();
        // 第一阶段:直接 INSERT 两条 model_elo_scores 行。
        {
            let conn = store.raw_connection();
            let g = conn.lock();
            g.execute(
                "INSERT INTO model_elo_scores (model, elo, matches_played, updated_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params!["model_loaded_a", 1450.0_f32, 10_i64, 12345_i64],
            )
            .expect("insert model_loaded_a");
            g.execute(
                "INSERT INTO model_elo_scores (model, elo, matches_played, updated_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params!["model_loaded_b", 980.0_f32, 5_i64, 67890_i64],
            )
            .expect("insert model_loaded_b");
        }
        assert_eq!(elo_scores_count(&store), 2, "pre-seeded 2 elo rows");

        // 第二阶段:新构造 ArenaLeaderboard + with_store + load_from_store。
        let lb = Arc::new(ArenaLeaderboard::new().with_store(store.clone()));
        lb.load_from_store().await.expect("load_from_store");

        // 内存 elo_scores 应包含两条记录,数值匹配。
        let board = lb.leaderboard().await;
        let elo_a = board
            .iter()
            .find(|(m, _)| m == "model_loaded_a")
            .map(|(_, e)| *e)
            .expect("model_loaded_a should be loaded");
        let elo_b = board
            .iter()
            .find(|(m, _)| m == "model_loaded_b")
            .map(|(_, e)| *e)
            .expect("model_loaded_b should be loaded");
        assert!((elo_a - 1450.0).abs() < 1e-6, "elo_a mismatch: {}", elo_a);
        assert!((elo_b - 980.0).abs() < 1e-6, "elo_b mismatch: {}", elo_b);
    }
}
