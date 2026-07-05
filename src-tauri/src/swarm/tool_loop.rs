//! T-E-S-02: 工具调用循环 — LLM function calling 的核心执行引擎。
//!
//! 当 LLM 返回 `tool_calls` 时,本模块负责:
//! 1. 对每个 tool_call,从 ToolRegistry 查找工具并执行
//! 2. 把工具结果作为 `role: "tool"` 消息追加到对话
//! 3. 再次调用 LLM,若仍返回 tool_calls 则继续循环
//! 4. 直到 LLM 返回无 tool_calls(最终 content)或达到 max_iterations
//!
//! ## 错误恢复
//! 工具执行失败时,把错误信息作为 tool 消息 content 追加,
//! 让 LLM 决定下一步(重试 / 换工具 / 放弃)。

use std::time::Instant;

use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::llm::{ChatMessage, ChatResponse, LlmGateway, ToolSpec};
use crate::swarm::events::SwarmEvent;
use crate::tools::{ToolInput, ToolRegistry};

/// T-E-S-02: 工具调用循环的默认最大迭代次数。
pub const DEFAULT_MAX_ITERATIONS: usize = 10;

/// T-E-S-02: 执行 LLM 工具调用循环。
///
/// 流程:
/// 1. 调用 `llm.chat_with_tools(messages, tools)`
/// 2. 若响应无 tool_calls(或为空),返回 `resp.message.content`
/// 3. 若有 tool_calls,对每个 call 执行 `registry.invoke(ToolInput{...})`,
///    把结果(或错误)作为 `role: "tool"` 消息追加
/// 4. 把 assistant 的 tool_calls 响应也追加到 messages
/// 5. 回到步骤 1,直到无 tool_calls 或达 max_iterations
///
/// # 参数
/// - `llm`: LLM 网关
/// - `registry`: 工具注册表
/// - `messages`: 初始消息(含 system + user)
/// - `tools`: 可用工具规格
/// - `max_iterations`: 最大迭代次数(默认 10)
/// - `event_sender`: 可选的 SwarmEvent 广播发送器（用于 emit 工具调用事件）
/// - `agent_id`: 可选的 agent ID（emit 事件时使用）
/// - `agent_role`: 可选的 agent 角色（emit 事件时使用）
/// - `task_id`: 可选的 task ID（emit 事件时使用）
///
/// # 返回
/// 最终 assistant 消息的 content。若达 max_iterations 仍无最终 content,
/// 返回 "max iterations reached" 兜底。
pub async fn run_tool_loop(
    llm: &LlmGateway,
    registry: &ToolRegistry,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSpec>,
    max_iterations: usize,
    event_sender: Option<broadcast::Sender<SwarmEvent>>,
    agent_id: Option<&str>,
    agent_role: Option<&str>,
    task_id: Option<&str>,
) -> anyhow::Result<String> {
    let mut messages = messages;

    for iteration in 0..max_iterations {
        debug!(
            target: "nebula.swarm.tool_loop",
            iteration,
            max_iterations,
            msg_count = messages.len(),
            "tool_loop iteration"
        );

        let resp: ChatResponse =
            llm.chat_with_tools(messages.clone(), tools.clone()).await?;

        let tool_calls = resp.tool_calls.clone().unwrap_or_default();

        if tool_calls.is_empty() {
            // 无 tool_calls,返回最终 content
            let content = resp.message.content.clone();
            info!(
                target: "nebula.swarm.tool_loop",
                iteration,
                "tool_loop completed: no more tool_calls"
            );
            return Ok(content);
        }

        // 有 tool_calls:先把 assistant 消息(含 tool_calls)追加
        messages.push(resp.message.clone());

        // 对每个 tool_call 执行工具
        for call in &tool_calls {
            let tool_name = &call.function.name;
            let args_str = &call.function.arguments;

            // 解析 arguments(JSON 字符串 → Value)
            let args: serde_json::Value =
                serde_json::from_str(args_str).unwrap_or(serde_json::Value::Null);

            debug!(
                target: "nebula.swarm.tool_loop",
                iteration,
                tool = %tool_name,
                "executing tool"
            );

            let start = Instant::now();
            let start_ts = chrono::Utc::now().timestamp_millis();

            let invoke_result = registry.invoke(ToolInput {
                tool_name: tool_name.clone(),
                arguments: args,
            });

            let end_ts = chrono::Utc::now().timestamp_millis();
            let duration_ms = start.elapsed().as_millis() as u64;

            let tool_result: String = match &invoke_result {
                Ok(output) => {
                    debug!(
                        target: "nebula.swarm.tool_loop",
                        tool = %tool_name,
                        success = output.success,
                        "tool executed"
                    );
                    if output.success {
                        output.result.clone()
                    } else {
                        format!("Tool error: {}", output.error.clone().unwrap_or_default())
                    }
                }
                Err(e) => {
                    warn!(
                        target: "nebula.swarm.tool_loop",
                        tool = %tool_name,
                        error = %e,
                        "tool invocation failed"
                    );
                    format!("Error: {}", e)
                }
            };

            // T-E-D-10: emit AgentToolCall 事件（如果在 swarm 上下文中）
            if let (Some(sender), Some(aid), Some(arole), Some(tid)) = (
                event_sender.as_ref(),
                agent_id,
                agent_role,
                task_id,
            ) {
                let (success, output_preview, error) = match &invoke_result {
                    Ok(output) => {
                        let preview = if output.success {
                            Some(output.result.chars().take(200).collect::<String>())
                        } else {
                            None
                        };
                        (output.success, preview, output.error.clone())
                    }
                    Err(e) => (false, None, Some(format!("{e}"))),
                };

                let event = SwarmEvent::agent_tool_call(
                    aid,
                    arole,
                    tool_name,
                    start_ts,
                    end_ts,
                    duration_ms,
                    success,
                    output_preview,
                    error,
                    tid,
                );
                let _ = sender.send(event);
            }

            // 追加 tool 角色消息
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: tool_result,
                tool_calls: None,
                tool_call_id: Some(call.id.clone()),
                name: Some(tool_name.clone()),
                turn_id: None,
                images: Vec::new(),
            });
        }
    }

    // 达到 max_iterations,返回兜底
    warn!(
        target: "nebula.swarm.tool_loop",
        max_iterations,
        "tool_loop reached max iterations"
    );
    Ok("max iterations reached".to_string())
}

/// T-E-S-02: 便捷包装,使用默认最大迭代次数(10)。
pub async fn run_tool_loop_default(
    llm: &LlmGateway,
    registry: &ToolRegistry,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSpec>,
) -> anyhow::Result<String> {
    run_tool_loop(
        llm,
        registry,
        messages,
        tools,
        DEFAULT_MAX_ITERATIONS,
        None,
        None,
        None,
        None,
    )
    .await
}

/// T-E-D-10: 带事件发射器的便捷包装,使用默认最大迭代次数(10)。
pub async fn run_tool_loop_with_events(
    llm: &LlmGateway,
    registry: &ToolRegistry,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolSpec>,
    event_sender: broadcast::Sender<SwarmEvent>,
    agent_id: &str,
    agent_role: &str,
    task_id: &str,
) -> anyhow::Result<String> {
    run_tool_loop(
        llm,
        registry,
        messages,
        tools,
        DEFAULT_MAX_ITERATIONS,
        Some(event_sender),
        Some(agent_id),
        Some(agent_role),
        Some(task_id),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_iterations_is_ten() {
        assert_eq!(DEFAULT_MAX_ITERATIONS, 10);
    }

    #[test]
    fn tool_message_construction() {
        let msg = ChatMessage {
            role: "tool".to_string(),
            content: "result data".to_string(),
            tool_calls: None,
            tool_call_id: Some("call_abc".to_string()),
            name: Some("shell".to_string()),
            turn_id: None,
            images: Vec::new(),
        };
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_abc"));
        assert_eq!(msg.name.as_deref(), Some("shell"));
    }
}
