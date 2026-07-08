//! T-E-S-54 / T-E-S-55: 事件触发器 Tauri 命令 — 6 个命令与 `TriggerEngine` 配套:
//!
//! - `trigger_create(config)` — 创建触发器(持久化 + 启动 worker)
//! - `trigger_list()`        — 列出所有触发器
//! - `trigger_delete(id)`    — 删除触发器(停止 worker + 持久化删除)
//! - `trigger_enable(id, enabled)` — 启用/禁用触发器
//! - `trigger_fire_log(id)`  — 查询触发日志(最近 100 条)
//! - `watch_test(source)`    — T-E-S-55: 手动测试 WatchSource(单次轮询,不持久化)
//!
//! 设计要点:
//! * 命令是 `TriggerEngine` 的薄包装,所有逻辑由 engine 完成。
//! * DTO 直接复用 `TriggerConfig`(已实现 Serialize/Deserialize)。
//! * `trigger_create` 时若 `id` 为空则自动生成 UUIDv4。
//! * 阻塞 SQLite I/O 由 `TriggerStore` 内部 `parking_lot::Mutex` 保护,
//!   命令层无需额外 `spawn_blocking`(store 方法本身同步且短促)。

use tauri::State;
use tracing::instrument;

use crate::commands::error::CommandError;
use crate::triggers::watch::{
    apply_selector, find_upcoming_events, hash_text, parse_ics, SystemProbe, WatchSource,
    WebFetcher,
};
use crate::triggers::{FireLogRow, TriggerConfig};
use crate::AppState;

/// 创建触发器。
///
/// `config.id` 为空时自动生成 UUIDv4。返回最终持久化的 trigger id。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "trigger_create"))]
pub async fn trigger_create(
    state: State<'_, AppState>,
    mut config: TriggerConfig,
) -> Result<String, CommandError> {
    if config.id.is_empty() {
        config.id = uuid_v4_string();
    }
    if config.name.is_empty() {
        return Err(CommandError::validation("trigger name must not be empty"));
    }
    let id = config.id.clone();
    state
        .swarm
        .trigger_engine
        .create(config)
        .map_err(|e| CommandError::internal("trigger_create", &e))?;
    Ok(id)
}

/// 列出所有触发器(按 created_at ASC)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "trigger_list"))]
pub async fn trigger_list(state: State<'_, AppState>) -> Result<Vec<TriggerConfig>, CommandError> {
    state
        .swarm
        .trigger_engine
        .list()
        .map_err(|e| CommandError::internal("trigger_list", &e))
}

/// 删除触发器(停止 worker + 内存 map 移除 + DB 删除)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "trigger_delete"))]
pub async fn trigger_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    state
        .swarm
        .trigger_engine
        .delete(&id)
        .map_err(|e| CommandError::internal("trigger_delete", &e))
}

/// 启用/禁用触发器(更新 DB + 启停文件 worker)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "trigger_enable"))]
pub async fn trigger_enable(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), CommandError> {
    state
        .swarm
        .trigger_engine
        .set_enabled(&id, enabled)
        .map_err(|e| CommandError::internal("trigger_enable", &e))
}

/// 查询触发日志(最近 100 条,按 fired_at DESC)。
#[tauri::command]
#[instrument(skip(state), fields(otel.kind = "trigger_fire_log"))]
pub async fn trigger_fire_log(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<FireLogRow>, CommandError> {
    state
        .swarm
        .trigger_engine
        .fire_log(&id)
        .map_err(|e| CommandError::internal("trigger_fire_log", &e))
}

/// T-E-S-55: 手动测试 WatchSource(单次轮询,不持久化、不创建触发器)。
///
/// 用于前端配置面板预览:用户输入 URL/指标/ICS 路径后点击"测试",
/// 命令返回单次轮询结果(hash / 指标值 / 临近事件列表)。
#[tauri::command]
#[instrument(skip(_state), fields(otel.kind = "watch_test"))]
pub async fn watch_test(
    _state: State<'_, AppState>,
    source: WatchSource,
) -> Result<serde_json::Value, CommandError> {
    let result = match source {
        WatchSource::Web { url, selector, .. } => {
            let fetcher =
                WebFetcher::new().map_err(|e| CommandError::internal("watch_test", &e))?;
            let fetched = fetcher
                .fetch(&url)
                .await
                .map_err(|e| CommandError::internal("watch_test", &e))?;
            let content = selector
                .as_ref()
                .map(|s| apply_selector(&fetched.text, s))
                .unwrap_or(fetched.text);
            let hash = hash_text(&content);
            serde_json::json!({
                "type": "web",
                "url": url,
                "hash": hash,
                "content_preview": content.chars().take(500).collect::<String>(),
            })
        }
        WatchSource::System {
            metric,
            threshold,
            op,
            ..
        } => {
            let value = SystemProbe::read_metric(metric)
                .map_err(|e| CommandError::internal("watch_test", &e))?;
            let matches = SystemProbe::compare(value, op, threshold);
            serde_json::json!({
                "type": "system",
                "metric": metric,
                "value": value,
                "threshold": threshold,
                "op": op,
                "matches": matches,
            })
        }
        WatchSource::Calendar {
            ics_path,
            lead_minutes,
        } => {
            let content = std::fs::read_to_string(&ics_path)
                .map_err(|e| CommandError::internal("watch_test", &anyhow::anyhow!(e)))?;
            let events = parse_ics(&content);
            let now = chrono::Utc::now();
            let upcoming = find_upcoming_events(&events, now, lead_minutes);
            serde_json::json!({
                "type": "calendar",
                "ics_path": ics_path,
                "total_events": events.len(),
                "upcoming_count": upcoming.len(),
                "upcoming": upcoming.iter().map(|e| {
                    serde_json::json!({
                        "uid": e.uid,
                        "summary": e.summary,
                        "start": e.start.to_rfc3339(),
                    })
                }).collect::<Vec<_>>(),
            })
        }
    };
    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// 生成 UUIDv4 字符串(不引入 uuid crate 依赖,改用简易随机方案)。
///
/// 注:此处采用基于 `rand` 模块(若已存在)或系统时间 + 进程 id 的
/// 降级方案。考虑到 `uuid` 在 Cargo.lock 中作为 transitive dep 存在,
/// 但为遵循"不修改 Cargo.toml"约束,这里手写 v4 生成器。
fn uuid_v4_string() -> String {
    // 用 SystemTime + 进程 id + 一个原子计数器构造伪随机字节,
    // 然后按 RFC 4122 §4.4 格式化为 UUIDv4 字符串。
    // 这里复用 `chrono::Utc::now()` 的高精度时间戳作为种子。
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let pid = std::process::id() as u64;
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    // 16 字节 = 128 bit,按 v4 规则:version=4 (0b0100), variant=0b10。
    let mut bytes = [0u8; 16];
    let seed = now_ns ^ (pid << 32) ^ seq.rotate_left(13);
    // 简易 xorshift64 扩展到 128 字节
    let mut s = seed;
    for i in 0..2 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let off = i * 8;
        bytes[off..off + 8].copy_from_slice(&s.to_le_bytes());
    }
    // version
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    // variant
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format_uuid(&bytes)
}

/// 将 16 字节格式化为 `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`。
fn format_uuid(b: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_uuid_returns_canonical_form() {
        let b = [0u8; 16];
        let s = format_uuid(&b);
        assert_eq!(s, "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn test_uuid_v4_string_has_v4_version_and_variant() {
        let s = uuid_v4_string();
        assert_eq!(s.len(), 36);
        // version nibble 在字符串位置 14(第 3 段首字符)= '4'
        assert_eq!(s.as_bytes()[14], b'4');
        // variant 字节位置 19(第 4 段首字符)∈ {8, 9, a, b}
        let variant_char = s.as_bytes()[19];
        assert!(matches!(variant_char, b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn test_uuid_v4_string_unique() {
        let a = uuid_v4_string();
        let b = uuid_v4_string();
        assert_ne!(a, b, "consecutive UUIDs should differ");
    }
}
