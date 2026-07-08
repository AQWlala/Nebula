//! T-S2-B-02: MCP JSON-RPC 2.0 协议帧编解码。
//!
//! MCP（Model Context Protocol）使用 JSON-RPC 2.0 作为消息格式，
//! stdio 传输层以换行符分隔每条消息。本模块提供:
//!
//! * [`JsonRpcRequest`] / [`JsonRpcResponse`] / [`JsonRpcError`] — 消息结构
//! * [`parse_frame()`] — 反序列化一条 JSON-RPC 消息
//! * [`write_frame()`] — 序列化并追加换行符
//! * [`next_request_id()`] — 单调递增的请求 ID 生成器
//!
//! 参考: https://spec.modelcontextprotocol.io/specification/basic/messages/

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 请求/通知封装。
///
/// `id` 为 `None` 时表示通知（无需响应）; 为 `Some(n)` 时表示请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(method: &str, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            id: Some(Value::from(id)),
            params: None,
        }
    }

    pub fn notification(method: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            id: None,
            params: None,
        }
    }

    pub fn with_params(mut self, params: Value) -> Self {
        self.params = Some(params);
        self
    }
}

/// JSON-RPC 2.0 错误对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 响应封装。
///
/// 响应必含 `id`（与请求对应），且 `result` 与 `error` 互斥。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// 返回 `result` 字段（若响应成功）。
    pub fn result_value(&self) -> Result<&Value> {
        if let Some(err) = &self.error {
            anyhow::bail!(
                "JSON-RPC error code={} message={} data={:?}",
                err.code,
                err.message,
                err.data
            );
        }
        self.result
            .as_ref()
            .context("JSON-RPC response missing result field")
    }
}

/// JSON-RPC 标准错误码。
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// 解析一条 JSON-RPC 帧（单行 JSON 文本）。
///
/// 输入应为**不含**末尾换行符的 UTF-8 字符串。
pub fn parse_frame(line: &str) -> Result<JsonRpcResponse> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty JSON-RPC frame");
    }
    serde_json::from_str(trimmed)
        .with_context(|| format!("failed to parse JSON-RPC frame: {}", trimmed))
}

/// 将一条 JSON-RPC 请求序列化为单行帧（末尾带 `\n`）。
///
/// 返回的 `String` 可直接写入 stdio 子进程的 stdin。
pub fn write_frame(req: &JsonRpcRequest) -> Result<String> {
    let mut json = serde_json::to_string(req).context("failed to serialize JSON-RPC request")?;
    json.push('\n');
    Ok(json)
}

/// 单调递增的 JSON-RPC 请求 ID 生成器。
///
/// MCP 规范允许 id 为整数或字符串;此处使用 `u64` 整数以简化实现。
#[derive(Debug, Default)]
pub struct RequestIdGen {
    next: std::sync::atomic::AtomicU64,
}

impl RequestIdGen {
    pub fn new() -> Self {
        Self {
            next: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn next_id(&self) -> u64 {
        self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_id_and_params() {
        let req = JsonRpcRequest::new("tools/list", 1).with_params(serde_json::json!({}));
        let frame = write_frame(&req).expect("update should succeed");
        assert!(frame.ends_with('\n'));
        let parsed: Value = serde_json::from_str(frame.trim()).expect("parse should succeed");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["method"], "tools/list");
        assert_eq!(parsed["id"], 1);
    }

    #[test]
    fn notification_omits_id() {
        let req = JsonRpcRequest::notification("initialized");
        let json = serde_json::to_string(&req).expect("serialize should succeed");
        let parsed: Value = serde_json::from_str(&json).expect("parse should succeed");
        assert!(parsed.get("id").is_none() || parsed["id"].is_null());
        assert_eq!(parsed["method"], "initialized");
    }

    #[test]
    fn response_result_parsed() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp = parse_frame(raw).expect("parse should succeed");
        assert_eq!(resp.id, Value::from(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        let v = resp.result_value().expect("test op should succeed");
        assert_eq!(v["tools"], serde_json::json!([]));
    }

    #[test]
    fn response_error_propagates() {
        let raw =
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"method not found"}}"#;
        let resp = parse_frame(raw).expect("parse should succeed");
        let err = resp.result_value().unwrap_err();
        assert!(format!("{}", err).contains("-32601"));
        assert!(format!("{}", err).contains("method not found"));
    }

    #[test]
    fn empty_frame_errors() {
        let r = parse_frame("   ");
        assert!(r.is_err());
    }

    #[test]
    fn request_id_gen_is_monotonic() {
        let gen = RequestIdGen::new();
        assert_eq!(gen.next_id(), 1);
        assert_eq!(gen.next_id(), 2);
        assert_eq!(gen.next_id(), 3);
    }
}
