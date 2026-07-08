//! T-E-C-09: 任务录屏回放 — 操作记录日志。
//!
//! 每个 Shadow Workspace 拥有一条操作时间线,记录 Agent 在隔离环境中的
//! 每一步操作(文件修改 + 命令执行 + 备注),供用户在合并前回放审查。
//!
//! 设计:
//! - 纯内存态(`RwLock<HashMap<workspace_id, Vec<OperationRecord>>>`),
//!   与 ShadowWorkspaceEngine 的 workspace 索引架构一致。
//! - 录屏**不随 merge/abort 清除**——合并/丢弃后用户仍可回看 Agent 做了什么,
//!   仅在显式 `clear()` 或进程退出时丢失。
//! - `run_command()` 由引擎自动记录 Command 操作;文件操作由 Agent 显式调用
//!   `record()` 记录(因为文件写入不经过引擎)。
//! - `seq` 为 workspace 内自增序号(从 1 开始),供前端按顺序渲染。

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// 单条操作记录的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// 新建文件(worktree 中原先不存在)。
    FileCreate,
    /// 修改已存在的文件。
    FileWrite,
    /// 删除文件。
    FileDelete,
    /// 在 worktree 内执行命令(由 run_command 自动记录)。
    Command,
    /// Agent 备注(自由文本,如"修复了编译错误")。
    Note,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationKind::FileCreate => "file_create",
            OperationKind::FileWrite => "file_write",
            OperationKind::FileDelete => "file_delete",
            OperationKind::Command => "command",
            OperationKind::Note => "note",
        }
    }
}

/// 单条操作记录(序列化给前端回放)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    /// workspace 内自增序号(从 1 开始)。
    pub seq: u32,
    /// 操作时间(Unix 毫秒)。
    pub ts_ms: i64,
    /// 操作种类。
    pub kind: OperationKind,
    /// 操作目标:
    /// - File*: 相对 worktree 根的文件路径
    /// - Command: 程序名
    /// - Note: 空
    pub target: String,
    /// 操作详情:
    /// - File*: 新内容摘要(前 200 字符)或空
    /// - Command: 参数空格拼接
    /// - Note: 备注全文
    pub detail: String,
    /// 是否成功(命令退出码 0 / 文件操作未抛异常)。
    pub success: bool,
    /// 附加消息:
    /// - Command: stdout+stderr 摘要(前 500 字符)
    /// - 失败时:错误描述
    pub message: String,
}

/// 录屏日志存储(每个 workspace 一条时间线)。
///
/// 线程安全(`RwLock`),可被多线程并发读写。
/// 不持久化——与 workspace 索引一致,重启后清空。
pub struct RecordingLog {
    entries: RwLock<HashMap<String, Vec<OperationRecord>>>,
}

impl RecordingLog {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    fn now_ms() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// 截断字符串到最大长度,超出则追加 "…"。
    fn truncate(s: String, max: usize) -> String {
        if s.chars().count() <= max {
            s
        } else {
            let truncated: String = s.chars().take(max).collect();
            format!("{truncated}…")
        }
    }

    /// 追加一条操作记录,返回写入的记录(含分配的 seq)。
    pub fn record(
        &self,
        workspace_id: &str,
        kind: OperationKind,
        target: String,
        detail: String,
        success: bool,
        message: String,
    ) -> OperationRecord {
        let mut map = self.entries.write();
        let v = map.entry(workspace_id.to_string()).or_default();
        let seq = (v.len() as u32) + 1;
        let rec = OperationRecord {
            seq,
            ts_ms: Self::now_ms(),
            kind,
            target: Self::truncate(target, 300),
            detail: Self::truncate(detail, 200),
            success,
            message: Self::truncate(message, 500),
        };
        v.push(rec.clone());
        rec
    }

    /// 获取 workspace 的完整操作时间线(按 seq 升序)。
    /// workspace 无记录时返回空 Vec。
    pub fn list(&self, workspace_id: &str) -> Vec<OperationRecord> {
        self.entries
            .read()
            .get(workspace_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 清除 workspace 的录屏(合并/丢弃后可选清理)。
    pub fn clear(&self, workspace_id: &str) {
        self.entries.write().remove(workspace_id);
    }

    /// 已记录的操作总数(跨所有 workspace,主要用于诊断/测试)。
    pub fn total_count(&self) -> usize {
        self.entries.read().values().map(|v| v.len()).sum()
    }
}

impl Default for RecordingLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_assigns_sequential_seq() {
        let log = RecordingLog::new();
        let r1 = log.record(
            "ws1",
            OperationKind::Note,
            String::new(),
            "第一步".into(),
            true,
            String::new(),
        );
        let r2 = log.record(
            "ws1",
            OperationKind::Note,
            String::new(),
            "第二步".into(),
            true,
            String::new(),
        );
        let r3 = log.record(
            "ws1",
            OperationKind::Note,
            String::new(),
            "第三步".into(),
            true,
            String::new(),
        );
        assert_eq!(r1.seq, 1);
        assert_eq!(r2.seq, 2);
        assert_eq!(r3.seq, 3);
        // 时间戳单调非递减
        assert!(r2.ts_ms >= r1.ts_ms);
        assert!(r3.ts_ms >= r2.ts_ms);
    }

    #[test]
    fn list_returns_ops_in_seq_order() {
        let log = RecordingLog::new();
        log.record(
            "ws",
            OperationKind::FileCreate,
            "a.txt".into(),
            "content".into(),
            true,
            String::new(),
        );
        log.record(
            "ws",
            OperationKind::Command,
            "cargo".into(),
            "build".into(),
            true,
            "ok".into(),
        );
        log.record(
            "ws",
            OperationKind::FileWrite,
            "a.txt".into(),
            "changed".into(),
            true,
            String::new(),
        );

        let ops = log.list("ws");
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0].seq, 1);
        assert_eq!(ops[0].kind, OperationKind::FileCreate);
        assert_eq!(ops[1].seq, 2);
        assert_eq!(ops[1].kind, OperationKind::Command);
        assert_eq!(ops[2].seq, 3);
        assert_eq!(ops[2].kind, OperationKind::FileWrite);
    }

    #[test]
    fn list_unknown_workspace_returns_empty() {
        let log = RecordingLog::new();
        assert!(log.list("nope").is_empty());
    }

    #[test]
    fn workspaces_are_isolated() {
        let log = RecordingLog::new();
        log.record(
            "ws-a",
            OperationKind::Note,
            String::new(),
            "A1".into(),
            true,
            String::new(),
        );
        log.record(
            "ws-b",
            OperationKind::Note,
            String::new(),
            "B1".into(),
            true,
            String::new(),
        );
        log.record(
            "ws-a",
            OperationKind::Note,
            String::new(),
            "A2".into(),
            true,
            String::new(),
        );

        assert_eq!(log.list("ws-a").len(), 2);
        assert_eq!(log.list("ws-b").len(), 1);
        // ws-b 的 seq 独立从 1 开始
        assert_eq!(log.list("ws-b")[0].seq, 1);
    }

    #[test]
    fn clear_removes_workspace_timeline() {
        let log = RecordingLog::new();
        log.record(
            "ws",
            OperationKind::Note,
            String::new(),
            "x".into(),
            true,
            String::new(),
        );
        assert_eq!(log.list("ws").len(), 1);
        log.clear("ws");
        assert!(log.list("ws").is_empty());
        // clear 不存在的 workspace 不报错
        log.clear("never-existed");
    }

    #[test]
    fn long_detail_is_truncated() {
        let log = RecordingLog::new();
        let long = "x".repeat(500);
        let rec = log.record(
            "ws",
            OperationKind::Note,
            String::new(),
            long.clone(),
            true,
            String::new(),
        );
        // detail 截断到 200 字符 + "…"
        assert_eq!(rec.detail.chars().count(), 201);
        assert!(rec.detail.ends_with('…'));
        // 原始内容前 200 字符保留
        assert!(rec.detail.starts_with(&"x".repeat(200)));
    }

    #[test]
    fn total_count_sums_across_workspaces() {
        let log = RecordingLog::new();
        log.record(
            "a",
            OperationKind::Note,
            String::new(),
            "1".into(),
            true,
            String::new(),
        );
        log.record(
            "a",
            OperationKind::Note,
            String::new(),
            "2".into(),
            true,
            String::new(),
        );
        log.record(
            "b",
            OperationKind::Note,
            String::new(),
            "3".into(),
            true,
            String::new(),
        );
        assert_eq!(log.total_count(), 3);
    }

    #[test]
    fn kind_serializes_to_snake_case() {
        let json = serde_json::to_string(&OperationKind::FileCreate).expect("create should succeed");
        assert_eq!(json, "\"file_create\"");
        let json = serde_json::to_string(&OperationKind::Command).expect("serialize should succeed");
        assert_eq!(json, "\"command\"");
    }
}
