//! T-E-A-14: Arena A/B 测试 Tauri 命令。
//!
//! 3 个命令,均通过 `state.arena`(`Arc<ArenaLeaderboard>`)调用:
//! * `arena_create_match(prompt, model_a, model_b) -> String` — 创建对战,返回 match_id;
//! * `arena_vote(match_id, winner) -> ()` — 人工投票覆盖 winner;
//! * `arena_leaderboard() -> Vec<(String, f32)>` — 按 ELO 降序返回排行榜。
//!
//! 错误处理与 `commands::cost` 一致:用 `CommandError` 包装 anyhow::Error。

use tauri::State;
use tracing::instrument;

use crate::AppState;
use crate::commands::error::CommandError;

/// T-E-A-14: 创建一场 A/B 对战。
///
/// 后端并行调用 model_a / model_b 生成响应,可选自动评分,
/// 持久化到 `arena_matches` 表。返回 match_id(UUID v4)。
///
/// **当前为 stub**:模型调用与自动评分返回空/None(待接入 LlmGateway
/// 与 swarm::negotiator::score_response),仅持久化对战元数据。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "arena_create_match"))]
pub async fn arena_create_match(
    state: State<'_, AppState>,
    prompt: String,
    model_a: String,
    model_b: String,
) -> Result<String, CommandError> {
    state
        .arena
        .create_match(prompt, model_a, model_b)
        .await
        .map_err(|e| CommandError::internal("arena_create_match", &e))
}

/// T-E-A-14: 人工投票覆盖 winner。
///
/// `winner` 取值 `"a"` / `"b"` / `"tie"`。后端 UPDATE arena_matches.winner
/// 并触发 ELO 更新(不撤销旧 winner 的 ELO 影响,假设 vote 仅在 winner=NULL 时调用)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "arena_vote"))]
pub async fn arena_vote(
    state: State<'_, AppState>,
    match_id: String,
    winner: String,
) -> Result<(), CommandError> {
    state
        .arena
        .vote(&match_id, winner)
        .await
        .map_err(|e| CommandError::internal("arena_vote", &e))
}

/// T-E-A-14: 返回按 ELO 降序的排行榜。
///
/// 返回 `Vec<(model, elo)>`,前端 LeaderboardTable 组件渲染。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "arena_leaderboard"))]
pub async fn arena_leaderboard(
    state: State<'_, AppState>,
) -> Result<Vec<(String, f32)>, CommandError> {
    Ok(state.arena.leaderboard().await)
}
