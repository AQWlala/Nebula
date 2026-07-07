//! 进化日志（evolution_log.md）— M4 任务 #64。
//!
//! 每次 EvolutionEngine 执行 Phase（Extract/Compile/Reflect/Soul）后，
//! 写入一条 `EvolutionLogEntry` 到 `evolution_log.md`。
//!
//! ## 文件格式
//!
//! ```markdown
//! # Evolution Log
//!
//! ## [evolve_2026-07-05T10-30-00Z_extract] Phase: extract
//! - timestamp: 2026-07-05T10:30:00Z
//! - master_id: agent_a
//! - memory_id: abc-123
//! - content_bytes: 1024
//! - soul_md_path: (none)
//!
//! ## [evolve_2026-07-05T10-30-05Z_soul] Phase: soul
//! - timestamp: 2026-07-05T10:30:05Z
//! - master_id: agent_a
//! - memory_id: (none)
//! - content_bytes: 512
//! - soul_md_path: /path/to/SOUL.md
//! ```
//!
//! ## 设计要点
//!
//! - **追加式**：每次 append 一条，不重写整个文件
//! - **provenance 记录**：master_id + memory_id + 时间戳
//! - **可回滚**：rollback(N) 通过查找 entry_id 定位对应行
//! - **与 SOUL.md 同事务写入**：Phase 4 写完 SOUL.md 后立即写日志（pipeline.rs 已实现）
//! - **失败不阻断**：日志写入失败仅记 warning，不影响进化本身

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

use super::pipeline::EvolutionPhase;

/// 进化日志错误类型。
#[derive(Debug, Error)]
pub enum EvolutionLogError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialize error: {0}")]
    Serialize(String),
}

/// 单条进化日志条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionLogEntry {
    /// 唯一 ID（格式：`evolve_<timestamp>_<phase>`）。
    pub entry_id: String,
    /// Phase 类型。
    pub phase: EvolutionPhase,
    /// UTC 时间戳（RFC3339）。
    pub timestamp: String,
    /// master_id（domain 标识）。
    pub master_id: String,
    /// 写入的 memory ID（Phase 4 Soul 为空字符串）。
    pub memory_id: String,
    /// 写入内容字节数。
    pub content_bytes: u64,
    /// SOUL.md 路径（仅 Phase 4 Soul 非空）。
    pub soul_md_path: String,
}

impl EvolutionLogEntry {
    /// 构造新条目，自动生成 entry_id 和 timestamp。
    pub fn new(
        phase: EvolutionPhase,
        master_id: &str,
        memory_id: &str,
        content_bytes: u64,
    ) -> Self {
        let now = chrono::Utc::now();
        let timestamp = now.to_rfc3339();
        // 文件名安全的时间戳格式（避免冒号）
        let safe_ts = now.format("%Y-%m-%dT%H-%M-%SZ").to_string();
        let entry_id = format!("evolve_{safe_ts}_{}", phase.as_str());

        Self {
            entry_id,
            phase,
            timestamp,
            master_id: master_id.to_string(),
            memory_id: memory_id.to_string(),
            content_bytes,
            soul_md_path: String::new(),
        }
    }

    /// 设置 SOUL.md 路径（仅 Phase 4 使用）。
    pub fn with_soul_md_path(mut self, path: &str) -> Self {
        self.soul_md_path = path.to_string();
        self
    }

    /// 序列化为 Markdown 段落。
    pub fn to_markdown(&self) -> String {
        let memory_id_display = if self.memory_id.is_empty() {
            "(none)"
        } else {
            &self.memory_id
        };
        let soul_path_display = if self.soul_md_path.is_empty() {
            "(none)"
        } else {
            &self.soul_md_path
        };

        format!(
            "## [{entry_id}] Phase: {phase}\n\
             - timestamp: {timestamp}\n\
             - master_id: {master_id}\n\
             - memory_id: {memory_id}\n\
             - content_bytes: {content_bytes}\n\
             - soul_md_path: {soul_path}\n",
            entry_id = self.entry_id,
            phase = self.phase.as_str(),
            timestamp = self.timestamp,
            master_id = self.master_id,
            memory_id = memory_id_display,
            content_bytes = self.content_bytes,
            soul_path = soul_path_display,
        )
    }
}

/// 进化日志文件管理器。
///
/// 持有文件路径 + Mutex（保证并发写入互斥）。
/// 所有写入操作都是追加式，不重写整个文件。
pub struct EvolutionLog {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl std::fmt::Debug for EvolutionLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvolutionLog")
            .field("path", &self.path)
            .finish()
    }
}

impl EvolutionLog {
    /// 构造 EvolutionLog。若文件不存在，首次 append 时自动创建并写入文件头。
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
    }

    /// 返回日志文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一条日志条目。返回写入的 entry_id。
    ///
    /// 线程安全：通过 Mutex 保证并发写入互斥。
    /// 失败不 panic，返回 Err。
    pub async fn append(&self, entry: &EvolutionLogEntry) -> Result<String, EvolutionLogError> {
        let _guard = self.write_lock.lock().await;

        // 确保父目录存在
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let is_new = !self.path.exists();
        let mut content = String::new();
        if is_new {
            content.push_str("# Evolution Log\n\n");
        }
        content.push_str(&entry.to_markdown());
        content.push('\n');

        // 追加写入（不是原子写入，但日志文件丢失可接受）
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;

        Ok(entry.entry_id.clone())
    }

    /// 读取整个日志文件内容（用于回滚查询 / 前端展示）。
    pub fn read_all(&self) -> Result<String, EvolutionLogError> {
        if !self.path.exists() {
            return Ok(String::new());
        }
        std::fs::read_to_string(&self.path).map_err(Into::into)
    }

    /// 查找指定 entry_id 的日志条目。
    ///
    /// 通过简单的 Markdown 解析（查找 `## [<entry_id>]` 标记）。
    /// 找不到返回 None。
    pub fn find_entry(
        &self,
        entry_id: &str,
    ) -> Result<Option<EvolutionLogEntry>, EvolutionLogError> {
        let content = self.read_all()?;
        if content.is_empty() {
            return Ok(None);
        }

        let marker = format!("## [{entry_id}]");
        let start = match content.find(&marker) {
            Some(i) => i,
            None => return Ok(None),
        };

        // 条目结束于下一个 `## ` 或文件末尾
        let after_start = start + marker.len();
        let end = content[after_start..]
            .find("\n## ")
            .map(|i| after_start + i)
            .unwrap_or(content.len());

        let section = &content[start..end];
        Ok(parse_entry_from_markdown(section))
    }

    /// 列出所有条目（按写入顺序）。
    pub fn list_all(&self) -> Result<Vec<EvolutionLogEntry>, EvolutionLogError> {
        let content = self.read_all()?;
        if content.is_empty() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut remaining = content.as_str();

        while let Some(start) = remaining.find("## [") {
            let after_marker = &remaining[start..];
            // 下一个 `## [` 之前的内容
            let next_start = after_marker[4..]
                .find("\n## [")
                .map(|i| 4 + i + 1)
                .unwrap_or(after_marker.len());

            let section = &remaining[start..start + next_start];
            if let Some(entry) = parse_entry_from_markdown(section) {
                entries.push(entry);
            }
            remaining = &remaining[start + next_start..];
        }

        Ok(entries)
    }

    /// 删除指定 entry_id 的条目（用于回滚后清理日志）。
    ///
    /// 通过读取整个文件 → 删除对应段落 → 重写文件实现。
    /// 失败不 panic，返回 Err。
    pub async fn remove_entry(&self, entry_id: &str) -> Result<bool, EvolutionLogError> {
        let _guard = self.write_lock.lock().await;

        if !self.path.exists() {
            return Ok(false);
        }

        let content = std::fs::read_to_string(&self.path)?;
        let marker = format!("## [{entry_id}]");
        let start = match content.find(&marker) {
            Some(i) => i,
            None => return Ok(false),
        };

        // 段落结束于下一个 `## ` 或文件末尾
        let after_start = start + marker.len();
        let end = content[after_start..]
            .find("\n## ")
            .map(|i| after_start + i + 1)
            .unwrap_or(content.len());

        let new_content = format!("{}{}", &content[..start], &content[end..]);
        std::fs::write(&self.path, new_content)?;
        Ok(true)
    }
}

/// 从 Markdown 段落解析出 `EvolutionLogEntry`。
///
/// 解析规则：从 `## [<entry_id>] Phase: <phase>` 提取 entry_id 和 phase，
/// 然后从后续 `- key: value` 行提取其他字段。
fn parse_entry_from_markdown(section: &str) -> Option<EvolutionLogEntry> {
    let mut lines = section.lines();

    // 第一行：`## [<entry_id>] Phase: <phase>`
    let header = lines.next()?;
    let header = header.trim_start_matches('#').trim();
    // 格式：[<entry_id>] Phase: <phase>
    let header = header.strip_prefix('[')?;
    let close = header.find(']')?;
    let entry_id = header[..close].to_string();
    let after_bracket = &header[close + 1..].trim_start();
    let phase_str = after_bracket.strip_prefix("Phase: ")?.trim();

    let phase = match phase_str {
        "extract" => EvolutionPhase::Extract,
        "compile" => EvolutionPhase::Compile,
        "reflect" => EvolutionPhase::Reflect,
        "soul" => EvolutionPhase::Soul,
        _ => return None,
    };

    let mut timestamp = String::new();
    let mut master_id = String::new();
    let mut memory_id = String::new();
    let mut content_bytes: u64 = 0;
    let mut soul_md_path = String::new();

    for line in lines {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("- timestamp: ") {
            timestamp = v.to_string();
        } else if let Some(v) = line.strip_prefix("- master_id: ") {
            master_id = v.to_string();
        } else if let Some(v) = line.strip_prefix("- memory_id: ") {
            memory_id = if v == "(none)" {
                String::new()
            } else {
                v.to_string()
            };
        } else if let Some(v) = line.strip_prefix("- content_bytes: ") {
            content_bytes = v.parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("- soul_md_path: ") {
            soul_md_path = if v == "(none)" {
                String::new()
            } else {
                v.to_string()
            };
        }
    }

    Some(EvolutionLogEntry {
        entry_id,
        phase,
        timestamp,
        master_id,
        memory_id,
        content_bytes,
        soul_md_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_to_markdown_roundtrip() {
        let entry = EvolutionLogEntry::new(EvolutionPhase::Extract, "agent_a", "mem-123", 1024);
        let md = entry.to_markdown();
        assert!(md.contains("Phase: extract"));
        assert!(md.contains("master_id: agent_a"));
        assert!(md.contains("memory_id: mem-123"));
        assert!(md.contains("content_bytes: 1024"));
        assert!(md.contains("soul_md_path: (none)"));
    }

    #[test]
    fn entry_with_soul_md_path_serializes() {
        let entry = EvolutionLogEntry::new(EvolutionPhase::Soul, "agent_a", "", 512)
            .with_soul_md_path("/path/to/SOUL.md");
        let md = entry.to_markdown();
        assert!(md.contains("memory_id: (none)"));
        assert!(md.contains("soul_md_path: /path/to/SOUL.md"));
    }

    #[test]
    fn parse_entry_extracts_fields() {
        let section = "## [evolve_2026-07-05T10-30-00Z_extract] Phase: extract\n\
                        - timestamp: 2026-07-05T10:30:00+00:00\n\
                        - master_id: agent_a\n\
                        - memory_id: mem-123\n\
                        - content_bytes: 1024\n\
                        - soul_md_path: (none)\n";
        let entry = parse_entry_from_markdown(section).expect("parse should succeed");
        assert_eq!(entry.entry_id, "evolve_2026-07-05T10-30-00Z_extract");
        assert_eq!(entry.phase, EvolutionPhase::Extract);
        assert_eq!(entry.master_id, "agent_a");
        assert_eq!(entry.memory_id, "mem-123");
        assert_eq!(entry.content_bytes, 1024);
        assert_eq!(entry.soul_md_path, "");
    }

    #[test]
    fn parse_entry_returns_none_for_invalid_header() {
        assert!(parse_entry_from_markdown("not a valid header").is_none());
        assert!(parse_entry_from_markdown("## [id] Phase: unknown_phase\n").is_none());
    }
}
