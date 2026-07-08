//! T-E-S-02: Function Calling 类型定义。
//!
//! `ToolCall` 和 `ToolResult` 是 Swarm 层面(orchestrator / negotiator)
//! 使用的工具调用抽象,与 `ollama::ToolCall`(LLM 协议层)解耦:
//!
//! - `ollama::ToolCall` 含 `ty` + `FunctionCall`(arguments 为 JSON 字符串),
//!   严格对齐 OpenAI wire format;
//! - `swarm::tool_types::ToolCall` 含 `function_name` + `arguments`(已解析为 Value),
//!   更适合 orchestrator 内部逻辑使用。

use serde::{Deserialize, Serialize};

/// T-E-S-02: Agent 请求调用的工具。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// 调用 ID(如 "call_abc123"),用于关联 tool 角色响应。
    pub id: String,
    /// 函数名(对应 ToolRegistry 中注册的工具名)。
    pub function_name: String,
    /// 参数(已解析为 serde_json::Value,非原始 JSON 字符串)。
    pub arguments: serde_json::Value,
}

/// T-E-S-02: 工具执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// 关联的 tool_call ID。
    pub tool_call_id: String,
    /// 工具输出(成功为结果文本,失败为错误信息)。
    pub content: String,
    /// 是否为错误结果。
    pub is_error: bool,
}

/// T-E-S-02: 从 DeepSeek / OpenAI 兼容格式的 JSON 解析 tool_calls。
///
/// 输入格式:
/// ```json
/// {"choices":[{"message":{"tool_calls":[{"id":"call_xxx","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"...\"}"}}]}}]}
/// ```
pub fn parse_deepseek_tool_calls(json: &serde_json::Value) -> Vec<ToolCall> {
    let choices = match json.get("choices").and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };
    let first = match choices.first() {
        Some(c) => c,
        None => return Vec::new(),
    };
    let message = match first.get("message") {
        Some(m) => m,
        None => return Vec::new(),
    };
    let tool_calls = match message.get("tool_calls").and_then(|tc| tc.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };
    tool_calls
        .iter()
        .filter_map(|tc| {
            let id = tc.get("id")?.as_str()?.to_string();
            let func = tc.get("function")?;
            let name = func.get("name")?.as_str()?.to_string();
            let args_str = func
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let arguments = serde_json::from_str(args_str).unwrap_or(serde_json::Value::Null);
            Some(ToolCall {
                id,
                function_name: name,
                arguments,
            })
        })
        .collect()
}

/// T-E-S-02: 从 Anthropic 格式的 JSON 解析 tool_calls。
///
/// 输入格式:
/// ```json
/// {"content":[{"type":"tool_use","id":"tu_xxx","name":"read_file","input":{"path":"..."}}]}
/// ```
pub fn parse_anthropic_tool_calls(json: &serde_json::Value) -> Vec<ToolCall> {
    let content = match json.get("content").and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };
    content
        .iter()
        .filter_map(|item| {
            let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty != "tool_use" {
                return None;
            }
            let id = item.get("id")?.as_str()?.to_string();
            let name = item.get("name")?.as_str()?.to_string();
            let input = item
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Some(ToolCall {
                id,
                function_name: name,
                arguments: input,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_serialization() {
        let tc = ToolCall {
            id: "call_abc".to_string(),
            function_name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        };
        let json = serde_json::to_string(&tc).expect("serialize should succeed");
        let back: ToolCall = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(back.id, "call_abc");
        assert_eq!(back.function_name, "read_file");
        assert_eq!(back.arguments["path"], "/tmp/test.txt");

        let tr = ToolResult {
            tool_call_id: "call_abc".to_string(),
            content: "file contents here".to_string(),
            is_error: false,
        };
        let json = serde_json::to_string(&tr).expect("serialize should succeed");
        let back: ToolResult = serde_json::from_str(&json).expect("parse should succeed");
        assert_eq!(back.tool_call_id, "call_abc");
        assert_eq!(back.content, "file contents here");
        assert!(!back.is_error);
    }

    #[test]
    fn test_parse_deepseek_tool_calls() {
        let json = serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_001",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"/etc/hosts\"}"
                        }
                    }]
                }
            }]
        });
        let calls = parse_deepseek_tool_calls(&json);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_001");
        assert_eq!(calls[0].function_name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/etc/hosts");
    }

    #[test]
    fn test_parse_anthropic_tool_calls() {
        let json = serde_json::json!({
            "content": [
                {"type": "text", "text": "Let me read that file."},
                {"type": "tool_use", "id": "tu_001", "name": "read_file", "input": {"path": "/etc/hosts"}}
            ]
        });
        let calls = parse_anthropic_tool_calls(&json);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "tu_001");
        assert_eq!(calls[0].function_name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/etc/hosts");
    }

    #[test]
    fn test_parse_deepseek_empty_choices() {
        let json = serde_json::json!({"choices": []});
        assert!(parse_deepseek_tool_calls(&json).is_empty());
    }

    #[test]
    fn test_parse_anthropic_no_tool_use() {
        let json = serde_json::json!({
            "content": [{"type": "text", "text": "hello"}]
        });
        assert!(parse_anthropic_tool_calls(&json).is_empty());
    }
}
