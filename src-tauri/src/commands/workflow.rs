//! T-E-S-10: WorkflowCanvas 后端命令 — 工作流保存 / 加载 / 列表 / 执行。
//!
//! ## 注册说明(本文件已就绪,但受并发约束暂未注册)
//!
//! 要启用这些命令,需要在以下两处追加(由集成者完成,避免并发冲突):
//!
//! 1. `src-tauri/src/commands/mod.rs` 末尾添加:
//!    ```rust
//!    pub mod workflow;
//!    pub use workflow::*;
//!    ```
//! 2. `src-tauri/src/tauri_setup.rs` 的 `generate_handler!` 宏中追加:
//!    ```rust
//!    commands::workflow::workflow_save,
//!    commands::workflow::workflow_load,
//!    commands::workflow::workflow_list,
//!    commands::workflow::workflow_delete,
//!    commands::workflow::workflow_execute,
//!    ```
//!
//! 前端 `WorkflowCanvas.tsx` 已通过 `invokeTauri` 调用这些命令,
//! 后端未注册时会自动降级到 localStorage,保证前端可用。
//!
//! ## 持久化路径
//! 工作流 JSON 文档保存在 Nebula 数据目录下的 `workflows/` 子目录,
//! 每份文档一个 `<name>.json` 文件。文件名做基本净化(去除路径分隔符)。
//! 复用 `crate::backup::commands::resolve_app_data_dir()` 解析数据目录,
//! 与 nebula.db / Lance 存储路径保持一致。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::commands::error::CommandError;

/// 工作流保存请求(与前端 WorkflowDocument 对齐,字段宽松校验)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSaveRequest {
    /// 工作流名称(同时作为文件名)。
    pub name: String,
    /// 完整文档 JSON 字符串(前端序列化)。
    pub document_json: String,
}

/// 工作流执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowExecuteResult {
    /// 转换后的 SwarmTask 描述。
    pub description: String,
    /// 涉及的 Agent 角色列表。
    pub agents: Vec<String>,
    /// 最大重试次数。
    pub max_retries: u32,
}

/// 计算工作流存储目录: `<app_data_dir>/workflows/`。
fn workflows_dir() -> Result<PathBuf, CommandError> {
    let dir = crate::backup::commands::resolve_app_data_dir()
        .map_err(|e| CommandError::internal("workflow_save", &anyhow::anyhow!("{e}")))?;
    let wf_dir = dir.join("workflows");
    if !wf_dir.exists() {
        std::fs::create_dir_all(&wf_dir)
            .map_err(|e| CommandError::internal("workflow_save", &anyhow::anyhow!("{e}")))?;
    }
    Ok(wf_dir)
}

/// 净化工作流名称:仅保留字母数字 / 下划线 / 短横线 / 中文,
/// 防止路径遍历。返回 `<name>.json` 文件名。
fn safe_filename(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::validation("workflow_save: name is empty"));
    }
    let sanitized: String = trimmed
        .chars()
        .filter(|c| {
            c.is_alphanumeric()
                || *c == '_'
                || *c == '-'
                || *c == '.'
                || ('\u{4e00}'..='\u{9fff}').contains(c)
        })
        .collect();
    // Strip leading/trailing dots so that strings like "///\\..." (which
    // survive as "..." after filtering) are rejected.
    let sanitized = sanitized.trim_matches('.');
    if sanitized.is_empty() {
        return Err(CommandError::validation(
            "workflow_save: name contains no valid characters",
        ));
    }
    // 限制长度,避免文件系统错误。按字符截断(非字节),防止切断多字节 CJK 字符。
    let truncated: String = sanitized.chars().take(128).collect();
    Ok(format!("{truncated}.json"))
}

/// Tauri 命令:保存工作流文档到磁盘。
#[tauri::command]
#[instrument(skip(request), fields(otel.kind = "workflow_save", name = %request.name))]
pub async fn workflow_save(request: WorkflowSaveRequest) -> Result<bool, CommandError> {
    let dir = workflows_dir()?;
    let filename = safe_filename(&request.name)?;
    let path = dir.join(&filename);
    // 校验 JSON 合法性后再写入,避免损坏文件。
    serde_json::from_str::<serde_json::Value>(&request.document_json).map_err(|e| {
        CommandError::validation("workflow_save: invalid JSON").with_details(format!("{e}"))
    })?;
    // 原子写入:先写临时文件再重命名,避免崩溃导致半写文件。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &request.document_json)
        .map_err(|e| CommandError::internal("workflow_save", &anyhow::anyhow!("{e}")))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| CommandError::internal("workflow_save", &anyhow::anyhow!("{e}")))?;
    tracing::info!(
        target: "nebula.cmd",
        name = %request.name,
        filename = %filename,
        "workflow saved"
    );
    Ok(true)
}

/// Tauri 命令:从磁盘加载工作流文档,返回 JSON 字符串。
#[tauri::command]
#[instrument(skip(name), fields(otel.kind = "workflow_load", name = %name))]
pub async fn workflow_load(name: String) -> Result<Option<String>, CommandError> {
    let dir = workflows_dir()?;
    let filename = safe_filename(&name)?;
    let path = dir.join(&filename);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| CommandError::internal("workflow_load", &anyhow::anyhow!("{e}")))?;
    Ok(Some(content))
}

/// Tauri 命令:列出所有已保存的工作流名称(去掉 .json 后缀)。
#[tauri::command]
#[instrument(fields(otel.kind = "workflow_list"))]
pub async fn workflow_list() -> Result<Vec<String>, CommandError> {
    let dir = workflows_dir()?;
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(stripped) = name.strip_suffix(".json") {
                    names.push(stripped.to_string());
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Tauri 命令:删除指定名称的工作流。
#[tauri::command]
#[instrument(skip(name), fields(otel.kind = "workflow_delete", name = %name))]
pub async fn workflow_delete(name: String) -> Result<bool, CommandError> {
    let dir = workflows_dir()?;
    let filename = safe_filename(&name)?;
    let path = dir.join(&filename);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path)
        .map_err(|e| CommandError::internal("workflow_delete", &anyhow::anyhow!("{e}")))?;
    Ok(true)
}

/// Tauri 命令:解析工作流文档为 SwarmTask 描述(供前端执行前的服务端校验)。
///
/// 此命令为薄封装:解析前端传入的工作流 JSON,提取 agent 节点
/// 与拓扑顺序,组装 SwarmTask 描述。前端 `WorkflowCanvas.tsx` 默认
/// 直接调用 `swarm_execute`(已注册)完成执行;此命令提供"后端解析"
/// 的可选路径,供需要服务端编排校验时使用。
#[tauri::command]
#[instrument(skip(document_json), fields(otel.kind = "workflow_execute"))]
pub async fn workflow_execute(
    document_json: String,
) -> Result<WorkflowExecuteResult, CommandError> {
    let value: serde_json::Value = serde_json::from_str(&document_json).map_err(|e| {
        CommandError::validation("workflow_execute: invalid JSON").with_details(format!("{e}"))
    })?;
    // 解析节点与边(宽松字段访问,容错前端格式演进)。
    let nodes = value
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let edges = value
        .get("edges")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // 拓扑排序(Kahn 算法)。
    let mut in_degree: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut adj: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for n in &nodes {
        if let Some(id) = n.get("id").and_then(|v| v.as_str()) {
            in_degree.insert(id.to_string(), 0);
            adj.insert(id.to_string(), Vec::new());
        }
    }
    for e in &edges {
        let src = e.get("source").and_then(|v| v.as_str());
        let dst = e.get("target").and_then(|v| v.as_str());
        if let (Some(s), Some(d)) = (src, dst) {
            if in_degree.contains_key(s) && in_degree.contains_key(d) {
                adj.get_mut(s).unwrap().push(d.to_string());
                *in_degree.get_mut(d).unwrap() += 1;
            }
        }
    }
    let mut queue: std::collections::VecDeque<String> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(k, _)| k.clone())
        .collect();
    let mut order: Vec<String> = Vec::new();
    while let Some(cur) = queue.pop_front() {
        order.push(cur.clone());
        if let Some(nexts) = adj.get(&cur) {
            for next in nexts {
                if let Some(d) = in_degree.get_mut(next) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }
    // 若存在环,回退为节点原顺序。
    if order.len() != nodes.len() {
        order = nodes
            .iter()
            .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect();
    }

    // 按拓扑顺序拼接描述,收集 agent 角色。
    let node_map: std::collections::HashMap<String, &serde_json::Value> = nodes
        .iter()
        .filter_map(|n| {
            n.get("id")
                .and_then(|v| v.as_str())
                .map(|id| (id.to_string(), n))
        })
        .collect();
    let mut lines: Vec<String> = Vec::new();
    let mut agents: Vec<String> = Vec::new();
    let mut max_retries: u32 = 1;
    for id in &order {
        let Some(node) = node_map.get(id) else {
            continue;
        };
        let title = node
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(untitled)");
        let ntype = node.get("type").and_then(|v| v.as_str()).unwrap_or("task");
        let cfg = node
            .get("config")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match ntype {
            "io" => {
                let dir = cfg
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("input");
                if dir == "input" {
                    let content = cfg
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(empty)");
                    lines.push(format!("[Input] {title}: {content}"));
                } else {
                    let fmt = cfg.get("format").and_then(|v| v.as_str()).unwrap_or("text");
                    lines.push(format!("[Output] {title} ({fmt})"));
                }
            }
            "task" => {
                let desc = cfg
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let prog = cfg
                    .get("program")
                    .and_then(|v| v.as_str())
                    .unwrap_or("echo");
                let args = cfg.get("args").and_then(|v| v.as_str()).unwrap_or("");
                lines.push(format!("[Task] {title}: {desc} ({prog} {args})"));
            }
            "condition" => {
                let expr = cfg
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("true");
                lines.push(format!("[Condition] {title}: if ({expr})"));
            }
            "agent" => {
                let kind = cfg
                    .get("agent_kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("generic");
                let prompt = cfg
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no prompt)");
                lines.push(format!("[Agent:{kind}] {title}: {prompt}"));
                if !agents.contains(&kind.to_string()) {
                    agents.push(kind.to_string());
                }
                let retries = cfg.get("max_retries").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                if retries > max_retries {
                    max_retries = retries;
                }
            }
            _ => {}
        }
    }
    if agents.is_empty() {
        agents.push("generic".to_string());
    }

    tracing::info!(
        target: "nebula.cmd",
        node_count = nodes.len(),
        edge_count = edges.len(),
        agent_count = agents.len(),
        "workflow_execute: parsed workflow"
    );

    Ok(WorkflowExecuteResult {
        description: lines.join("\n"),
        agents,
        max_retries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_rejects_empty() {
        assert!(safe_filename("").is_err());
        assert!(safe_filename("   ").is_err());
    }

    #[test]
    fn safe_filename_strips_path_separators() {
        let f = safe_filename("..\\evil../path").unwrap();
        assert!(!f.contains('\\'));
        assert!(!f.contains('/'));
        assert!(f.ends_with(".json"));
    }

    #[test]
    fn safe_filename_keeps_alnum_and_cjk() {
        let f = safe_filename("写作工作流-1").unwrap();
        assert_eq!(f, "写作工作流-1.json");
    }

    #[test]
    fn safe_filename_rejects_only_symbols() {
        assert!(safe_filename("///\\\\...").is_err());
    }

    #[test]
    fn safe_filename_truncates_long_name() {
        let long = "a".repeat(300);
        let f = safe_filename(&long).unwrap();
        // 128 字符 + ".json"
        assert_eq!(f.len(), 128 + 5);
    }
}
